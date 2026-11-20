//! Windows 專屬實作。
//!
//! 所有 `unsafe` 都集中在這裡；呼叫端（`main.rs`）只看到 [`Error`] 與六個動作。
//!
//! 兩個架構共用同一個 CLSID 與 profile，差別只有 `InprocServer32` 寫在哪一個
//! 機碼檢視：
//!
//! | 目標 | 檢視 | 值 |
//! |---|---|---|
//! | x64 `jc-tsf.dll` | `KEY_WOW64_64KEY` | `%ProgramFiles%\jc-ime\jc-tsf.dll` |
//! | x86 `jc-tsf-x86.dll` | `KEY_WOW64_32KEY` | `%ProgramFiles%\jc-ime\jc-tsf-x86.dll` |
//!
//! 註冊分兩條路徑：
//!
//! - **同架構**（x64 安裝器 + x64 DLL）：`LoadLibraryW` 之後呼叫 DLL 自己的
//!   `DllRegisterServer`。
//! - **跨架構**（x64 安裝器 + x86 DLL）：Windows 不允許 64 位元行程載入 32 位元
//!   DLL，所以由這裡以 `KEY_WOW64_32KEY` 寫 `InprocServer32`。
//!
//! TSF 的 profile 與分類（`HKLM\SOFTWARE\Microsoft\CTF`）不是 WOW64 轉向機碼，
//! 兩個架構讀寫同一份資料，所以整支程式只註冊、也只移除一次。
//!
//! 移除一律刪兩個檢視的 `CLSID` 機碼，不依賴 DLL 還在：`UnregisterTip` 在
//! `RemoveFiles` 之前跑，但防毒可能早就把 DLL 隔離掉了。

use std::ffi::c_void;
use std::path::{Path, PathBuf};

use thiserror::Error;
use windows::core::{GUID, HRESULT, PCSTR, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    FreeLibrary, LocalFree, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HLOCAL,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSidToSidW, GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW,
    EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE, SET_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID,
    TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
};
use windows::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SUB_CONTAINERS_AND_OBJECTS_INHERIT,
};
use windows::Win32::Storage::FileSystem::{FILE_GENERIC_EXECUTE, FILE_GENERIC_READ};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegOpenKeyExW, RegSetValueExW, HKEY,
    HKEY_CLASSES_ROOT, HKEY_LOCAL_MACHINE, KEY_ALL_ACCESS, KEY_READ, KEY_WOW64_32KEY,
    KEY_WOW64_64KEY, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_SZ,
};
use windows::Win32::UI::TextServices::{
    CLSID_TF_CategoryMgr, CLSID_TF_InputProcessorProfiles, ITfCategoryMgr,
    ITfInputProcessorProfiles, GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER,
    GUID_TFCAT_TIPCAP_UIELEMENTENABLED, GUID_TFCAT_TIP_KEYBOARD,
};

