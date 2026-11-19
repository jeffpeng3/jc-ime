//! 日文候選重排模型（需要 `onnx` feature）。
//!
//! 模型輸入是 `input_ids`／`attention_mask`，輸出是每個候選字一個
//! `scores`。分數越高越相關。推論超過預算（預設 100 ms）就回 `None`，
//! 呼叫端保留統計排序。

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use ort::session::Session;

use crate::error::{ModelError, Result};
use crate::layout::{pack_batch, validate_layout};
use crate::manifest::ModelSpec;
use crate::runtime::{build_session, run_timed};
use crate::tokenizer::Tokenizer;
use crate::Role;

/// 一次推論最多幾個候選字（與前身一致）。
pub const MAX_CANDIDATES: usize = 8;
/// 允許的最大推論預算。
pub const MAX_TIMEOUT: Duration = Duration::from_millis(100);
/// 允許的最大 intra-op 執行緒數。
pub const MAX_THREADS: usize = 8;

/// 日文重排模型。
pub struct Reranker {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
    spec: ModelSpec,
}

impl Reranker {
    /// 以預設佈局載入角色目錄。
    pub fn load(dir: impl AsRef<Path>) -> Result<Self> {
        Self::load_with_spec(dir, &ModelSpec::default(), None)
    }

    /// 以指定佈局與 ORT 動態庫路徑載入。
    ///
    /// `ort_lib` 為 `None` 時使用 `ORT_DYLIB_PATH`。
    pub fn load_with_spec(
        dir: impl AsRef<Path>,
        spec: &ModelSpec,
        ort_lib: Option<&Path>,
    ) -> Result<Self> {
        validate(spec)?;
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
        Role::JaRerank
    }

    /// tokenizer。
    pub fn tokenizer(&self) -> &Tokenizer {
        &self.tokenizer
    }

    /// 佈局設定。
    pub fn spec(&self) -> &ModelSpec {
        &self.spec
    }

    /// 為單一候選字打分；逾時或失敗回 `None`。
    ///
    /// `left`／`right` 為 `None` 或空字串代表沒有那一側的上下文。
    pub fn score(&self, left: Option<&str>, candidate: &str, right: &str) -> Option<f32> {
        self.score_batch(left, right, &[candidate])
            .and_then(|mut v| if v.len() == 1 { v.pop() } else { None })
    }

    /// 為多個候選字打分；逾時或失敗回 `None`。
    pub fn score_batch(
        &self,
        left: Option<&str>,
        right: &str,
        candidates: &[&str],
    ) -> Option<Vec<f32>> {
        self.try_score_batch(left, right, candidates).ok()
    }

    /// 為多個候選字打分，保留錯誤原因。
    pub fn try_score_batch(
        &self,
        left: Option<&str>,
        right: &str,
        candidates: &[&str],
    ) -> Result<Vec<f32>> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        if candidates.len() > MAX_CANDIDATES {
            return Err(ModelError::Inference(format!(
                "候選字最多 {MAX_CANDIDATES} 個，收到 {}",
                candidates.len()
            )));
        }
        let right = (!right.is_empty()).then_some(right);
        let batch = pack_batch(&self.tokenizer, &self.spec, left, right, candidates)
            .map_err(ModelError::Inference)?;
        let mut session = self
            .session
            .lock()
            .map_err(|_| ModelError::Inference("session 鎖已損壞".into()))?;
        let data = run_timed(
            &mut session,
            batch.ids,
            batch.mask,
            batch.rows,
            batch.seq,
            self.spec.timeout(),
            "scores",
        )?;
        if data.len() != candidates.len() {
            return Err(ModelError::Inference(format!(
                "scores 長度為 {}，候選字有 {}",
                data.len(),
                candidates.len()
            )));
        }
        Ok(data)
    }
}

fn validate(spec: &ModelSpec) -> Result<()> {
    validate_layout(spec).map_err(ModelError::Manifest)?;
    let timeout = spec.timeout();
    if timeout.is_zero() || timeout > MAX_TIMEOUT {
        return Err(ModelError::Manifest(format!(
            "timeout 必須落在 1..={} ms，實際為 {} ms",
            MAX_TIMEOUT.as_millis(),
            timeout.as_millis()
        )));
    }
    if spec.threads() > MAX_THREADS {
        return Err(ModelError::Manifest(format!(
            "threads 最多 {MAX_THREADS}，實際為 {}",
            spec.threads()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_spec_is_valid() {
        assert!(validate(&ModelSpec::default()).is_ok());
    }

    #[test]
    fn timeout_above_contract_is_rejected() {
        let mut spec = ModelSpec {
            timeout_ms: Some(101),
            ..Default::default()
        };
        assert!(validate(&spec).is_err());
        spec.timeout_ms = Some(0);
        assert!(validate(&spec).is_err());
        spec.timeout_ms = Some(100);
        assert!(validate(&spec).is_ok());
    }

    #[test]
    fn layout_mismatch_is_rejected() {
        let spec = ModelSpec {
            candidate_slot: Some(10),
            context_tokens_per_side: Some(20),
            ..Default::default()
        };
        let err = validate(&spec).unwrap_err().to_string();
        assert!(err.contains("context_tokens"), "{err}");
    }

    #[test]
    fn too_many_threads_is_rejected() {
        let spec = ModelSpec {
            threads: Some(9),
            ..Default::default()
        };
        assert!(validate(&spec).is_err());
    }
}
