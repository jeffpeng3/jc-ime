//! 日文轉換執行期的語言模型：詞面 unigram 與 bigram 成本。
//!
//! 成本是負對數機率，越低越優先；查詢鍵是詞面字串，不含讀音、詞性與
//! backoff。這支 module 先提供記憶體實作與 `0CJLM001` 格式的解碼，讓
//! curated 資料與測試可以在沒有 `lm.bin` 的情況下先跑；真正的產品
//! `lm.bin` 之後由 `jc-data` 編譯器產生再接上。
//!
//! core 不碰檔案系統：呼叫端自己讀檔後交給 [`LanguageModel::decode`]。

use std::collections::{HashMap, HashSet};

use thiserror::Error;

/// `0CJLM001` 格式的 magic。
pub const MAGIC: &[u8; 8] = b"0CJLM001";

/// 語言模型解碼或建構時的錯誤。
#[derive(Debug, Error)]
pub enum LmError {
    /// magic 不符。
    #[error("不是 0c 日文語言模型檔")]
    BadMagic,
    /// 資料長度不足。
    #[error("語言模型資料在 offset {0} 截斷")]
    Truncated(usize),
    /// 字串不是合法 UTF-8。
    #[error("語言模型字串在 offset {0} 不是合法 UTF-8")]
    InvalidUtf8(usize),
    /// uvarint 超過 64 位元。
    #[error("uvarint 溢位")]
    VarintOverflow,
    /// unigram 數量不合理。
    #[error("unigram 數量 {0} 不合法")]
    BadUnigramCount(u64),
    /// bigram 數量不合理。
    #[error("bigram 數量 {0} 不合法")]
    BadBigramCount(u64),
    /// 成本是 NaN 或 Inf。
    #[error("成本 {0} 不合法")]
    BadCost(f64),
    /// 解碼完還有剩餘位元組。
    #[error("語言模型檔尾端有 {0} 位元組未使用")]
    Trailing(usize),
}

/// 詞面 unigram 與 bigram 的語言模型。
///
/// 未登錄的詞面成本為 0，代表沒有 LM 證據、不額外懲罰；沒有 bigram 時
/// 只回傳 unigram。查詢語意與前身 `core/ja/lm` 相同。
#[derive(Debug, Clone, Default)]
pub struct LanguageModel {
    unigram: HashMap<String, f64>,
    bigram: HashMap<String, HashMap<String, f64>>,
    left: HashSet<String>,
    encoded: usize,
}

impl LanguageModel {
    /// 建立空模型。
    pub fn new() -> Self {
        Self::default()
    }

    /// 由 unigram／bigram 建立記憶體模型；`encoded` 為 0。
    pub fn from_pairs(
        unigram: impl IntoIterator<Item = (String, f64)>,
        bigram: impl IntoIterator<Item = ((String, String), f64)>,
    ) -> Self {
        let mut model = Self::new();
        for (word, cost) in unigram {
            model.unigram.insert(word, cost);
        }
        for ((left, right), cost) in bigram {
            model.left.insert(left.clone());
            model.bigram.entry(left).or_default().insert(right, cost);
        }
        model
    }

    /// 加入或覆蓋一筆 unigram。
    pub fn insert_unigram(&mut self, word: impl Into<String>, cost: f64) {
        self.unigram.insert(word.into(), cost);
    }

    /// 加入或覆蓋一筆 bigram，並記錄左詞。
    pub fn insert_bigram(&mut self, left: impl Into<String>, right: impl Into<String>, cost: f64) {
        let left = left.into();
        self.left.insert(left.clone());
        self.bigram
            .entry(left)
            .or_default()
            .insert(right.into(), cost);
    }

    /// 解碼 `0CJLM001` 格式。格式錯誤回傳錯誤，不嘗試修補。
    pub fn decode(data: &[u8]) -> Result<Self, LmError> {
        if data.len() < MAGIC.len() || &data[..MAGIC.len()] != MAGIC {
            return Err(LmError::BadMagic);
        }
        let mut reader = Reader {
            data,
            pos: MAGIC.len(),
        };

        let count = reader.uvarint()?;
        if count > reader.remaining() as u64 / 8 {
            return Err(LmError::BadUnigramCount(count));
        }
        let mut model = Self::with_capacity(count as usize);
        for _ in 0..count {
            let word = reader.string()?;
            let cost = reader.cost()?;
            model.unigram.insert(word, cost);
        }

        let bigram_count = reader.uvarint()?;
        if bigram_count > reader.remaining() as u64 / 10 {
            return Err(LmError::BadBigramCount(bigram_count));
        }
        model.bigram.reserve(bigram_count as usize);
        for _ in 0..bigram_count {
            let left = reader.string()?;
            let right = reader.string()?;
            let cost = reader.cost()?;
            model.left.insert(left.clone());
            model.bigram.entry(left).or_default().insert(right, cost);
        }

        if reader.remaining() != 0 {
            return Err(LmError::Trailing(reader.remaining()));
        }
        model.encoded = data.len();
        Ok(model)
    }

