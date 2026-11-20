//! candle 版注音選字模型（`zuin-phonetic-v1`）。
//!
//! 這個模型不是標準 transformer encoder：輸入是九個張量
//! `ids, bases, tones, types, positions, mask, proj_pos, target_node,
//! target_ids`，輸出是每個候選列的 log-probability 總和。序列由三塊組成：
//!
//! ```text
//! [BOS] 前文(<=64 字) [SEP] | 共用音節(n-1) | 分支節點 + 候選前綴樹
//! ```
//!
//! 與 ORT 的 forward 逐節點對齊（輸出 `scores` 之前 404 個節點）：
//!
//! - 詞嵌入按 `types` 分派：`type == 1`（音節）用 `base + tone`，其餘用
//!   `char_embedding[ids]`，最後加 `type_embedding`。
//! - 8 層 pre-LN：`x = x + attn(norm1(x))`、`x = x + mlp(norm2(x))`。
//! - RoPE 是 GPT-NeoX 的 rotate-half 形式，只作用在 head 的前後半。
//! - attention 一律是 `[1, heads, seq, seq]`；遮罩用 `-1e9` 加成，與 ORT 的
//!   `-3.4e38` 等價（softmax 後都是 0）。
//! - 只對 `proj_pos` 的列做輸出頭：`dense -> gelu -> norm`，再乘上
//!   `char_embedding` 的全部列，取 `out_ids` 的 27,859 個位置做 log-softmax，
//!   最後依 `target_node`／`target_ids` 取值加總。
//!
//! 權重來源只有 ONNX。上游 `Funmula/0c-sequence-d512` 沒有 safetensors，
//! 因此 [`convert_onnx`] 直接讀 ONNX initializer 轉成 fp32 safetensors；
//! [`export_gguf`] 再把它轉成 Q8_0 等量化等級。兩者都只做格式轉換，不動數值，
//! 張量順序與 ONNX 節點一一對應。
//!
//! 量化版走同一條 forward：`L` 是 [`crate::candle::LinearOp`]，fp32 用
//! [`Fp32Dense`]，量化用 [`QuantDense`]；詞嵌入在量化路徑直接交給
//! [`QMatMul::embedding`]，同一份 `QTensor` 同時當輸入查表與輸出投影。

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use candle_core::quantized::{gguf_file, GgmlDType, QMatMul, QTensor};
use candle_core::{Device, Module, Tensor, D};
use candle_nn::ops::{layer_norm, log_softmax, softmax};
use rayon::prelude::*;

use super::quant::{ExportEntry, ExportReport, QuantDtype};
use super::{LinearOp, Lookup};
use crate::error::{ModelError, Result};

/// fp32 權重檔的預設檔名。
pub const DEFAULT_WEIGHTS_FILE: &str = "model.safetensors";
/// 量化權重檔的預設檔名。
pub const DEFAULT_QUANT_FILE: &str = "model.gguf";
/// 覆寫注音選字權重路徑的環境變數。
pub const WEIGHTS_ENV: &str = "JC_ZH_SELECT_WEIGHTS";
/// ggml 量化區塊的張量命名空間；與 `candle-quant` 的 reranker 檔一致。
const METADATA_QUANT: &str = "jc.quant";
/// GGUF metadata 的架構名稱。
const METADATA_ARCH: &str = "jc.zh-select";
/// LayerNorm 的 epsilon，與 ONNX 圖上的 `epsilon` 相同。
const LAYER_NORM_EPS: f32 = 1e-5;
/// 被遮罩的 attention key 加上這個負值；`exp` 後為 0。
const MASK_PENALTY: f32 = -1e9;
/// attention 分數的縮放，等於 `1 / sqrt(head_dim)`。
const ATTENTION_SCALE: f64 = 0.125;

/// 架構參數；預設值來自 `zh-select/manifest.json` 的 `spec`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZhConfig {
    /// 字元嵌入的列數，含特殊 token。
    pub vocab_size: usize,
    /// 注音 base 數。
    pub base_count: usize,
    /// 聲調數。
    pub tone_count: usize,
    /// 節點種類數。
    pub type_count: usize,
    /// 隱藏層寬度。
    pub hidden: usize,
    /// 層數。
    pub layers: usize,
    /// attention head 數。
    pub heads: usize,
    /// FFN 寬度。
    pub ffn: usize,
    /// 輸出投影實際使用的 token 數（`out_ids` 長度）。
    pub out_len: usize,
}

impl Default for ZhConfig {
    fn default() -> Self {
        Self {
            vocab_size: 27_865,
            base_count: 418,
            tone_count: 6,
            type_count: 3,
            hidden: 512,
            layers: 8,
            heads: 8,
            ffn: 768,
            out_len: 27_859,
        }
    }
}

impl ZhConfig {
    /// 每個 head 的維度。
    pub fn head_dim(&self) -> usize {
        self.hidden / self.heads
    }

    /// 輸出投影的權重列數；量化路徑用整張字元表，投影完才挑 `out_ids`。
    fn out_vocab(&self) -> usize {
        self.vocab_size
    }
}

/// 每層的 ONNX initializer 名稱，順序是 `q, k, v, out, gate+up, down`。
///
/// 名稱是 ONNX 匯出器產生的 `val_N`，沒有語意；形狀在轉檔時逐一驗證，錯了
/// 會在 [`convert_onnx`] 直接回錯，不會靜默錯位。
const LAYER_TENSORS: [[&str; 6]; 8] = [
    [
        "val_38", "val_45", "val_52", "val_118", "val_121", "val_130",
    ],
    [
        "val_133", "val_140", "val_147", "val_208", "val_211", "val_219",
    ],
    [
        "val_222", "val_229", "val_236", "val_297", "val_300", "val_308",
    ],
    [
        "val_311", "val_318", "val_325", "val_386", "val_389", "val_397",
    ],
    [
        "val_400", "val_407", "val_414", "val_475", "val_478", "val_486",
    ],
    [
        "val_489", "val_496", "val_503", "val_564", "val_567", "val_575",
    ],
    [
        "val_578", "val_585", "val_592", "val_653", "val_656", "val_664",
    ],
    [
        "val_667", "val_674", "val_681", "val_742", "val_745", "val_753",
    ],
];

/// 一次評分的輸入；欄位與 `jc_core::zh::select::Arrays` 同名同序。
#[derive(Debug, Clone, Copy)]
pub struct ZhInput<'a> {
    /// 序列長度。
    pub seq: usize,
    /// 字元 id 序列。
    pub ids: &'a [i64],
    /// 每個位置的注音 base id。
    pub bases: &'a [i64],
    /// 每個位置的聲調 id。
    pub tones: &'a [i64],
    /// 每個位置的節點種類。
    pub types: &'a [i64],
    /// 位置索引。
    pub positions: &'a [i64],
    /// `[seq, seq]` 的注意力遮罩，列優先；`true` 代表可 attend。
    pub mask: &'a [bool],
    /// 要投影的序列索引。
    pub proj_pos: &'a [i64],
    /// 每個候選列在 `proj_pos` 的索引，`[rows, cols]` 展開。
    pub target_node: &'a [i64],
    /// 每個候選列的目標字元位置（`out_ids` 的下標），`[rows, cols]` 展開。
    pub target_ids: &'a [i64],
    /// 候選列數。
    pub rows: usize,
    /// 每個列的候選字數。
    pub cols: usize,
}

