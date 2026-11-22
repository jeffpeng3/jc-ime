//! 註冊表探測（檢查 7、9、11，以及檢查 8 的安裝目錄）。
//!
//! 產品的 TSF DLL 有 x64 與 x86 兩份，**共用同一個 CLSID**：64 位元機碼檢視的
//! `InprocServer32` 指到 `jc-tsf.dll`，32 位元檢視（`Wow6432Node`）指到
//! `jc-tsf-x86.dll`。所以「裝好了」的定義是**兩個檢視都看得到同一個 CLSID**，
//! 只看到一邊代表有一個架構的程式打不出字。
//!
//! 這裡只讀機碼、不需要管理員。沒安裝不算失敗（`[SKIP]`）；只看得到一邊或指到
//! 錯的 DLL 算失敗（`[FAIL]`），訊息會指出是哪一個檢視。

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_ITEMS, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CLASSES_ROOT,
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_DWORD,
    REG_SAM_FLAGS, REG_VALUE_TYPE,
};

use crate::error::SmokeError;
use crate::profile::{self, LANGUAGE_ZH_HANT_TW, USER_PROFILE_KEY};
use crate::report;

/// TSF 的 TIP 清單機碼。
pub const TIP_KEY: &str = r"SOFTWARE\Microsoft\CTF\TIP";

/// 64 位元檢視應該指到的 DLL 檔名。
pub const DLL_X64: &str = "jc-tsf.dll";

/// 32 位元檢視應該指到的 DLL 檔名。
pub const DLL_X86: &str = "jc-tsf-x86.dll";

/// 一個機碼檢視的探測結果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewProbe {
    /// `CTF\TIP` 底下總共有幾個子機碼。
    pub tip_count: usize,
    /// `CTF\TIP\{CLSID}` 是否存在。
    pub profile: bool,
    /// `HKCR\CLSID\{CLSID}\InprocServer32` 的預設值；沒註冊就是 `None`。
    pub inproc: Option<String>,
}

impl ViewProbe {
    /// 這個檢視裡有沒有任何本產品的註冊痕跡。
    fn is_registered(&self) -> bool {
        self.profile || self.inproc.is_some()
    }
}

/// 兩個檢視的探測結果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Probe {
    /// 64 位元檢視。
    pub x64: ViewProbe,
    /// 32 位元檢視（`Wow6432Node`）。
    pub x86: ViewProbe,
}

/// 探測結果的判定。
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// 兩個檢視都註冊了同一個 CLSID，而且各指到對應架構的 DLL。
    Installed {
        /// 64 位元檢視的 `InprocServer32`。
        x64_dll: String,
        /// 32 位元檢視的 `InprocServer32`。
        x86_dll: String,
    },
    /// 兩個檢視都沒有註冊：還沒安裝，不是失敗。
    NotInstalled,
    /// 只有一邊註冊、或指到錯的 DLL；訊息說明哪裡不對。
    Broken(String),
}

impl Probe {
    /// 判定兩個檢視是否都正確註冊。
    pub fn verdict(&self, clsid_text: &str) -> Verdict {
        if !self.x64.is_registered() && !self.x86.is_registered() {
            return Verdict::NotInstalled;
        }

        let mut problems = Vec::new();
        for (view, label) in [(&self.x64, "64 位元"), (&self.x86, "32 位元")] {
            if !view.profile {
                problems.push(format!(r"{label}檢視的 CTF\TIP\{clsid_text} 不存在"));
            }
            if view.inproc.is_none() {
                problems.push(format!(
                    r"{label}檢視的 HKCR\CLSID\{clsid_text}\InprocServer32 不存在"
                ));
            }
        }

        if let (Some(x64), Some(x86)) = (&self.x64.inproc, &self.x86.inproc) {
            if !file_name_is(x64, DLL_X64) {
                problems.push(format!(
                    "64 位元檢視的 InprocServer32 是 {x64}，應該是 {DLL_X64}"
                ));
            }
            if !file_name_is(x86, DLL_X86) {
                problems.push(format!(
                    "32 位元檢視的 InprocServer32 是 {x86}，應該是 {DLL_X86}"
                ));
            }
        }

        if problems.is_empty() {
            Verdict::Installed {
                x64_dll: self.x64.inproc.clone().unwrap_or_default(),
                x86_dll: self.x86.inproc.clone().unwrap_or_default(),
            }
        } else {
            Verdict::Broken(problems.join("；"))
        }
    }
}

