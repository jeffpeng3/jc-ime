//! 個人化模型：階層式貝氏調適、學習規則、個人片語、自訂詞、代名詞與
//! 語言偏好。
//!
//! 這個模組對照前身 `core/personal`。依 Go 檔案拆成子模組：
//! `evidence`（evidence.go）、`diversity`（diversity.go）、`prune`（prune.go）、
//! `unlearn`（unlearn.go）、`preference`（preference.go）、`corrections`
//! （corrections.go）、`pronoun`（pronoun.go）、`english`（english.go）、
//! `language`（language.go）、`vocab`（vocab.go）、`zhuyin_keys`（zhuyin_keys.go）、
//! `emoji`（emoji.go）與 `ja`（ja.go）。Go `personal.go` 的 Store／Model／Learn
//! 留在本檔。
//!
//! 與前身的差異：
//! - 前身的 `lexicon.Source`、`lm.System`、`english.Dictionary` 換成本檔定義的
//!   三個 trait；`jc-core` 現有的 [`crate::lattice::InMemoryLexicon`] 與
//!   [`crate::score::BigramLm`] 都有對應實作。
//! - 自訂詞與代名詞等資料結構改寫成 Rust 的 map／set；JSON 欄位名稱用
//!   snake_case，格式版本從 1 起算，不假裝讀得懂前身格式 8。
//! - 尚未接上外殼的路徑（神經重排、混合輸入、英文補字）由 trait 的預設實作
//!   隔開，整合點寫在 `docs/port/personal-zh.md`。

pub mod corrections;
pub mod diversity;
pub mod emoji;
pub mod english;
pub mod evidence;
pub mod ja;
pub mod language;
pub mod preference;
pub mod pronoun;
pub mod prune;
pub mod unlearn;
pub mod vocab;
pub mod zhuyin_keys;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::lattice::{InMemoryLexicon, LexEntry};
use crate::score::BigramLm;
use crate::zh::syllable::{parse_reading, Syllable};

pub use corrections::{Correction, RejectedSeg};
pub use english::{english_token, EnglishCandidate, EnglishCommit, EnglishEvidence, NoEnglish};
pub use evidence::UsageEvidence;
pub use language::{CrossLanguage, LanguageId, LanguageNamespace, LanguageSelection};
pub use pronoun::{PronounCount, PronounHabit, PronounSite};
pub use prune::{PrunePolicy, PruneStats};
pub use unlearn::Undo;

/// 相容別名：外殼目前把個人資料當成一個值持有。
pub type PersonalStore = Store;

/// 系統語言模型的句首符號。
pub const BOS: &str = "<s>";

/// 系統語言模型沒有登錄的詞機率。
pub const UNSEEN: f64 = 1e-8;

/// 一天秒數。
pub const DAY_SECONDS: i64 = 24 * 60 * 60;

/// 一次手選的證據單位。
pub const CONFIRM_UNIT: f64 = 0.75;

/// 個人資料格式版本。
pub const STORE_FORMAT: u32 = 1;

/// 自訂詞在詞格中的成本。
pub(crate) const VOCAB_COST: i32 = 1;

/// 學習權重最多折抵多少成本。
const LIFT_CAP: i32 = 1500;

/// 系統語言模型。
pub trait SystemLm {
    /// P(w)；未登錄時回傳 [`UNSEEN`]。
    fn uni(&self, word: &str) -> f64;
    /// P(w | left)；沒有 bigram 時回傳 0。
    fn bi(&self, left: &str, word: &str) -> f64;
    /// 是否有以 left 為前詞的 bigram。
    fn has_left(&self, _left: &str) -> bool {
        true
    }
    /// 各字數的 unigram 中位數；沒有提供時回傳 `None`。
    fn uni_quantile_by_len(&self) -> Option<Vec<f64>> {
        None
    }
}

impl<T: BigramLm> SystemLm for T {
    fn uni(&self, word: &str) -> f64 {
        BigramLm::uni(self, word)
    }

    fn bi(&self, left: &str, word: &str) -> f64 {
        BigramLm::bi(self, left, word)
    }
}

/// 空語言模型：所有詞都是未登錄。
#[derive(Debug, Clone, Copy, Default)]
pub struct NoLm;

impl SystemLm for NoLm {
    fn uni(&self, _word: &str) -> f64 {
        UNSEEN
    }

    fn bi(&self, _left: &str, _word: &str) -> f64 {
        0.0
    }

    fn has_left(&self, _left: &str) -> bool {
        false
    }
}

/// 個人化模型使用的詞庫視圖。
pub trait LexiconSource {
    /// 讀音完全相等的詞條。
    fn exact(&self, syls: &[Syllable]) -> Vec<LexEntry>;
    /// 讀音以 `syls` 開頭的詞條。
    fn prefixed(&self, syls: &[Syllable]) -> Vec<LexEntry>;
    /// 詞面是否存在。
    fn has(&self, surface: &str) -> bool;
    /// 詞面所有讀音。
    fn lookup_surface(&self, surface: &str) -> Vec<LexEntry>;
    /// 最長詞條的音節數。
    fn longest_word(&self) -> usize;
}

