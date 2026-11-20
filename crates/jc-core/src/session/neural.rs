//! 日文神經轉換（Jinen）的接線、門檻與採用驗證。
//!
//! 統計轉換永遠先算好；只有 `top1` 與 `top2` 的成本差小於門檻時才問模型。
//! 模型輸出要通過五條驗證才替換統計結果，任何失敗、逾時或模型缺席都回到
//! 統計結果。這一層不認得 candle 或 GGUF：模型由 [`NeuralEngine`] 注入，
//! 測試可以用替身直接驗證門檻與驗證邏輯。
//!
//! 驗證順序照 Mozkey 的採用流程：
//!
//! 1. 空輸出直接拒絕。
//! 2. 與統計結果相同時視為「無變化」，直接接受但不替換。
//! 3. 長度不得超過 `reading_len * 3 + 8` 個字元。
//! 4. 左文脈尾端與輸出的開頭重疊不得構成 echo。
//! 5. 絵文字 cluster 序列必須與統計結果完全相同。
//! 6. 比統計結果短，且是統計結果的前綴或後綴，視為明顯截斷。

use std::collections::BTreeMap;
use std::fmt;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use thiserror::Error;

/// 預設的成本差門檻；低呼叫版，由 `gating-lowcall` 的 dev 掃描結果決定。
pub const DEFAULT_GAP_THRESHOLD: f32 = 472.5;

/// 品質優先版的成本差門檻；沿用 `jinen-gating-v2` 的 small 掃描結果。
pub const QUALITY_GAP_THRESHOLD: f32 = 460.0;

/// 預設的平均候選數下限；只有 `mean_candidates` 大於這個值才呼叫模型。
pub const DEFAULT_MIN_MEAN_CANDIDATES: f32 = 5.98;

/// 預設的推論預算，對齊 Mozkey 的互動上限。
pub const DEFAULT_TIMEOUT_MS: u64 = 180;

/// 左文脈預設取幾個字元。
pub const DEFAULT_LEFT_CONTEXT_CHARS: usize = 24;

/// 右文脈預設取幾個字元。
pub const DEFAULT_RIGHT_CONTEXT_CHARS: usize = 6;

/// 長度驗證的寬容倍率與固定加項，對齊 Mozkey `key_len * 3 + 8`。
const LENGTH_FACTOR: usize = 3;
const LENGTH_SLACK: usize = 8;

/// context echo 至少要重疊幾個字元才納入判斷。
const ECHO_MIN_OVERLAP: usize = 4;

/// 神經轉換使用的模型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NeuralModel {
    /// Jinen v2 xsmall。
    Xsmall,
    /// Jinen v2 small；預設模型。
    #[default]
    Small,
}

impl NeuralModel {
    /// 模型名稱，與 CLI 的 `xsmall`／`small` 一致。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Xsmall => "xsmall",
            Self::Small => "small",
        }
    }
}

/// 左右文脈各取幾個字元。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextChars {
    /// 左文脈由尾端取幾個字元。
    pub left: usize,
    /// 右文脈由開頭取幾個字元。
    pub right: usize,
}

impl Default for ContextChars {
    fn default() -> Self {
        Self {
            left: DEFAULT_LEFT_CONTEXT_CHARS,
            right: DEFAULT_RIGHT_CONTEXT_CHARS,
        }
    }
}

/// 日文神經轉換設定。
///
/// `runtime` 為 `None` 代表模型沒有載入：行為與關閉完全相同，不報錯、
/// 不增加延遲。`enabled` 是 `--neural off` 的開關，關閉時連門檻都不算。
///
/// 預設是低呼叫版：`gap < 472.5` 且 `mean_candidates > 5.98` 才呼叫模型。
/// 品質優先版用 [`NeuralConfig::quality`]。
#[derive(Clone)]
pub struct NeuralConfig {
    /// 是否啟用神經轉換；預設開啟。
    pub enabled: bool,
    /// `top1`／`top2` 的成本差門檻；小於這個值才呼叫模型。
    pub gap_threshold: f32,
    /// top-1 路徑平均候選數下限；大於這個值才呼叫模型。
    ///
    /// 設成 `0.0` 等於不檢查，品質優先版就是這樣。
    pub min_mean_candidates: f32,
    /// 使用的模型。
    pub model: NeuralModel,
    /// 推論預算；超過就沿用統計結果。
    pub timeout_ms: u64,
    /// 左右文脈長度上限。
    pub max_context_chars: ContextChars,
    /// 停頓後補正的額外嚴格倍率；套用在 `gap_threshold` 上。
    ///
    /// 補正會改寫使用者已看過的尾巴，只接受比即時轉換更確定的片段。
    /// `1.0` 表示與即時轉換同一門檻。
    pub correction_gap_ratio: f32,
    /// 已載入的模型執行環境；`None` 時不呼叫模型。
    pub runtime: Option<Arc<NeuralRuntime>>,
}

