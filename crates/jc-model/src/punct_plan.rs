//! 標點模型的共用視窗與後處理規則。
//!
//! 這裡只放純函式：滑動視窗起點、視窗合併、argmax 與字元判定。ONNX 推論
//! 在 [`crate::punctuation_model`] 與 [`crate::punct_zh`]。規則照前身
//! `internal/neural/native/punct_plan.go`。

#![cfg_attr(not(feature = "onnx"), allow(dead_code))]

/// 可插入的標點。
pub(crate) const MARKS: &str = "，。？！、；";

/// 要插入的標點與位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Edit {
    /// 插入位置，以 rune 計算。
    pub offset: usize,
    /// 要插入的標點。
    pub text: String,
}

impl Edit {
    /// 建立插入編輯。
    pub(crate) fn new(offset: usize, text: impl Into<String>) -> Self {
        Self {
            offset,
            text: text.into(),
        }
    }
}

/// 驗證後插入：位置嚴格遞增、只用 [`MARKS`]、不插在開頭。
///
/// `han_only` 為真時還要求插入點兩側都是漢字；`allow_end` 為真時允許插在
/// 段落尾端。任一編輯不合規就整批不套用。
pub(crate) fn apply(text: &str, edits: &[Edit], allow_end: bool, han_only: bool) -> (String, bool) {
    if edits.is_empty() {
        return (text.to_string(), false);
    }
    let runes: Vec<char> = text.chars().collect();
    let mut last = 0;
    for edit in edits {
        if edit.offset <= last
            || !allowed(&edit.text)
            || edit.offset > runes.len()
            || (!allow_end && edit.offset == runes.len())
        {
            return (text.to_string(), false);
        }
        if han_only && (!is_han(runes[edit.offset - 1]) || !is_han(runes[edit.offset])) {
            return (text.to_string(), false);
        }
        last = edit.offset;
    }
    let mut out: Vec<char> = Vec::with_capacity(runes.len() + edits.len());
    let mut at = 0;
    for (index, ch) in runes.iter().enumerate() {
        if at < edits.len() && edits[at].offset == index {
            out.extend(edits[at].text.chars());
            at += 1;
        }
        out.push(*ch);
    }
    while at < edits.len() && edits[at].offset == runes.len() {
        out.extend(edits[at].text.chars());
        at += 1;
    }
    (out.into_iter().collect(), true)
}

fn allowed(mark: &str) -> bool {
    let mut chars = mark.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => MARKS.contains(ch),
        _ => false,
    }
}

/// 滑動視窗起點：文字不超過視窗時只有一個 `0`，否則依 stride 前進，最後
/// 一個視窗貼齊結尾。
pub(crate) fn window_starts(n: usize, window: usize, stride: usize) -> Vec<usize> {
    if n == 0 || window == 0 {
        return Vec::new();
    }
    if n <= window {
        return vec![0];
    }
    let stride = stride.max(1);
    let mut out = Vec::with_capacity((n - window) / stride + 2);
    let mut start = 0;
    while start < n - window {
        out.push(start);
        start += stride;
    }
    out.push(n - window);
    out
}

/// 每個位置取「離視窗邊緣最遠」的那一列；距離相同時保留先出現的視窗。
///
/// `rows[i][j]` 是第 `i` 個視窗第 `j` 個位置的分數列。缺漏的位置回傳
/// `None`。
pub(crate) fn merge_window_rows(
    n: usize,
    window: usize,
    starts: &[usize],
    rows: &[Vec<Vec<f32>>],
) -> Vec<Option<Vec<f32>>> {
    let mut out: Vec<Option<Vec<f32>>> = vec![None; n];
    let mut best = vec![-1i32; n];
    for (wi, &start) in starts.iter().enumerate() {
        let Some(window_rows) = rows.get(wi) else {
            break;
        };
        let m = window_rows.len();
        for (j, row) in window_rows.iter().enumerate() {
            let Some(at) = start.checked_add(j) else {
                break;
            };
            if at >= n {
                break;
            }
            let margin = if n > window {
                j.min(m.saturating_sub(1).saturating_sub(j)) as i32
            } else {
                0
            };
            if margin > best[at] {
                best[at] = margin;
                out[at] = Some(row.clone());
            }
        }
    }
    out
}

