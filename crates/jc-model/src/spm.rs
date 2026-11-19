//! SentencePiece `ModelProto`（protobuf）解析與 unigram Viterbi 編碼。
//!
//! 這裡只實作推論需要的最小 schema：`pieces`（piece、score、type）、
//! `trainer_spec`（`byte_fallback` 與特殊 token id）與 `normalizer_spec`
//! （正規化規則名與三個開關）。用 `prost` 直接解 protobuf，不引入
//! SentencePiece 的 C++ 相依，也不需要 `protoc`。
//!
//! 編碼流程與 SentencePiece 的 unigram 模型一致：
//!
//! 1. 正規化：依 `normalizer_spec` 的規則名做 NFKC（`nmt_nfkc` 系）或
//!    原樣保留，再處理空白、加 dummy prefix、把空白轉成 U+2581。
//! 2. 用 piece 的 log-probability 建 lattice，跑 Viterbi 取整句總分最高的切法。
//! 3. 沒有單字 piece 可用的位置放入 unknown 節點；`byte_fallback` 開啟時
//!    在輸出階段把該節點的 UTF-8 位元組展開成 `<0xNN>` byte piece。
//!
//! 已知限制：`precompiled_charsmap`（SentencePiece 內建的雙陣列 trie）沒有
//! 實作，正規化只依規則名近似。`identity` 與 `nmt_nfkc` 這兩種常見設定都正確；
//! 自訂 TSV 規則會被當成 identity。

use std::collections::HashMap;

use prost::Message;
use unicode_normalization::UnicodeNormalization;

use crate::error::{ModelError, Result};
use crate::tokenizer::SPACE;

/// SentencePiece 的 piece 型別。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum PieceType {
    /// 一般 piece，可出現在切法裡。
    Normal = 1,
    /// 未知字，模型只有一個。
    Unknown = 2,
    /// 控制符（`<s>`、`</s>`、`<pad>`），不進 lattice。
    Control = 3,
    /// 使用者自訂 piece，訓練後強制保留。
    UserDefined = 4,
    /// 未使用。
    Unused = 5,
    /// byte fallback 用的 `<0xNN>`。
    Byte = 6,
}

/// `ModelProto` 的最小 schema。
#[derive(Clone, PartialEq, prost::Message)]
pub struct ModelProto {
    /// piece 表，索引就是 token id。
    #[prost(message, repeated, tag = "1")]
    pub pieces: Vec<Piece>,
    /// 訓練參數。
    #[prost(message, tag = "2")]
    pub trainer_spec: Option<TrainerSpec>,
    /// 正規化參數。
    #[prost(message, tag = "3")]
    pub normalizer_spec: Option<NormalizerSpec>,
}

/// 單一 piece。
#[derive(Clone, PartialEq, prost::Message)]
pub struct Piece {
    /// piece 文字；空白以 U+2581 表示。
    #[prost(string, tag = "1")]
    pub piece: String,
    /// unigram log-probability。
    #[prost(float, tag = "2")]
    pub score: f32,
    /// 型別，對應 [`PieceType`]。
    #[prost(enumeration = "PieceType", tag = "3")]
    pub r#type: i32,
}

/// `TrainerSpec` 的相關欄位。
#[derive(Clone, PartialEq, prost::Message)]
pub struct TrainerSpec {
    /// 模型型別，1 是 unigram。
    #[prost(int32, optional, tag = "3")]
    pub model_type: Option<i32>,
    /// 是否把未知字拆成 byte piece。
    #[prost(bool, optional, tag = "35")]
    pub byte_fallback: Option<bool>,
    /// `<unk>` id。
    #[prost(int32, optional, tag = "40")]
    pub unk_id: Option<i32>,
    /// `<s>` id。
    #[prost(int32, optional, tag = "41")]
    pub bos_id: Option<i32>,
    /// `</s>` id。
    #[prost(int32, optional, tag = "42")]
    pub eos_id: Option<i32>,
    /// `<pad>` id；`-1` 代表沒有。
    #[prost(int32, optional, tag = "43")]
    pub pad_id: Option<i32>,
}

