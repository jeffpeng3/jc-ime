//! Windows named pipe 傳輸。
//!
//! 與 unix socket 分支的行為一致：一次只服務一條連線、序列處理請求，
//! 用戶端斷線後回到等待下一個連線，收到 `Shutdown` 才關閉 pipe 並結束行程。
//! 訊框（長度前綴）讀寫完全沿用 `jc_proto`，這裡只負責搬位元組。
//!
//! 只使用 blocking Win32 API，沒有 async runtime，也沒有執行緒池：
//! `ConnectNamedPipe` 會阻塞到有 client 連上，之後整條連線在同一執行緒跑完。
//!
//! 整個檔案是 Windows 專屬：`main.rs` 用 `#[cfg(windows)] mod pipe_windows;`
//! 掛進來，檔案自己也再宣告一次 `#![cfg(windows)]`。若有人拿掉 `mod` 上的
//! `cfg`，非 Windows 建置會因為模組變空而編不過，不會靜靜地編出一份用不到的
//! Win32 程式碼。

#![cfg(windows)]

use std::io::{self, BufReader, BufWriter, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;

use anyhow::{bail, Context};
use jc_engine::{serve, Engine};
use windows_sys::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_MORE_DATA, ERROR_NO_DATA, ERROR_PIPE_CONNECTED,
    ERROR_PIPE_NOT_CONNECTED, HANDLE,
};
use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_MESSAGE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_WAIT,
};

/// named pipe 的固定前綴。所有 pipe 都是本機物件，沒有遠端命名空間。
const PIPE_PREFIX: &str = r"\\.\pipe\";

/// 單向緩衝區大小，與 Go 版 `winio.PipeConfig` 一致（64 KiB）。
const PIPE_BUFFER_SIZE: u32 = 1 << 16;

/// 同一個 pipe 只開一個 instance，序列處理連線。
const PIPE_MAX_INSTANCES: u32 = 1;

/// 在 `spec` 指定的 pipe 上監聽，收到 `Shutdown` 才回傳。
///
/// `spec` 可以只是名字（`jc-engine`），也可以是路徑或完整的 pipe 名稱
/// （`C:\tmp\engine.pipe`、`\\.\pipe\jc-engine`）；一律取最後一段並掛到
/// `\\.\pipe\` 之下，因此 `\\其他主機\pipe\x` 不會連到遠端，會變成
/// 本機的 `\\.\pipe\x`。
pub(crate) fn serve_pipe(engine: &mut Engine, spec: &Path, max_frame: usize) -> anyhow::Result<()> {
    let name = pipe_name(spec)?;
    let pipe = create_pipe(&name)?;
    log::info!("jc-engine 監聽 named pipe {name}");

    loop {
        match wait_for_client(&pipe) {
            Ok(()) => {}
            // client 在 `ConnectNamedPipe` 完成前就斷線：這條連線不算數，繼續等。
            Err(err) if is_client_gone(&err) => {
                log::warn!("連線在建立前中斷：{err}");
                continue;
            }
            Err(err) => {
                return Err(err).with_context(|| format!("等待 named pipe 連線失敗：{name}"));
            }
        }
        log::debug!("接受 named pipe 連線");

        let reader = PipeStream::from_handle(pipe.try_clone().context("複製 pipe handle 失敗")?);
        let writer = PipeStream::from_handle(pipe.try_clone().context("複製 pipe handle 失敗")?);
        let mut input = BufReader::new(reader);
        let mut output = BufWriter::new(writer);
        // 單一連線的錯誤不影響下一個連線。
        if let Err(err) = serve(engine, &mut input, &mut output, max_frame) {
            log::warn!("連線結束：{err}");
        }
        if let Err(err) = output.flush() {
            log::warn!("寫出最後一個回應失敗：{err}");
        }
        // 先放掉連線用的 handle，再切斷 instance，讓下一次 `ConnectNamedPipe` 乾淨。
        drop(input);
        drop(output);

        if engine.is_shutdown() {
            log::debug!("收到 Shutdown，結束行程");
            break;
        }

        if let Err(err) = disconnect(&pipe) {
            log::warn!("切斷 named pipe 連線失敗：{err}");
        }
        engine.reset_session();
    }

    Ok(())
}

/// 把 `--socket` 的值轉成完整的 named pipe 名稱。
///
/// 規則只有一條：取輸入的最後一段，掛到 `\\.\pipe\` 之下。因此裸名字、
/// Windows 路徑、已帶前綴的名稱都接受，遠端主機前綴一律被忽略。
pub(crate) fn pipe_name(spec: &Path) -> anyhow::Result<String> {
    let raw = spec.as_os_str().to_string_lossy();
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("--socket 需要 pipe 名稱或路徑");
    }
    let base = raw.rsplit(['\\', '/']).next().unwrap_or("");
    if base.is_empty() {
        bail!("--socket 的 pipe 名稱是空的：{raw}");
    }
    Ok(format!("{PIPE_PREFIX}{base}"))
}

