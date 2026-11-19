//! 詞格的邊（候選）與鍵。
//!
//! 對照前身 `core/lattice/candidate.go`。Rust 契約沒有語言標籤與 JSON，
//! 位置改成音節索引；`reading` 是音節以空格分隔的字串。

use crate::candidate::{Candidate, CandidateSource};

/// 詞格上的一條邊：從 `start` 走到 `end` 的一個候選詞。
///
/// `start` 與 `end` 是**音節索引**（半開區間 `[start, end)`），不是位元組
/// 位移；一個音節對應使用者打的一組注音。長詞候選會跨越多個音節，Viterbi
/// 選中時自然形成單一片段，不需要額外合併。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    /// 起點音節索引（含）。
    pub start: usize,
    /// 終點音節索引（不含）。
    pub end: usize,
    /// 輸出詞面。
    pub surface: String,
    /// 這條邊涵蓋的讀音，音節以單一空格分隔。
    pub reading: String,
    /// 含長度折扣的單詞成本，越小越優先。
    pub cost: i32,
    /// 詞性標籤，供連接成本查表。
    pub pos: u16,
    /// 候選來源。
    pub source: CandidateSource,
}

impl Edge {
    /// 這條邊涵蓋的音節數。
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// 這條邊是否為零長度。
    ///
    /// 詞格建構會拒絕零長度的邊，這個方法只為 [`Edge::len`] 的
    /// `clippy::len_without_is_empty` 慣例存在。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 穩定鍵，供除重、排序與測試使用。
    ///
    /// 格式：`[start,end)|reading|surface|source|pos`。
    pub fn key(&self) -> String {
        format!(
            "[{},{})|{}|{}|{}|{}",
            self.start,
            self.end,
            self.reading,
            self.surface,
            source_tag(self.source),
            self.pos
        )
    }

    /// 轉成對外候選；路徑輸出與候選窗共用同一種資料形狀。
    pub fn to_candidate(&self) -> Candidate {
        Candidate::new(
            self.surface.clone(),
            self.reading.clone(),
            self.cost,
            self.source,
        )
    }
}

/// 候選來源的穩定字串，寫進 [`Edge::key`]。
fn source_tag(source: CandidateSource) -> &'static str {
    match source {
        CandidateSource::Lexicon => "lexicon",
        CandidateSource::Personal => "personal",
        CandidateSource::Predicted => "predicted",
        CandidateSource::Literal => "literal",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(surface: &str, reading: &str, cost: i32) -> Edge {
        Edge {
            start: 0,
            end: 2,
            surface: surface.to_string(),
            reading: reading.to_string(),
            cost,
            pos: 3,
            source: CandidateSource::Lexicon,
        }
    }

    #[test]
    fn key_contains_span_reading_surface_source_and_pos() {
        let e = edge("注音", "ㄓㄨˋ ㄧㄣ", 1500);
        assert_eq!(e.key(), "[0,2)|ㄓㄨˋ ㄧㄣ|注音|lexicon|3");
        assert_eq!(e.len(), 2);
        assert!(!e.is_empty());
    }

    #[test]
    fn keys_differ_when_any_field_differs() {
        let base = edge("注音", "ㄓㄨˋ ㄧㄣ", 1);

        let mut other = base.clone();
        other.source = CandidateSource::Literal;
        assert_ne!(base.key(), other.key());

        let mut other = base.clone();
        other.pos = 4;
        assert_ne!(base.key(), other.key());

        let mut other = base.clone();
        other.end = 3;
        assert_ne!(base.key(), other.key());
    }

    #[test]
    fn zero_length_edge_is_empty() {
        let mut e = edge("x", "x", 0);
        e.end = e.start;
        assert_eq!(e.len(), 0);
        assert!(e.is_empty());
    }

    #[test]
    fn to_candidate_preserves_surface_reading_cost_and_source() {
        let e = edge("注音", "ㄓㄨˋ ㄧㄣ", 1500);
        let c = e.to_candidate();
        assert_eq!(c.surface, "注音");
        assert_eq!(c.reading, "ㄓㄨˋ ㄧㄣ");
        assert_eq!(c.cost, 1500);
        assert_eq!(c.source, CandidateSource::Lexicon);
    }
}
