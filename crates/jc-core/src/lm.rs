//! 系統語言模型：unigram 與 bigram 機率。
//!
//! 對照前身 Go `core/lm`：通用介面、記憶體實作、TSV 載入、緊湊唯讀格式的
//! 建置與編解碼，以及各字數的 unigram 分位數。
//!
//! 與 `jc-data` 的 `zh::lm` 的關係：那裡是 `lm.bin` 的零拷貝讀取器，只負責
//! 驗證與查詢既有檔案；本 module 另外提供可寫入的 [`Mem`]、可離線建置的
//! [`Compact`]，並對 `jc_data::zh::Lm` 實作 [`System`]，讓注音與日文共用同一個
//! 機率介面。兩者的檔案格式相同（`ZUINLM` 版本 2）。

use std::collections::HashMap;
use std::io::BufRead;
use std::sync::OnceLock;

use thiserror::Error;

/// 句首的虛擬詞。
pub const BOS: &str = "<s>";

/// 未登錄詞的 unigram 機率。
pub const UNSEEN: f64 = 1e-8;

/// `lm.bin` 的魔術字。
const MAGIC: &[u8] = b"ZUINLM\0\0";
/// `lm.bin` 的格式版本。
const VERSION: u32 = 2;

/// 量化：`log10 P ∈ [-Q_RANGE, 0]` 對應到 0..255；一階約 0.031。
const Q_RANGE: f64 = 8.0;

/// 語言模型的錯誤。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LmError {
    /// TSV 的某一行格式錯誤。
    #[error("第 {line} 行：{message}")]
    Line {
        /// 行號，從 1 起。
        line: usize,
        /// 原因。
        message: String,
    },
    /// 建置或解碼二進位資料時失敗。
    #[error("{0}")]
    Format(String),
}

impl LmError {
    fn line(line: usize, message: impl Into<String>) -> Self {
        LmError::Line {
            line,
            message: message.into(),
        }
    }
}

/// 系統語言模型。
pub trait System {
    /// 回傳 `P(w)`；未登錄回傳 [`UNSEEN`]。
    fn uni(&self, word: &str) -> f64;

    /// 回傳 `P(w | left)`；沒有這個 bigram 回傳 0。
    fn bi(&self, left: &str, word: &str) -> f64;
}

/// unigram 機率表。
pub type UniMap = HashMap<String, f64>;

/// bigram 機率表，鍵是（前詞, 後詞）。
pub type BiMap = HashMap<(String, String), f64>;

/// 記憶體內的實作。
#[derive(Debug, Clone, Default)]
pub struct Mem {
    uni: UniMap,
    bi: BiMap,
}

impl Mem {
    /// 建立空的語言模型。
    pub fn new() -> Self {
        Self::default()
    }

    /// 設定 unigram 機率。
    pub fn set_uni(&mut self, word: impl Into<String>, p: f64) {
        self.uni.insert(word.into(), p);
    }

    /// 設定 bigram 機率。
    pub fn set_bi(&mut self, left: impl Into<String>, word: impl Into<String>, p: f64) {
        self.bi.insert((left.into(), word.into()), p);
    }

    /// 回傳內部兩張表的唯讀參考，供 [`Compact::build`] 使用。
    pub fn entries(&self) -> (&UniMap, &BiMap) {
        (&self.uni, &self.bi)
    }

    /// 是否有以 `left` 為前詞的 bigram。
    pub fn has_left(&self, left: &str) -> bool {
        self.bi.keys().any(|(l, _)| l == left)
    }

    /// 各字數詞的 unigram 分位數。
    ///
    /// `Mem` 可以再加詞，所以每次重算。
    pub fn uni_quantile_by_len(&self) -> Vec<f64> {
        quantile_by_len(self.uni.iter().map(|(w, p)| (w.as_str(), *p)))
    }
}

impl System for Mem {
    fn uni(&self, word: &str) -> f64 {
        self.uni.get(word).copied().unwrap_or(UNSEEN)
    }

    fn bi(&self, left: &str, word: &str) -> f64 {
        self.bi
            .get(&(left.to_string(), word.to_string()))
            .copied()
            .unwrap_or(0.0)
    }
}