impl LexiconSource for InMemoryLexicon {
    fn exact(&self, syls: &[Syllable]) -> Vec<LexEntry> {
        let reading = reading_key(syls);
        use crate::lattice::Lexicon;
        Lexicon::lookup(self, &reading).to_vec()
    }

    fn prefixed(&self, syls: &[Syllable]) -> Vec<LexEntry> {
        let reading = reading_key(syls);
        use crate::lattice::Lexicon;
        Lexicon::prefix(self, &reading)
            .into_iter()
            .cloned()
            .collect()
    }

    fn has(&self, surface: &str) -> bool {
        self.entries().iter().any(|entry| entry.surface == surface)
    }

    fn lookup_surface(&self, surface: &str) -> Vec<LexEntry> {
        self.entries()
            .iter()
            .filter(|entry| entry.surface == surface)
            .cloned()
            .collect()
    }

    fn longest_word(&self) -> usize {
        self.entries()
            .iter()
            .map(LexEntry::units)
            .max()
            .unwrap_or(0)
    }
}

/// 空詞庫。
#[derive(Debug, Clone, Copy, Default)]
pub struct NoLexicon;

impl LexiconSource for NoLexicon {
    fn exact(&self, _syls: &[Syllable]) -> Vec<LexEntry> {
        Vec::new()
    }

    fn prefixed(&self, _syls: &[Syllable]) -> Vec<LexEntry> {
        Vec::new()
    }

    fn has(&self, _surface: &str) -> bool {
        false
    }

    fn lookup_surface(&self, _surface: &str) -> Vec<LexEntry> {
        Vec::new()
    }

    fn longest_word(&self) -> usize {
        0
    }
}

/// 英文詞典；英文補字與更正由外層提供。
pub trait EnglishLexicon {
    /// 以 prefix 開頭的詞。
    fn prefix(&self, _prefix: &str, _limit: usize) -> Vec<english::EnglishCandidate> {
        Vec::new()
    }
    /// text 的拼字候選。
    fn corrections(&self, _text: &str, _limit: usize) -> Vec<english::EnglishCandidate> {
        Vec::new()
    }
    /// 詞典是否已有這個詞。
    fn has(&self, _text: &str) -> bool {
        false
    }
    /// 這個詞是否允許個人化建議。
    fn suggestible(&self, _text: &str) -> bool {
        true
    }
}

/// 學習參數。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Params {
    /// 先驗強度下限。
    pub kappa_bi: f64,
    /// 無上下文偏好的先驗強度。
    pub kappa_uni: f64,
    /// 手選證據。
    pub w_manual: f64,
    /// 被動證據。
    pub w_passive: f64,
    /// 補全證據。
    pub w_completion: f64,
    /// 系統 bigram 與 unigram 的內插。
    pub mu: f64,
    /// 前文多樣性的先驗倍率。
    pub type_prior: f64,
    /// 片語長度上限。
    pub phrase_max_len: usize,
    /// 片語晉升次數。
    pub phrase_promote_at: i64,
    /// 自動修正被拒絕幾次後停用。
    pub rejects_to_disable: u32,
    /// 讀音習慣的分數上限。
    pub habit_lift: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            kappa_bi: 1.0,
            kappa_uni: 10_000.0,
            w_manual: 1.0,
            w_passive: 0.25,
            w_completion: 0.5,
            mu: 0.7,
            type_prior: 1.0,
            phrase_max_len: 4,
            phrase_promote_at: 2,
            rejects_to_disable: 2,
            habit_lift: 3.5,
        }
    }
}