    fn with_capacity(unigram: usize) -> Self {
        Self {
            unigram: HashMap::with_capacity(unigram),
            bigram: HashMap::new(),
            left: HashSet::new(),
            encoded: 0,
        }
    }

    /// unigram 詞數。
    pub fn len(&self) -> usize {
        self.unigram.len()
    }

    /// 是否沒有任何 unigram。
    pub fn is_empty(&self) -> bool {
        self.unigram.is_empty()
    }

    /// bigram 筆數。
    pub fn bigram_len(&self) -> usize {
        self.bigram.values().map(HashMap::len).sum()
    }

    /// 來源 `lm.bin` 的位元組數；記憶體模型為 0。
    pub fn encoded_size(&self) -> usize {
        self.encoded
    }

    /// 詞面 `word` 的 unigram 成本；未登錄回傳 0。
    pub fn unigram_cost(&self, word: &str) -> f64 {
        self.unigram.get(word).copied().unwrap_or(0.0)
    }

    /// `left` 接 `word` 的 bigram 成本；未登錄或 `left` 為空回傳 0。
    pub fn bigram_cost(&self, left: &str, word: &str) -> f64 {
        if left.is_empty() {
            return 0.0;
        }
        self.bigram
            .get(left)
            .and_then(|right| right.get(word))
            .copied()
            .unwrap_or(0.0)
    }

    /// `left` 之後接 `word` 的 LM 成本，等於 unigram + bigram。
    ///
    /// `left` 為空表示句首，只算 unigram。
    pub fn cost(&self, left: &str, word: &str) -> f64 {
        self.unigram_cost(word) + self.bigram_cost(left, word)
    }

    /// `left` 是否為任何 bigram 的左詞。
    ///
    /// Viterbi 用它決定是否要把前一個詞面放進狀態；不是左詞的歷史在
    /// bigram 上完全等價。
    pub fn has_left(&self, left: &str) -> bool {
        !left.is_empty() && self.left.contains(left)
    }
}

