//! GGUF 量化：匯出與推論。
//!
//! 這裡只處理權重格式，不動 forward：矩陣乘法的權重存成 ggml 的量化區塊
//! （`Q8_0`／`Q5K`／`Q4K`），載入後直接以量化形式算，不還原成 fp32。張量
//! 名稱沿用 safetensors 的名字，所以同一個 encoder 可以吃兩種權重。
//!
//! 量化等級與每個權重的實際位元數：
//!
//! | 等級 | ggml 型別 | 每 32 個權重 | 位元／權重 |
//! |---|---|---:|---:|
//! | `q8_0` | `Q8_0` | 34 B | 8.5 |
//! | `q5k` | `Q5K` | 176 B／256 個 | 5.5 |
//! | `q4k` | `Q4K` | 144 B／256 個 | 4.5 |
//!
//! 只量化矩陣乘法的權重（詞嵌入與四條 linear）；LayerNorm、bias、位置嵌入
//! 與 scalar head 保持 fp32，這些加起來不到 1 MB，量化只會白掉精度。
//!
//! 檔案放 `models/quant/candle/<model>/<dtype>/model.gguf`，原始
//! safetensors 不動。

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use candle_core::quantized::{gguf_file, GgmlDType, QMatMul, QTensor};
use candle_core::{Device, Module, Tensor};
use candle_nn::{Embedding, Linear};

use super::{inference, position_ids, score_packed_with, CandleConfig, Encoder, Layer};
use crate::error::{ModelError, Result};
use crate::layout::PackedBatch;
use crate::manifest::ModelSpec;

/// 量化權重檔的預設檔名。
pub const DEFAULT_QUANT_FILE: &str = "model.gguf";
/// ggml 量化區塊的張量命名空間；寫進 GGUF metadata 供事後辨識。
const METADATA_QUANT: &str = "jc.quant";
/// GGUF metadata 的架構名稱。
const METADATA_ARCH: &str = "jc.encoder";

/// 要產出的量化等級。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantDtype {
    /// 8.5 位元／權重，誤差最小。
    Q8_0,
    /// 5.5 位元／權重，k-quant。
    Q5K,
    /// 4.5 位元／權重，k-quant。
    Q4K,
}

impl QuantDtype {
    /// 全部等級，依檔案大小由大到小。
    pub fn all() -> [Self; 3] {
        [Self::Q8_0, Self::Q5K, Self::Q4K]
    }

    /// 目錄與 `--dtype` 用的名稱。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Q8_0 => "q8_0",
            Self::Q5K => "q5k",
            Self::Q4K => "q4k",
        }
    }

    /// 對應的 ggml 型別。
    pub fn ggml(self) -> GgmlDType {
        match self {
            Self::Q8_0 => GgmlDType::Q8_0,
            Self::Q5K => GgmlDType::Q5K,
            Self::Q4K => GgmlDType::Q4K,
        }
    }

    /// 每個權重的實際位元數（含區塊的 scale／min）。
    pub fn bits_per_weight(self) -> f64 {
        let dtype = self.ggml();
        (dtype.type_size() * 8) as f64 / dtype.block_size() as f64
    }
}

impl FromStr for QuantDtype {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "q8_0" | "Q8_0" => Ok(Self::Q8_0),
            "q5k" | "Q5K" | "q5_k" => Ok(Self::Q5K),
            "q4k" | "Q4K" | "q4_k" => Ok(Self::Q4K),
            other => Err(format!("量化等級只接受 q8_0、q5k、q4k，實際為 {other}")),
        }
    }
}

