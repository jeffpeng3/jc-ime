//! 對外快照：組字字串、游標、片段與候選。

use crate::candidate::Candidate;
use crate::segment::Segment;

/// 一次輸入手勢後的完整狀態。
///
/// 外殼不需要保存狀態，每次都以這份快照重畫。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snapshot {
    /// 目前組字字串。
    pub preedit: String,
    /// 游標位置，以 UTF-16 code unit 計算（配合各平台游標 API）。
    pub cursor_utf16: usize,
    /// 組字字串的分段。
    pub segments: Vec<Segment>,
    /// 目前可選的候選。
    pub candidates: Vec<Candidate>,
    /// 目前高亮的候選索引；沒有候選時為 0。
    pub highlighted: usize,
}

impl Snapshot {
    /// 建立空快照。
    pub fn empty() -> Self {
        Self::default()
    }

    /// 是否沒有任何組字內容。
    pub fn is_empty(&self) -> bool {
        self.preedit.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::CandidateSource;
    use crate::lang::Mode;
    use crate::segment::Script;

    #[test]
    fn empty_has_zero_cursor_and_no_items() {
        let s = Snapshot::empty();
        assert_eq!(s.preedit, "");
        assert_eq!(s.cursor_utf16, 0);
        assert!(s.segments.is_empty());
        assert!(s.candidates.is_empty());
        assert_eq!(s.highlighted, 0);
        assert!(s.is_empty());
    }

    #[test]
    fn populated_snapshot_keeps_highlighted_index() {
        let s = Snapshot {
            preedit: "ㄨㄛˇ".to_string(),
            cursor_utf16: 4,
            segments: vec![Segment::new(0, 9, Mode::Zh, Script::Bopomofo)],
            candidates: vec![Candidate::new("我", "ㄨㄛˇ", 1, CandidateSource::Lexicon)],
            highlighted: 0,
        };
        assert_eq!(s.cursor_utf16, 4);
        assert_eq!(s.segments[0].len(), 9);
        assert_eq!(s.candidates[0].surface, "我");
        assert!(!s.is_empty());
    }
}
