//! 引擎 pipe 的探測（檢查 6 與 12）。
//!
//! 用 `jc-windows` 的 client，與 `jc-tsf` 的 worker 走完全相同的路徑：連線時
//! 先送 `Hello` 握手，再送一個 `Key`，最後問 `Health`。引擎**不在**是正常情況
//! （還沒啟動或沒安裝），只印警告、不判定失敗；但連不上的原因要分清楚：
//! pipe 不存在、`CreateFileW` 被拒、握手失敗、協定版本不符，四種的修復方式不同。
//!
//! 連線與握手刻意分成兩步（`LocalSocket::connect` + `Client::handshake`），才能
//! 區分「連不上 pipe」與「連上了但握手失敗」。

use std::time::Duration;

use jc_proto::{HealthInfo, Key, KeyCode, Request, Response, PROTOCOL_VERSION};
use jc_windows::{Client, Error, LocalSocket};

use crate::engine_failure::{self, FailureKind};

/// 單次往返逾時；與 `jc-tsf` 一致。
const TIMEOUT: Duration = Duration::from_millis(500);
/// 連線嘗試次數。
const ATTEMPTS: usize = 3;
/// 兩次嘗試之間的間隔。
const BACKOFF: Duration = Duration::from_millis(200);

/// 握手時送出的客戶端名稱。
const CLIENT_NAME: &str = concat!("jc-tsf-smoke/", env!("CARGO_PKG_VERSION"));

/// 連上引擎後拿到的資訊。
#[derive(Debug, Clone, PartialEq)]
pub struct EngineInfo {
    /// 引擎回報的協定版本。
    pub protocol: u32,
    /// 引擎識別字串，例如 `jc-engine 0.1.4`。
    pub engine: String,
    /// 這條連線的 session id。
    pub session: u64,
    /// `Health` 的回報；失敗時是錯誤字串。
    pub health: Result<HealthInfo, String>,
    /// `Key(Escape)` 的回應摘要。
    pub key: String,
}

/// 引擎探測結果。
#[derive(Debug, Clone, PartialEq)]
pub enum Probe {
    /// 連線、握手與 `Key` 都成功。
    ///
    /// 用 `Box` 裝起來：`EngineInfo` 內含 `HealthInfo`，直接內嵌會讓兩個變體的
    /// 大小差距超過 clippy 門檻。
    Connected(Box<EngineInfo>),
    /// 連不上或握手失敗；`detail` 已分類並可直接輸出。
    Unavailable {
        /// 失敗種類。
        kind: FailureKind,
        /// 一行說明。
        detail: String,
    },
}

impl Probe {
    /// 檢查 6 的摘要。
    pub fn pipe_detail(&self) -> String {
        match self {
            Self::Connected(info) => format!(
                "Hello → 協定 {}、引擎 {}、session {}；Key(Escape) → {}",
                info.protocol, info.engine, info.session, info.key
            ),
            Self::Unavailable { detail, .. } => detail.clone(),
        }
    }

    /// 檢查 12 的摘要；連不上時回分類後的說明。
    pub fn engine_detail(&self) -> Result<String, String> {
        let info = match self {
            Self::Connected(info) => info,
            Self::Unavailable { kind, detail } => {
                return Err(format!("引擎連不上（{}）：{detail}", kind.code()))
            }
        };
        let health = match &info.health {
            Ok(health) => health,
            Err(error) => return Err(format!("引擎已連上，但 Health 失敗：{error}")),
        };
        let state = if health.draining {
            "交接中（draining=true）"
        } else {
            "正常"
        };
        let mut text = format!(
            "引擎 {}；協定 {}；session {}；健康狀態 {state}；連線 {}、組字中 {}、引擎 session 數 {}",
            info.engine,
            info.protocol,
            info.session,
            health.connections,
            health.composing,
            health.sessions
        );
        if let Some(pid) = health.pid {
            text.push_str(&format!("、pid {pid}"));
        }
        text.push_str(&format!("、英文詞表 {} 詞", health.english_words));
        Ok(text)
    }

