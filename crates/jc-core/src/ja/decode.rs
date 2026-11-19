//! 日文かな漢字変換的統計核心：かな lattice 上的完整 Viterbi。
//!
//! 流程與前身 `core/ja/decode` 相同：依讀音建立かな lattice，用 Viterbi
//! 選出最低成本的詞序列，再依詞性與連接成本合併成文節（[`JaClause`]）。
//! 對外輸出 [`Conversion`]，`segments` 是文節在 `surface` 上的位元組範圍，
//! 供 `jc-eval` 計算 `boundary_exact`。
//!
//! 成本尺度沿用 `j2-ja-lexicon`：詞成本與連接成本都是 `i32`（Mozc 原始整數，
//! 越小越優先），DP 內部除以 1000 對齊前身的浮點尺度；語言模型成本是
//! `f64`，預設權重 0，因此沒有 `lm.bin` 時完全等於詞成本＋連接成本。
//!
//! context ID 的處理與前身一致：狀態鍵保留前一個詞的 right context ID，
//! 詞弧保留 left context ID；只有兩邊都有 ID 且設定了 [`JaContextCost`] 時
//! 才走 context 矩陣，否則退回詞性連接成本。pack v1 沒有 context ID，所以
//! 正式詞典一律走詞性表；介面先留給帶 ID 的資料來源。
//!
//! 這個 module 不碰檔案系統與 session；詞典由呼叫端開啟後注入。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use unicode_script::{Script as UnicodeScript, UnicodeScript as _};

use crate::decode::Conversion;
use crate::ja::lexicon::{
    unknown_entry, ConnectionCost, JaConnectionCost, JaEntry, JaError, JaLexicon, JaPosClass,
    SingleColumnCost, DEFAULT_CONNECTION_COST, UNKNOWN_WORD_COST,
};
use crate::ja::lm::LanguageModel;
use crate::ja::rewrite::particle_reading_alternative;
use crate::lang::Mode;
use crate::segment::{Script, Segment};

/// 單一詞條可佔用的讀音單位數上限，對齊前身 `lang.DefaultLimits().MaxWordUnits`。
pub const DEFAULT_MAX_WORD_UNITS: usize = 32;

/// `Convert` 保留給驗證器重排的候選數，對齊前身 `Decoder.MaxNBest` 的預設值。
pub const DEFAULT_MAX_NBEST: usize = 8;

/// 一次列出的候選數上限，對齊前身 `lang.DefaultLimits().MaxCandidates`。
pub const DEFAULT_MAX_CANDIDATES: usize = 16;

/// 未知詞 fallback 的單一かな成本，沿用 [`UNKNOWN_WORD_COST`]。
pub const DEFAULT_UNKNOWN_WORD_COST: i32 = UNKNOWN_WORD_COST;

/// 詞典成本與前身浮點尺度之間的換算；pack 的 `i32` 是 Mozc 原始整數。
const COST_SCALE: f64 = 1_000.0;

/// 片假名詞面的容忍度（詞成本差距 500），對齊前身 `katakanaMargin = 0.5`。
pub const KATAKANA_MARGIN: i32 = 500;

/// 詞層級重排一次送進模型的候選上限，對齊模型的批次上限。
pub const WORD_RERANK_MAX_CANDIDATES: usize = 8;

/// 參與重排的詞面字數上限，對齊訓練資料的 `max_candidate_chars`。
pub const WORD_RERANK_MAX_CHARS: usize = 4;

/// 參與重排的最短讀音單位數；單かな的助詞不是訓練分佈。
pub const WORD_RERANK_MIN_READING_UNITS: usize = 2;

/// 候選成本的容忍視窗，對齊訓練資料的 `homophone_cost_window`。
///
/// 只把與最便宜候選相差不超過這個值的詞面送進模型；再貴的詞面統計已經
/// 排除，硬送進去只會讓模型把雜訊放大。
pub const WORD_RERANK_COST_WINDOW: i32 = 2_500;

/// 神經分數的融合權重，對齊前身 `Weights.Neural`。
pub const WORD_NEURAL_WEIGHT: f64 = 1.0;

/// 統計分數的融合權重，對齊前身 `Weights.Stats`。
pub const WORD_STATS_WEIGHT: f64 = 0.2;

/// 統計第一名的固定加分，對齊前身 `Weights.Draft`。
pub const WORD_DRAFT_BONUS: f64 = 0.1;

/// 一次轉換最多重排幾個詞群，控制延遲。
pub const WORD_RERANK_MAX_GROUPS: usize = 3;

/// 詞群要不要送模型的門檻。
///
/// 統計已經很確定的詞群不送模型：全量重排會把統計正確的簡單題改壞。
/// 門檻只看統計成本與候選結構，不看答案。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordGate {
    /// 不設門檻，所有合格詞群都送。
    Off,
    /// 前兩名成本差小於 `gap`（詞成本＋連接成本，Mozc 原始整數）才送。
    Gap(i32),
    /// 候選數至少 `candidates` 個才送。
    Candidates(usize),
}

impl WordGate {
    /// 這個詞群是否通過門檻。
    pub fn allows(&self, gap: i32, candidates: usize) -> bool {
        match *self {
            Self::Off => true,
            Self::Gap(limit) => gap < limit,
            Self::Candidates(min) => candidates >= min,
        }
    }
}

/// `--gate` 未指定時的建議門檻，由 `modern-500` 的掃描結果決定。
pub const DEFAULT_WORD_RERANK_GATE: WordGate = WordGate::Gap(100);

/// 融合分數差距換成詞成本的倍率。
///
/// 1.0 表示「神經分數差 1 分」等於「詞成本差 1.0」（DP 尺度，即詞典的
/// 1000 單位）。統計成本保留在原處，重排只加上這個增量。
pub const WORD_SCORE_SCALE: f64 = 1.0;

/// 詞性 `other` 的片假名容忍度（2000），對齊前身 `katakanaUnknownPOSMargin = 2.0`。
pub const KATAKANA_UNKNOWN_POS_MARGIN: i32 = 2_000;

/// 轉換時可用的上下文。
///
/// 對齊前身 `lang.Context`：`left` 是已確定的左側文字，`right` 是已存在的
/// 右側文字。學習與驗證鉤子可以讀取，解碼器不會修改。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JaContext {
    /// 左側文字。
    pub left: String,
    /// 右側文字。
    pub right: String,
}

/// 要列候選的讀音範圍，以かな單位編號，範圍是 `[start, start + length)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JaSpan {
    /// 起點（含）。
    pub start: usize,
    /// 長度。
    pub length: usize,
}

impl JaSpan {
    /// 建立範圍。
    pub fn new(start: usize, length: usize) -> Self {
        Self { start, length }
    }

    /// 結束位置（不含）。
    pub fn end(self) -> usize {
        self.start.saturating_add(self.length)
    }
}

/// 文節內部的一段詞，保留讀音與詞面的對應。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JaClauseSegment {
    /// 這一小段的讀音。
    pub reading: String,
    /// 這一小段的詞面。
    pub surface: String,
}

/// 一個文節。
///
/// `cost` 只含詞成本與詞性連接成本（與前身 `Clause.Score` 相同，不含 LM），
/// 用來讓外殼比較同句不同切分的統計品質。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JaClause {
    /// 文節讀音。
    pub reading: String,
    /// 文節詞面。
    pub surface: String,
    /// 文節內的詞。
    pub segments: Vec<JaClauseSegment>,
    /// 文節成本，越小越優先。
    pub cost: i32,
}

impl JaClause {
    /// 文節分數，等於 `-cost`，越大越優先（對齊前身 `Clause.Score`）。
    pub fn score(&self) -> f64 {
        -f64::from(self.cost)
    }
}

/// 一條完整轉換路徑。
#[derive(Debug, Clone, PartialEq)]
pub struct JaResult {
    /// 文節序列。
    pub clauses: Vec<JaClause>,
    /// 搜尋成本（前身的浮點尺度），越小越優先。
    pub cost: f64,
    /// 排序分數，等於 `-cost` 加上驗證器加分。
    pub score: f64,
}

impl JaResult {
    /// 串接所有文節詞面。
    pub fn surface(&self) -> String {
        self.clauses.iter().map(|c| c.surface.as_str()).collect()
    }

    /// 轉成 `jc-core` 的 [`Conversion`]。
    ///
    /// `segments` 是每個文節在 `surface` 上的 UTF-8 位元組範圍，連續且覆蓋
    /// 整個字串；`cost` 換算回詞典的原始整數尺度。
    pub fn conversion(&self) -> Conversion {
        let mut surface = String::new();
        let mut segments = Vec::with_capacity(self.clauses.len());
        for clause in &self.clauses {
            let start = surface.len();
            surface.push_str(&clause.surface);
            let end = surface.len();
            segments.push(Segment::new(start, end, Mode::Ja, Script::Kana));
        }
        Conversion {
            surface,
            segments,
            cost: raw_cost(self.cost),
        }
    }
}

/// 日文候選的來源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JaCandidateSource {
    /// 統計轉換（Viterbi 路徑或同讀音詞條）。
    Convert,
    /// 讀音前綴預測。
    Predict,
}

/// 候選視窗的一項。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JaCandidate {
    /// 輸出詞面。
    pub surface: String,
    /// 對應讀音。
    pub reading: String,
    /// 路徑成本，越小越優先。
    pub cost: i32,
    /// 來源。
    pub source: JaCandidateSource,
}

/// 依讀音查詞條的來源。
///
/// 正式實作是 [`JaDictionary`]；測試或小型情境可以直接提供自己的實作。
/// `matches` 必須走訪 `reading[start..]` 的所有前綴，讓可変長讀音被展開。
pub trait JaWordSource: Send + Sync {
    /// 最長詞條的讀音單位數。
    fn longest_reading_units(&self) -> usize;

    /// 依序回報從 `start` 開始的每個詞條讀音與結束位置。
    fn matches<'a>(&'a self, reading: &[char], start: usize, out: &mut dyn FnMut(JaWordMatch<'a>));

    /// 回傳讀音完全相等的詞條。
    fn exact(&self, reading: &[char]) -> Vec<&JaEntry>;

    /// 回傳讀音以 `reading` 開頭的詞條，含完全相等者。
    fn prefixed(&self, reading: &[char]) -> Vec<&JaEntry>;
}

/// 一次前綴查詢的結果。
#[derive(Debug, Clone, Copy)]
pub struct JaWordMatch<'a> {
    /// 讀音結束位置（不含）。
    pub end: usize,
    /// 詞條。
    pub entry: &'a JaEntry,
    /// 來源的 left context ID；沒有就是 `None`。
    pub left_id: Option<u16>,
    /// 來源的 right context ID；沒有就是 `None`。
    pub right_id: Option<u16>,
}

/// 以 [`JaLexicon`] 為底、依讀音索引的詞典。
///
/// pack 只提供依詞面排序的索引；かな lattice 需要依讀音查詢，因此在這裡
/// 建一份讀音到詞條編號的對照表。建表只做一次，查詢是雜湊表查找。
pub struct JaDictionary {
    lex: Arc<JaLexicon>,
    index: HashMap<Box<str>, Vec<u32>>,
    context_ids: Vec<(Option<u16>, Option<u16>)>,
    longest: usize,
}

impl std::fmt::Debug for JaDictionary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JaDictionary")
            .field("entries", &self.lex.len())
            .field("readings", &self.index.len())
            .field("longest", &self.longest)
            .finish()
    }
}

impl JaDictionary {
    /// 開啟 pack 並建立讀音索引。
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, JaError> {
        Ok(Self::new(Arc::new(JaLexicon::open(path)?)))
    }

    /// 用已開啟的詞典建立讀音索引。
    pub fn new(lex: Arc<JaLexicon>) -> Self {
        let mut index: HashMap<Box<str>, Vec<u32>> = HashMap::with_capacity(lex.len() / 2);
        let pack_entries = lex.pack().entries();
        let mut context_ids = Vec::with_capacity(lex.len());
        for (i, entry) in lex.entries().iter().enumerate() {
            match index.get_mut(entry.reading.as_ref()) {
                Some(ids) => ids.push(i as u32),
                None => {
                    index.insert(entry.reading.clone(), vec![i as u32]);
                }
            }
            let raw = &pack_entries[i];
            context_ids.push((raw.left_id(), raw.right_id()));
        }
        Self {
            longest: lex.longest_reading_units(),
            lex,
            index,
            context_ids,
        }
    }

    /// 底層詞典。
    pub fn lexicon(&self) -> &JaLexicon {
        &self.lex
    }

