//! candle 版日文標點模型。
//!
//! 與 ORT 路徑吃同一組 `input_ids`／`attention_mask`，輸出
//! `[rows][seq][4]` logits，標籤順序 `none`、`comma`、`period`、`question`。
//! encoder 與 reranker 共用 [`super::Encoder`]，差別只有位置是 `arange(seq)`
//! 與最後接一個 4 類分類頭。

use std::path::Path;

use candle_core::{Device, Module, Tensor};
use candle_nn::Linear;

use crate::error::{ModelError, Result};

use super::{additive_mask, load_weights, CandleConfig, Encoder};

/// 標籤數；與 [`crate::punctuation_model::LABELS`] 一致。
pub const NUM_LABELS: usize = 4;

/// 已載入權重的 candle 標點模型。
///
/// forward 沒有內部可變狀態，`&self` 可以跨執行緒共用；推論的 linear 以
/// 輸出維度切給 rayon，數值與單執行緒相同。
pub struct CandlePunctuation {
    config: CandleConfig,
    device: Device,
    encoder: Encoder<Linear>,
    classifier: Linear,
}

impl CandlePunctuation {
    /// 從 fp32 safetensors 載入權重。
    ///
    /// 張量名稱與 PyTorch checkpoint 相同；shape 對不上時回
    /// [`ModelError::Inference`]，不會靜默降級。
    pub fn load(weights: &Path) -> Result<Self> {
        let config = CandleConfig::for_punct();
        let (device, vb) = load_weights(weights)?;
        let encoder = <Encoder<Linear>>::load(&vb, &config)?;
        let classifier = Linear::new(
            vb.get((NUM_LABELS, config.d_model), "classifier.weight")
                .map_err(super::inference)?,
            Some(
                vb.get(NUM_LABELS, "classifier.bias")
                    .map_err(super::inference)?,
            ),
        );
        Ok(Self {
            config,
            device,
            encoder,
            classifier,
        })
    }

    /// 架構參數。
    pub fn config(&self) -> &CandleConfig {
        &self.config
    }

    /// 為多個 token 序列跑一次 batch forward。
    ///
    /// 回傳 `[rows][len][4]` logits；短序列補 pad 後以 attention mask 遮掉，
    /// 只回傳每一列自己的真實長度。空輸入回傳空向量。
    pub fn logits(&self, windows: &[Vec<i64>]) -> Result<Vec<Vec<Vec<f32>>>> {
        if windows.is_empty() {
            return Ok(Vec::new());
        }
        let rows = windows.len();
        let width = windows.iter().map(Vec::len).max().unwrap_or(0);
        if width == 0 {
            return Ok(vec![Vec::new(); rows]);
        }
        if width > self.config.max_positions {
            return Err(ModelError::Inference(format!(
                "candle：序列長度 {width} 超過 max_positions {}",
                self.config.max_positions
            )));
        }
        let mut ids = vec![0u32; rows * width];
        let mut mask = vec![0i64; rows * width];
        for (row, window) in windows.iter().enumerate() {
            for (column, id) in window.iter().enumerate() {
                ids[row * width + column] = u32::try_from(*id).map_err(|_| {
                    ModelError::Inference(format!("candle：token id {id} 不是合法的 u32"))
                })?;
                mask[row * width + column] = 1;
            }
        }
        let positions: Vec<u32> = (0..width as u32).cycle().take(rows * width).collect();
        let ids = Tensor::from_vec(ids, (rows, width), &self.device).map_err(super::inference)?;
        let positions =
            Tensor::from_vec(positions, (rows, width), &self.device).map_err(super::inference)?;
        let additive = additive_mask(&mask, rows, width);
        let hidden = self.encoder.forward(&ids, &positions, additive.as_ref())?;
        let logits = self
            .classifier
            .forward(&hidden)
            .and_then(|tensor| tensor.to_vec3::<f32>())
            .map_err(super::inference)?;
        Ok(logits
            .into_iter()
            .zip(windows)
            .map(|(row, window)| row.into_iter().take(window.len()).collect())
            .collect())
    }

