//! candle 版標點模型推論，共用 `candle-spike-ja` 的 encoder 實作。
//!
//! 標點模型與日文 reranker 是同一種 pre-LN transformer encoder：D512、8 層、
//! 8 heads、FFN 1024、詞表 32768。差別只在 head 與前處理：
//!
//! - reranker 在固定的 right-anchored 位置表上取單一 pool slot 打分；
//! - 標點模型用 `arange(seq)` 位置，每一個 token 各自輸出 4 類 logits。
//!
//! 因此 [`Encoder`]、[`Layer`] 與 [`linear_chunked`] 設計成兩個模型共用；
//! 本分支由 [`punct::CandlePunctuation`] 使用，reranker 端接在
//! `candle-spike-ja`。權重來自
//! PyTorch checkpoint 轉出的 safetensors，轉檔腳本見
//! `scripts/convert-punctuation-safetensors.py`。載入順序：
//!
//! 1. `JC_PUNCT_WEIGHTS` 環境變數指定的檔案；
//! 2. 模型目錄的 `model.safetensors`；
//! 3. manifest `model` 欄位同名但副檔名換成 `.safetensors`。
//!
//! 找不到時回 [`ModelError::Inference`]，呼叫端退回原文。`ort` 後端保留，
//! 用 [`Backend`] 切換，預設仍是 ORT。

pub mod punct;

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU8, Ordering};

use candle_core::{DType, Device, Module, Tensor, D};
use candle_nn::{ops::layer_norm, ops::softmax, Embedding, Linear, VarBuilder};
use rayon::prelude::*;

use crate::error::{ModelError, Result};
use crate::manifest::ModelSpec;

/// candle 後端的名稱。
pub const BACKEND_NAME: &str = "candle";
/// safetensors 權重檔的預設檔名。
pub const DEFAULT_WEIGHTS_FILE: &str = "model.safetensors";
/// 覆寫權重路徑的環境變數。
pub const WEIGHTS_ENV: &str = "JC_PUNCT_WEIGHTS";
/// 覆寫預設後端的環境變數；值為 `ort` 或 `candle`。
pub const BACKEND_ENV: &str = "JC_PUNCT_BACKEND";
/// LayerNorm 的 epsilon，與 PyTorch `nn.LayerNorm` 預設值一致。
const LAYER_NORM_EPS: f32 = 1e-5;
/// 被遮罩的 attention key 加上這個負值；`exp` 後為 0，避免 `-inf` 產生 NaN。
const MASK_PENALTY: f32 = -1e9;

/// 推論後端。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// 現行 ONNX Runtime 路徑。
    Ort,
    /// candle 路徑。
    Candle,
}

impl Backend {
    /// 後端名稱，與 `--backend` 的值一致。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ort => "ort",
            Self::Candle => "candle",
        }
    }
}

impl Default for Backend {
    /// 預設仍走 ONNX Runtime；candle 由 `--backend candle` 明確要求。
    fn default() -> Self {
        Self::Ort
    }
}

impl FromStr for Backend {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "ort" => Ok(Self::Ort),
            "candle" => Ok(Self::Candle),
            other => Err(format!("後端只接受 ort 或 candle，實際為 {other}")),
        }
    }
}

/// 尚未指定時的值。
const UNSET: u8 = 2;
static DEFAULT_BACKEND: AtomicU8 = AtomicU8::new(UNSET);

/// 設定本行程的預設推論後端。
///
/// 這是行程層級的設定，讓 `jc-core` 不必為後端選擇改介面：`--backend` 在
/// 載入模型前呼叫一次。明確傳 [`Backend`] 的 `load_with_backend` 不受影響。
pub fn set_default_backend(backend: Backend) {
    let value = match backend {
        Backend::Ort => 0,
        Backend::Candle => 1,
    };
    DEFAULT_BACKEND.store(value, Ordering::Relaxed);
}

/// 目前的預設後端。
///
/// 沒設定過時讀 [`BACKEND_ENV`]，預設 [`Backend::Ort`]。
pub fn default_backend() -> Backend {
    match DEFAULT_BACKEND.load(Ordering::Relaxed) {
        0 => Backend::Ort,
        1 => Backend::Candle,
        _ => std::env::var(BACKEND_ENV)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(Backend::Ort),
    }
}

/// encoder 架構參數。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandleConfig {
    /// 詞表大小。
    pub vocab_size: usize,
    /// 隱藏層寬度。
    pub d_model: usize,
    /// encoder 層數。
    pub num_layers: usize,
    /// attention head 數。
    pub num_heads: usize,
    /// FFN 寬度。
    pub ffn_dim: usize,
    /// 位置嵌入的列數。
    pub max_positions: usize,
}

