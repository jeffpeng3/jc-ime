//! 日文詞層級的神經重排（需要 `onnx` feature）。
//!
//! 模型是同一讀音多個候選詞的 cross-encoder：訓練資料每組共用左右文、
//! 只換 candidate slot。這裡照同樣的用法，在 lattice 的節點層對「同讀音
//! 多詞面」整批打分，分數轉成成本增量後交給 Viterbi，統計成本原樣保留。
//!
//! 上下文切法沿用前身 `core/ja/rerank`：左文取靠近游標的最後
//! [`CONTEXT_TOKENS_PER_SIDE`] 個 token（前身 `MaxLeftContextUnits` 的尾部
//! 裁切）。前身部署沒有 lookahead，右文只保留在介面上、不進張量。
//!
//! 融合公式與前身 `core/ja/rerank/config.go` 的預設權重一致
//! （`Weights{Neural: 1, Stats: 0.2, Draft: 0.1}`），實作在
//! [`super::decode`]（`fuse_word_scores`），因為它不需要 ONNX：
//!
//! ```text
//! stats_i  = (max_cost - cost_i) / (max_cost - min_cost)
//! fused_i  = NEURAL * raw_i + STATS * stats_i + if i == 0 { DRAFT } else { 0 }
//! delta_i  = (max_fused - fused_i) * WORD_SCORE_SCALE
//! ```
//!
//! 神經分數是模型輸出的原始分數，不做批內正規化。任何錯誤或逾時都回
//! `None`，該組候選保留統計成本。

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use jc_model::layout::candidate_piece;
use jc_model::manifest::ModelSpec;
use jc_model::{ModelError, ModelSet, Reranker, Role};

use super::decode::{JaReranker, WORD_RERANK_MAX_CANDIDATES};

/// 單側 context token 上限；總序列長度 124。
pub const CONTEXT_TOKENS_PER_SIDE: usize = 60;
/// 預設單次轉換預算；超過就整批放棄重排。
pub const RERANK_TIMEOUT: Duration = Duration::from_millis(100);

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
    /// 觸發重排的轉換次數。
    pub calls: u64,
    /// 送進模型的候選群組數。
    pub groups: u64,
    /// 成功取得分數的群組數。
    pub scored: u64,
    /// 重排後第一名與統計第一名不同的群組數。
    pub changed: u64,
    /// 超過預算、整批放棄的次數。
    pub timeouts: u64,
    /// 推論失敗的次數。
    pub errors: u64,
    /// 每次群組推論延遲的 p50，毫秒。
    pub p50_ms: f64,
    /// 每次群組推論延遲的 p95，毫秒。
    pub p95_ms: f64,
}

#[derive(Debug, Default)]
struct StatsInner {
    calls: u64,
    groups: u64,
    scored: u64,
    changed: u64,
    timeouts: u64,
    errors: u64,
    latencies: Vec<f64>,
}

