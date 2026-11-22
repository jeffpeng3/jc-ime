//! Zuin 的 COM 識別碼。改了 CLSID 等於換一個輸入法，安裝程式與註冊都要一起改。
use windows_core::GUID;

use crate::log::zlog;

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

// ---- 現行身分 ----
//
// 舊版 DLL 已在下一次 rc 移除。下面這一組是 rc.15 已發行的新值，GUID 保留不改，
// 只把描述從「jc 注音（新版）」收斂成「jc 注音」，pipe 與個人資料前綴也去掉 next。
// 已移除的舊值留在 *_LEGACY 常數，只供文件與測試對照，不再註冊。

/// 現行文字服務的 CLSID（InprocServer32）。rc.15 已發行值，保留不改。
pub const CLSID_JC_TSF: GUID = GUID::from_u128(0x2f6b41d8_9e07_4c3a_b5f1_8d2c6a9e0473);

/// 現行注音 profile 的 GUID。rc.15 已發行值，保留不改。
pub const GUID_PROFILE_JC_TSF: GUID = GUID::from_u128(0xc4e8d217_5b3f_4a96_8e0c_7f1a2b3d4e56);

/// 已移除的舊 CLSID；只留給文件與測試對照，不再註冊。
pub const CLSID_JC_TSF_LEGACY: GUID = GUID::from_u128(0x7b7d6e1a_3c1f_4b2e_9a55_6c1d2f3e4a01);

/// 已移除的舊 profile GUID；只留給文件與測試對照，不再註冊。
pub const GUID_PROFILE_JC_TSF_LEGACY: GUID =
    GUID::from_u128(0x8f0a9b3c_2d4e_4f50_a1b2_c3d4e5f60718);

/// 顯示名稱。rc.15 的「jc 注音（新版）」已收斂成單一「jc 注音」。
pub const PROFILE_DESCRIPTION: &str = "jc 注音";

/// 引擎 pipe 前綴，解析後是 `jc-ime-<SID 尾段>`。
pub const PIPE_PREFIX: &str = "jc-ime";

/// 個人學習檔名前綴，存成 `personal.json`。
pub const PERSONAL_FILE_PREFIX: &str = "personal";

/// TIP 讀取的 pipe 覆寫變數；引擎的 `--socket` 或這個環境變數要與它對齊。
pub const PIPE_ENV: &str = "JC_IME_PIPE";

/// 依 SID 尾段推出引擎的 pipe 名稱，不含 `\\.\pipe\`。
pub fn engine_pipe(rid: &str) -> String {
    let name = format!("{PIPE_PREFIX}-{rid}");
    zlog!("engine_pipe: rid={rid} name={name}");
    name
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    /// 現行身分不能與已移除的舊值相同；舊值在這裡重複一份只是為了鎖住這件事。
    #[test]
    fn current_identity_does_not_reuse_legacy_values() {
        assert_ne!(CLSID_JC_TSF, CLSID_JC_TSF_LEGACY);
        assert_ne!(GUID_PROFILE_JC_TSF, GUID_PROFILE_JC_TSF_LEGACY);
        assert_eq!(PROFILE_DESCRIPTION, "jc 注音");
        assert_eq!(PIPE_PREFIX, "jc-ime");
        assert_eq!(PERSONAL_FILE_PREFIX, "personal");
        assert_eq!(engine_pipe("1001"), "jc-ime-1001");
    }
}