/// 建立 message-mode 的雙向 pipe；名稱已被其他行程佔用時回傳錯誤，不重試。
fn create_pipe(name: &str) -> anyhow::Result<OwnedHandle> {
    let wide = wide(name);
    let handle = unsafe {
        CreateNamedPipeW(
            wide.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_MAX_INSTANCES,
            PIPE_BUFFER_SIZE,
            PIPE_BUFFER_SIZE,
            0,
            std::ptr::null(),
        )
    };
    if handle.is_null() {
        let err = io::Error::last_os_error();
        return Err(anyhow::Error::new(err)).with_context(|| {
            format!("建立 named pipe 失敗：{name}（若已有引擎使用同名 pipe，請先關閉它）")
        });
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

/// 阻塞到有 client 連上。`ERROR_PIPE_CONNECTED` 代表 client 在呼叫前就連上，
/// 對伺服器而言同樣是成功。
fn wait_for_client(pipe: &OwnedHandle) -> io::Result<()> {
    let ok = unsafe { ConnectNamedPipe(raw(pipe), std::ptr::null_mut()) };
    if ok != 0 {
        return Ok(());
    }
    let err = io::Error::last_os_error();
    if err.raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32) {
        return Ok(());
    }
    Err(err)
}

/// 這個錯誤是否只代表「client 已經離開」，不需要結束整個 server。
fn is_client_gone(err: &io::Error) -> bool {
    matches!(
        err.raw_os_error().map(|code| code as u32),
        Some(ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED | ERROR_NO_DATA)
    )
}

/// 切斷目前的 client，pipe instance 保留給下一個連線。
fn disconnect(pipe: &OwnedHandle) -> io::Result<()> {
    let ok = unsafe { DisconnectNamedPipe(raw(pipe)) };
    if ok != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `OwnedHandle` 轉成 Win32 的 `HANDLE`。兩者都是 `*mut c_void`，只差型別名稱。
fn raw(handle: &OwnedHandle) -> HANDLE {
    handle.as_raw_handle() as HANDLE
}

/// 把字串轉成 NUL 結尾的 UTF-16，供 `*W` API 使用。
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 單一方向的 pipe handle。讀寫都用 blocking 的 `ReadFile`／`WriteFile`。
///
/// 讀取端與寫入端各拿一個 `DuplicateHandle` 出來的副本，才能同時交給
/// `BufReader` 與 `BufWriter`；`OwnedHandle` 在 drop 時呼叫 `CloseHandle`。
struct PipeStream {
    handle: OwnedHandle,
}

impl PipeStream {
    /// 接管一個 pipe handle。
    fn from_handle(handle: OwnedHandle) -> Self {
        Self { handle }
    }
}

impl Read for PipeStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let len = buf.len().min(u32::MAX as usize) as u32;
        let mut read = 0u32;
        let ok = unsafe {
            ReadFile(
                raw(&self.handle),
                buf.as_mut_ptr(),
                len,
                &mut read,
                std::ptr::null_mut(),
            )
        };
        if ok != 0 {
            return Ok(read as usize);
        }
        let err = io::Error::last_os_error();
        match err.raw_os_error().map(|code| code as u32) {
            // message 比緩衝區大：這次先回傳已讀到的部分，剩下的下次再讀。
            Some(ERROR_MORE_DATA) if read > 0 => Ok(read as usize),
            Some(ERROR_MORE_DATA) => Err(io::Error::other("message-mode 讀取沒有進度")),
            // 用戶端斷線：當成一般 EOF，讓上層回到等待下一個連線。
            _ if is_client_gone(&err) => Ok(0),
            _ => Err(err),
        }
    }
}

impl Write for PipeStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let len = buf.len().min(u32::MAX as usize) as u32;
        let mut written = 0u32;
        let ok = unsafe {
            WriteFile(
                raw(&self.handle),
                buf.as_ptr(),
                len,
                &mut written,
                std::ptr::null_mut(),
            )
        };
        if ok != 0 {
            if written == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "pipe 沒有寫入任何位元組",
                ));
            }
            return Ok(written as usize);
        }
        let err = io::Error::last_os_error();
        if is_client_gone(&err) {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "用戶端已斷線"));
        }
        Err(err)
    }

    fn flush(&mut self) -> io::Result<()> {
        // WriteFile 直接寫進 pipe，沒有使用者空間緩衝。
        Ok(())
    }
}