impl Default for NeuralConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            gap_threshold: DEFAULT_GAP_THRESHOLD,
            min_mean_candidates: DEFAULT_MIN_MEAN_CANDIDATES,
            model: NeuralModel::Small,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            max_context_chars: ContextChars::default(),
            correction_gap_ratio: 0.5,
            runtime: None,
        }
    }
}

impl NeuralConfig {
    /// 品質優先版：只檢查 `gap < 460`，不看平均候選數。
    ///
    /// 用於 `--neural quality`；重現 `jinen-gating-v2` 的
    /// 93.6／80.0／71.5，代價是呼叫率與尾端延遲較高。
    pub fn quality() -> Self {
        Self {
            gap_threshold: QUALITY_GAP_THRESHOLD,
            min_mean_candidates: 0.0,
            ..Self::default()
        }
    }

    /// 關閉版：不呼叫模型，也不做門檻判斷。
    pub fn off() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
}

impl fmt::Debug for NeuralConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NeuralConfig")
            .field("enabled", &self.enabled)
            .field("gap_threshold", &self.gap_threshold)
            .field("min_mean_candidates", &self.min_mean_candidates)
            .field("model", &self.model)
            .field("timeout_ms", &self.timeout_ms)
            .field("max_context_chars", &self.max_context_chars)
            .field("correction_gap_ratio", &self.correction_gap_ratio)
            .field("runtime", &self.runtime.is_some())
            .finish()
    }
}

/// 一次神經轉換的請求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeuralRequest {
    /// かな讀音。
    pub reading: String,
    /// 左文脈；呼叫端已依設定截斷。
    pub left: String,
    /// 右文脈；呼叫端已依設定截斷。
    pub right: String,
}

/// 神經轉換失敗的原因。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NeuralError {
    /// 模型不可用或 worker 已結束。
    #[error("模型不可用：{0}")]
    Unavailable(String),
    /// 推論本身失敗。
    #[error("推論失敗：{0}")]
    Inference(String),
    /// 超過推論預算。
    #[error("推論逾時")]
    Timeout,
}

/// 模型後端；正式實作在 `jc-engine`，測試用替身。
pub trait NeuralEngine: Send {
    /// 把讀音轉成候選字串。
    fn convert(&mut self, request: &NeuralRequest) -> Result<String, NeuralError>;
}

/// 驗證拒絕原因；字串形式直接寫進統計與日誌。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RejectReason {
    /// 輸出是空字串。
    Empty,
    /// 超過 `reading_len * 3 + 8`。
    TooLong,
    /// 輸出重複了左文脈。
    ReadingEcho,
    /// 絵文字 cluster 序列改變。
    EmojiChanged,
    /// 比統計結果少了明顯的首或尾。
    Truncated,
}

impl RejectReason {
    /// 穩定的統計名稱。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::TooLong => "too_long",
            Self::ReadingEcho => "reading_echo",
            Self::EmojiChanged => "emoji_changed",
            Self::Truncated => "truncated",
        }
    }

    /// 全部原因，供統計輸出固定順序。
    pub const ALL: [Self; 5] = [
        Self::Empty,
        Self::TooLong,
        Self::ReadingEcho,
        Self::EmojiChanged,
        Self::Truncated,
    ];
}

impl fmt::Display for RejectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 通過基本驗證後的結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Validation {
    /// 輸出等於統計結果，不需要替換。
    Unchanged,
    /// 輸出可以替換統計結果。
    Accepted,
}

/// 一次轉換的輸入側資訊。
#[derive(Debug, Clone, Copy)]
pub struct NeuralInput<'a> {
    /// かな讀音。
    pub reading: &'a str,
    /// 左文脈（未截斷）。
    pub left: &'a str,
    /// 右文脈（未截斷）。
    pub right: &'a str,
    /// 統計轉換的第一名。
    pub statistical: &'a str,
    /// 統計第一名的原始成本。
    pub statistical_cost: i32,
    /// 統計第二名的原始成本；`None` 代表沒有備案，一定觸發。
    pub runner_up_cost: Option<i32>,
    /// top-1 路徑上每個詞段（同讀音去重後）的平均候選詞面數。
    ///
    /// 必須在統計轉換完成時算好，不能在等候模型的路徑上才算。
    pub mean_candidates: f32,
}

