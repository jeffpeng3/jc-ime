//! Windows 監聽端：直接用 Win32 `CreateNamedPipeW` 建立 named pipe。
//!
//! `interprocess` 2.4.9 複製含 `G:` 的 SDDL 時會把 group SID 寫到 owner，
//! `CreateNamedPipeW` 因此回 ERROR_INVALID_OWNER (1307)。這裡自己組
//! `SECURITY_ATTRIBUTES`，SDDL 字串照 `0c` 的
//! `cmd/0c-server/transport_windows.go`，含 AppContainer 的
//! ALL APPLICATION PACKAGES 與 ALL RESTRICTED APPLICATION PACKAGES。
//!
//! T4 起每條連線一個 worker thread：accept 迴圈每次用
//! `PIPE_UNLIMITED_INSTANCES` 建立一個新的 pipe instance，`ConnectNamedPipe`
//! 成功後把 handle 移進 worker，自己再建立下一個 instance。drain／shutdown 時
//! 對空閒 instance 與各 worker 的 handle 呼叫 `CancelIoEx` 中斷阻塞的
//! `ConnectNamedPipe`／`ReadFile`，不需要 async runtime。

use std::io::{self, BufReader, BufWriter, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr;
use std::sync::Arc;
use std::thread;

use anyhow::{anyhow, Context};
use windows_sys::Win32::Foundation::{LocalFree, ERROR_NO_DATA, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::IO::CancelIoEx;

use crate::{serve as serve_connection, Engine};

/// 從別的 thread 中斷 handle 上正在等待的 I/O。
///
/// 把 handle 轉成 `isize`（`Send + Sync`）帶過 thread 邊界，呼叫時再轉回
/// `HANDLE`。`lpOverlapped` 為 null 代表取消該 handle 上所有 I/O。
fn cancel_io(handle: isize) {
    // SAFETY: handle 是有效的 pipe handle，由持有它的 thread 保證生命週期；
    // CancelIoEx 對已關閉 handle 只會失敗，不會造成記憶體不安全。
    unsafe {
        CancelIoEx(handle as *mut core::ffi::c_void, ptr::null());
    }
}

/// Win32 管道完整路徑的前綴。
const PIPE_PREFIX: &str = r"\\.\pipe\";

/// 與 `0c` 相同的輸入／輸出 buffer 大小：`1 << 16`。
const BUFFER_SIZE: u32 = 1 << 16;

/// SDDL 修訂版常數 `SDDL_REVISION_1`。
const SDDL_REVISION_1: u32 = 1;

/// 完整管道名稱：`\\.\pipe\<name>`。
///
/// `JC_IME_PIPE` 非空時優先，與 T1b 的 `server_pipe()` 一致；呼叫端若把前綴
/// 也寫進覆寫值，這裡先剝掉，避免產生雙前綴。沒有覆寫時用 `--socket` 的
/// 最後一段（監督模式傳進来的就是 `resolve_pipe_name()` 的 SID 推導結果）。
fn pipe_name(path: &Path) -> io::Result<String> {
    if let Ok(value) = std::env::var(jc_windows::PIPE_ENV) {
        let value = value.trim();
        if !value.is_empty() {
            let value = value.strip_prefix(PIPE_PREFIX).unwrap_or(value);
            return Ok(format!("{PIPE_PREFIX}{value}"));
        }
    }
    let name = jc_windows::socket_name(path)?;
    Ok(format!("{PIPE_PREFIX}{name}"))
}

/// 《0c》逐字相同的 SDDL：本人完全存取，AppContainer 兩組 SID 可讀寫，
/// 低完整性標籤讓受保護模式宿主也能開啟，其他使用者沒有任何 ACE。
fn security_descriptor_sddl(sid: &str) -> String {
    format!("D:P(A;;GA;;;{sid})(A;;GRGW;;;S-1-15-2-1)(A;;GRGW;;;S-1-15-2-2)S:(ML;;NW;;;LW)")
}

/// 把 SDDL 轉成 `SECURITY_ATTRIBUTES`。
///
/// 回傳的 descriptor 由 `ConvertStringSecurityDescriptorToSecurityDescriptorW`
/// 以 `LocalAlloc` 配置，呼叫端必須用 `LocalFree` 釋放。
fn security_attributes(sid: &str) -> io::Result<(SECURITY_ATTRIBUTES, PSECURITY_DESCRIPTOR)> {
    let sddl = security_descriptor_sddl(sid);
    let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    let mut size = 0u32;
    // SAFETY: `wide` 以 NUL 結尾，`descriptor` 與 `size` 是有效的輸出指標。
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            &mut size,
        )
    };
    if ok == 0 || descriptor.is_null() {
        return Err(io::Error::last_os_error());
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    Ok((attributes, descriptor))
}

/// 已建立的 named pipe instance。持有 handle，drop 時自動關閉。
struct PipeStream {
    handle: OwnedHandle,
}

impl PipeStream {
    /// 建立監聽用的 pipe instance。
    fn listen(name: &str, sid: &str) -> io::Result<Self> {
        let (attributes, descriptor) = security_attributes(sid)?;
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: `attributes.lpSecurityDescriptor` 在本次呼叫期間有效；`wide`
        // 以 NUL 結尾；其餘參數都是數值常數。
        let raw = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                BUFFER_SIZE,
                BUFFER_SIZE,
                0,
                &attributes,
            )
        };
        // SAFETY: descriptor 由 SDDL 轉換函式配置，`CreateNamedPipeW` 只在呼叫
        // 期間讀取，現在可以釋放。
        unsafe { LocalFree(descriptor) };
        if raw == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` 是剛建立且尚未被其他 owner 接管的有效 pipe handle。
        Ok(Self {
            handle: unsafe { OwnedHandle::from_raw_handle(raw) },
        })
    }

    /// 等待用戶端連上。同步模式下會阻塞。
    fn connect(&self) -> io::Result<()> {
        // SAFETY: handle 有效；同步模式的 `lpOverlapped` 必須是 null。
        if unsafe { ConnectNamedPipe(self.handle.as_raw_handle(), ptr::null_mut()) } == 0 {
            let error = io::Error::last_os_error();
            // 前一條連線已關閉但還沒 Disconnect 時會拿到 ERROR_NO_DATA，
            // 代表連線其實建立過，交給上層照常服務即可。
            if error.raw_os_error() == Some(ERROR_NO_DATA as i32) {
                return Ok(());
            }
            return Err(error);
        }
        Ok(())
    }
}

// 用共用參考實作 Read/Write，讀寫端可以同時借用同一條 pipe，呼叫端因此能
// 直接套用 `crate::serve` 與 `BufReader`/`BufWriter`，不必複製 handle。
impl Read for &PipeStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut read = 0u32;
        // SAFETY: `buf` 有效，handle 有效，同步模式 `lpOverlapped` 為 null。
        let ok = unsafe {
            ReadFile(
                self.handle.as_raw_handle(),
                buf.as_mut_ptr(),
                buf.len().min(u32::MAX as usize) as u32,
                &mut read,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(read as usize)
    }
}

impl Write for &PipeStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut written = 0u32;
        // SAFETY: `buf` 有效，handle 有效，同步模式 `lpOverlapped` 為 null。
        let ok = unsafe {
            WriteFile(
                self.handle.as_raw_handle(),
                buf.as_ptr(),
                buf.len().min(u32::MAX as usize) as u32,
                &mut written,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(written as usize)
    }

    /// 同步寫入沒有緩衝，`WriteFile` 回傳時資料已送出。
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// 在 named pipe 上接受多條連線，每條一個 worker thread。
pub(super) fn serve(engine: &Engine, path: &Path, max_frame: usize) -> anyhow::Result<()> {
    let name = pipe_name(path)?;
    let sid = jc_windows::current_user_sid()
        .ok_or_else(|| anyhow!("取不到目前使用者 SID，無法建立含 AppContainer 的 SDDL"))?;
    log::info!("jc-engine 監聽 named pipe {name}（多連線）");

    loop {
        let pipe = PipeStream::listen(&name, &sid)
            .with_context(|| format!("建立 named pipe 失敗：{name}"))?;
        // 空閒 instance 也要能被 drain／shutdown 中斷，否則 accept 會一直卡在
        // ConnectNamedPipe。連上之後立刻清掉，避免誤砍 worker 的 ReadFile。
        let idle_handle = pipe.handle.as_raw_handle() as isize;
        engine.set_listener_disconnect(Arc::new(move || cancel_io(idle_handle)));
        if let Err(err) = pipe.connect() {
            if engine.is_shutdown() || engine.is_drain_done() {
                break;
            }
            log::warn!("等待 named pipe 連線失敗：{err}");
            continue;
        }
        engine.clear_listener_disconnect();
        log::debug!("接受 named pipe 連線");
        let connection = engine.accept();
        let handle = pipe.handle.as_raw_handle() as isize;
        connection.set_disconnect(Arc::new(move || cancel_io(handle)));
        let spawned = thread::Builder::new()
            .name("jc-engine-conn".into())
            .spawn(move || {
                let mut view = connection.into_engine();
                let mut input = BufReader::new(&pipe);
                let mut output = BufWriter::new(&pipe);
                if let Err(err) = serve_connection(&mut view, &mut input, &mut output, max_frame) {
                    log::warn!("連線結束：{err}");
                }
                if let Err(err) = output.flush() {
                    log::warn!("寫出最後一個回應失敗：{err}");
                }
                view.finish_connection();
            });
        if let Err(err) = spawned {
            log::warn!("衍生連線 worker 失敗：{err}");
        }
        if engine.is_shutdown() || engine.is_drain_done() {
            log::debug!("行程結束旗標已設定，離開 accept 迴圈");
            break;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sddl_matches_0c() {
        assert_eq!(
            security_descriptor_sddl("S-1-5-21-1-2-3-1001"),
            "D:P(A;;GA;;;S-1-5-21-1-2-3-1001)(A;;GRGW;;;S-1-15-2-1)\
             (A;;GRGW;;;S-1-15-2-2)S:(ML;;NW;;;LW)"
        );
    }

    #[test]
    fn pipe_name_uses_last_segment() {
        assert_eq!(
            pipe_name(Path::new(r"\\.\pipe\jc-ime-1001")).unwrap(),
            r"\\.\pipe\jc-ime-1001"
        );
    }
}
