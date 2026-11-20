//! Windows 專屬實作。
//!
//! 所有 `unsafe` 都集中在這裡；呼叫端（`main.rs`）只看到 [`Error`] 與五個動作。

use std::ffi::c_void;
use std::path::Path;

use thiserror::Error;
use windows::core::{GUID, HRESULT, PCSTR, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    FreeLibrary, LocalFree, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HLOCAL,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSidToSidW, GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW,
    EXPLICIT_ACCESS_W, GRANT_ACCESS, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, TRUSTEE_IS_SID,
    TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
};
use windows::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SUB_CONTAINERS_AND_OBJECTS_INHERIT,
};
use windows::Win32::Storage::FileSystem::{FILE_GENERIC_EXECUTE, FILE_GENERIC_READ};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteTreeW, RegOpenKeyExW, HKEY, HKEY_CLASSES_ROOT, HKEY_LOCAL_MACHINE,
    KEY_READ, KEY_WOW64_64KEY,
};
use windows::Win32::UI::TextServices::{
    CLSID_TF_CategoryMgr, CLSID_TF_InputProcessorProfiles, ITfCategoryMgr,
    ITfInputProcessorProfiles, GUID_TFCAT_TIP_KEYBOARD,
};

use crate::guids::{
    CLSID_JC_TSF, ILOT_INSTALL, ILOT_UNINSTALL, LANGID_ZH_TW, PROFILE_JC_TSF, TIP_STRING,
};

/// 輔助程式可能回報的錯誤。
#[derive(Debug, Error)]
pub enum Error {
    /// 命令列用法錯誤；`main` 會印出說明並回 2。
    #[error("{0}")]
    Usage(String),
    /// 一般的執行失敗。
    #[error("{0}")]
    Failure(String),
    /// 檔案系統操作失敗。
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Windows API 透過 `windows::core::Error` 回報的錯誤。
    #[error(transparent)]
    Windows(#[from] windows::core::Error),
}

/// `jc-tsf` 的 profile GUID；與 `guids.rs` 的字串版本相同。
const PROFILE_GUID: GUID = GUID::from_u128(0x8f0a9b3c_2d4e_4f50_a1b2_c3d4e5f60718);

/// `jc-tsf` 的 CLSID；與 `guids.rs` 的字串版本相同。
const CLSID_GUID: GUID = GUID::from_u128(0x7b7d6e1a_3c1f_4b2e_9a55_6c1d2f3e4a01);

/// 註冊文字服務：載入 DLL 呼叫 `DllRegisterServer`，再驗證 CTF 機碼。
pub fn machine_register(dll: &Path) -> Result<(), Error> {
    if !dll.is_file() {
        return Err(Error::Failure(format!(
            "找不到 {}；安裝套件不完整",
            dll.display()
        )));
    }

    let hr = call_export(dll, b"DllRegisterServer\0")?;
    if hr.is_err() {
        return Err(Error::Failure(format!(
            "{} 的 DllRegisterServer 回傳 {}。若系統還沒有安裝繁體中文（台灣）語言，\
             請先到「設定 → 時間與語言 → 語言」加入後再安裝。",
            dll.display(),
            hex(hr.0 as u32)
        )));
    }

    ensure_tip_key()?;

    // AppContainer（UWP）要讀得到安裝目錄才能載入 TIP。失敗只警告：
    // 桌面程式不受影響，而且安裝不該因為 ACL 而中斷。
    if let Some(dir) = dll.parent() {
        if let Err(error) = grant_app_container(dir) {
            eprintln!(
                "jc-tip-register：設定 AppContainer 讀取權限失敗（桌面程式不受影響）：{error}"
            );
        }
    }
    Ok(())
}

/// 移除文字服務。
///
/// 先嘗試 `DllUnregisterServer`；DLL 不在或回報失敗時，改用內建的退路直接
/// 移除 CTF profile 與分類，最後一律刪掉 `HKCR\CLSID\{...}`。這樣防毒隔離
/// DLL 之後，反安裝仍不會留下無法移除的 TIP。
pub fn machine_unregister(dll: Option<&Path>) -> Result<(), Error> {
    let mut handled = false;
    if let Some(dll) = dll {
        if dll.is_file() {
            match call_export(dll, b"DllUnregisterServer\0") {
                Ok(hr) if hr.is_ok() => handled = true,
                Ok(hr) => eprintln!(
                    "jc-tip-register：DllUnregisterServer 回傳 {}，改用內建清理",
                    hex(hr.0 as u32)
                ),
                Err(error) => eprintln!("jc-tip-register：{error}；改用內建清理"),
            }
        } else {
            eprintln!("jc-tip-register：{} 不存在，改用內建清理", dll.display());
        }
    }

    if !handled {
        unregister_by_api();
    }
    delete_clsid_tree()
}

/// 把 profile 加進目前使用者的語言清單。
pub fn user_install() -> Result<(), Error> {
    if let Some(dir) = user_dir() {
        std::fs::create_dir_all(&dir)?;
    }
    layout_or_tip(ILOT_INSTALL)
}

/// 從目前使用者的語言清單移除；`keep_user_data` 為 `false` 時一併刪除
/// `%LOCALAPPDATA%\jc-ime`（settings.json、personal.json、tsf.log）。
pub fn user_uninstall(keep_user_data: bool) -> Result<(), Error> {
    let removed = layout_or_tip(ILOT_UNINSTALL);
    if !keep_user_data {
        if let Some(dir) = user_dir() {
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => eprintln!(
                    "jc-tip-register：移除 {} 失敗（語言清單已處理）：{error}",
                    dir.display()
                ),
            }
        }
    }
    removed
}