/// 可序列化的個人資料。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    /// 格式版本。
    pub format: u32,
    /// 讀音區間＋左右鄰接音節 → 最近明確選取。
    pub corrections: BTreeMap<String, Correction>,
    /// 更正序號。
    pub correction_serial: u64,
    /// 「左詞\t詞」→ 權重。
    pub bi: BTreeMap<String, f64>,
    /// 左詞 → 權重。
    pub ctx: BTreeMap<String, f64>,
    /// 詞 → unigram 權重。
    pub uni: BTreeMap<String, f64>,
    /// 全部 unigram 權重。
    pub total: f64,
    /// 舊版提交次數。
    pub uses: BTreeMap<String, f64>,
    /// 「左詞\t詞」→ 有來源、時間的證據。
    pub evidence_by_context: BTreeMap<String, UsageEvidence>,
    /// 詞 → 弱全域證據。
    pub evidence_by_word: BTreeMap<String, UsageEvidence>,
    /// 宿主提供的 Unix 秒。
    pub time: i64,
    /// 時間是否設定過。
    pub time_set: bool,
    /// 音節 → 曾手選過的單字。
    pub picked: BTreeMap<String, Vec<String>>,
    /// 暫定片語出現次數。
    pub phrase_seen: BTreeMap<String, i64>,
    /// 詞面 → 讀音鍵。
    pub phrases: BTreeMap<String, String>,
    /// 片語晉升順序。
    pub phrase_order: Vec<String>,
    /// 不再學習的詞。
    pub tombstones: BTreeSet<String>,
    /// 「詞\t讀音鍵」：親手用這個讀音選過這個詞。
    pub readings: BTreeSet<String>,
    /// 「詞\t讀音鍵」：自動修正被推翻次數。
    pub rejects: BTreeMap<String, u32>,
    /// 多字詞手選紀錄。
    pub confirmed: BTreeMap<String, UsageEvidence>,
    /// 「前字\t注音符號」。
    pub zhuyin_keys: BTreeMap<String, UsageEvidence>,
    /// 中性代名詞 → 習慣。
    pub pronouns: BTreeMap<String, PronounHabit>,
    /// 正規化英文詞 → 證據。
    pub english: BTreeMap<String, EnglishEvidence>,
    /// 表情選單顯示順序。
    pub emoji_recent: Vec<String>,
    /// 表情常用分數。
    pub emoji_stats: HashMap<String, crate::emoji::Stat>,
    /// 上次選用的表情分頁。
    pub palette_tab: String,
    /// 符號選單顯示順序。
    pub symbol_recent: Vec<String>,
    /// 符號常用分數。
    pub symbol_stats: HashMap<String, crate::emoji::Stat>,
    /// 分語言學習。
    pub languages: BTreeMap<String, LanguageNamespace>,
    /// 跨語言弱偏好。
    pub cross_language: CrossLanguage,
    /// 自訂詞；前身放在 `Vocab` 而非 Store，不寫進個人資料。
    #[serde(skip)]
    pub vocab: BTreeMap<String, Vec<String>>,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            format: STORE_FORMAT,
            corrections: BTreeMap::new(),
            correction_serial: 0,
            bi: BTreeMap::new(),
            ctx: BTreeMap::new(),
            uni: BTreeMap::new(),
            total: 0.0,
            uses: BTreeMap::new(),
            evidence_by_context: BTreeMap::new(),
            evidence_by_word: BTreeMap::new(),
            time: 0,
            time_set: false,
            picked: BTreeMap::new(),
            phrase_seen: BTreeMap::new(),
            phrases: BTreeMap::new(),
            phrase_order: Vec::new(),
            tombstones: BTreeSet::new(),
            readings: BTreeSet::new(),
            rejects: BTreeMap::new(),
            confirmed: BTreeMap::new(),
            zhuyin_keys: BTreeMap::new(),
            pronouns: BTreeMap::new(),
            english: BTreeMap::new(),
            emoji_recent: Vec::new(),
            emoji_stats: HashMap::new(),
            palette_tab: String::new(),
            symbol_recent: Vec::new(),
            symbol_stats: HashMap::new(),
            languages: BTreeMap::new(),
            cross_language: CrossLanguage::default(),
            vocab: BTreeMap::new(),
        }
    }
}

/// 提交文字中的一段。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Seg {
    /// 詞面。
    pub word: String,
    /// 起點音節索引。
    pub start: usize,
    /// 終點音節索引（不含）。
    pub end: usize,
    /// 使用者手動選取。
    pub manual: bool,
    /// 接受系統補全。
    pub completion: bool,
    /// 接受神經次佳提示。
    pub suggested: bool,
    /// 尾端未打完的詞的前綴。
    pub partial: bool,
    /// 經模糊音讀音到達。
    pub fuzzy: bool,
}

/// 一次提交的學習事件。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Commit {
    /// 打出的音節。
    pub syls: Vec<Syllable>,
    /// 最後路徑。
    pub path: Vec<Seg>,
    /// 被手動換掉的原區段。
    pub rejected: Vec<RejectedSeg>,
    /// 被較長詞包住的手選區間。
    pub selected: Vec<Seg>,
}

