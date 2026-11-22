//! 本產品的固定識別碼。
//!
//! 值必須與 `crates/jc-tsf/src/guids.rs` 一致。`jc-tsf` 只產出 cdylib，冒煙程式
//! 沒辦法直接 `use` 它的常數，所以這裡留一份；兩邊都是發佈後不可改的契約值。

use windows::core::GUID;

/// 文字服務物件的 CLSID。
pub const CLSID_JC_TSF: GUID = GUID::from_u128(0x2f6b41d8_9e07_4c3a_b5f1_8d2c6a9e0473);

/// 注音 profile 的 GUID（只有報表會印，不參與冒煙判定）。
pub const GUID_PROFILE_JC_TSF: GUID = GUID::from_u128(0xc4e8d217_5b3f_4a96_8e0c_7f1a2b3d4e56);

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
            "{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}"
        );
    }

    #[test]
    fn profile_guid_is_stable() {
        assert_eq!(
            registry_key(&GUID_PROFILE_JC_TSF),
            "{C4E8D217-5B3F-4A96-8E0C-7F1A2B3D4E56}"
        );
        assert_eq!(LANGID_ZH_TW, 0x0404);
        assert_eq!(DISPLAY_NAME, "jc 注音");
    }
}
