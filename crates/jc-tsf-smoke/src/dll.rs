//! `jc-tsf.dll` 的載入與匯出呼叫。
//!
//! 只用 `LoadLibraryW` 與 `GetProcAddress`，不經過 COM 的註冊機制，因此不需要
//! 管理員、也不需要把 DLL 註冊進 `HKLM`。這正是「免管理員冒煙」的基礎。

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::core::{Interface, GUID, HRESULT, PCSTR, PCWSTR};
use windows::Win32::Foundation::{FreeLibrary, E_POINTER, FARPROC, HMODULE};
use windows::Win32::System::Com::IClassFactory;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

use crate::error::SmokeError;
use crate::report;

/// 預設的 DLL 檔名；找不到同目錄的檔案時交給 `LoadLibraryW` 的搜尋路徑。
pub const DLL_NAME: &str = "jc-tsf.dll";

/// 指定 DLL 路徑的環境變數。
pub const DLL_ENV: &str = "JC_TSF_DLL";

/// `DllGetClassObject` 的函式型別（COM 規範固定）。
pub type DllGetClassObjectFn =
    unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;

/// `DllCanUnloadNow` 的函式型別。
pub type DllCanUnloadNowFn = unsafe extern "system" fn() -> HRESULT;

/// `jc_test_key` 的函式型別。
pub type TestKeyFn = extern "system" fn(u32) -> i32;

/// 已載入的 `jc-tsf.dll`；丟掉時自動 `FreeLibrary`。
pub struct Library {
    module: HMODULE,
}

impl Library {
    /// 以 `LoadLibraryW` 載入指定路徑的 DLL。
    pub fn load(path: &Path) -> Result<Self, SmokeError> {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: wide 以 NUL 結尾，LoadLibraryW 只讀這塊緩衝。
        match unsafe { LoadLibraryW(PCWSTR(wide.as_ptr())) } {
            Ok(module) => Ok(Self { module }),
            Err(error) => Err(SmokeError::Load {
                path: path.display().to_string(),
                code: format!(
                    "{}（{}）",
                    report::hex(error.code().0 as u32),
                    error.message()
                ),
            }),
        }
    }

    /// 呼叫 `DllGetClassObject`，只接受 `IClassFactory`。
    pub fn get_class_object(&self, clsid: &GUID) -> Result<IClassFactory, SmokeError> {
        let address = self
            .proc(b"DllGetClassObject\0")
            .ok_or(SmokeError::MissingExport("DllGetClassObject"))?;
        // SAFETY: 匯出簽章由 COM 規範固定，DLL 與冒煙程式來自同一個 workspace。
        let function: DllGetClassObjectFn = unsafe { std::mem::transmute(address) };
        let mut raw: *mut c_void = std::ptr::null_mut();
        // SAFETY: clsid 是有效的 GUID；raw 是本機的有效輸出位置。
        let hr = unsafe { function(clsid, &IClassFactory::IID, &mut raw) };
        if hr.is_err() {
            return Err(SmokeError::hresult("DllGetClassObject", hr));
        }
        if raw.is_null() {
            return Err(SmokeError::hresult("DllGetClassObject", E_POINTER));
        }
        // SAFETY: 回 S_OK 且非空，所有權轉移給 IClassFactory。
        Ok(unsafe { IClassFactory::from_raw(raw) })
    }

    /// 呼叫 `DllCanUnloadNow` 並回傳原始 `HRESULT`。
    ///
    /// 找不到匯出回 [`SmokeError::MissingExport`]；這是檢查 1 的判定條件之一。
    pub fn can_unload_now(&self) -> Result<HRESULT, SmokeError> {
        let address = self
            .proc(b"DllCanUnloadNow\0")
            .ok_or(SmokeError::MissingExport("DllCanUnloadNow"))?;
        // SAFETY: 匯出沒有參數，回傳 HRESULT。
        let function: DllCanUnloadNowFn = unsafe { std::mem::transmute(address) };
        Ok(unsafe { function() })
    }

    /// 取得 `jc_test_key`；只有 debug 建置或開了 `test-export` feature 的 DLL 才有。
    pub fn test_key(&self) -> Option<TestKeyFn> {
        let address = self.proc(b"jc_test_key\0")?;
        // SAFETY: 簽章由 jc-tsf 的匯出定義固定。
        let function: TestKeyFn = unsafe { std::mem::transmute(address) };
        Some(function)
    }

    /// 找一個匯出函式。
    fn proc(&self, name: &'static [u8]) -> FARPROC {
        debug_assert!(name.ends_with(b"\0"), "匯出名稱要以 NUL 結尾");
        // SAFETY: name 是以 NUL 結尾的 ASCII；module 在 Drop 前都有效。
        unsafe { GetProcAddress(self.module, PCSTR(name.as_ptr())) }
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: module 由 LoadLibraryW 取得，這裡是唯一放手的地方。
        let _ = unsafe { FreeLibrary(self.module) };
    }
}

/// 決定要載入哪一個 DLL。
///
/// 順序：`--dll`／位置參數 → 環境變數 [`DLL_ENV`] → 執行檔同目錄 → 交給
/// `LoadLibraryW` 的標準搜尋路徑（目前目錄與 `PATH`）。
pub fn resolve_path(explicit: Option<&str>) -> PathBuf {
    if let Some(path) = explicit {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os(DLL_ENV) {
        return PathBuf::from(path);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(directory) = exe.parent() {
            let beside = directory.join(DLL_NAME);
            if beside.is_file() {
                return beside;
            }
        }
    }
    PathBuf::from(DLL_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_path_wins() {
        assert_eq!(
            resolve_path(Some(r"C:\tmp\jc-tsf.dll")),
            PathBuf::from(r"C:\tmp\jc-tsf.dll")
        );
    }

    #[test]
    fn default_is_dll_name() {
        // 沒有參數、沒有環境變數時，最差也會回到裸檔名讓 LoadLibraryW 自己找。
        let path = resolve_path(None);
        assert!(path.ends_with(DLL_NAME));
    }
}
