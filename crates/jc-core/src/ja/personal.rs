//! 日文個人學習：記住使用者選過的文節、它當時的讀音與左文節。
//!
//! 學習資料存成獨立的 `personal.ja.json`，與注音的個人資料檔分開。行為對照
//! Go `core/ja/personal`：
//!
//! - 鍵值是 [`reading::key`] 加上左文節正規化後的雜湊，換了前文就不會套用。
//! - 只調整排序，不改變讀音對應；加成有上限，不會壓過差距明顯的統計首選，
//!   也不動來源不是轉換的候選（記号、かな表記等明確候選類型）。
//! - 反悔（[`Store::undo`]）、忘記（[`Store::forget`]）、重置
//!   （[`Store::reset`]）與暫停學習（[`Store::set_paused`]）都只作用在這一份
//!   資料，注音學習檔一個位元組都不會變。
//!
//! # 整合點
//!
//! Go 版從 `core/personal` 取得共用的 `JaFileName`、`JaPath`。這個 workspace
//! 的共用 personal store 由 `personal-zh` 擁有，尚未合併；因此本模組先提供
//! [`FILE_NAME`] 與 [`ja_path`]，等共用 store 落地後再改成轉呼叫，避免兩份
//! 常數各自漂移。測試以 `#[cfg(test)]` 的本地 stub 驗證「日文學習不碰注音檔」
//! 這條契約。
//!
//! 存檔由宿主以「同目錄暫存檔、落盤、關閉後原子替換」實作；[`Store::save_file`]
//! 示範同一套協定，同一路徑的寫入仍由宿主序列化。

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::ja::decode::{
    JaCandidate, JaCandidateSource, JaClause, JaContext, JaDecoder, JaLearning, JaResult, JaSpan,
};
use crate::ja::lexicon::JaEntry;
use crate::reading::{self, Sequence, Unit};

/// `personal.ja.json` 的格式版本。新增或改變欄位意義時加一；
/// 較新的檔案不得以本版結構寫回。
pub const FORMAT: u32 = 1;

/// 這份學習檔的語言 ID。日文與注音不共用同一個 JSON。
pub const LANGUAGE: &str = "ja";

/// 日文個人學習檔的固定檔名。
///
/// 注音與日文分檔：注音沿用原本的 `user.json`（合約舊稱 `personal.json`），
/// 日文用這一個。重置、忘記或反悔日文學習都不會寫到注音檔。
pub const FILE_NAME: &str = "personal.ja.json";

/// 學習檔的條目上限。超過時丟掉次數最少、最久未用的條目，
/// 讓檔案大小與轉換成本有上界。
pub const MAX_ENTRIES: usize = 20_000;

/// 保留的反悔步數。
const MAX_UNDO: usize = 32;

/// 讀檔上限；超過視為損壞，不把整份檔案讀進記憶體。
const MAX_FILE_BYTES: u64 = 8 << 20;

/// 左文節雜湊的長度。8 bytes 足以區分日常前文，也讓鍵值保持好讀。
pub const CONTEXT_HASH_BYTES: usize = 8;

/// 參與雜湊的前文長度上限。只取靠近組字區的尾巴，
/// 文件再長也不會讓同一個前文的鍵跟著變長。
const MAX_CONTEXT_CHARS: usize = 24;

/// 學習成本尺度：`JaEntry` 的成本是 Mozc 原始整數，分數是除以 1000 的尺度。
const COST_SCALE: f64 = 1_000.0;

/// 使用者選字。
pub const SOURCE_SELECTION: &str = "selection";
/// 使用者改文節。
pub const SOURCE_CLAUSE: &str = "clause";
/// Tab 接受。
pub const SOURCE_ACCEPT: &str = "accept";

/// 寫暫存檔時遞增，讓同一路徑的並行存檔不會撞名。
static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// 單一學習條目，形狀對應 contracts.md 第 5 節。
///
/// `reading` 與 `segments` 逐項對應，一個元素是一個詞的かな讀音（例如
/// `"きょう"`）；讀音單位本身仍是一個かな字元，查詢時再攤平成
/// [`reading::Sequence`]。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    /// 學習鍵：讀音加左文節雜湊。
    pub key: String,
    /// 逐段讀音。
    pub reading: Vec<String>,
    /// 選定的詞面。
    pub surface: String,
    /// 文節內各詞的詞面。
    pub segments: Vec<String>,
    /// 學習次數。
    pub count: i64,
    /// 最後使用的 Unix 秒。
    pub last_used: i64,
    /// 學習來源，見 [`SOURCE_SELECTION`] 等常數。
    pub source: String,
}

impl Entry {
    /// 把條目的文節讀音攤平成讀音單位序列。
    pub fn units(&self) -> Sequence {
        let mut seq = Sequence::new();
        for part in &self.reading {
            for ch in part.chars() {
                seq.push(Unit::new(ch.to_string()));
            }
        }
        seq
    }
}

/// 一次學習觀察：某個文節在某個前文後面被選成某個詞面。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// 這個文節的讀音。
    pub reading: Sequence,
    /// 使用者選定的詞面。
    pub surface: String,
    /// 文節內各詞的詞面，供顯示與忘記使用；可為空。
    pub segments: Vec<String>,
    /// 與 `segments` 對齊的各詞讀音；可為空。
    ///
    /// 有提供時學習檔的 `reading` 會逐段對應 `segments`，和 contracts.md 的
    /// 範例一致。
    pub segment_readings: Vec<Sequence>,
    /// 這個文節左側已確定的文字；句首填空字串。
    pub left: String,
    /// 學習來源，見 [`SOURCE_SELECTION`] 等常數。
    pub source: String,
    /// 宿主提供的 Unix 秒；核心不自行讀時鐘，0 表示沿用舊值。
    pub at: i64,
}

/// 讀取學習檔的錯誤。
#[derive(Debug, Error)]
pub enum LoadError {
    /// 檔案由較新版本寫成；呼叫端不得以目前結構寫回。
    #[error("日文學習檔格式 {format} 比本版支援的 {supported} 新")]
    NewerFormat {
        /// 檔案宣稱的版本。
        format: i64,
        /// 本版支援的版本。
        supported: u32,
    },
    /// 檔案屬於其他語言，不得拿來當日文學習檔。
    #[error("學習檔語言是 {language:?}，不是 {expected:?}")]
    Language {
        /// 檔案宣稱的語言。
        language: String,
        /// 本版要求的語言。
        expected: &'static str,
    },
    /// JSON 解析失敗。
    #[error("解析日文學習檔失敗：{0}")]
    Json(#[from] serde_json::Error),
}

/// 寫出學習檔的錯誤。
#[derive(Debug, Error)]
pub enum SaveError {
    /// 序列化失敗。
    #[error("序列化日文學習檔失敗：{0}")]
    Json(#[from] serde_json::Error),
    /// 寫入失敗。
    #[error("寫入日文學習檔失敗：{0}")]
    Io(#[from] io::Error),
}

/// 從檔案讀取學習資料的錯誤。
///
/// 內容損壞時回傳 [`LoadFileError::Corrupt`]，裡面附帶空白 [`Store`]；
/// 呼叫端應保留原檔並照常輸入。較新版本或別的語言不得寫回。
#[derive(Debug, Error)]
pub enum LoadFileError {
    /// 開檔或讀檔失敗。
    #[error("{path}：{source}")]
    Io {
        /// 檔案路徑。
        path: PathBuf,
        /// 底層 I/O 錯誤。
        #[source]
        source: io::Error,
    },
    /// 檔案由較新版本寫成。
    #[error("{path}：日文學習檔格式 {format} 比本版支援的 {supported} 新")]
    NewerFormat {
        /// 檔案路徑。
        path: PathBuf,
        /// 檔案宣稱的版本。
        format: i64,
        /// 本版支援的版本。
        supported: u32,
    },
    /// 檔案屬於其他語言。
    #[error("{path}：學習檔語言是 {language:?}，不是 {expected:?}")]
    Language {
        /// 檔案路徑。
        path: PathBuf,
        /// 檔案宣稱的語言。
        language: String,
        /// 本版要求的語言。
        expected: &'static str,
    },
    /// 檔案內容不是可解析的日文學習檔。
    ///
    /// `core` 不碰檔案系統的改名；宿主收到這個錯誤時應把原檔改名保留
    /// （例如 `personal.ja.json.corrupt-時間`），再用附帶的空白 `Store`
    /// 繼續，下一次存檔從乾淨的版本開始。
    #[error("{name} 內容損壞：{source}")]
    Corrupt {
        /// 檔案名稱或路徑，供錯誤訊息使用。
        name: String,
        /// 解析失敗的原因。
        #[source]
        source: LoadError,
        /// 可直接繼續使用的空白學習資料。
        store: Box<Store>,
    },
}

impl LoadFileError {
    /// 內容損壞時附帶的空白學習資料。
    pub fn store(&self) -> Option<&Store> {
        match self {
            Self::Corrupt { store, .. } => Some(store.as_ref()),
            _ => None,
        }
    }
}

/// 日文的個人學習資料。
///
/// 所有方法都能同時呼叫；存檔與按鍵學習不會互相看到半份資料。
/// `Store::default()` 可用，但正式路徑請用 [`Store::new`] 或 [`load_file`]。
#[derive(Debug, Default)]
pub struct Store {
    inner: RwLock<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    entries: Vec<Entry>,
    index: HashMap<String, usize>,
    undo: Vec<Vec<EntryState>>,
    paused: bool,
}

/// 一次學習交易前的條目狀態；反悔時整筆還原。
#[derive(Debug, Clone)]
struct EntryState {
    key: String,
    entry: Entry,
    exists: bool,
}

impl Inner {
    fn reindex(&mut self) {
        self.index = HashMap::with_capacity(self.entries.len());
        for (i, entry) in self.entries.iter().enumerate() {
            self.index.insert(entry.key.clone(), i);
        }
    }

    fn state(&self, key: &str) -> EntryState {
        match self.index.get(key) {
            Some(&i) => EntryState {
                key: key.to_string(),
                entry: self.entries[i].clone(),
                exists: true,
            },
            None => EntryState {
                key: key.to_string(),
                entry: Entry::default(),
                exists: false,
            },
        }
    }

    fn trim_undo(&mut self) {
        if self.undo.len() > MAX_UNDO {
            let drop = self.undo.len() - MAX_UNDO;
            self.undo.drain(..drop);
        }
    }

    fn prune(&mut self) {
        if self.entries.len() <= MAX_ENTRIES {
            return;
        }
        self.entries.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| b.last_used.cmp(&a.last_used))
                .then_with(|| a.key.cmp(&b.key))
        });
        self.entries.truncate(MAX_ENTRIES);
        self.reindex();
    }
}

