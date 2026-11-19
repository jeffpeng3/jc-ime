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

/// 把左文與候選字打包成模型輸入。
///
/// `left` 為 `None` 或空字串時代表沒有左文：在最靠近分隔符的左文 slot 放
/// BOS，讓模型看到「上下文開始」而不是一整排 PAD。左文超過
/// `context_tokens` 時保留前段，與前身 `capIDs` 一致；候選字超過可用 slot
/// 時直接截斷，剩下一格放 EOS。
pub fn pack_batch(
    tokenizer: &Tokenizer,
    spec: &ModelSpec,
    left: Option<&str>,
    candidates: &[&str],
) -> Result<PackedBatch, String> {
    validate_layout(spec)?;
    let seq = spec.seq_len();
    let slot = spec.candidate_slot();
    let context = spec.context_tokens();
    let mut ids = vec![spec.padding_id(); candidates.len() * seq];
    let mut mask = vec![0i64; candidates.len() * seq];
    let mut left_ids = left.map(|text| tokenizer.encode(text)).unwrap_or_default();
    left_ids.truncate(context);
    let left_ids: Vec<i64> = left_ids.into_iter().map(i64::from).collect();

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
        let cap = seq - slot - 1;
        let mut cand_ids = tokenizer.encode(candidate);
        cand_ids.truncate(cap);
        let mut pos = slot;
        for id in cand_ids {
            row[pos] = i64::from(id);
            row_mask[pos] = 1;
            pos += 1;
        }
        if pos < seq {
            row[pos] = spec.eos_id();
            row_mask[pos] = 1;
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
        let batch = pack_batch(&t, &spec, Some("東京都"), &["東京"]).unwrap();
        assert_eq!(batch.seq, 124);
        assert_eq!(batch.rows, 1);
        assert_eq!(batch.ids.len(), 124);

        let row = &batch.ids[..124];
        assert_eq!(row[0], 2, "BOS");
        // 左文 3 個 token，靠右放在 1..=60；最短切法是「▁東京都」一個 piece。
        assert_eq!(row[1 + 60 - 1], id_of(&t, "▁東京都"));
        assert_eq!(&row[1..60], &[0; 59]);
        assert_eq!(row[61], 3, "slot 前的 EOS");
        assert_eq!(row[62], id_of(&t, "▁東京"));
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
    fn candidate_longer_than_slot_is_truncated() {
        let t = tokenizer();
        let spec = ModelSpec {
            seq_len: Some(8),
            candidate_slot: Some(4),
            context_tokens_per_side: Some(2),
            ..Default::default()
        };
        let long = "東京都東京都東京都";
        let batch = pack_batch(&t, &spec, Some("東京"), &[long]).unwrap();
        assert_eq!(batch.ids.len(), 8);
        // slot 4..6 給候選字（最多 3 個），最後一格 7 放 EOS。
        assert_eq!(batch.ids[7], 3);
        assert_eq!(batch.mask.iter().sum::<i64>(), 7);
    }

    #[test]
    fn empty_context_uses_bos_marker() {
        let t = tokenizer();
        for left in [None, Some("")] {
            let batch = pack_batch(&t, &ModelSpec::default(), left, &["東京"]).unwrap();
            assert_eq!(batch.ids[0], 2, "序列開頭的 BOS");
            assert_eq!(batch.ids[60], 2, "左文 slot 的 BOS 標記");
            assert_eq!(batch.ids[61], 3);
            assert_eq!(batch.ids[62], id_of(&t, "▁東京"));
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
        let batch = pack_batch(&t, &ModelSpec::default(), Some("東京 都"), &[]).unwrap();
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
        let batch = pack_batch(&t, &spec, Some("東京都東京"), &["東"]).unwrap();
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

        let batch = pack_batch(&t, &spec, Some(&left), &["東京"]).unwrap();
        let row = &batch.ids[..batch.seq];
        let want: Vec<i64> = all[..60].iter().map(|&id| i64::from(id)).collect();
        // 左文區 1..=60 全部填滿，而且是真的 token 不是 PAD。
        assert_eq!(batch.mask[1..=60].iter().sum::<i64>(), 60);
        assert!(row[1..=60].iter().all(|&id| id != spec.padding_id()));
        // 靠右對齊：最後一個左文 token 緊鄰分隔符，內容是前 60 個 token。
        assert_eq!(&row[1..=60], &want[..]);
        assert_eq!(row[61], spec.eos_id());
        assert_eq!(row[62] as u32, t.encode("東京")[0]);
    }
}