/// 分數列中最大值的索引；空列回傳 0。
pub(crate) fn argmax(row: &[f32]) -> usize {
    let mut best = 0;
    for (i, value) in row.iter().enumerate().skip(1) {
        if *value > row[best] {
            best = i;
        }
    }
    best
}

/// 字元是否為漢字（含擴充 A 與相容表意文字區）。
pub(crate) fn is_han(ch: char) -> bool {
    ('\u{3400}'..='\u{9FFF}').contains(&ch)
        || ('\u{20000}'..='\u{323AF}').contains(&ch)
        || ('\u{F900}'..='\u{FAFF}').contains(&ch)
}

/// 字元是否為不需要在旁邊再補標點的既有標點。
pub(crate) fn is_punct(ch: char) -> bool {
    matches!(
        ch,
        '。' | '、'
            | '，'
            | '．'
            | '！'
            | '？'
            | '；'
            | '：'
            | '!'
            | '?'
            | '.'
            | ','
            | ';'
            | ':'
            | '「'
            | '」'
            | '『'
            | '』'
            | '（'
            | '）'
            | '【'
            | '】'
            | '〈'
            | '〉'
            | '《'
            | '》'
            | '…'
            | '‥'
            | '・'
            | '～'
            | '〜'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_starts_matches_reference_rule() {
        assert!(window_starts(0, 128, 64).is_empty());
        assert_eq!(window_starts(10, 128, 64), vec![0]);
        assert_eq!(window_starts(128, 128, 64), vec![0]);
        assert_eq!(window_starts(129, 128, 64), vec![0, 1]);
        assert_eq!(window_starts(256, 128, 64), vec![0, 64, 128]);
        assert_eq!(window_starts(300, 128, 64), vec![0, 64, 128, 172]);
        // stride 0 視同 1，逐格前進後再補貼齊結尾的 172。
        let dense: Vec<usize> = (0..=172).collect();
        assert_eq!(window_starts(300, 128, 0), dense);
    }

    #[test]
    fn test_merge_window_rows_prefers_central_prediction() {
        let starts = vec![0, 1];
        let rows = vec![
            vec![vec![1.0, 0.0], vec![9.0, 0.0], vec![9.0, 0.0]],
            vec![vec![0.0, 1.0], vec![0.0, 1.0], vec![0.0, 1.0]],
        ];
        // n=4、window=3：位置 1 由第一個視窗的中央列勝出，位置 2 由第二個
        // 視窗的中央列勝出；邊緣列不會蓋掉中央列。
        let merged = merge_window_rows(4, 3, &starts, &rows);
        assert_eq!(merged[0], Some(vec![1.0, 0.0]));
        assert_eq!(merged[1], Some(vec![9.0, 0.0]));
        assert_eq!(merged[2], Some(vec![0.0, 1.0]));
        assert_eq!(merged[3], Some(vec![0.0, 1.0]));
    }

    #[test]
    fn test_merge_window_rows_keeps_missing_positions_empty() {
        let merged = merge_window_rows(3, 2, &[0], &[vec![vec![1.0], vec![2.0]]]);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0], Some(vec![1.0]));
        assert_eq!(merged[1], Some(vec![2.0]));
        assert_eq!(merged[2], None);
    }

    #[test]
    fn test_argmax_and_character_classes() {
        assert_eq!(argmax(&[]), 0);
        assert_eq!(argmax(&[0.1, 0.9, 0.2]), 1);
        assert!(is_han('我'));
        assert!(!is_han('ㄨ'));
        assert!(is_punct('。'));
        assert!(is_punct('」'));
        assert!(!is_punct('字'));
    }
}
