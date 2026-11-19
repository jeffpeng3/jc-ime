//! 日文模型包的 manifest、驗證與可用性判斷。
//!
//! 對照前身 `internal/modelsets/manifest.go`。產品只有一個固定的模型包，包含
//! CC BY-SA reranker 與 MIT punctuation，綁定一個日文詞典包的 manifest
//! SHA-256，並記錄 Hugging Face revision、逐檔大小與 SHA-256。使用者不需要也
//! 不能選擇模型組。
//!
//! 這個 module 只做資料與本機檔案；下載與 pin 不在本 crate（見
//! `docs/port/settings-core.md`）。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// 產品側 `model_package` 區段的格式版本。
pub const SCHEMA: &str = "0c.model-package/1";

/// 模型種類。
pub const KIND_RERANKER: &str = "reranker";
/// 模型種類。
pub const KIND_PUNCTUATION: &str = "punctuation";

/// artifact variant。
pub const VARIANT_PUBLIC_CORE: &str = "public-core";
/// artifact variant。
pub const VARIANT_CC_BY_SA: &str = "cc-by-sa";

/// 量化格式。
pub const FORMAT_INT8: &str = "int8";
/// 量化格式。
pub const FORMAT_FP32: &str = "fp32";

/// placeholder 模型未訓練的品質標記。
pub const QUALITY_UNTRAINED: &str = "untrained";

/// 模型包驗證錯誤；呼叫端用 variant 判斷分類，訊息帶可讀細節。
#[derive(Debug, Error)]
pub enum ModelSetError {
    /// manifest 結構或 schema 不符。
    #[error("model package schema mismatch: {0}")]
    Schema(String),
    /// reranker／punctuation 的 variant 或授權混用。
    #[error("model package artifact variant is invalid: {0}")]
    VariantMixed(String),
    /// 模型包不完整。
    #[error("model package is incomplete: {0}")]
    IncompleteSet(String),
    /// 與已安裝的字典包不符。
    #[error("model package dictionary mismatch: {0}")]
    DictionaryMismatch(String),
    /// 已安裝的字典包未知。
    #[error("installed dictionary pack is unknown: {0}")]
    DictionaryUnknown(String),
    /// 未允許載入 placeholder 模型。
    #[error("placeholder model package is not allowed: {0}")]
    Placeholder(String),
    /// placeholder 標記不完整。
    #[error("placeholder model package is not marked: {0}")]
    PlaceholderUnmarked(String),
    /// 張量契約不符。
    #[error("model package tensor contract mismatch: {0}")]
    TensorContract(String),
    /// 實際檔案與清單不符。
    #[error("model package file mismatch: {0}")]
    FileMismatch(String),
    /// 授權與 artifact 不符。
    #[error("model package license does not match artifact: {0}")]
    LicenseMixed(String),
    /// `active.json` 格式不符。
    #[error("model package selection is invalid: {0}")]
    Selection(String),
    /// Hugging Face revision 尚未 pin。
    #[error("model package Hugging Face revision is not pinned: {0}")]
    Unpinned(String),
    /// Hugging Face 來源不合法。
    #[error("model package Hugging Face source is invalid: {0}")]
    HfSource(String),
    /// 檔案雜湊不符。
    #[error("model file checksum mismatch: {0}")]
    Checksum(String),
    /// 離線時檔案不在快取內。
    #[error("model is not available offline: {0}")]
    Offline(String),
    /// 檔案系統錯誤。
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// 驗證結果的別名。
pub type Result<T> = std::result::Result<T, ModelSetError>;

/// `models/pairing.lock.json` 中本 crate 讀取的部分。
///
/// `model_package` 以外的鍵（既有注音字典配對）由其他 crate 解讀。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Lock {
    /// 外層配對清單的 schema，既有格式是 1。
    #[serde(default)]
    pub schema: i32,
    /// 唯一的日文模型包。
    #[serde(default)]
    pub model_package: Option<Package>,
}

/// 唯一出貨的日文模型包。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Package {
    /// 區段 schema。
    #[serde(default)]
    pub schema: String,
    /// 更新日期。
    #[serde(default)]
    pub updated: String,
    /// 模型包版本。
    #[serde(default)]
    pub version: String,
    /// 綁定的字典包。
    #[serde(default)]
    pub dictionary: Dictionary,
    /// 推論設定。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment: Option<Deployment>,
    /// 模型包授權。
    #[serde(default)]
    pub license: License,
    /// 姓名標示與來源帳本參考。
    #[serde(default)]
    pub attribution: Vec<String>,
    /// 是否為未訓練的 placeholder。
    #[serde(default)]
    pub placeholder: bool,
    /// 品質標記。
    #[serde(default)]
    pub quality: String,
    /// 是否可進入正式發行。
    #[serde(default)]
    pub production_eligible: bool,
    /// 模型檔集合。
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
}

/// 模型包的推論設定。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Deployment {
    /// 序列長度。
    #[serde(default)]
    pub sequence_length: i64,
    /// 候選左右兩側各自最多送入的 token 數。
    #[serde(default)]
    pub context_tokens_per_side: i64,
    /// 只送進模型的統計候選數。
    #[serde(default)]
    pub gating_top_k: i64,
    /// 量化格式。
    #[serde(default)]
    pub format: String,
    /// 候選篩選策略。
    #[serde(default)]
    pub gating_policy: String,
}

/// 模型包綁定的字典包。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dictionary {
    /// 字典包代號。
    #[serde(default)]
    pub pack_id: String,
    /// 語言，固定 `ja`。
    #[serde(default)]
    pub language: String,
    /// 字典包格式版本。
    #[serde(default)]
    pub format: i32,
    /// 字典包版本。
    #[serde(default)]
    pub version: String,
    /// 安裝後 `manifest.json` 的 SHA-256。
    #[serde(default)]
    pub manifest_sha256: String,
    /// 來源說明。
    #[serde(default)]
    pub source: String,
}

