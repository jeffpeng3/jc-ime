//! 日文逐字変換（ライブ変換）的控制器。
//!
//! 這個 module 只負責「什麼時候轉換、轉換哪一段、前次結果怎麼保留」，
//! 實際的かな漢字変換仍由 [`crate::ja::decode::JaDecoder`] 或任何
//! [`Converter`] 提供。因此它不改轉換演算法，也不碰 session、外殼、
//! 檔案或網路。
//!
//! 設計重點：
//! - 使用 debounce：每次かな增加只重設期限，安靜下來才轉換，避免每個按鍵
//!   都重算造成閃爍。
//! - 只轉換「尚未確定的尾端」。已經顯示過的文節會鎖住，後續轉換從鎖住
//!   位置之後開始，已確定的前段不可能因尾端變化而跳動。
//! - 前次結果是下一次的 draft。防抖期間與逾時退回路徑都回傳前次結果，
//!   外殼不需要先把畫面退回かな。
//! - 文節邊界一次最多變動 [`Config::max_boundary_changes`] 個；超過就先
//!   保留前次結果，最多 [`Config::hold_limit`] 次，之後仍會接受新結果，
//!   不會永久卡住。
//! - 神經轉換開啟時，每次實際轉換都先把統計結果算好，再交給
//!   [`NeuralGate`] 判斷是否替換；門檻、五條驗證與逾時回退都在共用層，
//!   這裡只準備請求。[`Config::neural`] 為 `None` 時行為與原本完全相同。
//! - 預設關閉。關閉時 [`Controller::input`] 不做任何事，外殼照一般変換
//!   流程處理。

use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::ja::decode::{
    mean_candidates, JaCandidate, JaCandidateSource, JaClause, JaContext, JaDecoder, JaResult,
    JaSpan,
};
use crate::reading::{Sequence, Unit};

/// 一個片段在目前結果中的位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClauseSpan {
    /// 讀音起始單位。
    pub start: usize,
    /// 讀音結束單位（不含）。
    pub end: usize,
    /// 目前顯示的詞面。
    pub surface: String,
}

/// 片段文字的轉換方式；純字串轉換，不呼叫模型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KanaTransform {
    /// 平假名。
    Hiragana,
    /// 全角片假名。
    Katakana,
    /// 半角片假名。
    HalfwidthKana,
    /// 全角英數。
    FullwidthLatin,
    /// 半角英數。
    HalfwidthLatin,
}

impl KanaTransform {
    /// 套用到指定字串。
    pub fn apply(self, text: &str) -> String {
        match self {
            Self::Hiragana => crate::ja::rewrite::to_hiragana(text),
            Self::Katakana => crate::ja::rewrite::to_katakana(text),
            Self::HalfwidthKana => crate::ja::rewrite::to_halfwidth_kana(text),
            Self::FullwidthLatin => crate::ja::rewrite::to_fullwidth_latin(text),
            Self::HalfwidthLatin => crate::ja::rewrite::to_halfwidth_latin(text),
        }
    }
}

/// ライブ変換的錯誤。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LiveError {
    /// 候選範圍超出目前的讀音。
    #[error("live: 候選範圍超出目前讀音")]
    Span,
    /// 轉換器回報錯誤。
    #[error(transparent)]
    Convert(#[from] ConvertError),
}

/// 轉換器回報的錯誤。
///
/// 前身的 `lang.Converter` 允許任意 `error`；Rust 這裡用一個帶訊息的型別，
/// 讓控制器可以把錯誤原樣退回成「保留 draft」而不需要泛型。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("live: {message}")]
pub struct ConvertError {
    /// 錯誤訊息。
    pub message: String,
}

impl ConvertError {
    /// 建立錯誤。
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// 逐字変換用的轉換器。
///
/// 對齊前身的 `lang.Converter`：`convert` 轉換整段讀音，`candidates` 只列
/// 指定範圍的候選。兩者都必須是唯讀操作；控制器本身不是併發安全的。
pub trait Converter {
    /// 轉換整段かな讀音。
    fn convert(&self, reading: &str, ctx: &JaContext) -> Result<JaResult, ConvertError>;

    /// 列出 `span` 範圍的候選。
    fn candidates(
        &self,
        reading: &str,
        span: JaSpan,
        ctx: &JaContext,
    ) -> Result<Vec<JaCandidate>, ConvertError>;

    /// 統計轉換加上神經門檻所需的訊號。
    ///
    /// 預設沿用 [`Converter::convert`]，沒有次佳路徑與候選數，等於門檻一定
    /// 觸發。實作要提供真正的 top-2 成本差時才覆寫；逐字路徑只在神經轉換
    /// 開啟時呼叫這個方法。
    fn statistical(&self, reading: &str, ctx: &JaContext) -> Result<Statistical, ConvertError> {
        Ok(Statistical::plain(self.convert(reading, ctx)?))
    }

    /// 整段讀音是否剛好是一個詞典詞條。
    ///
    /// 逐字変換的「完整詞」邊界觸發用；預設沒有詞典，回傳 `false`。
    /// 這是單次索引查詢，不是解碼，成本與長度無關。
    fn is_complete_word(&self, _reading: &str) -> bool {
        false
    }
}

/// 統計轉換結果與門檻訊號。
///
/// `runner_up_cost` 與 `mean_candidates` 的語意與一般轉換的 gating 相同，
/// 由同一次解碼產生，不在等候模型的路徑上補算。
#[derive(Debug, Clone, PartialEq)]
pub struct Statistical {
    /// 統計轉換結果。
    pub result: JaResult,
    /// 次佳路徑的原始成本；`None` 表示沒有備案，門檻一定觸發。
    pub runner_up_cost: Option<i32>,
    /// top-1 路徑上每個詞段的平均候選詞面數。
    pub mean_candidates: f32,
}

impl Statistical {
    /// 只有結果、沒有門檻訊號的版本。
    pub fn plain(result: JaResult) -> Self {
        Self {
            result,
            runner_up_cost: None,
            mean_candidates: 1.0,
        }
    }
}

/// 逐字変換採用神經輸出的鉤子。
///
/// 統計結果與門檻訊號都由控制器先算好；實作只做門檻判斷與採用驗證，
/// 不重新解碼，也不決定顯示方式。回傳 `Some` 代表整段尾端要換成該字串。
pub trait NeuralGate {
    /// 判斷是否採用神經輸出。
    fn adopt(&self, request: &GateRequest<'_>) -> Option<String>;

    /// 非同步送出一次請求；已有推論在跑或不需要模型時回傳 `false`。
    fn start(&self, _request: &GateRequest<'_>) -> bool {
        false
    }

    /// 取出已完成的非同步結果；還沒完成時回傳 `None`。
    fn take(&self) -> Option<FlightResult> {
        None
    }

    /// 是否還有推論在跑。
    fn busy(&self) -> bool {
        false
    }
}

/// 一次非同步推論的完成結果。
#[derive(Debug, Clone)]
pub struct FlightResult {
    /// 送出時的 generation。
    pub generation: u64,
    /// 送出時的讀音前綴。
    pub reading: String,
    /// 通過驗證的輸出；`None` 代表跳過、逾時或拒絕。
    pub surface: Option<String>,
    /// 推論耗時。
    pub elapsed: Duration,
}

/// 一次神經採用的請求。
///
/// 欄位與一般轉換的 [`NeuralInput`] 相同，只是逐字路徑由控制器提供；
/// 門檻與五條驗證仍由共用層執行。
///
/// [`NeuralInput`]: crate::session::neural::NeuralInput
#[derive(Debug, Clone, Copy)]
pub struct GateRequest<'a> {
    /// 目前要轉換的かな讀音。
    pub reading: &'a str,
    /// 左文脈，含已鎖定的詞面。
    pub left: &'a str,
    /// 右文脈；目前沒有來源，固定空字串。
    pub right: &'a str,
    /// 統計轉換的第一名。
    pub statistical: &'a str,
    /// 統計第一名的原始成本。
    pub statistical_cost: i32,
    /// 統計第二名的原始成本；`None` 代表沒有備案。
    pub runner_up_cost: Option<i32>,
    /// top-1 路徑的平均候選數。
    pub mean_candidates: f32,
    /// 這次採用是即時轉換或停頓後補正。
    pub phase: GatePhase,
    /// 送出的輸入世代。
    pub generation: u64,
}

impl Converter for JaDecoder {
    fn convert(&self, reading: &str, ctx: &JaContext) -> Result<JaResult, ConvertError> {
        let units: Vec<char> = reading.chars().collect();
        self.results(&units, ctx, 1)
            .into_iter()
            .next()
            .ok_or_else(|| ConvertError::new("沒有轉換結果"))
    }

    fn statistical(&self, reading: &str, ctx: &JaContext) -> Result<Statistical, ConvertError> {
        let units: Vec<char> = reading.chars().collect();
        let (results, gap) = self.results_with_gap(&units, ctx, 2);
        let Some(best) = results.first() else {
            return Err(ConvertError::new("沒有轉換結果"));
        };
        let cost = best.conversion().cost;
        let mean = mean_candidates(best, self.source().as_ref());
        let result = best.clone();
        Ok(Statistical {
            result,
            runner_up_cost: gap.map(|gap| cost.saturating_add(gap)),
            mean_candidates: mean,
        })
    }

    fn candidates(
        &self,
        reading: &str,
        span: JaSpan,
        ctx: &JaContext,
    ) -> Result<Vec<JaCandidate>, ConvertError> {
        let units: Vec<char> = reading.chars().collect();
        Ok(JaDecoder::candidates(self, &units, span, ctx))
    }

    fn is_complete_word(&self, reading: &str) -> bool {
        let units: Vec<char> = reading.chars().collect();
        if units.is_empty() {
            return false;
        }
        let total = units.len();
        let mut found = false;
        self.source().matches(&units, 0, &mut |word| {
            found |= word.end == total;
        });
        found
    }
}

/// 神經結果的套用範圍。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NeuralScope {
    /// 整段尚未鎖定的尾端（現況）。
    #[default]
    WholeTail,
    /// 只處理最後一個尚未鎖定的文節。
    ///
    /// 統計結果仍是文節邊界的來源，已鎖定的前綴與較早的文節直接重用；
    /// 模型只補最後一個還會變動的文節，不會把整段尾端併成一個文節而讓
    /// 鎖定永遠無法前進。
    Clause,
}

/// 神經結果的套用時機。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NeuralTiming {
    /// 與統計轉換同一次呼叫（現況）。
    #[default]
    Inline,
    /// 先顯示統計結果；停頓後再由外殼呼叫 [`Controller::poll_neural`] 補正。
    Deferred,
}

/// 自動変換的觸發條件；三個條件可以組合。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoTrigger {
    /// 未轉換的假名累積到幾個單位才轉換；`0` 關閉長度觸發。
    pub min_units: usize,
    /// 讀音結尾是助詞、標點或詞典完整詞時轉換。
    pub boundary: bool,
    /// 停頓後由 [`Controller::idle`] 轉換與補正。
    pub idle: bool,
}

impl AutoTrigger {
    /// 關閉所有自動觸發，等同每鍵轉換以外的保守設定。
    pub fn off() -> Self {
        Self {
            min_units: 0,
            boundary: false,
            idle: false,
        }
    }
}

impl Default for AutoTrigger {
    /// Microsoft IME 形狀的建議值：12 字元、邊界、停頓都開。
    fn default() -> Self {
        Self {
            min_units: 12,
            boundary: true,
            idle: true,
        }
    }
}

/// 什麼時候真正執行轉換。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Trigger {
    /// 每次讀音更新都轉換（現況）。
    #[default]
    EveryKey,
    /// 自動変換：長度、邊界、停頓三種條件可組合。
    Auto(AutoTrigger),
}

/// 神經採用的階段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GatePhase {
    /// 逐鍵或自動変換的即時轉換。
    #[default]
    Live,
    /// 停頓後的尾巴補正；門檻更嚴，避免改寫使用者已看過的片段。
    Correction,
}

