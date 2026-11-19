//! 模型集合 manifest 與可用性探測。
//!
//! 模型目錄可含一份 `manifest.json`：
//!
//! ```json
//! {
//!   "schema": "jc.model-set/1",
//!   "models": {
//!     "ja-rerank": { "dir": "reranker-extra" },
//!     "ja-punct":  { "dir": "punctuation-public-core" }
//!   }
//! }
//! ```
//!
//! 每個角色目錄內預設放 `model.int8.onnx` 與 `vocab.json`，可用 `model`、
//! `vocab` 欄位覆寫。manifest 或模型檔缺席都不算錯誤，只是該角色
//! [`Availability::Unavailable`]。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{ModelError, Result};
use crate::Role;

/// 集合 manifest 的 schema 名稱。
pub const SET_SCHEMA: &str = "jc.model-set/1";
/// 預設模型檔名。
pub const DEFAULT_MODEL_FILE: &str = "model.int8.onnx";
/// 預設 tokenizer 檔名。
pub const DEFAULT_VOCAB_FILE: &str = "vocab.json";
/// 重排模型的預設序列長度。
pub const DEFAULT_SEQ_LEN: usize = 124;
/// 重排模型候選字起始位置。
pub const DEFAULT_CANDIDATE_SLOT: usize = 62;
/// 重排模型單側 context token 上限。
pub const DEFAULT_CONTEXT_TOKENS: usize = 60;
/// 預設 BOS id。
pub const DEFAULT_BOS_ID: i64 = 2;
/// 預設 EOS id。
pub const DEFAULT_EOS_ID: i64 = 3;
/// 預設 PAD id。
pub const DEFAULT_PAD_ID: i64 = 0;
/// 推論預設預算，毫秒。
pub const DEFAULT_TIMEOUT_MS: u64 = 100;

/// manifest 內單一模型的設定。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ModelSpec {
    /// 角色目錄，相對於集合根目錄；絕對路徑照用。
    #[serde(default)]
    pub dir: Option<String>,
    /// ONNX 檔名，相對於角色目錄。
    #[serde(default)]
    pub model: Option<String>,
    /// vocab.json 檔名，相對於角色目錄。
    #[serde(default)]
    pub vocab: Option<String>,
    /// 重排模型序列長度。
    #[serde(default)]
    pub seq_len: Option<usize>,
    /// 重排模型候選字起始 slot。
    #[serde(default)]
    pub candidate_slot: Option<usize>,
    /// 重排模型單側 context token 上限。
    #[serde(default)]
    pub context_tokens_per_side: Option<usize>,
    /// BOS token id。
    #[serde(default)]
    pub bos_id: Option<i64>,
    /// EOS token id。
    #[serde(default)]
    pub eos_id: Option<i64>,
    /// PAD token id。
    #[serde(default)]
    pub padding_id: Option<i64>,
    /// ONNX Runtime intra-op 執行緒數。
    #[serde(default)]
    pub threads: Option<usize>,
    /// 單次推論預算，毫秒。
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

impl ModelSpec {
    /// 以預設檔名建立指向 `dir` 的設定。
    pub fn for_dir(dir: impl Into<String>) -> Self {
        Self {
            dir: Some(dir.into()),
            ..Self::default()
        }
    }

    /// 模型檔名。
    pub fn model_file(&self) -> &str {
        self.model.as_deref().unwrap_or(DEFAULT_MODEL_FILE)
    }

    /// tokenizer 檔名。
    pub fn vocab_file(&self) -> &str {
        self.vocab.as_deref().unwrap_or(DEFAULT_VOCAB_FILE)
    }

    /// 序列長度。
    pub fn seq_len(&self) -> usize {
        self.seq_len.unwrap_or(DEFAULT_SEQ_LEN)
    }

    /// 候選字起始 slot。
    pub fn candidate_slot(&self) -> usize {
        self.candidate_slot.unwrap_or(DEFAULT_CANDIDATE_SLOT)
    }

    /// 單側 context token 上限。
    pub fn context_tokens(&self) -> usize {
        self.context_tokens_per_side
            .unwrap_or(DEFAULT_CONTEXT_TOKENS)
    }

    /// BOS id。
    pub fn bos_id(&self) -> i64 {
        self.bos_id.unwrap_or(DEFAULT_BOS_ID)
    }

    /// EOS id。
    pub fn eos_id(&self) -> i64 {
        self.eos_id.unwrap_or(DEFAULT_EOS_ID)
    }

    /// PAD id。
    pub fn padding_id(&self) -> i64 {
        self.padding_id.unwrap_or(DEFAULT_PAD_ID)
    }

    /// 執行緒數；0 表示交給 ONNX Runtime 預設值。
    pub fn threads(&self) -> usize {
        self.threads.unwrap_or(1).max(1)
    }

