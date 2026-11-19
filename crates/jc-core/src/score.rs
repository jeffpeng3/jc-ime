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
//!
//! 有 bigram 語言模型時，[`LmScorer`] 把詞成本裡的 unigram 部分換成前身
//! `personal.Model.Prob` 的內插：`μ·P_bi(w|prev) + (1−μ)·P_uni(w)`。
//! μ 的預設值 [`DEFAULT_BIGRAM_MU`] 與調整方式都照前身 `personal.DefaultParams`。

use std::cell::Cell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::candidate::CandidateSource;
use crate::lattice::Edge;

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

/// 系統 bigram 與 unigram 的內插權重。
///
/// 對應前身 `core/personal.DefaultParams().Mu`：0.7 表示 bigram 佔七成。
/// 前身只透過 `Params.Mu` 調整（`0c-eval --mu`），沒有其他預設值。
pub const DEFAULT_BIGRAM_MU: f64 = 0.7;

/// 句首的虛擬前詞；只當前詞查 bigram，不會出現在輸出。
///
/// 對應前身 `core/lm.BOS`。
pub const BOS_WORD: &str = "<s>";

/// 路徑起點（BOS）的詞性狀態鍵，不會與任何詞性衝突。
pub const BOS_POS: u16 = u16::MAX;

/// 未登錄詞的 unigram 機率；對應前身 `core/lm.Unseen`。
pub const UNSEEN_UNIGRAM: f64 = 1e-8;

/// 機率轉成整數毫成本 `-ln(p)·1000`；0 與極小值給很大的成本。
///
/// 詞典與語言模型都用這個換算，讓機率可以直接相加比較。
pub fn probability_cost(probability: f64) -> i32 {
    if probability <= 0.0 || !probability.is_finite() {
        return 100_000;
    }
    (-probability.ln() * 1000.0).round().min(100_000.0) as i32
}

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
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreContext {
    /// 詞性對的連接成本表。
    pub connection: ConnectionMatrix,
    /// 未知詞每個音節的成本。
    pub unknown_cost: i32,
    /// 每多一個音節折抵的單詞成本。
    pub length_bonus: i32,
    /// 建詞格時單詞的最大音節數；0 表示不限制。
    pub max_word_units: usize,
    /// bigram 與 unigram 的內插權重；[`LmScorer`] 使用。
    pub bigram_mu: f64,
}

