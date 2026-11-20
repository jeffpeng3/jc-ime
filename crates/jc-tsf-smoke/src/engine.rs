//! 引擎 pipe 的探測（檢查 6）。
//!
//! 用 `jc-windows` 的 client，與 `jc-tsf` 的 worker 走完全相同的路徑：連線時
//! 先送 `Hello` 握手，再送一個 `Key`。引擎**不在**是正常情況（還沒啟動或沒安裝），
//! 只印警告、不判定失敗。

use std::time::Duration;

use jc_proto::{Key, KeyCode, Request, Response, PROTOCOL_VERSION};
use jc_windows::Client;
use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};

/// 沒有設定時使用的 pipe 名稱，與 `jc-tsf` 相同。
pub const DEFAULT_PIPE: &str = "jc-ime";

/// 覆寫 pipe 名稱的環境變數（除錯用）。
pub const PIPE_ENV: &str = "JC_IME_PIPE";

/// 註冊表裡指定引擎 pipe 名稱的位置，與 `jc-tsf` 相同。
const PIPE_REGISTRY_KEY: &str = r"Software\jc-ime";
/// 註冊表值的名稱。
const PIPE_REGISTRY_VALUE: &str = "EnginePipe";

/// 單次往返逾時；與 `jc-tsf` 一致。
const TIMEOUT: Duration = Duration::from_millis(500);
/// 連線嘗試次數。
const ATTEMPTS: usize = 3;
/// 兩次嘗試之間的間隔。
const BACKOFF: Duration = Duration::from_millis(200);

/// 握手時送出的客戶端名稱。
const CLIENT_NAME: &str = concat!("jc-tsf-smoke/", env!("CARGO_PKG_VERSION"));

/// 讀 `HKCU\Software\jc-ime\EnginePipe`，沒有就用 [`DEFAULT_PIPE`]。
///
/// `jc-tsf` 只產出 cdylib，這個小函式沒辦法直接借用，只能照同一條規則重寫。
pub fn pipe_name() -> String {
    if let Ok(name) = std::env::var(PIPE_ENV) {
        if !name.trim().is_empty() {
            return name;
        }
    }

    let key = wide(PIPE_REGISTRY_KEY);
    let value = wide(PIPE_REGISTRY_VALUE);
    let mut buffer = [0u16; 256];
    let mut size = (buffer.len() * std::mem::size_of::<u16>()) as u32;

    // SAFETY: buffer 與 size 的長度一致；兩個字串都以 NUL 結尾。
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr() as *mut core::ffi::c_void),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS || size < 2 {
        return DEFAULT_PIPE.to_string();
    }

    let units = (size as usize / 2).saturating_sub(1).min(buffer.len());
    let text = String::from_utf16_lossy(&buffer[..units]);
    let text = text.trim_matches('\0').trim();
    if text.is_empty() {
        DEFAULT_PIPE.to_string()
    } else {
        text.to_string()
    }
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
        Response::Ready { protocol, engine } => format!("Hello → 協定 {protocol}、引擎 {engine}"),
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
        Response::Ready { protocol, engine } => format!("Ready（協定 {protocol}、引擎 {engine}）"),
        Response::State {
            preedit,
            candidates,
            ..
        } => format!("State（組字 {preedit:?}、候選 {} 個）", candidates.len()),
        Response::Committed { text } => format!("Committed（{text:?}）"),
        Response::Error { code, message } => format!("Error（{code:?}：{message}）"),
    }
}

/// 把字串轉成以 NUL 結尾的 UTF-16。
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_pipe_name_is_stable() {
        assert_eq!(DEFAULT_PIPE, "jc-ime");
    }

    #[test]
    fn wide_adds_terminator() {
        assert_eq!(wide("jc"), vec![b'j' as u16, b'c' as u16, 0]);
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
                engine: "jc-engine 0.1.0".to_string()
            }),
            "Ready（協定 3、引擎 jc-engine 0.1.0）"
        );
    }
}
