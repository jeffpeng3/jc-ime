//! 固定的 CLSID、profile GUID 與 `InstallLayoutOrTip` 的參數。
//!
//! 這些值必須與 `crates/jc-tsf/src/guids.rs` 一致：機碼、TSF profile 資料庫與
//! 使用者語言清單都以它們為鍵。安裝器不 import `jc-tsf`（DLL 沒有 lib 可用），
//! 所以這裡放一份常數，並用測試把格式固定住。

/// 文字服務物件的 CLSID；與 `jc-tsf` 相同。
pub const CLSID_JC_TSF: &str = "{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}";

/// 注音 profile 的 GUID；與 `jc-tsf` 相同。
pub const PROFILE_JC_TSF: &str = "{8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}";

/// 第一版只註冊繁體中文（台灣）。
pub const LANGID_ZH_TW: u16 = 0x0404;

/// `InstallLayoutOrTip` 的文字服務字串：`<LangID>:<profile GUID><CLSID>`。
pub const TIP_STRING: &str =
    "0x0404:{8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}";

/// `InstallLayoutOrTip` 的移除旗標（`input.h` 沒有公開這個常數）。
pub const ILOT_UNINSTALL: u32 = 0x0000_0001;

/// `InstallLayoutOrTip` 的安裝旗標：文件沒有列，實際值是 0。
///
/// 不帶 `ILOT_DEFPROFILE`：不改使用者的預設輸入法。
pub const ILOT_INSTALL: u32 = 0x0000_0000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tip_string_uses_langid_then_profile_then_clsid() {
        assert_eq!(
            TIP_STRING,
            format!("0x{LANGID_ZH_TW:04X}:{PROFILE_JC_TSF}{CLSID_JC_TSF}")
        );
    }

    #[test]
    fn clsid_matches_tsf_crate() {
        assert_eq!(CLSID_JC_TSF, "{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}");
        assert_eq!(PROFILE_JC_TSF, "{8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}");
    }

    #[test]
    fn install_flag_is_zero_and_uninstall_is_one() {
        assert_eq!(ILOT_INSTALL, 0);
        assert_eq!(ILOT_UNINSTALL, 1);
    }
}