impl Default for ScoreContext {
    fn default() -> Self {
        Self {
            connection: ConnectionMatrix::new(),
            unknown_cost: DEFAULT_UNKNOWN_COST,
            length_bonus: DEFAULT_LENGTH_BONUS,
            max_word_units: MAX_WORD_UNITS,
            bigram_mu: DEFAULT_BIGRAM_MU,
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
/// 單詞成本在建邊時就算進 [`crate::lattice::Edge::cost`]，這裡負責邊與邊之間
/// 的連接成本，以及（可選的）前一個詞造成的語言模型成本。
pub trait Scorer {
    /// 前一條邊的詞性接到下一條邊的連接成本，越小越優先。
    ///
    /// 路徑起點（BOS）不會呼叫這個方法。
    fn connection_cost(&self, prev_pos: u16, next_pos: u16) -> i32;

    /// 單詞本身加上語言模型的成本；`left` 是前一個詞面，`None` 表示句首。
    ///
    /// 預設直接用建邊時算好的 `word_cost`，等於沒有 bigram 的行為。
    /// 有 bigram 的實作（例如 [`LmScorer`]）在這裡替換 unigram 部分。
    fn word_cost(
        &self,
        left: Option<&str>,
        surface: &str,
        word_cost: i32,
        source: CandidateSource,
    ) -> i32 {
        let _ = (left, surface, source);
        word_cost
    }

    /// 一條弧接在前一個詞後面的總成本（單詞＋語言模型＋連接）。
    fn arc_cost(
        &self,
        left: Option<&str>,
        prev_pos: u16,
        word_cost: i32,
        pos: u16,
        surface: &str,
        source: CandidateSource,
    ) -> i32 {
        let connection = if prev_pos == BOS_POS {
            0
        } else {
            self.connection_cost(prev_pos, pos)
        };
        self.word_cost(left, surface, word_cost, source)
            .saturating_add(connection)
    }

    /// 一條邊接在前一個詞後面的總成本。
    fn edge_cost(&self, left: Option<&str>, prev_pos: u16, edge: &Edge) -> i32 {
        self.arc_cost(
            left,
            prev_pos,
            edge.cost,
            edge.pos,
            &edge.surface,
            edge.source,
        )
    }
}

impl Scorer for ScoreContext {
    fn connection_cost(&self, prev_pos: u16, next_pos: u16) -> i32 {
        self.connection.cost(prev_pos, next_pos)
    }
}

/// bigram 語言模型的唯讀查詢介面。
///
/// 對應前身 `core/lm.System` 的 `Uni` 與 `Bi`；實作由 [`ZhLm`] 包住
/// `jc-data` 的 `lm.bin`。
pub trait BigramLm {
    /// P(w)；未登錄回傳 [`UNSEEN_UNIGRAM`]。
    fn uni(&self, word: &str) -> f64;

    /// P(w | left)；沒有這筆 bigram 回傳 0。
    fn bi(&self, left: &str, word: &str) -> f64;

    /// bigram 筆數；0 表示這份模型沒有 bigram。
    fn bigram_count(&self) -> usize;
}

/// `jc-data` 讀進來的注音語言模型（`lm.bin`）。
///
/// `jc-core` 只透過 [`BigramLm`] 查詢，不依賴檔案的記憶體布局。
#[derive(Clone)]
pub struct ZhLm {
    inner: Arc<jc_data::zh::Lm>,
    bi_queries: Arc<std::sync::atomic::AtomicU64>,
    bi_hits: Arc<std::sync::atomic::AtomicU64>,
}

impl ZhLm {
    /// 開啟並驗證 `lm.bin`（不檢查 manifest）。
    pub fn open(path: impl AsRef<Path>) -> Result<Self, jc_data::zh::ZhDataError> {
        Ok(Self {
            inner: Arc::new(jc_data::zh::Lm::open(path)?),
            bi_queries: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            bi_hits: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        })
    }

    /// 包住已開啟的語言模型。
    pub fn from_lm(lm: jc_data::zh::Lm) -> Self {
        Self {
            inner: Arc::new(lm),
            bi_queries: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            bi_hits: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// bigram 筆數。
    pub fn bigrams(&self) -> usize {
        self.inner.bigrams()
    }

    /// 詞數。
    pub fn words(&self) -> usize {
        self.inner.words()
    }

    /// 已查詢的 bigram 次數（所有 [`BigramLm::bi`] 呼叫）。
    pub fn bi_queries(&self) -> u64 {
        self.bi_queries.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// 查到的 bigram 次數（`P_bi > 0`）。
    pub fn bi_hits(&self) -> u64 {
        self.bi_hits.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl std::fmt::Debug for ZhLm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZhLm")
            .field("words", &self.inner.words())
            .field("bigrams", &self.inner.bigrams())
            .finish()
    }
}

impl BigramLm for ZhLm {
    fn uni(&self, word: &str) -> f64 {
        self.inner.uni(word)
    }

    fn bi(&self, left: &str, word: &str) -> f64 {
        let p = self.inner.bi(left, word);
        self.bi_queries
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if p > 0.0 {
            self.bi_hits
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        p
    }

    fn bigram_count(&self) -> usize {
        self.inner.bigrams()
    }
}

impl BigramLm for jc_data::zh::Lm {
    fn uni(&self, word: &str) -> f64 {
        jc_data::zh::Lm::uni(self, word)
    }

    fn bi(&self, left: &str, word: &str) -> f64 {
        jc_data::zh::Lm::bi(self, left, word)
    }

    fn bigram_count(&self) -> usize {
        self.bigrams()
    }
}

/// 把 bigram 內插接進路徑評分的評分器。
///
/// 成本公式照前身 `personal.Model.Prob`（沒有個人資料時）：
/// `P(w|left) = μ·P_bi(w|left) + (1−μ)·P_uni(w)`。詞典邊的成本已含
/// `-ln P_uni(w)`，這裡只加上「內插後 − unigram」的差額，因此單詞成本、
/// `P(r|w)` 與長度折扣都保持原樣。
///
/// `ctx.bigram_mu` 為 μ；模型沒有 bigram 時 [`LmScorer::word_cost`] 直接回傳
/// 原本的 `word_cost`，與沒有評分器完全相同。
pub struct LmScorer<'a> {
    ctx: &'a ScoreContext,
    lm: &'a dyn BigramLm,
    /// 查過幾次 bigram（不含模型沒有 bigram 的提早返回）。
    queries: Cell<u64>,
    /// 其中真的有這筆 bigram 的次數。
    hits: Cell<u64>,
}

impl<'a> LmScorer<'a> {
    /// 用成本設定的 μ 與指定的語言模型建立評分器。
    pub fn new(ctx: &'a ScoreContext, lm: &'a dyn BigramLm) -> Self {
        Self {
            ctx,
            lm,
            queries: Cell::new(0),
            hits: Cell::new(0),
        }
    }

    /// 已查詢的 bigram 次數。
    pub fn queries(&self) -> u64 {
        self.queries.get()
    }

    /// 查到的 bigram 次數（`P_bi > 0`）。
    pub fn hits(&self) -> u64 {
        self.hits.get()
    }

    /// 內插後的機率 `μ·P_bi(w|left) + (1−μ)·P_uni(w)`。
    pub fn probability(&self, left: &str, word: &str) -> f64 {
        let mu = self.ctx.bigram_mu;
        let p_uni = self.lm.uni(word);
        mu * self.lm.bi(left, word) + (1.0 - mu) * p_uni
    }

    /// 內插後相對 unigram 的成本差（毫成本）。
    ///
    /// 每次呼叫都計入 [`LmScorer::queries`]；`P_bi > 0` 時也計入
    /// [`LmScorer::hits`]，供 CLI 證明 bigram 真的被查。
    pub fn delta(&self, left: &str, word: &str) -> i32 {
        if self.lm.bigram_count() == 0 {
            return 0;
        }
        self.queries.set(self.queries.get() + 1);
        let p_bi = self.lm.bi(left, word);
        if p_bi > 0.0 {
            self.hits.set(self.hits.get() + 1);
        }
        let p_uni = self.lm.uni(word);
        let mu = self.ctx.bigram_mu;
        let interpolated = mu * p_bi + (1.0 - mu) * p_uni;
        probability_cost(interpolated).saturating_sub(probability_cost(p_uni))
    }
}

impl Scorer for LmScorer<'_> {
    fn connection_cost(&self, prev_pos: u16, next_pos: u16) -> i32 {
        self.ctx.connection.cost(prev_pos, next_pos)
    }

    fn word_cost(
        &self,
        left: Option<&str>,
        surface: &str,
        word_cost: i32,
        source: CandidateSource,
    ) -> i32 {
        // 未知詞邊沒有語言模型成分，不能套內插差額。
        if source != CandidateSource::Lexicon || self.lm.bigram_count() == 0 {
            return word_cost;
        }
        let left = left.unwrap_or(BOS_WORD);
        word_cost.saturating_add(self.delta(left, surface))
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
        assert_eq!(ctx.bigram_mu, 0.7);
        assert_eq!(ctx.connection.fallback(), 1);
        assert_eq!(DEFAULT_CONNECTION_COST, 1);
        assert_eq!(DEFAULT_UNKNOWN_COST, 8);
        assert_eq!(MAX_WORD_UNITS, 8);
        assert_eq!(DEFAULT_BIGRAM_MU, 0.7);
        assert_eq!(BOS_WORD, "<s>");
        assert!(ctx.connection.is_empty());
    }

    /// 測試用的 bigram 模型：固定 unigram 與少量 bigram。
    struct FakeLm {
        uni: f64,
        bigrams: usize,
        pairs: Vec<((&'static str, &'static str), f64)>,
    }

    impl FakeLm {
        fn new(pairs: &[((&'static str, &'static str), f64)]) -> Self {
            Self {
                uni: 0.001,
                bigrams: pairs.len(),
                pairs: pairs.to_vec(),
            }
        }
    }

    impl BigramLm for FakeLm {
        fn uni(&self, _word: &str) -> f64 {
            self.uni
        }

        fn bi(&self, left: &str, word: &str) -> f64 {
            self.pairs
                .iter()
                .find(|((l, w), _)| *l == left && *w == word)
                .map(|(_, p)| *p)
                .unwrap_or(0.0)
        }

        fn bigram_count(&self) -> usize {
            self.bigrams
        }
    }

    #[test]
    fn lm_scorer_interpolates_like_predecessor() {
        let ctx = ScoreContext::default();
        let lm = FakeLm::new(&[(("<s>", "在"), 0.6), (("在", "見"), 0.8)]);
        let scorer = LmScorer::new(&ctx, &lm);
        // μ·P_bi + (1−μ)·P_uni，μ = 0.7。
        assert!((scorer.probability("<s>", "在") - (0.7 * 0.6 + 0.3 * 0.001)).abs() < 1e-12);
        assert!((scorer.probability("在", "見") - (0.7 * 0.8 + 0.3 * 0.001)).abs() < 1e-12);
        // 沒有 bigram 時只剩 (1−μ)·P_uni。
        assert!((scorer.probability("我", "在") - 0.3 * 0.001).abs() < 1e-12);
    }

    #[test]
    fn lm_scorer_delta_is_interpolated_minus_unigram() {
        let ctx = ScoreContext::default();
        let lm = FakeLm::new(&[(("我", "愛"), 0.0044)]);
        let scorer = LmScorer::new(&ctx, &lm);
        let p = 0.7 * 0.0044 + 0.3 * 0.001;
        let expected = probability_cost(p) - probability_cost(0.001);
        assert_eq!(scorer.delta("我", "愛"), expected);
        assert!(scorer.delta("我", "愛") < 0, "常用組合要比 unigram 便宜");
        // 沒有這筆 bigram 時，內插只剩 unigram 的部分。
        let none = probability_cost(0.3 * 0.001) - probability_cost(0.001);
        assert_eq!(scorer.delta("你", "愛"), none);
    }

    #[test]
    fn lm_scorer_without_bigrams_keeps_word_cost() {
        let ctx = ScoreContext::default();
        let lm = FakeLm {
            uni: 0.001,
            bigrams: 0,
            pairs: Vec::new(),
        };
        let scorer = LmScorer::new(&ctx, &lm);
        assert_eq!(scorer.delta("<s>", "我"), 0);
        assert_eq!(
            scorer.word_cost(None, "我", 1234, CandidateSource::Lexicon),
            1234
        );
    }

    #[test]
    fn lm_scorer_skips_unknown_edges() {
        let ctx = ScoreContext::default();
        let lm = FakeLm::new(&[(("我", "ㄞˋ"), 0.9)]);
        let scorer = LmScorer::new(&ctx, &lm);
        // 未知詞邊的詞面是讀音，沒有語言模型成分。
        assert_eq!(
            scorer.word_cost(Some("我"), "ㄞˋ", 30000, CandidateSource::Literal),
            30000
        );
    }

    #[test]
    fn lm_scorer_counts_queries_and_hits() {
        let ctx = ScoreContext::default();
        let lm = FakeLm::new(&[(("我", "愛"), 0.0044)]);
        let scorer = LmScorer::new(&ctx, &lm);
        assert_eq!((scorer.queries(), scorer.hits()), (0, 0));
        scorer.delta("我", "愛");
        scorer.delta("我", "艾");
        assert_eq!(scorer.queries(), 2);
        assert_eq!(scorer.hits(), 1);
    }

    #[test]
    fn mu_zero_reproduces_unigram_costs() {
        let mut ctx = ScoreContext::default();
        ctx.bigram_mu = 0.0;
        let lm = FakeLm::new(&[(("我", "愛"), 0.9)]);
        let scorer = LmScorer::new(&ctx, &lm);
        assert_eq!(scorer.delta("我", "愛"), 0);
        assert_eq!(
            scorer.word_cost(Some("我"), "愛", 5000, CandidateSource::Lexicon),
            5000
        );
    }

    #[test]
    fn probability_cost_matches_log_scale() {
        assert_eq!(probability_cost(1.0), 0);
        assert_eq!(probability_cost(std::f64::consts::E.recip()), 1000);
        assert_eq!(probability_cost(0.0), 100_000);
        assert_eq!(probability_cost(f64::NAN), 100_000);
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
