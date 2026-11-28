//! `DllRegisterServer`／`DllUnregisterServer` 共用的註冊實作。
//!
//! `jc-tsf` 在自己的 COM 進入點呼叫 [`register`]／[`register_with`] 與
//! [`unregister`]／[`unregister_with`]，註冊邏輯只存在這一份 rlib。
//!
//! **MSI 安裝不走這條路徑。** `installer/jc-ime.wxs` 在兩份 TSF DLL 的 `File` 各
//! 掛一個 `<Class>`，`HKCR\CLSID\{CLSID}\InprocServer32` 由 Windows Installer 的
//! `WriteRegistryValues` 寫入（x64 元件寫 64 位元檢視，`Bitness="always32"` 的
//! x86 元件寫 `Wow6432Node`），profile 與分類則由 `jc-tip-register machine-register`
//! 負責。安裝期間沒有任何程式呼叫這裡的 `register()`。
//!
//! 這個檔案留給手動修復：`regsvr32 jc-tsf.dll` 在沒有 MSI 的機器上重建同一組機碼。
//! 手動執行時它自己寫 `InprocServer32`，訊息與 MSI 版本相同。
//!
//! 為什麼 MSI 不借用這個函式：rc.5／rc.6 由 `jc-tip-register` 載入 DLL、呼叫
//! `DllRegisterServer`，結果 `RegisterCategory` 在 64 位元檢視的 CLSID 還不存在時
//! 存取違規（`0xC0000005`）。把 CLSID 交給 MSI 之後，順序由
//! `WriteRegistryValues` → `RegisterTip` 保證。
//!
//! **兩個架構共用同一個 CLSID 與 profile**（`guids.rs`）。差別只有寫入哪一個
//! 機碼檢視：這支 DLL 依 `target_pointer_width` 明確指定 `KEY_WOW64_64KEY`
//! （x64）或 `KEY_WOW64_32KEY`（i686），不依賴「行程位元數決定預設檢視」的
//! 隱含行為。同一個 CLSID 在兩個檢視各有一個 `InprocServer32`，指向該架構的
//! DLL，TSF 依宿主行程挑一個。
//!
//! `HKLM\SOFTWARE\Microsoft\CTF`（profile 與分類）不是 WOW64 轉向機碼，兩個
//! 架構讀寫的是同一份資料，所以 profile 只會有一組。
//!
//! 註冊八件事：
//!
//! 1. `HKCR\CLSID\{CLSID}\InprocServer32` 指到這個 DLL，`ThreadingModel`
//!    為 `Apartment`。
//! 2. 呼叫 `ITfInputProcessorProfiles::Register` 與 `AddLanguageProfile`
//!    （zh-TW `0x0404`、注音 profile）。
//! 3. 把 CLSID 註冊到 `GUID_TFCAT_TIP_KEYBOARD` 分類，Windows 8 之後的輸入法
//!    設定頁才會把它列出來。
//! 4. 註冊 `GUID_TFCAT_TIPCAP_UIELEMENTENABLED`，允許 TIP 使用
//!    `ITfUIElementMgr`（候選窗就是靠它交給 Windows 畫）。
//! 5. 註冊 `GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT`，Windows Store／MSIX 等
//!    immersive 宿主（Windows Terminal 的 TSF client 會帶
//!    `TF_TMF_IMMERSIVEMODE`）才會載入這個 TIP；少了它，語言清單看得到
//!    「jc 注音」但切不過去。
//! 6. 註冊 `GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT`，LangBar 才能在通知區放托盤圖示。
//! 7. 註冊 `GUID_TFCAT_TIPCAP_COMLESS`，COM-less 啟用的宿主才會載入這個 TIP，
//!    否則工作列顯示「中」但按鍵直接以英文送出。
//! 8. 註冊 `GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER`，Windows 才會用這個 CLSID
//!    的 `ITfDisplayAttributeProvider` 解讀組字底線的顯示屬性 atom。
//!
//! **不啟用 profile**：讓使用者在系統設定裡自己選，不偷改使用者的預設輸入法。

