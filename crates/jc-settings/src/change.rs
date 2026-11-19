//! 在讀改寫流程裡套用多個設定鍵。
//!
//! 對照前身 `internal/settings/change.go`。前身用檔案鎖序列化 `0c-update set`
//! 與引擎的寫入；本 crate 不含跨 process 的鎖，呼叫端必須自己在
//! `<資料根目錄>/update/settings-write.lock` 上互斥。

use std::path::Path;

use crate::paths;
use crate::settings::{read, save, Settings, SettingsError};

/// 讀改寫設定檔時應持有的鎖檔名（放在資料根目錄的 `update` 目錄）。
pub const WRITE_LOCK: &str = "settings-write.lock";

/// 讀出設定檔，依序套用鍵值，有改變才存檔；回傳是否寫了檔案。
///
/// 設定檔損毀或讀不到時不覆寫、回傳錯誤；來自較新版本時回
/// [`SettingsError::ReadOnly`]。
pub fn change(
    user_store: &Path,
    pairs: &[(&str, &str)],
    goos: &str,
) -> Result<bool, SettingsError> {
    let path = paths::settings_path_for_store(user_store);
    let report = read(&path, goos);
    if let Some(err) = report.error {
        return Err(err);
    }
    let mut settings = report.settings;
    if settings.read_only() {
        return Err(SettingsError::ReadOnly);
    }
    let mut changed = false;
    for (key, value) in pairs {
        let from = settings.value(key);
        settings.set(key, value, goos)?;
        let to = settings.value(key);
        changed = changed || from != to;
    }
    if !changed {
        return Ok(false);
    }
    save(&path, &settings)?;
    Ok(true)
}

/// 只讀地確認目前設定是否可寫。
pub fn writable(settings: &Settings) -> bool {
    !settings.read_only()
}