    /// 讀音種類數。
    pub fn reading_count(&self) -> usize {
        self.index.len()
    }
}

impl JaWordSource for JaDictionary {
    fn longest_reading_units(&self) -> usize {
        self.longest
    }

    fn matches<'a>(&'a self, reading: &[char], start: usize, out: &mut dyn FnMut(JaWordMatch<'a>)) {
        if start >= reading.len() {
            return;
        }
        let mut key = String::with_capacity(16);
        let last = start.saturating_add(self.longest).min(reading.len());
        for end in start + 1..=last {
            key.push(reading[end - 1]);
            let Some(ids) = self.index.get(key.as_str()) else {
                continue;
            };
            for &id in ids {
                let (left_id, right_id) = self.context_ids[id as usize];
                out(JaWordMatch {
                    end,
                    entry: &self.lex.entries()[id as usize],
                    left_id,
                    right_id,
                });
            }
        }
    }

    fn exact(&self, reading: &[char]) -> Vec<&JaEntry> {
        let key: String = reading.iter().collect();
        match self.index.get(key.as_str()) {
            Some(ids) => ids
                .iter()
                .map(|&id| &self.lex.entries()[id as usize])
                .collect(),
            None => Vec::new(),
        }
    }

    fn prefixed(&self, reading: &[char]) -> Vec<&JaEntry> {
        if reading.is_empty() {
            return Vec::new();
        }
        let prefix: String = reading.iter().collect();
        let mut out: Vec<&JaEntry> = Vec::new();
        for (key, ids) in &self.index {
            if !key.starts_with(&prefix) {
                continue;
            }
            for &id in ids {
                out.push(&self.lex.entries()[id as usize]);
            }
        }
        out.sort_by(|a, b| {
            a.cost
                .cmp(&b.cost)
                .then_with(|| a.surface.cmp(&b.surface))
                .then_with(|| a.reading.cmp(&b.reading))
        });
        out
    }
}

/// lattice 上的一條弧：從 `start` 走到 `end` 的一個詞。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JaNode {
    /// 起點，以かな單位計。
    pub start: usize,
    /// 終點（不含），以かな單位計。
    pub end: usize,
    /// 詞條。
    pub entry: JaEntry,
    /// 是否為未知詞 fallback。
    pub unknown: bool,
    /// left context ID；未知詞固定 `None`。
    pub left_id: Option<u16>,
    /// right context ID；未知詞固定 `None`。
    pub right_id: Option<u16>,
}

impl JaNode {
    /// 建立詞弧。
    pub fn new(start: usize, end: usize, entry: JaEntry) -> Self {
        Self {
            start,
            end,
            entry,
            unknown: false,
            left_id: None,
            right_id: None,
        }
    }

    /// 詞弧的讀音單位數。
    pub fn units(&self) -> usize {
        self.end.saturating_sub(self.start)
    }
}

/// 依起點分組的詞弧。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lattice {
    starts: Vec<Vec<JaNode>>,
}

impl Lattice {
    /// 回傳從 `pos` 開始的所有詞弧；超出範圍回傳空切片。
    pub fn nodes(&self, pos: usize) -> &[JaNode] {
        self.starts.get(pos).map(Vec::as_slice).unwrap_or(&[])
    }

    /// 位置數（等於讀音單位數）。
    pub fn len(&self) -> usize {
        self.starts.len()
    }

    /// 是否為空 lattice。
    pub fn is_empty(&self) -> bool {
        self.starts.is_empty()
    }
}

/// 個人學習的鉤子。
///
/// 回傳指定詞條在該位置的額外成本；0 表示不調整，負值可以提高優先度。
pub trait JaLearning: Send + Sync {
    /// 額外成本。
    fn cost(&self, reading: &[char], start: usize, entry: &JaEntry, ctx: &JaContext) -> i32;
}

/// 沒有學習資料時的空實作。
#[derive(Debug, Clone, Copy, Default)]
pub struct NoLearning;

impl JaLearning for NoLearning {
    fn cost(&self, _: &[char], _: usize, _: &JaEntry, _: &JaContext) -> i32 {
        0
    }
}

/// 驗證器的鉤子。
///
/// `score` 回傳整句結果的額外加分，越高越好；不得改變讀音對應。
pub trait JaVerifier: Send + Sync {
    /// 額外加分。
    fn score(&self, reading: &[char], result: &JaResult, ctx: &JaContext) -> f64;
}

/// 沒有驗證模型時的空實作。
#[derive(Debug, Clone, Copy, Default)]
pub struct NoVerifier;

impl JaVerifier for NoVerifier {
    fn score(&self, _: &[char], _: &JaResult, _: &JaContext) -> f64 {
        0.0
    }
}

/// 詞層級重排鉤子。
///
/// 一組候選是同一段讀音的多個詞面，`candidates` 依統計成本由低到高排序，
/// 第一筆是統計第一名。回傳與 `candidates` 等長的分數，越高越好；回
/// `None` 代表這一組放棄，解碼器保留統計成本。
pub trait JaReranker: Send + Sync {
    /// 為同一讀音的多個詞面打分。
    ///
    /// `left`／`right` 是候選詞前後的已解碼文字；`candidates` 依統計成本
    /// 由低到高排序。
    fn score_words(&self, left: &str, right: &str, candidates: &[&str]) -> Option<Vec<f32>>;

    /// 一次轉換開始；預設不做事，實作可累計呼叫次數。
    fn begin(&self) {}

    /// 一次轉換結束；`timed_out` 代表預算用盡、整批放棄。
    fn end(&self, _timed_out: bool) {}

    /// 某一組候選的排序被改寫；預設不做事。
    fn note_changed(&self) {}

    /// 一次轉換的詞群統計：`total` 是門檻前的合格詞群數，`sent` 是實際
    /// 送進模型的群組數。預設不做事。
    fn note_gated(&self, _total: usize, _sent: usize) {}

    /// 單次轉換的推論預算。
    fn timeout(&self) -> Duration {
        Duration::from_millis(100)
    }
}

/// 依讀音建立かな lattice。
///
/// 每個起點都走訪詞典的所有讀音前綴，另外固定加入一個單字かな的未知詞
/// fallback。`prefer_hiragana` 開啟時套用 [`apply_script_policy`]。
#[derive(Default)]
pub struct JaBuilder<'a> {
    /// 詞條來源；`None` 時只有未知詞。
    pub source: Option<&'a dyn JaWordSource>,
    /// 單一詞條可佔用的讀音單位數；0 使用 [`DEFAULT_MAX_WORD_UNITS`]。
    pub max_word_units: usize,
    /// 未知詞的單一かな成本；0 使用 [`DEFAULT_UNKNOWN_WORD_COST`]。
    pub unknown_word_cost: i32,
    /// 學習鉤子。
    pub learning: Option<&'a dyn JaLearning>,
    /// 轉換上下文。
    pub context: JaContext,
    /// 對平假名讀音套用詞面書寫系統偏好。
    pub prefer_hiragana: bool,
    /// 對助詞同音かな額外嘗試另一種讀音（わ→は、え→へ、お→を）。
    pub particle_rewrite: bool,
}

impl<'a> JaBuilder<'a> {
    /// 建立 lattice。
    pub fn build(&self, reading: &[char]) -> Lattice {
        if reading.is_empty() {
            return Lattice::default();
        }
        let max_units = if self.max_word_units == 0 {
            DEFAULT_MAX_WORD_UNITS
        } else {
            self.max_word_units
        };
        let unknown_cost = if self.unknown_word_cost <= 0 {
            DEFAULT_UNKNOWN_WORD_COST
        } else {
            self.unknown_word_cost
        };
        let mut starts = Vec::with_capacity(reading.len());
        let mut swapped = reading.to_vec();
        let ambiguous: Vec<(usize, char)> = reading
            .iter()
            .enumerate()
            .filter_map(|(pos, &unit)| particle_reading_alternative(unit).map(|alt| (pos, alt)))
            .collect();
        for start in 0..reading.len() {
            let mut nodes: Vec<JaNode> = Vec::new();
            if let Some(source) = self.source {
                let mut push = |reading: &[char], start: usize, m: JaWordMatch<'_>| {
                    if m.end.saturating_sub(start) > max_units {
                        return;
                    }
                    let mut entry = m.entry.clone();
                    if let Some(learning) = self.learning {
                        let delta = learning.cost(reading, start, &entry, &self.context);
                        entry.cost = entry.cost.saturating_add(delta);
                    }
                    nodes.push(JaNode {
                        start,
                        end: m.end,
                        entry,
                        unknown: false,
                        left_id: m.left_id,
                        right_id: m.right_id,
                    });
                };
                source.matches(reading, start, &mut |m| push(reading, start, m));
                if self.particle_rewrite {
                    // 只在一個詞的長度範圍內替換讀音，避免為整句每個同音かな
                    // 都重建一次索引查詢。
                    let longest = source.longest_reading_units().max(1);
                    let stop = reading.len().min(start.saturating_add(longest));
                    for &(pos, alt) in ambiguous
                        .iter()
                        .filter(|(pos, _)| *pos >= start && *pos < stop)
                    {
                        let original = swapped[pos];
                        let offset = pos - start;
                        swapped[pos] = alt;
                        source.matches(&swapped, start, &mut |m| {
                            // 只收真正用到交替讀音的詞條；其餘與原本查詢重複。
                            if m.entry.reading.chars().nth(offset) != Some(alt) {
                                return;
                            }
                            if !particle_alternation_allowed(original, m.entry) {
                                return;
                            }
                            push(&swapped, start, m);
                        });
                        swapped[pos] = original;
                    }
                }
            }
            if self.prefer_hiragana {
                apply_script_policy(&mut nodes, reading, start);
            }
            let unit: String = reading[start].to_string();
            let mut entry = unknown_entry(&unit);
            entry.cost = unknown_cost;
            nodes.push(JaNode {
                start,
                end: start + 1,
                entry,
                unknown: true,
                left_id: None,
                right_id: None,
            });
            starts.push(nodes);
        }
        Lattice { starts }
    }
}

/// 交替讀音是否可以用在這筆詞條。
///
/// 「わ→は」除了助詞，還要允許 `こんにちは`、`あるいは` 這種整詞讀音本來就
/// 以「は」表記的詞條，否則使用者把整詞打成同音かな時無法還原。「え→へ」與
/// 「お→を」只接受助詞，避免把 `えき` 讀成 `へき`（壁）這類同音詞。
fn particle_alternation_allowed(original: char, entry: &JaEntry) -> bool {
    if entry.pos_class() == JaPosClass::Particle {
        return true;
    }
    original == 'わ' && entry.reading.chars().count() > 1
}

/// 文節分割器。
///
/// 規則是固定的：付属語依附前一個詞，接頭詞與下一個詞黏在一起，自立語
/// 開始新文節；只有 `merge_cost > 0` 且連接成本不超過它時才合併複合詞。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JaSegmenter {
    /// 複合詞合併門檻（成本）；0 表示不合併。
    pub merge_cost: i32,
}

impl JaSegmenter {
    /// 建立分割器。
    pub fn new(merge_cost: i32) -> Self {
        Self { merge_cost }
    }

    /// `cur` 是否要開始一個新文節。
    pub fn should_break(&self, prev: &JaEntry, cur: &JaEntry, conn: &dyn ConnectionCost) -> bool {
        let prev_pos = prev.pos_class();
        if prev.surface.is_empty() || prev_pos == JaPosClass::Bos {
            return false;
        }
        if prev_pos == JaPosClass::Prefix {
            return false;
        }
        let cur_pos = cur.pos_class();
        if cur_pos == JaPosClass::Prefix {
            return true;
        }
        if is_dependent(cur_pos) {
            return false;
        }
        if is_dependent(prev_pos) {
            return true;
        }
        if !is_independent(cur_pos) {
            return false;
        }
        if self.merge_cost > 0 && conn.cost(prev_pos, cur_pos) <= self.merge_cost {
            return false;
        }
        true
    }
}

/// 把詞序列合併成文節。
///
/// `nodes` 必須依序列順序排列且覆蓋完整讀音。
pub fn clauses(nodes: &[JaNode], conn: &dyn ConnectionCost, seg: JaSegmenter) -> Vec<JaClause> {
    let refs: Vec<&JaNode> = nodes.iter().collect();
    clauses_from_refs(&refs, conn, seg)
}

