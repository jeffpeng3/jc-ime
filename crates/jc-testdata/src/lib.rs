//! 測試資料定位工具。
//!
//! 測試資料放在 repo 之外（預設 `/mnt/work/jc/jc-ime-data`），可以用
//! `JC_TESTDATA` 覆寫。資料不存在時測試應該跳過而不是失敗，因此提供
//! [`path`]、[`have`] 與 [`require_data!`]。

use std::env;
use std::path::{Path, PathBuf};

/// 測試資料的預設根目錄。
pub const DEFAULT_ROOT: &str = "/mnt/work/jc/jc-ime-data";

/// 回傳測試資料根目錄。
///
/// 依序採用 `JC_TESTDATA` 環境變數與 [`DEFAULT_ROOT`]；兩者都不存在時
/// 回傳 `None`。空字串視同未設定。
pub fn root() -> Option<PathBuf> {
    let candidate = match env::var_os("JC_TESTDATA") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from(DEFAULT_ROOT),
    };
    candidate.is_dir().then_some(candidate)
}

/// 把相對於資料根目錄的路徑解析成絕對路徑。
///
/// 資料根目錄或目標不存在時回傳 `None`；`rel` 不可以是絕對路徑。
pub fn path(rel: &str) -> Option<PathBuf> {
    let rel = Path::new(rel);
    if rel.is_absolute() {
        return None;
    }
    let p = root()?.join(rel);
    p.exists().then_some(p)
}

/// 回報相對於資料根目錄的檔案或目錄是否存在。
pub fn have(rel: &str) -> bool {
    path(rel).is_some()
}

/// 取得測試資料路徑；資料不存在時印出訊息並從測試 `return`。
///
/// ```ignore
/// #[test]
/// fn lexicon_loads() {
///     let p = jc_testdata::require_data!("build-zh/lexicon.bin");
///     assert!(p.is_file());
/// }
/// ```
#[macro_export]
macro_rules! require_data {
    ($rel:expr) => {
        match $crate::path($rel) {
            Some(p) => p,
            None => {
                eprintln!(
                    "缺少測試資料：{}（JC_TESTDATA 未設定或路徑不存在）",
                    $rel
                );
                return;
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::Mutex;

    /// 序列化所有會改動 `JC_TESTDATA` 的測試，避免平行執行時互相覆蓋。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env<T>(value: Option<&str>, f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old: Option<OsString> = env::var_os("JC_TESTDATA");
        match value {
            Some(v) => env::set_var("JC_TESTDATA", v),
            None => env::remove_var("JC_TESTDATA"),
        }
        let result = f();
        match old {
            Some(v) => env::set_var("JC_TESTDATA", v),
            None => env::remove_var("JC_TESTDATA"),
        }
        result
    }

    #[test]
    fn root_uses_env_when_it_points_to_a_directory() {
        let tmp = env::temp_dir();
        let got = with_env(Some(tmp.to_str().unwrap()), root);
        assert_eq!(got.as_deref(), Some(tmp.as_path()));
    }

    #[test]
    fn root_returns_none_when_env_points_to_missing_path() {
        let missing = env::temp_dir().join("jc-testdata-does-not-exist-4f1b");
        let got = with_env(Some(missing.to_str().unwrap()), root);
        assert_eq!(got, None);
    }

    #[test]
    fn root_ignores_empty_env_and_falls_back_to_default() {
        let got = with_env(Some(""), root);
        if Path::new(DEFAULT_ROOT).is_dir() {
            assert_eq!(got.as_deref(), Some(Path::new(DEFAULT_ROOT)));
        } else {
            assert_eq!(got, None);
        }
    }

    fn fixture_root(tag: &str) -> PathBuf {
        let root = env::temp_dir().join(format!("jc-testdata-{tag}-9c2e"));
        std::fs::create_dir_all(root.join("nested")).expect("建立測試資料夾");
        std::fs::write(root.join("nested/file.txt"), b"ok").expect("建立測試檔");
        root
    }

    #[test]
    fn path_resolves_relative_to_root_and_rejects_absolute() {
        let root = fixture_root("path");
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old = env::var_os("JC_TESTDATA");
        env::set_var("JC_TESTDATA", &root);

        assert_eq!(path("nested/file.txt"), Some(root.join("nested/file.txt")));
        assert_eq!(path("nested"), Some(root.join("nested")));
        assert!(have("nested/file.txt"));
        assert!(!have("nested/other.txt"));

        assert_eq!(path("nested/other.txt"), None);
        assert_eq!(path("/etc/hostname"), None);

        match old {
            Some(v) => env::set_var("JC_TESTDATA", v),
            None => env::remove_var("JC_TESTDATA"),
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn require_data_returns_path_when_present() {
        let root = fixture_root("present");
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old = env::var_os("JC_TESTDATA");
        env::set_var("JC_TESTDATA", &root);

        let p = require_data!("nested/file.txt");
        assert_eq!(p, root.join("nested/file.txt"));
        assert_eq!(std::fs::read(&p).expect("讀取測試檔"), b"ok");

        match old {
            Some(v) => env::set_var("JC_TESTDATA", v),
            None => env::remove_var("JC_TESTDATA"),
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn require_data_skips_without_panicking_when_missing() {
        let missing = env::temp_dir().join("jc-testdata-missing-9c2e");
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        env::set_var("JC_TESTDATA", &missing);

        // 這裡真的會提早 return；後面的斷言不應該被執行。
        let p = require_data!("anything");
        panic!("缺資料時 require_data! 應該已經 return，卻得到 {p:?}");
    }
}
