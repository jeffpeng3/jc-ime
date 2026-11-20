//! candle 版日文 reranker 推論。
//!
//! 這個模組是 ORT 退場的試金石：同一個 checkpoint、同一組 token id、同一份
//! 部署佈局，只把 forward 換成 candle。模型是 pre-LN transformer encoder：
//!
//! ```text
//! <bos> left(60) <eos> candidate <eos> right(60)
//!  0     1..60    61      62       63     64..123
//! ```
//!
//! 絕對位置照訓練 checkpoint 的 right-anchored 佈局：最近的左文 token 用
//! 位置 120，候選用 122，右文用 124..183。序列長度 124，池化 slot 62。
//! 這些數字都從 [`ModelSpec`] 推導，不是寫死的常數；換模型時只要
//! `seq_len = 2 * context_tokens + 4` 且 `candidate_slot = context_tokens + 2`
//! 就成立，否則載入時就報錯。
//!
//! 同一個 encoder 只服務 reranker 的部署佈局；位置改成 `arange(seq)` 的
//! 標點 forward 已於 2026-11-20 移除。
//!
//! 權重來自 PyTorch checkpoint 轉出的 safetensors，轉檔腳本見
//! `scripts/convert-reranker-safetensors.py`。載入順序：
//!
//! 1. `JC_RERANK_WEIGHTS` 環境變數指定的檔案；
//! 2. 模型目錄的 `model.safetensors`；
//! 3. manifest `model` 欄位同名但副檔名換成 `.safetensors`。
//!
//! 模型檔不進 git；找不到時回 [`ModelError::Candle`]，呼叫端退回統計排序。
//!
//! 量化權重是同一條 forward 的另一種權重格式：[`QuantReranker`] 從 GGUF
//! 讀 ggml 量化區塊，載入後不還原成 fp32。檔案放
//! `models/quant/candle/<model>/<dtype>/model.gguf`，`.gguf` 副檔名會自動走
//! 量化路徑。

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU8, Ordering};

use candle_core::{DType, Device, Module, Tensor, D};
use candle_nn::{ops::layer_norm, ops::softmax, Embedding, Linear, VarBuilder};
use rayon::prelude::*;

use crate::error::{ModelError, Result};
use crate::layout::PackedBatch;
use crate::manifest::ModelSpec;

pub mod jinen;
pub mod quant;

pub use quant::{
    export_gguf, export_gguf_with_embedding, EmbeddingDtype, QuantDtype, QuantReranker,
    DEFAULT_QUANT_FILE,
};

/// candle 後端的名稱。
pub const BACKEND_NAME: &str = "candle";
/// safetensors 權重檔的預設檔名。
pub const DEFAULT_WEIGHTS_FILE: &str = "model.safetensors";
/// 覆寫 reranker 權重路徑的環境變數。
pub const WEIGHTS_ENV: &str = "JC_RERANK_WEIGHTS";
/// 覆寫 reranker 預設後端的環境變數；值為 `ort` 或 `candle`。
pub const BACKEND_ENV: &str = "JC_RERANK_BACKEND";
/// LayerNorm 的 epsilon，與 PyTorch `nn.LayerNorm` 預設值一致。
const LAYER_NORM_EPS: f32 = 1e-5;
/// 位置嵌入的列數；訓練設定 `max_positions = 512`。
const MAX_POSITIONS: usize = 512;
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
    /// 預設仍走 ONNX Runtime；candle 是本輪的試金石。
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
/// 載入模型前呼叫一次，之後 [`crate::Reranker::load_with_spec`] 都照這個值。
/// 明確傳 [`Backend`] 的 `load_with_backend` 不受影響。
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

/// 架構參數；與 `oc_training.model.reranker.RerankerConfig` 對應。
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
    /// 池化 slot，等於 `candidate_slot`。
    pub pool_slot: usize,
    /// 位置嵌入的列數；兩個模型都是 512。
    pub max_positions: usize,
}

