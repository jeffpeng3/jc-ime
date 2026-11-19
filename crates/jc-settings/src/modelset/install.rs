//! 模型包的安裝與使用中狀態（`active.json`）。
//!
//! 對照前身 `internal/modelsets/install.go`。模型檔一律由呼叫端提供，不從網路
//! 下載；repo 不保存 `.onnx`。

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::manifest::{
    verify_file, ArtifactFile, Dictionary, Lock, ModelSetError, Options, Result, FORMAT_INT8,
};

/// 安裝後 `active.json` 的格式。
pub const SELECTION_SCHEMA: &str = "0c.model-package.selection/1";

/// `active.json` 的內容：目前模型包的版本與實際安裝的檔案。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Selection {
    /// 格式版本。
    #[serde(default)]
    pub schema: String,
    /// 模型包版本。
    #[serde(default)]
    pub version: String,
    /// 量化格式。
    #[serde(default)]
    pub format: String,
    /// 是否為 placeholder。
    #[serde(default)]
    pub placeholder: bool,
    /// 品質標記。
    #[serde(default)]
    pub quality: String,
    /// 綁定的字典包。
    #[serde(default)]
    pub dictionary: Dictionary,
    /// 使用中的模型檔。
    #[serde(default)]
    pub artifacts: Vec<SelectionArtifact>,
    /// 安裝在權重旁的非權重檔。
    #[serde(default)]
    pub assets: Vec<SelectionAsset>,
}

/// 已安裝的非權重檔。路徑相對於模型根目錄。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionAsset {
    /// 所屬 artifact。
    pub artifact_id: String,
    /// 邏輯名稱。
    pub name: String,
    /// 相對路徑。
    pub path: String,
    /// 檔案大小。
    pub bytes: i64,
    /// 檔案 SHA-256。
    pub sha256: String,
}

/// 使用中的模型檔。路徑相對於模型根目錄。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionArtifact {
    /// artifact 代號。
    pub artifact_id: String,
    /// 模型種類。
    pub kind: String,
    /// artifact variant。
    pub variant: String,
    /// 量化格式。
    pub format: String,
    /// 相對路徑。
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

/// 安裝模型包的參數。
#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    /// 已下載／匯出的模型根目錄（含 manifest 與 ONNX）。
    pub source_root: PathBuf,
    /// 安裝根目錄，`active.json` 會寫在這裡。
    pub dest_root: PathBuf,
    /// 量化格式；空字串表示 `int8`。
    pub format: String,
    /// 允許安裝 placeholder 模型。
    pub allow_placeholders: bool,
    /// 正式發行的嚴格驗證。
    pub release: bool,
    /// 已安裝字典包的 manifest SHA-256。
    pub installed_dictionary_sha256: String,
    /// 已安裝字典包的代號。
    pub installed_dictionary_pack_id: String,
    /// 覆寫既有的模型目錄。
    pub force: bool,
}

/// 安裝根目錄的 `active.json` 路徑。
pub fn selection_path(root: &Path) -> PathBuf {
    root.join("active.json")
}

/// 讀取使用中的模型包；檔案不存在時回傳 [`ModelSetError::Io`]。
pub fn read_selection(root: &Path) -> Result<Selection> {
    let path = selection_path(root);
    let bytes = fs::read(&path)?;
    let selection: Selection =
        serde_json::from_slice(&bytes).map_err(|err| ModelSetError::Selection(err.to_string()))?;
    if selection.schema != SELECTION_SCHEMA
        || selection.version.is_empty()
        || selection.artifacts.is_empty()
    {
        return Err(ModelSetError::Selection("active.json 格式不符".to_string()));
    }
    Ok(selection)
}

/// 驗證並安裝唯一的模型包。
///
/// 只放入 lock 列的模型檔；不清除其他版本或語言的目錄。回傳的 [`Selection`]
/// 已寫入 `<dest_root>/active.json`。
pub fn install(lock: &Lock, opts: &InstallOptions) -> Result<Selection> {
    let package = lock
        .model_package
        .as_ref()
        .ok_or_else(|| ModelSetError::Schema("缺少 model_package".to_string()))?;
    let format = if opts.format.is_empty() {
        FORMAT_INT8
    } else {
        opts.format.as_str()
    };
    package.validate(&Options {
        allow_placeholders: opts.allow_placeholders,
        release: opts.release,
        installed_dictionary_sha256: opts.installed_dictionary_sha256.clone(),
        installed_dictionary_pack_id: opts.installed_dictionary_pack_id.clone(),
        artifact_root: opts.source_root.clone(),
        format: format.to_string(),
        verify_files: true,
    })?;
    if opts.source_root.as_os_str().is_empty() || opts.dest_root.as_os_str().is_empty() {
        return Err(ModelSetError::Selection("需要來源與目的目錄".to_string()));
    }
    fs::create_dir_all(&opts.dest_root)?;
    let stage = create_stage(&opts.dest_root)?;
    let result = install_into(package, opts, format, &stage);
    let _ = fs::remove_dir_all(&stage);
    result
}

