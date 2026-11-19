//! 日文 N-best 的神經重排（需要 `onnx` feature）。
//!
//! 接在 [`JaDecoder`](super::decode::JaDecoder) 的統計候選之後：讀音不變，
//! 只把同一組候選依模型分數重新排序。模型缺席、逾時或推論錯誤時原順序
//! 返回，轉換結果與沒開 feature 時完全相同。
//!
//! 上下文切法沿用前身 `core/ja/rerank`：左文取靠近游標的最後 60 個
//! token（前身 `MaxLeftContextUnits` 的尾部裁切），序列長度 124。沒有左文
//! 時模型端用 BOS 佔住緊鄰分隔符的 slot，不留空。前身部署沒有 lookahead，
//! 右文只保留在介面上、不進張量；這裡同樣只送左文。
//!
//! 分數公式沿用前身 `core/ja/rerank/config.go` 的預設權重
//! （`Weights{Neural: 1, Stats: 0.2, Draft: 0.1}`）：
//!
//! ```text
//! stats_i  = (score_i - min_score) / (max_score - min_score)
//! final_i  = NEURAL_WEIGHT * raw_i + STATS_WEIGHT * stats_i
//!          + if i == 0 { DRAFT_BONUS } else { 0 }
//! ```
//!
//! 神經分數是模型輸出的原始分數，不做批內正規化；前身的驗證器直接回
//! 長度正規化 logprob，`Neural` 乘的就是原值。統計分數以本批候選的範圍
//! 正規化到 `[0, 1]`，只在神經分數接近時拆解平手。同分時保留原本順序，
//! 讓輸出可重現。`span == 0` 時正規化給 0.5。

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use jc_model::manifest::ModelSpec;
use jc_model::{ModelError, ModelSet, Reranker, Role};

use super::decode::{JaContext, JaReranker, JaResult};

/// 一次送進重排的候選數；與 `ja/rules.json` 的 `top_n` 一致。
pub const RERANK_NBEST: usize = 5;
/// 單側 context token 上限；總序列長度 124。
pub const CONTEXT_TOKENS_PER_SIDE: usize = 60;
/// 預設推論預算；超過就放棄重排。
pub const RERANK_TIMEOUT: Duration = Duration::from_millis(100);
/// 神經分數的融合權重；對齊前身 `Weights.Neural`。
pub const NEURAL_WEIGHT: f64 = 1.0;
/// 統計分數的融合權重；對齊前身 `Weights.Stats`。
pub const STATS_WEIGHT: f64 = 0.2;
/// 統計第一名的固定加分；對齊前身 `Weights.Draft`。
pub const DRAFT_BONUS: f64 = 0.1;

/// 重排模型載入或設定的錯誤。
#[derive(Debug, thiserror::Error)]
pub enum RerankError {
    /// 模型載入失敗。
    #[error("reranker 載入失敗：{0}")]
    Model(#[from] ModelError),
    /// 模型集合沒有可用的 `ja-rerank`。
    #[error("沒有可用的 ja-rerank 模型：{0}")]
    Unavailable(String),
}

/// 重排的累計統計，不含輸入文字。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RerankStats {
    /// 呼叫次數。
    pub calls: u64,
    /// 成功取得分數的次數。
    pub scored: u64,
    /// 超過 100 ms 預算的次數。
    pub timeouts: u64,
    /// 推論失敗的次數。
    pub errors: u64,
    /// 重排後第一名與統計第一名不同的次數。
    pub changed: u64,
    /// 每次呼叫延遲的 p50，毫秒。
    pub p50_ms: f64,
    /// 每次呼叫延遲的 p95，毫秒。
    pub p95_ms: f64,
}

#[derive(Debug, Default)]
struct StatsInner {
    calls: u64,
    scored: u64,
    timeouts: u64,
    errors: u64,
    changed: u64,
    latencies: Vec<f64>,
}

