//! 引擎連線與 worker 執行緒。
//!
//! UI 執行緒只負責把 [`Job`] 丟進 channel；連線、握手、逾時與重連都在 worker
//! 執行緒完成。回應先推進 [`Queue`]，再用 `PostMessageW` 通知 UI 執行緒，
//! 所以按鍵處理永遠不會卡在 IPC 上。
//!
//! 逾時固定 500 ms：引擎的神經模型硬上限是 180 ms，超過就是連線壞掉，依
//! `jc-windows` 的契約必須重連，不能重送同一個按鍵。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jc_proto::Request;
use jc_windows::{Client, NamedPipe};
use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::logging;

/// 單次往返的逾時。
pub const REQUEST_TIMEOUT: Duration = Duration::from_millis(500);

/// 斷線後重試連線的最短間隔。
const RECONNECT_BACKOFF: Duration = Duration::from_secs(2);

/// 註冊表裡指定引擎 pipe 名稱的位置。
const PIPE_REGISTRY_KEY: &str = r"Software\jc-ime";
/// 註冊表值的名稱。
const PIPE_REGISTRY_VALUE: &str = "EnginePipe";
/// 沒有設定時使用的 pipe 名稱。
pub const DEFAULT_PIPE: &str = "jc-ime";

/// 握手時送出的客戶端名稱。
const CLIENT_NAME: &str = concat!("jc-tsf/", env!("CARGO_PKG_VERSION"));

/// UI 執行緒收到「引擎有回應」的自訂訊息。
pub const WM_ENGINE_RESPONSE: u32 = WM_APP + 1;

/// 送給 worker 的一個請求。
pub struct Job {
    /// 送出時的按鍵世代；只有 `State` 會被這個值淘汰。
    pub generation: u64,
    /// 要送出的協定請求。
    pub request: Request,
}

/// worker 送回 UI 執行緒的結果。
pub struct Incoming {
    /// 對應的按鍵世代。
    pub generation: u64,
    /// 引擎回應或錯誤訊息。
    pub result: std::result::Result<jc_proto::Response, String>,
}

/// UI 與 worker 共用的回應佇列。
///
/// 只有一把鎖、一次處理一個請求，不需要額外的同步機制。
pub struct Queue {
    items: Mutex<VecDeque<Incoming>>,
    /// 訊息視窗的 handle；`0` 代表還沒建立。
    hwnd: AtomicIsize,
    /// 目前是否已連上引擎。
    connected: AtomicBool,
}

impl Queue {
    /// 建立空佇列。
    pub fn new() -> Self {
        Self {
            items: Mutex::new(VecDeque::new()),
            hwnd: AtomicIsize::new(0),
            connected: AtomicBool::new(false),
        }
    }

    /// 綁定訊息視窗，之後 worker 才能喚醒 UI 執行緒。
    pub fn attach(&self, hwnd: isize) {
        self.hwnd.store(hwnd, Ordering::Release);
    }

    /// 引擎是否已連線。
    ///
    /// 沒連線時 `OnTestKeyDown` 直接放行，讓使用者至少能打出原來的字母。
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    /// worker 端：推入一筆結果並喚醒 UI 執行緒。
    fn push(&self, item: Incoming) {
        if let Ok(mut items) = self.items.lock() {
            items.push_back(item);
        }
        let hwnd = self.hwnd.load(Ordering::Acquire);
        if hwnd != 0 {
            // SAFETY: hwnd 由 UI 執行緒在自己的窗口建立後寫入，DestroyWindow
            // 之前會先用 detach 清成 0。
            unsafe {
                let _ = PostMessageW(
                    Some(windows::Win32::Foundation::HWND(
                        hwnd as *mut std::ffi::c_void,
                    )),
                    WM_ENGINE_RESPONSE,
                    Default::default(),
                    Default::default(),
                );
            }
        }
    }

