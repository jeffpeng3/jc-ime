//! 0c 日文 SentencePiece 詞表與前處理。
//!
//! 有兩種來源：
//!
//! - **SentencePiece 模型**（`*.model`，`ModelProto`）：含 piece 的
//!   log-probability，走真正的 unigram Viterbi，見 [`crate::spm`]。這是預設路徑。
//! - **只有詞表的 `vocab.json`**（`0c.sentencepiece.vocab/1`）：沒有 piece
//!   分數，無法還原訓練時的切法，只能改用最長前綴匹配（maximal munch）近似。
//!   這個格式留著給舊模型包與對照用。
//!
//! 兩個來源共用同一套正規化語意（identity 規則、合併空白、dummy prefix、
//! 空白轉 U+2581）；`vocab.json` 沒有 `normalizer_spec`，用固定預設值。

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::error::{ModelError, Result};
use crate::spm;

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

/// 支援的 `vocab.json` 格式。
pub const VOCAB_SCHEMA: &str = "0c.sentencepiece.vocab/1";

/// 空白轉義字元（U+2581）。
pub const SPACE: char = '\u{2581}';

/// 編碼策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// unigram Viterbi，使用 piece 分數；需要 SentencePiece 模型。
    Unigram,
    /// 最長前綴匹配；只有詞表時的近似解。
    MaxMatch,
}

