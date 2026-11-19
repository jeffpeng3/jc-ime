//! 0CJADIC v1／v2 pack 的零拷貝讀取與驗證。
//!
//! 檔案用 `memmap2` 映射，結構驗證與字串解碼都在開啟時完成一次；
//! 之後 `lookup`／`prefix` 只做二分搜尋與切片。
//!
//! v2 在 v1 之上加了每筆詞條的 left／right context id 與連接成本矩陣，
//! v1 仍然完整支援。格式定義見
//! `jc-ime-data/plan/reference/pack-format.md` 與 `pack-v2-spec.md`。

use std::fs::File;
use std::path::Path;

use memmap2::Mmap;
use sha2::{Digest, Sha256};

use super::error::PackError;

/// pack magic。
pub const MAGIC: [u8; 8] = *b"0CJADIC\0";
/// v1 格式版本；既有呼叫端仍以這個常數代表「基準格式」。
pub const FORMAT_VERSION: u16 = 1;
/// v2 格式版本。
pub const FORMAT_VERSION_V2: u16 = 2;
/// v1 固定檔頭大小。
pub const HEADER_SIZE: usize = 80;
/// v2 固定檔頭大小。
pub const HEADER_SIZE_V2: usize = 96;
/// v1 固定詞條大小。
pub const ENTRY_SIZE: usize = 20;
/// v2 固定詞條大小。
pub const ENTRY_SIZE_V2: usize = 24;
/// v1 surface index 每筆大小。
pub const INDEX_ENTRY_SIZE: usize = 8;
/// 檔尾 payload SHA-256 大小。
pub const TRAILER_SIZE: usize = 32;
/// 詞成本合法下限（-32768 保留）。
pub const MIN_COST: i16 = -32767;

/// pack 檔頭。v1 是 80 位元組，v2 是 96 位元組；所有 offset 從檔案開頭算起。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// 格式版本。
    pub format_version: u16,
    /// 檔頭大小。
    pub header_size: u16,
    /// 保留旗標。
    pub flags: u32,
    /// 詞條數。
    pub entry_count: u32,
    /// 唯一 surface 數。
    pub surface_count: u32,
    /// POS table 筆數。
    pub pos_count: u16,
    /// 活用型筆數。
    pub conj_type_count: u16,
    /// 活用形筆數。
    pub conj_form_count: u16,
    /// context 數；v2 專用，v1 這個位置是保留欄位，必須是 0。
    pub context_count: u16,
    /// surface blob 起點。
    pub surface_blob_offset: u32,
    /// surface blob 長度。
    pub surface_blob_length: u32,
    /// reading blob 起點。
    pub reading_blob_offset: u32,
    /// reading blob 長度。
    pub reading_blob_length: u32,
    /// entries section 起點。
    pub entries_offset: u32,
    /// entries section 長度。
    pub entries_length: u32,
    /// surface index 起點。
    pub index_offset: u32,
    /// surface index 長度。
    pub index_length: u32,
    /// POS table 起點。
    pub pos_offset: u32,
    /// POS table 長度。
    pub pos_length: u32,
    /// 活用 table 起點。
    pub conj_offset: u32,
    /// 活用 table 長度。
    pub conj_length: u32,
    /// 連接矩陣起點；v2 專用，v1 是 0。
    pub connection_offset: u32,
    /// 連接矩陣長度；v2 專用，v1 是 0。
    pub connection_length: u32,
    /// v2 保留欄位，必須都是 0。
    pub reserved: [u32; 2],
}

impl Header {
    /// 依版本解析檔頭；`b` 至少要有該版本的檔頭長度。
    fn parse(b: &[u8], version: u16) -> Header {
        let u16_at = |off: usize| u16::from_le_bytes([b[off], b[off + 1]]);
        let u32_at = |off: usize| u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]]);
        let mut header = Header {
            format_version: u16_at(8),
            header_size: u16_at(10),
            flags: u32_at(12),
            entry_count: u32_at(16),
            surface_count: u32_at(20),
            pos_count: u16_at(24),
            conj_type_count: u16_at(26),
            conj_form_count: u16_at(28),
            context_count: u16_at(30),
            surface_blob_offset: u32_at(32),
            surface_blob_length: u32_at(36),
            reading_blob_offset: u32_at(40),
            reading_blob_length: u32_at(44),
            entries_offset: u32_at(48),
            entries_length: u32_at(52),
            index_offset: u32_at(56),
            index_length: u32_at(60),
            pos_offset: u32_at(64),
            pos_length: u32_at(68),
            conj_offset: u32_at(72),
            conj_length: u32_at(76),
            connection_offset: 0,
            connection_length: 0,
            reserved: [0; 2],
        };
        if version == FORMAT_VERSION_V2 {
            header.connection_offset = u32_at(80);
            header.connection_length = u32_at(84);
            header.reserved = [u32_at(88), u32_at(92)];
        }
        header
    }
}

