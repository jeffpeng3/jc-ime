//! 反引號（`）符號選單的分類目錄與「最近」分頁清理規則。
//!
//! 只有手選的 Unicode 字串，沒有讀音、詞庫或語言模型關係。分類參考開源
//! 輸入法的符號表：新酷音 libchewing 的 `symbols.dat`、小麥注音系 vChewing
//! 的符號選單與 RIME rime-prelude 的 `symbols.yaml`。分頁不求齊全：只收中文
//! 書寫常用、各平台字型都畫得出來的符號，每個符號只列在一個分頁，這樣 ↑
//! 叫出選單時能定位到它所在的分頁。
//!
//! 另外收錄前身的全形英數規則（`core/ja/mixed/width.go`），供候選清單提供
//! `0-9` → `０-９` 這類寬度變體。

/// 一個符號分頁。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tab {
    /// 分頁名稱。
    pub name: &'static str,
    /// 分頁內的符號，依顯示順序。
    pub items: &'static [&'static str],
}

/// 第一個分頁的名稱：最近、常用的符號，由個人資料排序，不在 [`TABS`] 裡。
pub const RECENT: &str = "最近";

/// 「最近」分頁保存的符號數，剛好一頁。
pub const MAX_RECENT: usize = crate::candidate::PAGE_SIZE;

/// 符號候選 ID 的起點，讓它與其他候選分開；ID 不隨最近順序或分頁改變。
pub const ID_BASE: usize = 3 << 20;

/// 依序是「最近」之後的固定分頁。每頁第一頁（前 10 個）放最常用的。
pub const TABS: &[Tab] = &[
    Tab {
        name: "標點",
        items: &[
            "，", "、", "。", "？", "！", "：", "；", "……", "——", "～", "．", "‧", "·", "…", "⋯⋯",
            "⋯", "—", "―", "─", "–", "〜", "＿", "／", "＼", "｜", "＆", "＠", "＃", "＊", "〃",
            "﹐", "､", "｡", "﹖", "﹗", "﹕", "﹔", "﹠", "/", "&", "^", "_",
        ],
    },
    Tab {
        name: "括號引號",
        items: &[
            "「", "」", "『", "』", "（", "）", "【", "】", "《", "》", "〈", "〉", "〔", "〕",
            "［", "］", "｛", "｝", "“", "”", "‘", "’", "﹁", "﹂", "﹃", "﹄", "〖", "〗", "﹙",
            "﹚", "﹝", "﹞", "﹛", "﹜", "︵", "︶", "︹", "︺", "︷", "︸", "︻", "︼", "︽",
            "︾", "︿", "﹀",
        ],
    },
    Tab {
        name: "數學",
        items: &[
            "＋", "－", "×", "÷", "＝", "≠", "≒", "≈", "±", "√", "＜", "＞", "≦", "≧", "∞", "∴",
            "∵", "∠", "⊥", "∟", "∫", "∮", "∑", "∏", "∩", "∪", "∈", "⊿", "≡", "π", "°", "℃", "℉",
            "％", "‰", "½", "¼", "¾", "′", "″", "＄", "￥", "€", "￡", "￠",
        ],
    },
    Tab {
        name: "箭頭",
        items: &[
            "←", "→", "↑", "↓", "↖", "↗", "↘", "↙", "↔", "↕", "⇐", "⇒", "⇑", "⇓", "⇔", "⇕", "↩",
            "↪", "↺", "↻", "⇦", "⇨", "⇧", "⇩", "➔", "➤",
        ],
    },
    Tab {
        name: "圖形",
        items: &[
            "※", "★", "☆", "○", "●", "◎", "◇", "◆", "□", "■", "△", "▲", "▽", "▼", "✓", "✗", "☑",
            "☐", "♀", "♂", "♠", "♥", "♣", "♦", "♤", "♡", "♧", "♢", "♪", "♫", "♬", "☀", "☁", "☂",
            "☎", "§", "¶", "©", "®", "™", "㊣", "〒", "№", "℡",
        ],
    },
    Tab {
        name: "編號",
        items: &[
            "①", "②", "③", "④", "⑤", "⑥", "⑦", "⑧", "⑨", "⑩", "⑪", "⑫", "⑬", "⑭", "⑮", "⑯", "⑰",
            "⑱", "⑲", "⑳", "⑴", "⑵", "⑶", "⑷", "⑸", "⑹", "⑺", "⑻", "⑼", "⑽", "㈠", "㈡", "㈢",
            "㈣", "㈤", "㈥", "㈦", "㈧", "㈨", "㈩", "Ⅰ", "Ⅱ", "Ⅲ", "Ⅳ", "Ⅴ", "Ⅵ", "Ⅶ", "Ⅷ", "Ⅸ",
            "Ⅹ", "Ⅺ", "Ⅻ", "ⅰ", "ⅱ", "ⅲ", "ⅳ", "ⅴ", "ⅵ", "ⅶ", "ⅷ", "ⅸ", "ⅹ",
        ],
    },
    Tab {
        name: "注音",
        items: &[
            "ㄅ", "ㄆ", "ㄇ", "ㄈ", "ㄉ", "ㄊ", "ㄋ", "ㄌ", "ㄍ", "ㄎ", "ㄏ", "ㄐ", "ㄑ", "ㄒ",
            "ㄓ", "ㄔ", "ㄕ", "ㄖ", "ㄗ", "ㄘ", "ㄙ", "ㄧ", "ㄨ", "ㄩ", "ㄚ", "ㄛ", "ㄜ", "ㄝ",
            "ㄞ", "ㄟ", "ㄠ", "ㄡ", "ㄢ", "ㄣ", "ㄤ", "ㄥ", "ㄦ", "ˉ", "ˊ", "ˇ", "ˋ", "˙",
        ],
    },
];