/// 由 `jc-model` 驅動的日文詞層級重排器。
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
            groups: inner.groups,
            scored: inner.scored,
            changed: inner.changed,
            timeouts: inner.timeouts,
            errors: inner.errors,
            p50_ms: percentile(&latencies, 50.0),
            p95_ms: percentile(&latencies, 95.0),
        }
    }

    /// 清空累計統計。
    pub fn reset_stats(&self) {
        *self.lock() = StatsInner::default();
    }

    /// 單次轉換的推論預算；不超過 [`RERANK_TIMEOUT`]。
    pub fn timeout(&self) -> Duration {
        self.model.spec().timeout().min(RERANK_TIMEOUT)
    }

    /// 取靠近游標、最多 `max` 個 token 的左文。
    ///
    /// 先從尾端逐字往前擴，直到編碼超過上限；保留最後一個合法後綴。
    /// 模型端仍會再做一次右對齊與裁切，這裡先縮小輸入可避免它留下
    /// 離游標最遠的那一段。
    fn clip_left(&self, text: &str, max: usize) -> String {
        left_context(self.model.tokenizer(), text, max)
    }

    /// 取游標後、最多 `max` 個 token 的右文。
    fn clip_right(&self, text: &str, max: usize) -> String {
        right_context(self.model.tokenizer(), text, max)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, StatsInner> {
        self.stats
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn note_timeout(&self, group_timed_out: bool) {
        let mut inner = self.lock();
        if group_timed_out {
            inner.timeouts += 1;
        } else {
            inner.errors += 1;
        }
    }
}

impl JaReranker for NeuralReranker {
    fn score_words(&self, left: &str, right: &str, candidates: &[&str]) -> Option<Vec<f32>> {
        if candidates.len() < 2 || candidates.len() > WORD_RERANK_MAX_CANDIDATES {
            return None;
        }
        // 模型只有一個 candidate slot，只接受單 piece 詞面；其餘留給統計。
        let scorables: Vec<usize> = (0..candidates.len())
            .filter(|&index| candidate_piece(self.model.tokenizer(), candidates[index]).is_some())
            .collect();
        if scorables.len() < 2 {
            return None;
        }
        let packed: Vec<&str> = scorables.iter().map(|&index| candidates[index]).collect();
        let max_context = self
            .model
            .spec()
            .context_tokens()
            .min(CONTEXT_TOKENS_PER_SIDE);
        let left = self.clip_left(left, max_context);
        // 空字串代表沒有左文，交給模型端放 BOS 標記。
        let left = (!left.is_empty()).then_some(left.as_str());
        let right = self.clip_right(right, max_context);
        let right = (!right.is_empty()).then_some(right.as_str());
        let started = Instant::now();
        self.lock().groups += 1;

        let scores = match self
            .model
            .try_score_batch(left, right.unwrap_or(""), &packed)
        {
            Ok(scores) if scores.len() == packed.len() => scores,
            Ok(scores) => {
                log::debug!(
                    "日文 reranker 分數長度 {} 與候選 {} 不符",
                    scores.len(),
                    packed.len()
                );
                self.note_timeout(false);
                return None;
            }
            Err(err) => {
                let elapsed = started.elapsed();
                if elapsed >= self.timeout() {
                    self.note_timeout(true);
                } else {
                    log::debug!("日文 reranker 推論失敗，保留統計成本：{err}");
                    self.note_timeout(false);
                }
                return None;
            }
        };
        let mut out = vec![f32::NAN; candidates.len()];
        for (slot, &index) in scorables.iter().enumerate() {
            out[index] = scores[slot];
        }
        let elapsed = ms(started.elapsed());
        let mut inner = self.lock();
        inner.scored += 1;
        inner.latencies.push(elapsed);
        Some(out)
    }

    fn begin(&self) {
        self.lock().calls += 1;
    }

    fn end(&self, timed_out: bool) {
        if timed_out {
            self.lock().timeouts += 1;
        }
    }

    fn note_changed(&self) {
        self.lock().changed += 1;
    }

    fn timeout(&self) -> Duration {
        NeuralReranker::timeout(self)
    }
}

/// 取靠近游標、最多 `max` 個 token 的左文。
///
/// 先從尾端逐字往前擴，直到編碼超過上限；保留最後一個合法後綴。
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

/// 取游標後、最多 `max` 個 token 的右文；保留最靠近游標的前段。
fn right_context(tokenizer: &jc_model::Tokenizer, text: &str, max: usize) -> String {
    if max == 0 || text.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut end = 0;
    for index in 0..chars.len() {
        let candidate: String = chars[..=index].iter().collect();
        if tokenizer.encode(&candidate).len() > max {
            break;
        }
        end = index + 1;
    }
    chars[..end].iter().collect()
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
    fn right_context_keeps_the_prefix_nearest_the_cursor() {
        let tokenizer = tokenizer();
        assert_eq!(right_context(&tokenizer, "東京都", 2), "東京都");
        assert_eq!(right_context(&tokenizer, "東京都東京", 1), "東京");
        assert_eq!(right_context(&tokenizer, "", 60), "");
    }

    #[test]
    fn stats_start_at_zero() {
        let stats = RerankStats::default();
        assert_eq!(stats.calls, 0);
        assert_eq!(stats.groups, 0);
        assert_eq!(stats.p95_ms, 0.0);
    }
}