/// `LanguageProfile\0x00000404\{PROFILE}` 底下的三個值。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MachineProfile {
    /// `Description`；沒有或空字串代表 msctf 不會顯示名稱。
    pub description: Option<String>,
    /// `Enable`（`REG_DWORD`）。
    pub enable: Option<u32>,
    /// `IconFile`；沒有圖示是正常出貨狀態。
    pub icon_file: Option<String>,
}

/// 目前使用者的語言清單。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserLanguageList {
    /// `Languages` 的每一項，例如 `zh-Hant-TW`。
    pub languages: Vec<String>,
    /// `zh-Hant-TW\InputMethodTips` 的每一項。
    pub input_method_tips: Vec<String>,
}

/// 取路徑的檔名並比較（不分大小寫）。
fn file_name_is(path: &str, expected: &str) -> bool {
    path.rsplit(['\\', '/'])
        .next()
        .is_some_and(|name| name.eq_ignore_ascii_case(expected))
}

/// 同時探測 64 位元與 32 位元檢視。
///
/// 機碼讀不到（不是「不存在」）回 [`SmokeError::Registry`]；呼叫端當 `[SKIP]`。
pub fn probe(clsid_text: &str) -> Result<Probe, SmokeError> {
    Ok(Probe {
        x64: probe_view(clsid_text, KEY_WOW64_64KEY)?,
        x86: probe_view(clsid_text, KEY_WOW64_32KEY)?,
    })
}

/// 探測單一檢視。
fn probe_view(clsid_text: &str, view: REG_SAM_FLAGS) -> Result<ViewProbe, SmokeError> {
    let (tip_count, profile) = match open(HKEY_LOCAL_MACHINE, TIP_KEY, view)? {
        None => (0, false),
        Some(key) => {
            let result = count_and_find(key, clsid_text);
            // SAFETY: key 由上面的 open 取得，這裡是唯一放手的地方。
            unsafe {
                let _ = RegCloseKey(key);
            }
            result?
        }
    };

    Ok(ViewProbe {
        tip_count,
        profile,
        inproc: read_inproc(clsid_text, view)?,
    })
}

/// 列出 `CTF\TIP` 的子機碼，回傳（總數, 是否含本產品的 CLSID）。
fn count_and_find(key: HKEY, clsid_text: &str) -> Result<(usize, bool), SmokeError> {
    let mut total = 0usize;
    let mut found = false;
    loop {
        let mut name = [0u16; 128];
        let mut length = name.len() as u32;
        // SAFETY: name 是長度正確的可寫緩衝；length 一併傳入長度。
        let status = unsafe {
            RegEnumKeyExW(
                key,
                total as u32,
                Some(PWSTR(name.as_mut_ptr())),
                &mut length,
                None,
                None,
                None,
                None,
            )
        };
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        if status != ERROR_SUCCESS {
            return Err(SmokeError::Registry(format!(
                "RegEnumKeyExW 第 {total} 項 = {}",
                report::hex(status.0)
            )));
        }
        total += 1;
        let text = String::from_utf16_lossy(&name[..length as usize]);
        if text.eq_ignore_ascii_case(clsid_text) {
            found = true;
        }
    }
    Ok((total, found))
}

/// 讀 `HKCR\CLSID\{CLSID}\InprocServer32` 的預設值。
fn read_inproc(clsid_text: &str, view: REG_SAM_FLAGS) -> Result<Option<String>, SmokeError> {
    let path = format!(r"CLSID\{clsid_text}\InprocServer32");
    let Some(key) = open(HKEY_CLASSES_ROOT, &path, view)? else {
        return Ok(None);
    };
    let result = query_default_string(key);
    // SAFETY: key 由上面的 open 取得，這裡是唯一放手的地方。
    unsafe {
        let _ = RegCloseKey(key);
    }
    result.map(Some)
}

/// 取安裝用的 `InprocServer32`：先看 64 位元檢視，沒有才退回 32 位元。
pub fn read_install_inproc(clsid_text: &str) -> Result<Option<String>, SmokeError> {
    match read_inproc(clsid_text, KEY_WOW64_64KEY)? {
        Some(path) => Ok(Some(path)),
        None => read_inproc(clsid_text, KEY_WOW64_32KEY),
    }
}

/// 讀某個檢視的 machine profile 機碼；不存在回 `None`。
pub fn read_machine_profile(
    clsid_text: &str,
    langid: u16,
    profile_text: &str,
    view: REG_SAM_FLAGS,
) -> Result<Option<MachineProfile>, SmokeError> {
    let path = profile::machine_profile_key(clsid_text, langid, profile_text);
    read_machine_profile_at(&path, view)
}

