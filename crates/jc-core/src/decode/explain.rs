//! 診斷用的成本拆解。
//!
//! 只在離線評測呼叫，不在逐鍵熱路徑收集或保存文字。

use super::{readings_key, Decoder, Lattice, Locks, Seg};

/// 一個片段的成本組成。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Contribution {
    /// 詞面。
    pub word: String,
    /// 詞庫讀音。
    pub reading: Vec<String>,
    /// 詞條本身的成本。
    pub word_cost: i32,
    /// 前後詞的連接成本。
    pub connection_cost: i32,
    /// 兩者相加。
    pub total: i32,
}

/// 單一片段的解釋。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplainedSegment {
    /// 這個片段。
    pub seg: Seg,
    /// 前一個詞。
    pub left: String,
    /// 這段累計成本。
    pub cost: i32,
    /// 各詞條的貢獻；目前只有最短路徑的一項。
    pub contributions: Vec<Contribution>,
}

/// 一次完整解碼的解釋。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Explanation {
    /// 是否走得到。
    pub reachable: bool,
    /// 整體成本。
    pub cost: i32,
    /// 逐片段解釋。
    pub segments: Vec<ExplainedSegment>,
}

impl Decoder<'_> {
    /// 最佳路徑的完整解釋。
    pub fn explain_best(&self, lat: &Lattice, locks: &Locks, prefix: bool) -> Explanation {
        let (path, cost) = self.best(lat, locks, prefix);
        if path.is_empty() {
            return Explanation::default();
        }
        let mut out = Explanation {
            reachable: true,
            cost,
            ..Default::default()
        };
        let mut prev = super::BOS;
        let mut left = String::new();
        for seg in &path {
            let left_word = (!left.is_empty()).then_some(left.as_str());
            let (word_cost, connection, pos) = if seg.partial {
                let k = seg.end - seg.start;
                let entry = self
                    .tail_entries(lat, seg.start, k)
                    .into_iter()
                    .find(|e| super::first_chars(&e.surface, k) == seg.word);
                match entry {
                    Some(e) => {
                        let word_cost = self.ctx.word_cost(e.cost, e.units());
                        let total = self.scorer.arc_cost(
                            left_word,
                            prev,
                            word_cost,
                            e.pos,
                            &e.surface,
                            crate::candidate::CandidateSource::Lexicon,
                        );
                        (word_cost, total.saturating_sub(word_cost), e.pos)
                    }
                    None => (0, 0, super::BOS),
                }
            } else {
                match lat
                    .edges_from(seg.start)
                    .iter()
                    .find(|e| e.end == seg.end && e.surface == seg.word)
                {
                    Some(e) => {
                        let total = self.scorer.edge_cost(left_word, prev, e);
                        (e.cost, total.saturating_sub(e.cost), e.pos)
                    }
                    None => (0, 0, super::BOS),
                }
            };
            let reading: Vec<String> = readings_key(&lat.readings()[seg.start..seg.end])
                .split_whitespace()
                .map(|s| s.to_string())
                .collect();
            out.segments.push(ExplainedSegment {
                seg: seg.clone(),
                left: left.clone(),
                cost: word_cost.saturating_add(connection),
                contributions: vec![Contribution {
                    word: seg.word.clone(),
                    reading,
                    word_cost,
                    connection_cost: connection,
                    total: word_cost.saturating_add(connection),
                }],
            });
            prev = pos;
            left = seg.word.clone();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::{InMemoryLexicon, LexEntry};
    use crate::score::{ConnectionMatrix, ScoreContext};

    fn entry(surface: &str, reading: &str, cost: i32) -> LexEntry {
        LexEntry::new(surface, reading, cost, 1)
    }

    #[test]
    fn explain_reports_cost_per_segment() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("注", "ㄓㄨˋ", 10));
        lex.push(entry("音", "ㄧㄣ", 10));
        let ctx = ScoreContext {
            length_bonus: 0,
            unknown_cost: 100,
            connection: ConnectionMatrix::with_fallback(2),
            ..Default::default()
        };
        let lat = Lattice::build(&["ㄓㄨˋ", "ㄧㄣ"], &lex, &ctx).expect("建立詞格");
        let d = Decoder::new(&lex, &ctx);
        let e = d.explain_best(&lat, &Locks::new(), false);
        assert!(e.reachable);
        assert_eq!(e.cost, 22);
        assert_eq!(e.segments.len(), 2);
        assert_eq!(e.segments[0].contributions[0].word_cost, 10);
        assert_eq!(e.segments[1].contributions[0].connection_cost, 2);
        assert_eq!(e.segments[1].contributions[0].total, 12);
    }
}