use crate::arch::Arch;
use crate::guids::{
    tip_string, CLSID_JC_TSF, ILOT_INSTALL, ILOT_UNINSTALL, LANGID_ZH_TW, PROFILE_DESCRIPTION,
    PROFILE_JC_TSF,
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

/// 文字服務的 CLSID；與 `guids.rs` 的字串版本相同。
const CLSID_GUID: GUID = GUID::from_u128(0x7b7d6e1a_3c1f_4b2e_9a55_6c1d2f3e4a01);

/// 注音 profile 的 GUID；與 `guids.rs` 的字串版本相同。
const PROFILE_GUID: GUID = GUID::from_u128(0x8f0a9b3c_2d4e_4f50_a1b2_c3d4e5f60718);

/// 一個架構要寫入的機碼檢視與它的 DLL。
#[derive(Debug, Clone, Copy)]
struct Target {
    /// x64 或 x86。
    arch: Arch,
    /// `HKCR\CLSID` 要開哪一個檢視。
    view: REG_SAM_FLAGS,
}

impl Target {
    /// 某個架構對應的機碼檢視。
    fn new(arch: Arch) -> Self {
        match arch {
            Arch::X64 => Self {
                arch,
                view: KEY_WOW64_64KEY,
            },
            Arch::X86 => Self {
                arch,
                view: KEY_WOW64_32KEY,
            },
        }
    }
}

/// 解析目標架構：`--arch` 優先，否則讀 PE 標頭。
fn resolve_arch(dll: &Path, requested: Option<Arch>) -> Result<Arch, Error> {
    match requested {
        Some(requested) => {
            // 明確指定卻與檔案不符時擋下來：寫錯檢視會留下一個永遠載入不了的 TIP。
            if let Ok(detected) = Arch::detect(dll) {
                if detected != requested {
                    return Err(Error::Failure(format!(
                        "{} 實際上是 {detected}，與 --arch {requested} 不符",
                        dll.display()
                    )));
                }
            }
            Ok(requested)
        }
        None => Arch::detect(dll).map_err(Error::Failure),
    }
}

/// 註冊文字服務：每個 DLL 寫自己的機碼檢視，profile 只註冊一次。
///
/// `dlls` 通常是 `[jc-tsf.dll, jc-tsf-x86.dll]`；也可以只給一個，用於手動修復。
pub fn machine_register(dlls: &[PathBuf], requested: Option<Arch>) -> Result<(), Error> {
    let mut need_store = false;
    let mut directory = None;

    for dll in dlls {
        if !dll.is_file() {
            return Err(Error::Failure(format!(
                "找不到 {}；安裝套件不完整",
                dll.display()
            )));
        }
        let arch = resolve_arch(dll, requested)?;
        directory.get_or_insert_with(|| dll.parent().map(Path::to_path_buf));

        if arch.is_current_process() {
            let hr = call_export(dll, b"DllRegisterServer\0")?;
            if hr.is_err() {
                return Err(Error::Failure(format!(
                    "{} 的 DllRegisterServer 回傳 {}。若系統還沒有安裝繁體中文（台灣）\
                     語言，請先到「設定 → 時間與語言 → 語言」加入後再安裝。",
                    dll.display(),
                    hex(hr.0 as u32)
                )));
            }
        } else {
            // 32 位元的 DLL 不能在 64 位元的行程裡執行，只能自己寫同一組機碼。
            eprintln!(
                "jc-tip-register：{} 是 {arch}，由安裝器直接註冊（跨架構不載入 DLL）",
                dll.display()
            );
            let target = Target::new(arch);
            write_com_registration(target, &dll.to_string_lossy())?;
            need_store = true;
        }
    }

    // 同架構的 DLL 註冊時已經寫過 profile；只有跨架構那條路徑需要補。
    if need_store {
        register_in_store()?;
    }
    ensure_tip_key()?;

    // AppContainer（UWP）要讀得到安裝目錄才能載入 TIP。兩個架構在同一個目錄，
    // 但 ACL 是冪等的（`SET_ACCESS`），重跑不會堆疊重複的 ACE。
    // 失敗只警告：桌面程式不受影響，而且安裝不該因為 ACL 而中斷。
    if let Some(Some(dir)) = directory {
        if let Err(error) = grant_app_container(&dir) {
            eprintln!(
                "jc-tip-register：設定 AppContainer 讀取權限失敗（桌面程式不受影響）：{error}"
            );
        }
    }
    Ok(())
}

/// 寫 `HKCR\CLSID\{...}\InprocServer32`；檢視由 `target.view` 決定。
fn write_com_registration(target: Target, dll: &str) -> Result<(), Error> {
    let base = format!("CLSID\\{CLSID_JC_TSF}");
    let inproc = format!("{base}\\InprocServer32");
    set_string(target, &inproc, None, dll)?;
    set_string(target, &inproc, Some("ThreadingModel"), "Apartment")?;
    Ok(())
}

/// 寫一個 `REG_SZ` 值；`name` 為 `None` 時寫預設值。
///
/// 用 `RegCreateKeyExW` 才能明確指定檢視；`RegCreateKeyW` 只有預設檢視。
fn set_string(target: Target, path: &str, name: Option<&str>, value: &str) -> Result<(), Error> {
    let path_wide = wide(path);
    let mut key = HKEY::default();
    // SAFETY: path_wide 以 NUL 結尾，key 由 RegCloseKey 釋放。
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CLASSES_ROOT,
            PCWSTR(path_wide.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE | target.view,
            None,
            &mut key,
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(Error::Failure(format!(
            "建立 HKCR\\{path}（{}）失敗：{}",
            target.arch,
            hex(status.0)
        )));
    }

    let name_wide = name.map(wide);
    let value_wide = wide(value);
    let bytes: Vec<u8> = value_wide
        .iter()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    let pointer = name_wide
        .as_ref()
        .map_or(PCWSTR::null(), |text| PCWSTR(text.as_ptr()));

    // SAFETY: bytes 是完整的 UTF-16LE 內容，含結尾 NUL。
    let status = unsafe { RegSetValueExW(key, pointer, None, REG_SZ, Some(&bytes)) };
    // SAFETY: key 是上面建立的有效 handle。
    unsafe {
        let _ = RegCloseKey(key);
    }
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(Error::Failure(format!(
            "寫入 HKCR\\{path} 失敗：{}",
            hex(status.0)
        )))
    }
}