/// 詞條的 split 類別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Split {
    /// 0：C。
    C = 0,
    /// 1：A。
    A = 1,
    /// 2：B。
    B = 2,
}

impl Split {
    /// 由 flags 的低 2 位解碼；3 不是合法值。
    pub fn from_bits(bits: u16) -> Option<Split> {
        match bits & 0x3 {
            0 => Some(Split::C),
            1 => Some(Split::A),
            2 => Some(Split::B),
            _ => None,
        }
    }

    /// 回傳原始數值。
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// 一筆解碼後的詞條。成本越小越優先。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// surface 字串。
    pub surface: Box<str>,
    /// 讀音（平假名）。
    pub reading: Box<str>,
    /// POS table 的索引。
    pub pos_id: u16,
    /// 活用型索引。
    pub conj_type: u8,
    /// 活用形索引。
    pub conj_form: u8,
    /// 詞成本。
    pub cost: i16,
    /// split 類別。
    pub split: Split,
    /// 原始 flags。
    pub flags: u16,
    /// left context id；v1 詞條沒有這個欄位。
    left_id: Option<u16>,
    /// right context id；v1 詞條沒有這個欄位。
    right_id: Option<u16>,
}

impl Entry {
    /// left context id。v1 pack 回傳 `None`。
    pub fn left_id(&self) -> Option<u16> {
        self.left_id
    }

    /// right context id。v1 pack 回傳 `None`。
    pub fn right_id(&self) -> Option<u16> {
        self.right_id
    }

    /// 測試用建構子：產生一筆沒有 context id 的 v1 詞條。
    #[cfg(test)]
    pub(crate) fn for_test(
        surface: &str,
        reading: &str,
        pos_id: u16,
        conj: (u8, u8),
        cost: i16,
        split: Split,
        flags: u16,
    ) -> Entry {
        Entry {
            surface: surface.into(),
            reading: reading.into(),
            pos_id,
            conj_type: conj.0,
            conj_form: conj.1,
            cost,
            split,
            flags,
            left_id: None,
            right_id: None,
        }
    }
}

/// 解析時算出的統計。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    /// 最小詞成本。
    pub cost_min: i16,
    /// 最大詞成本。
    pub cost_max: i16,
    /// 負成本詞條數。
    pub negative_costs: usize,
    /// 各 split 的詞條數。
    pub split_counts: [usize; 3],
    /// 同一 surface 的最大詞條數。
    pub max_readings: usize,
    /// 最長讀音的字元數。
    pub max_reading_unit: usize,
}

/// pack 的能力回報。v2 才有 context id 與連接矩陣。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// 是否帶 left/right context ID。
    pub context_ids: bool,
    /// 是否內含連接成本矩陣。
    pub connection_matrix: bool,
}

struct SurfaceRef {
    surface: String,
    first_entry: u32,
    count: u32,
}

struct Section {
    name: &'static str,
    start: u64,
    end: u64,
}

/// 已開啟並驗證的 pack。
pub struct Pack {
    _map: Mmap,
    header: Header,
    entry_size: usize,
    payload_sha256: [u8; 32],
    file_sha256: [u8; 32],
    surfaces: Vec<SurfaceRef>,
    entries: Vec<Entry>,
    pos: Vec<String>,
    conj_types: Vec<String>,
    conj_forms: Vec<String>,
    stats: Stats,
}

