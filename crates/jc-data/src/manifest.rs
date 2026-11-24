//! 資料根目錄的 `manifest.json`：型別、產生與驗證。
//!
//! 這份 manifest 描述「安裝版資料根目錄有哪些檔、各是什麼角色、能不能隨產品
//! 散布」。它與 [`crate::zh::Manifest`]、[`crate::ja::Manifest`] 描述的單一
//! 資料包 manifest 不同：這裡是整個資料根目錄的索引，供安裝器挑檔與引擎在
//! 啟動時做低成本的完整性檢查。
//!
//! # JSON schema
//!
//! ```json
//! {
//!   "schema": "0c.data-pack/1",
//!   "version": "2026-11-20",
//!   "generated_at": "2026-11-20T12:34:56Z",
//!   "files": [
//!     {
//!       "path": "build-zh/lexicon.bin",
//!       "bytes": 8791416,
//!       "sha256": "46721ccb95e55c7fc36abd3a351f9191eef087e47bd30b580c5711c05dc30f57",
//!       "role": "zh_lexicon",
//!       "license": ["L-04", "L-05", "L-06", "L-13", "L-14", "L-15", "L-16", "L-17"],
//!       "shipped": true
//!     }
//!   ]
//! }
//! ```
//!
//! | 欄位 | 型別 | 說明 |
//! |---|---|---|
//! | `schema` | string | 格式版本，固定 [`SCHEMA`]。不符就拒絕載入。 |
//! | `version` | string | 資料內容版本標籤，預設是產生當天的 UTC 日期；只記錄與顯示，不參與比對。 |
//! | `generated_at` | string | 產生時間，RFC 3339 UTC，例如 `2026-11-20T12:34:56Z`。 |
//! | `files[].path` | string | 相對於資料根目錄的路徑，一律用 `/`；不可絕對路徑、不可含 `..`。 |
//! | `files[].bytes` | integer | 檔案大小，位元組。 |
//! | `files[].sha256` | string | 小寫十六進位 SHA-256。 |
//! | `files[].role` | string | 見 [`DataRole`]。 |
//! | `files[].license` | string[] | 對應 `legal/README.md` 的授權條目，例如 `L-11`。 |
//! | `files[].shipped` | bool | `true` 隨產品散布；`false` 由使用者另外下載。 |
//! | `files[].url` | string? | 下載項的來源 URL；隨附檔通常沒有。 |
//!
//! `settings.json` 之類的使用者資料不列入 manifest。
//!
//! # 驗證分級
//!
//! - [`VerifyLevel::Startup`]：只檢查存在與大小，給引擎啟動用；不算雜湊，
//!   避免每次啟動重算上百 MB。
//! - [`VerifyLevel::Full`]：再加 SHA-256，給 `jc-cli data verify` 與安裝器用。
//!
//! `shipped: true` 的檔案缺席、或任何已存在檔案的大小／雜湊不符，都算錯誤；
//! `shipped: false` 的檔案缺席只算警告。

use std::fs;
use std::io::{BufReader, Read};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// 資料根目錄 manifest 的檔名。
pub const MANIFEST_FILE: &str = "manifest.json";

/// 資料根目錄 manifest 的格式版本。
pub const SCHEMA: &str = "0c.data-pack/1";

/// 驗證深度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyLevel {
    /// 只檢查存在與大小；引擎啟動用。
    Startup,
    /// 檢查存在、大小與 SHA-256。
    Full,
}

/// 資料檔在產品中的角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataRole {
    /// 日文詞典 pack（Mozc v2 或 public-core）。
    JaLexicon,
    /// 注音詞典 `build-zh/lexicon.bin`。
    ZhLexicon,
    /// 注音語言模型 `build-zh-bigram/lm.bin`。
    ZhLm,
    /// 英文詞表 `english/words.bin`。
    English,
    /// 日文神經轉換模型（Jinen GGUF）。
    JaNeural,
    /// Jinen 的 tokenizer `tokenizer.json`；與 [`Self::JaNeural`] 同授權。
    JaTokenizer,
    /// 注音選字模型（Funmula，僅下載）。
    ZhSelect,
}

