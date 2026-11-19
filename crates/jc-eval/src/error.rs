//! 評測 crate 的錯誤型別。
//!
//! 所有讀取題庫的錯誤都帶檔案路徑；JSON 解析錯誤另外由 `serde_json` 提供
//! 行號與欄位。

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use thiserror::Error;

/// 評測流程的錯誤。
#[derive(Debug, Error)]
pub enum EvalError {
    /// 檔案讀取失敗。
    #[error("{path}：讀取失敗：{source}")]
    Io {
        /// 發生錯誤的檔案。
        path: PathBuf,
        /// 底層 I/O 錯誤。
        #[source]
        source: std::io::Error,
    },
    /// JSON 解析失敗；訊息含行號與欄位。
    #[error("{path}：JSON 解析失敗：{source}")]
    Json {
        /// 發生錯誤的檔案。
        path: PathBuf,
        /// `serde_json` 的解析錯誤。
        #[source]
        source: serde_json::Error,
    },
    /// 題庫內容不一致；`message` 會帶題號或案例名稱。
    #[error("{path}：{message}")]
    Invalid {
        /// 出問題的檔案。
        path: PathBuf,
        /// 具體原因。
        message: String,
    },
    /// 輸出報告時序列化失敗。
    #[error("報告序列化失敗：{0}")]
    Encode(String),
}

impl EvalError {
    /// 建立內容不一致的錯誤。
    pub fn invalid(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::Invalid {
            path: path.into(),
            message: message.into(),
        }
    }
}

/// 評測 crate 的結果型別。
pub type Result<T> = std::result::Result<T, EvalError>;

/// 讀取整個檔案；錯誤帶路徑。
pub(crate) fn read_bytes(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|source| EvalError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// 把 JSON 位元組解析成 `T`；錯誤帶路徑與行號。
pub(crate) fn parse_json<T: DeserializeOwned>(path: &Path, raw: &[u8]) -> Result<T> {
    serde_json::from_slice(raw).map_err(|source| EvalError::Json {
        path: path.to_path_buf(),
        source,
    })
}