use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use windows::core::{Result, GUID, PCWSTR};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HMODULE, WIN32_ERROR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegOpenKeyExW, RegSetValueExW, HKEY,
    HKEY_CLASSES_ROOT, KEY_ALL_ACCESS, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_SZ,
};
use windows::Win32::UI::TextServices::{
    CLSID_TF_CategoryMgr, CLSID_TF_InputProcessorProfiles, ITfCategoryMgr,
    ITfInputProcessorProfiles, GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER, GUID_TFCAT_TIPCAP_COMLESS,
    GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT, GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
    GUID_TFCAT_TIPCAP_UIELEMENTENABLED, GUID_TFCAT_TIP_KEYBOARD,
};

use crate::guids::{
    registry_key, CLSID_JC_TSF, GUID_PROFILE_JC_TSF, LANGID_ZH_TW, PIPE_PREFIX, PROFILE_DESCRIPTION,
};

/// `AddLanguageProfile` 的 `pchIconFile`：指向一個真正的 NUL，長度仍是 0。
///
/// 不能傳 `&[]`：空 slice 的 `as_ptr()` 是對齊用的非 NULL 位址，`windows` 0.62
/// 會把它原樣交給 msctf，配上 `cchIconFile = 0`。msctf 若先讀指標再判斷長度就是
/// 存取違規。這裡的指標有效可讀，長度 0，等於「沒有圖示」。
static NO_ICON: [u16; 1] = [0];

/// 這份 TIP 宣告的分類。註冊與反註冊共用同一份，避免只加一邊。
///
/// 順序與日誌名稱固定；`installer/jc-tip-register` 有一份同樣的清單，
/// 兩邊的測試各自鎖住內容。
const CATEGORIES: [(&str, GUID); 6] = [
    ("GUID_TFCAT_TIP_KEYBOARD", GUID_TFCAT_TIP_KEYBOARD),
    (
        "GUID_TFCAT_TIPCAP_UIELEMENTENABLED",
        GUID_TFCAT_TIPCAP_UIELEMENTENABLED,
    ),
    (
        "GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT",
        GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT,
    ),
    (
        "GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT",
        GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
    ),
    ("GUID_TFCAT_TIPCAP_COMLESS", GUID_TFCAT_TIPCAP_COMLESS),
    (
        "GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER",
        GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER,
    ),
];

/// 一組可註冊的 TSF 身分。
///
/// [`Registration::default()`] 就是現行 DLL 的身分，值與 `guids.rs` 完全一致。
/// 這個結構刻意不放 DLL 路徑：`InprocServer32` 永遠指向呼叫 `DllRegisterServer` 的那份 DLL。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    /// 文字服務的 CLSID，同時是 `RegisterCategory` 的物件識別。
    pub clsid: GUID,
    /// 語言 profile 的 GUID。
    pub profile: GUID,
    /// 出現在系統輸入法清單的名稱。
    pub description: &'static str,
    /// profile 的語言 ID。
    pub langid: u16,
    /// 引擎 pipe 前綴；寫入 CLSID 機碼供啟動器讀取，也用來推導 pipe 名稱。
    pub pipe_prefix: &'static str,
}

impl Default for Registration {
    /// 現行身分。GUID 是 rc.15 已發行值，任何情況下都不能改，否則使用者的輸入法
    /// 設定會失效。
    fn default() -> Self {
        Self {
            clsid: CLSID_JC_TSF,
            profile: GUID_PROFILE_JC_TSF,
            description: PROFILE_DESCRIPTION,
            langid: LANGID_ZH_TW,
            pipe_prefix: PIPE_PREFIX,
        }
    }
}

impl Registration {
    /// 依 SID 尾段推出這組身分對應的引擎 pipe 名稱，不含 `\\.\pipe\`。
    #[allow(dead_code)]
    pub fn engine_pipe_name(&self, rid: &str) -> String {
        format!("{}-{rid}", self.pipe_prefix)
    }
}

/// `jc-tip-register` 傳進來的安裝日誌路徑。
const INSTALL_LOG_ENV: &str = "JC_INSTALL_LOG";

/// 這份 DLL 寫入的機碼檢視。
///
/// 明確指定，不靠「行程位元數決定預設檢視」：x64 DLL 寫 64 位元檢視，i686 DLL
/// 寫 `Wow6432Node`。同一個 CLSID 在兩個檢視各指到自己。
#[cfg(target_pointer_width = "64")]
const REGISTRY_VIEW: REG_SAM_FLAGS = windows::Win32::System::Registry::KEY_WOW64_64KEY;

