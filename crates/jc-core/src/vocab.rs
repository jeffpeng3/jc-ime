//! 使用者親手加入的詞彙（自訂詞）與「我的片語」。
//!
//! 對照前身 Go `core/vocab`。自訂詞和自動學到的個人資料分開：存在 `vocab.json`，
//! 不衰減、不被容量逐出，只有使用者（或使用者授權的工具）明確增刪。每次修改
//! 記成一個批次，最近 [`KEEP_BATCHES`] 批可以整批復原。
//!
//! 這個 module 是純邏輯：時間由呼叫端傳入，讀寫檔案由宿主負責。

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};

use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

use crate::zh::han::is_han;
use crate::zh::syllable::{key as syllables_key, parse_reading, Syllable};

/// `vocab.json` 的格式版本，與 `user.json` 的個人資料格式各自獨立。
///
/// 英文詞沒有加一：它和漢字詞一樣存在 `words`，只是詞面是英文、`readings`
/// 是空的。不認得英文詞的版本把它當成「手動改壞」的詞：不使用、原樣保留到
/// 下次存檔，其餘詞照常讀寫。
pub const FORMAT: i32 = 1;

/// 每個詞 1～32 字，讀音音節數相同。
pub const MAX_WORD_RUNES: usize = 32;
/// 英文詞 1～32 個字元（ASCII 字母、數字與 `-` `_` `.`）。
pub const MAX_ENGLISH_LEN: usize = 32;
/// 自訂詞總數。
pub const MAX_WORDS: usize = 10_000;
/// 同一個詞的讀音數。
pub const MAX_READINGS: usize = 4;
/// 「我的片語」數量。
pub const MAX_PHRASES: usize = 200;
/// 每則片語長度。
pub const MAX_PHRASE_RUNES: usize = 1_000;
/// 單次修改的筆數。
pub const MAX_BATCH_ITEMS: usize = 500;
/// 可整批復原的最近批次。
pub const KEEP_BATCHES: usize = 50;

/// 設定程式「我的詞彙」或命令列。
pub const SOURCE_MANUAL: &str = "manual";
/// 組字中選範圍加入。
pub const SOURCE_COMPOSE: &str = "compose";
/// 本機 API／MCP（使用者用的 AI 工具）。
pub const SOURCE_ASSISTANT: &str = "assistant";
/// 自動學到的片語轉成自訂詞。
pub const SOURCE_LEARNED: &str = "learned";

/// 空字串、含非漢字（也不是英文詞）、超過長度上限。
pub const CODE_INVALID_WORD: &str = "invalid_word";
/// 音節不存在、音節數與字數不同。
pub const CODE_INVALID_READING: &str = "invalid_reading";
/// 自訂詞數量已達上限。
pub const CODE_LIMIT_WORDS: &str = "limit_words";
/// 同一個詞的讀音數已達上限。
pub const CODE_LIMIT_READINGS: &str = "limit_readings";
/// 片語數量已達上限。
pub const CODE_LIMIT_PHRASES: &str = "limit_phrases";
/// 單次修改筆數已達上限。
pub const CODE_LIMIT_BATCH: &str = "limit_batch";
/// 檔案已超過上限（手動編輯）：不再新增。
pub const CODE_OVER_LIMIT: &str = "over_limit";
/// 找不到指定的詞、片語或批次。
pub const CODE_NOT_FOUND: &str = "not_found";
/// 復原時資料已被後來的修改改過。
pub const CODE_CONFLICT: &str = "conflict";
/// 沒有要修改的項目。
pub const CODE_EMPTY: &str = "empty";

/// 修改被拒絕的原因。整批拒絕，不做部分寫入。
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize)]
#[error("{msg}")]
pub struct Error {
    /// 錯誤代碼，供 API 與設定程式判斷。
    pub code: String,
    /// 批次中第幾筆（從 0 起）；與單筆無關時為 -1。
    pub item: i32,
    /// 相關的詞或片語內容。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// 給使用者看的訊息。
    #[serde(rename = "message")]
    pub msg: String,
    /// 相關的上限。
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub limit: i32,
    /// 還能加幾筆。
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub left: i32,
}

fn is_zero_i32(value: &i32) -> bool {
    *value == 0
}

fn is_zero_i64(value: &i64) -> bool {
    *value == 0
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

fn fail(code: &str, item: i32, text: &str, msg: impl Into<String>) -> Error {
    Error {
        code: code.to_string(),
        item,
        text: text.to_string(),
        msg: msg.into(),
        limit: 0,
        left: 0,
    }
}

/// 檔案由較新版本寫成；呼叫端不得以目前的結構寫回。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("自訂詞彙格式 {format} 比本版支援的 {FORMAT} 新")]
pub struct NewerFormatError {
    /// 檔案宣告的格式版本。
    pub format: i32,
}

/// 載入 `vocab.json` 的錯誤。
#[derive(Debug, Error)]
pub enum LoadError {
    /// 檔案格式比本版新。
    #[error(transparent)]
    NewerFormat(#[from] NewerFormatError),
    /// JSON 格式錯誤。
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// 一個自訂詞。
///
/// `readings` 是以空白分隔音節的讀音，依加入順序。英文詞沒有讀音，
/// `readings` 是空的。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Word {
    /// 詞面。
    #[serde(rename = "word")]
    pub text: String,
    /// 讀音；英文詞為空。
    #[serde(default, deserialize_with = "deserialize_readings")]
    pub readings: Vec<String>,
    /// 來源。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// Unix 秒。
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub added: i64,
    /// 最後修改它的批次。
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub batch: u64,
}

impl Word {
    /// 建立一個詞。
    pub fn new(text: impl Into<String>, readings: Vec<String>) -> Self {
        Self {
            text: text.into(),
            readings,
            source: String::new(),
            added: 0,
            batch: 0,
        }
    }

    /// 回報這是英文詞（沒有讀音、混打時照字面送出）。
    pub fn english(&self) -> bool {
        is_english(&self.text)
    }
}

fn deserialize_readings<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<Vec<String>>::deserialize(deserializer)?.unwrap_or_default())
}

/// 回報 `text` 是否為英文詞的詞面。
///
/// 1～[`MAX_ENGLISH_LEN`] 個 ASCII 字母、數字與 `-` `_` `.`，至少有一個字母。
/// 大小寫照字面：`ls` 和 `LS` 是兩個不同的詞。
pub fn is_english(text: &str) -> bool {
    if text.is_empty() || text.len() > MAX_ENGLISH_LEN {
        return false;
    }
    let mut letter = false;
    for ch in text.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' => letter = true,
            '0'..='9' | '-' | '_' | '.' => {}
            _ => return false,
        }
    }
    letter
}

/// 只有 ASCII 可見字元，看起來是想加英文詞（給錯誤訊息用）。
fn english_like(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|ch| ch > ' ' && ch <= '~')
}

/// 「我的片語」的一則：沒有讀音、不參與選字，從 `=` 選單選取。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phrase {
    /// 片語編號。
    pub id: u64,
    /// 片語內容。
    #[serde(default)]
    pub text: String,
    /// Unix 秒。
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub added: i64,
    /// 最後修改它的批次。
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub batch: u64,
}

/// 批次中的一筆變更。
///
/// `before` 為 `None` 表示新增，`after` 為 `None` 表示刪除；片語用
/// `phrase_before`／`phrase_after`，規則相同。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    /// 變更前的詞。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Word>,
    /// 變更後的詞。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Word>,
    /// 變更前的片語。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phrase_before: Option<Phrase>,
    /// 變更後的片語。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phrase_after: Option<Phrase>,
}

