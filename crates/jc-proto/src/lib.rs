//! 引擎與 wrapper 之間的協定。長度前綴 + JSON，跨行程且可除錯。
//!
//! 骨架：先固定請求／回應的外形與編解碼，欄位在 M1 起逐步長出。

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

/// wrapper 送給引擎的請求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// 握手，確認版本與可用能力。
    Hello { protocol: u32 },
    /// 送出一個按鍵。
    Key { key: String },
    /// 取得目前組字狀態。
    Snapshot,
    /// 送出目前的組字結果。
    Commit,
}

/// 引擎回給 wrapper 的結果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub protocol: u32,
    pub preedit: String,
    pub cursor: usize,
    pub candidates: Vec<String>,
}

/// 以 4-byte 小端長度前綴包住 JSON 內容。
pub fn frame(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 4);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_prefixes_length() {
        let f = frame(b"{}");
        assert_eq!(&f[..4], &2u32.to_le_bytes());
        assert_eq!(&f[4..], b"{}");
    }

    #[test]
    fn request_round_trips() {
        let r = Request::Key { key: "a".into() };
        let text = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), r);
    }
}