impl std::fmt::Display for QuantDtype {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 單一 GGUF 張量的匯出結果。
#[derive(Debug, Clone)]
pub struct ExportEntry {
    /// 張量名稱。
    pub name: String,
    /// 存進檔案的 ggml 型別名稱。
    pub dtype: String,
    /// 參數量。
    pub params: usize,
    /// 佔用的位元組數。
    pub bytes: usize,
}

/// 匯出報告。
#[derive(Debug, Clone)]
pub struct ExportReport {
    /// 輸出的 GGUF。
    pub path: PathBuf,
    /// 量化等級。
    pub quant: QuantDtype,
    /// 檔案大小（位元組）。
    pub file_bytes: u64,
    /// 每個張量的實際格式。
    pub entries: Vec<ExportEntry>,
}

impl ExportReport {
    /// 量化張量的位元數／權重，用實際位元組數回推。
    pub fn quantized_bits_per_weight(&self) -> f64 {
        let mut bytes = 0usize;
        let mut params = 0usize;
        for entry in &self.entries {
            if entry.dtype != "F32" {
                bytes += entry.bytes;
                params += entry.params;
            }
        }
        if params == 0 {
            return 0.0;
        }
        (bytes * 8) as f64 / params as f64
    }
}

/// 詞嵌入要不要用不同的等級；`None` 代表跟其他權重一樣。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingDtype {
    /// 不量化，留 fp32。
    F32,
    /// 用指定等級。
    Quant(QuantDtype),
}

/// 要量化的張量：四條 linear 與詞嵌入。
fn is_quantized(name: &str) -> bool {
    matches!(
        name,
        "token_embedding.weight"
            | "encoder.layers.0.self_attn.in_proj_weight"
            | "encoder.layers.0.self_attn.out_proj.weight"
            | "encoder.layers.0.linear1.weight"
            | "encoder.layers.0.linear2.weight"
    ) || (name.starts_with("encoder.layers.")
        && (name.ends_with(".self_attn.in_proj_weight")
            || name.ends_with(".self_attn.out_proj.weight")
            || name.ends_with(".linear1.weight")
            || name.ends_with(".linear2.weight")))
}

/// 接在 encoder 後面的分類頭；reranker 是 scalar，標點是 4 類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Head {
    /// reranker 的 `score_head`，輸出 1 維。
    Score,
    /// 標點模型的 `classifier`，輸出 4 類。
    Classifier,
}

impl Head {
    fn weights(self) -> [&'static str; 2] {
        match self {
            Self::Score => ["score_head.weight", "score_head.bias"],
            Self::Classifier => ["classifier.weight", "classifier.bias"],
        }
    }
}

/// 模型裡所有張量名稱，順序固定。
fn tensor_names(config: &CandleConfig, head: Head) -> Vec<String> {
    let mut names = vec![
        "token_embedding.weight".to_string(),
        "position_embedding.weight".to_string(),
        "embedding_norm.weight".to_string(),
        "embedding_norm.bias".to_string(),
        "encoder.norm.weight".to_string(),
        "encoder.norm.bias".to_string(),
    ];
    for name in head.weights() {
        names.push(name.to_string());
    }
    for index in 0..config.num_layers {
        let prefix = format!("encoder.layers.{index}");
        for suffix in [
            "self_attn.in_proj_weight",
            "self_attn.in_proj_bias",
            "self_attn.out_proj.weight",
            "self_attn.out_proj.bias",
            "linear1.weight",
            "linear1.bias",
            "linear2.weight",
            "linear2.bias",
            "norm1.weight",
            "norm1.bias",
            "norm2.weight",
            "norm2.bias",
        ] {
            names.push(format!("{prefix}.{suffix}"));
        }
    }
    names
}

/// 把 fp32 safetensors 轉成指定等級的 GGUF。
///
/// 原始檔只讀；輸出的父目錄不存在時自動建立。回傳每個張量實際使用的格式
/// 與位元組數，方便報告直接引用，不必再猜。
pub fn export_gguf(
    input: &Path,
    output: &Path,
    quant: QuantDtype,
    spec: &ModelSpec,
) -> Result<ExportReport> {
    export_with_config(
        input,
        output,
        quant,
        &CandleConfig::for_spec(spec),
        Head::Score,
    )
}

/// 把標點模型的 fp32 safetensors 轉成指定等級的 GGUF。
///
/// 標點模型與 reranker 共用 encoder，差別只有最後的分類頭名稱。
pub fn export_punct_gguf(input: &Path, output: &Path, quant: QuantDtype) -> Result<ExportReport> {
    export_with_config(
        input,
        output,
        quant,
        &CandleConfig::for_punct(),
        Head::Classifier,
    )
}

