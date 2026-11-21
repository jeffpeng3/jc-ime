//! 引擎 pipe 的探測（檢查 6）。
//!
//! 用 `jc-windows` 的 client，與 `jc-tsf` 的 worker 走完全相同的路徑：連線時
//! 先送 `Hello` 握手，再送一個 `Key`。引擎**不在**是正常情況（還沒啟動或沒安裝），
//! 只印警告、不判定失敗。

use std::time::Duration;

use jc_proto::{Key, KeyCode, Request, Response, PROTOCOL_VERSION};
use jc_windows::Client;

/// 單次往返逾時；與 `jc-tsf` 一致。
const TIMEOUT: Duration = Duration::from_millis(500);
/// 連線嘗試次數。
const ATTEMPTS: usize = 3;
/// 兩次嘗試之間的間隔。
const BACKOFF: Duration = Duration::from_millis(200);

/// 握手時送出的客戶端名稱。
const CLIENT_NAME: &str = concat!("jc-tsf-smoke/", env!("CARGO_PKG_VERSION"));

/// 解析引擎 pipe 名稱；規則與 `jc-tsf`、`jc-engine` 共用 `jc-windows`。
///
/// 沒有另寫一份：`JC_IME_PIPE` > `HKCU\Software\jc-ime\EnginePipe` >
/// `jc-ime-<SID 尾段>`，取不到 SID 才退回 `jc-ime`。警告只在解析失敗時出現一次。
pub fn pipe_name() -> String {
    let resolved = jc_windows::resolve_pipe_name();
    if let Some(warning) = &resolved.warning {
        eprintln!("[jc-tsf-smoke 警告] {warning}");
    }
    resolved.name
}

/// 連上引擎、送 `Hello` 與一個 `Key`，回傳可以印出來的結果摘要。
///
/// 失敗一律當成「引擎不在」，由呼叫端印 `[SKIP]`；這裡不分辨「沒開」與「開了但
/// 壞掉」，因為兩者對免管理員冒煙來說都只是警告。
pub fn probe() -> Result<String, String> {
    let name = pipe_name();
    let mut client = Client::connect_with_retry(&name, CLIENT_NAME, TIMEOUT, ATTEMPTS, BACKOFF)
        .map_err(|error| format!("pipe {name} 連不上：{error}"))?;

    // 握手在 connect 內已經做過一次；再送一次 Hello 是為了把引擎的 Ready
    // 回應（版本與引擎名稱）印出來，引擎對重複的 Hello 只會再回一次 Ready。
    let hello = client
        .request(
            &Request::Hello {
                protocol: PROTOCOL_VERSION,
                client: CLIENT_NAME.to_string(),
            },
            TIMEOUT,
        )
        .map_err(|error| format!("Hello 往返失敗：{error}"))?;
    let hello = match hello {
        Response::Ready {
            protocol, engine, ..
        } => format!("Hello → 協定 {protocol}、引擎 {engine}"),
        other => format!("Hello → {}", describe(&other)),
    };

    // 送 Escape 而不是注音鍵：引擎若正好在別人的組字中，這個鍵只會清掉狀態，
    // 不會往使用者的文件插入字元。
    let key = client
        .request(
            &Request::Key {
                key: Key::new(KeyCode::Escape, None, false),
            },
            TIMEOUT,
        )
        .map_err(|error| format!("Key 往返失敗：{error}"))?;

    Ok(format!("{hello}；Key(Escape) → {}", describe(&key)))
}

/// 把回應壓成一行，避免把整個 `State` 的除錯輸出塞進報表。
fn describe(response: &Response) -> String {
    match response {
        Response::Ready {
            protocol, engine, ..
        } => format!("Ready（協定 {protocol}、引擎 {engine}）"),
        Response::State {
            preedit,
            candidates,
            ..
        } => format!("State（組字 {preedit:?}、候選 {} 個）", candidates.len()),
        Response::Committed { text } => format!("Committed（{text:?}）"),
        Response::Error { code, message } => format!("Error（{code:?}：{message}）"),
        // T3 擴充 op 的回報，smoke 只確認往返成功，不再往下拆欄位。
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 名稱解析共用 `jc-windows`，這裡只確認不會拿到空字串。
    #[test]
    fn pipe_name_is_never_empty() {
        assert!(!pipe_name().is_empty());
    }

    #[test]
    fn describe_summarizes_each_response() {
        let state = Response::State {
            preedit: "ㄨ".to_string(),
            cursor: 1,
            candidates: vec!["我".to_string(), "五".to_string()],
            details: Vec::new(),
            segments: Vec::new(),
            highlighted: 0,
            cursor_rune: 1,
            suggestion: None,
            label: None,
            navigating: false,
        };
        assert_eq!(describe(&state), "State（組字 \"ㄨ\"、候選 2 個）");
        assert_eq!(
            describe(&Response::Committed {
                text: "我".to_string()
            }),
            "Committed（\"我\"）"
        );
        assert_eq!(
            describe(&Response::Ready {
                protocol: 3,
                engine: "jc-engine 0.1.0".to_string(),
                session: 1,
            }),
            "Ready（協定 3、引擎 jc-engine 0.1.0）"
        );
    }
}
