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

use windows_core::{BOOL, HRESULT};

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
///
/// 一般 COM 方法用這個：panic 時回 `E_FAIL`。按鍵邊界要用 [`guarded_bool`]。
pub fn guarded<T, F: FnOnce() -> windows_core::Result<T>>(
    what: &str,
    f: F,
) -> windows_core::Result<T> {
    guarded_with(what, Err(e_fail()), || {}, f)
}

/// 按鍵邊界的攔截：panic 時降級成「這個鍵我不處理」（`Ok(FALSE)`）。
///
/// 回 `FALSE` 而不是 `E_FAIL`：TSF 對按鍵回錯誤碼的行為各家宿主不一致，
/// 「沒處理」是每一家都懂的意思，那一鍵會原樣送進文件。
pub fn guarded_bool<F: FnOnce() -> windows_core::Result<BOOL>>(
    what: &str,
    f: F,
) -> windows_core::Result<BOOL> {
    guarded_bool_with(what, || {}, f)
}

/// 同 [`guarded_bool`]，額外帶一個 panic 後的重設動作。
///
/// `service` 用這個把組字狀態歸零；`log.rs` 不依賴 `service`，避免循環。
pub fn guarded_bool_with<F, G>(what: &str, on_panic: G, f: F) -> windows_core::Result<BOOL>
where
    F: FnOnce() -> windows_core::Result<BOOL>,
    G: FnOnce(),
{
    guarded_with(what, Ok(BOOL(0)), on_panic, f)
}

/// 攔截 panic 並降級成 `fallback`；panic 後先執行 `on_panic` 再回降級值。
///
/// `on_panic` 是狀態重設，本身不得再 panic：這裡再包一層 `catch_unwind`，
/// 失敗只記 log，仍然回 `fallback`。
pub fn guarded_with<T, F, G>(what: &str, fallback: T, on_panic: G, f: F) -> T
where
    F: FnOnce() -> T,
    G: FnOnce(),
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // 注入點一定要在閉包裡：放外面就等於沒有保護，panic 會直接穿過 COM 邊界。
        maybe_panic(what);
        f()
    })) {
        Ok(value) => value,
        Err(_) => {
            log(&format!("panic 已攔截：{what}"));
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(on_panic)).is_err() {
                log(&format!("panic 後重設失敗：{what}"));
            }
            fallback
        }
    }
}

/// `E_FAIL`：不依賴 windows crate 的 target-specific 常數。
fn e_fail() -> windows_core::Error {
    windows_core::Error::from_hresult(HRESULT(0x8000_4005u32 as i32))
}

/// `panic-test` 的注入點：`%LOCALAPPDATA%\jc-ime\panic.on` 的內容是 `what` 的子字串時炸一次。
///
/// 只在 `panic-test` feature 開啟時編譯，預設關閉，release DLL 不受影響。
#[cfg(feature = "panic-test")]
fn maybe_panic(what: &str) {
    let Some(dir) = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|base| base.join("jc-ime"))
    else {
        return;
    };
    if !take_panic_request(&dir, what) {
        return;
    }
    panic!("panic-test：在「{what}」故意炸一次（來自 panic.on）");
}

/// 讀 `dir/panic.on`：內容比對 `what` 的子字串，符合就**先刪檔**再回 true（只炸一次）。
#[cfg(feature = "panic-test")]
fn take_panic_request(dir: &std::path::Path, what: &str) -> bool {
    let file = dir.join("panic.on");
    let Ok(want) = std::fs::read_to_string(&file) else {
        return false;
    };
    let want = want.trim();
    if want.is_empty() || !what.contains(want) {
        return false;
    }
    // 先刪再炸：反過來就刪不掉，使用者會卡在每一鍵都炸的狀態。
    let _ = std::fs::remove_file(&file);
    true
}

/// 正式版什麼都不做，而且會被整個最佳化掉。
#[cfg(not(feature = "panic-test"))]
#[inline(always)]
fn maybe_panic(_what: &str) {}

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// 按鍵邊界 panic 要降級成「沒處理」，不是 E_FAIL：那一鍵原樣送進文件。
    #[test]
    fn guarded_bool_panic_returns_false() {
        let r = guarded_bool("OnKeyDown", || panic!("裝的"));
        assert_eq!(r.unwrap(), BOOL(0), "panic 要回 Ok(FALSE)");
    }

    #[test]
    fn guarded_bool_passes_value_through() {
        let r = guarded_bool("OnKeyDown", || Ok(BOOL(1)));
        assert!(r.unwrap().as_bool(), "沒 panic 時原樣回傳");
    }

    /// 一般 COM 方法維持 E_FAIL，只有按鍵邊界降級成 FALSE。
    #[test]
    fn guarded_panic_returns_e_fail() {
        let r: windows_core::Result<()> = guarded("DoEditSession", || panic!("裝的"));
        assert_eq!(r.unwrap_err().code(), HRESULT(0x8000_4005u32 as i32));
    }

    /// panic 後要先重設狀態，才回降級值。
    #[test]
    fn guarded_bool_runs_reset_after_panic() {
        let reset = Cell::new(false);
        let r = guarded_bool_with("OnKeyDown", || reset.set(true), || panic!("裝的"));
        assert!(!r.unwrap().as_bool());
        assert!(reset.get(), "panic 後要呼叫重設");
    }

    #[test]
    fn reset_not_called_without_panic() {
        let reset = Cell::new(false);
        let r = guarded_bool_with("OnKeyDown", || reset.set(true), || Ok(BOOL(0)));
        assert!(!r.unwrap().as_bool());
        assert!(!reset.get(), "沒 panic 不該重設");
    }

    /// 重設本身 panic 也不能穿出去，降級值照樣回。
    #[test]
    fn reset_panic_does_not_escape() {
        let r = guarded_bool_with("OnKeyDown", || panic!("重設也炸"), || panic!("裝的"));
        assert!(!r.unwrap().as_bool(), "重設失敗仍要回 Ok(FALSE)");
    }

    #[cfg(feature = "panic-test")]
    fn panic_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jc-tsf-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 注入只炸一次：先刪檔再炸，下一次按鍵才能驗「重設後還能組字」。
    #[cfg(feature = "panic-test")]
    #[test]
    fn panic_request_is_one_shot() {
        let dir = panic_dir("panic-once");
        std::fs::write(dir.join("panic.on"), "OnKey\n").unwrap();
        assert!(take_panic_request(&dir, "OnKeyDown"), "子字串比對要命中");
        assert!(
            !take_panic_request(&dir, "OnKeyDown"),
            "炸完就刪檔，只炸一次"
        );
        assert!(!dir.join("panic.on").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 沒命中的邊界不能把注入檔吃掉。
    #[cfg(feature = "panic-test")]
    #[test]
    fn panic_request_keeps_file_for_other_boundaries() {
        let dir = panic_dir("panic-keep");
        std::fs::write(dir.join("panic.on"), "OnKeyDown").unwrap();
        assert!(!take_panic_request(&dir, "OnSetFocus"));
        assert!(dir.join("panic.on").exists(), "沒命中不能刪檔");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
