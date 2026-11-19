//! 路徑評分。
//!
//! 詞格的成本模型由四項組成：詞條本身的單詞成本、詞性對之間的連接成本、
//! 詞典查不到時的未知詞成本，以及讓長詞優先的長度折扣。成本一律
//! **越小越優先**，與 [`crate::candidate::Candidate::cost`] 的 MeCab 語意一致。
//!
//! 數值沿用前身 `0c-ime`：連接成本保底值取自 `core/ja/lexicon`
//! 的 `DefaultConnectionCost`，未知詞成本取自 `core/ja/decode` 的
//! `DefaultUnknownCost`，單詞長度上限取自 `core/decode.MaxWordLen`。
//! 長度折扣對應前身 Viterbi 在成本相同時偏好邊數較少的路徑。

use std::collections::HashMap;

/// 詞性對沒有指定連接成本時的保底值。
///
/// 對應前身 `core/ja/lexicon.DefaultConnectionCost`。
pub const DEFAULT_CONNECTION_COST: i32 = 1;

/// 未知詞每個音節的成本。
///
/// 夠高讓已知詞優先，但仍能輸出詞典沒有收錄的音節。
/// 對應前身 `core/ja/decode.DefaultUnknownCost`。
pub const DEFAULT_UNKNOWN_COST: i32 = 8;

/// 單詞讀音的音節數上限。
///
/// 對應前身 `core/decode.MaxWordLen`；詞庫宣告更長的詞時以詞庫為準。
pub const MAX_WORD_UNITS: usize = 8;

/// 每多一個音節折抵的單詞成本，用來實現長詞優先。
pub const DEFAULT_LENGTH_BONUS: i32 = 1;

/// 詞性對的連接成本表。
///
/// 左側是前一條邊的詞性，右側是下一條邊的詞性；沒有登記的組合回傳
/// [`ConnectionMatrix::fallback`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionMatrix {
    costs: HashMap<(u16, u16), i32>,
    fallback: i32,
}

impl ConnectionMatrix {
    /// 建立只有保底值的空表。
    pub fn new() -> Self {
        Self::with_fallback(DEFAULT_CONNECTION_COST)
    }

    /// 建立空表並指定未登記組合的保底成本。
    pub fn with_fallback(fallback: i32) -> Self {
        Self {
            costs: HashMap::new(),
            fallback,
        }
    }

    /// 登記一組詞性對的連接成本。
    pub fn set(&mut self, left: u16, right: u16, cost: i32) {
        self.costs.insert((left, right), cost);
    }

    /// 查詢連接成本；未登記時回傳保底值。
    pub fn cost(&self, left: u16, right: u16) -> i32 {
        self.costs
            .get(&(left, right))
            .copied()
            .unwrap_or(self.fallback)
    }

    /// 未登記組合的保底成本。
    pub fn fallback(&self) -> i32 {
        self.fallback
    }

    /// 已登記的詞性對數量。
    pub fn len(&self) -> usize {
        self.costs.len()
    }

    /// 是否沒有任何已登記的詞性對。
    pub fn is_empty(&self) -> bool {
        self.costs.is_empty()
    }
}

impl Default for ConnectionMatrix {
    fn default() -> Self {
        Self::new()
    }
}

/// 建立與評分詞格時使用的成本設定。
///
/// 這個型別同時是 [`crate::lattice::Lattice::build`] 的 `ctx` 與 Viterbi 的
/// 評分器，讓建邊與解碼使用同一組數字。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoreContext {
    /// 詞性對的連接成本表。
    pub connection: ConnectionMatrix,
    /// 未知詞每個音節的成本。
    pub unknown_cost: i32,
    /// 每多一個音節折抵的單詞成本。
    pub length_bonus: i32,
    /// 建詞格時單詞的最大音節數；0 表示不限制。
    pub max_word_units: usize,
}

impl Default for ScoreContext {
    fn default() -> Self {
        Self {
            connection: ConnectionMatrix::new(),
            unknown_cost: DEFAULT_UNKNOWN_COST,
            length_bonus: DEFAULT_LENGTH_BONUS,
            max_word_units: MAX_WORD_UNITS,
        }
    }
}

impl ScoreContext {
    /// 設定單詞成本：詞條成本加上長度折扣。
    ///
    /// 長度折扣讓同樣總成本的長詞勝過多個短詞，等於前身 Viterbi 在
    /// 同成本時偏好較少邊數的行為。
    pub fn word_cost(&self, dictionary_cost: i32, units: usize) -> i32 {
        dictionary_cost.saturating_add(self.length_adjustment(units))
    }

    /// 長度折扣。單音節為 0，每多一個音節扣掉 [`ScoreContext::length_bonus`]。
    pub fn length_adjustment(&self, units: usize) -> i32 {
        let extra = units_to_i32(units.saturating_sub(1));
        self.length_bonus.saturating_mul(extra).saturating_neg()
    }

