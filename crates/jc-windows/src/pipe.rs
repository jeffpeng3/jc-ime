//! Windows named pipe 傳輸。
//!
//! 用 `CreateFileW` 開 `\\.\pipe\<name>`，權限 `GENERIC_READ | GENERIC_WRITE`，
//! 只連既有的 pipe（`OPEN_EXISTING`）。Win32 錯誤一律轉成 [`std::io::Error`]，
//! 不讓錯誤碼出現在公開 API。
//!
//! 非 Windows 平台保留 [`NamedPipe`] 型別，但 [`NamedPipe::connect`] 回
//! [`crate::Error::Unsupported`]，讓 Linux CI 也能編譯與跑測試。

use std::io::{self, Read, Write};
use std::time::Duration;

use crate::Error;

/// pipe 路徑前綴。
const PIPE_PREFIX: &str = r"\\.\pipe\";

/// Windows named pipe 連線。
pub struct NamedPipe {
    /// 平台實作。
    inner: imp::Pipe,
}

impl NamedPipe {
    /// 連線到 `\\.\pipe\<name>`。
    ///
    /// `name` 可以只寫 pipe 名稱，也可以寫完整的 `\\.\pipe\` 路徑。非 Windows
    /// 平台回 [`Error::Unsupported`]。
    pub fn connect(name: &str) -> Result<Self, Error> {
        Self::connect_with_wait(name, Duration::ZERO)
    }

    /// 連線到 `\\.\pipe\<name>`；所有 instance 都忙時用 `WaitNamedPipeW` 等待。
    ///
    /// `wait` 是最多等待的時間。Windows 的 `CreateFile` 在 instance 全被佔用時
    /// 回 `ERROR_PIPE_BUSY`，直接重試只會造成 TSF 端反覆 231；照微軟的用戶端
    /// 做法呼叫 `WaitNamedPipeW`，有 instance 釋出時才再試一次 `CreateFile`。
    /// `wait` 為零時不等待，行為與 [`NamedPipe::connect`] 相同。
    pub fn connect_with_wait(name: &str, wait: Duration) -> Result<Self, Error> {
        imp::Pipe::open_with_wait(&pipe_path(name), wait).map(|inner| Self { inner })
    }
}

impl Read for NamedPipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Write for NamedPipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// 把 pipe 名稱補成完整路徑；已經是完整路徑就原樣使用。
fn pipe_path(name: &str) -> String {
    if name.starts_with(PIPE_PREFIX) {
        name.to_string()
    } else {
        format!("{PIPE_PREFIX}{name}")
    }
}

#[cfg(windows)]
mod imp {
    use std::fs::File;
    use std::io::{self, Read, Write};
    use std::os::windows::io::{FromRawHandle, RawHandle};
    use std::ptr;
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{
        ERROR_PIPE_BUSY, ERROR_SEM_TIMEOUT, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, OPEN_EXISTING};
    use windows_sys::Win32::System::Pipes::WaitNamedPipeW;

    use crate::Error;

    /// 一條已連線的 byte-mode named pipe。
    pub(crate) struct Pipe {
        /// 用 `File` 管理 handle 的關閉。
        file: File,
    }

    impl Pipe {
        /// 開啟既有的 pipe；`ERROR_PIPE_BUSY` 時呼叫 `WaitNamedPipeW` 等待最多
        /// `wait`，timeout 後回報原本的 busy 錯誤。
        ///
        /// `WaitNamedPipeW` 成功只代表「當下有 instance 可用」，`CreateFileW`
        /// 仍可能被別的 client 搶先，因此回到迴圈重試，直到期限用完。
        pub(crate) fn open_with_wait(path: &str, wait: Duration) -> Result<Self, Error> {
            let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
            let deadline = Instant::now() + wait;
            loop {
                // SAFETY: wide 以 NUL 結尾；安全性與共享參數都是預設值。
                let handle = unsafe {
                    CreateFileW(
                        wide.as_ptr(),
                        GENERIC_READ | GENERIC_WRITE,
                        0,
                        ptr::null(),
                        OPEN_EXISTING,
                        0,
                        ptr::null_mut(),
                    )
                };
                if handle != INVALID_HANDLE_VALUE {
                    // SAFETY: handle 剛由 CreateFileW 建立，所有權交給這個 File。
                    let file = unsafe { File::from_raw_handle(handle as RawHandle) };
                    return Ok(Self { file });
                }
                let err = io::Error::last_os_error();
                if err.raw_os_error() != Some(ERROR_PIPE_BUSY as i32) || wait.is_zero() {
                    return Err(Error::Io(err));
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(Error::Io(err));
                }
                let millis = remaining.as_millis().min(u32::MAX as u128) as u32;
                // SAFETY: wide 以 NUL 結尾；WaitNamedPipeW 只讀這個字串。
                let ready = unsafe { WaitNamedPipeW(wide.as_ptr(), millis) };
                if ready == 0 {
                    let wait_err = io::Error::last_os_error();
                    // 等不到 instance：回報原本的 busy，讓呼叫端決定是否重連。
                    if wait_err.raw_os_error() == Some(ERROR_SEM_TIMEOUT as i32) {
                        return Err(Error::Io(err));
                    }
                    return Err(Error::Io(wait_err));
                }
            }
        }
    }

