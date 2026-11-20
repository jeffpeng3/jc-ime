//! 記錄：`OutputDebugStringW` 加上可選的日誌檔。
//!
//! 日誌一律走 `OutputDebugStringW`（DebugView／VS 輸出視窗看得到）。設定環境
//! 變數 `JC_TSF_LOG=1` 時另外附加到 `%LOCALAPPDATA%\jc-ime\tsf.log`。兩個管道
//! 都不能影響輸入流程，所以任何失敗都直接吞掉。

use std::fmt;
use std::io::Write;
use std::sync::OnceLock;

use windows::core::PCWSTR;
use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;

/// 是否要把日誌寫進檔案；只在第一次使用時讀環境變數。
static FILE_LOG: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();

/// 日誌檔路徑；`JC_TSF_LOG` 不是 `1` 時回 `None`。
fn log_path() -> Option<std::path::PathBuf> {
    FILE_LOG
        .get_or_init(|| {
            if std::env::var("JC_TSF_LOG").ok().as_deref() != Some("1") {
                return None;
            }
            let base = std::env::var("LOCALAPPDATA")
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().into_owned());
            let path = std::path::PathBuf::from(base)
                .join("jc-ime")
                .join("tsf.log");
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            Some(path)
        })
        .clone()
}

/// 寫一行日誌。
pub fn write(level: &str, args: fmt::Arguments<'_>) {
    let text = format!("[jc-tsf {} {}] {}\n", std::process::id(), level, args);

    let debug: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: debug 以 NUL 結尾，且呼叫期間保持存活。
    unsafe { OutputDebugStringW(PCWSTR(debug.as_ptr())) };

    if let Some(path) = log_path() {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = file.write_all(text.as_bytes());
        }
    }
}

/// 一般訊息。
pub fn info(args: impl Into<String>) {
    write("info", format_args!("{}", args.into()));
}

/// 需要注意但不致命的訊息。
pub fn warn(args: impl Into<String>) {
    write("warn", format_args!("{}", args.into()));
}

/// 除錯用訊息；只在 `JC_TSF_LOG=1` 時真的落檔，但一律送到 debugger。
pub fn debug(args: impl Into<String>) {
    write("debug", format_args!("{}", args.into()));
}
