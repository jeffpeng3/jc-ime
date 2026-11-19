//! 組字區片段與書寫系統。

use crate::lang::Mode;

/// 片段使用的書寫系統。
///
/// 只用來描述片段本身的字面種類，不單獨決定語言；例如 [`Script::Latin`]
/// 同時可能是英文或日文羅馬字。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Script {
    /// 注音符號。
    Bopomofo,
    /// 平假名、片假名、長音符與踊り字。
    Kana,
    /// 拉丁字母。
    Latin,
    /// 數字。
    Digit,
    /// 標點、空白與其他符號。
    Symbol,
    /// 其他無法歸類的字元（包含漢字）。
    Other,
}

impl Script {
    /// 回報這個書寫系統是否為語言的強訊號。
    ///
    /// 注音與假名只能來自特定語言，因此是強訊號；其餘都不是。
    pub fn is_strong(self) -> bool {
        matches!(self, Script::Bopomofo | Script::Kana)
    }
}

/// 組字區中的一個片段。
///
/// `start` 與 `end` 是組字字串的 UTF-8 位元組位移，範圍為半開區間
/// `[start, end)`。片段長度一律用 [`Segment::len`] 取得。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// 片段起點（含）。
    pub start: usize,
    /// 片段終點（不含）。
    pub end: usize,
    /// 這個片段所屬的輸入模式。
    pub mode: Mode,
    /// 這個片段的書寫系統。
    pub script: Script,
}

impl Segment {
    /// 建立片段。
    pub fn new(start: usize, end: usize, mode: Mode, script: Script) -> Self {
        Self {
            start,
            end,
            mode,
            script,
        }
    }

    /// 片段長度（位元組）。
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// 片段是否為空。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_length_is_end_minus_start() {
        let s = Segment::new(2, 5, Mode::Ja, Script::Kana);
        assert_eq!(s.len(), 3);
        assert!(!s.is_empty());
        assert_eq!(s.start, 2);
        assert_eq!(s.end, 5);
    }

    #[test]
    fn inverted_range_has_zero_length() {
        let s = Segment::new(7, 3, Mode::Zh, Script::Bopomofo);
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
    }

    #[test]
    fn only_bopomofo_and_kana_are_strong_scripts() {
        assert!(Script::Bopomofo.is_strong());
        assert!(Script::Kana.is_strong());
        assert!(!Script::Latin.is_strong());
        assert!(!Script::Digit.is_strong());
        assert!(!Script::Symbol.is_strong());
        assert!(!Script::Other.is_strong());
    }
}