/// 一次修改。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Batch {
    /// 批次編號。
    pub id: u64,
    /// Unix 秒。
    pub time: i64,
    /// 來源。
    #[serde(default)]
    pub source: String,
    /// `add`、`remove`、`readings`、`undo`、`phrase-add`、`phrase-remove`、
    /// `phrase-move`。
    pub op: String,
    /// 復原的是哪一批。
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub undo_of: u64,
    /// 已被後來的批次復原。
    #[serde(default, skip_serializing_if = "is_false")]
    pub undone: bool,
    /// 這批的變更。
    #[serde(default)]
    pub changes: Vec<Change>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct FileData {
    #[serde(default)]
    format: i32,
    #[serde(default)]
    version: u64,
    #[serde(default)]
    words: Vec<Word>,
    #[serde(default)]
    phrases: Vec<Phrase>,
    #[serde(default)]
    batches: Vec<Batch>,
}

/// 讀音前綴樹的一個節點。
///
/// 自訂詞最長 32 字，比一般系統詞的 8 個音節長得多，所以另建這棵樹，
/// 只有前綴相符時才往後找。
#[derive(Debug, Clone, Default)]
struct Node {
    children: HashMap<Syllable, Node>,
    /// 讀音恰好走到這裡的詞（檔案位置），依檔案順序。
    words: Vec<usize>,
    /// 子樹中的讀音數；`has_prefix` 用。
    size: usize,
}

/// 一個讀音命中的自訂詞。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// 詞面。
    pub text: String,
    /// 完整讀音。
    pub reading: Vec<Syllable>,
}

/// 載入記憶體的自訂詞彙與讀音索引。
///
/// 零值不可用，請用 [`Set::new`] 或 [`Set::load`]。不是並行安全的：由引擎在
/// 自己的鎖內使用。
#[derive(Debug, Clone)]
pub struct Set {
    file: FileData,
    index: Node,
    /// 詞 → `file.words` 的位置（只含有效、在上限內的詞，含英文詞）。
    by_text: HashMap<String, usize>,
    /// 有效的英文詞（沒有讀音，不進讀音前綴樹）。
    english: HashSet<String>,
    problems: Vec<String>,
    /// 有效詞的最長音節數。
    longest: usize,
    /// 同 `english`，依檔案順序（`english_prefix`）。
    english_words: Vec<String>,
}

impl Set {
    /// 回傳空的詞彙。
    pub fn new() -> Self {
        let mut set = Set {
            file: FileData {
                format: FORMAT,
                ..FileData::default()
            },
            index: Node::default(),
            by_text: HashMap::new(),
            english: HashSet::new(),
            problems: Vec::new(),
            longest: 0,
            english_words: Vec::new(),
        };
        set.reindex();
        set
    }

    /// 讀回 `vocab.json`。
    ///
    /// 較新版本寫成的檔案回傳 [`LoadError::NewerFormat`]。內容有問題（手動
    /// 改壞、超過上限）時照樣載入：只使用上限內、有效的詞，其餘原樣保留到
    /// 下次存檔。
    pub fn load(reader: impl Read) -> Result<Self, LoadError> {
        let mut file: FileData = serde_json::from_reader(reader)?;
        if file.format > FORMAT {
            return Err(NewerFormatError {
                format: file.format,
            }
            .into());
        }
        file.format = FORMAT;
        let mut set = Set {
            file,
            index: Node::default(),
            by_text: HashMap::new(),
            english: HashSet::new(),
            problems: Vec::new(),
            longest: 0,
            english_words: Vec::new(),
        };
        set.reindex();
        Ok(set)
    }

    /// 寫出整份詞彙。
    pub fn save(&self, writer: impl Write) -> Result<(), serde_json::Error> {
        let mut writer = writer;
        serde_json::to_writer_pretty(&mut writer, &self.file)?;
        writer.write_all(b"\n").map_err(serde_json::Error::io)
    }

    /// 在每次修改時加一，給管理頁判斷是否需要重新整理。
    pub fn version(&self) -> u64 {
        self.file.version
    }

    /// 回傳有效、在上限內的詞數。
    pub fn len(&self) -> usize {
        self.by_text.len()
    }

    /// 是否沒有任何有效的詞。
    pub fn is_empty(&self) -> bool {
        self.by_text.is_empty()
    }

    /// 回傳檔案中的詞數（含無效或超過上限而沒有使用的）。
    pub fn stored(&self) -> usize {
        self.file.words.len()
    }

    /// 說明載入時沒有使用的內容，給記錄與管理頁說明。
    pub fn problems(&self) -> Vec<String> {
        self.problems.clone()
    }

    /// 回傳有效詞的最長音節數。
    pub fn longest(&self) -> usize {
        self.longest
    }

    /// 依檔案順序回傳有效的詞（複本）。
    pub fn words(&self) -> Vec<Word> {
        let mut out = Vec::with_capacity(self.by_text.len());
        for (index, word) in self.file.words.iter().enumerate() {
            if self.by_text.get(&word.text) == Some(&index) {
                out.push(word.clone());
            }
        }
        out
    }

    /// 回傳某個詞。
    pub fn lookup(&self, text: &str) -> Option<Word> {
        self.by_text
            .get(text)
            .map(|index| self.file.words[*index].clone())
    }

    /// 回報 `text` 是否為讀音 key 的自訂詞。
    pub fn has_reading(&self, text: &str, key: &str) -> bool {
        self.by_text
            .get(text)
            .is_some_and(|index| self.file.words[*index].readings.iter().any(|r| r == key))
    }

    /// 依使用者排的順序回傳「我的片語」。
    pub fn phrases(&self) -> Vec<Phrase> {
        self.file.phrases.clone()
    }

    /// 回傳保留的批次，最新的在前。
    pub fn batches(&self) -> Vec<Batch> {
        let mut out = self.file.batches.clone();
        out.reverse();
        out
    }

    /// 回傳指定的批次。
    pub fn batch(&self, id: u64) -> Option<Batch> {
        self.file.batches.iter().find(|b| b.id == id).cloned()
    }

    /// 加入一批詞。
    ///
    /// 已經在詞彙裡的詞合併讀音；整批有任何一筆不合格就全部不加，回傳
    /// [`Error`]。沒有任何實際變更（全部都已存在）時回傳 `Ok(None)`。
    pub fn add(
        &mut self,
        entries: &[Entry],
        source: &str,
        now: i64,
    ) -> Result<Option<Batch>, Error> {
        self.check_batch(entries.len())?;
        let mut batch = Batch {
            id: 0,
            time: now,
            source: source.to_string(),
            op: "add".to_string(),
            undo_of: 0,
            undone: false,
            changes: Vec::new(),
        };
        let mut pending: HashMap<String, Word> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        for (item, entry) in entries.iter().enumerate() {
            let word = normalize(&entry.text, &entry.readings, item as i32)?;
            if !pending.contains_key(&word.text) {
                let current = self.lookup(&word.text).unwrap_or_else(|| Word {
                    text: word.text.clone(),
                    readings: Vec::new(),
                    source: source.to_string(),
                    added: now,
                    batch: 0,
                });
                order.push(word.text.clone());
                pending.insert(word.text.clone(), current);
            }
            let current = pending.get_mut(&word.text).expect("剛插入");
            for reading in &word.readings {
                if !current.readings.iter().any(|r| r == reading) {
                    current.readings.push(reading.clone());
                }
            }
            if current.readings.len() > MAX_READINGS {
                let mut err = fail(
                    CODE_LIMIT_READINGS,
                    item as i32,
                    &word.text,
                    format!(
                        "「{}」加上原有讀音共 {} 個，同一個詞最多 {} 個",
                        word.text,
                        current.readings.len(),
                        MAX_READINGS
                    ),
                );
                err.limit = MAX_READINGS as i32;
                return Err(err);
            }
        }
        let mut added = 0usize;
        for text in &order {
            let word = pending.get(text).expect("批次中的詞");
            let old = self.lookup(text);
            if old
                .as_ref()
                .is_some_and(|old| old.readings == word.readings)
            {
                continue;
            }
            let after = word.clone();
            let mut change = Change {
                after: Some(after),
                ..Change::default()
            };
            if let Some(old) = old {
                change.before = Some(old);
            } else {
                added += 1;
            }
            batch.changes.push(change);
        }
        if added > 0 {
            self.check_room(added)?;
        }
        if batch.changes.is_empty() {
            return Ok(None);
        }
        Ok(Some(self.commit(batch)))
    }

