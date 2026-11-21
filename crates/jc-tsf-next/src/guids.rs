//! Zuin 的 COM 識別碼。改了 CLSID 等於換一個輸入法，安裝程式與註冊都要一起改。
use windows_core::GUID;

/// text service 的 CLSID（InprocServer32）。
pub const CLSID_ZUIN: GUID = GUID::from_u128(0x138c35d0_7f08_4cef_ba90_c34c16301528);
/// 唯一的語言 profile：zh-TW 底下的 Mixed。沿用注音版的 GUID，升級不必重新加入輸入法；
/// profile 只是 Windows 的註冊身分，不是引擎的輸入語言（P2-40）。
pub const GUID_PROFILE: GUID = GUID::from_u128(0xef62f743_9855_4c94_a658_1f0f98b7b840);
/// 舊版（T28）ja-JP 的「靈犀日本語」profile GUID。不再註冊，只在註冊與解除安裝時移除。
pub const GUID_PROFILE_JA: GUID = GUID::from_u128(0x3d6a8b21_5c47_4f9e_b0d2_71e4a9c8f653);
/// 組字區的顯示屬性（底線）。
pub const GUID_DISPLAY_ATTR_INPUT: GUID = GUID::from_u128(0xa6ca2572_a961_4492_975f_29a4ebb7c312);

pub const GUID_DISPLAY_ATTR_LOCKED: GUID = GUID::from_u128(0x7a4b5f38_7c42_4315_95b8_6b29f72c5b3b);
pub const GUID_DISPLAY_ATTR_CORRECTED: GUID =
    GUID::from_u128(0x923cdd42_5c7e_4781_8fbe_12232f607742);
/// 加詞標示（#123）。
pub const GUID_DISPLAY_ATTR_MARKED: GUID = GUID::from_u128(0x7f850af6_4e7a_4b41_a24f_baeca02ce683);
/// 中英混打時按空白會組成中文的數字（#162）。
pub const GUID_DISPLAY_ATTR_READING: GUID = GUID::from_u128(0xcbedd798_5d14_4f4d_9917_26b934dfa003);
/// 日文変換中的一般文節（T15）。
pub const GUID_DISPLAY_ATTR_CLAUSE: GUID = GUID::from_u128(0x08933a6a_3bfa_4550_9423_15da82013a11);
/// 日文変換中目前的文節（T15）。
pub const GUID_DISPLAY_ATTR_CLAUSE_ACTIVE: GUID =
    GUID::from_u128(0x4f4e536f_ad3e_420b_9417_59e8bf791aa1);

/// Mixed profile 的宿主語言 ID。Windows 要求 profile 有語言；內容是混合語言，
/// 由引擎處理，不是這個語言 ID 決定的。
pub const LANGID: u16 = 0x0404;
/// 舊版（T28）ja-JP profile 的語言 ID；只用來移除。
pub const LANGID_JA: u16 = 0x0411;
pub const DESCRIPTION: &str = "Mixed";
