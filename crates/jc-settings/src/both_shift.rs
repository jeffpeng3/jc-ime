//! 把舊版的右 Shift 換成左右 Shift（#206）。
//!
//! **已停用，保留相容**：中英切換現在固定為單按左／右 Shift，不再需要這套
//! 一次性遷移。目前沒有生產呼叫端；函式與測試留著，避免舊整合程式找不到 API。
//!
//! 對照前身 `internal/settings/both_shift.go`。0.2.4 以前每次存檔都把當時的
//! 預設 `right` 寫進 `toggle_key`，檔案分不出那是使用者選的還是順手寫進去的，
//! 所以第一次以新版啟動時改一次 `toggle_key=right` 為 `both` 並記下 marker。
//!
//! 平台預設值由呼叫端決定；這裡只做檔案層級的資料遷移，不含檔案鎖。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::atomic_file::write_atomic;
use crate::paths;
use crate::settings::{Settings, SettingsError, FILE_NAME, TOGGLE_BOTH, TOGGLE_RIGHT};

/// marker 檔名：記下這個設定檔已經不必再換成左右 Shift。
pub const MARKER: &str = "toggle-key-both";

/// marker 的完整路徑：設定檔旁的 `update` 目錄。
pub fn both_shift_marker(settings_path: &Path) -> PathBuf {
    match settings_path.parent() {
        Some(dir) => dir.join("update").join(MARKER),
        None => PathBuf::from("update").join(MARKER),
    }
}

/// Windows 的中英切換鍵預設從右 Shift 改成左右 Shift，沒有自己選過的舊用戶
/// 跟著改一次。
///
/// `user_store` 是個人詞庫檔；設定檔在它的資料根目錄。回傳是否改寫了檔案。
pub fn adopt_both_shift(user_store: &Path) -> Result<bool, SettingsError> {
    let path = paths::settings_path_for_store(user_store);
    if both_shift_marker(&path).exists() {
        return Ok(false);
    }
    let (bytes, missing, failure) = read_file(&path);
    if let Some(err) = failure {
        return Err(SettingsError::Io(err));
    }
    let wrote = if missing {
        false
    } else {
        adopt(&path, &bytes)?
    };
    mark_both_shift(&path)?;
    Ok(wrote)
}

/// 記下這個設定檔已經不必再換成左右 Shift。
pub fn mark_both_shift(settings_path: &Path) -> io::Result<()> {
    let marker = both_shift_marker(settings_path);
    if let Some(dir) = marker.parent() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(dir)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options.open(&marker)?;
    file.write_all(b"206\n")
}

fn adopt(path: &Path, bytes: &[u8]) -> Result<bool, SettingsError> {
    let settings =
        Settings::from_json_for(bytes, "linux").map_err(|err| SettingsError::Corrupt {
            path: path.to_path_buf(),
            detail: err.to_string(),
        })?;
    if settings.read_only() || settings.profiles.is_some() {
        return Ok(false);
    }
    let members = ordered_members(bytes).map_err(|detail| SettingsError::Corrupt {
        path: path.to_path_buf(),
        detail,
    })?;
    let mut found = false;
    for (key, value) in &members {
        if key == "toggle_key" {
            if value.as_str() != Some(TOGGLE_RIGHT) {
                return Ok(false);
            }
            found = true;
        }
    }
    if !found {
        return Ok(false);
    }
    // 只改切换鍵與舊欄位 left_shift 的值；其他成員照原樣寫回。
    let mut out = serde_json::Map::new();
    for (key, value) in members {
        if key == "toggle_key" {
            out.insert(key, Value::String(TOGGLE_BOTH.to_string()));
        } else if key == "left_shift" && value == Value::Bool(false) {
            out.insert(key, Value::Bool(true));
        } else {
            out.insert(key, value);
        }
    }
    let mut bytes = serde_json::to_vec_pretty(&Value::Object(out)).map_err(io::Error::other)?;
    bytes.push(b'\n');
    // 寫入前再核對一次：只有切換鍵變了。
    let check = Settings::from_json_for(&bytes, "linux").map_err(|err| SettingsError::Corrupt {
        path: path.to_path_buf(),
        detail: err.to_string(),
    })?;
    if check.toggle_key != TOGGLE_BOTH || check.read_only() {
        return Err(SettingsError::Corrupt {
            path: path.to_path_buf(),
            detail: "改寫後驗證失敗".to_string(),
        });
    }
    write_atomic(path, &bytes)?;
    Ok(true)
}

fn read_file(path: &Path) -> (Vec<u8>, bool, Option<io::Error>) {
    match fs::read(path) {
        Ok(bytes) => (bytes, false, None),
        Err(err) if err.kind() == io::ErrorKind::NotFound => (Vec::new(), true, None),
        Err(err) => (Vec::new(), false, Some(err)),
    }
}

/// 依檔案中的順序取出最上層的成員。
fn ordered_members(bytes: &[u8]) -> Result<Vec<(String, Value)>, String> {
    struct Ordered(Vec<(String, Value)>);

    impl<'de> Deserialize<'de> for Ordered {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct Members;

            impl<'de> Visitor<'de> for Members {
                type Value = Vec<(String, Value)>;

                fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    formatter.write_str("JSON 物件")
                }

                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                    let mut out = Vec::new();
                    while let Some((key, value)) = map.next_entry::<String, Value>()? {
                        out.push((key, value));
                    }
                    Ok(out)
                }
            }

            deserializer.deserialize_map(Members).map(Ordered)
        }
    }

    serde_json::from_slice::<Ordered>(bytes)
        .map(|ordered| ordered.0)
        .map_err(|err| err.to_string())
}

/// 設定檔路徑；方便呼叫端組 marker 路徑。
pub fn settings_file(data_root: &Path) -> PathBuf {
    data_root.join(FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_marker_sits_beside_settings() {
        let path = Path::new("/data/0c/settings.json");
        assert_eq!(
            both_shift_marker(path),
            PathBuf::from("/data/0c/update/toggle-key-both")
        );
        assert_eq!(settings_file(Path::new("/data/0c")), path);
    }
}
