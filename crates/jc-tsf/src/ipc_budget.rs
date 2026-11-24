//! 一個 IPC 請求的整體預算分類。
//!
//! 單次 I/O 的逾時仍由 `ipc.rs` 的 [`crate::ipc::IO_TIMEOUT_MS`] 控制；這裡只決定
//! 「這個請求最多等多久」。兩者分開，才能讓收尾請求快速放行，互動請求仍有足夠
//! 時間完成組字。
//!
//! 分類刻意集中在 [`request_budget_ms`]，避免每個呼叫端各自記一組 op 清單。

use std::borrow::Cow;

use serde_json::Value;

use crate::session::TEARDOWN_HINT;
/// 收尾／探測請求的整體預算。
///
/// `Session::drop`、切換視窗與 `doctor` 都會送這些請求；引擎卡死時最多只等 1 秒，
/// 不讓宿主關窗或 TIP 卸載卡住。逾時後照舊不重播，下一次用 snapshot 對齊。
pub const TEARDOWN_TIMEOUT_MS: u32 = 1_000;

/// 互動請求的整體預算。
///
/// `key`／`toggle`／`commit`／`snapshot` 可能牽涉組字與候選計算，10 秒是給慢機器
/// 的上限；不因為超過 1 秒就中止，因為中止會讓引擎狀態與外殼不同步。
pub const HANDLE_TIMEOUT_MS: u32 = 10_000;

/// 這個請求要用哪一個整體預算（毫秒）。
///
/// `close`／`display`／`health`／`hello` 是收尾或探測，用短預算；其餘（包含
/// `key`、`toggle`、`commit`、`snapshot`）維持 10 秒。`display` 在協定上轉成
/// `snapshot`，所以由 `session` 帶 [`TEARDOWN_HINT`] 標記，避免和互動用的
/// snapshot 混在一起。
pub fn request_budget_ms(request: &Value) -> u32 {
    let op = request["op"].as_str().unwrap_or("");
    let teardown = request[TEARDOWN_HINT].as_bool() == Some(true)
        || matches!(op, "close" | "display" | "health" | "hello");
    if teardown {
        TEARDOWN_TIMEOUT_MS
    } else {
        HANDLE_TIMEOUT_MS
    }
}

/// 送訊框前拿掉內部預算標記；沒有標記時不複製請求。
pub fn strip_teardown_hint(request: &Value) -> Cow<'_, Value> {
    if request.get(TEARDOWN_HINT).is_none() {
        return Cow::Borrowed(request);
    }
    let mut cleaned = request.clone();
    if let Some(map) = cleaned.as_object_mut() {
        map.remove(TEARDOWN_HINT);
    }
    Cow::Owned(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn teardown_requests_get_one_second() {
        for op in ["close", "display", "health", "hello"] {
            assert_eq!(
                request_budget_ms(&json!({"op": op})),
                TEARDOWN_TIMEOUT_MS,
                "{op}"
            );
        }
        // 0c 的 display 在線路上是 snapshot，靠內部標記維持短預算。
        assert_eq!(
            request_budget_ms(&json!({"op": "snapshot", TEARDOWN_HINT: true})),
            TEARDOWN_TIMEOUT_MS
        );
    }

    #[test]
    fn interactive_requests_get_ten_seconds() {
        for op in ["key", "toggle", "commit", "snapshot"] {
            assert_eq!(
                request_budget_ms(&json!({"op": op})),
                HANDLE_TIMEOUT_MS,
                "{op}"
            );
        }
        assert_eq!(TEARDOWN_TIMEOUT_MS, 1_000);
        assert_eq!(HANDLE_TIMEOUT_MS, 10_000);
    }

    #[test]
    fn teardown_hint_never_reaches_the_engine() {
        let request = json!({"op": "snapshot", TEARDOWN_HINT: true});
        assert_eq!(request_budget_ms(&request), TEARDOWN_TIMEOUT_MS);
        let wire = strip_teardown_hint(&request);
        assert_eq!(wire.as_ref(), &json!({"op": "snapshot"}));
        assert!(wire.get(TEARDOWN_HINT).is_none());
    }
}
