//! Jinen v2（Qwen3 架構 GGUF）的日文轉換推論。
//!
//! Jinen 是生成式かな漢字変換模型：輸入片假名讀音（可附左文脈），輸出
//! 轉換後的漢字かな交じり文。這一輪只提供離線推論路徑，不接進 `Session`。
//!
//! Prompt 照模型卡：
//!
//! ```text
//! U+EE02<left_context>U+EE00<katakana_reading>U+EE01
//! ```
//!
//! - 讀音先從平假名一對一轉成片假名。
//! - 沒有左文脈時整段 `U+EE02<left_context>` 省略。
//! - 整段 prompt 做 NFKC；模型卡明寫不做 NFKC 精度會大幅下降。
//! - 解碼固定 greedy（等同 llama.cpp 的 `temperature=0`、`top_k=1`），
//!   遇到 `</s>` 或滿 `max_new_tokens` 停止。
//!
//! Tokenizer 用 `tokenizers` crate 讀 HF repo 的 `tokenizer.json`
//! （BPE、NFKC normalizer、Digits pre-tokenizer、byte fallback）。這與
//! GGUF 內的詞表逐 id 相同，另外比 llama.cpp 多了 BOS 的處理：llama.cpp
//! 的 `tokenize` 預設 `add_bos=True`，所以這裡也手動補上 `BOS`。
//!
//! 量化格式由 candle 的 `quantized_qwen3` 直接吃 GGUF（small 用 Q4_K_M：
//! 大部分張量 Q4_K，`ffn_down` 與少數 `attn_v` 是 Q6_K，norm 等其餘是 F32）。

use std::path::Path;

use candle_core::quantized::gguf_file;
use candle_core::{DType, Device, Tensor};
use candle_transformers::models::quantized_qwen3::ModelWeights;
use tokenizers::Tokenizer;
use unicode_normalization::UnicodeNormalization;

use crate::error::{ModelError, Result};

/// 左文脈最多取幾個字元。
pub const CONTEXT_CHARS: usize = 64;
/// 模型卡的最小值；實際值從 GGUF metadata 讀。
pub const DEFAULT_BOS_ID: u32 = 2;
/// 模型卡的最小值；實際值從 GGUF metadata 讀。
pub const DEFAULT_EOS_ID: u32 = 3;
/// 預設最多產生幾個 token，與 Python 量測腳本相同。
pub const DEFAULT_MAX_NEW_TOKENS: usize = 256;

const INPUT_START: char = '\u{ee00}';
const OUTPUT_START: char = '\u{ee01}';
const CONTEXT_START: char = '\u{ee02}';

/// 一次轉換的結果。
#[derive(Debug, Clone)]
pub struct Generation {
    /// 模型產生的字串（已去掉特殊 token）。
    pub text: String,
    /// 模型實際產生的 token id，不含 prompt。
    pub tokens: Vec<u32>,
    /// prompt 的 token 數（含 BOS）。
    pub prompt_tokens: usize,
    /// 是否因 `max_new_tokens` 截斷。`</s>` 收尾時為 `false`。
    pub truncated: bool,
}

/// 單一解碼步的 logit 資訊，用來診斷 greedy 在接近平手時的翻轉。
#[derive(Debug, Clone, Copy)]
pub struct Step {
    /// 這一步選出的 token。
    pub token: u32,
    /// 最高分。
    pub top_logit: f32,
    /// 第二高分。
    pub runner_up_logit: f32,
}

/// Jinen v2 模型與 tokenizer。
pub struct Jinen {
    model: ModelWeights,
    tokenizer: Tokenizer,
    device: Device,
    bos_id: u32,
    eos_id: u32,
    context_length: usize,
}

