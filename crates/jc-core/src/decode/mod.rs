//! 轉換結果、解碼器與引擎錯誤。
//!
//! 這個 module 是 `core/decode` 的移植，接在 `z2-zh-mid` 的
//! [`crate::lattice`] 與 [`crate::score`] 之上：
//!
//! - 詞格、邊、詞典與成本模型用 `z2` 的 [`Lattice`]、[`Edge`]、
//!   [`Lexicon`]、[`ScoreContext`]，本 module 不重複定義。
//! - 成本沿用 `z2` 的 `i32` 語意，越小越優先；Go 版的機率與 bigram
//!   由詞性連接成本取代。
//! - 音節假設格（模糊音、無聲調）用 `z1-zh-front` 的
//!   [`crate::zh::input::Lattice`]，見 [`SyllableLattice`]。
//! - 鎖定、尾端前綴、N-best、候選與補全在邊圖上實作；不使用
//!   `personal`，個人化屬 M4。
//!
//! 因為鎖定與尾端前綴需要過濾邊，本 module 自己做前向 DP；沒有鎖定時
//! 的結果與 [`crate::lattice::Viterbi`] 相同（成本、邊數、穩定鍵的
//! tie-break 都對齊）。

use std::collections::{BTreeMap, HashMap};

use crate::candidate::CandidateSource;
use crate::lang::Lang;
use crate::lattice::{Edge, Lattice, LexEntry, Lexicon};
use crate::score::{ScoreContext, Scorer};
use crate::segment::{Script, Segment};

pub mod across;
pub mod bias;
pub mod explain;
pub mod nbest;
pub mod repeat;
pub mod untoned;

pub use nbest::{AuditTarget, Hypothesis, SearchStats};
pub use untoned::{
    cap_untoned_arcs, correction_reading, reading, untoned_pos, untoned_span, MAX_UNTONED_ARCS,
};

/// `z1` 的音節假設。
pub use crate::zh::input::Hyp;
/// `z1` 的音節假設格：每個位置是一組帶成本的讀音假設。
pub use crate::zh::input::Lattice as SyllableLattice;

/// 路徑起點（BOS）的狀態鍵，與 [`crate::lattice::Viterbi`] 相同。
pub const BOS: u16 = crate::score::BOS_POS;

/// 沒有語言上限時的詞條讀音長度搜尋上限。
pub const MAX_WORD_LEN: usize = crate::score::MAX_WORD_UNITS;

/// 尾端前綴計算預設取幾個最佳狀態。
pub const DEFAULT_TOP_B: usize = 4;

/// 有 bigram 時每個位置最多展開幾個前詞狀態。
///
/// 對應前身 `core/session.Options.Beam` 的預設值 8。沒有 bigram 時
/// 狀態數本來就小，解碼器不套用上限，結果與逐詞性保留完全相同。
pub const DEFAULT_BEAM: usize = 8;

/// 一次轉換的結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversion {
    /// 輸出字面。
    pub surface: String,
    /// 產生這個字面的片段。
    pub segments: Vec<Segment>,
    /// 整體路徑成本，越小越優先。
    pub cost: i32,
}

/// 核心對外回報的錯誤。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    /// 沒有可轉換的輸入。
    #[error("沒有可轉換的輸入")]
    EmptyInput,
    /// 讀音格式不合法。
    #[error("無效的讀音：{0}")]
    InvalidReading(String),
    /// 目前模式不支援這個操作。
    #[error("模式 {0:?} 不支援這個操作")]
    UnsupportedMode(Lang),
    /// 找不到必要的資料檔。
    #[error("找不到資料：{0}")]
    MissingData(String),
}

/// 解碼結果的一段。
///
/// 對應 Go 的 `decode.Seg`；`correction` 與 `matched` 在成本模型下
/// 還沒有來源，固定 `false` 與空字串。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seg {
    /// 這一段的字面。
    pub word: String,
    /// 起點音節索引（含）。
    pub start: usize,
    /// 終點音節索引（不含）。
    pub end: usize,
    /// 尾端：某個更長的詞的前綴。
    pub partial: bool,
    /// 經自動修正讀音到達。
    pub correction: bool,
    /// 詞庫讀音與打的音不同時，實際走的讀音。
    pub matched: String,
}

impl Seg {
    /// 建立一段完整詞。
    pub fn new(word: impl Into<String>, start: usize, end: usize) -> Self {
        Self {
            word: word.into(),
            start,
            end,
            partial: false,
            correction: false,
            matched: String::new(),
        }
    }

    /// 由詞格的邊建立片段。
    pub fn from_edge(edge: &Edge) -> Self {
        Self::new(edge.surface.clone(), edge.start, edge.end)
    }