/// 由 `jc-model` 驅動的日文候選重排器。
///
/// 同一個值可以同時服務多個轉換請求；內部 session 用鎖序列化推論。
pub struct NeuralReranker {
    model: Reranker,
    stats: Mutex<StatsInner>,
}

impl NeuralReranker {
    /// 以預設佈局載入角色目錄。
    pub fn load(dir: impl AsRef<Path>) -> Result<Self, RerankError> {
        Self::load_with_ort_lib(dir, None)
    }

    /// 由模型集合根目錄的 `manifest.json` 找出 `ja-rerank` 並載入。
    ///
    /// 使用 manifest 內的 `ModelSpec`（序列長度、slot、執行緒數與逾時），
    /// 不是預設值。目錄或 manifest 缺席時回 [`RerankError::Unavailable`]，
    /// 不 panic。
    pub fn load_from_models(models: impl AsRef<Path>) -> Result<Self, RerankError> {
        let set = ModelSet::load(models)?;
        let dir = set.model_dir(Role::JaRerank).ok_or_else(|| {
            RerankError::Unavailable(format!("{:?}", set.availability(Role::JaRerank)))
        })?;
        let spec = set.spec(Role::JaRerank).cloned().unwrap_or_default();
        Self::load_with_spec(dir, &spec, None)
    }

    /// 以預設佈局載入，並指定 ONNX Runtime 動態庫。
    ///
    /// `ort_lib` 為 `None` 時使用 `ORT_DYLIB_PATH`。
    pub fn load_with_ort_lib(
        dir: impl AsRef<Path>,
        ort_lib: Option<&Path>,
    ) -> Result<Self, RerankError> {
        Self::load_with_spec(dir, &ModelSpec::default(), ort_lib)
    }

    /// 以指定佈局載入，並指定 ONNX Runtime 動態庫。
    pub fn load_with_spec(
        dir: impl AsRef<Path>,
        spec: &ModelSpec,
        ort_lib: Option<&Path>,
    ) -> Result<Self, RerankError> {
        let model = Reranker::load_with_spec(dir, spec, ort_lib)?;
        Ok(Self {
            model,
            stats: Mutex::new(StatsInner::default()),
        })
    }

    /// 目前累計統計。
    pub fn stats(&self) -> RerankStats {
        let inner = self.lock();
        let mut latencies = inner.latencies.clone();
        latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        RerankStats {
            calls: inner.calls,
            scored: inner.scored,
            timeouts: inner.timeouts,
            errors: inner.errors,
            changed: inner.changed,
            p50_ms: percentile(&latencies, 50.0),
            p95_ms: percentile(&latencies, 95.0),
        }
    }

    /// 清空累計統計。
    pub fn reset_stats(&self) {
        *self.lock() = StatsInner::default();
    }