/// 一次轉換的決定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NeuralDecision {
    /// 關閉或門檻未通過。
    Skipped,
    /// 模型沒有載入或 worker 不在。
    Unavailable,
    /// 超過推論預算。
    Timeout,
    /// 模型輸出沒有通過驗證。
    Rejected(RejectReason),
    /// 模型輸出等於統計結果。
    Unchanged,
    /// 採用模型輸出。
    Accepted {
        /// 要替換成的字串。
        surface: String,
    },
}

impl NeuralDecision {
    /// 要替換成的字串；沒有替換時為 `None`。
    pub fn accepted_surface(&self) -> Option<&str> {
        match self {
            Self::Accepted { surface } => Some(surface),
            _ => None,
        }
    }
}

/// 驗證模型輸出。
///
/// 只做採用條件，不呼叫模型；測試與事後分析可以直接用。
pub fn validate(candidate: &str, input: &NeuralInput<'_>) -> Result<Validation, RejectReason> {
    if candidate.is_empty() {
        return Err(RejectReason::Empty);
    }
    if candidate == input.statistical {
        return Ok(Validation::Unchanged);
    }
    let reading_len = input.reading.chars().count();
    let limit = reading_len
        .saturating_mul(LENGTH_FACTOR)
        .saturating_add(LENGTH_SLACK);
    if candidate.chars().count() > limit {
        return Err(RejectReason::TooLong);
    }
    if context_echo(candidate, input) {
        return Err(RejectReason::ReadingEcho);
    }
    if emoji_clusters(candidate) != emoji_clusters(input.statistical) {
        return Err(RejectReason::EmojiChanged);
    }
    if truncated(candidate, input.statistical) {
        return Err(RejectReason::Truncated);
    }
    Ok(Validation::Accepted)
}

/// 左文脈尾端是否被模型回吐。
///
/// 重疊不到 4 個字元不算；達到 4 個字元後，還要超過 Mozkey 的門檻才算
/// echo：`overlap >= max(1, max(reading_len, baseline_len)) * 2 + 1`，或
/// 輸出長度至少是「重疊＋統計結果長度」。
fn context_echo(candidate: &str, input: &NeuralInput<'_>) -> bool {
    let left = input.left;
    if left.is_empty() {
        return false;
    }
    let output: Vec<char> = candidate.chars().collect();
    let left_chars: Vec<char> = left.chars().collect();
    let max = left_chars.len().min(output.len());
    let mut overlap = 0usize;
    for len in (ECHO_MIN_OVERLAP..=max).rev() {
        if left_chars[left_chars.len() - len..] == output[..len] {
            overlap = len;
            break;
        }
    }
    if overlap < ECHO_MIN_OVERLAP {
        return false;
    }
    let baseline_len = input.statistical.chars().count();
    let reading_len = input.reading.chars().count();
    let threshold = reading_len.max(baseline_len).max(1) * 2 + 1;
    overlap >= threshold || output.len() >= overlap + baseline_len
}

/// 輸出是否比統計結果短，而且只留下頭或尾。
fn truncated(candidate: &str, baseline: &str) -> bool {
    let candidate_len = candidate.chars().count();
    let baseline_len = baseline.chars().count();
    if candidate_len >= baseline_len {
        return false;
    }
    baseline.starts_with(candidate) || baseline.ends_with(candidate)
}

/// 抽出有序的絵文字 cluster 序列。
///
/// cluster 會把 ZWJ、膚色修飾、variation selector 與 keycap 一起帶走，
/// 因此「同一個絵文字換膚色」也會被視為序列改變。
pub fn emoji_clusters(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut clusters = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        if !is_emoji_base(chars[index]) {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        loop {
            if index >= chars.len() {
                break;
            }
            let ch = chars[index];
            // ZWJ 必須先把下一個本體一起吃進來，不能當成一般修飾字元，
            // 否則 👨‍👩‍👧 會被拆成三個 cluster。
            if ch == '\u{200d}' {
                if index + 1 < chars.len() && is_emoji_base(chars[index + 1]) {
                    index += 2;
                    continue;
                }
                break;
            }
            if is_emoji_modifier(ch) {
                index += 1;
                continue;
            }
            break;
        }
        clusters.push(chars[start..index].iter().collect());
    }
    clusters
}

