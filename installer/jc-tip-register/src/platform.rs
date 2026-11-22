//! Windows 專屬實作。
//!
//! 所有 `unsafe` 都集中在這裡；呼叫端（`main.rs`）只看到 [`Error`] 與各個動作。
//!
//! 註冊分成兩層，各自負責不同機碼：
//!
//! | 機碼 | 誰寫 | 何時 |
//! |---|---|---|
//! | `HKCR\CLSID\{CLSID}\InprocServer32`（64 位元與 `Wow6432Node`） | MSI（WXS 的 `<Class>`） | `WriteRegistryValues` |
//! | `HKLM\SOFTWARE\Microsoft\CTF\TIP\{CLSID}` 與分類 | 這支程式 | `WriteRegistryValues` 之後 |
//!
//! CLSID 交給 MSI 是 rc.7 的修正：rc.5／rc.6 由 `DllRegisterServer` 自己寫，當時
//! 64 位元檢視的 CLSID 在 `RegisterCategory` 執行時還不存在，msctf 直接存取違規
//! （`0xC0000005`）。`<Class Advertise="no">` 產生 Registry table 的資料列，
//! `WriteRegistryValues` 排在 `RegisterTip` 之前，這支程式因此完全不需要 DLL
//! 路徑與架構判斷。
//!
//! `HKLM\SOFTWARE\Microsoft\CTF` 不是 WOW64 轉向機碼，兩個架構讀寫同一份資料，
//! 所以 profile 與分類只註冊一次、也只移除一次。

use std::ffi::c_void;
use std::path::Path;

use thiserror::Error;
use windows::core::{Interface, GUID, PCSTR, PCWSTR, PWSTR};
use windows::Win32::Foundation::{FreeLibrary, LocalFree, ERROR_SUCCESS, HLOCAL};
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
    RegCloseKey, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY,
    KEY_WOW64_64KEY,
};
use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
use windows::Win32::UI::TextServices::{
    CLSID_TF_CategoryMgr, CLSID_TF_InputProcessorProfiles, ITfCategoryMgr,
    ITfInputProcessorProfileMgr, ITfInputProcessorProfiles, GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER,
    GUID_TFCAT_TIPCAP_COMLESS, GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
    GUID_TFCAT_TIPCAP_UIELEMENTENABLED, GUID_TFCAT_TIP_KEYBOARD,
};

use crate::guids::{
    tip_string, CLSID_JC_TSF, CLSID_JC_TSF_LEGACY, ILOT_INSTALL, LANGID_ZH_TW,
    PROFILE_DESCRIPTION, PROFILE_JC_TSF, PROFILE_JC_TSF_LEGACY,
};
use crate::log;

/// `RegisterProfile` 的 `pchiconfile`：指向一個真正的 NUL，長度仍是 0。
///
/// `windows` 0.62 的 `RegisterProfile` 用 slice 長度當 `cchIconFile`，傳 `&[]` 會
/// 拿到對齊用的非 NULL 位址；msctf 若先讀指標再判斷長度就是存取違規。這裡的指標
/// 有效可讀、長度 0，等於「沒有圖示」。
static NO_ICON: [u16; 1] = [0];

/// 註冊的五個分類；順序與日誌名稱一起固定在這裡。
const CATEGORIES: [(&str, GUID); 5] = [
    // 沒有這個分類，Windows 8 之後的輸入法設定頁不會列出「jc 注音」。
    ("GUID_TFCAT_TIP_KEYBOARD", GUID_TFCAT_TIP_KEYBOARD),
    // 沒有這個能力分類，Windows 不會接受 ITfUIElementMgr::BeginUIElement。
    (
        "GUID_TFCAT_TIPCAP_UIELEMENTENABLED",
        GUID_TFCAT_TIPCAP_UIELEMENTENABLED,
    ),
    // LangBar 托盤圖示：少了這個分類，工作列不會出現語言指示器。
    (
        "GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT",
        GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
    ),
    // COM-less 宿主（GLFW／LWJGL 等）只載入宣告這個分類的輸入法；少了它，
    // 宿主顯示「中」但按鍵直接以英文送出。
    ("GUID_TFCAT_TIPCAP_COMLESS", GUID_TFCAT_TIPCAP_COMLESS),
    // 組字底線：這個 CLSID 同時是 ITfDisplayAttributeProvider 的實作。
    (
        "GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER",
        GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER,
    ),
];

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