    /// 刪除一批詞。
    ///
    /// 不存在的詞整批拒絕（API 與管理頁應該只刪看得到的詞）。
    pub fn remove(&mut self, texts: &[String], source: &str, now: i64) -> Result<Batch, Error> {
        self.check_batch(texts.len())?;
        let mut batch = Batch {
            id: 0,
            time: now,
            source: source.to_string(),
            op: "remove".to_string(),
            undo_of: 0,
            undone: false,
            changes: Vec::new(),
        };
        let mut seen: HashSet<String> = HashSet::new();
        for (item, raw) in texts.iter().enumerate() {
            let text = raw.trim();
            let old = self.lookup(text).ok_or_else(|| {
                fail(
                    CODE_NOT_FOUND,
                    item as i32,
                    text,
                    format!("「{text}」不在自訂詞彙裡"),
                )
            })?;
            if !seen.insert(text.to_string()) {
                continue;
            }
            batch.changes.push(Change {
                before: Some(old),
                ..Change::default()
            });
        }
        Ok(self.commit(batch))
    }

    /// 改寫一個詞的全部讀音（管理頁點選破音字、刪掉錯的讀音）。
    pub fn set_readings(
        &mut self,
        text: &str,
        readings: &[String],
        source: &str,
        now: i64,
    ) -> Result<Option<Batch>, Error> {
        let text = text.trim();
        let old = self
            .lookup(text)
            .ok_or_else(|| fail(CODE_NOT_FOUND, 0, text, format!("「{text}」不在自訂詞彙裡")))?;
        let word = normalize(&old.text, readings, 0)?;
        if word.readings == old.readings {
            return Ok(None);
        }
        let mut after = old.clone();
        after.readings = word.readings;
        let batch = Batch {
            id: 0,
            time: now,
            source: source.to_string(),
            op: "readings".to_string(),
            undo_of: 0,
            undone: false,
            changes: vec![Change {
                before: Some(old),
                after: Some(after),
                ..Change::default()
            }],
        };
        Ok(Some(self.commit(batch)))
    }

    /// 在「我的片語」最後面加入幾則。
    pub fn add_phrases(
        &mut self,
        texts: &[String],
        source: &str,
        now: i64,
    ) -> Result<Batch, Error> {
        self.check_batch(texts.len())?;
        if self.file.phrases.len() + texts.len() > MAX_PHRASES {
            let mut err = fail(
                CODE_LIMIT_PHRASES,
                -1,
                "",
                format!(
                    "「我的片語」最多 {} 則，目前 {} 則，這次要加 {} 則",
                    MAX_PHRASES,
                    self.file.phrases.len(),
                    texts.len()
                ),
            );
            err.limit = MAX_PHRASES as i32;
            err.left = MAX_PHRASES.saturating_sub(self.file.phrases.len()) as i32;
            return Err(err);
        }
        let mut next = 1u64;
        for phrase in &self.file.phrases {
            next = next.max(phrase.id + 1);
        }
        // 刪掉的片語編號也不重用，復原時才不會撞號。
        for batch in &self.file.batches {
            for change in &batch.changes {
                if let Some(phrase) = &change.phrase_before {
                    next = next.max(phrase.id + 1);
                }
            }
        }
        let mut batch = Batch {
            id: 0,
            time: now,
            source: source.to_string(),
            op: "phrase-add".to_string(),
            undo_of: 0,
            undone: false,
            changes: Vec::new(),
        };
        for (item, text) in texts.iter().enumerate() {
            let n = text.chars().count();
            if text.trim().is_empty() {
                return Err(fail(
                    CODE_EMPTY,
                    item as i32,
                    text,
                    format!("第 {} 則片語是空的", item + 1),
                ));
            }
            if n > MAX_PHRASE_RUNES {
                let mut err = fail(
                    CODE_INVALID_WORD,
                    item as i32,
                    "",
                    format!(
                        "第 {} 則片語有 {n} 個字，最多 {MAX_PHRASE_RUNES} 字",
                        item + 1
                    ),
                );
                err.limit = MAX_PHRASE_RUNES as i32;
                return Err(err);
            }
            let phrase = Phrase {
                id: next,
                text: text.clone(),
                added: now,
                batch: 0,
            };
            next += 1;
            batch.changes.push(Change {
                phrase_after: Some(phrase),
                ..Change::default()
            });
        }
        Ok(self.commit(batch))
    }

    /// 刪除幾則片語。
    pub fn remove_phrases(&mut self, ids: &[u64], source: &str, now: i64) -> Result<Batch, Error> {
        self.check_batch(ids.len())?;
        let mut batch = Batch {
            id: 0,
            time: now,
            source: source.to_string(),
            op: "phrase-remove".to_string(),
            undo_of: 0,
            undone: false,
            changes: Vec::new(),
        };
        for (item, id) in ids.iter().enumerate() {
            let phrase = self
                .file
                .phrases
                .iter()
                .find(|p| p.id == *id)
                .ok_or_else(|| {
                    fail(
                        CODE_NOT_FOUND,
                        item as i32,
                        "",
                        format!("沒有編號 {id} 的片語"),
                    )
                })?;
            batch.changes.push(Change {
                phrase_before: Some(phrase.clone()),
                ..Change::default()
            });
        }
        Ok(self.commit(batch))
    }

    /// 把一則片語移到第 `to` 個位置（從 0 起）。
    pub fn move_phrase(
        &mut self,
        id: u64,
        to: i32,
        source: &str,
        now: i64,
    ) -> Result<Option<Batch>, Error> {
        let Some(from) = self.file.phrases.iter().position(|p| p.id == id) else {
            return Err(fail(CODE_NOT_FOUND, 0, "", format!("沒有編號 {id} 的片語")));
        };
        let last = self.file.phrases.len() - 1;
        let to = to.clamp(0, last as i32) as usize;
        if to == from {
            return Ok(None);
        }
        let phrase = self.file.phrases.remove(from);
        self.file.phrases.insert(to, phrase);
        // 移動不記變更明細：復原只把順序換回來太複雜，所以移動不可整批復原。
        Ok(Some(self.commit(Batch {
            id: 0,
            time: now,
            source: source.to_string(),
            op: "phrase-move".to_string(),
            undo_of: 0,
            undone: false,
            changes: Vec::new(),
        })))
    }

