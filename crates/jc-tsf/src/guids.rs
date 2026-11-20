//! 固定的 CLSID 與 profile GUID。
//!
//! 這些值一旦發佈就不能改：註冊機碼、TSF 的 profile 資料庫與使用者的輸入法
//! 設定都以它們為鍵。`docs/port/tsf-skeleton.md` 與 `docs/port/x86-tip.md`
//! 有相同的清單。
//!
//! x64 與 x86 各自一組：TSF 依宿主行程的位元數，到對應的 COM 機碼檢視
//! （64 位元或 `Wow6432Node`）找 `InprocServer32`。兩份 DLL 用不同 CLSID 時，
//! 系統管理員在機碼裡一眼看得出哪個 CLSID 屬於哪個架構，也不會因為同一個
//! CLSID 在兩個檢視指向不同檔案而混淆。代價是使用者的語言清單會有兩個項目，
//! 詳見 `docs/port/x86-tip.md`。

// 兩個架構的常數都編進來；本架構的程式碼只選得到其中一組，另一組由測試
// （`arch_specific_guids_differ`）與文件對照用，所以在這裡關掉 dead_code。
#![allow(dead_code)]

use windows::core::GUID;

/// x64 文字服務物件的 CLSID。
///
/// 註冊機碼為 `HKCR\CLSID\{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}`（64 位元檢視）。
pub const CLSID_JC_TSF_X64: GUID = GUID::from_u128(0x7b7d6e1a_3c1f_4b2e_9a55_6c1d2f3e4a01);

/// x86 文字服務物件的 CLSID。
///
/// 註冊機碼為 `HKCR\Wow6432Node\CLSID\{3E9A5C72-8B14-4D6F-A0C8-5F2B7E9D1346}`
/// （32 位元檢視）。
pub const CLSID_JC_TSF_X86: GUID = GUID::from_u128(0x3e9a5c72_8b14_4d6f_a0c8_5f2b7e9d1346);

/// x64 注音 profile 的 GUID。
///
/// 語言固定為 `0x0404`（zh-TW）。這個 GUID 只在 TSF 的 profile 資料庫裡使用，
/// 不會出現在註冊機碼。
pub const GUID_PROFILE_JC_TSF_X64: GUID = GUID::from_u128(0x8f0a9b3c_2d4e_4f50_a1b2_c3d4e5f60718);

/// x86 注音 profile 的 GUID。
pub const GUID_PROFILE_JC_TSF_X86: GUID = GUID::from_u128(0x6b1f8d43_5a27_4c90_9e36_7d4b2f0a8c51);

/// 這份 DLL 依自身架構選用的 CLSID。
///
/// x64 建置拿到 [`CLSID_JC_TSF_X64`]，i686 建置拿到 [`CLSID_JC_TSF_X86`]。
/// `DllGetClassObject` 只認這一個值，所以 x86 DLL 不會被誤認成 x64 那份。
#[cfg(target_pointer_width = "64")]
pub const CLSID_JC_TSF: GUID = CLSID_JC_TSF_X64;

/// 這份 DLL 依自身架構選用的 CLSID；見 [`CLSID_JC_TSF_X64`] 與 [`CLSID_JC_TSF_X86`]。
#[cfg(target_pointer_width = "32")]
pub const CLSID_JC_TSF: GUID = CLSID_JC_TSF_X86;

/// 這份 DLL 依自身架構選用的 profile GUID。
#[cfg(target_pointer_width = "64")]
pub const GUID_PROFILE_JC_TSF: GUID = GUID_PROFILE_JC_TSF_X64;

/// 這份 DLL 依自身架構選用的 profile GUID。
#[cfg(target_pointer_width = "32")]
pub const GUID_PROFILE_JC_TSF: GUID = GUID_PROFILE_JC_TSF_X86;

/// 候選清單 UI 元素的 GUID。
///
/// `ITfUIElement::GetGUID` 用它識別「這是候選窗」。同一種 UI 元素共用一個
/// GUID，不是每個實例不同；發佈後不可改。兩個架構共用同一個值：這個 GUID
/// 不經過 COM，UI 元素管理器以行程內的指標比對。
pub const GUID_CANDIDATE_UI_ELEMENT: GUID = GUID::from_u128(0x9c4e7a21_6b3d_4f08_b5a7_2e1d8c6f9034);

