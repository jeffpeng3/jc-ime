//! Windows TSF wrapper 用的引擎 client。
//!
//! 這個 crate 提供 named pipe 連線、握手與請求／回應往返。訊框與 JSON 一律交給
//! [`jc_proto`]，這裡不重新實作長度前綴或訊息格式。
//!
//! # 逾時設計
//!
//! 每次往返都在一條背景執行緒上完成「寫出請求、讀回回應」，呼叫端用
//! `mpsc::recv_timeout` 等結果。選這個做法而不是 overlapped I/O 的理由：同一套
//! 程式碼可以套用在任何 `Read + Write` 串流（named pipe、stdio、測試用的假
//! 串流），不必為傳輸另外寫一份非同步 I/O，也不用引入 async runtime。
//!
//! **逾時之後這條連線就不可信**：管線裡可能還留著半個回應，下一個請求讀到的
//! 會是上一個請求的尾巴。因此 [`Client::request`] 逾時後把 client 標成不可用，
//! 之後的呼叫直接回 [`Error::Timeout`]；要繼續用就必須重新連線。這個設計讓
//! 「重送 `Key` 造成重複輸入」不可能發生，代價是呼叫端要處理重連。
//!
//! # 重連
//!
//! [`Client::connect_with_retry`] 只在啟動階段使用（引擎還沒起來時重試幾次）。
//! 已連線後斷線不會自動重送請求，因為重送 `Key` 會造成重複輸入。
//!
//! # pipe 名稱
//!
//! 引擎 pipe 名稱的解析規則放在 [`pipe_name`] 模組，`jc-tsf`、`jc-engine` 與這個
//! crate 共用同一份實作。名稱帶使用者 SID 尾段，避免同機多使用者共用一條 pipe。

mod pipe;
mod pipe_name;
mod worker;

use std::io::{self, Read, Write};
use std::marker::PhantomData;
use std::time::Duration;

pub use pipe::NamedPipe;
pub use pipe_name::{
    choose_pipe_name, current_user_sid, name_from_sid, resolve_pipe_name, ResolvedPipeName,
    BASE_PIPE_NAME, PIPE_ENV,
};

use jc_proto::{ErrorCode, Request, Response, PROTOCOL_VERSION};
use worker::Worker;

/// client 的錯誤。
///
/// 錯誤種類對應 wrapper 要做的處置：`Unsupported` 是平台不支援，`Timeout` 與
/// `Io` 代表連線不能用、要重連，`VersionMismatch` 代表引擎與 wrapper 版本不同。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 底層讀寫失敗，或連線已結束。
    #[error("讀寫失敗：{0}")]
    Io(#[from] io::Error),
    /// 引擎回報的協定錯誤。
    #[error("引擎回報協定錯誤：{0:?}")]
    Protocol(ErrorCode),
    /// 握手時版本不同。
    ///
    /// `engine` 為 0 表示引擎只回了版本不符的錯誤碼、沒有回報自己的版本。
    #[error("協定版本不符：引擎 {engine}，客戶端 {client}")]
    VersionMismatch {
        /// 引擎的協定版本；未知時為 0。
        engine: u32,
        /// 這個 client 使用的協定版本。
        client: u32,
    },
    /// 等待回應逾時。連線已不可信，必須重新連線。
    #[error("等待引擎回應逾時；連線已不可信，請重新連線")]
    Timeout,
    /// 這個平台尚未支援。非 Windows 平台呼叫 [`Client::connect`] 會得到這個錯誤。
    #[error("這個平台尚未支援")]
    Unsupported,
    /// 訊框不正確（超長、截斷或 JSON 解不開）。
    #[error("訊框不正確：{0}")]
    BadFrame(String),
}

/// client 目前能不能送請求。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// 握手完成，可以正常往返。
    Ready,
    /// 曾經逾時，串流裡可能還有殘留資料。
    Unusable,
    /// 引擎要求關閉，或讀到 EOF。
    Closed,
}

/// 與 `jc-engine` 連線的 client。
///
/// `S` 是傳輸串流；正常情況是 [`NamedPipe`]，測試與 stdio 模式可以用任何
/// `Read + Write` 串流。串流的所有權會移到背景執行緒，所以 `S` 必須是
/// `Send + 'static`。
pub struct Client<S> {
    /// 負責實際讀寫的執行緒。
    worker: Worker,
    /// 連線狀態。
    state: State,
    /// 串流已經移到背景執行緒，這裡只保留型別參數。
    _stream: PhantomData<fn() -> S>,
}

