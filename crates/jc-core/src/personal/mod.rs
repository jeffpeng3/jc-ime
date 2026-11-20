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
pub use english::{
    english_token, EnglishCandidate, EnglishCommit, EnglishCorrection, EnglishEvidence, NoEnglish,
};
pub use evidence::UsageEvidence;
pub use language::{
    CrossLanguage, LanguageEvidence, LanguageId, LanguageNamespace, LanguageSelection,
};
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

/// rollback 窗口長度，毫秒。
///
/// 提交後的學習在這段時間內都還能補償；對照 Mozkey 的 rollback window
/// （`docs/reports/mozkey-study-2026-11-20.md` 第 5 節與 `accepted_rollback`）。
pub const ROLLBACK_WINDOW_MS: i64 = 5_000;

/// 連續跳過幾次後，這個學習詞不再自動排第一。
///
/// 對照 Mozkey 的弱負向訊號：跳過只降權，不刪除，選回來就自我修復。
pub const SKIP_LIMIT: u32 = 3;

/// 每跳過一次折抵的成本（毫成本）。
///
/// 門檻 [`SKIP_LIMIT`] 之前只做少量降權，達到門檻才整組排到後面。
pub const SKIP_PENALTY: i32 = 40;

/// 最短觸發長度：讀音不到這個字元數時，只記錄證據，不自動改變排序。
///
/// 對照 Mozkey 的 feedback 最短自動套用長度（7 字元）。
pub const MIN_TRIGGER_LEN: usize = 7;

/// 跳過計數表的容量上限。
const MAX_SKIPS: usize = 1024;

/// 每個語言命名空間、每張表的容量上限。
///
/// 日文學習與注音學習共用 [`Store`]，兩邊的語言資料都受這個上限約束。
pub const MAX_LANGUAGE_TABLE: usize = 1024;

/// 分語言學習的反悔步數。
const MAX_LANGUAGE_NOTE_UNDO: usize = 32;

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
    /// 打錯的英文拼法（小寫）→ 學到的修正。
    ///
    /// 只記使用者接受過的拼字修正；內建詞表仍然先查，這份表讓學過的修正
    /// 排到前面。連續跳過 [`SKIP_LIMIT`] 次後降權，手選一次就恢復。
    pub english_pairs: BTreeMap<String, EnglishCorrection>,
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
    /// 分語言學習的反悔紀錄；不寫進個人資料。
    #[serde(skip)]
    pub(crate) language_notes: Vec<LanguageNote>,
    /// 詞面 → 連續被跳過的次數（弱負向訊號）。
    ///
    /// 只記學過的詞；使用者再選它一次就清掉，選回來不刪除。
    pub skips: BTreeMap<String, u32>,
    /// 最近一次提交的學習快照，只在 rollback 窗口內存在，不寫進個人資料。
    #[serde(skip)]
    pub(crate) rollback: Option<Box<RollbackWindow>>,
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
            english_pairs: BTreeMap::new(),
            emoji_recent: Vec::new(),
            emoji_stats: HashMap::new(),
            palette_tab: String::new(),
            symbol_recent: Vec::new(),
            symbol_stats: HashMap::new(),
            languages: BTreeMap::new(),
            cross_language: CrossLanguage::default(),
            language_notes: Vec::new(),
            skips: BTreeMap::new(),
            rollback: None,
            vocab: BTreeMap::new(),
        }
    }
}