fn clauses_from_refs(
    nodes: &[&JaNode],
    conn: &dyn ConnectionCost,
    seg: JaSegmenter,
) -> Vec<JaClause> {
    if nodes.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<JaClause> = Vec::new();
    let mut group: Vec<&JaNode> = Vec::new();
    for node in nodes {
        if let Some(last) = group.last() {
            if seg.should_break(&last.entry, &node.entry, conn) {
                out.push(clause(&group, conn));
                group.clear();
            }
        }
        group.push(node);
    }
    if !group.is_empty() {
        out.push(clause(&group, conn));
    }
    out
}

fn clause(nodes: &[&JaNode], conn: &dyn ConnectionCost) -> JaClause {
    let mut surface = String::new();
    let mut reading = String::new();
    let mut segments = Vec::with_capacity(nodes.len());
    for node in nodes {
        surface.push_str(&node.entry.surface);
        reading.push_str(&node.entry.reading);
        segments.push(JaClauseSegment {
            reading: node.entry.reading.to_string(),
            surface: node.entry.surface.to_string(),
        });
    }
    JaClause {
        reading,
        surface,
        segments,
        cost: clause_cost(nodes, conn),
    }
}

fn clause_cost(nodes: &[&JaNode], conn: &dyn ConnectionCost) -> i32 {
    let mut cost = 0i32;
    let mut prev = JaPosClass::Bos;
    for node in nodes {
        let pos = node.entry.pos_class();
        cost = cost.saturating_add(node.entry.cost);
        cost = cost.saturating_add(conn.cost(prev, pos));
        prev = pos;
    }
    cost
}

/// 連接成本表的 context ID 介面。
///
/// 只有詞弧帶 left／right ID 且解碼器設定了這個矩陣時才會使用；否則一律
/// 退回 [`ConnectionCost`] 的詞性表。
pub trait JaContextCost: Send + Sync {
    /// 回傳 `left` 接 `right` 的連接成本；ID 0 是 BOS／EOS。
    fn cost(&self, left: u16, right: u16) -> i32;
}

/// 沒有 context 矩陣時的保底實作。
#[derive(Debug, Clone, Copy, Default)]
pub struct NoContextCost;

impl JaContextCost for NoContextCost {
    fn cost(&self, _: u16, _: u16) -> i32 {
        DEFAULT_CONNECTION_COST
    }
}

impl JaContextCost for JaConnectionCost {
    /// 越界或沒有矩陣時退回 [`DEFAULT_CONNECTION_COST`]，與前身
    /// `Connections.ContextCost` 的保底行為一致。
    fn cost(&self, left: u16, right: u16) -> i32 {
        JaConnectionCost::cost(self, left, right).unwrap_or(DEFAULT_CONNECTION_COST)
    }
}

/// 日文轉換器：かな字串進，N-best 的 [`Conversion`] 出。
///
/// 解碼器不保存輸入狀態，同一個實作可以同時服務多個轉換請求。
#[derive(Clone)]
pub struct JaDecoder {
    source: Arc<dyn JaWordSource>,
    conn: Arc<dyn ConnectionCost>,
    context: Option<Arc<dyn JaContextCost>>,
    segmenter: JaSegmenter,
    max_word_units: usize,
    unknown_word_cost: i32,
    max_nbest: usize,
    max_candidates: usize,
    learning: Option<Arc<dyn JaLearning>>,
    verifier: Option<Arc<dyn JaVerifier>>,
    reranker: Option<Arc<dyn JaReranker>>,
    rerank_enabled: bool,
    rerank_gate: WordGate,
    prefer_hiragana: bool,
    particle_rewrite: bool,
    lm: Option<Arc<LanguageModel>>,
    lm_weight: f64,
}

impl std::fmt::Debug for JaDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JaDecoder")
            .field("segmenter", &self.segmenter)
            .field("max_word_units", &self.max_word_units)
            .field("unknown_word_cost", &self.unknown_word_cost)
            .field("max_nbest", &self.max_nbest)
            .field("max_candidates", &self.max_candidates)
            .field("has_context", &self.context.is_some())
            .field("prefer_hiragana", &self.prefer_hiragana)
            .field("particle_rewrite", &self.particle_rewrite)
            .field("lm_weight", &self.lm_weight)
            .finish()
    }
}

impl JaDecoder {
    /// 建立解碼器；連接成本預設為全部 [`DEFAULT_CONNECTION_COST`]。
    pub fn new(source: Arc<dyn JaWordSource>) -> Self {
        Self {
            source,
            conn: Arc::new(SingleColumnCost::new()),
            context: None,
            segmenter: JaSegmenter::default(),
            max_word_units: 0,
            unknown_word_cost: 0,
            max_nbest: 0,
            max_candidates: 0,
            learning: None,
            verifier: None,
            reranker: None,
            rerank_enabled: true,
            rerank_gate: DEFAULT_WORD_RERANK_GATE,
            prefer_hiragana: false,
            particle_rewrite: false,
            lm: None,
            lm_weight: 0.0,
        }
    }

    /// 開啟 pack 並建立解碼器。
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, JaError> {
        Ok(Self::new(Arc::new(JaDictionary::open(path)?)))
    }

    /// 開啟 pack 並接上 context ID 連接矩陣。
    ///
    /// pack v2 會同時取得詞條的 left／right ID 與 2,672×2,672 矩陣；v1
    /// pack 仍可開啟，但查詢一律退回詞性連接成本。詞典與矩陣共用同一份
    /// 映射，不會重複解碼。
    pub fn open_with_context(path: impl AsRef<std::path::Path>) -> Result<Self, JaError> {
        let lex = Arc::new(JaLexicon::open(path)?);
        let context = Arc::new(lex.connection_cost());
        let dict = Arc::new(JaDictionary::new(Arc::clone(&lex)));
        Ok(Self::new(dict).with_context_cost(context))
    }

    /// 設定詞性連接成本。
    pub fn with_connection(mut self, conn: Arc<dyn ConnectionCost>) -> Self {
        self.conn = conn;
        self
    }

    /// 設定 context ID 連接矩陣。
    pub fn with_context_cost(mut self, context: Arc<dyn JaContextCost>) -> Self {
        self.context = Some(context);
        self
    }

    /// 設定文節分割器。
    pub fn with_segmenter(mut self, segmenter: JaSegmenter) -> Self {
        self.segmenter = segmenter;
        self
    }

    /// 設定單一詞條可佔用的讀音單位數；0 使用預設值。
    pub fn with_max_word_units(mut self, units: usize) -> Self {
        self.max_word_units = units;
        self
    }

    /// 設定未知詞的單一かな成本；0 使用預設值。
    pub fn with_unknown_word_cost(mut self, cost: i32) -> Self {
        self.unknown_word_cost = cost;
        self
    }

    /// 設定 `Convert` 保留的候選數；0 使用 [`DEFAULT_MAX_NBEST`]。
    pub fn with_max_nbest(mut self, n: usize) -> Self {
        self.max_nbest = n;
        self
    }

    /// 設定候選數上限；0 使用 [`DEFAULT_MAX_CANDIDATES`]。
    pub fn with_max_candidates(mut self, n: usize) -> Self {
        self.max_candidates = n;
        self
    }

    /// 設定學習鉤子。
    pub fn with_learning(mut self, learning: Arc<dyn JaLearning>) -> Self {
        self.learning = Some(learning);
        self
    }

    /// 設定驗證鉤子。
    pub fn with_verifier(mut self, verifier: Arc<dyn JaVerifier>) -> Self {
        self.verifier = Some(verifier);
        self
    }

    /// 設定重排鉤子。
    pub fn with_reranker(mut self, reranker: Arc<dyn JaReranker>) -> Self {
        self.reranker = Some(reranker);
        self
    }

    /// 執行期開關；關閉時保留重排器但不呼叫，排序與統計結果相同。
    pub fn with_rerank_enabled(mut self, enabled: bool) -> Self {
        self.rerank_enabled = enabled;
        self
    }

    /// 設定詞群門檻；只有統計不確定的詞群會送模型。
    pub fn with_rerank_gate(mut self, gate: WordGate) -> Self {
        self.rerank_gate = gate;
        self
    }

    /// 目前的詞群門檻。
    pub fn rerank_gate(&self) -> WordGate {
        self.rerank_gate
    }

    /// 重排器目前是否會實際執行。
    pub fn rerank_enabled(&self) -> bool {
        self.rerank_enabled && self.reranker.is_some()
    }

    /// 設定平假名讀音的詞面偏好。
    pub fn with_prefer_hiragana(mut self, prefer: bool) -> Self {
        self.prefer_hiragana = prefer;
        self
    }

    /// 開啟或關閉助詞讀音交替。
    ///
    /// 開啟後，建立 lattice 時會對「わ」「え」「お」額外嘗試「は」「へ」「を」
    /// 的讀音，由詞典與 Viterbi 決定是否採用；詞面本身不在這裡改寫。
    pub fn with_particle_rewrite(mut self, enabled: bool) -> Self {
        self.particle_rewrite = enabled;
        self
    }

    /// 助詞讀音交替是否開啟。
    pub fn particle_rewrite(&self) -> bool {
        self.particle_rewrite
    }

    /// 設定語言模型。
    pub fn with_lm(mut self, lm: Arc<LanguageModel>) -> Self {
        self.lm = Some(lm);
        self
    }

    /// 設定語言模型權重；0 表示即使載入模型也不改變排序。
    pub fn with_lm_weight(mut self, weight: f64) -> Self {
        self.lm_weight = weight;
        self
    }

    /// 詞條來源。
    pub fn source(&self) -> &Arc<dyn JaWordSource> {
        &self.source
    }

    /// 詞性連接成本。
    pub fn connection(&self) -> &Arc<dyn ConnectionCost> {
        &self.conn
    }

    /// 回傳整段讀音的最佳轉換；空讀音回傳 `None`。
    ///
    /// 有驗證器時保留數條統計候選再重排；詞層級重排直接在 lattice 內作用，
    /// 不需要額外的 N-best。
    pub fn convert(&self, reading: &str, ctx: &JaContext) -> Option<Conversion> {
        let units: Vec<char> = reading.chars().collect();
        if units.is_empty() {
            return None;
        }
        let k = if self.verifier.is_some() {
            self.desired_nbest()
        } else {
            1
        };
        self.results(&units, ctx, k)
            .into_iter()
            .next()
            .map(|r| r.conversion())
    }

    /// 回傳前 `n` 條轉換結果，依分數由高到低排序。
    pub fn nbest(&self, reading: &str, ctx: &JaContext, n: usize) -> Vec<Conversion> {
        if n == 0 {
            return Vec::new();
        }
        let units: Vec<char> = reading.chars().collect();
        self.results(&units, ctx, n)
            .into_iter()
            .map(|r| r.conversion())
            .collect()
    }

