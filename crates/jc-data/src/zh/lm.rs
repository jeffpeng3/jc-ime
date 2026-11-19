//! `lm.bin` 的零拷貝讀取：unigram 與 bigram。
//!
//! 對照前身 Go `core/lm/compact.go`。詞依字串排序，unigram 用二分搜尋；
//! bigram 依（前詞, 後詞）排序，每筆只存 3 位元組後詞 ID 與 1 位元組量化
//! 機率，前詞的範圍由 `leftStart` 隱含。沒有 bigram 的資料包（`bigrams`
//! 為 0）所有 bigram 查詢都回傳 0，unigram 照常可用。

use std::fs::File;
use std::ops::Range;
use std::path::Path;

use memmap2::Mmap;

use super::error::ZhDataError;
use super::format::{
    le_f32, le_u32, partition_point, span, FileInfo, Reader, LM_MAGIC, LM_VERSION,
};

/// 句首的虛擬詞。
pub const BOS: &str = "<s>";
/// 未登錄詞的 unigram 機率。
pub const UNSEEN: f64 = 1e-8;
/// bigram 機率量化的 log10 範圍。
pub const Q_RANGE: f64 = 8.0;

/// 已開啟的注音語言模型。
pub struct Lm {
    map: Mmap,
    strs: Range<usize>,
    off: Range<usize>,
    uni: Range<usize>,
    left_start: Range<usize>,
    right: Range<usize>,
    q: Range<usize>,
    words: usize,
    bigrams: usize,
}

impl Lm {
    /// 開啟並驗證 `lm.bin`，不檢查 manifest。
    pub fn open(path: impl AsRef<Path>) -> Result<Lm, ZhDataError> {
        Lm::open_inner(path.as_ref(), None)
    }

    /// 開啟 `lm.bin`，並比對 manifest 的大小與 SHA-256。
    pub fn open_verified(path: impl AsRef<Path>, info: &FileInfo) -> Result<Lm, ZhDataError> {
        Lm::open_inner(path.as_ref(), Some(info))
    }

