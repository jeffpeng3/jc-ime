//! 設定檔路徑解析。
//!
//! 對照前身 `internal/settings/paths.go`。前身直接讀 `runtime.GOOS` 與使用者
//! 家目錄；本 crate 改成由呼叫端傳入資料根目錄，Linux 以外的平台照傳入值使用。

use std::path::{Path, PathBuf};

/// 設定檔名。
pub const FILE_NAME: &str = "settings.json";

/// 回傳資料根目錄下的設定檔路徑。
///
/// Linux 的標準 `zuin` 資料目錄改用同層的 `0c`；其他自訂目錄照用，避免隔離
/// 測試或自訂設定互相影響。
pub fn settings_path(data_root: &Path) -> PathBuf {
    if is_linux_data_dir(data_root, "zuin") {
        if let Some(parent) = data_root.parent() {
            return parent.join("0c").join(FILE_NAME);
        }
    }
    data_root.join(FILE_NAME)
}

/// 由個人詞庫檔（例如 `user.json`）推算設定檔路徑。
///
/// 呼叫端傳的是資料根目錄內的檔案時，設定檔與它放在同一個資料根目錄。
pub fn settings_path_for_store(user_store: &Path) -> PathBuf {
    match user_store.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => settings_path(dir),
        _ => settings_path(Path::new(".")),
    }
}

/// 只為 Linux 的標準新設定位置提供舊路徑；自訂檔名或其他平台不回傳。
pub fn legacy_path(path: &Path) -> Option<PathBuf> {
    let dir = path.parent()?;
    if path.file_name().is_some_and(|name| name == FILE_NAME) && is_linux_data_dir(dir, "0c") {
        return Some(dir.parent()?.join("zuin").join(FILE_NAME));
    }
    None
}

fn is_linux_data_dir(dir: &Path, name: &str) -> bool {
    if !dir.is_absolute() {
        return false;
    }
    if dir.file_name().is_none_or(|base| base != name) {
        return false;
    }
    let Some(share) = dir.parent() else {
        return false;
    };
    if share.file_name().is_none_or(|base| base != "share") {
        return false;
    }
    let Some(local) = share.parent() else {
        return false;
    };
    local.file_name().is_some_and(|base| base == ".local")
}
