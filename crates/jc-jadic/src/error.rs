//! 匯入與編碼的錯誤型別。
//!
//! 解析錯誤一律帶檔名與行號；編碼錯誤帶出問題的欄位與數值。

use std::path::PathBuf;

/// Mozc 匯入與 pack v2 編碼的錯誤。
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// 檔案無法開啟或讀取。
    #[error("讀取 {path} 失敗：{source}")]
    Io {
        /// 出錯的檔案路徑。
        path: PathBuf,
        /// 底層 I/O 錯誤。
        #[source]
        source: std::io::Error,
    },

    /// 來源檔某一行的格式不合法。
    #[error("{path} 第 {line} 行：{reason}")]
    Parse {
        /// 來源檔路徑。
        path: PathBuf,
        /// 1 起始的行號。
        line: usize,
        /// 失敗原因。
        reason: String,
    },

    /// `id.def` 的 context 數量不是預期的 2672。
    #[error("id.def 的 context_count 是 {actual}，預期 2672")]
    ContextCount {
        /// 實際算出的數量。
        actual: u32,
    },

    /// `id.def` 的 ID 重複、缺少或超出範圍。
    #[error("id.def 第 {line} 行：ID {id} 重複、缺少或超出 0..{context_count}")]
    BadId {
        /// 行號。
        line: usize,
        /// 出問題的 ID。
        id: u32,
        /// 已宣告的 context 數量。
        context_count: u32,
    },

    /// 連接矩陣的維度與 `id.def` 不一致。
    #[error("連接矩陣是 {actual}×{actual}，id.def 的 context_count 是 {expected}")]
    MatrixSize {
        /// 檔案宣告的維度。
        actual: u32,
        /// `id.def` 的 context 數量。
        expected: u32,
    },

    /// 連接矩陣的值超出 i16 合法範圍。
    #[error("{path} 第 {line} 行：連接成本 {value} 超出 -32767..32767")]
    ConnectionRange {
        /// 來源檔路徑。
        path: PathBuf,
        /// 1 起始的行號。
        line: usize,
        /// 實際數值。
        value: i64,
    },

    /// 詞條成本超出 i16 合法範圍。
    #[error("{path} 第 {line} 行：詞成本 {cost} 超出 -32767..32767")]
    CostRange {
        /// 來源檔路徑。
        path: PathBuf,
        /// 1 起始的行號。
        line: usize,
        /// 實際數值。
        cost: i64,
    },

    /// 詞條的左右 ID 超出 `id.def` 範圍。
    #[error("{path} 第 {line} 行：left_id {left} 或 right_id {right} 超出 0..{context_count}")]
    ContextIdRange {
        /// 來源檔路徑。
        path: PathBuf,
        /// 1 起始的行號。
        line: usize,
        /// 左 ID。
        left: u32,
        /// 右 ID。
        right: u32,
        /// 已宣告的 context 數量。
        context_count: u32,
    },

    /// 要寫進 pack 的數量超過欄位寬度。
    #[error("{field} 是 {value}，超過 {limit}")]
    CountOverflow {
        /// 欄位名稱。
        field: &'static str,
        /// 實際數量。
        value: usize,
        /// 欄位上限。
        limit: u64,
    },

    /// pack 的 section 偏移超過 u32。
    #[error("pack 的 {section} 偏移 {offset} 超過 u32")]
    OffsetOverflow {
        /// section 名稱。
        section: &'static str,
        /// 實際偏移。
        offset: usize,
    },

    /// 寫入輸出檔失敗。
    #[error("寫入 {path} 失敗：{source}")]
    Write {
        /// 輸出檔路徑。
        path: PathBuf,
        /// 底層 I/O 錯誤。
        #[source]
        source: std::io::Error,
    },
}

impl ImportError {
    /// 建立帶檔名與行號的解析錯誤。
    pub(crate) fn parse(
        path: impl Into<PathBuf>,
        line: usize,
        reason: impl Into<String>,
    ) -> ImportError {
        ImportError::Parse {
            path: path.into(),
            line,
            reason: reason.into(),
        }
    }
}