/// `NormalizerSpec` 的相關欄位。
#[derive(Clone, PartialEq, prost::Message)]
pub struct NormalizerSpec {
    /// 規則名稱，例如 `identity` 或 `nmt_nfkc`。
    #[prost(string, tag = "1")]
    pub name: String,
    /// 預先編譯的字元對照表；本模組沒有實作，只用規則名近似。
    #[prost(bytes, tag = "2")]
    pub precompiled_charsmap: Vec<u8>,
    /// 是否在開頭加一個空白。
    #[prost(bool, optional, tag = "3")]
    pub add_dummy_prefix: Option<bool>,
    /// 是否移除多餘空白。
    #[prost(bool, optional, tag = "4")]
    pub remove_extra_whitespaces: Option<bool>,
    /// 是否把空白轉成 U+2581。
    #[prost(bool, optional, tag = "5")]
    pub escape_whitespaces: Option<bool>,
}

/// 正規化參數（已套用預設值）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Normalizer {
    /// 規則名稱。
    pub rule: String,
    /// 是否做 NFKC。
    pub nfkc: bool,
    /// 是否加 dummy prefix。
    pub add_dummy_prefix: bool,
    /// 是否合併多餘空白。
    pub remove_extra_whitespaces: bool,
    /// 是否把空白轉成 U+2581。
    pub escape_whitespaces: bool,
}

impl Normalizer {
    /// 從 `normalizer_spec` 建立；缺席時用 SentencePiece 的預設值。
    pub fn from_spec(spec: Option<&NormalizerSpec>) -> Self {
        let rule = spec
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "identity".into());
        Self {
            nfkc: uses_nfkc(&rule),
            rule,
            add_dummy_prefix: spec.and_then(|s| s.add_dummy_prefix).unwrap_or(true),
            remove_extra_whitespaces: spec
                .and_then(|s| s.remove_extra_whitespaces)
                .unwrap_or(true),
            escape_whitespaces: spec.and_then(|s| s.escape_whitespaces).unwrap_or(true),
        }
    }

    /// 正規化文字：NFKC（依規則）→ 空白處理 → dummy prefix → 空白轉義。
    pub fn normalize(&self, text: &str) -> String {
        if text.is_empty() {
            return String::new();
        }
        let mapped = if self.nfkc {
            text.nfkc().collect::<String>()
        } else {
            text.to_string()
        };
        let mut out = String::with_capacity(mapped.len() + 4);
        let mut prev_space = false;
        for ch in mapped.chars() {
            if ch == ' ' && self.remove_extra_whitespaces {
                if prev_space {
                    continue;
                }
                prev_space = true;
            } else if ch != ' ' {
                prev_space = false;
            }
            out.push(ch);
        }
        if self.remove_extra_whitespaces {
            while out.ends_with(' ') {
                out.pop();
            }
        }
        if out.is_empty() {
            return out;
        }
        if self.add_dummy_prefix {
            out.insert(0, ' ');
        }
        if self.escape_whitespaces {
            out = out.replace(' ', &SPACE.to_string());
        }
        out
    }
}

/// 規則名是否要走 NFKC。
fn uses_nfkc(rule: &str) -> bool {
    let lower = rule.to_ascii_lowercase();
    lower.contains("nfkc") || lower.contains("nfkd")
}

/// piece 前綴樹；用一個攤平的 `(節點, 字元)` 對照表，避免每個節點各自配置。
#[derive(Debug, Clone, Default)]
struct Trie {
    edges: HashMap<(u32, char), u32>,
    /// 節點上的 piece：`(id, score)`。
    values: Vec<Option<(u32, f32)>>,
}

impl Trie {
    fn new() -> Self {
        Self {
            edges: HashMap::new(),
            values: vec![None],
        }
    }