/// 授權與散布邊界。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct License {
    /// 授權代號，例如 `MIT`、`CC-BY-SA-4.0`。
    #[serde(default)]
    pub id: String,
    /// 授權範圍，對應 artifact variant。
    #[serde(default)]
    pub scope: String,
    /// 散布條件。
    #[serde(default)]
    pub distribution: String,
    /// repo 內的授權全文參考。
    #[serde(default)]
    pub notice: String,
}

/// 模型的來源與可重現資訊。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Source {
    /// 訓練資料說明。
    #[serde(default)]
    pub training_data: String,
    /// 使用的資料集。
    #[serde(default)]
    pub datasets: Vec<String>,
    /// 上游 checkpoint。
    #[serde(default)]
    pub checkpoint: String,
    /// 產生方式。
    #[serde(default)]
    pub generator: String,
    /// 可重現指紋。
    #[serde(default)]
    pub fingerprint: String,
}

/// 模型包內的一個模型檔集合。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Artifact {
    /// 這個 artifact 的 manifest 路徑，相對於模型根目錄。
    #[serde(default)]
    pub manifest: String,
    /// artifact 代號。
    #[serde(default)]
    pub artifact_id: String,
    /// 模型種類。
    #[serde(default)]
    pub kind: String,
    /// artifact variant。
    #[serde(default)]
    pub variant: String,
    /// artifact 版本。
    #[serde(default)]
    pub version: String,
    /// 參數量。
    #[serde(default)]
    pub parameters: i64,
    /// 是否為未訓練的 placeholder。
    #[serde(default)]
    pub placeholder: bool,
    /// 品質標記。
    #[serde(default)]
    pub quality: String,
    /// 是否可進入正式發行。
    #[serde(default)]
    pub production_eligible: bool,
    /// 綁定的字典 manifest SHA-256。
    #[serde(default)]
    pub dictionary_manifest_sha256: String,
    /// artifact 授權。
    #[serde(default)]
    pub license: License,
    /// 姓名標示。
    #[serde(default)]
    pub attribution: Vec<String>,
    /// 來源資訊。
    #[serde(default)]
    pub source: Source,
    /// 以 `int8`／`fp32` 為鍵的本機檔案。
    #[serde(default)]
    pub files: BTreeMap<String, ArtifactFile>,
    /// 與權重放在一起的非權重檔。
    #[serde(default)]
    pub assets: Vec<ArtifactAsset>,
    /// Hugging Face 來源。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hf: Option<HfSource>,
    /// 張量契約。
    #[serde(default)]
    pub io: TensorIo,
}

impl Artifact {
    /// artifact 目錄；目錄必須含它自己的 `manifest.json`。
    pub fn package_dir(&self) -> String {
        match Path::new(&self.manifest).parent() {
            Some(dir) if !self.manifest.is_empty() => slash_dir(dir),
            _ => String::new(),
        }
    }

    /// 安裝根目錄下的絕對 artifact 目錄。
    pub fn package_root(&self, root: &Path) -> PathBuf {
        let dir = self.package_dir();
        if dir.is_empty() {
            return root.to_path_buf();
        }
        root.join(Path::new(&dir))
    }

    /// 具名非權重檔的安裝路徑。
    pub fn asset_path(&self, root: &Path, name: &str) -> Option<PathBuf> {
        self.asset(name).map(|asset| root.join(&asset.path))
    }

    /// 依名稱找非權重檔。
    pub fn asset(&self, name: &str) -> Option<&ArtifactAsset> {
        self.assets.iter().find(|asset| asset.name == name)
    }
}

/// 與權重同在一個目錄的非權重檔。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactAsset {
    /// 穩定的邏輯名稱，例如 `vocab.json`。
    #[serde(default)]
    pub name: String,
    /// 相對於模型根目錄的路徑。
    #[serde(default)]
    pub path: String,
    /// 檔案大小。
    #[serde(default)]
    pub bytes: i64,
    /// 檔案 SHA-256。
    #[serde(default)]
    pub sha256: String,
}

/// 一個實際的 ONNX 檔。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactFile {
    /// 相對於模型根目錄的路徑。
    #[serde(default)]
    pub path: String,
    /// 檔案大小；未 pin 時可以是 0。
    #[serde(default)]
    pub bytes: i64,
    /// 檔案 SHA-256；未 pin 時可以是空字串。
    #[serde(default)]
    pub sha256: String,
    /// 量化方式。
    #[serde(default)]
    pub quantization: String,
}

/// 模型檔在 Hugging Face 的來源。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HfSource {
    /// repo 名稱。
    #[serde(default)]
    pub repository: String,
    /// commit 雜湊。
    #[serde(default)]
    pub revision: String,
    /// repo 授權。
    #[serde(default)]
    pub license: String,
    /// 以格式為鍵的權重檔。
    #[serde(default)]
    pub files: BTreeMap<String, HfFile>,
    /// 以 asset 名稱為鍵的非權重檔。
    #[serde(default)]
    pub assets: BTreeMap<String, HfFile>,
}

impl HfSource {
    /// 是否已有合法 revision 與逐檔大小／雜湊。
    pub fn pinned(&self) -> bool {
        if self.repository.is_empty() || !valid_revision(&self.revision) || self.files.is_empty() {
            return false;
        }
        self.files.values().all(pinned_file) && self.assets.values().all(pinned_file)
    }
}