impl Store {
    /// 回傳空白的日文學習資料。
    pub fn new() -> Self {
        Self::default()
    }

    fn read(&self) -> RwLockReadGuard<'_, Inner> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, Inner> {
        self.inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 記下一次學習。暫停學習或資料不合法時回傳 `false`。
    pub fn record(&self, sel: Selection) -> bool {
        self.record_all(std::slice::from_ref(&sel)) == 1
    }

    /// 把多個文節記成一次學習交易，回傳實際記下的筆數。
    ///
    /// 一次提交（含後續文節）只佔一步反悔。
    pub fn record_all(&self, sels: &[Selection]) -> usize {
        let mut inner = self.write();
        if inner.paused || sels.is_empty() {
            return 0;
        }
        let mut rec = Vec::new();
        let mut seen: HashSet<String> = HashSet::with_capacity(sels.len());
        let mut n = 0;
        for sel in sels {
            let Some(entry) = entry_from_selection(sel) else {
                continue;
            };
            if seen.insert(entry.key.clone()) {
                rec.push(inner.state(&entry.key));
            }
            if let Some(&i) = inner.index.get(&entry.key) {
                let cur = &mut inner.entries[i];
                cur.count += 1;
                cur.last_used = cur.last_used.max(entry.last_used);
                cur.surface = entry.surface;
                cur.segments = entry.segments;
                cur.reading = entry.reading;
                cur.source = entry.source;
            } else {
                let key = entry.key.clone();
                inner.entries.push(entry);
                let i = inner.entries.len() - 1;
                inner.index.insert(key, i);
            }
            n += 1;
        }
        if n == 0 {
            return 0;
        }
        inner.undo.push(rec);
        inner.trim_undo();
        inner.prune();
        n
    }

    /// 回傳同讀音、同前文的學習條目。
    pub fn lookup(&self, seq: &[Unit], left: &str) -> Option<Entry> {
        let inner = self.read();
        let k = key(seq, left);
        inner.index.get(&k).map(|&i| inner.entries[i].clone())
    }

    /// 依完整鍵值回傳學習條目，供設定程式顯示學到的內容。
    pub fn lookup_key(&self, key: &str) -> Option<Entry> {
        let inner = self.read();
        inner.index.get(key).map(|&i| inner.entries[i].clone())
    }

    /// 還原最近一次學習交易。沒有可反悔的紀錄時回傳 `false`。
    pub fn undo(&self) -> bool {
        let mut inner = self.write();
        let Some(rec) = inner.undo.pop() else {
            return false;
        };
        for st in rec {
            if let Some(&i) = inner.index.get(&st.key) {
                if st.exists {
                    inner.entries[i] = st.entry;
                } else {
                    inner.entries.remove(i);
                    inner.reindex();
                }
                continue;
            }
            if st.exists {
                let key = st.key.clone();
                inner.entries.push(st.entry);
                let i = inner.entries.len() - 1;
                inner.index.insert(key, i);
            }
        }
        true
    }

    /// 移除與詞面有關的學習：主詞面相同，或出現在文節內任一段。
    ///
    /// 回傳移除的筆數。忘記本身可以反悔。
    pub fn forget(&self, surface: &str) -> usize {
        self.forget_all(std::slice::from_ref(&surface.to_string()))
    }

    /// 移除與任一個詞面有關的學習，每筆資料只掃一遍。
    pub fn forget_all(&self, surfaces: &[String]) -> usize {
        let want: HashSet<&str> = surfaces
            .iter()
            .map(String::as_str)
            .filter(|w| !w.is_empty())
            .collect();
        if want.is_empty() {
            return 0;
        }
        let mut inner = self.write();
        let mut rec = Vec::new();
        let mut kept = Vec::with_capacity(inner.entries.len());
        for entry in std::mem::take(&mut inner.entries) {
            let matched = want.contains(entry.surface.as_str())
                || entry.segments.iter().any(|s| want.contains(s.as_str()));
            if matched {
                rec.push(EntryState {
                    key: entry.key.clone(),
                    entry,
                    exists: true,
                });
            } else {
                kept.push(entry);
            }
        }
        let removed = rec.len();
        if removed == 0 {
            inner.entries = kept;
            return 0;
        }
        inner.entries = kept;
        inner.reindex();
        inner.undo.push(rec);
        inner.trim_undo();
        removed
    }

    /// 清空全部日文學習與反悔紀錄。注音學習檔不在這裡，不會被碰到。
    ///
    /// 暫停狀態保留：使用者按了暫停，不會因為清除資料就被偷偷恢復學習。
    pub fn reset(&self) {
        let mut inner = self.write();
        inner.entries.clear();
        inner.index.clear();
        inner.undo.clear();
    }

    /// 設定是否暫停學習。暫停時仍會套用既有學習排序，只是不再記新的。
    pub fn set_paused(&self, paused: bool) {
        self.write().paused = paused;
    }

    /// 回報是否暫停學習。
    pub fn paused(&self) -> bool {
        self.read().paused
    }

    /// 回傳學習條目數。
    pub fn len(&self) -> usize {
        self.read().entries.len()
    }

    /// 是否沒有任何學習條目。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 全部學習條目的快照，依目前內部順序排列。
    pub fn entries(&self) -> Vec<Entry> {
        self.read().entries.clone()
    }

    /// 以固定順序寫出 JSON：鍵值排序，存檔內容可重現。
    pub fn save(&self, mut writer: impl Write) -> Result<(), SaveError> {
        let bytes = self.encode()?;
        writer.write_all(&bytes)?;
        Ok(())
    }

    /// 原子寫入檔案：同目錄暫存檔落盤後替換。
    ///
    /// 同一路徑的並行寫入仍由宿主序列化；這裡只保證單次寫入不會留下半份
    /// 檔案，失敗時保留原檔並刪掉暫存檔。
    pub fn save_file(&self, path: impl AsRef<Path>) -> Result<(), SaveError> {
        let path = path.as_ref();
        let bytes = self.encode()?;
        let parent = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| FILE_NAME.to_string());
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let temp = parent.join(format!(".{name}-{}-{seq}", std::process::id()));
        let result = write_atomic(&temp, path, &bytes);
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result.map_err(SaveError::Io)
    }

    fn encode(&self) -> Result<Vec<u8>, SaveError> {
        let inner = self.read();
        let mut entries: Vec<&Entry> = inner.entries.iter().collect();
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        let wire = WireStore {
            format: FORMAT,
            language: LANGUAGE,
            entries,
        };
        let mut out = serde_json::to_vec(&wire)?;
        // Go 的 json.Encoder 會在結尾補一個換行；保留同樣的檔案形狀。
        out.push(b'\n');
        Ok(out)
    }
}

#[derive(Serialize)]
struct WireStore<'a> {
    format: u32,
    language: &'static str,
    entries: Vec<&'a Entry>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawStore {
    format: i64,
    language: String,
    entries: Vec<Entry>,
}

fn write_atomic(temp: &Path, path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = File::create(temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temp, path)
}

/// 回傳與注音個人資料檔同目錄的日文學習檔路徑。
///
/// 只取目錄，不看注音檔的檔名：無論呼叫端用 `user.json` 還是
/// `personal.json`，日文檔一律叫 [`FILE_NAME`]。
///
/// 整合點：共用 personal store 落地後，這個函式應改成呼叫共用版本。
pub fn ja_path(personal_path: impl AsRef<Path>) -> PathBuf {
    let path = personal_path.as_ref();
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join(FILE_NAME),
        _ => PathBuf::from(FILE_NAME),
    }
}

/// 學習檔的鍵值：讀音加左文節正規化雜湊。
pub fn key(seq: &[Unit], left: &str) -> String {
    reading::key(seq) + "|" + &context_hash(left)
}

/// 正規化左文節：移除空白、英文字母轉小寫，只留最後
/// [`MAX_CONTEXT_CHARS`] 個字元。
///
/// 換前文時雜湊一定不同，相同前文的空白差異不會拆成兩筆。
pub fn normalize_left(left: &str) -> String {
    let mut out: String = left
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    let chars: Vec<char> = out.chars().collect();
    if chars.len() > MAX_CONTEXT_CHARS {
        out = chars[chars.len() - MAX_CONTEXT_CHARS..].iter().collect();
    }
    out
}