/// 啟動 `jc-settings-ui.exe`。
///
/// 捷徑只帶 `settings` 一個字，`%LOCALAPPDATA%` 由這裡展開。MSI 建置的 `.lnk`
/// 不會在啟動時展開環境變數；把路徑寫死在 per-machine 的捷徑也對其他使用者
/// 不成立，所以交給這支程式在執行時組合。
pub fn open_settings() -> Result<(), Error> {
    let current = std::env::current_exe()?;
    let dir = current
        .parent()
        .ok_or_else(|| Error::Failure("無法取得安裝目錄".into()))?;
    let ui = dir.join("jc-settings-ui.exe");
    if !ui.is_file() {
        return Err(Error::Failure(format!(
            "找不到 {}；安裝目錄不完整",
            ui.display()
        )));
    }

    let program_data = std::env::var_os("ProgramData")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .ok_or_else(|| Error::Failure("讀不到 %ProgramData%".into()))?;
    let data = program_data.join("jc-ime").join("data");

    let settings_dir = user_dir().ok_or_else(|| Error::Failure("讀不到 %LOCALAPPDATA%".into()))?;
    std::fs::create_dir_all(&settings_dir)?;
    let settings = settings_dir.join("settings.json");

    std::process::Command::new(&ui)
        .arg("--data")
        .arg(&data)
        .arg("--settings")
        .arg(&settings)
        .spawn()
        .map_err(|error| Error::Failure(format!("啟動 {} 失敗：{error}", ui.display())))?;
    Ok(())
}

/// 讀 `%LOCALAPPDATA%` 下的 `jc-ime`。
fn user_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    if base.is_empty() {
        return None;
    }
    Some(std::path::PathBuf::from(base).join("jc-ime"))
}

/// 載入 DLL、取出匯出函式、呼叫後釋放。回傳 DLL 的 `HRESULT`。
fn call_export(dll: &Path, name: &[u8]) -> Result<HRESULT, Error> {
    let path = wide(&dll.to_string_lossy());
    // SAFETY: path 以 NUL 結尾且呼叫期間保持存活。
    let module = unsafe { LoadLibraryW(PCWSTR(path.as_ptr())) }.map_err(|error| {
        Error::Failure(format!("LoadLibraryW({}) 失敗：{error}", dll.display()))
    })?;

    let result = (|| {
        // SAFETY: name 是以 NUL 結尾的 ASCII 名稱。
        let proc = unsafe { GetProcAddress(module, PCSTR(name.as_ptr())) }.ok_or_else(|| {
            Error::Failure(format!(
                "{} 沒有匯出 {}",
                dll.display(),
                String::from_utf8_lossy(&name[..name.len() - 1])
            ))
        })?;
        // SAFETY: 匯出符號的簽章是 `HRESULT WINAPI DllRegisterServer(void)`；
        // 兩個函式都是 extern "system"。
        let function: unsafe extern "system" fn() -> HRESULT = unsafe { std::mem::transmute(proc) };
        // SAFETY: 由 DLL 自己保證簽章，這裡只呼叫一次。
        Ok(unsafe { function() })
    })();

    // SAFETY: module 由上面的 LoadLibraryW 取得。
    unsafe {
        let _ = FreeLibrary(module);
    }
    result
}