    /// 建立尾端前綴片段。
    pub fn partial(word: impl Into<String>, start: usize, end: usize) -> Self {
        Self {
            partial: true,
            ..Self::new(word, start, end)
        }
    }

    /// 穩定鍵，供除重與決定性排序使用。
    pub fn key(&self) -> String {
        format!(
            "[{},{}]|{}|{}|{}",
            self.start, self.end, self.word, self.partial, self.matched
        )
    }
}

/// 使用者鎖定的區間：從某位置起 `len` 個音節必須是 `word`。硬條件。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lock {
    /// 明確的詞界：鎖定的詞不得與鄰居合併。
    pub segmentation: bool,
    /// 鎖定的字面。
    pub word: String,
    /// 佔幾個音節。
    pub len: usize,
    /// 鎖定來自 Tab 補全。
    pub completion: bool,
    /// 鎖定來自接受神經次佳提示。
    pub suggested: bool,
    /// 被字面文字切開的暫時邊界：-1 靠左、+1 靠右、0 一般。
    pub literal_boundary: i32,
    /// 跨音手選時詞庫的對應讀音。
    pub matched: String,
}

/// 鎖定表：起點音節索引 → 鎖定。
pub type Locks = HashMap<usize, Lock>;

/// 一條搜尋路徑：邊依輸入順序排列，附上累計成本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathState {
    pub(crate) last: u16,
    pub(crate) cost: i32,
    pub(crate) segs: Vec<Seg>,
    /// 路徑套用了非詞疊字的成本懲罰。
    pub(crate) capped: bool,
}

/// 空路徑的指紋種子；n-best 的共享節點路徑用。
pub(crate) const ROOT_FINGERPRINT: u64 = 0xcbf2_9ce4_8422_2325;

impl PathState {
    /// 路徑的穩定鍵。
    pub(crate) fn key(&self) -> String {
        self.segs
            .iter()
            .map(Seg::key)
            .collect::<Vec<_>>()
            .join("\x1f")
    }

    /// 前一個詞面；句首為 `None`。
    pub(crate) fn prev_word(&self) -> Option<&str> {
        self.segs.last().map(|seg| seg.word.as_str())
    }
}

/// 把一段片段接在既有指紋後面。
pub(crate) fn fingerprint_after(previous: u64, seg: &Seg) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    previous.hash(&mut hasher);
    seg.word.hash(&mut hasher);
    seg.start.hash(&mut hasher);
    seg.end.hash(&mut hasher);
    seg.partial.hash(&mut hasher);
    seg.matched.hash(&mut hasher);
    hasher.finish()
}

/// 路徑排序：成本、段數，最後才比完整穩定鍵。
///
/// 用在需要與舊行為逐位一致的地方（Viterbi、beam 剪枝、前綴延伸）；n-best
/// 的插入熱路徑改用共享節點的指紋，避免每次比較都重建路徑字串。
pub(crate) fn path_less_stable(a: &PathState, b: &PathState) -> std::cmp::Ordering {
    (a.cost, a.segs.len(), a.key()).cmp(&(b.cost, b.segs.len(), b.key()))
}

/// 尾端補全的最佳完整詞與信心。
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    /// 最佳完整詞。
    pub entry: LexEntry,
    /// 最佳完整詞的相對信心（0 到 1）。
    pub conf: f64,
}

impl Default for Completion {
    fn default() -> Self {
        Self {
            entry: LexEntry::new(String::new(), String::new(), 0, 0),
            conf: 0.0,
        }
    }
}

impl Completion {
    /// 是否有結果。
    pub fn found(&self) -> bool {
        !self.entry.surface.is_empty()
    }
}

/// 注音／假名解碼器。
///
/// 解碼器只借用詞典、成本設定與評分器，不持有資料來源。
pub struct Decoder<'a> {
    /// 詞典。
    pub lex: &'a dyn Lexicon,
    /// 建詞格與補全用的成本設定。
    pub ctx: &'a ScoreContext,
    /// 連接成本；預設就是 [`Decoder::ctx`]。
    pub scorer: &'a dyn Scorer,
    /// 尾端前綴取幾個最佳狀態。
    pub top_b: usize,
    /// 每個位置最多展開幾個前詞狀態；0 表示不限。
    ///
    /// 有 bigram 時狀態數會隨前詞增加，前身用 beam 把成本壓在延遲預算內。
    pub beam: usize,
    /// 候選查詢的詞面數上限；0 不限。
    pub max_candidates: usize,
}

impl<'a> Decoder<'a> {
    /// 用詞格的成本設定當評分器建立解碼器。
    pub fn new(lex: &'a dyn Lexicon, ctx: &'a ScoreContext) -> Self {
        Self {
            lex,
            ctx,
            scorer: ctx,
            top_b: 0,
            beam: 0,
            max_candidates: 0,
        }
    }