    /// 單一序列的 logits；回傳 `[len][4]`。
    pub fn logits_one(&self, ids: &[i64]) -> Result<Vec<Vec<f32>>> {
        let mut rows = self.logits(std::slice::from_ref(&ids.to_vec()))?;
        Ok(rows.pop().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 固定輸入的 logits 必須與 ONNX fp32 一致；期望值由
    /// `punctuation-public-core.fp32.onnx` 在相同 id／mask 上跑出來。
    #[test]
    #[allow(clippy::excessive_precision)]
    fn matches_the_onnx_fp32_reference() {
        let weights =
            jc_testdata::require_data!("models/punctuation-public-core/model.safetensors");
        let model = CandlePunctuation::load(&weights).unwrap();
        assert_eq!(model.config().vocab_size, 32768);
        assert_eq!(model.config().num_layers, 8);
        let logits = model.logits_one(&[2, 100, 200, 300, 400, 500, 3]).unwrap();
        assert_eq!(logits.len(), 7);
        assert_eq!(logits[0].len(), 4);
        let want = [
            [0.493_629_9, -0.480_246_5, -1.641_901_4, -7.442_322],
            [0.573_491_4, -0.572_031_1, -1.776_131_5, -7.477_048_4],
            [1.437_404_8, -1.533_432_5, -2.522_755, -8.952_353_5],
            [1.560_160_9, -1.276_383_5, -2.883_434, -7.723_038],
            [0.618_127_2, 0.779_12, -5.317_123_4, -9.992_228_5],
            [-0.119_844_7, -1.056_407_9, 0.752_5, -5.887_038],
            [0.771_299_4, -0.885_814_9, -2.071_783, -7.526_392_5],
        ];
        for (index, (got, want)) in logits.iter().zip(want).enumerate() {
            for (class, (got, want)) in got.iter().zip(want).enumerate() {
                let diff = (got - want).abs();
                assert!(
                    diff < 1e-3,
                    "token {index} class {class}：candle {got} vs ONNX {want}，誤差 {diff}"
                );
            }
        }
    }

    /// 被遮罩的 token 不能影響其他位置；期望值是相同 id 但尾端 mask 0。
    #[test]
    #[allow(clippy::excessive_precision)]
    fn masked_tail_matches_the_onnx_reference() {
        let weights =
            jc_testdata::require_data!("models/punctuation-public-core/model.safetensors");
        let model = CandlePunctuation::load(&weights).unwrap();
        // 第二列只有 6 個真 token，第 7 格是 pad；ORT 對照是同一組 id 把
        // mask[0, -1] 設為 0。
        let logits = model
            .logits(&[
                vec![2, 100, 200, 300, 400, 500, 3],
                vec![2, 100, 200, 300, 400, 500],
            ])
            .unwrap();
        assert_eq!(logits[1].len(), 6);
        let want = [
            [0.776_121_7, -0.824_034_9, -1.948_210_8, -6.918_453_7],
            [1.029_271_7, -1.017_109_4, -2.448_363_5, -7.130_779_7],
            [2.378_249_9, -2.627_466_4, -3.605_098, -8.975_429_5],
            [1.927_455_2, -1.796_426_7, -3.040_175_9, -7.359_041_7],
            [-0.527_724_6, 1.554_240_9, -2.843_426_9, -8.942_622_2],
            [2.449_200_4, -2.166_866_8, -4.086_864, -9.712_142],
        ];
        for (index, (got, want)) in logits[1].iter().zip(want).enumerate() {
            for (class, (got, want)) in got.iter().zip(want).enumerate() {
                let diff = (got - want).abs();
                assert!(
                    diff < 1e-3,
                    "token {index} class {class}：candle {got} vs ONNX {want}，誤差 {diff}"
                );
            }
        }
    }
}