/// 測試用的 client：開啟既有的 pipe。名稱還沒建立或 instance 已被佔用時
/// 回傳錯誤，交給呼叫端重試。
#[cfg(test)]
fn connect_client(name: &str) -> io::Result<OwnedHandle> {
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, OPEN_EXISTING};

    let wide = wide(name);
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc_proto::{
        read_message, write_message, Request, Response, DEFAULT_MAX_FRAME, PROTOCOL_VERSION,
    };
    use std::thread;
    use std::time::{Duration, Instant};

    fn test_pipe_name(tag: &str) -> String {
        format!(r"\\.\pipe\jc-engine-{}-{tag}", std::process::id())
    }

    /// 等到 pipe 可以被開啟（最多 5 秒），回傳已連上的 client。
    fn connect_with_retry(name: &str) -> OwnedHandle {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match connect_client(name) {
                Ok(handle) => return handle,
                Err(err) => {
                    if Instant::now() >= deadline {
                        panic!("連不上 {name}：{err}");
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }

    fn hello(name: &str) -> Request {
        Request::Hello {
            protocol: PROTOCOL_VERSION,
            client: name.to_string(),
        }
    }

    /// 用一條既有連線送一個請求並讀回回應。
    fn exchange(
        input: &mut BufReader<PipeStream>,
        output: &mut BufWriter<PipeStream>,
        request: &Request,
    ) -> Response {
        write_message(output, request, DEFAULT_MAX_FRAME).expect("送出請求");
        output.flush().expect("flush 請求");
        read_message(input, DEFAULT_MAX_FRAME)
            .expect("讀取回應")
            .expect("應該有回應")
    }

    /// 開一條連線、送一個請求、讀回回應後關閉連線。
    fn round_trip(handle: OwnedHandle, request: &Request) -> Response {
        let mut input = BufReader::new(PipeStream::from_handle(handle.try_clone().unwrap()));
        let mut output = BufWriter::new(PipeStream::from_handle(handle));
        exchange(&mut input, &mut output, request)
    }

    #[test]
    fn bare_name_becomes_local_pipe_path() {
        assert_eq!(
            pipe_name(Path::new("jc-engine")).unwrap(),
            r"\\.\pipe\jc-engine"
        );
    }

    #[test]
    fn local_pipe_path_is_kept_once() {
        assert_eq!(
            pipe_name(Path::new(r"\\.\pipe\jc-engine")).unwrap(),
            r"\\.\pipe\jc-engine"
        );
        assert_eq!(
            pipe_name(Path::new(r"\\.\PIPE\jc-engine")).unwrap(),
            r"\\.\pipe\jc-engine"
        );
    }

    #[test]
    fn windows_path_uses_basename() {
        assert_eq!(
            pipe_name(Path::new(r"C:\tmp\engine.pipe")).unwrap(),
            r"\\.\pipe\engine.pipe"
        );
        assert_eq!(
            pipe_name(Path::new("/tmp/engine.sock")).unwrap(),
            r"\\.\pipe\engine.sock"
        );
    }

    /// 遠端主機前綴不會被保留，名稱一律落到本機 pipe 命名空間。
    #[test]
    fn remote_host_prefix_is_reduced_to_local_name() {
        assert_eq!(
            pipe_name(Path::new(r"\\otherhost\pipe\jc-engine")).unwrap(),
            r"\\.\pipe\jc-engine"
        );
    }

    #[test]
    fn empty_name_is_rejected() {
        assert!(pipe_name(Path::new("")).is_err());
        assert!(pipe_name(Path::new("   ")).is_err());
        assert!(pipe_name(Path::new(r"\\.\pipe\")).is_err());
    }

    /// 斷線後可以重連；`Shutdown` 讓 server 迴圈正常結束。
    #[test]
    fn pipe_survives_disconnect_and_stops_on_shutdown() {
        let name = test_pipe_name("reconnect");
        let server_name = name.clone();
        let server = thread::spawn(move || {
            let mut engine = Engine::new();
            serve_pipe(&mut engine, Path::new(&server_name), DEFAULT_MAX_FRAME)
        });

        // 第一條連線：握手後直接斷線，server 應回到等待狀態。
        let response = round_trip(connect_with_retry(&name), &hello("first"));
        assert!(
            matches!(response, Response::Ready { protocol, .. } if protocol == PROTOCOL_VERSION),
            "第一次連線應完成握手，得到 {response:?}"
        );

        // 第二條連線：握手後送 Shutdown，server 迴圈結束。
        let handle = connect_with_retry(&name);
        let mut input = BufReader::new(PipeStream::from_handle(handle.try_clone().unwrap()));
        let mut output = BufWriter::new(PipeStream::from_handle(handle));
        let response = exchange(&mut input, &mut output, &hello("second"));
        assert!(matches!(response, Response::Ready { .. }), "{response:?}");
        let response = exchange(&mut input, &mut output, &Request::Shutdown);
        assert!(matches!(response, Response::State { .. }), "{response:?}");

        server
            .join()
            .expect("server 執行緒不該 panic")
            .expect("server 應正常結束");
    }
}