impl DataRole {
    /// `manifest.json` 裡使用的字串。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::JaLexicon => "ja_lexicon",
            Self::ZhLexicon => "zh_lexicon",
            Self::ZhLm => "zh_lm",
            Self::English => "english",
            Self::JaNeural => "ja_neural",
            Self::JaTokenizer => "ja_tokenizer",
            Self::ZhSelect => "zh_select",
        }
    }
}

/// 一筆 manifest 記錄。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataFile {
    /// 相對於資料根目錄的路徑，一律使用 `/`。
    pub path: String,
    /// 檔案大小，位元組。
    pub bytes: u64,
    /// 小寫十六進位 SHA-256。
    pub sha256: String,
    /// 檔案角色。
    pub role: DataRole,
    /// 對應 `legal/README.md` 的授權條目。
    pub license: Vec<String>,
    /// `true` 隨產品散布；`false` 由使用者另外下載。
    pub shipped: bool,
    /// 下載項的來源 URL。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// 資料根目錄的 manifest。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataManifest {
    /// 格式版本，必須等於 [`SCHEMA`]。
    pub schema: String,
    /// 資料內容版本標籤；只記錄與顯示。
    pub version: String,
    /// 產生時間，RFC 3339 UTC。
    pub generated_at: String,
    /// 逐檔記錄。
    pub files: Vec<DataFile>,
}

/// manifest 讀取、產生或驗證失敗。
#[derive(Debug, Error)]
pub enum ManifestError {
    /// 檔案開不起來或讀不到。
    #[error("讀取 {path} 失敗：{source}")]
    Io {
        /// 出問題的路徑。
        path: PathBuf,
        /// 底層 I/O 錯誤。
        #[source]
        source: std::io::Error,
    },
    /// JSON 解析失敗。
    #[error("解析 {path} 失敗：{source}")]
    Json {
        /// manifest 路徑。
        path: PathBuf,
        /// JSON 解析錯誤。
        #[source]
        source: serde_json::Error,
    },
    /// 格式版本不符。
    #[error("manifest schema 不符：程式需要 {expected}，實際是 {actual}")]
    Schema {
        /// 程式支援的版本。
        expected: &'static str,
        /// manifest 記錄的版本。
        actual: String,
    },
    /// 路徑不是安全的相對路徑。
    #[error("manifest 路徑不合法：{0}")]
    UnsafePath(String),
    /// 同一個路徑出現兩次。
    #[error("manifest 路徑重複：{0}")]
    DuplicatePath(String),
    /// SHA-256 不是 64 個小寫十六進位字元。
    #[error("manifest 的 sha256 格式不合法：{path}")]
    BadHash {
        /// 出問題的檔案路徑。
        path: String,
    },
    /// 授權條目不是 `L-xx`。
    #[error("manifest 的授權條目不合法：{path} 的 {license}")]
    BadLicense {
        /// 檔案路徑。
        path: String,
        /// 不合法的授權條目。
        license: String,
    },
    /// manifest 沒有列出任何檔案。
    #[error("manifest 沒有列出任何檔案")]
    Empty,
    /// 必要檔案不存在，無法產生 manifest。
    #[error("缺少 {role} 的必要檔案，找過：{candidates}")]
    MissingRequired {
        /// 角色名稱。
        role: &'static str,
        /// 依序嘗試過的路徑。
        candidates: String,
    },
}

