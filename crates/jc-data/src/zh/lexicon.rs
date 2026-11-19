//! `lexicon.bin` 的零拷貝讀取。
//!
//! 對照前身 Go `core/lexicon/compact.go`：詞條依讀音（音節 ID 序列）排序，
//! 同一前綴的詞條連續，所以精確查詢、前綴查詢與詞面查詢都是二分搜尋。
//! 檔案用 `memmap2` 映射，版面驗證只在開啟時做一次；查詢只做切片與
//! 數值比較，不複製詞面或讀音。
//!
//! 詞面與讀音都以檔案內容為準；[`EntryView`] 是借用映射內容的檢視，
//! 生命週期跟著 [`Lexicon`]。

use std::collections::HashMap;
use std::fs::File;
use std::ops::Range;
use std::path::Path;

use memmap2::Mmap;

use super::error::ZhDataError;
use super::format::{
    le_f32, le_u16, le_u32, partition_point, span, FileInfo, Reader, LEXICON_MAGIC, LEXICON_VERSION,
};

/// `flags` 的 bit 0：自動修正用的錯誤讀音。
pub const FLAG_CORRECTION: u8 = 1;
/// `flags` 的 bit 1：罕用字。
pub const FLAG_RARE: u8 = 2;

/// 一筆詞條的檢視。
///
/// 詞面借用映射內容；讀音用 [`Lexicon::reading`] 或
/// [`Lexicon::reading_units`] 依索引取出。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntryView<'a> {
    /// 詞條索引（儲存順序）。
    pub index: usize,
    /// 詞面。
    pub surface: &'a str,
    /// 讀音相容權重 P(r|w)，範圍 (0, 1]。
    pub prw: f32,
    /// 是否為自動修正用的錯誤讀音。
    pub correction: bool,
    /// 是否為罕用字。
    pub rare: bool,
}

/// 已開啟的注音詞庫。
pub struct Lexicon {
    map: Mmap,
    rd: Range<usize>,
    rd_off: Range<usize>,
    sf_off: Range<usize>,
    sf_len: Range<usize>,
    rd_len: Range<usize>,
    flags: Range<usize>,
    prw: Range<usize>,
    strs: Range<usize>,
    by_text: Range<usize>,
    count: usize,
    /// 音節 ID → 音節字串；只有音節表需要複製，詞條與詞面仍是映射內容。
    units: Vec<String>,
    ids: HashMap<String, u16>,
    longest: usize,
}

impl Lexicon {
    /// 開啟並驗證 `lexicon.bin`，不檢查 manifest。
    pub fn open(path: impl AsRef<Path>) -> Result<Lexicon, ZhDataError> {
        Lexicon::open_inner(path.as_ref(), None)
    }

    /// 開啟 `lexicon.bin`，並比對 manifest 的大小與 SHA-256。
    pub fn open_verified(path: impl AsRef<Path>, info: &FileInfo) -> Result<Lexicon, ZhDataError> {
        Lexicon::open_inner(path.as_ref(), Some(info))
    }