fn export_with_config(
    input: &Path,
    output: &Path,
    quant: QuantDtype,
    config: &CandleConfig,
    head: Head,
) -> Result<ExportReport> {
    export_with_embedding(input, output, quant, None, config, head)
}

/// 與 [`export_gguf`] 相同，但詞嵌入可以另外指定精度。
pub fn export_gguf_with_embedding(
    input: &Path,
    output: &Path,
    quant: QuantDtype,
    embedding: EmbeddingDtype,
    spec: &ModelSpec,
) -> Result<ExportReport> {
    export_with_embedding(
        input,
        output,
        quant,
        Some(embedding),
        &CandleConfig::for_spec(spec),
        Head::Score,
    )
}

fn export_with_embedding(
    input: &Path,
    output: &Path,
    quant: QuantDtype,
    embedding: Option<EmbeddingDtype>,
    config: &CandleConfig,
    head: Head,
) -> Result<ExportReport> {
    let device = Device::Cpu;
    let tensors = candle_core::safetensors::load(input, &device).map_err(inference)?;
    let mut entries = Vec::new();
    let mut quantized = Vec::new();
    for name in tensor_names(config, head) {
        let tensor = tensors
            .get(&name)
            .ok_or_else(|| ModelError::Candle(format!("{} 缺少張量 {name}", input.display())))?;
        let target = if name == "token_embedding.weight" {
            match embedding {
                None => quant.ggml(),
                Some(EmbeddingDtype::F32) => GgmlDType::F32,
                Some(EmbeddingDtype::Quant(dtype)) => dtype.ggml(),
            }
        } else if is_quantized(&name) {
            quant.ggml()
        } else {
            GgmlDType::F32
        };
        let qtensor = QTensor::quantize(tensor, target).map_err(inference)?;
        entries.push(ExportEntry {
            name: name.clone(),
            dtype: format!("{:?}", qtensor.dtype()),
            params: tensor.elem_count(),
            bytes: qtensor.storage_size_in_bytes(),
        });
        quantized.push((name, qtensor));
    }
    let metadata = [
        (
            "general.architecture",
            gguf_file::Value::String(METADATA_ARCH.into()),
        ),
        (
            "general.name",
            gguf_file::Value::String(
                output
                    .parent()
                    .and_then(Path::file_name)
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "jc-encoder".into()),
            ),
        ),
        (
            METADATA_QUANT,
            gguf_file::Value::String(quant.as_str().into()),
        ),
        (
            "jc.source",
            gguf_file::Value::String(input.display().to_string()),
        ),
    ];
    let metadata: Vec<(&str, &gguf_file::Value)> =
        metadata.iter().map(|(key, value)| (*key, value)).collect();
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = File::create(output)?;
    let mut writer = BufWriter::new(file);
    let borrowed: Vec<(&str, &QTensor)> = quantized
        .iter()
        .map(|(name, tensor)| (name.as_str(), tensor))
        .collect();
    gguf_file::write(&mut writer, &metadata, &borrowed).map_err(inference)?;
    let file_bytes = std::fs::metadata(output)?.len();
    Ok(ExportReport {
        path: output.to_path_buf(),
        quant,
        file_bytes,
        entries,
    })
}

/// 量化後的 linear 層。
///
/// 直接交給 candle 的 [`QMatMul`]：權重留在量化格式，輸入是 fp32。量化
/// matmul 的算術量與 fp32 相近，但 candle 的 CPU 實作沒有切給 rayon；在共用
/// 機器上分塊反而會被執行緒争用拖慢（實測 8 塊比單塊慢一個數量級），所以
/// 這裡不分塊。
struct QuantLinear {
    weight: QMatMul,
    bias: Tensor,
    out: usize,
}

impl QuantLinear {
    fn from_weight(weight: QTensor, bias: Tensor) -> Result<Self> {
        let (out, _) = weight.shape().dims2().map_err(inference)?;
        Ok(Self {
            weight: QMatMul::from_arc(Arc::new(weight)).map_err(inference)?,
            bias,
            out,
        })
    }
}

/// GGUF 的張量索引與檔案 handle。
///
/// 量化張量留在量化格式，只有 LayerNorm／bias 這種小張量才還原成 fp32。
struct GgufReader<'a> {
    content: &'a gguf_file::Content,
    file: File,
}