/// 現行 CLSID 的 GUID；與 `guids.rs` 的字串版本相同。
const CLSID_GUID: GUID = GUID::from_u128(0x2f6b41d8_9e07_4c3a_b5f1_8d2c6a9e0473);

/// 現行 profile 的 GUID；與 `guids.rs` 的字串版本相同。
const PROFILE_GUID: GUID = GUID::from_u128(0xc4e8d217_5b3f_4a96_8e0c_7f1a2b3d4e56);

/// 已移除舊版的 CLSID GUID；只給語言清單清理對照。
const CLSID_GUID_LEGACY: GUID = GUID::from_u128(0x7b7d6e1a_3c1f_4b2e_9a55_6c1d2f3e4a01);

/// 已移除舊版的 profile GUID；只給語言清單清理對照。
const PROFILE_GUID_LEGACY: GUID = GUID::from_u128(0x8f0a9b3c_2d4e_4f50_a1b2_c3d4e5f60718);

/// 一組註冊身分。註冊與清理共用同一套流程，只有 CLSID、profile 與描述不同。
struct Identity {
    /// CLSID 的字串形式，用在 CTF 機碼路徑與日誌。
    clsid: &'static str,
    /// CLSID 的 GUID，用在 COM 呼叫。
    clsid_guid: GUID,
    /// profile 的字串形式。
    profile: &'static str,
    /// profile 的 GUID。
    profile_guid: GUID,
    /// 顯示名稱。
    description: &'static str,
}

/// 現行身分；`machine-register`／`machine-unregister`／`user-install` 用。
const IDENTITY_CURRENT: Identity = Identity {
    clsid: CLSID_JC_TSF,
    clsid_guid: CLSID_GUID,
    profile: PROFILE_JC_TSF,
    profile_guid: PROFILE_GUID,
    description: PROFILE_DESCRIPTION,
};

/// 已移除的舊身分；只給 `user-cleanup-legacy` 清語言清單用，不再註冊。
const IDENTITY_LEGACY: Identity = Identity {
    clsid: CLSID_JC_TSF_LEGACY,
    clsid_guid: CLSID_GUID_LEGACY,
    profile: PROFILE_JC_TSF_LEGACY,
    profile_guid: PROFILE_GUID_LEGACY,
    description: PROFILE_DESCRIPTION,
};

/// 註冊文字服務的 profile 與五個分類。
///
/// CLSID 的 `InprocServer32` 由 MSI 寫好才呼叫這裡。`bEnabledByDefault` 傳
/// `true`：profile 預設啟用（`LanguageProfile` 機碼的 `Enable=1`），使用者在語言
/// 設定加入鍵盤後就能切換；仍然不呼叫 `ActivateProfile`，不改系統預設輸入法。
pub fn machine_register() -> Result<(), Error> {
    machine_register_impl(&IDENTITY_CURRENT)
}