/// 註冊 TSF profile 與三個分類；內容與 `crates/jc-tsf/src/register.rs` 相同。
///
/// `HKLM\SOFTWARE\Microsoft\CTF` 不是 WOW64 轉向機碼，兩個架構共用同一份，
/// 所以只呼叫一次。
fn register_in_store() -> Result<(), Error> {
    // SAFETY: CoInitializeEx 對已初始化的執行緒是無害的。
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let profiles: ITfInputProcessorProfiles =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;
        profiles.Register(&CLSID_GUID)?;

        let name = wide(PROFILE_DESCRIPTION);
        // 去掉結尾的 NUL：cchDesc 是字元數，不是緩衝長度。
        let name = &name[..name.len() - 1];
        profiles.AddLanguageProfile(&CLSID_GUID, LANGID_ZH_TW, &PROFILE_GUID, name, &[], 0)?;

        let categories: ITfCategoryMgr =
            CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        // 沒有 `GUID_TFCAT_TIP_KEYBOARD`，輸入法設定頁不會列出來。
        categories.RegisterCategory(&CLSID_GUID, &GUID_TFCAT_TIP_KEYBOARD, &CLSID_GUID)?;
        // 沒有這個能力分類，Windows 不會接受 `ITfUIElementMgr::BeginUIElement`。
        categories.RegisterCategory(
            &CLSID_GUID,
            &GUID_TFCAT_TIPCAP_UIELEMENTENABLED,
            &CLSID_GUID,
        )?;
        // 組字底線：這個 CLSID 同時是 `ITfDisplayAttributeProvider` 的實作。
        categories.RegisterCategory(
            &CLSID_GUID,
            &GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER,
            &CLSID_GUID,
        )?;
    }
    Ok(())
}

/// 移除文字服務：profile 只清一次，兩個檢視的 CLSID 機碼都刪。
///
/// 給了 DLL 時，同架構的那一個優先交給它自己的 `DllUnregisterServer`（它只清
/// 自己那個檢視），另一個檢視由這裡補。DLL 不在、讀不出架構或回報失敗都不影響
/// 流程：反安裝一定要清乾淨。
pub fn machine_unregister(dlls: &[PathBuf]) -> Result<(), Error> {
    let mut handled: Vec<Arch> = Vec::new();
    for dll in dlls {
        if !dll.is_file() {
            eprintln!("jc-tip-register：{} 不存在，改用內建清理", dll.display());
            continue;
        }
        let arch = match Arch::detect(dll) {
            Ok(arch) => arch,
            Err(error) => {
                eprintln!("jc-tip-register：{error}；改用內建清理");
                continue;
            }
        };
        if !arch.is_current_process() {
            eprintln!(
                "jc-tip-register：{} 是 {arch}，由安裝器直接移除（跨架構不載入 DLL）",
                dll.display()
            );
            continue;
        }
        match call_export(dll, b"DllUnregisterServer\0") {
            Ok(hr) if hr.is_ok() => handled.push(arch),
            Ok(hr) => eprintln!(
                "jc-tip-register：DllUnregisterServer 回傳 {}，改用內建清理",
                hex(hr.0 as u32)
            ),
            Err(error) => eprintln!("jc-tip-register：{error}；改用內建清理"),
        }
    }

    unregister_in_store();
    for arch in Arch::ALL {
        if handled.contains(&arch) {
            continue;
        }
        delete_clsid_tree(Target::new(arch))?;
    }
    Ok(())
}