    /// `Health` 回報是否載入 bigram；沒有 health 或引擎太舊時是 `None`。
    pub fn has_bigram(&self) -> Option<bool> {
        let Self::Connected(info) = self else {
            return None;
        };
        let Ok(health) = &info.health else {
            return None;
        };
        health.capabilities.get("has_bigram").copied()
    }

    /// `Health` 回報的英文詞數；沒有 health 或引擎太舊時是 `None`。
    pub fn english_words(&self) -> Option<usize> {
        let Self::Connected(info) = self else {
            return None;
        };
        let Ok(health) = &info.health else {
            return None;
        };
        (health.english_words > 0).then_some(health.english_words)
    }

    /// 引擎是否在交接中（`Health.draining`）。
    pub fn draining(&self) -> bool {
        let Self::Connected(info) = self else {
            return false;
        };
        matches!(&info.health, Ok(HealthInfo { draining: true, .. }))
    }

    /// 最近一次慢請求 `(op, 毫秒, 輸入長度)`；沒有慢請求或 health 不可用時為 `None`。
    pub fn last_slow(&self) -> Option<(String, u64, usize)> {
        let Self::Connected(info) = self else {
            return None;
        };
        let Ok(health) = &info.health else {
            return None;
        };
        let op = health.last_slow_op.clone()?;
        Some((
            op,
            health.last_slow_ms.unwrap_or(0),
            health.last_slow_input_len.unwrap_or(0),
        ))
    }
}

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

/// 連上引擎、握手、送 `Hello`／`Key(Escape)`／`Health`。
pub fn probe() -> Probe {
    let name = pipe_name();

    let socket = match connect(&name) {
        Ok(socket) => socket,
        Err((kind, message)) => {
            return Probe::Unavailable {
                kind,
                detail: engine_failure::describe(kind, &name, &message),
            }
        }
    };

    let mut client = match Client::handshake(socket, CLIENT_NAME, TIMEOUT) {
        Ok(client) => client,
        Err(error) => {
            let kind = classify_handshake(&error);
            return Probe::Unavailable {
                kind,
                detail: engine_failure::describe(kind, &name, &error.to_string()),
            };
        }
    };

    // 握手已經做過一次 Hello；再送一次是為了拿到引擎名稱與 session id。
    let (protocol, engine, session) = match client.request(
        &Request::Hello {
            protocol: PROTOCOL_VERSION,
            client: CLIENT_NAME.to_string(),
        },
        TIMEOUT,
    ) {
        Ok(Response::Ready {
            protocol,
            engine,
            session,
            ..
        }) => (protocol, engine, session),
        Ok(other) => {
            return Probe::Unavailable {
                kind: FailureKind::HandshakeFailed,
                detail: format!(
                    "pipe {name} 已連上，但 Hello 收到非 Ready 的回應：{}",
                    describe(&other)
                ),
            }
        }
        Err(error) => return unavailable_request(&name, "Hello", &error),
    };

    // 送 Escape 而不是注音鍵：引擎若正好在別人的組字中，這個鍵只會清掉狀態，
    // 不會往使用者的文件插入字元。
    let key = match client.request(
        &Request::Key {
            key: Key::new(KeyCode::Escape, None, false),
        },
        TIMEOUT,
    ) {
        Ok(response) => describe(&response),
        Err(error) => return unavailable_request(&name, "Key(Escape)", &error),
    };

    // Health 放最後：它失敗時連線可能已不可信，不能影響前面的 Key 判定。
    let health = match client.request(&Request::Health, TIMEOUT) {
        Ok(Response::Health { info }) => Ok(info),
        Ok(other) => Err(format!("Health 收到非預期回應：{}", describe(&other))),
        Err(error) => Err(error.to_string()),
    };

    Probe::Connected(Box::new(EngineInfo {
        protocol,
        engine,
        session,
        health,
        key,
    }))
}

