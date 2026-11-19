//! 錯誤型別。
//!
//! 模型缺席不是錯誤：那由 [`crate::Availability::Unavailable`] 表示。
//! 這裡只涵蓋設定檔損壞、I/O 失敗與（開啟 `onnx` 時）推論失敗。

/// jc-model 的錯誤。
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    /// 讀寫檔案失敗。
    #[error("I/O 失敗：{0}")]
    Io(#[from] std::io::Error),
    /// manifest.json 不是合法 JSON 或違反格式契約。
    #[error("模型 manifest 錯誤：{0}")]
    Manifest(String),
    /// vocab.json 不是合法 JSON 或違反格式契約。
    #[error("tokenizer 錯誤：{0}")]
    Tokenizer(String),
    /// ONNX Runtime 無法初始化。
    #[cfg(feature = "onnx")]
    #[error("ONNX Runtime 初始化失敗：{0}")]
    Runtime(String),
    /// ONNX 推論失敗。
    #[cfg(feature = "onnx")]
    #[error("ONNX 推論失敗：{0}")]
    Inference(String),
}

impl ModelError {
    /// 建立 manifest 錯誤。
    pub fn manifest(msg: impl Into<String>) -> Self {
        Self::Manifest(msg.into())
    }

    /// 建立 tokenizer 錯誤。
    pub fn tokenizer(msg: impl Into<String>) -> Self {
        Self::Tokenizer(msg.into())
    }
}

/// jc-model 的結果別名。
pub type Result<T> = std::result::Result<T, ModelError>;