/// 舊安裝器寫的 `0x0404` 機碼；只用來提示名稱非標準。
pub fn read_legacy_machine_profile(
    clsid_text: &str,
    langid: u16,
    profile_text: &str,
    view: REG_SAM_FLAGS,
) -> Result<Option<MachineProfile>, SmokeError> {
    let path = profile::legacy_machine_profile_key(clsid_text, langid, profile_text);
    read_machine_profile_at(&path, view)
}

/// 讀指定路徑的 profile 機碼；不存在回 `None`。
fn read_machine_profile_at(
    path: &str,
    view: REG_SAM_FLAGS,
) -> Result<Option<MachineProfile>, SmokeError> {
    let Some(key) = open(HKEY_LOCAL_MACHINE, path, view)? else {
        return Ok(None);
    };
    let result = (|| {
        Ok(MachineProfile {
            description: query_string(key, "Description")?,
            enable: query_dword(key, "Enable")?,
            icon_file: query_string(key, "IconFile")?,
        })
    })();
    // SAFETY: key 由上面的 open 取得，這裡是唯一放手的地方。
    unsafe {
        let _ = RegCloseKey(key);
    }
    result.map(Some)
}

/// 讀目前使用者的 `Languages` 與 `zh-Hant-TW\InputMethodTips`。
///
/// 值不存在時回空清單；機碼本身讀不到才是錯誤。
pub fn read_user_language_list() -> Result<UserLanguageList, SmokeError> {
    let Some(key) = open(HKEY_CURRENT_USER, USER_PROFILE_KEY, REG_SAM_FLAGS(0))? else {
        return Ok(UserLanguageList::default());
    };
    let languages = query_multi_string(key, "Languages");
    // SAFETY: key 由上面的 open 取得，這裡是唯一放手的地方。
    unsafe {
        let _ = RegCloseKey(key);
    }
    let languages = languages?.unwrap_or_default();

    let tips_path = format!(r"{USER_PROFILE_KEY}\{LANGUAGE_ZH_HANT_TW}");
    let tips = match open(HKEY_CURRENT_USER, &tips_path, REG_SAM_FLAGS(0))? {
        None => Vec::new(),
        Some(key) => {
            let result = query_multi_string(key, "InputMethodTips");
            // SAFETY: key 由上面的 open 取得，這裡是唯一放手的地方。
            unsafe {
                let _ = RegCloseKey(key);
            }
            result?.unwrap_or_default()
        }
    };

    Ok(UserLanguageList {
        languages,
        input_method_tips: tips,
    })
}

/// 讀某個機碼的預設值（`REG_SZ`）。
fn query_default_string(key: HKEY) -> Result<String, SmokeError> {
    query_string_raw(key, None)?
        .ok_or_else(|| SmokeError::Registry("InprocServer32 的預設值不存在".to_string()))
}

/// 讀具名 `REG_SZ`；值不存在回 `None`。
fn query_string(key: HKEY, name: &str) -> Result<Option<String>, SmokeError> {
    query_string_raw(key, Some(name))
}

/// 讀字串值；`name` 為 `None` 代表預設值。
fn query_string_raw(key: HKEY, name: Option<&str>) -> Result<Option<String>, SmokeError> {
    let name = name.map(wide);
    let name_ptr = name
        .as_ref()
        .map_or(PCWSTR::null(), |name| PCWSTR(name.as_ptr()));

    let mut size = 0u32;
    let mut kind = REG_VALUE_TYPE::default();
    // SAFETY: 只查詢長度，資料指標傳 null；name 在呼叫期間保持存活。
    let status =
        unsafe { RegQueryValueExW(key, name_ptr, None, Some(&mut kind), None, Some(&mut size)) };
    match status {
        ERROR_SUCCESS => {}
        ERROR_FILE_NOT_FOUND => return Ok(None),
        other => {
            return Err(SmokeError::Registry(format!(
                "RegQueryValueExW({}) = {}",
                name.as_ref()
                    .map(|name| String::from_utf16_lossy(&name[..name.len() - 1]))
                    .unwrap_or_else(|| "(預設值)".to_string()),
                report::hex(other.0)
            )))
        }
    }

    let mut buffer = vec![0u16; (size as usize).div_ceil(2).max(1)];
    // SAFETY: buffer 的大小就是 size 給的位元組數（向上取整到 u16）。
    let status = unsafe {
        RegQueryValueExW(
            key,
            name_ptr,
            None,
            Some(&mut kind),
            Some(buffer.as_mut_ptr().cast::<u8>()),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(SmokeError::Registry(format!(
            "RegQueryValueExW(讀值) = {}",
            report::hex(status.0)
        )));
    }

    let units = &buffer[..(size as usize / 2).min(buffer.len())];
    Ok(Some(
        String::from_utf16_lossy(units)
            .trim_end_matches('\0')
            .to_string(),
    ))
}

/// 讀 `REG_DWORD`；值不存在回 `None`。
fn query_dword(key: HKEY, name: &str) -> Result<Option<u32>, SmokeError> {
    let name = wide(name);
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let mut kind = REG_VALUE_TYPE::default();
    // SAFETY: value 是 4 bytes 的可寫緩衝，size 與它一致。
    let status = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            Some((&mut value as *mut u32).cast::<u8>()),
            Some(&mut size),
        )
    };
    match status {
        ERROR_SUCCESS if kind == REG_DWORD => Ok(Some(value)),
        ERROR_SUCCESS => Ok(None),
        ERROR_FILE_NOT_FOUND => Ok(None),
        other => Err(SmokeError::Registry(format!(
            "RegQueryValueExW({name:?}, REG_DWORD) = {}",
            report::hex(other.0)
        ))),
    }
}