/// `0CJLM001` 的解碼游標。
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn byte(&mut self) -> Result<u8, LmError> {
        let value = *self
            .data
            .get(self.pos)
            .ok_or(LmError::Truncated(self.pos))?;
        self.pos += 1;
        Ok(value)
    }

    fn uvarint(&mut self) -> Result<u64, LmError> {
        let mut value = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = self.byte()?;
            if byte < 0x80 {
                if shift == 63 && byte > 1 {
                    return Err(LmError::VarintOverflow);
                }
                value |= u64::from(byte) << shift;
                return Ok(value);
            }
            if shift >= 63 {
                return Err(LmError::VarintOverflow);
            }
            value |= u64::from(byte & 0x7f) << shift;
            shift += 7;
        }
    }

    fn string(&mut self) -> Result<String, LmError> {
        let length = self.uvarint()?;
        if length > self.remaining() as u64 {
            return Err(LmError::Truncated(self.pos));
        }
        let start = self.pos;
        self.pos += length as usize;
        std::str::from_utf8(&self.data[start..self.pos])
            .map(str::to_owned)
            .map_err(|_| LmError::InvalidUtf8(start))
    }

    fn cost(&mut self) -> Result<f64, LmError> {
        if self.remaining() < 8 {
            return Err(LmError::Truncated(self.pos));
        }
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&self.data[self.pos..self.pos + 8]);
        self.pos += 8;
        let value = f64::from_le_bytes(bytes);
        if !value.is_finite() {
            return Err(LmError::BadCost(value));
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 依 `0CJLM001` 格式編碼，角色等同 Go 測試的 `jaimport.NewLM().Encode()`。
    fn encode(unigram: &[(&str, f64)], bigram: &[(&str, &str, f64)]) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        put_uvarint(&mut out, unigram.len() as u64);
        for (word, cost) in unigram {
            put_string(&mut out, word);
            out.extend_from_slice(&cost.to_le_bytes());
        }
        put_uvarint(&mut out, bigram.len() as u64);
        for (left, right, cost) in bigram {
            put_string(&mut out, left);
            put_string(&mut out, right);
            out.extend_from_slice(&cost.to_le_bytes());
        }
        out
    }

    fn put_uvarint(out: &mut Vec<u8>, mut value: u64) {
        while value >= 0x80 {
            out.push((value as u8) | 0x80);
            value >>= 7;
        }
        out.push(value as u8);
    }

    fn put_string(out: &mut Vec<u8>, value: &str) {
        put_uvarint(out, value.len() as u64);
        out.extend_from_slice(value.as_bytes());
    }

    #[test]
    fn test_decode_matches_compiler_format() {
        let data = encode(&[("私", 1.25), ("は", 0.5)], &[("私", "は", 2.5)]);
        let model = LanguageModel::decode(&data).unwrap();
        assert_eq!(model.len(), 2);
        assert_eq!(model.bigram_len(), 1);
        assert_eq!(model.encoded_size(), data.len());
        assert_eq!(model.unigram_cost("私"), 1.25);
        assert_eq!(model.unigram_cost("知らない"), 0.0);
        assert_eq!(model.bigram_cost("私", "は"), 2.5);
        assert_eq!(model.bigram_cost("は", "私"), 0.0);
        assert!(model.has_left("私"));
        assert!(!model.has_left("は"));
        assert_eq!(model.cost("私", "は"), 3.0);
        assert_eq!(model.cost("", "は"), 0.5);
    }

    #[test]
    fn test_decode_empty_model() {
        let model = LanguageModel::decode(&encode(&[], &[])).unwrap();
        assert_eq!(model.len(), 0);
        assert_eq!(model.bigram_len(), 0);
        assert!(model.is_empty());
        assert_eq!(model.cost("左", "右"), 0.0);
    }

    #[test]
    fn test_decode_rejects_malformed() {
        let good = encode(&[("語", 1.0)], &[("語", "語", 2.0)]);
        let mut trailing = good.clone();
        trailing.push(0);
        let mut bad_magic = good.clone();
        bad_magic[..8].copy_from_slice(b"NOPE0000");
        let cases = [
            ("空檔", Vec::new()),
            ("magic", bad_magic),
            ("截斷", good[..good.len() - 1].to_vec()),
            ("尾端", trailing),
        ];
        for (name, data) in cases {
            let err = LanguageModel::decode(&data).expect_err(name);
            match name {
                "空檔" | "magic" => assert!(matches!(err, LmError::BadMagic), "{name}"),
                "截斷" => assert!(matches!(err, LmError::Truncated(_)), "{name}"),
                "尾端" => assert!(matches!(err, LmError::Trailing(1)), "{name}"),
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn test_negative_cost_accepted() {
        let data = encode(&[("x", -1.5)], &[]);
        let model = LanguageModel::decode(&data).unwrap();
        assert_eq!(model.unigram_cost("x"), -1.5);
    }

    #[test]
    fn test_nan_rejected() {
        let mut data = encode(&[("x", 1.0)], &[]);
        let nan = f64::NAN.to_bits();
        let start = data.len() - 8;
        data[start..].copy_from_slice(&nan.to_le_bytes());
        assert!(matches!(
            LanguageModel::decode(&data),
            Err(LmError::BadCost(_))
        ));
    }

    #[test]
    fn test_empty_model_is_neutral() {
        let model = LanguageModel::new();
        assert_eq!(model.len(), 0);
        assert_eq!(model.bigram_len(), 0);
        assert_eq!(model.encoded_size(), 0);
        assert_eq!(model.cost("a", "b"), 0.0);
        assert!(!model.has_left("a"));
    }

    #[test]
    fn test_from_pairs_builds_left_set() {
        let model = LanguageModel::from_pairs(
            vec![("私".to_owned(), 1.25), ("は".to_owned(), 0.5)],
            vec![(("私".to_owned(), "は".to_owned()), 2.5)],
        );
        assert_eq!(model.len(), 2);
        assert_eq!(model.bigram_len(), 1);
        assert_eq!(model.cost("私", "は"), 3.0);
        assert!(model.has_left("私"));
        assert!(!model.has_left("は"));
    }
}
