//! 固定的 CLSID、profile GUID 與 `InstallLayoutOrTip` 的參數。
//!
//! 這些值必須與 `crates/jc-tsf/src/guids.rs` 一致：機碼、TSF profile 資料庫與
//! 使用者語言清單都以它們為鍵。安裝器不 import `jc-tsf`（DLL 沒有 lib 可用），
//! 所以這裡放一份常數，並用測試把格式與「兩個架構不同」固定住。
//!
//! x64 與 x86 各一組。`jc-tip-register` 依 [`Arch`] 選用其中一組；移除時
//! 兩組都要清。

/// x64 文字服務物件的 CLSID；與 `jc-tsf` 的 x64 建置相同。
pub const CLSID_JC_TSF_X64: &str = "{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}";

/// x86 文字服務物件的 CLSID；與 `jc-tsf` 的 i686 建置相同。
pub const CLSID_JC_TSF_X86: &str = "{3E9A5C72-8B14-4D6F-A0C8-5F2B7E9D1346}";

/// x64 注音 profile 的 GUID。
pub const PROFILE_JC_TSF_X64: &str = "{8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}";

/// x86 注音 profile 的 GUID。
pub const PROFILE_JC_TSF_X86: &str = "{6B1F8D43-5A27-4C90-9E36-7D4B2F0A8C51}";

/// 第一版只註冊繁體中文（台灣）。
pub const LANGID_ZH_TW: u16 = 0x0404;

/// x64 profile 的顯示名稱；與 `jc-tsf` 的 x64 建置相同。
pub const PROFILE_DESCRIPTION_X64: &str = "jc 注音";

/// x86 profile 的顯示名稱；與 `jc-tsf` 的 i686 建置相同。
///
/// 兩個架構各自一個 profile，使用者的輸入法清單會同時出現兩個項目，名稱必須
/// 分得出來。
pub const PROFILE_DESCRIPTION_X86: &str = "jc 注音（32 位元）";

/// `InstallLayoutOrTip` 的移除旗標（`input.h` 沒有公開這個常數）。
pub const ILOT_UNINSTALL: u32 = 0x0000_0001;

/// `InstallLayoutOrTip` 的安裝旗標：文件沒有列，實際值是 0。
///
/// 不帶 `ILOT_DEFPROFILE`：不改使用者的預設輸入法。
pub const ILOT_INSTALL: u32 = 0x0000_0000;

/// 指定架構的 CLSID 字串。
pub fn clsid(arch: crate::arch::Arch) -> &'static str {
    match arch {
        crate::arch::Arch::X64 => CLSID_JC_TSF_X64,
        crate::arch::Arch::X86 => CLSID_JC_TSF_X86,
    }
}

/// 指定架構的 profile GUID 字串。
pub fn profile(arch: crate::arch::Arch) -> &'static str {
    match arch {
        crate::arch::Arch::X64 => PROFILE_JC_TSF_X64,
        crate::arch::Arch::X86 => PROFILE_JC_TSF_X86,
    }
}

/// 指定架構的 `InstallLayoutOrTip` 字串：`<LangID>:<profile GUID><CLSID>`。
pub fn tip_string(arch: crate::arch::Arch) -> String {
    format!("0x{LANGID_ZH_TW:04X}:{}{}", profile(arch), clsid(arch))
}

/// 指定架構的 profile 顯示名稱。
pub fn description(arch: crate::arch::Arch) -> &'static str {
    match arch {
        crate::arch::Arch::X64 => PROFILE_DESCRIPTION_X64,
        crate::arch::Arch::X86 => PROFILE_DESCRIPTION_X86,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::Arch;

    #[test]
    fn tip_string_uses_langid_then_profile_then_clsid() {
        assert_eq!(
            tip_string(Arch::X64),
            format!("0x{LANGID_ZH_TW:04X}:{PROFILE_JC_TSF_X64}{CLSID_JC_TSF_X64}")
        );
        assert_eq!(
            tip_string(Arch::X86),
            format!("0x{LANGID_ZH_TW:04X}:{PROFILE_JC_TSF_X86}{CLSID_JC_TSF_X86}")
        );
        // 語言在前、profile 在中、CLSID 在後；`InstallLayoutOrTip` 只認這個順序。
        assert!(tip_string(Arch::X64).starts_with("0x0404:{8F0A9B3C-"));
        assert!(tip_string(Arch::X86).ends_with("{3E9A5C72-8B14-4D6F-A0C8-5F2B7E9D1346}"));
    }

    #[test]
    fn clsid_matches_tsf_crate() {
        assert_eq!(CLSID_JC_TSF_X64, "{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}");
        assert_eq!(PROFILE_JC_TSF_X64, "{8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}");
        assert_eq!(CLSID_JC_TSF_X86, "{3E9A5C72-8B14-4D6F-A0C8-5F2B7E9D1346}");
        assert_eq!(PROFILE_JC_TSF_X86, "{6B1F8D43-5A27-4C90-9E36-7D4B2F0A8C51}");
    }

    /// 兩個架構必須是不同識別碼，否則「各自註冊」就沒有意義。
    #[test]
    fn arch_specific_guids_differ() {
        assert_ne!(clsid(Arch::X64), clsid(Arch::X86));
        assert_ne!(profile(Arch::X64), profile(Arch::X86));
        assert_ne!(tip_string(Arch::X64), tip_string(Arch::X86));
        assert_ne!(description(Arch::X64), description(Arch::X86));
        assert_eq!(description(Arch::X64), "jc 注音");
        assert_eq!(description(Arch::X86), "jc 注音（32 位元）");
    }

    #[test]
    fn install_flag_is_zero_and_uninstall_is_one() {
        assert_eq!(ILOT_INSTALL, 0);
        assert_eq!(ILOT_UNINSTALL, 1);
    }
}