/// ライブ変換的行為開關。
///
/// 預設關閉。要開啟時從 [`Config::default`] 開始改，或至少把
/// [`Config::enabled`] 設為 `true`。
#[derive(Clone)]
pub struct Config {
    /// 總開關，預設 `false`；關閉時行為與一般変換完全相同。
    pub enabled: bool,
    /// 最後一次かな輸入後要等多久才轉換。0 表示立即轉換（測試用）。
    pub debounce: Duration,
    /// 停頓多久算「一段打完」，預設 300 ms。
    ///
    /// 只在 [`Trigger::Auto`] 開啟 `idle` 時使用；外殼要在期限到了呼叫
    /// [`Controller::tick`]。
    pub idle_delay: Duration,
    /// 單次轉換的時間預算；超過就退回前次結果，不鎖定新結果。
    /// 0 表示不檢查。
    pub budget: Duration,
    /// 單次轉換允許改變的文節邊界數，預設 1。負值表示不限制。
    pub max_boundary_changes: isize,
    /// 邊界變動過大時最多保留前次結果幾次，預設 2。0 表示不保留。
    pub hold_limit: usize,
    /// 開始ライブ変換所需的最少讀音單位，預設 1。
    pub min_units: usize,
    /// 尾端保留幾個文節不鎖定，預設 1（現況）。
    ///
    /// 停頓補正要有內容可補時可以設 2，代價是每鍵會多轉一個文節。
    pub keep_tail: usize,
    /// 轉換時機；預設每鍵轉換。
    pub trigger: Trigger,
    /// 神經結果的套用範圍；預設整段尾端。
    pub neural_scope: NeuralScope,
    /// 神經結果的套用時機；預設與統計轉換同一次呼叫。
    pub neural_timing: NeuralTiming,
    /// 非同步補正的推論預算；模型超過這個時間就丟棄結果，維持統計輸出。
    /// 0 表示不檢查，只用於 [`NeuralTiming::Deferred`]。
    pub neural_budget: Duration,
    /// single-flight：同時最多一個推論，完成後套用到送出的讀音前綴。
    ///
    /// 開啟時忽略長度／邊界／停頓觸發，每一鍵都會先顯示統計結果，模型在
    /// 背景跑；期間的新輸入只增加尾巴，不排隊。
    pub neural_flight: bool,
    /// 延遲整句轉換：打字期間只顯示假名，停頓後才做一次完整轉換。
    ///
    /// 轉換後才顯示漢字；任何輸入讓目前轉換失效，回到假名。同一時間最多
    /// 一次推論；停頓後又開始打字時，尚未完成的結果直接丟棄。
    pub deferred_sentence: bool,
    /// 用「前綴快取＋失效重算」取代鎖定片段。
    ///
    /// 快取存的是讀音前綴與已轉換文字；新按鍵只轉未快取的尾巴，左文脈用
    /// 快取文字。前面被修改或刪除時，從共同前綴之後全部失效重算。
    pub prefix_cache: bool,
    /// 前綴快取保留幾個文節不進快取，預設 1。
    pub cache_keep: usize,
    /// 非同步回應只要求「目前讀音仍以送出前綴開頭」；`false` 時要求
    /// generation 完全相同。
    pub flight_append: bool,
    /// 神經採用器；`None` 時逐字路徑完全不呼叫模型，行為與關閉相同。
    ///
    /// 由 [`crate::session::neural::live_gate`] 依 `ja_neural` 建立；只有
    /// 開啟神經轉換且模型已載入時才會有值。
    pub neural: Option<Rc<dyn NeuralGate>>,
    /// 時間來源；`None` 使用 [`Instant::now`]。測試可注入虛擬時鐘。
    pub clock: Option<Rc<dyn Fn() -> Instant>>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("enabled", &self.enabled)
            .field("debounce", &self.debounce)
            .field("idle_delay", &self.idle_delay)
            .field("budget", &self.budget)
            .field("max_boundary_changes", &self.max_boundary_changes)
            .field("hold_limit", &self.hold_limit)
            .field("min_units", &self.min_units)
            .field("keep_tail", &self.keep_tail)
            .field("trigger", &self.trigger)
            .field("neural_scope", &self.neural_scope)
            .field("neural_timing", &self.neural_timing)
            .field("neural_budget", &self.neural_budget)
            .field("neural_flight", &self.neural_flight)
            .field("deferred_sentence", &self.deferred_sentence)
            .field("prefix_cache", &self.prefix_cache)
            .field("cache_keep", &self.cache_keep)
            .field("flight_append", &self.flight_append)
            .finish_non_exhaustive()
    }
}

impl Default for Config {
    /// 建議值。`enabled` 保持 `false`，符合「預設關閉」。
    fn default() -> Self {
        Self {
            enabled: true,
            debounce: Duration::from_millis(120),
            idle_delay: Duration::from_millis(300),
            budget: Duration::from_millis(100),
            max_boundary_changes: 1,
            hold_limit: 2,
            min_units: 1,
            keep_tail: 1,
            trigger: Trigger::Auto(AutoTrigger::default()),
            neural_scope: NeuralScope::Clause,
            neural_timing: NeuralTiming::Deferred,
            neural_budget: Duration::from_millis(35),
            neural_flight: false,
            deferred_sentence: true,
            prefix_cache: false,
            cache_keep: 1,
            flight_append: true,
            neural: None,
            clock: None,
        }
    }
}

/// 建議的設定值，對齊前身的 `DefaultConfig`。
pub fn default_config() -> Config {
    Config::default()
}

/// 一次 [`Controller::input`] 或 [`Controller::tick`] 之後，控制器給外殼
/// 的決定。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    /// `result` 是這次新轉換的結果，可以直接顯示。
    pub convert: bool,
    /// `result` 是前次結果（draft），本次沒有新結果。
    pub preview: bool,
    /// debounce 還在等；`deadline` 到了再呼叫 [`Controller::tick`]。
    pub pending: bool,
    /// 目前可顯示的轉換結果。沒有結果時為 `None`。
    pub result: Option<JaResult>,
    /// debounce 到期時間，只在 `pending` 時有意義。
    pub deadline: Option<Instant>,
    /// 本次候選結果相對前次尾端的文節邊界變動數。
    pub boundary_changes: usize,
    /// 這次轉換超過 [`Config::budget`]，已退回前次結果。
    pub timed_out: bool,
}

/// 控制器累計的執行統計，供效能測試與外殼診斷使用。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// 實際呼叫轉換器的次數。
    pub conversions: usize,
    /// 因文節邊界變動過大而保留前次結果的次數。
    pub holds: usize,
    /// 超過時間預算的次數。
    pub timeouts: usize,
    /// 最近一次轉換耗時。
    pub last_duration: Duration,
    /// 最近一次轉換的文節邊界變動數。
    pub last_boundary_changes: usize,
    /// 送進神經採用器的請求數；模型是否真的被呼叫由執行環境統計。
    pub neural_requests: usize,
    /// 神經輸出實際替換統計結果的次數。
    pub neural_adopted: usize,
    /// 停頓後補正的嘗試次數。
    pub corrections: usize,
    /// 停頓後補正實際替換的次數。
    pub correction_adopted: usize,
    /// 因為使用者繼續輸入而丟棄的補正次數。
    pub stale_discards: usize,
    /// 因為超過 [`Config::neural_budget`] 而丟棄的補正次數。
    pub late_discards: usize,
    /// 已顯示片段總數（每次更新都重新計數）。
    pub fragments_shown: usize,
    /// 已顯示片段在後續更新中改變的次數。
    pub fragments_changed: usize,
    /// 已顯示片段中「不是當下尾巴」的數量；這些片段理論上不該再變。
    pub settled_shown: usize,
    /// 已穩定片段在後續更新中改變的次數。
    pub settled_changed: usize,
}

/// 一次已顯示的片段；以讀音單位定位，方便跨次比對同一段文字。
#[derive(Debug, Clone)]
struct DisplayFragment {
    /// 讀音起始單位。
    start: usize,
    /// 讀音結束單位（不含）。
    end: usize,
    /// 當時顯示的詞面。
    surface: String,
    /// 當時是否仍是かな（尚未轉成漢字）。
    kana: bool,
}

/// 等待非同步補正的一次請求。
#[derive(Debug, Clone)]
struct PendingNeural {
    /// 建立請求時的輸入世代；使用者一打字就作廢。
    generation: u64,
    /// 要補正的讀音。
    reading: String,
    /// 左文脈，含已鎖定與較早文節的詞面。
    left: String,
    /// 右文脈。
    right: String,
    /// 統計基線。
    statistical: String,
    /// 統計基線成本。
    statistical_cost: i32,
    /// 次佳成本。
    runner_up_cost: Option<i32>,
    /// 平均候選數。
    mean_candidates: f32,
    /// 統計基線的文節成本總和。
    cost: i32,
    /// 要補正的讀音單位數；尾端標點不在內。
    units: usize,
}

/// 使用者手動選過或編輯過的片段；重轉時保持固定。
///
/// 控制器只把這份清單當成轉換快取：所有權由 `jc-core` 的
/// [`crate::composition::Composition`] 決定，`Session` 每次重建都把
/// `Composition` 的 `User` 標記推進來。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSegment {
    /// 讀音單位起點。
    pub start: usize,
    /// 讀音單位數。
    pub units: usize,
    /// 固定時的讀音。
    pub reading: String,
    /// 固定時的詞面。
    pub surface: String,
}

/// 已轉換的讀音前綴與文字。
#[derive(Debug, Clone, Default)]
struct PrefixCache {
    reading: String,
    clauses: Vec<JaClause>,
}

impl PrefixCache {
    fn text(&self) -> String {
        self.clauses
            .iter()
            .map(|clause| clause.surface.as_str())
            .collect()
    }

    fn units(&self) -> usize {
        self.clauses.iter().map(clause_units).sum()
    }
}

/// 一次 single-flight 推論的送出狀態。
#[derive(Debug, Clone)]
struct FlightRequest {
    /// 送出時的完整前綴讀音（快取＋尾巴）。
    prefix: String,
    /// 送出的未快取尾巴讀音。
    reading: String,
    /// 尾巴的讀音單位數。
    units: usize,
    /// 尾巴的統計詞面。
    statistical: String,
    /// 尾巴的統計成本。
    cost: i32,
}

/// 最後一個未鎖定文節的統計基線與文脈。
#[derive(Debug, Clone)]
struct ClauseRequest {
    reading: String,
    left: String,
    right: String,
    statistical: String,
    statistical_cost: i32,
    runner_up_cost: Option<i32>,
    mean_candidates: f32,
}

/// 整段未鎖定尾巴的補正請求內容。
#[derive(Debug, Clone)]
struct PendingRequest {
    reading: String,
    left: String,
    right: String,
    statistical: String,
    statistical_cost: i32,
    runner_up_cost: Option<i32>,
    mean_candidates: f32,
    cost: i32,
    units: usize,
}

/// ライブ変換的狀態機。零值不可用，請用 [`Controller::new`] 建立。
///
/// 控制器不是併發安全的；語言／session 本身也是單執行緒處理按鍵，
/// 呼叫端不需要額外鎖。
pub struct Controller {
    cfg: Config,
    conv: Box<dyn Converter>,

    seq: Sequence,
    ctx: JaContext,

    /// 已鎖定的讀音前綴長度；`locked` 是對應的文節，`locked_seq` 是比對用
    /// 的讀音快照。`tail` 是最近一次轉換中尚未鎖定的文節。
    cut: usize,
    locked: Vec<JaClause>,
    locked_seq: Sequence,
    tail: Vec<JaClause>,

    draft: Option<JaResult>,
    pending: bool,
    due: Option<Instant>,
    idle_due: Option<Instant>,
    held: usize,
    /// 上一次接受的結果是神經輸出。
    ///
    /// 神經輸出沒有詞邊界，跟統計結果的文節切法不能直接比；下一次轉換
    /// 跳過邊界變動比較，直接以新的統計結果重建基準，否則每次採用神經
    /// 輸出都會被誤判成大幅變動而保留舊畫面。
    tail_neural: bool,
    /// 輸入世代；每次讀音更新遞增，用來丟棄過期補正。
    generation: u64,
    /// 最近一次真正轉換到的讀音長度；自動変換的長度觸發從這裡起算。
    converted_upto: usize,
    /// 上一次顯示的片段，用於計算已顯示片段的變動率。
    prev_display: Vec<DisplayFragment>,
    /// 等待停頓後補正的請求。
    pending_neural: Option<PendingNeural>,
    /// 這次轉換是停頓觸發；先不鎖定新的文節，留給補正。
    provisional: bool,
    /// 前綴快取：已轉換的讀音前綴與文字。
    cache: PrefixCache,
    /// 等待中的 single-flight 推論。
    flight: Option<FlightRequest>,
    /// 延遲整句模式上一次轉換的讀音；用來判斷是單純追加還是編輯。
    sentence_reading: String,
    /// 使用者固定的片段；重轉時不變。
    user_segments: Vec<UserSegment>,
    /// 最近一次統計結果的總成本。
    last_cost: f64,
    /// 最近一次統計結果的分數。
    last_score: f64,

    stats: Stats,
}

impl Controller {
    /// 建立控制器。
    ///
    /// `enabled` 為 `false` 時控制器維持停用，[`Controller::input`] 與
    /// [`Controller::tick`] 都不會呼叫轉換器。
    pub fn new(conv: impl Converter + 'static, cfg: Config) -> Self {
        Self {
            cfg: normalize_config(cfg),
            conv: Box::new(conv),
            seq: Sequence::new(),
            ctx: JaContext::default(),
            cut: 0,
            locked: Vec::new(),
            locked_seq: Sequence::new(),
            tail: Vec::new(),
            draft: None,
            pending: false,
            due: None,
            idle_due: None,
            held: 0,
            tail_neural: false,
            generation: 0,
            converted_upto: 0,
            prev_display: Vec::new(),
            pending_neural: None,
            provisional: false,
            cache: PrefixCache::default(),
            flight: None,
            sentence_reading: String::new(),
            user_segments: Vec::new(),
            last_cost: 0.0,
            last_score: 0.0,
            stats: Stats::default(),
        }
    }

    /// ライブ変換是否開啟。
    pub fn enabled(&self) -> bool {
        self.cfg.enabled
    }

    /// 更新設定。
    ///
    /// 任何變更都會清掉暫態，避免用舊設定產生的前綴繼續被當成已確定內容；
    /// 外殼在設定變更後重新送出目前讀音即可。
    pub fn set_config(&mut self, cfg: Config) {
        self.cfg = normalize_config(cfg);
        self.reset();
    }

    /// 只切換總開關。
    pub fn set_enabled(&mut self, on: bool) {
        let mut cfg = self.cfg.clone();
        cfg.enabled = on;
        self.set_config(cfg);
    }

    /// 累計統計。
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// 前次可顯示的結果。
    pub fn draft(&self) -> Option<&JaResult> {
        self.draft.as_ref()
    }

    /// debounce 是否還在等待。
    pub fn pending(&self) -> bool {
        self.pending
    }

