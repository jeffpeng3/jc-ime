//! 符號選單、表情選單與「最近使用」的行為。
//!
//! 選單狀態（[`Session`] 的 `menu`）與候選重建仍在主狀態機；這裡放選單自己
//! 的規則：分頁提示列與上游 `symbolGroups` 的同組符號表。
//!
//! 符號目錄本身在 [`crate::symbols`]。本模組同名，因此把呼叫端用到的目錄
//! 項目重新匯出，讓 `super::core` 維持 `symbols::TABS` 這種寫法。

use super::*;

pub(super) use crate::symbols::{
    clean_recent, contains, tab_items, width_variant, MAX_RECENT, RECENT, TABS,
};

/// 表情分頁的顯示名稱（上游 `paletteNames`）。
///
/// 提示列寫給使用者看，用的是顯示名稱而不是 `kind`。
const PALETTE_NAMES: [(&str, &str); 2] = [("emoji", "Emoji"), ("kaomoji", "顏文字")];

/// 同組符號：在同一組裡用 ↓ 互換（上游 `symbolGroups`）。
///
/// 每一組是一個實體鍵或一類標點的變體。同一符號出現在多組時以最後一組
/// 為它的本組，`↓` 列本組；反引號的符號選單另有分類目錄（[`TABS`]），
/// 這裡的每個符號都要列在選單的某個分頁裡。
///
/// 左右括號分開列，避免把開括號換成閉括號；刪節號、破折號、箭頭各成一組；
/// `^`、`_` 兩組最後附半形原字元，讓中文模式要打半形的人換得回來；
/// 斜線與問號共用一個實體鍵，最後一組是注音符號與聲調。
///
/// 目前只有資料與查詢，實際的 `↓` 換字還沒移植（見
/// `docs/port/session-g3-symbols.md` 的 deferred 清單）。
pub const GROUPS: &[&[&str]] = &[
    &["「", "『", "﹁", "﹃", "“", "‘"],
    &["」", "』", "﹂", "﹄", "”", "’"],
    &["（", "〔", "【", "〈", "《", "［", "｛"],
    &["）", "〕", "】", "〉", "》", "］", "｝"],
    &["，", "、", "﹐", "､"],
    &["。", "．", "｡", "⋯", "…"],
    &["？", "！", "﹖", "﹗", "／", "/"],
    &["：", "；", "﹕", "﹔"],
    &["～", "〜", "—", "–", "－"],
    &["＆", "&", "﹠"],
    &["……", "⋯⋯", "…", "⋯", "︿", "^"],
    &["——", "—", "―", "─", "－", "＿", "←", "→", "_"],
    &["←", "→", "↑", "↓"],
    &[
        "ㄅ", "ㄆ", "ㄇ", "ㄈ", "ㄉ", "ㄊ", "ㄋ", "ㄌ", "ㄍ", "ㄎ", "ㄏ", "ㄐ", "ㄑ", "ㄒ", "ㄓ",
        "ㄔ", "ㄕ", "ㄖ", "ㄗ", "ㄘ", "ㄙ", "ㄧ", "ㄨ", "ㄩ", "ㄚ", "ㄛ", "ㄜ", "ㄝ", "ㄞ", "ㄟ",
        "ㄠ", "ㄡ", "ㄢ", "ㄣ", "ㄤ", "ㄥ", "ㄦ", "ˉ", "ˊ", "ˇ", "ˋ", "˙",
    ],
];

/// 多字元的符號：在組字區裡像一個字一樣移動、刪除，按 ↓ 整個換掉。
///
/// 由 [`GROUPS`] 推導而來，只有刪節號與破折號兩類。
pub const UNITS: &[&str] = &["……", "⋯⋯", "——"];

/// 回傳符號的本組序號；不在任何組時回傳 `None`。
///
/// 同時屬於多組時看最後一組，與 `↓` 列出的組一致。
pub fn home(item: &str) -> Option<usize> {
    GROUPS.iter().rposition(|group| group.contains(&item))
}

/// 回傳符號的整組變體，第一項是目前符號；不在任何組時只有它自己。
pub fn alternatives(current: &str) -> Vec<String> {
    let mut out = vec![current.to_string()];
    let Some(index) = home(current) else {
        return out;
    };
    for item in GROUPS[index] {
        if *item != current {
            out.push((*item).to_string());
        }
    }
    out
}

impl Session {
    /// 選單的提示列；沒有開選單時回傳 `None`。
    ///
    /// 符號選單列出目前分頁與 Tab 的下一個分頁（`標點 · Tab → 括號引號`），
    /// 因為 ↑ 定位會直接開在某個分類。表情選單只列下一個分頁
    /// （`Tab → 顏文字`），與上游 `symbolTitle`／`paletteTitle` 相同。
    pub fn menu_title(&self) -> Option<String> {
        let menu = self.menu?;
        let names = self.menu_tab_names(menu.kind);
        if names.is_empty() {
            return None;
        }
        let at = menu.tab.min(names.len() - 1);
        let next = names[(at + 1) % names.len()];
        Some(match menu.kind {
            MenuKind::Symbols => format!("{} · Tab → {}", names[at], next),
            MenuKind::Emoji => {
                format!("Tab → {}", palette_name(next))
            }
        })
    }
}