/// 依指定身分註冊 profile 與五個分類。
fn machine_register_impl(identity: &Identity) -> Result<(), Error> {
    log::step("machine-register：開始");
    log::step(&format!(
        "CLSID {}：InprocServer32 由 MSI 寫入（WXS 的 <Class>）；這裡只註冊 profile 與分類",
        identity.clsid
    ));

    // SAFETY: CoInitializeEx 對已初始化的執行緒是無害的。
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        log::step(&format!("CoInitializeEx = 0x{:08X}", hr.0 as u32));

        let profiles: ITfInputProcessorProfileMgr = create(
            "CLSID_TF_InputProcessorProfiles",
            &CLSID_TF_InputProcessorProfiles,
        )?;

        // 去掉結尾的 NUL：windows 0.62 用 slice 長度當 cchDesc。
        let description = wide(identity.description);
        let description = &description[..description.len() - 1];
        log::step(&format!("RegisterProfile({})：開始", identity.description));
        let result = profiles.RegisterProfile(
            &identity.clsid_guid,
            LANGID_ZH_TW,
            &identity.profile_guid,
            description,
            &NO_ICON[..0],
            0,
            HKL::default(),
            0,
            // bEnabledByDefault：寫入 LanguageProfile 的 Enable=1，加入鍵盤即可切換。
            true,
            0,
        );
        log_hr(
            &format!("RegisterProfile({})", identity.description),
            &result,
        );
        result?;

        let categories: ITfCategoryMgr = create("CLSID_TF_CategoryMgr", &CLSID_TF_CategoryMgr)?;
        for (label, category) in CATEGORIES {
            log::step(&format!("RegisterCategory({label})：開始"));
            let result =
                categories.RegisterCategory(&identity.clsid_guid, &category, &identity.clsid_guid);
            log_hr(&format!("RegisterCategory({label})"), &result);
            result?;
        }
    }

    log::step("ensure_tip_key：開始");
    if let Err(error) = ensure_tip_key(identity) {
        log::step(&format!("ensure_tip_key：失敗（{error}）"));
        return Err(error);
    }
    log::step("ensure_tip_key：成功");

    // AppContainer（UWP）要讀得到安裝目錄才能載入 TIP。目錄取自這支 exe 的位置，
    // 不需要 MSI 再傳路徑進來。ACL 是冪等的（`SET_ACCESS`），重跑不會堆疊重複的
    // ACE。這是 UWP 才需要的權限，桌面程式不受影響；預設關閉，避免手工 ACL 操作
    // 落在安裝關鍵路徑上（見 docs/port/tsf-installer.md 的 RC5 bisect）。
    if app_container_enabled() {
        match install_dir() {
            Some(dir) => {
                log::step(&format!("grant_app_container：{}", dir.display()));
                if let Err(error) = grant_app_container(&dir) {
                    log::step(&format!("grant_app_container：失敗（{error}）"));
                    eprintln!(
                        "jc-tip-register：設定 AppContainer 讀取權限失敗（桌面程式不受影響）：{error}"
                    );
                } else {
                    log::step("grant_app_container：成功");
                }
            }
            None => log::step("grant_app_container：取不到安裝目錄，跳過"),
        }
    } else {
        log::step("grant_app_container：已跳過（未設定 JC_IME_APP_CONTAINER）");
    }
    log::step("machine-register：成功");
    Ok(())
}

/// 只有明確設定 `JC_IME_APP_CONTAINER=1` 才執行 AppContainer ACL。
fn app_container_enabled() -> bool {
    std::env::var_os("JC_IME_APP_CONTAINER")
        .as_deref()
        .and_then(std::ffi::OsStr::to_str)
        == Some("1")
}

/// 這支 exe 所在的安裝目錄；`jc-tsf.dll` 與它在同一層。
fn install_dir() -> Option<std::path::PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
}

/// `CoCreateInstance` 並在安裝日誌留下前後各一行。
///
/// 沒有這段日誌，想知道是哪一個物件建立失敗只能重跑；deferred 動作在 SYSTEM
/// 身分執行，stderr 收不到。
fn create<T: Interface>(label: &str, clsid: &GUID) -> Result<T, Error> {
    log::step(&format!("CoCreateInstance({label})：開始"));
    // SAFETY: clsid 指向有效的 GUID，None 表示不聚合外層物件。
    match unsafe { CoCreateInstance(clsid, None, CLSCTX_INPROC_SERVER) } {
        Ok(object) => {
            log::step(&format!(
                "CoCreateInstance({label})：成功（HRESULT 0x00000000）"
            ));
            Ok(object)
        }
        Err(error) => {
            log::step(&format!(
                "CoCreateInstance({label})：失敗（HRESULT 0x{:08X}：{error}）",
                error.code().0 as u32
            ));
            Err(error.into())
        }
    }
}