/// 絵文字的本體；修飾字元由 [`is_emoji_modifier`] 處理。
fn is_emoji_base(ch: char) -> bool {
    let code = ch as u32;
    matches!(
        code,
        0x00a9 | 0x00ae | 0x203c | 0x2049 | 0x2122 | 0x2139 | 0x3030 | 0x303d | 0x3297 | 0x3299
    ) || (0x2190..=0x21ff).contains(&code)
        || (0x2600..=0x27bf).contains(&code)
        || (0x2b00..=0x2bff).contains(&code)
        || (0x1f000..=0x1faff).contains(&code)
        || (0x1f1e6..=0x1f1ff).contains(&code)
}

/// 絵文字修飾：膚色、variation selector、keycap 與 ZWJ 本身。
fn is_emoji_modifier(ch: char) -> bool {
    let code = ch as u32;
    ch == '\u{fe0f}' || code == 0x20e3 || (0x1f3fb..=0x1f3ff).contains(&code)
}

/// 執行緒內的請求；回應走自己的 channel，逾時就丟掉。
struct Job {
    request: NeuralRequest,
    reply: mpsc::Sender<Result<String, NeuralError>>,
}

/// 模型 worker 與統計。
///
/// 所有請求都由同一個執行緒處理；佇列中只保留最新的一筆，避免使用者快速
/// 輸入時把過期讀音一個一個算完。呼叫端等超過預算就回 [`NeuralError::Timeout`]。
pub struct NeuralRuntime {
    tx: Mutex<mpsc::Sender<Job>>,
    stats: Arc<Mutex<NeuralStatsSnapshot>>,
}

impl fmt::Debug for NeuralRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NeuralRuntime")
            .field("stats", &self.stats())
            .finish()
    }
}

impl NeuralRuntime {
    /// 啟動 worker；模型已載入時才呼叫。
    pub fn spawn(engine: Box<dyn NeuralEngine>) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        let spawned = std::thread::Builder::new()
            .name("jc-jinen".to_string())
            .spawn(move || {
                let mut engine = engine;
                while let Ok(mut job) = rx.recv() {
                    while let Ok(newer) = rx.try_recv() {
                        job = newer;
                    }
                    let result = engine.convert(&job.request);
                    let _ = job.reply.send(result);
                }
            });
        if let Err(err) = spawned {
            log::warn!("無法啟動 Jinen worker：{err}");
        }
        Self {
            tx: Mutex::new(tx),
            stats: Arc::new(Mutex::new(NeuralStatsSnapshot::default())),
        }
    }

    /// 送一次請求，最多等 `timeout`。
    pub fn convert(
        &self,
        request: &NeuralRequest,
        timeout: Duration,
    ) -> Result<String, NeuralError> {
        let (reply, rx) = mpsc::channel();
        let job = Job {
            request: request.clone(),
            reply,
        };
        self.tx
            .lock()
            .map_err(|_| NeuralError::Unavailable("worker 鎖壞掉".to_string()))?
            .send(job)
            .map_err(|_| NeuralError::Unavailable("worker 已結束".to_string()))?;
        match rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => Err(NeuralError::Timeout),
            Err(RecvTimeoutError::Disconnected) => {
                Err(NeuralError::Unavailable("worker 已結束".to_string()))
            }
        }
    }

    /// 目前的統計快照。
    pub fn stats(&self) -> NeuralStatsSnapshot {
        self.stats
            .lock()
            .map(|stats| stats.clone())
            .unwrap_or_default()
    }

    fn record(&self, decision: &NeuralDecision) {
        let Ok(mut stats) = self.stats.lock() else {
            return;
        };
        match decision {
            NeuralDecision::Skipped => stats.skipped += 1,
            NeuralDecision::Unavailable => stats.unavailable += 1,
            NeuralDecision::Timeout => stats.timeouts += 1,
            NeuralDecision::Unchanged => stats.unchanged += 1,
            NeuralDecision::Accepted { .. } => stats.accepted += 1,
            NeuralDecision::Rejected(reason) => {
                *stats.rejected.entry(*reason).or_insert(0) += 1;
            }
        }
    }
}

/// 神經轉換的累計次數；CLI 在評測結束後印出。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NeuralStatsSnapshot {
    /// 門檻未通過。
    pub skipped: u64,
    /// 模型不可用。
    pub unavailable: u64,
    /// 逾時。
    pub timeouts: u64,
    /// 輸出等於統計結果。
    pub unchanged: u64,
    /// 採用模型輸出。
    pub accepted: u64,
    /// 各驗證原因的拒絕次數。
    pub rejected: BTreeMap<RejectReason, u64>,
}

