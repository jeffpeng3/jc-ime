//! 日文模型包的資料模型：manifest、可用性與安裝狀態。
//!
//! 對照前身 `internal/modelsets`。只處理資料與本機檔案；下載（`fetch.go`）與
//! Hugging Face pin（`pin.go`）屬於網路流程，不在這個 crate。

pub mod install;
pub mod manifest;

pub use install::{
    install, read_selection, selection_path, selection_result, InstallOptions, Selection,
    SelectionArtifact, SelectionAsset, SELECTION_SCHEMA,
};
pub use manifest::{
    dictionary_pack_id, dictionary_sha256, load_lock, parse_lock, Artifact, ArtifactAsset,
    ArtifactFile, ArtifactStatus, Deployment, Dictionary, HfFile, HfSource, License, Lock,
    ModelSetError, Options, Package, Source, Tensor, TensorIo, ValidationResult, FORMAT_FP32,
    FORMAT_INT8, KIND_PUNCTUATION, KIND_RERANKER, QUALITY_UNTRAINED, SCHEMA, VARIANT_CC_BY_SA,
    VARIANT_PUBLIC_CORE,
};

#[cfg(test)]
mod tests;
