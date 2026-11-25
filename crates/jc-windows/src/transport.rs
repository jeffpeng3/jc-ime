//! 跨平台 local socket 傳輸。
//!
//! Windows 底層是 named pipe，Unix 底層是 unix domain socket；呼叫端只看到
//! [`LocalSocket`] 一種串流型別。名稱一律走
//! [`interprocess::local_socket::GenericNamespaced`]：Windows 自動補上
//! `\\.\pipe\`，Linux 用 abstract namespace，其他 Unix 落在 `/tmp/` 之下。
//!
//! 訊框與 JSON 序列化仍由 `jc_proto` 處理，這裡只搬位元組，不插入任何額外格式。

use std::io::{self, Read, Write};
use std::path::Path;
use std::time::Duration;

use interprocess::local_socket::{ConnectOptions, GenericNamespaced, Name, Stream, ToNsName};
#[cfg(windows)]
use interprocess::ConnectWaitMode;

use crate::Error;

/// 把 `--socket` 的值轉成 local socket 識別子（不含平台前綴）。
///
/// Windows 與其他 Unix 取最後一段：前者交給 interprocess 補 `\\.\pipe\`，
/// 後者補 `/tmp/`。Linux 保留完整字串，讓 abstract namespace 沿用原本
/// `XDG_RUNTIME_DIR` 的每使用者路徑，避免不同使用者共用同一個名稱。
pub fn socket_name(spec: &Path) -> io::Result<String> {
    let raw = spec.as_os_str().to_string_lossy();
    let raw = raw.trim();
    if raw.is_empty() || raw.ends_with(['\\', '/']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--socket 需要 socket 名稱或路徑",
        ));
    }

    #[cfg(target_os = "linux")]
    {
        Ok(raw.to_string())
    }

    #[cfg(not(target_os = "linux"))]
    {
        let base = raw.rsplit(['\\', '/']).next().unwrap_or("");
        if base.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "--socket 的 socket 名稱是空的",
            ));
        }
        Ok(base.to_string())
    }
}

/// 把識別子轉成 interprocess 的 [`Name`]。
///
/// 名稱過長或含有平台不允許的字元時回傳錯誤，由呼叫端附上上下文。
pub fn ns_name(name: &str) -> io::Result<Name<'_>> {
    name.to_ns_name::<GenericNamespaced>()
}

/// 已連線的 local socket 串流。
///
/// 內部是 interprocess 的 enum dispatch：Windows 為 named pipe，Unix 為 unix
/// domain socket。讀寫直接使用阻塞 I/O，沒有 async runtime。
pub struct LocalSocket {
    inner: Stream,
}

impl LocalSocket {
    /// 連到 `name`；Windows 上最多等 `wait` 讓忙碌的 pipe instance 釋出。
    ///
    /// interprocess 的 `ConnectWaitMode::Timeout` 在 Windows 內部的流程就是
    /// `CreateFileW` → `ERROR_PIPE_BUSY` → `WaitNamedPipeW` → 重試，因此 pipe-busy
    /// 修正的有界等待原樣保留。預設的 `Unbounded` 會無限期等 instance，TSF 的
    /// 500 ms 預算不能被它吃掉，所以這裡一定要覆寫。Linux 的 abstract socket
    /// 沒有忙碌狀態，`wait` 不影響連線；其餘平台仍有
    /// [`Client::connect_with_retry`] 的有界重試。
    ///
    /// [`Client::connect_with_retry`]: crate::Client::connect_with_retry
    pub fn connect(name: &str, wait: Duration) -> Result<Self, Error> {
        let ns = ns_name(name)?;
        let options = ConnectOptions::new().name(ns);
        #[cfg(windows)]
        let options = options.wait_mode(ConnectWaitMode::Timeout(wait));
        #[cfg(not(windows))]
        let _ = wait;
        Ok(Self {
            inner: options.connect_sync()?,
        })
    }
}

impl Read for LocalSocket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Write for LocalSocket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use interprocess::local_socket::{traits::Listener as _, ListenerOptions};
    use jc_proto::{
        read_message, write_message, Request, Response, DEFAULT_MAX_FRAME, PROTOCOL_VERSION,
    };

    #[test]
    #[cfg(target_os = "linux")]
    fn socket_name_keeps_full_path_on_linux() {
        assert_eq!(
            socket_name(Path::new("/tmp/jc-ime-engine.sock")).unwrap(),
            "/tmp/jc-ime-engine.sock"
        );
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn socket_name_uses_last_segment() {
        assert_eq!(
            socket_name(Path::new(r"\\.\pipe\jc-ime-1001")).unwrap(),
            "jc-ime-1001"
        );
        assert_eq!(
            socket_name(Path::new("/tmp/jc-ime.sock")).unwrap(),
            "jc-ime.sock"
        );
    }

    #[test]
    fn socket_name_rejects_empty_values() {
        assert!(socket_name(Path::new("")).is_err());
        assert!(socket_name(Path::new("   ")).is_err());
        assert!(socket_name(Path::new(r"\\.\pipe\")).is_err());
    }

    /// server 端用 interprocess 監聽，client 端用 [`LocalSocket::connect`] 連上，
    /// 再走一次 `jc_proto` 的訊框讀寫，確認換 transport 沒有動到序列化。
    #[test]
    fn local_socket_round_trip() {
        let tag = format!(
            "jc-windows-transport-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let ns = ns_name(&tag).expect("名稱可用");
        let listener = ListenerOptions::new()
            .name(ns)
            .create_sync()
            .expect("建立 listener");
        let server = std::thread::spawn(move || {
            let mut stream = listener.accept().expect("接受連線");
            let request = read_message::<_, Request>(&mut stream, DEFAULT_MAX_FRAME)
                .expect("讀取請求")
                .expect("應該有請求");
            write_message(
                &mut stream,
                &Response::Ready {
                    protocol: PROTOCOL_VERSION,
                    engine: "test".into(),
                    session: 1,
                    bindings: Default::default(),
                },
                DEFAULT_MAX_FRAME,
            )
            .expect("寫出回應");
            assert!(matches!(request, Request::Hello { .. }), "{request:?}");
        });

        let mut client =
            LocalSocket::connect(&tag, Duration::from_secs(2)).expect("連上 local socket");
        let hello = Request::Hello {
            protocol: PROTOCOL_VERSION,
            client: "transport-test".into(),
        };
        write_message(&mut client, &hello, DEFAULT_MAX_FRAME).expect("送出請求");
        let response = read_message::<_, Response>(&mut client, DEFAULT_MAX_FRAME)
            .expect("讀取回應")
            .expect("應該有回應");
        assert_eq!(
            response,
            Response::Ready {
                protocol: PROTOCOL_VERSION,
                engine: "test".into(),
                session: 1,
                bindings: Default::default(),
            }
        );
        server.join().expect("server 執行緒不該 panic");
    }
}