/// HF repo 內一個檔案的名字、大小與 SHA-256。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HfFile {
    /// 檔名。
    #[serde(default)]
    pub name: String,
    /// 檔案大小。
    #[serde(default)]
    pub bytes: i64,
    /// 檔案 SHA-256。
    #[serde(default)]
    pub sha256: String,
}

fn pinned_file(file: &HfFile) -> bool {
    !file.name.is_empty() && file.bytes > 0 && valid_hex(&file.sha256)
}

/// 模型檔的輸入輸出契約。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TensorIo {
    /// 輸入張量。
    #[serde(default)]
    pub inputs: Vec<Tensor>,
    /// 輸出張量。
    #[serde(default)]
    pub outputs: Vec<Tensor>,
}

/// 一個輸入或輸出張量。`shape` 的元素是字串（batch、seq）或固定整數。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tensor {
    /// 張量名稱。
    #[serde(default)]
    pub name: String,
    /// 說明。
    #[serde(default)]
    pub description: String,
    /// 資料型別。
    #[serde(default)]
    pub dtype: String,
    /// 形狀。
    #[serde(default)]
    pub shape: Vec<Value>,
}

/// 驗證範圍。
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// 允許載入 placeholder 模型。
    pub allow_placeholders: bool,
    /// 正式發行前的嚴格驗證。
    pub release: bool,
    /// 實際安裝的字典包 manifest SHA-256。
    pub installed_dictionary_sha256: String,
    /// 實際安裝的字典包代號；空字串表示不檢查。
    pub installed_dictionary_pack_id: String,
    /// 模型根目錄。
    pub artifact_root: PathBuf,
    /// 量化格式；空字串表示 `int8`。
    pub format: String,
    /// 實際核對檔案大小與 SHA-256。
    pub verify_files: bool,
}

/// 驗證通過（或安裝）後的模型檔摘要。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactStatus {
    /// artifact 代號。
    pub artifact_id: String,
    /// 模型種類。
    pub kind: String,
    /// artifact variant。
    pub variant: String,
    /// 量化格式。
    pub format: String,
    /// 權重檔路徑。
    pub path: String,
    /// 檔案大小。
    pub bytes: i64,
    /// 檔案 SHA-256。
    pub sha256: String,
    /// 是否為 placeholder。
    pub placeholder: bool,
    /// 品質標記。
    pub quality: String,
}

/// 模型包的驗證結果。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationResult {
    /// 模型包版本。
    pub version: String,
    /// 是否為 placeholder。
    pub placeholder: bool,
    /// 品質標記。
    pub quality: String,
    /// 綁定的字典包。
    pub dictionary: Dictionary,
    /// 字典包是否相符。
    pub dictionary_matches: bool,
    /// 實際安裝的字典 manifest SHA-256。
    pub installed_dictionary_sha256: String,
    /// 各模型檔摘要。
    pub artifacts: Vec<ArtifactStatus>,
}

impl Dictionary {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.pack_id.is_empty()
            || self.language != "ja"
            || self.format <= 0
            || self.version.is_empty()
            || !valid_hex(&self.manifest_sha256)
        {
            return Err(ModelSetError::Schema(format!("dictionary={self:?}")));
        }
        Ok(())
    }
}

/// 解析 `pairing.lock.json` 的 `model_package` 區段，並做不需要檔案的結構驗證。
pub fn parse_lock(bytes: &[u8]) -> Result<Lock> {
    let lock: Lock =
        serde_json::from_slice(bytes).map_err(|err| ModelSetError::Schema(err.to_string()))?;
    let package = lock
        .model_package
        .as_ref()
        .ok_or_else(|| ModelSetError::Schema("缺少 model_package".to_string()))?;
    if package.schema != SCHEMA {
        return Err(ModelSetError::Schema(format!(
            "model_package.schema={:?}",
            package.schema
        )));
    }
    if let Some(deployment) = package.deployment.as_ref() {
        if deployment.sequence_length <= 0
            || deployment.context_tokens_per_side <= 0
            || deployment.gating_top_k <= 0
        {
            return Err(ModelSetError::Schema(
                "deployment 需要正數的 sequence_length、context_tokens_per_side 與 gating_top_k"
                    .to_string(),
            ));
        }
        if !deployment.format.is_empty()
            && deployment.format != FORMAT_INT8
            && deployment.format != FORMAT_FP32
        {
            return Err(ModelSetError::Schema(format!(
                "deployment.format={:?}",
                deployment.format
            )));
        }
    }
    package.validate_structure()?;
    Ok(lock)
}

/// 讀取並解析模型包清單。
pub fn load_lock(path: &Path) -> Result<Lock> {
    let bytes = fs::read(path)?;
    parse_lock(&bytes)
}

impl Package {
    /// 只驗證 manifest 本身：完整、授權、variant 與張量契約。
    ///
    /// 不讀取已安裝字典包或 ONNX 檔案。未 pin 的 HF revision 與空的大小／雜湊
    /// 在這裡只做形狀檢查。
    pub fn validate_structure(&self) -> Result<()> {
        self.check_structure(false)
    }

    /// 正式發行前的嚴格驗證：HF revision 必須已 pin、每個檔案的大小與 SHA-256
    /// 必須齊全，placeholder 與 `production_eligible=false` 一律拒絕。
    pub fn validate_release(&self) -> Result<()> {
        self.check_structure(true)
    }

    /// 列出尚未 pin 的 HF 來源，development 模式用它產生警告。
    pub fn unpinned(&self) -> Vec<String> {
        let mut out = Vec::new();
        for artifact in &self.artifacts {
            let Some(hf) = artifact.hf.as_ref() else {
                continue;
            };
            if hf.revision.is_empty() {
                out.push(format!(
                    "{}（{}）缺少 revision",
                    artifact.artifact_id, hf.repository
                ));
                continue;
            }
            for (format, file) in &hf.files {
                if file.bytes == 0 || file.sha256.is_empty() {
                    out.push(format!(
                        "{}/{format} 缺少大小或 SHA-256",
                        artifact.artifact_id
                    ));
                }
            }
        }
        out
    }

