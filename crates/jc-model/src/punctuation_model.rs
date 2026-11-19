//! 日文標點模型。
//!
//! 模型輸入是 `input_ids`／`attention_mask`，輸出每個 token 4 個標籤的
//! `logits`：`none`、`comma`、`period`、`question`。推論以 sliding window
//! 前進，逐 token 取 argmax，再把 token 結尾映射回原字串的 rune 位移。
//! 套用沿用 [`crate::punct::apply_japanese`]，只插入、不改寫原字。

use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use ort::session::Session;

use crate::error::{ModelError, Result};
use crate::manifest::ModelSpec;
use crate::punct_plan::{apply, argmax, is_punct, merge_window_rows, window_starts, Edit};
use crate::runtime::{build_session, run_timed};
use crate::tokenizer::{TokenSpan, Tokenizer, SPACE};
use crate::Role;

/// 單次標點推論的 token 視窗。
pub const WINDOW: usize = 256;
/// 滑動視窗前進量。
pub const STRIDE: usize = WINDOW / 2;
/// 模型標籤；索引 0 是不插入。
pub const LABELS: [&str; 4] = ["none", "comma", "period", "question"];
/// 對應標籤要插入的標點。
pub const MARKS: [&str; 4] = ["", "、", "。", "？"];

/// 標點模型。
pub struct PunctuationModel {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
    spec: ModelSpec,
}

impl PunctuationModel {
    /// 以預設檔名載入角色目錄。
    pub fn load(dir: impl AsRef<Path>) -> Result<Self> {
        Self::load_with_spec(dir, &ModelSpec::default(), None)
    }

    /// 以指定設定與 ORT 動態庫路徑載入。
    pub fn load_with_spec(
        dir: impl AsRef<Path>,
        spec: &ModelSpec,
        ort_lib: Option<&Path>,
    ) -> Result<Self> {
        if spec.timeout().is_zero() {
            return Err(ModelError::Manifest("timeout 必須大於 0".into()));
        }
        crate::runtime::init_runtime(ort_lib)?;
        let dir = dir.as_ref();
        let tokenizer = Tokenizer::load(dir.join(spec.vocab_file()))?;
        let session = build_session(&dir.join(spec.model_file()), spec.threads())?;
        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
            spec: spec.clone(),
        })
    }

    /// 模型角色。
    pub fn role() -> Role {
        Role::JaPunct
    }

    /// tokenizer。
    pub fn tokenizer(&self) -> &Tokenizer {
        &self.tokenizer
    }

    /// 在文字中插入標點。
    ///
    /// 只插入模型判定的標點；逾時、推論失敗或沒有合規編輯時原樣回傳。
    /// 使用者已輸入的標點旁邊不再補，避免出現「。。」或「、、」。
    pub fn punctuate(&self, text: &str) -> String {
        self.try_punctuate(text).unwrap_or_else(|| text.to_string())
    }

    fn try_punctuate(&self, text: &str) -> Option<String> {
        if text.is_empty() {
            return Some(text.to_string());
        }
        let (tokens, norm, map) = self.tokenizer.encode_with_map(text);
        if tokens.is_empty() {
            return Some(text.to_string());
        }
        let starts = window_starts(tokens.len(), WINDOW, STRIDE);
        let mut rows = Vec::with_capacity(starts.len());
        for &start in &starts {
            let end = (start + WINDOW).min(tokens.len());
            let ids: Vec<i64> = tokens[start..end]
                .iter()
                .map(|span| i64::from(span.id))
                .collect();
            rows.push(self.run_window(ids)?);
        }
        let merged = merge_window_rows(tokens.len(), WINDOW, &starts, &rows);
        let decisions: Vec<usize> = merged
            .iter()
            .map(|row| row.as_ref().map(|row| argmax(row)).unwrap_or(0))
            .collect();
        let edits = token_edits(text, &norm, &tokens, &map, &decisions);
        if edits.is_empty() {
            return Some(text.to_string());
        }
        let (out, ok) = apply(text, &edits, true, false);
        Some(if ok { out } else { text.to_string() })
    }

    /// 跑一個 token 視窗，回傳 `[token][class]` 的 logits。
    fn run_window(&self, ids: Vec<i64>) -> Option<Vec<Vec<f32>>> {
        let seq = ids.len();
        if seq == 0 {
            return Some(Vec::new());
        }
        let mask = vec![1i64; seq];
        let mut session = self.session.lock().ok()?;
        let data = run_timed(
            &mut session,
            ids,
            mask,
            1,
            seq,
            self.spec.timeout(),
            "logits",
        )
        .ok()?;
        if data.len() % seq != 0 {
            return None;
        }
        let classes = data.len() / seq;
        if classes == 0 {
            return None;
        }
        let mut rows = Vec::with_capacity(seq);
        for i in 0..seq {
            rows.push(data[i * classes..(i + 1) * classes].to_vec());
        }
        Some(rows)
    }

    /// 跑一次模型，回傳 `[seq][classes]` 的 logits；逾時或失敗回 `None`。
    ///
    /// 只處理前 [`WINDOW`] 個 token，供 probe 與舊呼叫端使用。
    pub fn logits(&self, text: &str) -> Option<Vec<Vec<f32>>> {
        let mut ids = self.tokenizer.encode_i64(text);
        if ids.is_empty() {
            return Some(Vec::new());
        }
        ids.truncate(WINDOW);
        self.run_window(ids)
    }
}

