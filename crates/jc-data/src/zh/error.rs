//! 注音資料包的讀取錯誤。
//!
//! 訊息盡量帶上檔案、位移或欄位名稱，讓損毀的資料包能在載入時就指出
//! 是哪一段壞掉，而不是查詢時才 panic。

use std::path::PathBuf;

use thiserror::Error;

/// 注音資料包（`lexicon.bin`、`lm.bin`、`manifest.json`）的錯誤。
#[derive(Debug, Error)]
pub enum ZhDataError {
    /// 檔案開不起來或讀不到。
    #[error("讀取 {path} 失敗：{source}")]
    Io {
        /// 出問題的路徑。
        path: PathBuf,
        /// 底層 I/O 錯誤。
        #[source]
        source: std::io::Error,
    },
    /// 檔案長度不足以容納魔術字、版本與 CRC。
    #[error("資料檔太小：{size} 位元組，至少需要 {minimum}")]
    TooSmall {
        /// 實際長度。
        size: usize,
        /// 最低需求。
        minimum: usize,
    },
    /// 魔術字不符。
    #[error("不是預期的資料檔：魔術字是 {actual}，需要 {expected}")]
    BadMagic {
        /// 預期的魔術字（可讀文字）。
        expected: &'static str,
        /// 檔案開頭實際的位元組。
        actual: String,
    },
    /// 格式版本不符。
    #[error("資料檔版本 {actual}，程式需要 {expected}")]
    UnsupportedVersion {
        /// 檔案版本。
        actual: u32,
        /// 程式支援的版本。
        expected: u32,
    },
    /// CRC32 不符。
    #[error("資料檔損毀：CRC32 是 {actual:08x}，檔尾記錄 {expected:08x}")]
    CrcMismatch {
        /// 檔尾記錄的值。
        expected: u32,
        /// 實際算出的值。
        actual: u32,
    },
    /// 區段長度不足。
    #[error("資料檔截斷")]
    Truncated,
    /// 讀完所有區段後還有剩餘位元組。
    #[error("資料檔結尾有 {0} 位元組未讀")]
    TrailingBytes(usize),
    /// 某個字串區段不是合法 UTF-8。
    #[error("{what} 不是合法 UTF-8")]
    InvalidUtf8 {
        /// 區段名稱。
        what: &'static str,
    },
    /// 欄位之間的長度、位移或排序不一致。
    #[error("資料檔版面不合法：{0}")]
    Layout(String),
    /// `manifest.json` 解析失敗。
    #[error("讀取 {path} 失敗：{source}")]
    BadManifest {
        /// manifest 路徑。
        path: PathBuf,
        /// JSON 解析錯誤。
        #[source]
        source: serde_json::Error,
    },
    /// 檔案內容與 manifest 記錄不符。
    #[error("{file} 的 {field} 不符：manifest 記錄 {expected}，實際 {actual}")]
    ManifestMismatch {
        /// 檔名。
        file: String,
        /// 比對的欄位。
        field: &'static str,
        /// manifest 的值。
        expected: String,
        /// 實際檔案的值。
        actual: String,
    },
}
