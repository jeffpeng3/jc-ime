//! Unix domain socket 傳輸（目前是佔位）。
//!
//! 這個檔案只在 unix 目標編譯。`UnixStream`、`UnixListener` 與 `std::os::unix`
//! 的用法一律留在這裡：非 unix 目標改編 `unsupported.rs`，碰不到這些型別。
//!
//! 目前只是佔位：真的實作會用 `UnixStream::connect(path)` 連上引擎的
//! `--socket`，握手、逾時與重連沿用 `jc-windows` 的設計；`launchd` 的 socket
//! 活化與 plist 不在這個 crate。

#![cfg(unix)]

use crate::Error;

/// 佔位的 Unix domain socket 傳輸。
///
/// 之後會包住一條 `UnixStream`；在那之前所有連線都回 [`Error::Unsupported`]。
pub struct UnixSocket;

impl UnixSocket {
    /// 連線到 Unix domain socket。尚未實作，一律回 [`Error::Unsupported`]。
    pub fn connect(_path: &str) -> Result<Self, Error> {
        Err(Error::Unsupported)
    }
}