/// 回傳左文節正規化後的雜湊，作為鍵值的後半。
pub fn context_hash(left: &str) -> String {
    let digest = Sha256::digest(normalize_left(left).as_bytes());
    let mut out = String::with_capacity(CONTEXT_HASH_BYTES * 2);
    for byte in &digest[..CONTEXT_HASH_BYTES] {
        // 寫入 String 不會失敗。
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// 把一段かな讀音拆成讀音單位序列。
pub fn reading_units(reading: &str) -> Sequence {
    reading
        .chars()
        .map(|ch| Unit::new(ch.to_string()))
        .collect()
}

fn reading_parts(sel: &Selection) -> Vec<String> {
    let full = reading::key(&sel.reading);
    let mut parts = Vec::with_capacity(sel.segments.len().max(1));
    if !sel.segments.is_empty() && sel.segment_readings.len() == sel.segments.len() {
        let mut total = 0;
        for rd in &sel.segment_readings {
            if rd.is_empty() {
                return vec![full.replace(' ', "")];
            }
            parts.push(reading::key(rd).replace(' ', ""));
            total += rd.len();
        }
        // 逐段讀音必須剛好拼回整個文節。
        if total == sel.reading.len() {
            return parts;
        }
    }
    vec![full.replace(' ', "")]
}

fn entry_from_selection(sel: &Selection) -> Option<Entry> {
    if sel.reading.is_empty() || sel.surface.is_empty() {
        return None;
    }
    if sel.reading.iter().any(|u| !u.valid()) {
        return None;
    }
    let segments = if sel.segments.is_empty() {
        vec![sel.surface.clone()]
    } else {
        sel.segments.clone()
    };
    let mut source = sel.source.clone();
    if source.is_empty() {
        source = SOURCE_SELECTION.to_string();
    }
    Some(Entry {
        key: key(&sel.reading, &sel.left),
        reading: reading_parts(sel),
        surface: sel.surface.clone(),
        segments,
        count: 1,
        last_used: sel.at,
        source,
    })
}

/// 讀回日文學習檔。較新版本或別的語言回傳錯誤，不猜測內容。
pub fn load(reader: impl Read) -> Result<Store, LoadError> {
    let raw: RawStore = serde_json::from_reader(reader.take(MAX_FILE_BYTES))?;
    if raw.format > i64::from(FORMAT) {
        return Err(LoadError::NewerFormat {
            format: raw.format,
            supported: FORMAT,
        });
    }
    if !raw.language.is_empty() && raw.language != LANGUAGE {
        return Err(LoadError::Language {
            language: raw.language,
            expected: LANGUAGE,
        });
    }
    let store = Store::new();
    let mut inner = store.write();
    for mut entry in raw.entries {
        entry.key = entry.key.trim().to_string();
        if entry.key.is_empty() || entry.surface.is_empty() {
            continue;
        }
        if entry.reading.is_empty() {
            if let Some((reading_key, _)) = entry.key.split_once('|') {
                let derived = reading_key.replace(' ', "");
                if !derived.is_empty() {
                    entry.reading = vec![derived];
                }
            }
        }
        let units = entry.units();
        if units.is_empty() || units.iter().any(|u| !u.valid()) {
            continue;
        }
        if entry.count <= 0 {
            continue;
        }
        entry.last_used = entry.last_used.max(0);
        if let Some(&i) = inner.index.get(&entry.key) {
            let cur = &mut inner.entries[i];
            let newer = entry.last_used >= cur.last_used;
            cur.count += entry.count;
            cur.last_used = cur.last_used.max(entry.last_used);
            if newer {
                cur.surface = entry.surface;
                cur.segments = entry.segments;
                cur.reading = entry.reading;
                cur.source = entry.source;
            }
            continue;
        }
        let key = entry.key.clone();
        inner.entries.push(entry);
        let i = inner.entries.len() - 1;
        inner.index.insert(key, i);
    }
    drop(inner);
    Ok(store)
}

/// 從 `path` 讀回學習資料。檔案不存在時回傳空白資料。
///
/// 內容損壞時回傳 [`LoadFileError::Corrupt`]，附帶可直接使用的空白
/// [`Store`]；呼叫端應保留原檔並照常輸入。較新版本或別的語言回傳對應錯誤，
/// 不得寫回。
pub fn load_file(path: impl AsRef<Path>) -> Result<Store, LoadFileError> {
    let path = path.as_ref();
    let file = match File::open(path) {
        Ok(file) => file,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(Store::new()),
        Err(source) => {
            return Err(LoadFileError::Io {
                path: path.to_path_buf(),
                source,
            })
        }
    };
    match load(file) {
        Ok(store) => Ok(store),
        Err(LoadError::NewerFormat { format, .. }) => Err(LoadFileError::NewerFormat {
            path: path.to_path_buf(),
            format,
            supported: FORMAT,
        }),
        Err(LoadError::Language { language, .. }) => Err(LoadFileError::Language {
            path: path.to_path_buf(),
            language,
            expected: LANGUAGE,
        }),
        Err(source) => Err(LoadFileError::Corrupt {
            name: path.display().to_string(),
            source,
            store: Box::new(Store::new()),
        }),
    }
}

/// 學習加成的預設值。
///
/// `DEFAULT_BOOST` 是每次出現的加成，取 log2 讓第二次以後遞減；
/// `DEFAULT_MAX_BOOST` 是單一文節的上限。上限刻意不大：統計成本差距超過
/// 上限的候選，學習不該翻盤，這樣才不會因為選過一次罕見詞，就把明顯正確的
/// 候選擠到後面。
pub const DEFAULT_BOOST: f64 = 0.75;

/// 單一文節的學習加成上限。
pub const DEFAULT_MAX_BOOST: f64 = 2.0;

/// 重排時保留的候選數。
pub const DEFAULT_NBEST: usize = 8;

/// 把個人學習套到轉換結果上，並負責接收學習事件。
///
/// `Learner` 可被多個執行緒同時使用；[`Store`] 本身有鎖。`store` 為 `None`
/// 時不學習也不加成，方便外殼在沒有學習資料時照常運作。
#[derive(Debug, Clone, Default)]
pub struct Learner {
    /// 學習資料；`None` 表示停用。
    pub store: Option<Arc<Store>>,
    /// 每次學習的加成；0 使用 [`DEFAULT_BOOST`]。
    pub boost: f64,
    /// 單一文節的加成上限；0 使用 [`DEFAULT_MAX_BOOST`]。
    pub max_boost: f64,
    /// 重排時保留的候選數；0 使用 [`DEFAULT_NBEST`]。
    pub nbest: usize,
}

impl Learner {
    /// 建立使用 `store` 的學習器；`store` 可以是 `None`（不學習，也不加成）。
    pub fn new(store: Option<Arc<Store>>) -> Self {
        Self {
            store,
            ..Self::default()
        }
    }

    fn boost_value(&self) -> f64 {
        if self.boost > 0.0 {
            self.boost
        } else {
            DEFAULT_BOOST
        }
    }

    fn max_boost_value(&self) -> f64 {
        if self.max_boost > 0.0 {
            self.max_boost
        } else {
            DEFAULT_MAX_BOOST
        }
    }

    fn nbest_value(&self) -> usize {
        if self.nbest == 0 {
            DEFAULT_NBEST
        } else {
            self.nbest
        }
    }

    /// 單一學習條目的加成。次數越多加成越高，但有上限。
    pub fn bonus(&self, entry: &Entry) -> f64 {
        if entry.count <= 0 {
            return 0.0;
        }
        let bonus = self.boost_value() * (1.0 + entry.count as f64).log2();
        bonus.min(self.max_boost_value())
    }

    /// 回傳某個文節在指定前文下的加成；只看同讀音、同前文、同詞面。
    pub fn clause_bonus(&self, clause: &JaClause, left: &str) -> f64 {
        let Some(store) = self.store.as_ref() else {
            return 0.0;
        };
        if clause.surface.is_empty() {
            return 0.0;
        }
        let rd = clause_reading(clause);
        let Some(entry) = store.lookup(&rd, left) else {
            return 0.0;
        };
        if entry.surface != clause.surface {
            return 0.0;
        }
        self.bonus(&entry)
    }

    /// 回傳整串文節的學習加成。
    ///
    /// 第一個文節用 `left` 的左文；之後每個文節用前一個文節的詞面，和學習時
    /// 記下的左文節一致。換了前文就不會套用不相關的學習。
    pub fn score(&self, clauses: &[JaClause], left: &str) -> f64 {
        if self.store.is_none() {
            return 0.0;
        }
        let mut total = 0.0;
        let mut left = left.to_string();
        for clause in clauses {
            total += self.clause_bonus(clause, &left);
            if !clause.surface.is_empty() {
                left = clause.surface.clone();
            }
        }
        total
    }

    /// 把一次提交的文節記成一次學習交易，回傳記下的筆數。
    ///
    /// 第一個文節的前文是 `left`，其餘用前一個文節的詞面。時間由宿主另行
    /// 提供時用 [`Learner::learn_clauses_at`]。
    pub fn learn_clauses(&self, clauses: &[JaClause], left: &str, source: &str) -> usize {
        self.learn_clauses_at(clauses, left, source, 0)
    }

    /// 與 [`Learner::learn_clauses`] 相同，但把宿主提供的 Unix 秒記進
    /// `last_used`。
    pub fn learn_clauses_at(
        &self,
        clauses: &[JaClause],
        left: &str,
        source: &str,
        at: i64,
    ) -> usize {
        let Some(store) = self.store.as_ref() else {
            return 0;
        };
        let mut sels = Vec::with_capacity(clauses.len());
        let mut left = left.to_string();
        for clause in clauses {
            let rd = clause_reading(clause);
            if rd.is_empty() || clause.surface.is_empty() {
                continue;
            }
            sels.push(Selection {
                reading: rd,
                surface: clause.surface.clone(),
                segments: segment_surfaces(clause),
                segment_readings: segment_readings(clause),
                left: left.clone(),
                source: source.to_string(),
                at,
            });
            left = clause.surface.clone();
        }
        store.record_all(&sels)
    }

    /// 記下使用者明確選字。
    pub fn learn_selection(&self, clauses: &[JaClause], left: &str) -> usize {
        self.learn_clauses(clauses, left, SOURCE_SELECTION)
    }

    /// 記下使用者改文節後的結果。
    pub fn learn_clause_change(&self, clauses: &[JaClause], left: &str) -> usize {
        self.learn_clauses(clauses, left, SOURCE_CLAUSE)
    }

    /// 記下 Tab 接受的候選。
    pub fn learn_accepted(&self, clauses: &[JaClause], left: &str) -> usize {
        self.learn_clauses(clauses, left, SOURCE_ACCEPT)
    }

    /// 依候選內容學習。
    ///
    /// Rust 的 [`JaCandidate`] 沒有文節清單，一律把整筆候選當一個文節；
    /// Go 版在候選帶有文節時會逐節學習。
    pub fn learn_candidate(&self, cand: &JaCandidate, left: &str, source: &str) -> usize {
        if cand.surface.is_empty() || cand.reading.is_empty() {
            return 0;
        }
        let clause = JaClause {
            reading: cand.reading.clone(),
            surface: cand.surface.clone(),
            segments: Vec::new(),
            cost: cand.cost,
        };
        self.learn_clauses(std::slice::from_ref(&clause), left, source)
    }
}

impl JaLearning for Learner {
    /// 符合學習的詞條回傳負成本，讓 Viterbi 優先選它。
    ///
    /// 只有句首（`start == 0`）能確定左文就是 `ctx.left`；句中位置的真實左文
    /// 是前一個詞，不是 `ctx.left`，硬套會把不相關的學習用到別的位置，所以
    /// 一律回 0。整句重排由 [`Converter`] 依文節處理。
    fn cost(&self, reading: &[char], start: usize, entry: &JaEntry, ctx: &JaContext) -> i32 {
        if start != 0 {
            return 0;
        }
        let Some(store) = self.store.as_ref() else {
            return 0;
        };
        if entry.reading.is_empty() {
            return 0;
        }
        let entry_chars: Vec<char> = entry.reading.chars().collect();
        if start + entry_chars.len() > reading.len() {
            return 0;
        }
        if reading[start..start + entry_chars.len()] != entry_chars[..] {
            return 0;
        }
        let seq: Sequence = entry_chars
            .iter()
            .map(|ch| Unit::new(ch.to_string()))
            .collect();
        let Some(found) = store.lookup(&seq, &ctx.left) else {
            return 0;
        };
        if found.surface != entry.surface.as_ref() {
            return 0;
        }
        -((self.bonus(&found) * COST_SCALE).round() as i32)
    }
}

/// 接上個人學習的日文轉換器，包裝統計解碼器。
///
/// 只調整排序與分數比較，不改變讀音對應、不丟掉候選。來源不是轉換的候選
/// （記号、かな表記等）不加成，維持明確候選類型原本的相對順序。
#[derive(Debug, Clone, Copy)]
pub struct Converter<'a> {
    /// 統計解碼器。
    pub decoder: &'a JaDecoder,
    /// 學習器；`None` 時直接交給解碼器。
    pub learner: Option<&'a Learner>,
}

impl<'a> Converter<'a> {
    /// 建立學習轉換器。
    pub fn new(decoder: &'a JaDecoder, learner: Option<&'a Learner>) -> Self {
        Self { decoder, learner }
    }

    /// 回傳重排後的最佳轉換結果。沒有學習資料時直接交給解碼器，行為不變。
    pub fn convert(&self, reading: &[char], ctx: &JaContext) -> Option<JaResult> {
        if reading.is_empty() {
            return None;
        }
        let Some(learner) = self.learner else {
            return self.decoder.results(reading, ctx, 1).into_iter().next();
        };
        if learner.store.as_ref().map(|s| s.len()).unwrap_or(0) == 0 {
            return self.decoder.results(reading, ctx, 1).into_iter().next();
        }
        let results = self.decoder.results(reading, ctx, learner.nbest_value());
        if results.is_empty() {
            return self.decoder.results(reading, ctx, 1).into_iter().next();
        }
        let mut best = results[0].clone();
        let mut best_adj = best.score + learner.score(&best.clauses, &ctx.left);
        for result in results.into_iter().skip(1) {
            let adj = result.score + learner.score(&result.clauses, &ctx.left);
            if adj > best_adj || (adj == best_adj && better_result(&result, &best)) {
                best = result;
                best_adj = adj;
            }
        }
        Some(best)
    }

    /// 回傳重排後的候選。學習加成只作用在同讀音、同前文的轉換候選。
    pub fn candidates(&self, reading: &[char], span: JaSpan, ctx: &JaContext) -> Vec<JaCandidate> {
        let cands = self.decoder.candidates(reading, span, ctx);
        if cands.len() < 2 {
            return cands;
        }
        let Some(learner) = self.learner else {
            return cands;
        };
        if learner.store.as_ref().map(|s| s.len()).unwrap_or(0) == 0 {
            return cands;
        }
        let mut ranked: Vec<(JaCandidate, f64)> = cands
            .into_iter()
            .map(|cand| {
                let mut score = -f64::from(cand.cost) / COST_SCALE;
                if boostable(&cand) {
                    let clause = JaClause {
                        reading: cand.reading.clone(),
                        surface: cand.surface.clone(),
                        segments: Vec::new(),
                        cost: cand.cost,
                    };
                    score += learner.score(std::slice::from_ref(&clause), &ctx.left);
                }
                (cand, score)
            })
            .collect();
        ranked.sort_by(|(a, score_a), (b, score_b)| {
            score_b
                .total_cmp(score_a)
                .then_with(|| a.cost.cmp(&b.cost))
                .then_with(|| a.surface.cmp(&b.surface))
        });
        ranked.into_iter().map(|(cand, _)| cand).collect()
    }

    /// 直接交給解碼器；預測補全不因學習改變。
    pub fn prefixes(&self, reading: &[char]) -> Vec<JaCandidate> {
        self.decoder.prefixes(reading)
    }
}

/// 同分時的穩定順序：原始分數高者先，再依詞面。
fn better_result(a: &JaResult, b: &JaResult) -> bool {
    if a.score != b.score {
        return a.score > b.score;
    }
    a.surface() < b.surface()
}

/// 明確的候選類型（記号等）不加成、也不因學習被換到別的類型前面。
///
/// 對齊 Go `boostable`：只有轉換來源（`convert`／`personal`）可以重排。
fn boostable(cand: &JaCandidate) -> bool {
    matches!(cand.source, JaCandidateSource::Convert)
}

fn clause_reading(clause: &JaClause) -> Sequence {
    if !clause.reading.is_empty() {
        return reading_units(&clause.reading);
    }
    let mut seq = Sequence::new();
    for seg in &clause.segments {
        seq.extend(reading_units(&seg.reading));
    }
    seq
}

fn segment_surfaces(clause: &JaClause) -> Vec<String> {
    clause
        .segments
        .iter()
        .filter(|seg| !seg.surface.is_empty())
        .map(|seg| seg.surface.clone())
        .collect()
}

fn segment_readings(clause: &JaClause) -> Vec<Sequence> {
    if clause.segments.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(clause.segments.len());
    for seg in &clause.segments {
        if seg.reading.is_empty() {
            return Vec::new();
        }
        out.push(reading_units(&seg.reading));
    }
    out
}

/// 共用 personal store 的日文學習視圖。
///
/// 日文學習與注音學習寫在同一份 [`crate::personal::Store`]，以
/// [`crate::personal::LanguageId::Ja`] 命名空間區分；這個視圖只讀該命名空間，
/// 加成公式與尺度沿用前身 `core/ja/personal`：
///
/// `bonus = min(boost × log2(1 + 次數), max_boost)`
///
/// 同讀音、同左文、同詞面的紀錄給完整加成；只有同讀音、同詞面但左文不同的
/// 紀錄給 [`UNIFIED_GLOBAL_FACTOR`] 倍的加成。後者讓「同一句再打一次」在
/// 新的前文下仍然看得到學習結果，但單次學習不會壓過成本差距明顯的首選。
#[derive(Debug, Clone, Copy)]
pub struct UnifiedLearning<'a> {
    /// 共用的個人資料。
    pub store: &'a crate::personal::PersonalStore,
    /// 每次學習的加成；0 使用 [`DEFAULT_BOOST`]。
    pub boost: f64,
    /// 單一文節的加成上限；0 使用 [`DEFAULT_MAX_BOOST`]。
    pub max_boost: f64,
}