    /// 單次推論預算；不超過 [`RERANK_TIMEOUT`]。
    pub fn timeout(&self) -> Duration {
        self.model.spec().timeout().min(RERANK_TIMEOUT)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, StatsInner> {
        self.stats
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn note_timeout(&self, elapsed: f64) {
        let mut inner = self.lock();
        inner.timeouts += 1;
        inner.latencies.push(elapsed);
    }

    fn note_error(&self, elapsed: f64) {
        let mut inner = self.lock();
        inner.errors += 1;
        inner.latencies.push(elapsed);
    }
}

impl JaReranker for NeuralReranker {
    fn rerank(&self, _reading: &[char], drafts: Vec<JaResult>, ctx: &JaContext) -> Vec<JaResult> {
        if drafts.len() < 2 {
            return drafts;
        }
        if drafts.iter().any(|draft| draft.surface().is_empty()) {
            return drafts;
        }
        let surfaces: Vec<String> = drafts.iter().map(JaResult::surface).collect();
        let candidates: Vec<&str> = surfaces.iter().map(String::as_str).collect();
        let max_context = self
            .model
            .spec()
            .context_tokens()
            .min(CONTEXT_TOKENS_PER_SIDE);
        let left = left_context(self.model.tokenizer(), &ctx.left, max_context);
        // 空字串代表沒有左文，交給模型端放 BOS 標記。
        let left = (!left.is_empty()).then_some(left.as_str());
        let started = Instant::now();
        self.lock().calls += 1;

        let scores = match self.model.try_score_batch(left, &candidates) {
            Ok(scores) => scores,
            Err(err) => {
                let elapsed = started.elapsed();
                if elapsed >= self.timeout() {
                    self.note_timeout(ms(elapsed));
                } else {
                    log::debug!("日文 reranker 推論失敗，保留統計順序：{err}");
                    self.note_error(ms(elapsed));
                }
                return drafts;
            }
        };
        if scores.len() != drafts.len() {
            self.note_error(ms(started.elapsed()));
            return drafts;
        }
        let stats: Vec<f64> = drafts.iter().map(|draft| draft.score).collect();
        let fused = fuse_scores(&scores, &stats);
        let order = order_by_scores(&fused);
        let changed = order.first().is_some_and(|&index| index != 0);

        let mut indexed: Vec<Option<JaResult>> = drafts.into_iter().map(Some).collect();
        let mut out = Vec::with_capacity(indexed.len());
        for index in order {
            if let Some(draft) = indexed[index].take() {
                out.push(draft);
            }
        }

        let elapsed = ms(started.elapsed());
        {
            let mut inner = self.lock();
            inner.scored += 1;
            inner.latencies.push(elapsed);
            if changed {
                inner.changed += 1;
            }
        }
        out
    }
}

/// 把統計分數與神經分數融合成最終排序分數。
///
/// 神經分數直接用模型原值：前身的驗證器回長度正規化 logprob，`Neural`
/// 乘的就是原值，不對同一批候選再做 [0, 1] 正規化。
fn fuse_scores(neural: &[f32], stats: &[f64]) -> Vec<f64> {
    let stats_norm = normalize(stats);
    let mut fused = Vec::with_capacity(neural.len());
    for index in 0..neural.len().min(stats_norm.len()) {
        let draft = if index == 0 { DRAFT_BONUS } else { 0.0 };
        fused.push(
            NEURAL_WEIGHT * f64::from(neural[index]) + STATS_WEIGHT * stats_norm[index] + draft,
        );
    }
    fused
}

/// 最小值／最大值正規化到 `[0, 1]`；所有值相同時給 0.5。
fn normalize(values: &[f64]) -> Vec<f64> {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for &value in values {
        min = min.min(value);
        max = max.max(value);
    }
    let span = max - min;
    values
        .iter()
        .map(|&value| {
            if span > 0.0 && span.is_finite() {
                (value - min) / span
            } else {
                0.5
            }
        })
        .collect()
}

/// 依最終分數排出候選索引；同分保留原本順序。
fn order_by_scores(scores: &[f64]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..scores.len()).collect();
    order.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]).then_with(|| a.cmp(&b)));
    order
}