    /// 復原一個批次：以一個新的批次把它的每筆變更反過來做。
    ///
    /// 那些詞後來又被改過（目前內容和批次的結果不同）時整批拒絕，回傳
    /// [`CODE_CONFLICT`]。
    pub fn undo(&mut self, id: u64, source: &str, now: i64) -> Result<Batch, Error> {
        let Some(index) = self.file.batches.iter().position(|b| b.id == id) else {
            return Err(fail(
                CODE_NOT_FOUND,
                -1,
                "",
                format!("找不到批次 {id}（只保留最近 {KEEP_BATCHES} 批）"),
            ));
        };
        let original = self.file.batches[index].clone();
        if original.undone {
            return Err(fail(CODE_CONFLICT, -1, "", format!("批次 {id} 已經復原過")));
        }
        if original.op == "phrase-move" {
            return Err(fail(CODE_CONFLICT, -1, "", "片語排序不能整批復原"));
        }
        let mut batch = Batch {
            id: 0,
            time: now,
            source: source.to_string(),
            op: "undo".to_string(),
            undo_of: id,
            undone: false,
            changes: Vec::new(),
        };
        let mut added = 0usize;
        for item in (0..original.changes.len()).rev() {
            let change = &original.changes[item];
            if change.before.is_some() || change.after.is_some() {
                let text = change
                    .before
                    .as_ref()
                    .or(change.after.as_ref())
                    .expect("至少一邊有詞")
                    .text
                    .clone();
                let current = self.lookup(&text);
                let conflict = match &change.after {
                    None => current.is_some(),
                    Some(after) => current
                        .as_ref()
                        .is_none_or(|current| current.readings != after.readings),
                };
                if conflict {
                    return Err(fail(
                        CODE_CONFLICT,
                        item as i32,
                        &text,
                        format!("「{text}」在這批之後又改過，不能整批復原"),
                    ));
                }
                let Some(after) = change.before.clone() else {
                    let current = current.expect("前面已確認存在");
                    batch.changes.push(Change {
                        before: Some(current),
                        ..Change::default()
                    });
                    continue;
                };
                let mut next = Change {
                    after: Some(after),
                    ..Change::default()
                };
                if let Some(current) = current {
                    next.before = Some(current);
                } else {
                    added += 1;
                }
                batch.changes.push(next);
            } else {
                let pid = change
                    .phrase_before
                    .as_ref()
                    .or(change.phrase_after.as_ref())
                    .expect("至少一邊有片語")
                    .id;
                let at = self.file.phrases.iter().position(|p| p.id == pid);
                let conflict = match &change.phrase_after {
                    None => at.is_some(),
                    Some(_) => at.is_none(),
                };
                if conflict {
                    return Err(fail(
                        CODE_CONFLICT,
                        item as i32,
                        "",
                        format!("片語 {pid} 在這批之後又改過，不能整批復原"),
                    ));
                }
                if change.phrase_after.is_some() {
                    let phrase = self.file.phrases[at.expect("存在")].clone();
                    batch.changes.push(Change {
                        phrase_before: Some(phrase),
                        ..Change::default()
                    });
                } else {
                    let phrase = change.phrase_before.clone().expect("有片語");
                    batch.changes.push(Change {
                        phrase_after: Some(phrase),
                        ..Change::default()
                    });
                }
            }
        }
        if added > 0 {
            self.check_room(added)?;
        }
        self.file.batches[index].undone = true;
        Ok(self.commit(batch))
    }

    fn check_batch(&self, n: usize) -> Result<(), Error> {
        if n == 0 {
            return Err(fail(CODE_EMPTY, -1, "", "沒有要修改的項目"));
        }
        if n > MAX_BATCH_ITEMS {
            let mut err = fail(
                CODE_LIMIT_BATCH,
                -1,
                "",
                format!("一次最多修改 {MAX_BATCH_ITEMS} 筆，這次有 {n} 筆；請分批進行"),
            );
            err.limit = MAX_BATCH_ITEMS as i32;
            return Err(err);
        }
        Ok(())
    }

    fn check_room(&self, added: usize) -> Result<(), Error> {
        if self.file.words.len() > MAX_WORDS {
            let mut err = fail(
                CODE_OVER_LIMIT,
                -1,
                "",
                format!(
                    "自訂詞彙檔有 {} 個詞，超過上限 {MAX_WORDS}，在刪到上限以內之前不能新增",
                    self.file.words.len()
                ),
            );
            err.limit = MAX_WORDS as i32;
            return Err(err);
        }
        if self.file.words.len() + added > MAX_WORDS {
            let left = MAX_WORDS - self.file.words.len();
            let mut err = fail(
                CODE_LIMIT_WORDS,
                -1,
                "",
                format!(
                    "自訂詞最多 {MAX_WORDS} 個，目前 {} 個，這次要新增 {added} 個，只剩 {left} 個空位",
                    self.file.words.len()
                ),
            );
            err.limit = MAX_WORDS as i32;
            err.left = left as i32;
            return Err(err);
        }
        Ok(())
    }

    /// 套用批次的變更、編號、記錄並重建索引。呼叫前已確認變更都合法。
    fn commit(&mut self, mut batch: Batch) -> Batch {
        self.file.version += 1;
        batch.id = self
            .file
            .batches
            .last()
            .map_or(1, |last| last.id.saturating_add(1));
        for change in &mut batch.changes {
            if let Some(after) = &mut change.after {
                after.batch = batch.id;
                match self.file.words.iter().position(|w| w.text == after.text) {
                    Some(index) => self.file.words[index] = after.clone(),
                    None => self.file.words.push(after.clone()),
                }
            } else if let Some(before) = &change.before {
                self.file.words.retain(|w| w.text != before.text);
            } else if let Some(after) = &mut change.phrase_after {
                after.batch = batch.id;
                self.file.phrases.push(after.clone());
            } else if let Some(before) = &change.phrase_before {
                let id = before.id;
                self.file.phrases.retain(|p| p.id != id);
            }
        }
        self.file.batches.push(batch);
        if self.file.batches.len() > KEEP_BATCHES {
            let keep = self.file.batches.len() - KEEP_BATCHES;
            self.file.batches.drain(..keep);
        }
        self.reindex();
        self.file.batches.last().expect("剛加入").clone()
    }

    /// 依檔案內容重建索引：只用有效、在上限內的詞；其餘記進 `problems`，
    /// 檔案內容不動。
    fn reindex(&mut self) {
        self.index = Node::default();
        self.by_text.clear();
        self.english.clear();
        self.english_words.clear();
        self.problems.clear();
        self.longest = 0;
        for (index, word) in self.file.words.iter().enumerate() {
            if self.by_text.contains_key(&word.text) {
                self.problems
                    .push(format!("「{}」重複出現，只使用第一筆", word.text));
                continue;
            }
            if self.by_text.len() >= MAX_WORDS {
                self.problems.push(format!(
                    "自訂詞超過上限 {MAX_WORDS}，第 {} 筆以後沒有使用",
                    index + 1
                ));
                break;
            }
            let normalized = match normalize(&word.text, &word.readings, index as i32) {
                Ok(normalized) => normalized,
                Err(err) => {
                    self.problems.push(format!("沒有使用：{}", err.msg));
                    continue;
                }
            };
            if normalized.text != word.text {
                self.problems.push("沒有使用：詞面前後有空白".to_string());
                continue;
            }
            self.by_text.insert(word.text.clone(), index);
            if is_english(&word.text) {
                self.english.insert(word.text.clone());
                self.english_words.push(word.text.clone());
                continue;
            }
            for reading in &normalized.readings {
                let syllables = parse_reading(reading);
                self.longest = self.longest.max(syllables.len());
                let mut node = &mut self.index;
                node.size += 1;
                for syllable in syllables {
                    node = node.children.entry(syllable).or_default();
                    node.size += 1;
                }
                node.words.push(index);
            }
        }
    }

    fn find(&self, syllables: &[Syllable]) -> Option<&Node> {
        let mut node = &self.index;
        for syllable in syllables {
            node = node.children.get(syllable)?;
        }
        Some(node)
    }

    /// 回傳讀音恰為 `syllables` 的自訂詞，依檔案順序。
    pub fn exact(&self, syllables: &[Syllable]) -> Vec<Match> {
        if syllables.is_empty() {
            return Vec::new();
        }
        let Some(node) = self.find(syllables) else {
            return Vec::new();
        };
        node.words
            .iter()
            .map(|index| Match {
                text: self.file.words[*index].text.clone(),
                reading: syllables.to_vec(),
            })
            .collect()
    }

    /// 回報是否有讀音以 `syllables` 開頭的自訂詞（含恰好相等）。
    pub fn has_prefix(&self, syllables: &[Syllable]) -> bool {
        self.find(syllables).is_some_and(|node| node.size > 0)
    }

    /// 回傳讀音以 `syllables` 開頭的自訂詞（含恰好相等），依讀音長度、
    /// 檔案順序排列。`limit` 大於 0 時至多回傳 `limit` 筆。
    pub fn prefixed(&self, syllables: &[Syllable], limit: usize) -> Vec<Match> {
        if syllables.is_empty() {
            return Vec::new();
        }
        let Some(root) = self.find(syllables) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut level = vec![(root, syllables.to_vec())];
        while !level.is_empty() {
            let mut next = Vec::new();
            for (node, path) in level {
                for index in &node.words {
                    out.push(Match {
                        text: self.file.words[*index].text.clone(),
                        reading: path.clone(),
                    });
                    if limit > 0 && out.len() >= limit {
                        return out;
                    }
                }
                let mut keys: Vec<&Syllable> = node.children.keys().collect();
                keys.sort();
                for key in keys {
                    let mut child_path = path.clone();
                    child_path.push(key.clone());
                    next.push((&node.children[key], child_path));
                }
            }
            level = next;
        }
        out
    }

