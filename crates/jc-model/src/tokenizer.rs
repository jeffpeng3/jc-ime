//! 0c SentencePiece 詞表與前處理。
//!
//! 模型包附的 `vocab.json` 格式是 `0c.sentencepiece.vocab/1`：只有
//! `id_to_piece` 與 `special_tokens`，**沒有** SentencePiece 的 piece
//! log-probability。因此這裡不能用 unigram Viterbi 還原訓練時的切法，
//! 改採最長前綴匹配（maximal munch），找不到 piece 的位元組再走 byte
//! fallback。這個限制寫在 `docs/port/jc-model.md`。
//!
//! 正規化沿用前身 `internal/spm` 的 identity normalizer：移除多餘空白、
//! 加上 dummy prefix、把空白轉成 U+2581。

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::error::{ModelError, Result};

/// 支援的 vocab 格式。
pub const VOCAB_SCHEMA: &str = "0c.sentencepiece.vocab/1";

/// 空白轉義字元（U+2581）。
pub const SPACE: char = '\u{2581}';

/// 前處理後的 tokenizer。
#[derive(Debug, Clone)]
pub struct Tokenizer {
    pieces: Vec<String>,
    ids: HashMap<String, u32>,
    byte_ids: [Option<u32>; 256],
    max_piece_bytes: usize,
    unk_id: u32,
    bos_id: u32,
    eos_id: u32,
    pad_id: u32,
    model_name: String,
}

#[derive(Debug, Deserialize)]
struct RawVocab {
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    vocab_size: Option<usize>,
    #[serde(default)]
    special_tokens: BTreeMap<String, u32>,
    id_to_piece: Vec<String>,
}