impl<S: Read + Write + Send + 'static> Client<S> {
    /// 以既有串流建立 client（測試與 stdio 用），完成握手。
    ///
    /// 這裡比任務書的 `impl<S: Read + Write>` 多了 `Send + 'static`：逾時要靠
    /// 一條持有串流的背景執行緒，串流因此必須能移到另一條執行緒。
    ///
    /// 握手一律使用 [`jc_proto::PROTOCOL_VERSION`]，不會退回較舊的版本。
    pub fn handshake(stream: S, client_name: &str, timeout: Duration) -> Result<Self, Error> {
        let worker = Worker::spawn(stream)?;
        let mut client = Self {
            worker,
            state: State::Ready,
            _stream: PhantomData,
        };
        client.complete_handshake(client_name, timeout)?;
        Ok(client)
    }

    /// 送一個請求，等一個回應。
    ///
    /// 版本不符或引擎回 `Error` 時回 [`Error::Protocol`] 或
    /// [`Error::VersionMismatch`]。逾時後 client 進入不可用狀態，之後的呼叫
    /// 直接回 [`Error::Timeout`]；呼叫端必須重新連線。
    pub fn request(&mut self, request: &Request, timeout: Duration) -> Result<Response, Error> {
        match self.state {
            State::Ready => {}
            State::Unusable => return Err(Error::Timeout),
            State::Closed => return Err(closed_error()),
        }

        let shuts_down = matches!(request, Request::Shutdown);
        let Some(response) = self.exchange(request.clone(), timeout)? else {
            self.state = State::Closed;
            return Err(eof_error());
        };
        if shuts_down {
            self.state = State::Closed;
        }

        match response {
            Response::Error { code, .. } => {
                if code == ErrorCode::VersionMismatch {
                    self.state = State::Closed;
                }
                Err(Error::Protocol(code))
            }
            response => Ok(response),
        }
    }

    /// 引擎是否已要求關閉：送出 `Shutdown`，或讀到 EOF。
    ///
    /// 逾時造成的不可用不算關閉，這種 client 的 `is_closed` 仍是 `false`。
    pub fn is_closed(&self) -> bool {
        matches!(self.state, State::Closed)
    }

    /// 握手：送 `Hello`，確認引擎回的版本與 [`jc_proto::PROTOCOL_VERSION`] 相同。
    fn complete_handshake(&mut self, client_name: &str, timeout: Duration) -> Result<(), Error> {
        let hello = Request::Hello {
            protocol: PROTOCOL_VERSION,
            client: client_name.to_string(),
        };
        let Some(response) = self.exchange(hello, timeout)? else {
            self.state = State::Closed;
            return Err(eof_error());
        };

        match response {
            Response::Ready { protocol, .. } if protocol == PROTOCOL_VERSION => Ok(()),
            Response::Ready { protocol, .. } => {
                self.state = State::Closed;
                Err(Error::VersionMismatch {
                    engine: protocol,
                    client: PROTOCOL_VERSION,
                })
            }
            Response::Error {
                code: ErrorCode::VersionMismatch,
                ..
            } => {
                self.state = State::Closed;
                Err(Error::VersionMismatch {
                    engine: 0,
                    client: PROTOCOL_VERSION,
                })
            }
            Response::Error { code, .. } => {
                self.state = State::Closed;
                Err(Error::Protocol(code))
            }
            other => {
                self.state = State::Closed;
                Err(Error::BadFrame(format!(
                    "握手時收到非 Ready 的回應：{other:?}"
                )))
            }
        }
    }

    /// 一次往返；逾時時把 client 標成不可用。
    fn exchange(&mut self, request: Request, timeout: Duration) -> Result<Option<Response>, Error> {
        match self.worker.round_trip(request, timeout) {
            Ok(response) => Ok(response),
            Err(Error::Timeout) => {
                self.state = State::Unusable;
                Err(Error::Timeout)
            }
            Err(error) => Err(error),
        }
    }
}

impl Client<NamedPipe> {
    /// 連線到 `\\.\pipe\<name>` 並完成握手。
    ///
    /// `name` 可以只寫 pipe 名稱，也可以寫完整的 `\\.\pipe\` 路徑。非 Windows
    /// 平台回 [`Error::Unsupported`]。
    pub fn connect(name: &str, client_name: &str, timeout: Duration) -> Result<Self, Error> {
        // 引擎一次只服務一條連線；instance 短暫被佔用時用同一個 timeout 等它
        // 釋出，避免 TSF 端立刻拿到 ERROR_PIPE_BUSY 也不斷重試。
        let pipe = NamedPipe::connect_with_wait(name, timeout)?;
        Self::handshake(pipe, client_name, timeout)
    }

    /// 啟動階段的重連：最多嘗試 `attempts` 次，每次間隔 `backoff`。
    ///
    /// 只用於啟動時等引擎起來。`attempts` 為 0 時視同 1 次。已連線後斷線不會
    /// 自動重送請求，因為重送 `Key` 會造成重複輸入；呼叫端要自己決定是否重連。
    pub fn connect_with_retry(
        name: &str,
        client_name: &str,
        timeout: Duration,
        attempts: usize,
        backoff: Duration,
    ) -> Result<Self, Error> {
        let attempts = attempts.max(1);
        let mut last_error = None;

        for attempt in 0..attempts {
            match Self::connect(name, client_name, timeout) {
                Ok(client) => return Ok(client),
                Err(error) => {
                    last_error = Some(error);
                    if attempt + 1 < attempts && !backoff.is_zero() {
                        std::thread::sleep(backoff);
                    }
                }
            }
        }

        match last_error {
            Some(error) => Err(error),
            None => Err(Error::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "重連次數必須大於 0",
            ))),
        }
    }
}

/// 連線已關閉的錯誤。
fn closed_error() -> Error {
    Error::Io(io::Error::new(
        io::ErrorKind::NotConnected,
        "連線已關閉，請重新連線",
    ))
}

/// 對端在回應前結束的錯誤。
fn eof_error() -> Error {
    Error::Io(io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "引擎在回應前就關閉連線",
    ))
}
