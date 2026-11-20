//! `DllRegisterServer`／`DllUnregisterServer` 的實作。
//!
//! **這裡的 CLSID、profile GUID 與顯示名稱都由 `guids.rs` 依 `target_pointer_width`
//! 選好**：x64 建置註冊 `{7B7D6E1A-…}`，i686 建置註冊 `{3E9A5C72-…}`。同一個
//! 函式在兩種架構下做的步驟完全一樣，只有值不同。
//!
//! 機碼會落在哪個檢視由**呼叫端行程的位元數**決定，不是由 DLL 決定：
//! `regsvr32.exe` 或安裝器以 32 位元身分載入 x86 DLL 時，`HKCR\CLSID` 的寫入
//! 由登錄轉向自動落到 `Wow6432Node`，TSF 的 `CTF\TIP` 機碼則不分架構共用。
//! `installer/jc-tip-register` 是 x64 程式，載入不了 x86 DLL，跨架構時它自己
//! 用 `KEY_WOW64_32KEY` 寫同一個位置（見 `docs/port/x86-tip.md`）。
//!
//! 註冊五件事：
//!
//! 1. `HKCR\CLSID\{CLSID}\InprocServer32` 指到這個 DLL，`ThreadingModel`
//!    為 `Apartment`。
//! 2. 呼叫 `ITfInputProcessorProfiles::Register` 與 `AddLanguageProfile`
//!    （zh-TW `0x0404`、注音 profile）。
//! 3. 把 CLSID 註冊到 `GUID_TFCAT_TIP_KEYBOARD` 分類，Windows 8 之後的輸入法
//!    設定頁才會把它列出來。
//! 4. 註冊 `GUID_TFCAT_TIPCAP_UIELEMENTENABLED`，允許 TIP 使用
//!    `ITfUIElementMgr`（候選窗就是靠它交給 Windows 畫）。
//! 5. 註冊 `GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER`，Windows 才會用這個 CLSID
//!    的 `ITfDisplayAttributeProvider` 解讀組字底線的顯示屬性 atom。
//!
//! **不啟用 profile**：讓使用者在系統設定裡自己選，不偷改使用者的預設輸入法。

use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyW, RegDeleteTreeW, RegSetValueExW, HKEY, HKEY_CLASSES_ROOT, REG_SZ,
};
use windows::Win32::UI::TextServices::{
    CLSID_TF_CategoryMgr, CLSID_TF_InputProcessorProfiles, ITfCategoryMgr,
    ITfInputProcessorProfiles, GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER,
    GUID_TFCAT_TIPCAP_UIELEMENTENABLED, GUID_TFCAT_TIP_KEYBOARD,
};

use crate::guids::{registry_key, CLSID_JC_TSF, GUID_PROFILE_JC_TSF, LANGID_ZH_TW, PROFILE_NAME};

/// 註冊文字服務。
pub fn register() -> Result<()> {
    let dll = module_path()?;
    write_com_registration(&dll)?;

    // SAFETY: CoInitializeEx 對已初始化的執行緒是無害的；regsvr32 通常已經做過。
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let profiles: ITfInputProcessorProfiles =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;
        profiles.Register(&CLSID_JC_TSF)?;

        let description = crate::wide(PROFILE_NAME);
        // 去掉結尾的 NUL：cchDesc 是字元數，不是緩衝長度。
        let description = &description[..description.len() - 1];
        profiles.AddLanguageProfile(
            &CLSID_JC_TSF,
            LANGID_ZH_TW,
            &GUID_PROFILE_JC_TSF,
            description,
            &[],
            0,
        )?;

        let categories: ITfCategoryMgr =
            CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        categories.RegisterCategory(&CLSID_JC_TSF, &GUID_TFCAT_TIP_KEYBOARD, &CLSID_JC_TSF)?;
        // 沒有這個能力分類，Windows 不會接受 ITfUIElementMgr::BeginUIElement。
        categories.RegisterCategory(
            &CLSID_JC_TSF,
            &GUID_TFCAT_TIPCAP_UIELEMENTENABLED,
            &CLSID_JC_TSF,
        )?;
        // 組字底線：這個 CLSID 同時是 ITfDisplayAttributeProvider 的實作。
        categories.RegisterCategory(
            &CLSID_JC_TSF,
            &GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER,
            &CLSID_JC_TSF,
        )?;
    }

    // 刻意不呼叫 ITfInputProcessorProfileMgr::ActivateProfile。
    Ok(())
}

/// 反註冊文字服務；個別步驟失敗不阻止其他步驟。
pub fn unregister() -> Result<()> {
    // SAFETY: 與註冊相同，COM 沒有初始化時 CoCreateInstance 會失敗，忽略即可。
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if let Ok(profiles) = CoCreateInstance::<_, ITfInputProcessorProfiles>(
            &CLSID_TF_InputProcessorProfiles,
            None,
            CLSCTX_INPROC_SERVER,
        ) {
            let _ =
                profiles.RemoveLanguageProfile(&CLSID_JC_TSF, LANGID_ZH_TW, &GUID_PROFILE_JC_TSF);
            let _ = profiles.Unregister(&CLSID_JC_TSF);
        }
        if let Ok(categories) =
            CoCreateInstance::<_, ITfCategoryMgr>(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)
        {
            let _ = categories.UnregisterCategory(
                &CLSID_JC_TSF,
                &GUID_TFCAT_TIP_KEYBOARD,
                &CLSID_JC_TSF,
            );
            let _ = categories.UnregisterCategory(
                &CLSID_JC_TSF,
                &GUID_TFCAT_TIPCAP_UIELEMENTENABLED,
                &CLSID_JC_TSF,
            );
            let _ = categories.UnregisterCategory(
                &CLSID_JC_TSF,
                &GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER,
                &CLSID_JC_TSF,
            );
        }
    }

    let path = crate::wide(&format!("CLSID\\{}", registry_key(&CLSID_JC_TSF)));
    // SAFETY: path 以 NUL 結尾。
    let status = unsafe { RegDeleteTreeW(HKEY_CLASSES_ROOT, PCWSTR(path.as_ptr())) };
    if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(win32_error(status))
    }
}

/// 寫 `InprocServer32`。
fn write_com_registration(dll: &str) -> Result<()> {
    let base = format!("CLSID\\{}", registry_key(&CLSID_JC_TSF));
    let inproc = format!("{base}\\InprocServer32");
    set_string(&inproc, None, dll)?;
    set_string(&inproc, Some("ThreadingModel"), "Apartment")?;
    Ok(())
}

/// 寫一個 `REG_SZ` 值；`name` 為 `None` 時寫預設值。
fn set_string(path: &str, name: Option<&str>, value: &str) -> Result<()> {
    let path_wide = crate::wide(path);
    let mut key = HKEY::default();
    // SAFETY: 兩個字串都以 NUL 結尾，key 由 RegCloseKey 釋放。
    let status = unsafe { RegCreateKeyW(HKEY_CLASSES_ROOT, PCWSTR(path_wide.as_ptr()), &mut key) };
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

/// 取得這個 DLL 的完整路徑。
fn module_path() -> Result<String> {
    let mut buffer = vec![0u16; 512];
    loop {
        // SAFETY: buffer 的長度就是傳進去的 nSize。
        let written = unsafe { GetModuleFileNameW(None, &mut buffer) } as usize;
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

/// `WIN32_ERROR` 轉成 `windows::core::Error`。
fn win32_error(status: WIN32_ERROR) -> windows::core::Error {
    windows::core::HRESULT::from_win32(status.0).into()
}