/// 轉檔報告。
#[derive(Debug, Clone)]
pub struct ConvertReport {
    /// 輸出的 safetensors。
    pub path: PathBuf,
    /// 來源 ONNX。
    pub source: PathBuf,
    /// 張量數。
    pub tensors: usize,
    /// 參數量。
    pub params: usize,
    /// 檔案大小。
    pub bytes: u64,
}

/// 解析後的 ONNX 張量。
struct OnnxTensor<'a> {
    dims: Vec<usize>,
    data_type: i64,
    raw: &'a [u8],
    floats: Vec<f32>,
    ints: Vec<i64>,
}

impl OnnxTensor<'_> {
    fn is_float(&self) -> bool {
        self.data_type == 1
    }

    fn is_int64(&self) -> bool {
        self.data_type == 7
    }

    fn f32s(&self) -> Result<Vec<f32>> {
        if !self.is_float() {
            return Err(ModelError::Candle(format!(
                "張量不是 float32（data_type {}）",
                self.data_type
            )));
        }
        if !self.floats.is_empty() {
            return Ok(self.floats.clone());
        }
        let (chunks, rest) = self.raw.as_chunks::<4>();
        if !rest.is_empty() {
            return Err(ModelError::Candle("raw_data 長度不是 4 的倍數".into()));
        }
        Ok(chunks
            .iter()
            .map(|chunk| f32::from_le_bytes(*chunk))
            .collect())
    }

    fn i64s(&self) -> Result<Vec<i64>> {
        if !self.is_int64() {
            return Err(ModelError::Candle(format!(
                "張量不是 int64（data_type {}）",
                self.data_type
            )));
        }
        if !self.ints.is_empty() {
            return Ok(self.ints.clone());
        }
        let (chunks, rest) = self.raw.as_chunks::<8>();
        if !rest.is_empty() {
            return Err(ModelError::Candle("raw_data 長度不是 8 的倍數".into()));
        }
        Ok(chunks
            .iter()
            .map(|chunk| i64::from_le_bytes(*chunk))
            .collect())
    }
}

/// 最小 protobuf 讀取器；只解 ONNX 的 `ModelProto -> GraphProto ->
/// TensorProto` 三層。
struct ProtoReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ProtoReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn varint(&mut self) -> Result<u64> {
        let mut value = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = *self
                .data
                .get(self.pos)
                .ok_or_else(|| ModelError::Candle("protobuf 提前結束".into()))?;
            self.pos += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
            if shift >= 64 {
                return Err(ModelError::Candle("protobuf varint 過長".into()));
            }
        }
    }

    fn bytes(&mut self) -> Result<&'a [u8]> {
        let len = usize::try_from(self.varint()?)
            .map_err(|_| ModelError::Candle("protobuf 長度超出範圍".into()))?;
        let end = self
            .pos
            .checked_add(len)
            .filter(|end| *end <= self.data.len())
            .ok_or_else(|| ModelError::Candle("protobuf 長度越界".into()))?;
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn skip(&mut self, wire: u64) -> Result<()> {
        match wire {
            0 => {
                self.varint()?;
            }
            1 => self.pos += 8,
            2 => {
                self.bytes()?;
            }
            5 => self.pos += 4,
            other => {
                return Err(ModelError::Candle(format!(
                    "不支援的 protobuf wire type {other}"
                )));
            }
        }
        if self.pos > self.data.len() {
            return Err(ModelError::Candle("protobuf 越界".into()));
        }
        Ok(())
    }
}

/// 從 ONNX 位元組讀出所有 initializer。
fn read_initializers(data: &[u8]) -> Result<HashMap<String, OnnxTensor<'_>>> {
    let mut reader = ProtoReader::new(data);
    let mut graph = None;
    while reader.pos < reader.data.len() {
        let key = reader.varint()?;
        let (field, wire) = ((key >> 3) as u32, key & 7);
        if field == 7 && wire == 2 {
            graph = Some(reader.bytes()?);
        } else {
            reader.skip(wire)?;
        }
    }
    let graph = graph.ok_or_else(|| ModelError::Candle("ONNX 沒有 graph".into()))?;
    let mut reader = ProtoReader::new(graph);
    let mut tensors = HashMap::new();
    while reader.pos < reader.data.len() {
        let key = reader.varint()?;
        let (field, wire) = ((key >> 3) as u32, key & 7);
        if field == 5 && wire == 2 {
            let (name, tensor) = read_tensor(reader.bytes()?)?;
            tensors.insert(name, tensor);
        } else {
            reader.skip(wire)?;
        }
    }
    Ok(tensors)
}

fn read_tensor(data: &[u8]) -> Result<(String, OnnxTensor<'_>)> {
    let mut reader = ProtoReader::new(data);
    let mut name = String::new();
    let mut dims = Vec::new();
    let mut data_type = 0i64;
    let mut raw = &[][..];
    let mut floats = Vec::new();
    let mut ints = Vec::new();
    while reader.pos < reader.data.len() {
        let key = reader.varint()?;
        let (field, wire) = ((key >> 3) as u32, key & 7);
        match field {
            1 => {
                if wire == 2 {
                    let mut packed = ProtoReader::new(reader.bytes()?);
                    while packed.pos < packed.data.len() {
                        dims.push(packed.varint()? as usize);
                    }
                } else {
                    dims.push(reader.varint()? as usize);
                }
            }
            2 => data_type = reader.varint()? as i64,
            4 => {
                let bytes = reader.bytes()?;
                let (chunks, rest) = bytes.as_chunks::<4>();
                if !rest.is_empty() {
                    return Err(ModelError::Candle("float_data 長度不是 4 的倍數".into()));
                }
                floats.extend(chunks.iter().map(|c| f32::from_le_bytes(*c)));
            }
            7 => {
                if wire == 2 {
                    let mut packed = ProtoReader::new(reader.bytes()?);
                    while packed.pos < packed.data.len() {
                        ints.push(packed.varint()? as i64);
                    }
                } else {
                    ints.push(reader.varint()? as i64);
                }
            }
            8 => {
                name = String::from_utf8(reader.bytes()?.to_vec())
                    .map_err(|_| ModelError::Candle("張量名稱不是 UTF-8".into()))?;
            }
            9 => raw = reader.bytes()?,
            _ => reader.skip(wire)?,
        }
    }
    Ok((
        name,
        OnnxTensor {
            dims,
            data_type,
            raw,
            floats,
            ints,
        },
    ))
}