impl CandleConfig {
    /// reranker-extra 的架構：D512、8 層、8 heads、FFN 1024。
    pub fn for_spec(spec: &ModelSpec) -> Self {
        Self {
            vocab_size: 32768,
            d_model: 512,
            num_layers: 8,
            num_heads: 8,
            ffn_dim: 1024,
            pool_slot: spec.candidate_slot(),
            max_positions: MAX_POSITIONS,
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

/// 部署佈局的絕對位置表。
///
/// `[0, 61..=120, 121, 122, 123, 124..=183]`，長度
/// `2 * context_tokens + 4`。與 `oc_training.eval.deploy.DeploymentLayout`
/// 的 `position_ids()` 相同。
pub fn position_ids(spec: &ModelSpec) -> std::result::Result<Vec<u32>, String> {
    let seq = spec.seq_len();
    let slot = spec.candidate_slot();
    let context = spec.context_tokens();
    if seq != 2 * context + 4 || slot != context + 2 {
        return Err(format!(
            "candle 只支援 right-anchored 佈局：seq_len 應為 {}、candidate_slot 應為 {}，\
             實際為 {seq}／{slot}",
            2 * context + 4,
            context + 2
        ));
    }
    // 訓練 checkpoint 的候選位置；部署時縮短序列但不移動絕對位置。
    let candidate = slot + context;
    let mut positions = Vec::with_capacity(seq);
    positions.push(0u32);
    for position in (candidate - context - 1)..(candidate - 1) {
        positions.push(position as u32);
    }
    for position in (candidate - 1)..=(candidate + 1) {
        positions.push(position as u32);
    }
    for position in (candidate + 2)..(candidate + 2 + context) {
        positions.push(position as u32);
    }
    debug_assert_eq!(positions.len(), seq);
    Ok(positions)
}

/// model 目錄裡實際要載入的 safetensors 路徑。
///
/// 依序看 [`WEIGHTS_ENV`]、`model.safetensors`、`<model 檔名>.safetensors`。
/// 都不存在時回 `None`。
pub fn weights_path(dir: &Path, spec: &ModelSpec) -> Option<PathBuf> {
    weights_path_with_env(dir, spec, WEIGHTS_ENV)
}

fn weights_path_with_env(dir: &Path, spec: &ModelSpec, env: &str) -> Option<PathBuf> {
    if let Some(value) = std::env::var_os(env) {
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

/// linear 層的最小介面。
///
/// fp32 的 [`Linear`] 與 [`quant::QuantLinear`] 都接到同一個 encoder；量化版
/// 的 forward 一樣輸出 fp32，差異只來自權重本身。
pub(super) trait LinearOp: Send + Sync {
    fn apply(&self, x: &Tensor) -> Result<Tensor>;
}

impl LinearOp for Linear {
    fn apply(&self, x: &Tensor) -> Result<Tensor> {
        linear_chunked(x, self)
    }
}

/// 詞嵌入查表；量化版直接把 quantized 權重當查表用。
pub(super) trait Lookup: Send + Sync {
    fn lookup(&self, ids: &Tensor) -> Result<Tensor>;
}

impl Lookup for Embedding {
    fn lookup(&self, ids: &Tensor) -> Result<Tensor> {
        Embedding::forward(self, ids).map_err(inference)
    }
}

struct Layer<L> {
    attn_in: L,
    attn_out: L,
    linear1: L,
    linear2: L,
    norm1_weight: Tensor,
    norm1_bias: Tensor,
    norm2_weight: Tensor,
    norm2_bias: Tensor,
}

impl Layer<Linear> {
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
        Ok(Self::new(
            attn_in,
            attn_out,
            linear1,
            linear2,
            (
                vb.get(d, &format!("{prefix}.norm1.weight"))
                    .map_err(inference)?,
                vb.get(d, &format!("{prefix}.norm1.bias"))
                    .map_err(inference)?,
            ),
            (
                vb.get(d, &format!("{prefix}.norm2.weight"))
                    .map_err(inference)?,
                vb.get(d, &format!("{prefix}.norm2.bias"))
                    .map_err(inference)?,
            ),
        ))
    }
}

impl<L: LinearOp> Layer<L> {
    fn new(
        attn_in: L,
        attn_out: L,
        linear1: L,
        linear2: L,
        norm1: (Tensor, Tensor),
        norm2: (Tensor, Tensor),
    ) -> Self {
        Self {
            attn_in,
            attn_out,
            linear1,
            linear2,
            norm1_weight: norm1.0,
            norm1_bias: norm1.1,
            norm2_weight: norm2.0,
            norm2_bias: norm2.1,
        }
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
        let ffn = self.linear1.apply(&normed)?;
        let ffn = ffn.gelu_erf().map_err(inference)?;
        let ffn = self.linear2.apply(&ffn)?;
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
        let projected = self.attn_in.apply(&normed)?;
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
        let scores = query
            .matmul(&key.transpose(2, 3).map_err(inference)?)
            .and_then(|t| t.affine(scale, 0.0))
            .map_err(inference)?;
        let scores = match mask {
            Some(mask) => scores.broadcast_add(mask).map_err(inference)?,
            None => scores,
        };
        let scores = softmax(&scores, D::Minus1).map_err(inference)?;
        let context = scores
            .matmul(&value)
            .and_then(|t| t.transpose(1, 2))
            .and_then(|t| t.contiguous())
            .and_then(|t| t.reshape((batch, seq, config.d_model)))
            .map_err(inference)?;
        self.attn_out.apply(&context)
    }
}

/// 不含量化細節的 encoder；權重來源由 `L` 決定。
struct Encoder<L> {
    config: CandleConfig,
    token_embedding: Box<dyn Lookup>,
    position_embedding: Embedding,
    embedding_norm_weight: Tensor,
    embedding_norm_bias: Tensor,
    layers: Vec<Layer<L>>,
    final_norm_weight: Tensor,
    final_norm_bias: Tensor,
    score_head: Option<Linear>,
}

impl<L: LinearOp> Encoder<L> {
    fn new(
        config: CandleConfig,
        token_embedding: Box<dyn Lookup>,
        position_embedding: Embedding,
        embedding_norm: (Tensor, Tensor),
        layers: Vec<Layer<L>>,
        final_norm: (Tensor, Tensor),
        score_head: Option<Linear>,
    ) -> Self {
        Self {
            config,
            token_embedding,
            position_embedding,
            embedding_norm_weight: embedding_norm.0,
            embedding_norm_bias: embedding_norm.1,
            layers,
            final_norm_weight: final_norm.0,
            final_norm_bias: final_norm.1,
            score_head,
        }
    }

    /// encoder forward；`ids` 與 `positions` 同形，`mask` 是
    /// `(rows, 1, 1, width)` 的加成遮罩，`None` 代表全部可 attend。
    fn forward(&self, ids: &Tensor, positions: &Tensor, mask: Option<&Tensor>) -> Result<Tensor> {
        let tokens = self.token_embedding.lookup(ids)?;
        let (rows, width) = positions.dims2().map_err(inference)?;
        let positions = self
            .position_embedding
            .forward(&positions.reshape(rows * width).map_err(inference)?)
            .and_then(|t| t.reshape((rows, width, self.config.d_model)))
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
            hidden = layer.forward(&hidden, mask, &self.config)?;
        }
        let hidden = layer_norm(
            &hidden,
            &self.final_norm_weight,
            &self.final_norm_bias,
            LAYER_NORM_EPS,
        )
        .map_err(inference)?;
        Ok(hidden)
    }

    /// 候選分數；`pooled` 是 `(rows, d_model)`。
    fn score_head(&self, pooled: &Tensor) -> Result<Vec<f32>> {
        let head = self
            .score_head
            .as_ref()
            .ok_or_else(|| ModelError::Candle("encoder 沒有 scalar head".into()))?;
        Linear::forward(head, pooled)
            .and_then(|t| t.to_dtype(DType::F32))
            .and_then(|t| t.flatten_all())
            .and_then(|t| t.to_vec1::<f32>())
            .map_err(inference)
    }
}

/// 已載入權重的 candle reranker。
///
/// `forward` 不做內部可變狀態，`&self` 可以跨執行緒共用；推論本身把 linear
/// 的輸出維度切給 rayon 平行，數值與單執行緒相同。
pub struct CandleReranker {
    config: CandleConfig,
    device: Device,
    encoder: Encoder<Linear>,
    positions: Vec<u32>,
}

impl CandleReranker {
    /// 從 safetensors 載入權重。
    pub fn load(weights: &Path, spec: &ModelSpec) -> Result<Self> {
        let config = CandleConfig::for_spec(spec);
        let positions = position_ids(spec).map_err(ModelError::Manifest)?;
        let device = Device::Cpu;
        // SAFETY: 權重檔在行程生命週期內不會被改寫；mmap 只讀取。
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights], DType::F32, &device)
                .map_err(inference)?
        };
        let d = config.d_model;
        let token_embedding = Embedding::new(
            vb.get((config.vocab_size, d), "token_embedding.weight")
                .map_err(inference)?,
            d,
        );
        let position_embedding = Embedding::new(
            vb.get((MAX_POSITIONS, d), "position_embedding.weight")
                .map_err(inference)?,
            d,
        );
        let layers = (0..config.num_layers)
            .map(|index| Layer::load(&vb, &format!("encoder.layers.{index}"), &config))
            .collect::<Result<Vec<_>>>()?;
        let score_head = Linear::new(
            vb.get((1, d), "score_head.weight").map_err(inference)?,
            Some(vb.get(1, "score_head.bias").map_err(inference)?),
        );
        let encoder = Encoder::new(
            config,
            Box::new(token_embedding),
            position_embedding,
            (
                vb.get(d, "embedding_norm.weight").map_err(inference)?,
                vb.get(d, "embedding_norm.bias").map_err(inference)?,
            ),
            layers,
            (
                vb.get(d, "encoder.norm.weight").map_err(inference)?,
                vb.get(d, "encoder.norm.bias").map_err(inference)?,
            ),
            Some(score_head),
        );
        Ok(Self {
            config,
            device,
            encoder,
            positions,
        })
    }