impl<'a> UnifiedLearning<'a> {
    /// 以預設加成建立視圖。
    pub fn new(store: &'a crate::personal::PersonalStore) -> Self {
        Self {
            store,
            boost: 0.0,
            max_boost: 0.0,
        }
    }

    fn boost_value(&self) -> f64 {
        if self.boost > 0.0 {
            self.boost
        } else {
            DEFAULT_BOOST
        }
    }

    fn max_boost_value(&self) -> f64 {
        if self.max_boost > 0.0 {
            self.max_boost
        } else {
            DEFAULT_MAX_BOOST
        }
    }

    fn bonus(&self, count: u32) -> f64 {
        if count == 0 {
            return 0.0;
        }
        let bonus = self.boost_value() * (1.0 + f64::from(count)).log2();
        bonus.min(self.max_boost_value())
    }

    /// 某個文節在指定前文下的學習加成。
    pub fn clause_bonus(&self, clause: &JaClause, left: &str) -> f64 {
        let seq = clause_reading(clause);
        if seq.is_empty() || clause.surface.is_empty() {
            return 0.0;
        }
        let reading = reading::key(&seq);
        let context = self.store.language_context_count(
            crate::personal::LanguageId::Ja,
            &normalize_left(left),
            &clause.surface,
        );
        let reading_count = self.store.language_reading_count(
            crate::personal::LanguageId::Ja,
            &reading,
            &clause.surface,
        );
        // 同前文的次數不會超過同讀音的次數；取較小值是這一組（讀音、前文、
        // 詞面）的聯合次數上界。
        let exact = self.bonus(context.min(reading_count));
        let global = self.bonus(reading_count) * UNIFIED_GLOBAL_FACTOR;
        exact.max(global)
    }

    /// 整串文節的學習加成；前文逐節串接，與學習時一致。
    pub fn score(&self, clauses: &[JaClause], left: &str) -> f64 {
        let mut total = 0.0;
        let mut left = left.to_string();
        for clause in clauses {
            total += self.clause_bonus(clause, &left);
            if !clause.surface.is_empty() {
                left = clause.surface.clone();
            }
        }
        total
    }