/// 確認 `HKLM\SOFTWARE\Microsoft\CTF\TIP\{CLSID}` 已經出現。
fn ensure_tip_key() -> Result<(), Error> {
    let path = wide(&format!(r"SOFTWARE\Microsoft\CTF\TIP\{CLSID_JC_TSF}"));
    let mut key = HKEY::default();
    // SAFETY: path 以 NUL 結尾；key 成功時由 RegOpenKeyExW 寫入。
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(path.as_ptr()),
            None,
            KEY_READ | KEY_WOW64_64KEY,
            &mut key,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(Error::Failure(format!(
            "註冊後仍找不到 HKLM\\SOFTWARE\\Microsoft\\CTF\\TIP\\{CLSID_JC_TSF}\\
             \\LanguageProfile\\{LANGID_ZH_TW:#06X}\\{PROFILE_JC_TSF}（{}）。\
             這通常表示 AddLanguageProfile 失敗；請確認已安裝繁體中文（台灣）語言。",
            hex(status.0)
        )));
    }
    // SAFETY: key 由成功的 RegOpenKeyExW 取得。
    unsafe {
        let _ = RegCloseKey(key);
    }
    Ok(())
}

/// 不依賴 DLL 的移除路徑：直接呼叫 TSF 的介面移除 profile 與分類。
fn unregister_by_api() {
    // SAFETY: COM 未初始化時 CoCreateInstance 會失敗，忽略即可。
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if let Ok(profiles) = CoCreateInstance::<_, ITfInputProcessorProfiles>(
            &CLSID_TF_InputProcessorProfiles,
            None,
            CLSCTX_INPROC_SERVER,
        ) {
            let _ = profiles.RemoveLanguageProfile(&CLSID_GUID, LANGID_ZH_TW, &PROFILE_GUID);
            let _ = profiles.Unregister(&CLSID_GUID);
        }
        if let Ok(categories) =
            CoCreateInstance::<_, ITfCategoryMgr>(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)
        {
            let _ =
                categories.UnregisterCategory(&CLSID_GUID, &GUID_TFCAT_TIP_KEYBOARD, &CLSID_GUID);
        }
    }
}

/// 刪掉 `HKCR\CLSID\{...}` 整棵樹。
fn delete_clsid_tree() -> Result<(), Error> {
    let path = wide(&format!(r"CLSID\{CLSID_JC_TSF}"));
    // SAFETY: path 以 NUL 結尾。
    let status = unsafe { RegDeleteTreeW(HKEY_CLASSES_ROOT, PCWSTR(path.as_ptr())) };
    if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(Error::Failure(format!(
            r"刪除 HKCR\CLSID\{CLSID_JC_TSF} 失敗：{}",
            hex(status.0)
        )))
    }
}

/// 呼叫 `input.dll` 的 `InstallLayoutOrTip`。
fn layout_or_tip(flags: u32) -> Result<(), Error> {
    let module = wide("input.dll");
    // SAFETY: 模組名稱以 NUL 結尾。
    let input = unsafe { LoadLibraryW(PCWSTR(module.as_ptr())) }
        .map_err(|error| Error::Failure(format!("載入 input.dll 失敗：{error}")))?;

    let result = (|| {
        // SAFETY: 名稱以 NUL 結尾。
        let proc =
            unsafe { GetProcAddress(input, PCSTR(c"InstallLayoutOrTip".as_ptr() as *const u8)) }
                .ok_or_else(|| Error::Failure("input.dll 沒有 InstallLayoutOrTip".into()))?;
        // SAFETY: 簽章是 `BOOL WINAPI InstallLayoutOrTip(LPCWSTR, DWORD)`。
        let function: unsafe extern "system" fn(*const u16, u32) -> i32 =
            unsafe { std::mem::transmute(proc) };
        let tip = wide(TIP_STRING);
        // SAFETY: tip 以 NUL 結尾且呼叫期間保持存活。
        let ok = unsafe { function(tip.as_ptr(), flags) };
        if ok == 0 {
            Err(Error::Failure(format!(
                "InstallLayoutOrTip({TIP_STRING}, 0x{flags:08X}) 失敗"
            )))
        } else {
            Ok(())
        }
    })();

    // SAFETY: input 由上面的 LoadLibraryW 取得。
    unsafe {
        let _ = FreeLibrary(input);
    }
    result
}

