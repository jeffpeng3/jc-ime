//! 按鍵流座標與引擎狀態版本。

use super::LanguageId;

/// 按鍵流的 UTF-8 位元組半開區間。
///
/// 起點與終點必須落在 char boundary。`AsciiInput::typed()`、`literal_spans`
/// 與 [`super::Hypothesis::span`] 都用這個座標；`CompSegment::span` 是顯示字串
/// 的 rune 座標，兩者不可混用。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[allow(dead_code)] // P1 只保留型別；P2 的合併與 late result 會用。
pub struct Span {
    /// 起點（含）。
    pub start: usize,
    /// 終點（不含）。
    pub end: usize,
}

#[allow(dead_code)] // P1 的座標查詢；P2 的 SpanGraph 會用。
impl Span {
    /// 建立一段範圍。
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// 位元組長度。
    pub fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// 是否為空範圍。
    pub fn is_empty(self) -> bool {
        self.start >= self.end
    }

    /// 位置 `at` 是否落在範圍內。
    pub fn contains(self, at: usize) -> bool {
        self.start <= at && at < self.end
    }

    /// 兩段是否重疊。
    pub fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }
}

/// 引擎狀態版本；late result 丟棄與平行合併的 tie-break 依據。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // P1 只保留型別；P2 的 epoch／late result 會用。
pub struct StateToken {
    /// 產生結果的引擎。
    pub engine: LanguageId,
    /// 每收到一鍵就遞增；late result 的 epoch 較舊就丟棄。
    pub epoch: u64,
    /// 同一鍵內引擎自己的修訂號。
    pub revision: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_is_half_open() {
        let span = Span::new(2, 5);
        assert_eq!(span.len(), 3);
        assert!(span.contains(2));
        assert!(span.contains(4));
        assert!(!span.contains(5));
        assert!(!span.is_empty());
        assert!(Span::new(3, 3).is_empty());
    }

    #[test]
    fn overlap_needs_a_real_intersection() {
        assert!(Span::new(0, 4).overlaps(Span::new(3, 8)));
        assert!(!Span::new(0, 4).overlaps(Span::new(4, 8)));
        assert!(!Span::new(0, 0).overlaps(Span::new(0, 4)));
    }
}
