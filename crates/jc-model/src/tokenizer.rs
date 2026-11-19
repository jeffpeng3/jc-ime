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

/// 一個 token 在正規化字串中的 rune 範圍，以及是否來自 byte fallback。
///
/// `start`／`end` 以正規化字串的 rune 索引計算；byte fallback token 的
/// 範圍涵蓋它所屬的單一原字，插入標點時要跳過。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenSpan {
    /// token id。
    pub id: u32,
    /// 起始 rune。
    pub start: usize,
    /// 結束 rune（不含）。
    pub end: usize,
    /// 是否為 byte fallback。
    pub byte: bool,
}

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
        self.normalize_map(text).0
    }

    /// 前處理，並回傳每個正規化 rune 對應的原字串 rune 索引。
    ///
    /// `-1` 表示這個 rune 是前處理插入的（dummy prefix 或空白轉成的
    /// `▁`）；其他值可以直接拿回原文定位。
    pub fn normalize_map(&self, text: &str) -> (String, Vec<i32>) {
        if text.is_empty() {
            return (String::new(), Vec::new());
        }
        let runes: Vec<char> = text.chars().collect();
        let mut first = 0;
        while first < runes.len() && runes[first] == ' ' {
            first += 1;
        }
        if first == runes.len() {
            return (String::new(), Vec::new());
        }
        let mut out = String::with_capacity(text.len() + 4);
        let mut map = Vec::with_capacity(runes.len() + 1);
        out.push(SPACE);
        map.push(-1);
        let mut prev_space = true;
        for (index, ch) in runes.iter().enumerate().skip(first) {
            if *ch == ' ' {
                if prev_space {
                    continue;
                }
                out.push(SPACE);
                map.push(-1);
                prev_space = true;
            } else {
                out.push(*ch);
                map.push(index as i32);
                prev_space = false;
            }
        }
        while out.ends_with(SPACE) {
            out.pop();
            map.pop();
        }
        (out, map)
    }

    /// 編碼成 token id。
    pub fn encode(&self, text: &str) -> Vec<u32> {
        self.encode_with_map(text)
            .0
            .into_iter()
            .map(|span| span.id)
            .collect()
    }

    /// 編碼成 token，並保留每個 token 在正規化字串中的 rune 範圍。
    ///
    /// 回傳 `(tokens, norm, map)`；`map` 的格式見 [`Tokenizer::normalize_map`]。
    /// byte fallback 的 token 標成 `byte=true`，呼叫端不應據此插入標點。
    pub fn encode_with_map(&self, text: &str) -> (Vec<TokenSpan>, String, Vec<i32>) {
        let (norm, map) = self.normalize_map(text);
        let mut out = Vec::new();
        if norm.is_empty() {
            return (out, norm, map);
        }
        let mut pos = 0;
        let mut rune = 0;
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
            // 上面的迴圈只走到最後一個字元前，字串結尾要另外試。
            if norm.len() - pos <= self.max_piece_bytes {
                if let Some(&id) = self.ids.get(&norm[pos..]) {
                    best = Some((norm.len(), id));
                }
            }
            if let Some((end, id)) = best {
                let len = norm[pos..end].chars().count();
                out.push(TokenSpan {
                    id,
                    start: rune,
                    end: rune + len,
                    byte: false,
                });
                rune += len;
                pos = end;
                continue;
            }

            let ch = norm[pos..].chars().next().expect("位置必在字元邊界");
            let next = pos + ch.len_utf8();
            let mut buf = [0u8; 4];
            for b in ch.encode_utf8(&mut buf).as_bytes() {
                out.push(TokenSpan {
                    id: self.byte_ids[*b as usize].unwrap_or(self.unk_id),
                    start: rune,
                    end: rune + 1,
                    byte: true,
                });
            }
            rune += 1;
            pos = next;
        }
        (out, norm, map)
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
    fn encode_with_map_reports_token_spans_and_original_runes() {
        let t = tokenizer();
        let (tokens, norm, map) = t.encode_with_map("東京都 大");
        assert_eq!(norm, "▁東京都▁大");
        assert_eq!(map, vec![-1, 0, 1, 2, -1, 4]);
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].start, 0);
        assert_eq!(tokens[0].end, 4);
        assert!(!tokens[0].byte);
        assert_eq!(tokens[1].start, 4);
        assert_eq!(tokens[1].end, 5);
        assert_eq!(tokens[2].start, 5);
        assert_eq!(tokens[2].end, 6);
        // 與單純編碼的 id 完全一致。
        let ids: Vec<u32> = tokens.iter().map(|token| token.id).collect();
        assert_eq!(ids, t.encode("東京都 大"));
    }

    #[test]
    fn encode_with_map_marks_byte_fallback() {
        let t = tokenizer();
        let (tokens, _norm, map) = t.encode_with_map("あ");
        assert_eq!(map, vec![-1, 0]);
        let bytes: Vec<_> = tokens.iter().filter(|token| token.byte).collect();
        assert_eq!(bytes.len(), 3, "U+3042 是三個 UTF-8 位元組");
        assert!(bytes.iter().all(|token| token.start == 1 && token.end == 2));
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
        // 「あ」不在詞表，U+3042 = E3 81 82；前面仍會有正規化的 ▁。
        assert_eq!(
            t.encode("あ"),
            vec![
                id_of(&t, "▁"),
                id_of(&t, "<0xE3>"),
                id_of(&t, "<0x81>"),
                id_of(&t, "<0x82>"),
            ]
        );
    }

    #[test]
    fn decode_roundtrips_pieces_and_byte_fallback() {
        let t = tokenizer();
        assert_eq!(t.decode(&t.encode("東京都")), " 東京都");
        assert_eq!(t.decode(&t.encode("あ")), " あ");
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