/// 把 TSF 呼叫的 HRESULT 寫進安裝日誌。
fn log_hr<T>(label: &str, result: &windows::core::Result<T>) {
    match result {
        Ok(_) => log::step(&format!("{label}：成功（HRESULT 0x00000000）")),
        Err(error) => log::step(&format!(
            "{label}：失敗（HRESULT 0x{:08X}：{error}）",
            error.code().0 as u32
        )),
    }
}

/// 移除 TSF 的 profile 與五個分類。
///
/// 不刪 `HKCR\CLSID`：MSI 的 `RemoveRegistryValues` 會把兩個檢視的 CLSID 機碼
/// 一起移除，這裡若先刪就會跟 MSI 搶，也會讓 `RemoveRegistryValues` 找不到目標。
///
/// 個別步驟失敗只記日誌、不中斷：反安裝不能被殘缺的 CTF 狀態卡住，MSI 這一端
/// 也是 `Return="ignore"`。
pub fn machine_unregister() -> Result<(), Error> {
    machine_unregister_impl(&IDENTITY_CURRENT)
}

/// 依指定身分移除 profile 與五個分類。
fn machine_unregister_impl(identity: &Identity) -> Result<(), Error> {
    log::step("machine-unregister：開始");
    log::step(&format!(
        "CLSID {} 的機碼由 MSI 的 RemoveRegistryValues 移除；這裡只清 profile 與分類",
        identity.clsid
    ));

    // SAFETY: 與註冊相同；COM 沒有初始化時 CoCreateInstance 會失敗，記錄後繼續。
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        log::step(&format!("CoInitializeEx = 0x{:08X}", hr.0 as u32));

        match create::<ITfCategoryMgr>("CLSID_TF_CategoryMgr", &CLSID_TF_CategoryMgr) {
            Ok(categories) => {
                for (label, category) in CATEGORIES {
                    log::step(&format!("UnregisterCategory({label})：開始"));
                    let result = categories.UnregisterCategory(
                        &identity.clsid_guid,
                        &category,
                        &identity.clsid_guid,
                    );
                    log_hr(&format!("UnregisterCategory({label})"), &result);
                }
            }
            Err(error) => log::step(&format!("略過分類移除：{error}")),
        }

        match create::<ITfInputProcessorProfileMgr>(
            "CLSID_TF_InputProcessorProfiles",
            &CLSID_TF_InputProcessorProfiles,
        ) {
            Ok(profiles) => {
                log::step(&format!(
                    "UnregisterProfile({})：開始",
                    identity.description
                ));
                let result = profiles.UnregisterProfile(
                    &identity.clsid_guid,
                    LANGID_ZH_TW,
                    &identity.profile_guid,
                    0,
                );
                log_hr(
                    &format!("UnregisterProfile({})", identity.description),
                    &result,
                );
            }
            Err(error) => log::step(&format!("略過 profile 移除：{error}")),
        }
    }

    log::step("machine-unregister：成功");
    Ok(())
}

/// 把 profile 加進目前使用者的語言清單。
///
/// 不改預設輸入法：`InstallLayoutOrTip` 的旗標是 0，不帶 `ILOT_DEFPROFILE`。
pub fn user_install() -> Result<(), Error> {
    if let Some(dir) = user_dir() {
        std::fs::create_dir_all(&dir)?;
    }
    layout_or_tip(&tip_string(), ILOT_INSTALL)
}