/// 個人資料的讀寫錯誤。
#[derive(Debug, thiserror::Error)]
pub enum PersonalError {
    /// 檔案由較新版本寫成。
    #[error("個人資料格式 {found} 比本版支援的 {supported} 新")]
    NewerFormat {
        /// 檔案版本。
        found: u32,
        /// 本版支援的版本。
        supported: u32,
    },
    /// 檔案讀寫失敗。
    #[error("讀寫個人資料失敗：{0}")]
    Io(#[from] std::io::Error),
    /// JSON 解析或序列化失敗。
    #[error("解析個人資料失敗：{0}")]
    Json(#[from] serde_json::Error),
}

impl Store {
    /// 建立空白個人資料。
    pub fn new() -> Self {
        Self::default()
    }

    /// 讀取個人資料並清理不合法欄位。
    pub fn load(path: &Path) -> Result<Self, PersonalError> {
        let raw = std::fs::read(path)?;
        let store: Self = serde_json::from_slice(&raw)?;
        if store.format > STORE_FORMAT {
            return Err(PersonalError::NewerFormat {
                found: store.format,
                supported: STORE_FORMAT,
            });
        }
        Ok(store.cleaned())
    }

    /// 寫回檔案。
    pub fn save(&self, path: &Path) -> Result<(), PersonalError> {
        let mut store = self.clone();
        store.format = STORE_FORMAT;
        let raw = serde_json::to_vec_pretty(&store)?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("personal.json");
        let temp = path.with_file_name(format!(".{name}.tmp"));
        {
            use std::io::Write;
            let mut file = std::fs::File::create(&temp)?;
            file.write_all(&raw)?;
            file.sync_all()?;
        }
        std::fs::rename(&temp, path)?;
        Ok(())
    }

    /// 時間前進；排序計數每小時至多整批衰減一次。
    ///
    /// 回傳是否真的做了衰減；第一次設定時間與未滿一小時都回傳 `false`。
    pub fn advance_time(&mut self, now: i64) -> bool {
        let now = now / 3600 * 3600;
        if !self.time_set {
            self.time = now;
            self.time_set = true;
            return false;
        }
        if now - self.time < 3600 {
            return false;
        }
        let factor = evidence::decay(1.0, now - self.time, 30 * DAY_SECONDS);
        for value in self.bi.values_mut() {
            *value *= factor;
        }
        for value in self.ctx.values_mut() {
            *value *= factor;
        }
        for value in self.uni.values_mut() {
            *value *= factor;
        }
        self.total *= factor;
        self.time = now;
        true
    }

    /// 清理不合法欄位並補齊舊格式缺少的索引。
    pub fn cleaned(mut self) -> Self {
        self.emoji_recent = crate::emoji::clean_recent(&self.emoji_recent);
        self.symbol_recent = crate::symbols::clean_recent(&self.symbol_recent);
        if self.confirmed.is_empty() && self.format < 6 {
            self.seed_confirmed();
        }
        self.clean_english();
        self.clean_languages();
        self.cross_language.clean();
        self
    }

    /// 加入自訂詞讀音。
    pub fn add_vocab(&mut self, text: &str, reading: &str) {
        self.vocab
            .entry(text.to_string())
            .or_default()
            .push_unique(reading);
    }

    /// 自訂詞表；與前身一樣只存在記憶體中，不寫進個人資料。
    pub fn vocab(&self) -> &BTreeMap<String, Vec<String>> {
        &self.vocab
    }

    /// 自訂詞是否登記了這個讀音。
    pub fn vocab_has_reading(&self, text: &str, reading: &str) -> bool {
        self.vocab
            .get(text)
            .is_some_and(|readings| readings.iter().any(|item| item == reading))
    }

    /// 刪除自訂詞。
    pub fn remove_vocab(&mut self, text: &str) -> bool {
        self.vocab.remove(text).is_some()
    }

    /// 是否為自訂詞。
    pub fn is_vocab(&self, text: &str) -> bool {
        self.vocab.contains_key(text)
    }

    /// 自訂詞與讀音。
    pub fn vocab_entries(&self) -> impl Iterator<Item = (&str, &[String])> {
        self.vocab
            .iter()
            .map(|(text, readings)| (text.as_str(), readings.as_slice()))
    }

    /// 個人詞典。
    pub fn augment_lexicon(&self, base: &InMemoryLexicon) -> InMemoryLexicon {
        let mut lexicon = base.clone();
        for (text, readings) in self.vocab_entries() {
            for reading in readings {
                lexicon.push(LexEntry::new(text, reading, VOCAB_COST, 1));
            }
        }
        lexicon
    }

    /// 候選排序鍵：自訂詞與手選單字釘選，其餘依學習權重折抵成本。
    pub fn rank_key(&self, text: &str, reading: &str, cost: i32) -> (u8, i32) {
        let single = reading.split_whitespace().count() == 1;
        let pinned =
            self.vocab_has_reading(text, reading) || (single && self.is_picked(reading, text));
        let group = if pinned { 0 } else { 1 };
        let lift = (self.uni.get(text).copied().unwrap_or(0.0) * 1000.0).round() as i32;
        (group, cost.saturating_sub(lift.min(LIFT_CAP)))
    }

    /// 某個音節是否手選過這個單字。
    pub fn is_picked(&self, syllable: &str, word: &str) -> bool {
        self.picked
            .get(syllable)
            .is_some_and(|words| words.iter().any(|item| item == word))
    }

    /// 是否學過這個詞。
    pub fn is_learned(&self, word: &str) -> bool {
        self.phrases.contains_key(word) || self.uni.get(word).copied().unwrap_or(0.0) > 0.0
    }

    /// 是否不再推薦。
    pub fn is_forgotten(&self, word: &str) -> bool {
        self.tombstones.contains(word)
    }

    /// 學習權重換算的成本折扣。
    pub fn lift(&self, word: &str) -> i32 {
        if self.tombstones.contains(word) {
            return 0;
        }
        let evidence = self.uni.get(word).copied().unwrap_or(0.0);
        (evidence * 1000.0).round().min(LIFT_CAP as f64) as i32
    }

    /// 記下一次手選：解除 tombstone，記錄單字釘選與個人讀音，累積證據。
    pub fn note_manual(&mut self, word: &str, reading: &str, syllables: usize) {
        self.tombstones.remove(word);
        if syllables == 1 {
            let words = self.picked.entry(reading.to_string()).or_default();
            if !words.iter().any(|item| item == word) {
                words.push(word.to_string());
            }
        }
        self.readings.insert(pair(word, reading));
        self.observe("", word, CONFIRM_UNIT);
    }

    /// 記下一次被動提交。
    pub fn note_passive(&mut self, word: &str, reading: &str) {
        let _ = reading;
        if self.tombstones.contains(word) {
            return;
        }
        self.observe("", word, self.passive_weight());
    }

    fn passive_weight(&self) -> f64 {
        0.25
    }

    /// 以指定次數加入被動證據。
    pub fn note_seed(&mut self, word: &str, units: f64) {
        if self.tombstones.contains(word) || units <= 0.0 {
            return;
        }
        self.observe("", word, units);
    }

    /// 忘記一個詞：撤銷學習並留下 tombstone。
    pub fn forget(&mut self, word: &str) -> bool {
        let changed = self.is_learned(word) || self.is_picked_any(word);
        self.remove_word(word);
        let inserted = self.tombstones.insert(word.to_string());
        changed || inserted
    }

    /// 恢復學習。
    pub fn relearn(&mut self, word: &str) -> bool {
        self.tombstones.remove(word)
    }

    fn is_picked_any(&self, word: &str) -> bool {
        self.picked
            .values()
            .any(|words| words.iter().any(|item| item == word))
    }

    fn remove_word(&mut self, word: &str) {
        self.phrases.remove(word);
        self.phrase_order.retain(|item| item != word);
        self.uni.remove(word);
        self.uses.remove(word);
    }

    /// 依個人權重重排 N-best 路徑。
    pub fn rank_hypotheses(&self, hypotheses: &mut [crate::decode::Hypothesis], reading: &str) {
        hypotheses.sort_by_key(|item| self.rank_key(&item.text, reading, item.cost));
    }

    /// 依個人權重重排候選。
    pub fn rank_candidates(&self, candidates: &mut [crate::Candidate], reading: &str) {
        candidates.sort_by_key(|item| self.rank_key(&item.surface, reading, item.cost));
    }

    /// 增加一筆 unigram／bigram 觀察。
    pub(crate) fn observe(&mut self, left: &str, word: &str, weight: f64) {
        *self.bi.entry(pair(left, word)).or_default() += weight;
        *self.ctx.entry(left.to_string()).or_default() += weight;
        *self.uni.entry(word.to_string()).or_default() += weight;
        self.total += weight;
        *self.uses.entry(word.to_string()).or_default() += weight;
    }
}

/// 兩個字串組成 map 鍵。
pub fn pair(a: &str, b: &str) -> String {
    format!("{a}\t{b}")
}

/// 讀音鍵：音節以單一空格分隔。
pub fn reading_key(syls: &[Syllable]) -> String {
    syls.iter()
        .map(Syllable::as_str)
        .collect::<Vec<_>>()
        .join(" ")
}

/// 讀音鍵轉回音節序列。
pub fn reading_seq(reading: &str) -> Vec<Syllable> {
    parse_reading(reading)
}

/// 切割「左詞\t詞」鍵。
pub(crate) fn cut_pair(key: &str) -> (&str, &str) {
    match key.split_once('\t') {
        Some((left, right)) => (left, right),
        None => ("", key),
    }
}

/// 套用個人資料之後的詞庫視圖。
pub struct PersonalView<'a> {
    /// 個人化模型。
    pub model: &'a Model,
}

impl Model {
    /// 回傳套用個人資料之後的詞庫視圖。
    ///
    /// 個人讀音覆寫與自動修正停用目前只影響成本；前身的 `PRW=1` 由
    /// [`VOCAB_COST`] 與釘選候選取代。
    pub fn source(&self) -> PersonalView<'_> {
        PersonalView { model: self }
    }
}