    /// 架構參數。
    pub fn config(&self) -> &CandleConfig {
        &self.config
    }

    /// 為已打包的 batch 打分。
    ///
    /// 先丟掉被遮罩的格子：key 被遮罩的 attention 權重是 0，對沒被遮罩的
    /// query 沒有貢獻，所以只算真 token 與直接算 124 格等價。位置嵌入仍用
    /// 原本的絕對位置，池化改成每個列自己的候選索引。序列長度 124 全滿時
    /// 就是原本的計算。
    pub fn score_packed(&self, batch: &PackedBatch) -> Result<Vec<f32>> {
        score_packed_with(&self.encoder, &self.positions, &self.device, batch)
    }
}

/// 兩個 reranker 共用的打分路徑；`positions` 是部署佈局的絕對位置表。
fn score_packed_with<L: LinearOp>(
    encoder: &Encoder<L>,
    positions_table: &[u32],
    device: &Device,
    batch: &PackedBatch,
) -> Result<Vec<f32>> {
    let config = &encoder.config;
    if batch.rows == 0 {
        return Ok(Vec::new());
    }
    if batch.seq != positions_table.len() {
        return Err(ModelError::Candle(format!(
            "序列長度 {} 與位置的 {} 不符",
            batch.seq,
            positions_table.len()
        )));
    }
    let rows = batch.rows;
    let mut kept: Vec<Vec<usize>> = Vec::with_capacity(rows);
    let mut pools = Vec::with_capacity(rows);
    for row in 0..rows {
        let base = row * batch.seq;
        let slots: Vec<usize> = (0..batch.seq)
            .filter(|&slot| batch.mask[base + slot] != 0)
            .collect();
        let pool = slots
            .iter()
            .position(|&slot| slot == config.pool_slot)
            .ok_or_else(|| {
                ModelError::Candle(format!(
                    "第 {row} 列的候選 slot {} 被遮罩",
                    config.pool_slot
                ))
            })?;
        pools.push(pool);
        kept.push(slots);
    }
    let width = kept.iter().map(Vec::len).max().unwrap_or(1);
    let mut ids = vec![0u32; rows * width];
    let mut positions = vec![0u32; rows * width];
    let mut mask = vec![MASK_PENALTY; rows * width];
    for (row, slots) in kept.iter().enumerate() {
        let base = row * width;
        for (index, &slot) in slots.iter().enumerate() {
            let id = batch.ids[row * batch.seq + slot];
            ids[base + index] = u32::try_from(id)
                .map_err(|_| ModelError::Candle(format!("token id {id} 不是合法的 u32")))?;
            positions[base + index] = positions_table[slot];
            mask[base + index] = 0.0;
        }
    }
    let ids = Tensor::from_vec(ids, (rows, width), device).map_err(inference)?;
    let positions = Tensor::from_vec(positions, (rows, width), device).map_err(inference)?;
    let mask = Tensor::from_vec(mask, (rows, 1, 1, width), device).map_err(inference)?;
    let hidden = encoder.forward(&ids, &positions, Some(&mask))?;
    let mut pooled = Vec::with_capacity(rows);
    for (row, &pool) in pools.iter().enumerate() {
        pooled.push(
            hidden
                .narrow(0, row, 1)
                .and_then(|t| t.narrow(1, pool, 1))
                .map_err(inference)?,
        );
    }
    let pooled = Tensor::cat(&pooled, 0).map_err(inference)?;
    encoder.score_head(&pooled)
}