impl DataManifest {
    /// 讀取並驗證 manifest。
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ManifestError> {
        let path = path.as_ref();
        let data = fs::read(path).map_err(|source| ManifestError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_slice(&data).map_err(|err| match err {
            ManifestError::Json { source, .. } => ManifestError::Json {
                path: path.to_path_buf(),
                source,
            },
            other => other,
        })
    }

    /// 從 JSON 位元組解析並驗證 manifest。
    pub fn from_slice(data: &[u8]) -> Result<Self, ManifestError> {
        let manifest: Self =
            serde_json::from_slice(data).map_err(|source| ManifestError::Json {
                path: PathBuf::from(MANIFEST_FILE),
                source,
            })?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// 產生資料根目錄的 manifest。
    ///
    /// 依 [`CATALOG`] 的順序逐檔計算大小與 SHA-256；下載項不存在就略過，
    /// 隨附檔不存在則回傳錯誤。`version` 是資料內容標籤，`generated_at`
    /// 由呼叫端提供，讓測試可以固定時間。
    pub fn generate(
        root: impl AsRef<Path>,
        version: impl Into<String>,
        generated_at: impl Into<String>,
    ) -> Result<Self, ManifestError> {
        let root = root.as_ref();
        let mut files = Vec::new();
        for spec in CATALOG {
            let Some(path) = spec.candidates.iter().find(|rel| root.join(rel).is_file()) else {
                if spec.shipped {
                    return Err(ManifestError::MissingRequired {
                        role: spec.role.as_str(),
                        candidates: spec.candidates.join("、"),
                    });
                }
                continue;
            };
            let full = root.join(path);
            let (bytes, sha256) = sha256_file(&full)?;
            files.push(DataFile {
                path: normalize_rel_path(path),
                bytes,
                sha256,
                role: spec.role,
                license: spec.licenses.iter().map(|id| id.to_string()).collect(),
                shipped: spec.shipped,
                url: spec.url.map(str::to_string),
            });
        }
        let manifest = Self {
            schema: SCHEMA.to_string(),
            version: version.into(),
            generated_at: generated_at.into(),
            files,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// 產生 manifest，時間戳與資料版本都取當下 UTC。
    pub fn generate_now(root: impl AsRef<Path>) -> Result<Self, ManifestError> {
        let now = utc_now_rfc3339();
        let version = now[..10].to_string();
        Self::generate(root, version, now)
    }

    /// 檢查 schema、路徑、雜湊格式與授權代號。
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.schema != SCHEMA {
            return Err(ManifestError::Schema {
                expected: SCHEMA,
                actual: self.schema.clone(),
            });
        }
        if self.files.is_empty() {
            return Err(ManifestError::Empty);
        }
        let mut seen = std::collections::BTreeSet::new();
        for file in &self.files {
            if !is_safe_rel_path(&file.path) {
                return Err(ManifestError::UnsafePath(file.path.clone()));
            }
            if !seen.insert(file.path.as_str()) {
                return Err(ManifestError::DuplicatePath(file.path.clone()));
            }
            if !is_sha256_hex(&file.sha256) {
                return Err(ManifestError::BadHash {
                    path: file.path.clone(),
                });
            }
            if file.license.is_empty() {
                return Err(ManifestError::BadLicense {
                    path: file.path.clone(),
                    license: String::new(),
                });
            }
            for id in &file.license {
                if !is_license_id(id) {
                    return Err(ManifestError::BadLicense {
                        path: file.path.clone(),
                        license: id.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    /// 逐項比對資料根目錄。
    ///
    /// 這個函式不因個別檔案失敗而中斷；呼叫端用 [`VerifyReport::is_ok`]
    /// 判斷要不要擋下啟動或安裝。
    pub fn verify(&self, root: impl AsRef<Path>, level: VerifyLevel) -> VerifyReport {
        let root = root.as_ref();
        let checks = self
            .files
            .iter()
            .map(|file| check_file(root, file, level))
            .collect();
        VerifyReport { checks }
    }

    /// 需要隨產品散布的檔案。
    pub fn shipped_files(&self) -> impl Iterator<Item = &DataFile> {
        self.files.iter().filter(|file| file.shipped)
    }

    /// 需要使用者另外下載的檔案。
    pub fn download_files(&self) -> impl Iterator<Item = &DataFile> {
        self.files.iter().filter(|file| !file.shipped)
    }

    /// 輸出穩定的縮排 JSON，結尾帶換行。
    pub fn to_json_pretty(&self) -> Result<String, ManifestError> {
        let mut text =
            serde_json::to_string_pretty(self).map_err(|source| ManifestError::Json {
                path: PathBuf::from(MANIFEST_FILE),
                source,
            })?;
        text.push('\n');
        Ok(text)
    }

    /// 資料檔總大小。
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.bytes).sum()
    }
}

/// 單一檔案的檢查結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileCheck {
    /// 相對於資料根目錄的路徑。
    pub path: String,
    /// 檔案角色。
    pub role: DataRole,
    /// 是否隨產品散布。
    pub shipped: bool,
    /// 檢查結果。
    pub status: CheckStatus,
}

impl FileCheck {
    /// 這個失敗是否該擋下啟動或安裝。
    pub fn is_error(&self) -> bool {
        match &self.status {
            CheckStatus::Ok => false,
            CheckStatus::Missing => self.shipped,
            CheckStatus::Size { .. } | CheckStatus::Hash { .. } | CheckStatus::Read { .. } => true,
        }
    }

    /// 這個失敗是否只是警告。
    pub fn is_warning(&self) -> bool {
        self.status == CheckStatus::Missing && !self.shipped
    }
}

/// 單一檔案的檢查狀態。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CheckStatus {
    /// 通過。
    Ok,
    /// 檔案不存在。
    Missing,
    /// 大小不符。
    Size {
        /// manifest 記錄的大小。
        expected: u64,
        /// 實際大小。
        actual: u64,
    },
    /// SHA-256 不符。
    Hash {
        /// manifest 記錄的雜湊。
        expected: String,
        /// 實際算出的雜湊。
        actual: String,
    },
    /// 檔案存在但讀不到。
    Read {
        /// 錯誤訊息。
        message: String,
    },
}

/// 整份 manifest 的驗證結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerifyReport {
    /// 逐檔結果。
    pub checks: Vec<FileCheck>,
}

impl VerifyReport {
    /// 有幾個錯誤。
    pub fn errors(&self) -> usize {
        self.checks.iter().filter(|check| check.is_error()).count()
    }

    /// 有幾個警告。
    pub fn warnings(&self) -> usize {
        self.checks
            .iter()
            .filter(|check| check.is_warning())
            .count()
    }

    /// 通過幾個檔案。
    pub fn passed(&self) -> usize {
        self.checks
            .iter()
            .filter(|check| check.status == CheckStatus::Ok)
            .count()
    }

    /// 沒有錯誤就回傳 `true`；警告不影響結果。
    pub fn is_ok(&self) -> bool {
        self.errors() == 0
    }

    /// 給記錄用的一行摘要。
    pub fn summary(&self) -> String {
        format!(
            "通過 {}/{}，錯誤 {}，警告 {}",
            self.passed(),
            self.checks.len(),
            self.errors(),
            self.warnings()
        )
    }
}

/// 產生 manifest 的固定清單。
///
/// 一筆一個檔案。`candidates` 由左到右找第一個存在的路徑，讓同一角色可以
/// 相容舊的開發目錄；找到後記錄實際相對路徑。
#[derive(Debug)]
pub struct DataFileSpec {
    /// 檔案角色。
    pub role: DataRole,
    /// 對應 `legal/README.md` 的授權條目。
    pub licenses: &'static [&'static str],
    /// 是否隨產品散布。
    pub shipped: bool,
    /// 候選相對路徑，依序嘗試。
    pub candidates: &'static [&'static str],
    /// 下載來源；隨附檔為 `None`。
    pub url: Option<&'static str>,
}

const ZH_LICENSES: &[&str] = &[
    "L-04", "L-05", "L-06", "L-13", "L-14", "L-15", "L-16", "L-17",
];

/// 目前產品資料檔的清單。
///
/// 注音詞典與語言模型的授權見 `legal/licenses/ZH-DATA-NOTICE.txt`；Funmula
/// 的選字模型依 MIT 全文可散布，安裝包帶轉好的 Q8_0 GGUF、`vocab.json` 與
/// 兩份模型 manifest，原始 ONNX 不進資料包。
pub const CATALOG: &[DataFileSpec] = &[
    DataFileSpec {
        role: DataRole::JaLexicon,
        licenses: &["L-01"],
        shipped: true,
        candidates: &["packs/ja-lexicon-mozc-v2.pack"],
        url: None,
    },
    DataFileSpec {
        role: DataRole::ZhLexicon,
        licenses: ZH_LICENSES,
        shipped: true,
        candidates: &["build-zh/lexicon.bin"],
        url: None,
    },
    DataFileSpec {
        role: DataRole::ZhLm,
        licenses: ZH_LICENSES,
        shipped: true,
        candidates: &["build-zh-bigram/lm.bin"],
        url: None,
    },
    // Bundle::open 一定要讀這份 manifest 驗證 lexicon.bin；缺席會讓引擎
    // 啟動 fatal。與 english 的三件式中繼檔是同一類問題。
    DataFileSpec {
        role: DataRole::ZhLexicon,
        licenses: ZH_LICENSES,
        shipped: true,
        candidates: &["build-zh/manifest.json"],
        url: None,
    },
    DataFileSpec {
        role: DataRole::English,
        licenses: &["L-08"],
        shipped: true,
        candidates: &["english/words.bin"],
        url: None,
    },
    // words.bin 的驗證中繼檔；英文載入器要求三者同時存在，缺一就退回原型詞彙。
    DataFileSpec {
        role: DataRole::English,
        licenses: &["L-08"],
        shipped: true,
        candidates: &["english/manifest.json"],
        url: None,
    },
    DataFileSpec {
        role: DataRole::English,
        licenses: &["L-08"],
        shipped: true,
        candidates: &["english/NOTICE.txt"],
        url: None,
    },
    DataFileSpec {
        role: DataRole::JaNeural,
        licenses: &["L-21"],
        shipped: true,
        candidates: &["models/external/jinen-v2-small/jinen-v2-small-Q5_K_M.gguf"],
        url: None,
    },
    DataFileSpec {
        role: DataRole::JaTokenizer,
        // small 沿用 xsmall 的 tokenizer，授權與 Jinen 權重同一筆 L-21。
        licenses: &["L-21"],
        shipped: true,
        candidates: &["models/quant/jinen-v2-xsmall/tokenizer.json"],
        url: None,
    },
    // Funmula 模型集合與 zh-select 自己的 runtime manifest；兩者都是載入
    // SelectModel 的路徑解析依據，體積很小，跟 GGUF 一起隨 MSI 安裝。
    DataFileSpec {
        role: DataRole::ZhSelect,
        licenses: &["L-11"],
        shipped: true,
        candidates: &["models/manifest.json"],
        url: None,
    },
    DataFileSpec {
        role: DataRole::ZhSelect,
        licenses: &["L-11"],
        shipped: true,
        candidates: &["models/zh-select/manifest.json"],
        url: None,
    },
    DataFileSpec {
        role: DataRole::ZhSelect,
        licenses: &["L-11"],
        shipped: true,
        candidates: &["models/zh-select/vocab.json"],
        url: None,
    },
    // ONNX 上游檔只當轉檔來源；GGUF 是本地衍生物，隨 MSI 散布並附 L-11 全文。
    DataFileSpec {
        role: DataRole::ZhSelect,
        licenses: &["L-11"],
        shipped: true,
        candidates: &["models/zh-select/model.q8_0.gguf"],
        url: None,
    },
];

fn check_file(root: &Path, file: &DataFile, level: VerifyLevel) -> FileCheck {
    let path = root.join(&file.path);
    let status = match fs::metadata(&path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => CheckStatus::Missing,
        Err(err) => CheckStatus::Read {
            message: err.to_string(),
        },
        Ok(meta) => {
            let actual = meta.len();
            if actual != file.bytes {
                CheckStatus::Size {
                    expected: file.bytes,
                    actual,
                }
            } else if level == VerifyLevel::Startup {
                CheckStatus::Ok
            } else {
                match sha256_file(&path) {
                    Err(err) => CheckStatus::Read {
                        message: err.to_string(),
                    },
                    Ok((_, actual)) => {
                        if actual.eq_ignore_ascii_case(&file.sha256) {
                            CheckStatus::Ok
                        } else {
                            CheckStatus::Hash {
                                expected: file.sha256.clone(),
                                actual,
                            }
                        }
                    }
                }
            }
        }
    };
    FileCheck {
        path: file.path.clone(),
        role: file.role,
        shipped: file.shipped,
        status,
    }
}

/// 計算檔案大小與 SHA-256。
pub fn sha256_file(path: impl AsRef<Path>) -> Result<(u64, String), ManifestError> {
    let path = path.as_ref();
    let file = fs::File::open(path).map_err(|source| ManifestError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let bytes = file
        .metadata()
        .map_err(|source| ManifestError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    let mut reader = BufReader::with_capacity(1 << 16, file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 1 << 16];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|source| ManifestError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok((bytes, hex(&hasher.finalize())))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_license_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 4
        && bytes[0] == b'L'
        && bytes[1] == b'-'
        && bytes[2].is_ascii_digit()
        && bytes[3].is_ascii_digit()
}

fn is_safe_rel_path(value: &str) -> bool {
    if value.is_empty() || value.contains('\\') {
        return false;
    }
    let path = Path::new(value);
    if path.is_absolute() {
        return false;
    }
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
}

fn normalize_rel_path(value: &str) -> String {
    value.replace('\\', "/")
}

/// 目前 UTC 時間的 RFC 3339 表示，精度到秒。
pub fn utc_now_rfc3339() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    rfc3339_from_unix(seconds)
}

fn rfc3339_from_unix(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// Howard Hinnant 的 `civil_from_days`：把 1970-01-01 起算的天數轉成日期。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("jc-data-manifest-{name}-{}", std::process::id()));
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

    #[test]
    fn role_names_are_snake_case() {
        assert_eq!(DataRole::JaLexicon.as_str(), "ja_lexicon");
        assert_eq!(DataRole::ZhLm.as_str(), "zh_lm");
        assert_eq!(DataRole::JaNeural.as_str(), "ja_neural");
        assert_eq!(DataRole::JaTokenizer.as_str(), "ja_tokenizer");
        assert_eq!(DataRole::ZhSelect.as_str(), "zh_select");
        assert_eq!(
            serde_json::to_string(&DataRole::ZhSelect).unwrap(),
            "\"zh_select\""
        );
    }

    #[test]
    fn generated_manifest_records_real_size_and_hash() {
        let dir = temp_dir("generate");
        for spec in CATALOG.iter().filter(|spec| spec.shipped) {
            write(&dir, spec.candidates[0], b"data");
        }
        let manifest = DataManifest::generate(&dir, "2026-11-20", "2026-11-20T00:00:00Z").unwrap();
        assert_eq!(manifest.schema, SCHEMA);
        assert_eq!(manifest.version, "2026-11-20");
        // 只列出存在的隨附檔；現在 zh-select 的四個檔也都是隨附檔。
        assert_eq!(
            manifest.files.len(),
            CATALOG.iter().filter(|spec| spec.shipped).count()
        );
        let lexicon = manifest
            .files
            .iter()
            .find(|file| file.role == DataRole::ZhLexicon)
            .unwrap();
        assert_eq!(lexicon.path, "build-zh/lexicon.bin");
        assert_eq!(lexicon.bytes, 4);
        assert_eq!(
            lexicon.sha256,
            "3a6eb0790f39ac87c94f3856b2dd2c5d110e6811602261a9a923d3bb23adc8b7"
        );
        assert!(lexicon.license.contains(&"L-13".to_string()));
        assert!(lexicon.shipped);
        let tokenizer = manifest
            .files
            .iter()
            .find(|file| file.role == DataRole::JaTokenizer)
            .expect("manifest 要收錄 Jinen tokenizer");
        assert_eq!(
            tokenizer.path,
            "models/quant/jinen-v2-xsmall/tokenizer.json"
        );
        assert_eq!(tokenizer.license, vec!["L-21".to_string()]);
        assert!(tokenizer.shipped);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_zh_manifest_is_cataloged() {
        let dir = temp_dir("build-zh-meta");
        for spec in CATALOG.iter().filter(|spec| spec.shipped) {
            write(&dir, spec.candidates[0], b"data");
        }
        let manifest = DataManifest::generate(&dir, "2026-11-20", "2026-11-20T00:00:00Z").unwrap();
        let zh: Vec<_> = manifest
            .files
            .iter()
            .filter(|file| file.role == DataRole::ZhLexicon)
            .collect();
        assert_eq!(
            zh.len(),
            2,
            "注音資料包要一次收錄 lexicon.bin 與 manifest.json"
        );
        for path in ["build-zh/lexicon.bin", "build-zh/manifest.json"] {
            let file = zh
                .iter()
                .find(|file| file.path == path)
                .unwrap_or_else(|| panic!("manifest 缺少 {path}"));
            assert_eq!(
                file.license.iter().map(String::as_str).collect::<Vec<_>>(),
                ZH_LICENSES
            );
            assert!(file.shipped);
        }
        // Bundle::open 也要讀 lm.bin，雖然它的角色是 ZhLm；產品只帶 bigram。
        assert!(
            manifest
                .files
                .iter()
                .any(|file| file.path == "build-zh-bigram/lm.bin" && file.shipped),
            "注音資料包要收錄 build-zh-bigram/lm.bin"
        );
        assert!(
            !manifest
                .files
                .iter()
                .any(|file| file.path == "build-zh/lm.bin"),
            "unigram-only 的 build-zh/lm.bin 已退役"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn english_pack_metadata_is_cataloged() {
        let dir = temp_dir("english-meta");
        for spec in CATALOG.iter().filter(|spec| spec.shipped) {
            write(&dir, spec.candidates[0], b"data");
        }
        let manifest = DataManifest::generate(&dir, "2026-11-20", "2026-11-20T00:00:00Z").unwrap();
        let english: Vec<_> = manifest
            .files
            .iter()
            .filter(|file| file.role == DataRole::English)
            .collect();
        assert_eq!(english.len(), 3, "英文資料包要一次收錄詞表與兩份中繼檔");
        for path in [
            "english/words.bin",
            "english/manifest.json",
            "english/NOTICE.txt",
        ] {
            let file = english
                .iter()
                .find(|file| file.path == path)
                .unwrap_or_else(|| panic!("manifest 缺少 {path}"));
            assert_eq!(file.license, vec!["L-08".to_string()]);
            assert!(file.shipped);
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_shipped_file_fails_generation() {
        let dir = temp_dir("missing-required");
        write(&dir, "build-zh/lexicon.bin", b"data");
        let err = DataManifest::generate(&dir, "v", "t").unwrap_err();
        assert!(
            err.to_string().contains("ja_lexicon"),
            "錯誤應指出角色：{err}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn full_verify_detects_hash_mismatch_and_startup_ignores_hash() {
        let dir = temp_dir("tamper");
        for spec in CATALOG.iter().filter(|spec| spec.shipped) {
            write(&dir, spec.candidates[0], b"data");
        }
        let manifest = DataManifest::generate(&dir, "2026-11-20", "2026-11-20T00:00:00Z").unwrap();
        assert!(manifest.verify(&dir, VerifyLevel::Full).is_ok());

        write(&dir, "build-zh/lexicon.bin", b"datb");
        let startup = manifest.verify(&dir, VerifyLevel::Startup);
        assert!(startup.is_ok(), "啟動只比大小，內容壞掉仍放行");
        let full = manifest.verify(&dir, VerifyLevel::Full);
        assert!(!full.is_ok());
        assert_eq!(full.errors(), 1);
        assert!(matches!(
            full.checks
                .iter()
                .find(|check| check.path == "build-zh/lexicon.bin")
                .unwrap()
                .status,
            CheckStatus::Hash { .. }
        ));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_download_is_warning_but_missing_shipped_is_error() {
        let dir = temp_dir("missing-split");
        for spec in CATALOG.iter().filter(|spec| spec.shipped) {
            write(&dir, spec.candidates[0], b"data");
        }
        let manifest = DataManifest::generate(&dir, "2026-11-20", "2026-11-20T00:00:00Z").unwrap();
        let report = manifest.verify(&dir, VerifyLevel::Full);
        assert!(report.is_ok());
        assert_eq!(report.warnings(), 0);

        let mut with_download = manifest.clone();
        with_download.files.push(DataFile {
            path: "models/zh-select/model.onnx".to_string(),
            bytes: 10,
            sha256: "0".repeat(64),
            role: DataRole::ZhSelect,
            license: vec!["L-11".to_string()],
            shipped: false,
            url: Some("https://example.invalid/model.onnx".to_string()),
        });
        let report = with_download.verify(&dir, VerifyLevel::Full);
        assert!(report.is_ok(), "下載項缺席只是警告");
        assert_eq!(report.warnings(), 1);

        fs::remove_file(dir.join("build-zh/lexicon.bin")).unwrap();
        let report = manifest.verify(&dir, VerifyLevel::Full);
        assert!(!report.is_ok(), "隨附檔缺席是錯誤");
        assert_eq!(report.errors(), 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn json_round_trip_keeps_fields() {
        let dir = temp_dir("json");
        for spec in CATALOG.iter().filter(|spec| spec.shipped) {
            write(&dir, spec.candidates[0], b"data");
        }
        let manifest = DataManifest::generate(&dir, "2026-11-20", "2026-11-20T00:00:00Z").unwrap();
        let text = manifest.to_json_pretty().unwrap();
        assert!(text.contains("\"schema\": \"0c.data-pack/1\""));
        assert!(text.contains("\"role\": \"zh_lexicon\""));
        assert!(text.ends_with('\n'));
        let parsed = DataManifest::from_slice(text.as_bytes()).unwrap();
        assert_eq!(parsed, manifest);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn schema_mismatch_is_rejected() {
        let json = br#"{
            "schema": "0c.data-pack/2",
            "version": "v",
            "generated_at": "t",
            "files": [{
                "path": "a",
                "bytes": 1,
                "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                "role": "english",
                "license": ["L-08"],
                "shipped": true
            }]
        }"#;
        let err = DataManifest::from_slice(json).unwrap_err();
        assert!(matches!(err, ManifestError::Schema { .. }));
    }

    #[test]
    fn unsafe_paths_and_bad_hashes_are_rejected() {
        let base = br#"{
            "schema": "0c.data-pack/1",
            "version": "v",
            "generated_at": "t",
            "files": [{
                "path": "PATH",
                "bytes": 1,
                "sha256": "HASH",
                "role": "english",
                "license": ["L-08"],
                "shipped": true
            }]
        }"#;
        let good_hash = "0".repeat(64);
        let unsafe_path = String::from_utf8(base.to_vec())
            .unwrap()
            .replace("PATH", "../escape")
            .replace("HASH", &good_hash);
        assert!(matches!(
            DataManifest::from_slice(unsafe_path.as_bytes()).unwrap_err(),
            ManifestError::UnsafePath(_)
        ));

        let bad_hash = String::from_utf8(base.to_vec())
            .unwrap()
            .replace("PATH", "english/words.bin")
            .replace("HASH", "zz");
        assert!(matches!(
            DataManifest::from_slice(bad_hash.as_bytes()).unwrap_err(),
            ManifestError::BadHash { .. }
        ));
    }

    #[test]
    fn utc_format_is_stable() {
        assert_eq!(rfc3339_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_from_unix(1_700_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn real_data_root_verifies() {
        let Some(root) = jc_testdata::root() else {
            eprintln!("缺少測試資料：JC_TESTDATA");
            return;
        };
        let manifest = DataManifest::generate_now(&root).unwrap();
        assert_eq!(manifest.files.len(), CATALOG.len());
        assert_eq!(
            manifest.shipped_files().count(),
            CATALOG.iter().filter(|spec| spec.shipped).count()
        );
        assert_eq!(manifest.download_files().count(), 0, "不再有下載項");
        let gguf = manifest
            .shipped_files()
            .find(|file| file.path == "models/zh-select/model.q8_0.gguf")
            .expect("GGUF 要隨 MSI 散布");
        assert_eq!(gguf.role, DataRole::ZhSelect);
        assert_eq!(gguf.license, vec!["L-11".to_string()]);
        assert!(gguf.bytes > 30 * 1024 * 1024);
        let report = manifest.verify(&root, VerifyLevel::Full);
        assert!(report.is_ok(), "{}", report.summary());
        assert_eq!(report.warnings(), 0);
    }
}