impl Pack {
    /// 開啟並驗證 pack 檔。v1 與 v2 都支援。
    pub fn open(path: impl AsRef<Path>) -> Result<Pack, PackError> {
        let path = path.as_ref();
        let file = File::open(path).map_err(|source| PackError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let length = file.metadata().map(|m| m.len()).unwrap_or(0);
        if (length as usize) < HEADER_SIZE + TRAILER_SIZE {
            return Err(PackError::TooSmall {
                size: length as usize,
                minimum: HEADER_SIZE + TRAILER_SIZE,
            });
        }
        // SAFETY: 檔案在 Pack 存活期間保持映射；pack 是不可變的資料檔。
        let map = unsafe { Mmap::map(&file) }.map_err(|source| PackError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Pack::parse(map)
    }

    fn parse(map: Mmap) -> Result<Pack, PackError> {
        let data: &[u8] = &map;
        if data.len() < HEADER_SIZE + TRAILER_SIZE {
            return Err(PackError::TooSmall {
                size: data.len(),
                minimum: HEADER_SIZE + TRAILER_SIZE,
            });
        }
        if data[..MAGIC.len()] != MAGIC {
            return Err(PackError::BadMagic);
        }

        let version = u16::from_le_bytes([data[8], data[9]]);
        let (header_size, entry_size) = match version {
            FORMAT_VERSION => (HEADER_SIZE, ENTRY_SIZE),
            FORMAT_VERSION_V2 => (HEADER_SIZE_V2, ENTRY_SIZE_V2),
            actual => {
                return Err(PackError::UnsupportedVersion {
                    actual,
                    expected: FORMAT_VERSION_V2,
                })
            }
        };
        if data.len() < header_size + TRAILER_SIZE {
            return Err(PackError::TooSmall {
                size: data.len(),
                minimum: header_size + TRAILER_SIZE,
            });
        }

        let header = Header::parse(&data[..header_size], version);
        if header.header_size as usize != header_size {
            return Err(PackError::BadHeaderSize {
                actual: header.header_size,
                expected: header_size as u16,
            });
        }
        if version == FORMAT_VERSION {
            if header.context_count != 0 {
                return Err(PackError::NonZeroReserved(header.context_count));
            }
        } else {
            if header.context_count == 0 {
                return Err(PackError::EmptyCount {
                    field: "context_count",
                });
            }
            if let Some(&value) = header.reserved.iter().find(|&&v| v != 0) {
                return Err(PackError::NonZeroReserved32(value));
            }
        }
        if header.flags != 0 {
            return Err(PackError::UnsupportedFlags(header.flags));
        }
        if header.entry_count == 0 {
            return Err(PackError::EmptyCount {
                field: "entry_count",
            });
        }
        if header.surface_count == 0 {
            return Err(PackError::EmptyCount {
                field: "surface_count",
            });
        }
        if header.pos_count == 0 {
            return Err(PackError::EmptyCount { field: "pos_count" });
        }

        let payload_end = data.len() - TRAILER_SIZE;
        let payload_sum = sha256(&data[..payload_end]);
        if payload_sum != data[payload_end..] {
            return Err(PackError::PayloadHashMismatch {
                trailer: hex(&data[payload_end..]),
                computed: hex(&payload_sum),
            });
        }

        validate_sections(&header, payload_end)?;
        check_section_lengths(&header)?;

        let pos = parse_string_table(
            data,
            &header,
            header.pos_offset,
            header.pos_length,
            header.pos_count as usize,
            "POS table",
            false,
        )?;
        let (conj_types, conj_forms) = parse_conj_table(data, &header)?;
        let raw_index = parse_surface_index(data, &header)?;
        let entries = decode_entries(data, &header, entry_size, &pos, &conj_types, &conj_forms)?;
        let surfaces = build_surfaces(data, &header, &raw_index, &entries)?;
        let stats = collect_stats(&entries, &raw_index, header.entry_count)?;
        let file_sha256 = sha256(data);

        Ok(Pack {
            _map: map,
            header,
            entry_size,
            payload_sha256: payload_sum,
            file_sha256,
            surfaces,
            entries,
            pos,
            conj_types,
            conj_forms,
            stats,
        })
    }

    /// 重新驗證：payload 雜湊、整檔雜湊、section 界線與 count 長度。
    ///
    /// 開啟時已經完整驗證過一次；這個入口供載入後重新核對用。
    /// v1 與 v2 的檢查項目相同，v2 另外要求連接矩陣存在且長度正確。
    pub fn verify(&self) -> Result<(), PackError> {
        let data: &[u8] = &self._map;
        let payload_end = data.len() - TRAILER_SIZE;
        let sum = sha256(&data[..payload_end]);
        if sum != self.payload_sha256 {
            return Err(PackError::PayloadHashMismatch {
                trailer: hex(&self.payload_sha256),
                computed: hex(&sum),
            });
        }
        if sum != data[payload_end..] {
            return Err(PackError::PayloadHashMismatch {
                trailer: hex(&data[payload_end..]),
                computed: hex(&sum),
            });
        }
        let file_sum = sha256(data);
        if file_sum != self.file_sha256 {
            return Err(PackError::FileHashMismatch {
                expected: hex(&self.file_sha256),
                actual: hex(&file_sum),
            });
        }
        validate_sections(&self.header, payload_end)?;
        check_section_lengths(&self.header)
    }

    /// 檔頭。
    pub fn header(&self) -> Header {
        self.header
    }

    /// 詞條數。
    pub fn entry_count(&self) -> usize {
        self.header.entry_count as usize
    }

    /// 格式版本。
    pub fn format_version(&self) -> u16 {
        self.header.format_version
    }

    /// 每筆詞條的位元組數（v1 是 20、v2 是 24）。
    pub fn entry_size(&self) -> usize {
        self.entry_size
    }

    /// context 數。v1 沒有連接矩陣，回傳 0。
    pub fn context_count(&self) -> u16 {
        self.header.context_count
    }

    /// 查連接成本：`matrix[left_id * context_count + right_id]`。
    ///
    /// v1、越界或沒有連接矩陣時回傳 `None`，不 panic。
    pub fn connection(&self, left: u16, right: u16) -> Option<i32> {
        if self.header.format_version != FORMAT_VERSION_V2 {
            return None;
        }
        let count = self.header.context_count;
        if left >= count || right >= count {
            return None;
        }
        let index = left as usize * count as usize + right as usize;
        let start = self.header.connection_offset as usize + index * 2;
        let bytes = self._map.get(start..start + 2)?;
        Some(i16::from_le_bytes([bytes[0], bytes[1]]) as i32)
    }

    /// 唯一 surface 數。
    pub fn surface_count(&self) -> usize {
        self.header.surface_count as usize
    }

    /// POS table，順序與 pack 內一致。
    pub fn pos_list(&self) -> &[String] {
        &self.pos
    }

    /// 活用型 table。
    pub fn conj_types(&self) -> &[String] {
        &self.conj_types
    }

    /// 活用形 table。
    pub fn conj_forms(&self) -> &[String] {
        &self.conj_forms
    }

    /// 解析時算出的統計。
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// 依格式版本回報能力：v2 有 context id 與連接矩陣，v1 都沒有。
    pub fn capabilities(&self) -> Capabilities {
        let v2 = self.header.format_version == FORMAT_VERSION_V2;
        Capabilities {
            context_ids: v2,
            connection_matrix: v2,
        }
    }

    /// trailer 內的 payload SHA-256（小寫 hex）。
    pub fn payload_sha256(&self) -> String {
        hex(&self.payload_sha256)
    }

    /// 整檔 SHA-256（小寫 hex）。
    pub fn file_sha256(&self) -> String {
        hex(&self.file_sha256)
    }

    /// 檔案大小。
    pub fn bytes(&self) -> u64 {
        self._map.len() as u64
    }

    /// 全部詞條，順序與 pack 內相同。
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// 查 POS 字串。
    pub fn pos_str(&self, id: u16) -> Option<&str> {
        self.pos.get(id as usize).map(String::as_str)
    }

    /// 查活用型字串。
    pub fn conj_type(&self, id: u8) -> Option<&str> {
        self.conj_types.get(id as usize).map(String::as_str)
    }

    /// 查活用形字串。
    pub fn conj_form(&self, id: u8) -> Option<&str> {
        self.conj_forms.get(id as usize).map(String::as_str)
    }

    /// 回傳 surface 完全相等的所有詞條，順序與 pack 內相同。
    pub fn lookup(&self, surface: &str) -> &[Entry] {
        let idx = self
            .surfaces
            .partition_point(|s| s.surface.as_str() < surface);
        let Some(found) = self.surfaces.get(idx) else {
            return &[];
        };
        if found.surface != surface {
            return &[];
        }
        let start = found.first_entry as usize;
        &self.entries[start..start + found.count as usize]
    }

    /// surface 是否存在。
    pub fn has(&self, surface: &str) -> bool {
        !self.lookup(surface).is_empty()
    }

    /// 以 prefix 開頭的 surface 的所有詞條，依 surface 排序。
    pub fn prefix<'a>(&'a self, prefix: &str) -> PrefixIter<'a> {
        let idx = self
            .surfaces
            .partition_point(|s| s.surface.as_str() < prefix);
        PrefixIter {
            pack: self,
            surface_idx: idx,
            entry_idx: 0,
            started: false,
            finished: idx >= self.surfaces.len(),
            prefix: prefix.to_owned(),
        }
    }
}