/// 把逐 token 的標籤決策映射回原文 rune 位移。
///
/// token 與 `map` 都指向正規化字串；只有完整落在原字上的 token 可以插入
/// 標點。byte fallback token、空白 token 與重複位移都跳過。
fn token_edits(
    text: &str,
    norm: &str,
    tokens: &[TokenSpan],
    map: &[i32],
    decisions: &[usize],
) -> Vec<Edit> {
    let norm_runes: Vec<char> = norm.chars().collect();
    let orig: Vec<char> = text.chars().collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if token.byte || token.start >= token.end || token.end > norm_runes.len() {
            continue;
        }
        let Some(&label) = decisions.get(index) else {
            continue;
        };
        if label == 0 || label >= MARKS.len() {
            continue;
        }
        let surface: String = norm_runes[token.start..token.end]
            .iter()
            .map(|ch| if *ch == SPACE { ' ' } else { *ch })
            .collect();
        if surface.trim().is_empty() {
            continue;
        }
        let offset = original_offset(token.end, map);
        if offset <= 0 || offset as usize > orig.len() {
            continue;
        }
        let offset = offset as usize;
        if !seen.insert(offset) {
            continue;
        }
        if touches_punctuation(&orig, offset) {
            continue;
        }
        out.push(Edit::new(offset, MARKS[label]));
    }
    out
}

fn original_offset(norm_end: usize, map: &[i32]) -> i32 {
    for i in (0..norm_end).rev() {
        if let Some(&at) = map.get(i) {
            if at >= 0 {
                return at + 1;
            }
        }
    }
    -1
}

fn touches_punctuation(orig: &[char], offset: usize) -> bool {
    offset
        .checked_sub(1)
        .and_then(|i| orig.get(i))
        .is_some_and(|ch| is_punct(*ch))
        || orig.get(offset).is_some_and(|ch| is_punct(*ch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_punctuate_returns_input() {
        // 不需要載入模型就能驗證 identity stub 契約。
        assert_eq!(crate::punctuate("今日は晴れ"), "今日は晴れ");
        assert_eq!(crate::punctuate(""), "");
    }

    #[test]
    fn window_is_bounded() {
        const { assert!(WINDOW > 0 && WINDOW <= 512) };
        const { assert!(STRIDE > 0 && STRIDE < WINDOW) };
    }

    #[test]
    fn test_labels_and_marks_align() {
        assert_eq!(LABELS[0], "none");
        assert_eq!(MARKS[0], "");
        assert_eq!(MARKS[1], "、");
        assert_eq!(MARKS[2], "。");
        assert_eq!(MARKS[3], "？");
    }

    #[test]
    fn test_token_edits_maps_end_back_to_original_runes() {
        // 正規化：▁今日は▁天気 → 原字：今日は 天気
        let text = "今日は 天気";
        let norm = "▁今日は▁天気";
        let map = vec![-1, 0, 1, 2, -1, 4, 5];
        let tokens = vec![
            TokenSpan {
                id: 1,
                start: 0,
                end: 1,
                byte: false,
            },
            TokenSpan {
                id: 2,
                start: 1,
                end: 4,
                byte: false,
            },
            TokenSpan {
                id: 3,
                start: 4,
                end: 5,
                byte: false,
            },
            TokenSpan {
                id: 4,
                start: 5,
                end: 7,
                byte: false,
            },
        ];
        // 在「今日は」後補「、」，在「天気」後補「。」。
        let edits = token_edits(text, norm, &tokens, &map, &[0, 1, 0, 2]);
        assert_eq!(edits, vec![Edit::new(3, "、"), Edit::new(6, "。")]);
        let (out, ok) = apply(text, &edits, true, false);
        assert!(ok);
        assert_eq!(out, "今日は、 天気。");
    }

    #[test]
    fn test_token_edits_skips_byte_tokens_and_existing_punctuation() {
        let text = "今日は。";
        let norm = "▁今日は。";
        let map = vec![-1, 0, 1, 2, 3];
        let tokens = vec![
            TokenSpan {
                id: 1,
                start: 0,
                end: 1,
                byte: false,
            },
            TokenSpan {
                id: 2,
                start: 1,
                end: 4,
                byte: false,
            },
            TokenSpan {
                id: 3,
                start: 4,
                end: 5,
                byte: true,
            },
        ];
        assert!(token_edits(text, norm, &tokens, &map, &[0, 2, 2]).is_empty());
    }

    #[test]
    fn test_original_offset_walks_back_over_inserted_spaces() {
        let map = vec![-1, 0, 1, -1];
        assert_eq!(original_offset(3, &map), 2);
        assert_eq!(original_offset(4, &map), 2);
        assert_eq!(original_offset(0, &map), -1);
    }
}
