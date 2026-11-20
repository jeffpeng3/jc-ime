//! 冒煙程式的錯誤型別。
//!
//! 每個錯誤都帶著可以印出來的 HRESULT 或 Win32 錯誤碼；檢查流程只負責決定這一
//! 項要算 `[FAIL]` 還是 `[SKIP]`。

use windows::core::HRESULT;

use crate::report;

/// 冒煙流程的錯誤。
#[derive(Debug, thiserror::Error)]
pub enum SmokeError {
    /// `LoadLibraryW` 失敗。
    #[error("LoadLibraryW({path}) 失敗，錯誤碼 {code}")]
    Load {
        /// 嘗試載入的路徑。
        path: String,
        /// Win32 錯誤碼（十六進位）與系統訊息。
        code: String,
    },
    /// 找不到 DLL 的匯出函式。
    #[error("找不到匯出 {0}")]
    MissingExport(&'static str),
    /// 某個 COM／TSF 呼叫回失敗的 `HRESULT`。
    #[error("{call} 回 HRESULT {code}")]
    Hresult {
        /// 呼叫的名稱。
        call: &'static str,
        /// `HRESULT` 的十六進位值。
        code: String,
    },
    /// 註冊表讀取失敗。
    #[error("註冊表讀取失敗：{0}")]
    Registry(String),
}

impl SmokeError {
    /// 由 `HRESULT` 建立錯誤。
    pub fn hresult(call: &'static str, hresult: HRESULT) -> Self {
        Self::Hresult {
            call,
            code: report::hex(hresult.0 as u32),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hresult_error_carries_hex_code() {
        let error = SmokeError::hresult("DllGetClassObject", HRESULT(0x8004_0111u32 as i32));
        assert_eq!(error.to_string(), "DllGetClassObject 回 HRESULT 0x80040111");
    }
}
