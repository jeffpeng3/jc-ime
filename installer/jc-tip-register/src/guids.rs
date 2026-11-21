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

// ---- T9／T10b：平行建置的新版身分 ----
//
// 這些值與 `crates/jc-tsf-next/src/guids.rs` 的 T9 段一致。舊版已發行常數保持不動，
// 灰度期間只新增，不替換。新舊各有自己的 CLSID、profile、描述與 pipe 前綴，
// 所以可以在同一台機器上平行註冊，也能用系統輸入法切換。

/// 新版文字服務的 CLSID；與 `CLSID_JC_TSF` 不同，可同時註冊。
pub const CLSID_JC_TSF_NEXT: &str = "{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}";

/// 新版注音 profile 的 GUID；與 `PROFILE_JC_TSF` 並存。
pub const PROFILE_JC_TSF_NEXT: &str = "{C4E8D217-5B3F-4A96-8E0C-7F1A2B3D4E56}";

/// 新版的顯示名稱；與舊版不同，系統清單才會出現兩項。
pub const PROFILE_DESCRIPTION_NEXT: &str = "jc 注音（新版）";

/// 新版引擎的 pipe 前綴，解析後是 `jc-ime-next-<SID 尾段>`。
///
/// MSI 的 `<Class>` 把這個值寫進新版 CLSID 機碼的 `PipePrefix`；這支程式只讀
/// CLSID 與 profile，所以執行期用不到，保留常數是為了與 WXS 對照及測試鎖值。
#[allow(dead_code)]
pub const PIPE_PREFIX_NEXT: &str = "jc-ime-next";

/// 舊版已發行的 pipe 前綴；只用於相容檢查與文件，新版連線不使用。
#[allow(dead_code)]
pub const PIPE_PREFIX_OLD: &str = "jc-ime";

/// 共用 `InstallLayoutOrTip` 字串格式：`<LangID>:<profile GUID><CLSID>`。
fn tip_string_for(profile: &str, clsid: &str) -> String {
    format!("0x{LANGID_ZH_TW:04X}:{profile}{clsid}")
}

/// 舊版的 `InstallLayoutOrTip` 字串。
pub fn tip_string() -> String {
    tip_string_for(PROFILE_JC_TSF, CLSID_JC_TSF)
}

/// 新版的 `InstallLayoutOrTip` 字串，用新 profile 與新 CLSID 並存註冊。
pub fn tip_string_next() -> String {
    tip_string_for(PROFILE_JC_TSF_NEXT, CLSID_JC_TSF_NEXT)
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

    /// 新版身分不能與已發行值相同；舊值在這裡重複一份只是為了鎖住這件事。
    #[test]
    fn next_identity_does_not_reuse_published_values() {
        assert_eq!(CLSID_JC_TSF_NEXT, "{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}");
        assert_eq!(
            PROFILE_JC_TSF_NEXT,
            "{C4E8D217-5B3F-4A96-8E0C-7F1A2B3D4E56}"
        );
        assert_eq!(PROFILE_DESCRIPTION_NEXT, "jc 注音（新版）");
        assert_eq!(PIPE_PREFIX_NEXT, "jc-ime-next");
        assert_ne!(CLSID_JC_TSF_NEXT, CLSID_JC_TSF);
        assert_ne!(PROFILE_JC_TSF_NEXT, PROFILE_JC_TSF);
        assert_ne!(PROFILE_DESCRIPTION_NEXT, PROFILE_DESCRIPTION);
        assert_ne!(PIPE_PREFIX_NEXT, PIPE_PREFIX_OLD);
    }

    #[test]
    fn tip_string_next_uses_new_identity() {
        assert_eq!(
            tip_string_next(),
            format!("0x{LANGID_ZH_TW:04X}:{PROFILE_JC_TSF_NEXT}{CLSID_JC_TSF_NEXT}")
        );
        assert!(tip_string_next().starts_with("0x0404:{C4E8D217-"));
        assert!(tip_string_next().ends_with("{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}"));
        assert_ne!(tip_string_next(), tip_string());
    }
}