    /// 這個語言是否還沒有任何學習紀錄。
    pub fn is_empty(&self) -> bool {
        self.store
            .language_selections(crate::personal::LanguageId::Ja)
            == 0
    }

    /// 依學習加成重排 N-best；同分維持原始分數與詞面順序。
    pub fn rank_results(&self, results: &mut [JaResult], left: &str) {
        if self.is_empty() {
            return;
        }
        results.sort_by(|a, b| {
            let score_a = a.score + self.score(&a.clauses, left);
            let score_b = b.score + self.score(&b.clauses, left);
            score_b
                .total_cmp(&score_a)
                .then_with(|| b.score.total_cmp(&a.score))
                .then_with(|| a.surface().cmp(&b.surface()))
        });
    }

    /// 依學習加成重排候選；只調整轉換來源的候選。
    pub fn rank_candidates(&self, candidates: &mut [JaCandidate], left: &str) {
        if self.is_empty() {
            return;
        }
        let mut ranked: Vec<(JaCandidate, f64)> = candidates
            .iter()
            .cloned()
            .map(|cand| {
                let mut score = -f64::from(cand.cost) / COST_SCALE;
                if boostable(&cand) && !cand.reading.is_empty() && !cand.surface.is_empty() {
                    let clause = JaClause {
                        reading: cand.reading.clone(),
                        surface: cand.surface.clone(),
                        segments: Vec::new(),
                        cost: cand.cost,
                    };
                    score += self.score(std::slice::from_ref(&clause), left);
                }
                (cand, score)
            })
            .collect();
        ranked.sort_by(|(a, score_a), (b, score_b)| {
            score_b
                .total_cmp(score_a)
                .then_with(|| a.cost.cmp(&b.cost))
                .then_with(|| a.surface.cmp(&b.surface))
        });
        for (slot, (cand, _)) in candidates.iter_mut().zip(ranked) {
            *slot = cand;
        }
    }
}

/// 沒有同前文紀錄時，同讀音學習的加成折減。
pub const UNIFIED_GLOBAL_FACTOR: f64 = 0.5;

/// 被動接受首選的證據權重。
pub const UNIFIED_PASSIVE_WEIGHT: u32 = 1;

/// 明確手選（含非首選）的證據權重。
///
/// 選了非首選比照前身的手選來源，給兩倍證據，讓下一次優先反映這個選擇。
pub const UNIFIED_MANUAL_WEIGHT: u32 = 2;

