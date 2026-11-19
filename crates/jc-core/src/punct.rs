//! 提交時補標點的純資料契約。
//!
//! 評分器只回傳「在第幾個字元前插入哪個標點」，核心驗證後才套用；不改、不刪
//! 使用者的任何原字。模型載入、時間與取消由宿主負責。

use crate::zh::han::is_han;

/// 允許插入的標點。
pub const MARKS: &str = "，。？！、；";

/// 在原字串第 `offset` 個字元（rune）之前插入 `text`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// 插入位置，以 rune 計算。
    pub offset: usize,
    /// 要插入的標點。
    pub text: String,
}

impl Edit {
    /// 建立一個插入編輯。
    pub fn new(offset: usize, text: impl Into<String>) -> Self {
        Self {
            offset,
            text: text.into(),
        }
    }
}

/// 補標點模型。
pub trait Punctuator {
    /// 回傳要插入的標點編輯；沒有建議時回傳空清單。
    fn punctuate(&self, text: &str) -> Result<Vec<Edit>, PunctError>;
}

/// 補標點模型錯誤。
#[derive(Debug, thiserror::Error)]
pub enum PunctError {
    /// 模型載入或執行失敗。
    #[error("補標點模型失敗：{0}")]
    Model(String),
}

fn allowed(mark: &str) -> bool {
    let mut chars = mark.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => MARKS.contains(ch),
        _ => false,
    }
}

/// 驗證後插入：位置嚴格遞增、在兩個漢字之間、不在開頭或段尾、只用 [`MARKS`]。
///
/// 任一編輯不合規就整批不套用（回傳原文與 `false`），避免部分套用造成意外結果。
pub fn apply(text: &str, edits: &[Edit]) -> (String, bool) {
    apply_with(text, edits, false, true)
}

/// 日文模型的插入規則：標點可以接在假名、漢字或段落尾端，但仍然只在原字
/// 之間插入，不刪除、不改寫任何原字。開頭不加、位置嚴格遞增、只接受
/// [`MARKS`]。中文路徑維持 [`apply`] 的漢字邊界。
pub fn apply_japanese(text: &str, edits: &[Edit]) -> (String, bool) {
    apply_with(text, edits, true, false)
}

fn apply_with(text: &str, edits: &[Edit], allow_end: bool, han_only: bool) -> (String, bool) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_japanese_inserts_after_kana_and_at_end() {
        let text = "今日は天気です";
        let (got, ok) = apply_japanese(text, &[Edit::new(3, "、"), Edit::new(7, "。")]);
        assert!(ok);
        assert_eq!(got, "今日は、天気です。");
        // 只拿掉插入的標點就能完全還原原文。
        assert_eq!(got.chars().count(), text.chars().count() + 2);

        for bad in [
            vec![Edit::new(0, "。")],
            vec![Edit::new(9, "。")],
            vec![Edit::new(2, ".")],
            vec![Edit::new(4, "、")], // 超過較短輸入的長度
        ] {
            let (got, ok) = apply_japanese("今日は", &bad);
            assert!(!ok, "不合規的日文編輯不套用：{bad:?}");
            assert_eq!(got, "今日は");
        }
    }

    #[test]
    fn test_apply_inserts_only_between_han() {
        let (got, ok) = apply("明天早上九點開會請記得帶資料", &[Edit::new(8, "，")]);
        assert!(ok);
        assert_eq!(got, "明天早上九點開會，請記得帶資料");

        for bad in [
            vec![Edit::new(0, "，")],                     // 開頭
            vec![Edit::new(3, "，")],                     // 段尾
            vec![Edit::new(1, ".")],                      // 不允許的符號
            vec![Edit::new(2, "，"), Edit::new(1, "。")], // 非遞增
            vec![Edit::new(1, "，，")],                   // 多字元
        ] {
            let (got, ok) = apply("好嗎?", &bad);
            assert!(!ok, "不合規的編輯不套用：{bad:?}");
            assert_eq!(got, "好嗎?");
        }
        let (got, ok) = apply("價格100元", &[Edit::new(2, "，")]);
        assert!(!ok, "不得插在非漢字旁");
        assert_eq!(got, "價格100元");
    }

    #[test]
    fn test_empty_edits_leave_text_untouched() {
        let (got, ok) = apply("今天天氣很好", &[]);
        assert_eq!(got, "今天天氣很好");
        assert!(!ok);
    }
}
