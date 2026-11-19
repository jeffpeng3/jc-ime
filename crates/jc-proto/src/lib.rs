//! 引擎與 wrapper 之間的協定。長度前綴 + JSON，跨行程且可除錯。
//!
//! 訊框是 4-byte 小端長度前綴加上 JSON 內容；請求與回應都是列舉，以 `op`
//! 欄位區分。回應一律是完整快照，wrapper 不需要自己保存組字狀態。

pub mod frame;
pub mod message;

use serde::{Deserialize, Serialize};

pub use frame::{DEFAULT_MAX_FRAME, HEADER_LEN};
pub use jc_core::{Key, KeyCode, Snapshot};
pub use message::{read_message, read_payload, write_message};

/// 協定版本。握手時兩邊的版本必須相同。
///
/// 版本 2 在 [`Response::State`] 加上 `cursor_rune`、`suggestion`、`label` 與
/// `navigating`。這些欄位都有預設值，舊 wrapper 讀到的 JSON 形狀不變，但握手
/// 版本一定要是 2，不能靜默接受版本 1。
pub const PROTOCOL_VERSION: u32 = 2;

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
    ///
    /// 版本 2 起多了 `cursor_rune`、`suggestion`、`label` 與 `navigating`；
    /// 空值時不寫進 JSON，舊 wrapper 看到的形狀與版本 1 相同。
    State {
        preedit: String,
        cursor: usize,
        candidates: Vec<String>,
        /// 游標的 rune 位置；與 `cursor` 同值，供需要 rune 位移的外殼使用。
        #[serde(default, skip_serializing_if = "is_zero")]
        cursor_rune: usize,
        /// Tab 會接受的補全尾綴。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        suggestion: Option<String>,
        /// 目前高亮候選的顯示原因。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        /// 是否已進入明確的候選選取模式。
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        navigating: bool,
    },
    /// 一次提交的結果。
    Committed { text: String },
    /// 請求被拒絕或處理失敗。
    Error { code: ErrorCode, message: String },
}

/// serde 的 `skip_serializing_if`：值為 0 時省略。
fn is_zero(value: &usize) -> bool {
    *value == 0
}

impl Response {
    /// 建立一個錯誤回應。
    pub fn error(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::Error {
            code,
            message: message.into(),
        }
    }

    /// 由核心快照建立 [`Response::State`]。
    ///
    /// 協定的 `cursor` 沿用版本 1 的定義：組字字串結尾的字元數。需要游標
    /// 真實位置的外殼用 `cursor_rune`。目前核心的游標都在結尾，兩者相同。
    pub fn state(snapshot: jc_core::Snapshot) -> Self {
        let jc_core::Snapshot {
            preedit,
            cursor_rune,
            candidates,
            suggestion,
            label,
            navigating,
            ..
        } = snapshot;
        let cursor = preedit.chars().count();
        Self::State {
            cursor,
            candidates: candidates
                .into_iter()
                .map(|candidate| candidate.surface)
                .collect(),
            cursor_rune,
            suggestion,
            label,
            navigating,
            preedit,
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
        assert_eq!(text, r#"{"op":"hello","protocol":2,"client":"test"}"#);
        assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), hello);
    }

    #[test]
    fn protocol_version_is_two() {
        assert_eq!(PROTOCOL_VERSION, 2, "版本 2 才認得新的 State 欄位");
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

    #[test]
    fn state_omits_empty_optional_fields() {
        let resp = Response::state(jc_core::Snapshot::from_preedit("ㄇㄚ"));
        let text = serde_json::to_string(&resp).unwrap();
        assert_eq!(
            text,
            r#"{"op":"state","preedit":"ㄇㄚ","cursor":2,"candidates":[],"cursor_rune":2}"#
        );
    }

    #[test]
    fn state_serializes_suggestion_label_and_navigating() {
        let snapshot = jc_core::Snapshot {
            suggestion: Some("ㄚ".into()),
            label: Some("common".into()),
            navigating: true,
            ..jc_core::Snapshot::from_preedit("ㄇ")
        };
        let text = serde_json::to_string(&Response::state(snapshot)).unwrap();
        assert_eq!(
            text,
            r#"{"op":"state","preedit":"ㄇ","cursor":1,"candidates":[],"cursor_rune":1,"suggestion":"ㄚ","label":"common","navigating":true}"#
        );
    }

    #[test]
    fn state_accepts_version_one_shape() {
        let old = r#"{"op":"state","preedit":"我","cursor":1,"candidates":["我"]}"#;
        let resp: Response = serde_json::from_str(old).unwrap();
        assert_eq!(
            resp,
            Response::State {
                preedit: "我".into(),
                cursor: 1,
                candidates: vec!["我".into()],
                cursor_rune: 0,
                suggestion: None,
                label: None,
                navigating: false,
            }
        );
    }

    #[test]
    fn state_carries_candidate_surfaces() {
        let snapshot = jc_core::Snapshot {
            candidates: vec![
                jc_core::candidate::Candidate::new(
                    "我",
                    "ㄨㄛˇ",
                    1,
                    jc_core::candidate::CandidateSource::Lexicon,
                ),
                jc_core::candidate::Candidate::new(
                    "窩",
                    "ㄨㄛ",
                    2,
                    jc_core::candidate::CandidateSource::Lexicon,
                ),
            ],
            ..jc_core::Snapshot::from_preedit("ㄨㄛˇ")
        };
        match Response::state(snapshot) {
            Response::State {
                candidates,
                cursor,
                cursor_rune,
                ..
            } => {
                assert_eq!(candidates, ["我", "窩"]);
                assert_eq!(cursor, 3);
                assert_eq!(cursor_rune, 3);
            }
            other => panic!("應是 State，得到 {other:?}"),
        }
    }
}
