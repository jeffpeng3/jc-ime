//! 重排模型的固定部署佈局。
//!
//! 與前身 `internal/neural/native/rerank_plan.go` 相同：
//!
//! ```text
//! <bos> [left context（右對齊，最多 60 token）] <eos> candidate <eos> [pad...]
//! ```
//!
//! 沒有左文時左文區放一個 BOS 當標記，不留空。候選字固定在
//! `candidate_slot`，模型永遠讀同一個 pooling 位置。前身的部署沒有右文
//! （IME 沒有 lookahead），所以右文參數只保留在介面上、不進張量。

use crate::manifest::ModelSpec;
use crate::tokenizer::Tokenizer;

/// 一批候選字的張量內容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedBatch {
    /// `input_ids`，列優先，每列 [`PackedBatch::seq`] 個元素。
    pub ids: Vec<i64>,
    /// `attention_mask`，與 [`PackedBatch::ids`] 同形。
    pub mask: Vec<i64>,
    /// 序列長度。
    pub seq: usize,
    /// 候選字數量。
    pub rows: usize,
}

/// 驗證佈局參數。
pub fn validate_layout(spec: &ModelSpec) -> Result<(), String> {
    let seq = spec.seq_len();
    let slot = spec.candidate_slot();
    let context = spec.context_tokens();
    if seq < 4 {
        return Err(format!("seq_len 太小：{seq}"));
    }
    if slot == 0 || slot >= seq - 1 {
        return Err(format!("candidate_slot {slot} 必須落在 1..{}", seq - 1));
    }
    if context == 0 || context > slot - 2 {
        return Err(format!(
            "context_tokens {context} 必須落在 1..={}",
            slot - 2
        ));
    }
    Ok(())
}

/// 上下文的 token id。
///
/// `encode` 會為了詞首自動加一個 `▁`；訓練資料的 context 是連續語料的
/// token，沒有這個假邊界。留著會讓整個 60 格視窗多一格並擠掉最舊的
/// token，實測掉 5 個百分點，所以在這裡只丟掉獨立的那一個 `▁`。
fn context_ids(tokenizer: &Tokenizer, text: &str) -> Vec<u32> {
    let mut ids = tokenizer.encode(text);
    let space = tokenizer.piece_id(crate::tokenizer::SPACE.encode_utf8(&mut [0; 4]));
    if !ids.is_empty() && Some(ids[0]) == space {
        ids.remove(0);
    }
    ids
}

/// 候選字放在模型 slot 的單一 piece id。
///
/// 訓練時 candidate slot 放的是一個 SentencePiece piece：優先裸 piece
/// （`橋`），詞表只有行首變體時用 `▁橋`。兩者都沒有回 `None`，呼叫端
/// 不應把多 piece 詞面塞進這個 slot，模型只會看到第一個 piece。
pub fn candidate_piece(tokenizer: &Tokenizer, surface: &str) -> Option<u32> {
    if surface.is_empty() {
        return None;
    }
    if let Some(id) = tokenizer.piece_id(surface) {
        return Some(id);
    }
    let mut prefixed = String::with_capacity(surface.len() + 3);
    prefixed.push(crate::tokenizer::SPACE);
    prefixed.push_str(surface);
    tokenizer.piece_id(&prefixed)
}