/// `Pack::prefix` 的迭代器。
pub struct PrefixIter<'a> {
    pack: &'a Pack,
    surface_idx: usize,
    entry_idx: usize,
    started: bool,
    finished: bool,
    prefix: String,
}

impl<'a> Iterator for PrefixIter<'a> {
    type Item = &'a Entry;

    fn next(&mut self) -> Option<&'a Entry> {
        loop {
            if self.finished {
                return None;
            }
            let Some(current) = self.pack.surfaces.get(self.surface_idx) else {
                self.finished = true;
                return None;
            };
            if !self.started {
                if !current.surface.starts_with(&self.prefix) {
                    self.finished = true;
                    return None;
                }
                self.started = true;
                self.entry_idx = current.first_entry as usize;
            }
            let end = current.first_entry as usize + current.count as usize;
            if self.entry_idx < end {
                let entry = &self.pack.entries[self.entry_idx];
                self.entry_idx += 1;
                return Some(entry);
            }
            self.surface_idx += 1;
            self.started = false;
        }
    }
}

pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn validate_sections(header: &Header, payload_end: usize) -> Result<(), PackError> {
    let mut items = vec![
        (
            "surface blob",
            header.surface_blob_offset,
            header.surface_blob_length,
        ),
        (
            "reading blob",
            header.reading_blob_offset,
            header.reading_blob_length,
        ),
        ("entries", header.entries_offset, header.entries_length),
        ("surface index", header.index_offset, header.index_length),
        ("POS table", header.pos_offset, header.pos_length),
        ("conjugation table", header.conj_offset, header.conj_length),
    ];
    if header.format_version == FORMAT_VERSION_V2 {
        items.push((
            "connection",
            header.connection_offset,
            header.connection_length,
        ));
    }
    let mut sections = Vec::with_capacity(items.len());
    for (name, offset, length) in items {
        let start = offset as u64;
        let end = start + length as u64;
        if start < header.header_size as u64 {
            return Err(PackError::SectionInsideHeader {
                section: name,
                start,
            });
        }
        if end > payload_end as u64 {
            return Err(PackError::SectionOutOfBounds {
                section: name,
                end,
                payload_end: payload_end as u64,
            });
        }
        sections.push(Section { name, start, end });
    }
    sections.sort_by_key(|s| s.start);
    for pair in sections.windows(2) {
        let (first, second) = (&pair[0], &pair[1]);
        if second.start < first.end {
            return Err(PackError::SectionOverlap {
                first: first.name,
                first_start: first.start,
                first_end: first.end,
                second: second.name,
                second_start: second.start,
                second_end: second.end,
            });
        }
    }
    Ok(())
}