    fn insert(&mut self, piece: &str, id: u32, score: f32) {
        let mut node = 0u32;
        for ch in piece.chars() {
            let next = match self.edges.get(&(node, ch)) {
                Some(&next) => next,
                None => {
                    let next = self.values.len() as u32;
                    self.values.push(None);
                    self.edges.insert((node, ch), next);
                    next
                }
            };
            node = next;
        }
        if let Some(slot) = self.values.get_mut(node as usize) {
            // 同一個 piece 重複出現時保留先出現的 id，與 SentencePiece 一致。
            slot.get_or_insert((id, score));
        }
    }

    fn next(&self, node: u32, ch: char) -> Option<u32> {
        self.edges.get(&(node, ch)).copied()
    }

    fn value(&self, node: u32) -> Option<(u32, f32)> {
        self.values.get(node as usize).copied().flatten()
    }
}

/// lattice 上的一個節點。
#[derive(Debug, Clone, Copy)]
struct Node {
    /// 起始字元索引（含）。
    start: usize,
    /// 結束字元索引（不含）。
    end: usize,
    /// token id。
    id: u32,
    /// 該 piece 的分數；unknown 節點是 `min_score - 10`。
    score: f32,
}

/// unknown 節點相對最低分的懲罰，與 SentencePiece `kUnkPenalty` 相同。
const UNK_PENALTY: f32 = 10.0;

/// 載入後的 SentencePiece 模型。
#[derive(Debug, Clone)]
pub struct Model {
    pieces: Vec<String>,
    scores: Vec<f32>,
    types: Vec<PieceType>,
    trie: Trie,
    byte_ids: [Option<u32>; 256],
    unk_id: u32,
    bos_id: u32,
    eos_id: u32,
    pad_id: u32,
    byte_fallback: bool,
    min_score: f32,
    normalizer: Normalizer,
}