    /// 指定另一個連接成本評分器。
    pub fn with_scorer(mut self, scorer: &'a dyn Scorer) -> Self {
        self.scorer = scorer;
        self
    }

    fn top_b(&self) -> usize {
        if self.top_b == 0 {
            DEFAULT_TOP_B
        } else {
            self.top_b
        }
    }

    /// 讀音長度搜尋上限。
    pub fn max_len(&self) -> usize {
        if self.ctx.max_word_units > 0 {
            self.ctx.max_word_units
        } else {
            MAX_WORD_LEN.max(self.lex.longest_word_units())
        }
    }

    /// 解碼，回傳最佳路徑的片段。
    pub fn decode(&self, lat: &Lattice, locks: &Locks, prefix: bool) -> Vec<Seg> {
        self.best(lat, locks, prefix).0
    }

    /// 解碼，回傳最佳路徑與成本。
    pub fn best(&self, lat: &Lattice, locks: &Locks, prefix: bool) -> (Vec<Seg>, i32) {
        let mut states = self.forward(lat, locks);
        let mut winner = states
            .pop()
            .unwrap_or_default()
            .into_values()
            .min_by(path_less_stable);
        if prefix {
            for st in self.prefix_states(lat, locks, &states) {
                if winner
                    .as_ref()
                    .is_none_or(|w| path_less_stable(&st, w).is_lt())
                {
                    winner = Some(st);
                }
            }
        }
        match winner {
            Some(w) => (w.segs, w.cost),
            None => (Vec::new(), i32::MAX),
        }
    }

    /// 前向 DP：每個位置、每個（前一個詞面, 最後詞性）保留成本最低的路徑。
    ///
    /// 鍵要含前一個詞面，bigram 成本才不會被錯誤合併；沒有 bigram 時
    /// 同一詞性的不同前詞會各自保留，結果與只看詞性相同。
    fn forward(&self, lat: &Lattice, locks: &Locks) -> Vec<BTreeMap<(String, u16), PathState>> {
        let mut best: Vec<BTreeMap<(String, u16), PathState>> =
            vec![BTreeMap::new(); lat.len() + 1];
        best[0].insert(
            (String::new(), BOS),
            PathState {
                last: BOS,
                cost: 0,
                segs: Vec::new(),
                capped: false,
            },
        );
        for pos in 0..lat.len() {
            let states = best[pos].clone();
            if states.is_empty() {
                continue;
            }
            let frontier = self.frontier(&states);
            for edge in self.edges_at(lat, pos, locks) {
                for st in &frontier {
                    let step = self.step_cost(st, edge);
                    let mut next = PathState {
                        last: edge.pos,
                        cost: st.cost.saturating_add(step),
                        segs: st.segs.clone(),
                        capped: st.capped,
                    };
                    next.segs.push(Seg::from_edge(edge));
                    put_best(&mut best[edge.end], next);
                }
            }
        }
        best
    }