/// 組字底線的顯示屬性 GUID。
///
/// 這個 GUID 同時是 `ITfDisplayAttributeInfo::GetGUID` 的回傳值，也是
/// `ITfProperty`（`GUID_PROP_ATTRIBUTE`）值裡的 `TfGuidAtom` 所指的對象。
/// 發佈後不可改，否則已經記住 atom 的應用程式會拿到錯的顯示屬性。兩個架構
/// 共用同一個值：`TfGuidAtom` 只在單一行程內有意義。
pub const GUID_DISPLAY_ATTRIBUTE_INPUT: GUID =
    GUID::from_u128(0xa2c9f1d4_7b3e_4c58_9e21_5d8a6f0b4c72);

/// 文字服務註冊的語言。第一版只有繁體中文（台灣）。
pub const LANGID_ZH_TW: u16 = 0x0404;

/// x64 profile 的顯示名稱。語言列與輸入法清單看到的就是這個字串。
pub const PROFILE_DESCRIPTION: &str = "jc 注音";

/// x86 profile 的顯示名稱。
///
/// 兩個架構各自一個 profile，使用者的語言清單會同時出現兩個項目；名稱必須
/// 分得出來，否則使用者無從選擇。實際只有架構相符的那一個能在該程式裡載入。
pub const PROFILE_DESCRIPTION_X86: &str = "jc 注音（32 位元）";

/// 這份 DLL 依自身架構選用的顯示名稱。
#[cfg(target_pointer_width = "64")]
pub const PROFILE_NAME: &str = PROFILE_DESCRIPTION;

/// 這份 DLL 依自身架構選用的顯示名稱。
#[cfg(target_pointer_width = "32")]
pub const PROFILE_NAME: &str = PROFILE_DESCRIPTION_X86;

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
            registry_key(&CLSID_JC_TSF_X64),
            "{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}"
        );
        assert_eq!(
            registry_key(&CLSID_JC_TSF_X86),
            "{3E9A5C72-8B14-4D6F-A0C8-5F2B7E9D1346}"
        );
    }

    #[test]
    fn profile_key_is_stable() {
        assert_eq!(
            registry_key(&GUID_PROFILE_JC_TSF_X64),
            "{8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}"
        );
        assert_eq!(
            registry_key(&GUID_PROFILE_JC_TSF_X86),
            "{6B1F8D43-5A27-4C90-9E36-7D4B2F0A8C51}"
        );
    }

    #[test]
    fn display_attribute_key_is_stable() {
        assert_eq!(
            registry_key(&GUID_DISPLAY_ATTRIBUTE_INPUT),
            "{A2C9F1D4-7B3E-4C58-9E21-5D8A6F0B4C72}"
        );
    }

    /// 兩個架構的 CLSID 與 profile 必須不同；相同就不再是「各自註冊」。
    #[test]
    fn arch_specific_guids_differ() {
        assert_ne!(CLSID_JC_TSF_X64, CLSID_JC_TSF_X86);
        assert_ne!(GUID_PROFILE_JC_TSF_X64, GUID_PROFILE_JC_TSF_X86);
        assert_ne!(
            registry_key(&CLSID_JC_TSF_X64),
            registry_key(&CLSID_JC_TSF_X86)
        );
        assert_ne!(
            registry_key(&GUID_PROFILE_JC_TSF_X64),
            registry_key(&GUID_PROFILE_JC_TSF_X86)
        );
    }

    /// 這份 DLL 選到的必須是它自己架構的那一組。
    #[test]
    fn active_guids_follow_target_arch() {
        if cfg!(target_pointer_width = "64") {
            assert_eq!(CLSID_JC_TSF, CLSID_JC_TSF_X64);
            assert_eq!(GUID_PROFILE_JC_TSF, GUID_PROFILE_JC_TSF_X64);
            assert_eq!(PROFILE_NAME, PROFILE_DESCRIPTION);
        } else {
            assert_eq!(CLSID_JC_TSF, CLSID_JC_TSF_X86);
            assert_eq!(GUID_PROFILE_JC_TSF, GUID_PROFILE_JC_TSF_X86);
            assert_eq!(PROFILE_NAME, PROFILE_DESCRIPTION_X86);
        }
    }

    /// 兩個 profile 的顯示名稱不能一樣，否則使用者的清單無從分辨。
    #[test]
    fn profile_names_differ() {
        assert_ne!(PROFILE_DESCRIPTION, PROFILE_DESCRIPTION_X86);
    }
}
