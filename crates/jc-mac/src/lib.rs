//! macOS wrapper 用的引擎 client（佔位）。
//!
//! 公開介面形狀與 `jc-windows` 相同，之後接 Unix domain socket；常駐引擎的
//! 啟動與監看會走 launchd。目前所有連線嘗試都回 [`Error::Unsupported`]：沒有
//! `todo!()`、不會 panic。
//!
//! 傳輸規劃：以 `UnixStream` 連 `--socket <path>`，握手、逾時與重連策略沿用
//! `jc-windows` 的設計。

use std::io::{Read, Write};
use std::marker::PhantomData;
use std::time::Duration;

use jc_proto::{ErrorCode, Request, Response};

/// 佔位的 Unix domain socket 傳輸。
pub struct UnixSocket;

impl UnixSocket {
    /// 連線到 Unix domain socket。尚未實作，一律回 [`Error::Unsupported`]。
    pub fn connect(_path: &str) -> Result<Self, Error> {
        Err(Error::Unsupported)
    }
}

/// 佔位 client。
///
/// 形狀與 `jc_windows::Client` 相同，等 Unix socket 傳輸補上後才會有實際行為。
pub struct Client<S> {
    /// 佔位；真的實作會把串流移到背景執行緒。
    _transport: PhantomData<fn() -> S>,
}

impl Client<UnixSocket> {
    /// 連線到 `path` 並完成握手。尚未實作，回 [`Error::Unsupported`]。
    pub fn connect(_path: &str, _client_name: &str, _timeout: Duration) -> Result<Self, Error> {
        Err(Error::Unsupported)
    }

    /// 啟動階段的重連。尚未實作，回 [`Error::Unsupported`]。
    pub fn connect_with_retry(
        _path: &str,
        _client_name: &str,
        _timeout: Duration,
        _attempts: usize,
        _backoff: Duration,
    ) -> Result<Self, Error> {
        Err(Error::Unsupported)
    }
}

impl<S: Read + Write> Client<S> {
    /// 以既有串流建立 client。尚未實作，回 [`Error::Unsupported`]。
    pub fn handshake(_stream: S, _client_name: &str, _timeout: Duration) -> Result<Self, Error> {
        Err(Error::Unsupported)
    }

    /// 送一個請求等一個回應。尚未實作，回 [`Error::Unsupported`]。
    pub fn request(&mut self, _request: &Request, _timeout: Duration) -> Result<Response, Error> {
        Err(Error::Unsupported)
    }

    /// 引擎是否已要求關閉。佔位實作永遠回 `false`：不會有可用的 client。
    pub fn is_closed(&self) -> bool {
        false
    }
}

/// client 的錯誤，變體與 `jc-windows` 相同。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 底層讀寫失敗。
    #[error("讀寫失敗：{0}")]
    Io(#[from] std::io::Error),
    /// 引擎回報的協定錯誤。
    #[error("引擎回報協定錯誤：{0:?}")]
    Protocol(ErrorCode),
    /// 握手時版本不同。
    #[error("協定版本不符：引擎 {engine}，客戶端 {client}")]
    VersionMismatch {
        /// 引擎的協定版本；未知時為 0。
        engine: u32,
        /// 這個 client 使用的協定版本。
        client: u32,
    },
    /// 等待回應逾時。
    #[error("等待引擎回應逾時；連線已不可信，請重新連線")]
    Timeout,
    /// 這個平台尚未支援。佔位 crate 的所有連線嘗試都會回這個錯誤。
    #[error("這個平台尚未支援")]
    Unsupported,
    /// 訊框不正確。
    #[error("訊框不正確：{0}")]
    BadFrame(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn connect_returns_unsupported() {
        let err = Client::connect("/tmp/jc-engine.sock", "test", Duration::from_millis(10)).err();
        assert!(matches!(err, Some(Error::Unsupported)));
    }

    #[test]
    fn retry_returns_unsupported_without_sleeping() {
        let err = Client::connect_with_retry(
            "/tmp/jc-engine.sock",
            "test",
            Duration::from_millis(10),
            3,
            Duration::ZERO,
        )
        .err();
        assert!(matches!(err, Some(Error::Unsupported)));
    }

    #[test]
    fn handshake_returns_unsupported() {
        let stream = Cursor::new(Vec::<u8>::new());
        let err = Client::handshake(stream, "test", Duration::from_millis(10)).err();
        assert!(matches!(err, Some(Error::Unsupported)));
    }
}
