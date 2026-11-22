//! 純字串／路徑的產品知識，不碰 Win32。
//!
//! 機碼路徑、`InstallLayoutOrTip` 的 `InputMethodTips` 格式與安裝目錄推導都放在
//! 這裡，Linux 的 `cargo check` 與單元測試才能一起驗證。真正讀機碼的程式在
//! `registry.rs`，這裡只負責產生名稱與比對字串。

use std::path::PathBuf;

/// `HKLM` 下 TIP profile 的相對路徑。
///
/// 標準格式是 `0x00000404`（八位十六進位）；舊安裝器曾寫成 `0x0404`，所以
/// 另外保留 [`legacy_machine_profile_key`] 供診斷比對。
pub fn machine_profile_key(clsid_text: &str, langid: u16, profile_text: &str) -> String {
    format!(
        r"SOFTWARE\Microsoft\CTF\TIP\{clsid_text}\LanguageProfile\{}\{profile_text}",
        langid_key(langid)
    )
}

/// 非標準的舊機碼名（`0x0404`），只用來提示「有寫但 msctf 讀不到」。
pub fn legacy_machine_profile_key(clsid_text: &str, langid: u16, profile_text: &str) -> String {
    format!(
        r"SOFTWARE\Microsoft\CTF\TIP\{clsid_text}\LanguageProfile\0x{langid:04X}\{profile_text}"
    )
}

/// 機碼名稱用的 LANGID 格式：`0x00000404`。
pub fn langid_key(langid: u16) -> String {
    format!("0x{langid:08X}")
}

/// 目前使用者的語言清單機碼（相對於 `HKEY_CURRENT_USER`）。
pub const USER_PROFILE_KEY: &str = r"Control Panel\International\User Profile";

/// 繁體中文（台灣）在語言清單裡的名稱。
pub const LANGUAGE_ZH_HANT_TW: &str = "zh-Hant-TW";

/// `InputMethodTips` 的標準字串：`0404:{CLSID}{PROFILE}`。
pub fn input_method_tip(clsid_text: &str, profile_text: &str, langid: u16) -> String {
    format!(
        "{:04X}:{}{}",
        langid,
        ensure_braces(clsid_text),
        ensure_braces(profile_text)
    )
}

/// 比對一個 `InputMethodTips` 項目是否就是本產品。
///
/// 接受有無大括號、大小寫不同、以及 `0x0404:` 前綴；Windows 各版本寫入的格式
/// 不完全一致，但語意相同。
pub fn tip_matches(entry: &str, clsid_text: &str, profile_text: &str, langid: u16) -> bool {
    let Some((language, rest)) = entry.trim().split_once(':') else {
        return false;
    };
    if normalize_langid(language) != format!("{langid:04X}") {
        return false;
    }
    let rest = normalize_guid_pair(rest);
    let expected = normalize_guid_pair(&format!("{clsid_text}{profile_text}"));
    rest == expected
}

/// 比對語言清單的 `Languages` 項目（不分大小寫）。
pub fn language_matches(entry: &str, language: &str) -> bool {
    entry.trim().eq_ignore_ascii_case(language)
}

/// 目前使用者 profile 的判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserProfileVerdict {
    /// 已列舉且已啟用。
    Ready,
    /// 已列舉，但 `IsEnabledLanguageProfile` 為 false。
    Disabled,
    /// `EnumLanguageProfiles(0x0404)` 沒有列出本產品：machine profile 沒寫好。
    NotEnumerated,
}

/// 由 TSF API 的兩個結果決定使用者 profile 的狀態。
pub fn user_profile_verdict(enabled: bool, enumerated: bool) -> UserProfileVerdict {
    if !enumerated {
        UserProfileVerdict::NotEnumerated
    } else if !enabled {
        UserProfileVerdict::Disabled
    } else {
        UserProfileVerdict::Ready
    }
}

/// 從 `InprocServer32` 的完整路徑推出安裝目錄。
///
/// 同時接受反斜線與斜線，並去掉路徑兩側的引號；沒有目錄成分時回 `None`。
pub fn install_dir(inproc: &str) -> Option<PathBuf> {
    let trimmed = inproc.trim().trim_matches('"');
    let index = trimmed.rfind(['\\', '/'])?;
    if index == 0 {
        return None;
    }
    Some(PathBuf::from(&trimmed[..index]))
}

/// 補上大括號（已有就不重複加）。
fn ensure_braces(guid: &str) -> String {
    let trimmed = guid.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        trimmed.to_string()
    } else {
        format!("{{{trimmed}}}")
    }
}

/// 把語言前綴正規化成四位十六進位：`0404`、`0x0404`、`0X0404` 都接受。
fn normalize_langid(text: &str) -> String {
    let text = text.trim();
    let text = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    format!("{:0>4}", text.to_uppercase())
}