    /// UI 端：取出所有待處理結果。
    pub fn drain(&self) -> Vec<Incoming> {
        match self.items.lock() {
            Ok(mut items) => items.drain(..).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// 標記連線狀態。
    fn set_connected(&self, connected: bool) {
        self.connected.store(connected, Ordering::Release);
    }
}

impl Default for Queue {
    fn default() -> Self {
        Self::new()
    }
}

/// 啟動 worker 執行緒。
///
/// worker 沒有 join handle：`Deactivate` 把 sender 丟掉之後 channel 斷線，worker
/// 自己結束。這條執行緒不碰任何 COM 物件，只讀寫 pipe 與推佇列。
pub fn spawn(jobs: Receiver<Job>, queue: Arc<Queue>) {
    let _ = std::thread::Builder::new()
        .name("jc-tsf-engine".into())
        .spawn(move || run(jobs, queue));
}

/// worker 主迴圈。
fn run(jobs: Receiver<Job>, queue: Arc<Queue>) {
    // Activate 之後立刻試一次，失敗才進入 2 秒一次的退避。
    let mut client = connect(&queue);
    let mut last_attempt = Some(std::time::Instant::now());

    loop {
        // 沒連線時用 recv_timeout 定期重試；連線中就一直等到有工作或斷線。
        let job = if client.is_none() {
            match jobs.recv_timeout(RECONNECT_BACKOFF) {
                Ok(job) => Some(job),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match jobs.recv() {
                Ok(job) => Some(job),
                Err(_) => break,
            }
        };

        if client.is_none() {
            // 最多每 `RECONNECT_BACKOFF` 一次，不能在每個按鍵都阻塞 500 ms 握手。
            try_connect(&mut client, &mut last_attempt, &queue);
        }

        let Some(job) = job else {
            continue;
        };

        let Some(active) = client.as_mut() else {
            queue.push(Incoming {
                generation: job.generation,
                result: Err("引擎未連線".into()),
            });
            continue;
        };

        let result = match active.request(&job.request, REQUEST_TIMEOUT) {
            Ok(response) => Ok(response),
            Err(error) => {
                // 逾時或讀寫失敗都代表這條連線不可信，不能再送任何請求。
                logging::warn(format!("引擎請求失敗：{error}"));
                client = None;
                queue.set_connected(false);
                Err(error.to_string())
            }
        };

        queue.push(Incoming {
            generation: job.generation,
            result,
        });
    }

    queue.set_connected(false);
    logging::debug("引擎 worker 結束");
}

/// 有退避的重連：距離上次嘗試不到 [`RECONNECT_BACKOFF`] 就跳過。
fn try_connect(
    client: &mut Option<Client<NamedPipe>>,
    last_attempt: &mut Option<std::time::Instant>,
    queue: &Queue,
) {
    let due = last_attempt.is_none_or(|at| at.elapsed() >= RECONNECT_BACKOFF);
    if !due {
        return;
    }
    *last_attempt = Some(std::time::Instant::now());
    *client = connect(queue);
}

/// 連上引擎並握手；失敗只記日誌。
fn connect(queue: &Queue) -> Option<Client<NamedPipe>> {
    let name = pipe_name();
    match Client::connect(&name, CLIENT_NAME, REQUEST_TIMEOUT) {
        Ok(client) => {
            logging::info(format!("已連上引擎 pipe：{name}"));
            queue.set_connected(true);
            Some(client)
        }
        Err(error) => {
            logging::debug(format!("連線引擎 pipe {name} 失敗：{error}"));
            queue.set_connected(false);
            None
        }
    }
}

/// 讀 `HKCU\Software\jc-ime\EnginePipe`；沒有設定就用 [`DEFAULT_PIPE`]。
pub fn pipe_name() -> String {
    let key = crate::wide(PIPE_REGISTRY_KEY);
    let value = crate::wide(PIPE_REGISTRY_VALUE);
    let mut buffer = [0u16; 256];
    let mut size = (buffer.len() * std::mem::size_of::<u16>()) as u32;

    // SAFETY: buffer 與 size 的長度一致，字串都以 NUL 結尾。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_pipe_name_is_stable() {
        assert_eq!(DEFAULT_PIPE, "jc-ime");
    }

    #[test]
    fn queue_marks_disconnected_by_default() {
        let queue = Queue::new();
        assert!(!queue.is_connected());
        queue.set_connected(true);
        assert!(queue.is_connected());
        queue.set_connected(false);
        assert!(!queue.is_connected());
    }
}
