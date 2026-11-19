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

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use unicode_script::{Script as UnicodeScript, UnicodeScript as _};

use crate::decode::Conversion;
use crate::ja::lexicon::{
    unknown_entry, ConnectionCost, JaConnectionCost, JaEntry, JaError, JaLexicon, JaPosClass,
    SingleColumnCost, DEFAULT_CONNECTION_COST, UNKNOWN_WORD_COST,
};
use crate::ja::lm::LanguageModel;
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
pub trait JaWordSource {
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
pub trait JaLearning {
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
pub trait JaVerifier {
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

/// 重排鉤子。
///
/// 收到已按統計成本排序的候選，回傳完整排序；實作在任何錯誤時應原樣回傳。
pub trait JaReranker {
    /// 重排候選。
    fn rerank(&self, reading: &[char], drafts: Vec<JaResult>, ctx: &JaContext) -> Vec<JaResult>;
}

/// 依讀音建立かな lattice。
///
/// 每個起點都走訪詞典的所有讀音前綴，另外固定加入一個單字かな的未知詞
/// fallback。`prefer_hiragana` 開啟時套用 [`apply_script_policy`]。
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
}

impl Default for JaBuilder<'_> {
    fn default() -> Self {
        Self {
            source: None,
            max_word_units: 0,
            unknown_word_cost: 0,
            learning: None,
            context: JaContext::default(),
            prefer_hiragana: false,
        }
    }
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
        for start in 0..reading.len() {
            let mut nodes: Vec<JaNode> = Vec::new();
            if let Some(source) = self.source {
                source.matches(reading, start, &mut |m| {
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
                });
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
pub trait JaContextCost {
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
    prefer_hiragana: bool,
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
            prefer_hiragana: false,
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

    /// 設定平假名讀音的詞面偏好。
    pub fn with_prefer_hiragana(mut self, prefer: bool) -> Self {
        self.prefer_hiragana = prefer;
        self
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
    /// 有驗證器或重排器時保留數條統計候選再重排；否則只算最佳路徑。
    pub fn convert(&self, reading: &str, ctx: &JaContext) -> Option<Conversion> {
        let units: Vec<char> = reading.chars().collect();
        if units.is_empty() {
            return None;
        }
        let k = if self.verifier.is_some() || self.reranker.is_some() {
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
        if let Some(reranker) = &self.reranker {
            if !out.is_empty() {
                let reranked = reranker.rerank(reading, out.clone(), ctx);
                if !reranked.is_empty() {
                    out = reranked;
                }
            }
        }
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

        if let Some(reranker) = &self.reranker {
            if !cands.is_empty() {
                cands = self.rerank_candidates(sub, cands, ctx, reranker.as_ref());
            }
        }
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

    fn rerank_candidates(
        &self,
        reading: &[char],
        cands: Vec<JaCandidate>,
        ctx: &JaContext,
        reranker: &dyn JaReranker,
    ) -> Vec<JaCandidate> {
        let mut drafts: Vec<JaResult> = Vec::with_capacity(cands.len());
        let mut by_surface: HashMap<String, JaCandidate> = HashMap::with_capacity(cands.len());
        for cand in &cands {
            if cand.surface.is_empty() {
                continue;
            }
            drafts.push(JaResult {
                clauses: vec![JaClause {
                    reading: cand.reading.clone(),
                    surface: cand.surface.clone(),
                    segments: Vec::new(),
                    cost: cand.cost,
                }],
                cost: go_cost(cand.cost),
                score: -go_cost(cand.cost),
            });
            by_surface.insert(cand.surface.clone(), cand.clone());
        }
        if drafts.is_empty() {
            return cands;
        }
        let reranked = reranker.rerank(reading, drafts, ctx);
        if reranked.is_empty() {
            return cands;
        }
        let mut out: Vec<JaCandidate> = Vec::with_capacity(cands.len());
        let mut seen: HashSet<String> = HashSet::new();
        for result in &reranked {
            let surface = result.surface();
            let Some(cand) = by_surface.get(&surface) else {
                continue;
            };
            if !seen.insert(surface) {
                continue;
            }
            out.push(cand.clone());
        }
        for cand in cands {
            if !seen.contains(&cand.surface) {
                out.push(cand);
            }
        }
        out
    }

    fn builder<'a>(&'a self, ctx: &JaContext) -> JaBuilder<'a> {
        JaBuilder {
            source: Some(self.source.as_ref()),
            max_word_units: self.max_word_units,
            unknown_word_cost: self.unknown_word_cost,
            learning: self.learning.as_deref(),
            context: ctx.clone(),
            prefer_hiragana: self.prefer_hiragana,
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
        let len = reading.len();
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
                            cost: hyps[hyp_id].cost + go_cost(node.entry.cost) + conn + lm,
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
        (hyps, finals, lat)
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
        if !katakana_surface(&node.entry.surface) && !latin_surface(&node.entry.surface) {
            if !have_plain || node.entry.cost < best_plain {
                best_plain = node.entry.cost;
                have_plain = true;
            }
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