/// 把左文、候選字與右文打包成模型輸入。
///
/// 佈局與訓練 checkpoint 一致：
///
/// ```text
/// <bos> [left 最多 60 token] <eos> candidate <eos> [right 最多 60 token]
///  0      1..60               61      62         63      64..123
/// ```
///
/// 位置表是 `[0, 61, 62, …, 183]`：左文 60 格、分隔符 121、候選 122、
/// 候選後的 EOS 123、右文 124..183。`left` 為 `None` 或空字串時，最靠近
/// 分隔符的左文 slot 放 BOS；左文超過 `context_tokens` 時保留前段。`right`
/// 取前 `context_tokens` 個 token，緊接在候選後的 EOS 之後。
pub fn pack_batch(
    tokenizer: &Tokenizer,
    spec: &ModelSpec,
    left: Option<&str>,
    right: Option<&str>,
    candidates: &[&str],
) -> Result<PackedBatch, String> {
    validate_layout(spec)?;
    let seq = spec.seq_len();
    let slot = spec.candidate_slot();
    let context = spec.context_tokens();
    let mut ids = vec![spec.padding_id(); candidates.len() * seq];
    let mut mask = vec![0i64; candidates.len() * seq];
    let mut left_ids = left
        .map(|text| context_ids(tokenizer, text))
        .unwrap_or_default();
    left_ids.truncate(context);
    let left_ids: Vec<i64> = left_ids.into_iter().map(i64::from).collect();
    let mut right_ids = right
        .map(|text| context_ids(tokenizer, text))
        .unwrap_or_default();
    right_ids.truncate(context);
    let right_ids: Vec<i64> = right_ids.into_iter().map(i64::from).collect();

    for (row_idx, candidate) in candidates.iter().enumerate() {
        let base = row_idx * seq;
        let row = &mut ids[base..base + seq];
        let row_mask = &mut mask[base..base + seq];
        row[0] = spec.bos_id();
        row_mask[0] = 1;
        if left_ids.is_empty() {
            // 沒有左文時用 BOS 佔住緊鄰分隔符的左文 slot。
            row[slot - 2] = spec.bos_id();
            row_mask[slot - 2] = 1;
        } else {
            // 左文靠右，讓最後一個 token 緊鄰分隔符。
            let start = 1 + context - left_ids.len();
            for (i, &id) in left_ids.iter().enumerate() {
                row[start + i] = id;
                row_mask[start + i] = 1;
            }
        }
        row[slot - 1] = spec.eos_id();
        row_mask[slot - 1] = 1;
        // 候選後面一定要留一格放 EOS；再後面才是右文。
        let cap = seq - slot - 2;
        // 單 piece 詞面直接放那個 piece，與訓練的 candidate slot 一致；
        // 多 piece 詞面才退回一般編碼（只會被截到 slot 起點）。
        let mut cand_ids = match candidate_piece(tokenizer, candidate) {
            Some(id) => vec![id],
            None => tokenizer.encode(candidate),
        };
        cand_ids.truncate(cap);
        let mut pos = slot;
        for id in cand_ids {
            row[pos] = i64::from(id);
            row_mask[pos] = 1;
            pos += 1;
        }
        // 候選後的 EOS；右文從下一格開始，與訓練的 123／124 對齊。
        row[pos] = spec.eos_id();
        row_mask[pos] = 1;
        pos += 1;
        for id in right_ids.iter().take(seq.saturating_sub(pos)) {
            row[pos] = *id;
            row_mask[pos] = 1;
            pos += 1;
        }
    }
    Ok(PackedBatch {
        ids,
        mask,
        seq,
        rows: candidates.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ModelSpec;
    use crate::tokenizer::{Tokenizer, VOCAB_SCHEMA};

    fn tokenizer() -> Tokenizer {
        let mut pieces = vec![
            "<pad>".to_string(),
            "<unk>".to_string(),
            "<s>".to_string(),
            "</s>".to_string(),
        ];
        for b in 0..=255u16 {
            pieces.push(format!("<0x{b:02X}>"));
        }
        for p in ["▁", "東", "京", "都", "▁東", "▁東京", "東京", "▁東京都"] {
            pieces.push(p.to_string());
        }
        let json = serde_json::json!({
            "format": VOCAB_SCHEMA,
            "vocab_size": pieces.len(),
            "special_tokens": {"pad": 0, "unk": 1, "bos": 2, "eos": 3},
            "id_to_piece": pieces,
        });
        Tokenizer::from_json(&serde_json::to_vec(&json).unwrap()).unwrap()
    }

    fn id_of(t: &Tokenizer, piece: &str) -> i64 {
        t.piece_id(piece).unwrap() as i64
    }

    #[test]
    fn packs_bos_left_eos_candidate_eos() {
        let t = tokenizer();
        let spec = ModelSpec::default();
        let batch = pack_batch(&t, &spec, Some("東京都"), None, &["東京"]).unwrap();
        assert_eq!(batch.seq, 124);
        assert_eq!(batch.rows, 1);
        assert_eq!(batch.ids.len(), 124);

        let row = &batch.ids[..124];
        assert_eq!(row[0], 2, "BOS");
        // 左文 3 個 token，靠右放在 1..=60；最短切法是「▁東京都」一個 piece。
        assert_eq!(row[1 + 60 - 1], id_of(&t, "▁東京都"));
        assert_eq!(&row[1..60], &[0; 59]);
        assert_eq!(row[61], 3, "slot 前的 EOS");
        // candidate slot 放裸 piece「東京」，與訓練的單一 piece 一致；沒有裸
        // piece 才退回「▁東京」。
        assert_eq!(row[62], id_of(&t, "東京"));
        assert_eq!(row[63], 3, "候選字後的 EOS");
        assert!(row[64..].iter().all(|&id| id == 0));

        let mask = &batch.mask[..124];
        assert_eq!(mask[0], 1);
        assert_eq!(mask[60], 1);
        assert_eq!(mask[61], 1);
        assert_eq!(mask[62], 1);
        assert_eq!(mask[63], 1);
        assert_eq!(mask[64..].iter().sum::<i64>(), 0);
    }

    #[test]
    fn candidate_piece_prefers_the_bare_piece() {
        let t = tokenizer();
        assert_eq!(candidate_piece(&t, "東京"), Some(id_of(&t, "東京") as u32));
        // 裸「東」存在時優先用它，不換成行首變體「▁東」。
        assert_eq!(candidate_piece(&t, "東"), Some(id_of(&t, "東") as u32));
        // 沒有裸 piece 才退回行首變體。
        assert_eq!(
            candidate_piece(&t, "東京都"),
            Some(id_of(&t, "▁東京都") as u32)
        );
        // 多 piece 詞面回 None，呼叫端不該塞進 slot。
        assert_eq!(candidate_piece(&t, "京都"), None);
        assert_eq!(candidate_piece(&t, ""), None);
    }

    #[test]
    fn candidate_longer_than_slot_is_truncated() {
        let t = tokenizer();
        let spec = ModelSpec {
            seq_len: Some(8),
            candidate_slot: Some(4),
            context_tokens_per_side: Some(2),
            ..Default::default()
        };
        let long = "東京都東京都東京都";
        let batch = pack_batch(&t, &spec, Some("東京"), None, &[long]).unwrap();
        assert_eq!(batch.ids.len(), 8);
        // slot 4..5 給候選字（最多 2 個），slot 6 放 EOS，slot 7 留給右文。
        assert_eq!(batch.ids[6], 3);
        // 有內容的格子：BOS、1 個左文 token、分隔 EOS、2 個候選、候選後 EOS。
        assert_eq!(batch.mask.iter().sum::<i64>(), 6);
    }

    #[test]
    fn right_context_follows_the_candidate_eos() {
        let t = tokenizer();
        let spec = ModelSpec::default();
        let batch = pack_batch(&t, &spec, Some("東京"), Some("東京都東京"), &["東"]).unwrap();
        let row = &batch.ids[..batch.seq];
        assert_eq!(row[62], id_of(&t, "東"));
        assert_eq!(row[63], 3, "候選後的 EOS");
        // 右文從 64 起依序放，位置 123 對應訓練的 183。
        assert_eq!(row[64], id_of(&t, "▁東京都"));
        assert_eq!(row[65], id_of(&t, "東京"));
        assert_eq!(batch.mask[64], 1);
        assert_eq!(batch.mask[65], 1);
        assert_eq!(batch.mask[66..].iter().sum::<i64>(), 0);
    }

    #[test]
    fn context_drops_the_artificial_word_boundary() {
        let t = tokenizer();
        // 「京」沒有「▁京」這個 piece，編碼會先吐出獨立的「▁」。
        assert_eq!(t.encode("京")[0], id_of(&t, "▁") as u32);
        let batch = pack_batch(&t, &ModelSpec::default(), Some("京"), None, &["東"]).unwrap();
        let row = &batch.ids[..124];
        assert_eq!(row[60], id_of(&t, "京"), "右對齊後只剩真正的 token");
        assert_eq!(&row[1..60], &[0; 59]);
        assert_eq!(batch.mask[1..60].iter().sum::<i64>(), 0);
    }

    #[test]
    fn empty_context_uses_bos_marker() {
        let t = tokenizer();
        for left in [None, Some("")] {
            let batch = pack_batch(&t, &ModelSpec::default(), left, None, &["東京"]).unwrap();
            assert_eq!(batch.ids[0], 2, "序列開頭的 BOS");
            assert_eq!(batch.ids[60], 2, "左文 slot 的 BOS 標記");
            assert_eq!(batch.ids[61], 3);
            // candidate slot 放裸 piece「東京」，沒有才退回「▁東京」。
            assert_eq!(batch.ids[62], id_of(&t, "東京"));
            // 1..60 只有最後一格是 BOS，其餘是 PAD 且被遮罩。
            assert_eq!(&batch.ids[1..60], &[0; 59]);
            assert_eq!(&batch.mask[1..60], &[0; 59]);
            assert_eq!(batch.mask[60], 1);
        }
    }

    #[test]
    fn invalid_layout_is_rejected() {
        let mut spec = ModelSpec {
            candidate_slot: Some(123),
            ..Default::default()
        };
        assert!(validate_layout(&spec).is_err());
        spec.candidate_slot = Some(62);
        spec.context_tokens_per_side = Some(61);
        assert!(validate_layout(&spec).is_err());
    }

    #[test]
    fn normalized_spaces_are_preserved_in_layout() {
        let t = tokenizer();
        let batch = pack_batch(&t, &ModelSpec::default(), Some("東京 都"), None, &[]).unwrap();
        assert_eq!(batch.rows, 0);
        assert!(batch.ids.is_empty());
        // 空白不影響 encode 的 dummy prefix 規則。
        assert_eq!(t.normalize("東京 都"), "▁東京▁都");
    }

    #[test]
    fn long_left_context_keeps_the_head() {
        let t = tokenizer();
        let spec = ModelSpec {
            seq_len: Some(8),
            candidate_slot: Some(4),
            context_tokens_per_side: Some(2),
            ..Default::default()
        };
        // 「東京都東京」在前身 `capIDs` 的切法是保留前 2 個 token。
        let batch = pack_batch(&t, &spec, Some("東京都東京"), None, &["東"]).unwrap();
        assert_eq!(batch.ids[1], id_of(&t, "▁東京都"));
        assert_eq!(batch.ids[2], id_of(&t, "東京"));
        assert_eq!(batch.ids[3], 3, "分隔符固定在 slot - 1");
    }

    /// 真正的 SentencePiece 模型：左文上限 60 token、靠右對齊、保留前段。
    #[test]
    fn real_tokenizer_fills_the_60_token_left_window() {
        let path = jc_testdata::require_data!("models/ja-tokenizer/public_core_ja.model");
        let t = Tokenizer::load(&path).unwrap();
        let spec = ModelSpec::default();
        // 「東京都」每次 1 個 token，這裡湊出遠超過 60 個 token 的左文。
        let left = "東京都".repeat(80);
        let all = t.encode(&left);
        assert!(all.len() > 60, "左文必須超過上限，實際 {} token", all.len());
        // `context_ids` 會剝掉獨立詞首「▁」；這個模型把「▁」併進後面的
        // piece，所以先依同樣規則算出真正的左文再比對。
        let skip = usize::from(all[0] == t.piece_id("▁").unwrap());
        let expected = &all[skip..];
        assert_eq!(context_ids(&t, &left), expected);

        let batch = pack_batch(&t, &spec, Some(&left), None, &["東京"]).unwrap();
        let row = &batch.ids[..batch.seq];
        let want: Vec<i64> = expected[..60].iter().map(|&id| i64::from(id)).collect();
        // 左文區 1..=60 全部填滿，而且是真的 token 不是 PAD。
        assert_eq!(batch.mask[1..=60].iter().sum::<i64>(), 60);
        assert!(row[1..=60].iter().all(|&id| id != spec.padding_id()));
        // 靠右對齊：最後一個左文 token 緊鄰分隔符，內容是前 60 個 token。
        assert_eq!(&row[1..=60], &want[..]);
        assert_eq!(row[61], spec.eos_id());
        // candidate slot 用裸 piece「東京」；encode 會給「▁東京」，兩者不同。
        assert_eq!(row[62] as u32, candidate_piece(&t, "東京").unwrap());
    }
}