impl GgufReader<'_> {
    fn quantized(&mut self, name: &str) -> Result<QTensor> {
        self.content
            .tensor(&mut self.file, name, &Device::Cpu)
            .map_err(inference)
    }

    fn f32(&mut self, name: &str) -> Result<Tensor> {
        let device = Device::Cpu;
        self.quantized(name)?.dequantize(&device).map_err(inference)
    }

    fn linear(&mut self, weight: &str, bias: &str) -> Result<QuantLinear> {
        let weight = self.quantized(weight)?;
        let bias = self.f32(bias)?;
        QuantLinear::from_weight(weight, bias)
    }
}

impl super::LinearOp for QuantLinear {
    fn apply(&self, x: &Tensor) -> Result<Tensor> {
        let dims = x.dims().to_vec();
        let k = *dims
            .last()
            .ok_or_else(|| ModelError::Candle("linear 的輸入沒有維度".into()))?;
        let rows = x.elem_count() / k.max(1);
        let flat = x.reshape((rows, k)).map_err(inference)?;
        let out = self.weight.forward(&flat).map_err(inference)?;
        let mut out_dims = dims;
        if let Some(last) = out_dims.last_mut() {
            *last = self.out;
        }
        out.broadcast_add(&self.bias)
            .and_then(|t| t.reshape(out_dims))
            .map_err(inference)
    }
}

/// 量化權重的查表；詞嵌入直接用 ggml 的 get_rows。
struct QuantEmbedding {
    weight: QTensor,
    hidden: usize,
}

impl super::Lookup for QuantEmbedding {
    fn lookup(&self, ids: &Tensor) -> Result<Tensor> {
        let dims = ids.dims().to_vec();
        let tensor = self.weight.embedding(ids).map_err(inference)?;
        let mut out = dims;
        out.push(self.hidden);
        tensor.reshape(out).map_err(inference)
    }
}

/// 載入 GGUF 量化權重的 candle reranker。
pub struct QuantReranker {
    config: CandleConfig,
    device: Device,
    encoder: Encoder<QuantLinear>,
    positions: Vec<u32>,
    quant: QuantDtype,
}

impl QuantReranker {
    /// 從 GGUF 載入權重。
    ///
    /// 每條 linear 沿輸出維度切成目前執行緒數那麼多塊，平行算；切點在列
    /// 邊界，量化區塊以列為單位，切開不影響數值。
    pub fn load(weights: &Path, spec: &ModelSpec) -> Result<Self> {
        let config = CandleConfig::for_spec(spec);
        let positions = position_ids(spec).map_err(ModelError::Manifest)?;
        let device = Device::Cpu;
        let mut file = File::open(weights)?;
        let content = gguf_file::Content::read(&mut file).map_err(inference)?;
        let quant = read_quant(&content, weights)?;
        let mut reader = GgufReader {
            content: &content,
            file,
        };
        let encoder = load_encoder(&mut reader, &config)?;
        let score_head = Linear::new(
            reader.f32("score_head.weight")?,
            Some(reader.f32("score_head.bias")?),
        );
        Ok(Self {
            config,
            device,
            encoder: with_head(encoder, score_head),
            positions,
            quant,
        })
    }

    /// 架構參數。
    pub fn config(&self) -> &CandleConfig {
        &self.config
    }

    /// 量化等級。
    pub fn quant(&self) -> QuantDtype {
        self.quant
    }

    /// 與 fp32 版相同的打分路徑。
    pub fn score_packed(&self, batch: &PackedBatch) -> Result<Vec<f32>> {
        score_packed_with(&self.encoder, &self.positions, &self.device, batch)
    }
}

/// 載入 GGUF 量化權重的 candle 標點模型。
pub struct QuantPunctuation {
    config: CandleConfig,
    device: Device,
    encoder: Encoder<QuantLinear>,
    classifier: Linear,
    quant: QuantDtype,
}

