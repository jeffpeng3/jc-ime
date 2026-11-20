//! 固定的 CLSID 與 profile GUID。
//!
//! 這些值一旦發佈就不能改：註冊機碼、TSF 的 profile 資料庫與使用者的輸入法
//! 設定都以它們為鍵。`docs/port/tsf-skeleton.md` 有一份相同的清單。

use windows::core::GUID;

/// 文字服務物件的 CLSID。
///
/// 註冊機碼為 `HKCR\CLSID\{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}`。
pub const CLSID_JC_TSF: GUID = GUID::from_u128(0x7b7d6e1a_3c1f_4b2e_9a55_6c1d2f3e4a01);

/// 注音 profile 的 GUID。
///
/// 語言固定為 `0x0404`（zh-TW）。這個 GUID 只在 TSF 的 profile 資料庫裡使用，
/// 不會出現在註冊機碼。
pub const GUID_PROFILE_JC_TSF: GUID = GUID::from_u128(0x8f0a9b3c_2d4e_4f50_a1b2_c3d4e5f60718);

/// 候選清單 UI 元素的 GUID。
///
/// `ITfUIElement::GetGUID` 用它識別「這是候選窗」。同一種 UI 元素共用一個
/// GUID，不是每個實例不同；發佈後不可改。
pub const GUID_CANDIDATE_UI_ELEMENT: GUID = GUID::from_u128(0x9c4e7a21_6b3d_4f08_b5a7_2e1d8c6f9034);

/// 組字底線的顯示屬性 GUID。
///
/// 這個 GUID 同時是 `ITfDisplayAttributeInfo::GetGUID` 的回傳值，也是
/// `ITfProperty`（`GUID_PROP_ATTRIBUTE`）值裡的 `TfGuidAtom` 所指的對象。
/// 發佈後不可改，否則已經記住 atom 的應用程式會拿到錯的顯示屬性。
pub const GUID_DISPLAY_ATTRIBUTE_INPUT: GUID =
    GUID::from_u128(0xa2c9f1d4_7b3e_4c58_9e21_5d8a6f0b4c72);

/// 文字服務註冊的語言。第一版只有繁體中文（台灣）。
pub const LANGID_ZH_TW: u16 = 0x0404;

/// profile 的顯示名稱。
pub const PROFILE_DESCRIPTION: &str = "jc 注音";

/// 把 GUID 格式化成註冊機碼用的 `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}`。
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
    fn clsid_key_uses_braces_and_upper_hex() {
        assert_eq!(
            registry_key(&CLSID_JC_TSF),
            "{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}"
        );
    }

    #[test]
    fn profile_key_is_stable() {
        assert_eq!(
            registry_key(&GUID_PROFILE_JC_TSF),
            "{8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}"
        );
    }

    #[test]
    fn display_attribute_key_is_stable() {
        assert_eq!(
            registry_key(&GUID_DISPLAY_ATTRIBUTE_INPUT),
            "{A2C9F1D4-7B3E-4C58-9E21-5D8A6F0B4C72}"
        );
    }
}