/// 前處理後的 tokenizer。
#[derive(Debug, Clone)]
pub struct Tokenizer {
    pieces: Vec<String>,
    /// 只含可匹配 piece 的索引，給最長前綴匹配用。
    ids: HashMap<String, u32>,
    byte_ids: [Option<u32>; 256],
    max_piece_bytes: usize,
    unk_id: u32,
    bos_id: u32,
    eos_id: u32,
    pad_id: u32,
    model_name: String,
    strategy: Strategy,
    spm: Option<spm::Model>,
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
    /// 載入 tokenizer 檔。
    ///
    /// 內容以 `{` 開頭視為 `vocab.json`，其餘視為 SentencePiece 的
    /// `ModelProto`；不看副檔名，方便 manifest 任意命名。
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = fs::read(path)?;
        Self::from_bytes(&raw)
            .map_err(|e| ModelError::tokenizer(format!("{}：{e}", path.display())))
    }

    /// 從檔案內容載入，自動判斷格式。
    pub fn from_bytes(raw: &[u8]) -> Result<Self> {
        let head = raw.iter().find(|b| !b.is_ascii_whitespace());
        match head {
            Some(b'{') => Self::from_json(raw),
            Some(_) => Self::from_spm_bytes(raw),
            None => Err(ModelError::tokenizer("tokenizer 檔是空的")),
        }
    }

    /// 從 SentencePiece `ModelProto` 載入，使用 unigram Viterbi。
    pub fn from_spm_bytes(raw: &[u8]) -> Result<Self> {
        let model = spm::Model::parse(raw)?;
        let n = model.len();
        let mut ids = HashMap::with_capacity(n);
        let mut byte_ids = [None; 256];
        for id in 0..n as u32 {
            let Some(piece) = model.piece(id) else {
                continue;
            };
            if piece.is_empty() {
                continue;
            }
            match model.piece_type(id) {
                Some(spm::PieceType::Normal) | Some(spm::PieceType::UserDefined) => {
                    // 重複的 piece 保留先出現的 id，與 SentencePiece 一致。
                    ids.entry(piece.to_string()).or_insert(id);
                }
                Some(spm::PieceType::Byte) => {
                    if let Some(b) = parse_byte_piece(piece) {
                        byte_ids[b as usize].get_or_insert(id);
                    }
                }
                _ => {}
            }
        }
        let max_piece_bytes = ids.keys().map(|p| p.len()).max().unwrap_or(0).min(128);
        Ok(Self {
            pieces: (0..n as u32)
                .map(|id| model.piece(id).unwrap_or_default().to_string())
                .collect(),
            ids,
            byte_ids,
            max_piece_bytes,
            unk_id: model.unk_id(),
            bos_id: model.bos_id(),
            eos_id: model.eos_id(),
            pad_id: model.pad_id(),
            model_name: "sentencepiece".to_string(),
            strategy: Strategy::Unigram,
            spm: Some(model),
        })
    }

    /// 從 `vocab.json` 載入，使用最長前綴匹配。
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

        let special_ids = [unk_id, bos_id, eos_id, pad_id];
        let mut ids = HashMap::with_capacity(n);
        let mut byte_ids = [None; 256];
        let mut max_piece_bytes = 0;
        for (idx, piece) in v.id_to_piece.iter().enumerate() {
            let id = idx as u32;
            if let Some(b) = parse_byte_piece(piece) {
                byte_ids[b as usize].get_or_insert(id);
                continue;
            }
            if piece.is_empty() || special_ids.contains(&id) {
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
            strategy: Strategy::MaxMatch,
            spm: None,
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

    /// 模型名稱：`vocab.json` 的 `model` 欄位，或 `sentencepiece`。
    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    /// 目前使用的編碼策略。
    pub fn strategy(&self) -> Strategy {
        self.strategy
    }

    /// 正規化規則名稱；只有 SentencePiece 模型有。
    pub fn normalizer_rule(&self) -> Option<&str> {
        self.spm.as_ref().map(|m| m.normalizer().rule.as_str())
    }

    /// 是否支援 byte fallback。
    pub fn byte_fallback(&self) -> bool {
        match &self.spm {
            Some(model) => model.byte_fallback(),
            None => self.byte_ids.iter().any(Option::is_some),
        }
    }

    /// 同一份詞表改用最長前綴匹配。
    ///
    /// 詞表不變，只換切法；用來量測「沒有 piece 分數」與「有 piece 分數」
    /// 的差別，見 `docs/port/rerank-tokenizer.md`。
    pub fn as_max_match(&self) -> Self {
        Self {
            strategy: Strategy::MaxMatch,
            ..self.clone()
        }
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
    ///
    /// 與 SentencePiece 的 `PieceToId` 一樣看得到控制符與 byte piece（線性
    /// 掃描詞表，只給測試與工具用；編碼走另一份只看一般 piece 的索引）。
    pub fn piece_id(&self, piece: &str) -> Option<u32> {
        self.pieces
            .iter()
            .position(|p| p == piece)
            .map(|id| id as u32)
    }

    /// 查 id 的 piece 文字。
    pub fn piece(&self, id: u32) -> Option<&str> {
        self.pieces.get(id as usize).map(String::as_str)
    }

    /// 查 id 的 unigram 分數；最長前綴匹配或 `vocab.json` 沒有分數時回傳 `None`。
    pub fn score(&self, id: u32) -> Option<f32> {
        if self.strategy != Strategy::Unigram {
            return None;
        }
        self.spm.as_ref().and_then(|m| m.score(id))
    }

    /// 前處理：trim、合併空白、加 dummy prefix、把空白轉成 U+2581。
    pub fn normalize(&self, text: &str) -> String {
        if let Some(model) = &self.spm {
            return model.normalize(text);
        }
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
        if self.strategy == Strategy::Unigram {
            if let Some(model) = &self.spm {
                return model.encode(text);
            }
        }
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

/// 解析 `<0xNN>` 形式的 byte piece。
fn parse_byte_piece(piece: &str) -> Option<u8> {
    let hex = piece.strip_prefix("<0x")?.strip_suffix('>')?;
    if hex.len() != 2 {
        return None;
    }
    u8::from_str_radix(hex, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spm::{ModelProto, NormalizerSpec, Piece, PieceType, TrainerSpec};
    use prost::Message;

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

    /// 小 SentencePiece 模型：`▁東京都` 的分數很差，Viterbi 會拆開。
    fn spm_bytes() -> Vec<u8> {
        let mut pieces = vec![
            piece("<pad>", 0.0, PieceType::Control),
            piece("<unk>", 0.0, PieceType::Unknown),
            piece("<s>", 0.0, PieceType::Control),
            piece("</s>", 0.0, PieceType::Control),
        ];
        for b in 0..=255u16 {
            pieces.push(piece(&format!("<0x{b:02X}>"), 0.0, PieceType::Byte));
        }
        for (p, score) in [
            ("▁", -3.0),
            ("▁東", -4.0),
            ("▁東京", -2.0),
            ("▁東京都", -9.0),
            ("東京", -1.0),
            ("東", -2.5),
            ("京", -2.5),
            ("都", -2.0),
        ] {
            pieces.push(piece(p, score, PieceType::Normal));
        }
        let proto = ModelProto {
            pieces,
            trainer_spec: Some(TrainerSpec {
                model_type: Some(1),
                byte_fallback: Some(true),
                unk_id: Some(1),
                bos_id: Some(2),
                eos_id: Some(3),
                pad_id: Some(0),
            }),
            normalizer_spec: Some(NormalizerSpec {
                name: "identity".into(),
                precompiled_charsmap: Vec::new(),
                add_dummy_prefix: Some(true),
                remove_extra_whitespaces: Some(true),
                escape_whitespaces: Some(true),
            }),
        };
        let mut buf = Vec::new();
        proto.encode(&mut buf).unwrap();
        buf
    }

    fn piece(text: &str, score: f32, kind: PieceType) -> Piece {
        Piece {
            piece: text.to_string(),
            score,
            r#type: kind as i32,
        }
    }

    fn spm_tokenizer() -> Tokenizer {
        Tokenizer::from_spm_bytes(&spm_bytes()).unwrap()
    }

    /// 前身 `internal/spm/spm_test.go` 的 `testModel`：同一份 id 配置與分數。
    fn go_model(byte_fallback: bool) -> Vec<u8> {
        let mut pieces = vec![
            piece("<unk>", 0.0, PieceType::Unknown),
            piece("<s>", 0.0, PieceType::Control),
            piece("</s>", 0.0, PieceType::Control),
            piece("<pad>", 0.0, PieceType::Control),
            piece("▁", -1.0, PieceType::Normal),
            piece("▁a", -2.0, PieceType::Normal),
            piece("a", -3.0, PieceType::Normal),
            piece("b", -4.0, PieceType::Normal),
            piece("▁ab", -5.0, PieceType::Normal),
            piece("ab", -6.0, PieceType::Normal),
        ];
        if byte_fallback {
            for name in ["<0xE3>", "<0x81>", "<0x82>", "<0x80>"] {
                pieces.push(piece(name, 0.0, PieceType::Byte));
            }
        }
        let proto = ModelProto {
            pieces,
            trainer_spec: Some(TrainerSpec {
                model_type: Some(1),
                byte_fallback: Some(byte_fallback),
                unk_id: Some(0),
                bos_id: Some(1),
                eos_id: Some(2),
                pad_id: Some(3),
            }),
            normalizer_spec: Some(NormalizerSpec {
                name: "identity".into(),
                precompiled_charsmap: Vec::new(),
                add_dummy_prefix: Some(true),
                remove_extra_whitespaces: Some(true),
                escape_whitespaces: Some(true),
            }),
        };
        let mut buf = Vec::new();
        proto.encode(&mut buf).unwrap();
        buf
    }

    fn go_tokenizer(byte_fallback: bool) -> Tokenizer {
        Tokenizer::from_spm_bytes(&go_model(byte_fallback)).unwrap()
    }

    /// 前身 `TestNormalize`。
    #[test]
    fn test_normalize() {
        let t = go_tokenizer(false);
        for (input, want) in [
            ("", ""),
            ("   ", ""),
            ("a", "▁a"),
            ("  a  b ", "▁a▁b"),
            ("a b", "▁a▁b"),
            ("a  b  c", "▁a▁b▁c"),
            ("\t a", "▁\t▁a"),
        ] {
            assert_eq!(t.normalize(input), want, "normalize({input:?})");
        }
    }

    /// 前身 `TestEncodeViterbi`。
    #[test]
    fn test_encode_viterbi() {
        let t = go_tokenizer(false);
        assert!(t.encode("").is_empty());
        // ▁a 勝過 ▁+a；▁ab 勝過 ▁a+b。
        assert_eq!(t.encode("a"), vec![5]);
        assert_eq!(t.encode("ab"), vec![8]);
        // 這個模型沒有 ▁b，所以「a b」是 ▁a ▁ b。
        assert_eq!(t.encode("a b"), vec![5, 4, 7]);
    }

    /// 前身 `TestEncodeUnknown`。
    #[test]
    fn test_encode_unknown() {
        // 沒有 byte fallback：未知字是一個 unknown piece。
        assert_eq!(go_tokenizer(false).encode("あ"), vec![4, 0]);
        // 有 byte fallback：展開成 UTF-8 byte piece。
        let t = go_tokenizer(true);
        let want = vec![
            4,
            t.piece_id("<0xE3>").unwrap(),
            t.piece_id("<0x81>").unwrap(),
            t.piece_id("<0x82>").unwrap(),
        ];
        assert_eq!(t.encode("あ"), want);
    }

    #[test]
    fn normalize_adds_dummy_prefix_and_collapses_spaces() {
        for t in [tokenizer(), spm_tokenizer()] {
            assert_eq!(t.normalize("東京都"), "▁東京都");
            assert_eq!(t.normalize("  東京  都  "), "▁東京▁都");
            assert_eq!(t.normalize(""), "");
            assert_eq!(t.normalize("   "), "");
        }
    }

    #[test]
    fn encode_prefers_longest_piece_without_scores() {
        let t = tokenizer();
        assert_eq!(t.strategy(), Strategy::MaxMatch);
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
        for t in [tokenizer(), spm_tokenizer()] {
            assert_eq!(t.decode(&t.encode("東京都")), " 東京都");
            assert_eq!(t.decode(&t.encode("あ")), " あ");
            assert_eq!(t.decode(&[t.pad_id()]), "");
        }
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
    fn from_bytes_detects_both_formats() {
        assert_eq!(
            Tokenizer::from_bytes(&vocab()).unwrap().strategy(),
            Strategy::MaxMatch
        );
        let t = Tokenizer::from_bytes(&spm_bytes()).unwrap();
        assert_eq!(t.strategy(), Strategy::Unigram);
        assert_eq!(t.model_name(), "sentencepiece");
        assert_eq!(t.normalizer_rule(), Some("identity"));
        assert!(t.byte_fallback());
        assert!(matches!(
            Tokenizer::from_bytes(b""),
            Err(ModelError::Tokenizer(_))
        ));
    }

    #[test]
    fn unigram_beats_max_match_when_the_long_piece_scores_badly() {
        let t = spm_tokenizer();
        assert_eq!(
            t.encode("東京都"),
            vec![t.piece_id("▁東京").unwrap(), t.piece_id("都").unwrap()]
        );
        // 同一份詞表改成最長前綴匹配，會挑最長的「▁東京都」。
        let plain = t.as_max_match();
        assert_eq!(plain.strategy(), Strategy::MaxMatch);
        assert_eq!(plain.encode("東京都"), vec![t.piece_id("▁東京都").unwrap()]);
        assert_eq!(plain.score(plain.encode("東京都")[0]), None);
    }

    #[test]
    fn real_model_metadata_matches_the_released_file() {
        let path = jc_testdata::require_data!("models/ja-tokenizer/public_core_ja.model");
        let t = Tokenizer::load(&path).unwrap();
        assert_eq!(t.len(), 32768);
        assert_eq!(t.strategy(), Strategy::Unigram);
        assert_eq!(t.normalizer_rule(), Some("identity"));
        assert!(t.byte_fallback());
        assert_eq!(t.unk_id(), 1);
        assert_eq!(t.bos_id(), 2);
        assert_eq!(t.eos_id(), 3);
        assert_eq!(t.pad_id(), 0);
        assert_eq!(t.piece(263), Some("▁"));
        assert!(t.score(263).is_some());
    }

    /// 期望值由前身的 Python `sentencepiece` 0.2.3 產生。
    #[test]
    fn real_model_matches_sentencepiece_exactly() {
        let path = jc_testdata::require_data!("models/ja-tokenizer/public_core_ja.model");
        let t = Tokenizer::load(&path).unwrap();
        let cases: [(&str, &[u32]); 8] = [
            ("東京都に行く", &[7693, 11446]),
            (
                "吾輩は猫である。名前はまだ無い。",
                &[263, 8563, 13065, 264, 5268, 291, 261, 6205, 5140, 6580, 261],
            ),
            ("ｶﾀｶﾅ", &[263, 28718, 11642, 28718, 31062]),
            ("😀絵文字", &[263, 244, 163, 156, 132, 20938]),
            ("ABC abc 123", &[263, 22351, 263, 5953, 676, 1445, 332]),
            ("コンピューター", &[263, 3353, 461]),
            ("東京  都", &[10015, 263, 3468]),
            ("<0x41>", &[263, 4470, 1256, 1152, 4137, 1498]),
        ];
        for (text, want) in cases {
            assert_eq!(t.encode(text), want, "{text}");
        }
        assert!(t.encode("").is_empty());
        assert!(t.encode("   ").is_empty());
    }

    #[test]
    fn real_model_decode_roundtrips() {
        let path = jc_testdata::require_data!("models/ja-tokenizer/public_core_ja.model");
        let t = Tokenizer::load(&path).unwrap();
        for text in ["東京都に行く", "ｶﾀｶﾅ", "😀絵文字", "ABC abc 123"] {
            let ids = t.encode(text);
            assert_eq!(t.decode(&ids), t.normalize(text).replace(SPACE, " "));
        }
    }
}