    /// 未知詞的單詞成本：每音節固定成本乘以音節數。
    pub fn unknown_word_cost(&self, units: usize) -> i32 {
        self.unknown_cost.saturating_mul(units_to_i32(units))
    }

    /// 查詢連接成本，語意同 [`ConnectionMatrix::cost`]。
    pub fn connection_cost(&self, left: u16, right: u16) -> i32 {
        self.connection.cost(left, right)
    }
}

/// Viterbi 需要的評分介面。
///
/// 單詞成本在建邊時就算進 [`crate::lattice::Edge::cost`]，這裡只負責
/// 邊與邊之間的連接成本。
pub trait Scorer {
    /// 前一條邊的詞性接到下一條邊的連接成本，越小越優先。
    ///
    /// 路徑起點（BOS）不會呼叫這個方法。
    fn connection_cost(&self, prev_pos: u16, next_pos: u16) -> i32;
}

impl Scorer for ScoreContext {
    fn connection_cost(&self, prev_pos: u16, next_pos: u16) -> i32 {
        self.connection.cost(prev_pos, next_pos)
    }
}

/// 把音節數轉成 `i32`；超出範圍時飽和。
fn units_to_i32(units: usize) -> i32 {
    i32::try_from(units).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_predecessor_numbers() {
        let ctx = ScoreContext::default();
        assert_eq!(ctx.unknown_cost, 8);
        assert_eq!(ctx.length_bonus, 1);
        assert_eq!(ctx.max_word_units, 8);
        assert_eq!(ctx.connection.fallback(), 1);
        assert_eq!(DEFAULT_CONNECTION_COST, 1);
        assert_eq!(DEFAULT_UNKNOWN_COST, 8);
        assert_eq!(MAX_WORD_UNITS, 8);
        assert!(ctx.connection.is_empty());
    }

    #[test]
    fn length_adjustment_grows_with_units() {
        let ctx = ScoreContext::default();
        assert_eq!(ctx.length_adjustment(1), 0);
        assert_eq!(ctx.length_adjustment(2), -1);
        assert_eq!(ctx.length_adjustment(4), -3);
        assert_eq!(ctx.length_adjustment(0), 0);
    }

    #[test]
    fn word_cost_is_dictionary_cost_with_length_bonus() {
        let ctx = ScoreContext::default();
        assert_eq!(ctx.word_cost(1000, 1), 1000);
        assert_eq!(ctx.word_cost(1000, 2), 999);
        assert_eq!(ctx.word_cost(1000, 3), 998);
        assert_eq!(ctx.word_cost(0, 2), -1);
    }

    #[test]
    fn zero_length_bonus_keeps_dictionary_cost() {
        let ctx = ScoreContext {
            length_bonus: 0,
            ..Default::default()
        };
        assert_eq!(ctx.word_cost(42, 5), 42);
        assert_eq!(ctx.length_adjustment(5), 0);
    }

    #[test]
    fn unknown_word_cost_is_per_unit() {
        let ctx = ScoreContext::default();
        assert_eq!(ctx.unknown_word_cost(1), 8);
        assert_eq!(ctx.unknown_word_cost(2), 16);
        assert_eq!(ctx.unknown_word_cost(0), 0);
    }

    #[test]
    fn connection_matrix_uses_fallback_for_unknown_pairs() {
        let mut matrix = ConnectionMatrix::new();
        assert_eq!(matrix.cost(1, 2), 1);
        matrix.set(1, 2, 30);
        assert_eq!(matrix.cost(1, 2), 30);
        assert_eq!(matrix.cost(2, 1), 1);
        assert_eq!(matrix.len(), 1);
        assert!(!matrix.is_empty());
    }

    #[test]
    fn connection_matrix_accepts_custom_fallback() {
        let mut matrix = ConnectionMatrix::with_fallback(7);
        matrix.set(3, 4, 0);
        assert_eq!(matrix.cost(3, 4), 0);
        assert_eq!(matrix.cost(4, 3), 7);
        assert_eq!(matrix.fallback(), 7);
    }

    #[test]
    fn score_context_is_a_scorer() {
        let mut ctx = ScoreContext::default();
        ctx.connection.set(1, 1, 0);
        ctx.connection.set(1, 2, 5);
        // 透過 trait 呼叫，確認 Viterbi 看到的是同一張表。
        fn cost<S: Scorer>(s: &S, left: u16, right: u16) -> i32 {
            s.connection_cost(left, right)
        }
        assert_eq!(cost(&ctx, 1, 1), 0);
        assert_eq!(cost(&ctx, 1, 2), 5);
        assert_eq!(cost(&ctx, 9, 9), 1);
    }

    #[test]
    fn huge_unit_counts_saturate_instead_of_overflowing() {
        let ctx = ScoreContext::default();
        assert_eq!(ctx.unknown_word_cost(usize::MAX), i32::MAX);
        assert_eq!(ctx.length_adjustment(usize::MAX), -i32::MAX);
        assert_eq!(ctx.word_cost(i32::MAX, usize::MAX), 0);
    }
}