impl Jinen {
    /// 從 GGUF 與 `tokenizer.json` 載入模型。
    pub fn load(gguf_path: &Path, tokenizer_path: &Path) -> Result<Self> {
        let device = Device::Cpu;
        let mut file = std::fs::File::open(gguf_path)?;
        let content = gguf_file::Content::read(&mut file).map_err(candle_err)?;

        let bos_id =
            metadata_u32(&content, "tokenizer.ggml.bos_token_id").unwrap_or(DEFAULT_BOS_ID);
        let eos_id =
            metadata_u32(&content, "tokenizer.ggml.eos_token_id").unwrap_or(DEFAULT_EOS_ID);
        let context_length = metadata_u32(&content, "qwen3.context_length")
            .map(|v| v as usize)
            .unwrap_or(1024);
        let architecture = content
            .metadata
            .get("general.architecture")
            .and_then(|value| value.to_string().ok())
            .cloned()
            .unwrap_or_default();
        if architecture != "qwen3" {
            return Err(ModelError::Candle(format!(
                "{} 的架構是 {architecture:?}，只支援 qwen3",
                gguf_path.display()
            )));
        }

        let model = ModelWeights::from_gguf(content, &mut file, &device).map_err(candle_err)?;
        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|err| ModelError::Tokenizer(format!("{}：{err}", tokenizer_path.display())))?;
        Ok(Self {
            model,
            tokenizer,
            device,
            bos_id,
            eos_id,
            context_length,
        })
    }

    /// 詞表大小。
    pub fn vocab_size(&self) -> usize {
        self.tokenizer.get_vocab_size(true)
    }

    /// BOS token id。
    pub fn bos_id(&self) -> u32 {
        self.bos_id
    }

    /// EOS token id。
    pub fn eos_id(&self) -> u32 {
        self.eos_id
    }

    /// 模型可接受的最長序列。
    pub fn context_length(&self) -> usize {
        self.context_length
    }

    /// 把平假名讀音轉成轉換結果。
    ///
    /// `context` 只取尾端 [`CONTEXT_CHARS`] 個字元；空字串代表不給文脈，
    /// prompt 會省略 `U+EE02` 前綴。
    pub fn generate(
        &mut self,
        reading: &str,
        context: &str,
        max_new_tokens: usize,
    ) -> Result<Generation> {
        Ok(self.generate_traced(reading, context, max_new_tokens)?.0)
    }

    /// 與 [`Self::generate`] 相同，另外回傳每一步的 logit 差距。
    pub fn generate_traced(
        &mut self,
        reading: &str,
        context: &str,
        max_new_tokens: usize,
    ) -> Result<(Generation, Vec<Step>)> {
        let prompt = build_prompt(reading, context);
        let mut ids = self.encode(&prompt)?;
        let prompt_tokens = ids.len();
        if prompt_tokens >= self.context_length {
            return Err(ModelError::Candle(format!(
                "prompt {prompt_tokens} tokens 超過 context {}",
                self.context_length
            )));
        }

        self.model.clear_kv_cache();
        let mut input =
            Tensor::from_slice(&ids, (1, ids.len()), &self.device).map_err(candle_err)?;
        let mut offset = 0usize;
        let mut generated = Vec::with_capacity(max_new_tokens);
        let mut steps = Vec::with_capacity(max_new_tokens);
        let mut truncated = true;

        while generated.len() < max_new_tokens {
            let logits = self.model.forward(&input, offset).map_err(candle_err)?;
            offset += ids.len();
            let (next, top, runner_up) = top_two(&logits)?;
            if next == self.eos_id {
                truncated = false;
                break;
            }
            generated.push(next);
            steps.push(Step {
                token: next,
                top_logit: top,
                runner_up_logit: runner_up,
            });
            if generated.len() >= max_new_tokens {
                break;
            }
            ids = vec![next];
            input = Tensor::from_slice(&ids, (1, 1), &self.device).map_err(candle_err)?;
        }

        let text = self
            .tokenizer
            .decode(&generated, true)
            .map_err(|err| ModelError::Tokenizer(format!("decode：{err}")))?;
        Ok((
            Generation {
                text,
                tokens: generated,
                prompt_tokens,
                truncated,
            },
            steps,
        ))
    }

    /// 把 prompt 編成 token id；前面補上 llama.cpp 預設的 BOS。
    pub fn encode(&self, prompt: &str) -> Result<Vec<u32>> {
        let encoding = self
            .tokenizer
            .encode(prompt, false)
            .map_err(|err| ModelError::Tokenizer(format!("encode：{err}")))?;
        let mut ids = Vec::with_capacity(encoding.len() + 1);
        ids.push(self.bos_id);
        ids.extend_from_slice(encoding.get_ids());
        Ok(ids)
    }

    /// 只解 token id 回字串（除錯與對照用）。
    pub fn decode(&self, ids: &[u32]) -> Result<String> {
        self.tokenizer
            .decode(ids, true)
            .map_err(|err| ModelError::Tokenizer(format!("decode：{err}")))
    }
}