fn check_section_lengths(header: &Header) -> Result<(), PackError> {
    let entry_size = if header.format_version == FORMAT_VERSION_V2 {
        ENTRY_SIZE_V2
    } else {
        ENTRY_SIZE
    };
    let entries_expected = header.entry_count as u64 * entry_size as u64;
    if header.entries_length as u64 != entries_expected {
        return Err(PackError::SectionLengthMismatch {
            section: "entries",
            actual: header.entries_length as u64,
            expected: entries_expected,
        });
    }
    let index_expected = header.surface_count as u64 * INDEX_ENTRY_SIZE as u64;
    if header.index_length as u64 != index_expected {
        return Err(PackError::SectionLengthMismatch {
            section: "surface index",
            actual: header.index_length as u64,
            expected: index_expected,
        });
    }
    if header.format_version == FORMAT_VERSION_V2 {
        let contexts = header.context_count as u64;
        let connection_expected = contexts * contexts * 2;
        if header.connection_length as u64 != connection_expected {
            return Err(PackError::SectionLengthMismatch {
                section: "connection",
                actual: header.connection_length as u64,
                expected: connection_expected,
            });
        }
    }
    Ok(())
}

fn section<'a>(
    data: &'a [u8],
    offset: u32,
    length: u32,
    name: &'static str,
) -> Result<&'a [u8], PackError> {
    let start = offset as usize;
    let end = start + length as usize;
    data.get(start..end).ok_or(PackError::SectionOutOfBounds {
        section: name,
        end: end as u64,
        payload_end: data.len() as u64,
    })
}

fn parse_string_table(
    data: &[u8],
    _header: &Header,
    offset: u32,
    length: u32,
    want: usize,
    name: &'static str,
    allow_empty: bool,
) -> Result<Vec<String>, PackError> {
    let bytes = section(data, offset, length, name)?;
    if bytes.len() < 2 {
        return Err(PackError::TableEntry {
            table: name,
            index: 0,
            reason: "section 太短，缺少筆數".into(),
        });
    }
    let count = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
    if count != want {
        return Err(PackError::TableCountMismatch {
            table: name,
            actual: count,
            expected: want,
        });
    }
    let mut out = Vec::with_capacity(count);
    let mut pos = 2usize;
    for i in 0..count {
        if pos + 2 > bytes.len() {
            return Err(PackError::TableEntry {
                table: name,
                index: i + 1,
                reason: "缺少長度前綴".into(),
            });
        }
        let len = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]) as usize;
        pos += 2;
        if len == 0 || pos + len > bytes.len() {
            return Err(PackError::TableEntry {
                table: name,
                index: i + 1,
                reason: format!("長度 {len} 超出 section"),
            });
        }
        let raw = &bytes[pos..pos + len];
        pos += len;
        let s = std::str::from_utf8(raw).map_err(|_| PackError::TableEntry {
            table: name,
            index: i + 1,
            reason: "不是合法 UTF-8".into(),
        })?;
        if s.is_empty() && !allow_empty {
            return Err(PackError::TableEntry {
                table: name,
                index: i + 1,
                reason: "是空字串".into(),
            });
        }
        if let Some(prev) = out.last() {
            let prev: &String = prev;
            if prev.as_str() >= s {
                return Err(PackError::TableEntry {
                    table: name,
                    index: i + 1,
                    reason: format!("未依字串嚴格排序（{prev:?} 之後是 {s:?}）"),
                });
            }
        }
        out.push(s.to_owned());
    }
    if pos != bytes.len() {
        return Err(PackError::TableEntry {
            table: name,
            index: count,
            reason: format!("尾端有 {} 位元組未使用", bytes.len() - pos),
        });
    }
    Ok(out)
}