    fn check_structure(&self, release: bool) -> Result<()> {
        if self.version.is_empty() {
            return Err(ModelSetError::Schema("缺少 version".to_string()));
        }
        self.dictionary.validate()?;
        check_package_license(&self.license)?;
        if self.attribution.is_empty() {
            return Err(ModelSetError::Schema("缺少 attribution 參考".to_string()));
        }
        if self.artifacts.is_empty() {
            return Err(ModelSetError::IncompleteSet("沒有模型".to_string()));
        }

        let mut kinds = BTreeMap::<String, usize>::new();
        let mut ids = BTreeSet::new();
        let placeholder = self.artifacts[0].placeholder;
        for artifact in &self.artifacts {
            if artifact.artifact_id.is_empty() || !ids.insert(artifact.artifact_id.clone()) {
                return Err(ModelSetError::Schema(format!(
                    "模型 ID 空白或重複：{:?}",
                    artifact.artifact_id
                )));
            }
            match artifact.kind.as_str() {
                KIND_RERANKER | KIND_PUNCTUATION => {}
                _ => {
                    return Err(ModelSetError::Schema(format!(
                        "{} 的 kind={:?}",
                        artifact.artifact_id, artifact.kind
                    )));
                }
            }
            match artifact.variant.as_str() {
                VARIANT_PUBLIC_CORE | VARIANT_CC_BY_SA => {}
                _ => {
                    return Err(ModelSetError::Schema(format!(
                        "{} 的 variant={:?}",
                        artifact.artifact_id, artifact.variant
                    )));
                }
            }
            if artifact.kind == KIND_RERANKER && artifact.variant != VARIANT_CC_BY_SA {
                return Err(ModelSetError::VariantMixed(format!(
                    "reranker 必須是 cc-by-sa，得到 {}",
                    artifact.variant
                )));
            }
            if artifact.kind == KIND_PUNCTUATION && artifact.variant != VARIANT_PUBLIC_CORE {
                return Err(ModelSetError::VariantMixed(format!(
                    "punctuation 必須是 public-core，得到 {}",
                    artifact.variant
                )));
            }
            *kinds.entry(artifact.kind.clone()).or_default() += 1;

            if artifact.version.is_empty() {
                return Err(ModelSetError::Schema(format!(
                    "{} 缺少 version",
                    artifact.artifact_id
                )));
            }
            check_license(&artifact.variant, &artifact.license)
                .map_err(|err| prefix(&artifact.artifact_id, err))?;
            if artifact.attribution.is_empty() {
                return Err(ModelSetError::Schema(format!(
                    "{} 缺少 attribution 參考",
                    artifact.artifact_id
                )));
            }
            if artifact.dictionary_manifest_sha256 != self.dictionary.manifest_sha256 {
                return Err(ModelSetError::DictionaryMismatch(format!(
                    "{} 記的字典 {} 與模型包 {} 不符",
                    artifact.artifact_id,
                    artifact.dictionary_manifest_sha256,
                    self.dictionary.manifest_sha256
                )));
            }
            if artifact.placeholder != placeholder {
                return Err(ModelSetError::PlaceholderUnmarked(format!(
                    "{} 的 placeholder={} 與模型包 {} 不一致",
                    artifact.artifact_id, artifact.placeholder, placeholder
                )));
            }
            if artifact.placeholder
                && (artifact.quality != QUALITY_UNTRAINED || artifact.production_eligible)
            {
                return Err(ModelSetError::PlaceholderUnmarked(format!(
                    "{} 是 placeholder 但 quality={:?} production_eligible={}",
                    artifact.artifact_id, artifact.quality, artifact.production_eligible
                )));
            }
            validate_hf(artifact, release).map_err(|err| prefix(&artifact.artifact_id, err))?;
            validate_files(artifact, release).map_err(|err| prefix(&artifact.artifact_id, err))?;
            validate_assets(artifact, release).map_err(|err| prefix(&artifact.artifact_id, err))?;
            validate_io(&artifact.kind, &artifact.io)
                .map_err(|err| prefix(&artifact.artifact_id, err))?;
            if artifact.source.training_data.is_empty() || artifact.source.generator.is_empty() {
                return Err(ModelSetError::Schema(format!(
                    "{} 缺少 source",
                    artifact.artifact_id
                )));
            }
        }
        let rerankers = kinds.get(KIND_RERANKER).copied().unwrap_or(0);
        let punctuations = kinds.get(KIND_PUNCTUATION).copied().unwrap_or(0);
        if rerankers != 1 || punctuations != 1 {
            return Err(ModelSetError::IncompleteSet(format!(
                "模型包需要各一個 reranker 與 punctuation，目前 reranker={rerankers} punctuation={punctuations}"
            )));
        }
        if self.placeholder != placeholder {
            return Err(ModelSetError::PlaceholderUnmarked(format!(
                "模型包 placeholder={} 與模型 {} 不一致",
                self.placeholder, placeholder
            )));
        }
        if self.placeholder && self.quality != QUALITY_UNTRAINED {
            return Err(ModelSetError::PlaceholderUnmarked(format!(
                "placeholder 模型包 quality={:?}",
                self.quality
            )));
        }
        if self.placeholder && self.production_eligible {
            return Err(ModelSetError::PlaceholderUnmarked(
                "placeholder 模型包不得 production_eligible".to_string(),
            ));
        }
        if release {
            if self.placeholder {
                return Err(ModelSetError::Placeholder(
                    "模型包是 placeholder，不得進入正式發行".to_string(),
                ));
            }
            if !self.production_eligible {
                return Err(ModelSetError::HfSource(
                    "模型包沒有標示 production_eligible".to_string(),
                ));
            }
        }
        Ok(())
    }

