//! 日文候選重排模型。
//!
//! 模型輸入是 `input_ids`／`attention_mask`，輸出是每個候選字一個
//! `scores`。分數越高越相關。推論失敗或超時由呼叫端保留統計排序。

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use crate::candle::{quant, CandleReranker, QuantReranker};
use crate::error::{ModelError, Result};
use crate::layout::{pack_batch, validate_layout, PackedBatch};
use crate::manifest::ModelSpec;
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
    engine: Engine,
    tokenizer: Tokenizer,
    spec: ModelSpec,
}

/// 實際執行 forward 的權重格式。
enum Engine {
    /// candle fp32（`model.safetensors`）；forward 沒有內部可變狀態。
    Fp32(Mutex<CandleReranker>),
    /// candle + GGUF 量化權重。
    Quant(Mutex<QuantReranker>),
}

impl Reranker {
    /// 以預設佈局載入角色目錄。
    pub fn load(dir: impl AsRef<Path>) -> Result<Self> {
        Self::load_with_spec(dir, &ModelSpec::default())
    }

    /// 以指定佈局載入角色目錄。
    ///
    /// 依 [`crate::candle::weights_path`] 找 safetensors 或 GGUF；`.gguf`
    /// 自動走量化路徑。找不到權重時回 [`ModelError::Candle`]，呼叫端退回
    /// 統計排序。
    pub fn load_with_spec(dir: impl AsRef<Path>, spec: &ModelSpec) -> Result<Self> {
        validate(spec)?;
        let dir = dir.as_ref();
        let tokenizer = Tokenizer::load(dir.join(spec.vocab_file()))?;
        let weights = crate::candle::weights_path(dir, spec).ok_or_else(|| {
            ModelError::Candle(format!(
                "找不到 candle 權重，預期 {} 或 ${} 指定的檔案",
                dir.join(crate::candle::DEFAULT_WEIGHTS_FILE).display(),
                crate::candle::WEIGHTS_ENV
            ))
        })?;
        // `.gguf` 走量化權重；副檔名是 safetensors 時走 fp32。
        let engine = if quant::is_gguf(&weights) {
            Engine::Quant(Mutex::new(QuantReranker::load(&weights, spec)?))
        } else {
            Engine::Fp32(Mutex::new(CandleReranker::load(&weights, spec)?))
        };
        Ok(Self {
            engine,
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

    /// 為單一候選字打分；失敗回 `None`。
    ///
    /// `left`／`right` 為 `None` 或空字串代表沒有那一側的上下文。
    pub fn score(&self, left: Option<&str>, candidate: &str, right: &str) -> Option<f32> {
        self.score_batch(left, right, &[candidate])
            .and_then(|mut v| if v.len() == 1 { v.pop() } else { None })
    }

    /// 為多個候選字打分；失敗回 `None`。
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
        let data = self.score_packed(&batch)?;
        if data.len() != candidates.len() {
            return Err(ModelError::Inference(format!(
                "scores 長度為 {}，候選字有 {}",
                data.len(),
                candidates.len()
            )));
        }
        Ok(data)
    }

    /// 為已打包的 batch 打分，保留錯誤原因。
    ///
    /// candle 的 forward 無法中途停止；群組之間的預算由呼叫端控制。
    pub fn score_packed(&self, batch: &PackedBatch) -> Result<Vec<f32>> {
        if batch.rows == 0 {
            return Ok(Vec::new());
        }
        match &self.engine {
            Engine::Fp32(model) => {
                let model = model
                    .lock()
                    .map_err(|_| ModelError::Candle("模型鎖已損壞".into()))?;
                model.score_packed(batch)
            }
            Engine::Quant(model) => {
                let model = model
                    .lock()
                    .map_err(|_| ModelError::Candle("模型鎖已損壞".into()))?;
                model.score_packed(batch)
            }
        }
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
