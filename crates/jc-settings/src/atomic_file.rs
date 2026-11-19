//! 原子寫檔：先寫同目錄的暫存檔，成功後再改名。
//!
//! 對照前身 `internal/atomicfile`。設定檔與 `active.json` 都靠這支工具避免
//! 中斷時留下半份檔案。暫存檔建立在目的目錄內，改名才不會跨檔案系統。

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use serde::Serialize;

/// 暫存檔名稱的序號，避免同一個 process 併發寫入時撞名。
static NEXT: AtomicU32 = AtomicU32::new(0);

/// 把 `bytes` 原子寫進 `path`。
///
/// 目錄不存在時以 0700 建立（Unix）。寫入、`sync`、改名任一步失敗都會刪掉
/// 暫存檔，原本的檔案保持不動。
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".settings-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> io::Result<()> {
        let mut file = open_private(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// 把 `value` 序列化成縮排 JSON（結尾加換行）後原子寫入。
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

fn create_dir_all(dir: &Path) -> io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

fn open_private(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_atomic_replaces_content() {
        let dir = std::env::temp_dir().join(format!("jc-settings-atomic-{}", std::process::id()));
        let path = dir.join("settings.json");
        write_atomic(&path, b"first").expect("第一次寫入");
        write_atomic(&path, b"second").expect("第二次寫入");
        assert_eq!(fs::read(&path).expect("讀回"), b"second");
        let leftovers = fs::read_dir(&dir)
            .expect("列目錄")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name() != "settings.json")
            .count();
        assert_eq!(leftovers, 0, "暫存檔應該清乾淨");
        let _ = fs::remove_dir_all(&dir);
    }
}
