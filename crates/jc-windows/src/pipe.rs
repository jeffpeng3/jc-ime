//! Windows named pipe 傳輸。
//!
//! 用 `CreateFileW` 開 `\\.\pipe\<name>`，權限 `GENERIC_READ | GENERIC_WRITE`，
//! 只連既有的 pipe（`OPEN_EXISTING`）。Win32 錯誤一律轉成 [`std::io::Error`]，
//! 不讓錯誤碼出現在公開 API。
//!
//! 非 Windows 平台保留 [`NamedPipe`] 型別，但 [`NamedPipe::connect`] 回
//! [`crate::Error::Unsupported`]，讓 Linux CI 也能編譯與跑測試。

use std::io::{self, Read, Write};

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
        imp::Pipe::open(&pipe_path(name)).map(|inner| Self { inner })
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

    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, OPEN_EXISTING};

    use crate::Error;

    /// 一條已連線的 byte-mode named pipe。
    pub(crate) struct Pipe {
        /// 用 `File` 管理 handle 的關閉。
        file: File,
    }

    impl Pipe {
        /// 開啟既有的 pipe。
        pub(crate) fn open(path: &str) -> Result<Self, Error> {
            let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
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
            if handle == INVALID_HANDLE_VALUE {
                return Err(Error::Io(io::Error::last_os_error()));
            }
            // SAFETY: handle 剛由 CreateFileW 建立，所有權交給這個 File。
            let file = unsafe { File::from_raw_handle(handle as RawHandle) };
            Ok(Self { file })
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
        /// 非 Windows 平台沒有 named pipe。
        pub(crate) fn open(_path: &str) -> Result<Self, Error> {
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
}
