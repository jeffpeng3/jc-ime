//! 引擎連線失敗的分類。
//!
//! 以前只印「連不上」，但「pipe 不存在」、「CreateFileW 被拒」、「握手失敗」與
//! 「協定版本不符」的修復方式完全不同：前者是引擎沒啟動，第二個是權限或安全
//! 原則，第三個是引擎與 DLL 不相容。這個模組把判斷與文案集中起來，Linux 的
//! 單元測試也能驗證（不碰 `jc-windows` 的型別）。

/// 引擎連線失敗的種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// pipe 不存在：引擎沒啟動，或 Run 機碼還沒生效。
    PipeMissing,
    /// `CreateFileW` 回 `ERROR_ACCESS_DENIED`。
    AccessDenied,
    /// 其他連線錯誤（pipe 忙碌、名稱無效、路徑不存在等）。
    ConnectFailed,
    /// 連上了，但 `Hello` 握手失敗（讀寫錯誤、訊框壞掉或引擎回錯誤）。
    HandshakeFailed,
    /// 握手時協定版本不符。
    ProtocolMismatch,
    /// 握手或往返逾時。
    Timeout,
}

impl FailureKind {
    /// 給 JSON／人類輸出用的穩定名稱。
    pub fn code(self) -> &'static str {
        match self {
            Self::PipeMissing => "pipe-missing",
            Self::AccessDenied => "access-denied",
            Self::ConnectFailed => "connect-failed",
            Self::HandshakeFailed => "handshake-failed",
            Self::ProtocolMismatch => "protocol-mismatch",
            Self::Timeout => "timeout",
        }
    }
}

/// 把 `CreateFileW` 的 Win32 錯誤碼轉成分類。
///
/// 2／3 是「找不到檔案／路徑」，對 named pipe 而言就是引擎沒在聽；5 是存取被拒。
pub fn classify_connect_os_error(code: Option<i32>) -> FailureKind {
    match code {
        Some(2 | 3) => FailureKind::PipeMissing,
        Some(5) => FailureKind::AccessDenied,
        _ => FailureKind::ConnectFailed,
    }
}

/// 產生一行可讀的失敗說明；`message` 是底層錯誤字串。
pub fn describe(kind: FailureKind, pipe: &str, message: &str) -> String {
    match kind {
        FailureKind::PipeMissing => {
            format!("pipe {pipe} 不存在（引擎未啟動，或開機 Run 機碼還沒生效）；{message}")
        }
        FailureKind::AccessDenied => format!(
            "pipe {pipe} 存在但 CreateFileW 被拒（錯誤 5：存取被拒）；\
             可能是其他使用者或安全性原則佔用；{message}"
        ),
        FailureKind::ConnectFailed => format!("pipe {pipe} 連線失敗：{message}"),
        FailureKind::HandshakeFailed => format!(
            "pipe {pipe} 已連上，但 Hello 握手失敗：{message}；\
             引擎與 jc-tsf.dll 可能不是同一個版本"
        ),
        FailureKind::ProtocolMismatch => format!(
            "pipe {pipe} 握手時協定版本不符：{message}；\
             請用同一個安裝包更新 jc-engine.exe 與 jc-tsf.dll"
        ),
        FailureKind::Timeout => format!(
            "pipe {pipe} 握手或往返逾時（500 ms）：{message}；引擎可能卡住或正在服務另一條連線"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_error_codes_map_to_distinct_kinds() {
        assert_eq!(classify_connect_os_error(Some(2)), FailureKind::PipeMissing);
        assert_eq!(classify_connect_os_error(Some(3)), FailureKind::PipeMissing);
        assert_eq!(
            classify_connect_os_error(Some(5)),
            FailureKind::AccessDenied
        );
        assert_eq!(
            classify_connect_os_error(Some(231)),
            FailureKind::ConnectFailed
        );
        assert_eq!(classify_connect_os_error(None), FailureKind::ConnectFailed);
    }

    #[test]
    fn descriptions_name_the_failure_kind() {
        let pipe = "jc-ime-1001";
        assert!(describe(FailureKind::PipeMissing, pipe, "x").contains("不存在"));
        assert!(describe(FailureKind::AccessDenied, pipe, "x").contains("存取被拒"));
        assert!(describe(FailureKind::HandshakeFailed, pipe, "x").contains("握手失敗"));
        assert!(describe(FailureKind::ProtocolMismatch, pipe, "x").contains("協定版本不符"));
        assert!(describe(FailureKind::Timeout, pipe, "x").contains("逾時"));
    }

    #[test]
    fn codes_are_stable() {
        assert_eq!(FailureKind::PipeMissing.code(), "pipe-missing");
        assert_eq!(FailureKind::AccessDenied.code(), "access-denied");
        assert_eq!(FailureKind::ProtocolMismatch.code(), "protocol-mismatch");
    }
}
