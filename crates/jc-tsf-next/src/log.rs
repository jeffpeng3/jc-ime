//! 診斷紀錄與 COM 邊界的共用設施（T6 從 T7a 的過渡實作改接真正的 crate 層）。
//!
//! 只記事件與錯誤，不記任何輸入的文字（隱私）。Windows 上寫
//! `%LOCALAPPDATA%\jc-ime\tsf.log`；目錄不存在時改送 OutputDebugString。
//! Linux host 上只保留去重語意，不產生輸出，讓 `cargo check --workspace` 能通過。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use windows_core::HRESULT;

const MAX_BYTES: u64 = 4 << 20;
const ONCE_LIMIT: usize = 32;

/// 目前仍然活著的 COM 物件數量，供 DLL 的 `DllCanUnloadNow` 判斷能否卸載。
/// LangBar 的背景工作與 edit session 也會計入，避免執行緒還在跑就被卸載。
pub static LIVE_OBJECTS: AtomicUsize = AtomicUsize::new(0);

/// 建立一個 COM 物件時計入 DLL 參考。
pub fn object_created() {
    LIVE_OBJECTS.fetch_add(1, Ordering::Relaxed);
}

/// 釋放一個 COM 物件時扣掉 DLL 參考。
pub fn object_destroyed() {
    LIVE_OBJECTS.fetch_sub(1, Ordering::Relaxed);
}

fn path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let base = std::env::var_os("LOCALAPPDATA")?;
    let dir = PathBuf::from(base).join("jc-ime");
    if dir.is_dir() {
        Some(dir.join("tsf.log"))
    } else {
        None
    }
}

/// 除錯紀錄是否開啟：只有 `%LOCALAPPDATA%\jc-ime\tsf-debug` 這個檔案存在時才寫
/// （會記按鍵的虛擬鍵碼，等於記下輸入內容，日常不可開啟）。
pub fn debug_enabled() -> bool {
    use std::sync::OnceLock;
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("LOCALAPPDATA")
            .map(|b| PathBuf::from(b).join("jc-ime").join("tsf-debug").is_file())
            .unwrap_or(false)
    })
}

/// 寫一行事件紀錄（不含文字）。
pub fn log(msg: &str) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let line = format!("{} pid={} {}\n", now, std::process::id(), msg);
    if let Some(p) = path() {
        if let Ok(meta) = std::fs::metadata(&p) {
            if meta.len() > MAX_BYTES {
                let _ = std::fs::remove_file(&p);
            }
        }
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&p) {
            let _ = f.write_all(line.as_bytes());
            return;
        }
    }
    #[cfg(windows)]
    {
        use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
        use windows_core::PCWSTR;
        let mut wide: Vec<u16> = format!("jc-tsf: {line}").encode_utf16().collect();
        wide.push(0);
        unsafe { OutputDebugStringW(PCWSTR::from_raw(wide.as_ptr())) };
    }
    #[cfg(not(windows))]
    {
        let _ = line;
    }
}

/// 同一則訊息每個行程只記一次，最多 [`ONCE_LIMIT`] 則。訊息不可含文字或鍵碼。
pub fn once(msg: &str) {
    static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if seen.len() < ONCE_LIMIT && !seen.iter().any(|m| m == msg) {
        seen.push(msg.to_owned());
        log(msg);
    }
}

/// 宿主程式的執行檔名稱（不含路徑）。
pub fn exe_name() -> &'static str {
    use std::sync::OnceLock;
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "?".into())
    })
}

/// panic 時記下發生位置（檔案:行號），再由 [`guarded`] 記下攔截到的入口。
pub fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            let at = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_else(|| "?".into());
            log(&format!("panic 位置：{at}"));
        }));
    });
}

/// 攔截 panic：COM 邊界不得 unwind（漏一個，panic 穿過 FFI 就終止宿主）。
pub fn guarded<T, F: FnOnce() -> windows_core::Result<T>>(
    what: &str,
    f: F,
) -> windows_core::Result<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(_) => {
            log(&format!("panic 已攔截：{what}"));
            // E_FAIL，不依賴 windows crate 的 target-specific 常數。
            Err(windows_core::Error::from_hresult(HRESULT(
                0x8000_4005u32 as i32,
            )))
        }
    }
}

#[allow(unused_macros)]
macro_rules! zlog {
    ($($arg:tt)*) => { $crate::log::log(&format!($($arg)*)) };
}
#[allow(unused_macros)]
macro_rules! zdebug {
    ($($arg:tt)*) => {
        if $crate::log::debug_enabled() {
            $crate::log::log(&format!("debug {}", format!($($arg)*)))
        }
    };
}
#[allow(unused_imports)] // Linux 上只有 session 編譯，macro 沒有使用端。
pub(crate) use zdebug;
#[allow(unused_imports)]
pub(crate) use zlog;

/// 把 Rust 字串轉成以 NUL 結尾的 UTF-16（給 Win32 的 PCWSTR 用）。
#[cfg(windows)]
pub(crate) fn wide(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}