    /// 回報 `raw` 是否恰好是一個英文詞（大小寫照字面）。
    pub fn has_english(&self, raw: &str) -> bool {
        self.english.contains(raw)
    }

    /// 回報是否有英文詞以 `raw` 開頭（含恰好相等）。
    ///
    /// 英文詞裡有聲調鍵（`ipv6`、`s3`）時，混打打到一半的那個數字不當聲調。
    pub fn has_english_prefix(&self, raw: &str) -> bool {
        if raw.is_empty() {
            return false;
        }
        self.english.iter().any(|word| word.starts_with(raw))
    }

    /// 列出以 `raw` 開頭的英文詞（含恰好相等）。
    ///
    /// 前綴不分大小寫，回傳照詞條字面（`pc` → `pCloud`）。短的在前，同長度
    /// 照檔案順序。`limit` 大於 0 時至多回傳 `limit` 筆。
    pub fn english_prefix(&self, raw: &str, limit: usize) -> Vec<String> {
        if raw.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<String> = self
            .english_words
            .iter()
            .filter(|word| {
                word.get(..raw.len())
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case(raw))
            })
            .cloned()
            .collect();
        out.sort_by_key(|word| word.len());
        if limit > 0 && out.len() > limit {
            out.truncate(limit);
        }
        out
    }

    /// 回報有效的英文詞數。
    pub fn english_len(&self) -> usize {
        self.english.len()
    }
}

impl Default for Set {
    fn default() -> Self {
        Self::new()
    }
}

/// 一筆要加入的詞（讀音可以有多個；英文詞沒有讀音）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// 詞面。
    #[serde(rename = "word")]
    pub text: String,
    /// 讀音。
    #[serde(default)]
    pub readings: Vec<String>,
}

/// 檢查並整理一個詞與它的讀音。
///
/// 讀音的音節以空白分隔、去掉多餘空白，重複的讀音只留一個。英文詞不能有
/// 讀音。`item` 是批次中的位置，用在錯誤訊息。
pub fn normalize(text: &str, readings: &[String], item: i32) -> Result<Word, Error> {
    let text = text.trim();
    let n = text.chars().count();
    if n == 0 {
        return Err(fail(
            CODE_INVALID_WORD,
            item,
            text,
            format!("第 {} 筆是空的", item + 1),
        ));
    }
    if english_like(text) {
        return normalize_english(text, readings, item);
    }
    if n > MAX_WORD_RUNES {
        let mut err = fail(
            CODE_INVALID_WORD,
            item,
            text,
            format!(
                "「{text}」有 {n} 個字，自訂詞最多 {MAX_WORD_RUNES} 字；更長的內容請放「我的片語」"
            ),
        );
        err.limit = MAX_WORD_RUNES as i32;
        return Err(err);
    }
    for ch in text.chars() {
        if !is_han(ch) {
            return Err(fail(
                CODE_INVALID_WORD,
                item,
                text,
                format!(
                    "「{text}」含有「{ch}」；自訂詞只收漢字，或由英文字母、數字與 - _ . 組成的英文詞，其他文字請放「我的片語」"
                ),
            ));
        }
    }
    if readings.is_empty() {
        return Err(fail(
            CODE_INVALID_READING,
            item,
            text,
            format!("「{text}」沒有讀音"),
        ));
    }
    let mut word = Word::new(text, Vec::new());
    for raw in readings {
        let syllables = parse_reading(raw);
        if syllables.len() != n {
            return Err(fail(
                CODE_INVALID_READING,
                item,
                text,
                format!(
                    "「{text}」有 {n} 個字，讀音「{}」卻有 {} 個音節",
                    raw.trim(),
                    syllables.len()
                ),
            ));
        }
        for syllable in &syllables {
            if !syllable.legal() {
                return Err(fail(
                    CODE_INVALID_READING,
                    item,
                    text,
                    format!(
                        "「{text}」的讀音「{}」中「{}」不是注音音節",
                        raw.trim(),
                        syllable
                    ),
                ));
            }
        }
        let key = syllables_key(&syllables);
        if !word.readings.contains(&key) {
            word.readings.push(key);
        }
    }
    if word.readings.len() > MAX_READINGS {
        let mut err = fail(
            CODE_LIMIT_READINGS,
            item,
            text,
            format!(
                "「{text}」有 {} 個讀音，同一個詞最多 {MAX_READINGS} 個",
                word.readings.len()
            ),
        );
        err.limit = MAX_READINGS as i32;
        return Err(err);
    }
    Ok(word)
}