/// 給 `ALL APPLICATION PACKAGES` 與 `ALL RESTRICTED APPLICATION PACKAGES`
/// 加上安裝目錄的讀取／執行權限，UWP 應用程式才載入得到 TIP。
fn grant_app_container(dir: &Path) -> Result<(), Error> {
    /// 兩個 AppContainer 的 SID 字串。
    const SIDS: [&str; 2] = ["S-1-15-2-1", "S-1-15-2-2"];

    let dir_wide = wide(&dir.to_string_lossy());
    let permissions = FILE_GENERIC_READ.0 | FILE_GENERIC_EXECUTE.0;

    let mut entries = Vec::with_capacity(SIDS.len());
    let mut allocated: Vec<PSID> = Vec::with_capacity(SIDS.len());
    for text in SIDS {
        let text_wide = wide(text);
        let mut sid = PSID::default();
        // SAFETY: text_wide 以 NUL 結尾；成功時 sid 由 LocalAlloc 配置。
        let status = unsafe { ConvertStringSidToSidW(PCWSTR(text_wide.as_ptr()), &mut sid) };
        if status.is_err() {
            free_sids(&allocated);
            return Err(Error::Failure(format!("無法解析 SID {text}")));
        }
        allocated.push(sid);
        entries.push(EXPLICIT_ACCESS_W {
            grfAccessPermissions: permissions,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: std::ptr::null_mut(),
                MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                ptstrName: PWSTR(sid.0 as *mut u16),
            },
        });
    }

    let mut old_dacl: *mut ACL = std::ptr::null_mut();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: dir_wide 以 NUL 結尾；old_dacl 與 descriptor 都是有效輸出位址。
    let status = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(dir_wide.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut old_dacl),
            None,
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        free_sids(&allocated);
        return Err(Error::Failure(format!(
            "GetNamedSecurityInfoW({}) = {}",
            dir.display(),
            hex(status.0)
        )));
    }

    let mut new_dacl: *mut ACL = std::ptr::null_mut();
    // SAFETY: entries 是剛才組好的陣列；old_dacl 由 GetNamedSecurityInfoW 取得。
    let status = unsafe { SetEntriesInAclW(Some(&entries), Some(old_dacl), &mut new_dacl) };
    if status != ERROR_SUCCESS {
        // SAFETY: descriptor 由 GetNamedSecurityInfoW 配置。
        unsafe {
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        }
        free_sids(&allocated);
        return Err(Error::Failure(format!(
            "SetEntriesInAclW = {}",
            hex(status.0)
        )));
    }

    // SAFETY: 三個指標都來自上面的成功呼叫。
    let status = unsafe {
        SetNamedSecurityInfoW(
            PCWSTR(dir_wide.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(new_dacl),
            None,
        )
    };

    // SAFETY: new_dacl 由 SetEntriesInAclW 配置、descriptor 由
    // GetNamedSecurityInfoW 配置，兩者都只釋放一次。
    unsafe {
        let _ = LocalFree(Some(HLOCAL(new_dacl as *mut c_void)));
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
    }
    free_sids(&allocated);

    if status != ERROR_SUCCESS {
        return Err(Error::Failure(format!(
            "SetNamedSecurityInfoW({}) = {}",
            dir.display(),
            hex(status.0)
        )));
    }
    Ok(())
}

/// 釋放 `ConvertStringSidToSidW` 配置的 SID。
fn free_sids(sids: &[PSID]) {
    for sid in sids {
        // SAFETY: sid 由 ConvertStringSidToSidW 配置，只釋放一次。
        unsafe {
            let _ = LocalFree(Some(HLOCAL(sid.0)));
        }
    }
}

/// 把字串轉成以 NUL 結尾的 UTF-16。
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 格式化 Win32 錯誤碼。
fn hex(code: u32) -> String {
    format!("0x{code:08X}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_constants_match_strings() {
        assert_eq!(guid_text(&CLSID_GUID), CLSID_JC_TSF);
        assert_eq!(guid_text(&PROFILE_GUID), PROFILE_JC_TSF);
    }

    #[test]
    fn windows_are_nul_terminated() {
        let value = wide("jc");
        assert_eq!(value, vec![0x006A, 0x0063, 0]);
    }

    fn guid_text(guid: &GUID) -> String {
        let d = guid.data4;
        format!(
            "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
            guid.data1, guid.data2, guid.data3, d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]
        )
    }
}
