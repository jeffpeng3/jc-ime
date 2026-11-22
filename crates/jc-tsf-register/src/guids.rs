//! 註冊用的固定 CLSID 與 profile GUID。
//!
//! 這些值一旦發佈就不能改：註冊機碼、TSF 的 profile 資料庫與使用者的輸入法
//! 設定都以它們為鍵。與 `jc-tsf` 的 `guids.rs` 是同一組已發行值，兩邊的測試
//! 各自鎖住數值；改動其中一邊必須同步另一邊。

use windows::core::GUID;

/// 文字服務物件的 CLSID；兩個架構共用。
pub const CLSID_JC_TSF: GUID = GUID::from_u128(0x7b7d6e1a_3c1f_4b2e_9a55_6c1d2f3e4a01);

/// 注音 profile 的 GUID；兩個架構共用。
pub const GUID_PROFILE_JC_TSF: GUID = GUID::from_u128(0x8f0a9b3c_2d4e_4f50_a1b2_c3d4e5f60718);

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
    fn registry_key_uses_braces_and_upper_hex() {
        assert_eq!(
            registry_key(&CLSID_JC_TSF),
            "{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}"
        );
    }
}
