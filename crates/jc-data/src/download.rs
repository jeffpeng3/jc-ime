//! 下載項（`shipped: false`）的狀態查詢。
//!
//! 這裡只讀 manifest 與檔案系統，不連網、不下載。真正的下載器在
//! `jc-cli`，因為 HTTP 客戶端不屬於資料格式層。
//!
//! 狀態分三類，對應安裝器與 `jc-cli data status` 要回答的問題：
//!
//! | 狀態 | 意義 | 下一步 |
//! |---|---|---|
//! | [`DownloadState::Present`] | 檔案存在、大小與 SHA-256 都相符 | 不用動作 |
//! | [`DownloadState::Missing`] | 檔案不存在 | 下載 |
//! | [`DownloadState::SizeMismatch`]／[`DownloadState::HashMismatch`] | 內容不對 | 刪掉重下 |
//!
//! 另外 [`DownloadStatus::partial_bytes`] 會回報 `.part` 暫存檔的大小，讓使用者
//! 知道中斷的下載還能不能續傳。查詢會實際算 SHA-256，因此對 130 MB 的模型
//! 需要讀完整個檔案；引擎啟動時不該呼叫這個函式，那只比大小（見
//! [`crate::manifest::VerifyLevel::Startup`]）。

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::manifest::{sha256_file, DataFile, DataManifest, DataRole};

/// 下載中的暫存檔後綴；完成後才改名成正式檔名。
pub const PART_SUFFIX: &str = ".part";

/// 單一下載項的狀態。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DownloadState {
    /// 檔案存在且大小、SHA-256 都相符。
    Present,
    /// 檔案不存在，也沒有可續傳的暫存檔。
    Missing,
    /// 檔案存在但大小不符。
    SizeMismatch {
        /// manifest 記錄的大小。
        expected: u64,
        /// 實際大小。
        actual: u64,
    },
    /// 檔案存在、大小相符，但 SHA-256 不符。
    HashMismatch {
        /// manifest 記錄的雜湊。
        expected: String,
        /// 實際算出的雜湊。
        actual: String,
    },
    /// 檔案存在但讀不到。
    Unreadable {
        /// 錯誤訊息。
        message: String,
    },
}

impl DownloadState {
    /// 這個項目是否已經是可以用的完整檔案。
    pub fn is_present(&self) -> bool {
        matches!(self, Self::Present)
    }

    /// 這個項目是否完全不存在（沒有正式檔，也沒有暫存檔）。
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }

    /// 這個項目是否存在但內容不對。
    pub fn is_mismatch(&self) -> bool {
        matches!(
            self,
            Self::SizeMismatch { .. } | Self::HashMismatch { .. } | Self::Unreadable { .. }
        )
    }
}

/// 一個下載項的查詢結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DownloadStatus {
    /// 相對於資料根目錄的路徑。
    pub path: String,
    /// 檔案角色。
    pub role: DataRole,
    /// 下載來源；manifest 沒寫時是 `None`，下載器會拒絕該項。
    pub url: Option<String>,
    /// manifest 記錄的大小。
    pub bytes: u64,
    /// manifest 記錄的 SHA-256。
    pub sha256: String,
    /// 目前狀態。
    pub state: DownloadState,
    /// `.part` 暫存檔的大小；沒有暫存檔時是 `None`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial_bytes: Option<u64>,
}

/// 整份 manifest 的下載項查詢結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DownloadReport {
    /// 查詢的資料根目錄。
    pub root: PathBuf,
    /// manifest 的資料版本標籤。
    pub manifest_version: String,
    /// 逐項狀態，順序與 manifest 相同。
    pub items: Vec<DownloadStatus>,
}

impl DownloadReport {
    /// 查詢所有 `shipped: false` 的項目。
    ///
    /// 會實際算 SHA-256；檔案很大時這個函式也跟著變慢。
    pub fn inspect(root: impl AsRef<Path>, manifest: &DataManifest) -> Self {
        let root = root.as_ref();
        let items = manifest
            .download_files()
            .map(|file| inspect_file(root, file))
            .collect();
        Self {
            root: root.to_path_buf(),
            manifest_version: manifest.version.clone(),
            items,
        }
    }

    /// 已存在的項目數。
    pub fn present(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.state.is_present())
            .count()
    }

    /// 缺少的項目數。
    pub fn missing(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.state.is_missing())
            .count()
    }

    /// 存在但內容不對的項目數。
    pub fn mismatched(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.state.is_mismatch())
            .count()
    }

    /// 全部項目都可用時回傳 `true`。
    pub fn is_complete(&self) -> bool {
        self.items.iter().all(|item| item.state.is_present())
    }

    /// 給記錄用的一行摘要。
    pub fn summary(&self) -> String {
        format!(
            "已存在 {}/{}，缺少 {}，不符 {}",
            self.present(),
            self.items.len(),
            self.missing(),
            self.mismatched()
        )
    }
}

/// 查詢單一 manifest 記錄。
pub fn inspect_file(root: impl AsRef<Path>, file: &DataFile) -> DownloadStatus {
    let root = root.as_ref();
    let target = root.join(&file.path);
    let partial_bytes = fs::metadata(part_path(&target))
        .ok()
        .filter(|meta| meta.is_file())
        .map(|meta| meta.len())
        .filter(|bytes| *bytes > 0);
    let state = state_of(&target, file);
    DownloadStatus {
        path: file.path.clone(),
        role: file.role,
        url: file.url.clone(),
        bytes: file.bytes,
        sha256: file.sha256.clone(),
        state,
        partial_bytes,
    }
}

