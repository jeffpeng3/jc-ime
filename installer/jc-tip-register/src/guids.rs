//! 固定的 CLSID、profile GUID 與 `InstallLayoutOrTip` 的參數。
//!
//! 這些值必須與 `crates/jc-tsf/src/guids.rs`、`crates/jc-tsf/src/register.rs` 與
//! `installer/jc-ime.wxs` 的 `<Class Id="...">` 一致：機碼、TSF profile 資料庫與
//! 使用者語言清單都以它們為鍵。安裝器不 import `jc-tsf`（DLL 沒有 lib 可用），
//! 所以這裡放一份常數，並用測試把格式固定住。
//!
//! **只有一個 CLSID 與一個 profile**：x64 與 x86 的 `InprocServer32` 各自寫在
//! 64 位元檢視與 `Wow6432Node`，CLSID 相同。使用者的語言清單因此只有一項。

/// 文字服務物件的 CLSID；兩個架構共用。
pub const CLSID_JC_TSF: &str = "{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}";

/// 注音 profile 的 GUID；兩個架構共用。
pub const PROFILE_JC_TSF: &str = "{8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}";

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

/// `InstallLayoutOrTip` 的字串：`<LangID>:<profile GUID><CLSID>`。
pub fn tip_string() -> String {
    format!("0x{LANGID_ZH_TW:04X}:{PROFILE_JC_TSF}{CLSID_JC_TSF}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tip_string_uses_langid_then_profile_then_clsid() {
        assert_eq!(
            tip_string(),
            format!("0x{LANGID_ZH_TW:04X}:{PROFILE_JC_TSF}{CLSID_JC_TSF}")
        );
        assert!(tip_string().starts_with("0x0404:{8F0A9B3C-"));
        assert!(tip_string().ends_with("{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}"));
    }

    #[test]
    fn clsid_matches_tsf_crate() {
        assert_eq!(CLSID_JC_TSF, "{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}");
        assert_eq!(PROFILE_JC_TSF, "{8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}");
        assert_eq!(PROFILE_DESCRIPTION, "jc 注音");
    }

    #[test]
    fn install_flag_is_zero() {
        assert_eq!(ILOT_INSTALL, 0);
    }
}
