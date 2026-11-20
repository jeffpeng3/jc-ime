//! 模型推論接線：日文 reranker 與 Jinen 都走 candle。
//!
//! ONNX Runtime 後端已於 2026-11-20 移除。權重是 checkpoint 轉出的
//! safetensors（fp32）或 GGUF（量化），由 [`candle`] 載入，不需要外部
//! 動態庫。模型缺席時模型集合標成 [`Availability::Unavailable`]，引擎退回
//! 統計路徑，不 panic。
//!
//! 提供：
//!
//! - [`Reranker`]：日文候選重排，`score(left, candidate, right) -> Option<f32>`。
//!
//! 模型目錄預設放 `model.safetensors` 與 tokenizer；`model.gguf` 會自動走
//! 量化路徑。tokenizer 可以是 SentencePiece 的 `*.model`（走 unigram
//! Viterbi，見 [`spm`]），也可以是只有詞表的 `vocab.json`
//! （`0c.sentencepiece.vocab/1`，退回最長前綴匹配）。

pub mod candle;
pub mod error;
pub mod layout;
pub mod manifest;
pub mod rerank;
pub mod role;
pub mod spm;
pub mod tokenizer;

pub use candle::jinen::{Generation, Jinen};
pub use candle::{CandleReranker, QuantDtype, BACKEND_NAME};
pub use error::{ModelError, Result};
pub use manifest::{
    Availability, ModelSet, ModelSpec, DEFAULT_BOS_ID, DEFAULT_CANDIDATE_SLOT,
    DEFAULT_CONTEXT_TOKENS, DEFAULT_EOS_ID, DEFAULT_MODEL_FILE, DEFAULT_PAD_ID, DEFAULT_SEQ_LEN,
    DEFAULT_TIMEOUT_MS, DEFAULT_VOCAB_FILE, SET_SCHEMA,
};
pub use rerank::Reranker;
pub use role::Role;
pub use tokenizer::{Strategy, Tokenizer};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_is_not_an_error() {
        let a = Availability::Unavailable {
            role: Role::JaRerank,
            reason: "no model".into(),
        };
        assert!(!a.is_loaded());
        assert_eq!(a.role(), Role::JaRerank);
    }

    #[test]
    fn role_names_roundtrip() {
        for role in Role::all() {
            assert_eq!(role.as_str().parse::<Role>(), Ok(role));
        }
        assert_eq!(Role::ZhSelect.as_str(), "zh-select");
    }

    #[test]
    fn public_types_are_send() {
        fn assert_send<T: Send>() {}
        assert_send::<ModelSet>();
        assert_send::<Tokenizer>();
        assert_send::<Availability>();
    }
}