/// 取靠近游標、最多 `max` 個 token 的左文。
///
/// 先從尾端逐字往前擴，直到編碼超過上限；保留最後一個合法後綴。
/// 模型端仍會再做一次右對齊與裁切，這裡先縮小輸入可避免它留下
/// 離游標最遠的那一段。
fn left_context(tokenizer: &jc_model::Tokenizer, text: &str, max: usize) -> String {
    if max == 0 || text.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut start = chars.len();
    for index in (0..chars.len()).rev() {
        let candidate: String = chars[index..].iter().collect();
        if tokenizer.encode(&candidate).len() > max {
            break;
        }
        start = index;
    }
    chars[start..].iter().collect()
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// 最近排名分位數；空輸入回傳 0。
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[index]
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc_model::tokenizer::VOCAB_SCHEMA;
    use jc_model::Tokenizer;

    fn tokenizer() -> Tokenizer {
        let mut pieces = vec![
            "<pad>".to_string(),
            "<unk>".to_string(),
            "<s>".to_string(),
            "</s>".to_string(),
        ];
        for b in 0..=255u16 {
            pieces.push(format!("<0x{b:02X}>"));
        }
        for piece in ["▁", "東", "京", "都", "▁東", "▁東京", "▁東京都"] {
            pieces.push(piece.to_string());
        }
        let json = serde_json::json!({
            "format": VOCAB_SCHEMA,
            "vocab_size": pieces.len(),
            "special_tokens": {"pad": 0, "unk": 1, "bos": 2, "eos": 3},
            "id_to_piece": pieces,
        });
        Tokenizer::from_json(&serde_json::to_vec(&json).unwrap()).unwrap()
    }

    #[test]
    fn percentile_is_nearest_rank() {
        assert_eq!(percentile(&[], 50.0), 0.0);
        assert_eq!(percentile(&[1.0, 2.0, 3.0, 4.0], 50.0), 2.0);
        assert_eq!(percentile(&[1.0, 2.0, 3.0, 4.0], 95.0), 4.0);
    }

    #[test]
    fn higher_scores_come_first_and_ties_keep_order() {
        assert_eq!(order_by_scores(&[0.1, 0.9, 0.5]), vec![1, 2, 0]);
        assert_eq!(order_by_scores(&[0.5, 0.5, 0.1]), vec![0, 1, 2]);
        assert_eq!(order_by_scores(&[]), Vec::<usize>::new());
    }

    #[test]
    fn fusion_follows_predecessor_weights() {
        // 神經原值乘 1.0，統計正規化後乘 0.2，第一名再加 0.1。
        let fused = fuse_scores(&[1.0, 2.0], &[-20.0, -24.0]);
        assert!((fused[0] - 1.3).abs() < 1e-9, "{fused:?}");
        assert!((fused[1] - 2.0).abs() < 1e-9, "{fused:?}");
    }

    #[test]
    fn draft_bonus_only_breaks_near_ties() {
        // 神經差距 0.05 小於 draft 加分 0.1，統計第一名保留。
        let fused = fuse_scores(&[0.0, 0.05], &[-20.0, -20.0]);
        assert_eq!(order_by_scores(&fused), vec![0, 1]);
        // 神經差距 0.5 明顯大於統計項與 draft 加分，模型可以翻盤。
        let fused = fuse_scores(&[0.0, 0.5], &[-20.0, -20.0]);
        assert_eq!(order_by_scores(&fused), vec![1, 0]);
    }

    #[test]
    fn stats_normalization_breaks_neural_ties() {
        // 神經同分時，統計分數高的在前；同分再保留原順序。
        let fused = fuse_scores(&[0.4, 0.4, 0.4], &[-24.0, -20.0, -22.0]);
        assert_eq!(order_by_scores(&fused), vec![1, 0, 2]);
    }

    #[test]
    fn normalize_handles_flat_values() {
        assert_eq!(normalize(&[]), Vec::<f64>::new());
        assert_eq!(normalize(&[3.0, 3.0]), vec![0.5, 0.5]);
        assert_eq!(normalize(&[1.0, 3.0]), vec![0.0, 1.0]);
    }

    #[test]
    fn left_context_keeps_the_suffix_nearest_the_cursor() {
        let tokenizer = tokenizer();
        // 「▁東京都」是一個 token，所以 2 個 token 上限保留整個詞。
        assert_eq!(left_context(&tokenizer, "東京都", 2), "東京都");
        // 上限 1 時只留最後一個 token；「東京」是最靠近游標的合法後綴。
        let one = left_context(&tokenizer, "東京都東京", 1);
        assert_eq!(one, "東京");
        assert!(tokenizer.encode(&one).len() <= 1);
        assert_eq!(left_context(&tokenizer, "", 60), "");
    }

    #[test]
    fn stats_start_at_zero() {
        let stats = RerankStats::default();
        assert_eq!(stats.calls, 0);
        assert_eq!(stats.p95_ms, 0.0);
    }
}