/// 這份 DLL 寫入的機碼檢視；見上面 64 位元的說明。
#[cfg(target_pointer_width = "32")]
const REGISTRY_VIEW: REG_SAM_FLAGS = windows::Win32::System::Registry::KEY_WOW64_32KEY;

/// 用現行身分註冊文字服務；`DllRegisterServer` 走這條。
pub fn register() -> Result<()> {
    register_with(&Registration::default())
}

/// 用指定的身分註冊文字服務。
///
/// 一般情況用 [`register`]；需要覆寫身分時才傳入自己的 [`Registration`]。
pub fn register_with(reg: &Registration) -> Result<()> {
    install_log(&format!(
        "jc-tsf register：開始（CLSID {}，描述 {}，pipe 前綴 {}）",
        registry_key(&reg.clsid),
        reg.description,
        reg.pipe_prefix
    ));
    let dll = match module_path() {
        Ok(dll) => {
            install_log(&format!("module_path：{dll}"));
            dll
        }
        Err(error) => {
            install_log(&format!("module_path：失敗（{error}）"));
            return Err(error);
        }
    };
    if let Err(error) = write_com_registration(&dll, reg) {
        install_log(&format!("寫入 InprocServer32：失敗（{error}）"));
        return Err(error);
    }
    install_log("寫入 InprocServer32：成功");

    // SAFETY: CoInitializeEx 對已初始化的執行緒是無害的；regsvr32 通常已經做過。
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        install_log(&format!("CoInitializeEx = 0x{:08X}", hr.0 as u32));

        let profiles: ITfInputProcessorProfiles =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)
                .map_err(|error| {
                    install_log(&format!(
                        "CoCreateInstance(CLSID_TF_InputProcessorProfiles)：失敗（{error}）"
                    ));
                    error
                })?;
        install_log("CoCreateInstance(CLSID_TF_InputProcessorProfiles)：成功");

        let result = profiles.Register(&reg.clsid);
        log_result("ITfInputProcessorProfiles::Register", &result);
        result?;

        let description = crate::wide(reg.description);
        // 去掉結尾的 NUL：cchDesc 是字元數，不是緩衝長度。
        let description = &description[..description.len() - 1];
        // `&[]` 的 as_ptr() 是非 NULL 的對齊位址，msctf 可能先讀再判斷長度；
        // 指向真正的 NUL、長度 0 才是「沒有圖示」。
        let result = profiles.AddLanguageProfile(
            &reg.clsid,
            reg.langid,
            &reg.profile,
            description,
            &NO_ICON[..0],
            0,
        );
        log_result("ITfInputProcessorProfiles::AddLanguageProfile", &result);
        result?;

        install_log("CoCreateInstance(CLSID_TF_CategoryMgr)：開始");
        let categories: ITfCategoryMgr =
            CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER).map_err(
                |error| {
                    install_log(&format!(
                        "CoCreateInstance(CLSID_TF_CategoryMgr)：失敗（HRESULT 0x{:08X}：{error}）",
                        error.code().0 as u32
                    ));
                    error
                },
            )?;
        install_log("CoCreateInstance(CLSID_TF_CategoryMgr)：成功（HRESULT 0x00000000）");

        for (label, category) in CATEGORIES {
            install_log(&format!("RegisterCategory({label})：開始"));
            let result = categories.RegisterCategory(&reg.clsid, &category, &reg.clsid);
            log_result(&format!("RegisterCategory({label})"), &result);
            result?;
        }
    }

    // 刻意不呼叫 ITfInputProcessorProfileMgr::ActivateProfile。
    install_log("jc-tsf register：成功");
    Ok(())
}

/// 用現行身分反註冊；`DllUnregisterServer` 走這條。
pub fn unregister() -> Result<()> {
    unregister_with(&Registration::default())
}