/// 清除已移除舊版與 rc.15 舊描述殘留的語言清單項目。
///
/// 呼叫 `ITfInputProcessorProfiles::RemoveLanguageProfile` 移除目前使用者 HKCU 的
/// 舊項目，不使用 `ILOT_UNINSTALL`（移除語系最後一個 layout 時會把系統地區改成
/// 英文）。升級時 Windows 不會自動清掉這些孤兒項目，所以要排在新的 `user-install`
/// 之前：先清掉舊描述，再由 `user-install` 加回單一「jc 注音」。
///
/// 個別步驟失敗只記日誌、不回錯誤：這一步是清理，不能擋住安裝（WXS 端也是
/// `Return="ignore"`）。
pub fn user_cleanup_legacy() -> Result<(), Error> {
    log::step("user-cleanup-legacy：開始");
    // SAFETY: CoInitializeEx 對已初始化的執行緒是無害的。
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        log::step(&format!("CoInitializeEx = 0x{:08X}", hr.0 as u32));

        match create::<ITfInputProcessorProfiles>(
            "CLSID_TF_InputProcessorProfiles",
            &CLSID_TF_InputProcessorProfiles,
        ) {
            Ok(profiles) => {
                for (label, identity) in [
                    ("已移除的舊版 jc 注音", &IDENTITY_LEGACY),
                    ("rc.15 的 jc 注音（新版）", &IDENTITY_CURRENT),
                ] {
                    log::step(&format!(
                        "RemoveLanguageProfile({label}, {})：開始",
                        identity.clsid
                    ));
                    let result = profiles.RemoveLanguageProfile(
                        &identity.clsid_guid,
                        LANGID_ZH_TW,
                        &identity.profile_guid,
                    );
                    log_hr(&format!("RemoveLanguageProfile({label})"), &result);
                }
            }
            Err(error) => log::step(&format!("略過語言清單清理：{error}")),
        }
    }
    log::step("user-cleanup-legacy：成功");
    Ok(())
}

/// 刪除使用者資料；**不**移除語言清單裡的「jc 注音」。
///
/// 移除語系的最後一個 layout 時，Windows 會把系統地區改成英文或其他可用語言，
/// 使用者的整個介面因此變調。參考專案在 `disable()` 明確停用
/// `InstallLayoutOrTip(ILOT_UNINSTALL)`，理由與追蹤見
/// <https://codeberg.org/chewing/windows-chewing-tsf/issues/553>。這裡照做，只留
/// 一行日誌說明：MSI 移除 profile 之後，殘留的語言清單項目會自然變成無效項目，
/// 由使用者自己在語言設定移除。
///
/// `keep_user_data` 為 `false` 時一併刪除 `%LOCALAPPDATA%\jc-ime`
/// （settings.json、personal.json、tsf.log）。
pub fn user_uninstall(keep_user_data: bool) -> Result<(), Error> {
    log::step("user-uninstall：不呼叫 InstallLayoutOrTip(ILOT_UNINSTALL)");
    log::step(
        "user-uninstall：移除語系的最後一個 layout 會讓 Windows 把系統地區改成英文；\
         語言清單項目留給使用者自己移除（windows-chewing-tsf issue 553）",
    );
    if !keep_user_data {
        if let Some(dir) = user_dir() {
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => log::step(&format!("已刪除 {}", dir.display())),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    log::step(&format!("{} 不存在，略過", dir.display()));
                }
                Err(error) => eprintln!("jc-tip-register：移除 {} 失敗：{error}", dir.display()),
            }
        }
    }
    Ok(())
}

/// 啟動 `jc-settings-ui.exe`（相容入口）。
///
/// 早期的開始功能表捷徑只帶 `settings` 一個字，`%LOCALAPPDATA%` 由這裡展開；
/// 現在 Shortcut 直連 `jc-settings-ui.exe`，路徑展開也搬到那支程式。保留這個
/// 子命令給既有呼叫方式，行為不變。
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