    /// 收到一次かな增加（或任何讀音更新）。
    ///
    /// 每次呼叫都會重新計算 debounce 期限；[`Config::debounce`] 為 0 時
    /// 立刻轉換。若新的讀音不再是已鎖定前綴的延伸（例如倒退鍵），會先清掉
    /// 舊的鎖定與 draft，再從頭轉換，避免把不同讀音的前綴接在一起。
    pub fn input(&mut self, seq: &[Unit], ctx: &JaContext) -> Plan {
        if !self.enabled() {
            return Plan::default();
        }
        if !self.cfg.prefix_cache && !self.prefix_intact(seq) {
            self.reset();
        }
        self.seq.clear();
        self.seq.extend_from_slice(seq);
        self.ctx = ctx.clone();
        self.generation += 1;
        if self.pending_neural.take().is_some() {
            self.stats.stale_discards += 1;
        }
        if self.cfg.deferred_sentence {
            self.pending = false;
            self.due = None;
            let current: String = self.seq.iter().map(Unit::as_str).collect();
            // 單純往後追加時保留已顯示的轉換；刪除或改中間才讓編輯點之後的
            // Auto 片段失效。哪些 User 片段還在由 `Composition` 決定，
            // `Session` 會在每次重建前先裁好再推進來。
            if !self.sentence_reading.is_empty() && !current.starts_with(&self.sentence_reading) {
                self.invalidate_sentence();
            }
            self.idle_due = Some(self.now() + self.cfg.idle_delay);
            return self.preview_plan();
        }
        if self.cfg.prefix_cache {
            self.pending = false;
            self.due = None;
            self.idle_due = None;
            self.sync_cache();
            return self.convert_cache();
        }
        if matches!(self.cfg.trigger, Trigger::Auto(trigger) if trigger.idle) {
            self.idle_due = Some(self.now() + self.cfg.idle_delay);
        }

        if self.seq.len() < self.cfg.min_units {
            self.pending = false;
            self.due = None;
            return self.preview_plan();
        }
        if !self.should_convert() {
            // 自動変換還沒累積到觸發條件；保留已顯示的結果，新的かな由
            // 外殼接在後面顯示。
            self.pending = false;
            self.due = None;
            return self.preview_plan();
        }
        if self.cfg.debounce > Duration::ZERO {
            self.pending = true;
            let due = self.now() + self.cfg.debounce;
            self.due = Some(due);
            let mut plan = self.preview_plan();
            plan.pending = true;
            plan.deadline = Some(due);
            return plan;
        }
        self.pending = false;
        self.due = None;
        self.convert()
    }

    /// 在 debounce 到期時執行轉換。
    ///
    /// 回傳的第二個值表示這次是否真的做了轉換（或逾時退回）；期限未到、
    /// 沒有待處理輸入或功能關閉時回傳 `false`。
    pub fn tick(&mut self) -> (Plan, bool) {
        if !self.enabled() {
            return (Plan::default(), false);
        }
        if self.pending {
            if let Some(due) = self.due {
                if self.now() >= due {
                    self.pending = false;
                    self.due = None;
                    return (self.convert(), true);
                }
            }
        }
        if let Some(due) = self.idle_due {
            if self.now() >= due {
                self.idle_due = None;
                return self.idle();
            }
        }
        (Plan::default(), false)
    }

    /// 取消尚未執行的 debounce，保留目前已顯示的結果。
    ///
    /// 回傳是否有東西被取消。
    pub fn cancel_pending(&mut self) -> bool {
        let had = self.pending || self.idle_due.is_some();
        self.pending = false;
        self.due = None;
        self.idle_due = None;
        had
    }

    /// 停頓計時器是否還在等待。
    pub fn idle_pending(&self) -> bool {
        self.idle_due.is_some()
    }

    /// 停頓期限；外殼用它安排計時器。
    pub fn idle_deadline(&self) -> Option<Instant> {
        self.idle_due
    }

    /// 清掉所有組字、鎖定與 draft 狀態，但保留累計統計。
    pub fn reset(&mut self) {
        self.reset_state();
        self.seq.clear();
        self.ctx = JaContext::default();
    }

    /// 回傳目前讀音指定範圍的候選。
    ///
    /// 候選仍由原本的轉換器產生，只是把已鎖定的前綴放進 [`JaContext::left`]，
    /// 讓尾端候選有正確的左文脈。除了字典候選，一定附上目前詞面、原讀音的
    /// 平假名與片假名（以及半角かな），讓使用者不用記功能鍵也選得到。
    /// 功能關閉時回傳空清單，外殼走一般変換。
    pub fn candidates(&self, span: JaSpan) -> Result<Vec<JaCandidate>, LiveError> {
        if !self.enabled() {
            return Ok(Vec::new());
        }
        if span.end() > self.seq.len() {
            return Err(LiveError::Span);
        }
        let reading: String = self.seq.iter().map(Unit::as_str).collect();
        let mut out = self
            .conv
            .candidates(&reading, span, &self.convert_context())?;
        let target: String = self.seq[span.start..span.end()]
            .iter()
            .map(Unit::as_str)
            .collect();
        let mut seen: HashSet<String> = out.iter().map(|c| c.surface.clone()).collect();
        let mut push = |surface: String, source: JaCandidateSource| {
            if surface.is_empty() || !seen.insert(surface.clone()) {
                return;
            }
            out.push(JaCandidate {
                surface,
                reading: target.clone(),
                cost: i32::MAX / 2,
                source,
            });
        };
        if let Some(clause) = self.clause_at(span) {
            push(clause.surface.clone(), JaCandidateSource::Convert);
        }
        push(
            KanaTransform::Hiragana.apply(&target),
            JaCandidateSource::Hiragana,
        );
        push(
            KanaTransform::Katakana.apply(&target),
            JaCandidateSource::Katakana,
        );
        push(
            KanaTransform::HalfwidthKana.apply(&target),
            JaCandidateSource::HalfwidthKana,
        );
        Ok(out)
    }

    /// 目前結果的片段位置與詞面。
    pub fn clause_spans(&self) -> Vec<ClauseSpan> {
        let mut out = Vec::new();
        let mut start = 0;
        for clause in self.all_clauses() {
            let units = clause_units(&clause);
            out.push(ClauseSpan {
                start,
                end: start + units,
                surface: clause.surface,
            });
            start += units;
        }
        out
    }

    /// 目前固定的使用者片段。
    ///
    /// 這份清單是轉換快取，權威來源是 `jc-core` 的
    /// [`crate::composition::Composition`]。
    pub fn user_segments(&self) -> &[UserSegment] {
        &self.user_segments
    }

    /// 以 `Composition` 推導出的清單覆寫固定片段。
    ///
    /// `Session` 每次重建組字時呼叫；控制器不再自行決定哪一段被鎖定。
    pub fn set_user_segments(&mut self, segments: Vec<UserSegment>) {
        self.user_segments = segments;
        self.user_segments.sort_by_key(|segment| segment.start);
    }

    /// 把指定片段標成 User；重轉時保持固定。
    fn mark_user(&mut self, index: usize) {
        let Some(span) = self.clause_spans().get(index).cloned() else {
            return;
        };
        let reading: String = self.seq[span.start..span.end]
            .iter()
            .map(Unit::as_str)
            .collect();
        self.user_segments.retain(|segment| {
            segment.start + segment.units <= span.start || segment.start >= span.end
        });
        self.user_segments.push(UserSegment {
            start: span.start,
            units: span.end - span.start,
            reading,
            surface: span.surface,
        });
        self.user_segments.sort_by_key(|segment| segment.start);
    }

    /// 只換指定片段的詞面；讀音、鎖定範圍與其他片段都不變。
    ///
    /// 回傳是否真的換了。片段索引依 [`Controller::clause_spans`] 的順序。
    pub fn replace_clause(&mut self, index: usize, surface: String) -> bool {
        if surface.is_empty() {
            return false;
        }
        let locked = self.locked.len();
        let target = if index < locked {
            &mut self.locked[index]
        } else if index - locked < self.tail.len() {
            &mut self.tail[index - locked]
        } else {
            return false;
        };
        if target.surface == surface {
            return false;
        }
        target.surface = surface;
        target.segments.clear();
        self.tail_neural = false;
        self.mark_user(index);
        self.record_stability(&self.all_clauses());
        let cost = self.last_cost;
        let score = self.last_score;
        self.draft = Some(JaResult {
            clauses: self.all_clauses(),
            cost,
            score,
        });
        true
    }

    /// 對指定片段套用假名／英數轉換。
    pub fn transform_clause(&mut self, index: usize, transform: KanaTransform) -> bool {
        let Some(clause) = self.all_clauses().get(index).cloned() else {
            return false;
        };
        // かな系變換鍵看讀音：選到「今日」按 F6 要得到「きょう」。
        // 英數系變換看目前詞面。
        let text = match transform {
            KanaTransform::Hiragana | KanaTransform::Katakana | KanaTransform::HalfwidthKana => {
                clause_reading(&clause)
            }
            KanaTransform::FullwidthLatin | KanaTransform::HalfwidthLatin => clause.surface.clone(),
        };
        let surface = transform.apply(&text);
        self.replace_clause(index, surface)
    }

    /// 在平假名與全角片假名之間切換指定片段。
    ///
    /// 漢字片段先還原成讀音（平假名）；下一次呼叫才變片假名。回傳新的
    /// 片段詞面。讀音與片段範圍不變。
    pub fn toggle_script(&mut self, span: JaSpan) -> Option<String> {
        let index = self.clause_index_at(span)?;
        let clause = self.all_clauses().get(index)?.clone();
        let reading = clause_reading(&clause);
        let surface = if has_katakana(&clause.surface) {
            crate::ja::rewrite::to_hiragana(&clause.surface)
        } else if has_hiragana(&clause.surface) {
            crate::ja::rewrite::to_katakana(&clause.surface)
        } else {
            // 漢字（或英數）先回到讀音，維持平假名。
            crate::ja::rewrite::to_hiragana(&reading)
        };
        self.replace_clause(index, surface.clone())
            .then_some(surface)
    }

    /// 把片段還原成讀音，並搬回未鎖定的尾巴。
    ///
    /// 已鎖定的前綴只保留到目標片段之前；目標與其後已鎖定的文節一起回到
    /// `tail`，下一次轉換可以重新切分。回傳還原後的讀音。
    pub fn unconvert(&mut self, span: JaSpan) -> Option<String> {
        let index = self.clause_index_at(span)?;
        let reading = clause_reading(self.all_clauses().get(index)?);
        if reading.is_empty() {
            return None;
        }
        if index < self.locked.len() {
            let mut moved = self.locked.split_off(index);
            let kept: usize = self.locked.iter().map(clause_units).sum();
            moved[0].surface = reading.clone();
            moved[0].segments.clear();
            self.locked_seq.truncate(kept);
            self.cut = kept;
            moved.append(&mut self.tail);
            self.tail = moved;
        } else {
            let tail_index = index - self.locked.len();
            let clause = &mut self.tail[tail_index];
            clause.surface = reading.clone();
            clause.segments.clear();
        }
        self.tail_neural = false;
        if let Some(index) = self.clause_index_at(span) {
            self.mark_user(index);
        }
        self.record_stability(&self.all_clauses());
        let cost = self.last_cost;
        let score = self.last_score;
        self.draft = Some(JaResult {
            clauses: self.all_clauses(),
            cost,
            score,
        });
        Some(reading)
    }

    /// 片段索引；`span` 必須剛好等於一個片段的讀音範圍。
    pub fn clause_index_at(&self, span: JaSpan) -> Option<usize> {
        let mut start = 0;
        for (index, clause) in self.all_clauses().iter().enumerate() {
            let end = start + clause_units(clause);
            if start == span.start && end == span.end() {
                return Some(index);
            }
            start = end;
        }
        None
    }

    /// 把指定片段（含之前尚未鎖定的文節）鎖定。
    ///
    /// 使用者明確選字或按假名變換鍵後呼叫；後續輸入只會轉換更後面的尾端，
    /// 已選的片段不會因為下一個かな而變回去。
    pub fn lock_through(&mut self, index: usize) -> bool {
        if index < self.locked.len() {
            return false;
        }
        let tail_index = index - self.locked.len();
        if tail_index >= self.tail.len() {
            return false;
        }
        let units: usize = self.tail[..=tail_index].iter().map(clause_units).sum();
        if units == 0 || self.cut + units > self.seq.len() {
            return false;
        }
        self.locked.extend(self.tail.drain(..=tail_index));
        self.locked_seq
            .extend_from_slice(&self.seq[self.cut..self.cut + units]);
        self.cut += units;
        self.tail_neural = false;
        true
    }

    fn clause_at(&self, span: JaSpan) -> Option<&JaClause> {
        let mut start = 0;
        for clause in &self.locked {
            let end = start + clause_units(clause);
            if start == span.start && end == span.end() {
                return Some(clause);
            }
            start = end;
        }
        for clause in &self.tail {
            let end = start + clause_units(clause);
            if start == span.start && end == span.end() {
                return Some(clause);
            }
            start = end;
        }
        None
    }

    fn reset_state(&mut self) {
        self.cut = 0;
        self.locked.clear();
        self.locked_seq.clear();
        self.tail.clear();
        self.draft = None;
        self.pending = false;
        self.due = None;
        self.idle_due = None;
        self.held = 0;
        self.tail_neural = false;
        self.generation += 1;
        self.converted_upto = 0;
        self.prev_display.clear();
        self.pending_neural = None;
        self.provisional = false;
        self.cache = PrefixCache::default();
        self.flight = None;
        self.sentence_reading.clear();
        self.user_segments.clear();
    }

    fn now(&self) -> Instant {
        match &self.cfg.clock {
            Some(clock) => clock(),
            None => Instant::now(),
        }
    }

    fn preview_plan(&self) -> Plan {
        let mut plan = Plan::default();
        if let Some(draft) = &self.draft {
            plan.preview = true;
            plan.result = Some(draft.clone());
        }
        plan
    }