fn parse_conj_table(data: &[u8], header: &Header) -> Result<(Vec<String>, Vec<String>), PackError> {
    let bytes = section(
        data,
        header.conj_offset,
        header.conj_length,
        "conjugation table",
    )?;
    let mut pos = 0usize;
    let mut read_table = |name: &'static str, want: usize| -> Result<Vec<String>, PackError> {
        if pos + 2 > bytes.len() {
            return Err(PackError::TableEntry {
                table: name,
                index: 0,
                reason: "缺少筆數".into(),
            });
        }
        let count = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]) as usize;
        pos += 2;
        if count != want {
            return Err(PackError::TableCountMismatch {
                table: name,
                actual: count,
                expected: want,
            });
        }
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            if pos + 2 > bytes.len() {
                return Err(PackError::TableEntry {
                    table: name,
                    index: i + 1,
                    reason: "缺少長度前綴".into(),
                });
            }
            let len = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]) as usize;
            pos += 2;
            if len == 0 || pos + len > bytes.len() {
                return Err(PackError::TableEntry {
                    table: name,
                    index: i + 1,
                    reason: format!("長度 {len} 超出 section"),
                });
            }
            let raw = &bytes[pos..pos + len];
            pos += len;
            let s = std::str::from_utf8(raw).map_err(|_| PackError::TableEntry {
                table: name,
                index: i + 1,
                reason: "不是合法 UTF-8".into(),
            })?;
            if let Some(prev) = out.last() {
                let prev: &String = prev;
                if prev.as_str() >= s {
                    return Err(PackError::TableEntry {
                        table: name,
                        index: i + 1,
                        reason: format!("未依字串嚴格排序（{prev:?} 之後是 {s:?}）"),
                    });
                }
            }
            out.push(s.to_owned());
        }
        Ok(out)
    };
    let types = read_table("活用型", header.conj_type_count as usize)?;
    let forms = read_table("活用形", header.conj_form_count as usize)?;
    if pos != bytes.len() {
        return Err(PackError::TableEntry {
            table: "conjugation table",
            index: 0,
            reason: format!("尾端有 {} 位元組未使用", bytes.len() - pos),
        });
    }
    Ok((types, forms))
}

fn parse_surface_index(data: &[u8], header: &Header) -> Result<Vec<(u32, u32)>, PackError> {
    let bytes = section(
        data,
        header.index_offset,
        header.index_length,
        "surface index",
    )?;
    let count = header.surface_count as usize;
    let mut out = Vec::with_capacity(count);
    let mut prev_offset = 0u32;
    let mut prev_first = 0u32;
    for i in 0..count {
        let base = i * INDEX_ENTRY_SIZE;
        let offset = u32::from_le_bytes([
            bytes[base],
            bytes[base + 1],
            bytes[base + 2],
            bytes[base + 3],
        ]);
        let first = u32::from_le_bytes([
            bytes[base + 4],
            bytes[base + 5],
            bytes[base + 6],
            bytes[base + 7],
        ]);
        if i == 0 {
            if offset != 0 {
                return Err(PackError::FirstSurfaceOffset(offset));
            }
        } else {
            if offset <= prev_offset {
                return Err(PackError::SurfaceOffsetNotIncreasing {
                    index: i + 1,
                    actual: offset,
                    previous: prev_offset,
                });
            }
            if first <= prev_first {
                return Err(PackError::FirstEntryNotIncreasing {
                    index: i + 1,
                    actual: first,
                    previous: prev_first,
                });
            }
        }
        if first >= header.entry_count {
            return Err(PackError::FirstEntryOutOfRange {
                index: i + 1,
                value: first,
                entry_count: header.entry_count,
            });
        }
        if offset >= header.surface_blob_length {
            return Err(PackError::SurfaceOffsetOutOfRange {
                index: i + 1,
                offset,
                blob_len: header.surface_blob_length,
            });
        }
        out.push((offset, first));
        prev_offset = offset;
        prev_first = first;
    }
    Ok(out)
}

