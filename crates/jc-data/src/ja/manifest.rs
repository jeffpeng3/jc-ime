//! `manifest.json` 讀取與配對驗證。
//!
//! manifest 記錄 pack 的數量、大小與兩層雜湊；`verify` 會與已開啟的
//! [`Pack`] 逐項比對，`verify_file` 則重新核對 manifest 檔本身的雜湊。

use std::fs;
use std::path::Path;

use serde::Deserialize;

use super::error::PackError;
use super::pack::{hex, sha256, Pack, FORMAT_VERSION};

/// manifest 支援的 schema。
pub const MANIFEST_SCHEMA: &str = "0c-lexicon-manifest/1";

/// manifest 的 `pack_format` 區塊。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PackFormat {
    /// 格式名稱。
    pub name: String,
    /// 格式版本。
    pub version: i64,
}

/// manifest 宣告的數量。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ManifestCounts {
    /// 詞條數。
    pub entries: i64,
    /// 唯一 surface 數。
    pub surfaces: i64,
    /// POS table 筆數。
    pub pos: i64,
    /// 活用型筆數。
    pub conj_types: i64,
    /// 活用形筆數。
    pub conj_forms: i64,
}

/// manifest 的 `outputs` 區塊。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ManifestOutputs {
    /// pack 檔的宣告。
    pub pack: ManifestPack,
}

/// manifest 對 pack 檔的宣告。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ManifestPack {
    /// 相對檔名。
    pub path: String,
    /// 檔案大小。
    pub bytes: i64,
    /// 整檔 SHA-256。
    pub sha256: String,
    /// payload SHA-256。
    pub payload_sha256: String,
}

/// 已讀取的 manifest。只解碼驗證與配對需要的欄位。
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    /// schema 名稱。
    pub schema: String,
    /// lexicon 名稱。
    pub lexicon: String,
    /// target 名稱。
    pub target: String,
    /// 格式名稱與版本。
    pub pack_format: PackFormat,
    /// 讀音文字系統。
    pub reading_script: String,
    /// 宣告的數量。
    pub counts: ManifestCounts,
    /// 輸出檔宣告。
    pub outputs: ManifestOutputs,
    /// 允許的來源清單。
    #[serde(default)]
    pub allowed_sources: Vec<String>,
    /// manifest 檔本身的 SHA-256，載入時計算。
    #[serde(skip)]
    pub file_sha256: String,
}

impl Manifest {
    /// 讀取 manifest.json，並記錄檔案本身的 SHA-256。
    pub fn load(path: impl AsRef<Path>) -> Result<Manifest, PackError> {
        let path = path.as_ref();
        let data = fs::read(path).map_err(|source| PackError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let mut manifest: Manifest =
            serde_json::from_slice(&data).map_err(|source| PackError::ManifestJson {
                path: path.to_path_buf(),
                source,
            })?;
        manifest.file_sha256 = hex(&sha256(&data));
        Ok(manifest)
    }

    /// 重新讀取 manifest 檔並比對載入時記錄的 SHA-256。
    pub fn verify_file(&self, path: impl AsRef<Path>) -> Result<(), PackError> {
        if self.file_sha256.is_empty() {
            return Err(PackError::ManifestHashMissing);
        }
        let path = path.as_ref();
        let data = fs::read(path).map_err(|source| PackError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let actual = hex(&sha256(&data));
        if !self.file_sha256.eq_ignore_ascii_case(&actual) {
            return Err(PackError::ManifestFileHash {
                manifest: self.file_sha256.clone(),
                actual,
            });
        }
        Ok(())
    }

    /// 檢查 manifest 與已解析的 pack 是否配對。
    pub fn verify(&self, pack: &Pack) -> Result<(), PackError> {
        if self.schema != MANIFEST_SCHEMA {
            return Err(PackError::ManifestSchema {
                actual: self.schema.clone(),
                expected: MANIFEST_SCHEMA,
            });
        }
        if self.pack_format.name != "0c-jadic" {
            return Err(PackError::ManifestFormatName {
                actual: self.pack_format.name.clone(),
                expected: "0c-jadic",
            });
        }
        if self.pack_format.version != FORMAT_VERSION as i64 {
            return Err(PackError::ManifestFormatVersion {
                actual: self.pack_format.version,
                expected: FORMAT_VERSION as i64,
            });
        }
        match self.reading_script.as_str() {
            "hiragana" | "katakana" => {}
            _ => {
                return Err(PackError::ManifestReadingScript {
                    actual: self.reading_script.clone(),
                });
            }
        }

        let header = pack.header();
        let counts = [
            (
                "counts.entries",
                self.counts.entries,
                "entry_count",
                header.entry_count as i64,
            ),
            (
                "counts.surfaces",
                self.counts.surfaces,
                "surface_count",
                header.surface_count as i64,
            ),
            (
                "counts.pos",
                self.counts.pos,
                "pos_count",
                header.pos_count as i64,
            ),
            (
                "counts.conj_types",
                self.counts.conj_types,
                "conj_type_count",
                header.conj_type_count as i64,
            ),
            (
                "counts.conj_forms",
                self.counts.conj_forms,
                "conj_form_count",
                header.conj_form_count as i64,
            ),
        ];
        for (field, manifest, pack_field, actual) in counts {
            if manifest != actual {
                return Err(PackError::ManifestCount {
                    field,
                    manifest,
                    pack_field,
                    pack: actual,
                });
            }
        }

        let output = &self.outputs.pack;
        if output.bytes != pack.bytes() as i64 {
            return Err(PackError::ManifestBytes {
                manifest: output.bytes,
                actual: pack.bytes(),
            });
        }
        if !output.sha256.eq_ignore_ascii_case(&pack.file_sha256()) {
            return Err(PackError::ManifestFileHash {
                manifest: output.sha256.clone(),
                actual: pack.file_sha256(),
            });
        }
        if !output
            .payload_sha256
            .eq_ignore_ascii_case(&pack.payload_sha256())
        {
            return Err(PackError::ManifestPayloadHash {
                manifest: output.payload_sha256.clone(),
                trailer: pack.payload_sha256(),
            });
        }
        Ok(())
    }
}
