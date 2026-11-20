//! `jc-cli data`：資料根目錄 manifest 的產生與驗證。
//!
//! 只做檔案掃描、雜湊與比對，不搬動資料、不改寫 `settings.json`。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use jc_data::manifest::{CheckStatus, DataManifest, FileCheck, VerifyLevel, MANIFEST_FILE};

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
    // 人讀輸出跳過缺席的下載項：`shipped: false` 本來就不隨產品散布，缺席是
    // 正常狀態。--json 仍輸出完整 report，引擎啟動日誌也仍讀 report.checks
    // 逐項提示「相關功能維持停用」，不能改 DataManifest::verify 的回傳。
    let shown: Vec<&FileCheck> = report
        .checks
        .iter()
        .filter(|check| !is_absent_download(check))
        .collect();
    let shown_summary = format!(
        "通過 {}/{}，錯誤 {}，警告 {}",
        shown
            .iter()
            .filter(|check| check.status == CheckStatus::Ok)
            .count(),
        shown.len(),
        shown.iter().filter(|check| check.is_error()).count(),
        shown.iter().filter(|check| check.is_warning()).count(),
    );
    if json {
        let text =
            serde_json::to_string_pretty(&report).map_err(|err| CliError::data(err.to_string()))?;
        writeln!(stream, "{text}")?;
    } else {
        for check in &shown {
            writeln!(stream, "{}", describe(check))?;
        }
        writeln!(stream, "驗證 {}：{}", path.display(), shown_summary)?;
    }
    if !report.is_ok() {
        return Err(CliError::data(format!("資料驗證失敗：{}", shown_summary)));
    }
    Ok(())
}

/// 缺席的下載項不進人讀輸出：不是錯誤，也不是需要使用者處理的警告。
fn is_absent_download(check: &FileCheck) -> bool {
    check.status == CheckStatus::Missing && !check.shipped
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

#[cfg(test)]
mod tests {
    use super::*;
    use jc_data::manifest::{DataFile, DataRole, SCHEMA};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jc-cli-verify-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, rel: &str, bytes: &[u8]) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn fixture(name: &str) -> PathBuf {
        let dir = temp_dir(name);
        write(&dir, "build-zh/lexicon.bin", b"data");
        let (bytes, sha256) =
            jc_data::manifest::sha256_file(dir.join("build-zh/lexicon.bin")).unwrap();
        let manifest = DataManifest {
            schema: SCHEMA.to_string(),
            version: "test".to_string(),
            generated_at: "2026-11-20T00:00:00Z".to_string(),
            files: vec![
                DataFile {
                    path: "build-zh/lexicon.bin".to_string(),
                    bytes,
                    sha256,
                    role: DataRole::ZhLexicon,
                    license: vec!["L-13".to_string()],
                    shipped: true,
                    url: None,
                },
                DataFile {
                    path: "models/zh-select/model.onnx".to_string(),
                    bytes: 10,
                    sha256: "0".repeat(64),
                    role: DataRole::ZhSelect,
                    license: vec!["L-11".to_string()],
                    shipped: false,
                    url: Some("https://example.invalid/model.onnx".to_string()),
                },
            ],
        };
        write(
            &dir,
            MANIFEST_FILE,
            manifest.to_json_pretty().unwrap().as_bytes(),
        );
        dir
    }

    #[test]
    fn human_output_hides_absent_downloads() {
        let dir = fixture("human");
        let mut out = Vec::new();
        verify(&dir, None, false, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("通過      build-zh/lexicon.bin（zh_lexicon）"));
        assert!(!text.contains("缺少"));
        assert!(text.contains("通過 1/1，錯誤 0，警告 0"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn json_output_keeps_full_report() {
        let dir = fixture("json");
        let mut out = Vec::new();
        verify(&dir, None, true, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let report: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(report["checks"].as_array().unwrap().len(), 2);
        assert_eq!(
            report["checks"][1]["status"]["status"].as_str(),
            Some("missing")
        );
        fs::remove_dir_all(&dir).ok();
    }
}
