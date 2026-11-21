//! 資料根目錄的唯讀檢查：manifest 版本與模型檔是否存在。
//!
//! 只讀檔案，不改資料。模型路徑一律從 `jc-data` 的 [`CATALOG`] 角色解析；
//! catalog 改目錄或檔名時模型頁跟著改，不再重複一份硬編碼路徑。Jinen
//! tokenizer 的探測順序與 `jc-engine::resolve_ja_neural` 一致：環境變數、
//! 資料 manifest、GGUF 同目錄、隨附候選。
//!
//! 已移除的角色不在此探測：`ja-rerank` 已棄用且預設關閉，`ja-punct` 與舊的
//! `models/ja-tokenizer/public_core_ja.model` 已不再載入、不隨 MSI 散布。
//! 把它們當成「缺少」會誤導使用者，因此 UI 完全不列出。Jinen 的 tokenizer
//! 走 `tokenizer.json`，隨 MSI 散布。

use std::fs;
use std::path::{Path, PathBuf};

use jc_data::manifest::{DataManifest, DataRole, CATALOG, MANIFEST_FILE};
use jc_settings::FileState;

/// 覆寫 Jinen tokenizer 的路徑；與引擎的 `JA_NEURAL_TOKENIZER_ENV` 相同。
const JA_NEURAL_TOKENIZER_ENV: &str = "JC_JINEN_TOKENIZER";

/// Jinen tokenizer 的隨附候選，順序與引擎 `JA_NEURAL_TOKENIZERS` 一致。
const JA_NEURAL_TOKENIZERS: [&str; 3] = [
    "models/external/jinen-v2-small/tokenizer.json",
    "models/external/jinen-v2-xsmall/tokenizer.json",
    "models/quant/jinen-v2-xsmall/tokenizer.json",
];

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
    /// Jinen 的 tokenizer；與 GGUF 分開隨 MSI 散布，可被環境變數或 manifest 覆寫。
    pub jinen_tokenizer: Option<PathBuf>,
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
            jinen_tokenizer: jinen_tokenizer(root),
        }
    }
}

/// 某個相對路徑的檔案是否存在。
fn existing(root: &Path, relative: &str) -> Option<PathBuf> {
    let path = root.join(relative);
    path.is_file().then_some(path)
}

/// 依序找第一個存在的候選路徑。
fn first_existing(root: &Path, relatives: &[&str]) -> Option<PathBuf> {
    relatives
        .iter()
        .find_map(|relative| existing(root, relative))
}

/// 從 catalog 的某個角色挑第一個符合條件的候選檔案。
///
/// catalog 的 `candidates` 順序就是權威順序；UI 不再自己維護一份路徑表。
fn first_catalog_file(
    root: &Path,
    role: DataRole,
    select: impl Fn(&str) -> bool,
) -> Option<PathBuf> {
    CATALOG
        .iter()
        .filter(|spec| spec.role == role)
        .flat_map(|spec| spec.candidates.iter().copied())
        .filter(|candidate| select(candidate))
        .find_map(|candidate| existing(root, candidate))
}

/// 模型權重檔；避開同角色的 `manifest.json` 與 `vocab.json`。
fn is_model_weight(relative: &str) -> bool {
    [".gguf", ".safetensors", ".onnx"]
        .iter()
        .any(|suffix| relative.ends_with(suffix))
}

/// 解析 Jinen tokenizer，順序與引擎一致。
fn jinen_tokenizer(root: &Path) -> Option<PathBuf> {
    if let Some(value) = std::env::var_os(JA_NEURAL_TOKENIZER_ENV) {
        let path = PathBuf::from(value);
        return path.is_file().then_some(path);
    }
    if let Ok(manifest) = DataManifest::load(root.join(MANIFEST_FILE)) {
        let from_manifest = manifest
            .files
            .iter()
            .find(|file| file.role == DataRole::JaTokenizer)
            .map(|file| root.join(&file.path))
            .filter(|path| path.is_file());
        if from_manifest.is_some() {
            return from_manifest;
        }
    }
    if let Some(gguf) = first_catalog_file(root, DataRole::JaNeural, is_model_weight) {
        if let Some(dir) = gguf.parent() {
            let beside = dir.join("tokenizer.json");
            if beside.is_file() {
                return Some(beside);
            }
        }
    }
    first_existing(root, &JA_NEURAL_TOKENIZERS)
}