/// 讀取 TSV。
///
/// 兩欄（詞、機率）為 unigram；三欄（前詞、詞、機率）為 bigram。
pub fn load(reader: impl BufRead) -> Result<Mem, LmError> {
    let mut model = Mem::new();
    for (index, line) in reader.lines().enumerate() {
        let line = line.map_err(|err| LmError::line(index + 1, err.to_string()))?;
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 2 && fields.len() != 3 {
            return Err(LmError::line(index + 1, "需要 2 或 3 欄"));
        }
        let raw = fields[fields.len() - 1];
        let p: f64 = raw
            .parse()
            .map_err(|_| LmError::line(index + 1, format!("機率 {raw:?} 須在 (0,1]")))?;
        if !(p > 0.0 && p <= 1.0) {
            return Err(LmError::line(index + 1, format!("機率 {raw:?} 須在 (0,1]")));
        }
        if fields.len() == 2 {
            model.set_uni(fields[0], p);
        } else {
            model.set_bi(fields[0], fields[1], p);
        }
    }
    Ok(model)
}

/// 量化一個機率。
fn quantize(p: f64) -> u8 {
    let x = -p.log10() / Q_RANGE * 255.0;
    x.clamp(0.0, 255.0).round() as u8
}

/// 還原一個量化值。
fn dequantize(b: u8) -> f64 {
    10f64.powf(-(b as f64) / 255.0 * Q_RANGE)
}

fn quantize_table() -> [f64; 256] {
    let mut table = [0.0; 256];
    for (index, slot) in table.iter_mut().enumerate() {
        *slot = dequantize(index as u8);
    }
    table
}

/// 緊湊的唯讀語言模型。
///
/// 詞依字串排序，unigram 以二分搜尋查詢。bigram 依（前詞、後詞）排序，
/// 前詞由 `left_start` 隱含：每筆只存後詞 ID（3 位元組）與量化後的機率
/// （1 位元組，log10 刻度），共 4 位元組。
#[derive(Debug, Clone)]
pub struct Compact {
    bucket: HashMap<char, (u32, u32)>,
    strs: String,
    off: Vec<u32>,
    uni: Vec<f32>,
    left_start: Vec<u32>,
    right: Vec<u8>,
    q: Vec<u8>,
    quantile: OnceLock<Vec<f64>>,
}

impl Compact {
    /// 由 unigram 與 bigram 建出緊湊模型。
    ///
    /// bigram 的兩個詞都必須有 unigram，機率必須在 (0,1]。
    pub fn build(uni: &UniMap, bi: &BiMap) -> Result<Self, LmError> {
        let mut words: Vec<&str> = uni.keys().map(String::as_str).collect();
        for (word, p) in uni {
            if !(*p > 0.0 && *p <= 1.0) {
                return Err(LmError::Format(format!("{word:?} 的機率 {p} 不在 (0,1]")));
            }
        }
        words.sort_unstable();
        if words.len() >= 1 << 24 {
            return Err(LmError::Format("詞數超過 3 位元組 ID 的上限".to_string()));
        }
        let mut compact = Compact {
            bucket: HashMap::new(),
            strs: String::new(),
            off: Vec::with_capacity(words.len() + 1),
            uni: Vec::with_capacity(words.len()),
            left_start: Vec::new(),
            right: Vec::new(),
            q: Vec::new(),
            quantile: OnceLock::new(),
        };
        let mut builder = String::new();
        for word in &words {
            compact.off.push(builder.len() as u32);
            builder.push_str(word);
            compact.uni.push(uni[*word] as f32);
        }
        compact.off.push(builder.len() as u32);
        compact.strs = builder;
        compact.index();

        let mut pairs: Vec<(u32, u32, u8)> = Vec::with_capacity(bi.len());
        for ((left, right), p) in bi {
            let Some(l) = compact.find(left) else {
                return Err(LmError::Format(format!(
                    "bigram {left:?}→{right:?} 的詞沒有 unigram"
                )));
            };
            let Some(r) = compact.find(right) else {
                return Err(LmError::Format(format!(
                    "bigram {left:?}→{right:?} 的詞沒有 unigram"
                )));
            };
            if !(*p > 0.0 && *p <= 1.0) {
                return Err(LmError::Format(format!(
                    "bigram {left:?}→{right:?} 的機率 {p} 不在 (0,1]"
                )));
            }
            pairs.push((l as u32, r as u32, quantize(*p)));
        }
        pairs.sort_unstable_by_key(|(left, right, _)| (*left, *right));
        compact.left_start = vec![0; words.len() + 1];
        compact.right = vec![0; 3 * pairs.len()];
        compact.q = vec![0; pairs.len()];
        let mut k = 0;
        for i in 0..words.len() {
            compact.left_start[i] = k as u32;
            while k < pairs.len() && pairs[k].0 as usize == i {
                let (_, right, q) = pairs[k];
                compact.right[3 * k] = right as u8;
                compact.right[3 * k + 1] = (right >> 8) as u8;
                compact.right[3 * k + 2] = (right >> 16) as u8;
                compact.q[k] = q;
                k += 1;
            }
        }
        compact.left_start[words.len()] = k as u32;
        Ok(compact)
    }