/// 讀 `REG_MULTI_SZ`；值不存在回 `None`。
fn query_multi_string(key: HKEY, name: &str) -> Result<Option<Vec<String>>, SmokeError> {
    let name_wide = wide(name);
    let mut size = 0u32;
    let mut kind = REG_VALUE_TYPE::default();
    // SAFETY: 只查詢長度，資料指標傳 null。
    let status = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(name_wide.as_ptr()),
            None,
            Some(&mut kind),
            None,
            Some(&mut size),
        )
    };
    match status {
        ERROR_SUCCESS => {}
        ERROR_FILE_NOT_FOUND => return Ok(None),
        other => {
            return Err(SmokeError::Registry(format!(
                "RegQueryValueExW({name}, REG_MULTI_SZ) = {}",
                report::hex(other.0)
            )))
        }
    }

    let mut buffer = vec![0u16; (size as usize).div_ceil(2).max(1)];
    // SAFETY: buffer 的大小就是 size 給的位元組數（向上取整到 u16）。
    let status = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(name_wide.as_ptr()),
            None,
            Some(&mut kind),
            Some(buffer.as_mut_ptr().cast::<u8>()),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(SmokeError::Registry(format!(
            "RegQueryValueExW({name}, 讀值) = {}",
            report::hex(status.0)
        )));
    }

    let units = &buffer[..(size as usize / 2).min(buffer.len())];
    let entries = units
        .split(|unit| *unit == 0)
        .filter(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect();
    Ok(Some(entries))
}

/// 用指定檢視開機碼；不存在時回 `None`。
fn open(root: HKEY, path: &str, view: REG_SAM_FLAGS) -> Result<Option<HKEY>, SmokeError> {
    let path_wide = wide(path);
    let mut key = HKEY::default();
    // SAFETY: path_wide 以 NUL 結尾；key 在成功時由 RegOpenKeyExW 寫入。
    let status = unsafe {
        RegOpenKeyExW(
            root,
            PCWSTR(path_wide.as_ptr()),
            None,
            KEY_READ | view,
            &mut key,
        )
    };
    match status {
        ERROR_SUCCESS => Ok(Some(key)),
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(None),
        other => Err(SmokeError::Registry(format!(
            "RegOpenKeyExW({path}) = {}",
            report::hex(other.0)
        ))),
    }
}