fn blob_str(
    data: &[u8],
    blob_offset: u32,
    blob_len: u32,
    offset: u32,
    length: u16,
    field: &'static str,
    index: u32,
) -> Result<String, PackError> {
    let end = offset as u64 + length as u64;
    if end > blob_len as u64 {
        return Err(match field {
            "surface" => PackError::SurfaceOutOfBounds {
                index,
                offset,
                length,
                blob_len,
            },
            _ => PackError::ReadingOutOfBounds {
                index,
                offset,
                length,
                blob_len,
            },
        });
    }
    let start = blob_offset as usize + offset as usize;
    let stop = start + length as usize;
    let raw = &data[start..stop];
    std::str::from_utf8(raw)
        .map(str::to_owned)
        .map_err(|_| PackError::InvalidUtf8 { index, field })
}

fn decode_entries(
    data: &[u8],
    header: &Header,
    entry_size: usize,
    pos: &[String],
    conj_types: &[String],
    conj_forms: &[String],
) -> Result<Vec<Entry>, PackError> {
    let count = header.entry_count as usize;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let base = header.entries_offset as usize + i * entry_size;
        let b = &data[base..base + entry_size];
        let surface_offset = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let surface_len = u16::from_le_bytes([b[4], b[5]]);
        let reading_offset = u32::from_le_bytes([b[6], b[7], b[8], b[9]]);
        let reading_len = u16::from_le_bytes([b[10], b[11]]);
        let pos_id = u16::from_le_bytes([b[12], b[13]]);
        let conj_type = b[14];
        let conj_form = b[15];
        let cost = i16::from_le_bytes([b[16], b[17]]);
        let flags = u16::from_le_bytes([b[18], b[19]]);
        let (left_id, right_id) = if entry_size == ENTRY_SIZE_V2 {
            let left = u16::from_le_bytes([b[20], b[21]]);
            let right = u16::from_le_bytes([b[22], b[23]]);
            for (side, id) in [("left_id", left), ("right_id", right)] {
                if id >= header.context_count {
                    return Err(PackError::EntryContextOutOfRange {
                        index: i as u32,
                        side,
                        id,
                        context_count: header.context_count,
                    });
                }
            }
            (Some(left), Some(right))
        } else {
            (None, None)
        };
        let index = i as u32;

        if pos_id as usize >= pos.len() {
            return Err(PackError::PosIdOutOfRange {
                index,
                id: pos_id,
                count: pos.len(),
            });
        }
        if conj_type as usize >= conj_types.len() {
            return Err(PackError::ConjTypeIdOutOfRange {
                index,
                id: conj_type,
                count: conj_types.len(),
            });
        }
        if conj_form as usize >= conj_forms.len() {
            return Err(PackError::ConjFormIdOutOfRange {
                index,
                id: conj_form,
                count: conj_forms.len(),
            });
        }
        if flags & !0x3 != 0 {
            return Err(PackError::UndefinedFlags { index, flags });
        }
        let Some(split) = Split::from_bits(flags) else {
            return Err(PackError::InvalidSplit {
                index,
                split: flags & 0x3,
            });
        };
        if cost < MIN_COST {
            return Err(PackError::CostOutOfRange {
                index,
                cost,
                minimum: MIN_COST,
            });
        }

        let surface = blob_str(
            data,
            header.surface_blob_offset,
            header.surface_blob_length,
            surface_offset,
            surface_len,
            "surface",
            index,
        )?;
        let reading = blob_str(
            data,
            header.reading_blob_offset,
            header.reading_blob_length,
            reading_offset,
            reading_len,
            "reading",
            index,
        )?;
        if surface.is_empty() {
            return Err(PackError::EmptySurface { index });
        }
        if reading.is_empty() {
            return Err(PackError::EmptyReading { index });
        }
        check_reading(index, &surface, &reading)?;

        out.push(Entry {
            surface: surface.into_boxed_str(),
            reading: reading.into_boxed_str(),
            pos_id,
            conj_type,
            conj_form,
            cost,
            split,
            flags,
            left_id,
            right_id,
        });
    }
    Ok(out)
}

pub(crate) fn is_hiragana(ch: char) -> bool {
    matches!(ch as u32, 0x3041..=0x3096 | 0x309D..=0x309F)
}

fn is_katakana(ch: char) -> bool {
    matches!(
        ch as u32,
        0x30A1..=0x30FA | 0x30FD..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9D
    )
}