/// 一次提交的學習快照。
///
/// 只在記憶體中存在；[`Store::save`] 不寫入，重啟後沒有待確認的學習。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RollbackWindow {
    /// 學習前的整份資料。
    before: Box<Store>,
    /// 建立時刻，Unix 毫秒。
    at: i64,
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
        // 分語言學習（含日文）使用整數次數，套用同一個半衰期；至少保留 1 次，
        // 讓單筆學習不會因為一次衰減就消失，容量淘汰再處理長期不用的條目。
        for ns in self.languages.values_mut() {
            for entry in ns.surfaces.values_mut() {
                entry.count = decayed_count(entry.count, factor);
            }
            for entry in ns.readings.values_mut() {
                entry.count = decayed_count(entry.count, factor);
            }
            for entry in ns.contexts.values_mut() {
                entry.count = decayed_count(entry.count, factor);
            }
            ns.selections = ns.surfaces.values().map(|entry| entry.count).sum();
        }
        self.total *= factor;
        self.time = now;
        true
    }

    /// 清理不合法欄位並補齊舊格式缺少的索引。
    pub fn cleaned(mut self) -> Self {
        self.emoji_recent = crate::emoji::clean_recent(&self.emoji_recent);
        self.symbol_recent = crate::symbols::clean_recent(&self.symbol_recent);
        self.skips.retain(|_, count| *count > 0);
        trim_skips(&mut self.skips);
        self.rollback = None;
        if self.confirmed.is_empty() && self.format < 6 {
            self.seed_confirmed();
        }
        self.clean_english();
        self.clean_languages();
        self.cross_language.clean();
        self
    }

    /// 開始一個 rollback 窗口，先記下學習前的資料。
    ///
    /// 提交後立刻呼叫；窗口長度見 [`ROLLBACK_WINDOW_MS`]。重複呼叫會覆蓋上
    /// 一個窗口，因此記憶體最多只保留一份快照。
    pub fn begin_rollback(&mut self, at: i64) {
        let mut before = self.clone();
        before.rollback = None;
        self.rollback = Some(Box::new(RollbackWindow {
            before: Box::new(before),
            at,
        }));
    }

    /// 是否有等待確認的學習。
    pub fn has_pending_learning(&self) -> bool {
        self.rollback.is_some()
    }

    /// 窗口內撤銷最近一次學習；回傳是否真的還原。
    ///
    /// 超出窗口時只結束窗口，學習保留。還原是整份快照換回，連同那次提交的
    /// 跳過計數一起復原。
    pub fn rollback_learning(&mut self, now: i64) -> bool {
        let Some(pending) = self.rollback.take() else {
            return false;
        };
        if now.saturating_sub(pending.at) > ROLLBACK_WINDOW_MS {
            return false;
        }
        let mut before = *pending.before;
        before.rollback = None;
        *self = before;
        true
    }

    /// 結束窗口，保留這次學習。
    ///
    /// 下一次輸入不是撤銷鍵，或窗口到期時呼叫。
    pub fn accept_learning(&mut self) {
        self.rollback = None;
    }

    /// 記一次跳過；回傳目前連續跳過次數。
    pub fn note_skip(&mut self, word: &str) -> u32 {
        if word.is_empty() {
            return 0;
        }
        let count = self.skips.entry(word.to_string()).or_default();
        *count = count.saturating_add(1);
        let count = *count;
        trim_skips(&mut self.skips);
        count
    }

    /// 清掉跳過計數；回傳是否真的有紀錄。
    pub fn clear_skip(&mut self, word: &str) -> bool {
        self.skips.remove(word).is_some()
    }

    /// 連續跳過次數。
    pub fn skip_count(&self, word: &str) -> u32 {
        self.skips.get(word).copied().unwrap_or(0)
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
    ///
    /// 弱負向訊號在此生效：連續跳過 [`SKIP_LIMIT`] 次的詞排到一般候選之後，
    /// 但仍留在候選清單裡；不到門檻時每次跳過折抵 [`SKIP_PENALTY`] 毫成本。
    /// 明確手選的釘選候選優先於弱負向訊號。
    pub fn rank_key(&self, text: &str, reading: &str, cost: i32) -> (u8, i32) {
        let single = reading.split_whitespace().count() == 1;
        let pinned =
            self.vocab_has_reading(text, reading) || (single && self.is_picked(reading, text));
        let skips = self.skip_count(text).min(SKIP_LIMIT);
        let group = if pinned {
            0
        } else if skips >= SKIP_LIMIT {
            2
        } else {
            1
        };
        let lift = (self.uni.get(text).copied().unwrap_or(0.0) * 1000.0).round() as i32;
        let penalty = i32::try_from(skips).unwrap_or(0) * SKIP_PENALTY;
        let lift = (lift.min(LIFT_CAP) - penalty).max(0);
        (group, cost.saturating_sub(lift))
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
    ///
    /// 手選是明確訊號，會一併清掉這個詞的跳過計數。
    pub fn note_manual(&mut self, word: &str, reading: &str, syllables: usize) {
        self.tombstones.remove(word);
        self.skips.remove(word);
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

    /// 記下一次被動提交，但只留證據、不影響排序。
    ///
    /// 讀音短於 [`MIN_TRIGGER_LEN`] 時走這條：`evidence_by_word` 有紀錄，
    /// 但 `uni`／`bi`／`ctx` 不動，因此 [`Store::rank_key`] 的折抵維持 0。
    pub fn note_passive_evidence(&mut self, word: &str, weight: f64) {
        if self.tombstones.contains(word) || weight <= 0.0 {
            return;
        }
        let entry = self
            .evidence_by_word
            .get(word)
            .copied()
            .unwrap_or_default()
            .add(self.time, weight);
        self.evidence_by_word.insert(word.to_string(), entry);
    }

    /// 被動提交的證據權重。
    ///
    /// 對照 [`Params::w_passive`] 的預設值；外殼只有 [`Store`] 時用這個入口
    /// 取得同一個數字。
    pub fn passive_weight(&self) -> f64 {
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
        self.skips.remove(word);
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

/// 分語言學習的一次反悔紀錄。
///
/// [`Store::note_language_weighted`] 只會加權單一條目，因此反悔也只需要還原
/// 這一個詞面的次數、它所屬命名空間的計數與主要語言計數。
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LanguageNote {
    language: LanguageId,
    surface: String,
    surface_before: LanguageEvidence,
    surface_had: bool,
    reading: String,
    reading_before: LanguageEvidence,
    reading_had: bool,
    context: String,
    context_before: LanguageEvidence,
    context_had: bool,
    selections_before: u32,
    updated_before: i64,
    primary_before: u32,
    primary_had: bool,
    weight: u32,
}

/// 兩個字串組成 map 鍵。
pub fn pair(a: &str, b: &str) -> String {
    format!("{a}\t{b}")
}

/// 分語言資料的容量清理結果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LanguagePruneStats {
    /// 詞面表移除的筆數。
    pub surfaces: usize,
    /// 讀音表移除的筆數。
    pub readings: usize,
    /// 左文表移除的筆數。
    pub contexts: usize,
}

impl LanguagePruneStats {
    /// 合計移除筆數。
    pub fn total(&self) -> usize {
        self.surfaces + self.readings + self.contexts
    }
}

impl Store {
    /// 在指定語言記一次選字。
    ///
    /// 這是日文與注音共用的學習入口：`reading` 是空白分隔的讀音鍵，`left`
    /// 是呼叫端已正規化的左文，`surface` 是選定的詞面。`weight` 讓明確手選
    /// 比被動接受累積更多次數。時間 `at` 為 0 時沿用 store 目前時間。
    ///
    /// 記錄會推進該語言的主要語言計數，並把三張表裁到
    /// [`MAX_LANGUAGE_TABLE`]。同一次記錄可以反悔一次。
    pub fn note_language_weighted(
        &mut self,
        id: LanguageId,
        reading: &str,
        surface: &str,
        left: &str,
        weight: u32,
        at: i64,
    ) {
        if id == LanguageId::Und || surface.is_empty() || weight == 0 {
            return;
        }
        let at = if at > 0 { at } else { self.time };
        let reading_key = if reading.is_empty() {
            String::new()
        } else {
            pair(reading, surface)
        };
        let context_key = if left.is_empty() {
            String::new()
        } else {
            pair(left, surface)
        };

        let ns = self.languages.entry(id.as_str().to_string()).or_default();
        let note = LanguageNote {
            language: id,
            surface: surface.to_string(),
            surface_before: ns.surfaces.get(surface).copied().unwrap_or_default(),
            surface_had: ns.surfaces.contains_key(surface),
            reading: reading_key.clone(),
            reading_before: ns.readings.get(&reading_key).copied().unwrap_or_default(),
            reading_had: ns.readings.contains_key(&reading_key),
            context: context_key.clone(),
            context_before: ns.contexts.get(&context_key).copied().unwrap_or_default(),
            context_had: ns.contexts.contains_key(&context_key),
            selections_before: ns.selections,
            updated_before: ns.updated,
            primary_before: self
                .cross_language
                .primary
                .get(id.as_str())
                .copied()
                .unwrap_or(0),
            primary_had: self.cross_language.primary.contains_key(id.as_str()),
            weight,
        };

        let entry = ns.surfaces.get(surface).copied().unwrap_or_default();
        ns.surfaces
            .insert(surface.to_string(), add_weight(entry, weight, at));
        ns.selections = ns.selections.saturating_add(weight);
        ns.updated = ns.updated.max(at);
        if !reading_key.is_empty() {
            let entry = ns.readings.get(&reading_key).copied().unwrap_or_default();
            ns.readings
                .insert(reading_key, add_weight(entry, weight, at));
        }
        if !context_key.is_empty() {
            let entry = ns.contexts.get(&context_key).copied().unwrap_or_default();
            ns.contexts
                .insert(context_key, add_weight(entry, weight, at));
        }
        *self
            .cross_language
            .primary
            .entry(id.as_str().to_string())
            .or_default() += weight;
        self.trim_language_tables(id);

        self.language_notes.push(note);
        if self.language_notes.len() > MAX_LANGUAGE_NOTE_UNDO {
            let drop = self.language_notes.len() - MAX_LANGUAGE_NOTE_UNDO;
            self.language_notes.drain(..drop);
        }
    }

    /// 在指定語言記一次選字，權重 1。
    pub fn note_language(
        &mut self,
        id: LanguageId,
        reading: &str,
        surface: &str,
        left: &str,
        at: i64,
    ) {
        self.note_language_weighted(id, reading, surface, left, 1, at);
    }

    /// 在指定語言只記詞面證據，不改讀音與左文表。
    ///
    /// 讀音短於 [`MIN_TRIGGER_LEN`] 時使用：詞面次數照記，但日文重排讀的是
    /// 讀音與左文表，因此候選排序不變。這筆紀錄不進反悔 journal。
    pub fn note_language_surface(&mut self, id: LanguageId, surface: &str, weight: u32, at: i64) {
        if id == LanguageId::Und || surface.is_empty() || weight == 0 {
            return;
        }
        let at = if at > 0 { at } else { self.time };
        let ns = self.languages.entry(id.as_str().to_string()).or_default();
        let entry = ns.surfaces.get(surface).copied().unwrap_or_default();
        ns.surfaces
            .insert(surface.to_string(), add_weight(entry, weight, at));
        ns.selections = ns.selections.saturating_add(weight);
        ns.updated = ns.updated.max(at);
        *self
            .cross_language
            .primary
            .entry(id.as_str().to_string())
            .or_default() += weight;
        self.trim_language_tables(id);
    }

    /// 某語言選過這個詞面的次數。
    pub fn language_surface_count(&self, id: LanguageId, surface: &str) -> u32 {
        self.languages
            .get(id.as_str())
            .and_then(|ns| ns.surfaces.get(surface))
            .map_or(0, |entry| entry.count)
    }

    /// 某語言在「讀音＋詞面」上被選過的次數。
    pub fn language_reading_count(&self, id: LanguageId, reading: &str, surface: &str) -> u32 {
        self.languages
            .get(id.as_str())
            .and_then(|ns| ns.readings.get(&pair(reading, surface)))
            .map_or(0, |entry| entry.count)
    }

    /// 某語言在「左文＋詞面」上被選過的次數。
    pub fn language_context_count(&self, id: LanguageId, left: &str, surface: &str) -> u32 {
        self.languages
            .get(id.as_str())
            .and_then(|ns| ns.contexts.get(&pair(left, surface)))
            .map_or(0, |entry| entry.count)
    }

    /// 某語言的選取總次數。
    pub fn language_selections(&self, id: LanguageId) -> u32 {
        self.languages
            .get(id.as_str())
            .map_or(0, |ns| ns.selections)
    }

    /// 反悔最近一次 [`Store::note_language_weighted`]；回傳是否真的還原。
    ///
    /// 只還原那一筆加權；記錄時被容量上限淘汰的其他條目不會回來。
    pub fn undo_language_note(&mut self) -> bool {
        let Some(note) = self.language_notes.pop() else {
            return false;
        };
        let Some(ns) = self.languages.get_mut(note.language.as_str()) else {
            return false;
        };
        restore_weight(
            &mut ns.surfaces,
            &note.surface,
            note.surface_before,
            note.surface_had,
            note.weight,
        );
        if !note.reading.is_empty() {
            restore_weight(
                &mut ns.readings,
                &note.reading,
                note.reading_before,
                note.reading_had,
                note.weight,
            );
        }
        if !note.context.is_empty() {
            restore_weight(
                &mut ns.contexts,
                &note.context,
                note.context_before,
                note.context_had,
                note.weight,
            );
        }
        ns.selections = ns.selections.saturating_sub(note.weight);
        ns.updated = note.updated_before;
        if note.primary_had {
            self.cross_language
                .primary
                .insert(note.language.as_str().to_string(), note.primary_before);
        } else {
            self.cross_language.primary.remove(note.language.as_str());
        }
        if ns.surfaces.is_empty() && ns.selections == 0 {
            self.languages.remove(note.language.as_str());
        }
        true
    }

    /// 依 [`PrunePolicy`] 的容量上限清理每個語言命名空間。
    ///
    /// 上限取 `policy.max_word` 與 [`MAX_LANGUAGE_TABLE`] 的較小值；次數為 0
    /// 的條目一律移除，其餘先淘汰次數少、時間舊的條目。日文與注音共用同一份
    /// store，因此這裡的清理同時涵蓋兩邊。
    pub fn prune_languages(&mut self, policy: PrunePolicy) -> LanguagePruneStats {
        let limit = if policy.max_word == 0 {
            MAX_LANGUAGE_TABLE
        } else {
            policy.max_word.min(MAX_LANGUAGE_TABLE)
        };
        let mut stats = LanguagePruneStats::default();
        for ns in self.languages.values_mut() {
            stats.surfaces += trim_language_table(&mut ns.surfaces, limit);
            stats.readings += trim_language_table(&mut ns.readings, limit);
            stats.contexts += trim_language_table(&mut ns.contexts, limit);
            ns.selections = ns.surfaces.values().map(|entry| entry.count).sum();
        }
        self.languages
            .retain(|_, ns| !ns.surfaces.is_empty() || ns.selections > 0);
        stats
    }

    fn trim_language_tables(&mut self, id: LanguageId) {
        let Some(ns) = self.languages.get_mut(id.as_str()) else {
            return;
        };
        trim_language_table(&mut ns.surfaces, MAX_LANGUAGE_TABLE);
        trim_language_table(&mut ns.readings, MAX_LANGUAGE_TABLE);
        trim_language_table(&mut ns.contexts, MAX_LANGUAGE_TABLE);
    }
}

fn add_weight(entry: LanguageEvidence, weight: u32, at: i64) -> LanguageEvidence {
    LanguageEvidence {
        count: entry.count.saturating_add(weight),
        updated: entry.updated.max(at),
    }
}

/// 依時間因子衰減整數次數，至少保留 1。
fn decayed_count(count: u32, factor: f64) -> u32 {
    if count <= 1 {
        return count;
    }
    let scaled = (f64::from(count) * factor).round();
    if scaled <= 1.0 {
        1
    } else if scaled >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        scaled as u32
    }
}

fn trim_language_table(table: &mut BTreeMap<String, LanguageEvidence>, limit: usize) -> usize {
    let mut removed = 0;
    table.retain(|_, entry| {
        if entry.count == 0 {
            removed += 1;
            false
        } else {
            true
        }
    });
    if table.len() <= limit {
        return removed;
    }
    let mut keys: Vec<_> = table.keys().cloned().collect();
    keys.sort_by_key(|key| {
        let entry = table.get(key).copied().unwrap_or_default();
        (entry.count, entry.updated, key.clone())
    });
    let drop = table.len() - limit;
    for key in keys.into_iter().take(drop) {
        table.remove(&key);
        removed += 1;
    }
    removed
}

fn restore_weight(
    table: &mut BTreeMap<String, LanguageEvidence>,
    key: &str,
    before: LanguageEvidence,
    had: bool,
    weight: u32,
) {
    if had {
        table.insert(key.to_string(), before);
    } else if let Some(entry) = table.get_mut(key) {
        entry.count = entry.count.saturating_sub(weight);
        if entry.count == 0 {
            table.remove(key);
        }
    }
}

/// 讀音鍵：音節以單一空格分隔。
pub fn reading_key(syls: &[Syllable]) -> String {
    syls.iter()
        .map(Syllable::as_str)
        .collect::<Vec<_>>()
        .join(" ")
}

/// 讀音的字元數，不計分隔空白。
///
/// 最短觸發長度用這個值比較；注音一個音節是 1 到 3 個符號，日文一個單位
/// 是一個假名，兩邊都不含空白。
pub fn reading_len(reading: &str) -> usize {
    reading.chars().filter(|ch| !ch.is_whitespace()).count()
}

/// 跳過計數表超過上限時，先淘汰次數最少的詞。
fn trim_skips(skips: &mut BTreeMap<String, u32>) {
    if skips.len() <= MAX_SKIPS {
        return;
    }
    let mut keys: Vec<_> = skips.keys().cloned().collect();
    keys.sort_by_key(|key| (skips.get(key).copied().unwrap_or(0), key.clone()));
    let drop = skips.len() - MAX_SKIPS;
    for key in keys.into_iter().take(drop) {
        skips.remove(&key);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ranked(reading: &str, surfaces: &[(&str, i32)]) -> Vec<crate::Candidate> {
        surfaces
            .iter()
            .map(|(surface, cost)| {
                crate::Candidate::new(
                    *surface,
                    reading,
                    *cost,
                    crate::candidate::CandidateSource::Lexicon,
                )
            })
            .collect()
    }

    #[test]
    fn test_note_language_weighted_counts_and_undo() {
        let mut store = Store::new();
        store.note_language_weighted(LanguageId::Ja, "き ょ う", "京", "京", 2, 100);
        assert_eq!(store.language_surface_count(LanguageId::Ja, "京"), 2);
        assert_eq!(
            store.language_reading_count(LanguageId::Ja, "き ょ う", "京"),
            2
        );
        assert_eq!(store.language_context_count(LanguageId::Ja, "京", "京"), 2);
        assert_eq!(store.language_selections(LanguageId::Ja), 2);
        assert_eq!(store.cross_language.primary["ja"], 2);

        assert!(store.undo_language_note());
        assert_eq!(store.language_selections(LanguageId::Ja), 0);
        assert!(!store.languages.contains_key("ja"));
        assert!(!store.cross_language.primary.contains_key("ja"));
        assert!(!store.undo_language_note());
    }

    #[test]
    fn test_note_language_keeps_tables_bounded() {
        let mut store = Store::new();
        for index in 0..(MAX_LANGUAGE_TABLE + 20) {
            store.note_language(LanguageId::Ja, "あ", &format!("語{index}"), "", 1);
        }
        let ns = &store.languages["ja"];
        assert_eq!(ns.surfaces.len(), MAX_LANGUAGE_TABLE);
        assert_eq!(
            ns.selections,
            (MAX_LANGUAGE_TABLE + 20) as u32,
            "selections 是累計次數，不因容量淘汰而減少"
        );
        // 次數相同時淘汰鍵值最小的條目，最新的 1024 筆留下。
        assert!(!ns.surfaces.contains_key("語0"));
        assert!(ns
            .surfaces
            .contains_key(&format!("語{}", MAX_LANGUAGE_TABLE + 19)));
    }

    #[test]
    fn test_prune_languages_uses_policy_cap() {
        let mut store = Store::new();
        for index in 0..10 {
            store.note_language(LanguageId::Ja, "あ", &format!("語{index}"), "", 1);
        }
        let policy = PrunePolicy {
            max_word: 4,
            ..PrunePolicy::default()
        };
        let stats = store.prune_languages(policy);
        assert_eq!(stats.surfaces, 6);
        assert_eq!(store.languages["ja"].surfaces.len(), 4);
        // selections 依留下的詞面重算。
        assert_eq!(store.language_selections(LanguageId::Ja), 4);
    }

    #[test]
    fn test_language_counts_decay_but_keep_single_observation() {
        let mut store = Store::new();
        // 第一次推進只設定時間，不做衰減。
        assert!(!store.advance_time(0));
        store.note_language_weighted(LanguageId::Ja, "あ", "亜", "", 1, 0);
        store.note_language_weighted(LanguageId::Ja, "あ", "唖", "", 100, 0);
        assert!(store.advance_time(30 * DAY_SECONDS));
        // 單次紀錄保留 1；100 次約剩一半。
        assert_eq!(store.language_surface_count(LanguageId::Ja, "亜"), 1);
        let kept = store.language_surface_count(LanguageId::Ja, "唖");
        assert!((40..=60).contains(&kept), "30 日後約剩一半，得到 {kept}");
    }

    #[test]
    fn test_rollback_restores_learning_inside_window() {
        let mut store = Store::new();
        store.note_passive("景", "ㄐㄧㄥˇ");
        store.begin_rollback(5_000);
        store.note_passive("井", "ㄐㄧㄥˇ");
        assert!(store.has_pending_learning());
        assert!(store.is_learned("井"));

        // 邊界值仍算在窗口內。
        assert!(store.rollback_learning(5_000 + ROLLBACK_WINDOW_MS));
        assert!(!store.is_learned("井"), "窗口內撤銷要回到提交前");
        assert!(store.is_learned("景"), "更早的學習要保留");
        assert!(!store.has_pending_learning());
        assert!(!store.rollback_learning(5_001));
    }

    #[test]
    fn test_rollback_window_expires_and_keeps_learning() {
        let mut store = Store::new();
        store.begin_rollback(1_000);
        store.note_passive("井", "ㄐㄧㄥˇ");
        assert!(!store.rollback_learning(1_000 + ROLLBACK_WINDOW_MS + 1));
        assert!(store.is_learned("井"), "超出窗口要保留學習");
        assert!(!store.has_pending_learning());
    }

    #[test]
    fn test_skip_signal_penalises_without_deleting() {
        let mut store = Store::new();
        store.note_passive("井", "ㄐㄧㄥˇ");
        let mut list = ranked("ㄐㄧㄥˇ", &[("井", 1000), ("景", 1000)]);
        store.rank_candidates(&mut list, "ㄐㄧㄥˇ");
        assert_eq!(list[0].surface, "井");

        // 前兩次跳過只降權：折抵從 250 掉到 170。
        assert_eq!(store.note_skip("井"), 1);
        assert_eq!(store.note_skip("井"), 2);
        assert_eq!(store.rank_key("井", "ㄐㄧㄥˇ", 1000), (1, 830));

        // 第三次達到門檻：不再排第一，但仍在候選裡。
        assert_eq!(store.note_skip("井"), SKIP_LIMIT);
        let mut list = ranked("ㄐㄧㄥˇ", &[("井", 1000), ("景", 1000)]);
        store.rank_candidates(&mut list, "ㄐㄧㄥˇ");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].surface, "景");
        assert_eq!(list[1].surface, "井");
    }

    #[test]
    fn test_selecting_skipped_word_restores_priority() {
        let mut store = Store::new();
        store.note_passive("井", "ㄐㄧㄥˇ");
        for _ in 0..SKIP_LIMIT {
            store.note_skip("井");
        }
        assert_eq!(store.skip_count("井"), SKIP_LIMIT);

        store.note_manual("井", "ㄐㄧㄥˇ", 1);
        assert_eq!(store.skip_count("井"), 0, "選回來就自我修復");
        let mut list = ranked("ㄐㄧㄥˇ", &[("井", 1000), ("景", 1000)]);
        store.rank_candidates(&mut list, "ㄐㄧㄥˇ");
        assert_eq!(list[0].surface, "井");
    }

    #[test]
    fn test_short_reading_records_evidence_only() {
        let mut store = Store::new();
        // 讀音 4 個注音符號，不到 7 字元。
        assert!(reading_len("ㄐㄧㄥˇ") < MIN_TRIGGER_LEN);
        store.note_passive_evidence("井", store.passive_weight());
        assert_eq!(store.evidence_by_word["井"].short, 0.25, "證據要留");
        assert!(!store.is_learned("井"));
        assert_eq!(
            store.rank_key("井", "ㄐㄧㄥˇ", 1000),
            (1, 1000),
            "證據-only 不改排序"
        );

        // 對照：一般被動學習會折抵成本。
        store.note_passive("井", "ㄐㄧㄥˇ");
        assert_eq!(store.rank_key("井", "ㄐㄧㄥˇ", 1000), (1, 750));
    }

    #[test]
    fn test_short_reading_language_evidence_only() {
        let mut store = Store::new();
        store.note_language_surface(LanguageId::Ja, "京", 1, 10);
        assert_eq!(store.language_surface_count(LanguageId::Ja, "京"), 1);
        assert_eq!(store.language_selections(LanguageId::Ja), 1);
        assert_eq!(
            store.language_reading_count(LanguageId::Ja, "き ょ う", "京"),
            0,
            "證據不進讀音表，日文重排因此不變"
        );
        assert_eq!(reading_len("き ょ う"), 3);
        assert!(reading_len("きょうははれです") >= MIN_TRIGGER_LEN);
    }
}
