//! 非 unix 平台的佔位介面。
//!
//! 這個檔案只在非 unix 目標編譯，目的只有一個：讓 [`crate::Client`] 的公開
//! 介面在 Windows 也能通過 `cargo check --workspace`。這裡不得出現任何
//! Unix domain socket 型別。

#![cfg(not(unix))]

use crate::Error;

/// 佔位傳輸；這個平台沒有 Unix domain socket。
pub struct UnixSocket;

impl UnixSocket {
    /// 這個平台沒有 Unix domain socket，一律回 [`Error::Unsupported`]。
    pub fn connect(_path: &str) -> Result<Self, Error> {
        Err(Error::Unsupported)
    }
}
