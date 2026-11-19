//! 模型推論接線：日文標點模型支援 ONNX Runtime（`ort`）與 candle。
//!
//! `ort` 是一般依賴，使用 `load-dynamic`：建置時不下載、不連結 ONNX
//! Runtime。執行期由 [`ModelRuntime::probe`] 偵測；沒有動態庫時模型集合
//! 全部 [`Availability::Unavailable`]，引擎退回統計路徑，不 panic。
//!
//! candle 後端讀 checkpoint 轉出的 safetensors，不需要 ONNX Runtime。兩個
//! 後端用 [`Backend`] 切換，預設仍是 ORT；`--backend candle` 在載入模型前
//! 呼叫 [`set_default_backend`]。candle 的 [`candle::Encoder`] 與
//! `candle-spike-ja` 的 reranker 共用，本分支只接上標點模型。
//!
//! 提供：
//!
//! - [`Reranker`]：日文候選重排，`score(left, candidate, right) -> Option<f32>`。
//! - [`PunctuationModel`]：日文標點推論，載入模型後 `punctuate(text)` 直接
//!   回傳插入標點的文字。
//! - [`ZhPunctuationModel`]：注音標點推論，規則同前身字元模型。
//!
//! 模型目錄預設放 `model.int8.onnx` 與 tokenizer。tokenizer 可以是
//! SentencePiece 的 `*.model`（走 unigram Viterbi，見 [`spm`]），也可以是
//! 只有詞表的 `vocab.json`（`0c.sentencepiece.vocab/1`，退回最長前綴匹配）。

pub mod candle;
pub mod error;
pub mod layout;
pub mod manifest;
pub mod punct;
mod punct_plan;
pub mod punct_zh;
pub mod punctuation_model;
pub mod rerank;
pub mod role;
pub mod runtime;
pub mod spm;
pub mod tokenizer;

pub use candle::{default_backend, set_default_backend, Backend};
pub use error::{ModelError, Result};
pub use manifest::{
    Availability, ModelSet, ModelSpec, DEFAULT_BOS_ID, DEFAULT_CANDIDATE_SLOT,
    DEFAULT_CONTEXT_TOKENS, DEFAULT_EOS_ID, DEFAULT_MODEL_FILE, DEFAULT_PAD_ID, DEFAULT_SEQ_LEN,
    DEFAULT_TIMEOUT_MS, DEFAULT_VOCAB_FILE, SET_SCHEMA,
};
pub use punct::{punctuate, IdentityPunctuator, Punctuator};
pub use punct_zh::ZhPunctuationModel;
pub use punctuation_model::PunctuationModel;
pub use rerank::Reranker;
pub use role::Role;
pub use runtime::{init_runtime, ModelRuntime, RuntimeStatus};
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
        assert_eq!(Role::JaPunct.as_str(), "ja-punct");
    }

    #[test]
    fn public_types_are_send() {
        fn assert_send<T: Send>() {}
        assert_send::<ModelSet>();
        assert_send::<Tokenizer>();
        assert_send::<Availability>();
        assert_send::<ModelRuntime>();
    }

    /// 沒有 ONNX Runtime 的環境也要能探測，只回報原因而不 panic。
    #[test]
    fn probe_never_panics_and_explains_failure() {
        let runtime = ModelRuntime::probe();
        match runtime.status() {
            RuntimeStatus::Available => assert!(runtime.reason().is_none()),
            RuntimeStatus::Unavailable { reason } => assert!(!reason.is_empty()),
        }
    }
}