    fn right_at(&self, index: usize) -> u32 {
        u32::from_le_bytes([
            self.right[3 * index],
            self.right[3 * index + 1],
            self.right[3 * index + 2],
            0,
        ])
    }

    /// 建立首字分桶：以同一個字開頭的詞在排序後是連續的一段。
    fn index(&mut self) {
        self.bucket.clear();
        for index in 0..self.uni.len() {
            let Some(first) = self.word(index).chars().next() else {
                continue;
            };
            let entry = self.bucket.entry(first).or_insert((index as u32, 0));
            entry.1 = (index + 1) as u32;
        }
    }

    fn word(&self, index: usize) -> &str {
        &self.strs[self.off[index] as usize..self.off[index + 1] as usize]
    }

    fn find(&self, word: &str) -> Option<usize> {
        let first = word.chars().next()?;
        let (lo, hi) = *self.bucket.get(&first)?;
        let (lo, hi) = (lo as usize, hi as usize);
        let mut low = lo;
        let mut high = hi;
        while low < high {
            let mid = low + (high - low) / 2;
            if self.word(mid) < word {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        (low < hi && self.word(low) == word).then_some(low)
    }

    /// 詞數。
    pub fn words(&self) -> usize {
        self.uni.len()
    }

    /// bigram 筆數。
    pub fn bigrams(&self) -> usize {
        self.q.len()
    }

    /// 是否有以 `left` 為前詞的 bigram。
    pub fn has_left(&self, left: &str) -> bool {
        if self.q.is_empty() {
            return false;
        }
        self.find(left)
            .is_some_and(|l| self.left_start[l] != self.left_start[l + 1])
    }

    /// 依前詞、後詞的順序列出所有 bigram 與量化後的機率。
    pub fn each_bigram(&self, mut f: impl FnMut(&str, &str, f64)) {
        let table = quantize_table();
        for left in 0..self.uni.len() {
            for index in self.left_start[left] as usize..self.left_start[left + 1] as usize {
                f(
                    self.word(left),
                    self.word(self.right_at(index) as usize),
                    table[self.q[index] as usize],
                );
            }
        }
    }

    /// 各字數詞的 unigram 在 [`VOCAB_QUANTILE`] 的值。
    ///
    /// 第 n 項是 n 個字的詞（第 0 項不用）。只算一次，之後重用。
    pub fn uni_quantile_by_len(&self) -> &[f64] {
        self.quantile.get_or_init(|| {
            quantile_by_len((0..self.uni.len()).map(|i| (self.word(i), self.uni[i] as f64)))
        })
    }

    /// 依 Go `core/lm` 的 `ZUINLM` 版本 2 格式編碼。
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        put_string(&mut out, &self.strs);
        put_u32s(&mut out, &self.off);
        put_f32s(&mut out, &self.uni);
        put_u32s(&mut out, &self.left_start);
        put_bytes(&mut out, &self.right);
        put_bytes(&mut out, &self.q);
        let crc = crc32fast::hash(&out);
        out.extend_from_slice(&crc.to_le_bytes());
        out
    }

    /// 解碼 `ZUINLM` 版本 2 的位元組，並驗證各欄長度、排序與 CRC32。
    pub fn decode(bytes: &[u8]) -> Result<Self, LmError> {
        let mut reader = jc_data::zh::format::Reader::new(bytes, MAGIC, VERSION)
            .map_err(|err| LmError::Format(err.to_string()))?;
        let strs = reader
            .string()
            .map_err(|err| LmError::Format(err.to_string()))?
            .to_string();
        let off = to_u32s(
            reader
                .u32s()
                .map_err(|err| LmError::Format(err.to_string()))?,
        );
        let uni = to_f32s(
            reader
                .f32s()
                .map_err(|err| LmError::Format(err.to_string()))?,
        );
        let left_start = to_u32s(
            reader
                .u32s()
                .map_err(|err| LmError::Format(err.to_string()))?,
        );
        let right = reader
            .bytes()
            .map_err(|err| LmError::Format(err.to_string()))?
            .to_vec();
        let q = reader
            .bytes()
            .map_err(|err| LmError::Format(err.to_string()))?
            .to_vec();
        reader
            .done()
            .map_err(|err| LmError::Format(err.to_string()))?;

        let words = uni.len();
        if off.len() != words + 1 || left_start.len() != words + 1 || right.len() != 3 * q.len() {
            return Err(LmError::Format("資料檔各欄長度不一致".to_string()));
        }
        if left_start[words] as usize != q.len() {
            return Err(LmError::Format("資料檔各欄長度不一致".to_string()));
        }
        let mut compact = Compact {
            bucket: HashMap::new(),
            strs,
            off,
            uni,
            left_start,
            right,
            q,
            quantile: OnceLock::new(),
        };
        for i in 0..words {
            if compact.off[i] > compact.off[i + 1]
                || compact.off[i + 1] as usize > compact.strs.len()
                || !compact.strs.is_char_boundary(compact.off[i] as usize)
                || !compact.strs.is_char_boundary(compact.off[i + 1] as usize)
            {
                return Err(LmError::Format(format!("詞 {i} 的位移超出範圍")));
            }
            if i > 0 && compact.word(i - 1) >= compact.word(i) {
                return Err(LmError::Format("詞表未排序".to_string()));
            }
            if compact.left_start[i] > compact.left_start[i + 1] {
                return Err(LmError::Format("bigram 索引未排序".to_string()));
            }
            for k in compact.left_start[i] as usize..compact.left_start[i + 1] as usize {
                if compact.right_at(k) as usize >= words
                    || (k > compact.left_start[i] as usize
                        && compact.right_at(k - 1) >= compact.right_at(k))
                {
                    return Err(LmError::Format(
                        "bigram 後詞 ID 超出範圍或未排序".to_string(),
                    ));
                }
            }
        }
        compact.index();
        Ok(compact)
    }
}

impl System for Compact {
    fn uni(&self, word: &str) -> f64 {
        match self.find(word) {
            Some(index) => self.uni[index] as f64,
            None => UNSEEN,
        }
    }