/// 組出模型卡格式的 prompt，回傳已 NFKC 的字串。
pub fn build_prompt(reading: &str, context: &str) -> String {
    let mut prompt = String::new();
    if !context.is_empty() {
        prompt.push(CONTEXT_START);
        let tail: Vec<char> = context.chars().rev().take(CONTEXT_CHARS).collect();
        prompt.extend(tail.into_iter().rev());
    }
    prompt.push(INPUT_START);
    prompt.push_str(&hiragana_to_katakana(reading));
    prompt.push(OUTPUT_START);
    prompt.nfkc().collect()
}

/// 對字串做 NFKC，與模型卡的輸入前提相同。
pub fn nfkc(text: &str) -> String {
    text.nfkc().collect()
}

/// 平假名轉片假名；其他字元原樣保留。
/// 只做一對一碼位平移（U+3041..=U+3096 → +0x60），與 Python 版相同。
pub fn hiragana_to_katakana(text: &str) -> String {
    text.chars()
        .map(|ch| {
            let code = ch as u32;
            if (0x3041..=0x3096).contains(&code) {
                char::from_u32(code + 0x60).unwrap_or(ch)
            } else {
                ch
            }
        })
        .collect()
}

fn metadata_u32(content: &gguf_file::Content, key: &str) -> Option<u32> {
    content
        .metadata
        .get(key)
        .and_then(|value| value.to_u32().ok())
}

fn top_two(logits: &Tensor) -> Result<(u32, f32, f32)> {
    let values = logits
        .to_dtype(DType::F32)
        .and_then(|t| t.flatten_all())
        .and_then(|t| t.to_vec1::<f32>())
        .map_err(candle_err)?;
    let mut best = (0usize, f32::NEG_INFINITY);
    let mut second = f32::NEG_INFINITY;
    for (i, &v) in values.iter().enumerate() {
        if v > best.1 {
            second = best.1;
            best = (i, v);
        } else if v > second {
            second = v;
        }
    }
    Ok((best.0 as u32, best.1, second))
}

fn candle_err(err: candle_core::Error) -> ModelError {
    ModelError::Candle(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hiragana_to_katakana() {
        assert_eq!(hiragana_to_katakana("せいねんし"), "セイネンシ");
        assert_eq!(hiragana_to_katakana("ヴぁ"), "ヴァ");
        assert_eq!(hiragana_to_katakana("abc漢字"), "abc漢字");
    }

    #[test]
    fn test_build_prompt_without_context() {
        assert_eq!(build_prompt("せいねんし", ""), "\u{ee00}セイネンシ\u{ee01}");
    }

    #[test]
    fn test_build_prompt_with_context() {
        let prompt = build_prompt("あめ", "雨が");
        assert_eq!(prompt, "\u{ee02}雨が\u{ee00}アメ\u{ee01}");
    }

    #[test]
    fn test_build_prompt_keeps_context_tail() {
        let context = "あ".repeat(70);
        let prompt = build_prompt("あ", &context);
        let head: String = "\u{ee02}".to_string() + &"あ".repeat(CONTEXT_CHARS);
        assert_eq!(prompt, format!("{head}\u{ee00}ア\u{ee01}"));
    }

    #[test]
    fn test_build_prompt_nfkc() {
        // 全角括號與半角在 NFKC 後相同，模型卡明寫這件事是必要條件。
        assert_eq!(build_prompt("（", ""), build_prompt("(", ""));
        assert_eq!(build_prompt("ｶﾞ", ""), "\u{ee00}ガ\u{ee01}");
    }
}