impl PersonalView<'_> {
    /// 詞面是否存在（含個人片語與自訂詞）。
    pub fn has(&self, surface: &str) -> bool {
        self.model.store.phrases.contains_key(surface)
            || self.model.store.is_vocab(surface)
            || self.model.lexicon.has(surface)
    }
}

impl LexiconSource for PersonalView<'_> {
    fn exact(&self, syls: &[Syllable]) -> Vec<LexEntry> {
        let mut entries = self.model.lexicon.exact(syls);
        let reading = reading_key(syls);
        for (phrase, phrase_reading) in &self.model.store.phrases {
            if phrase_reading == &reading {
                entries.push(LexEntry::new(phrase, reading.clone(), VOCAB_COST, 1));
            }
        }
        for (text, readings) in self.model.store.vocab_entries() {
            if readings.iter().any(|item| item == &reading) {
                entries.push(LexEntry::new(text, reading.clone(), VOCAB_COST, 1));
            }
        }
        entries
    }

    fn prefixed(&self, syls: &[Syllable]) -> Vec<LexEntry> {
        let mut entries = self.model.lexicon.prefixed(syls);
        let reading = reading_key(syls);
        let is_prefix = |full: &str| full == reading || full.starts_with(&format!("{reading} "));
        for (phrase, phrase_reading) in &self.model.store.phrases {
            if is_prefix(phrase_reading) {
                entries.push(LexEntry::new(phrase, phrase_reading.clone(), VOCAB_COST, 1));
            }
        }
        for (text, readings) in self.model.store.vocab_entries() {
            for candidate in readings {
                if is_prefix(candidate) {
                    entries.push(LexEntry::new(text, candidate.clone(), VOCAB_COST, 1));
                }
            }
        }
        entries
    }

    fn has(&self, surface: &str) -> bool {
        self.model.store.phrases.contains_key(surface)
            || self.model.store.is_vocab(surface)
            || self.model.lexicon.has(surface)
    }

    fn lookup_surface(&self, surface: &str) -> Vec<LexEntry> {
        let mut entries = self.model.lexicon.lookup_surface(surface);
        if let Some(reading) = self.model.store.phrases.get(surface) {
            entries.push(LexEntry::new(surface, reading.clone(), VOCAB_COST, 1));
        }
        if let Some(readings) = self.model.store.vocab.get(surface) {
            for reading in readings {
                entries.push(LexEntry::new(surface, reading.clone(), VOCAB_COST, 1));
            }
        }
        entries
    }

    fn longest_word(&self) -> usize {
        let mut longest = self.model.lexicon.longest_word();
        for readings in self.model.store.vocab.values() {
            for reading in readings {
                longest = longest.max(reading.split_whitespace().count());
            }
        }
        longest
    }
}

