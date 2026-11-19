//! 日文標點模型（需要 `onnx` feature）。
//!
//! 推論介面已接上：吃 token、吐每個位置的 class logits。把 logits 轉成
//! 插入位置、以及 [`PunctuationModel::punctuate`] 仍是 stub，會原樣回傳
//! 輸入；這是任務書允許的階段性實作。

use std::path::Path;
use std::sync::Mutex;

use ort::session::Session;

use crate::error::{ModelError, Result};
use crate::manifest::ModelSpec;
use crate::runtime::{build_session, run_timed};
use crate::tokenizer::Tokenizer;
use crate::Role;

/// 單次標點推論的 token 視窗。
pub const WINDOW: usize = 256;

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
    /// **Stub**：目前原樣回傳。`logits` 已可取得模型輸出，後續接上
    /// decision／插入規則後再替換。
    pub fn punctuate(&self, text: &str) -> String {
        text.to_string()
    }

    /// 跑一次模型，回傳 `[seq][classes]` 的 logits；逾時或失敗回 `None`。
    pub fn logits(&self, text: &str) -> Option<Vec<Vec<f32>>> {
        let mut ids = self.tokenizer.encode_i64(text);
        if ids.is_empty() {
            return Some(Vec::new());
        }
        ids.truncate(WINDOW);
        let seq = ids.len();
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
        assert!(WINDOW > 0 && WINDOW <= 512);
    }
}