    /// 驗證模型包可以安裝或成為使用中的模型包。
    ///
    /// 檢查結構、已安裝字典包雜湊與 placeholder 設定，並可選核對實際檔案大小
    /// 與 SHA-256。
    pub fn validate(&self, opts: &Options) -> Result<ValidationResult> {
        self.check_structure(opts.release)?;
        if opts.installed_dictionary_sha256.is_empty() {
            return Err(ModelSetError::DictionaryUnknown(
                "無法取得已安裝字典包的 manifest 雜湊".to_string(),
            ));
        }
        if opts.installed_dictionary_sha256 != self.dictionary.manifest_sha256 {
            return Err(ModelSetError::DictionaryMismatch(format!(
                "模型包記的是 {}，已安裝字典包是 {}",
                self.dictionary.manifest_sha256, opts.installed_dictionary_sha256
            )));
        }
        if !opts.installed_dictionary_pack_id.is_empty()
            && opts.installed_dictionary_pack_id != self.dictionary.pack_id
        {
            return Err(ModelSetError::DictionaryMismatch(format!(
                "模型包要 {:?}，已安裝字典包是 {:?}",
                self.dictionary.pack_id, opts.installed_dictionary_pack_id
            )));
        }
        if self.placeholder && !opts.allow_placeholders {
            return Err(ModelSetError::Placeholder(format!(
                "權重未訓練（quality={}），設定未允許 placeholder",
                self.quality
            )));
        }
        let format = if opts.format.is_empty() {
            FORMAT_INT8
        } else {
            opts.format.as_str()
        };
        let mut result = ValidationResult {
            version: self.version.clone(),
            placeholder: self.placeholder,
            quality: self.quality.clone(),
            dictionary: self.dictionary.clone(),
            dictionary_matches: true,
            installed_dictionary_sha256: opts.installed_dictionary_sha256.clone(),
            artifacts: Vec::with_capacity(self.artifacts.len()),
        };
        for artifact in &self.artifacts {
            let file = artifact.files.get(format).ok_or_else(|| {
                ModelSetError::Schema(format!("{} 沒有 {format} 檔案", artifact.artifact_id))
            })?;
            let mut status = ArtifactStatus {
                artifact_id: artifact.artifact_id.clone(),
                kind: artifact.kind.clone(),
                variant: artifact.variant.clone(),
                format: format.to_string(),
                path: file.path.clone(),
                bytes: file.bytes,
                sha256: file.sha256.clone(),
                placeholder: artifact.placeholder,
                quality: artifact.quality.clone(),
            };
            if opts.verify_files {
                if opts.artifact_root.as_os_str().is_empty() {
                    return Err(ModelSetError::FileMismatch("沒有模型根目錄".to_string()));
                }
                let actual = verify_file(&opts.artifact_root, file)
                    .map_err(|err| prefix(&artifact.artifact_id, err))?;
                status.bytes = actual.bytes;
                status.sha256 = actual.sha256;
                for asset in &artifact.assets {
                    verify_file(
                        &opts.artifact_root,
                        &ArtifactFile {
                            path: asset.path.clone(),
                            bytes: asset.bytes,
                            sha256: asset.sha256.clone(),
                            quantization: String::new(),
                        },
                    )
                    .map_err(|err| {
                        prefix(&format!("{}/{}", artifact.artifact_id, asset.name), err)
                    })?;
                }
            }
            result.artifacts.push(status);
        }
        Ok(result)
    }

    /// 模型包是否為 placeholder 且未被允許。
    pub fn check_placeholder_allowed(&self, allow: bool) -> Result<()> {
        if self.placeholder && !allow {
            return Err(ModelSetError::Placeholder(format!(
                "權重未訓練（quality={}），設定未允許 placeholder",
                self.quality
            )));
        }
        Ok(())
    }
}

fn prefix(id: &str, err: ModelSetError) -> ModelSetError {
    match err {
        ModelSetError::Schema(detail) => ModelSetError::Schema(format!("{id}: {detail}")),
        ModelSetError::VariantMixed(detail) => {
            ModelSetError::VariantMixed(format!("{id}: {detail}"))
        }
        ModelSetError::IncompleteSet(detail) => {
            ModelSetError::IncompleteSet(format!("{id}: {detail}"))
        }
        ModelSetError::DictionaryMismatch(detail) => {
            ModelSetError::DictionaryMismatch(format!("{id}: {detail}"))
        }
        ModelSetError::DictionaryUnknown(detail) => {
            ModelSetError::DictionaryUnknown(format!("{id}: {detail}"))
        }
        ModelSetError::Placeholder(detail) => ModelSetError::Placeholder(format!("{id}: {detail}")),
        ModelSetError::PlaceholderUnmarked(detail) => {
            ModelSetError::PlaceholderUnmarked(format!("{id}: {detail}"))
        }
        ModelSetError::TensorContract(detail) => {
            ModelSetError::TensorContract(format!("{id}: {detail}"))
        }
        ModelSetError::FileMismatch(detail) => {
            ModelSetError::FileMismatch(format!("{id}: {detail}"))
        }
        ModelSetError::LicenseMixed(detail) => {
            ModelSetError::LicenseMixed(format!("{id}: {detail}"))
        }
        ModelSetError::Selection(detail) => ModelSetError::Selection(format!("{id}: {detail}")),
        ModelSetError::Unpinned(detail) => ModelSetError::Unpinned(format!("{id}: {detail}")),
        ModelSetError::HfSource(detail) => ModelSetError::HfSource(format!("{id}: {detail}")),
        ModelSetError::Checksum(detail) => ModelSetError::Checksum(format!("{id}: {detail}")),
        ModelSetError::Offline(detail) => ModelSetError::Offline(format!("{id}: {detail}")),
        ModelSetError::Io(err) => ModelSetError::Io(err),
    }
}