/// 向量加入不重複的字串。
trait PushUnique {
    fn push_unique(&mut self, value: &str);
}

impl PushUnique for Vec<String> {
    fn push_unique(&mut self, value: &str) {
        if !self.iter().any(|item| item == value) {
            self.push(value.to_string());
        }
    }
}

/// 個人化模型：系統詞庫、語言模型與學習資料的合成。
pub struct Model {
    /// 個人資料。
    pub store: Store,
    /// 系統詞庫。
    pub lexicon: Box<dyn LexiconSource>,
    /// 系統語言模型。
    pub lm: Box<dyn SystemLm>,
    /// 學習參數。
    pub params: Params,
    /// 資料版本；每次學習遞增。
    pub gen: u64,
    /// 由宿主注入的時間來源；`None` 時凍結在 0，讓測試與重播可重現。
    pub now_source: Option<Box<dyn Fn() -> i64>>,
    /// 英文詞典。
    pub english_base: Option<Box<dyn EnglishLexicon>>,
    /// 個人資料的識別碼；重置後遞增，避免把反悔套到另一份資料。
    pub(crate) store_id: u64,
    /// 這個行程最後一次清理的 UTC 日。
    pub(crate) pruned_day: i64,
    /// 多樣性索引。
    pub(crate) diversity: diversity::DiversityIndex,
    /// 更正索引。
    pub(crate) correction_index: corrections::CorrectionIndex,
    /// 讀音習慣索引。
    pub(crate) habit: preference::HabitIndex,
    /// 英文索引。
    pub(crate) english_index: english::EnglishIndex,
    /// 分語言反悔紀錄。
    pub(crate) language_journal: Vec<language::LanguageUndo>,
}

impl Model {
    /// 建立模型。
    pub fn new(lexicon: Box<dyn LexiconSource>, lm: Box<dyn SystemLm>) -> Self {
        Self {
            store: Store::new(),
            lexicon,
            lm,
            params: Params::default(),
            gen: 0,
            now_source: None,
            english_base: None,
            store_id: 1,
            pruned_day: 0,
            diversity: diversity::DiversityIndex::default(),
            correction_index: corrections::CorrectionIndex::default(),
            habit: preference::HabitIndex::default(),
            english_index: english::EnglishIndex::default(),
            language_journal: Vec::new(),
        }
    }

    /// 目前時間；沒有注入時回傳 0。
    pub fn now(&self) -> i64 {
        self.now_source.as_ref().map_or(0, |now| now())
    }

    /// 同步時間；只前進，不因時鐘倒退增加權重。
    pub fn sync_time(&mut self) {
        let now = self.now();
        self.advance_time(now);
    }

    /// 清掉全部學習資料。
    pub fn reset_store(&mut self) {
        self.store = Store::new();
        self.store_id += 1;
        self.diversity = diversity::DiversityIndex::default();
        self.correction_index = corrections::CorrectionIndex::default();
        self.habit = preference::HabitIndex::default();
        self.english_index = english::EnglishIndex::default();
        self.language_journal.clear();
        self.gen += 1;
    }

    /// 個人化的系統機率 P(w | left)。
    pub fn prob(&self, left: &str, word: &str) -> f64 {
        let s = &self.store;
        let p_uni = (s.uni.get(word).copied().unwrap_or(0.0)
            + self.params.kappa_uni * self.sys_uni(word))
            / (s.total + self.params.kappa_uni);
        let p_sys = self.params.mu * self.lm.bi(left, word) + (1.0 - self.params.mu) * p_uni;
        let ctx = s.ctx.get(left).copied().unwrap_or(0.0);
        if ctx == 0.0 {
            return p_sys;
        }
        let kappa = self.kappa(left);
        (s.bi.get(&pair(left, word)).copied().unwrap_or(0.0) + kappa * p_sys) / (ctx + kappa)
    }