    fn open_inner(path: &Path, info: Option<&FileInfo>) -> Result<Lexicon, ZhDataError> {
        let file = File::open(path).map_err(|source| ZhDataError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        // SAFETY: 檔案在 Lexicon 存活期間保持映射，資料檔是不可變的唯讀內容。
        let map = unsafe { Mmap::map(&file) }.map_err(|source| ZhDataError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if let Some(info) = info {
            verify_image(path, info, &map)?;
        }
        Lexicon::parse(map)
    }

    fn parse(map: Mmap) -> Result<Lexicon, ZhDataError> {
        let (
            rd,
            rd_off,
            sf_off,
            sf_len,
            rd_len,
            flags,
            prw,
            strs,
            by_text,
            count,
            units,
            ids,
            strs_total,
        ) = {
            let file: &[u8] = &map;
            let mut reader = Reader::new(file, LEXICON_MAGIC, LEXICON_VERSION)?;
            let syls = reader.string()?;
            let rd = span(file, reader.u16s()?);
            let rd_off = span(file, reader.u32s()?);
            let sf_off = span(file, reader.u32s()?);
            let sf_len = span(file, reader.u16s()?);
            let rd_len = span(file, reader.bytes()?);
            let flags = span(file, reader.bytes()?);
            let prw = span(file, reader.f32s()?);
            let strs = span(file, reader.string()?.as_bytes());
            let by_text = span(file, reader.u32s()?);
            reader.done()?;

            let count = rd_off.len() / 4;
            if sf_off.len() / 4 != count
                || sf_len.len() / 2 != count
                || rd_len.len() != count
                || flags.len() != count
                || prw.len() / 4 != count
                || by_text.len() / 4 != count
            {
                return Err(ZhDataError::Layout("各欄長度不一致".into()));
            }

            let strs_total = std::str::from_utf8(&file[strs.clone()])
                .map_err(|_| ZhDataError::InvalidUtf8 {
                    what: "詞面區段"
                })?
                .len();

            let mut units = Vec::new();
            for unit in syls.split_ascii_whitespace() {
                units.push(unit.to_string());
            }
            if units.len() > u16::MAX as usize + 1 {
                return Err(ZhDataError::Layout(format!(
                    "音節種類 {} 超過 65536",
                    units.len()
                )));
            }
            let mut ids = HashMap::with_capacity(units.len());
            for (index, unit) in units.iter().enumerate() {
                ids.insert(unit.clone(), index as u16);
            }
            (
                rd, rd_off, sf_off, sf_len, rd_len, flags, prw, strs, by_text, count, units, ids,
                strs_total,
            )
        };

        let mut lexicon = Lexicon {
            map,
            rd,
            rd_off,
            sf_off,
            sf_len,
            rd_len,
            flags,
            prw,
            strs,
            by_text,
            count,
            units,
            ids,
            longest: 0,
        };

        let rd_total = lexicon.rd.len() / 2;
        for index in 0..count {
            let read_off = lexicon.rd_off(index) as usize;
            let read_len = lexicon.rd_len(index) as usize;
            if read_len == 0 {
                return Err(ZhDataError::Layout(format!("詞條 {index} 的讀音是空的")));
            }
            if read_off + read_len > rd_total {
                return Err(ZhDataError::Layout(format!(
                    "詞條 {index} 的讀音位移超出範圍"
                )));
            }
            for k in 0..read_len {
                if lexicon.rd_id(read_off + k) as usize >= lexicon.units.len() {
                    return Err(ZhDataError::Layout(format!(
                        "詞條 {index} 的音節 ID 超出範圍"
                    )));
                }
            }
            let surface_off = lexicon.sf_off(index) as usize;
            let surface_len = lexicon.sf_len(index) as usize;
            if surface_off + surface_len > strs_total {
                return Err(ZhDataError::Layout(format!(
                    "詞條 {index} 的詞面位移超出範圍"
                )));
            }
            let prw = lexicon.prw(index);
            if !(prw > 0.0 && prw <= 1.0) {
                return Err(ZhDataError::Layout(format!(
                    "詞條 {index} 的 P(r|w) 不在 (0,1]"
                )));
            }
            lexicon.longest = lexicon.longest.max(read_len);
        }
        for index in 0..count {
            if lexicon.by_text(index) as usize >= count {
                return Err(ZhDataError::Layout("詞面索引超出範圍".into()));
            }
        }

        Ok(lexicon)
    }

    /// 詞條數。
    pub fn len(&self) -> usize {
        self.count
    }

    /// 是否沒有詞條。
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// 最長詞條的讀音音節數。
    pub fn longest_word(&self) -> usize {
        self.longest
    }

    /// 檔案大小（位元組）。
    pub fn bytes(&self) -> u64 {
        self.map.len() as u64
    }

    fn bytes_at(&self, range: &Range<usize>) -> &[u8] {
        &self.map[range.clone()]
    }

    fn rd_off(&self, index: usize) -> u32 {
        le_u32(self.bytes_at(&self.rd_off), index)
    }

    fn sf_off(&self, index: usize) -> u32 {
        le_u32(self.bytes_at(&self.sf_off), index)
    }

    fn sf_len(&self, index: usize) -> u16 {
        le_u16(self.bytes_at(&self.sf_len), index)
    }

    fn rd_len(&self, index: usize) -> u8 {
        self.bytes_at(&self.rd_len)[index]
    }

    fn flags(&self, index: usize) -> u8 {
        self.bytes_at(&self.flags)[index]
    }

    fn prw(&self, index: usize) -> f32 {
        le_f32(self.bytes_at(&self.prw), index)
    }

    fn rd_id(&self, index: usize) -> u16 {
        le_u16(self.bytes_at(&self.rd), index)
    }

    fn by_text(&self, index: usize) -> u32 {
        le_u32(self.bytes_at(&self.by_text), index)
    }

    fn surface(&self, index: usize) -> &str {
        let start = self.sf_off(index) as usize;
        let end = start + self.sf_len(index) as usize;
        // 開啟時已驗證整個詞面區段是 UTF-8。
        std::str::from_utf8(&self.map[self.strs.start + start..self.strs.start + end])
            .expect("詞面區段已驗證")
    }

    /// 依索引取讀音音節。
    pub fn reading_units(&self, index: usize) -> Vec<&str> {
        let start = self.rd_off(index) as usize;
        let len = self.rd_len(index) as usize;
        (0..len)
            .map(|k| {
                let id = self.rd_id(start + k) as usize;
                self.units[id].as_str()
            })
            .collect()
    }

    /// 依索引取讀音字串，音節以單一空格分隔。
    pub fn reading(&self, index: usize) -> String {
        self.reading_units(index).join(" ")
    }

    /// 依索引取詞條檢視。
    pub fn entry(&self, index: usize) -> EntryView<'_> {
        let flags = self.flags(index);
        EntryView {
            index,
            surface: self.surface(index),
            prw: self.prw(index),
            correction: flags & FLAG_CORRECTION != 0,
            rare: flags & FLAG_RARE != 0,
        }
    }

    /// 依儲存順序走訪所有詞條。
    pub fn entries(&self) -> Entries<'_> {
        Entries {
            lexicon: self,
            next: 0,
            end: self.count,
        }
    }