/// 計算字典包 manifest 的雜湊。
///
/// `path` 可以是 `manifest.json` 本身，或含 `manifest.json` 的目錄。
pub fn dictionary_sha256(path: &Path) -> Result<String> {
    if path.as_os_str().is_empty() {
        return Err(ModelSetError::DictionaryUnknown(
            "沒有字典包路徑".to_string(),
        ));
    }
    let manifest = if path.is_dir() {
        path.join("manifest.json")
    } else {
        path.to_path_buf()
    };
    let bytes = fs::read(&manifest).map_err(|err| {
        ModelSetError::DictionaryUnknown(format!("{}: {err}", manifest.display()))
    })?;
    Ok(hex_digest(&bytes))
}

/// 讀取字典包 manifest 的 `lexicon` 或 `pack_id` 欄位；沒有時回傳空字串。
pub fn dictionary_pack_id(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        return String::new();
    }
    let manifest = if path.is_dir() {
        path.join("manifest.json")
    } else {
        path.to_path_buf()
    };
    let Ok(bytes) = fs::read(&manifest) else {
        return String::new();
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return String::new();
    };
    value
        .get("pack_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .or_else(|| value.get("lexicon").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

fn check_package_license(license: &License) -> Result<()> {
    if license.id.is_empty()
        || license.scope.is_empty()
        || license.distribution.is_empty()
        || license.notice.is_empty()
    {
        return Err(ModelSetError::Schema(format!("license={license:?}")));
    }
    if !license.id.to_uppercase().contains("CC-BY-SA")
        || license.scope.to_lowercase() != VARIANT_CC_BY_SA
    {
        return Err(ModelSetError::LicenseMixed(format!(
            "模型包包含 CC BY-SA reranker，授權應為 CC-BY-SA，得到 {}",
            license.id
        )));
    }
    Ok(())
}

fn check_license(variant: &str, license: &License) -> Result<()> {
    if license.id.is_empty()
        || license.scope.is_empty()
        || license.distribution.is_empty()
        || license.notice.is_empty()
    {
        return Err(ModelSetError::Schema(format!("license={license:?}")));
    }
    let id = license.id.to_uppercase();
    let scope = license.scope.to_lowercase();
    match variant {
        VARIANT_PUBLIC_CORE => {
            if scope != VARIANT_PUBLIC_CORE {
                return Err(ModelSetError::LicenseMixed(format!(
                    "public-core 模型含 scope={:?} 的授權",
                    license.scope
                )));
            }
            if id.contains("-SA") || id.contains("-NC") {
                return Err(ModelSetError::LicenseMixed(format!(
                    "public-core 模型不得使用 {}",
                    license.id
                )));
            }
        }
        VARIANT_CC_BY_SA => {
            if scope != VARIANT_CC_BY_SA {
                return Err(ModelSetError::LicenseMixed(format!(
                    "cc-by-sa 模型含 scope={:?} 的授權",
                    license.scope
                )));
            }
            if !id.contains("CC-BY-SA") {
                return Err(ModelSetError::LicenseMixed(format!(
                    "cc-by-sa 模型的授權應為 CC-BY-SA，得到 {}",
                    license.id
                )));
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_hf(artifact: &Artifact, release: bool) -> Result<()> {
    let Some(hf) = artifact.hf.as_ref() else {
        if release {
            return Err(ModelSetError::HfSource(format!(
                "{} 缺少 hf 來源",
                artifact.artifact_id
            )));
        }
        return Ok(());
    };
    if artifact.placeholder {
        return Err(ModelSetError::HfSource(format!(
            "{} 是 placeholder，不能有 HF 來源",
            artifact.artifact_id
        )));
    }
    if hf.repository.is_empty() {
        return Err(ModelSetError::HfSource(format!(
            "{} 缺少 repository",
            artifact.artifact_id
        )));
    }
    if hf.revision.is_empty() {
        if release {
            return Err(ModelSetError::Unpinned(format!(
                "{}（{}）缺少 revision；先執行 pin --repo {} --revision <commit>",
                artifact.artifact_id, hf.repository, hf.repository
            )));
        }
    } else if !valid_revision(&hf.revision) {
        return Err(ModelSetError::HfSource(format!(
            "{} 的 revision {:?} 不是 commit 雜湊",
            artifact.artifact_id, hf.revision
        )));
    }
    if hf.license.is_empty() {
        return Err(ModelSetError::HfSource(format!(
            "{} 缺少 HF license",
            artifact.artifact_id
        )));
    }
    if hf.files.is_empty() {
        if release {
            return Err(ModelSetError::HfSource(format!(
                "{} 缺少 hf.files",
                artifact.artifact_id
            )));
        }
        return Ok(());
    }
    for (format, remote) in &hf.files {
        let Some(file) = artifact.files.get(format) else {
            return Err(ModelSetError::HfSource(format!(
                "{} 的 hf 檔 {format} 沒有對應的本機格式",
                artifact.artifact_id
            )));
        };
        if remote.name.is_empty() || !is_bare_file_name(&remote.name) {
            return Err(ModelSetError::HfSource(format!(
                "{}/{format} 的檔名 {:?} 不合法",
                artifact.artifact_id, remote.name
            )));
        }
        if remote.bytes == 0 && remote.sha256.is_empty() {
            if release {
                return Err(ModelSetError::Unpinned(format!(
                    "{}/{format} 尚未 pin 大小與 SHA-256",
                    artifact.artifact_id
                )));
            }
            continue;
        }
        if remote.bytes <= 0 || !valid_hex(&remote.sha256) {
            return Err(ModelSetError::HfSource(format!(
                "{}/{format} 的大小或 SHA-256 不正確",
                artifact.artifact_id
            )));
        }
        if file.bytes != 0 && file.bytes != remote.bytes {
            return Err(ModelSetError::HfSource(format!(
                "{}/{format} 的本機大小與 hf 不符",
                artifact.artifact_id
            )));
        }
        if !file.sha256.is_empty() && file.sha256 != remote.sha256 {
            return Err(ModelSetError::HfSource(format!(
                "{}/{format} 的本機 SHA-256 與 hf 不符",
                artifact.artifact_id
            )));
        }
    }
    if artifact.assets.is_empty() {
        return Ok(());
    }
    for asset in &artifact.assets {
        let Some(remote) = hf.assets.get(&asset.name) else {
            if release {
                return Err(ModelSetError::HfSource(format!(
                    "{}/{} 缺少 hf asset",
                    artifact.artifact_id, asset.name
                )));
            }
            continue;
        };
        if remote.name.is_empty() || !is_bare_file_name(&remote.name) {
            return Err(ModelSetError::HfSource(format!(
                "{}/{} 的 hf 檔名 {:?} 不合法",
                artifact.artifact_id, asset.name, remote.name
            )));
        }
        if remote.bytes == 0 && remote.sha256.is_empty() {
            if release {
                return Err(ModelSetError::Unpinned(format!(
                    "{}/{} 尚未 pin 大小與 SHA-256",
                    artifact.artifact_id, asset.name
                )));
            }
            continue;
        }
        if remote.bytes <= 0 || !valid_hex(&remote.sha256) {
            return Err(ModelSetError::HfSource(format!(
                "{}/{} 的大小或 SHA-256 不正確",
                artifact.artifact_id, asset.name
            )));
        }
        if asset.bytes != remote.bytes || asset.sha256 != remote.sha256 {
            return Err(ModelSetError::HfSource(format!(
                "{}/{} 的本機大小或 SHA-256 與 hf 不符",
                artifact.artifact_id, asset.name
            )));
        }
    }
    Ok(())
}

fn validate_files(artifact: &Artifact, release: bool) -> Result<()> {
    if artifact.files.is_empty() {
        return Err(ModelSetError::Schema("沒有檔案".to_string()));
    }
    if !artifact.files.contains_key(FORMAT_INT8) {
        return Err(ModelSetError::Schema(format!("缺少 {FORMAT_INT8} 檔案")));
    }
    let unpinned = artifact.hf.as_ref().is_some_and(|hf| !hf.pinned()) && !release;
    for (format, file) in &artifact.files {
        if format != FORMAT_INT8 && format != FORMAT_FP32 {
            return Err(ModelSetError::Schema(format!("未知格式 {format:?}")));
        }
        validate_file(file, unpinned)?;
        if file.path.contains(".placeholder.") && !artifact.placeholder {
            return Err(ModelSetError::PlaceholderUnmarked(format!(
                "{} 的檔名標示 placeholder，但 manifest 沒有標記",
                file.path
            )));
        }
    }
    Ok(())
}

fn validate_assets(artifact: &Artifact, release: bool) -> Result<()> {
    let mut seen_name = BTreeSet::new();
    let mut seen_path = BTreeSet::new();
    let dir = artifact.package_dir();
    for asset in &artifact.assets {
        if asset.name.is_empty() || !seen_name.insert(asset.name.clone()) {
            return Err(ModelSetError::Schema(format!(
                "asset 名稱空白或重複：{:?}",
                asset.name
            )));
        }
        if asset.path.is_empty() || !is_local_relative(&asset.path) {
            return Err(ModelSetError::Schema(format!(
                "asset {} 路徑 {:?} 不是相對路徑",
                asset.name, asset.path
            )));
        }
        if !seen_path.insert(asset.path.clone()) {
            return Err(ModelSetError::Schema(format!(
                "asset 路徑重複：{}",
                asset.path
            )));
        }
        if !dir.is_empty()
            && slash_dir(Path::new(&asset.path).parent().unwrap_or(Path::new(""))) != dir
        {
            return Err(ModelSetError::Schema(format!(
                "asset {} 必須與 manifest 同在 {dir}",
                asset.name
            )));
        }
        if asset.bytes == 0 && asset.sha256.is_empty() && !release {
            continue;
        }
        if asset.bytes <= 0 || !valid_hex(&asset.sha256) {
            return Err(ModelSetError::Schema(format!(
                "asset {} 的大小或 SHA-256 不正確",
                asset.name
            )));
        }
    }
    if artifact.assets.is_empty() {
        if release {
            return Err(ModelSetError::IncompleteSet(
                "缺少 manifest/vocab/tokenizer assets".to_string(),
            ));
        }
        return Ok(());
    }
    for name in ["manifest.json", "vocab.json", "tokenizer.model"] {
        let Some(asset) = artifact.asset(name) else {
            return Err(ModelSetError::IncompleteSet(format!("缺少 asset {name}")));
        };
        if !artifact.manifest.is_empty()
            && asset.name == "manifest.json"
            && asset.path != artifact.manifest
        {
            return Err(ModelSetError::Schema(format!(
                "manifest asset 路徑 {:?} 與 manifest={:?} 不符",
                asset.path, artifact.manifest
            )));
        }
    }
    Ok(())
}

fn validate_file(file: &ArtifactFile, unpinned: bool) -> Result<()> {
    if file.path.is_empty() || !is_local_relative(&file.path) {
        return Err(ModelSetError::Schema(format!(
            "路徑 {:?} 不是模型根目錄內的相對路徑",
            file.path
        )));
    }
    if Path::new(&file.path)
        .extension()
        .is_none_or(|ext| ext != "onnx")
    {
        return Err(ModelSetError::Schema(format!(
            "{:?} 不是 ONNX 檔",
            file.path
        )));
    }
    if unpinned && file.bytes == 0 && file.sha256.is_empty() {
        return Ok(());
    }
    if file.bytes <= 0 || !valid_hex(&file.sha256) {
        return Err(ModelSetError::Schema(format!(
            "{:?} 的大小或 SHA-256 不正確",
            file.path
        )));
    }
    Ok(())
}

/// Hugging Face 的 revision 是提交的雜湊（小寫十六進位）。
fn valid_revision(revision: &str) -> bool {
    (7..=64).contains(&revision.len())
        && revision
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn valid_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_bare_file_name(name: &str) -> bool {
    Path::new(name).file_name().is_some_and(|file| file == name)
}

/// 相對路徑且不含 `..`；對照 Go 的 `filepath.IsLocal`。
pub fn is_local_relative(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    let mut normal = 0_usize;
    for component in Path::new(path).components() {
        match component {
            Component::Normal(_) => normal += 1,
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    normal > 0
}

fn slash_dir(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn validate_io(kind: &str, io: &TensorIo) -> Result<()> {
    if io.inputs.len() != 2 {
        return Err(ModelSetError::TensorContract(format!(
            "需要 input_ids 與 attention_mask 兩個輸入，得到 {}",
            io.inputs.len()
        )));
    }
    let mut want = BTreeMap::from([("input_ids", false), ("attention_mask", false)]);
    for input in &io.inputs {
        let Some(seen) = want.get_mut(input.name.as_str()) else {
            return Err(ModelSetError::TensorContract(format!(
                "未知或重複的輸入 {:?}",
                input.name
            )));
        };
        if *seen {
            return Err(ModelSetError::TensorContract(format!(
                "未知或重複的輸入 {:?}",
                input.name
            )));
        }
        *seen = true;
        if input.dtype != "int64"
            || !shape_is(&input.shape, &[Shape::Name("batch"), Shape::Name("seq")])
        {
            return Err(ModelSetError::TensorContract(format!(
                "{} 必須是 int64 [batch, seq]",
                input.name
            )));
        }
    }
    for (name, seen) in want {
        if !seen {
            return Err(ModelSetError::TensorContract(format!("缺少輸入 {name}")));
        }
    }
    if io.outputs.len() != 1 {
        return Err(ModelSetError::TensorContract(format!(
            "需要一個輸出，得到 {}",
            io.outputs.len()
        )));
    }
    let output = &io.outputs[0];
    match kind {
        KIND_RERANKER => {
            if output.name != "scores"
                || output.dtype != "float32"
                || !shape_is(&output.shape, &[Shape::Name("batch")])
            {
                return Err(ModelSetError::TensorContract(
                    "reranker 輸出必須是 float32 scores [batch]".to_string(),
                ));
            }
        }
        KIND_PUNCTUATION
            if output.name != "logits"
                || output.dtype != "float32"
                || !shape_is(
                    &output.shape,
                    &[Shape::Name("batch"), Shape::Name("seq"), Shape::Int(4)],
                ) =>
        {
            return Err(ModelSetError::TensorContract(
                "punctuation 輸出必須是 float32 logits [batch, seq, 4]".to_string(),
            ));
        }
        _ => {}
    }
    Ok(())
}

enum Shape {
    Name(&'static str),
    Int(i64),
}

fn shape_is(got: &[Value], want: &[Shape]) -> bool {
    if got.len() != want.len() {
        return false;
    }
    got.iter().zip(want).all(|(got, want)| match want {
        Shape::Name(name) => got.as_str() == Some(name),
        Shape::Int(value) => got.as_f64().is_some_and(|n| n == *value as f64),
    })
}

/// 核對一個模型的實際大小與 SHA-256，回傳實際值。
pub fn verify_file(root: &Path, file: &ArtifactFile) -> Result<ArtifactFile> {
    let path = root.join(&file.path);
    let metadata = fs::metadata(&path)
        .map_err(|err| ModelSetError::FileMismatch(format!("{}: {err}", file.path)))?;
    if !metadata.is_file() {
        return Err(ModelSetError::FileMismatch(format!(
            "{} 不是一般檔案",
            file.path
        )));
    }
    if metadata.len() as i64 != file.bytes {
        return Err(ModelSetError::FileMismatch(format!(
            "{} 大小 {}，清單記 {}",
            file.path,
            metadata.len(),
            file.bytes
        )));
    }
    let mut handle = fs::File::open(&path)
        .map_err(|err| ModelSetError::FileMismatch(format!("{}: {err}", file.path)))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = handle
            .read(&mut buffer)
            .map_err(|err| ModelSetError::FileMismatch(format!("{}: {err}", file.path)))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = format!("{:x}", hasher.finalize());
    if digest != file.sha256 {
        return Err(ModelSetError::FileMismatch(format!(
            "{} SHA-256 {}，清單記 {}",
            file.path, digest, file.sha256
        )));
    }
    Ok(ArtifactFile {
        path: file.path.clone(),
        bytes: metadata.len() as i64,
        sha256: digest,
        quantization: file.quantization.clone(),
    })
}

/// 計算 bytes 的 SHA-256 十六進位字串。
pub fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
