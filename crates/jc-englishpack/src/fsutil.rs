//! 檔案讀寫小工具：限制大小、拒絕符號連結、原子寫入。

use std::io::Write;
use std::path::Path;

use crate::error::BuildError;

/// 讀取一般檔案；拒絕符號連結與超過 `limit` 的內容。
pub(crate) fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, BuildError> {
    let info = std::fs::symlink_metadata(path).map_err(|err| BuildError::io(path, err))?;
    if !info.file_type().is_file() {
        return Err(BuildError::Source(format!(
            "不是一般檔案：{}",
            path.display()
        )));
    }
    if info.len() > limit {
        return Err(BuildError::Source(format!("檔案過大：{}", path.display())));
    }
    let data = std::fs::read(path).map_err(|err| BuildError::io(path, err))?;
    if data.len() as u64 > limit {
        return Err(BuildError::Source(format!("檔案過大：{}", path.display())));
    }
    Ok(data)
}

/// 先寫暫存檔再改名，失敗時不留下半成品。
pub(crate) fn write_atomic(path: &Path, data: &[u8]) -> Result<(), BuildError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "out".to_string());
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    let result = (|| -> Result<(), BuildError> {
        let mut file = std::fs::File::create(&tmp).map_err(|err| BuildError::io(&tmp, err))?;
        file.write_all(data)
            .map_err(|err| BuildError::io(&tmp, err))?;
        file.sync_all().map_err(|err| BuildError::io(&tmp, err))?;
        std::fs::rename(&tmp, path).map_err(|err| BuildError::io(path, err))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}