/// 轉置 `[rows, cols]` 的 fp32 張量。
fn transpose(data: &[f32], rows: usize, cols: usize) -> Result<Vec<f32>> {
    if data.len() != rows * cols {
        return Err(ModelError::Candle(format!(
            "轉置長度 {} 與 {}x{} 不符",
            data.len(),
            rows,
            cols
        )));
    }
    let mut out = vec![0.0f32; data.len()];
    for row in 0..rows {
        for col in 0..cols {
            out[col * rows + row] = data[row * cols + col];
        }
    }
    Ok(out)
}

fn expect_dims(text: &OnnxTensor<'_>, name: &str, want: &[usize]) -> Result<()> {
    if text.dims != want {
        return Err(ModelError::Candle(format!(
            "{name} 的形狀 {:?} 與預期的 {want:?} 不符",
            text.dims
        )));
    }
    Ok(())
}

/// 把 ONNX 的 initializer 轉成 candle 的 fp32 safetensors。
///
/// 名稱換成 forward 用的語意名稱，MatMul 的 `[in, out]` 權重轉成 PyTorch
/// 慣例的 `[out, in]`；q／k／v 與 gate／up 在轉檔時各自沿輸出維度合併，
/// 少兩次 matmul。轉完不改任何數值。
pub fn convert_onnx(input: &Path, output: &Path) -> Result<ConvertReport> {
    let config = ZhConfig::default();
    // ONNX 檔在 125 MB 左右，整檔讀進來比手刻 mmap 簡單；只做一次。
    let raw = std::fs::read(input)?;
    let tensors = read_initializers(&raw)?;
    let mut out: HashMap<String, Tensor> = HashMap::new();
    let device = Device::Cpu;

    let get = |name: &str| -> Result<&OnnxTensor<'_>> {
        tensors
            .get(name)
            .ok_or_else(|| ModelError::Candle(format!("ONNX 缺少張量 {name}")))
    };
    let tensor = |name: &str, dims: &[usize], data: Vec<f32>| -> Result<(String, Tensor)> {
        Ok((
            name.to_string(),
            Tensor::from_vec(data, dims.to_vec(), &device).map_err(super::inference)?,
        ))
    };

    // 字元表同時當輸入查表與輸出投影，形狀 [vocab, hidden]。
    let char_table = get("m.network.decoder.weight")?;
    expect_dims(
        char_table,
        "char_embedding",
        &[config.vocab_size, config.hidden],
    )?;
    let (name, value) = tensor(
        "char_embedding.weight",
        &[config.vocab_size, config.hidden],
        char_table.f32s()?,
    )?;
    out.insert(name, value);

    let base = get("m.base_embedding.weight")?;
    expect_dims(base, "base_embedding", &[config.base_count, config.hidden])?;
    let (name, value) = tensor(
        "base_embedding.weight",
        &[config.base_count, config.hidden],
        base.f32s()?,
    )?;
    out.insert(name, value);

    let tone = get("m.tone_embedding.weight")?;
    expect_dims(tone, "tone_embedding", &[config.tone_count, config.hidden])?;
    let (name, value) = tensor(
        "tone_embedding.weight",
        &[config.tone_count, config.hidden],
        tone.f32s()?,
    )?;
    out.insert(name, value);

    let types = get("m.type_embedding.weight")?;
    expect_dims(types, "type_embedding", &[config.type_count, config.hidden])?;
    let (name, value) = tensor(
        "type_embedding.weight",
        &[config.type_count, config.hidden],
        types.f32s()?,
    )?;
    out.insert(name, value);

    let norm = get("m.network.model.embeddings.norm.weight")?;
    expect_dims(norm, "embedding_norm", &[config.hidden])?;
    let (name, value) = tensor("embedding_norm.weight", &[config.hidden], norm.f32s()?)?;
    out.insert(name, value);

    let d = config.hidden;
    let ffn = config.ffn;
    for (index, names) in LAYER_TENSORS.iter().enumerate() {
        // 第 0 層的 attention norm 就是 `embeddings.norm`，已經寫成
        // `embedding_norm.weight`，forward 在進第一層前就套用過了。
        if index > 0 {
            let attn_norm = get(&format!("m.network.model.layers.{index}.attn_norm.weight"))?;
            expect_dims(attn_norm, "attn_norm", &[d])?;
            let (name, value) = tensor(
                &format!("layers.{index}.attn_norm.weight"),
                &[d],
                attn_norm.f32s()?,
            )?;
            out.insert(name, value);
        }

        let mlp_norm = get(&format!("m.network.model.layers.{index}.mlp_norm.weight"))?;
        expect_dims(mlp_norm, "mlp_norm", &[d])?;
        let (name, value) = tensor(
            &format!("layers.{index}.mlp_norm.weight"),
            &[d],
            mlp_norm.f32s()?,
        )?;
        out.insert(name, value);

        // q、k、v 各自是 [d, d] 的 MatMul 權重，合併成 [3d, d]。
        let mut in_proj = Vec::with_capacity(3 * d * d);
        for weight in &names[..3] {
            let tensor = get(weight)?;
            expect_dims(tensor, weight, &[d, d])?;
            in_proj.extend(transpose(&tensor.f32s()?, d, d)?);
        }
        let (name, value) = tensor(
            &format!("layers.{index}.in_proj.weight"),
            &[3 * d, d],
            in_proj,
        )?;
        out.insert(name, value);

        let attn_out = get(names[3])?;
        expect_dims(attn_out, names[3], &[d, d])?;
        let (name, value) = tensor(
            &format!("layers.{index}.out_proj.weight"),
            &[d, d],
            transpose(&attn_out.f32s()?, d, d)?,
        )?;
        out.insert(name, value);

        // gate 與 up 在 ONNX 已經是同一條 [d, 2*ffn] 的 MatMul。
        let up = get(names[4])?;
        expect_dims(up, names[4], &[d, 2 * ffn])?;
        let (name, value) = tensor(
            &format!("layers.{index}.mlp_up.weight"),
            &[2 * ffn, d],
            transpose(&up.f32s()?, d, 2 * ffn)?,
        )?;
        out.insert(name, value);

        // down 是 MatMul 的 `[in=ffn, out=d]`，轉成 `[out, in]`。
        let down = get(names[5])?;
        expect_dims(down, names[5], &[ffn, d])?;
        let (name, value) = tensor(
            &format!("layers.{index}.mlp_down.weight"),
            &[d, ffn],
            transpose(&down.f32s()?, ffn, d)?,
        )?;
        out.insert(name, value);
    }

    let final_norm = get("m.network.model.final_norm.weight")?;
    expect_dims(final_norm, "final_norm", &[d])?;
    let (name, value) = tensor("final_norm.weight", &[d], final_norm.f32s()?)?;
    out.insert(name, value);

    let dense = get("m.network.lm_head.dense.weight")?;
    expect_dims(dense, "head.dense", &[d, d])?;
    let (name, value) = tensor("head.dense.weight", &[d, d], dense.f32s()?)?;
    out.insert(name, value);

    let head_norm = get("m.network.lm_head.norm.weight")?;
    expect_dims(head_norm, "head.norm", &[d])?;
    let (name, value) = tensor("head.norm.weight", &[d], head_norm.f32s()?)?;
    out.insert(name, value);

    let bias = get("val_779")?;
    expect_dims(bias, "head.bias", &[config.vocab_size])?;
    let (name, value) = tensor("head.bias", &[config.vocab_size], bias.f32s()?)?;
    out.insert(name, value);

    // out_ids 是輸出投影用的 token 清單；存成 f32 只為了沿用同一種張量格式，
    // 值都是整數，27,865 以內沒有精度問題。
    let out_ids = get("out_ids")?;
    expect_dims(out_ids, "char_out_ids", &[config.out_len])?;
    let ids: Vec<f32> = out_ids.i64s()?.into_iter().map(|id| id as f32).collect();
    let (name, value) = tensor("char_out_ids", &[config.out_len, 1], ids)?;
    out.insert(name, value);

    let inv_freq = get("expand")?;
    expect_dims(inv_freq, "rope_inv_freq", &[1, config.head_dim() / 2, 1])?;
    let (name, value) = tensor(
        "rope_inv_freq",
        &[1, config.head_dim() / 2],
        inv_freq.f32s()?,
    )?;
    out.insert(name, value);

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    candle_core::safetensors::save(&out, output).map_err(super::inference)?;

    let params = out.values().map(|tensor| tensor.elem_count()).sum();
    Ok(ConvertReport {
        path: output.to_path_buf(),
        source: input.to_path_buf(),
        tensors: out.len(),
        params,
        bytes: std::fs::metadata(output)?.len(),
    })
}