    /// 推論預算。
    pub fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS))
    }
}

/// manifest.json 的頂層結構。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SetManifest {
    /// schema 名稱，必須是 [`SET_SCHEMA`]。
    #[serde(default)]
    pub schema: Option<String>,
    /// 角色名稱到模型設定的對應。
    #[serde(default)]
    pub models: BTreeMap<String, ModelSpec>,
}

/// 模型可用狀態；缺模型時引擎退回統計路徑，不應失敗。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// 模型檔存在，可以載入。
    Loaded {
        /// 角色。
        role: Role,
        /// ONNX 檔大小。
        bytes: u64,
    },
    /// 沒有可用的模型。
    Unavailable {
        /// 角色。
        role: Role,
        /// 原因（不含輸入文字）。
        reason: String,
    },
}

impl Availability {
    /// 是否可用。
    pub fn is_loaded(&self) -> bool {
        matches!(self, Self::Loaded { .. })
    }

    /// 角色。
    pub fn role(&self) -> Role {
        match self {
            Self::Loaded { role, .. } | Self::Unavailable { role, .. } => *role,
        }
    }
}

#[derive(Debug, Clone)]
struct Resolved {
    dir: PathBuf,
    model: PathBuf,
    vocab: PathBuf,
}

/// 一組模型目錄的解析結果。
#[derive(Debug, Clone)]
pub struct ModelSet {
    root: PathBuf,
    specs: BTreeMap<Role, ModelSpec>,
    resolved: BTreeMap<Role, Resolved>,
    availability: BTreeMap<Role, Availability>,
}