impl CandleConfig {
    /// 標點模型與 reranker 共用的架構：D512、8 層、8 heads、FFN 1024。
    pub fn for_punct() -> Self {
        Self {
            vocab_size: 32768,
            d_model: 512,
            num_layers: 8,
            num_heads: 8,
            ffn_dim: 1024,
            max_positions: 512,
        }
    }

    fn head_dim(&self) -> std::result::Result<usize, String> {
        if !self.d_model.is_multiple_of(self.num_heads) {
            return Err(format!(
                "d_model {} 不能被 num_heads {} 整除",
                self.d_model, self.num_heads
            ));
        }
        Ok(self.d_model / self.num_heads)
    }
}

/// model 目錄裡實際要載入的 safetensors 路徑。
///
/// 依序看 [`WEIGHTS_ENV`]、`model.safetensors`、`<model 檔名>.safetensors`。
/// 都不存在時回 `None`。
pub fn weights_path(dir: &Path, spec: &ModelSpec) -> Option<PathBuf> {
    if let Some(value) = std::env::var_os(WEIGHTS_ENV) {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Some(path);
        }
    }
    let default = dir.join(DEFAULT_WEIGHTS_FILE);
    if default.is_file() {
        return Some(default);
    }
    let stem = Path::new(spec.model_file())
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())?;
    let alternative = dir.join(format!("{stem}.safetensors"));
    alternative.is_file().then_some(alternative)
}

/// pre-LN transformer encoder；reranker 與標點模型共用。
pub(crate) struct Encoder {
    token_embedding: Embedding,
    position_embedding: Embedding,
    embedding_norm_weight: Tensor,
    embedding_norm_bias: Tensor,
    layers: Vec<Layer>,
    final_norm_weight: Tensor,
    final_norm_bias: Tensor,
}

impl Encoder {
    pub(crate) fn load(vb: &VarBuilder, config: &CandleConfig) -> Result<Self> {
        let d = config.d_model;
        let token_embedding = Embedding::new(
            vb.get((config.vocab_size, d), "token_embedding.weight")
                .map_err(inference)?,
            d,
        );
        let position_embedding = Embedding::new(
            vb.get((config.max_positions, d), "position_embedding.weight")
                .map_err(inference)?,
            d,
        );
        let layers = (0..config.num_layers)
            .map(|index| Layer::load(vb, &format!("encoder.layers.{index}"), config))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            token_embedding,
            position_embedding,
            embedding_norm_weight: vb.get(d, "embedding_norm.weight").map_err(inference)?,
            embedding_norm_bias: vb.get(d, "embedding_norm.bias").map_err(inference)?,
            layers,
            final_norm_weight: vb.get(d, "encoder.norm.weight").map_err(inference)?,
            final_norm_bias: vb.get(d, "encoder.norm.bias").map_err(inference)?,
        })
    }

    /// 完整 forward；`positions` 與 `ids` 同形，`mask` 是
    /// `(rows, 1, 1, width)` 的加成遮罩，`None` 代表全部可 attend。
    fn forward(
        &self,
        ids: &Tensor,
        positions: &Tensor,
        mask: Option<&Tensor>,
        config: &CandleConfig,
    ) -> Result<Tensor> {
        let tokens = self.token_embedding.forward(ids).map_err(inference)?;
        let (rows, width) = positions.dims2().map_err(inference)?;
        let positions = self
            .position_embedding
            .forward(&positions.reshape(rows * width).map_err(inference)?)
            .and_then(|t| t.reshape((rows, width, config.d_model)))
            .map_err(inference)?;
        let mut hidden = tokens.broadcast_add(&positions).map_err(inference)?;
        hidden = layer_norm(
            &hidden,
            &self.embedding_norm_weight,
            &self.embedding_norm_bias,
            LAYER_NORM_EPS,
        )
        .map_err(inference)?;
        for layer in &self.layers {
            hidden = layer.forward(&hidden, mask, config)?;
        }
        layer_norm(
            &hidden,
            &self.final_norm_weight,
            &self.final_norm_bias,
            LAYER_NORM_EPS,
        )
        .map_err(inference)
    }
}

struct Layer {
    attn_in: Linear,
    attn_out: Linear,
    linear1: Linear,
    linear2: Linear,
    norm1_weight: Tensor,
    norm1_bias: Tensor,
    norm2_weight: Tensor,
    norm2_bias: Tensor,
}