    /// 回傳前 `n` 條完整結果（含文節）；空讀音回傳空清單。
    pub fn results(&self, reading: &[char], ctx: &JaContext, n: usize) -> Vec<JaResult> {
        if n == 0 || reading.is_empty() {
            return Vec::new();
        }
        let (hyps, finals, lat) = self.search(reading, ctx, n);
        let mut out: Vec<JaResult> = Vec::with_capacity(finals.len());
        let mut seen: HashSet<String> = HashSet::new();
        for sr in &finals {
            let mut result = self.result(&hyps, sr, &lat);
            let surface = result.surface();
            if !seen.insert(surface) {
                continue;
            }
            if let Some(verifier) = &self.verifier {
                result.score += verifier.score(reading, &result, ctx);
            }
            out.push(result);
        }
        out.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.surface().cmp(&b.surface()))
        });
        out
    }

    /// 回傳指定讀音範圍的候選，依成本由低到高排序。
    ///
    /// 範圍不合法時回傳空清單，不視為錯誤：外殼可能在游標移動後送出舊範圍。
    pub fn candidates(&self, reading: &[char], span: JaSpan, ctx: &JaContext) -> Vec<JaCandidate> {
        if span.length == 0 || span.start > reading.len() || span.end() > reading.len() {
            return Vec::new();
        }
        let limit = self.desired_candidates();
        let sub = &reading[span.start..span.end()];
        let sub_reading: String = sub.iter().collect();

        let mut cands: Vec<JaCandidate> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for result in self.results(sub, ctx, limit) {
            let surface = result.surface();
            if surface.is_empty() || !seen.insert(surface.clone()) {
                continue;
            }
            cands.push(JaCandidate {
                surface,
                reading: sub_reading.clone(),
                cost: raw_cost(result.cost),
                source: JaCandidateSource::Convert,
            });
        }
        for entry in self.source.exact(sub) {
            if !seen.insert(entry.surface.to_string()) {
                continue;
            }
            let cost = self.candidate_cost(entry);
            cands.push(JaCandidate {
                surface: entry.surface.to_string(),
                reading: sub_reading.clone(),
                cost: raw_cost(cost),
                source: JaCandidateSource::Convert,
            });
        }
        cands.sort_by(|a, b| a.cost.cmp(&b.cost).then_with(|| a.surface.cmp(&b.surface)));
        cands.truncate(limit);
        cands
    }

    /// 回傳讀音以 `reading` 開頭的較長詞條，供預測補全。
    pub fn prefixes(&self, reading: &[char]) -> Vec<JaCandidate> {
        if reading.is_empty() {
            return Vec::new();
        }
        let limit = self.desired_candidates();
        let units = reading.len();
        let mut cands: Vec<JaCandidate> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for entry in self.source.prefixed(reading) {
            if entry.units() <= units || !seen.insert(entry.surface.to_string()) {
                continue;
            }
            cands.push(JaCandidate {
                surface: entry.surface.to_string(),
                reading: entry.reading.to_string(),
                cost: entry.cost,
                source: JaCandidateSource::Predict,
            });
        }
        cands.sort_by(|a, b| {
            a.cost
                .cmp(&b.cost)
                .then_with(|| a.surface.cmp(&b.surface))
                .then_with(|| a.reading.cmp(&b.reading))
        });
        cands.truncate(limit);
        cands
    }

    fn desired_nbest(&self) -> usize {
        if self.max_nbest == 0 {
            DEFAULT_MAX_NBEST
        } else {
            self.max_nbest
        }
    }

    fn desired_candidates(&self) -> usize {
        if self.max_candidates == 0 {
            DEFAULT_MAX_CANDIDATES
        } else {
            self.max_candidates
        }
    }

    fn candidate_cost(&self, entry: &JaEntry) -> f64 {
        let pos = entry.pos_class();
        let base = go_cost(entry.cost)
            + go_cost(self.conn.cost(JaPosClass::Bos, pos))
            + go_cost(self.conn.cost(pos, JaPosClass::Eos));
        base + self.lm_cost("", &entry.surface)
    }

    fn builder<'a>(&'a self, ctx: &JaContext) -> JaBuilder<'a> {
        JaBuilder {
            source: Some(self.source.as_ref()),
            max_word_units: self.max_word_units,
            unknown_word_cost: self.unknown_word_cost,
            learning: self.learning.as_deref(),
            context: ctx.clone(),
            prefer_hiragana: self.prefer_hiragana,
            particle_rewrite: self.particle_rewrite,
        }
    }

    fn initial_key(&self) -> StateKey {
        StateKey {
            pos: JaPosClass::Bos,
            right_id: self.context.as_ref().map(|_| 0),
            ctx: None,
        }
    }

    fn connection_cost(&self, left: &StateKey, right: &JaNode) -> i32 {
        if let Some(context) = &self.context {
            if let (Some(rid), Some(lid)) = (left.right_id, right.left_id) {
                return context.cost(rid, lid);
            }
        }
        self.conn.cost(left.pos, right.entry.pos_class())
    }

    fn final_connection_cost(&self, left: &StateKey) -> i32 {
        if let Some(context) = &self.context {
            if let Some(rid) = left.right_id {
                return context.cost(rid, 0);
            }
        }
        self.conn.cost(left.pos, JaPosClass::Eos)
    }

    fn lm_cost(&self, left: &str, cur: &str) -> f64 {
        match &self.lm {
            Some(lm) if self.lm_weight != 0.0 => self.lm_weight * lm.cost(left, cur),
            _ => 0.0,
        }
    }

    fn search(
        &self,
        reading: &[char],
        ctx: &JaContext,
        k: usize,
    ) -> (Vec<Hypothesis>, Vec<SearchResult>, Lattice) {
        let k = k.max(1);
        let lat = self.builder(ctx).build(reading);
        let (base_hyps, base_finals) = self.viterbi(&lat, k, None);
        let Some(reranker) = self.reranker.as_deref().filter(|_| self.rerank_enabled()) else {
            return (base_hyps, base_finals, lat);
        };
        let started = Instant::now();
        reranker.begin();
        let deltas = match base_finals.first() {
            Some(best) => {
                let prefixes = path_prefixes(&base_hyps, &lat, best.hyp, reading.len());
                let suffixes = path_suffixes(&base_hyps, &lat, best.hyp, reading.len());
                let states = self.path_states(&base_hyps, &lat, best.hyp, reading.len());
                self.word_deltas(ctx, &lat, &prefixes, &suffixes, &states, reranker, started)
            }
            None => {
                reranker.end(false);
                None
            }
        };
        match deltas {
            Some(deltas) if !deltas.is_empty() => {
                let (hyps, finals) = self.viterbi(&lat, k, Some(&deltas));
                (hyps, finals, lat)
            }
            _ => (base_hyps, base_finals, lat),
        }
    }

    /// 純統計 Viterbi；`deltas` 是詞層級重排加上的成本增量。
    fn viterbi(
        &self,
        lat: &Lattice,
        k: usize,
        deltas: Option<&HashMap<(usize, usize), f64>>,
    ) -> (Vec<Hypothesis>, Vec<SearchResult>) {
        let len = lat.len();
        let mut states: Vec<StateSet> = (0..=len).map(|_| StateSet::default()).collect();
        let mut hyps: Vec<Hypothesis> = Vec::new();
        hyps.push(Hypothesis {
            cost: 0.0,
            prev: None,
            node: None,
            key: String::new(),
        });
        states[0].add(self.initial_key(), 0, &hyps, k);

        for start in 0..len {
            if states[start].order.is_empty() {
                continue;
            }
            let current = std::mem::take(&mut states[start]);
            for (node_index, node) in lat.nodes(start).iter().enumerate() {
                let next_pos = node.entry.pos_class();
                let mut next = StateKey {
                    pos: next_pos,
                    right_id: node.right_id,
                    ctx: None,
                };
                if let Some(lm) = &self.lm {
                    if self.lm_weight != 0.0 && lm.has_left(&node.entry.surface) {
                        next.ctx = Some(node.entry.surface.clone());
                    }
                }
                let entry_key = entry_key(&node.entry);
                let total = go_cost(node.entry.cost)
                    + deltas
                        .and_then(|map| map.get(&(start, node_index)))
                        .copied()
                        .unwrap_or(0.0);
                for key in &current.order {
                    let conn = go_cost(self.connection_cost(key, node));
                    let hyps_for_key = current.map.get(key).map(Vec::as_slice).unwrap_or(&[]);
                    for &hyp_id in hyps_for_key {
                        let left_surface = match hyps[hyp_id].node {
                            Some((pos, idx)) => lat.nodes(pos)[idx].entry.surface.as_ref(),
                            None => "",
                        };
                        let lm = self.lm_cost(left_surface, &node.entry.surface);
                        let key_text = format!("{}\u{0}{}", hyps[hyp_id].key, entry_key);
                        hyps.push(Hypothesis {
                            cost: hyps[hyp_id].cost + total + conn + lm,
                            prev: Some(hyp_id),
                            node: Some((start, node_index)),
                            key: key_text,
                        });
                        let new_id = hyps.len() - 1;
                        states[node.end].add(next.clone(), new_id, &hyps, k);
                    }
                }
            }
            states[start] = current;
        }

        let mut finals: Vec<SearchResult> = Vec::new();
        let last = &states[len];
        for key in &last.order {
            let conn = go_cost(self.final_connection_cost(key));
            for &hyp_id in last.map.get(key).map(Vec::as_slice).unwrap_or(&[]) {
                finals.push(SearchResult {
                    hyp: hyp_id,
                    cost: hyps[hyp_id].cost + conn,
                });
            }
        }
        finals.sort_by(|a, b| {
            a.cost
                .total_cmp(&b.cost)
                .then_with(|| hyps[a.hyp].key.cmp(&hyps[b.hyp].key))
        });
        finals.truncate(k);
        (hyps, finals)
    }

    /// 每個位置在統計第一名路徑上的左側狀態，供詞群的連接成本使用。
    ///
    /// 最佳路徑的節點邊界一定有值；節點內部（沒有詞弧起點）沿用進入該節點
    /// 的狀態，只是估計不確定度用的近似值。
    fn path_states(
        &self,
        hyps: &[Hypothesis],
        lat: &Lattice,
        best: usize,
        len: usize,
    ) -> Vec<StateKey> {
        let mut out: Vec<Option<StateKey>> = vec![None; len + 1];
        let mut nodes: Vec<&JaNode> = Vec::new();
        let mut cursor = Some(best);
        while let Some(id) = cursor {
            if let Some((pos, index)) = hyps[id].node {
                nodes.push(&lat.nodes(pos)[index]);
            }
            cursor = hyps[id].prev;
        }
        nodes.reverse();
        let mut prev = self.initial_key();
        for node in nodes {
            for slot in out.iter_mut().take(node.end.min(len)).skip(node.start) {
                if slot.is_none() {
                    *slot = Some(prev.clone());
                }
            }
            prev = StateKey {
                pos: node.entry.pos_class(),
                right_id: node.right_id,
                ctx: None,
            };
        }
        out[len] = Some(prev);
        out.into_iter()
            .map(|state| state.unwrap_or_else(|| self.initial_key()))
            .collect()
    }

    /// 對每個「同讀音多候選」的詞弧群組打分，回傳 `(start, node) -> 成本增量`。
    ///
    /// 左文是統計第一名的已解碼前文加上外部 `ctx.left`。任何一組失敗都只
    /// 影響自己；預算用盡時整批放棄（回 `None`），使用純統計結果。
    ///
    /// 只有通過 [`WordGate`] 的詞群才送模型；統計已經確定答案的詞群直接
    /// 留給 Viterbi，避免模型把簡單題改壞。
    #[allow(clippy::too_many_arguments)]
    fn word_deltas(
        &self,
        ctx: &JaContext,
        lat: &Lattice,
        prefixes: &[String],
        suffixes: &[String],
        states: &[StateKey],
        reranker: &dyn JaReranker,
        started: Instant,
    ) -> Option<HashMap<(usize, usize), f64>> {
        let budget = reranker.timeout();
        let mut pending: Vec<WordGroup<'_>> = Vec::new();
        for (start, _) in prefixes.iter().enumerate() {
            for group in self.word_groups(lat, start, &states[start]) {
                pending.push(group);
            }
        }
        let total = pending.len();
        let gate = self.rerank_gate;
        pending.retain(|group| gate.allows(group.gap, group.candidates.len()));
        // 成本差最小的詞群最不確定，先送；超過上限的留給統計。
        pending.sort_by_key(|group| (group.gap, group.start));
        pending.truncate(WORD_RERANK_MAX_GROUPS);
        reranker.note_gated(total, pending.len());

        let mut deltas: HashMap<(usize, usize), f64> = HashMap::new();
        for group in pending {
            let start = group.start;
            let candidates = &group.candidates;
            if started.elapsed() >= budget {
                reranker.end(true);
                return None;
            }
            let end = candidates
                .first()
                .and_then(|cand| cand.nodes.first())
                .map(|&node| lat.nodes(start)[node].end)
                .unwrap_or(start);
            let left = format!("{}{}", ctx.left, prefixes[start]);
            let right = format!("{}{}", suffixes[end], ctx.right);
            let surfaces: Vec<&str> = candidates.iter().map(|c| c.surface).collect();
            let Some(scores) = reranker.score_words(&left, &right, &surfaces) else {
                continue;
            };
            if scores.len() != surfaces.len() {
                continue;
            }
            let costs: Vec<i32> = candidates.iter().map(|c| c.cost).collect();
            let best_index = order_by_scores(
                &scores
                    .iter()
                    .map(|s| {
                        if s.is_finite() {
                            f64::from(*s)
                        } else {
                            f64::NEG_INFINITY
                        }
                    })
                    .collect::<Vec<_>>(),
            )[0];
            if best_index != 0 && scores[0].is_finite() {
                reranker.note_changed();
            }
            let delta = word_delta(&costs, &scores);
            for (index, cand) in candidates.iter().enumerate() {
                if delta[index] == 0.0 {
                    continue;
                }
                for &node in &cand.nodes {
                    deltas.insert((start, node), delta[index]);
                }
            }
        }
        reranker.end(false);
        Some(deltas)
    }

    /// 列出 `start` 起點上「同讀音多候選」的詞群。
    ///
    /// 群組以讀音終點分開，候選依「詞成本＋連接成本」排序（第一筆是統計
    /// 第一名），並截斷到模型批次上限。連接成本用 `left` 這個統計路徑上的
    /// 左側狀態計算，所以不確定度反映的是這句裡實際的競爭關係。
    fn word_groups<'a>(
        &self,
        lat: &'a Lattice,
        start: usize,
        left: &StateKey,
    ) -> Vec<WordGroup<'a>> {
        let mut by_end: BTreeMap<usize, Vec<WordCandidate<'_>>> = BTreeMap::new();
        for (index, node) in lat.nodes(start).iter().enumerate() {
            if !scorable_surface(node) {
                continue;
            }
            let total = node
                .entry
                .cost
                .saturating_add(self.connection_cost(left, node));
            let group = by_end.entry(node.end).or_default();
            match group
                .iter_mut()
                .find(|cand| cand.surface == node.entry.surface.as_ref())
            {
                Some(cand) => {
                    cand.cost = cand.cost.min(node.entry.cost);
                    cand.total = cand.total.min(total);
                    cand.nodes.push(index);
                }
                None => group.push(WordCandidate {
                    surface: node.entry.surface.as_ref(),
                    cost: node.entry.cost,
                    total,
                    nodes: vec![index],
                }),
            }
        }
        by_end
            .into_values()
            .filter_map(|mut candidates| {
                if candidates.len() < 2 {
                    return None;
                }
                candidates.sort_by(|a, b| {
                    a.total
                        .cmp(&b.total)
                        .then_with(|| a.cost.cmp(&b.cost))
                        .then_with(|| a.surface.cmp(b.surface))
                });
                let floor = candidates.first().map(|cand| cand.total).unwrap_or(0);
                candidates
                    .retain(|cand| cand.total.saturating_sub(floor) <= WORD_RERANK_COST_WINDOW);
                candidates.truncate(WORD_RERANK_MAX_CANDIDATES);
                if candidates.len() < 2 {
                    return None;
                }
                let gap = candidates[1].total.saturating_sub(candidates[0].total);
                Some(WordGroup {
                    start,
                    gap,
                    candidates,
                })
            })
            .collect()
    }

    fn result(&self, hyps: &[Hypothesis], sr: &SearchResult, lat: &Lattice) -> JaResult {
        let mut nodes: Vec<&JaNode> = Vec::new();
        let mut cursor = Some(sr.hyp);
        while let Some(id) = cursor {
            let hyp = &hyps[id];
            if let Some((pos, idx)) = hyp.node {
                nodes.push(&lat.nodes(pos)[idx]);
            }
            cursor = hyp.prev;
        }
        nodes.reverse();
        JaResult {
            clauses: clauses_from_refs(&nodes, self.conn.as_ref(), self.segmenter),
            cost: sr.cost,
            score: -sr.cost,
        }
    }
}

