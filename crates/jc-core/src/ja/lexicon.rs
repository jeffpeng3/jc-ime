//! 日文詞典的查詢層：詞條、詞性與連接成本。
//!
//! 詞條本身由 [`jc_data::ja::Pack`]（`0CJADIC\0` v1）解碼；這一層把 pack 的
//! POS／活用索引換成字串，整理成 [`JaEntry`]，並提供依詞面查詢的介面。
//! 詞面與讀音的排序完全沿用 pack：entries 依 surface 的 Unicode code point
//! 排列，同一 surface 的詞條連續。
//!
//! 成本沿用 Mozc／MeCab 的語意，越小越優先；pack 的 `i16` 原值直接放大成
//! `i32`，不做任何縮放。連接成本用同一個尺度（Mozc 原始整數），curated 的
//! `ja-src/connection.tsv` 以小數記錄，讀入時乘上 1000 對齊。
//!
//! 完整的 context ID 連接矩陣不在這個階段：pack v1 沒有 left/right context
//! ID，只有 [`ConnectionCost`] 這個 trait 先凍結介面。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use jc_data::ja::{Pack, PackError};
use thiserror::Error;

pub use jc_data::ja::Split;

/// 未知詞的單一かな成本，對齊前身的 `decode.DefaultUnknownCost = 8.0`。
///
/// 夠高讓已知詞優先，但仍能轉換詞典沒有收錄的かな。
pub const UNKNOWN_WORD_COST: i32 = 8_000;

/// 連接成本表沒有指定詞性對時的保底值（前身 `DefaultConnectionCost = 1.0`）。
pub const DEFAULT_CONNECTION_COST: i32 = 1_000;

/// 詞成本合法下限，與 pack 的 `i16` 範圍一致。
pub const MIN_WORD_COST: i32 = -32_767;

/// 詞成本合法上限，與 pack 的 `i16` 範圍一致。
pub const MAX_WORD_COST: i32 = 32_767;

/// 未知詞固定使用的 split 類別。
///
/// pack 的 split 只用來標記輸出粒度，查詢層原樣保留；解碼器的 fallback
/// 沒有來源資訊，一律視為長單位 C。
pub const UNKNOWN_SPLIT: Split = Split::C;

/// 日文詞典層的錯誤。
#[derive(Debug, Error)]
pub enum JaError {
    /// pack 開啟或驗證失敗。
    #[error("日文詞典 pack：{0}")]
    Pack(#[from] PackError),
    /// 讀取外部檔案失敗。
    #[error("讀取 {path} 失敗：{source}")]
    Io {
        /// 檔案路徑。
        path: PathBuf,
        /// 底層 I/O 錯誤。
        #[source]
        source: std::io::Error,
    },
    /// pack 內的索引指向不存在的 POS／活用字串。
    #[error("日文詞典 pack 內容不一致：{0}")]
    BrokenPack(String),
    /// 連接成本 TSV 的某一行不合法。
    #[error("連接成本第 {line} 行：{reason}")]
    BadConnectionLine {
        /// 1 起算的行號。
        line: usize,
        /// 錯誤原因。
        reason: String,
    },
}

/// 連接成本使用的粗詞性。
///
/// pack 的 POS 是 UniDic／IPADic 的細分類（例如
/// `名詞,普通名詞,一般,*`），curated 的連接成本表用粗分類。兩者靠
/// [`JaPosClass::classify`] 對應。`Bos` 與 `Eos` 只出現在連接成本表，
/// 不會是詞條的詞性。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JaPosClass {
    /// 其他。
    Other,
    /// 名詞。
    Noun,
    /// 動詞。
    Verb,
    /// 形容詞。
    Adjective,
    /// 形容動詞語幹（形状詞）。
    AdjectivalNoun,
    /// 副詞。
    Adverb,
    /// 助詞。
    Particle,
    /// 助動詞。
    Auxiliary,
    /// 接続詞。
    Conjunction,
    /// 接頭語。
    Prefix,
    /// 接尾語。
    Suffix,
    /// 連体詞。
    Determiner,
    /// 感動詞。
    Interjection,
    /// 数詞。
    Number,
    /// 記号。
    Symbol,
    /// 句首（`<s>`），只在連接成本表使用。
    Bos,
    /// 句尾（`</s>`），只在連接成本表使用。
    Eos,
}