/// 去掉大括號、空白並轉大寫，讓 `{A}{B}` 與 `AB` 可以相比。
fn normalize_guid_pair(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '{' && *ch != '}')
        .collect::<String>()
        .to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLSID: &str = "{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}";
    const PROFILE: &str = "{C4E8D217-5B3F-4A96-8E0C-7F1A2B3D4E56}";

    #[test]
    fn machine_profile_key_uses_eight_hex_digits() {
        assert_eq!(
            machine_profile_key(CLSID, 0x0404, PROFILE),
            r"SOFTWARE\Microsoft\CTF\TIP\{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}\LanguageProfile\0x00000404\{C4E8D217-5B3F-4A96-8E0C-7F1A2B3D4E56}"
        );
        assert_eq!(langid_key(0x0404), "0x00000404");
        assert_eq!(langid_key(0x0409), "0x00000409");
    }

    #[test]
    fn legacy_machine_profile_key_is_distinct() {
        let canonical = machine_profile_key(CLSID, 0x0404, PROFILE);
        let legacy = legacy_machine_profile_key(CLSID, 0x0404, PROFILE);
        assert_ne!(canonical, legacy);
        assert!(legacy.contains(r"\LanguageProfile\0x0404\"));
    }

    #[test]
    fn user_profile_key_matches_windows() {
        assert_eq!(
            USER_PROFILE_KEY,
            r"Control Panel\International\User Profile"
        );
    }

    #[test]
    fn input_method_tip_uses_langid_then_clsid_then_profile() {
        assert_eq!(
            input_method_tip(CLSID, PROFILE, 0x0404),
            format!("0404:{CLSID}{PROFILE}")
        );
    }

    #[test]
    fn tip_matches_accepts_braces_and_case() {
        let with_braces = format!("0404:{CLSID}{PROFILE}");
        assert!(tip_matches(&with_braces, CLSID, PROFILE, 0x0404));

        let without_braces = format!(
            "0404:{}{}",
            CLSID.trim_matches(['{', '}']),
            PROFILE.trim_matches(['{', '}'])
        );
        assert!(tip_matches(&without_braces, CLSID, PROFILE, 0x0404));

        let lowercase = format!("0404:{CLSID}{PROFILE}").to_lowercase();
        assert!(tip_matches(&lowercase, CLSID, PROFILE, 0x0404));

        let with_prefix = format!("0x0404:{CLSID}{PROFILE}");
        assert!(tip_matches(&with_prefix, CLSID, PROFILE, 0x0404));
    }

    #[test]
    fn tip_matches_rejects_wrong_language_or_profile() {
        let entry = format!("0404:{CLSID}{PROFILE}");
        assert!(!tip_matches(&entry, CLSID, PROFILE, 0x0409));
        assert!(!tip_matches(
            &format!("0409:{CLSID}{PROFILE}"),
            CLSID,
            PROFILE,
            0x0404
        ));
        assert!(!tip_matches(
            &format!("0404:{CLSID}{{00000000-0000-0000-0000-000000000000}}"),
            CLSID,
            PROFILE,
            0x0404
        ));
        assert!(!tip_matches("zh-Hant-TW", CLSID, PROFILE, 0x0404));
    }

    #[test]
    fn language_matches_ignores_case_and_space() {
        assert!(language_matches("zh-Hant-TW", LANGUAGE_ZH_HANT_TW));
        assert!(language_matches(" ZH-hant-tw ", LANGUAGE_ZH_HANT_TW));
        assert!(!language_matches("en-US", LANGUAGE_ZH_HANT_TW));
    }

    #[test]
    fn user_profile_verdict_prioritises_enumeration() {
        assert_eq!(user_profile_verdict(true, true), UserProfileVerdict::Ready);
        assert_eq!(
            user_profile_verdict(false, true),
            UserProfileVerdict::Disabled
        );
        assert_eq!(
            user_profile_verdict(false, false),
            UserProfileVerdict::NotEnumerated
        );
        assert_eq!(
            user_profile_verdict(true, false),
            UserProfileVerdict::NotEnumerated
        );
    }

    #[test]
    fn install_dir_strips_quotes_and_handles_both_separators() {
        assert_eq!(
            install_dir(r"C:\Program Files\jc-ime\jc-tsf.dll"),
            Some(PathBuf::from(r"C:\Program Files\jc-ime"))
        );
        assert_eq!(
            install_dir(r#""C:\Program Files\jc-ime\jc-tsf-x86.dll""#),
            Some(PathBuf::from(r"C:\Program Files\jc-ime"))
        );
        assert_eq!(
            install_dir("/opt/jc/jc-tsf.dll"),
            Some(PathBuf::from("/opt/jc"))
        );
        assert_eq!(install_dir("jc-tsf.dll"), None);
    }
}
