//! 引擎與 wrapper 之間的協定。長度前綴 + JSON，跨行程且可除錯。
//!
//! 訊框是 4-byte 小端長度前綴加上 JSON 內容；請求與回應都是列舉，以 `op`
//! 欄位區分。回應一律是完整快照，wrapper 不需要自己保存組字狀態。

pub mod frame;
pub mod message;

use serde::{Deserialize, Serialize};

pub use frame::{DEFAULT_MAX_FRAME, HEADER_LEN};
pub use jc_core::{Key, KeyCode};
pub use message::{read_message, read_payload, write_message};

/// 協定版本。握手時兩邊的版本必須相同。
pub const PROTOCOL_VERSION: u32 = 1;

/// 引擎端回報的錯誤種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// 握手的協定版本與 [`PROTOCOL_VERSION`] 不同。
    VersionMismatch,
    /// JSON 內容無法解碼，或缺少必要欄位。
    BadFrame,
    /// 訊框超過長度上限。
    FrameTooLarge,
    /// `op` 不是引擎認得的操作。
    UnknownOp,
    /// 引擎內部狀態不允許這個請求。
    Internal,
}

/// wrapper 送給引擎的請求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// 握手，確認版本與客戶端身分。
    Hello { protocol: u32, client: String },
    /// 送出一個按鍵。
    Key { key: Key },
    /// 取得目前組字狀態。
    Snapshot,
    /// 送出目前的組字結果。
    Commit,
    /// 要求引擎結束行程。
    Shutdown,
    /// 引擎不認得的操作。由 serde 的 `other` 收下，讓引擎可以回
    /// [`ErrorCode::UnknownOp`] 而不是直接斷線。
    #[serde(other)]
    Unknown,
}

/// 引擎回給 wrapper 的結果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Response {
    /// 握手成功，附上引擎識別字串。
    Ready { protocol: u32, engine: String },
    /// 目前的完整組字快照。`cursor` 單位是字元（Unicode scalar value）。
    State {
        preedit: String,
        cursor: usize,
        candidates: Vec<String>,
    },
    /// 一次提交的結果。
    Committed { text: String },
    /// 請求被拒絕或處理失敗。
    Error { code: ErrorCode, message: String },
}

impl Response {
    /// 建立一個錯誤回應。
    pub fn error(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::Error {
            code,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_uses_op_tag() {
        let text = serde_json::to_string(&Request::Snapshot).unwrap();
        assert_eq!(text, r#"{"op":"snapshot"}"#);

        let hello = Request::Hello {
            protocol: PROTOCOL_VERSION,
            client: "test".into(),
        };
        let text = serde_json::to_string(&hello).unwrap();
        assert_eq!(
            text,
            format!(r#"{{"op":"hello","protocol":1,"client":"test"}}"#)
        );
        assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), hello);
    }

    #[test]
    fn key_round_trips() {
        for key in [
            Key::char('ㄇ', false),
            Key::new(KeyCode::Backspace, None, false),
        ] {
            let req = Request::Key { key };
            let text = serde_json::to_string(&req).unwrap();
            assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), req);
        }
    }

    #[test]
    fn key_uses_core_shape() {
        let text = serde_json::to_string(&Request::Key {
            key: Key::char('ㄇ', false),
        })
        .unwrap();
        assert_eq!(
            text,
            r#"{"op":"key","key":{"code":{"kind":"char","value":"ㄇ"},"text":"ㄇ","shift":false}}"#
        );
    }

    #[test]
    fn unknown_op_is_captured_instead_of_failing() {
        let req: Request = serde_json::from_str(r#"{"op":"teleport"}"#).unwrap();
        assert_eq!(req, Request::Unknown);
    }

    #[test]
    fn error_codes_are_snake_case() {
        let resp = Response::error(ErrorCode::VersionMismatch, "版本不符");
        let text = serde_json::to_string(&resp).unwrap();
        assert_eq!(
            text,
            r#"{"op":"error","code":"version_mismatch","message":"版本不符"}"#
        );
        assert_eq!(serde_json::from_str::<Response>(&text).unwrap(), resp);
    }
}