    /// 這個位置要繼續展開的狀態：成本最低的前 `beam` 個；`beam` 為 0 時全部。
    fn frontier<'s>(&self, states: &'s BTreeMap<(String, u16), PathState>) -> Vec<&'s PathState> {
        let mut all: Vec<&PathState> = states.values().collect();
        if self.beam == 0 || all.len() <= self.beam {
            return all;
        }
        all.sort_by(|a, b| path_less_stable(a, b));
        all.truncate(self.beam);
        all
    }

    /// 尾端前綴延伸：只保留每個顯示字串成本最低者。
    fn prefix_states(
        &self,
        lat: &Lattice,
        locks: &Locks,
        best: &[BTreeMap<(String, u16), PathState>],
    ) -> Vec<PathState> {
        let n = lat.len();
        let top_b = self.top_b();
        let mut out: Vec<PathState> = Vec::new();
        for (i, slot) in best.iter().enumerate().take(n) {
            if locked_in(locks, i, n) || slot.is_empty() {
                continue;
            }
            let k = n - i;
            let mut states: Vec<PathState> = slot.values().cloned().collect();
            states.sort_by(path_less_stable);
            states.truncate(top_b);
            let mut group: BTreeMap<String, PathState> = BTreeMap::new();
            for entry in self.tail_entries(lat, i, k) {
                let display = first_chars(&entry.surface, k);
                for st in &states {
                    let cost = st.cost.saturating_add(self.scorer.arc_cost(
                        st.prev_word(),
                        st.last,
                        self.ctx.word_cost(entry.cost, entry.units()),
                        entry.pos,
                        &entry.surface,
                        CandidateSource::Lexicon,
                    ));
                    let mut segs = st.segs.clone();
                    segs.push(Seg::partial(display.clone(), i, n));
                    let cand = PathState {
                        last: entry.pos,
                        cost,
                        segs,
                        capped: false,
                    };
                    match group.get(&display) {
                        Some(old) if !path_less_stable(&cand, old).is_lt() => {}
                        _ => {
                            group.insert(display.clone(), cand);
                        }
                    }
                }
            }
            out.extend(group.into_values());
        }
        out
    }

    /// 一條邊接在 `prev` 後面的成本。
    fn step_cost(&self, prev: &PathState, edge: &Edge) -> i32 {
        self.scorer.edge_cost(prev.prev_word(), prev.last, edge)
    }

    fn step_cost_raw(&self, prev: u16, next_pos: u16) -> i32 {
        if prev == BOS {
            0
        } else {
            self.scorer.connection_cost(prev, next_pos)
        }
    }

    /// 合法且通過鎖定的邊。
    pub(crate) fn edges_at<'l>(
        &'l self,
        lat: &'l Lattice,
        pos: usize,
        locks: &'l Locks,
    ) -> impl Iterator<Item = &'l Edge> + 'l {
        lat.edges_from(pos)
            .iter()
            .filter(move |e| allowed(locks, e.start, e.end, &e.surface))
    }

    /// 尾端詞條：讀音以 `lat.readings()[i..]` 開頭、比已輸入部分更長的詞。
    pub(crate) fn tail_entries(&self, lat: &Lattice, i: usize, k: usize) -> Vec<LexEntry> {
        let reading = readings_key(&lat.readings()[i..]);
        self.lex
            .prefix(&reading)
            .into_iter()
            .filter(|e| e.units() > k)
            .cloned()
            .collect()
    }

    /// 候選：`[start, start+length)` 的邊，依成本排序。
    pub fn candidates(
        &self,
        lat: &Lattice,
        start: usize,
        length: usize,
        pinned: Option<&dyn Fn(&str) -> bool>,
    ) -> Vec<Edge> {
        let mut out: Vec<Edge> = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();
        if length == 0 {
            return out;
        }
        for edge in lat.edges_from(start) {
            if edge.end != start + length {
                continue;
            }
            match seen.get(&edge.surface) {
                Some(&i) => {
                    if edge.cost < out[i].cost {
                        out[i] = edge.clone();
                    }
                }
                None => {
                    seen.insert(edge.surface.clone(), out.len());
                    out.push(edge.clone());
                }
            }
        }
        out.sort_by(|a, b| {
            (a.cost, block(&a.surface), &a.surface).cmp(&(b.cost, block(&b.surface), &b.surface))
        });
        if let Some(pinned) = pinned {
            if out.len() > 2 {
                let rest: Vec<Edge> = out[2..].to_vec();
                for c in rest {
                    if !pinned(&c.surface) {
                        continue;
                    }
                    if let Some(i) = out.iter().position(|e| e.surface == c.surface) {
                        let c = out.remove(i);
                        out.insert(1, c);
                    }
                }
            }
        }
        if self.max_candidates > 0 {
            out.truncate(self.max_candidates);
        }
        out
    }

    /// 候選：加上鎖定後左右已輸入音節的最佳完整路徑。
    pub fn candidates_in_context(
        &self,
        lat: &Lattice,
        start: usize,
        length: usize,
        locks: &Locks,
        pinned: Option<&dyn Fn(&str) -> bool>,
    ) -> Vec<Edge> {
        let end = start + length;
        if length == 0 || end > lat.len() {
            return Vec::new();
        }
        let mut kept = Locks::new();
        for (at, lock) in locks {
            if *at >= end || at + lock.len <= start {
                kept.insert(*at, lock.clone());
            }
        }
        let left = self.forward_until(lat, start, &kept);
        let suffix = self.suffix_table(lat, end, &kept);
        let mut out: Vec<Edge> = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut scored: Vec<(Edge, i32)> = Vec::new();
        for edge in lat.edges_from(start) {
            if edge.end != end || !allowed(&kept, edge.start, edge.end, &edge.surface) {
                continue;
            }
            let mut best = i32::MAX;
            for (last, cost) in &left {
                let s = cost
                    .saturating_add(self.step_cost_raw(*last, edge.pos))
                    .saturating_add(edge.cost)
                    .saturating_add(suffix_cost(&suffix, end, edge.pos));
                best = best.min(s);
            }
            scored.push((edge.clone(), best));
        }
        scored.sort_by(|a, b| {
            (a.1, block(&a.0.surface), &a.0.surface).cmp(&(b.1, block(&b.0.surface), &b.0.surface))
        });
        for (edge, cost) in scored {
            match seen.get(&edge.surface) {
                Some(&i) => {
                    if cost < out[i].cost {
                        out[i] = edge.clone();
                        out[i].cost = cost;
                    }
                }
                None => {
                    seen.insert(edge.surface.clone(), out.len());
                    let mut e = edge;
                    e.cost = cost;
                    out.push(e);
                }
            }
        }
        if let Some(pinned) = pinned {
            if out.len() > 2 {
                let rest: Vec<Edge> = out[2..].to_vec();
                for c in rest {
                    if !pinned(&c.surface) {
                        continue;
                    }
                    if let Some(i) = out.iter().position(|e| e.surface == c.surface) {
                        let c = out.remove(i);
                        out.insert(1, c);
                    }
                }
            }
        }
        out
    }

    /// 走到 `end` 為止的左側最佳成本表：最後詞性 → 成本。
    fn forward_until(&self, lat: &Lattice, end: usize, locks: &Locks) -> BTreeMap<u16, i32> {
        let mut best: Vec<BTreeMap<u16, i32>> = vec![BTreeMap::new(); end + 1];
        best[0].insert(BOS, 0);
        for pos in 0..end {
            let states = std::mem::take(&mut best[pos]);
            if states.is_empty() {
                continue;
            }
            for edge in lat.edges_from(pos) {
                if edge.end > end || !allowed(locks, edge.start, edge.end, &edge.surface) {
                    continue;
                }
                for (last, cost) in &states {
                    let next = cost
                        .saturating_add(self.step_cost_raw(*last, edge.pos))
                        .saturating_add(edge.cost);
                    let slot = best[edge.end].entry(edge.pos).or_insert(i32::MAX);
                    *slot = (*slot).min(next);
                }
            }
        }
        best[end].clone()
    }

    /// 從 `at` 到結尾的最佳成本表：前一詞性 → 成本。
    fn suffix_table(&self, lat: &Lattice, at: usize, locks: &Locks) -> Vec<BTreeMap<u16, i32>> {
        let n = lat.len();
        let mut all_pos: Vec<u16> = vec![BOS];
        for pos in 0..n {
            for e in lat.edges_from(pos) {
                if allowed(locks, e.start, e.end, &e.surface) && !all_pos.contains(&e.pos) {
                    all_pos.push(e.pos);
                }
            }
        }
        let mut table: Vec<BTreeMap<u16, i32>> = vec![BTreeMap::new(); n + 1];
        for pos in (at..=n).rev() {
            let mut row: BTreeMap<u16, i32> = BTreeMap::new();
            if pos == n {
                for prev in &all_pos {
                    row.insert(*prev, 0);
                }
                table[pos] = row;
                continue;
            }
            let edges: Vec<Edge> = lat
                .edges_from(pos)
                .iter()
                .filter(|e| allowed(locks, e.start, e.end, &e.surface))
                .cloned()
                .collect();
            for prev in &all_pos {
                let mut best = i32::MAX;
                for e in &edges {
                    let tail = table[e.end].get(&e.pos).copied().unwrap_or(i32::MAX);
                    if tail == i32::MAX {
                        continue;
                    }
                    let v = self
                        .step_cost_raw(*prev, e.pos)
                        .saturating_add(e.cost)
                        .saturating_add(tail);
                    best = best.min(v);
                }
                if best != i32::MAX {
                    row.insert(*prev, best);
                }
            }
            table[pos] = row;
        }
        table
    }

    /// 尾端補全：`path` 最後一段之後還有更長的詞。
    pub fn complete(&self, lat: &Lattice, path: &[Seg]) -> (Completion, bool) {
        let (c, _, ok) = self.complete_mass(lat, path);
        (c, ok)
    }

    /// 與 [`Decoder::complete`] 相同，另回傳尾端詞群的權重總和。
    pub(crate) fn complete_mass(&self, lat: &Lattice, path: &[Seg]) -> (Completion, f64, bool) {
        let Some(last) = path.last() else {
            return (Completion::default(), 0.0, false);
        };
        let k = last.end.saturating_sub(last.start);
        let entries = self.tail_entries(lat, last.start, k);
        let weights: Vec<f64> = entries.iter().map(|e| self.entry_weight(e, k)).collect();
        let total: f64 = weights.iter().sum();
        if total <= 0.0 {
            return (Completion::default(), 0.0, false);
        }
        let mut best = Completion::default();
        let mut best_weight = 0.0;
        for (e, w) in entries.iter().zip(weights.iter()) {
            if *w > best_weight {
                best_weight = *w;
                best = Completion {
                    entry: e.clone(),
                    conf: w / total,
                };
            }
        }
        (best, total, true)
    }

    /// 比較同一注音前綴的完整延伸詞。
    pub fn complete_homophones(
        &self,
        lat: &Lattice,
        start: usize,
        _left: &str,
    ) -> (Completion, bool) {
        let typed = lat.len().saturating_sub(start);
        if typed < 2 || typed >= self.max_len() {
            return (Completion::default(), false);
        }
        let reading = readings_key(&lat.readings()[start..]);
        let entries: Vec<LexEntry> = self
            .lex
            .prefix(&reading)
            .into_iter()
            .filter(|e| e.units() > typed)
            .cloned()
            .collect();
        let weights: Vec<f64> = entries
            .iter()
            .map(|e| self.entry_weight(e, typed))
            .collect();
        let total: f64 = weights.iter().sum();
        if total <= 0.0 {
            return (Completion::default(), false);
        }
        let mut best = Completion::default();
        let mut best_weight = 0.0;
        for (e, w) in entries.iter().zip(weights.iter()) {
            if *w > best_weight {
                best_weight = *w;
                best = Completion {
                    entry: e.clone(),
                    conf: w / total,
                };
            }
        }
        let found = best.found();
        (best, found)
    }

    /// 詞條相對於其他同前綴詞條的權重。
    fn entry_weight(&self, entry: &LexEntry, _typed: usize) -> f64 {
        // 成本以千分之一為單位（毫成本），換回 nats 再取指數，避免下溢。
        let cost = f64::from(self.ctx.word_cost(entry.cost, entry.units()));
        (-cost / 1000.0).exp()
    }

    /// 把片段路徑轉成對外 [`Conversion`]。
    ///
    /// 位元組位移以詞格讀音依序相接計算。
    pub fn conversion(&self, lat: &Lattice, segs: &[Seg], mode: Lang) -> Conversion {
        let mut offsets = Vec::with_capacity(lat.len() + 1);
        let mut cursor = 0usize;
        offsets.push(0);
        for reading in lat.readings() {
            cursor += reading.len();
            offsets.push(cursor);
        }
        let script = match mode {
            Lang::Zh => Script::Bopomofo,
            Lang::Ja => Script::Kana,
            _ => Script::Other,
        };
        let mut surface = String::new();
        let mut segments = Vec::with_capacity(segs.len());
        for seg in segs {
            surface.push_str(&seg.word);
            let start = *offsets.get(seg.start).unwrap_or(&0);
            let end = *offsets.get(seg.end).unwrap_or(&start);
            segments.push(Segment::new(start, end, mode, script));
        }
        Conversion {
            surface,
            segments,
            cost: 0,
        }
    }
}