    /// 把音節字串轉成 ID；有詞庫沒有的音節時回傳 `None`。
    fn lookup_ids(&self, units: &[&str]) -> Option<Vec<u16>> {
        let mut out = Vec::with_capacity(units.len());
        for unit in units {
            out.push(*self.ids.get(*unit)?);
        }
        Some(out)
    }

    fn cmp_ids(&self, index: usize, query: &[u16]) -> std::cmp::Ordering {
        let start = self.rd_off(index) as usize;
        let len = self.rd_len(index) as usize;
        for (k, &right) in query.iter().enumerate().take(len) {
            let left = self.rd_id(start + k);
            if left != right {
                return left.cmp(&right);
            }
        }
        len.cmp(&query.len())
    }

    fn lower(&self, query: &[u16]) -> usize {
        partition_point(self.count, |index| self.cmp_ids(index, query).is_lt())
    }

    fn has_prefix_ids(&self, index: usize, query: &[u16]) -> bool {
        let start = self.rd_off(index) as usize;
        let len = self.rd_len(index) as usize;
        if len < query.len() {
            return false;
        }
        (0..query.len()).all(|k| self.rd_id(start + k) == query[k])
    }

    /// 讀音完全相等的詞條。
    ///
    /// `reading` 是空白分隔的注音音節；含有詞庫沒有的音節時回傳空向量。
    pub fn exact(&self, reading: &str) -> Vec<EntryView<'_>> {
        let units: Vec<&str> = reading.split_ascii_whitespace().collect();
        self.exact_units(&units)
    }

    /// 讀音完全相等的詞條，音節已拆好。
    pub fn exact_units(&self, units: &[&str]) -> Vec<EntryView<'_>> {
        let Some(query) = self.lookup_ids(units) else {
            return Vec::new();
        };
        let start = self.lower(&query);
        let mut out = Vec::new();
        let mut index = start;
        while index < self.count && self.cmp_ids(index, &query).is_eq() {
            out.push(self.entry(index));
            index += 1;
        }
        out
    }

    /// 讀音以 `reading` 開頭的詞條，包含完全相等者。
    pub fn prefixed(&self, reading: &str) -> Vec<EntryView<'_>> {
        let units: Vec<&str> = reading.split_ascii_whitespace().collect();
        self.prefixed_units(&units)
    }

    /// 讀音以 `units` 開頭的詞條，音節已拆好。
    pub fn prefixed_units(&self, units: &[&str]) -> Vec<EntryView<'_>> {
        let Some(query) = self.lookup_ids(units) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut index = self.lower(&query);
        while index < self.count && self.has_prefix_ids(index, &query) {
            out.push(self.entry(index));
            index += 1;
        }
        out
    }

    /// 是否存在讀音以 `reading` 開頭的詞條。
    pub fn has_prefix(&self, reading: &str) -> bool {
        let units: Vec<&str> = reading.split_ascii_whitespace().collect();
        let Some(query) = self.lookup_ids(&units) else {
            return false;
        };
        let index = self.lower(&query);
        index < self.count && self.has_prefix_ids(index, &query)
    }

    /// 詞面是否存在于任一讀音。
    pub fn has(&self, surface: &str) -> bool {
        let index = self.text_lower(surface);
        index < self.count && self.surface(self.by_text(index) as usize) == surface
    }

    /// 某個詞面的所有讀音，依儲存順序。
    pub fn lookup(&self, surface: &str) -> Vec<EntryView<'_>> {
        let mut out = Vec::new();
        let mut index = self.text_lower(surface);
        while index < self.count && self.surface(self.by_text(index) as usize) == surface {
            out.push(self.entry(self.by_text(index) as usize));
            index += 1;
        }
        out
    }

    fn text_lower(&self, surface: &str) -> usize {
        partition_point(self.count, |index| {
            self.surface(self.by_text(index) as usize) < surface
        })
    }
}