impl Layer {
    fn load(vb: &VarBuilder, prefix: &str, config: &CandleConfig) -> Result<Self> {
        let (d, ffn) = (config.d_model, config.ffn_dim);
        let attn_in = Linear::new(
            vb.get((3 * d, d), &format!("{prefix}.self_attn.in_proj_weight"))
                .map_err(inference)?,
            Some(
                vb.get(3 * d, &format!("{prefix}.self_attn.in_proj_bias"))
                    .map_err(inference)?,
            ),
        );
        let attn_out = Linear::new(
            vb.get((d, d), &format!("{prefix}.self_attn.out_proj.weight"))
                .map_err(inference)?,
            Some(
                vb.get(d, &format!("{prefix}.self_attn.out_proj.bias"))
                    .map_err(inference)?,
            ),
        );
        let linear1 = Linear::new(
            vb.get((ffn, d), &format!("{prefix}.linear1.weight"))
                .map_err(inference)?,
            Some(
                vb.get(ffn, &format!("{prefix}.linear1.bias"))
                    .map_err(inference)?,
            ),
        );
        let linear2 = Linear::new(
            vb.get((d, ffn), &format!("{prefix}.linear2.weight"))
                .map_err(inference)?,
            Some(
                vb.get(d, &format!("{prefix}.linear2.bias"))
                    .map_err(inference)?,
            ),
        );
        Ok(Self {
            attn_in,
            attn_out,
            linear1,
            linear2,
            norm1_weight: vb
                .get(d, &format!("{prefix}.norm1.weight"))
                .map_err(inference)?,
            norm1_bias: vb
                .get(d, &format!("{prefix}.norm1.bias"))
                .map_err(inference)?,
            norm2_weight: vb
                .get(d, &format!("{prefix}.norm2.weight"))
                .map_err(inference)?,
            norm2_bias: vb
                .get(d, &format!("{prefix}.norm2.bias"))
                .map_err(inference)?,
        })
    }

    fn forward(
        &self,
        hidden: &Tensor,
        mask: Option<&Tensor>,
        config: &CandleConfig,
    ) -> Result<Tensor> {
        let attended = self.attention(hidden, mask, config)?;
        let hidden = (hidden + attended).map_err(inference)?;
        let normed = self.norm2(&hidden)?;
        let ffn = linear_chunked(&normed, &self.linear1)?;
        let ffn = ffn.gelu_erf().map_err(inference)?;
        let ffn = linear_chunked(&ffn, &self.linear2)?;
        (hidden + ffn).map_err(inference)
    }

    fn norm2(&self, hidden: &Tensor) -> Result<Tensor> {
        layer_norm(hidden, &self.norm2_weight, &self.norm2_bias, LAYER_NORM_EPS).map_err(inference)
    }

    fn attention(
        &self,
        hidden: &Tensor,
        mask: Option<&Tensor>,
        config: &CandleConfig,
    ) -> Result<Tensor> {
        let (batch, seq, _) = hidden.dims3().map_err(inference)?;
        let head_dim = config.head_dim().map_err(ModelError::Manifest)?;
        let normed = layer_norm(hidden, &self.norm1_weight, &self.norm1_bias, LAYER_NORM_EPS)
            .map_err(inference)?;
        let projected = linear_chunked(&normed, &self.attn_in)?;
        let parts = projected.chunk(3, D::Minus1).map_err(inference)?;
        let shape = (batch, seq, config.num_heads, head_dim);
        let query = parts[0]
            .reshape(shape)
            .and_then(|t| t.transpose(1, 2))
            .and_then(|t| t.contiguous())
            .map_err(inference)?;
        let key = parts[1]
            .reshape(shape)
            .and_then(|t| t.transpose(1, 2))
            .and_then(|t| t.contiguous())
            .map_err(inference)?;
        let value = parts[2]
            .reshape(shape)
            .and_then(|t| t.transpose(1, 2))
            .and_then(|t| t.contiguous())
            .map_err(inference)?;
        let scale = 1.0 / (head_dim as f64).sqrt();
        let mut scores = query
            .matmul(&key.transpose(2, 3).map_err(inference)?)
            .and_then(|t| t.affine(scale, 0.0))
            .map_err(inference)?;
        if let Some(mask) = mask {
            scores = scores.broadcast_add(mask).map_err(inference)?;
        }
        let scores = softmax(&scores, D::Minus1).map_err(inference)?;
        let context = scores
            .matmul(&value)
            .and_then(|t| t.transpose(1, 2))
            .and_then(|t| t.contiguous())
            .and_then(|t| t.reshape((batch, seq, config.d_model)))
            .map_err(inference)?;
        linear_chunked(&context, &self.attn_out)
    }
}

