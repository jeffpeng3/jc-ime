//! 建置錯誤。
//!
//! 錯訊息帶上檔名或固定說明，讓失敗可以直接定位；不使用 `unwrap()`。

use std::path::PathBuf;

/// 英文詞表建置或驗證的錯誤。
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// 讀寫檔案失敗。
    #[error("讀取 {path} 失敗：{source}")]
    Io {
        /// 出錯的路徑。
        path: PathBuf,
        /// 底層錯誤。
        #[source]
        source: std::io::Error,
    },
    /// 來源內容不合法。
    #[error("英文來源不合法：{0}")]
    Source(String),
    /// 資料包內容不合法。
    #[error("英文資料包不合法：{0}")]
    Pack(String),
    /// manifest JSON 解析或輸出失敗。
    #[error("英文 manifest JSON 不合法：{0}")]
    Json(#[from] serde_json::Error),
}

impl BuildError {
    /// 建立帶路徑的讀寫錯誤。
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