impl NeuralStatsSnapshot {
    /// 模型實際被呼叫的次數（含逾時與失敗）。
    pub fn calls(&self) -> u64 {
        self.unavailable
            + self.timeouts
            + self.unchanged
            + self.accepted
            + self.rejected.values().sum::<u64>()
    }

    /// 一行摘要；`--neural off` 或模型缺席時為 `None`。
    pub fn summary(&self) -> String {
        let rejected: Vec<String> = self
            .rejected
            .iter()
            .map(|(reason, count)| format!("{} {count}", reason.as_str()))
            .collect();
        format!(
            "neural：calls {}，accepted {}，unchanged {}，skipped {}，timeout {}，\
             unavailable {}，rejected [{}]",
            self.calls(),
            self.accepted,
            self.unchanged,
            self.skipped,
            self.timeouts,
            self.unavailable,
            rejected.join("、")
        )
    }
}

/// 依設定決定是否呼叫模型，並套用五條驗證。
///
/// 這個函式不修改任何統計結果；呼叫端只在拿到
/// [`NeuralDecision::Accepted`] 時才替換。
///
/// 一般轉換與逐字変換共用這一份門檻與驗證：統計結果等於讀音時代表只是
/// 組字中的未知詞 fallback（半截讀音），不論哪條路徑都直接跳過。
pub fn maybe_convert(config: &NeuralConfig, input: &NeuralInput<'_>) -> NeuralDecision {
    if !config.enabled {
        return NeuralDecision::Skipped;
    }
    // 半截讀音：統計結果等於讀音，表示整段仍是かな，等完整讀音再問模型。
    // 這條規則放在共用層，一般轉換與逐字変換不會各自實作。
    if input.statistical == input.reading {
        return NeuralDecision::Skipped;
    }
    let Some(runtime) = &config.runtime else {
        return NeuralDecision::Unavailable;
    };
    if let Some(runner_up) = input.runner_up_cost {
        let gap = runner_up.saturating_sub(input.statistical_cost);
        if gap as f32 >= config.gap_threshold {
            runtime.record(&NeuralDecision::Skipped);
            return NeuralDecision::Skipped;
        }
    }
    if input.mean_candidates <= config.min_mean_candidates {
        runtime.record(&NeuralDecision::Skipped);
        return NeuralDecision::Skipped;
    }
    if input.reading.is_empty() {
        runtime.record(&NeuralDecision::Skipped);
        return NeuralDecision::Skipped;
    }
    let request = NeuralRequest {
        reading: input.reading.to_string(),
        left: tail_chars(input.left, config.max_context_chars.left),
        right: head_chars(input.right, config.max_context_chars.right),
    };
    let timeout = Duration::from_millis(config.timeout_ms.max(1));
    let decision = match runtime.convert(&request, timeout) {
        Ok(candidate) => match validate(&candidate, input) {
            Ok(Validation::Unchanged) => NeuralDecision::Unchanged,
            Ok(Validation::Accepted) => NeuralDecision::Accepted { surface: candidate },
            Err(reason) => {
                log::debug!(
                    "Jinen 輸出被拒絕（{}）：reading={:?} baseline={:?} candidate={:?}",
                    reason.as_str(),
                    input.reading,
                    input.statistical,
                    candidate
                );
                NeuralDecision::Rejected(reason)
            }
        },
        Err(NeuralError::Timeout) => NeuralDecision::Timeout,
        Err(err) => {
            log::debug!("Jinen 推論失敗，沿用統計結果：{err}");
            NeuralDecision::Unavailable
        }
    };
    runtime.record(&decision);
    decision
}

fn tail_chars(text: &str, limit: usize) -> String {
    if limit == 0 {
        return String::new();
    }
    let len = text.chars().count();
    text.chars().skip(len.saturating_sub(limit)).collect()
}

fn head_chars(text: &str, limit: usize) -> String {
    if limit == 0 {
        return String::new();
    }
    text.chars().take(limit).collect()
}

/// 逐字変換的神經採用器。
///
/// 控制器先算好統計結果與門檻訊號，這裡只把它們轉成 [`NeuralInput`] 再走
/// [`maybe_convert`]；門檻與五條驗證與一般轉換共用同一份實作。
#[derive(Clone)]
pub struct LiveGate {
    config: NeuralConfig,
}

impl LiveGate {
    /// 用指定設定建立採用器。
    pub fn new(config: NeuralConfig) -> Self {
        Self { config }
    }