impl JaPosClass {
    /// 把連接成本表的名稱轉成粗詞性；不認得的名稱回傳 `None`。
    ///
    /// 名稱沿用前身：`other`、`noun`、`verb`、`adjective`、
    /// `adjectival_noun`、`adverb`、`particle`、`auxiliary`、
    /// `conjunction`、`prefix`、`suffix`、`determiner`、
    /// `interjection`、`number`、`symbol`、`<s>`、`</s>`。
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "other" => Self::Other,
            "noun" => Self::Noun,
            "verb" => Self::Verb,
            "adjective" => Self::Adjective,
            "adjectival_noun" => Self::AdjectivalNoun,
            "adverb" => Self::Adverb,
            "particle" => Self::Particle,
            "auxiliary" => Self::Auxiliary,
            "conjunction" => Self::Conjunction,
            "prefix" => Self::Prefix,
            "suffix" => Self::Suffix,
            "determiner" => Self::Determiner,
            "interjection" => Self::Interjection,
            "number" => Self::Number,
            "symbol" => Self::Symbol,
            "<s>" => Self::Bos,
            "</s>" => Self::Eos,
            _ => return None,
        })
    }

    /// 連接成本表使用的名稱。
    pub fn name(self) -> &'static str {
        match self {
            Self::Other => "other",
            Self::Noun => "noun",
            Self::Verb => "verb",
            Self::Adjective => "adjective",
            Self::AdjectivalNoun => "adjectival_noun",
            Self::Adverb => "adverb",
            Self::Particle => "particle",
            Self::Auxiliary => "auxiliary",
            Self::Conjunction => "conjunction",
            Self::Prefix => "prefix",
            Self::Suffix => "suffix",
            Self::Determiner => "determiner",
            Self::Interjection => "interjection",
            Self::Number => "number",
            Self::Symbol => "symbol",
            Self::Bos => "<s>",
            Self::Eos => "</s>",
        }
    }

    /// 由 pack 的 POS 字串（可含逗號細分類）判斷粗詞性。
    ///
    /// UniDic 與 IPADic 都以第一個逗號欄位表示大分類，所以同一個函式可以
    /// 處理兩套 POS；英文粗分類名稱也直接接受，方便測試與 curated 資料。
    /// 不認得的值回傳 [`JaPosClass::Other`]。
    pub fn classify(pos: &str) -> Self {
        if pos == "<s>" {
            return Self::Bos;
        }
        if pos == "</s>" {
            return Self::Eos;
        }
        if let Some(class) = Self::from_name(pos) {
            return class;
        }
        let mut fields = pos.split(',');
        let head = fields.next().unwrap_or("");
        let sub = fields.next().unwrap_or("");
        match head {
            "名詞" => match sub {
                "数詞" => Self::Number,
                "接尾" => Self::Suffix,
                "形容動詞語幹" => Self::AdjectivalNoun,
                "ナイ形容詞語幹" => Self::Adjective,
                _ => Self::Noun,
            },
            "代名詞" => Self::Noun,
            "動詞" => Self::Verb,
            "形容詞" => Self::Adjective,
            "形状詞" => Self::AdjectivalNoun,
            "副詞" => Self::Adverb,
            "助詞" => Self::Particle,
            "助動詞" => Self::Auxiliary,
            "接続詞" => Self::Conjunction,
            "接頭辞" | "接頭詞" => Self::Prefix,
            "接尾辞" => Self::Suffix,
            "連体詞" => Self::Determiner,
            "感動詞" | "フィラー" => Self::Interjection,
            "記号" | "補助記号" => Self::Symbol,
            _ => Self::Other,
        }
    }
}

/// 一筆日文詞條。
///
/// `pos`、`conj_type`、`conj_form` 是 pack 查表後的字串；相同的值共用一個
/// [`Arc`]，所以大量詞條不會各自複製一份詞性字串。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JaEntry {
    /// 詞面。
    pub surface: Box<str>,
    /// 讀音（平假名，一個字元一個單位）。
    pub reading: Box<str>,
    /// pack 的原始 POS 字串，例如 `名詞,普通名詞,一般,*`。
    pub pos: Arc<str>,
    /// 活用型，例如 `五段-マ行`；無活用時是 `*`。
    pub conj_type: Arc<str>,
    /// 活用形，例如 `終止形-一般`；無活用時是 `*`。
    pub conj_form: Arc<str>,
    /// 詞成本，越小越優先；範圍 [`MIN_WORD_COST`]～[`MAX_WORD_COST`]。
    pub cost: i32,
    /// 來源的 split 類別（C／A／B）。
    pub split: Split,
}