/// 確認 profile 已寫進 TSF 的 store，而且 32 位元檢視也讀得到。
///
/// `HKLM\SOFTWARE\Microsoft\CTF` 不是 WOW64 轉向機碼，兩個架構讀的是同一份
/// 資料；這裡仍分別用 `KEY_WOW64_64KEY` 與 `KEY_WOW64_32KEY` 各讀一次，把
/// 「32 位元程式看得到同一個 TIP」變成安裝時就驗得到的事。64 位元檢視讀不到
/// 就是註冊失敗；32 位元檢視讀不到只警告，因為 profile 是共用的一份，真正的
/// 載入問題會在實機上以 `reg query /reg:32` 驗。
fn ensure_tip_key(identity: &Identity) -> Result<(), Error> {
    let path = wide(&format!(r"SOFTWARE\Microsoft\CTF\TIP\{}", identity.clsid));
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
            log::step(&format!(
                "CTF\\TIP\\{}（{label}檢視）：存在",
                identity.clsid
            ));
            // SAFETY: key 由成功的 RegOpenKeyExW 取得。
            unsafe {
                let _ = RegCloseKey(key);
            }
            continue;
        }
        log::step(&format!(
            "CTF\\TIP\\{}（{label}檢視）：讀不到（{}）",
            identity.clsid,
            hex(status.0)
        ));
        if view == KEY_WOW64_64KEY {
            return Err(Error::Failure(format!(
                "註冊後仍找不到 HKLM\\SOFTWARE\\Microsoft\\CTF\\TIP\\{}\
                 \\LanguageProfile\\{LANGID_ZH_TW:#06X}\\{}（{}）。\
                 這通常表示 RegisterProfile 失敗；請確認已安裝繁體中文（台灣）語言。",
                identity.clsid,
                identity.profile,
                hex(status.0)
            )));
        }
        debug_warning(&format!(
            "CTF\\TIP\\{} 在{label}檢視讀不到（{}）；\
             若 reg query /reg:32 也看不到，代表這台機器把 CTF 當成轉向機碼。",
            identity.clsid,
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
/// CLSID 與 profile，所以只有一組。只用在安裝，移除刻意不做，理由見
/// [`user_uninstall`]。`text` 由呼叫端從 `guids.rs` 的 `tip_string()` 組好，
/// 帶現行身分。
fn layout_or_tip(text: &str, flags: u32) -> Result<(), Error> {
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
        let tip = wide(text);
        // SAFETY: tip 以 NUL 結尾且在呼叫期間保持存活。
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

    #[test]
    fn legacy_guid_constants_match_strings() {
        assert_eq!(guid_text(&CLSID_GUID_LEGACY), CLSID_JC_TSF_LEGACY);
        assert_eq!(guid_text(&PROFILE_GUID_LEGACY), PROFILE_JC_TSF_LEGACY);
        assert_ne!(CLSID_GUID, CLSID_GUID_LEGACY);
        assert_ne!(PROFILE_GUID, PROFILE_GUID_LEGACY);
    }

    #[test]
    fn identities_carry_their_own_values() {
        assert_eq!(IDENTITY_CURRENT.clsid, CLSID_JC_TSF);
        assert_eq!(IDENTITY_CURRENT.profile, PROFILE_JC_TSF);
        assert_eq!(IDENTITY_CURRENT.description, PROFILE_DESCRIPTION);
        assert_eq!(IDENTITY_LEGACY.clsid, CLSID_JC_TSF_LEGACY);
        assert_eq!(IDENTITY_LEGACY.profile, PROFILE_JC_TSF_LEGACY);
    }

    /// 五個分類都要有，而且互不相同；少一個就少一項 Windows 功能。
    #[test]
    fn categories_are_five_distinct_guids() {
        assert_eq!(CATEGORIES.len(), 5);
        for (index, (label, category)) in CATEGORIES.iter().enumerate() {
            assert!(!label.is_empty());
            assert_ne!(*category, GUID::zeroed());
            assert!(
                !CATEGORIES[..index]
                    .iter()
                    .any(|(_, earlier)| earlier == category),
                "{label} 重複"
            );
        }
        assert_eq!(
            CATEGORIES.map(|(_, category)| category),
            [
                GUID_TFCAT_TIP_KEYBOARD,
                GUID_TFCAT_TIPCAP_UIELEMENTENABLED,
                GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
                GUID_TFCAT_TIPCAP_COMLESS,
                GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER,
            ]
        );
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