impl Tokenizer {
    /// 從 `vocab.json` 載入。
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = fs::read(path)?;
        Self::from_json(&raw).map_err(|e| ModelError::tokenizer(format!("{}：{e}", path.display())))
    }

    /// 從 JSON bytes 載入。
    pub fn from_json(raw: &[u8]) -> Result<Self> {
        let v: RawVocab = serde_json::from_slice(raw)
            .map_err(|e| ModelError::tokenizer(format!("JSON 解析失敗：{e}")))?;
        if let Some(format) = &v.format {
            if format != VOCAB_SCHEMA {
                return Err(ModelError::tokenizer(format!(
                    "格式應為 {VOCAB_SCHEMA}，實際為 {format}"
                )));
            }
        }
        if v.id_to_piece.is_empty() {
            return Err(ModelError::tokenizer("id_to_piece 是空的"));
        }
        if let Some(size) = v.vocab_size {
            if size != v.id_to_piece.len() {
                return Err(ModelError::tokenizer(format!(
                    "vocab_size 為 {size}，但 id_to_piece 有 {}",
                    v.id_to_piece.len()
                )));
            }
        }
        let unk_id = *v.special_tokens.get("unk").unwrap_or(&1);
        let bos_id = *v.special_tokens.get("bos").unwrap_or(&2);
        let eos_id = *v.special_tokens.get("eos").unwrap_or(&3);
        let pad_id = *v.special_tokens.get("pad").unwrap_or(&0);
        let n = v.id_to_piece.len();
        for (name, id) in [
            ("unk", unk_id),
            ("bos", bos_id),
            ("eos", eos_id),
            ("pad", pad_id),
        ] {
            if id as usize >= n {
                return Err(ModelError::tokenizer(format!(
                    "special token {name} 的 id {id} 超出詞表"
                )));
            }
        }

        let mut byte_ids = [None; 256];
        for (b, slot) in byte_ids.iter_mut().enumerate() {
            let upper = format!("<0x{b:02X}>");
            let lower = format!("<0x{b:02x}>");
            if let Some((idx, _)) = v
                .id_to_piece
                .iter()
                .enumerate()
                .find(|(_, p)| **p == upper || **p == lower)
            {
                *slot = Some(idx as u32);
            }
        }

        let special_ids = [unk_id, bos_id, eos_id, pad_id];
        let mut ids = HashMap::with_capacity(v.id_to_piece.len());
        let mut max_piece_bytes = 0;
        for (idx, piece) in v.id_to_piece.iter().enumerate() {
            let id = idx as u32;
            if piece.is_empty()
                || special_ids.contains(&id)
                || piece.starts_with("<0x") && piece.ends_with('>')
            {
                continue;
            }
            ids.entry(piece.clone()).or_insert(id);
            max_piece_bytes = max_piece_bytes.max(piece.len());
        }
        // 避免使用者輸入的超長字串讓匹配退化成 O(n^2)。
        let max_piece_bytes = max_piece_bytes.min(128);

        Ok(Self {
            pieces: v.id_to_piece,
            ids,
            byte_ids,
            max_piece_bytes,
            unk_id,
            bos_id,
            eos_id,
            pad_id,
            model_name: v.model.unwrap_or_default(),
        })
    }

    /// 詞表大小。
    pub fn len(&self) -> usize {
        self.pieces.len()
    }

    /// 詞表是否為空（合法 tokenizer 不會是空的）。
    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    /// 模型名稱（vocab.json 的 `model` 欄位）。
    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    /// `<unk>` id。
    pub fn unk_id(&self) -> u32 {
        self.unk_id
    }

    /// `<s>` id。
    pub fn bos_id(&self) -> u32 {
        self.bos_id
    }

    /// `</s>` id。
    pub fn eos_id(&self) -> u32 {
        self.eos_id
    }

    /// `<pad>` id。
    pub fn pad_id(&self) -> u32 {
        self.pad_id
    }

    /// 查 piece 的 id；不存在時回傳 `None`。
    pub fn piece_id(&self, piece: &str) -> Option<u32> {
        self.ids.get(piece).copied()
    }

    /// 前處理：trim、合併空白、加 dummy prefix、把空白轉成 U+2581。
    pub fn normalize(&self, text: &str) -> String {
        if text.is_empty() {
            return String::new();
        }
        let trimmed = text.trim_start_matches(' ');
        if trimmed.is_empty() {
            return String::new();
        }
        let mut out = String::with_capacity(trimmed.len() + 4);
        out.push(SPACE);
        let mut prev_space = true;
        for ch in trimmed.chars() {
            if ch == ' ' {
                if prev_space {
                    continue;
                }
                out.push(SPACE);
                prev_space = true;
            } else {
                out.push(ch);
                prev_space = false;
            }
        }
        while out.ends_with(SPACE) {
            out.pop();
        }
        out
    }

    /// 編碼成 token id。
    pub fn encode(&self, text: &str) -> Vec<u32> {
        let norm = self.normalize(text);
        let mut out = Vec::new();
        if norm.is_empty() {
            return out;
        }
        let mut pos = 0;
        while pos < norm.len() {
            let mut best: Option<(usize, u32)> = None;
            for (off, _) in norm[pos..].char_indices().skip(1) {
                let end = pos + off;
                if end - pos > self.max_piece_bytes {
                    break;
                }
                if let Some(&id) = self.ids.get(&norm[pos..end]) {
                    best = Some((end, id));
                }
            }
            // 最后一個字元也要試（上面的 skip(1) 不含字串結尾）。
            if best.is_none() {
                if let Some(&id) = self.ids.get(&norm[pos..]) {
                    best = Some((norm.len(), id));
                }
            }
            if let Some((end, id)) = best {
                out.push(id);
                pos = end;
                continue;
            }

            let ch = norm[pos..].chars().next().expect("位置必在字元邊界");
            let next = pos + ch.len_utf8();
            let mut buf = [0u8; 4];
            for b in ch.encode_utf8(&mut buf).as_bytes() {
                out.push(self.byte_ids[*b as usize].unwrap_or(self.unk_id));
            }
            pos = next;
        }
        out
    }

    /// 編碼成 i64，供 ONNX 張量直接使用。
    pub fn encode_i64(&self, text: &str) -> Vec<i64> {
        self.encode(text).into_iter().map(i64::from).collect()
    }

    /// 把 token id 還原成文字；byte fallback 會還原成原始位元組。
    pub fn decode(&self, ids: &[u32]) -> String {
        let mut out = String::new();
        let mut bytes: Vec<u8> = Vec::new();
        for &id in ids {
            if [self.unk_id, self.bos_id, self.eos_id, self.pad_id].contains(&id) {
                continue;
            }
            if let Some(b) = self.byte_value(id) {
                bytes.push(b);
                continue;
            }
            if !bytes.is_empty() {
                out.push_str(&String::from_utf8_lossy(&bytes));
                bytes.clear();
            }
            let Some(piece) = self.pieces.get(id as usize) else {
                continue;
            };
            out.push_str(&piece.replace(SPACE, " "));
        }
        if !bytes.is_empty() {
            out.push_str(&String::from_utf8_lossy(&bytes));
        }
        out
    }

    fn byte_value(&self, id: u32) -> Option<u8> {
        self.byte_ids
            .iter()
            .position(|slot| *slot == Some(id))
            .map(|b| b as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一份小詞表：4 個 special、256 個 byte piece、其餘為測試 piece。
    fn vocab() -> Vec<u8> {
        let mut pieces = vec![
            "<pad>".to_string(),
            "<unk>".to_string(),
            "<s>".to_string(),
            "</s>".to_string(),
        ];
        for b in 0..=255u16 {
            pieces.push(format!("<0x{b:02X}>"));
        }
        for p in [
            "▁",
            "東",
            "京",
            "都",
            "▁東",
            "▁東京",
            "東京",
            "▁東京都",
            "大",
            "阪",
        ] {
            pieces.push(p.to_string());
        }
        let json = serde_json::json!({
            "format": VOCAB_SCHEMA,
            "model": "test",
            "vocab_size": pieces.len(),
            "special_tokens": {"pad": 0, "unk": 1, "bos": 2, "eos": 3},
            "id_to_piece": pieces,
        });
        serde_json::to_vec(&json).unwrap()
    }

    fn tokenizer() -> Tokenizer {
        Tokenizer::from_json(&vocab()).unwrap()
    }

    fn id_of(t: &Tokenizer, piece: &str) -> u32 {
        t.pieces.iter().position(|p| p == piece).unwrap() as u32
    }

    #[test]
    fn normalize_adds_dummy_prefix_and_collapses_spaces() {
        let t = tokenizer();
        assert_eq!(t.normalize("東京都"), "▁東京都");
        assert_eq!(t.normalize("  東京  都  "), "▁東京▁都");
        assert_eq!(t.normalize(""), "");
        assert_eq!(t.normalize("   "), "");
    }

    #[test]
    fn encode_prefers_longest_piece() {
        let t = tokenizer();
        assert_eq!(t.encode("東京都"), vec![id_of(&t, "▁東京都")]);
        assert_eq!(
            t.encode("大阪"),
            vec![id_of(&t, "▁"), id_of(&t, "大"), id_of(&t, "阪")]
        );
    }

    #[test]
    fn encode_spaces_between_words() {
        let t = tokenizer();
        assert_eq!(
            t.encode("東京 都"),
            vec![id_of(&t, "▁東京"), id_of(&t, "▁"), id_of(&t, "都")]
        );
    }

    #[test]
    fn unknown_runes_use_byte_fallback() {
        let t = tokenizer();
        // 「あ」不在詞表，U+3042 = E3 81 82。
        assert_eq!(t.encode("あ"), vec![0xE3, 0x81, 0x82]);
    }

    #[test]
    fn decode_roundtrips_pieces_and_byte_fallback() {
        let t = tokenizer();
        assert_eq!(t.decode(&t.encode("東京都")), "▁東京都");
        assert_eq!(t.decode(&t.encode("あ")), "あ");
        assert_eq!(t.decode(&[t.pad_id()]), "");
    }

    #[test]
    fn bad_schema_is_rejected() {
        let raw = br#"{"format":"something-else","id_to_piece":["<pad>"]}"#;
        match Tokenizer::from_json(raw) {
            Err(ModelError::Tokenizer(msg)) => assert!(msg.contains("0c.sentencepiece.vocab/1")),
            other => panic!("未預期結果：{other:?}"),
        }
    }

    #[test]
    fn vocab_size_mismatch_is_rejected() {
        let raw = br#"{"format":"0c.sentencepiece.vocab/1","vocab_size":5,
            "id_to_piece":["<pad>","<unk>","<s>","</s>"]}"#;
        match Tokenizer::from_json(raw) {
            Err(ModelError::Tokenizer(msg)) => assert!(msg.contains("vocab_size"), "{msg}"),
            other => panic!("未預期結果：{other:?}"),
        }
    }

    #[test]
    fn real_vocab_encodes_japanese() {
        let path = jc_testdata::require_data!("models/reranker-extra/vocab.json");
        let t = Tokenizer::load(&path).unwrap();
        assert_eq!(t.len(), 32768);
        assert_eq!(t.model_name(), "public_core_ja");
        let ids = t.encode("東京都に行く");
        assert!(!ids.is_empty());
        assert!(ids.len() <= "東京都に行く".len() * 3);
        assert!(ids.iter().all(|&id| (id as usize) < t.len()));
        assert_eq!(
            t.decode(&ids),
            t.normalize("東京都に行く").replace(SPACE, " ")
        );
    }
}
