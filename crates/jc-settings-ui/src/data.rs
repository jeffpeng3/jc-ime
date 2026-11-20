//! 資料根目錄的唯讀檢查：manifest 版本與模型檔是否存在。
//!
//! 只讀檔案，不改資料。判斷規則跟 `jc-engine` 的載入路徑一致：模型集合
//! manifest 在 `models/manifest.json`，日文神經模型與詞典 pack 用固定的相對
//! 路徑。

use std::fs;
use std::path::{Path, PathBuf};

use jc_settings::FileState;

/// 資料根目錄的目前狀態。
pub struct DataStatus {
    /// 資料根目錄。
    pub root: PathBuf,
    /// 設定檔路徑。
    pub settings_path: PathBuf,
    /// 設定檔的讀取狀態。
    pub settings_state: FileState,
    /// `models/manifest.json` 的 schema。
    pub model_manifest: Option<String>,
    /// `build-zh/manifest.json` 的摘要。
    pub zh_manifest: Option<String>,
    /// 注音選字模型權重。
    pub zh_select: Option<PathBuf>,
    /// 日文神經模型（Jinen）。
    pub jinen: Option<PathBuf>,
    /// 日文 reranker。
    pub reranker: Option<PathBuf>,
    /// 日文分詞器。
    pub tokenizer: Option<PathBuf>,
    /// 日文詞典 pack。
    pub ja_pack: Option<PathBuf>,
}

impl DataStatus {
    /// 檢查資料根目錄；缺少的檔案只在欄位留 `None`。
    pub fn probe(root: &Path, settings_path: &Path, settings_state: FileState) -> Self {
        Self {
            root: root.to_path_buf(),
            settings_path: settings_path.to_path_buf(),
            settings_state,
            model_manifest: model_manifest(root),
            zh_manifest: zh_manifest(root),
            zh_select: first_existing(
                root,
                &[
                    "models/zh-select/model.q8_0.gguf",
                    "models/zh-select/model.safetensors",
                    "models/zh-select/model.onnx",
                ],
            ),
            jinen: existing(
                root,
                "models/external/jinen-v2-small/jinen-v2-small-Q5_K_M.gguf",
            ),
            reranker: existing(root, "models/reranker-extra/model.int8.onnx"),
            tokenizer: existing(root, "models/ja-tokenizer/public_core_ja.model"),
            ja_pack: first_existing(
                root,
                &[
                    "packs/ja-lexicon-mozc-v2.pack",
                    "packs/ja-lexicon-public-core.pack",
                ],
            ),
        }
    }

    /// 注音選字權重是否存在。
    pub fn zh_select_ready(&self) -> bool {
        self.zh_select.is_some()
    }
}

fn existing(root: &Path, relative: &str) -> Option<PathBuf> {
    let path = root.join(relative);
    path.is_file().then_some(path)
}

fn first_existing(root: &Path, relatives: &[&str]) -> Option<PathBuf> {
    relatives
        .iter()
        .find_map(|relative| existing(root, relative))
}

fn model_manifest(root: &Path) -> Option<String> {
    let path = root.join("models/manifest.json");
    let bytes = fs::read(path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let schema = value.get("schema")?.as_str()?;
    Some(schema.to_string())
}

fn zh_manifest(root: &Path) -> Option<String> {
    let path = root.join("build-zh/manifest.json");
    let bytes = fs::read(path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let format = value.get("format").and_then(serde_json::Value::as_i64);
    let entries = value
        .get("counts")
        .and_then(|counts| counts.get("entries"))
        .and_then(serde_json::Value::as_i64);
    match (format, entries) {
        (Some(format), Some(entries)) => Some(format!("format {format}，{entries} 個詞條")),
        (Some(format), None) => Some(format!("format {format}")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!("jc-settings-ui-{tag}-{stamp}"));
            fs::create_dir_all(&path).expect("建立暫存目錄");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn test_probe_reports_missing_models() {
        let dir = TempDir::new("empty");
        let status = DataStatus::probe(&dir.0, &dir.0.join("settings.json"), FileState::Missing);
        assert!(!status.zh_select_ready());
        assert!(status.model_manifest.is_none());
        assert_eq!(status.settings_state, FileState::Missing);
    }

    #[test]
    fn test_probe_finds_zh_select_and_manifest() {
        let dir = TempDir::new("zh");
        fs::create_dir_all(dir.0.join("models/zh-select")).expect("建立模型目錄");
        fs::write(dir.0.join("models/zh-select/model.q8_0.gguf"), b"x").expect("寫入權重");
        fs::write(
            dir.0.join("models/manifest.json"),
            br#"{"schema":"jc.model-set/1","models":{}}"#,
        )
        .expect("寫入 manifest");
        let status = DataStatus::probe(&dir.0, &dir.0.join("settings.json"), FileState::Ok);
        assert!(status.zh_select_ready());
        assert_eq!(status.model_manifest.as_deref(), Some("jc.model-set/1"));
    }

    #[test]
    fn test_zh_manifest_summary() {
        let dir = TempDir::new("build");
        fs::create_dir_all(dir.0.join("build-zh")).expect("建立資料目錄");
        fs::write(
            dir.0.join("build-zh/manifest.json"),
            br#"{"format":1,"counts":{"entries":271170}}"#,
        )
        .expect("寫入 manifest");
        let status = DataStatus::probe(&dir.0, &dir.0.join("settings.json"), FileState::Ok);
        assert_eq!(
            status.zh_manifest.as_deref(),
            Some("format 1，271170 個詞條")
        );
    }
}