    fn bi(&self, left: &str, word: &str) -> f64 {
        if self.q.is_empty() {
            return 0.0;
        }
        let Some(l) = self.find(left) else {
            return 0.0;
        };
        if self.left_start[l] == self.left_start[l + 1] {
            return 0.0;
        }
        let Some(r) = self.find(word) else {
            return 0.0;
        };
        let lo = self.left_start[l] as usize;
        let hi = self.left_start[l + 1] as usize;
        let mut low = lo;
        let mut high = hi;
        while low < high {
            let mid = low + (high - low) / 2;
            if self.right_at(mid) < r as u32 {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        if low < hi && self.right_at(low) == r as u32 {
            quantize_table()[self.q[low] as usize]
        } else {
            0.0
        }
    }
}

impl System for jc_data::zh::Lm {
    fn uni(&self, word: &str) -> f64 {
        jc_data::zh::Lm::uni(self, word)
    }

    fn bi(&self, left: &str, word: &str) -> f64 {
        jc_data::zh::Lm::bi(self, left, word)
    }
}

fn put_string(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u32).to_le_bytes());
    out.extend_from_slice(value.as_bytes());
}

fn put_u32s(out: &mut Vec<u8>, values: &[u32]) {
    out.extend_from_slice(&(values.len() as u32).to_le_bytes());
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

fn put_f32s(out: &mut Vec<u8>, values: &[f32]) {
    out.extend_from_slice(&(values.len() as u32).to_le_bytes());
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

fn put_bytes(out: &mut Vec<u8>, values: &[u8]) {
    out.extend_from_slice(&(values.len() as u32).to_le_bytes());
    out.extend_from_slice(values);
}

fn to_u32s(bytes: &[u8]) -> Vec<u32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| u32::from_le_bytes(*chunk))
        .collect()
}

fn to_f32s(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect()
}

/// 自訂詞下限取的百分位。
///
/// 中位數不行：七成以上的系統詞停在量化的最低機率，各字數的中位數都一樣，
/// 自訂姓名會輸給單字拼湊；第 90 百分位約是常見詞的入門。
pub const VOCAB_QUANTILE: f64 = 0.9;

fn quantile_by_len<'a>(items: impl Iterator<Item = (&'a str, f64)>) -> Vec<f64> {
    let mut by_len: Vec<Vec<f64>> = Vec::new();
    for (word, p) in items {
        let n = word.chars().count();
        while by_len.len() <= n {
            by_len.push(Vec::new());
        }
        by_len[n].push(p);
    }
    let mut out = vec![0.0; by_len.len()];
    for (n, mut ps) in by_len.into_iter().enumerate() {
        if ps.is_empty() {
            continue;
        }
        ps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        out[n] = ps[(VOCAB_QUANTILE * (ps.len() - 1) as f64) as usize];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compact_matches_mem() {
        let path = jc_testdata::require_data!("0c-testdata/golden/zh/data/lm.tsv");
        let file = std::fs::File::open(path).expect("開啟 lm.tsv");
        let model = load(std::io::BufReader::new(file)).expect("載入 lm.tsv");
        let built = Compact::build(model.entries().0, model.entries().1).expect("建置緊湊模型");
        let compact = Compact::decode(&built.encode()).expect("解碼緊湊模型");

        let (uni, bi) = model.entries();
        let mut words = vec!["不存在".to_string(), BOS.to_string(), String::new()];
        for (word, p) in uni {
            words.push(word.clone());
            assert_eq!(compact.uni(word) as f32, *p as f32, "Uni({word:?})");
        }
        for left in &words {
            for word in &words {
                let want = bi
                    .get(&(left.clone(), word.clone()))
                    .copied()
                    .unwrap_or(0.0);
                let got = compact.bi(left, word);
                let matches = (want == 0.0) == (got == 0.0)
                    && (want == 0.0 || (got / want - 1.0).abs() <= 0.037);
                assert!(matches, "Bi({left:?},{word:?}) = {got} 應約為 {want}");
            }
        }
        for left in &words {
            assert_eq!(
                compact.has_left(left),
                model.has_left(left),
                "HasLeft({left:?})"
            );
        }
        let mut count = 0;
        compact.each_bigram(|left, word, p| {
            count += 1;
            assert_eq!(p, compact.bi(left, word), "EachBigram({left:?},{word:?})");
        });
        assert_eq!(count, bi.len());
        assert_eq!(count, compact.bigrams());
        assert_eq!(compact.uni("不存在"), UNSEEN);

        let missing = HashMap::from([("a".to_string(), 0.1)]);
        let dangling = HashMap::from([(("a".to_string(), "b".to_string()), 0.1)]);
        assert!(
            Compact::build(&missing, &dangling).is_err(),
            "bigram 的詞必須有 unigram"
        );

        let mut bytes = built.encode();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 1;
        assert!(Compact::decode(&bytes).is_err(), "CRC 應抓到損毀");
    }

    #[test]
    fn test_quantize() {
        for p in [1.0, 0.5, 0.1, 1e-3, 1e-6, 1e-8] {
            assert!(
                (dequantize(quantize(p)) / p - 1.0).abs() <= 0.037,
                "{p} → {}",
                dequantize(quantize(p))
            );
        }
        assert_eq!(quantize(1e-12), 255, "超出範圍應夾在兩端");
        assert_eq!(quantize(1.0), 0, "超出範圍應夾在兩端");
    }

    #[test]
    fn test_load_rejects_malformed_lines() {
        assert!(load("只有一欄\n".as_bytes()).is_err());
        assert!(load("詞\t0\n".as_bytes()).is_err());
        assert!(load("詞\t2\n".as_bytes()).is_err());
        assert!(load("# 註解\n\n詞\t0.5\n".as_bytes()).is_ok());
    }

    #[test]
    fn test_quantile_by_len_uses_ordinal_position() {
        let model = Compact::build(&HashMap::from([("我".to_string(), 0.5)]), &HashMap::new())
            .expect("建置");
        let quantiles = model.uni_quantile_by_len();
        assert_eq!(quantiles.len(), 2);
        assert_eq!(quantiles[0], 0.0);
        assert_eq!(quantiles[1], 0.5);
    }
}