/// Viterbi 的狀態鍵。
///
/// `pos` 是前一個詞的粗詞性；`right_id` 是前一個詞的 right context ID；
/// `ctx` 是語言模型 bigram 的左詞面，只有會成為左詞的詞面才拆狀態。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct StateKey {
    pos: JaPosClass,
    right_id: Option<u16>,
    ctx: Option<Box<str>>,
}

/// 一條部分路徑。
struct Hypothesis {
    cost: f64,
    prev: Option<usize>,
    node: Option<(usize, usize)>,
    key: String,
}

/// 某個位置上的狀態集合，保留插入順序讓結果可重現。
#[derive(Default)]
struct StateSet {
    order: Vec<StateKey>,
    map: HashMap<StateKey, Vec<usize>>,
}

impl StateSet {
    fn add(&mut self, key: StateKey, hyp: usize, hyps: &[Hypothesis], k: usize) {
        match self.map.get_mut(&key) {
            Some(list) => insert_hypothesis(list, hyps, hyp, k),
            None => {
                let list = vec![hyp];
                self.map.insert(key.clone(), list);
                self.order.push(key);
            }
        }
    }
}

/// 一條完整路徑與含句尾連接成本的總成本。
struct SearchResult {
    hyp: usize,
    cost: f64,
}

fn insert_hypothesis(list: &mut Vec<usize>, hyps: &[Hypothesis], h: usize, k: usize) {
    let mut i = 0;
    while i < list.len() && !hypothesis_less(&hyps[h], &hyps[list[i]]) {
        i += 1;
    }
    if i >= k {
        return;
    }
    list.insert(i, h);
    if list.len() > k {
        list.truncate(k);
    }
}

fn hypothesis_less(a: &Hypothesis, b: &Hypothesis) -> bool {
    if a.cost != b.cost {
        return a.cost < b.cost;
    }
    a.key < b.key
}

fn entry_key(entry: &JaEntry) -> String {
    format!("{}|{}|{}", entry.surface, entry.reading, entry.pos)
}

/// 同一段讀音的一個詞弧候選。
struct WordCandidate<'a> {
    surface: &'a str,
    /// 統計成本（詞典原始整數）。
    cost: i32,
    /// 排序成本：詞成本＋連接成本，只用來決定統計名次與不確定度。
    total: i32,
    /// 這個詞面在 lattice 上的詞弧編號；同詞面的重複詞條共用一個增量。
    nodes: Vec<usize>,
}

/// 同一段讀音、同一終點的多個詞面。
struct WordGroup<'a> {
    /// 起點（かな單位）。
    start: usize,
    /// 統計前兩名的排序成本差；越小越不確定。
    gap: i32,
    /// 依排序成本由低到高；第一筆是統計第一名。
    candidates: Vec<WordCandidate<'a>>,
}

/// 這個詞弧是否值得送進詞層級重排。
///
/// 訓練資料的候選是 2 到 4 字、全漢字、讀音至少 2 個單位、成本落在
/// [`WORD_RERANK_COST_WINDOW`] 內的詞面；不在這個範圍的詞弧統計已經決定，
/// 送進模型只會被分佈外的輸入帶偏。
fn scorable_surface(node: &JaNode) -> bool {
    if node.unknown || node.units() < WORD_RERANK_MIN_READING_UNITS {
        return false;
    }
    let surface = node.entry.surface.as_ref();
    let chars = surface.chars().count();
    if chars == 0 || chars > WORD_RERANK_MAX_CHARS {
        return false;
    }
    surface
        .chars()
        .all(|c| matches!(c, '々' | '〆') || c.script() == UnicodeScript::Han)
}

/// 每組候選的成本增量（DP 尺度）。
///
/// 先照前身 `core/ja/rerank` 的權重把神經分數與統計分數融合，再取與最高分
/// 的差距換成成本：模型最喜歡的候選不加成本，其餘候選按差距加上去。統計
/// 成本本身保留在詞弧上，增量只負責在 Viterbi 裡調整相對順序。
fn word_delta(costs: &[i32], scores: &[f32]) -> Vec<f64> {
    let fused = fuse_word_scores(costs, scores);
    let order = order_by_scores(&fused);
    let top = fused[order[0]];
    fused
        .iter()
        .map(|value| {
            if value.is_finite() {
                (top - value).max(0.0) * WORD_SCORE_SCALE
            } else {
                0.0
            }
        })
        .collect()
}

/// 把統計成本與神經分數融合成越高越好的分數。
///
/// `stats_i = (max_cost - cost_i) / (max_cost - min_cost)`，所有成本相同時
/// 給 0.5；第一筆（統計第一名）另外加 [`WORD_DRAFT_BONUS`]。
/// `neural` 不是有限值的候選沒有分數，融合結果是 `NEG_INFINITY`，呼叫端
/// 不對它加成本增量。
fn fuse_word_scores(costs: &[i32], neural: &[f32]) -> Vec<f64> {
    let valid: Vec<usize> = (0..costs.len().min(neural.len()))
        .filter(|&index| neural[index].is_finite())
        .collect();
    let min = valid.iter().map(|&i| costs[i]).min().unwrap_or(0);
    let max = valid.iter().map(|&i| costs[i]).max().unwrap_or(0);
    let span = f64::from(max.saturating_sub(min));
    (0..costs.len())
        .map(|index| {
            if !neural.get(index).is_some_and(|score| score.is_finite()) {
                return f64::NEG_INFINITY;
            }
            let norm = if span > 0.0 {
                f64::from(max - costs[index]) / span
            } else {
                0.5
            };
            let draft = if index == 0 { WORD_DRAFT_BONUS } else { 0.0 };
            WORD_NEURAL_WEIGHT * f64::from(neural[index]) + WORD_STATS_WEIGHT * norm + draft
        })
        .collect()
}

/// 依分數由高到低排出索引；同分保留原本順序。
fn order_by_scores(scores: &[f64]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..scores.len()).collect();
    order.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]).then_with(|| a.cmp(&b)));
    order
}

/// 統計第一名的已解碼後文：`suffixes[pos]` 是 `pos` 之後所有詞面的串接。
fn path_suffixes(hyps: &[Hypothesis], lat: &Lattice, best: usize, len: usize) -> Vec<String> {
    let mut nodes: Vec<&JaNode> = Vec::new();
    let mut cursor = Some(best);
    while let Some(id) = cursor {
        if let Some((pos, index)) = hyps[id].node {
            nodes.push(&lat.nodes(pos)[index]);
        }
        cursor = hyps[id].prev;
    }
    nodes.reverse();
    let mut out = vec![String::new(); len + 1];
    let mut current = String::new();
    let mut next = nodes.len();
    for pos in (0..=len).rev() {
        while next > 0 && nodes[next - 1].start >= pos {
            let node = nodes[next - 1];
            current.insert_str(0, &node.entry.surface);
            next -= 1;
        }
        out[pos].push_str(&current);
    }
    out
}

/// 統計第一名的已解碼前文：`prefixes[pos]` 是 `pos` 之前所有詞面的串接。
fn path_prefixes(hyps: &[Hypothesis], lat: &Lattice, best: usize, len: usize) -> Vec<String> {
    let mut nodes: Vec<&JaNode> = Vec::new();
    let mut cursor = Some(best);
    while let Some(id) = cursor {
        if let Some((pos, index)) = hyps[id].node {
            nodes.push(&lat.nodes(pos)[index]);
        }
        cursor = hyps[id].prev;
    }
    nodes.reverse();
    let mut out = vec![String::new(); len + 1];
    let mut current = String::new();
    let mut next = 0;
    for (pos, slot) in out.iter_mut().enumerate() {
        while next < nodes.len() && nodes[next].start < pos {
            current.push_str(&nodes[next].entry.surface);
            next += 1;
        }
        slot.push_str(&current);
    }
    out
}

/// 詞典成本換成 DP 使用的尺度（Mozc 原始整數除以 1000）。
fn go_cost(raw: i32) -> f64 {
    f64::from(raw) / COST_SCALE
}

