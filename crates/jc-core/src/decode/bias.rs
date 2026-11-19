//! 詞弧成本與偏置。
//!
//! Go 的 `bias.go` 把語言模型機率、個人證據與修正懲罰分開；Rust 的成本模型
//! 只有 `i32` 成本。個人化（Evidence、CorrectionPenalty）屬 M4，這裡留下
//! 成本組合與兩個回傳 0 的掛鉤。

use super::repeat::REPEAT_COST;
use crate::lattice::Edge;
use crate::score::Scorer;

/// 一條邊接在前一條邊後面的成本，含連接成本與 bigram。
///
/// `left` 是前一個詞面（句首為 `None`）；BOS 不加連接成本，與
/// [`crate::lattice::Viterbi`] 相同。
pub fn step_cost(scorer: &dyn Scorer, left: Option<&str>, prev_pos: u16, edge: &Edge) -> i32 {
    scorer.arc_cost(
        left,
        prev_pos,
        edge.cost,
        edge.pos,
        &edge.surface,
        edge.source,
    )
}

/// 修正讀音的懲罰；成本模型還沒有修正來源。
pub fn correction_penalty() -> i32 {
    0
}

/// 讀音偏好加成；沒有個人化時為 0。
pub fn reading_lift() -> i32 {
    0
}

/// 非詞疊字的固定成本。
pub fn non_word_repeat_cost() -> i32 {
    REPEAT_COST
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::CandidateSource;
    use crate::decode::BOS;
    use crate::score::{ConnectionMatrix, ScoreContext};

    fn edge(pos: u16, cost: i32) -> Edge {
        Edge {
            start: 0,
            end: 1,
            surface: "甲".to_string(),
            reading: "ㄐㄧㄚˇ".to_string(),
            cost,
            pos,
            source: CandidateSource::Lexicon,
        }
    }

    #[test]
    fn bos_has_no_connection_cost() {
        let ctx = ScoreContext::default();
        assert_eq!(step_cost(&ctx, None, BOS, &edge(1, 5)), 5);
    }

    #[test]
    fn connection_cost_is_added_after_the_first_edge() {
        let ctx = ScoreContext {
            connection: ConnectionMatrix::with_fallback(3),
            ..Default::default()
        };
        assert_eq!(step_cost(&ctx, Some("前"), 1, &edge(2, 5)), 8);
    }

    #[test]
    fn personal_hooks_are_zero() {
        assert_eq!(correction_penalty(), 0);
        assert_eq!(reading_lift(), 0);
        assert_eq!(non_word_repeat_cost(), REPEAT_COST);
    }
}