/// 移除 TSF 的 profile 與分類；個別步驟失敗不阻止其他步驟。
fn unregister_in_store() {
    // SAFETY: 與註冊相同，COM 沒有初始化時 CoCreateInstance 會失敗，忽略即可。
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
            let _ = categories.UnregisterCategory(
                &CLSID_GUID,
                &GUID_TFCAT_TIPCAP_UIELEMENTENABLED,
                &CLSID_GUID,
            );
            let _ = categories.UnregisterCategory(
                &CLSID_GUID,
                &GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER,
                &CLSID_GUID,
            );
        }
    }
}

/// 刪掉指定檢視的 `HKCR\CLSID\{...}` 整棵樹。
///
/// `RegDeleteTreeW` 沿用父機碼 handle 的檢視，所以先用 `target.view` 開
/// `HKCR\CLSID`，再相對刪除子機碼。
fn delete_clsid_tree(target: Target) -> Result<(), Error> {
    let path = wide("CLSID");
    let mut root = HKEY::default();
    // SAFETY: path 以 NUL 結尾；root 成功時由 RegOpenKeyExW 寫入。
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CLASSES_ROOT,
            PCWSTR(path.as_ptr()),
            None,
            KEY_ALL_ACCESS | target.view,
            &mut root,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    if status != ERROR_SUCCESS {
        return Err(Error::Failure(format!(
            "開啟 HKCR\\CLSID（{}）失敗：{}",
            target.arch,
            hex(status.0)
        )));
    }

    let name = wide(CLSID_JC_TSF);
    // SAFETY: name 以 NUL 結尾，root 是上面開啟的有效 handle。
    let status = unsafe { RegDeleteTreeW(root, PCWSTR(name.as_ptr())) };
    // SAFETY: root 由成功的 RegOpenKeyExW 取得。
    unsafe {
        let _ = RegCloseKey(root);
    }
    if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(Error::Failure(format!(
            r"刪除 HKCR\CLSID\{CLSID_JC_TSF}（{}）失敗：{}",
            target.arch,
            hex(status.0)
        )))
    }
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

