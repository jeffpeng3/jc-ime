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

// ---- T9：平行建置的新身分 ----
//
// 這一組是 jc 自己的全新值，不是上面的 0c／Zuin 識別碼，也不是 jc-tsf 已發行值。
// 新版 DLL 與舊版同時註冊時，CLSID、profile、描述、pipe 前綴、個人資料檔名前綴
// 與監督者實例名各自獨立；新舊共用同一份資料格式，回退不必搬語料。

/// 新版文字服務的 CLSID。與 `jc-tsf` 的 `CLSID_JC_TSF` 不同，可同時註冊。
pub const CLSID_JC_TSF_NEXT: GUID = GUID::from_u128(0x2f6b41d8_9e07_4c3a_b5f1_8d2c6a9e0473);

/// 新版 profile 的 GUID。`AddLanguageProfile` 用它，與舊 profile 並存。
pub const GUID_PROFILE_JC_TSF_NEXT: GUID = GUID::from_u128(0xc4e8d217_5b3f_4a96_8e0c_7f1a2b3d4e56);

/// 新版的顯示名稱。必須與舊版不同，否則系統輸入法清單出現兩項同名。
pub const PROFILE_DESCRIPTION_NEXT: &str = "jc 注音（新版）";

/// 新版引擎的 pipe 前綴，解析後是 `jc-ime-next-<SID 尾段>`。
pub const PIPE_PREFIX_NEXT: &str = "jc-ime-next";

/// 舊版已發行的 pipe 前綴。只用於相容檢查與文件，新版連線不使用。
pub const PIPE_PREFIX_OLD: &str = "jc-ime";

/// 新版個人學習檔名前綴，存成 `personal-next-<id>.json`，與舊版 `personal-<id>.json` 隔離。
pub const PERSONAL_FILE_PREFIX_NEXT: &str = "personal-next";

/// 新版引擎監督者的實例名，用來產生獨立的鎖與 log 檔名。
pub const SUPERVISOR_INSTANCE_NEXT: &str = "next";

/// 新版 TIP 專用的 pipe 覆寫變數。
///
/// 刻意不讀 `JC_IME_PIPE`：舊版引擎的覆寫值不該讓新版 TIP 連到舊 pipe。新版引擎
/// 用 `--socket jc-ime-next-<rid>` 或這個環境變數對齊。
pub const NEXT_PIPE_ENV: &str = "JC_IME_NEXT_PIPE";

/// 依 SID 尾段推出新版引擎的 pipe 名稱，不含 `\\.\pipe\`。
pub fn next_engine_pipe(rid: &str) -> String {
    format!("{PIPE_PREFIX_NEXT}-{rid}")
}

#[cfg(test)]
mod t9_tests {
    use super::*;

    /// 新版身分不能與 jc-tsf 已發行值相同；舊值在這裡重複一份只是為了鎖住這件事。
    #[test]
    fn parallel_identity_does_not_reuse_published_values() {
        let old_clsid = GUID::from_u128(0x7b7d6e1a_3c1f_4b2e_9a55_6c1d2f3e4a01);
        let old_profile = GUID::from_u128(0x8f0a9b3c_2d4e_4f50_a1b2_c3d4e5f60718);
        assert_ne!(CLSID_JC_TSF_NEXT, old_clsid);
        assert_ne!(GUID_PROFILE_JC_TSF_NEXT, old_profile);
        assert_ne!(PIPE_PREFIX_NEXT, PIPE_PREFIX_OLD);
        assert_ne!(PROFILE_DESCRIPTION_NEXT, "jc 注音");
        assert_eq!(next_engine_pipe("1001"), "jc-ime-next-1001");
    }
}