/// 詞格的讀音串成詞典鍵。
pub(crate) fn readings_key(readings: &[String]) -> String {
    readings.join(" ")
}

/// 前 `n` 個字元。
pub(crate) fn first_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// 把路徑插進「每個（前詞, 最後詞性）只留最佳」的表。
pub(crate) fn put_best(slot: &mut BTreeMap<(String, u16), PathState>, cand: PathState) {
    let key = (cand.prev_word().unwrap_or_default().to_string(), cand.last);
    match slot.get(&key) {
        Some(old) if !path_less_stable(&cand, old).is_lt() => {}
        _ => {
            slot.insert(key, cand);
        }
    }
}

fn suffix_cost(table: &[BTreeMap<u16, i32>], at: usize, pos: u16) -> i32 {
    table
        .get(at)
        .and_then(|row| row.get(&pos))
        .copied()
        .unwrap_or(i32::MAX)
}

fn locked_in(locks: &Locks, i: usize, n: usize) -> bool {
    locks.keys().any(|s| i <= *s && *s < n)
}

/// 詞 `w` 佔 `[i, j)` 是否符合所有鎖定。
pub(crate) fn allowed(locks: &Locks, i: usize, j: usize, w: &str) -> bool {
    let mut runes: Option<Vec<char>> = None;
    for (s, l) in locks {
        let s = *s;
        if s < i || s >= j || (s == i && l.word == w && l.len == j - i) {
            continue;
        }
        if !l.segmentation && l.literal_boundary == 0 && s + l.len <= j {
            let rs = runes.get_or_insert_with(|| w.chars().collect());
            if rs.len() == j - i
                && l.word.chars().count() == l.len
                && rs[s - i..s - i + l.len].iter().collect::<String>() == l.word
            {
                continue;
            }
        }
        return false;
    }
    true
}