    fn open_inner(path: &Path, info: Option<&FileInfo>) -> Result<Lm, ZhDataError> {
        let file = File::open(path).map_err(|source| ZhDataError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        // SAFETY: 檔案在 Lm 存活期間保持映射，資料檔是不可變的唯讀內容。
        let map = unsafe { Mmap::map(&file) }.map_err(|source| ZhDataError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if let Some(info) = info {
            verify_image(path, info, &map)?;
        }
        Lm::parse(map)
    }

    fn parse(map: Mmap) -> Result<Lm, ZhDataError> {
        let file: &[u8] = &map;
        let mut reader = Reader::new(file, LM_MAGIC, LM_VERSION)?;
        let strs = span(file, reader.string()?.as_bytes());
        let off = span(file, reader.u32s()?);
        let uni = span(file, reader.f32s()?);
        let left_start = span(file, reader.u32s()?);
        let right = span(file, reader.bytes()?);
        let q = span(file, reader.bytes()?);
        reader.done()?;

        std::str::from_utf8(&file[strs.clone()]).map_err(|_| ZhDataError::InvalidUtf8 {
            what: "詞表區段",
        })?;

        let words = uni.len() / 4;
        if off.len() / 4 != words + 1
            || left_start.len() / 4 != words + 1
            || right.len() != 3 * q.len()
        {
            return Err(ZhDataError::Layout("各欄長度不一致".into()));
        }

        let lm = Lm {
            map,
            strs,
            off,
            uni,
            left_start,
            right,
            q,
            words,
            bigrams: 0,
        };
        let bigrams = lm.bytes_at(&lm.q).len();
        if lm.left_start(words) as usize != bigrams {
            return Err(ZhDataError::Layout("bigram 索引長度不符".into()));
        }
        let mut lm = Lm { bigrams, ..lm };
        lm.validate()?;
        Ok(lm)
    }

    fn validate(&mut self) -> Result<(), ZhDataError> {
        self.bytes_at(&self.strs);
        let words = self.words;
        for index in 0..words {
            let start = self.off(index) as usize;
            let end = self.off(index + 1) as usize;
            if start > end || end > self.bytes_at(&self.strs).len() {
                return Err(ZhDataError::Layout(format!("詞 {index} 的位移超出範圍")));
            }
            if index > 0 && self.word(index - 1) >= self.word(index) {
                return Err(ZhDataError::Layout("詞表未排序".into()));
            }
        }
        for index in 0..=words {
            if index < words && self.left_start(index) > self.left_start(index + 1) {
                return Err(ZhDataError::Layout("bigram 索引未排序".into()));
            }
            if index >= words {
                continue;
            }
            let from = self.left_start(index) as usize;
            let to = self.left_start(index + 1) as usize;
            for k in from..to {
                let right = self.right_at(k) as usize;
                if right >= words || (k > from && self.right_at(k - 1) as usize >= right) {
                    return Err(ZhDataError::Layout(
                        "bigram 後詞 ID 超出範圍或未排序".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    fn bytes_at(&self, range: &Range<usize>) -> &[u8] {
        &self.map[range.clone()]
    }

    fn off(&self, index: usize) -> u32 {
        le_u32(self.bytes_at(&self.off), index)
    }

    fn left_start(&self, index: usize) -> u32 {
        le_u32(self.bytes_at(&self.left_start), index)
    }

    fn uni_prob(&self, index: usize) -> f32 {
        le_f32(self.bytes_at(&self.uni), index)
    }

    fn right_at(&self, index: usize) -> u32 {
        let bytes = self.bytes_at(&self.right);
        let at = index * 3;
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], 0])
    }

    fn word(&self, index: usize) -> &str {
        let start = self.off(index) as usize;
        let end = self.off(index + 1) as usize;
        // 開啟時已驗證整個詞表區段是 UTF-8。
        std::str::from_utf8(&self.map[self.strs.start + start..self.strs.start + end])
            .expect("詞表區段已驗證")
    }

    fn find(&self, word: &str) -> Option<usize> {
        let index = partition_point(self.words, |index| self.word(index) < word);
        (index < self.words && self.word(index) == word).then_some(index)
    }

    fn dequantize(q: u8) -> f64 {
        10f64.powf(-(q as f64) / 255.0 * Q_RANGE)
    }

    /// 詞數。
    pub fn words(&self) -> usize {
        self.words
    }

    /// bigram 筆數；0 表示這個資料包沒有 bigram。
    pub fn bigrams(&self) -> usize {
        self.bigrams
    }

    /// 檔案大小（位元組）。
    pub fn bytes(&self) -> u64 {
        self.map.len() as u64
    }

    /// P(w)；未登錄回傳 [`UNSEEN`]。
    pub fn uni(&self, word: &str) -> f64 {
        match self.find(word) {
            Some(index) => self.uni_prob(index) as f64,
            None => UNSEEN,
        }
    }

    /// P(w | left)；沒有這個 bigram 回傳 0。
    pub fn bi(&self, left: &str, word: &str) -> f64 {
        if self.bigrams == 0 {
            return 0.0;
        }
        let Some(left) = self.find(left) else {
            return 0.0;
        };
        let from = self.left_start(left) as usize;
        let to = self.left_start(left + 1) as usize;
        if from == to {
            return 0.0;
        }
        let Some(right) = self.find(word) else {
            return 0.0;
        };
        let offset = from
            + partition_point(to - from, |step| {
                (self.right_at(from + step) as usize) < right
            });
        if offset < to && self.right_at(offset) as usize == right {
            Lm::dequantize(self.bytes_at(&self.q)[offset])
        } else {
            0.0
        }
    }

    /// 是否有以 `left` 為前詞的 bigram。
    pub fn has_left(&self, left: &str) -> bool {
        if self.bigrams == 0 {
            return false;
        }
        match self.find(left) {
            Some(index) => self.left_start(index) != self.left_start(index + 1),
            None => false,
        }
    }

    /// 依詞表順序走訪所有 unigram。
    pub fn unigrams(&self) -> impl Iterator<Item = (&str, f64)> + '_ {
        (0..self.words).map(|index| (self.word(index), self.uni_prob(index) as f64))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zh::testutil::encode_lm;

    fn write_temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("jc-data-lm-{name}-{}", std::process::id()));
        std::fs::write(&path, bytes).expect("寫入暫存語言模型");
        path
    }

    #[test]
    fn unigram_and_bigram_follow_storage_order() {
        let uni = vec![("你", 0.125), ("愛", 0.25), ("我", 0.5)];
        let bi = vec![("我", "愛", 0.75), ("你", "我", 0.5)];
        let bytes = encode_lm(&uni, &bi);
        let path = write_temp("full", &bytes);
        let lm = Lm::open(&path).unwrap();
        assert_eq!(lm.words(), 3);
        assert_eq!(lm.bigrams(), 2);
        assert!((lm.uni("我") - 0.5).abs() < 1e-6);
        assert!((lm.bi("我", "愛") - 0.75).abs() < 0.05);
        assert!((lm.bi("你", "我") - 0.5).abs() < 0.05);
        assert_eq!(lm.bi("愛", "我"), 0.0);
        assert!(lm.has_left("我"));
        assert!(!lm.has_left("愛"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn unseen_word_returns_unseen_probability() {
        let bytes = encode_lm(&[("我", 0.5)], &[]);
        let path = write_temp("unseen", &bytes);
        let lm = Lm::open(&path).unwrap();
        assert_eq!(lm.uni("我"), 0.5);
        assert_eq!(lm.uni("不存在"), UNSEEN);
        assert_eq!(lm.bi("我", "不存在"), 0.0);
        assert_eq!(lm.bigrams(), 0);
        assert!(!lm.has_left("我"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn unsorted_words_are_rejected() {
        let bytes = encode_lm(&[("你", 0.5), ("我", 0.5)], &[]);
        let path = write_temp("unsorted", &bytes);
        assert!(Lm::open(&path).is_err());
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn bigram_quantization_keeps_order() {
        let bytes = encode_lm(
            &[("a", 0.5), ("b", 0.5)],
            &[("a", "b", 0.9), ("b", "a", 0.1)],
        );
        let path = write_temp("order", &bytes);
        let lm = Lm::open(&path).unwrap();
        assert!(lm.bi("a", "b") > lm.bi("b", "a"));
        std::fs::remove_file(path).ok();
    }
}
