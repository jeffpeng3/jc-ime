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
//! - 預設關閉。關閉時 [`Controller::input`] 不做任何事，外殼照一般変換
//!   流程處理。

use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::ja::decode::{JaCandidate, JaClause, JaContext, JaDecoder, JaResult, JaSpan};
use crate::reading::{Sequence, Unit};

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
}

impl Converter for JaDecoder {
    fn convert(&self, reading: &str, ctx: &JaContext) -> Result<JaResult, ConvertError> {
        let units: Vec<char> = reading.chars().collect();
        self.results(&units, ctx, 1)
            .into_iter()
            .next()
            .ok_or_else(|| ConvertError::new("沒有轉換結果"))
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
    /// 單次轉換的時間預算；超過就退回前次結果，不鎖定新結果。
    /// 0 表示不檢查。
    pub budget: Duration,
    /// 單次轉換允許改變的文節邊界數，預設 1。負值表示不限制。
    pub max_boundary_changes: isize,
    /// 邊界變動過大時最多保留前次結果幾次，預設 2。0 表示不保留。
    pub hold_limit: usize,
    /// 開始ライブ変換所需的最少讀音單位，預設 1。
    pub min_units: usize,
    /// 時間來源；`None` 使用 [`Instant::now`]。測試可注入虛擬時鐘。
    pub clock: Option<Rc<dyn Fn() -> Instant>>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("enabled", &self.enabled)
            .field("debounce", &self.debounce)
            .field("budget", &self.budget)
            .field("max_boundary_changes", &self.max_boundary_changes)
            .field("hold_limit", &self.hold_limit)
            .field("min_units", &self.min_units)
            .finish_non_exhaustive()
    }
}

impl Default for Config {
    /// 建議值。`enabled` 保持 `false`，符合「預設關閉」。
    fn default() -> Self {
        Self {
            enabled: false,
            debounce: Duration::from_millis(120),
            budget: Duration::from_millis(100),
            max_boundary_changes: 1,
            hold_limit: 2,
            min_units: 1,
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
    held: usize,

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
            held: 0,
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
        if !self.prefix_intact(seq) {
            self.reset();
        }
        self.seq.clear();
        self.seq.extend_from_slice(seq);
        self.ctx = ctx.clone();

        if self.seq.len() < self.cfg.min_units {
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
        if !self.enabled() || !self.pending {
            return (Plan::default(), false);
        }
        let Some(due) = self.due else {
            return (Plan::default(), false);
        };
        if self.now() < due {
            return (Plan::default(), false);
        }
        self.pending = false;
        self.due = None;
        (self.convert(), true)
    }

    /// 取消尚未執行的 debounce，保留目前已顯示的結果。
    ///
    /// 回傳是否有東西被取消。
    pub fn cancel_pending(&mut self) -> bool {
        if !self.pending {
            return false;
        }
        self.pending = false;
        self.due = None;
        true
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
    /// 讓尾端候選有正確的左文脈。功能關閉時回傳空清單，外殼走一般変換。
    pub fn candidates(&self, span: JaSpan) -> Result<Vec<JaCandidate>, LiveError> {
        if !self.enabled() {
            return Ok(Vec::new());
        }
        if span.end() > self.seq.len() {
            return Err(LiveError::Span);
        }
        let reading: String = self.seq.iter().map(Unit::as_str).collect();
        Ok(self
            .conv
            .candidates(&reading, span, &self.convert_context())?)
    }

    fn reset_state(&mut self) {
        self.cut = 0;
        self.locked.clear();
        self.locked_seq.clear();
        self.tail.clear();
        self.draft = None;
        self.pending = false;
        self.due = None;
        self.held = 0;
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
        if seq.len() <= self.cut || self.locked_seq.len() < self.cut {
            return false;
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

    fn convert(&mut self) -> Plan {
        if self.cut > self.seq.len() {
            self.reset();
            return Plan::default();
        }
        let suffix = &self.seq[self.cut..];
        if suffix.is_empty() {
            return self.preview_plan();
        }
        let reading: String = suffix.iter().map(Unit::as_str).collect();
        let start = self.now();
        let converted = self.conv.convert(&reading, &self.convert_context());
        let elapsed = self.now().saturating_duration_since(start);
        self.stats.conversions += 1;
        self.stats.last_duration = elapsed;

        let timed_out = self.cfg.budget > Duration::ZERO && elapsed > self.cfg.budget;
        if timed_out {
            self.stats.timeouts += 1;
        }

        let result = match converted {
            Ok(result)
                if !result.clauses.is_empty() && clauses_units(&result.clauses) == suffix.len() =>
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

        let changes = boundary_changes(&self.tail, &result.clauses);
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
        self.advance_lock();
        let draft = JaResult {
            clauses: self.all_clauses(),
            cost: result.cost,
            score: result.score,
        };
        self.draft = Some(draft);
        Plan {
            convert: true,
            result: self.draft.clone(),
            boundary_changes: changes,
            ..Plan::default()
        }
    }

    /// 把尾端除了最後一個文節以外的部分鎖住。
    ///
    /// 最後一個文節仍可能隨下一個かな改變，所以留在 `tail`；已鎖住的文節
    /// 往後不會再被轉換。詞面仍是かな的文節（詞面與讀音相同）也留在
    /// `tail`：那通常只是長詞的片段，例如輸入「きょ」時只有未知語かな，
    /// 真正的詞要等「う」進來才成立。若鎖住這種前綴，跨界的詞就永遠
    /// 找不到。
    fn advance_lock(&mut self) {
        if self.tail.len() <= 1 {
            return;
        }
        let mut n = self.tail.len() - 1;
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

    /// 記錄呼叫內容的假轉換器；`split` 決定文節切法。
    struct FakeConverter {
        state: Rc<RefCell<FakeState>>,
        clock: Option<FakeClock>,
        cost: Rc<Cell<Duration>>,
        fail: Rc<Cell<bool>>,
        split: Split,
    }

    #[derive(Default)]
    struct FakeState {
        calls: usize,
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
                split: Box::new(split),
            }
        }

        fn with_clock(mut self, clock: &FakeClock) -> Self {
            self.clock = Some(clock.clone());
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
            clock: Some(clock.closure()),
            ..Config::default()
        }
    }

    #[test]
    fn test_disabled_by_default() {
        let clock = FakeClock::new();
        let conv = FakeConverter::new(split_every(2)).with_clock(&clock);
        let state = conv.state();
        let mut controller = Controller::new(conv, Config::default());
        assert!(!controller.enabled(), "Config::default 必須預設關閉");
        let plan = controller.input(&units("きょう"), &JaContext::default());
        assert!(!plan.convert && !plan.preview && !plan.pending, "{plan:?}");
        assert!(!controller.tick().1, "關閉時 tick 不得轉換");
        let cands = controller
            .candidates(JaSpan::new(0, 1))
            .expect("關閉時不得報錯");
        assert!(cands.is_empty(), "關閉時不得產生候選");
        assert_eq!(state.borrow().calls, 0, "關閉時呼叫了轉換器");
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
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].surface, "候補");
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
    fn test_zero_config_is_disabled() {
        let mut controller = Controller::new(FakeConverter::new(split_every(2)), Config::default());
        assert!(!controller.enabled(), "零值 Config 必須關閉");
        let plan = controller.input(&units("き"), &JaContext::default());
        assert!(!plan.convert && !plan.pending, "{plan:?}");
    }
}