fn normalize_english(text: &str, readings: &[String], item: i32) -> Result<Word, Error> {
    if text.len() > MAX_ENGLISH_LEN {
        let mut err = fail(
            CODE_INVALID_WORD,
            item,
            text,
            format!(
                "「{text}」有 {} 個字元，英文詞最多 {MAX_ENGLISH_LEN} 個；更長的內容請放「我的片語」",
                text.len()
            ),
        );
        err.limit = MAX_ENGLISH_LEN as i32;
        return Err(err);
    }
    if !is_english(text) {
        for ch in text.chars() {
            if !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.') {
                return Err(fail(
                    CODE_INVALID_WORD,
                    item,
                    text,
                    format!(
                        "「{text}」含有「{ch}」；英文詞只收英文字母、數字與 - _ .，其他文字請放「我的片語」"
                    ),
                ));
            }
        }
        return Err(fail(
            CODE_INVALID_WORD,
            item,
            text,
            format!("「{text}」沒有英文字母；英文詞至少要有一個字母"),
        ));
    }
    for reading in readings {
        if !reading.trim().is_empty() {
            return Err(fail(
                CODE_INVALID_READING,
                item,
                text,
                format!("「{text}」是英文詞，不需要讀音"),
            ));
        }
    }
    Ok(Word::new(text, Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARMSTRONG: &str = "新阿姆斯特朗回旋加速噴氣式阿姆斯特朗砲";
    const ARMSTRONG_READING: &str = "ㄒㄧㄣ ㄚ ㄇㄨˇ ㄙ ㄊㄜˋ ㄌㄤˇ ㄏㄨㄟˊ ㄒㄩㄢˊ ㄐㄧㄚ ㄙㄨˋ ㄆㄣ ㄑㄧˋ ㄕˋ ㄚ ㄇㄨˇ ㄙ ㄊㄜˋ ㄌㄤˇ ㄆㄠˋ";

    fn code(err: &Error) -> &str {
        &err.code
    }

    fn entry(text: &str, readings: &[&str]) -> Entry {
        Entry {
            text: text.to_string(),
            readings: readings.iter().map(|r| r.to_string()).collect(),
        }
    }

    fn han_name(i: usize) -> String {
        let first = char::from_u32(0x4E00 + (i / 100) as u32).expect("漢字");
        let second = char::from_u32(0x4E00 + (i % 100) as u32).expect("漢字");
        format!("{first}{second}")
    }

    #[test]
    fn test_add_lookup_and_long_prefix() {
        let mut set = Set::new();
        let batch = set
            .add(
                &[
                    entry(ARMSTRONG, &[ARMSTRONG_READING]),
                    entry("陳怡君", &["ㄔㄣˊ  ㄧˊ ㄐㄩㄣ"]),
                ],
                SOURCE_MANUAL,
                100,
            )
            .expect("加入")
            .expect("有變更");
        assert_eq!(batch.id, 1);
        assert_eq!(batch.changes.len(), 2);
        assert_eq!(set.len(), 2);
        assert_eq!(set.version(), 1);
        assert_eq!(set.longest(), 19);

        let word = set.lookup("陳怡君").expect("查得到");
        assert_eq!(word.readings[0], "ㄔㄣˊ ㄧˊ ㄐㄩㄣ");
        assert_eq!(word.source, SOURCE_MANUAL);
        assert_eq!(word.added, 100);
        assert_eq!(word.batch, 1);

        let syllables = parse_reading(ARMSTRONG_READING);
        let matches = set.exact(&syllables);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].text, ARMSTRONG);
        assert!(set.has_prefix(&syllables[..3]));
        assert!(set.has_prefix(&syllables[..12]));
        assert!(!set.has_prefix(&parse_reading("ㄒㄧㄣ ㄇㄨˇ")));

        let prefixed = set.prefixed(&syllables[..3], 0);
        assert_eq!(prefixed.len(), 1);
        assert_eq!(prefixed[0].reading.len(), 19);
        assert!(set.has_reading(ARMSTRONG, ARMSTRONG_READING));
        assert!(!set.has_reading(ARMSTRONG, "ㄒㄧㄣ"));
    }

    #[test]
    fn test_add_merges_readings_and_skips_noop() {
        let mut set = Set::new();
        set.add(&[entry("曾", &["ㄗㄥ"])], SOURCE_MANUAL, 1)
            .expect("加入");
        let batch = set
            .add(&[entry("曾", &["ㄘㄥˊ", "ㄗㄥ"])], SOURCE_COMPOSE, 2)
            .expect("加入")
            .expect("有變更");
        assert_eq!(batch.changes.len(), 1);
        assert!(batch.changes[0].before.is_some());
        let word = set.lookup("曾").expect("查得到");
        assert_eq!(word.readings.join("|"), "ㄗㄥ|ㄘㄥˊ");
        assert_eq!(word.source, SOURCE_MANUAL);
        assert_eq!(word.added, 1);
        let again = set
            .add(&[entry("曾", &["ㄘㄥˊ"])], SOURCE_MANUAL, 3)
            .expect("沒有錯誤");
        assert!(again.is_none());
        assert_eq!(set.version(), 2);
    }

    #[test]
    fn test_validation_rejects_whole_batch() {
        let cases: Vec<(Entry, &str)> = vec![
            (entry("", &["ㄅ"]), CODE_INVALID_WORD),
            (entry("abc", &["ㄅ ㄅ ㄅ"]), CODE_INVALID_READING),
            (entry("ab c", &[]), CODE_INVALID_WORD),
            (entry("a+b", &[]), CODE_INVALID_WORD),
            (entry("123", &[]), CODE_INVALID_WORD),
            (entry("世a", &["ㄕˋ ㄚ"]), CODE_INVALID_WORD),
            (entry(&"a".repeat(33), &[]), CODE_INVALID_WORD),
            (entry("ネオ", &["ㄋㄟ ㄛ"]), CODE_INVALID_WORD),
            (
                entry(&"好".repeat(33), &["ㄏㄠˇ ".repeat(33).trim()]),
                CODE_INVALID_WORD,
            ),
            (entry("世傑", &["ㄕˋ"]), CODE_INVALID_READING),
            (entry("世傑", &["ㄕˋ ㄅㄩ"]), CODE_INVALID_READING),
            (entry("世傑", &[]), CODE_INVALID_READING),
            (
                entry("了", &["ㄌㄜ˙", "ㄌㄧㄠˇ", "ㄌㄧㄠˋ", "ㄌㄧㄠˊ", "ㄌㄜˋ"]),
                CODE_LIMIT_READINGS,
            ),
        ];
        for (bad, want) in cases {
            let mut set = Set::new();
            let err = set
                .add(
                    &[entry("王小明", &["ㄨㄤˊ ㄒㄧㄠˇ ㄇㄧㄥˊ"]), bad.clone()],
                    SOURCE_MANUAL,
                    1,
                )
                .expect_err("整批拒絕");
            assert_eq!(code(&err), want, "{bad:?}：{err:?}");
            assert_eq!(set.len(), 0, "{bad:?}：不能部分寫入");
            assert_eq!(set.version(), 0, "{bad:?}：不能部分寫入");
        }
        let mut set = Set::new();
        set.add(
            &[entry(&"好".repeat(32), &["ㄏㄠˇ ".repeat(32).trim()])],
            SOURCE_MANUAL,
            1,
        )
        .expect("32 字可以");
        let mut set = Set::new();
        set.add(&[entry("呢", &["ㄋㄜ˙"])], SOURCE_MANUAL, 1)
            .expect("輕聲");
    }

    #[test]
    fn test_limits() {
        let mut set = Set::new();
        let many: Vec<Entry> = (0..=MAX_BATCH_ITEMS).map(|_| Entry::default()).collect();
        assert_eq!(
            code(&set.add(&many, SOURCE_ASSISTANT, 1).expect_err("單批上限")),
            CODE_LIMIT_BATCH
        );
        for i in 0..MAX_WORDS {
            set.file
                .words
                .push(Word::new(han_name(i), vec!["ㄨㄤˊ ㄒㄧㄠˇ".to_string()]));
        }
        set.reindex();
        let err = set
            .add(
                &[entry("王大明", &["ㄨㄤˊ ㄉㄚˋ ㄇㄧㄥˊ"])],
                SOURCE_MANUAL,
                1,
            )
            .expect_err("已滿");
        assert_eq!(err.code, CODE_LIMIT_WORDS);
        assert_eq!(err.left, 0);
        assert_eq!(err.limit, MAX_WORDS as i32);
        // 已有的詞加讀音不佔新名額。
        set.add(&[entry(&han_name(0), &["ㄨㄤˊ ㄒㄧㄠˋ"])], SOURCE_MANUAL, 1)
            .expect("加讀音");
    }

    #[test]
    fn test_over_limit_file_kept_but_not_used() {
        let mut json = String::from(r#"{"format":1,"words":["#);
        for i in 0..MAX_WORDS + 5 {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&format!(
                r#"{{"word":"{}","readings":["ㄨㄤˊ ㄒㄧㄠˇ"]}}"#,
                han_name(i)
            ));
        }
        json.push_str(r#",{"word":"bad","readings":["ㄅ"]}]}"#);
        let mut set = Set::load(json.as_bytes()).expect("載入");
        assert_eq!(set.len(), MAX_WORDS);
        assert_eq!(set.stored(), MAX_WORDS + 6);
        assert!(!set.problems().is_empty());
        assert!(set.lookup(&han_name(MAX_WORDS + 1)).is_none());
        assert_eq!(
            code(
                &set.add(
                    &[entry("王大明", &["ㄨㄤˊ ㄉㄚˋ ㄇㄧㄥˊ"])],
                    SOURCE_MANUAL,
                    1
                )
                .expect_err("超過上限期間拒絕新增")
            ),
            CODE_OVER_LIMIT
        );
        set.remove(&[han_name(0)], SOURCE_MANUAL, 2).expect("刪除");
        let mut saved = Vec::new();
        set.save(&mut saved).expect("存檔");
        let text = String::from_utf8(saved.clone()).expect("UTF-8");
        let back = Set::load(saved.as_slice()).expect("讀回");
        assert_eq!(back.stored(), MAX_WORDS + 5);
        assert!(text.contains(r#""bad""#));
        assert!(
            back.lookup(&han_name(MAX_WORDS)).is_some(),
            "刪掉一個後，原本超出的第一個詞進入上限內"
        );
    }

    #[test]
    fn test_remove_and_undo() {
        let mut set = Set::new();
        let add = set
            .add(
                &[
                    entry("陳怡君", &["ㄔㄣˊ ㄧˊ ㄐㄩㄣ"]),
                    entry("林世傑", &["ㄌㄧㄣˊ ㄕˋ ㄐㄧㄝˊ"]),
                ],
                SOURCE_ASSISTANT,
                1,
            )
            .expect("加入")
            .expect("有變更");
        assert_eq!(
            code(
                &set.remove(&["不存在".to_string()], SOURCE_MANUAL, 2)
                    .expect_err("刪不存在的詞")
            ),
            CODE_NOT_FOUND
        );
        let remove = set
            .remove(&["陳怡君".to_string()], SOURCE_MANUAL, 2)
            .expect("刪除");
        assert_eq!(set.len(), 1);
        set.undo(remove.id, SOURCE_MANUAL, 3).expect("復原刪除");
        assert_eq!(set.len(), 2);
        assert_eq!(
            code(
                &set.undo(remove.id, SOURCE_MANUAL, 3)
                    .expect_err("同一批不能復原兩次")
            ),
            CODE_CONFLICT
        );
        set.set_readings(
            "林世傑",
            &[
                "ㄌㄧㄣˊ ㄕˋ ㄐㄧㄝˊ".to_string(),
                "ㄌㄧㄣˊ ㄕˋ ㄐㄧㄝˋ".to_string(),
            ],
            SOURCE_MANUAL,
            4,
        )
        .expect("改讀音");
        assert_eq!(
            code(&set.undo(add.id, SOURCE_MANUAL, 5).expect_err("後來改過")),
            CODE_CONFLICT
        );
        assert_eq!(set.len(), 2);
        let last = set.batches()[0].clone();
        assert_eq!(last.op, "readings", "最新的批次在前");
        set.undo(last.id, SOURCE_MANUAL, 6).expect("復原讀音");
        set.undo(add.id, SOURCE_MANUAL, 7)
            .expect("讀音改回來後可以整批撤回");
        assert_eq!(set.len(), 0);
    }

    #[test]
    fn test_batches_kept_and_ids_monotonic() {
        let mut set = Set::new();
        for i in 0..KEEP_BATCHES + 10 {
            set.add(
                &[entry(&han_name(i), &["ㄨㄤˊ ㄒㄧㄠˇ"])],
                SOURCE_MANUAL,
                i as i64,
            )
            .expect("加入");
        }
        let batches = set.batches();
        assert_eq!(batches.len(), KEEP_BATCHES);
        assert_eq!(batches[0].id, (KEEP_BATCHES + 10) as u64);
        assert_eq!(set.len(), KEEP_BATCHES + 10);
        assert_eq!(
            code(&set.undo(1, SOURCE_MANUAL, 0).expect_err("太舊的批次")),
            CODE_NOT_FOUND
        );
    }

    #[test]
    fn test_phrases() {
        let mut set = Set::new();
        set.add_phrases(
            &[
                "——".to_string(),
                "台北市信義區".to_string(),
                "……".to_string(),
            ],
            SOURCE_MANUAL,
            1,
        )
        .expect("加入片語");
        assert_eq!(set.phrases().len(), 3);
        assert_eq!(set.phrases()[0].id, 1);
        set.move_phrase(3, 0, SOURCE_MANUAL, 2).expect("移動");
        assert_eq!(set.phrases()[0].text, "……");
        let remove = set
            .remove_phrases(&[2], SOURCE_MANUAL, 3)
            .expect("刪除片語");
        assert_eq!(set.phrases().len(), 2);
        set.add_phrases(&["新的".to_string()], SOURCE_MANUAL, 4)
            .expect("加入片語");
        assert_eq!(set.phrases()[2].id, 4, "刪掉的編號不重用");
        set.undo(remove.id, SOURCE_MANUAL, 5).expect("復原");
        assert_eq!(set.phrases().len(), 4);
        assert_eq!(set.phrases()[3].text, "台北市信義區");
        assert_eq!(
            code(
                &set.add_phrases(&["字".repeat(MAX_PHRASE_RUNES + 1)], SOURCE_MANUAL, 6)
                    .expect_err("片語長度")
            ),
            CODE_INVALID_WORD
        );
        assert_eq!(
            code(
                &set.add_phrases(&["  ".to_string()], SOURCE_MANUAL, 6)
                    .expect_err("空片語")
            ),
            CODE_EMPTY
        );
    }

    #[test]
    fn test_save_load_round_trip_and_newer_format() {
        let mut set = Set::new();
        set.add(&[entry(ARMSTRONG, &[ARMSTRONG_READING])], SOURCE_COMPOSE, 9)
            .expect("加入");
        set.add_phrases(&["地址".to_string()], SOURCE_MANUAL, 10)
            .expect("加入片語");
        let mut buffer = Vec::new();
        set.save(&mut buffer).expect("存檔");
        let back = Set::load(buffer.as_slice()).expect("讀回");
        assert_eq!(back.len(), 1);
        assert_eq!(back.phrases().len(), 1);
        assert_eq!(back.version(), 2);
        assert_eq!(back.batches().len(), 2);
        assert_eq!(back.exact(&parse_reading(ARMSTRONG_READING)).len(), 1);
        let err = Set::load(r#"{"format":2,"words":[]}"#.as_bytes()).expect_err("較新格式");
        match err {
            LoadError::NewerFormat(newer) => assert_eq!(newer.format, 2),
            other => panic!("應為較新格式：{other:?}"),
        }
    }

    #[test]
    fn test_clone_is_independent() {
        let mut set = Set::new();
        set.add(&[entry("陳怡君", &["ㄔㄣˊ ㄧˊ ㄐㄩㄣ"])], SOURCE_MANUAL, 1)
            .expect("加入");
        let copy = set.clone();
        set.set_readings(
            "陳怡君",
            &["ㄔㄣˊ ㄧˊ ㄐㄩㄣˋ".to_string()],
            SOURCE_MANUAL,
            2,
        )
        .expect("改讀音");
        let word = copy.lookup("陳怡君").expect("查得到");
        assert_eq!(word.readings[0], "ㄔㄣˊ ㄧˊ ㄐㄩㄣ");
        assert_eq!(copy.version(), 1);
    }

    #[test]
    fn test_entry_json() {
        let entries: Vec<Entry> =
            serde_json::from_str(r#"[{"word":"陳怡君","readings":["ㄔㄣˊ ㄧˊ ㄐㄩㄣ"]}]"#)
                .expect("解析");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "陳怡君");
        assert_eq!(entries[0].readings[0], "ㄔㄣˊ ㄧˊ ㄐㄩㄣ");
    }

    /// 產品品質回歸用的 500 筆詞要能完整載入。
    #[test]
    fn test_quality_fixture_loads() {
        let path = jc_testdata::require_data!("0c-testdata/vocab/fixture-500.json");
        let file = std::fs::File::open(path).expect("開啟 fixture");
        let set = Set::load(file).expect("載入 fixture");
        assert_eq!(set.len(), 500);
        assert!(set.problems().is_empty(), "{:?}", set.problems());
    }

    #[test]
    fn test_english_words() {
        let mut set = Set::new();
        let batch = set
            .add(
                &[
                    entry("ls", &[]),
                    entry(" kubectl ", &[""]),
                    entry("git-lfs", &[]),
                    entry("LS", &[]),
                    entry("a.b_c-1", &[]),
                    entry("世傑", &["ㄕˋ ㄐㄧㄝˊ"]),
                ],
                SOURCE_MANUAL,
                5,
            )
            .expect("加入")
            .expect("有變更");
        assert_eq!(batch.changes.len(), 6);
        for word in ["ls", "kubectl", "git-lfs", "LS", "a.b_c-1"] {
            assert!(set.has_english(word), "{word} 要是英文詞");
            let got = set.lookup(word).expect("查得到");
            assert!(got.english(), "{word}：{got:?}");
            assert!(got.readings.is_empty(), "{word}：{got:?}");
        }
        for word in ["Ls", "lS", "l", "kube", "世傑"] {
            assert!(!set.has_english(word), "{word} 不是英文詞");
        }
        assert!(set.has_english_prefix("kube"));
        assert!(set.has_english_prefix("git-"));
        assert!(!set.has_english_prefix("lsx"));
        assert!(!set.has_english_prefix(""));
        assert_eq!(set.len(), 6);
        assert_eq!(set.english_len(), 5);
        assert_eq!(set.longest(), 2);
        assert!(set.problems().is_empty());
        assert_eq!(set.exact(&parse_reading("ㄕˋ ㄐㄧㄝˊ")).len(), 1);
        assert!(set
            .add(&[entry("ls", &[])], SOURCE_MANUAL, 6)
            .expect("重複加入沒有變更")
            .is_none());
        assert_eq!(
            code(
                &set.set_readings("ls", &["ㄌ ㄙ".to_string()], SOURCE_MANUAL, 7)
                    .expect_err("英文詞不能設讀音")
            ),
            CODE_INVALID_READING
        );
        let remove = set
            .remove(&["ls".to_string()], SOURCE_MANUAL, 8)
            .expect("刪除");
        assert!(!set.has_english("ls"));
        assert!(set.has_english("LS"));
        set.undo(remove.id, SOURCE_MANUAL, 9).expect("復原");
        assert!(set.has_english("ls"));
        let mut buffer = Vec::new();
        set.save(&mut buffer).expect("存檔");
        let saved = String::from_utf8(buffer.clone()).expect("UTF-8");
        assert!(saved.contains(r#""word": "ls""#), "{saved}");
        assert!(saved.contains(r#""readings": []"#), "{saved}");
        let back = Set::load(buffer.as_slice()).expect("讀回");
        assert!(back.has_english("kubectl"));
        assert_eq!(back.len(), 6);
    }

    /// 加入英文詞前（格式 1）的 `vocab.json` 結構，照當時的 `vocab.go` 凍結。
    #[derive(Debug, Serialize, Deserialize)]
    struct LegacyWord {
        #[serde(rename = "word")]
        text: String,
        #[serde(default)]
        readings: Vec<String>,
        #[serde(default)]
        source: String,
        #[serde(default)]
        added: i64,
        #[serde(default)]
        batch: u64,
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct LegacyFile {
        format: i32,
        version: u64,
        words: Vec<LegacyWord>,
        phrases: Vec<Phrase>,
        batches: serde_json::Value,
    }

    /// 資料格式相容：英文詞不調高 `FORMAT`。
    #[test]
    fn test_english_words_readable_by_format1_readers() {
        let mut set = Set::new();
        set.add(
            &[
                entry("世傑", &["ㄕˋ ㄐㄧㄝˊ"]),
                entry("ls", &[]),
                entry("kubectl", &[]),
            ],
            SOURCE_MANUAL,
            1,
        )
        .expect("加入");
        let mut buffer = Vec::new();
        set.save(&mut buffer).expect("存檔");
        let old: LegacyFile = serde_json::from_slice(&buffer).expect("舊格式讀取器");
        assert_eq!(old.format, 1);
        assert_eq!(old.words.len(), 3);
        for word in &old.words {
            let han = word.text.chars().all(is_han);
            if !han && !word.readings.is_empty() {
                panic!("英文詞不能帶讀音：{word:?}");
            }
        }
        let again = serde_json::to_vec(&old).expect("舊格式寫回");
        let back = Set::load(again.as_slice()).expect("讀回");
        assert!(back.has_english("ls"));
        assert!(back.has_english("kubectl"));
        assert_eq!(back.len(), 3);
        let legacy = r#"{"format":1,"version":1,"words":[{"word":"ls","readings":null},{"word":"rm"},{"word":"世傑","readings":["ㄕˋ ㄐㄧㄝˊ"]}],"phrases":[],"batches":[]}"#;
        let back = Set::load(legacy.as_bytes()).expect("讀回");
        assert!(back.has_english("ls"));
        assert!(back.has_english("rm"));
        assert_eq!(back.len(), 3);
        assert_eq!(
            back.lookup("rm").expect("查得到").readings,
            Vec::<String>::new()
        );
    }

    /// 換電腦：設定程式把 `words` 匯出成文字檔，另一台讀回後以
    /// [`MAX_BATCH_ITEMS`] 一批送 `add`。
    #[test]
    fn test_import_in_batches_reproduces_words() {
        let mut source = Set::new();
        let mut entries: Vec<Entry> = (0..1200)
            .map(|i| Entry {
                text: format!("w{i}"),
                readings: Vec::new(),
            })
            .collect();
        entries.push(entry("曾國城", &["ㄗㄥ ㄍㄨㄛˊ ㄔㄥˊ"]));
        entries.push(entry("銀行", &["ㄧㄣˊ ㄒㄧㄥˊ", "ㄧㄣˊ ㄏㄤˊ"]));
        entries.push(entry("朋友們", &["ㄆㄥˊ ㄧㄡˇ ㄇㄣ˙"]));
        for chunk in entries.chunks(MAX_BATCH_ITEMS) {
            source.add(chunk, SOURCE_MANUAL, 1).expect("加入");
        }
        let mut target = Set::new();
        target
            .add(
                &[
                    entry("銀行", &["ㄧㄣˊ ㄏㄤˊ"]),
                    entry("只在這台", &["ㄓˇ ㄗㄞˋ ㄓㄜˋ ㄊㄞˊ"]),
                ],
                SOURCE_MANUAL,
                1,
            )
            .expect("加入");
        let send: Vec<Entry> = source
            .words()
            .into_iter()
            .map(|word| Entry {
                text: word.text,
                readings: word.readings,
            })
            .collect();
        assert_eq!(
            code(
                &target
                    .add(&send, SOURCE_MANUAL, 2)
                    .expect_err("超過 MaxBatchItems 的一批要整批拒絕")
            ),
            CODE_LIMIT_BATCH
        );
        for chunk in send.chunks(MAX_BATCH_ITEMS) {
            target.add(chunk, SOURCE_MANUAL, 2).expect("匯入");
        }
        assert_eq!(target.len(), source.len() + 1);
        for word in source.words() {
            let got = target.lookup(&word.text).expect("匯入後查得到");
            let want = if word.text == "銀行" {
                vec!["ㄧㄣˊ ㄏㄤˊ".to_string(), "ㄧㄣˊ ㄒㄧㄥˊ".to_string()]
            } else {
                word.readings
            };
            assert_eq!(got.readings, want, "{}", word.text);
        }
        assert!(
            target.lookup("只在這台").is_some(),
            "匯入不刪除檔案裡沒有的詞"
        );
    }

    #[test]
    fn test_english_prefix() {
        let mut set = Set::new();
        let entries: Vec<Entry> = [
            "pCloud", "Rclone", "PCIe", "ls", "LS", "pcb", "git-lfs", "ipv6",
        ]
        .iter()
        .map(|word| Entry {
            text: word.to_string(),
            readings: Vec::new(),
        })
        .chain(std::iter::once(entry("世傑", &["ㄕˋ ㄐㄧㄝˊ"])))
        .collect();
        set.add(&entries, SOURCE_MANUAL, 1).expect("加入");
        let cases: &[(&str, usize, &[&str])] = &[
            ("pc", 0, &["pcb", "PCIe", "pCloud"]),
            ("PC", 0, &["pcb", "PCIe", "pCloud"]),
            ("pc", 2, &["pcb", "PCIe"]),
            ("rcl", 0, &["Rclone"]),
            ("pcloud", 0, &["pCloud"]),
            ("ls", 0, &["ls", "LS"]),
            ("git", 0, &["git-lfs"]),
            ("ip", 0, &["ipv6"]),
            ("pcloudx", 0, &[]),
            ("zz", 0, &[]),
            ("", 0, &[]),
            ("世", 0, &[]),
        ];
        for (raw, limit, want) in cases {
            assert_eq!(
                set.english_prefix(raw, *limit),
                want.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                "EnglishPrefix({raw:?}, {limit})"
            );
        }
        set.remove(&["PCIe".to_string()], SOURCE_MANUAL, 2)
            .expect("刪除");
        assert_eq!(
            set.english_prefix("pc", 0),
            vec!["pcb".to_string(), "pCloud".to_string()]
        );
        assert_eq!(
            set.clone().english_prefix("pc", 0),
            vec!["pcb".to_string(), "pCloud".to_string()]
        );
    }
}
