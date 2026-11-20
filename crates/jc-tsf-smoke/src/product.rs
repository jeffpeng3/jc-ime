//! 本產品的固定識別碼。
//!
//! 值必須與 `crates/jc-tsf/src/guids.rs` 一致。`jc-tsf` 只產出 cdylib，冒煙程式
//! 沒辦法直接 `use` 它的常數，所以這裡留一份；兩邊都是發佈後不可改的契約值。

use windows::core::GUID;

/// 文字服務物件的 CLSID。
pub const CLSID_JC_TSF: GUID = GUID::from_u128(0x7b7d6e1a_3c1f_4b2e_9a55_6c1d2f3e4a01);

/// 注音 profile 的 GUID（只有報表會印，不參與冒煙判定）。
pub const GUID_PROFILE_JC_TSF: GUID = GUID::from_u128(0x8f0a9b3c_2d4e_4f50_a1b2_c3d4e5f60718);

/// 文字服務註冊的語言（zh-TW）。
pub const LANGID_ZH_TW: u16 = 0x0404;

/// 語言列與設定頁看到的名稱。
pub const DISPLAY_NAME: &str = "jc 注音";

/// 把 GUID 格式化成註冊機碼用的 `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}`。
///
/// 與 `jc_tsf::guids::registry_key` 相同：大寫十六進位、加花括號。
pub fn registry_key(guid: &GUID) -> String {
    let d4 = guid.data4;
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        guid.data1, guid.data2, guid.data3, d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clsid_text_matches_tsf_skeleton() {
        assert_eq!(
            registry_key(&CLSID_JC_TSF),
            "{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}"
        );
    }

    #[test]
    fn profile_guid_is_stable() {
        assert_eq!(
            registry_key(&GUID_PROFILE_JC_TSF),
            "{8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}"
        );
        assert_eq!(LANGID_ZH_TW, 0x0404);
        assert_eq!(DISPLAY_NAME, "jc 注音");
    }
}