/// 同分候選的排序分區：常用漢字、其他基本平面、擴充 A、其餘。
fn block(s: &str) -> u8 {
    match s.chars().next() {
        Some(r) if ('\u{4E00}'..='\u{9FFF}').contains(&r) => 0,
        Some(r) if ('\u{3400}'..='\u{4DBF}').contains(&r) => 2,
        Some(r) if (r as u32) <= 0xFFFF => 1,
        Some(_) => 3,
        None => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::CandidateSource;
    use crate::lattice::InMemoryLexicon;

    fn entry(surface: &str, reading: &str, cost: i32) -> LexEntry {
        LexEntry::new(surface, reading, cost, 1)
    }

    fn plain_ctx() -> ScoreContext {
        ScoreContext {
            length_bonus: 0,
            unknown_cost: 100,
            connection: crate::score::ConnectionMatrix::with_fallback(0),
            ..Default::default()
        }
    }

    fn fixture() -> (InMemoryLexicon, ScoreContext) {
        let mut lex = InMemoryLexicon::new();
        for (w, r, c) in [
            ("因", "ㄧㄣ", 10),
            ("應", "ㄧㄥ", 5),
            ("該", "ㄍㄞ", 6),
            ("應該", "ㄧㄥ ㄍㄞ", 4),
            ("因為", "ㄧㄣ ㄨㄟˋ", 7),
            ("為", "ㄨㄟˋ", 8),
        ] {
            lex.push(entry(w, r, c));
        }
        (lex, plain_ctx())
    }

    fn text(path: &[Seg]) -> String {
        path.iter().map(|s| s.word.as_str()).collect()
    }

    fn build(readings: &[&str], lex: &InMemoryLexicon, ctx: &ScoreContext) -> Lattice {
        Lattice::build(readings, lex, ctx).expect("建立詞格")
    }

    #[test]
    fn conversion_keeps_surface_segments_and_cost() {
        let c = Conversion {
            surface: "わたし".to_string(),
            segments: vec![Segment::new(0, 9, Lang::Ja, Script::Kana)],
            cost: 42,
        };
        assert_eq!(c.surface, "わたし");
        assert_eq!(c.segments.len(), 1);
        assert_eq!(c.cost, 42);
    }

    #[test]
    fn engine_error_messages_include_details() {
        let e = EngineError::UnsupportedMode(Lang::En);
        assert_eq!(e.to_string(), "模式 En 不支援這個操作");

        let e = EngineError::InvalidReading("ㄅ".to_string());
        assert_eq!(e.to_string(), "無效的讀音：ㄅ");

        let e = EngineError::EmptyInput;
        assert_eq!(e.to_string(), "沒有可轉換的輸入");

        let e = EngineError::MissingData("lexicon.bin".to_string());
        assert_eq!(e.to_string(), "找不到資料：lexicon.bin");
    }

    #[test]
    fn decoder_prefers_lower_cost_long_word() {
        let (lex, ctx) = fixture();
        let d = Decoder::new(&lex, &ctx);
        let lat = build(&["ㄧㄥ", "ㄍㄞ"], &lex, &ctx);
        assert_eq!(text(&d.decode(&lat, &Locks::new(), false)), "應該");
    }

    #[test]
    fn locks_are_hard_constraints() {
        let (lex, ctx) = fixture();
        let d = Decoder::new(&lex, &ctx);
        let lat = build(&["ㄧㄥ", "ㄍㄞ"], &lex, &ctx);
        assert_eq!(text(&d.decode(&lat, &Locks::new(), false)), "應該");

        let mut same_char = Locks::new();
        same_char.insert(
            0,
            Lock {
                word: "應".to_string(),
                len: 1,
                ..Default::default()
            },
        );
        assert_eq!(text(&d.decode(&lat, &same_char, false)), "應該");

        let mut other = Locks::new();
        other.insert(
            0,
            Lock {
                word: "鷹".to_string(),
                len: 1,
                ..Default::default()
            },
        );
        // 詞庫沒有「鷹」時鎖定無法成立，所有邊都被擋掉。
        let path = d.decode(&lat, &other, false);
        assert!(path.is_empty());
    }

    #[test]
    fn lock_with_existing_word_blocks_longer_entry() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("應", "ㄧㄥ", 5));
        lex.push(entry("鷹", "ㄧㄥ", 1));
        lex.push(entry("該", "ㄍㄞ", 6));
        lex.push(entry("應該", "ㄧㄥ ㄍㄞ", 4));
        let ctx = plain_ctx();
        let d = Decoder::new(&lex, &ctx);
        let lat = build(&["ㄧㄥ", "ㄍㄞ"], &lex, &ctx);

        let mut locks = Locks::new();
        locks.insert(
            0,
            Lock {
                word: "鷹".to_string(),
                len: 1,
                ..Default::default()
            },
        );
        let path = d.decode(&lat, &locks, false);
        assert_eq!(text(&path), "鷹該");
        assert_eq!(path.len(), 2);
    }

    #[test]
    fn empty_lattice_returns_nothing() {
        let (lex, ctx) = fixture();
        let d = Decoder::new(&lex, &ctx);
        let empty = build(&["ㄅ"], &InMemoryLexicon::new(), &ctx);
        // 未知詞邊仍可解碼；真正的空輸入在建立詞格時就被拒絕。
        assert_eq!(d.decode(&empty, &Locks::new(), true).len(), 1);
        assert_eq!(
            Lattice::build(&[] as &[&str], &lex, &ctx),
            Err(crate::lattice::LatticeError::EmptyInput)
        );
    }

    #[test]
    fn prefix_extension_offers_a_longer_word() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("沒", "ㄇㄟˊ", 5));
        lex.push(entry("沒有", "ㄇㄟˊ ㄧㄡˇ", 2));
        let ctx = plain_ctx();
        let d = Decoder::new(&lex, &ctx);
        let lat = build(&["ㄇㄟˊ"], &lex, &ctx);
        let path = d.decode(&lat, &Locks::new(), true);
        assert_eq!(text(&path), "沒");
        assert!(path[0].partial);
    }

    #[test]
    fn candidates_are_sorted_by_cost_and_deduped() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("甲", "ㄧ", 30));
        lex.push(entry("乙", "ㄧ", 10));
        lex.push(entry("丙", "ㄧ", 20));
        let ctx = plain_ctx();
        let d = Decoder::new(&lex, &ctx);
        let lat = build(&["ㄧ"], &lex, &ctx);
        let got = d.candidates(&lat, 0, 1, None);
        let words: Vec<&str> = got.iter().take(3).map(|e| e.surface.as_str()).collect();
        assert_eq!(words, vec!["乙", "丙", "甲"]);
    }

    #[test]
    fn conversion_maps_syllable_indices_to_byte_offsets() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("應該", "ㄧㄥ ㄍㄞ", 1));
        let ctx = plain_ctx();
        let d = Decoder::new(&lex, &ctx);
        let lat = build(&["ㄧㄥ", "ㄍㄞ"], &lex, &ctx);
        let segs = vec![Seg::new("應該", 0, 2)];
        let c = d.conversion(&lat, &segs, Lang::Zh);
        assert_eq!(c.surface, "應該");
        assert_eq!(c.segments.len(), 1);
        assert_eq!(c.segments[0].start, 0);
        assert_eq!(c.segments[0].end, 12);
        assert_eq!(c.segments[0].script, Script::Bopomofo);
    }

    #[test]
    fn completion_prefers_longer_word_prefix() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("沒有", "ㄇㄟˊ ㄧㄡˇ", 5));
        lex.push(entry("沒有不同", "ㄇㄟˊ ㄧㄡˇ ㄅㄨˋ ㄊㄨㄥˊ", 9));
        let ctx = plain_ctx();
        let d = Decoder::new(&lex, &ctx);
        let lat = build(&["ㄇㄟˊ", "ㄧㄡˇ"], &lex, &ctx);
        let path = vec![Seg::new("沒有", 0, 2)];
        let (c, ok) = d.complete(&lat, &path);
        assert!(ok);
        assert_eq!(c.entry.surface, "沒有不同");
    }

    #[test]
    fn test_limits_control_word_length() {
        let reading = "ㄚ ㄇㄨˇ ㄙ ㄊㄜˋ ㄌㄤˇ ㄏㄨㄟˊ ㄒㄩㄢˊ ㄐㄧㄚ ㄙㄨˋ ㄆㄣ";
        let word = "阿姆斯特朗回旋加速噴";
        let mut lex = InMemoryLexicon::new();
        lex.push(entry(word, reading, 1));
        let mut ctx = plain_ctx();
        ctx.max_word_units = 12;
        let d = Decoder::new(&lex, &ctx);
        assert_eq!(d.max_len(), 12);
        let syls: Vec<&str> = reading.split_whitespace().collect();
        let lat = build(&syls, &lex, &ctx);
        assert_eq!(text(&d.decode(&lat, &Locks::new(), false)), word);

        ctx.max_word_units = 0;
        let d = Decoder::new(&lex, &ctx);
        assert_eq!(d.max_len(), 10, "沒有語言上限時沿用詞庫長度");
    }

    #[test]
    fn source_is_carried_by_built_edges() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("台", "ㄊㄞˊ", 5));
        let ctx = plain_ctx();
        let lat = build(&["ㄊㄞˊ"], &lex, &ctx);
        let got = d_edges(&lat);
        assert!(got
            .iter()
            .any(|e| e.surface == "台" && e.source == CandidateSource::Lexicon));
    }

    fn d_edges(lat: &Lattice) -> Vec<Edge> {
        lat.edges_from(0).to_vec()
    }
}