fn install_into(
    package: &super::manifest::Package,
    opts: &InstallOptions,
    format: &str,
    stage: &Path,
) -> Result<Selection> {
    let mut selection = Selection {
        schema: SELECTION_SCHEMA.to_string(),
        version: package.version.clone(),
        format: format.to_string(),
        placeholder: package.placeholder,
        quality: package.quality.clone(),
        dictionary: package.dictionary.clone(),
        artifacts: Vec::with_capacity(package.artifacts.len()),
        assets: Vec::new(),
    };
    for artifact in &package.artifacts {
        let file = artifact.files.get(format).ok_or_else(|| {
            ModelSetError::Schema(format!("{} 沒有 {format} 檔案", artifact.artifact_id))
        })?;
        copy_file(
            &opts.source_root.join(&file.path),
            &stage.join(&file.path),
            file.bytes,
        )?;
        selection.artifacts.push(SelectionArtifact {
            artifact_id: artifact.artifact_id.clone(),
            kind: artifact.kind.clone(),
            variant: artifact.variant.clone(),
            format: format.to_string(),
            path: file.path.clone(),
            bytes: file.bytes,
            sha256: file.sha256.clone(),
            placeholder: artifact.placeholder,
            quality: artifact.quality.clone(),
        });
        for asset in &artifact.assets {
            copy_file(
                &opts.source_root.join(&asset.path),
                &stage.join(&asset.path),
                asset.bytes,
            )?;
            selection.assets.push(SelectionAsset {
                artifact_id: artifact.artifact_id.clone(),
                name: asset.name.clone(),
                path: asset.path.clone(),
                bytes: asset.bytes,
                sha256: asset.sha256.clone(),
            });
        }
    }

    let mut moved = std::collections::BTreeSet::new();
    for artifact in &package.artifacts {
        let mut paths = Vec::new();
        if let Some(file) = artifact.files.get(format) {
            paths.push(file.path.clone());
        }
        paths.extend(artifact.assets.iter().map(|asset| asset.path.clone()));
        for path in paths {
            let top = top_dir(&path);
            if top.is_empty() || !moved.insert(top.clone()) {
                continue;
            }
            let target = opts.dest_root.join(&top);
            if target.exists() && !opts.force {
                return Err(ModelSetError::Selection(format!(
                    "{} 已存在（重裝請加 force）",
                    target.display()
                )));
            }
            if target.exists() {
                fs::remove_dir_all(&target)?;
            }
            fs::rename(stage.join(&top), &target)?;
        }
    }
    crate::atomic_file::write_json_atomic(&selection_path(&opts.dest_root), &selection)?;
    // 重新核對安裝後的大小與雜湊，避免複製失敗留下看似可用的檔案。
    for artifact in &package.artifacts {
        if let Some(file) = artifact.files.get(format) {
            verify_file(&opts.dest_root, file)?;
        }
        for asset in &artifact.assets {
            verify_file(
                &opts.dest_root,
                &ArtifactFile {
                    path: asset.path.clone(),
                    bytes: asset.bytes,
                    sha256: asset.sha256.clone(),
                    quantization: String::new(),
                },
            )?;
        }
    }
    Ok(selection)
}

/// 驗證 `active.json` 指向的模型包仍與目前 lock、字典包及實際檔案一致。
pub fn selection_result(
    lock: &Lock,
    selection: &Selection,
    root: &Path,
    installed_dictionary_sha256: &str,
    allow_placeholders: bool,
) -> Result<super::manifest::ValidationResult> {
    let package = lock
        .model_package
        .as_ref()
        .ok_or_else(|| ModelSetError::Schema("缺少 model_package".to_string()))?;
    if !selection.version.is_empty() && selection.version != package.version {
        return Err(ModelSetError::Selection(format!(
            "active.json 的版本是 {:?}，清單是 {:?}",
            selection.version, package.version
        )));
    }
    if !selection.dictionary.manifest_sha256.is_empty()
        && selection.dictionary.manifest_sha256 != package.dictionary.manifest_sha256
    {
        return Err(ModelSetError::DictionaryMismatch(
            "active.json 綁定的字典與清單不同".to_string(),
        ));
    }
    package.validate(&Options {
        allow_placeholders,
        release: false,
        installed_dictionary_sha256: installed_dictionary_sha256.to_string(),
        installed_dictionary_pack_id: String::new(),
        artifact_root: root.to_path_buf(),
        format: selection.format.clone(),
        verify_files: true,
    })
}

fn create_stage(root: &Path) -> Result<PathBuf> {
    for attempt in 0..1000_u32 {
        let path = root.join(format!(".model-package-{}-{attempt}", std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err.into()),
        }
    }
    Err(ModelSetError::Selection("無法建立暫存目錄".to_string()))
}

fn top_dir(path: &str) -> String {
    match path.split_once('/') {
        Some((top, _)) if !top.is_empty() => top.to_string(),
        _ => path.to_string(),
    }
}

fn copy_file(from: &Path, to: &Path, want: i64) -> Result<()> {
    if let Some(dir) = to.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut source = fs::File::open(from)?;
    let mut target = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut copied = 0_i64;
    let copied_result = (|| -> io::Result<()> {
        loop {
            let read = source.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            target.write_all(&buffer[..read])?;
            copied += read as i64;
        }
        target.sync_all()
    })();
    if let Err(err) = copied_result {
        let _ = fs::remove_file(to);
        return Err(err.into());
    }
    drop(target);
    if want > 0 && copied != want {
        let _ = fs::remove_file(to);
        return Err(ModelSetError::FileMismatch(format!(
            "{} 複製 {copied} bytes，清單記 {want}",
            from.display()
        )));
    }
    Ok(())
}