fn verify_image(path: &Path, info: &FileInfo, map: &[u8]) -> Result<(), ZhDataError> {
    if info.size != map.len() as u64 {
        return Err(ZhDataError::ManifestMismatch {
            file: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
            field: "size",
            expected: info.size.to_string(),
            actual: map.len().to_string(),
        });
    }
    let actual = super::format::sha256_hex(map);
    if !info.sha256.eq_ignore_ascii_case(&actual) {
        return Err(ZhDataError::ManifestMismatch {
            file: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
            field: "sha256",
            expected: info.sha256.clone(),
            actual,
        });
    }
    Ok(())
}

/// [`Lexicon::entries`] 的迭代器。
pub struct Entries<'a> {
    lexicon: &'a Lexicon,
    next: usize,
    end: usize,
}

impl<'a> Iterator for Entries<'a> {
    type Item = EntryView<'a>;

    fn next(&mut self) -> Option<EntryView<'a>> {
        if self.next >= self.end {
            return None;
        }
        let view = self.lexicon.entry(self.next);
        self.next += 1;
        Some(view)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.end - self.next;
        (left, Some(left))
    }
}

impl ExactSizeIterator for Entries<'_> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zh::testutil::{encode_lexicon, Sample};

    fn sample() -> Vec<Sample> {
        vec![
            Sample::new("我", &["ㄨㄛˇ"], 1.0),
            Sample::new("愛", &["ㄞˋ"], 1.0),
            Sample::new("我愛", &["ㄨㄛˇ", "ㄞˋ"], 0.8),
            Sample::new("臥", &["ㄨㄛˋ"], 0.5),
            Sample::new("我", &["ㄜˇ"], 0.3),
        ]
    }

    fn write_temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("jc-data-zh-{name}-{}", std::process::id()));
        std::fs::write(&path, bytes).expect("寫入暫存詞庫");
        path
    }

    #[test]
    fn exact_returns_only_equal_reading() {
        let bytes = encode_lexicon(&sample());
        let path = write_temp("exact", &bytes);
        let lexicon = Lexicon::open(&path).unwrap();
        let got: Vec<&str> = lexicon.exact("ㄨㄛˇ").iter().map(|e| e.surface).collect();
        assert_eq!(got, vec!["我"]);
        assert_eq!(lexicon.exact("ㄨㄛ").len(), 0);
        assert_eq!(lexicon.exact("ㄅㄨˋ").len(), 0);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn prefixed_matches_syllable_boundary() {
        let bytes = encode_lexicon(&sample());
        let path = write_temp("prefix", &bytes);
        let lexicon = Lexicon::open(&path).unwrap();
        let got: Vec<&str> = lexicon
            .prefixed("ㄨㄛˇ")
            .iter()
            .map(|e| e.surface)
            .collect();
        assert_eq!(got, vec!["我", "我愛"]);
        assert!(lexicon.has_prefix("ㄨㄛˇ"));
        assert!(!lexicon.has_prefix("ㄨㄛ"));
        assert_eq!(lexicon.longest_word(), 2);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn lookup_finds_all_readings_of_surface() {
        let bytes = encode_lexicon(&sample());
        let path = write_temp("lookup", &bytes);
        let lexicon = Lexicon::open(&path).unwrap();
        assert!(lexicon.has("我"));
        let got: Vec<String> = lexicon
            .lookup("我")
            .iter()
            .map(|view| lexicon.reading(view.index))
            .collect();
        // 同詞面的讀音依讀音排序，不由插入順序決定。
        assert_eq!(got, vec!["ㄜˇ", "ㄨㄛˇ"]);
        assert!(!lexicon.has("我愛你"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn entry_flags_are_exposed() {
        let mut samples = sample();
        samples.push(Sample::flags("𠮷", &["ㄐㄧˊ"], 0.2, FLAG_RARE));
        let bytes = encode_lexicon(&samples);
        let path = write_temp("flags", &bytes);
        let lexicon = Lexicon::open(&path).unwrap();
        let entry = lexicon.exact("ㄐㄧˊ")[0];
        assert!(entry.rare);
        assert!(!entry.correction);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn corrupted_image_is_rejected() {
        let mut bytes = encode_lexicon(&sample());
        // 改壞一個位元組；CRC 檢查會先擋下。
        let offset = LEXICON_MAGIC.len() + 4;
        bytes[offset] ^= 0xff;
        let path = write_temp("layout", &bytes);
        assert!(Lexicon::open(&path).is_err());
        std::fs::remove_file(path).ok();
    }
}