/// 模型集合的 manifest schema；路徑從 catalog 的 `zh_select` 角色解析。
fn model_manifest(root: &Path) -> Option<String> {
    let path = first_catalog_file(root, DataRole::ZhSelect, |relative| {
        relative.ends_with("manifest.json")
    })?;
    let bytes = fs::read(path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let schema = value.get("schema")?.as_str()?;
    Some(schema.to_string())
}

/// 注音詞典 manifest 的摘要；路徑從 catalog 的 `zh_lexicon` 角色解析。
fn zh_manifest(root: &Path) -> Option<String> {
    let path = first_catalog_file(root, DataRole::ZhLexicon, |relative| {
        relative.ends_with("manifest.json")
    })?;
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
    use jc_data::manifest::{DataFile, SCHEMA};
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

    /// 寫一份只含指定角色路徑的資料 manifest。
    fn write_data_manifest(root: &Path, entries: &[(DataRole, &str)]) {
        let files: Vec<DataFile> = entries
            .iter()
            .map(|(role, path)| DataFile {
                path: (*path).to_string(),
                bytes: 1,
                sha256: "0".repeat(64),
                role: *role,
                license: vec!["L-21".to_string()],
                shipped: true,
                url: None,
            })
            .collect();
        let manifest = DataManifest {
            schema: SCHEMA.to_string(),
            version: "test".to_string(),
            generated_at: "2026-11-20T00:00:00Z".to_string(),
            files,
        };
        fs::write(root.join(MANIFEST_FILE), manifest.to_json_pretty().unwrap())
            .expect("寫入資料 manifest");
    }

    #[test]
    fn test_probe_reports_missing_models() {
        let dir = TempDir::new("empty");
        let status = DataStatus::probe(&dir.0, &dir.0.join("settings.json"), FileState::Missing);
        assert!(status.jinen_tokenizer.is_none());
        assert!(status.model_manifest.is_none());
        assert_eq!(status.settings_state, FileState::Missing);
    }

    #[test]
    fn test_probe_finds_model_manifest() {
        let dir = TempDir::new("zh");
        fs::write(
            dir.0.join("models/manifest.json"),
            br#"{"schema":"jc.model-set/1","models":{}}"#,
        )
        .expect("寫入 manifest");
        let status = DataStatus::probe(&dir.0, &dir.0.join("settings.json"), FileState::Ok);
        assert_eq!(status.model_manifest.as_deref(), Some("jc.model-set/1"));
    }

    #[test]
    fn test_probe_finds_shipped_jinen_tokenizer() {
        let dir = TempDir::new("jinen");
        fs::create_dir_all(dir.0.join("models/external/jinen-v2-small"))
            .expect("建立 Jinen 模型目錄");
        fs::create_dir_all(dir.0.join("models/quant/jinen-v2-xsmall"))
            .expect("建立 Jinen tokenizer 目錄");
        fs::write(
            dir.0
                .join("models/external/jinen-v2-small/jinen-v2-small-Q5_K_M.gguf"),
            b"gguf",
        )
        .expect("寫入 Jinen GGUF");
        fs::write(
            dir.0.join("models/quant/jinen-v2-xsmall/tokenizer.json"),
            b"{}",
        )
        .expect("寫入 Jinen tokenizer");

        // 已移除角色的舊檔案不影響判定，也不該被當成 Jinen tokenizer。
        fs::create_dir_all(dir.0.join("models/reranker-extra")).expect("建立舊 reranker 目錄");
        fs::create_dir_all(dir.0.join("models/ja-tokenizer")).expect("建立舊分詞器目錄");
        fs::write(dir.0.join("models/reranker-extra/model.int8.onnx"), b"x")
            .expect("寫入舊 reranker");
        fs::write(dir.0.join("models/ja-tokenizer/public_core_ja.model"), b"x")
            .expect("寫入舊分詞器");

        let tokenizer_path = dir.0.join("models/quant/jinen-v2-xsmall/tokenizer.json");
        let status = DataStatus::probe(&dir.0, &dir.0.join("settings.json"), FileState::Ok);
        assert_eq!(
            status.jinen_tokenizer.as_deref(),
            Some(tokenizer_path.as_path())
        );
    }

    #[test]
    fn test_jinen_tokenizer_prefers_manifest_path() {
        let dir = TempDir::new("manifest-tokenizer");
        fs::create_dir_all(dir.0.join("custom/jinen")).expect("建立自訂目錄");
        let custom = dir.0.join("custom/jinen/tokenizer.json");
        fs::write(&custom, b"{}").expect("寫入 tokenizer");
        fs::create_dir_all(dir.0.join("models/quant/jinen-v2-xsmall")).expect("建立隨附目錄");
        fs::write(
            dir.0.join("models/quant/jinen-v2-xsmall/tokenizer.json"),
            b"{}",
        )
        .expect("寫入隨附 tokenizer");
        write_data_manifest(
            &dir.0,
            &[(DataRole::JaTokenizer, "custom/jinen/tokenizer.json")],
        );

        let status = DataStatus::probe(&dir.0, &dir.0.join("settings.json"), FileState::Ok);
        assert_eq!(
            status.jinen_tokenizer.as_deref(),
            Some(custom.as_path()),
            "manifest 指定的路徑優先於隨附候選"
        );
    }

    #[test]
    fn test_jinen_tokenizer_falls_back_to_gguf_directory() {
        let dir = TempDir::new("beside-tokenizer");
        let model_dir = dir.0.join("models/external/jinen-v2-small");
        fs::create_dir_all(&model_dir).expect("建立 Jinen 模型目錄");
        fs::write(model_dir.join("jinen-v2-small-Q5_K_M.gguf"), b"gguf").expect("寫入 Jinen GGUF");
        let beside = model_dir.join("tokenizer.json");
        fs::write(&beside, b"{}").expect("寫入同目錄 tokenizer");

        let status = DataStatus::probe(&dir.0, &dir.0.join("settings.json"), FileState::Ok);
        assert_eq!(status.jinen_tokenizer.as_deref(), Some(beside.as_path()));
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