/// 連線（含重試），回傳 socket 或（分類, 錯誤訊息）。
fn connect(name: &str) -> Result<LocalSocket, (FailureKind, String)> {
    let mut last: Option<(FailureKind, String)> = None;
    for attempt in 0..ATTEMPTS.max(1) {
        match LocalSocket::connect(name, TIMEOUT) {
            Ok(socket) => return Ok(socket),
            Err(Error::Io(error)) => {
                let kind = engine_failure::classify_connect_os_error(error.raw_os_error());
                last = Some((kind, error.to_string()));
            }
            Err(error) => {
                last = Some((FailureKind::ConnectFailed, error.to_string()));
            }
        }
        if attempt + 1 < ATTEMPTS {
            std::thread::sleep(BACKOFF);
        }
    }
    Err(last.unwrap_or_else(|| (FailureKind::ConnectFailed, "重連次數必須大於 0".to_string())))
}

/// 握手失敗的分類。
fn classify_handshake(error: &Error) -> FailureKind {
    match error {
        Error::VersionMismatch { .. } => FailureKind::ProtocolMismatch,
        Error::Timeout => FailureKind::Timeout,
        Error::Protocol(_) | Error::BadFrame(_) | Error::Io(_) => FailureKind::HandshakeFailed,
        Error::Unsupported => FailureKind::ConnectFailed,
    }
}

/// 連線後的請求失敗（Hello／Key）。
fn unavailable_request(name: &str, request: &str, error: &Error) -> Probe {
    let kind = classify_handshake(error);
    let detail = match kind {
        FailureKind::ProtocolMismatch => engine_failure::describe(kind, name, &error.to_string()),
        FailureKind::Timeout => {
            format!("pipe {name} 已連上，但 {request} 逾時（500 ms）：{error}")
        }
        _ => format!("pipe {name} 已連上，但 {request} 往返失敗：{error}"),
    };
    Probe::Unavailable { kind, detail }
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
        Response::Health { info } => format!(
            "Health（連線 {}、session {}、組字中 {}）",
            info.connections, info.sessions, info.composing
        ),
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
            handled: None,
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
                bindings: Default::default(),
            }),
            "Ready（協定 3、引擎 jc-engine 0.1.0）"
        );
    }

    #[test]
    fn engine_detail_reports_health_and_sessions() {
        let probe = Probe::Connected(Box::new(EngineInfo {
            protocol: PROTOCOL_VERSION,
            engine: "jc-engine 0.1.4".to_string(),
            session: 2,
            health: Ok(HealthInfo {
                connections: 1,
                sessions: 3,
                composing: 1,
                pid: Some(4242),
                ..HealthInfo::default()
            }),
            key: "State（組字 \"\"、候選 0 個）".to_string(),
        }));
        let detail = probe.engine_detail().unwrap();
        assert!(detail.contains("jc-engine 0.1.4"), "{detail}");
        assert!(detail.contains("session 2"), "{detail}");
        assert!(detail.contains("引擎 session 數 3"), "{detail}");
        assert!(detail.contains("pid 4242"), "{detail}");
        assert!(detail.contains("健康狀態 正常"), "{detail}");
        assert!(!probe.draining());
    }

    #[test]
    fn draining_engine_is_flagged() {
        let probe = Probe::Connected(Box::new(EngineInfo {
            protocol: PROTOCOL_VERSION,
            engine: "jc-engine 0.1.4".to_string(),
            session: 1,
            health: Ok(HealthInfo {
                draining: true,
                ..HealthInfo::default()
            }),
            key: "State".to_string(),
        }));
        assert!(probe.draining());
        assert!(probe.engine_detail().unwrap().contains("draining=true"));
    }

    #[test]
    fn unavailable_engine_detail_keeps_classification() {
        let probe = Probe::Unavailable {
            kind: FailureKind::PipeMissing,
            detail: "pipe jc-ime-1001 不存在".to_string(),
        };
        assert!(probe.engine_detail().is_err());
        assert!(probe.pipe_detail().contains("不存在"));
    }
}