/// 用指定的身分反註冊；個別步驟失敗不阻止其他步驟。
pub fn unregister_with(reg: &Registration) -> Result<()> {
    install_log(&format!(
        "jc-tsf unregister：開始（CLSID {}）",
        registry_key(&reg.clsid)
    ));
    // SAFETY: 與註冊相同，COM 沒有初始化時 CoCreateInstance 會失敗，忽略即可。
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if let Ok(profiles) = CoCreateInstance::<_, ITfInputProcessorProfiles>(
            &CLSID_TF_InputProcessorProfiles,
            None,
            CLSCTX_INPROC_SERVER,
        ) {
            let _ = profiles.RemoveLanguageProfile(&reg.clsid, reg.langid, &reg.profile);
            let _ = profiles.Unregister(&reg.clsid);
        }
        if let Ok(categories) =
            CoCreateInstance::<_, ITfCategoryMgr>(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)
        {
            for (_, category) in CATEGORIES {
                let _ = categories.UnregisterCategory(&reg.clsid, &category, &reg.clsid);
            }
        }
    }

    // 只刪這份 DLL 自己那個檢視；另一個檢視由另一個架構的 DLL（或安裝器）負責。
    let result = delete_clsid_tree(reg);
    match &result {
        Ok(()) => install_log("jc-tsf unregister：成功"),
        Err(error) => install_log(&format!("delete_clsid_tree：失敗（{error}）")),
    }
    result
}

/// 刪掉這個檢視的 `HKCR\CLSID\{...}` 整棵樹。
fn delete_clsid_tree(reg: &Registration) -> Result<()> {
    let root_path = crate::wide("CLSID");
    let mut root = HKEY::default();
    // SAFETY: root_path 以 NUL 結尾；root 成功時由 RegOpenKeyExW 寫入。
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CLASSES_ROOT,
            PCWSTR(root_path.as_ptr()),
            None,
            KEY_ALL_ACCESS | REGISTRY_VIEW,
            &mut root,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    if status != ERROR_SUCCESS {
        return Err(win32_error(status));
    }

    let path = crate::wide(&registry_key(&reg.clsid));
    // SAFETY: path 以 NUL 結尾；root 是上面開啟的 handle，檢視由它決定。
    let status = unsafe { RegDeleteTreeW(root, PCWSTR(path.as_ptr())) };
    // SAFETY: root 由成功的 RegOpenKeyExW 取得。
    unsafe {
        let _ = RegCloseKey(root);
    }
    if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(win32_error(status))
    }
}

/// 寫 `InprocServer32` 與這組身分的 pipe 前綴。
fn write_com_registration(dll: &str, reg: &Registration) -> Result<()> {
    let base = format!("CLSID\\{}", registry_key(&reg.clsid));
    let inproc = format!("{base}\\InprocServer32");
    set_string(&inproc, None, dll)?;
    set_string(&inproc, Some("ThreadingModel"), "Apartment")?;
    // T9：讓讀取這個 CLSID 的啟動器知道該連哪一條引擎 pipe。Windows 忽略不認識的值。
    set_string(&base, Some("PipePrefix"), reg.pipe_prefix)?;
    Ok(())
}

/// 寫一個 `REG_SZ` 值；`name` 為 `None` 時寫預設值。
///
/// 用 `RegCreateKeyExW` 才能指定 `REGISTRY_VIEW`；`RegCreateKeyW` 只有預設檢視。
fn set_string(path: &str, name: Option<&str>, value: &str) -> Result<()> {
    let path_wide = crate::wide(path);
    let mut key = HKEY::default();
    // SAFETY: 兩個字串都以 NUL 結尾，key 由 RegCloseKey 釋放。
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CLASSES_ROOT,
            PCWSTR(path_wide.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE | REGISTRY_VIEW,
            None,
            &mut key,
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(win32_error(status));
    }

    let name_wide = name.map(crate::wide);
    let value_wide = crate::wide(value);
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
        Err(win32_error(status))
    }
}

