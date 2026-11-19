//! ONNX Runtime 接線（`ort` crate）。模型維持 ONNX，不進 git。
//!
//! 預設不啟用 `onnx` feature：建置時不拉入 `ort`、不下載也不連結
//! ONNX Runtime，公開 API 也不會出現任何 `ort` 型別。引擎在模型缺席時
//! 一律退回統計路徑。
//!
//! 啟用 `onnx` 後提供：
//!
//! - [`Reranker`]：日文候選重排，`score(left, candidate, right) -> Option<f32>`。
//! - [`PunctuationModel`]：日文標點推論（`punctuate` 仍是 identity stub）。
//!
//! 模型目錄預設放 `model.int8.onnx` 與 `vocab.json`；`vocab.json` 是
//! `0c.sentencepiece.vocab/1`，只有詞表、沒有 piece 分數，因此
//! [`Tokenizer`] 採最長前綴匹配，詳見模組文件。

pub mod error;
pub mod layout;
pub mod manifest;
pub mod punct;
pub mod role;
pub mod tokenizer;

#[cfg(feature = "onnx")]
pub mod punctuation_model;
#[cfg(feature = "onnx")]
pub mod rerank;
#[cfg(feature = "onnx")]
pub mod runtime;

pub use error::{ModelError, Result};
pub use manifest::{
    Availability, ModelSet, ModelSpec, DEFAULT_BOS_ID, DEFAULT_CANDIDATE_SLOT,
    DEFAULT_CONTEXT_TOKENS, DEFAULT_EOS_ID, DEFAULT_MODEL_FILE, DEFAULT_PAD_ID, DEFAULT_SEQ_LEN,
    DEFAULT_TIMEOUT_MS, DEFAULT_VOCAB_FILE, SET_SCHEMA,
};
pub use punct::{punctuate, IdentityPunctuator, Punctuator};
pub use role::Role;
pub use tokenizer::Tokenizer;

#[cfg(feature = "onnx")]
pub use punctuation_model::PunctuationModel;
#[cfg(feature = "onnx")]
pub use rerank::Reranker;
#[cfg(feature = "onnx")]
pub use runtime::init_runtime;

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

    /// 預設 feature 下公開 API 只由本地型別組成；`Reranker` 等 ort 接線
    /// 必須完全不存在。這個測試在開啟 `onnx` 時改驗證型別可用。
    #[test]
    fn public_types_stay_feature_gated() {
        fn assert_send<T: Send>() {}
        assert_send::<ModelSet>();
        assert_send::<Tokenizer>();
        assert_send::<Availability>();
        #[cfg(feature = "onnx")]
        assert_send::<Reranker>();
    }
}