    /// 目前的設定。
    pub fn config(&self) -> &NeuralConfig {
        &self.config
    }
}

impl fmt::Debug for LiveGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveGate")
            .field("config", &self.config)
            .finish()
    }
}

impl crate::ja::live::NeuralGate for LiveGate {
    fn adopt(&self, request: &crate::ja::live::GateRequest<'_>) -> Option<String> {
        // 停頓補正會改寫已看過的尾巴；用更嚴的成本差門檻，只接受統計
        // 本身也高度歧義的片段。
        let config = if request.phase == crate::ja::live::GatePhase::Correction {
            NeuralConfig {
                gap_threshold: self.config.gap_threshold * self.config.correction_gap_ratio,
                ..self.config.clone()
            }
        } else {
            self.config.clone()
        };
        let decision = maybe_convert(
            &config,
            &NeuralInput {
                reading: request.reading,
                left: request.left,
                right: request.right,
                statistical: request.statistical,
                statistical_cost: request.statistical_cost,
                runner_up_cost: request.runner_up_cost,
                mean_candidates: request.mean_candidates,
            },
        );
        decision.accepted_surface().map(str::to_string)
    }
}

/// 逐字変換用的採用器；關閉或模型缺席時回傳 `None`。
///
/// `None` 代表控制器連門檻訊號都不算，逐字路徑與沒有神經轉換完全相同。
pub fn live_gate(config: &NeuralConfig) -> Option<std::rc::Rc<dyn crate::ja::live::NeuralGate>> {
    if !config.enabled || config.runtime.is_none() {
        return None;
    }
    Some(std::rc::Rc::new(LiveGate::new(config.clone())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn input<'a>(reading: &'a str, left: &'a str, statistical: &'a str) -> NeuralInput<'a> {
        NeuralInput {
            reading,
            left,
            right: "",
            statistical,
            statistical_cost: 1_000,
            runner_up_cost: Some(1_200),
            mean_candidates: 8.0,
        }
    }

    /// 長度：超過 `reading_len * 3 + 8` 就拒絕。
    #[test]
    fn test_validate_rejects_too_long() {
        // 讀音 1 字元 → 上限 11 字元。
        let candidate = "あ".repeat(12);
        let want = Err(RejectReason::TooLong);
        assert_eq!(validate(&candidate, &input("あ", "", "あ")), want);
        assert_eq!(
            validate(&"あ".repeat(11), &input("あ", "", "あ")),
            Ok(Validation::Accepted)
        );
    }

    /// 等於統計結果：無變化，直接接受。
    #[test]
    fn test_validate_accepts_unchanged_output() {
        let output = "今日は晴れ";
        assert_eq!(
            validate(output, &input("きょうははれ", "", output)),
            Ok(Validation::Unchanged)
        );
    }

    /// context echo：重疊 >= 4 且超過門檻就拒絕。
    #[test]
    fn test_validate_rejects_context_echo() {
        let baseline = "天気がいい";
        // 左文脈尾端「日は天気」正好是輸出的開頭，重疊 4 字元；輸出長度
        // 10 也超過「重疊 4 ＋ baseline 5」的門檻。
        let candidate = "日は天気がいいですね";
        assert_eq!(
            validate(candidate, &input("てんきがいい", "今日は天気", baseline)),
            Err(RejectReason::ReadingEcho)
        );
    }

    /// 重疊不足 4 個字元時允許，即使輸出比統計結果短也不得踩到截斷規則。
    #[test]
    fn test_validate_allows_short_overlap() {
        let baseline = "東京都に行く";
        // 左文脈尾端「東京」與輸出開頭「京の」只重疊 1 字元；輸出不是
        // baseline 的前綴或後綴，不會被截斷規則擋下。
        assert_eq!(
            validate("京の都へ", &input("とうきょうにいく", "東京", baseline)),
            Ok(Validation::Accepted)
        );
        // 「京都」重疊 2 字元，仍低於 4。
        assert_eq!(
            validate("京都へ行く", &input("とうきょうにいく", "京都", baseline)),
            Ok(Validation::Accepted)
        );
    }

    /// 絵文字：cluster 序列改變就拒絕；序列相同時位置可以換。
    #[test]
    fn test_validate_rejects_emoji_changes() {
        let baseline = "天気😀";
        assert_eq!(
            validate("天気", &input("てんき", "", baseline)),
            Err(RejectReason::EmojiChanged)
        );
        assert_eq!(
            validate("天気😃", &input("てんき", "", baseline)),
            Err(RejectReason::EmojiChanged)
        );
        // 位置可以換，序列相同即可。
        assert_eq!(
            validate("😀天気", &input("てんき", "", baseline)),
            Ok(Validation::Accepted)
        );
        let skin = "天気👍🏽";
        assert_eq!(
            validate("天気👍", &input("てんき", "", skin)),
            Err(RejectReason::EmojiChanged)
        );
    }

    /// 首尾截斷：比統計結果短，而且是它的前綴或後綴就拒絕。
    #[test]
    fn test_validate_rejects_head_or_tail_truncation() {
        let baseline = "東京都に行く";
        assert_eq!(
            validate("東京都", &input("とうきょうにいく", "", baseline)),
            Err(RejectReason::Truncated)
        );
        assert_eq!(
            validate("に行く", &input("とうきょうにいく", "", baseline)),
            Err(RejectReason::Truncated)
        );
        // 短但不只是頭或尾，允許模型改寫。
        assert_eq!(
            validate("都内へ", &input("とうきょうにいく", "", baseline)),
            Ok(Validation::Accepted)
        );
    }

    /// 模型缺席時不呼叫、不替換，也不報錯。
    #[test]
    fn test_missing_runtime_is_unavailable() {
        let config = NeuralConfig::default();
        let decision = maybe_convert(&config, &input("きょう", "", "今日"));
        assert_eq!(decision, NeuralDecision::Unavailable);
    }

    /// 關閉時連門檻都不算。
    #[test]
    fn test_disabled_config_skips() {
        let config = NeuralConfig {
            enabled: false,
            ..NeuralConfig::default()
        };
        let decision = maybe_convert(&config, &input("きょう", "", "今日"));
        assert_eq!(decision, NeuralDecision::Skipped);
    }

    struct Stub {
        reply: String,
        calls: Arc<AtomicUsize>,
    }

    impl NeuralEngine for Stub {
        fn convert(&mut self, _request: &NeuralRequest) -> Result<String, NeuralError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.reply.clone())
        }
    }

    fn runtime(reply: &str) -> (Arc<NeuralRuntime>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let runtime = NeuralRuntime::spawn(Box::new(Stub {
            reply: reply.to_string(),
            calls: calls.clone(),
        }));
        (Arc::new(runtime), calls)
    }

    /// 預設是低呼叫版：gap 472.5 與平均候選數 5.98。
    #[test]
    fn test_default_thresholds_are_low_call() {
        let config = NeuralConfig::default();
        assert!(config.enabled);
        assert_eq!(config.gap_threshold, 472.5);
        assert_eq!(config.min_mean_candidates, 5.98);
        let quality = NeuralConfig::quality();
        assert_eq!(quality.gap_threshold, 460.0);
        assert_eq!(quality.min_mean_candidates, 0.0);
    }

    /// 門檻以上不呼叫模型。
    #[test]
    fn test_gap_threshold_skips_engine() {
        let (runtime, calls) = runtime("京");
        let config = NeuralConfig {
            runtime: Some(runtime),
            ..NeuralConfig::default()
        };
        let mut input = input("きょう", "", "今日");
        input.statistical_cost = 1_000;
        input.runner_up_cost = Some(1_473);
        assert_eq!(maybe_convert(&config, &input), NeuralDecision::Skipped);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(config.runtime.unwrap().stats().skipped, 1);
    }

    /// 門檻以下會呼叫並採用通過驗證的輸出。
    #[test]
    fn test_gap_threshold_accepts_engine_output() {
        let (runtime, calls) = runtime("京");
        let config = NeuralConfig {
            runtime: Some(runtime),
            ..NeuralConfig::default()
        };
        let mut input = input("きょう", "", "今日");
        input.statistical_cost = 1_000;
        input.runner_up_cost = Some(1_472);
        let decision = maybe_convert(&config, &input);
        assert_eq!(
            decision,
            NeuralDecision::Accepted {
                surface: "京".to_string()
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// 平均候選數沒有超過下限時不呼叫模型。
    #[test]
    fn test_mean_candidates_threshold_skips_engine() {
        let (runtime, calls) = runtime("京");
        let config = NeuralConfig {
            runtime: Some(runtime),
            ..NeuralConfig::default()
        };
        let mut input = input("きょう", "", "今日");
        input.mean_candidates = 5.98;
        assert_eq!(maybe_convert(&config, &input), NeuralDecision::Skipped);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        input.mean_candidates = 5.99;
        assert_eq!(
            maybe_convert(&config, &input),
            NeuralDecision::Accepted {
                surface: "京".to_string()
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// 品質優先版只看 gap，不看平均候選數。
    #[test]
    fn test_quality_mode_ignores_mean_candidates() {
        let (runtime, calls) = runtime("京");
        let config = NeuralConfig {
            runtime: Some(runtime),
            ..NeuralConfig::quality()
        };
        let mut input = input("きょう", "", "今日");
        input.mean_candidates = 1.0;
        input.runner_up_cost = Some(1_459);
        assert_eq!(
            maybe_convert(&config, &input),
            NeuralDecision::Accepted {
                surface: "京".to_string()
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        input.runner_up_cost = Some(1_460);
        assert_eq!(maybe_convert(&config, &input), NeuralDecision::Skipped);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// 模型失敗時回退，不影響統計結果。
    #[test]
    fn test_engine_error_falls_back() {
        struct Failing;
        impl NeuralEngine for Failing {
            fn convert(&mut self, _request: &NeuralRequest) -> Result<String, NeuralError> {
                Err(NeuralError::Inference("壞了".to_string()))
            }
        }
        let config = NeuralConfig {
            runtime: Some(Arc::new(NeuralRuntime::spawn(Box::new(Failing)))),
            ..NeuralConfig::default()
        };
        let decision = maybe_convert(&config, &input("きょう", "", "今日"));
        assert_eq!(decision, NeuralDecision::Unavailable);
        assert_eq!(config.runtime.unwrap().stats().unavailable, 1);
    }

    /// 超過預算就用統計結果。
    #[test]
    fn test_timeout_falls_back() {
        struct Slow;
        impl NeuralEngine for Slow {
            fn convert(&mut self, _request: &NeuralRequest) -> Result<String, NeuralError> {
                std::thread::sleep(Duration::from_millis(120));
                Ok("京".to_string())
            }
        }
        let config = NeuralConfig {
            timeout_ms: 10,
            runtime: Some(Arc::new(NeuralRuntime::spawn(Box::new(Slow)))),
            ..NeuralConfig::default()
        };
        let decision = maybe_convert(&config, &input("きょう", "", "今日"));
        assert_eq!(decision, NeuralDecision::Timeout);
        assert_eq!(config.runtime.unwrap().stats().timeouts, 1);
    }

    /// 半截讀音：統計結果等於讀音時不呼叫模型，一般與逐字路徑共用。
    #[test]
    fn test_half_reading_skips_engine() {
        let (runtime, calls) = runtime("京");
        let config = NeuralConfig {
            runtime: Some(runtime),
            ..NeuralConfig::default()
        };
        let mut input = input("きょ", "", "きょ");
        assert_eq!(maybe_convert(&config, &input), NeuralDecision::Skipped);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "半截讀音不得呼叫模型");
        // 完整讀音的統計結果不會等於讀音；用「今日」當基準，模型回的「京」
        // 才是真的替換，而不是與統計結果相同的 Unchanged。
        input.reading = "きょう";
        input.statistical = "今日";
        assert_eq!(
            maybe_convert(&config, &input),
            NeuralDecision::Accepted {
                surface: "京".to_string()
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// 逐字採用器走同一份門檻與驗證：半截讀音跳過，完整讀音才採用。
    #[test]
    fn test_live_gate_uses_shared_thresholds() {
        use crate::ja::live::{GateRequest, NeuralGate};

        let (runtime, calls) = runtime("京");
        let gate = LiveGate::new(NeuralConfig {
            runtime: Some(runtime),
            ..NeuralConfig::default()
        });
        let half = GateRequest {
            reading: "きょ",
            left: "",
            right: "",
            statistical: "きょ",
            statistical_cost: 1_000,
            runner_up_cost: Some(1_100),
            mean_candidates: 8.0,
            phase: crate::ja::live::GatePhase::Live,
        };
        assert_eq!(gate.adopt(&half), None);
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let full = GateRequest {
            reading: "きょう",
            statistical: "今日",
            ..half
        };
        assert_eq!(gate.adopt(&full), Some("京".to_string()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // 成本差超過門檻時仍不呼叫，門檻與一般路徑相同。
        let clear = GateRequest {
            runner_up_cost: Some(2_000),
            ..full
        };
        assert_eq!(gate.adopt(&clear), None);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_emoji_cluster_keeps_zwj_and_skin_tone() {
        assert_eq!(emoji_clusters("天気👍🏽😀"), vec!["👍🏽", "😀"]);
        assert_eq!(emoji_clusters("👨‍👩‍👧"), vec!["👨‍👩‍👧"]);
    }
}