impl QuantPunctuation {
    /// 從 GGUF 載入權重；分類頭留 fp32。
    pub fn load(weights: &Path) -> Result<Self> {
        let config = CandleConfig::for_punct();
        let device = Device::Cpu;
        let mut file = File::open(weights)?;
        let content = gguf_file::Content::read(&mut file).map_err(inference)?;
        let quant = read_quant(&content, weights)?;
        let mut reader = GgufReader {
            content: &content,
            file,
        };
        let encoder = load_encoder(&mut reader, &config)?;
        let classifier = Linear::new(
            reader.f32("classifier.weight")?,
            Some(reader.f32("classifier.bias")?),
        );
        Ok(Self {
            config,
            device,
            encoder,
            classifier,
            quant,
        })
    }

    /// 架構參數。
    pub fn config(&self) -> &CandleConfig {
        &self.config
    }

    /// 量化等級。
    pub fn quant(&self) -> QuantDtype {
        self.quant
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
            return Err(ModelError::Candle(format!(
                "candle：序列長度 {width} 超過 max_positions {}",
                self.config.max_positions
            )));
        }
        let mut ids = vec![0u32; rows * width];
        let mut mask = vec![0i64; rows * width];
        for (row, window) in windows.iter().enumerate() {
            for (column, id) in window.iter().enumerate() {
                ids[row * width + column] = u32::try_from(*id)
                    .map_err(|_| ModelError::Candle(format!("token id {id} 不是合法的 u32")))?;
                mask[row * width + column] = 1;
            }
        }
        let positions: Vec<u32> = (0..width as u32).cycle().take(rows * width).collect();
        let ids = Tensor::from_vec(ids, (rows, width), &self.device).map_err(inference)?;
        let positions =
            Tensor::from_vec(positions, (rows, width), &self.device).map_err(inference)?;
        let additive = super::additive_mask(&mask, rows, width);
        let hidden = self.encoder.forward(&ids, &positions, additive.as_ref())?;
        let logits = self
            .classifier
            .forward(&hidden)
            .and_then(|tensor| tensor.to_vec3::<f32>())
            .map_err(inference)?;
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

/// 讀 GGUF metadata 裡的量化等級。
fn read_quant(content: &gguf_file::Content, path: &Path) -> Result<QuantDtype> {
    content
        .metadata
        .get(METADATA_QUANT)
        .and_then(|value| match value {
            gguf_file::Value::String(value) => value.parse().ok(),
            _ => None,
        })
        .ok_or_else(|| {
            ModelError::Candle(format!(
                "{} 的 metadata 沒有 {METADATA_QUANT}",
                path.display()
            ))
        })
}

/// 從 GGUF 讀出 encoder 的全部權重；分類頭由呼叫端接上。
fn load_encoder(reader: &mut GgufReader, config: &CandleConfig) -> Result<Encoder<QuantLinear>> {
    let d = config.d_model;
    let token_embedding = QuantEmbedding {
        weight: reader.quantized("token_embedding.weight")?,
        hidden: d,
    };
    let position_embedding = Embedding::new(reader.f32("position_embedding.weight")?, d);
    let mut layers = Vec::with_capacity(config.num_layers);
    for index in 0..config.num_layers {
        let prefix = format!("encoder.layers.{index}");
        layers.push(Layer::new(
            reader.linear(
                &format!("{prefix}.self_attn.in_proj_weight"),
                &format!("{prefix}.self_attn.in_proj_bias"),
            )?,
            reader.linear(
                &format!("{prefix}.self_attn.out_proj.weight"),
                &format!("{prefix}.self_attn.out_proj.bias"),
            )?,
            reader.linear(
                &format!("{prefix}.linear1.weight"),
                &format!("{prefix}.linear1.bias"),
            )?,
            reader.linear(
                &format!("{prefix}.linear2.weight"),
                &format!("{prefix}.linear2.bias"),
            )?,
            (
                reader.f32(&format!("{prefix}.norm1.weight"))?,
                reader.f32(&format!("{prefix}.norm1.bias"))?,
            ),
            (
                reader.f32(&format!("{prefix}.norm2.weight"))?,
                reader.f32(&format!("{prefix}.norm2.bias"))?,
            ),
        ));
    }
    Ok(Encoder::new(
        *config,
        Box::new(token_embedding),
        position_embedding,
        (
            reader.f32("embedding_norm.weight")?,
            reader.f32("embedding_norm.bias")?,
        ),
        layers,
        (
            reader.f32("encoder.norm.weight")?,
            reader.f32("encoder.norm.bias")?,
        ),
        None,
    ))
}

/// 把 scalar head 接到 encoder 上；reranker 的 forward 會用到。
fn with_head(mut encoder: Encoder<QuantLinear>, head: Linear) -> Encoder<QuantLinear> {
    encoder.score_head = Some(head);
    encoder
}

/// 檔案是不是 GGUF。
pub fn is_gguf(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("gguf"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candle::{position_ids, CandleReranker};
    use crate::layout::PackedBatch;

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

    #[test]
    fn quant_names_roundtrip() {
        for quant in QuantDtype::all() {
            assert_eq!(quant.as_str().parse::<QuantDtype>(), Ok(quant));
            assert!(quant.bits_per_weight() > 4.0);
        }
        assert!(QuantDtype::Q8_0.bits_per_weight() > QuantDtype::Q4K.bits_per_weight());
        assert!("q3k".parse::<QuantDtype>().is_err());
    }

    #[test]
    fn only_matmul_weights_are_quantized() {
        assert!(is_quantized("token_embedding.weight"));
        assert!(is_quantized("encoder.layers.3.linear1.weight"));
        assert!(!is_quantized("encoder.layers.3.linear1.bias"));
        assert!(!is_quantized("encoder.layers.3.norm1.weight"));
        assert!(!is_quantized("score_head.weight"));
    }

    /// 標點模型的量化等級：載入、跑 forward，argmax 與 fp32 一致。
    #[test]
    fn punct_quantized_levels_load() {
        let weights =
            jc_testdata::require_data!("models/punctuation-public-core/model.safetensors");
        let fp32 = crate::candle::CandlePunctuation::load(&weights).unwrap();
        let windows = vec![vec![2i64, 100, 200, 300, 400, 500, 3]];
        let want = fp32.logits(&windows).unwrap();
        let dir = std::env::temp_dir().join(format!("jc-quant-punct-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for quant in QuantDtype::all() {
            let path = dir.join(format!("{quant}.gguf"));
            export_punct_gguf(&weights, &path, quant).unwrap();
            let model = QuantPunctuation::load(&path).unwrap();
            assert_eq!(model.quant(), quant);
            let got = model.logits(&windows).unwrap();
            for (row, (got, want)) in got[0].iter().zip(&want[0]).enumerate() {
                let argmax = |values: &[f32]| {
                    values
                        .iter()
                        .enumerate()
                        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                        .map(|(index, _)| index)
                };
                assert_eq!(argmax(got), argmax(want), "{quant} token {row}");
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 三種量化等級都要能載入並跑 forward，且 logits 與 fp32 差距有限。
    #[test]
    fn quantized_levels_stay_close_to_fp32() {
        let weights = jc_testdata::require_data!("models/reranker-extra/model.safetensors");
        let spec = ModelSpec::default();
        let fp32 = CandleReranker::load(&weights, &spec).unwrap();
        let want = fp32.score_packed(&reference_batch()).unwrap()[0];
        assert_eq!(position_ids(&spec).unwrap().len(), 124);
        let dir = std::env::temp_dir().join(format!("jc-quant-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // 門檻依實測差距訂：Q8_0 0.0127、Q5K 0.0228、Q4K 0.1036。
        let tolerance = [
            (QuantDtype::Q8_0, 2e-2),
            (QuantDtype::Q5K, 5e-2),
            (QuantDtype::Q4K, 1.5e-1),
        ];
        for (quant, limit) in tolerance {
            let path = dir.join(format!("{quant}.gguf"));
            let report = export_gguf(&weights, &path, quant, &spec).unwrap();
            assert!(report.file_bytes > 0);
            let model = QuantReranker::load(&path, &spec).unwrap();
            assert_eq!(model.quant(), quant);
            let got = model.score_packed(&reference_batch()).unwrap()[0];
            assert!(
                (got - want).abs() < limit,
                "{quant}: {got} vs fp32 {want}，差 {}",
                (got - want).abs()
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