/// 把字串轉成以 NUL 結尾的 UTF-16。
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(profile: bool, inproc: Option<&str>) -> ViewProbe {
        ViewProbe {
            tip_count: 3,
            profile,
            inproc: inproc.map(str::to_string),
        }
    }

    const CLSID: &str = "{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}";
    const X64_PATH: &str = r"C:\Program Files\jc-ime\jc-tsf.dll";
    const X86_PATH: &str = r"C:\Program Files\jc-ime\jc-tsf-x86.dll";

    #[test]
    fn tip_key_path_is_stable() {
        assert_eq!(TIP_KEY, r"SOFTWARE\Microsoft\CTF\TIP");
    }

    #[test]
    fn clsid_comparison_ignores_case() {
        let expected = "{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}";
        assert!(expected.eq_ignore_ascii_case("{2f6b41d8-9e07-4c3a-b5f1-8d2c6a9e0473}"));
    }

    #[test]
    fn both_views_registered_is_installed() {
        let probe = Probe {
            x64: view(true, Some(X64_PATH)),
            x86: view(true, Some(X86_PATH)),
        };
        assert_eq!(
            probe.verdict(CLSID),
            Verdict::Installed {
                x64_dll: X64_PATH.to_string(),
                x86_dll: X86_PATH.to_string(),
            }
        );
    }

    #[test]
    fn neither_view_is_not_installed() {
        let probe = Probe {
            x64: view(false, None),
            x86: view(false, None),
        };
        assert_eq!(probe.verdict(CLSID), Verdict::NotInstalled);
    }

    /// 只註冊 32 位元那一邊：64 位元程式會看不到輸入法，訊息要指出來。
    #[test]
    fn only_the_x86_view_is_broken_and_says_so() {
        let probe = Probe {
            x64: view(false, None),
            x86: view(true, Some(X86_PATH)),
        };
        match probe.verdict(CLSID) {
            Verdict::Broken(message) => {
                assert!(message.contains("64 位元檢視"), "{message}");
                assert!(!message.contains("32 位元檢視的 CTF"), "{message}");
            }
            other => panic!("預期 Broken，得到 {other:?}"),
        }
    }

    /// 只註冊 64 位元那一邊：32 位元程式（舊版 Office）會打不出字。
    #[test]
    fn only_the_x64_view_is_broken_and_says_so() {
        let probe = Probe {
            x64: view(true, Some(X64_PATH)),
            x86: view(false, None),
        };
        match probe.verdict(CLSID) {
            Verdict::Broken(message) => {
                assert!(message.contains("32 位元檢視"), "{message}");
            }
            other => panic!("預期 Broken，得到 {other:?}"),
        }
    }

    /// 兩個檢視都指到同一份 DLL：其中一個架構一定載入失敗。
    #[test]
    fn same_dll_in_both_views_is_broken() {
        let probe = Probe {
            x64: view(true, Some(X64_PATH)),
            x86: view(true, Some(X64_PATH)),
        };
        match probe.verdict(CLSID) {
            Verdict::Broken(message) => {
                assert!(message.contains(DLL_X86), "{message}");
            }
            other => panic!("預期 Broken，得到 {other:?}"),
        }
    }

    /// profile 有、但 COM 機碼不見（例如被手動刪掉）。
    #[test]
    fn missing_com_key_is_broken() {
        let probe = Probe {
            x64: view(true, None),
            x86: view(true, Some(X86_PATH)),
        };
        match probe.verdict(CLSID) {
            Verdict::Broken(message) => {
                assert!(message.contains("InprocServer32"), "{message}");
            }
            other => panic!("預期 Broken，得到 {other:?}"),
        }
    }

    #[test]
    fn file_name_matching_ignores_case_and_separators() {
        assert!(file_name_is(X64_PATH, "JC-TSF.DLL"));
        assert!(file_name_is("/opt/jc/jc-tsf-x86.dll", DLL_X86));
        assert!(!file_name_is(X64_PATH, DLL_X86));
    }

    /// 這台機器真的讀得到目前使用者的語言清單時，值必須是字串清單；讀不到
    /// （極精簡的 CI 容器）就跳過，不讓測試失敗。
    #[cfg(windows)]
    #[test]
    fn user_language_list_reads_on_windows() {
        match read_user_language_list() {
            Ok(list) => {
                assert!(list.languages.iter().all(|entry| !entry.is_empty()));
                assert!(list.input_method_tips.iter().all(|entry| !entry.is_empty()));
            }
            Err(error) => eprintln!("略過：{error}"),
        }
    }

    /// Windows 上一定讀得到 `HKLM\SOFTWARE\Microsoft\CTF` 或得到明確錯誤；
    /// 這裡只鎖住「不會 panic、錯誤會帶路徑」。
    #[cfg(windows)]
    #[test]
    fn machine_profile_read_returns_or_reports() {
        match read_machine_profile(
            CLSID,
            0x0404,
            "{C4E8D217-5B3F-4A96-8E0C-7F1A2B3D4E56}",
            KEY_WOW64_64KEY,
        ) {
            Ok(Some(profile)) => {
                if let Some(description) = &profile.description {
                    assert!(!description.is_empty(), "Description 不該是空字串");
                }
                if let Some(enable) = profile.enable {
                    assert!(enable <= 1, "Enable 只該是 0 或 1，得到 {enable}");
                }
            }
            Ok(None) => {}
            Err(error) => assert!(error.to_string().contains("註冊表")),
        }
    }
}