/// 分頁 `kind` 的顯示名稱；未知名稱照原樣回傳。
fn palette_name(kind: &str) -> &str {
    PALETTE_NAMES
        .iter()
        .find(|(name, _)| *name == kind)
        .map(|(_, label)| *label)
        .unwrap_or(kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn session() -> Session {
        Session::new(SessionConfig::new(Mode::Zh))
    }

    /// 對照 `TestSymbolMenuTabs`／`TestSymbolMenuRecentTab` 的標題斷言：
    /// 符號選單列目前分頁與下一個分頁，頭尾相接。
    #[test]
    fn test_symbol_menu_titles() {
        let mut s = session();
        assert_eq!(s.menu_title(), None, "沒有選單就沒有提示列");

        s.press(Key::char('`', false));
        assert_eq!(s.menu_title().as_deref(), Some("標點 · Tab → 括號引號"));

        s.press(Key::new(KeyCode::Tab, None, false));
        assert_eq!(s.menu_title().as_deref(), Some("括號引號 · Tab → 數學"));

        for _ in 0..5 {
            s.press(Key::new(KeyCode::Tab, None, false));
        }
        assert_eq!(s.menu_tab(), Some("注音"));
        assert_eq!(
            s.menu_title().as_deref(),
            Some("注音 · Tab → 標點"),
            "頭尾相接"
        );

        s.press(Key::new(KeyCode::Tab, None, true));
        assert_eq!(s.menu_title().as_deref(), Some("編號 · Tab → 注音"));

        // 有最近使用時從「最近」開，下一個分頁是標點。
        s.press(Key::new(KeyCode::Escape, None, false));
        s.press(Key::char('`', false));
        s.press(Key::char('1', false));
        assert_eq!(s.commit(), "，");
        s.press(Key::char('`', false));
        assert_eq!(s.menu_tab(), Some(RECENT));
        assert_eq!(s.menu_title().as_deref(), Some("最近 · Tab → 標點"));
    }

    /// 對照 `TestPaletteDirectSelectionAndEscapes`：表情提示列只有下一個分頁。
    #[test]
    fn test_palette_titles() {
        let mut s = session();
        s.press(Key::char('=', false));
        assert_eq!(s.menu_tab(), Some("emoji"));
        assert_eq!(s.menu_title().as_deref(), Some("Tab → 顏文字"));

        s.press(Key::new(KeyCode::Tab, None, false));
        assert_eq!(s.menu_tab(), Some("kaomoji"));
        assert_eq!(s.menu_title().as_deref(), Some("Tab → Emoji"));

        s.press(Key::new(KeyCode::Tab, None, true));
        assert_eq!(s.menu_title().as_deref(), Some("Tab → 顏文字"));
    }

    /// 對照 `TestSymbolMenuKeepsEveryGroupSymbol`：↓ 各組的每個符號都要在
    /// 選單的某個分頁裡，否則換得到卻叫不回來。
    #[test]
    fn test_symbol_groups_cover_menu_catalog() {
        for group in GROUPS {
            assert!(!group.is_empty(), "空的分組");
            for item in *group {
                assert!(!item.is_empty(), "空符號");
                assert!(contains(item), "符號選單少了 {item:?}");
            }
        }
    }

    /// 上游 `symbolHome` 以最後一組為本組；`alternatives` 第一項是原符號。
    #[test]
    fn test_symbol_groups_home_and_alternatives() {
        assert_eq!(home("，"), Some(4));
        assert_eq!(alternatives("，"), ["，", "、", "﹐", "､"]);

        // ← 與 → 同時在破折號組與箭頭組，本組是最後一組。
        assert_eq!(home("→"), Some(12));
        assert_eq!(alternatives("→"), ["→", "←", "↑", "↓"]);
        assert_eq!(alternatives("↑"), ["↑", "←", "→", "↓"]);

        // 「—」只在破折號組。
        assert_eq!(home("—"), Some(11));
        assert!(alternatives("—").iter().any(|item| item == "＿"));

        // 不在分組表裡的符號沒有其他變體，只有它自己。
        assert_eq!(home("我"), None);
        assert_eq!(alternatives("我"), ["我"]);
    }

    #[test]
    fn test_symbol_units_are_multichar() {
        let mut want: Vec<&str> = Vec::new();
        for group in GROUPS {
            for item in *group {
                if item.chars().count() > 1 && !want.contains(item) {
                    want.push(item);
                }
            }
        }
        assert_eq!(want, UNITS);
        assert!(UNITS.iter().all(|unit| unit.chars().count() > 1));
        assert!(UNITS.iter().all(|unit| contains(unit)));
    }

    /// 對照 `TestSymbolMenuRecentTab` 的上限：最近只放一頁，第 10 個之後
    /// 就擠掉舊的；重複選同一個不佔兩格。
    #[test]
    fn test_symbol_recent_is_one_page() {
        let mut s = session();
        // 標點分頁前 9 項各選一次；頁面固定 9 項，`0` 不是選字鍵。
        for digit in "123456789".chars() {
            s.open_symbol_tab("標點");
            s.press(Key::char(digit, false));
            assert!(!s.commit().is_empty());
        }
        assert_eq!(s.symbol_recent().len(), PAGE_SIZE);

        // 再選第 10、11 項：超過上限就淘汰。
        for index in [PAGE_SIZE, PAGE_SIZE + 1] {
            s.open_symbol_tab("標點");
            s.page(1);
            s.select(index);
            assert!(!s.commit().is_empty());
        }
        assert_eq!(s.symbol_recent().len(), MAX_RECENT);
        assert_eq!(MAX_RECENT, PAGE_SIZE, "最近剛好一頁");
        let unique: HashSet<&String> = s.symbol_recent().iter().collect();
        assert_eq!(unique.len(), MAX_RECENT, "不重複");

        let snap = s.press(Key::char('`', false));
        assert_eq!(s.menu_tab(), Some(RECENT));
        assert_eq!(snap.candidates.len(), MAX_RECENT, "最近分頁只放一頁");
        assert_eq!(s.menu_title().as_deref(), Some("最近 · Tab → 標點"));
    }
}