/// 把一次提交的文節寫進共用 store 的日文命名空間。
///
/// 第一個文節的前文是 `left`，其餘用前一個文節的詞面；讀音以
/// [`reading::key`] 的空白分隔形式儲存，和注音共用同一個鍵的形狀。
/// 回傳實際記下的文節數。
pub fn learn_unified(
    store: &mut crate::personal::PersonalStore,
    clauses: &[JaClause],
    left: &str,
    weight: u32,
    at: i64,
) -> usize {
    let mut left = left.to_string();
    let mut recorded = 0;
    for clause in clauses {
        let seq = clause_reading(clause);
        if seq.is_empty() || clause.surface.is_empty() {
            continue;
        }
        store.note_language_weighted(
            crate::personal::LanguageId::Ja,
            &reading::key(&seq),
            &clause.surface,
            &normalize_left(&left),
            weight,
            at,
        );
        left = clause.surface.clone();
        recorded += 1;
    }
    recorded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ja::decode::{JaClauseSegment, JaWordMatch, JaWordSource};
    use crate::ja::lexicon::Split;

    static TEST_SEQ: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("jc-ja-personal-{tag}-{}-{seq}", std::process::id()));
            std::fs::create_dir_all(&path).expect("建立測試資料夾");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn seq(parts: &[&str]) -> Sequence {
        parts.iter().map(|s| Unit::new(*s)).collect()
    }

    fn chars(reading: &str) -> Vec<char> {
        reading.chars().collect()
    }

    fn write_store_file(dir: &Path, name: &str, store: &Store) {
        store.save_file(dir.join(name)).expect("寫出學習檔");
    }

    // ---- 與共用 personal store 的整合契約（本地 stub） ----
    //
    // 整合點：personal-zh 合併後，這段 stub 由共用的 personal 模組取代；
    // 契約本身不變：日文與注音同目錄、不同檔名，日文操作不碰注音檔。

    const ZH_STUB_FILE_NAME: &str = "user.json";

    fn zh_stub_path(dir: &Path) -> PathBuf {
        dir.join(ZH_STUB_FILE_NAME)
    }

    #[test]
    fn test_ja_path_uses_same_directory_and_separate_file() {
        let dir = TempDir::new("ja-path");
        for zh in ["user.json", "personal.json"] {
            let got = ja_path(dir.join(zh));
            assert_eq!(got, dir.join(FILE_NAME));
        }
    }

    #[test]
    fn test_ja_file_name_matches_shared_store_stub() {
        assert_eq!(FILE_NAME, "personal.ja.json");
        assert_ne!(FILE_NAME, ZH_STUB_FILE_NAME);
    }

    #[test]
    fn test_japanese_learning_never_touches_zhuyin_store() {
        let dir = TempDir::new("ja-zh-isolation");
        let zh_path = zh_stub_path(dir.path());
        let zh_bytes = r#"{"format":7,"bi":{"我\t是":1},"readings":{"我\tㄨㄛˇ":true}}"#.as_bytes();
        std::fs::write(&zh_path, zh_bytes).expect("寫入注音 stub");

        let ja_path = ja_path(&zh_path);
        let store = Store::new();
        store.record(Selection {
            reading: seq(&["き", "ょ", "う"]),
            surface: "今日".to_string(),
            ..Selection::default()
        });
        store.save_file(&ja_path).expect("寫日文學習檔");
        assert_eq!(
            std::fs::read(&zh_path).expect("讀注音檔"),
            zh_bytes.to_vec()
        );

        // 重置日文學習只清日文檔，注音檔位元組不變。
        let reloaded = load_file(&ja_path).expect("讀回日文學習檔");
        reloaded.reset();
        reloaded.save_file(&ja_path).expect("重寫日文學習檔");
        assert_eq!(
            std::fs::read(&zh_path).expect("讀注音檔"),
            zh_bytes.to_vec()
        );
        let after = load_file(&ja_path).expect("再讀日文學習檔");
        assert_eq!(after.len(), 0);
    }

    // ---- store.go ----

    #[test]
    fn test_key_uses_reading_and_normalized_left_context() {
        let s = seq(&["き", "ょ", "う"]);
        assert_eq!(key(&s, ""), format!("き ょ う|{}", context_hash("")));
        assert_ne!(key(&s, "今日は"), key(&s, "空気が"));
        assert_eq!(key(&s, " 今日　は "), key(&s, "今日は"));
        assert_eq!(key(&s, "ABC"), key(&s, "abc"));
        let long = "あ".repeat(100);
        assert_eq!(normalize_left(&long), "あ".repeat(MAX_CONTEXT_CHARS));
    }

    #[test]
    fn test_record_lookup_and_json_contract() {
        let store = Store::new();
        let s = seq(&["き", "ょ", "う"]);
        assert!(store.record(Selection {
            reading: s.clone(),
            surface: "今日".to_string(),
            segments: vec!["今日".to_string()],
            left: String::new(),
            at: 1_760_000_000,
            ..Selection::default()
        }));
        let entry = store.lookup(&s, "").expect("應有學習條目");
        assert_eq!(entry.surface, "今日");
        assert_eq!(entry.count, 1);
        assert_eq!(entry.last_used, 1_760_000_000);
        assert_eq!(entry.source, SOURCE_SELECTION);
        assert!(store.record(Selection {
            reading: s.clone(),
            surface: "今日".to_string(),
            ..Selection::default()
        }));
        assert_eq!(store.lookup(&s, "").unwrap().count, 2);
    }

    #[test]
    fn test_json_contract_shape() {
        let store = Store::new();
        store.record(Selection {
            reading: seq(&["き", "ょ", "う", "は"]),
            surface: "今日は".to_string(),
            segments: vec!["今日".to_string(), "は".to_string()],
            segment_readings: vec![seq(&["き", "ょ", "う"]), seq(&["は"])],
            left: String::new(),
            at: 1_760_000_000,
            ..Selection::default()
        });
        let mut buf = Vec::new();
        store.save(&mut buf).expect("存檔");
        let doc: serde_json::Value = serde_json::from_slice(&buf).expect("解析存檔 JSON");
        assert_eq!(doc["format"], 1);
        assert_eq!(doc["language"], "ja");
        let entries = doc["entries"].as_array().expect("entries");
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        for k in [
            "key",
            "reading",
            "surface",
            "segments",
            "count",
            "last_used",
            "source",
        ] {
            assert!(e.get(k).is_some(), "缺少 {k}：{e}");
        }
        let k = e["key"].as_str().expect("key");
        assert!(k.contains('|'));
        assert!(k.starts_with("き ょ う は|"));
        assert_eq!(
            e["reading"],
            serde_json::json!(["きょう", "は"]),
            "reading 應與 segments 逐段對應"
        );
        assert_eq!(e["segments"], serde_json::json!(["今日", "は"]));
    }

    #[test]
    fn test_save_load_round_trip_is_stable() {
        let dir = TempDir::new("round-trip");
        let store = Store::new();
        store.record_all(&[
            Selection {
                reading: seq(&["き", "ょ", "う"]),
                surface: "今日".to_string(),
                segments: vec!["今日".to_string()],
                at: 3,
                ..Selection::default()
            },
            Selection {
                reading: seq(&["く", "う", "き"]),
                surface: "空気".to_string(),
                segments: vec!["空気".to_string()],
                left: "今日".to_string(),
                at: 4,
                ..Selection::default()
            },
        ]);
        write_store_file(dir.path(), FILE_NAME, &store);
        let first = std::fs::read(dir.join(FILE_NAME)).expect("第一次存檔");
        let got = load_file(dir.join(FILE_NAME)).expect("讀回");
        assert_eq!(got.len(), 2);
        assert_eq!(
            got.lookup(&seq(&["き", "ょ", "う"]), "").unwrap().surface,
            "今日"
        );
        assert_eq!(
            got.lookup(&seq(&["く", "う", "き"]), "今日")
                .unwrap()
                .surface,
            "空気"
        );
        write_store_file(dir.path(), FILE_NAME, &got);
        let second = std::fs::read(dir.join(FILE_NAME)).expect("第二次存檔");
        assert_eq!(first, second, "存檔不穩定");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(".personal.ja.json-")
            })
            .collect();
        assert!(leftovers.is_empty(), "改名後不該留下暫存檔：{leftovers:?}");
    }

    #[test]
    fn test_save_file_replaces_whole_file_and_cleans_temp() {
        let dir = TempDir::new("replace");
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, "舊內容").expect("先寫舊檔");
        let store = Store::new();
        store.record(Selection {
            reading: seq(&["き", "ょ", "う"]),
            surface: "今日".to_string(),
            ..Selection::default()
        });
        write_store_file(dir.path(), FILE_NAME, &store);
        let got = load_file(&path).expect("讀回");
        assert_eq!(got.len(), 1);
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(".personal.ja.json-")
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "成功改名後不該留下暫存檔：{leftovers:?}"
        );
    }

    #[test]
    fn test_save_file_failure_keeps_original_and_cleans_temp() {
        let dir = TempDir::new("save-fail");
        let path = dir.join("target");
        std::fs::create_dir(&path).expect("建立目標資料夾");
        std::fs::write(path.join("keep"), []).expect("寫入 keep");
        let store = Store::new();
        store.record(Selection {
            reading: seq(&["あ"]),
            surface: "亜".to_string(),
            ..Selection::default()
        });
        assert!(store.save_file(&path).is_err(), "對目錄改名應失敗");
        assert!(path.join("keep").exists(), "原本的內容不應被動到");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".target-"))
            .collect();
        assert!(leftovers.is_empty(), "失敗時應刪掉暫存檔：{leftovers:?}");
    }

    #[test]
    fn test_load_missing_file_is_empty() {
        let dir = TempDir::new("missing");
        let store = load_file(dir.join(FILE_NAME)).expect("缺檔視為空白");
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn test_load_corrupt_file_reports_and_can_recover() {
        let dir = TempDir::new("corrupt");
        let path = dir.join(FILE_NAME);
        let corrupt = r#"{"format":1,"entries":["#;
        std::fs::write(&path, corrupt).expect("寫入損壞檔");
        let err = load_file(&path).expect_err("損壞檔應回錯誤");
        let blank = err.store().expect("應附帶空白 Store");
        assert_eq!(blank.len(), 0);
        match &err {
            LoadFileError::Corrupt { name, .. } => assert_eq!(*name, path.display().to_string()),
            other => panic!("應為 Corrupt：{other}"),
        }
        // 宿主收到 Corrupt 後把原檔改名保留，下一次讀取視為不存在。
        let saved = dir.join("personal.ja.json.corrupt-20261117T000000Z");
        std::fs::rename(&path, &saved).expect("隔離損壞檔");
        assert_eq!(std::fs::read_to_string(&saved).unwrap(), corrupt);
        let store = load_file(&path).expect("隔離後視為空白");
        assert_eq!(store.len(), 0);
        // 下一次存檔要能建立乾淨的檔案。
        store.record(Selection {
            reading: seq(&["あ"]),
            surface: "亜".to_string(),
            ..Selection::default()
        });
        write_store_file(dir.path(), FILE_NAME, &store);
        let got = load_file(&path).expect("重建後可讀");
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn test_load_newer_format_and_language_rejected() {
        let dir = TempDir::new("reject");
        let cases = [
            (
                "newer",
                r#"{"format":99,"language":"ja","entries":[]}"#,
                true,
            ),
            (
                "other-language",
                r#"{"format":1,"language":"zh","entries":[]}"#,
                false,
            ),
        ];
        for (name, body, newer) in cases {
            let path = dir.join(&format!("{name}.json"));
            std::fs::write(&path, body).expect("寫入測試檔");
            let err = load_file(&path).expect_err("應拒絕");
            if newer {
                assert!(matches!(&err, LoadFileError::NewerFormat { .. }), "{err}");
            } else {
                assert!(matches!(&err, LoadFileError::Language { .. }), "{err}");
            }
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                body,
                "不得改寫原檔"
            );
        }
    }

    #[test]
    fn test_load_skips_invalid_entries() {
        let ok_key = key(&seq(&["あ"]), "");
        let derived_key = key(&seq(&["お"]), "");
        let body = format!(
            r#"{{"format":1,"language":"ja","entries":[
                {{"key":"{ok_key}","reading":["あ"],"surface":"亜","count":2}},
                {{"key":"","reading":["い"],"surface":"以","count":1}},
                {{"key":"う|","reading":["う"],"surface":"","count":1}},
                {{"key":"え|","reading":["え"],"surface":"江","count":0}},
                {{"key":"{derived_key}","reading":[],"surface":"尾","count":1}}
            ]}}"#
        );
        let store = load(body.as_bytes()).expect("解析");
        assert_eq!(store.len(), 2);
        let entry = store.lookup(&seq(&["お"]), "").expect("應由鍵值推回");
        assert_eq!(entry.surface, "尾");
        assert_eq!(entry.units().len(), 1);
    }

    #[test]
    fn test_undo_restores_whole_transaction() {
        let store = Store::new();
        store.record(Selection {
            reading: seq(&["き", "ょ", "う"]),
            surface: "今日".to_string(),
            ..Selection::default()
        });
        let n = store.record_all(&[
            Selection {
                reading: seq(&["く", "う", "き"]),
                surface: "空気".to_string(),
                ..Selection::default()
            },
            Selection {
                reading: seq(&["が"]),
                surface: "が".to_string(),
                ..Selection::default()
            },
        ]);
        assert_eq!(n, 2);
        assert_eq!(store.len(), 3);
        assert!(store.undo());
        assert_eq!(store.len(), 1, "反悔後應只剩第一次");
        assert!(store.lookup(&seq(&["く", "う", "き"]), "").is_none());
        let entry = store
            .lookup(&seq(&["き", "ょ", "う"]), "")
            .expect("較早的學習");
        assert_eq!(entry.count, 1);
        assert!(store.undo());
        assert_eq!(store.len(), 0);
        assert!(!store.undo());
    }

    #[test]
    fn test_forget_matches_surface_and_segments() {
        let store = Store::new();
        store.record(Selection {
            reading: seq(&["き", "ょ", "う"]),
            surface: "今日".to_string(),
            segments: vec!["今日".to_string()],
            ..Selection::default()
        });
        store.record(Selection {
            reading: seq(&["く", "う", "き", "が"]),
            surface: "空気が".to_string(),
            segments: vec!["空気".to_string(), "が".to_string()],
            ..Selection::default()
        });
        assert_eq!(store.forget("空気"), 1);
        assert_eq!(store.len(), 1);
        assert!(store.lookup(&seq(&["く", "う", "き", "が"]), "").is_none());
        assert!(store.undo());
        assert_eq!(store.len(), 2, "忘記應可反悔");
        let n = store.forget_all(&["今日".to_string(), "が".to_string()]);
        assert_eq!(n, 2);
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn test_reset_clears_learning_but_keeps_paused() {
        let store = Store::new();
        store.record(Selection {
            reading: seq(&["あ"]),
            surface: "亜".to_string(),
            ..Selection::default()
        });
        store.set_paused(true);
        store.reset();
        assert_eq!(store.len(), 0);
        assert!(store.paused());
        assert!(!store.record(Selection {
            reading: seq(&["い"]),
            surface: "以".to_string(),
            ..Selection::default()
        }));
        store.set_paused(false);
        assert!(store.record(Selection {
            reading: seq(&["い"]),
            surface: "以".to_string(),
            ..Selection::default()
        }));
    }

    #[test]
    fn test_paused_still_applies_existing_ranking() {
        let store = Store::new();
        store.record(Selection {
            reading: seq(&["あ"]),
            surface: "亜".to_string(),
            ..Selection::default()
        });
        store.set_paused(true);
        let entry = store.lookup(&seq(&["あ"]), "").expect("既有排序");
        assert_eq!(entry.count, 1);
        assert!(!store.record(Selection {
            reading: seq(&["あ"]),
            surface: "亜".to_string(),
            ..Selection::default()
        }));
        assert_eq!(store.lookup(&seq(&["あ"]), "").unwrap().count, 1);
    }

    #[test]
    fn test_concurrent_record_on_one_store() {
        let store = Arc::new(Store::new());
        const WORKERS: usize = 8;
        const EACH: usize = 50;
        let mut handles = Vec::new();
        for w in 0..WORKERS {
            let store = Arc::clone(&store);
            handles.push(std::thread::spawn(move || {
                for i in 0..EACH {
                    let a = char::from_u32('あ' as u32 + w as u32).unwrap();
                    let b = char::from_u32('あ' as u32 + i as u32).unwrap();
                    store.record(Selection {
                        reading: seq(&["あ", &a.to_string(), &b.to_string()]),
                        surface: format!("語{a}{b}"),
                        ..Selection::default()
                    });
                }
            }));
        }
        for handle in handles {
            handle.join().expect("執行緒結束");
        }
        assert_eq!(store.len(), WORKERS * EACH);
    }

    #[test]
    fn test_concurrent_writers_leave_complete_file() {
        let dir = TempDir::new("concurrent-writers");
        const WRITERS: usize = 8;
        const EACH: usize = 100;
        let mut handles = Vec::new();
        for _ in 0..WRITERS {
            let path = dir.join(FILE_NAME);
            handles.push(std::thread::spawn(move || {
                let store = Store::new();
                for i in 0..EACH {
                    // 兩個かな欄位各取安全範圍，讓 100 筆鍵值都不同又都是合法單位。
                    let a = char::from_u32('あ' as u32 + (i / 50) as u32).unwrap();
                    let b = char::from_u32('あ' as u32 + (i % 50) as u32).unwrap();
                    store.record(Selection {
                        reading: seq(&["あ", &a.to_string(), &b.to_string()]),
                        surface: format!("語{a}{b}"),
                        ..Selection::default()
                    });
                }
                if let Err(err) = store.save_file(&path) {
                    panic!("存檔：{err}");
                }
            }));
        }
        for handle in handles {
            handle.join().expect("執行緒結束");
        }
        let got = load_file(dir.join(FILE_NAME)).expect("最後一份檔應完整可讀");
        assert_eq!(got.len(), EACH);
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(".personal.ja.json-")
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "並行存檔後不該留下暫存檔：{leftovers:?}"
        );
    }

    #[test]
    fn test_concurrent_record_and_save() {
        let store = Arc::new(Store::new());
        store.record(Selection {
            reading: seq(&["あ"]),
            surface: "亜".to_string(),
            ..Selection::default()
        });
        const READERS: usize = 4;
        const WRITERS: usize = 4;
        const EACH: usize = 50;
        let mut handles = Vec::new();
        for w in 0..WRITERS {
            let store = Arc::clone(&store);
            handles.push(std::thread::spawn(move || {
                for i in 0..EACH {
                    let a = char::from_u32('い' as u32 + w as u32).unwrap();
                    let b = char::from_u32('あ' as u32 + (i % 40) as u32).unwrap();
                    store.record(Selection {
                        reading: seq(&["あ", &a.to_string(), &b.to_string()]),
                        surface: format!("語{b}"),
                        ..Selection::default()
                    });
                }
            }));
        }
        for _ in 0..READERS {
            let store = Arc::clone(&store);
            handles.push(std::thread::spawn(move || {
                for _ in 0..EACH {
                    let mut buf = Vec::new();
                    if let Err(err) = store.save(&mut buf) {
                        panic!("Save：{err}");
                    }
                    let doc: RawStore =
                        serde_json::from_slice(&buf).expect("同時存檔讀到半份 JSON");
                    assert_eq!(doc.format, i64::from(FORMAT));
                    assert_eq!(doc.language, LANGUAGE);
                }
            }));
        }
        for handle in handles {
            handle.join().expect("執行緒結束");
        }
    }

    #[test]
    fn test_zero_store_is_usable() {
        let store = Store::default();
        assert_eq!(store.len(), 0);
        assert!(store.record(Selection {
            reading: seq(&["あ"]),
            surface: "亜".to_string(),
            ..Selection::default()
        }));
        assert_eq!(store.lookup(&seq(&["あ"]), "").unwrap().surface, "亜");
        let mut buf = Vec::new();
        store.save(&mut buf).expect("存檔");
        let text = String::from_utf8(buf).expect("UTF-8");
        assert!(text.contains(r#""format":1"#));
        assert!(text.contains(r#""language":"ja""#));
    }

    #[test]
    fn test_lookup_key_and_len() {
        let store = Store::new();
        store.record(Selection {
            reading: seq(&["あ"]),
            surface: "亜".to_string(),
            ..Selection::default()
        });
        let k = key(&seq(&["あ"]), "");
        assert_eq!(store.lookup_key(&k).unwrap().surface, "亜");
        assert!(store.lookup_key("ない|").is_none());
        assert_eq!(store.len(), 1);
    }

    // ---- learning.go ----

    struct TestSource {
        words: Vec<JaEntry>,
        longest: usize,
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
            for entry in &self.words {
                let units: Vec<char> = entry.reading.chars().collect();
                let end = start + units.len();
                if end > reading.len() || reading[start..end] != units[..] {
                    continue;
                }
                out(JaWordMatch {
                    end,
                    entry,
                    left_id: None,
                    right_id: None,
                });
            }
        }

        fn exact(&self, reading: &[char]) -> Vec<&JaEntry> {
            self.words
                .iter()
                .filter(|entry| entry.reading.chars().eq(reading.iter().copied()))
                .collect()
        }

        fn prefixed(&self, reading: &[char]) -> Vec<&JaEntry> {
            let mut out: Vec<&JaEntry> = self
                .words
                .iter()
                .filter(|entry| {
                    let units: Vec<char> = entry.reading.chars().collect();
                    units.len() >= reading.len() && units[..reading.len()] == reading[..]
                })
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

    fn test_entry(surface: &str, cost: i32, reading: &str) -> JaEntry {
        JaEntry {
            surface: surface.into(),
            reading: reading.into(),
            pos: Arc::from("名詞,普通名詞,一般,*"),
            conj_type: Arc::from("*"),
            conj_form: Arc::from("*"),
            cost,
            split: Split::C,
        }
    }

    fn test_decoder(entries: Vec<JaEntry>) -> JaDecoder {
        let longest = entries.iter().map(JaEntry::units).max().unwrap_or(0);
        JaDecoder::new(Arc::new(TestSource {
            words: entries,
            longest,
        }))
    }

    fn convert_surface(converter: &Converter<'_>, reading: &str, left: &str) -> String {
        let ctx = JaContext {
            left: left.to_string(),
            right: String::new(),
        };
        converter
            .convert(&chars(reading), &ctx)
            .unwrap_or_else(|| panic!("{reading} 沒有轉換結果"))
            .surface()
    }

    #[test]
    fn test_learned_selection_ranks_first() {
        let dec = test_decoder(vec![
            test_entry("今日", 1_000, "きょう"),
            test_entry("京", 1_500, "きょう"),
        ]);
        let store = Arc::new(Store::new());
        let learner = Learner::new(Some(Arc::clone(&store)));
        let converter = Converter::new(&dec, Some(&learner));
        let reading = "きょう";

        assert_eq!(convert_surface(&converter, reading, ""), "今日");
        let clause = [JaClause {
            reading: reading.to_string(),
            surface: "京".to_string(),
            segments: Vec::new(),
            cost: 1_500,
        }];
        assert_eq!(learner.learn_selection(&clause, ""), 1);
        assert_eq!(convert_surface(&converter, reading, ""), "京");
    }

    #[test]
    fn test_different_left_context_does_not_reuse_learning() {
        let dec = test_decoder(vec![
            test_entry("今日", 1_000, "きょう"),
            test_entry("京", 1_500, "きょう"),
        ]);
        let store = Arc::new(Store::new());
        let learner = Learner::new(Some(Arc::clone(&store)));
        let converter = Converter::new(&dec, Some(&learner));
        let reading = "きょう";

        let clause = [JaClause {
            reading: reading.to_string(),
            surface: "京".to_string(),
            segments: Vec::new(),
            cost: 1_500,
        }];
        learner.learn_selection(&clause, "今日は");
        assert_eq!(convert_surface(&converter, reading, "今日は"), "京");
        assert_eq!(
            convert_surface(&converter, reading, "空気が"),
            "今日",
            "換前文不該沿用學習"
        );
        assert_eq!(
            convert_surface(&converter, reading, ""),
            "今日",
            "句首不該沿用句中學習"
        );
    }

    #[test]
    fn test_learning_does_not_override_clear_statistical_winner() {
        let dec = test_decoder(vec![
            test_entry("今日", 1_000, "きょう"),
            test_entry("京", 10_000, "きょう"),
        ]);
        let store = Arc::new(Store::new());
        let learner = Learner::new(Some(Arc::clone(&store)));
        let converter = Converter::new(&dec, Some(&learner));
        let reading = "きょう";
        let clause = [JaClause {
            reading: reading.to_string(),
            surface: "京".to_string(),
            segments: Vec::new(),
            cost: 10_000,
        }];
        for _ in 0..8 {
            learner.learn_selection(&clause, "");
        }
        assert_eq!(
            convert_surface(&converter, reading, ""),
            "今日",
            "差距明顯時學習不該翻盤"
        );
        assert_eq!(
            store.lookup(&seq(&["き", "ょ", "う"]), "").unwrap().count,
            8
        );
    }

    #[test]
    fn test_candidates_put_learned_first() {
        let dec = test_decoder(vec![
            test_entry("今日", 1_000, "きょう"),
            test_entry("京", 1_500, "きょう"),
        ]);
        let store = Arc::new(Store::new());
        let learner = Learner::new(Some(Arc::clone(&store)));
        let converter = Converter::new(&dec, Some(&learner));
        let reading = chars("きょう");
        let span = JaSpan::new(0, 3);

        let cands = converter.candidates(&reading, span, &JaContext::default());
        assert!(cands.len() >= 2);
        assert_eq!(cands[0].surface, "今日");
        let clause = [JaClause {
            reading: "きょう".to_string(),
            surface: "京".to_string(),
            segments: Vec::new(),
            cost: 1_500,
        }];
        learner.learn_accepted(&clause, "");
        let cands = converter.candidates(&reading, span, &JaContext::default());
        assert!(cands.len() >= 2);
        assert_eq!(cands[0].surface, "京");
    }

    #[test]
    fn test_learning_sources_are_recorded() {
        enum Kind {
            Selection,
            Clause,
            Accept,
        }
        for (name, kind, want) in [
            ("selection", Kind::Selection, SOURCE_SELECTION),
            ("clause", Kind::Clause, SOURCE_CLAUSE),
            ("accept", Kind::Accept, SOURCE_ACCEPT),
        ] {
            let store = Arc::new(Store::new());
            let learner = Learner::new(Some(Arc::clone(&store)));
            let clause = [JaClause {
                reading: "きょう".to_string(),
                surface: "今日".to_string(),
                segments: Vec::new(),
                cost: 1_000,
            }];
            let n = match kind {
                Kind::Selection => learner.learn_selection(&clause, "前"),
                Kind::Clause => learner.learn_clause_change(&clause, "前"),
                Kind::Accept => learner.learn_accepted(&clause, "前"),
            };
            assert_eq!(n, 1, "{name}");
            let entry = store
                .lookup(&seq(&["き", "ょ", "う"]), "前")
                .unwrap_or_else(|| panic!("{name} 應有條目"));
            assert_eq!(entry.source, want, "{name}");
        }
    }

    #[test]
    fn test_learn_clauses_chains_left_context_and_segments() {
        let store = Arc::new(Store::new());
        let learner = Learner::new(Some(Arc::clone(&store)));
        let clauses = [
            JaClause {
                reading: "くうき".to_string(),
                surface: "空気".to_string(),
                segments: vec![JaClauseSegment {
                    reading: "くうき".to_string(),
                    surface: "空気".to_string(),
                    pos: None,
                }],
                cost: 1_000,
            },
            JaClause {
                reading: "が".to_string(),
                surface: "が".to_string(),
                segments: vec![JaClauseSegment {
                    reading: "が".to_string(),
                    surface: "が".to_string(),
                    pos: None,
                }],
                cost: 200,
            },
        ];
        assert_eq!(learner.learn_selection(&clauses, "今日"), 2);
        let first = store
            .lookup(&seq(&["く", "う", "き"]), "今日")
            .expect("第一個文節");
        assert_eq!(first.surface, "空気");
        assert_eq!(first.reading, vec!["くうき".to_string()]);
        let second = store.lookup(&seq(&["が"]), "空気").expect("第二個文節");
        assert_eq!(second.surface, "が");
        assert_eq!(second.segments, vec!["が".to_string()]);
        assert!(learner.score(&clauses, "今日") > 0.0);
        assert!(
            learner.score(&clauses, "空気が") > 0.0,
            "後續文節的前文仍是空気"
        );
    }

    #[test]
    fn test_cost_only_at_sentence_start_with_matching_context() {
        let store = Arc::new(Store::new());
        let learner = Learner::new(Some(Arc::clone(&store)));
        let clause = [JaClause {
            reading: "きょう".to_string(),
            surface: "京".to_string(),
            segments: Vec::new(),
            cost: 1_500,
        }];
        learner.learn_selection(&clause, "空気が");
        let e = test_entry("京", 1_500, "きょう");
        let reading = chars("きょう");
        let matching = JaContext {
            left: "空気が".to_string(),
            right: String::new(),
        };
        assert!(
            learner.cost(&reading, 0, &e, &matching) < 0,
            "同前文成本應為負值"
        );
        let other = JaContext {
            left: "今日は".to_string(),
            right: String::new(),
        };
        assert_eq!(learner.cost(&reading, 0, &e, &other), 0);
        assert_eq!(learner.cost(&reading, 1, &e, &matching), 0);
        let other_entry = test_entry("今日", 1_000, "きょう");
        assert_eq!(learner.cost(&reading, 0, &other_entry, &matching), 0);
    }

    #[test]
    fn test_learn_clauses_at_records_time() {
        let store = Arc::new(Store::new());
        let learner = Learner::new(Some(Arc::clone(&store)));
        let clause = [JaClause {
            reading: "あ".to_string(),
            surface: "亜".to_string(),
            segments: Vec::new(),
            cost: 1_000,
        }];
        assert_eq!(
            learner.learn_clauses_at(&clause, "", SOURCE_ACCEPT, 1_760_000_123),
            1
        );
        let entry = store.lookup(&seq(&["あ"]), "").expect("條目");
        assert_eq!(entry.last_used, 1_760_000_123);
    }

    #[test]
    fn test_learn_candidate_without_clauses() {
        let store = Arc::new(Store::new());
        let learner = Learner::new(Some(Arc::clone(&store)));
        let cand = JaCandidate {
            surface: "京".to_string(),
            reading: "きょう".to_string(),
            cost: 1_000,
            source: JaCandidateSource::Convert,
        };
        assert_eq!(learner.learn_candidate(&cand, "今日", SOURCE_CLAUSE), 1);
        let entry = store
            .lookup(&seq(&["き", "ょ", "う"]), "今日")
            .expect("條目");
        assert_eq!(entry.surface, "京");
        assert_eq!(entry.source, SOURCE_CLAUSE);
    }

    #[test]
    fn test_nil_store_learner_is_inert() {
        let learner = Learner::new(None);
        let clause = [JaClause {
            reading: "あ".to_string(),
            surface: "亜".to_string(),
            segments: Vec::new(),
            cost: 1_000,
        }];
        assert_eq!(learner.learn_selection(&clause, ""), 0);
        assert_eq!(learner.score(&clause, ""), 0.0);
        assert_eq!(
            learner.cost(
                &chars("あ"),
                0,
                &test_entry("亜", 1_000, "あ"),
                &JaContext::default()
            ),
            0
        );
    }

    #[test]
    fn test_converter_prefixes_delegate() {
        let dec = test_decoder(vec![
            test_entry("今日", 1_000, "きょう"),
            test_entry("今日は", 1_000, "きょうは"),
        ]);
        let store = Arc::new(Store::new());
        let learner = Learner::new(Some(Arc::clone(&store)));
        let converter = Converter::new(&dec, Some(&learner));
        let got = converter.prefixes(&chars("きょ"));
        assert!(!got.is_empty(), "預測前綴不該因學習包裝而消失");
    }

    #[test]
    fn test_last_used_zero_and_timestamps_are_provided_by_host() {
        // 核心不讀時鐘：`at` 為 0 時保留舊值，由宿主補上 Unix 秒。
        let store = Store::new();
        store.record(Selection {
            reading: seq(&["あ"]),
            surface: "亜".to_string(),
            at: 1_760_000_000,
            ..Selection::default()
        });
        store.record(Selection {
            reading: seq(&["あ"]),
            surface: "亜".to_string(),
            at: 0,
            ..Selection::default()
        });
        let entry = store.lookup(&seq(&["あ"]), "").expect("條目");
        assert_eq!(entry.count, 2);
        assert_eq!(entry.last_used, 1_760_000_000);
    }

    // ---- 共用 personal store 的日文學習 ----

    use crate::personal::{LanguageId, PersonalStore};

    fn clause(reading: &str, surface: &str, cost: i32) -> JaClause {
        JaClause {
            reading: reading.to_string(),
            surface: surface.to_string(),
            segments: Vec::new(),
            cost,
        }
    }

    #[test]
    fn test_unified_learning_records_into_shared_store() {
        let mut store = PersonalStore::new();
        let clauses = [clause("きょう", "京", 1_400)];
        assert_eq!(
            learn_unified(&mut store, &clauses, "", UNIFIED_MANUAL_WEIGHT, 10),
            1
        );
        assert_eq!(
            store.language_reading_count(LanguageId::Ja, "き ょ う", "京"),
            2
        );
        assert_eq!(store.language_context_count(LanguageId::Ja, "", "京"), 0);
        assert_eq!(store.language_selections(LanguageId::Ja), 2);
        // 注音命名空間不受影響。
        assert_eq!(store.language_selections(LanguageId::Zh), 0);

        let learning = UnifiedLearning::new(&store);
        let bonus = learning.clause_bonus(&clauses[0], "");
        assert!((bonus - 0.75 * 3.0_f64.log2() * UNIFIED_GLOBAL_FACTOR).abs() < 1e-9);
    }

    #[test]
    fn test_unified_bonus_is_capped_like_go() {
        let mut store = PersonalStore::new();
        let clauses = [clause("きょう", "京", 1_400)];
        for _ in 0..40 {
            learn_unified(&mut store, &clauses, "京", 1, 10);
        }
        let learning = UnifiedLearning::new(&store);
        assert_eq!(learning.clause_bonus(&clauses[0], "京"), DEFAULT_MAX_BOOST);
    }

    #[test]
    fn test_unified_learning_does_not_override_clear_statistical_winner() {
        let mut store = PersonalStore::new();
        let winner = clause("きょう", "今日", 1_000);
        let picked = clause("きょう", "きょう", 3_000);
        learn_unified(
            &mut store,
            std::slice::from_ref(&picked),
            "",
            UNIFIED_MANUAL_WEIGHT,
            10,
        );
        let learning = UnifiedLearning::new(&store);
        let mut results = vec![
            JaResult {
                clauses: vec![winner],
                cost: 1.0,
                score: -1.0,
            },
            JaResult {
                clauses: vec![picked],
                cost: 3.0,
                score: -3.0,
            },
        ];
        learning.rank_results(&mut results, "");
        assert_eq!(results[0].surface(), "今日");
    }

    #[test]
    fn test_unified_undo_restores_ranking() {
        let mut store = PersonalStore::new();
        let clauses = [clause("きょう", "京", 1_400)];
        learn_unified(&mut store, &clauses, "", UNIFIED_MANUAL_WEIGHT, 10);
        assert_eq!(store.language_selections(LanguageId::Ja), 2);
        assert!(store.undo_language_note());
        assert_eq!(store.language_selections(LanguageId::Ja), 0);
        assert!(!store.languages.contains_key("ja"));
        assert!(!store.undo_language_note());
    }

    #[test]
    fn test_unified_store_round_trips_json() {
        let dir = TempDir::new("unified-json");
        let path = dir.join("personal.json");
        let mut store = PersonalStore::new();
        learn_unified(
            &mut store,
            &[clause("きょう", "京", 1_400)],
            "",
            UNIFIED_MANUAL_WEIGHT,
            1_760_000_000,
        );
        store.save(&path).expect("寫出個人資料");
        let loaded = PersonalStore::load(&path).expect("讀回個人資料");
        assert_eq!(
            loaded.language_reading_count(LanguageId::Ja, "き ょ う", "京"),
            2
        );
        assert_eq!(loaded.language_surface_count(LanguageId::Ja, "京"), 2);
    }
}