impl ModelSet {
    /// 讀取模型集合目錄。
    ///
    /// 目錄或 `manifest.json` 缺席時回傳全部 `Unavailable` 的集合，
    /// 不視為錯誤；manifest 損壞才回傳 [`ModelError::Manifest`]。
    pub fn load(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let mut specs = BTreeMap::new();
        let path = root.join("manifest.json");
        match fs::read(&path) {
            Ok(raw) => {
                let m: SetManifest = serde_json::from_slice(&raw)
                    .map_err(|e| ModelError::manifest(format!("{}：{e}", path.display())))?;
                if let Some(schema) = &m.schema {
                    if schema != SET_SCHEMA {
                        return Err(ModelError::manifest(format!(
                            "schema 應為 {SET_SCHEMA}，實際為 {schema}"
                        )));
                    }
                }
                for (name, spec) in m.models {
                    let role: Role = name.parse().map_err(|e: String| ModelError::manifest(e))?;
                    specs.insert(role, spec);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }

        let mut resolved = BTreeMap::new();
        let mut availability = BTreeMap::new();
        for role in Role::all() {
            let Some(spec) = specs.get(&role).cloned() else {
                availability.insert(
                    role,
                    Availability::Unavailable {
                        role,
                        reason: "manifest 未列出模型".into(),
                    },
                );
                continue;
            };
            match resolve(&root, role, &spec) {
                Ok(r) => {
                    let bytes = fs::metadata(&r.model).map(|m| m.len()).unwrap_or(0);
                    availability.insert(role, Availability::Loaded { role, bytes });
                    resolved.insert(role, r);
                }
                Err(reason) => {
                    availability.insert(role, Availability::Unavailable { role, reason });
                }
            }
        }
        Ok(Self {
            root,
            specs,
            resolved,
            availability,
        })
    }

    /// 集合根目錄。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 指定角色的可用狀態。
    pub fn availability(&self, role: Role) -> &Availability {
        &self.availability[&role]
    }

    /// 所有角色的可用狀態，依 [`Role::all`] 的順序。
    pub fn availabilities(&self) -> impl Iterator<Item = &Availability> {
        Role::all()
            .into_iter()
            .map(|role| &self.availability[&role])
    }

    /// 指定角色的 manifest 設定。
    pub fn spec(&self, role: Role) -> Option<&ModelSpec> {
        self.specs.get(&role)
    }

    /// 已解析的角色：目錄、模型檔與 tokenizer。
    pub fn model_dir(&self, role: Role) -> Option<&Path> {
        self.resolved.get(&role).map(|r| r.dir.as_path())
    }

    /// 模型檔路徑。
    pub fn model_path(&self, role: Role) -> Option<&Path> {
        self.resolved.get(&role).map(|r| r.model.as_path())
    }

    /// tokenizer 路徑。
    pub fn vocab_path(&self, role: Role) -> Option<&Path> {
        self.resolved.get(&role).map(|r| r.vocab.as_path())
    }
}

fn resolve(root: &Path, role: Role, spec: &ModelSpec) -> std::result::Result<Resolved, String> {
    let dir = match spec.dir.as_deref() {
        Some(d) => absolute(root, d),
        None => root.join(role.as_str()),
    };
    if !dir.is_dir() {
        return Err(format!("模型目錄不存在：{}", dir.display()));
    }
    let model = absolute(&dir, spec.model_file());
    if !model.is_file() {
        return Err(format!("模型檔不存在：{}", model.display()));
    }
    let vocab = absolute(&dir, spec.vocab_file());
    if !vocab.is_file() {
        return Err(format!("tokenizer 不存在：{}", vocab.display()));
    }
    Ok(Resolved { dir, model, vocab })
}

fn absolute(base: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn tempdir(tag: &str) -> PathBuf {
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("jc-model-{tag}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_manifest_is_unavailable_not_error() {
        let dir = tempdir("empty");
        let set = ModelSet::load(&dir).unwrap();
        for role in Role::all() {
            let a = set.availability(role);
            assert!(!a.is_loaded());
            match a {
                Availability::Unavailable { reason, .. } => {
                    assert_eq!(reason, "manifest 未列出模型");
                }
                Availability::Loaded { .. } => unreachable!(),
            }
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_model_files_are_unavailable() {
        let dir = tempdir("missing");
        fs::write(
            dir.join("manifest.json"),
            r#"{"schema":"jc.model-set/1","models":{"ja-rerank":{"dir":"rerank"}}}"#,
        )
        .unwrap();
        fs::create_dir_all(dir.join("rerank")).unwrap();
        let set = ModelSet::load(&dir).unwrap();
        match set.availability(Role::JaRerank) {
            Availability::Unavailable { reason, .. } => {
                assert!(reason.starts_with("模型檔不存在："), "{reason}");
                assert!(reason.ends_with("model.int8.onnx"), "{reason}");
            }
            other => panic!("未預期的狀態：{other:?}"),
        }
        assert!(!set.availability(Role::JaPunct).is_loaded());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn manifest_resolves_files_and_reports_size() {
        let dir = tempdir("loaded");
        fs::create_dir_all(dir.join("rerank")).unwrap();
        fs::create_dir_all(dir.join("punct")).unwrap();
        fs::write(dir.join("rerank/model.int8.onnx"), vec![0u8; 1234]).unwrap();
        fs::write(dir.join("rerank/vocab.json"), b"{}").unwrap();
        fs::write(dir.join("punct/model.int8.onnx"), vec![0u8; 7]).unwrap();
        fs::write(dir.join("punct/ja.bin"), b"{}").unwrap();
        fs::write(
            dir.join("manifest.json"),
            r#"{
              "schema": "jc.model-set/1",
              "models": {
                "ja-rerank": {"dir": "rerank"},
                "ja-punct": {"dir": "punct", "vocab": "ja.bin",
                  "seq_len": 96, "candidate_slot": 48, "timeout_ms": 50}
              }
            }"#,
        )
        .unwrap();

        let set = ModelSet::load(&dir).unwrap();
        assert_eq!(
            set.availability(Role::JaRerank),
            &Availability::Loaded {
                role: Role::JaRerank,
                bytes: 1234
            }
        );
        assert_eq!(
            set.availability(Role::JaPunct),
            &Availability::Loaded {
                role: Role::JaPunct,
                bytes: 7
            }
        );
        assert_eq!(
            set.model_path(Role::JaPunct).unwrap(),
            dir.join("punct/model.int8.onnx")
        );
        assert_eq!(
            set.vocab_path(Role::JaPunct).unwrap(),
            dir.join("punct/ja.bin")
        );
        let spec = set.spec(Role::JaPunct).unwrap();
        assert_eq!(spec.seq_len(), 96);
        assert_eq!(spec.candidate_slot(), 48);
        assert_eq!(spec.timeout(), std::time::Duration::from_millis(50));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn malformed_manifest_is_an_error() {
        let dir = tempdir("bad");
        fs::write(dir.join("manifest.json"), b"not json").unwrap();
        assert!(matches!(ModelSet::load(&dir), Err(ModelError::Manifest(_))));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_role_is_an_error() {
        let dir = tempdir("role");
        fs::write(
            dir.join("manifest.json"),
            r#"{"schema":"jc.model-set/1","models":{"klingon":{"dir":"x"}}}"#,
        )
        .unwrap();
        match ModelSet::load(&dir) {
            Err(ModelError::Manifest(msg)) => assert!(msg.contains("klingon"), "{msg}"),
            other => panic!("未預期結果：{other:?}"),
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn spec_defaults_match_deployment_layout() {
        let spec = ModelSpec::default();
        assert_eq!(spec.seq_len(), 124);
        assert_eq!(spec.candidate_slot(), 62);
        assert_eq!(spec.context_tokens(), 60);
        assert_eq!(spec.bos_id(), 2);
        assert_eq!(spec.eos_id(), 3);
        assert_eq!(spec.padding_id(), 0);
        assert_eq!(spec.timeout(), std::time::Duration::from_millis(100));
    }
}