    fn prefix_intact(&self, seq: &[Unit]) -> bool {
        if self.cut == 0 {
            return true;
        }
        // 整段都被鎖定且沒有未鎖定尾巴時，`seq.len() == cut` 是同一次重畫，
        // 不能當成前綴被破壞；否則片段選字後的重畫會把鎖定清掉。
        // 還有尾巴時代表使用者倒退到只剩鎖定前綴，必須從頭轉換。
        if seq.len() < self.cut || self.locked_seq.len() < self.cut {
            return false;
        }
        if seq.len() == self.cut {
            return self.tail.is_empty() && self.locked_seq[..self.cut] == seq[..self.cut];
        }
        self.locked_seq[..self.cut] == seq[..self.cut]
    }

    fn convert_context(&self) -> JaContext {
        let mut ctx = self.ctx.clone();
        ctx.left = format!("{}{}", self.locked_text(), ctx.left);
        ctx
    }

    fn locked_text(&self) -> String {
        self.locked
            .iter()
            .map(|clause| clause.surface.as_str())
            .collect()
    }

    /// 輸入結束時把尚未轉換的讀音補齊。
    ///
    /// 自動変換只在觸發條件成立時轉換；使用者停止輸入後外殼應呼叫這個
    /// 方法，讓最後一段假名也得到統計結果。已轉換到最新讀音時不做任何事。
    pub fn flush(&mut self) -> (Plan, bool) {
        if !self.enabled() || self.seq.is_empty() {
            return (Plan::default(), false);
        }
        if self.cfg.deferred_sentence {
            // Enter 以外的結束不先轉；停頓到齊才轉換。
            return (self.preview_plan(), false);
        }
        if self.cut >= self.seq.len() || self.seq.len() <= self.converted_upto {
            return (self.preview_plan(), false);
        }
        self.pending = false;
        self.due = None;
        // 輸入結束不再需要避開邊界抖動；直接跳過 hold 才能把最後一段轉完。
        self.held = self.cfg.hold_limit;
        let plan = self.convert();
        let converted = plan.convert;
        (plan, converted)
    }

    /// 停頓後的自動変換與補正。
    ///
    /// 外殼在使用者閒置 250–400 ms 後呼叫。`Trigger::Auto` 且開啟 `idle`
    /// 時，先把尚未轉換的讀音轉成統計結果；`NeuralTiming::Deferred` 時再
    /// 把最後一個尚未鎖定的文節送模型補正。回傳的第二個值表示畫面是否有
    /// 更新。
    pub fn idle(&mut self) -> (Plan, bool) {
        if !self.enabled() {
            return (Plan::default(), false);
        }
        let mut updated = false;
        if self.cfg.deferred_sentence {
            self.idle_due = None;
            let plan = self.convert_sentence();
            let converted = plan.convert;
            return (plan, converted);
        }
        let idle_convert = match self.cfg.trigger {
            Trigger::Auto(trigger) => trigger.idle,
            Trigger::EveryKey => false,
        };
        if idle_convert && self.cut < self.seq.len() && self.seq.len() > self.converted_upto {
            self.pending = false;
            self.due = None;
            self.provisional = true;
            let plan = self.convert();
            self.provisional = false;
            updated |= plan.convert;
            if plan.timed_out {
                return (plan, updated);
            }
        }
        if let Some(plan) = self.poll_neural() {
            return (plan, true);
        }
        (self.preview_plan(), updated)
    }

    /// 套用等待中的非同步補正。
    ///
    /// 使用者已經繼續輸入時直接丟棄；模型超過 [`Config::neural_budget`]
    /// 也丟棄，維持統計結果。成功的補正只換最後一個尚未鎖定的文節。
    pub fn poll_neural(&mut self) -> Option<Plan> {
        if !self.enabled() {
            return None;
        }
        let request = self.pending_neural.take()?;
        if request.generation != self.generation {
            self.stats.stale_discards += 1;
            return None;
        }
        if self.seq.len() < self.cut + request.reading.chars().count() {
            self.stats.stale_discards += 1;
            return None;
        }
        let gate = self.cfg.neural.clone()?;
        let started = self.now();
        self.stats.neural_requests += 1;
        let decision = gate.adopt(&GateRequest {
            reading: &request.reading,
            left: &request.left,
            right: &request.right,
            statistical: &request.statistical,
            statistical_cost: request.statistical_cost,
            runner_up_cost: request.runner_up_cost,
            mean_candidates: request.mean_candidates,
            phase: GatePhase::Correction,
            generation: self.generation,
        });
        let elapsed = self.now().saturating_duration_since(started);
        self.stats.corrections += 1;
        if self.cfg.neural_budget > Duration::ZERO && elapsed > self.cfg.neural_budget {
            self.stats.late_discards += 1;
            return None;
        }
        let surface = decision?;
        if surface == request.statistical {
            return None;
        }
        let mut covered = 0;
        let mut index = 0;
        while index < self.tail.len() && covered < request.units {
            covered += clause_units(&self.tail[index]);
            index += 1;
        }
        if covered != request.units {
            self.stats.stale_discards += 1;
            return None;
        }
        let current: String = self.tail[..index].iter().map(clause_reading).collect();
        if current != request.reading {
            self.stats.stale_discards += 1;
            return None;
        }
        let mut clauses = vec![JaClause {
            reading: request.reading.clone(),
            surface,
            segments: Vec::new(),
            cost: request.cost,
        }];
        clauses.extend(self.tail[index..].iter().cloned());
        self.tail = clauses;
        self.stats.correction_adopted += 1;
        self.stats.neural_adopted += 1;
        let cost = self.last_cost;
        let score = self.last_score;
        Some(self.publish(0, cost, score))
    }

    fn should_convert(&self) -> bool {
        match self.cfg.trigger {
            Trigger::EveryKey => true,
            Trigger::Auto(trigger) => {
                let reached_min_units = trigger.min_units > 0 && {
                    let from = self.cut.max(self.converted_upto);
                    self.seq.len().saturating_sub(from) >= trigger.min_units
                };
                reached_min_units || (trigger.boundary && self.is_boundary())
            }
        }
    }

    /// 目前尚未鎖定的讀音是否到達一個自然的斷點。
    fn is_boundary(&self) -> bool {
        if self.cut >= self.seq.len() {
            return false;
        }
        let reading: String = self.seq[self.cut..].iter().map(Unit::as_str).collect();
        if ends_with_punctuation(&reading) || ends_with_particle(&reading) {
            return true;
        }
        self.conv.is_complete_word(&reading)
    }

    fn convert(&mut self) -> Plan {
        let legacy = matches!(self.cfg.trigger, Trigger::EveryKey)
            && self.cfg.neural_scope == NeuralScope::WholeTail
            && self.cfg.neural_timing == NeuralTiming::Inline;
        if legacy {
            return self.convert_legacy();
        }
        self.convert_auto()
    }

    fn convert_legacy(&mut self) -> Plan {
        if self.cut > self.seq.len() {
            self.reset();
            return Plan::default();
        }
        let suffix = &self.seq[self.cut..];
        if suffix.is_empty() {
            return self.preview_plan();
        }
        let reading: String = suffix.iter().map(Unit::as_str).collect();
        let ctx = self.convert_context();
        let start = self.now();
        // 神經開啟時用同一次 top-2 搜尋取得門檻訊號；關閉時仍是原本的
        // 單一路徑轉換，行為與先前完全相同。
        let converted = if self.cfg.neural.is_some() {
            self.conv.statistical(&reading, &ctx)
        } else {
            self.conv.convert(&reading, &ctx).map(Statistical::plain)
        };
        let elapsed = self.now().saturating_duration_since(start);
        self.stats.conversions += 1;
        self.stats.last_duration = elapsed;

        // 時間預算只算統計轉換；神經呼叫有自己的 180 ms 上限，逾時會回統計
        // 結果，不讓外層預算把已經算好的統計結果一起丟掉。
        let timed_out = self.cfg.budget > Duration::ZERO && elapsed > self.cfg.budget;
        if timed_out {
            self.stats.timeouts += 1;
        }

        let statistical = match converted {
            Ok(statistical)
                if !statistical.result.clauses.is_empty()
                    && clauses_units(&statistical.result.clauses) == suffix.len() =>
            {
                statistical
            }
            _ => {
                let mut plan = self.preview_plan();
                plan.timed_out = timed_out;
                return plan;
            }
        };
        if timed_out {
            let mut plan = self.preview_plan();
            plan.timed_out = true;
            return plan;
        }

        let (result, neural) = self.adopt_neural(statistical, &reading, &ctx);
        self.stats.last_duration = self.now().saturating_duration_since(start);

        // 神經輸出沒有詞邊界；上一次是神經結果時不比較邊界，直接以這次的
        // 統計結果重建基準，否則每次採用神經輸出都會被誤判成大幅變動。
        let changes = if self.tail_neural {
            0
        } else {
            boundary_changes(&self.tail, &result.clauses)
        };
        self.stats.last_boundary_changes = changes;
        if self.cfg.max_boundary_changes > 0
            && self.draft.is_some()
            && changes as isize > self.cfg.max_boundary_changes
            && self.held < self.cfg.hold_limit
        {
            self.held += 1;
            self.stats.holds += 1;
            let mut plan = self.preview_plan();
            plan.boundary_changes = changes;
            return plan;
        }

        self.held = 0;
        self.tail = result.clauses.clone();
        self.tail_neural = neural;
        self.advance_lock();
        self.converted_upto = self.seq.len();
        self.last_cost = result.cost;
        self.last_score = result.score;
        let mut plan = self.publish(changes, result.cost, result.score);
        plan.boundary_changes = changes;
        plan
    }

    /// 讓目前的整句轉換失效，回到假名並記錄系統造成的變動。
    fn invalidate_sentence(&mut self) {
        if self.draft.is_some() {
            let changed = self
                .prev_display
                .iter()
                .filter(|fragment| !fragment.kana)
                .count();
            self.stats.fragments_shown += changed;
            self.stats.fragments_changed += changed;
            self.stats.settled_shown += changed;
            self.stats.settled_changed += changed;
            self.draft = None;
            self.prev_display.clear();
        }
        self.tail.clear();
        self.cut = 0;
        self.converted_upto = 0;
        self.sentence_reading.clear();
        self.user_segments.clear();
    }

    /// 停頓後做一次完整轉換；這是延遲整句模式的唯一轉換點。
    fn convert_sentence(&mut self) -> Plan {
        if self.seq.is_empty() {
            return self.preview_plan();
        }
        let reading: String = self.seq.iter().map(Unit::as_str).collect();
        let chars: Vec<char> = reading.chars().collect();
        let total = self.seq.len();
        let mut users = self.user_segments.clone();
        users.sort_by_key(|segment| segment.start);
        users.retain(|segment| segment.start + segment.units <= total);

        let mut clauses: Vec<JaClause> = Vec::new();
        let mut cost = 0.0;
        let mut score = 0.0;
        let mut cursor = 0usize;
        let mut elapsed = Duration::ZERO;
        for (index, segment) in users.iter().enumerate() {
            let gap: String = chars[cursor..segment.start].iter().collect();
            if !gap.is_empty() {
                let left = format!("{}{}", self.ctx.left, surface_of(&clauses));
                let mut right = self.ctx.right.clone();
                for later in &users[index..] {
                    right.push_str(&later.surface);
                }
                let started = self.now();
                let Some((result, _)) = self.convert_range(&gap, &JaContext { left, right }) else {
                    return self.preview_plan();
                };
                elapsed += self.now().saturating_duration_since(started);
                cost += result.cost;
                score += result.score;
                clauses.extend(result.clauses);
            }
            clauses.push(JaClause {
                reading: segment.reading.clone(),
                surface: segment.surface.clone(),
                segments: Vec::new(),
                cost: 0,
            });
            cursor = segment.start + segment.units;
        }
        if cursor < chars.len() {
            let gap: String = chars[cursor..].iter().collect();
            let left = format!("{}{}", self.ctx.left, surface_of(&clauses));
            let started = self.now();
            let Some((result, _)) = self.convert_range(
                &gap,
                &JaContext {
                    left,
                    right: self.ctx.right.clone(),
                },
            ) else {
                return self.preview_plan();
            };
            elapsed += self.now().saturating_duration_since(started);
            cost += result.cost;
            score += result.score;
            clauses.extend(result.clauses);
        }
        if clauses.is_empty() || clauses_units(&clauses) != total {
            return self.preview_plan();
        }
        self.stats.conversions += 1;
        self.stats.last_duration = elapsed;
        if self.cfg.budget > Duration::ZERO && elapsed > self.cfg.budget {
            self.stats.timeouts += 1;
            return self.preview_plan();
        }
        self.last_cost = cost;
        self.last_score = score;
        self.tail = clauses;
        self.tail_neural = false;
        self.cut = 0;
        self.converted_upto = self.seq.len();
        self.sentence_reading = reading;
        self.publish(0, cost, score)
    }

    /// 轉換一段讀音；神經開啟時套用現有門檻與五條驗證。
    fn convert_range(&self, reading: &str, ctx: &JaContext) -> Option<(JaResult, bool)> {
        let converted = if self.cfg.neural.is_some() {
            self.conv.statistical(reading, ctx)
        } else {
            self.conv.convert(reading, ctx).map(Statistical::plain)
        };
        let statistical = converted.ok()?;
        if statistical.result.clauses.is_empty()
            || clauses_units(&statistical.result.clauses) != reading.chars().count()
        {
            return None;
        }
        if self.cfg.neural.is_some() {
            Some(self.adopt_neural(statistical, reading, ctx))
        } else {
            Some((statistical.result, false))
        }
    }