/// 取得這份 DLL 的完整路徑。
///
/// 不能用 `GetModuleFileNameW(None, ...)`：`hModule = NULL` 拿到的是**行程的
/// exe**。`DllRegisterServer` 由 `regsvr32.exe`（或其他載入器）呼叫時，
/// 寫進 `InprocServer32` 的就會是那支 exe，TSF 之後載不到輸入法。這裡用本模組內
/// 的函式位址當錨點，取得真正裝著這段程式碼的 DLL。
pub fn module_path() -> Result<String> {
    let mut module = HMODULE::default();
    // SAFETY: anchor 是本模組內的函式位址；UNCHANGED_REFCOUNT 不增加參考計數，
    // 所以不需要（也不能）對 module 呼叫 FreeLibrary。
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(module_path as *const () as *const u16),
            &mut module,
        )?;
    }

    let mut buffer = vec![0u16; 512];
    loop {
        // SAFETY: buffer 的長度就是傳進去的 nSize；module 是上面取得的 DLL。
        let written = unsafe { GetModuleFileNameW(Some(module), &mut buffer) } as usize;
        if written == 0 {
            return Err(windows::core::Error::from_thread());
        }
        if written < buffer.len() {
            return Ok(String::from_utf16_lossy(&buffer[..written]));
        }
        // 被截斷就放大重試；32768 是 Win32 路徑的實務上限。
        if buffer.len() >= 32768 {
            return Err(windows::core::Error::from_thread());
        }
        buffer.resize(buffer.len() * 2, 0);
    }
}

/// 寫一行安裝日誌；沒有 `JC_INSTALL_LOG` 時什麼都不做。
///
/// 每次呼叫都獨立開檔、寫入、`flush`、`sync_all`：崩潰後最後一行仍在磁碟上。
fn install_log(message: &str) {
    let Some(path) = std::env::var_os(INSTALL_LOG_ENV).filter(|value| !value.is_empty()) else {
        return;
    };
    let line = format!(
        "[{}] [{}] {}\n",
        timestamp_millis(),
        std::process::id(),
        message.replace('\n', " ")
    );
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let _ = file.write_all(line.as_bytes());
    let _ = file.flush();
    let _ = file.sync_all();
}

/// 把 TSF 呼叫的結果與 HRESULT 寫成一行。
fn log_result<T>(label: &str, result: &Result<T>) {
    match result {
        Ok(_) => install_log(&format!("{label}：成功（HRESULT 0x00000000）")),
        Err(error) => install_log(&format!(
            "{label}：失敗（HRESULT 0x{:08X}：{error}）",
            error.code().0 as u32
        )),
    }
}

/// 目前時間的 Unix epoch 毫秒；系統時鐘早於 epoch 時回 0。
fn timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

/// `WIN32_ERROR` 轉成 `windows::core::Error`。
fn win32_error(status: WIN32_ERROR) -> windows::core::Error {
    windows::core::HRESULT::from_win32(status.0).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 現行 DLL 的預設值必須與 guids.rs 的已發行值一致。
    #[test]
    fn default_registration_keeps_published_values() {
        let reg = Registration::default();
        assert_eq!(reg.clsid, CLSID_JC_TSF);
        assert_eq!(reg.profile, GUID_PROFILE_JC_TSF);
        assert_eq!(
            registry_key(&reg.clsid),
            "{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}"
        );
        assert_eq!(
            registry_key(&reg.profile),
            "{C4E8D217-5B3F-4A96-8E0C-7F1A2B3D4E56}"
        );
        assert_eq!(reg.description, "jc 注音");
        assert_eq!(reg.langid, 0x0404);
        assert_eq!(reg.pipe_prefix, "jc-ime");
        assert_eq!(reg.engine_pipe_name("1001"), "jc-ime-1001");
    }

    /// Windows Terminal 等 immersive 宿主只載入宣告 IMMERSIVESUPPORT 的 TIP。
    /// 少了這項，語言清單有「jc 注音」但切不過去。
    #[test]
    fn categories_include_immersive_support() {
        assert!(
            CATEGORIES
                .iter()
                .any(|(_, category)| *category == GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT),
            "CATEGORIES 必須包含 GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT"
        );
    }

    /// 六個分類都要有，而且互不相同；註冊與反註冊共用同一份清單。
    #[test]
    fn categories_are_six_distinct_guids() {
        assert_eq!(CATEGORIES.len(), 6);
        for (index, (label, category)) in CATEGORIES.iter().enumerate() {
            assert!(!label.is_empty());
            assert!(
                !CATEGORIES[..index]
                    .iter()
                    .any(|(_, earlier)| earlier == category),
                "{label} 重複"
            );
        }
    }
}
