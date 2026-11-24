//! `--settings` 與 `--print-settings` 的行程層測試。
//!
//! 不需要題庫：`--allow-empty-data` 搭配 `--print-settings` 只驗證設定合併。

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// 建立含 `settings.json` 的暫存資料夾。
fn settings_dir(name: &str, body: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("jc-engine-wire-{name}-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("settings.json"), body).unwrap();
    dir
}

fn print_settings(args: &[&std::ffi::OsStr]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_jc-engine"))
        .arg("--print-settings")
        .arg("--allow-empty-data")
        .args(args)
        .output()
        .expect("啟動 jc-engine")
}

#[test]
fn print_settings_merges_file_and_cli() {
    let dir = settings_dir(
        "merge",
        br#"{"selection": "statistical", "mixed_input": true, "punctuation": true}"#,
    );
    let path = dir.join("settings.json");
    let output = print_settings(&[
        std::ffi::OsStr::new("--settings"),
        path.as_os_str(),
        std::ffi::OsStr::new("--mode"),
        std::ffi::OsStr::new("en"),
    ]);
    assert!(output.status.success(), "{output:?}");
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("--print-settings 必須輸出 JSON");
    assert_eq!(value["mode"], "mixed", "英文已併入混打");
    assert_eq!(value["selection"], "statistical");
    assert_eq!(value["neural"], false);
    assert_eq!(value["punct_mark"], "，");
    assert_eq!(value["settings_state"], "ok");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn print_settings_without_file_reports_defaults() {
    let output = print_settings(&[
        std::ffi::OsStr::new("--settings"),
        std::ffi::OsStr::new("/nonexistent/jc-settings/settings.json"),
    ]);
    assert!(output.status.success(), "{output:?}");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["settings_state"], "missing");
    assert_eq!(value["mode"], "mixed");
    assert_eq!(value["neural"], true);
    assert_eq!(value["punct_mark"], serde_json::Value::Null);
}

#[test]
fn corrupt_settings_prints_warning_and_defaults() {
    let dir = settings_dir("corrupt", b"{ not json");
    let path = dir.join("settings.json");
    let output = print_settings(&[std::ffi::OsStr::new("--settings"), path.as_os_str()]);
    assert!(
        output.status.success(),
        "壞檔不得讓引擎啟動失敗：{output:?}"
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["settings_state"], "corrupt");
    assert_eq!(value["mode"], "mixed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("使用預設值"),
        "應在 stderr 記一次警告：{stderr}"
    );
    fs::remove_dir_all(&dir).ok();
}