impl JaEntry {
    /// 這筆詞條的粗詞性，供連接成本查表。
    pub fn pos_class(&self) -> JaPosClass {
        JaPosClass::classify(&self.pos)
    }

    /// 讀音單位數（一個かな一個單位）。
    pub fn units(&self) -> usize {
        self.reading.chars().count()
    }
}

/// 依詞面查詢的日文詞典。
///
/// 內部保留 pack 以取得原始資料與驗證結果，另外整理一份 [`JaEntry`]，
/// 讓 `lookup` 可以回傳連續切片。
pub struct JaLexicon {
    pack: Pack,
    entries: Vec<JaEntry>,
    longest_reading_units: usize,
}

impl std::fmt::Debug for JaLexicon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JaLexicon")
            .field("entries", &self.entries.len())
            .field("longest_reading_units", &self.longest_reading_units)
            .finish()
    }
}

impl JaLexicon {
    /// 開啟 pack 並建立查詢層。
    pub fn open(path: impl AsRef<Path>) -> Result<Self, JaError> {
        let pack = Pack::open(path)?;
        let mut entries = Vec::with_capacity(pack.entry_count());

        let pos: Vec<Arc<str>> = pack
            .pos_list()
            .iter()
            .map(|s| Arc::from(s.as_str()))
            .collect();
        let conj_types: Vec<Arc<str>> = pack
            .conj_types()
            .iter()
            .map(|s| Arc::from(s.as_str()))
            .collect();
        let conj_forms: Vec<Arc<str>> = pack
            .conj_forms()
            .iter()
            .map(|s| Arc::from(s.as_str()))
            .collect();

        let mut longest_reading_units = 0usize;
        for entry in pack.entries() {
            let pos_str = pos.get(entry.pos_id as usize).ok_or_else(|| {
                JaError::BrokenPack(format!("POS id {} 超出 table", entry.pos_id))
            })?;
            let conj_type = conj_types.get(entry.conj_type as usize).ok_or_else(|| {
                JaError::BrokenPack(format!("活用型 id {} 超出 table", entry.conj_type))
            })?;
            let conj_form = conj_forms.get(entry.conj_form as usize).ok_or_else(|| {
                JaError::BrokenPack(format!("活用形 id {} 超出 table", entry.conj_form))
            })?;
            let units = entry.reading.chars().count();
            longest_reading_units = longest_reading_units.max(units);
            entries.push(JaEntry {
                surface: entry.surface.to_string().into_boxed_str(),
                reading: entry.reading.to_string().into_boxed_str(),
                pos: Arc::clone(pos_str),
                conj_type: Arc::clone(conj_type),
                conj_form: Arc::clone(conj_form),
                cost: i32::from(entry.cost),
                split: entry.split,
            });
        }

        Ok(Self {
            pack,
            entries,
            longest_reading_units,
        })
    }

    /// 底層 pack。
    pub fn pack(&self) -> &Pack {
        &self.pack
    }

    /// 詞條總數。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否沒有任何詞條。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 全部詞條，依 pack 的 surface 排序。
    pub fn entries(&self) -> &[JaEntry] {
        &self.entries
    }

    /// 回傳詞面完全相等的所有詞條，順序與 pack 相同。
    pub fn lookup(&self, surface: &str) -> &[JaEntry] {
        let start = self
            .entries
            .partition_point(|e| e.surface.as_ref() < surface);
        let rest = &self.entries[start..];
        let count = rest
            .iter()
            .take_while(|e| e.surface.as_ref() == surface)
            .count();
        &rest[..count]
    }

    /// 詞面是否存在。
    pub fn has(&self, surface: &str) -> bool {
        !self.lookup(surface).is_empty()
    }

    /// 回傳詞面以 `prefix` 開頭的所有詞條，依 surface 排序。
    ///
    /// 空字串回傳全部詞條，與 pack 的前綴語意一致。
    pub fn prefix(&self, prefix: &str) -> Vec<&JaEntry> {
        let start = self
            .entries
            .partition_point(|e| e.surface.as_ref() < prefix);
        self.entries[start..]
            .iter()
            .take_while(|e| e.surface.starts_with(prefix))
            .collect()
    }

    /// 最長詞條的讀音單位數。
    pub fn longest_reading_units(&self) -> usize {
        self.longest_reading_units
    }
}