    /// 完全不含個人資料的系統機率。
    pub fn system_prob(&self, left: &str, word: &str) -> f64 {
        self.params.mu * self.lm.bi(left, word) + (1.0 - self.params.mu) * self.sys_uni(word)
    }

    /// 系統 bigram 的實際觀察值。
    pub fn system_bigram(&self, left: &str, word: &str) -> f64 {
        self.params.mu * self.lm.bi(left, word)
    }

    /// 前文是否有任何個人或系統上下文。
    pub fn has_context(&self, left: &str) -> bool {
        self.store.ctx.get(left).copied().unwrap_or(0.0) > 0.0 || self.lm.has_left(left)
    }

    /// 是否提交過 left 後面緊接 word。
    pub fn observed_pair(&self, left: &str, word: &str) -> bool {
        self.store.bi.get(&pair(left, word)).copied().unwrap_or(0.0) > 0.0
    }

    /// 系統先驗。
    pub fn prior(&self, word: &str) -> f64 {
        self.sys_uni(word)
    }

    /// 是否有個人調適。
    pub fn has_adaptation(&self) -> bool {
        !self.store.bi.is_empty() || !self.store.uni.is_empty()
    }

    /// 資料版本與時間。
    pub fn mass_version(&self) -> (u64, i64) {
        (self.gen, self.store.time)
    }

    /// 不含個人正計數的先驗質量。
    pub fn prior_mass(&self, left: &str, word: &str) -> f64 {
        let kappa = self.kappa(left);
        self.system_prob(left, word) * kappa
            / (self.store.ctx.get(left).copied().unwrap_or(0.0) + kappa)
    }

    /// 是否學過這個詞。
    pub fn is_learned(&self, word: &str) -> bool {
        self.store.is_learned(word)
    }

    /// 是否親手用這個讀音選過這個詞。
    pub fn chose_reading(&self, word: &str, syls: &[Syllable]) -> bool {
        self.store
            .readings
            .contains(&pair(word, &reading_key(syls)))
            || self.store.vocab_has_reading(word, &reading_key(syls))
    }

    /// 是否以這個音節手選過這個單字。
    pub fn picked_single(&self, syllable: &Syllable, word: &str) -> bool {
        self.store.is_picked(syllable.as_str(), word)
    }

    /// 系統 unigram；自訂詞套用字數下限。
    pub(crate) fn sys_uni(&self, word: &str) -> f64 {
        let p = self.lm.uni(word);
        if !self.store.is_vocab(word) {
            return p;
        }
        p.max(self.vocab_floor(word.chars().count()))
    }

    /// 自訂詞的 unigram 下限。
    pub(crate) fn vocab_floor(&self, len: usize) -> f64 {
        let Some(table) = self.lm.uni_quantile_by_len() else {
            return 1e-6;
        };
        for k in (1..=len.min(table.len().saturating_sub(1))).rev() {
            if table[k] > UNSEEN {
                return table[k];
            }
        }
        1e-6
    }

    /// 增加一筆觀察；供學習與測試使用。
    pub fn observe(&mut self, left: &str, word: &str, weight: f64) {
        let before = self.store.bi.get(&pair(left, word)).copied().unwrap_or(0.0);
        self.store.observe(left, word, weight);
        let after = self.store.bi.get(&pair(left, word)).copied().unwrap_or(0.0);
        self.change_bi(left, before, after);
    }

    /// 依學習規則更新個人資料；只在提交後呼叫。
    pub fn learn(&mut self, commit: &Commit) {
        self.gen += 1;
        let mut path: Vec<Seg> = commit
            .path
            .iter()
            .filter(|seg| !seg.fuzzy)
            .cloned()
            .collect();
        self.learn_corrections(commit);
        self.learn_pronouns(&path);

        let mut no_learn: Vec<(usize, usize)> = Vec::new();
        for rejected in &commit.rejected {
            if rejected.correction {
                let reading = if rejected.reading.len() == rejected.end - rejected.start {
                    rejected.reading.clone()
                } else {
                    commit.syls[rejected.start..rejected.end].to_vec()
                };
                if !reading.iter().any(Syllable::untoned) {
                    let key = pair(&rejected.word, &reading_key(&reading));
                    *self.store.rejects.entry(key).or_default() += 1;
                }
                no_learn.push((rejected.start, rejected.end));
            }
        }
        path.retain(|seg| {
            !no_learn
                .iter()
                .any(|(start, end)| *start < seg.end && seg.start < *end)
        });

        // 合併已晉升的個人片語。
        let mut merged: Vec<Seg> = Vec::new();
        for seg in path {
            if let Some(last) = merged.last_mut() {
                if last.end == seg.start
                    && self
                        .store
                        .phrases
                        .contains_key(&format!("{}{}", last.word, seg.word))
                {
                    let explicit = last.manual && !last.completion || seg.manual && !seg.completion;
                    last.word.push_str(&seg.word);
                    last.end = seg.end;
                    last.manual |= seg.manual;
                    last.completion = (last.completion || seg.completion) && !explicit;
                    continue;
                }
            }
            merged.push(seg);
        }
        path = merged.into_iter().filter(|seg| !seg.partial).collect();

        for seg in &commit.selected {
            if !seg.fuzzy
                && !no_learn
                    .iter()
                    .any(|(start, end)| *start < seg.end && seg.start < *end)
            {
                self.note_manual_seg(&commit.syls, seg);
            }
        }
        for index in 0..path.len() {
            let seg = path[index].clone();
            if seg.manual {
                self.note_manual_seg(&commit.syls, &seg);
            }
            if self.store.tombstones.contains(&seg.word) {
                continue;
            }
            let weight = if seg.completion {
                self.params.w_completion
            } else if seg.manual {
                self.params.w_manual
            } else {
                self.params.w_passive
            };
            let left = if index == 0 || path[index - 1].end != seg.start {
                BOS.to_string()
            } else {
                path[index - 1].word.clone()
            };
            self.store.observe(&left, &seg.word, weight);
            self.observe_evidence(&left, &seg.word, weight);
            if seg.manual && left != BOS {
                let length = left.chars().count() + seg.word.chars().count();
                if length <= self.params.phrase_max_len {
                    let reading = reading_key(&commit.syls[path[index - 1].start..seg.end]);
                    let previous = if index >= 2 && path[index - 2].end == path[index - 1].start {
                        path[index - 2].word.clone()
                    } else {
                        BOS.to_string()
                    };
                    self.see_phrase(&format!("{left}{}", seg.word), &reading, &previous, weight);
                }
            }
        }
        for index in 1..path.len() {
            if path[index - 1].end != path[index].start || path[index].manual {
                continue;
            }
            let phrase = format!("{}{}", path[index - 1].word, path[index].word);
            if self.store.phrase_seen.contains_key(&phrase) {
                let reading = reading_key(&commit.syls[path[index - 1].start..path[index].end]);
                self.see_phrase(&phrase, &reading, BOS, self.params.w_passive);
            }
        }
    }