    /// 是否為延遲整句模式；外殼的 Enter 需要先同步轉換。
    pub fn sentence_mode(&self) -> bool {
        self.cfg.deferred_sentence
    }

    /// 立即做一次整句轉換；Enter 在假名狀態下使用。
    pub fn convert_now(&mut self) -> Plan {
        if !self.cfg.deferred_sentence {
            return self.convert();
        }
        self.idle_due = None;
        self.convert_sentence()
    }

    /// 把前綴快取裁到與目前讀音的共同前綴。
    fn sync_cache(&mut self) {
        let current: String = self.seq.iter().map(Unit::as_str).collect();
        let common = common_prefix_units(&self.cache.reading, &current);
        let mut units = 0;
        let mut keep = 0;
        for clause in &self.cache.clauses {
            let size = clause_units(clause);
            if units + size > common {
                break;
            }
            units += size;
            keep += 1;
        }
        self.cache.clauses.truncate(keep);
        self.cache.reading = self
            .cache
            .clauses
            .iter()
            .map(clause_reading)
            .collect::<String>();
    }

    /// 前綴快取路徑：只轉未快取的尾巴，結果接在快取後面。
    fn convert_cache(&mut self) -> Plan {
        if self.cache.units() > self.seq.len() {
            self.cache = PrefixCache::default();
        }
        let start = self.cache.units();
        let suffix = &self.seq[start..];
        if suffix.is_empty() {
            return self.preview_plan();
        }
        let reading: String = suffix.iter().map(Unit::as_str).collect();
        let mut ctx = self.ctx.clone();
        ctx.left = format!("{}{}", self.cache.text(), ctx.left);
        let started = self.now();
        let converted = self.conv.convert(&reading, &ctx);
        let elapsed = self.now().saturating_duration_since(started);
        let timed_out = self.cfg.budget > Duration::ZERO && elapsed > self.cfg.budget;
        self.stats.conversions += 1;
        self.stats.last_duration = elapsed;
        if timed_out {
            self.stats.timeouts += 1;
            let mut plan = self.preview_plan();
            plan.timed_out = true;
            return plan;
        }
        let result = match converted {
            Ok(result)
                if !result.clauses.is_empty() && clauses_units(&result.clauses) == suffix.len() =>
            {
                result
            }
            _ => return self.preview_plan(),
        };
        self.last_cost = result.cost;
        self.last_score = result.score;
        let mut clauses = result.clauses;
        if self.flight.is_some() {
            // 推論中：統計結果只當畫面，不快取，讓送出的前綴保持可套用。
            self.tail = clauses;
            let plan = self.publish(0, self.last_cost, self.last_score);
            self.pump_flight(true);
            return plan;
        }
        let keep = self.cfg.cache_keep.min(clauses.len());
        let cache_n = clauses.len() - keep;
        let tail = clauses.split_off(cache_n);
        self.cache.clauses.extend(clauses);
        self.cache.reading = self
            .cache
            .clauses
            .iter()
            .map(clause_reading)
            .collect::<String>();
        self.tail = tail;
        let plan = self.publish(0, self.last_cost, self.last_score);
        self.pump_flight(true);
        plan
    }

    /// 輪詢 single-flight 結果；套用後回傳新的顯示計畫。
    pub fn poll_flight(&mut self) -> Option<Plan> {
        if !self.cfg.neural_flight {
            return None;
        }
        self.pump_flight(true)
    }

    /// 取出結果、套用，並在需要時再送一次。
    fn pump_flight(&mut self, start_new: bool) -> Option<Plan> {
        let gate = self.cfg.neural.clone()?;
        let mut plan = None;
        if let Some(result) = gate.take() {
            plan = self.apply_flight(result);
        }
        if start_new && !gate.busy() {
            self.start_flight();
        }
        plan
    }

    /// 送出目前未快取尾巴的推論；已有請求時不做事。
    fn start_flight(&mut self) {
        let Some(gate) = self.cfg.neural.clone() else {
            return;
        };
        if gate.busy() || self.tail.is_empty() {
            return;
        }
        let reading: String = self.tail.iter().map(clause_reading).collect();
        if reading.is_empty() {
            return;
        }
        let mut ctx = self.ctx.clone();
        ctx.left = format!("{}{}", self.cache.text(), ctx.left);
        let Ok(statistical) = self.conv.statistical(&reading, &ctx) else {
            return;
        };
        if statistical.result.clauses.is_empty() {
            return;
        }
        let baseline = statistical.result.surface();
        let units: usize = self.tail.iter().map(clause_units).sum();
        let prefix = format!("{}{}", self.cache.reading, reading);
        let request = GateRequest {
            reading: &reading,
            left: &ctx.left,
            right: &ctx.right,
            statistical: &baseline,
            statistical_cost: statistical.result.conversion().cost,
            runner_up_cost: statistical.runner_up_cost,
            mean_candidates: statistical.mean_candidates,
            phase: GatePhase::Live,
            generation: self.generation,
        };
        self.stats.neural_requests += 1;
        if gate.start(&request) {
            self.flight = Some(FlightRequest {
                prefix,
                reading,
                units,
                statistical: baseline,
                cost: statistical
                    .result
                    .clauses
                    .iter()
                    .map(|clause| clause.cost)
                    .fold(0, i32::saturating_add),
            });
        }
    }

    /// 套用 single-flight 結果；只改尚未快取的尾巴。
    fn apply_flight(&mut self, result: FlightResult) -> Option<Plan> {
        let flight = self.flight.take()?;
        if !self.cfg.flight_append && result.generation != self.generation {
            self.stats.stale_discards += 1;
            return None;
        }
        let current: String = self.seq.iter().map(Unit::as_str).collect();
        let sent = format!("{}{}", flight.prefix, flight.reading);
        if !current.starts_with(&sent) || self.cache.reading != flight.prefix {
            self.stats.stale_discards += 1;
            return None;
        }
        let surface = result.surface?;
        if surface == flight.statistical {
            return None;
        }
        let mut covered = 0;
        let mut index = 0;
        while index < self.tail.len() && covered < flight.units {
            covered += clause_units(&self.tail[index]);
            index += 1;
        }
        if covered != flight.units {
            self.stats.stale_discards += 1;
            return None;
        }
        let read: String = self.tail[..index].iter().map(clause_reading).collect();
        if read != flight.reading {
            self.stats.stale_discards += 1;
            return None;
        }
        let neural = JaClause {
            reading: flight.reading.clone(),
            surface,
            segments: Vec::new(),
            cost: flight.cost,
        };
        let mut clauses = vec![neural];
        clauses.extend(self.tail[index..].iter().cloned());
        let keep = self.cfg.cache_keep.min(clauses.len());
        let cache_n = clauses.len() - keep;
        let tail = clauses.split_off(cache_n);
        self.cache.clauses.extend(clauses);
        self.cache.reading = self
            .cache
            .clauses
            .iter()
            .map(clause_reading)
            .collect::<String>();
        self.tail = tail;
        self.stats.neural_adopted += 1;
        self.last_cost = flight.cost as f64;
        Some(self.publish(0, self.last_cost, self.last_score))
    }

    /// 呼叫神經採用器；只有通過門檻與五條驗證時才換掉整段尾端。
    ///
    /// 神經輸出沒有詞邊界，替換時整段當一個文節，與一般轉換相同。門檻與
    /// 驗證都在共用層，這裡只負責準備請求與套用結果。
    fn adopt_neural(
        &self,
        statistical: Statistical,
        reading: &str,
        ctx: &JaContext,
    ) -> (JaResult, bool) {
        let Some(gate) = &self.cfg.neural else {
            return (statistical.result, false);
        };
        let baseline = statistical.result.surface();
        let clause_cost = statistical
            .result
            .clauses
            .iter()
            .map(|clause| clause.cost)
            .fold(0i32, i32::saturating_add);
        let request = GateRequest {
            reading,
            left: &ctx.left,
            right: &ctx.right,
            statistical: &baseline,
            statistical_cost: statistical.result.conversion().cost,
            runner_up_cost: statistical.runner_up_cost,
            mean_candidates: statistical.mean_candidates,
            phase: GatePhase::Live,
            generation: self.generation,
        };
        let Some(surface) = gate.adopt(&request) else {
            return (statistical.result, false);
        };
        let mut result = statistical.result;
        result.clauses = vec![JaClause {
            reading: reading.to_string(),
            surface,
            segments: Vec::new(),
            cost: clause_cost,
        }];
        (result, true)
    }

    /// 自動変換路徑：統計結果先顯示，神經只補最後一個未鎖定文節。
    ///
    /// 與 [`Controller::convert_legacy`] 的差異是神經輸出不再把整段尾端
    /// 併成一個文節；統計文節邊界會保留，[`Controller::advance_lock`] 因此
    /// 能繼續前進，已鎖定的前綴不會每一鍵重送。
    fn convert_auto(&mut self) -> Plan {
        if self.cut > self.seq.len() {
            self.reset();
            return Plan::default();
        }
        let suffix = &self.seq[self.cut..];
        if suffix.is_empty() {
            return self.preview_plan();
        }
        let reading: String = suffix.iter().map(Unit::as_str).collect();
        let ctx = self.convert_context();
        let start = self.now();
        let whole_tail_neural = self.cfg.neural_scope == NeuralScope::WholeTail
            && self.cfg.neural_timing == NeuralTiming::Inline
            && self.cfg.neural.is_some();
        let converted = if whole_tail_neural {
            self.conv.statistical(&reading, &ctx)
        } else {
            self.conv.convert(&reading, &ctx).map(Statistical::plain)
        };
        let elapsed = self.now().saturating_duration_since(start);
        self.stats.conversions += 1;
        self.stats.last_duration = elapsed;
        let timed_out = self.cfg.budget > Duration::ZERO && elapsed > self.cfg.budget;
        if timed_out {
            self.stats.timeouts += 1;
        }
        let statistical = match converted {
            Ok(result)
                if !result.result.clauses.is_empty()
                    && clauses_units(&result.result.clauses) == suffix.len() =>
            {
                result
            }
            _ => {
                let mut plan = self.preview_plan();
                plan.timed_out = timed_out;
                return plan;
            }
        };
        if timed_out {
            let mut plan = self.preview_plan();
            plan.timed_out = true;
            return plan;
        }

        let (statistical, neural) = if whole_tail_neural {
            self.stats.neural_requests += 1;
            self.adopt_neural(statistical, &reading, &ctx)
        } else {
            (statistical.result, false)
        };
        let changes = if self.tail_neural {
            0
        } else {
            boundary_changes(&self.tail, &statistical.clauses)
        };
        self.stats.last_boundary_changes = changes;
        if self.cfg.max_boundary_changes > 0
            && self.draft.is_some()
            && changes as isize > self.cfg.max_boundary_changes
            && self.held < self.cfg.hold_limit
        {
            self.held += 1;
            self.stats.holds += 1;
            let mut plan = self.preview_plan();
            plan.boundary_changes = changes;
            return plan;
        }

        self.held = 0;
        self.last_cost = statistical.cost;
        self.last_score = statistical.score;
        self.tail = statistical.clauses;
        self.tail_neural = neural;
        if !self.provisional {
            self.advance_lock();
        }
        self.converted_upto = self.seq.len();
        let plan = self.publish(changes, self.last_cost, self.last_score);

        match self.cfg.neural_timing {
            NeuralTiming::Inline => {
                if self.apply_clause_neural(GatePhase::Live) {
                    let cost = self.last_cost;
                    let score = self.last_score;
                    return self.publish(changes, cost, score);
                }
            }
            NeuralTiming::Deferred => self.queue_neural(),
        }
        plan
    }

    /// 對最後一個未鎖定文節呼叫神經採用器；成功時只換該文節的詞面。
    ///
    /// 統計訊號由該文節自己的讀音重新解一次 top-2 取得，較早的文節與已
    /// 鎖定前綴不重算。回傳是否真的換了詞面。
    fn apply_clause_neural(&mut self, phase: GatePhase) -> bool {
        if self.cfg.neural.is_none() {
            return false;
        }
        let Some(request) = self.clause_request(phase) else {
            return false;
        };
        let Some(gate) = &self.cfg.neural else {
            return false;
        };
        self.stats.neural_requests += 1;
        let started = self.now();
        let decision = gate.adopt(&GateRequest {
            reading: &request.reading,
            left: &request.left,
            right: &request.right,
            statistical: &request.statistical,
            statistical_cost: request.statistical_cost,
            runner_up_cost: request.runner_up_cost,
            mean_candidates: request.mean_candidates,
            phase,
            generation: self.generation,
        });
        let elapsed = self.now().saturating_duration_since(started);
        if self.cfg.neural_budget > Duration::ZERO && elapsed > self.cfg.neural_budget {
            if phase == GatePhase::Correction {
                self.stats.late_discards += 1;
            }
            return false;
        }
        let Some(surface) = decision else {
            return false;
        };
        if surface == request.statistical {
            return false;
        }
        let Some(last) = self.tail.last_mut() else {
            return false;
        };
        if clause_reading(last) != request.reading {
            return false;
        }
        last.surface = surface;
        last.segments.clear();
        self.stats.neural_adopted += 1;
        if phase == GatePhase::Correction {
            self.stats.correction_adopted += 1;
        }
        true
    }

    /// 建立補正請求，等停頓後再送模型。
    fn queue_neural(&mut self) {
        let Some(request) = self.tail_request() else {
            return;
        };
        self.pending_neural = Some(PendingNeural {
            generation: self.generation,
            reading: request.reading,
            left: request.left,
            right: request.right,
            statistical: request.statistical,
            statistical_cost: request.statistical_cost,
            runner_up_cost: request.runner_up_cost,
            mean_candidates: request.mean_candidates,
            cost: request.cost,
            units: request.units,
        });
    }