    impl Read for Pipe {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.file.read(buf)
        }
    }

    impl Write for Pipe {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.file.write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.file.flush()
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io::{self, Read, Write};

    use crate::Error;

    /// 佔位傳輸；非 Windows 平台不能用。
    pub(crate) struct Pipe;

    impl Pipe {
        /// 非 Windows 平台沒有 named pipe，等待也沒有意義。
        pub(crate) fn open_with_wait(
            _path: &str,
            _wait: std::time::Duration,
        ) -> Result<Self, Error> {
            Err(Error::Unsupported)
        }
    }

    impl Read for Pipe {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(unsupported())
        }
    }

    impl Write for Pipe {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(unsupported())
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(unsupported())
        }
    }

    /// 串流錯誤也要說清楚是平台不支援。
    fn unsupported() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "這個平台沒有 Windows named pipe",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_name_gets_pipe_prefix() {
        assert_eq!(pipe_path("jc-engine"), r"\\.\pipe\jc-engine");
        assert_eq!(pipe_path("jc-0c-S-1-5-21"), r"\\.\pipe\jc-0c-S-1-5-21");
    }

    #[test]
    fn full_path_is_kept() {
        assert_eq!(pipe_path(r"\\.\pipe\jc-engine"), r"\\.\pipe\jc-engine");
    }

    #[cfg(not(windows))]
    #[test]
    fn connect_is_unsupported_off_windows() {
        let err = NamedPipe::connect("jc-engine").err();
        assert!(matches!(err, Some(Error::Unsupported)));
    }

    /// 唯一的 instance 被 server 佔著時，client 要等它釋出，而不是立刻回 busy。
    #[cfg(windows)]
    #[test]
    fn busy_pipe_waits_for_a_free_instance() {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use std::thread;
        use windows_sys::Win32::Foundation::HANDLE;
        use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
        use windows_sys::Win32::System::Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_MESSAGE,
            PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_WAIT,
        };

        let name = format!(r"\\.\pipe\jc-windows-busy-{}", std::process::id());
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let server = thread::spawn(move || {
            let handle = unsafe {
                CreateNamedPipeW(
                    wide.as_ptr(),
                    PIPE_ACCESS_DUPLEX,
                    PIPE_TYPE_MESSAGE
                        | PIPE_READMODE_MESSAGE
                        | PIPE_WAIT
                        | PIPE_REJECT_REMOTE_CLIENTS,
                    1,
                    4096,
                    4096,
                    0,
                    std::ptr::null(),
                )
            };
            assert!(!handle.is_null(), "建立測試 pipe 失敗");
            let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
            let ok =
                unsafe { ConnectNamedPipe(handle.as_raw_handle() as HANDLE, std::ptr::null_mut()) };
            assert_ne!(ok, 0, "ConnectNamedPipe 失敗");
            thread::sleep(Duration::from_millis(300));
            unsafe {
                DisconnectNamedPipe(handle.as_raw_handle() as HANDLE);
            }
        });

        thread::sleep(Duration::from_millis(100));
        let pipe = NamedPipe::connect_with_wait(&name, Duration::from_secs(2))
            .expect("等待 busy instance 釋出後應連上");
        drop(pipe);
        server.join().expect("server 執行緒不該 panic");
    }
}