impl Model {
    /// 解析 `.model` 檔的內容。
    pub fn parse(raw: &[u8]) -> Result<Self> {
        let proto = ModelProto::decode(raw)
            .map_err(|e| ModelError::tokenizer(format!("SentencePiece protobuf 解析失敗：{e}")))?;
        if proto.pieces.is_empty() {
            return Err(ModelError::tokenizer("SentencePiece 模型的 piece 表是空的"));
        }
        let trainer = proto.trainer_spec.clone().unwrap_or_default();
        // 只支援 unigram；BPE、WORD、CHAR 的分數語意不同，硬編會錯。
        if let Some(kind) = trainer.model_type {
            if kind != 1 {
                return Err(ModelError::tokenizer(format!(
                    "只支援 unigram 模型，這個模型的 model_type 是 {kind}"
                )));
            }
        }
        let normalizer = Normalizer::from_spec(proto.normalizer_spec.as_ref());
        if let Some(spec) = proto.normalizer_spec.as_ref() {
            if !spec.precompiled_charsmap.is_empty() && !normalizer.nfkc {
                return Err(ModelError::tokenizer(format!(
                    "不支援 precompiled charsmap（正規化規則 {}）",
                    normalizer.rule
                )));
            }
        }
        let n = proto.pieces.len();
        let mut pieces = Vec::with_capacity(n);
        let mut scores = Vec::with_capacity(n);
        let mut types = Vec::with_capacity(n);
        let mut trie = Trie::new();
        let mut byte_ids = [None; 256];
        let mut min_score = f32::INFINITY;
        for (idx, piece) in proto.pieces.iter().enumerate() {
            let id = idx as u32;
            let kind = PieceType::try_from(piece.r#type).unwrap_or(PieceType::Normal);
            pieces.push(piece.piece.clone());
            scores.push(piece.score);
            types.push(kind);
            match kind {
                PieceType::Normal | PieceType::UserDefined => {
                    if piece.piece.is_empty() {
                        continue;
                    }
                    // 與前身 `internal/spm` 相同：使用者自訂 piece 用長度給分，
                    // 讓它優先吃整段；一般 piece 用模型的 log-probability。
                    let score = if kind == PieceType::UserDefined {
                        0.1 * (piece.piece.chars().count().saturating_sub(1)) as f32
                    } else {
                        piece.score
                    };
                    trie.insert(&piece.piece, id, score);
                    if kind == PieceType::Normal {
                        min_score = min_score.min(piece.score);
                    }
                }
                PieceType::Byte => {
                    if let Some(b) = parse_byte_piece(&piece.piece) {
                        byte_ids[b as usize].get_or_insert(id);
                    }
                }
                PieceType::Control | PieceType::Unknown | PieceType::Unused => {}
            }
        }
        if !min_score.is_finite() {
            min_score = 0.0;
        }
        let special = |value: Option<i32>, default: u32| -> u32 {
            match value {
                Some(v) if v >= 0 && (v as usize) < n => v as u32,
                _ => default,
            }
        };
        Ok(Self {
            unk_id: special(trainer.unk_id, 0),
            bos_id: special(trainer.bos_id, 1),
            eos_id: special(trainer.eos_id, 2),
            // `pad_id = -1` 代表沒有 pad；本引擎的張量布局一定要有，退回 0。
            pad_id: special(trainer.pad_id, 0),
            byte_fallback: trainer.byte_fallback.unwrap_or(false),
            min_score,
            normalizer,
            trie,
            byte_ids,
            pieces,
            scores,
            types,
        })
    }

    /// 詞表大小。
    pub fn len(&self) -> usize {
        self.pieces.len()
    }

    /// 詞表是否為空（合法模型不會是空的）。
    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    /// id 對應的 piece 文字。
    pub fn piece(&self, id: u32) -> Option<&str> {
        self.pieces.get(id as usize).map(String::as_str)
    }

    /// id 對應的分數。
    pub fn score(&self, id: u32) -> Option<f32> {
        self.scores.get(id as usize).copied()
    }

    /// id 對應的型別。
    pub fn piece_type(&self, id: u32) -> Option<PieceType> {
        self.types.get(id as usize).copied()
    }

    /// 正規化參數。
    pub fn normalizer(&self) -> &Normalizer {
        &self.normalizer
    }

    /// 是否開啟 byte fallback。
    pub fn byte_fallback(&self) -> bool {
        self.byte_fallback
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

    /// 最低 piece 分數；unknown 節點用它扣 [`UNK_PENALTY`]。
    pub fn min_score(&self) -> f32 {
        self.min_score
    }

    /// 正規化文字。
    pub fn normalize(&self, text: &str) -> String {
        self.normalizer.normalize(text)
    }

    /// 先把文字正規化再編碼。
    pub fn encode(&self, text: &str) -> Vec<u32> {
        self.encode_normalized(&self.normalize(text))
    }

    /// 直接編碼已正規化的文字（空白已轉成 U+2581、已加 dummy prefix）。
    pub fn encode_normalized(&self, text: &str) -> Vec<u32> {
        let chars: Vec<char> = text.chars().collect();
        let n = chars.len();
        if n == 0 {
            return Vec::new();
        }

        // 1. 建 lattice：每個起始位置最多一個 unknown 節點。
        let mut nodes: Vec<Node> = Vec::with_capacity(n * 2);
        let mut ends: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
        for start in 0..n {
            let mut node = 0u32;
            let mut has_single = false;
            for (offset, ch) in chars[start..].iter().enumerate() {
                let Some(next) = self.trie.next(node, *ch) else {
                    break;
                };
                node = next;
                let Some((id, score)) = self.trie.value(node) else {
                    continue;
                };
                let end = start + offset + 1;
                ends[end].push(nodes.len());
                nodes.push(Node {
                    start,
                    end,
                    id,
                    score,
                });
                if offset == 0 {
                    has_single = true;
                }
            }
            if !has_single {
                ends[start + 1].push(nodes.len());
                nodes.push(Node {
                    start,
                    end: start + 1,
                    id: self.unk_id,
                    score: self.min_score - UNK_PENALTY,
                });
            }
        }

        // 2. Viterbi：`best[i]` 是前 i 個字元的最高總分。
        let mut best = vec![f32::NEG_INFINITY; n + 1];
        let mut from: Vec<Option<usize>> = vec![None; n + 1];
        best[0] = 0.0;
        for end in 1..=n {
            for &idx in &ends[end] {
                let node = nodes[idx];
                let prev = best[node.start];
                if !prev.is_finite() {
                    continue;
                }
                let total = prev + node.score;
                if total > best[end] {
                    best[end] = total;
                    from[end] = Some(idx);
                }
            }
        }

        // 3. 回溯並展開 unknown 節點。
        let mut path: Vec<usize> = Vec::new();
        let mut pos = n;
        while let Some(idx) = from[pos] {
            path.push(idx);
            pos = nodes[idx].start;
        }
        path.reverse();
        let mut out = Vec::with_capacity(path.len());
        for idx in path {
            let node = nodes[idx];
            if node.id == self.unk_id && self.byte_fallback {
                for ch in &chars[node.start..node.end] {
                    let mut buf = [0u8; 4];
                    for &b in ch.encode_utf8(&mut buf).as_bytes() {
                        out.push(self.byte_ids[b as usize].unwrap_or(self.unk_id));
                    }
                }
            } else {
                out.push(node.id);
            }
        }
        out
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

    fn piece(text: &str, score: f32, kind: PieceType) -> Piece {
        Piece {
            piece: text.to_string(),
            score,
            r#type: kind as i32,
        }
    }

    fn proto(pieces: Vec<Piece>, byte_fallback: bool) -> ModelProto {
        ModelProto {
            pieces,
            trainer_spec: Some(TrainerSpec {
                model_type: Some(1),
                byte_fallback: Some(byte_fallback),
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
        }
    }

    fn build(pieces: Vec<Piece>, byte_fallback: bool) -> Model {
        let mut buf = Vec::new();
        proto(pieces, byte_fallback).encode(&mut buf).unwrap();
        Model::parse(&buf).unwrap()
    }

    /// 小模型：`▁東京都` 的分數刻意很差，Viterbi 應該拆成兩個 piece。
    fn small_pieces() -> Vec<Piece> {
        vec![
            piece("<pad>", 0.0, PieceType::Control),
            piece("<unk>", 0.0, PieceType::Unknown),
            piece("<s>", 0.0, PieceType::Control),
            piece("</s>", 0.0, PieceType::Control),
            piece("▁", -3.0, PieceType::Normal),
            piece("▁東", -4.0, PieceType::Normal),
            piece("▁東京", -2.0, PieceType::Normal),
            piece("▁東京都", -9.0, PieceType::Normal),
            piece("東京", -1.0, PieceType::Normal),
            piece("東", -2.5, PieceType::Normal),
            piece("京", -2.5, PieceType::Normal),
            piece("都", -2.0, PieceType::Normal),
        ]
    }

    fn small_model() -> Model {
        build(small_pieces(), true)
    }

    fn id(model: &Model, text: &str) -> u32 {
        model
            .pieces
            .iter()
            .position(|p| p == text)
            .expect("測試 piece 必須存在") as u32
    }

    #[test]
    fn proto_roundtrips_scores_and_types() {
        let model = small_model();
        assert_eq!(model.len(), 12);
        assert_eq!(model.piece(6), Some("▁東京"));
        assert_eq!(model.score(6), Some(-2.0));
        assert_eq!(model.piece_type(0), Some(PieceType::Control));
        assert_eq!(model.piece_type(1), Some(PieceType::Unknown));
        assert_eq!(model.unk_id(), 1);
        assert_eq!(model.bos_id(), 2);
        assert_eq!(model.pad_id(), 0);
        assert!(model.byte_fallback());
        assert_eq!(model.normalizer().rule, "identity");
        assert_eq!(model.min_score(), -9.0);
    }

    #[test]
    fn viterbi_picks_the_best_total_score_not_the_longest_piece() {
        let model = small_model();
        // 「▁東京」−2.0 比「▁東」+「京」−6.5 好，也比「▁」+「東京」−4.0 好。
        assert_eq!(model.encode("東京"), vec![id(&model, "▁東京")]);
        // 最長前綴會挑「▁東京都」（−9.0）；Viterbi 挑「▁東京」+「都」（−4.0）。
        assert_eq!(
            model.encode("東京都"),
            vec![id(&model, "▁東京"), id(&model, "都")]
        );
    }

    #[test]
    fn unknown_runes_expand_to_byte_pieces() {
        let mut pieces = small_pieces();
        for b in 0..=255u16 {
            pieces.push(piece(&format!("<0x{b:02X}>"), 0.0, PieceType::Byte));
        }
        let model = build(pieces, true);
        // 「あ」= E3 81 82，前面還有正規化的 dummy prefix。
        assert_eq!(
            model.encode("あ"),
            vec![
                id(&model, "▁"),
                id(&model, "<0xE3>"),
                id(&model, "<0x81>"),
                id(&model, "<0x82>"),
            ]
        );
    }

    #[test]
    fn unknown_runes_keep_unk_id_without_byte_pieces() {
        // 有開 byte_fallback 但詞表沒有 byte piece：每個位元組退回 `<unk>`。
        let model = small_model();
        assert_eq!(model.encode("あ"), vec![model.unk_id(); 3]);
        // 沒開 byte_fallback 時整個字就是一個 unknown 節點。
        let model = build(small_pieces(), false);
        assert_eq!(model.encode("あ"), vec![model.unk_id()]);
    }

    #[test]
    fn normalize_matches_sentencepiece_identity_rule() {
        let model = small_model();
        assert_eq!(model.normalize(""), "");
        assert_eq!(model.normalize("   "), "");
        assert_eq!(model.normalize("東京"), "▁東京");
        assert_eq!(model.normalize("  東京  都  "), "▁東京▁都");
        assert_eq!(model.normalize("東京\t都"), "▁東京\t都");
    }

    #[test]
    fn nfkc_rule_folds_fullwidth_forms() {
        let normalizer = Normalizer::from_spec(Some(&NormalizerSpec {
            name: "nmt_nfkc".into(),
            precompiled_charsmap: Vec::new(),
            add_dummy_prefix: Some(true),
            remove_extra_whitespaces: Some(true),
            escape_whitespaces: Some(true),
        }));
        assert!(normalizer.nfkc);
        assert_eq!(normalizer.normalize("ＡＢＣ"), "▁ABC");
        assert_eq!(normalizer.normalize("ｶﾀｶﾅ"), "▁カタカナ");
    }

    #[test]
    fn byte_piece_names_are_parsed() {
        assert_eq!(parse_byte_piece("<0x0A>"), Some(0x0A));
        assert_eq!(parse_byte_piece("<0xff>"), Some(0xFF));
        assert_eq!(parse_byte_piece("<0xGG>"), None);
        assert_eq!(parse_byte_piece("<pad>"), None);
    }

    #[test]
    fn empty_input_encodes_to_nothing() {
        let model = small_model();
        assert!(model.encode("").is_empty());
        assert!(model.encode("   ").is_empty());
    }

    #[test]
    fn non_unigram_models_are_rejected() {
        let mut spec = proto(small_pieces(), true);
        spec.trainer_spec.as_mut().unwrap().model_type = Some(2);
        let mut buf = Vec::new();
        spec.encode(&mut buf).unwrap();
        match Model::parse(&buf) {
            Err(ModelError::Tokenizer(msg)) => assert!(msg.contains("unigram"), "{msg}"),
            other => panic!("未預期結果：{other:?}"),
        }
    }

    #[test]
    fn custom_charsmap_is_rejected_instead_of_guessed() {
        let mut spec = proto(small_pieces(), true);
        if let Some(normalizer) = spec.normalizer_spec.as_mut() {
            normalizer.name = "custom_tsv".into();
            normalizer.precompiled_charsmap = vec![0x00, 0x01, 0x02];
        }
        let mut buf = Vec::new();
        spec.encode(&mut buf).unwrap();
        match Model::parse(&buf) {
            Err(ModelError::Tokenizer(msg)) => assert!(msg.contains("charsmap"), "{msg}"),
            other => panic!("未預期結果：{other:?}"),
        }
    }
}