    /// 尚未鎖定尾巴的補正請求。
    ///
    /// 停頓補正一次處理整段未鎖定尾巴，不動已鎖定前綴；最後一個文節若
    /// 仍是かな，單獨送模型會被半截讀音規則跳過，因此這裡整段送。
    fn tail_request(&mut self) -> Option<PendingRequest> {
        // 尾端標點不是模型要補的內容；補正只送標點前的未鎖定片段。
        let mut end = self.tail.len();
        while end > 0 && is_punctuation_reading(&clause_reading(&self.tail[end - 1])) {
            end -= 1;
        }
        if end == 0 {
            return None;
        }
        let reading: String = self.tail[..end].iter().map(clause_reading).collect();
        if reading.is_empty() {
            return None;
        }
        let units = self.tail[..end].iter().map(clause_units).sum();
        let mut left = self.ctx.left.clone();
        left.push_str(&self.locked_text());
        let right = self.ctx.right.clone();
        let ctx = JaContext {
            left: left.clone(),
            right: right.clone(),
        };
        let statistical = self.conv.statistical(&reading, &ctx).ok()?;
        if statistical.result.clauses.is_empty() {
            return None;
        }
        let cost = statistical
            .result
            .clauses
            .iter()
            .map(|clause| clause.cost)
            .fold(0i32, i32::saturating_add);
        Some(PendingRequest {
            reading,
            left,
            right,
            statistical: statistical.result.surface(),
            statistical_cost: statistical.result.conversion().cost,
            runner_up_cost: statistical.runner_up_cost,
            mean_candidates: statistical.mean_candidates,
            cost,
            units,
        })
    }

    /// 最後一個未鎖定文節的採用請求；沒有可補的片段時回傳 `None`。
    fn clause_request(&self, _phase: GatePhase) -> Option<ClauseRequest> {
        let clause = self.tail.last()?;
        let reading = clause_reading(clause);
        if reading.is_empty() {
            return None;
        }
        let mut left = self.ctx.left.clone();
        left.push_str(&self.locked_text());
        for earlier in &self.tail[..self.tail.len() - 1] {
            left.push_str(&earlier.surface);
        }
        let right = self.ctx.right.clone();
        let ctx = JaContext {
            left: left.clone(),
            right: right.clone(),
        };
        let statistical = self.conv.statistical(&reading, &ctx).ok()?;
        if statistical.result.clauses.is_empty() {
            return None;
        }
        Some(ClauseRequest {
            reading,
            left,
            right,
            statistical: statistical.result.surface(),
            statistical_cost: statistical.result.conversion().cost,
            runner_up_cost: statistical.runner_up_cost,
            mean_candidates: statistical.mean_candidates,
        })
    }

    /// 把目前的鎖定與文節組成結果，更新 draft 並記錄穩定度。
    fn publish(&mut self, changes: usize, cost: f64, score: f64) -> Plan {
        let clauses = self.all_clauses();
        self.record_stability(&clauses);
        self.draft = Some(JaResult {
            clauses,
            cost,
            score,
        });
        Plan {
            convert: true,
            result: self.draft.clone(),
            boundary_changes: changes,
            ..Plan::default()
        }
    }

    /// 比較這次顯示的片段與上次，累計「已顯示片段被改掉」的比例。
    ///
    /// 片段以讀音範圍定位。上次顯示的最後一個文節是即時尾巴，本來就會
    /// 隨輸入變動；另外統計「已穩定片段」的變動次數，那才是使用者感受到
    /// 「打完又變」的來源。
    fn record_stability(&mut self, clauses: &[JaClause]) {
        if self.prev_display.is_empty() {
            self.prev_display = display_fragments(clauses);
            return;
        }
        let next = display_fragments(clauses);
        let total: usize = next.last().map(|f| f.end).unwrap_or(0);
        for (index, prev) in self.prev_display.iter().enumerate() {
            if prev.end > total || prev.kana {
                // まだかなの片段本來就會被轉成漢字，不算「打完又變」。
                continue;
            }
            self.stats.fragments_shown += 1;
            let same = next
                .iter()
                .find(|f| f.start == prev.start && f.end == prev.end)
                .is_some_and(|f| f.surface == prev.surface);
            if !same {
                self.stats.fragments_changed += 1;
            }
            if index + 1 < self.prev_display.len() {
                self.stats.settled_shown += 1;
                if !same {
                    self.stats.settled_changed += 1;
                }
            }
        }
        self.prev_display = next;
    }

    /// 把尾端除了最後一個文節以外的部分鎖住。
    ///
    /// 最後一個文節仍可能隨下一個かな改變，所以留在 `tail`；已鎖住的文節
    /// 往後不會再被轉換。詞面仍是かな的文節（詞面與讀音相同）也留在
    /// `tail`：那通常只是長詞的片段，例如輸入「きょ」時只有未知語かな，
    /// 真正的詞要等「う」進來才成立。若鎖住這種前綴，跨界的詞就永遠
    /// 找不到。
    fn advance_lock(&mut self) {
        let keep = self.cfg.keep_tail.max(1);
        if self.tail.len() <= keep {
            return;
        }
        let mut n = self.tail.len() - keep;
        while n > 0 && is_kana_clause(&self.tail[n - 1]) {
            n -= 1;
        }
        if n == 0 {
            return;
        }
        let units = clauses_units(&self.tail[..n]);
        if units == 0 || self.cut + units > self.seq.len() {
            return;
        }
        self.locked.extend_from_slice(&self.tail[..n]);
        self.locked_seq
            .extend_from_slice(&self.seq[self.cut..self.cut + units]);
        self.cut += units;
        self.tail = self.tail[n..].to_vec();
    }

    fn all_clauses(&self) -> Vec<JaClause> {
        let mut out = Vec::with_capacity(self.locked.len() + self.tail.len());
        out.extend_from_slice(&self.locked);
        out.extend_from_slice(&self.tail);
        out
    }
}

/// 補齊設定的預設值，對齊前身 `New` 的處理。
fn normalize_config(mut cfg: Config) -> Config {
    if cfg.max_boundary_changes == 0 {
        cfg.max_boundary_changes = 1;
    }
    cfg
}

/// 比較舊尾端與新結果在重疊讀音內的文節邊界差異。
///
/// 超過重疊長度的新邊界不算變動，因為那只是尾端變長。
fn boundary_changes(old: &[JaClause], next: &[JaClause]) -> usize {
    let n = clauses_units(old).min(clauses_units(next));
    if n == 0 {
        return 0;
    }
    let old_set = boundaries(old, n);
    let new_set = boundaries(next, n);
    old_set.symmetric_difference(&new_set).count()
}

/// 回傳文節邊界位置，位置以讀音單位編號。
fn boundaries(clauses: &[JaClause], limit: usize) -> HashSet<usize> {
    let mut out = HashSet::new();
    let mut pos = 0;
    for (index, clause) in clauses.iter().enumerate() {
        pos += clause_units(clause);
        if index == clauses.len() - 1 || pos >= limit {
            break;
        }
        if pos > 0 && pos < limit {
            out.insert(pos);
        }
    }
    out
}

/// 文節序列的讀音單位總數。
fn clauses_units(clauses: &[JaClause]) -> usize {
    clauses.iter().map(clause_units).sum()
}

/// 單一文節的讀音單位數。
fn clause_units(clause: &JaClause) -> usize {
    clause_reading(clause).chars().count()
}

/// 詞面是否仍與讀音相同，也就是還沒轉換成漢字的かな文節。
fn is_kana_clause(clause: &JaClause) -> bool {
    clause.surface == clause_reading(clause)
}

/// 字串是否含平假名。
fn has_hiragana(text: &str) -> bool {
    text.chars()
        .any(|ch| ('ぁ'..='ゖ').contains(&ch) || ch == 'ゝ' || ch == 'ゞ')
}

/// 字串是否含片假名（含半角）。
fn has_katakana(text: &str) -> bool {
    text.chars().any(|ch| {
        ('ァ'..='ヺ').contains(&ch) || ch == 'ヽ' || ch == 'ヾ' || ('｡'..='ﾟ').contains(&ch)
    })
}

/// 文節讀音；`reading` 為空時由內部分詞的讀音串接。
fn clause_reading(clause: &JaClause) -> String {
    if !clause.reading.is_empty() {
        return clause.reading.clone();
    }
    clause
        .segments
        .iter()
        .map(|seg| seg.reading.as_str())
        .collect()
}

/// 把文節換成以讀音單位定位的顯示片段。
fn display_fragments(clauses: &[JaClause]) -> Vec<DisplayFragment> {
    let mut out = Vec::with_capacity(clauses.len());
    let mut pos = 0;
    for clause in clauses {
        let units = clause_units(clause);
        out.push(DisplayFragment {
            start: pos,
            end: pos + units,
            surface: clause.surface.clone(),
            kana: is_kana_clause(clause),
        });
        pos += units;
    }
    out
}

/// 文節序列的詞面串接。
fn surface_of(clauses: &[JaClause]) -> String {
    clauses
        .iter()
        .map(|clause| clause.surface.as_str())
        .collect()
}

/// 兩段讀音的共同前綴單位數。
fn common_prefix_units(a: &str, b: &str) -> usize {
    a.chars()
        .zip(b.chars())
        .take_while(|(left, right)| left == right)
        .count()
}

/// 讀音結尾是否為句讀或標點。
fn ends_with_punctuation(reading: &str) -> bool {
    matches!(
        reading.chars().last(),
        Some('、' | '。' | '，' | '．' | '！' | '？' | '!' | '?' | ',' | '.' | '…' | 'ー')
    )
}

/// 整段讀音是否只有標點。
fn is_punctuation_reading(reading: &str) -> bool {
    !reading.is_empty()
        && reading.chars().all(|ch| {
            matches!(
                ch,
                '、' | '。' | '，' | '．' | '！' | '？' | '!' | '?' | ',' | '.' | '…'
            )
        })
}