/// 回傳符號所在的分頁與位置；不在目錄裡時回傳 `None`。
pub fn locate(text: &str) -> Option<(usize, usize)> {
    for (tab, entry) in TABS.iter().enumerate() {
        if let Some(index) = entry.items.iter().position(|item| *item == text) {
            return Some((tab, index));
        }
    }
    None
}

/// 回報 `text` 是不是選單裡的符號。
pub fn contains(text: &str) -> bool {
    locate(text).is_some()
}

/// 符號的穩定候選 ID：`ID_BASE` 加上它在所有固定分頁裡的序號。
///
/// 不在目錄裡時回傳 `None`。
pub fn id(text: &str) -> Option<usize> {
    let mut offset = 0;
    for tab in TABS {
        if let Some(index) = tab.items.iter().position(|item| *item == text) {
            return Some(ID_BASE + offset + index);
        }
        offset += tab.items.len();
    }
    None
}

/// 指定名稱的分頁內容；名稱不存在時回傳 `None`。
pub fn tab_items(name: &str) -> Option<&'static [&'static str]> {
    TABS.iter()
        .find(|tab| tab.name == name)
        .map(|tab| tab.items)
}

/// 丟掉不在目錄裡的與重複的項目，最多 [`MAX_RECENT`] 項。
///
/// 不排序、不保存任意文字。回傳值不與目錄或輸入共用。
pub fn clean_recent(recent: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for text in recent {
        if contains(text) && !out.iter().any(|item| item == text) {
            out.push(text.clone());
            if out.len() == MAX_RECENT {
                break;
            }
        }
    }
    out
}

/// 回報 `ch` 是否為全形英數字。
pub fn is_full_width_alnum(ch: char) -> bool {
    matches!(ch, '０'..='９' | 'Ａ'..='Ｚ' | 'ａ'..='ｚ')
}

/// 回報 `text` 是否含全形英數字。
pub fn has_full_width_alnum(text: &str) -> bool {
    text.chars().any(is_full_width_alnum)
}

/// 回報 `text` 是否含 ASCII 英數字。
pub fn has_ascii_alnum(text: &str) -> bool {
    text.chars()
        .any(|ch| ch.is_ascii_digit() || ch.is_ascii_alphabetic())
}

/// 把全形英數字轉成半形，其他字元（含全形標點）保持原樣。
pub fn to_half_width_latin(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '０'..='９' => char::from_u32(ch as u32 - ('０' as u32 - '0' as u32)).unwrap_or(ch),
            'Ａ'..='Ｚ' => char::from_u32(ch as u32 - ('Ａ' as u32 - 'A' as u32)).unwrap_or(ch),
            'ａ'..='ｚ' => char::from_u32(ch as u32 - ('ａ' as u32 - 'a' as u32)).unwrap_or(ch),
            _ => ch,
        })
        .collect()
}

/// 把 ASCII 英數字轉成全形，其他字元保持原樣。
pub fn to_full_width_latin(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '0'..='9' => char::from_u32(ch as u32 + ('０' as u32 - '0' as u32)).unwrap_or(ch),
            'A'..='Z' => char::from_u32(ch as u32 + ('Ａ' as u32 - 'A' as u32)).unwrap_or(ch),
            'a'..='z' => char::from_u32(ch as u32 + ('ａ' as u32 - 'a' as u32)).unwrap_or(ch),
            _ => ch,
        })
        .collect()
}