/// 把 fp32 safetensors 轉成指定等級的 GGUF。
///
/// 只量化會做矩陣乘法的權重；LayerNorm、bias、位置頻率與 `out_ids` 留 fp32。
pub fn export_gguf(input: &Path, output: &Path, quant: QuantDtype) -> Result<ExportReport> {
    export_with_embedding(input, output, quant, true)
}

/// 與 [`export_gguf`] 相同，但可以選擇不量化字元表（供精度實驗用）。
pub fn export_with_embedding(
    input: &Path,
    output: &Path,
    quant: QuantDtype,
    quantize_embedding: bool,
) -> Result<ExportReport> {
    let config = ZhConfig::default();
    let device = Device::Cpu;
    let tensors = candle_core::safetensors::load(input, &device).map_err(super::inference)?;
    let mut entries = Vec::new();
    let mut quantized = Vec::new();
    for name in tensor_names(&config) {
        let tensor = tensors
            .get(&name)
            .ok_or_else(|| ModelError::Candle(format!("{} 缺少張量 {name}", input.display())))?;
        let target =
            if is_quantized(&name) && (quantize_embedding || name != "char_embedding.weight") {
                quant.ggml()
            } else {
                GgmlDType::F32
            };
        let qtensor = QTensor::quantize(tensor, target).map_err(super::inference)?;
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
        ("general.name", gguf_file::Value::String("zh-select".into())),
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
    let mut writer = std::io::BufWriter::new(file);
    let borrowed: Vec<(&str, &QTensor)> = quantized
        .iter()
        .map(|(name, tensor)| (name.as_str(), tensor))
        .collect();
    gguf_file::write(&mut writer, &metadata, &borrowed).map_err(super::inference)?;
    Ok(ExportReport {
        path: output.to_path_buf(),
        quant,
        file_bytes: std::fs::metadata(output)?.len(),
        entries,
    })
}

/// 會做矩陣乘法的權重。
fn is_quantized(name: &str) -> bool {
    name == "char_embedding.weight"
        || name == "head.dense.weight"
        || (name.starts_with("layers.")
            && (name.ends_with(".in_proj.weight")
                || name.ends_with(".out_proj.weight")
                || name.ends_with(".mlp_up.weight")
                || name.ends_with(".mlp_down.weight")))
}

/// 模型裡所有張量名稱，順序固定。
fn tensor_names(config: &ZhConfig) -> Vec<String> {
    let mut names = vec![
        "char_embedding.weight".to_string(),
        "base_embedding.weight".to_string(),
        "tone_embedding.weight".to_string(),
        "type_embedding.weight".to_string(),
        "embedding_norm.weight".to_string(),
        "final_norm.weight".to_string(),
        "head.dense.weight".to_string(),
        "head.norm.weight".to_string(),
        "head.bias".to_string(),
        "char_out_ids".to_string(),
        "rope_inv_freq".to_string(),
    ];
    for index in 0..config.layers {
        let prefix = format!("layers.{index}");
        let mut suffixes = vec![
            "mlp_norm.weight",
            "in_proj.weight",
            "out_proj.weight",
            "mlp_up.weight",
            "mlp_down.weight",
        ];
        if index > 0 {
            suffixes.insert(0, "attn_norm.weight");
        }
        for suffix in suffixes {
            names.push(format!("{prefix}.{suffix}"));
        }
    }
    names
}

/// 權重檔路徑：`JC_ZH_SELECT_WEIGHTS` → `model.safetensors` → `model.gguf`。
pub fn weights_path(dir: &Path) -> Option<PathBuf> {
    if let Some(value) = std::env::var_os(WEIGHTS_ENV) {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Some(path);
        }
    }
    for name in [DEFAULT_WEIGHTS_FILE, DEFAULT_QUANT_FILE] {
        let path = dir.join(name);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// fp32 的 dense 層；權重是 `[out, in]`，bias 可選。
struct Fp32Dense {
    weight: Tensor,
    bias: Option<Tensor>,
}

impl LinearOp for Fp32Dense {
    fn apply(&self, x: &Tensor) -> Result<Tensor> {
        dense_forward(x, &self.weight, self.bias.as_ref())
    }
}

/// 量化 dense 層；bias 留在 fp32。
struct QuantDense {
    weight: QMatMul,
    bias: Option<Tensor>,
}

impl LinearOp for QuantDense {
    fn apply(&self, x: &Tensor) -> Result<Tensor> {
        let dims = x.dims().to_vec();
        let k = *dims
            .last()
            .ok_or_else(|| ModelError::Candle("linear 的輸入沒有維度".into()))?;
        let rows = x.elem_count() / k.max(1);
        let flat = x.reshape((rows, k)).map_err(super::inference)?;
        let out = self.weight.forward(&flat).map_err(super::inference)?;
        let out = match &self.bias {
            Some(bias) => out.broadcast_add(bias).map_err(super::inference)?,
            None => out,
        };
        reshape_last(out, &dims)
    }
}

fn reshape_last(tensor: Tensor, dims: &[usize]) -> Result<Tensor> {
    let mut out_dims = dims.to_vec();
    if let Some(last) = out_dims.last_mut() {
        let out = tensor.dim(D::Minus1).map_err(super::inference)?;
        *last = out;
    }
    tensor.reshape(out_dims).map_err(super::inference)
}

/// 沿輸出維度切給 rayon 的 fp32 matmul；bias 可選。
fn dense_forward(x: &Tensor, weight: &Tensor, bias: Option<&Tensor>) -> Result<Tensor> {
    let (out, k) = weight.dims2().map_err(super::inference)?;
    let dims = x.dims().to_vec();
    let rows = x.elem_count() / k.max(1);
    // 每個 chunk 至少 64 個輸出，避免 gemm 太碎。
    let chunks = rayon::current_num_threads().min(out / 64).max(1);
    let flat = x.reshape((rows, k)).map_err(super::inference)?;
    if chunks <= 1 {
        let out = flat.matmul(&weight.t().map_err(super::inference)?);
        return match bias {
            Some(bias) => out
                .and_then(|t| t.broadcast_add(bias))
                .map_err(super::inference)
                .and_then(|t| reshape_last(t, &dims)),
            None => out
                .map_err(super::inference)
                .and_then(|t| reshape_last(t, &dims)),
        };
    }
    let parts: std::result::Result<Vec<Tensor>, candle_core::Error> = (0..chunks)
        .into_par_iter()
        .map(|index| {
            let start = index * out / chunks;
            let len = (index + 1) * out / chunks - start;
            let w = weight.narrow(0, start, len)?;
            let part = flat.matmul(&w.t()?)?;
            match bias {
                Some(bias) => part.broadcast_add(&bias.narrow(0, start, len)?),
                None => Ok(part),
            }
        })
        .collect();
    let parts = parts.map_err(super::inference)?;
    let out = Tensor::cat(&parts, 1).map_err(super::inference)?;
    reshape_last(out, &dims)
}

/// fp32 的詞嵌入查表。
struct Fp32Lookup {
    weight: Tensor,
}

impl Lookup for Fp32Lookup {
    fn lookup(&self, ids: &Tensor) -> Result<Tensor> {
        let dims = ids.dims().to_vec();
        let hidden = *self
            .weight
            .dims()
            .last()
            .ok_or_else(|| ModelError::Candle("char_embedding 沒有維度".into()))?;
        let flat = ids.flatten_all().map_err(super::inference)?;
        let mut out = dims;
        out.push(hidden);
        self.weight
            .index_select(&flat, 0)
            .and_then(|t| t.reshape(out))
            .map_err(super::inference)
    }
}

/// 量化的詞嵌入查表；與輸出投影共用同一個 [`QMatMul`]。
struct QuantLookup {
    weight: QMatMul,
}

impl Lookup for QuantLookup {
    fn lookup(&self, ids: &Tensor) -> Result<Tensor> {
        let dims = ids.dims().to_vec();
        let tensor = self.weight.embedding(ids).map_err(super::inference)?;
        let mut out = dims;
        out.push(tensor.dim(D::Minus1).map_err(super::inference)?);
        tensor.reshape(out).map_err(super::inference)
    }
}

/// 一層 pre-LN transformer block。
///
/// 第 0 層的 attention norm 是 `None`：ONNX 圖把 `embeddings.norm` 的輸出
/// 直接餵進第 0 層的 qkv，殘差流是從那份歸一化結果開始的，不能再歸一化
/// 一次。
struct ZhLayer<L> {
    attn_norm: Option<(Tensor, Tensor)>,
    mlp_norm: (Tensor, Tensor),
    in_proj: L,
    out_proj: L,
    mlp_up: L,
    mlp_down: L,
}

impl<L: LinearOp> ZhLayer<L> {
    fn forward(
        &self,
        hidden: &Tensor,
        cos: &Tensor,
        sin: &Tensor,
        mask: &Tensor,
        config: &ZhConfig,
    ) -> Result<Tensor> {
        let (seq, d) = hidden.dims2().map_err(super::inference)?;
        let normed = match &self.attn_norm {
            Some((weight, bias)) => {
                layer_norm(hidden, weight, bias, LAYER_NORM_EPS).map_err(super::inference)?
            }
            None => hidden.clone(),
        };
        let qkv = self.in_proj.apply(&normed)?;
        let head_dim = config.head_dim();
        let shape = (1, seq, config.heads, head_dim);
        let head = |start: usize| -> Result<Tensor> {
            qkv.narrow(1, start, config.hidden)
                .and_then(|t| t.reshape(shape))
                .and_then(|t| t.transpose(1, 2))
                .and_then(|t| t.contiguous())
                .map_err(super::inference)
        };
        let query = rope(&head(0)?, cos, sin)?;
        let key = rope(&head(config.hidden)?, cos, sin)?;
        let value = head(2 * config.hidden)?;
        let scores = query
            .matmul(&key.transpose(2, 3).map_err(super::inference)?)
            .and_then(|t| t.affine(ATTENTION_SCALE, 0.0))
            .and_then(|t| t.broadcast_add(mask))
            .map_err(super::inference)?;
        let scores = softmax(&scores, D::Minus1).map_err(super::inference)?;
        let context = scores
            .matmul(&value)
            .and_then(|t| t.transpose(1, 2))
            .and_then(|t| t.contiguous())
            .and_then(|t| t.reshape((seq, d)))
            .map_err(super::inference)?;
        let attended = self.out_proj.apply(&context)?;
        let hidden = (hidden + attended).map_err(super::inference)?;

        let normed = layer_norm(&hidden, &self.mlp_norm.0, &self.mlp_norm.1, LAYER_NORM_EPS)
            .map_err(super::inference)?;
        let up = self.mlp_up.apply(&normed)?;
        let gate = up.narrow(1, 0, config.ffn).map_err(super::inference)?;
        let value = up
            .narrow(1, config.ffn, config.ffn)
            .map_err(super::inference)?;
        let gated = gate
            .gelu_erf()
            .and_then(|t| t.mul(&value))
            .map_err(super::inference)?;
        let projected = self.mlp_down.apply(&gated)?;
        (hidden + projected).map_err(super::inference)
    }
}

/// RoPE：前後半互換後取負，與 ONNX 的 `cat(-x2, x1)` 相同。
fn rope(x: &Tensor, cos: &Tensor, sin: &Tensor) -> Result<Tensor> {
    let half = x.dim(D::Minus1).map_err(super::inference)? / 2;
    let first = x.narrow(3, 0, half).map_err(super::inference)?;
    let second = x.narrow(3, half, half).map_err(super::inference)?;
    let rotated = Tensor::cat(&[second.neg().map_err(super::inference)?, first], 3)
        .map_err(super::inference)?;
    x.broadcast_mul(cos)
        .and_then(|a| rotated.broadcast_mul(sin).and_then(|b| a.add(&b)))
        .map_err(super::inference)
}

/// 不含權重格式細節的網路；`L` 決定 fp32 或量化。
struct ZhNet<L> {
    config: ZhConfig,
    char_embedding: Box<dyn Lookup>,
    char_out: L,
    base_embedding: Tensor,
    tone_embedding: Tensor,
    type_embedding: Tensor,
    embedding_norm: (Tensor, Tensor),
    layers: Vec<ZhLayer<L>>,
    final_norm: (Tensor, Tensor),
    head_dense: L,
    head_norm: (Tensor, Tensor),
    head_bias: Tensor,
    out_ids: Vec<u32>,
    inv_freq: Tensor,
}

impl<L: LinearOp> ZhNet<L> {
    fn scores(&self, input: &ZhInput<'_>) -> Result<Vec<f32>> {
        let config = &self.config;
        check_input(config, input)?;
        let device = Device::Cpu;
        let seq = input.seq;
        let hidden = config.hidden;
        let index = |values: &[i64]| -> Result<Tensor> {
            let ids: Vec<u32> = values
                .iter()
                .map(|value| {
                    u32::try_from(*value)
                        .map_err(|_| ModelError::Candle(format!("索引 {value} 不是非負整數")))
                })
                .collect::<Result<_>>()?;
            Tensor::from_vec(ids, values.len(), &device).map_err(super::inference)
        };

        let chars = self.char_embedding.lookup(&index(input.ids)?)?;
        let base = self
            .base_embedding
            .index_select(&index(input.bases)?, 0)
            .map_err(super::inference)?;
        let tone = self
            .tone_embedding
            .index_select(&index(input.tones)?, 0)
            .map_err(super::inference)?;
        let sounds = base.add(&tone).map_err(super::inference)?;
        let types = index(input.types)?;
        let cond = types
            .eq(1u32)
            .and_then(|t| t.reshape((seq, 1)))
            .and_then(|t| t.broadcast_as((seq, hidden)))
            .map_err(super::inference)?;
        let mut x = cond.where_cond(&sounds, &chars).map_err(super::inference)?;
        let type_embedding = self
            .type_embedding
            .index_select(&types, 0)
            .map_err(super::inference)?;
        x = x.add(&type_embedding).map_err(super::inference)?;
        x = layer_norm(
            &x,
            &self.embedding_norm.0,
            &self.embedding_norm.1,
            LAYER_NORM_EPS,
        )
        .map_err(super::inference)?;

        // 位置頻率：freqs = positions × inv_freq，再複製成前後半。
        let positions: Vec<f32> = input
            .positions
            .iter()
            .map(|position| *position as f32)
            .collect();
        let positions = Tensor::from_vec(positions, (seq, 1), &device).map_err(super::inference)?;
        let freqs = positions.matmul(&self.inv_freq).map_err(super::inference)?;
        let half = Tensor::cat(&[&freqs, &freqs], 1).map_err(super::inference)?;
        let cos = half
            .cos()
            .and_then(|t| t.reshape((1, 1, seq, config.head_dim())))
            .map_err(super::inference)?;
        let sin = half
            .sin()
            .and_then(|t| t.reshape((1, 1, seq, config.head_dim())))
            .map_err(super::inference)?;

        let additive: Vec<f32> = input
            .mask
            .iter()
            .map(|attend| if *attend { 0.0 } else { MASK_PENALTY })
            .collect();
        let mask =
            Tensor::from_vec(additive, (1, 1, seq, seq), &device).map_err(super::inference)?;

        for layer in &self.layers {
            x = layer.forward(&x, &cos, &sin, &mask, config)?;
        }
        x = layer_norm(&x, &self.final_norm.0, &self.final_norm.1, LAYER_NORM_EPS)
            .map_err(super::inference)?;

        // 只投影 proj_pos 的列。
        let projected = x
            .index_select(&index(input.proj_pos)?, 0)
            .map_err(super::inference)?;
        let rows = projected.dims2().map_err(super::inference)?.0;
        let hidden = self.head_dense.apply(&projected)?;
        let hidden = hidden.gelu_erf().map_err(super::inference)?;
        let hidden = layer_norm(
            &hidden,
            &self.head_norm.0,
            &self.head_norm.1,
            LAYER_NORM_EPS,
        )
        .map_err(super::inference)?;
        let logits = self.char_out.apply(&hidden)?;
        let out_ids = Tensor::from_vec(self.out_ids.clone(), self.out_ids.len(), &device)
            .map_err(super::inference)?;
        let logits = logits.index_select(&out_ids, 1).map_err(super::inference)?;
        let bias = self
            .head_bias
            .index_select(&out_ids, 0)
            .map_err(super::inference)?;
        let logits = logits.broadcast_add(&bias).map_err(super::inference)?;
        let log_probs = log_softmax(&logits, D::Minus1).map_err(super::inference)?;

        // GatherND：列索引取 target_node，欄索引取 target_ids，再沿 cols 加總。
        let out_len = config.out_len as i64;
        let flat_index: Vec<i64> = input
            .target_node
            .iter()
            .zip(input.target_ids.iter())
            .map(|(row, token)| row * out_len + token)
            .collect();
        let flat = log_probs
            .reshape((rows * config.out_len,))
            .map_err(super::inference)?;
        let picked = flat
            .index_select(&index(&flat_index)?, 0)
            .and_then(|t| t.reshape((input.rows, input.cols)))
            .map_err(super::inference)?;
        picked
            .sum(1)
            .and_then(|t| t.to_vec1::<f32>())
            .map_err(super::inference)
    }
}

fn check_input(config: &ZhConfig, input: &ZhInput<'_>) -> Result<()> {
    let seq = input.seq;
    if seq == 0 {
        return Err(ModelError::Candle("序列長度是 0".into()));
    }
    for (name, len) in [
        ("ids", input.ids.len()),
        ("bases", input.bases.len()),
        ("tones", input.tones.len()),
        ("types", input.types.len()),
        ("positions", input.positions.len()),
    ] {
        if len != seq {
            return Err(ModelError::Candle(format!(
                "{name} 長度 {len} 與序列長度 {seq} 不符"
            )));
        }
    }
    if input.mask.len() != seq * seq {
        return Err(ModelError::Candle(format!(
            "mask 長度 {} 與 {seq}x{seq} 不符",
            input.mask.len()
        )));
    }
    if input.rows == 0 || input.cols == 0 {
        return Err(ModelError::Candle("候選列或欄數是 0".into()));
    }
    if input.target_node.len() != input.rows * input.cols
        || input.target_ids.len() != input.rows * input.cols
    {
        return Err(ModelError::Candle(
            "target_node／target_ids 長度與列數不符".into(),
        ));
    }
    if input.proj_pos.is_empty() || input.proj_pos.iter().any(|p| *p < 0 || *p as usize >= seq) {
        return Err(ModelError::Candle("proj_pos 超出序列範圍".into()));
    }
    for node in input.target_node {
        if *node < 0 || *node as usize >= input.proj_pos.len() {
            return Err(ModelError::Candle("target_node 超出投影範圍".into()));
        }
    }
    for token in input.target_ids {
        if *token < 0 || *token as usize >= config.out_len {
            return Err(ModelError::Candle("target_ids 超出輸出範圍".into()));
        }
    }
    Ok(())
}

/// 已載入權重的注音選字模型；`&self` 可以跨執行緒共用。
pub struct ZhSelect {
    config: ZhConfig,
    net: ZhNet<Fp32Dense>,
}

/// 量化權重的注音選字模型。
pub struct QuantZhSelect {
    config: ZhConfig,
    net: ZhNet<QuantDense>,
    quant: QuantDtype,
}

macro_rules! impl_select {
    ($ty:ty) => {
        impl $ty {
            /// 架構參數。
            pub fn config(&self) -> &ZhConfig {
                &self.config
            }

            /// 一次評分，回傳每個候選列的分數（`log_softmax` 的總和）。
            ///
            /// 列順序是分支優先、再依候選，與 `jc_core::zh::select::prepare`
            /// 的 `target_node` 一致；同一候選在不同分支取最大值。
            pub fn scores(&self, input: &ZhInput<'_>) -> Result<Vec<f32>> {
                self.net.scores(input)
            }
        }
    };
}

impl_select!(ZhSelect);
impl_select!(QuantZhSelect);

fn zero_bias(hidden: usize) -> Result<Tensor> {
    Tensor::zeros(hidden, candle_core::DType::F32, &Device::Cpu).map_err(super::inference)
}

fn norm_pair(weight: Tensor) -> Result<(Tensor, Tensor)> {
    let hidden = weight.dim(0).map_err(super::inference)?;
    Ok((weight, zero_bias(hidden)?))
}

impl ZhSelect {
    /// 從 fp32 safetensors 載入權重。
    pub fn load(weights: &Path) -> Result<Self> {
        let config = ZhConfig::default();
        let (_device, vb) = super::load_weights(weights)?;
        let d = config.hidden;
        let char_weight = vb
            .get((config.out_vocab(), d), "char_embedding.weight")
            .map_err(super::inference)?;
        let char_embedding: Box<dyn Lookup> = Box::new(Fp32Lookup {
            weight: char_weight.clone(),
        });
        let char_out = Fp32Dense {
            weight: char_weight,
            bias: None,
        };
        let mut layers = Vec::with_capacity(config.layers);
        for index in 0..config.layers {
            let prefix = format!("layers.{index}");
            let attn_norm = if index == 0 {
                None
            } else {
                Some(norm_pair(
                    vb.get(d, &format!("{prefix}.attn_norm.weight"))
                        .map_err(super::inference)?,
                )?)
            };
            layers.push(ZhLayer {
                attn_norm,
                mlp_norm: norm_pair(
                    vb.get(d, &format!("{prefix}.mlp_norm.weight"))
                        .map_err(super::inference)?,
                )?,
                in_proj: Fp32Dense {
                    weight: vb
                        .get((3 * d, d), &format!("{prefix}.in_proj.weight"))
                        .map_err(super::inference)?,
                    bias: None,
                },
                out_proj: Fp32Dense {
                    weight: vb
                        .get((d, d), &format!("{prefix}.out_proj.weight"))
                        .map_err(super::inference)?,
                    bias: None,
                },
                mlp_up: Fp32Dense {
                    weight: vb
                        .get((2 * config.ffn, d), &format!("{prefix}.mlp_up.weight"))
                        .map_err(super::inference)?,
                    bias: None,
                },
                mlp_down: Fp32Dense {
                    weight: vb
                        .get((d, config.ffn), &format!("{prefix}.mlp_down.weight"))
                        .map_err(super::inference)?,
                    bias: None,
                },
            });
        }
        let head_dense = Fp32Dense {
            weight: vb
                .get((d, d), "head.dense.weight")
                .map_err(super::inference)?,
            bias: None,
        };
        let net = ZhNet {
            config,
            char_embedding,
            char_out,
            base_embedding: vb
                .get((config.base_count, d), "base_embedding.weight")
                .map_err(super::inference)?,
            tone_embedding: vb
                .get((config.tone_count, d), "tone_embedding.weight")
                .map_err(super::inference)?,
            type_embedding: vb
                .get((config.type_count, d), "type_embedding.weight")
                .map_err(super::inference)?,
            embedding_norm: norm_pair(
                vb.get(d, "embedding_norm.weight")
                    .map_err(super::inference)?,
            )?,
            layers,
            final_norm: norm_pair(vb.get(d, "final_norm.weight").map_err(super::inference)?)?,
            head_dense,
            head_norm: norm_pair(vb.get(d, "head.norm.weight").map_err(super::inference)?)?,
            head_bias: vb
                .get(config.out_vocab(), "head.bias")
                .map_err(super::inference)?,
            out_ids: read_out_ids(
                &vb.get((config.out_len, 1), "char_out_ids")
                    .map_err(super::inference)?,
            )?,
            inv_freq: vb
                .get((1, config.head_dim() / 2), "rope_inv_freq")
                .map_err(super::inference)?,
        };
        Ok(Self { config, net })
    }
}

impl QuantZhSelect {
    /// 從 GGUF 載入量化權重；只有非矩陣乘法的張量還原成 fp32。
    pub fn load(weights: &Path) -> Result<Self> {
        let config = ZhConfig::default();
        let mut file = File::open(weights)?;
        let content = gguf_file::Content::read(&mut file).map_err(super::inference)?;
        let quant = read_quant(&content, weights)?;
        let mut reader = GgufReader {
            content: &content,
            file,
        };
        let char_weight = reader.quantized("char_embedding.weight")?;
        let table = QMatMul::from_arc(Arc::new(char_weight)).map_err(super::inference)?;
        let char_embedding: Box<dyn Lookup> = Box::new(QuantLookup {
            weight: table.clone(),
        });
        let char_out = QuantDense {
            weight: table,
            bias: None,
        };
        let mut layers = Vec::with_capacity(config.layers);
        for index in 0..config.layers {
            let prefix = format!("layers.{index}");
            let attn_norm = if index == 0 {
                None
            } else {
                Some(norm_pair(
                    reader.f32(&format!("{prefix}.attn_norm.weight"))?,
                )?)
            };
            layers.push(ZhLayer {
                attn_norm,
                mlp_norm: norm_pair(reader.f32(&format!("{prefix}.mlp_norm.weight"))?)?,
                in_proj: QuantDense {
                    weight: reader.qmatmul(&format!("{prefix}.in_proj.weight"))?,
                    bias: None,
                },
                out_proj: QuantDense {
                    weight: reader.qmatmul(&format!("{prefix}.out_proj.weight"))?,
                    bias: None,
                },
                mlp_up: QuantDense {
                    weight: reader.qmatmul(&format!("{prefix}.mlp_up.weight"))?,
                    bias: None,
                },
                mlp_down: QuantDense {
                    weight: reader.qmatmul(&format!("{prefix}.mlp_down.weight"))?,
                    bias: None,
                },
            });
        }
        let net = ZhNet {
            config,
            char_embedding,
            char_out,
            base_embedding: reader.f32("base_embedding.weight")?,
            tone_embedding: reader.f32("tone_embedding.weight")?,
            type_embedding: reader.f32("type_embedding.weight")?,
            embedding_norm: norm_pair(reader.f32("embedding_norm.weight")?)?,
            layers,
            final_norm: norm_pair(reader.f32("final_norm.weight")?)?,
            head_dense: QuantDense {
                weight: reader.qmatmul("head.dense.weight")?,
                bias: None,
            },
            head_norm: norm_pair(reader.f32("head.norm.weight")?)?,
            head_bias: reader.f32("head.bias")?,
            out_ids: read_out_ids(&reader.f32("char_out_ids")?)?,
            inv_freq: reader.f32("rope_inv_freq")?,
        };
        Ok(Self { config, net, quant })
    }

    /// 量化等級。
    pub fn quant(&self) -> QuantDtype {
        self.quant
    }
}

fn read_out_ids(tensor: &Tensor) -> Result<Vec<u32>> {
    let values = tensor
        .to_dtype(candle_core::DType::F32)
        .and_then(|t| t.flatten_all())
        .and_then(|t| t.to_vec1::<f32>())
        .map_err(super::inference)?;
    values
        .into_iter()
        .map(|value| {
            if value < 0.0 || value.fract() != 0.0 {
                return Err(ModelError::Candle(format!("out_ids 含非整數 {value}")));
            }
            u32::try_from(value as u64)
                .map_err(|_| ModelError::Candle(format!("out_ids 超出範圍：{value}")))
        })
        .collect()
}

/// GGUF 的張量索引與檔案 handle。
struct GgufReader<'a> {
    content: &'a gguf_file::Content,
    file: File,
}

impl GgufReader<'_> {
    fn quantized(&mut self, name: &str) -> Result<QTensor> {
        self.content
            .tensor(&mut self.file, name, &Device::Cpu)
            .map_err(super::inference)
    }

    fn qmatmul(&mut self, name: &str) -> Result<QMatMul> {
        QMatMul::from_qtensor(self.quantized(name)?).map_err(super::inference)
    }

    fn f32(&mut self, name: &str) -> Result<Tensor> {
        self.quantized(name)?
            .dequantize(&Device::Cpu)
            .map_err(super::inference)
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn reference_input() -> ZhInput<'static> {
        // `prepare()` 的 basic 案例（見 docs/port/model-wire-zh.md）：
        // 前文「一二」、讀音 a b、候選「三四」「五あ」。
        static IDS: [i64; 9] = [1, 8, 9, 2, 0, 0, 3, 10, 12];
        static BASES: [i64; 9] = [0, 0, 0, 0, 2, 3, 0, 0, 0];
        static TONES: [i64; 9] = [0, 0, 0, 0, 1, 1, 0, 0, 0];
        static TYPES: [i64; 9] = [0, 0, 0, 0, 1, 1, 0, 2, 2];
        static POSITIONS: [i64; 9] = [0, 1, 2, 3, 4, 5, 6, 7, 7];
        static MASK: [bool; 81] = {
            let mut mask = [false; 81];
            let mut q = 0;
            while q < 5 {
                let mut a = 0;
                while a <= q {
                    mask[q * 9 + a] = true;
                    a += 1;
                }
                q += 1;
            }
            while q < 9 {
                let mut a = 0;
                while a < 5 {
                    mask[q * 9 + a] = true;
                    a += 1;
                }
                q += 1;
            }
            mask[5 * 9 + 5] = true;
            mask[6 * 9 + 5] = true;
            mask[6 * 9 + 6] = true;
            mask[7 * 9 + 5] = true;
            mask[7 * 9 + 6] = true;
            mask[7 * 9 + 7] = true;
            mask[8 * 9 + 5] = true;
            mask[8 * 9 + 6] = true;
            mask[8 * 9 + 8] = true;
            mask
        };
        ZhInput {
            seq: 9,
            ids: &IDS,
            bases: &BASES,
            tones: &TONES,
            types: &TYPES,
            positions: &POSITIONS,
            mask: &MASK,
            proj_pos: &[6, 7, 8],
            target_node: &[0, 1, 0, 2],
            target_ids: &[4, 5, 6, 12],
            rows: 2,
            cols: 2,
        }
    }

    #[test]
    fn convert_matches_onnx_fp32() {
        let onnx = jc_testdata::require_data!("models/zh-select/model.onnx");
        let dir = std::env::temp_dir().join(format!("jc-zh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let weights = dir.join("model.safetensors");
        let report = convert_onnx(&onnx, &weights).unwrap();
        assert_eq!(report.tensors, 58);
        assert!(report.bytes > 100 * 1024 * 1024);

        let model = ZhSelect::load(&weights).unwrap();
        let scores = model.scores(&reference_input()).unwrap();
        // 期望值由 ORT 在同一組輸入上跑出來。
        let want = [-29.702_269_f32, -35.048_065];
        assert_eq!(scores.len(), want.len());
        for (got, want) in scores.iter().zip(want) {
            assert!(
                (got - want).abs() < 1e-3,
                "candle {got} vs ORT {want}，誤差 {}",
                (got - want).abs()
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn q8_0_stays_close_to_fp32() {
        let onnx = jc_testdata::require_data!("models/zh-select/model.onnx");
        let dir = std::env::temp_dir().join(format!("jc-zh-q-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let weights = dir.join("model.safetensors");
        convert_onnx(&onnx, &weights).unwrap();
        let fp32 = ZhSelect::load(&weights).unwrap();
        let want = fp32.scores(&reference_input()).unwrap();
        let gguf = dir.join("model.gguf");
        let report = export_gguf(&weights, &gguf, QuantDtype::Q8_0).unwrap();
        assert!(report.file_bytes < 40 * 1024 * 1024);
        let model = QuantZhSelect::load(&gguf).unwrap();
        assert_eq!(model.quant(), QuantDtype::Q8_0);
        let got = model.scores(&reference_input()).unwrap();
        // 這一組的實測差是 0.099；732 個候選的最大差是 0.313。
        for (got, want) in got.iter().zip(&want) {
            assert!(
                (got - want).abs() < 0.2,
                "Q8_0 {got} vs fp32 {want}，誤差 {}",
                (got - want).abs()
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_matmul_weights_are_quantized() {
        assert!(is_quantized("char_embedding.weight"));
        assert!(is_quantized("layers.3.in_proj.weight"));
        assert!(!is_quantized("layers.3.attn_norm.weight"));
        assert!(!is_quantized("base_embedding.weight"));
        assert!(!is_quantized("head.bias"));
    }

    #[test]
    fn config_matches_manifest() {
        let config = ZhConfig::default();
        assert_eq!(config.head_dim(), 64);
        assert_eq!(config.out_vocab() - config.out_len, 6);
        assert_eq!(tensor_names(&config).len(), 58);
    }

    #[test]
    fn invalid_input_is_rejected() {
        let mut input = reference_input();
        input.mask = &[true; 4];
        let err = check_input(&ZhConfig::default(), &input).unwrap_err();
        assert!(err.to_string().contains("mask"), "{err}");
    }
}