/// 連接成本介面。
///
/// 成本越小越優先，單位與 pack 的詞成本相同（Mozc 原始整數）。
/// 之後的 context ID 矩陣也會實作這個 trait，呼叫端不需要改。
pub trait ConnectionCost {
    /// 回傳 `left` 接 `right` 的連接成本。
    fn cost(&self, left: JaPosClass, right: JaPosClass) -> i32;
}

/// 以粗詞性對為索引的連接成本表。
///
/// 名稱沿用任務書的「單欄」：每一組（左詞性, 右詞性）只有一個成本值。
/// 完整 context ID 矩陣等 pack 帶 context ID 之後再接。
#[derive(Debug, Clone)]
pub struct SingleColumnCost {
    costs: HashMap<(JaPosClass, JaPosClass), i32>,
    fallback: i32,
}

impl SingleColumnCost {
    /// 建立空表，未指定的詞性對使用 [`DEFAULT_CONNECTION_COST`]。
    pub fn new() -> Self {
        Self {
            costs: HashMap::new(),
            fallback: DEFAULT_CONNECTION_COST,
        }
    }

    /// 讀取連接成本 TSV：左詞性、右詞性、成本。
    ///
    /// 以 `#` 起始的行與空行略過。成本是小數（`0.3` 代表 300），讀入時
    /// 乘上 1000 對齊 pack 的整數成本；不可為負、不可為 NaN／Inf。
    pub fn from_tsv(tsv: &str) -> Result<Self, JaError> {
        let mut table = Self::new();
        for (index, raw) in tsv.lines().enumerate() {
            let line = raw.trim_end_matches('\r');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line_no = index + 1;
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() != 3 {
                return Err(JaError::BadConnectionLine {
                    line: line_no,
                    reason: format!("需要左詞性、右詞性、成本三欄，得到 {} 欄", fields.len()),
                });
            }
            let left =
                JaPosClass::from_name(fields[0]).ok_or_else(|| JaError::BadConnectionLine {
                    line: line_no,
                    reason: format!("未知詞性 {:?}", fields[0]),
                })?;
            let right =
                JaPosClass::from_name(fields[1]).ok_or_else(|| JaError::BadConnectionLine {
                    line: line_no,
                    reason: format!("未知詞性 {:?}", fields[1]),
                })?;
            let value: f64 = fields[2].parse().map_err(|_| JaError::BadConnectionLine {
                line: line_no,
                reason: format!("成本 {:?} 不是數字", fields[2]),
            })?;
            if !value.is_finite() || value < 0.0 {
                return Err(JaError::BadConnectionLine {
                    line: line_no,
                    reason: format!("成本 {:?} 不合法", fields[2]),
                });
            }
            let scaled = (value * 1000.0).round();
            if scaled > i32::MAX as f64 {
                return Err(JaError::BadConnectionLine {
                    line: line_no,
                    reason: format!("成本 {:?} 超出範圍", fields[2]),
                });
            }
            table.set(left, right, scaled as i32);
        }
        Ok(table)
    }

    /// 從檔案讀取連接成本 TSV。
    pub fn load(path: impl AsRef<Path>) -> Result<Self, JaError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| JaError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_tsv(&text)
    }

    /// 設定一組詞性對的成本。
    pub fn set(&mut self, left: JaPosClass, right: JaPosClass, cost: i32) {
        self.costs.insert((left, right), cost);
    }

    /// 未指定詞性對的保底成本。
    pub fn fallback(&self) -> i32 {
        self.fallback
    }

    /// 設定未指定詞性對的保底成本。
    pub fn set_fallback(&mut self, cost: i32) {
        self.fallback = cost;
    }

    /// 已設定的詞性對數量。
    pub fn len(&self) -> usize {
        self.costs.len()
    }

    /// 是否沒有設定任何詞性對。
    pub fn is_empty(&self) -> bool {
        self.costs.is_empty()
    }
}

