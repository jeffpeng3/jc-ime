//! `jc-cli data`：資料根目錄 manifest 的產生與驗證。
//!
//! 只做檔案掃描、雜湊與比對，不搬動資料、不改寫 `settings.json`。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use jc_data::manifest::{DataManifest, FileCheck, VerifyLevel, MANIFEST_FILE};

use crate::cli::CliError;

fn data_error(err: impl std::fmt::Display) -> CliError {
    CliError::data(err.to_string())
}

/// 產生 `<root>/manifest.json`，或寫到 `out`。
pub fn manifest(
    root: &Path,
    out: Option<&Path>,
    version: Option<&str>,
    stream: &mut dyn Write,
) -> Result<(), CliError> {
    if !root.is_dir() {
        return Err(CliError::data(format!(
            "資料根目錄不存在：{}",
            root.display()
        )));
    }
    let manifest = match version {
        Some(version) => {
            DataManifest::generate(root, version, jc_data::manifest::utc_now_rfc3339())
        }
        None => DataManifest::generate_now(root),
    }
    .map_err(data_error)?;
    let text = manifest.to_json_pretty().map_err(data_error)?;
    let target = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.join(MANIFEST_FILE));
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| {
                CliError::data(format!("建立目錄 {} 失敗：{err}", parent.display()))
            })?;
        }
    }
    fs::write(&target, text)
        .map_err(|err| CliError::data(format!("寫入 {} 失敗：{err}", target.display())))?;

    writeln!(stream, "manifest：{}", target.display())?;
    writeln!(stream, "產生時間：{}", manifest.generated_at)?;
    writeln!(stream, "資料版本：{}", manifest.version)?;
    writeln!(
        stream,
        "檔案：{}（隨附 {}，下載 {}），共 {} 位元組",
        manifest.files.len(),
        manifest.shipped_files().count(),
        manifest.download_files().count(),
        manifest.total_bytes()
    )?;
    Ok(())
}

/// 驗證資料根目錄；預設讀 `<root>/manifest.json`。
pub fn verify(
    root: &Path,
    manifest: Option<&Path>,
    json: bool,
    stream: &mut dyn Write,
) -> Result<(), CliError> {
    if !root.is_dir() {
        return Err(CliError::data(format!(
            "資料根目錄不存在：{}",
            root.display()
        )));
    }
    let path: PathBuf = manifest
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.join(MANIFEST_FILE));
    let manifest = DataManifest::load(&path).map_err(data_error)?;
    let report = manifest.verify(root, VerifyLevel::Full);
    if json {
        let text =
            serde_json::to_string_pretty(&report).map_err(|err| CliError::data(err.to_string()))?;
        writeln!(stream, "{text}")?;
    } else {
        for check in &report.checks {
            writeln!(stream, "{}", describe(check))?;
        }
        writeln!(stream, "驗證 {}：{}", path.display(), report.summary())?;
    }
    if !report.is_ok() {
        return Err(CliError::data(format!(
            "資料驗證失敗：{}",
            report.summary()
        )));
    }
    Ok(())
}

fn describe(check: &FileCheck) -> String {
    match &check.status {
        jc_data::manifest::CheckStatus::Ok => {
            format!("通過      {}（{}）", check.path, check.role.as_str())
        }
        jc_data::manifest::CheckStatus::Missing if check.shipped => format!(
            "缺少      {}（{}，隨附檔，錯誤）",
            check.path,
            check.role.as_str()
        ),
        jc_data::manifest::CheckStatus::Missing => format!(
            "缺少      {}（{}，下載項，警告）",
            check.path,
            check.role.as_str()
        ),
        jc_data::manifest::CheckStatus::Size { expected, actual } => format!(
            "大小不符  {}：manifest {expected} 位元組，實際 {actual} 位元組",
            check.path
        ),
        jc_data::manifest::CheckStatus::Hash { expected, actual } => format!(
            "雜湊不符  {}：manifest {expected}，實際 {actual}",
            check.path
        ),
        jc_data::manifest::CheckStatus::Read { message } => {
            format!("讀取失敗  {}：{message}", check.path)
        }
    }
}