/// DP 成本換回詞典的原始整數尺度。
fn raw_cost(cost: f64) -> i32 {
    if !cost.is_finite() {
        return 0;
    }
    (cost * COST_SCALE)
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

/// 付属語：助詞、助動詞、接尾詞。
fn is_dependent(pos: JaPosClass) -> bool {
    matches!(
        pos,
        JaPosClass::Particle | JaPosClass::Auxiliary | JaPosClass::Suffix
    )
}

/// 自立語：可以開始一個新文節。
fn is_independent(pos: JaPosClass) -> bool {
    !matches!(
        pos,
        JaPosClass::Particle
            | JaPosClass::Auxiliary
            | JaPosClass::Suffix
            | JaPosClass::Prefix
            | JaPosClass::Bos
            | JaPosClass::Eos
    )
}

/// 依輸入讀音的書寫系統整理同一組候選詞。
///
/// 平假名讀音代表一般日文輸入：同讀音若已有平假名付属語，片假名與拉丁
/// 詞面視為雜訊；片假名自立語沒有明顯成本優勢時也排除。真正的外來語成本
/// 優勢通常大於容忍度，仍會保留。
fn apply_script_policy(nodes: &mut Vec<JaNode>, reading: &[char], start: usize) {
    if nodes.is_empty() {
        return;
    }
    let mut dependent_ends: HashSet<usize> = HashSet::new();
    let mut best_plain = 0i32;
    let mut have_plain = false;
    for node in nodes.iter() {
        let units = &reading[start..node.end];
        if !hiragana_units(units) {
            continue;
        }
        if !katakana_surface(&node.entry.surface)
            && !latin_surface(&node.entry.surface)
            && (!have_plain || node.entry.cost < best_plain)
        {
            best_plain = node.entry.cost;
            have_plain = true;
        }
        if is_dependent(node.entry.pos_class()) && hiragana_surface(&node.entry.surface) {
            dependent_ends.insert(node.end);
        }
    }
    nodes.retain(|node| {
        let units = &reading[start..node.end];
        if !hiragana_units(units) {
            return true;
        }
        let katakana = katakana_surface(&node.entry.surface);
        if dependent_ends.contains(&node.end) && (katakana || latin_surface(&node.entry.surface)) {
            return false;
        }
        if katakana && have_plain {
            let margin = if node.entry.pos_class() == JaPosClass::Other {
                KATAKANA_UNKNOWN_POS_MARGIN
            } else {
                KATAKANA_MARGIN
            };
            if best_plain.saturating_sub(node.entry.cost) <= margin {
                return false;
            }
        }
        true
    });
}

/// 這段讀音是否全部由平假名（含長音符、踊り字）組成。
fn hiragana_units(units: &[char]) -> bool {
    if units.is_empty() {
        return false;
    }
    units
        .iter()
        .all(|&c| matches!(c, 'ー' | 'ゝ' | 'ゞ') || c.script() == UnicodeScript::Hiragana)
}

/// 詞面是否至少有一個平假名，且其餘字元也都是平假名或長音符。
fn hiragana_surface(surface: &str) -> bool {
    let mut seen = false;
    for c in surface.chars() {
        match c {
            'ー' | 'ゝ' | 'ゞ' => {}
            _ if c.script() == UnicodeScript::Hiragana => seen = true,
            _ => return false,
        }
    }
    seen
}

/// 詞面是否至少有一個片假名，且其餘字元也都是かな或長音符。
fn katakana_surface(surface: &str) -> bool {
    let mut seen = false;
    for c in surface.chars() {
        match c {
            'ー' | 'ゝ' | 'ゞ' => {}
            _ if c.script() == UnicodeScript::Katakana => seen = true,
            _ if c.script() == UnicodeScript::Hiragana => {}
            _ => return false,
        }
    }
    seen
}

/// 詞面是否含拉丁字母（半形或全形）。
fn latin_surface(surface: &str) -> bool {
    surface.chars().any(|c| c.script() == UnicodeScript::Latin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;
    use std::time::Instant;

    /// 測試用詞條，可附加 context ID。
    struct TestWord {
        entry: JaEntry,
        left_id: Option<u16>,
        right_id: Option<u16>,
    }

    /// 線性掃描的記憶體詞條來源，角色等同 Go 的 `lexicon.Mem`。
    struct TestSource {
        words: Vec<TestWord>,
        longest: usize,
    }

    impl TestSource {
        fn new(words: Vec<TestWord>) -> Self {
            let longest = words
                .iter()
                .map(|w| w.entry.units())
                .max()
                .unwrap_or_default();
            Self { words, longest }
        }

        fn plain(entries: Vec<JaEntry>) -> Self {
            Self::new(
                entries
                    .into_iter()
                    .map(|entry| TestWord {
                        entry,
                        left_id: None,
                        right_id: None,
                    })
                    .collect(),
            )
        }
    }

    impl JaWordSource for TestSource {
        fn longest_reading_units(&self) -> usize {
            self.longest
        }

        fn matches<'a>(
            &'a self,
            reading: &[char],
            start: usize,
            out: &mut dyn FnMut(JaWordMatch<'a>),
        ) {
            for word in &self.words {
                let units: Vec<char> = word.entry.reading.chars().collect();
                let end = start + units.len();
                if end > reading.len() || reading[start..end] != units[..] {
                    continue;
                }
                out(JaWordMatch {
                    end,
                    entry: &word.entry,
                    left_id: word.left_id,
                    right_id: word.right_id,
                });
            }
        }

        fn exact(&self, reading: &[char]) -> Vec<&JaEntry> {
            self.words
                .iter()
                .filter(|w| w.entry.reading.chars().eq(reading.iter().copied()))
                .map(|w| &w.entry)
                .collect()
        }

        fn prefixed(&self, reading: &[char]) -> Vec<&JaEntry> {
            let mut out: Vec<&JaEntry> = self
                .words
                .iter()
                .filter(|w| {
                    let units: Vec<char> = w.entry.reading.chars().collect();
                    units.len() >= reading.len() && &units[..reading.len()] == reading
                })
                .map(|w| &w.entry)
                .collect();
            out.sort_by(|a, b| {
                a.cost
                    .cmp(&b.cost)
                    .then_with(|| a.surface.cmp(&b.surface))
                    .then_with(|| a.reading.cmp(&b.reading))
            });
            out
        }
    }

    fn entry(surface: &str, reading: &str, cost: i32, pos: &str) -> JaEntry {
        JaEntry {
            surface: surface.into(),
            reading: reading.into(),
            pos: Arc::from(pos),
            conj_type: Arc::from("*"),
            conj_form: Arc::from("*"),
            cost,
            split: crate::ja::lexicon::Split::C,
        }
    }

    fn with_context(entry: JaEntry, left: u16, right: u16) -> TestWord {
        TestWord {
            entry,
            left_id: Some(left),
            right_id: Some(right),
        }
    }

    fn units(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    fn surfaces(result: &JaResult) -> Vec<String> {
        result.clauses.iter().map(|c| c.surface.clone()).collect()
    }

    /// Go `testdata/lexicon.tsv` 的內容；成本 1.0 對應 1000。
    fn corpus_entries() -> Vec<JaEntry> {
        vec![
            entry("明日", "あした", 1_000, "noun"),
            entry("届く", "とどく", 1_000, "verb"),
            entry("らしい", "らしい", 500, "auxiliary"),
            entry("味", "あじ", 1_000, "noun"),
            entry("を", "を", 200, "particle"),
            entry("しめる", "しめる", 1_000, "verb"),
            entry("店", "みせ", 1_000, "noun"),
            entry("に", "に", 200, "particle"),
            entry("行かない", "いかない", 1_000, "verb"),
            entry("ので", "ので", 500, "particle"),
            entry("目", "め", 1_000, "noun"),
            entry("が", "が", 200, "particle"),
            entry("離せません", "はなせません", 1_000, "verb"),
            entry("醜い", "みにくい", 1_000, "adjective"),
            entry("アヒル", "あひる", 1_000, "noun"),
            entry("の", "の", 200, "particle"),
            entry("子", "こ", 1_000, "noun"),
            entry("読む", "よむ", 1_000, "verb"),
            entry("私", "わたし", 1_000, "noun"),
            entry("幸せ", "しあわせ", 1_000, "noun"),
            entry("やってくる", "やってくる", 1_000, "verb"),
            entry("のでしょうか", "のでしょうか", 500, "auxiliary"),
            entry("問題", "もんだい", 1_000, "noun"),
            entry("無い", "ない", 1_000, "adjective"),
            entry("か", "か", 200, "particle"),
            entry("確認して", "かくにんして", 1_000, "verb"),
            entry(
                "いただけないでしょうか",
                "いただけないでしょうか",
                500,
                "auxiliary",
            ),
            entry("今日", "きょう", 1_000, "noun"),
            entry("京", "きょう", 2_000, "noun"),
            entry("きょう", "きょう", 3_000, "noun"),
            entry("橋", "はし", 1_000, "noun"),
            entry("箸", "はし", 1_500, "noun"),
            entry("端", "はし", 2_000, "noun"),
        ]
    }

    fn test_source() -> Arc<TestSource> {
        Arc::new(TestSource::plain(corpus_entries()))
    }

    fn test_decoder() -> JaDecoder {
        JaDecoder::new(test_source())
    }

    const ANTHY_SAMPLE: &str = "\
|あした|とどくらしい| |明日|届くらしい|
|あじを|しめる| |味を|しめる|
|みせに|いかないので| |店に|行かないので|
|めが|はなせません| |目が|離せません|
|みにくい|あひるの|こを|よむ| |醜い|アヒルの|子を|読む|
|わたしに|しあわせが|やってくるのでしょうか| |私に|幸せが|やってくるのでしょうか|
|もんだいが|ないか|かくにんしていただけないでしょうか| |問題が|無いか|確認していただけないでしょうか|
";

    /// 解析 Go 的 corpus 樣本格式：`|讀音| |詞面|`。
    fn corpus() -> Vec<(Vec<char>, Vec<String>)> {
        let mut out = Vec::new();
        for line in ANTHY_SAMPLE.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut blocks = line.split("| |");
            let read_block = blocks.next().expect("讀音區塊");
            let surface_block = blocks.next().expect("詞面區塊");
            assert!(blocks.next().is_none());
            let read_segments = split_segments(read_block);
            let surface_segments = split_segments(surface_block);
            assert_eq!(read_segments.len(), surface_segments.len());
            let reading: Vec<char> = read_segments.concat().chars().collect();
            out.push((reading, surface_segments));
        }
        out
    }

    fn split_segments(block: &str) -> Vec<String> {
        let block = block.trim();
        let block = block.strip_prefix('|').unwrap_or(block);
        let block = block.strip_suffix('|').unwrap_or(block);
        if block.is_empty() {
            return Vec::new();
        }
        block
            .split('|')
            .map(|part| part.trim().to_string())
            .collect()
    }

    fn convert_surface(dec: &JaDecoder, reading: &str) -> String {
        dec.convert(reading, &JaContext::default())
            .unwrap_or_else(|| panic!("{reading} 沒有轉換結果"))
            .surface
    }

    #[test]
    fn test_decoder_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<JaDecoder>();
        assert_send_sync::<Arc<JaDecoder>>();
    }

    #[test]
    fn test_reranker_reorders_and_can_be_disabled() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        /// 把「箸」打成最高分，確認分數真的進入 lattice 成本。
        struct Prefer {
            calls: AtomicUsize,
        }

        impl JaReranker for Prefer {
            fn score_words(
                &self,
                _left: &str,
                _right: &str,
                candidates: &[&str],
            ) -> Option<Vec<f32>> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Some(
                    candidates
                        .iter()
                        .map(|surface| if *surface == "箸" { 2.0 } else { 0.0 })
                        .collect(),
                )
            }
        }

        let reranker = Arc::new(Prefer {
            calls: AtomicUsize::new(0),
        });
        let enabled = test_decoder()
            .with_reranker(reranker.clone())
            .with_rerank_gate(WordGate::Off);
        let reranked = enabled.results(&units("はし"), &JaContext::default(), 3);
        assert_eq!(reranked[0].surface(), "箸");
        assert_eq!(reranker.calls.load(Ordering::SeqCst), 1);

        let disabled = enabled.with_rerank_enabled(false);
        assert!(!disabled.rerank_enabled());
        let stats = disabled.results(&units("はし"), &JaContext::default(), 3);
        assert_eq!(stats[0].surface(), "橋");
        assert_eq!(reranker.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_word_reranker_sees_decoded_left_context() {
        use std::sync::Mutex;

        #[derive(Default)]
        struct Record {
            lefts: Mutex<Vec<String>>,
        }

        impl JaReranker for Record {
            fn score_words(
                &self,
                left: &str,
                _right: &str,
                candidates: &[&str],
            ) -> Option<Vec<f32>> {
                self.lefts.lock().unwrap().push(left.to_string());
                Some(vec![0.0; candidates.len()])
            }
        }

        let reranker = Arc::new(Record::default());
        let dec = test_decoder()
            .with_reranker(reranker.clone())
            .with_rerank_gate(WordGate::Off);
        let result = dec.results(&units("きょうはし"), &JaContext::default(), 1);
        assert_eq!(result[0].surface(), "今日橋");
        let lefts = reranker.lefts.lock().unwrap();
        // 第一個群組沒有前文，第二個群組的左文是統計第一名的已解碼前文。
        assert_eq!(*lefts, vec![String::new(), "今日".to_string()]);
    }

    #[test]
    fn test_word_reranker_timeout_keeps_statistics() {
        use std::time::Duration;

        struct Expired;

        impl JaReranker for Expired {
            fn score_words(
                &self,
                _left: &str,
                _right: &str,
                candidates: &[&str],
            ) -> Option<Vec<f32>> {
                Some(vec![0.0; candidates.len()])
            }

            fn timeout(&self) -> Duration {
                Duration::ZERO
            }
        }

        let dec = test_decoder()
            .with_reranker(Arc::new(Expired))
            .with_rerank_gate(WordGate::Off);
        let result = dec.results(&units("はし"), &JaContext::default(), 3);
        assert_eq!(result[0].surface(), "橋");
    }

    #[test]
    fn test_fuse_word_scores_follows_predecessor_weights() {
        // 成本 1000／2000 正規化成 1.0／0.0，第一名再加 0.1。
        let fused = fuse_word_scores(&[1_000, 2_000], &[0.0, 1.0]);
        assert!((fused[0] - 0.3).abs() < 1e-9, "{fused:?}");
        assert!((fused[1] - 1.0).abs() < 1e-9, "{fused:?}");
        // 成本相同時統計項給 0.5。
        let flat = fuse_word_scores(&[1_000, 1_000], &[0.0, 0.0]);
        assert!((flat[0] - 0.2).abs() < 1e-9, "{flat:?}");
        assert!((flat[1] - 0.1).abs() < 1e-9, "{flat:?}");
    }

    #[test]
    fn test_word_groups_start_at_statistical_best() {
        let dec = test_decoder();
        let reading = units("はし");
        let lat = dec.builder(&JaContext::default()).build(&reading);
        let groups = dec.word_groups(&lat, 0, &dec.initial_key());
        assert_eq!(groups.len(), 1);
        let group = &groups[0];
        let surfaces: Vec<&str> = group.candidates.iter().map(|c| c.surface).collect();
        assert_eq!(surfaces, vec!["橋", "箸", "端"]);
        assert_eq!(group.candidates[0].cost, 1_000);
        assert_eq!(
            group.gap,
            group.candidates[1].total - group.candidates[0].total
        );
        assert!(group.gap > 0);
    }

    #[test]
    fn test_word_gate_only_lets_uncertain_groups_through() {
        assert!(WordGate::Off.allows(10_000, 2));
        assert!(WordGate::Gap(200).allows(199, 2));
        assert!(!WordGate::Gap(200).allows(200, 2));
        assert!(WordGate::Candidates(3).allows(10_000, 3));
        assert!(!WordGate::Candidates(3).allows(0, 2));
    }

    #[test]
    fn test_gate_skips_confident_groups() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Default)]
        struct Count(AtomicUsize);

        impl JaReranker for Count {
            fn score_words(
                &self,
                _left: &str,
                _right: &str,
                candidates: &[&str],
            ) -> Option<Vec<f32>> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Some(vec![0.0; candidates.len()])
            }
        }

        // 「はし」的三個候選成本差都超過 1，門檻 Gap(1) 不會送模型。
        let reranker = Arc::new(Count::default());
        let dec = test_decoder()
            .with_reranker(reranker.clone())
            .with_rerank_gate(WordGate::Gap(1));
        let result = dec.results(&units("はし"), &JaContext::default(), 1);
        assert_eq!(result[0].surface(), "橋");
        assert_eq!(reranker.0.load(Ordering::SeqCst), 0);

        // 門檻放寬後同一組會進模型。
        let reranker = Arc::new(Count::default());
        let dec = test_decoder()
            .with_reranker(reranker.clone())
            .with_rerank_gate(WordGate::Off);
        let _ = dec.results(&units("はし"), &JaContext::default(), 1);
        assert_eq!(reranker.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_anthy_corpus_samples() {
        let dec = test_decoder();
        let cases = corpus();
        assert!(cases.len() >= 5);
        for (reading, want_segments) in cases {
            let want_surface: String = want_segments.concat();
            let result = dec
                .results(&reading, &JaContext::default(), 1)
                .into_iter()
                .next()
                .unwrap_or_else(|| panic!("{want_surface}: 沒有結果"));
            assert_eq!(result.surface(), want_surface);
            assert_eq!(surfaces(&result), want_segments);
            for (i, clause) in result.clauses.iter().enumerate() {
                assert!(
                    !clause.segments.is_empty(),
                    "{want_surface} 第 {i} 文節沒有 segments"
                );
                assert!(
                    !clause.reading.is_empty(),
                    "{want_surface} 第 {i} 文節沒有讀音"
                );
            }
        }
    }

    #[test]
    fn test_anthy_corpus_boundaries_reproducible() {
        let dec = test_decoder();
        let cases = corpus();
        for (reading, want_segments) in cases {
            let want_surface: String = want_segments.concat();
            let first = dec
                .results(&reading, &JaContext::default(), 1)
                .into_iter()
                .next()
                .expect("第一次轉換");
            for i in 0..5 {
                let next = dec
                    .results(&reading, &JaContext::default(), 1)
                    .into_iter()
                    .next()
                    .expect("重複轉換");
                assert_eq!(
                    surfaces(&first),
                    surfaces(&next),
                    "{want_surface} 第 {i} 次文節邊界不同"
                );
            }
        }
    }

    #[test]
    fn test_variable_length_reading() {
        let dec = test_decoder();
        let result = dec
            .results(&units("あした"), &JaContext::default(), 1)
            .into_iter()
            .next()
            .expect("あした");
        assert_eq!(result.surface(), "明日");
        assert_eq!(result.clauses[0].reading, "あした");
        assert_eq!(result.clauses[0].segments.len(), 1);
    }

    #[test]
    fn test_builder_variable_length_lattice() {
        let source = test_source();
        let builder = JaBuilder {
            source: Some(source.as_ref()),
            ..JaBuilder::default()
        };
        let lat = builder.build(&units("あした"));
        let nodes = lat.nodes(0);
        let mut ends: Vec<usize> = nodes.iter().map(|n| n.end).collect();
        ends.sort_unstable();
        assert!(ends.contains(&3), "lattice 沒有可変長讀音：{ends:?}");
        let end3 = nodes.iter().find(|n| n.end == 3).expect("end 3 的詞弧");
        assert_eq!(end3.entry.surface.as_ref(), "明日");
        let end1 = nodes.iter().find(|n| n.end == 1).expect("end 1 的詞弧");
        assert!(end1.unknown, "單字かな fallback 標記錯誤");
        assert!(lat.nodes(3).is_empty(), "序列尾端不應有詞弧");
        assert!(lat.nodes(usize::MAX).is_empty());
    }

    #[test]
    fn test_nbest_and_candidates() {
        let dec = test_decoder();
        let results = dec.results(&units("はし"), &JaContext::default(), 3);
        assert_eq!(results.len(), 3);
        let want = ["橋", "箸", "端"];
        for result in &results {
            assert!(want.contains(&result.surface().as_str()), "{result:?}");
        }
        for pair in results.windows(2) {
            assert!(pair[0].score >= pair[1].score, "NBest 未依分數排序");
        }

        let cands = dec.candidates(&units("きょう"), JaSpan::new(0, 3), &JaContext::default());
        assert!(cands.len() >= 2, "{cands:?}");
        assert_eq!(cands[0].surface, "今日");
        assert_eq!(cands[0].reading, "きょう");
        assert_eq!(cands[0].source, JaCandidateSource::Convert);

        let seq = units("あしたをよむ");
        let cands = dec.candidates(&seq, JaSpan::new(0, 3), &JaContext::default());
        assert!(!cands.is_empty());
        assert_eq!(cands[0].surface, "明日");
    }

    #[test]
    fn test_candidates_reject_bad_span() {
        let dec = test_decoder();
        let seq = units("あした");
        for span in [
            JaSpan::new(usize::MAX, 1),
            JaSpan::new(0, 0),
            JaSpan::new(2, 2),
            JaSpan::new(4, 1),
        ] {
            assert!(
                dec.candidates(&seq, span, &JaContext::default()).is_empty(),
                "span {span:?} 應被拒絕"
            );
        }
    }

    #[test]
    fn test_prefixes() {
        let dec = test_decoder();
        let cands = dec.prefixes(&units("あし"));
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].surface, "明日");
        assert_eq!(cands[0].source, JaCandidateSource::Predict);
        assert_eq!(cands[0].reading, "あした");

        let predicted = dec.prefixes(&units("は"));
        assert_eq!(predicted.len(), 4, "{predicted:?}");
        let surfaces: HashSet<&str> = predicted.iter().map(|c| c.surface.as_str()).collect();
        for want in ["橋", "箸", "端"] {
            assert!(surfaces.contains(want), "缺少 {want}");
        }

        assert!(dec.prefixes(&units("はし")).is_empty());
        assert!(dec.prefixes(&units("きょう")).is_empty());
        assert!(dec.prefixes(&[]).is_empty());
    }

    #[test]
    fn test_unknown_fallback() {
        let dec = JaDecoder::new(Arc::new(TestSource::new(Vec::new())));
        let result = dec
            .results(&units("ぬこ"), &JaContext::default(), 1)
            .into_iter()
            .next()
            .expect("未知詞");
        assert_eq!(result.surface(), "ぬこ");
        assert_eq!(surfaces(&result), ["ぬ", "こ"]);

        assert!(dec.convert("", &JaContext::default()).is_none());
        assert!(dec.nbest("", &JaContext::default(), 3).is_empty());
        assert!(dec.results(&[], &JaContext::default(), 3).is_empty());
    }

    /// 有 ID 但沒有設定矩陣時退回 POS 成本，不 panic。
    #[test]
    fn test_context_ids_without_matrix_fall_back_to_pos() {
        let source = Arc::new(TestSource::new(vec![
            with_context(entry("A", "あ", 1, "noun"), 1, 1),
            with_context(entry("B", "あ", 2, "noun"), 2, 2),
        ]));
        let mut pos = SingleColumnCost::new();
        pos.set(JaPosClass::Noun, JaPosClass::Eos, 0);
        pos.set(JaPosClass::Bos, JaPosClass::Noun, 0);
        let dec = JaDecoder::new(source).with_connection(Arc::new(pos));
        assert_eq!(convert_surface(&dec, "あ"), "A");
    }

    #[test]
    fn test_real_v2_pack_exposes_context_matrix() {
        let path = jc_testdata::require_data!("packs/ja-lexicon-mozc-v2.pack");
        let conn = crate::ja::lexicon::JaConnectionCost::open(&path).unwrap();
        assert_eq!(conn.context_count(), 2_672);
        assert!(conn.has_matrix());
        assert!(conn.cost(0, 0).is_some());
        assert!(conn.cost(1, 0).is_some());
        assert_eq!(conn.cost(2_672, 0), None);
        assert_eq!(conn.cost(u16::MAX, u16::MAX), None);
    }

    #[test]
    fn test_real_mozc_pack_watashiwa_cost_breakdown() {
        // 這一題的成本由 Mozc 的 context 連接矩陣決定：私(1902) 接 羽(1949)
        // 的連接成本 966，接 わ(431) 是 5556。詞成本差距 4346 抵不過連接成本
        // 差距 4590，所以 私羽 勝出 244。
        let path = jc_testdata::require_data!("packs/ja-lexicon-mozc-v2.pack");
        let dec = JaDecoder::open_with_context(&path).unwrap();
        let results = dec.results(&units("わたしわ"), &JaContext::default(), 2);
        assert_eq!(results[0].surface(), "私羽");
        assert_eq!(results[0].conversion().cost, 9_068);
        assert_eq!(results[1].surface(), "私わ");
        assert_eq!(results[1].conversion().cost, 9_312);
    }

    #[test]
    fn test_particle_rewrite_reading_alternation() {
        let path = jc_testdata::require_data!("packs/ja-lexicon-mozc-v2.pack");
        let plain = JaDecoder::open_with_context(&path).unwrap();
        let rewritten = JaDecoder::open_with_context(&path)
            .unwrap()
            .with_particle_rewrite(true);

        for (reading, want) in [
            // わ→は：助詞，以及整詞讀音本來就是「は」的詞。
            ("わたしわ", "私は"),
            ("こんにちわ", "こんにちは"),
            ("あるいわ", "あるいは"),
            // え→へ、お→を：格助詞。
            ("ほんおよむ", "本を読む"),
            ("みずおのむ", "水を飲む"),
            ("にほんえいく", "日本へ行く"),
            ("がっこうえいく", "学校へ行く"),
        ] {
            assert_eq!(
                convert_surface(&rewritten, reading),
                want,
                "助詞改寫開啟：{reading}"
            );
        }

        // 關閉時保留原本的統計結果，證明差異來自交替假設。
        assert_eq!(convert_surface(&plain, "わたしわ"), "私羽");
        assert_eq!(convert_surface(&plain, "ほんおよむ"), "本お読む");
        assert_eq!(convert_surface(&plain, "にほんえいく"), "日本栄区");

        // 一般詞不受影響：えき 不能變成 へき（壁），おちゃ 不能變成 をちゃ。
        assert_eq!(convert_surface(&rewritten, "えき"), "駅");
        assert_eq!(convert_surface(&rewritten, "おちゃ"), "お茶");
    }

    #[test]
    fn test_particle_rewrite_keeps_existing_wa_words() {
        let path = jc_testdata::require_data!("packs/ja-lexicon-mozc-v2.pack");
        let plain = JaDecoder::open_with_context(&path).unwrap();
        let rewritten = JaDecoder::open_with_context(&path)
            .unwrap()
            .with_particle_rewrite(true);

        // 這些詞本來就以「わ」表記，交替假設不得蓋掉詞典結果。
        for reading in ["すなわち", "いわば"] {
            let before = convert_surface(&plain, reading);
            let after = convert_surface(&rewritten, reading);
            assert_eq!(after, before, "{reading} 不該被改寫");
            assert_eq!(after, reading);
        }
    }

    #[test]
    fn test_connection_cost_changes_best_path() {
        let source = Arc::new(TestSource::plain(vec![
            entry("雨", "あめ", 1_000, "noun"),
            entry("飴", "あめ", 1_000, "adverb"),
        ]));
        let mut conn = SingleColumnCost::new();
        conn.set(JaPosClass::Noun, JaPosClass::Eos, 0);
        conn.set(JaPosClass::Adverb, JaPosClass::Eos, 10);
        let dec = JaDecoder::new(source).with_connection(Arc::new(conn));
        assert_eq!(convert_surface(&dec, "あめ"), "雨");
    }

    #[test]
    fn test_segmenter_clauses() {
        let conn = SingleColumnCost::new();
        let nodes = vec![
            JaNode::new(0, 2, entry("今日", "きょう", 1_000, "noun")),
            JaNode::new(2, 3, entry("は", "は", 1_000, "particle")),
            JaNode::new(3, 5, entry("いい", "いい", 1_000, "adjective")),
            JaNode::new(5, 8, entry("天気", "てんき", 1_000, "noun")),
            JaNode::new(8, 10, entry("です", "です", 1_000, "auxiliary")),
        ];
        let got: Vec<String> = clauses(&nodes, &conn, JaSegmenter::default())
            .into_iter()
            .map(|c| c.surface)
            .collect();
        assert_eq!(got, ["今日は", "いい", "天気です"]);

        let compound = vec![
            JaNode::new(0, 3, entry("日本", "にほん", 1_000, "noun")),
            JaNode::new(3, 4, entry("語", "ご", 1_000, "noun")),
        ];
        let mut conn2 = SingleColumnCost::new();
        conn2.set(JaPosClass::Noun, JaPosClass::Noun, 0);
        let merged = clauses(&compound, &conn2, JaSegmenter::new(500));
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].surface, "日本語");
        assert_eq!(clauses(&compound, &conn2, JaSegmenter::default()).len(), 2);

        let prefixed = vec![
            JaNode::new(0, 1, entry("お", "お", 1_000, "prefix")),
            JaNode::new(1, 3, entry("茶", "ちゃ", 1_000, "noun")),
            JaNode::new(3, 4, entry("を", "を", 1_000, "particle")),
        ];
        let got = clauses(&prefixed, &conn2, JaSegmenter::default());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].surface, "お茶を");
    }

    struct LearnChopsticks;

    impl JaLearning for LearnChopsticks {
        fn cost(&self, _: &[char], _: usize, entry: &JaEntry, _: &JaContext) -> i32 {
            if entry.surface.as_ref() == "箸" {
                -10_000
            } else {
                0
            }
        }
    }

    #[test]
    fn test_learning_hook() {
        let dec = test_decoder().with_learning(Arc::new(LearnChopsticks));
        assert_eq!(convert_surface(&dec, "はし"), "箸");
        let _ = NoLearning;
        let _ = NoVerifier;
    }

    struct VerifyHash;

    impl JaVerifier for VerifyHash {
        fn score(&self, _: &[char], result: &JaResult, _: &JaContext) -> f64 {
            if result.surface() == "端" {
                100.0
            } else {
                0.0
            }
        }
    }

    #[test]
    fn test_verifier_hook() {
        let dec = test_decoder().with_verifier(Arc::new(VerifyHash));
        assert_eq!(convert_surface(&dec, "はし"), "端");
    }

    #[test]
    fn test_32_kana_p95() {
        let dec = test_decoder();
        let reading = format!("{}ねこねこね", "あじをしめるみせに".repeat(3));
        let seq = units(&reading);
        assert_eq!(seq.len(), 32);
        const RUNS: usize = 200;
        let mut durations = Vec::with_capacity(RUNS);
        for _ in 0..RUNS {
            let started = Instant::now();
            assert!(dec.convert(&reading, &JaContext::default()).is_some());
            durations.push(started.elapsed());
        }
        durations.sort_unstable();
        let p95 = durations[RUNS * 95 / 100];
        assert!(
            p95 < std::time::Duration::from_millis(30),
            "32 かな P95 = {p95:?}"
        );
    }

    fn script_source() -> Arc<TestSource> {
        Arc::new(TestSource::plain(vec![
            entry("私", "わたし", 7_159, "other"),
            entry("ワタ", "わた", 7_194, "noun"),
            entry("私話", "しわ", 5_468, "noun"),
            entry("わ", "わ", 11_128, "particle"),
            entry("です", "です", 8_222, "auxiliary"),
            entry("デス", "です", 2_984, "noun"),
            entry("メール", "めーる", -252, "noun"),
            entry("めーる", "めーる", 7_769, "noun"),
        ]))
    }

    #[test]
    fn test_prefer_hiragana_keeps_plain_surfaces() {
        let source = script_source();
        let plain = JaDecoder::new(source.clone());
        assert_eq!(convert_surface(&plain, "わたしわ"), "ワタ私話");
        assert_eq!(convert_surface(&plain, "です"), "デス");

        let preferred = JaDecoder::new(source).with_prefer_hiragana(true);
        assert_eq!(convert_surface(&preferred, "わたしわ"), "私わ");
        assert_eq!(convert_surface(&preferred, "です"), "です");
    }

    #[test]
    fn test_prefer_hiragana_keeps_katakana_loanwords() {
        let dec = JaDecoder::new(script_source()).with_prefer_hiragana(true);
        assert_eq!(convert_surface(&dec, "めーる"), "メール");
    }

    fn lm(unigram: &[(&str, f64)], bigram: &[(&str, &str, f64)]) -> Arc<LanguageModel> {
        Arc::new(LanguageModel::from_pairs(
            unigram
                .iter()
                .map(|(word, cost)| ((*word).to_string(), *cost)),
            bigram
                .iter()
                .map(|(left, right, cost)| (((*left).to_string(), (*right).to_string()), *cost)),
        ))
    }

    #[test]
    fn test_lm_weight_zero_keeps_baseline() {
        let base = test_decoder();
        let with_lm = base
            .clone()
            .with_lm(lm(&[("明日", 50.0)], &[("明日", "届く", 100.0)]));
        for (reading, want_segments) in corpus() {
            let want: String = want_segments.concat();
            let reading: String = reading.iter().collect();
            assert_eq!(convert_surface(&base, &reading), want);
            assert_eq!(convert_surface(&with_lm, &reading), want);
        }
    }

    #[test]
    fn test_lm_bigram_changes_order() {
        let dec = test_decoder();
        assert_eq!(convert_surface(&dec, "はしを"), "橋を");

        let with_bi = dec
            .clone()
            .with_lm(lm(&[], &[("橋", "を", 100.0)]))
            .with_lm_weight(1.0);
        assert_eq!(convert_surface(&with_bi, "はしを"), "箸を");

        let zero = with_bi.clone().with_lm_weight(0.0);
        assert_eq!(convert_surface(&zero, "はしを"), "橋を");

        let other = dec
            .clone()
            .with_lm(lm(&[], &[("箸", "を", 100.0)]))
            .with_lm_weight(1.0);
        assert_eq!(convert_surface(&other, "はしを"), "橋を");
    }

    #[test]
    fn test_lm_is_deterministic() {
        let dec = test_decoder()
            .with_lm(lm(&[("明日", 1.0)], &[("明日", "届く", 1.0)]))
            .with_lm_weight(0.5);
        let first = convert_surface(&dec, "あしたとどく");
        for i in 0..10 {
            assert_eq!(convert_surface(&dec, "あしたとどく"), first, "第 {i} 次");
        }
    }

    /// 測試用 context 矩陣，角色等同 Go 的 `lexicon.NewContextMatrix`。
    struct TestContextCost {
        cells: HashMap<(u16, u16), i32>,
        fallback: i32,
    }

    impl TestContextCost {
        fn new(fallback: i32) -> Self {
            Self {
                cells: HashMap::new(),
                fallback,
            }
        }

        fn set(&mut self, left: u16, right: u16, cost: i32) {
            self.cells.insert((left, right), cost);
        }
    }

    impl JaContextCost for TestContextCost {
        fn cost(&self, left: u16, right: u16) -> i32 {
            self.cells
                .get(&(left, right))
                .copied()
                .unwrap_or(self.fallback)
        }
    }

    #[test]
    fn test_context_matrix_changes_path() {
        let source = Arc::new(TestSource::new(vec![
            with_context(entry("A", "あ", 1_000, "noun"), 1, 1),
            with_context(entry("B", "あ", 2_000, "noun"), 2, 2),
        ]));
        let mut pos = SingleColumnCost::new();
        pos.set(JaPosClass::Bos, JaPosClass::Noun, 0);
        pos.set(JaPosClass::Noun, JaPosClass::Eos, 0);

        let plain = JaDecoder::new(source.clone()).with_connection(Arc::new(pos.clone()));
        assert_eq!(convert_surface(&plain, "あ"), "A");

        let mut ctx = TestContextCost::new(1_000);
        ctx.set(0, 1, 2_000);
        ctx.set(0, 2, 0);
        ctx.set(1, 0, 0);
        ctx.set(2, 0, 0);
        let with_context = JaDecoder::new(source)
            .with_connection(Arc::new(pos))
            .with_context_cost(Arc::new(ctx));
        assert_eq!(convert_surface(&with_context, "あ"), "B");
    }

    fn real_decoder() -> Option<JaDecoder> {
        static CACHE: OnceLock<Option<Arc<JaDictionary>>> = OnceLock::new();
        let path = jc_testdata::path("packs/ja-lexicon-public-core.pack")?;
        let dict = CACHE
            .get_or_init(|| JaDictionary::open(&path).ok().map(Arc::new))
            .as_ref()?
            .clone();
        let mut dec = JaDecoder::new(dict).with_prefer_hiragana(true);
        if let Some(conn) = jc_testdata::path("ja-src/connection.tsv") {
            if let Ok(cost) = SingleColumnCost::load(conn) {
                dec = dec.with_connection(Arc::new(cost));
            }
        }
        Some(dec)
    }

    #[test]
    fn test_real_pack_converts_common_sentences() {
        let _path = jc_testdata::require_data!("packs/ja-lexicon-public-core.pack");
        let dec = real_decoder().expect("開啟真實 pack");
        assert_eq!(convert_surface(&dec, "きょう"), "今日");
        assert_eq!(convert_surface(&dec, "がっこう"), "学校");
        assert!(convert_surface(&dec, "にほんご").starts_with("日本語"));
    }

    #[test]
    fn test_real_pack_modern_500_first_20() {
        let _pack = jc_testdata::require_data!("packs/ja-lexicon-public-core.pack");
        let corpus_path = jc_testdata::require_data!("0c-testdata/ja/modern_500.json");
        let dec = real_decoder().expect("開啟真實 pack");
        let raw = std::fs::read_to_string(&corpus_path).expect("讀取題庫");
        let data: serde_json::Value = serde_json::from_str(&raw).expect("解析題庫");
        let items = data["items"].as_array().expect("items");
        assert!(items.len() >= 20);
        for item in items.iter().take(20) {
            let reading = item["reading"].as_str().expect("reading");
            let reading_units = reading.chars().count();
            let conversion = dec
                .convert(reading, &JaContext::default())
                .unwrap_or_else(|| panic!("{reading} 沒有結果"));
            assert!(!conversion.surface.is_empty(), "{reading} 轉出空字串");
            let surface_units = conversion.surface.chars().count();
            assert!(
                surface_units >= 1 && surface_units <= reading_units * 2,
                "{reading} 轉出長度 {surface_units} 不合理"
            );
            assert!(!conversion.segments.is_empty(), "{reading} 沒有詞邊界");
            let mut cursor = 0usize;
            for segment in &conversion.segments {
                assert_eq!(segment.start, cursor);
                cursor = segment.end;
            }
            assert_eq!(cursor, conversion.surface.len());
        }
    }
}