/// 讀音結尾是否為常見助詞或助動詞。
fn ends_with_particle(reading: &str) -> bool {
    const TAILS: &[&str] = &[
        "は",
        "が",
        "を",
        "に",
        "で",
        "と",
        "も",
        "の",
        "へ",
        "や",
        "か",
        "ね",
        "よ",
        "な",
        "ぞ",
        "さ",
        "わ",
        "から",
        "まで",
        "より",
        "ので",
        "のに",
        "けど",
        "です",
        "ます",
        "でした",
        "ました",
        "ません",
        "だろう",
        "でしょう",
    ];
    TAILS.iter().any(|tail| reading.ends_with(tail))
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;

    /// 可手動前進的虛擬時鐘。
    #[derive(Clone)]
    struct FakeClock {
        at: Rc<Cell<Instant>>,
    }

    impl FakeClock {
        fn new() -> Self {
            Self {
                at: Rc::new(Cell::new(Instant::now())),
            }
        }

        fn now(&self) -> Instant {
            self.at.get()
        }

        fn advance(&self, duration: Duration) {
            self.at.set(self.at.get() + duration);
        }

        fn closure(&self) -> Rc<dyn Fn() -> Instant> {
            let at = self.at.clone();
            Rc::new(move || at.get())
        }
    }

    type Split = Box<dyn Fn(&str) -> Vec<JaClause>>;

    /// 覆寫 `statistical()` 的門檻訊號：次佳成本與平均候選數。
    type Signals = Rc<Cell<Option<(Option<i32>, f32)>>>;

    /// 記錄呼叫內容的假轉換器；`split` 決定文節切法。
    struct FakeConverter {
        state: Rc<RefCell<FakeState>>,
        clock: Option<FakeClock>,
        cost: Rc<Cell<Duration>>,
        fail: Rc<Cell<bool>>,
        /// 覆寫 `statistical()` 的門檻訊號；`None` 時等於沒有次佳路徑。
        signals: Signals,
        split: Split,
    }

    #[derive(Default)]
    struct FakeState {
        calls: usize,
        statistical_calls: usize,
        contexts: Vec<JaContext>,
        spans: Vec<JaSpan>,
        seen: Vec<String>,
    }

    impl FakeConverter {
        fn new(split: impl Fn(&str) -> Vec<JaClause> + 'static) -> Self {
            Self {
                state: Rc::new(RefCell::new(FakeState::default())),
                clock: None,
                cost: Rc::new(Cell::new(Duration::ZERO)),
                fail: Rc::new(Cell::new(false)),
                signals: Rc::new(Cell::new(None)),
                split: Box::new(split),
            }
        }

        fn with_clock(mut self, clock: &FakeClock) -> Self {
            self.clock = Some(clock.clone());
            self
        }

        /// 設定 `statistical()` 要回報的門檻訊號。
        fn with_signals(self, runner_up_cost: Option<i32>, mean_candidates: f32) -> Self {
            self.signals.set(Some((runner_up_cost, mean_candidates)));
            self
        }

        fn state(&self) -> Rc<RefCell<FakeState>> {
            self.state.clone()
        }

        /// 取得錯誤開關；測試可以在控制器持有轉換器之後切換。
        fn fail_switch(&self) -> Rc<Cell<bool>> {
            self.fail.clone()
        }

        /// 取得耗時開關；測試可以在控制器持有轉換器之後調整。
        fn cost_switch(&self) -> Rc<Cell<Duration>> {
            self.cost.clone()
        }
    }

    impl Converter for FakeConverter {
        fn convert(&self, reading: &str, ctx: &JaContext) -> Result<JaResult, ConvertError> {
            let mut state = self.state.borrow_mut();
            state.calls += 1;
            state.contexts.push(ctx.clone());
            state.seen.push(reading.to_string());
            drop(state);
            if let Some(clock) = &self.clock {
                clock.advance(self.cost.get());
            }
            if self.fail.get() {
                return Err(ConvertError::new("boom"));
            }
            Ok(JaResult {
                clauses: (self.split)(reading),
                cost: 0.0,
                score: 0.0,
            })
        }

        fn statistical(&self, reading: &str, ctx: &JaContext) -> Result<Statistical, ConvertError> {
            self.state.borrow_mut().statistical_calls += 1;
            let result = self.convert(reading, ctx)?;
            let (runner_up_cost, mean_candidates) = self.signals.get().unwrap_or((None, 1.0));
            Ok(Statistical {
                result,
                runner_up_cost,
                mean_candidates,
            })
        }

        fn candidates(
            &self,
            reading: &str,
            span: JaSpan,
            _ctx: &JaContext,
        ) -> Result<Vec<JaCandidate>, ConvertError> {
            self.state.borrow_mut().spans.push(span);
            let units: Vec<char> = reading.chars().collect();
            let sub: String = units[span.start..span.end()].iter().collect();
            Ok(vec![JaCandidate {
                surface: "候補".to_string(),
                reading: sub,
                cost: 0,
                source: crate::ja::decode::JaCandidateSource::Convert,
            }])
        }
    }

    fn clause(reading: &str) -> JaClause {
        JaClause {
            reading: reading.to_string(),
            surface: format!("[{reading}]"),
            segments: Vec::new(),
            cost: 0,
        }
    }

    /// 每 `n` 個かな切成一個文節，詞面帶出讀音，方便觀察前綴是否穩定。
    fn split_every(n: usize) -> impl Fn(&str) -> Vec<JaClause> {
        move |reading| {
            let chars: Vec<char> = reading.chars().collect();
            let mut out = Vec::new();
            let mut at = 0;
            while at < chars.len() {
                let end = (at + n).min(chars.len());
                let part: String = chars[at..end].iter().collect();
                out.push(clause(&part));
                at = end;
            }
            out
        }
    }

    fn split_pattern(reading: &str, sizes: &[usize]) -> Vec<JaClause> {
        let chars: Vec<char> = reading.chars().collect();
        let mut out = Vec::new();
        let mut at = 0;
        for size in sizes {
            if *size == 0 || at >= chars.len() {
                break;
            }
            let end = (at + size).min(chars.len());
            let part: String = chars[at..end].iter().collect();
            out.push(clause(&part));
            at = end;
        }
        if at < chars.len() {
            let part: String = chars[at..].iter().collect();
            out.push(clause(&part));
        }
        out
    }

    fn units(text: &str) -> Sequence {
        text.chars().map(|ch| Unit::new(ch.to_string())).collect()
    }

    fn result_text(result: &JaResult) -> String {
        result
            .clauses
            .iter()
            .map(|clause| clause.surface.as_str())
            .collect()
    }

    /// 結果中「除了最後一個文節」的字串；ライブ変換承諾這一段不變。
    fn stable_text(result: &JaResult) -> String {
        if result.clauses.len() <= 1 {
            return String::new();
        }
        result.clauses[..result.clauses.len() - 1]
            .iter()
            .map(|clause| clause.surface.as_str())
            .collect()
    }

    fn test_config(clock: &FakeClock) -> Config {
        Config {
            enabled: true,
            trigger: Trigger::EveryKey,
            neural_scope: NeuralScope::WholeTail,
            neural_timing: NeuralTiming::Inline,
            deferred_sentence: false,
            prefix_cache: false,
            clock: Some(clock.closure()),
            ..Config::default()
        }
    }

    #[test]
    fn test_default_config_enables_auto_live() {
        let clock = FakeClock::new();
        let conv = FakeConverter::new(split_every(2)).with_clock(&clock);
        let state = conv.state();
        let cfg = test_config(&clock);
        assert!(cfg.enabled, "測試設定要開啟");
        let mut controller = Controller::new(conv, cfg);
        // 預設設定是 120 ms debounce＋自動変換：短讀音不會每鍵轉換。
        let plan = controller.input(&units("きょう"), &JaContext::default());
        assert!(!plan.convert, "自動変換不應在短讀音每鍵轉換：{plan:?}");
        assert_eq!(state.borrow().calls, 0, "未到觸發條件不得呼叫轉換器");
        // 停頓到期後仍會補一次統計結果。
        clock.advance(Duration::from_millis(400));
        let (plan, converted) = controller.tick();
        assert!(converted && plan.convert, "{plan:?}");
        assert_eq!(state.borrow().calls, 1);
    }

    #[test]
    fn test_debounce_and_tick() {
        let clock = FakeClock::new();
        let conv = FakeConverter::new(split_every(2)).with_clock(&clock);
        let state = conv.state();
        let mut cfg = test_config(&clock);
        cfg.debounce = Duration::from_millis(100);
        let mut controller = Controller::new(conv, cfg);

        let start = clock.now();
        let plan = controller.input(&units("きょう"), &JaContext::default());
        assert!(plan.pending && !plan.convert && !plan.preview);
        assert_eq!(plan.deadline, Some(start + Duration::from_millis(100)));
        clock.advance(Duration::from_millis(50));
        assert!(!controller.tick().1, "期限未到不得轉換");
        clock.advance(Duration::from_millis(50));
        let (plan, ok) = controller.tick();
        assert!(
            ok && plan.convert && !plan.preview && !plan.pending,
            "{plan:?}"
        );
        assert_eq!(state.borrow().calls, 1);
        assert!(!controller.tick().1, "轉換後不得重複執行");
    }

    #[test]
    fn test_debounce_restarts_and_cancel() {
        let clock = FakeClock::new();
        let conv = FakeConverter::new(split_every(2)).with_clock(&clock);
        let state = conv.state();
        let mut cfg = test_config(&clock);
        cfg.debounce = Duration::from_millis(100);
        let mut controller = Controller::new(conv, cfg);

        controller.input(&units("き"), &JaContext::default());
        clock.advance(Duration::from_millis(80));
        controller.input(&units("きょ"), &JaContext::default());
        clock.advance(Duration::from_millis(30));
        assert!(!controller.tick().1, "重設後不應在舊期限轉換");
        clock.advance(Duration::from_millis(70));
        let (plan, ok) = controller.tick();
        assert!(ok && plan.convert, "{plan:?}");

        controller.input(&units("きょう"), &JaContext::default());
        assert!(controller.pending());
        assert!(controller.cancel_pending());
        assert!(!controller.pending());
        clock.advance(Duration::from_secs(1));
        assert!(!controller.tick().1, "取消後不得轉換");
        assert_eq!(state.borrow().calls, 1);
    }

    #[test]
    fn test_previous_result_is_draft_while_pending() {
        let clock = FakeClock::new();
        let conv = FakeConverter::new(split_every(2)).with_clock(&clock);
        let mut cfg = test_config(&clock);
        cfg.debounce = Duration::from_millis(100);
        let mut controller = Controller::new(conv, cfg);

        controller.input(&units("きょう"), &JaContext::default());
        clock.advance(Duration::from_millis(100));
        let (first, ok) = controller.tick();
        assert!(ok && first.convert, "{first:?}");
        let draft = result_text(first.result.as_ref().expect("有結果"));

        let plan = controller.input(&units("きょうは"), &JaContext::default());
        assert!(plan.pending && plan.preview && !plan.convert, "{plan:?}");
        assert_eq!(result_text(plan.result.as_ref().expect("有結果")), draft);
        let stored = controller.draft().expect("有 draft");
        assert_eq!(result_text(stored), draft);
    }

    #[test]
    fn test_locked_prefix_stable_across_twenty_kana() {
        let clock = FakeClock::new();
        let conv = FakeConverter::new(split_every(2)).with_clock(&clock);
        let mut cfg = test_config(&clock);
        cfg.debounce = Duration::from_millis(30);
        let mut controller = Controller::new(conv, cfg);

        let keys = "あいうえおかきくけこさしすせそたちつてと";
        let runes: Vec<char> = keys.chars().collect();
        let mut locked = String::new();
        let mut conversions = 0usize;
        let mut snapshots: Vec<String> = Vec::new();

        for i in 0..runes.len() {
            clock.advance(Duration::from_millis(10));
            let seq = units(&runes[..=i].iter().collect::<String>());
            let plan = controller.input(&seq, &JaContext::default());
            if plan.convert {
                conversions += 1;
            }
            // 每三個かな模擬一次停頓，讓 debounce 到期。
            if (i + 1) % 3 == 0 {
                clock.advance(Duration::from_millis(40));
                loop {
                    let (plan, ok) = controller.tick();
                    if !ok {
                        break;
                    }
                    assert!(plan.convert, "第 {} 字轉換失敗：{plan:?}", i + 1);
                    conversions += 1;
                    let result = plan.result.as_ref().expect("有結果");
                    let got = stable_text(result);
                    assert!(
                        got.starts_with(&locked),
                        "第 {} 字已確定前段改變：{got:?} 不再是 {locked:?} 的延伸",
                        i + 1
                    );
                    locked = got;
                    snapshots.push(result_text(result));
                }
            }
        }
        clock.advance(Duration::from_secs(1));
        let (plan, ok) = controller.tick();
        if ok && plan.convert {
            conversions += 1;
            let result = plan.result.as_ref().expect("有結果");
            let got = stable_text(result);
            assert!(
                got.starts_with(&locked),
                "最後前段改變：{got:?} 不再是 {locked:?} 的延伸"
            );
        }
        assert!(conversions > 0, "20 字內完全沒有轉換");
        assert!(conversions <= 20, "轉換 {conversions} 次，超過按鍵數");
        assert!(!snapshots.is_empty(), "沒有轉換快照");
        assert!(snapshots.iter().all(|snapshot| !snapshot.is_empty()));
    }

    #[test]
    fn test_conversion_uses_locked_prefix_as_context() {
        let clock = FakeClock::new();
        let conv = FakeConverter::new(split_every(2));
        let state = conv.state();
        let mut cfg = test_config(&clock);
        cfg.debounce = Duration::ZERO;
        let mut controller = Controller::new(conv, cfg);

        let ctx = JaContext {
            left: "前".to_string(),
            right: String::new(),
        };
        let plan = controller.input(&units("きょう"), &ctx);
        assert!(plan.convert, "{plan:?}");
        {
            let state = state.borrow();
            assert_eq!(state.contexts[0].left, "前");
        }
        let locked = stable_text(plan.result.as_ref().expect("有結果"));
        assert!(!locked.is_empty(), "應至少鎖定一個文節");
        controller.input(&units("きょうは"), &ctx);
        {
            let state = state.borrow();
            assert_eq!(state.contexts.len(), 2);
            assert_eq!(state.contexts[1].left, format!("{locked}前"));
        }
    }

    #[test]
    fn test_backspace_drops_locked_prefix() {
        let clock = FakeClock::new();
        let conv = FakeConverter::new(split_every(2));
        let state = conv.state();
        let mut cfg = test_config(&clock);
        cfg.debounce = Duration::ZERO;
        let mut controller = Controller::new(conv, cfg);
        let ctx = JaContext {
            left: "前".to_string(),
            right: String::new(),
        };

        controller.input(&units("きょうは"), &ctx);
        // 倒退到比鎖定前綴更短，必須從頭轉換新的讀音。
        controller.input(&units("きょ"), &ctx);
        {
            let state = state.borrow();
            assert_eq!(state.seen.len(), 2);
            assert_eq!(state.seen[1], "きょ");
            assert_eq!(state.contexts[1].left, "前");
        }
    }

    #[test]
    fn test_budget_falls_back_to_draft() {
        let clock = FakeClock::new();
        let conv = FakeConverter::new(split_every(2)).with_clock(&clock);
        let cost = conv.cost_switch();
        let mut cfg = test_config(&clock);
        cfg.debounce = Duration::ZERO;
        cfg.budget = Duration::from_millis(100);
        let mut controller = Controller::new(conv, cfg);

        let first = controller.input(&units("きょう"), &JaContext::default());
        assert!(first.convert, "{first:?}");
        let draft = result_text(first.result.as_ref().expect("有結果"));
        cost.set(Duration::from_millis(150));

        let plan = controller.input(&units("きょうは"), &JaContext::default());
        assert!(plan.timed_out && !plan.convert && plan.preview, "{plan:?}");
        assert_eq!(result_text(plan.result.as_ref().expect("有結果")), draft);
        assert_eq!(controller.stats().timeouts, 1);
    }

    #[test]
    fn test_boundary_changes_are_capped() {
        let clock = FakeClock::new();
        let conv = FakeConverter::new(|reading| {
            let len = reading.chars().count();
            if len == 5 {
                split_pattern(reading, &[2, 3])
            } else {
                split_pattern(reading, &[1, 1, 1, len.saturating_sub(3)])
            }
        });
        let mut cfg = test_config(&clock);
        cfg.debounce = Duration::ZERO;
        cfg.max_boundary_changes = 1;
        cfg.hold_limit = 1;
        let mut controller = Controller::new(conv, cfg);

        let first = controller.input(&units("あいうえお"), &JaContext::default());
        assert!(first.convert, "{first:?}");
        let locked = stable_text(first.result.as_ref().expect("有結果"));
        let shown = result_text(first.result.as_ref().expect("有結果"));

        let plan = controller.input(&units("あいうえおか"), &JaContext::default());
        assert!(!plan.convert && plan.preview, "{plan:?}");
        assert_eq!(plan.boundary_changes, 2);
        assert_eq!(result_text(plan.result.as_ref().expect("有結果")), shown);
        assert_eq!(controller.stats().holds, 1);
        assert_eq!(stable_text(plan.result.as_ref().expect("有結果")), locked);

        // hold_limit 用完後仍要接受新結果，不能永久卡住。
        let plan = controller.input(&units("あいうえおか"), &JaContext::default());
        assert!(plan.convert, "{plan:?}");
        let got = stable_text(plan.result.as_ref().expect("有結果"));
        assert!(got.starts_with(&locked), "{got:?} 不再是 {locked:?} 的延伸");
    }

    #[test]
    fn test_conversion_error_keeps_draft() {
        let clock = FakeClock::new();
        let conv = FakeConverter::new(split_every(2));
        let fail = conv.fail_switch();
        let mut cfg = test_config(&clock);
        cfg.debounce = Duration::ZERO;
        let mut controller = Controller::new(conv, cfg);

        let first = controller.input(&units("きょう"), &JaContext::default());
        assert!(first.convert);
        let draft = result_text(first.result.as_ref().expect("有結果"));

        fail.set(true);
        let plan = controller.input(&units("きょうは"), &JaContext::default());
        assert!(!plan.convert && plan.preview, "{plan:?}");
        assert_eq!(result_text(plan.result.as_ref().expect("有結果")), draft);
    }

    #[test]
    fn test_candidates_and_invalid_span() {
        let clock = FakeClock::new();
        let conv = FakeConverter::new(split_every(2));
        let state = conv.state();
        let mut cfg = test_config(&clock);
        cfg.debounce = Duration::ZERO;
        let mut controller = Controller::new(conv, cfg);
        let ctx = JaContext {
            left: "前".to_string(),
            right: String::new(),
        };
        controller.input(&units("きょう"), &ctx);

        let cands = controller.candidates(JaSpan::new(1, 2)).expect("合法範圍");
        let surfaces: Vec<&str> = cands.iter().map(|c| c.surface.as_str()).collect();
        assert_eq!(surfaces, ["候補", "ょう", "ョウ", "ｮｳ"]);
        assert_eq!(state.borrow().spans, vec![JaSpan::new(1, 2)]);

        let err = controller.candidates(JaSpan::new(3, 9)).expect_err("越界");
        assert_eq!(err, LiveError::Span);
    }

    #[test]
    fn test_set_enabled_resets_state() {
        let clock = FakeClock::new();
        let conv = FakeConverter::new(split_every(2));
        let mut cfg = test_config(&clock);
        cfg.debounce = Duration::ZERO;
        let mut controller = Controller::new(conv, cfg);
        assert!(
            controller
                .input(&units("きょう"), &JaContext::default())
                .convert
        );
        assert!(controller.draft().is_some());

        controller.set_enabled(false);
        assert!(!controller.enabled(), "set_enabled(false) 沒有關閉");
        assert!(controller.draft().is_none(), "關閉後不得保留 draft");

        controller.set_enabled(true);
        let plan = controller.input(&units("きょう"), &JaContext::default());
        assert!(plan.convert, "重新開啟後應可轉換：{plan:?}");
    }

    #[test]
    fn test_disabled_config_is_noop() {
        let config = Config {
            enabled: false,
            ..Config::default()
        };
        let mut controller = Controller::new(FakeConverter::new(split_every(2)), config);
        assert!(!controller.enabled(), "enabled=false 必須關閉");
        let plan = controller.input(&units("き"), &JaContext::default());
        assert!(!plan.convert && !plan.pending, "{plan:?}");
    }

    /// 一次採用請求的快照，方便斷言控制器送了什麼。
    #[derive(Debug, Clone, PartialEq)]
    struct SeenRequest {
        reading: String,
        left: String,
        statistical: String,
        runner_up_cost: Option<i32>,
        mean_candidates: f32,
    }

    /// 固定回覆的神經採用器；回覆可事後調整。
    struct StubGate {
        reply: Rc<Cell<Option<&'static str>>>,
        seen: Rc<RefCell<Vec<SeenRequest>>>,
    }

    impl StubGate {
        fn new(reply: Option<&'static str>) -> Self {
            Self {
                reply: Rc::new(Cell::new(reply)),
                seen: Rc::new(RefCell::new(Vec::new())),
            }
        }
    }

    impl NeuralGate for StubGate {
        fn adopt(&self, request: &GateRequest<'_>) -> Option<String> {
            self.seen.borrow_mut().push(SeenRequest {
                reading: request.reading.to_string(),
                left: request.left.to_string(),
                statistical: request.statistical.to_string(),
                runner_up_cost: request.runner_up_cost,
                mean_candidates: request.mean_candidates,
            });
            self.reply.get().map(str::to_string)
        }
    }

    /// 採用神經輸出時整段尾端換成一個文節，門檻訊號跟著同一次轉換送出。
    #[test]
    fn test_neural_gate_replaces_whole_tail() {
        let clock = FakeClock::new();
        let gate = Rc::new(StubGate::new(Some("京")));
        let conv = FakeConverter::new(split_every(2)).with_signals(Some(1_472), 7.5);
        let state = conv.state();
        let mut cfg = test_config(&clock);
        cfg.debounce = Duration::ZERO;
        cfg.neural = Some(gate.clone());
        let mut controller = Controller::new(conv, cfg);

        let plan = controller.input(&units("きょう"), &JaContext::default());
        assert!(plan.convert, "{plan:?}");
        let result = plan.result.as_ref().expect("有結果");
        assert_eq!(result_text(result), "京");
        assert_eq!(result.clauses.len(), 1, "神經輸出整段當一個文節");
        assert_eq!(result.clauses[0].reading, "きょう");
        assert!(result.clauses[0].segments.is_empty());

        let seen = gate.seen.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].reading, "きょう");
        assert_eq!(seen[0].statistical, "[きょ][う]");
        assert_eq!(seen[0].runner_up_cost, Some(1_472));
        assert_eq!(seen[0].mean_candidates, 7.5);
        drop(seen);
        assert_eq!(
            state.borrow().statistical_calls,
            1,
            "神經開啟時走同一次 top-2 搜尋"
        );
    }

    /// 已鎖定的詞面會接進左文脈，與一般轉換的 context echo 判定一致。
    #[test]
    fn test_neural_gate_receives_locked_prefix_as_left() {
        let clock = FakeClock::new();
        let gate = Rc::new(StubGate::new(None));
        let conv = FakeConverter::new(split_every(2));
        let mut cfg = test_config(&clock);
        cfg.debounce = Duration::ZERO;
        cfg.neural = Some(gate.clone());
        let mut controller = Controller::new(conv, cfg);

        controller.input(&units("あいうえお"), &JaContext::default());
        controller.input(&units("あいうえおか"), &JaContext::default());
        let seen = gate.seen.borrow();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].reading, "あいうえお");
        assert_eq!(seen[0].left, "", "第一次轉換還沒有鎖定");
        // 逐字変換只送尚未鎖定的尾端；已鎖定的詞面接在左文脈，不重複放進
        // 讀音，否則模型輸出會跟鎖定前段重疊。
        assert_eq!(seen[1].reading, "おか", "只轉換尚未鎖定的尾端");
        assert_eq!(seen[1].left, "[あい][うえ]", "左文脈要含已鎖定的詞面");
    }

    /// 採用器回 `None` 時保留統計結果，連文節切法都不變。
    #[test]
    fn test_neural_gate_rejection_keeps_statistical_result() {
        let clock = FakeClock::new();
        let gate = Rc::new(StubGate::new(None));
        let conv = FakeConverter::new(split_every(2));
        let mut cfg = test_config(&clock);
        cfg.debounce = Duration::ZERO;
        cfg.neural = Some(gate.clone());
        let mut controller = Controller::new(conv, cfg);

        let plan = controller.input(&units("きょう"), &JaContext::default());
        let result = plan.result.as_ref().expect("有結果");
        assert_eq!(result_text(result), "[きょ][う]");
        assert_eq!(result.clauses.len(), 2);
        assert_eq!(gate.seen.borrow().len(), 1);
    }

    /// 沒有採用器時不呼叫統計訊號，結果與原本完全相同。
    #[test]
    fn test_neural_gate_absent_keeps_statistical_result() {
        let clock = FakeClock::new();
        let conv = FakeConverter::new(split_every(2));
        let state = conv.state();
        let mut cfg = test_config(&clock);
        cfg.debounce = Duration::ZERO;
        let mut controller = Controller::new(conv, cfg);

        let plan = controller.input(&units("きょう"), &JaContext::default());
        assert_eq!(
            result_text(plan.result.as_ref().expect("有結果")),
            "[きょ][う]"
        );
        assert_eq!(state.borrow().calls, 1);
        assert_eq!(
            state.borrow().statistical_calls,
            0,
            "關閉時不得為了門檻多解一次"
        );
    }

    /// 採用神經輸出後，下一次轉換不得被邊界變動誤判成大幅變動。
    ///
    /// 神經輸出整段只有一個文節，邊界集合是空的；若照統計結果的切法比較，
    /// 下一個統計結果的每個邊界都會被算成變動，把畫面鎖在舊結果。
    #[test]
    fn test_neural_accept_does_not_trigger_boundary_hold() {
        let clock = FakeClock::new();
        let gate = Rc::new(StubGate::new(Some("京")));
        let conv = FakeConverter::new(split_every(1));
        let mut cfg = test_config(&clock);
        cfg.debounce = Duration::ZERO;
        cfg.max_boundary_changes = 1;
        cfg.hold_limit = 1;
        cfg.neural = Some(gate.clone());
        let mut controller = Controller::new(conv, cfg);

        let first = controller.input(&units("あいう"), &JaContext::default());
        assert!(first.convert, "{first:?}");
        assert_eq!(result_text(first.result.as_ref().expect("有結果")), "京");

        gate.reply.set(None);
        let second = controller.input(&units("あいうえお"), &JaContext::default());
        assert!(second.convert, "神經採用後不得保留舊畫面：{second:?}");
        assert_eq!(second.boundary_changes, 0);
        assert_eq!(controller.stats().holds, 0);
        assert_eq!(
            result_text(second.result.as_ref().expect("有結果")),
            "[あ][い][う][え][お]"
        );
    }

    /// 片段候選包含目前詞面、原讀音平假名、片假名與半角かな。
    #[test]
    fn test_clause_candidates_include_kana_variants() {
        let clock = FakeClock::new();
        let conv = FakeConverter::new(split_every(2));
        let mut cfg = test_config(&clock);
        cfg.debounce = Duration::ZERO;
        let mut controller = Controller::new(conv, cfg);
        controller.input(&units("きょう"), &JaContext::default());
        let candidates = controller.candidates(JaSpan::new(0, 2)).expect("合法範圍");
        let surfaces: Vec<&str> = candidates
            .iter()
            .map(|candidate| candidate.surface.as_str())
            .collect();
        assert!(surfaces.contains(&"きょ"), "{surfaces:?}");
        assert!(surfaces.contains(&"キョ"), "{surfaces:?}");
        assert!(surfaces.contains(&"ｷｮ"), "{surfaces:?}");
    }

    /// 換片段只改那一段，其他片段、讀音與鎖定範圍都不變。
    #[test]
    fn test_replace_clause_keeps_other_clauses() {
        let clock = FakeClock::new();
        let conv = FakeConverter::new(split_every(2));
        let mut cfg = test_config(&clock);
        cfg.debounce = Duration::ZERO;
        let mut controller = Controller::new(conv, cfg);
        controller.input(&units("きょう"), &JaContext::default());
        let before = controller.clause_spans();
        assert_eq!(before.len(), 2);

        assert!(controller.replace_clause(0, "京".to_string()));
        let after = controller.clause_spans();
        assert_eq!(after[0].surface, "京");
        assert_eq!(after[1].surface, before[1].surface, "其他片段不得改變");
        assert_eq!((after[1].start, after[1].end), (2, 3), "讀音範圍不變");
        assert_eq!(result_text(controller.draft().expect("有結果")), "京[う]");
    }

    /// 假名變換鍵只換指定片段，回傳是否真的變了。
    #[test]
    fn test_transform_clause_kana_and_latin() {
        let clock = FakeClock::new();
        let conv = FakeConverter::new(|reading| {
            vec![JaClause {
                reading: reading.to_string(),
                surface: "abc".to_string(),
                segments: Vec::new(),
                cost: 0,
            }]
        });
        let mut cfg = test_config(&clock);
        cfg.debounce = Duration::ZERO;
        let mut controller = Controller::new(conv, cfg);
        controller.input(&units("えー"), &JaContext::default());
        assert!(controller.transform_clause(0, KanaTransform::FullwidthLatin));
        assert_eq!(controller.clause_spans()[0].surface, "ａｂｃ");
        assert!(controller.transform_clause(0, KanaTransform::HalfwidthLatin));
        assert_eq!(controller.clause_spans()[0].surface, "abc");
        assert!(!controller.transform_clause(0, KanaTransform::HalfwidthLatin));
    }

    /// 沒有鎖定時轉換整段；有鎖定時只動尾端片段。
    #[test]
    fn test_transform_clause_works_on_locked_prefix() {
        let clock = FakeClock::new();
        let conv = FakeConverter::new(split_every(2));
        let mut cfg = test_config(&clock);
        cfg.debounce = Duration::ZERO;
        let mut controller = Controller::new(conv, cfg);
        controller.input(&units("あいう"), &JaContext::default());
        // 第一次轉換鎖住 [あい]，尾端是 [う]。
        assert_eq!(controller.clause_spans()[0].surface, "[あい]");
        assert!(controller.transform_clause(0, KanaTransform::Katakana));
        assert_eq!(controller.clause_spans()[0].surface, "アイ");
        assert_eq!(controller.clause_spans()[1].surface, "[う]");
    }
}