/// 依輸出維度把 linear 切給 rayon 平行算。
///
/// candle 的 CPU gemm 一次 forward 只有 batch 維度可平行，IME 的 batch 很小
/// 時用不滿核心。把權重沿輸出維度切開後，每個 chunk 仍是同一個 gemm、每個
/// 元素仍是同一條 k 軸的和，數值與 [`Linear::forward`] 相同。
pub(crate) fn linear_chunked(x: &Tensor, layer: &Linear) -> Result<Tensor> {
    let Some(bias) = layer.bias() else {
        return layer.forward(x).map_err(inference);
    };
    let (out, _) = layer.weight().dims2().map_err(inference)?;
    let dims = x.dims().to_vec();
    let Some(&k) = dims.last() else {
        return layer.forward(x).map_err(inference);
    };
    let rows = x.elem_count() / k;
    // 每個 chunk 至少 64 個輸出，避免 gemm 太碎。
    let chunks = rayon::current_num_threads().min(out / 64).max(1);
    // 短序列（例如 IME 送出時的 10–30 token）讓 gemm 自己平行就好；再切
    // chunk 只會讓每個 gemm 更碎。長視窗時 candle 的 gemm 一次只平行
    // batch 維度，才需要沿輸出維度切分。
    if chunks <= 1 || rows < 64 {
        return layer.forward(x).map_err(inference);
    }
    let flat = x.reshape((rows, k)).map_err(inference)?;
    let weight = layer.weight();
    let parts: std::result::Result<Vec<Tensor>, candle_core::Error> = (0..chunks)
        .into_par_iter()
        .map(|index| {
            let start = index * out / chunks;
            let len = (index + 1) * out / chunks - start;
            let w = weight.narrow(0, start, len)?;
            let b = bias.narrow(0, start, len)?;
            flat.matmul(&w.t()?)?.broadcast_add(&b)
        })
        .collect();
    let parts = parts.map_err(inference)?;
    let mut out_dims = dims;
    if let Some(last) = out_dims.last_mut() {
        *last = out;
    }
    Tensor::cat(&parts, 1)
        .and_then(|t| t.reshape(out_dims))
        .map_err(inference)
}

/// candle 的錯誤轉成模型層錯誤。
fn inference(err: candle_core::Error) -> ModelError {
    ModelError::Inference(format!("candle：{err}"))
}

/// 把 padding mask 轉成 `(rows, 1, 1, width)` 的加成遮罩。
///
/// `None` 代表全部是真 token，可以完全跳過遮罩加法。
pub(crate) fn additive_mask(mask: &[i64], rows: usize, width: usize) -> Option<Tensor> {
    if mask[..rows * width].iter().all(|value| *value != 0) {
        return None;
    }
    let mut values = vec![MASK_PENALTY; rows * width];
    for (index, value) in mask[..rows * width].iter().enumerate() {
        if *value != 0 {
            values[index] = 0.0;
        }
    }
    Tensor::from_vec(values, (rows, 1, 1, width), &Device::Cpu).ok()
}

/// 載入 fp32 safetensors 的共用入口。
pub(crate) fn load_weights(weights: &Path) -> Result<(Device, VarBuilder<'static>)> {
    // VarBuilder 內含 mmap 的 tensor 資料，與 `weights` 的 lifetime 無關；
    // 這裡延長成 'static 讓呼叫端能把 encoder 與分類頭一起載入。
    let device = Device::Cpu;
    // SAFETY: 權重檔在行程生命週期內不會被改寫；mmap 只讀取。
    let builder: VarBuilder<'static> = unsafe {
        VarBuilder::from_mmaped_safetensors(&[weights.to_path_buf()], DType::F32, &device)
    }
    .map_err(inference)?;
    Ok((device, builder))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_names_roundtrip() {
        for backend in [Backend::Ort, Backend::Candle] {
            assert_eq!(backend.as_str().parse::<Backend>(), Ok(backend));
        }
        assert!("tensorrt".parse::<Backend>().is_err());
    }

    #[test]
    fn config_matches_the_checkpoint() {
        let config = CandleConfig::for_punct();
        assert_eq!(config.d_model, 512);
        assert_eq!(config.num_layers, 8);
        assert_eq!(config.head_dim(), Ok(64));
        assert_eq!(config.max_positions, 512);
    }

    #[test]
    fn weights_path_prefers_the_default_file() {
        let dir = std::env::temp_dir().join(format!("jc-candle-punct-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(weights_path(&dir, &ModelSpec::default()), None);
        std::fs::write(dir.join(DEFAULT_WEIGHTS_FILE), b"x").unwrap();
        assert_eq!(
            weights_path(&dir, &ModelSpec::default()),
            Some(dir.join(DEFAULT_WEIGHTS_FILE))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn additive_mask_is_skipped_when_every_token_is_real() {
        assert!(additive_mask(&[1, 1, 1, 1], 1, 4).is_none());
        let mask = additive_mask(&[1, 0, 1, 0], 1, 4).unwrap();
        let values = mask.flatten_all().unwrap().to_vec1::<f32>().unwrap();
        assert_eq!(values, vec![0.0, MASK_PENALTY, 0.0, MASK_PENALTY]);
    }
}