fn check_reading(index: u32, surface: &str, reading: &str) -> Result<(), PackError> {
    for ch in reading.chars() {
        if ch == 'ー' || is_hiragana(ch) {
            continue;
        }
        let surface = surface.to_owned();
        let reading = reading.to_owned();
        if is_katakana(ch) {
            return Err(PackError::KatakanaReading {
                index,
                surface,
                reading,
                ch,
            });
        }
        return Err(PackError::NonKanaReading {
            index,
            surface,
            reading,
            ch,
        });
    }
    Ok(())
}

fn build_surfaces(
    data: &[u8],
    header: &Header,
    raw_index: &[(u32, u32)],
    entries: &[Entry],
) -> Result<Vec<SurfaceRef>, PackError> {
    let mut surfaces = Vec::with_capacity(raw_index.len());
    for (i, &(offset, first)) in raw_index.iter().enumerate() {
        let end = raw_index
            .get(i + 1)
            .map(|r| r.1)
            .unwrap_or(header.entry_count);
        if end <= first {
            return Err(PackError::EmptySurfaceRange { index: i + 1 });
        }
        let entry = &entries[first as usize];
        if entry.surface.len() > u16::MAX as usize {
            return Err(PackError::SurfaceLengthOverflow {
                index: i + 1,
                length: entry.surface.len(),
            });
        }
        let indexed = blob_str(
            data,
            header.surface_blob_offset,
            header.surface_blob_length,
            offset,
            entry.surface.len() as u16,
            "surface",
            first,
        )?;
        if indexed != entry.surface.as_ref() {
            return Err(PackError::SurfaceIndexPointsElsewhere {
                index: i + 1,
                indexed,
                entry: entry.surface.to_string(),
            });
        }
        surfaces.push(SurfaceRef {
            surface: entry.surface.to_string(),
            first_entry: first,
            count: end - first,
        });
    }

    let mut surface_idx = 0usize;
    let mut previous: Option<&str> = None;
    for (i, entry) in entries.iter().enumerate() {
        if let Some(prev) = previous {
            if entry.surface.as_ref() < prev {
                return Err(PackError::SurfaceOrder {
                    index: i as u32,
                    surface: entry.surface.to_string(),
                    previous: prev.to_owned(),
                });
            }
        }
        let is_new = previous != Some(entry.surface.as_ref());
        if is_new {
            let Some(reference) = surfaces.get(surface_idx) else {
                return Err(PackError::UnindexedSurface {
                    index: i as u32,
                    surface: entry.surface.to_string(),
                });
            };
            if reference.first_entry != i as u32 {
                return Err(PackError::SurfaceFirstEntryMismatch {
                    index: surface_idx + 1,
                    expected: reference.first_entry,
                    actual: i as u32,
                });
            }
            if reference.surface.len() != entry.surface.len() {
                return Err(PackError::SurfaceLengthMismatch {
                    surface: entry.surface.to_string(),
                    length: entry.surface.len(),
                    indexed: reference.surface.len() as u16,
                });
            }
            surface_idx += 1;
        }
        previous = Some(entry.surface.as_ref());
    }
    if surface_idx != surfaces.len() {
        return Err(PackError::SurfaceIndexCountMismatch {
            index_count: surfaces.len(),
            matched: surface_idx,
        });
    }
    if let Some(last) = raw_index.last() {
        let last_len = entries[last.1 as usize].surface.len() as u64;
        if last.0 as u64 + last_len != header.surface_blob_length as u64 {
            return Err(PackError::SurfaceBlobTrailingBytes);
        }
    }
    Ok(surfaces)
}

fn collect_stats(
    entries: &[Entry],
    raw_index: &[(u32, u32)],
    entry_count: u32,
) -> Result<Stats, PackError> {
    let mut stats = Stats {
        cost_min: i16::MAX,
        cost_max: MIN_COST,
        negative_costs: 0,
        split_counts: [0; 3],
        max_readings: 0,
        max_reading_unit: 0,
    };
    for entry in entries {
        stats.cost_min = stats.cost_min.min(entry.cost);
        stats.cost_max = stats.cost_max.max(entry.cost);
        if entry.cost < 0 {
            stats.negative_costs += 1;
        }
        stats.split_counts[entry.split as usize] += 1;
        let units = entry.reading.chars().count();
        stats.max_reading_unit = stats.max_reading_unit.max(units);
    }
    for (i, &(_, first)) in raw_index.iter().enumerate() {
        let end = raw_index.get(i + 1).map(|r| r.1).unwrap_or(entry_count);
        stats.max_readings = stats.max_readings.max((end - first) as usize);
    }
    Ok(stats)
}
