//! 安裝日誌：一行一個動作，寫入後立即 `flush` 並 `sync_all`。
//!
//! MSI 的 deferred 自訂動作在 SYSTEM 身分執行，`stderr` 與 `OutputDebugStringW`
//! 都收不到。註冊若在 DLL 內崩潰，唯一留下來的線索就是這個檔案。因此每個標記
//! 都獨立開檔、寫入、同步，不靠 `BufWriter` 的聚合；行程被存取違規終止時，最後
//! 一行仍然在磁碟上。
//!
//! 位置：`%TEMP%\jc-ime-install.log`；取不到 `%TEMP%` 時寫在執行檔同目錄。
//! 路徑會透過環境變數 `JC_INSTALL_LOG` 傳給 `jc-tsf.dll`，讓 DLL 內部的標記
//! 附加到同一份檔案。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// 傳給 `jc-tsf.dll` 的環境變數名稱。
const LOG_ENV: &str = "JC_INSTALL_LOG";

/// 日誌檔名。
const LOG_FILE: &str = "jc-ime-install.log";

/// 這次行程要寫的日誌路徑；`None` 代表連執行檔目錄都取不到。
static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

/// 決定日誌路徑、設定 `JC_INSTALL_LOG`，並寫下第一行。
///
/// 必須在 `LoadLibraryW` 之前呼叫，DLL 才讀得到同一個路徑。
pub fn init() {
    let _ = PATH.set(log_path());
    if let Some(path) = path() {
        // 啟動階段尚未載入 DLL；之後 LoadLibraryW 進來的 jc-tsf.dll 會繼承這份環境。
        std::env::set_var(LOG_ENV, path);
        step(&format!("安裝日誌：{}", path.display()));
    }
}

/// 目前使用的日誌路徑；沒有日誌時回 `None`。
pub fn path() -> Option<&'static Path> {
    PATH.get().and_then(Option::as_ref).map(PathBuf::as_path)
}

/// 寫一個標記。格式是 `[epoch 毫秒] [pid] 內容`。
pub fn step(message: &str) {
    let Some(path) = path() else {
        return;
    };
    let line = format!(
        "[{}] [{}] {}\n",
        timestamp_millis(),
        std::process::id(),
        message.replace('\n', " ")
    );
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = file.write_all(line.as_bytes());
    let _ = file.flush();
    let _ = file.sync_all();
}

/// 目前時間的 Unix epoch 毫秒；系統時鐘早於 epoch 時回 0。
fn timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

/// `%TEMP%\jc-ime-install.log`，取不到 `%TEMP%` 就放在執行檔旁邊。
fn log_path() -> Option<PathBuf> {
    if let Some(temp) = std::env::var_os("TEMP").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(temp).join(LOG_FILE));
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(LOG_FILE)))
}