/// 正式檔旁邊的 `.part` 路徑。
pub fn part_path(target: impl AsRef<Path>) -> PathBuf {
    let target = target.as_ref();
    let mut name = target.as_os_str().to_os_string();
    name.push(PART_SUFFIX);
    PathBuf::from(name)
}

fn state_of(target: &Path, file: &DataFile) -> DownloadState {
    let meta = match fs::metadata(target) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return DownloadState::Missing,
        Err(err) => {
            return DownloadState::Unreadable {
                message: err.to_string(),
            }
        }
    };
    if !meta.is_file() {
        return DownloadState::Unreadable {
            message: "不是一般檔案".to_string(),
        };
    }
    let actual = meta.len();
    if actual != file.bytes {
        return DownloadState::SizeMismatch {
            expected: file.bytes,
            actual,
        };
    }
    match sha256_file(target) {
        Err(err) => DownloadState::Unreadable {
            message: err.to_string(),
        },
        Ok((_, actual)) => {
            if actual.eq_ignore_ascii_case(&file.sha256) {
                DownloadState::Present
            } else {
                DownloadState::HashMismatch {
                    expected: file.sha256.clone(),
                    actual,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("jc-data-download-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, rel: &str, bytes: &[u8]) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut file = fs::File::create(path).unwrap();
        file.write_all(bytes).unwrap();
    }

    fn manifest(path: &str, content: &[u8]) -> DataManifest {
        DataManifest {
            schema: crate::manifest::SCHEMA.to_string(),
            version: "test".to_string(),
            generated_at: "2026-11-20T00:00:00Z".to_string(),
            files: vec![DataFile {
                path: path.to_string(),
                bytes: content.len() as u64,
                sha256: sha256_hex(content),
                role: DataRole::ZhSelect,
                license: vec!["L-11".to_string()],
                shipped: false,
                url: Some("https://example.invalid/model.onnx".to_string()),
            }],
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    #[test]
    fn present_requires_size_and_hash() {
        let dir = temp_dir("present");
        let content = b"data";
        write(&dir, "models/zh-select/model.onnx", content);
        let report =
            DownloadReport::inspect(&dir, &manifest("models/zh-select/model.onnx", content));
        assert_eq!(report.present(), 1);
        assert!(report.is_complete());
        assert_eq!(report.summary(), "已存在 1/1，缺少 0，不符 0");
        assert_eq!(report.items[0].state, DownloadState::Present);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_and_partial_are_distinguished() {
        let dir = temp_dir("missing");
        let manifest = manifest("models/zh-select/model.onnx", b"data");
        let report = DownloadReport::inspect(&dir, &manifest);
        assert_eq!(report.missing(), 1);
        assert_eq!(report.items[0].state, DownloadState::Missing);
        assert_eq!(report.items[0].partial_bytes, None);

        // 中斷的下載留下 2 位元組的暫存檔，狀態仍是缺少，但要能看出可續傳。
        write(&dir, "models/zh-select/model.onnx.part", b"da");
        let report = DownloadReport::inspect(&dir, &manifest);
        assert_eq!(report.items[0].state, DownloadState::Missing);
        assert_eq!(report.items[0].partial_bytes, Some(2));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn same_size_with_different_bytes_is_hash_mismatch() {
        let dir = temp_dir("hash");
        let content = b"data";
        // 大小相同、內容不同：分類要走到雜湊，不是大小。
        write(&dir, "models/zh-select/model.onnx", b"datb");
        let report =
            DownloadReport::inspect(&dir, &manifest("models/zh-select/model.onnx", content));
        assert_eq!(report.mismatched(), 1);
        assert_eq!(report.present(), 0);
        assert!(matches!(
            report.items[0].state,
            DownloadState::HashMismatch { .. }
        ));
        assert_eq!(report.summary(), "已存在 0/1，缺少 0，不符 1");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn different_size_is_size_mismatch() {
        let dir = temp_dir("size");
        let content = b"data";
        write(&dir, "models/zh-select/model.onnx", b"data!");
        let report =
            DownloadReport::inspect(&dir, &manifest("models/zh-select/model.onnx", content));
        assert!(matches!(
            report.items[0].state,
            DownloadState::SizeMismatch {
                expected: 4,
                actual: 5
            }
        ));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn part_path_appends_suffix() {
        assert_eq!(
            part_path("/data/models/zh-select/model.onnx"),
            PathBuf::from("/data/models/zh-select/model.onnx.part")
        );
    }

    #[test]
    fn real_root_has_no_download_items() {
        let Some(root) = jc_testdata::root() else {
            eprintln!("缺少測試資料：JC_TESTDATA");
            return;
        };
        // 用現行 catalog 重新產生，避免讀到舊 manifest 裡已移除的角色。
        let manifest = DataManifest::generate_now(&root).unwrap();
        let report = DownloadReport::inspect(&root, &manifest);
        assert_eq!(report.items.len(), 0, "zh-select 改為隨 MSI 散布");
        assert!(report.is_complete());
    }
}