/// 開啟 `legal\THIRD-PARTY-NOTICES.md`。
///
/// 捷徑只帶 `legal` 一個字，路徑由安裝目錄組合，所以換安裝槽或目錄名稱都不受
/// 影響。用記事本開啟：乾淨的 Windows 對 `.md` 沒有預設關聯，交給 ShellExecute
/// 會跳出「選擇應用程式」，而授權文件本來就是純文字。
pub fn open_legal() -> Result<(), Error> {
    let current = std::env::current_exe()?;
    let dir = current
        .parent()
        .ok_or_else(|| Error::Failure("無法取得安裝目錄".into()))?;
    let notices = dir.join("legal").join("THIRD-PARTY-NOTICES.md");
    if !notices.is_file() {
        return Err(Error::Failure(format!(
            "找不到 {}；安裝目錄不完整",
            notices.display()
        )));
    }

    std::process::Command::new("notepad.exe")
        .arg(&notices)
        .spawn()
        .map_err(|error| Error::Failure(format!("開啟 {} 失敗：{error}", notices.display())))?;
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

/// 確認 profile 已寫進 TSF 的 store，而且 32 位元檢視也讀得到。
///
/// `HKLM\SOFTWARE\Microsoft\CTF` 不是 WOW64 轉向機碼，兩個架構讀的是同一份
/// 資料；這裡仍分別用 `KEY_WOW64_64KEY` 與 `KEY_WOW64_32KEY` 各讀一次，把
/// 「32 位元程式看得到同一個 TIP」變成安裝時就驗得到的事。64 位元檢視讀不到
/// 就是註冊失敗；32 位元檢視讀不到只警告，因為 profile 是共用的一份，真正的
/// 載入問題會在實機上以 `reg query /reg:32` 驗。
fn ensure_tip_key() -> Result<(), Error> {
    let path = wide(&format!(r"SOFTWARE\Microsoft\CTF\TIP\{CLSID_JC_TSF}"));
    for (view, label) in [(KEY_WOW64_64KEY, "64 位元"), (KEY_WOW64_32KEY, "32 位元")] {
        let mut key = HKEY::default();
        // SAFETY: path 以 NUL 結尾；key 成功時由 RegOpenKeyExW 寫入。
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(path.as_ptr()),
                None,
                KEY_READ | view,
                &mut key,
            )
        };
        if status == ERROR_SUCCESS {
            // SAFETY: key 由成功的 RegOpenKeyExW 取得。
            unsafe {
                let _ = RegCloseKey(key);
            }
            continue;
        }
        if view == KEY_WOW64_64KEY {
            return Err(Error::Failure(format!(
                "註冊後仍找不到 HKLM\\SOFTWARE\\Microsoft\\CTF\\TIP\\{CLSID_JC_TSF}\
                 \\LanguageProfile\\{LANGID_ZH_TW:#06X}\\{PROFILE_JC_TSF}（{}）。\
                 這通常表示 AddLanguageProfile 失敗；請確認已安裝繁體中文（台灣）語言。",
                hex(status.0)
            )));
        }
        debug_warning(&format!(
            "CTF\\TIP\\{CLSID_JC_TSF} 在{label}檢視讀不到（{}）；\
             若 reg query /reg:32 也看不到，代表這台機器把 CTF 當成轉向機碼，\
             需要改用 32 位元的註冊身分再寫一次。",
            hex(status.0)
        ));
    }
    Ok(())
}

/// 送到 debugger（DebugView／VS 輸出視窗）；MSI 的 deferred 動作看不到 stderr。
fn debug_warning(text: &str) {
    let text = wide(&format!("[jc-tip-register] {text}"));
    // SAFETY: text 以 NUL 結尾且呼叫期間保持存活。
    unsafe { OutputDebugStringW(PCWSTR(text.as_ptr())) };
}

/// 呼叫 `input.dll` 的 `InstallLayoutOrTip`。
///
/// 使用者的語言清單是 (LangID, profile GUID, CLSID) 三元組；兩個架構共用同一個
/// CLSID 與 profile，所以只有一組。
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
        let text = tip_string();
        let tip = wide(&text);
        // SAFETY: tip 以 NUL 結尾且呼叫期間保持存活。
        let ok = unsafe { function(tip.as_ptr(), flags) };
        if ok == 0 {
            Err(Error::Failure(format!(
                "InstallLayoutOrTip({text}, 0x{flags:08X}) 失敗"
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
///
/// 用 `SET_ACCESS` 而不是 `GRANT_ACCESS`：修復安裝會重跑，取代既有 ACE 才不會
/// 讓 DACL 愈長愈多。
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
            grfAccessMode: SET_ACCESS,
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

    /// 兩個架構只有機碼檢視不同，CLSID 與 profile 是同一組。
    #[test]
    fn targets_differ_only_by_registry_view() {
        let x64 = Target::new(Arch::X64);
        let x86 = Target::new(Arch::X86);
        assert_eq!(x64.view, KEY_WOW64_64KEY);
        assert_eq!(x86.view, KEY_WOW64_32KEY);
        assert_ne!(x64.view, x86.view);
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