    /// 記下明確手選。
    pub(crate) fn note_manual_seg(&mut self, syls: &[Syllable], seg: &Seg) {
        if syls[seg.start..seg.end].iter().any(Syllable::untoned) {
            return;
        }
        self.store.tombstones.remove(&seg.word);
        if seg.end - seg.start == 1 && !self.store.is_picked(syls[seg.start].as_str(), &seg.word) {
            self.store
                .picked
                .entry(syls[seg.start].as_str().to_string())
                .or_default()
                .push(seg.word.clone());
        }
        self.store
            .readings
            .insert(pair(&seg.word, &reading_key(&syls[seg.start..seg.end])));
    }

    /// 記下一筆語境證據。
    pub fn observe_evidence(&mut self, left: &str, word: &str, weight: f64) {
        let now = self.store.time;
        let local = self
            .store
            .evidence_by_context
            .get(&pair(left, word))
            .copied()
            .unwrap_or_default()
            .add(now, weight);
        self.store
            .evidence_by_context
            .insert(pair(left, word), local);
        let global = self
            .store
            .evidence_by_word
            .get(word)
            .copied()
            .unwrap_or_default()
            .add(now, weight);
        self.store.evidence_by_word.insert(word.to_string(), global);
    }

    /// 同一上下文的近期＋長期證據。
    pub fn evidence(&self, left: &str, word: &str) -> f64 {
        let now = self.store.time;
        let local = self
            .store
            .evidence_by_context
            .get(&pair(left, word))
            .copied()
            .unwrap_or_default()
            .at(now);
        let global = self
            .store
            .evidence_by_word
            .get(word)
            .copied()
            .unwrap_or_default()
            .at(now);
        local.short + local.long + (0.25 * (global.long - local.long).max(0.0)).min(2.0)
    }

    /// 字詞的使用證據。
    pub fn word_evidence(&self, word: &str) -> f64 {
        self.store
            .evidence_by_word
            .get(word)
            .copied()
            .unwrap_or_default()
            .at(self.store.time)
            .strength(self.store.time)
    }

    fn see_phrase(&mut self, phrase: &str, reading: &str, left: &str, weight: f64) {
        if self
            .store
            .tombstones
            .iter()
            .any(|tombstone| phrase.contains(tombstone))
        {
            return;
        }
        let now = self.store.time;
        let seen = self
            .store
            .phrase_seen
            .entry(phrase.to_string())
            .or_insert(0);
        *seen += 1;
        let seen_count = *seen;
        self.observe_evidence(left, phrase, weight);
        let promoted = self.store.phrases.contains_key(phrase);
        if !promoted
            && seen_count >= self.params.phrase_promote_at
            && !self.lexicon.has(phrase)
            && !self.store.is_vocab(phrase)
        {
            self.store
                .phrases
                .insert(phrase.to_string(), reading.to_string());
            self.store.phrase_order.push(phrase.to_string());
            let count = seen_count as f64;
            *self.store.uni.entry(phrase.to_string()).or_default() += count;
            self.store.total += count;
            *self.store.uses.entry(phrase.to_string()).or_default() += count;
        }
        let _ = now;
    }

    /// 時間前進；排序計數每小時至多整批衰減一次。
    pub fn advance_time(&mut self, now: i64) {
        if self.store.advance_time(now) {
            self.gen += 1;
        }
    }
}