/// 回傳 `text` 的另一種英數字寬度；沒有英數字時回傳 `None`。
///
/// 全形與半角混用時回傳全形（和日文輸入的預設一致），全部全形時回傳半形。
pub fn width_variant(text: &str) -> Option<String> {
    let has_full = has_full_width_alnum(text);
    let has_ascii = has_ascii_alnum(text);
    if !has_full && !has_ascii {
        return None;
    }
    Some(if has_full && has_ascii {
        to_full_width_latin(text)
    } else if has_full {
        to_half_width_latin(text)
    } else {
        to_full_width_latin(text)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    /// 每個符號只列在一個分頁（↑ 才定位得到），ID 不重複；分頁名稱不與「最近」相同。
    #[test]
    fn test_tabs_unique_and_ids_stable() {
        let mut seen: HashMap<&str, &str> = HashMap::new();
        let mut ids: HashMap<usize, &str> = HashMap::new();
        for tab in TABS {
            assert_ne!(tab.name, RECENT, "分頁名稱");
            assert!(!tab.items.is_empty(), "分頁 {}", tab.name);
            for (index, item) in tab.items.iter().enumerate() {
                if let Some(other) = seen.get(item) {
                    panic!("{item:?} 同時在 {other} 與 {}", tab.name);
                }
                seen.insert(item, tab.name);
                assert!(!item.is_empty(), "{} 第 {index} 項", tab.name);
                let id = id(item).expect("目錄裡的符號應有 ID");
                assert!(id >= ID_BASE, "{item:?} 的 ID {id}");
                assert!(!ids.contains_key(&id), "{item:?} 的 ID {id} 重複");
                ids.insert(id, item);
                assert_eq!(
                    locate(item),
                    Some((TABS.iter().position(|t| t.name == tab.name).unwrap(), index)),
                    "locate({item:?})"
                );
            }
        }
        assert_eq!(locate("任意文字"), None);
        assert!(!contains("任意文字"));
    }

    #[test]
    fn test_clean_recent() {
        let input = vec![
            "※".to_string(),
            "x".to_string(),
            "※".to_string(),
            "，".to_string(),
        ];
        let out = clean_recent(&input);
        assert_eq!(out, vec!["※".to_string(), "，".to_string()]);
        let mut changed = out.clone();
        changed[0] = "改".to_string();
        assert_eq!(input[0], "※", "不與輸入共用");

        let many: Vec<String> = TABS
            .iter()
            .flat_map(|tab| tab.items.iter().map(|item| item.to_string()))
            .collect();
        assert_eq!(clean_recent(&many).len(), MAX_RECENT, "有上限");
    }

    #[test]
    fn test_width_conversion() {
        let cases: &[(&str, &str, &str)] = &[
            ("ＡＢＣ１２３", "ABC123", "ＡＢＣ１２３"),
            ("API123", "API123", "ＡＰＩ１２３"),
            ("ＡＢc１２", "ABc12", "ＡＢｃ１２"),
            ("今日はＡＰＩ", "今日はAPI", "今日はＡＰＩ"),
        ];
        for &(input, half, full) in cases {
            assert_eq!(
                to_half_width_latin(input),
                half,
                "ToHalfWidthLatin({input:?})"
            );
            assert_eq!(
                to_full_width_latin(input),
                full,
                "ToFullWidthLatin({input:?})"
            );
        }
    }

    #[test]
    fn test_width_variant() {
        let cases: &[(&str, Option<&str>)] = &[
            ("API", Some("ＡＰＩ")),
            ("ＡＰＩ", Some("API")),
            ("API１２", Some("ＡＰＩ１２")),
            ("今日は", None),
            ("", None),
        ];
        for &(input, want) in cases {
            assert_eq!(
                width_variant(input).as_deref(),
                want,
                "WidthVariant({input:?})"
            );
        }
    }

    #[test]
    fn test_width_detectors() {
        assert!(has_ascii_alnum("abc"));
        assert!(!has_ascii_alnum("今日"));
        assert!(has_full_width_alnum("ＡＢＣ"));
        assert!(!has_full_width_alnum("ABC"));
        for ch in "ＡＺａｚ０９".chars() {
            assert!(is_full_width_alnum(ch), "IsFullWidthAlnum({ch:?})");
        }
        assert!(!is_full_width_alnum('A'));
        assert!(!is_full_width_alnum('日'));
    }

    /// 資料檔是手動整理的日文記号表；缺檔時 skip，存在時驗證格式與首列。
    #[test]
    fn test_symbols_data_rows_are_well_formed() {
        let path = jc_testdata::require_data!("ja-src/symbols/symbols.tsv");
        let text = std::fs::read_to_string(&path).expect("讀取 symbols.tsv");
        let rows: Vec<&str> = text
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect();
        assert!(!rows.is_empty(), "symbols.tsv 沒有資料列");
        for row in &rows {
            assert_eq!(row.split('\t').count(), 4, "欄位數：{row:?}");
        }
        assert_eq!(
            rows[0].split('\t').collect::<Vec<_>>(),
            ["きごう", "※", "米印", "記号"]
        );
        let seen: HashSet<&str> = rows
            .iter()
            .filter_map(|row| row.split('\t').nth(1))
            .collect();
        assert!(seen.contains("→"), "symbols.tsv 應有箭頭");
    }
}