impl Default for SingleColumnCost {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectionCost for SingleColumnCost {
    fn cost(&self, left: JaPosClass, right: JaPosClass) -> i32 {
        self.costs
            .get(&(left, right))
            .copied()
            .unwrap_or(self.fallback)
    }
}

/// 未知詞的總成本：每個讀音單位 [`UNKNOWN_WORD_COST`]。
pub fn unknown_word_cost(units: usize) -> i32 {
    UNKNOWN_WORD_COST.saturating_mul(units.min(i32::MAX as usize) as i32)
}

/// 建立一筆未知詞，詞面與讀音都是同一個かな單位。
///
/// 詞性固定是 [`JaPosClass::Other`]、split 固定是 [`UNKNOWN_SPLIT`]。
pub fn unknown_entry(unit: &str) -> JaEntry {
    JaEntry {
        surface: unit.into(),
        reading: unit.into(),
        pos: Arc::from(JaPosClass::Other.name()),
        conj_type: Arc::from("*"),
        conj_form: Arc::from("*"),
        cost: UNKNOWN_WORD_COST,
        split: UNKNOWN_SPLIT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc_data::ja::{ENTRY_SIZE, HEADER_SIZE, MAGIC, TRAILER_SIZE};
    use sha2::{Digest, Sha256};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// 測試平行執行時，每次寫檔都要有獨立路徑。
    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    /// 測試用的一筆詞條。
    struct RawEntry {
        surface: &'static str,
        reading: &'static str,
        pos: &'static str,
        conj_type: &'static str,
        conj_form: &'static str,
        cost: i16,
        split: u16,
    }

    /// 依 pack 格式產生最小合法檔案，讓查詢層的測試不必依賴外部資料。
    fn build_pack(mut entries: Vec<RawEntry>) -> Vec<u8> {
        // entries 必須依 surface 排序，同一 surface 連續。
        entries.sort_by(|a, b| a.surface.cmp(b.surface));

        let mut surfaces: Vec<&str> = Vec::new();
        for entry in &entries {
            if surfaces.last().copied() != Some(entry.surface) {
                surfaces.push(entry.surface);
            }
        }

        let mut pos_table: Vec<&str> = entries.iter().map(|e| e.pos).collect();
        pos_table.sort_unstable();
        pos_table.dedup();
        let mut conj_type_table: Vec<&str> = entries.iter().map(|e| e.conj_type).collect();
        conj_type_table.sort_unstable();
        conj_type_table.dedup();
        let mut conj_form_table: Vec<&str> = entries.iter().map(|e| e.conj_form).collect();
        conj_form_table.sort_unstable();
        conj_form_table.dedup();

        let pos_id = |value: &str| pos_table.iter().position(|v| *v == value).unwrap() as u16;
        let conj_type_id =
            |value: &str| conj_type_table.iter().position(|v| *v == value).unwrap() as u8;
        let conj_form_id =
            |value: &str| conj_form_table.iter().position(|v| *v == value).unwrap() as u8;

        let mut surface_blob = Vec::new();
        let mut surface_offsets = Vec::new();
        for surface in &surfaces {
            surface_offsets.push(surface_blob.len() as u32);
            surface_blob.extend_from_slice(surface.as_bytes());
        }

        let mut reading_blob = Vec::new();
        let mut reading_ranges = Vec::new();
        for entry in &entries {
            let start = reading_blob.len() as u32;
            reading_blob.extend_from_slice(entry.reading.as_bytes());
            reading_ranges.push((start, entry.reading.len() as u16));
        }

        let mut entry_bytes = Vec::with_capacity(entries.len() * ENTRY_SIZE);
        for (index, entry) in entries.iter().enumerate() {
            let surface_offset =
                surface_offsets[surfaces.iter().position(|s| *s == entry.surface).unwrap()];
            entry_bytes.extend_from_slice(&surface_offset.to_le_bytes());
            entry_bytes.extend_from_slice(&(entry.surface.len() as u16).to_le_bytes());
            entry_bytes.extend_from_slice(&reading_ranges[index].0.to_le_bytes());
            entry_bytes.extend_from_slice(&reading_ranges[index].1.to_le_bytes());
            entry_bytes.extend_from_slice(&pos_id(entry.pos).to_le_bytes());
            entry_bytes.push(conj_type_id(entry.conj_type));
            entry_bytes.push(conj_form_id(entry.conj_form));
            entry_bytes.extend_from_slice(&entry.cost.to_le_bytes());
            entry_bytes.extend_from_slice(&entry.split.to_le_bytes());
        }

        let mut index_bytes = Vec::with_capacity(surfaces.len() * 8);
        let mut first_entry = 0u32;
        for (i, surface) in surfaces.iter().enumerate() {
            index_bytes.extend_from_slice(&surface_offsets[i].to_le_bytes());
            index_bytes.extend_from_slice(&first_entry.to_le_bytes());
            first_entry += entries.iter().filter(|e| e.surface == *surface).count() as u32;
        }

        let mut pos_bytes = Vec::new();
        pos_bytes.extend_from_slice(&(pos_table.len() as u16).to_le_bytes());
        for value in &pos_table {
            pos_bytes.extend_from_slice(&(value.len() as u16).to_le_bytes());
            pos_bytes.extend_from_slice(value.as_bytes());
        }

        let mut conj_bytes = Vec::new();
        conj_bytes.extend_from_slice(&(conj_type_table.len() as u16).to_le_bytes());
        for value in &conj_type_table {
            conj_bytes.extend_from_slice(&(value.len() as u16).to_le_bytes());
            conj_bytes.extend_from_slice(value.as_bytes());
        }
        conj_bytes.extend_from_slice(&(conj_form_table.len() as u16).to_le_bytes());
        for value in &conj_form_table {
            conj_bytes.extend_from_slice(&(value.len() as u16).to_le_bytes());
            conj_bytes.extend_from_slice(value.as_bytes());
        }

        let surface_offset = HEADER_SIZE as u32;
        let reading_offset = surface_offset + surface_blob.len() as u32;
        let entries_offset = reading_offset + reading_blob.len() as u32;
        let index_offset = entries_offset + entry_bytes.len() as u32;
        let pos_offset = index_offset + index_bytes.len() as u32;
        let conj_offset = pos_offset + pos_bytes.len() as u32;

        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&(HEADER_SIZE as u16).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        out.extend_from_slice(&(surfaces.len() as u32).to_le_bytes());
        out.extend_from_slice(&(pos_table.len() as u16).to_le_bytes());
        out.extend_from_slice(&(conj_type_table.len() as u16).to_le_bytes());
        out.extend_from_slice(&(conj_form_table.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        for (offset, length) in [
            (surface_offset, surface_blob.len()),
            (reading_offset, reading_blob.len()),
            (entries_offset, entry_bytes.len()),
            (index_offset, index_bytes.len()),
            (pos_offset, pos_bytes.len()),
            (conj_offset, conj_bytes.len()),
        ] {
            out.extend_from_slice(&offset.to_le_bytes());
            out.extend_from_slice(&(length as u32).to_le_bytes());
        }
        assert_eq!(out.len(), HEADER_SIZE);
        out.extend_from_slice(&surface_blob);
        out.extend_from_slice(&reading_blob);
        out.extend_from_slice(&entry_bytes);
        out.extend_from_slice(&index_bytes);
        out.extend_from_slice(&pos_bytes);
        out.extend_from_slice(&conj_bytes);

        let digest = Sha256::digest(&out);
        out.extend_from_slice(&digest);
        assert!(out.len() > HEADER_SIZE + TRAILER_SIZE);
        out
    }

    fn write_temp(tag: &str, bytes: &[u8]) -> PathBuf {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "jc-core-ja-lexicon-{tag}-{}-{seq}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lexicon.pack");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn entry(
        surface: &'static str,
        reading: &'static str,
        pos: &'static str,
        cost: i16,
    ) -> RawEntry {
        RawEntry {
            surface,
            reading,
            pos,
            conj_type: "*",
            conj_form: "*",
            cost,
            split: 0,
        }
    }

    fn test_lexicon() -> JaLexicon {
        let pack = build_pack(vec![
            entry("あい", "あい", "名詞,普通名詞,一般,*", 11),
            entry("あ", "あ", "助詞,格助詞,*,*", 3),
            entry("明日", "あした", "名詞,普通名詞,副詞可能,*", 22),
            entry("明日", "あす", "名詞,普通名詞,副詞可能,*", 12),
            entry("ア", "あ", "記号,一般,*,*", 5),
        ]);
        let path = write_temp("lookup", &pack);
        let lex = JaLexicon::open(&path).unwrap();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        lex
    }

    #[test]
    fn test_open_and_lookup_homographs() {
        let lex = test_lexicon();
        assert_eq!(lex.len(), 5);
        assert_eq!(lex.pack().entry_count(), 5);

        let ashita = lex.lookup("明日");
        assert_eq!(ashita.len(), 2);
        assert_eq!(ashita[0].reading.as_ref(), "あした");
        assert_eq!(ashita[0].cost, 22);
        assert_eq!(ashita[0].pos.as_ref(), "名詞,普通名詞,副詞可能,*");
        assert_eq!(ashita[0].pos_class(), JaPosClass::Noun);
        assert_eq!(ashita[0].units(), 3);
        assert_eq!(ashita[0].split, Split::C);

        assert!(lex.lookup("存在しない").is_empty());
        assert!(!lex.has("存在しない"));
        assert!(lex.has("明日"));
    }

    #[test]
    fn test_prefix_follows_pack_surface_order() {
        let lex = test_lexicon();

        let surfaces: Vec<&str> = lex
            .prefix("あ")
            .iter()
            .map(|e| e.surface.as_ref())
            .collect();
        assert_eq!(surfaces, ["あ", "あい"]);

        let surfaces: Vec<&str> = lex
            .prefix("明")
            .iter()
            .map(|e| e.surface.as_ref())
            .collect();
        assert_eq!(surfaces, ["明日", "明日"]);

        assert!(lex.prefix("い").is_empty());
        assert_eq!(lex.prefix("").len(), 5);
    }

    #[test]
    fn test_longest_reading_units() {
        let lex = test_lexicon();
        assert_eq!(lex.longest_reading_units(), 3);
    }

    #[test]
    fn test_open_rejects_corrupt_pack() {
        let mut bytes = build_pack(vec![entry("あ", "あ", "名詞,一般,*,*", 1)]);
        bytes[..8].copy_from_slice(b"BROKEN\0\0");
        let path = write_temp("corrupt", &bytes);
        let err = JaLexicon::open(&path).expect_err("壞 magic 應開啟失敗");
        assert!(matches!(err, JaError::Pack(PackError::BadMagic)));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn test_pos_classify_unicdic_and_ipadic() {
        let cases = [
            ("名詞,普通名詞,一般,*", JaPosClass::Noun),
            ("名詞,数詞,*,*", JaPosClass::Number),
            ("名詞,接尾,一般,*", JaPosClass::Suffix),
            ("名詞,形容動詞語幹,*,*", JaPosClass::AdjectivalNoun),
            ("名詞,ナイ形容詞語幹,*,*", JaPosClass::Adjective),
            ("代名詞,*,*,*", JaPosClass::Noun),
            ("動詞,一般,*,*", JaPosClass::Verb),
            ("形容詞,一般,*,*", JaPosClass::Adjective),
            ("形状詞,一般,*,*", JaPosClass::AdjectivalNoun),
            ("副詞,一般,*,*", JaPosClass::Adverb),
            ("助詞,格助詞,*,*", JaPosClass::Particle),
            ("助動詞,*,*,*", JaPosClass::Auxiliary),
            ("接続詞,*,*,*", JaPosClass::Conjunction),
            ("接頭辞,*,*,*", JaPosClass::Prefix),
            ("接頭詞,名詞接続,*", JaPosClass::Prefix),
            ("接尾辞,名詞的,一般,*", JaPosClass::Suffix),
            ("連体詞,*,*,*", JaPosClass::Determiner),
            ("感動詞,一般,*,*", JaPosClass::Interjection),
            ("フィラー,*,*,*", JaPosClass::Interjection),
            ("記号,一般,*,*", JaPosClass::Symbol),
            ("補助記号,一般,*,*", JaPosClass::Symbol),
            ("noun", JaPosClass::Noun),
            ("<s>", JaPosClass::Bos),
            ("</s>", JaPosClass::Eos),
            ("unknown", JaPosClass::Other),
        ];
        for (pos, want) in cases {
            assert_eq!(JaPosClass::classify(pos), want, "{pos}");
        }
        assert_eq!(JaPosClass::Noun.name(), "noun");
        assert_eq!(
            JaPosClass::from_name("adjectival_noun"),
            Some(JaPosClass::AdjectivalNoun)
        );
        assert_eq!(JaPosClass::from_name("nope"), None);
    }

    #[test]
    fn test_connection_from_tsv() {
        let table = SingleColumnCost::from_tsv(
            "# left\tright\tcost\nnoun\tnoun\t0.5\n<s>\tnoun\t0\nnoun\t</s>\t0.25\nnoun\tparticle\t0.3\n",
        )
        .unwrap();
        assert_eq!(table.len(), 4);
        assert_eq!(table.cost(JaPosClass::Noun, JaPosClass::Noun), 500);
        assert_eq!(table.cost(JaPosClass::Bos, JaPosClass::Noun), 0);
        assert_eq!(table.cost(JaPosClass::Noun, JaPosClass::Eos), 250);
        assert_eq!(table.cost(JaPosClass::Noun, JaPosClass::Particle), 300);
        assert_eq!(
            table.cost(JaPosClass::Verb, JaPosClass::Noun),
            DEFAULT_CONNECTION_COST
        );
        assert_eq!(table.fallback(), DEFAULT_CONNECTION_COST);
    }

    #[test]
    fn test_connection_rejects_bad_rows() {
        for (source, line) in [
            ("noun\tnoun\n", 1),
            ("noun\tnoun\tabc\n", 1),
            ("noun\tnoun\t-0.1\n", 1),
            ("noun\tnoun\tNaN\n", 1),
            ("nope\tnoun\t0.5\n", 1),
            ("noun\tnope\t0.5\n", 1),
            ("# ok\nnoun\tnoun\t0.5\nnoun\n", 3),
        ] {
            let err = SingleColumnCost::from_tsv(source).expect_err(source);
            match err {
                JaError::BadConnectionLine { line: got, .. } => assert_eq!(got, line),
                other => panic!("預期 BadConnectionLine，得到 {other:?}"),
            }
        }
    }

    #[test]
    fn test_connection_loads_curated_file() {
        let path = jc_testdata::require_data!("ja-src/connection.tsv");
        let table = SingleColumnCost::load(&path).unwrap();
        assert_eq!(table.len(), 82);
        assert_eq!(table.cost(JaPosClass::Noun, JaPosClass::Particle), 300);
        assert_eq!(table.cost(JaPosClass::Verb, JaPosClass::Auxiliary), 300);
        assert_eq!(table.cost(JaPosClass::Symbol, JaPosClass::Symbol), 500);
        assert_eq!(table.cost(JaPosClass::Bos, JaPosClass::Noun), 200);
        assert_eq!(table.cost(JaPosClass::Adjective, JaPosClass::Noun), 1_000);
    }

    #[test]
    fn test_unknown_word_cost_and_entry() {
        assert_eq!(unknown_word_cost(1), 8_000);
        assert_eq!(unknown_word_cost(3), 24_000);

        let e = unknown_entry("あ");
        assert_eq!(e.surface.as_ref(), "あ");
        assert_eq!(e.reading.as_ref(), "あ");
        assert_eq!(e.cost, UNKNOWN_WORD_COST);
        assert_eq!(e.pos_class(), JaPosClass::Other);
        assert_eq!(e.split, Split::C);
        assert_eq!(e.units(), 1);
    }

    #[test]
    fn test_real_pack_lookup_prefix_and_pos() {
        let path = jc_testdata::require_data!("packs/ja-lexicon-public-core.pack");
        let lex = JaLexicon::open(&path).unwrap();
        assert_eq!(lex.len(), 1_147_021);
        assert_eq!(lex.pack().entry_count(), 1_147_021);

        let watashi = lex.lookup("私");
        assert_eq!(watashi.len(), 4);
        let found = watashi
            .iter()
            .find(|e| e.reading.as_ref() == "わたし")
            .expect("私 わたし");
        assert_eq!(found.cost, 7_159);
        assert_eq!(found.pos.as_ref(), "代名詞,*,*,*");
        assert_eq!(found.pos_class(), JaPosClass::Noun);
        assert_eq!(found.split, Split::C);

        let hon = lex.lookup("本");
        assert_eq!(hon.len(), 7);
        let found = hon
            .iter()
            .find(|e| e.pos.as_ref() == "名詞,普通名詞,一般,*")
            .expect("本 名詞");
        assert_eq!(found.reading.as_ref(), "ほん");
        assert_eq!(found.cost, 8_974);

        let yomu = lex.lookup("読む");
        assert_eq!(yomu.len(), 2);
        let found = yomu
            .iter()
            .find(|e| e.conj_form.as_ref() == "終止形-一般")
            .expect("読む 終止形");
        assert_eq!(found.reading.as_ref(), "よむ");
        assert_eq!(found.cost, 12_306);
        assert_eq!(found.conj_type.as_ref(), "五段-マ行");
        assert_eq!(found.pos_class(), JaPosClass::Verb);

        let nihongo = lex.prefix("日本");
        assert!(nihongo.len() > 1);
        assert!(nihongo.iter().all(|e| e.surface.starts_with("日本")));
        let found = nihongo
            .iter()
            .find(|e| e.surface.as_ref() == "日本語" && e.reading.as_ref() == "にほんご")
            .expect("日本語 にほんご");
        assert_eq!(found.cost, 276);
        assert!(!nihongo.iter().any(|e| e.surface.as_ref() == "日"));

        let negative = lex.lookup("これから");
        assert_eq!(negative.len(), 1);
        assert_eq!(negative[0].cost, -490);
    }
}
