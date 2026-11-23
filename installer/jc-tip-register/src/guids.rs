//! 固定的 CLSID、profile GUID 與 `InstallLayoutOrTip` 的參數。
//!
//! 這些值必須與 `crates/jc-tsf/src/guids.rs` 與 `installer/jc.iss` 的 `[Registry]`
//! 一致：機碼、TSF profile 資料庫與使用者語言清單都以它們為鍵。
//! 安裝器不 import `jc-tsf`（DLL 沒有 lib 可用），所以這裡放一份常數，並用測試把
//! 格式固定住。
//!
//! **只有一個 CLSID 與一個 profile**：x64 與 x86 的 `InprocServer32` 各自寫在
//! 64 位元檢視與 `Wow6432Node`，CLSID 相同。使用者的語言清單因此只有一項。
//! 已移除的舊值保留為 `*_LEGACY`，只給語言清單清理對照。

/// 現行文字服務物件的 CLSID；rc.15 已發行值，保留不改。
pub const CLSID_JC_TSF: &str = "{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}";

/// 現行注音 profile 的 GUID；rc.15 已發行值，保留不改。
pub const PROFILE_JC_TSF: &str = "{C4E8D217-5B3F-4A96-8E0C-7F1A2B3D4E56}";

/// 已移除的舊 CLSID；只給語言清單清理對照，不再註冊。
pub const CLSID_JC_TSF_LEGACY: &str = "{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}";

/// 已移除的舊 profile GUID；只給語言清單清理對照，不再註冊。
pub const PROFILE_JC_TSF_LEGACY: &str = "{8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}";

/// 第一版只註冊繁體中文（台灣）。
pub const LANGID_ZH_TW: u16 = 0x0404;

/// profile 的顯示名稱；與 `jc-tsf` 相同。
pub const PROFILE_DESCRIPTION: &str = "jc 注音";

/// `InstallLayoutOrTip` 的安裝旗標：文件沒有列，實際值是 0。
///
/// 不帶 `ILOT_DEFPROFILE`：不改使用者的預設輸入法。
///
/// 刻意沒有 `ILOT_UNINSTALL`：反安裝不從使用者的語言清單移除 layout，理由見
/// `platform::user_uninstall`。
pub const ILOT_INSTALL: u32 = 0x0000_0000;

/// 引擎 pipe 前綴；安裝器的 `[Registry]` 把它寫進 CLSID 機碼的 `PipePrefix`。
/// 這支程式只讀 CLSID 與 profile，保留常數是為了與 `jc.iss` 對照及測試鎖值。
#[allow(dead_code)]
pub const PIPE_PREFIX: &str = "jc-ime";

/// 共用 `InstallLayoutOrTip` 字串格式：`<LangID>:<profile GUID><CLSID>`。
/// `InstallLayoutOrTip` 的文字服務字串。
///
/// 官方格式是 `<LangID>:{CLSID}{profile};`（見 MicrosoftDocs/win32 的
/// `TSF/installlayoutortip.md` 範例）：
///
/// ```text
/// 0x0412:{A028AE76-...}{B5FE1F02-...};0x040C:0x0000040C
/// ```
///
/// 三個地方都不能省：LangID 要帶 `0x`、大括號裡先 CLSID 再 profile、結尾有分號。
/// 格式不對時 `InstallLayoutOrTip` 會回 TRUE 但什麼都不做，所以這些字元是鎖在
/// 測試裡的，不是排好看的。
fn tip_string_for(profile: &str, clsid: &str) -> String {
    format!("0x{LANGID_ZH_TW:04X}:{clsid}{profile};")
}

/// 現行身分的 `InstallLayoutOrTip` 字串。
pub fn tip_string() -> String {
    tip_string_for(PROFILE_JC_TSF, CLSID_JC_TSF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tip_string_is_langid_then_clsid_then_profile() {
        assert_eq!(
            tip_string(),
            format!("0x{LANGID_ZH_TW:04X}:{CLSID_JC_TSF}{PROFILE_JC_TSF};")
        );
        assert_eq!(
            tip_string(),
            "0x0404:{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}{C4E8D217-5B3F-4A96-8E0C-7F1A2B3D4E56};"
        );
        assert!(tip_string().starts_with("0x0404:{2F6B41D8-"));
        assert!(tip_string().ends_with("{C4E8D217-5B3F-4A96-8E0C-7F1A2B3D4E56};"));
    }

    #[test]
    fn clsid_matches_tsf_crate() {
        assert_eq!(CLSID_JC_TSF, "{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}");
        assert_eq!(PROFILE_JC_TSF, "{C4E8D217-5B3F-4A96-8E0C-7F1A2B3D4E56}");
        assert_eq!(PROFILE_DESCRIPTION, "jc 注音");
        assert_eq!(PIPE_PREFIX, "jc-ime");
    }

    #[test]
    fn install_flag_is_zero() {
        assert_eq!(ILOT_INSTALL, 0);
    }

    /// 現行身分不能與已移除的舊值相同；舊值在這裡重複一份只是為了鎖住這件事。
    #[test]
    fn current_identity_keeps_legacy_values_for_cleanup() {
        assert_eq!(
            CLSID_JC_TSF_LEGACY,
            "{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}"
        );
        assert_eq!(
            PROFILE_JC_TSF_LEGACY,
            "{8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}"
        );
        assert_ne!(CLSID_JC_TSF, CLSID_JC_TSF_LEGACY);
        assert_ne!(PROFILE_JC_TSF, PROFILE_JC_TSF_LEGACY);
    }
}