/// 依輸出維度把 linear 切給 rayon 平行算。
///
/// candle 的 CPU gemm 是單執行緒，一次 forward 只有 batch 維度可平行，遠遠
/// 用不滿 20 個核心。把權重沿輸出維度切開後，每個 chunk 仍是同一個 gemm、
/// 每個元素仍是同一條 k 軸的和，所以數值與 [`Linear::forward`] 相同。
fn linear_chunked(x: &Tensor, layer: &Linear) -> Result<Tensor> {
    let Some(bias) = layer.bias() else {
        return Linear::forward(layer, x).map_err(inference);
    };
    let (out, _) = layer.weight().dims2().map_err(inference)?;
    let dims = x.dims().to_vec();
    let Some(&k) = dims.last() else {
        return Linear::forward(layer, x).map_err(inference);
    };
    let rows = x.elem_count() / k;
    // 每個 chunk 至少 64 個輸出，避免 gemm 太碎。
    let chunks = rayon::current_num_threads().min(out / 64).max(1);
    if chunks <= 1 {
        return Linear::forward(layer, x).map_err(inference);
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

fn inference(err: candle_core::Error) -> ModelError {
    ModelError::Candle(err.to_string())
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
    fn default_layout_positions_are_right_anchored() {
        let positions = position_ids(&ModelSpec::default()).unwrap();
        assert_eq!(positions.len(), 124);
        assert_eq!(positions[0], 0);
        // 左文 60 格：61..=120，最後一格緊鄰分隔符。
        assert_eq!(positions[1], 61);
        assert_eq!(positions[60], 120);
        // 分隔、候選、分隔。
        assert_eq!(&positions[61..64], &[121, 122, 123]);
        // 右文 124..=183。
        assert_eq!(positions[64], 124);
        assert_eq!(positions[123], 183);
    }

    #[test]
    fn non_deployment_layout_is_rejected() {
        let spec = ModelSpec {
            seq_len: Some(120),
            ..Default::default()
        };
        let err = position_ids(&spec).unwrap_err();
        assert!(err.contains("124"), "{err}");
    }

    #[test]
    fn weights_path_prefers_the_default_file() {
        let dir = std::env::temp_dir().join(format!("jc-candle-{}", std::process::id()));
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

    /// 固定輸入的 logits 必須與 ONNX fp32 一致；期望值由
    /// `reranker-extra/model.onnx` 在相同 id／mask 上跑出來。
    #[test]
    fn matches_the_onnx_fp32_reference() {
        let weights = jc_testdata::require_data!("models/reranker-extra/model.safetensors");
        let model = CandleReranker::load(&weights, &ModelSpec::default()).unwrap();
        assert_eq!(model.config().vocab_size, 32768);
        assert_eq!(model.config().d_model, 512);
        assert_eq!(model.config().num_layers, 8);
        let scores = model.score_packed(&reference_batch()).unwrap();
        assert_eq!(scores.len(), 1);
        let want = -0.802_918_6_f32;
        let diff = (scores[0] - want).abs();
        assert!(
            diff < 1e-3,
            "candle {} vs ONNX {want}，誤差 {diff}",
            scores[0]
        );
    }

    /// 半數格子留白，順便驗證 attention mask。
    fn reference_batch() -> PackedBatch {
        let seq = 124;
        let mut ids = vec![0i64; seq];
        let mut mask = vec![0i64; seq];
        ids[0] = 2;
        mask[0] = 1;
        for (offset, id) in (100..130).enumerate() {
            ids[31 + offset] = id;
            mask[31 + offset] = 1;
        }
        ids[61] = 3;
        mask[61] = 1;
        ids[62] = 500;
        mask[62] = 1;
        ids[63] = 3;
        mask[63] = 1;
        for (offset, id) in (200..230).enumerate() {
            ids[64 + offset] = id;
            mask[64 + offset] = 1;
        }
        PackedBatch {
            ids,
            mask,
            seq,
            rows: 1,
        }
    }
}
