//! `ja-eval` bin 與 `jc-cli eval ja` 必須對同一個 pack、同一份題庫給出相同數字。
//!
//! 題庫取 `hard-cases` 前 5 題，寫成暫存 JSON 後同時餵給兩個入口。需要
//! v2 pack 與題庫，資料不存在時跳過。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 由 `0c-testdata/ja` 取上層 `0c-testdata` 目錄。
fn testdata_dir(ja_dir: &Path) -> PathBuf {
    ja_dir.parent().unwrap().to_path_buf()
}

/// 把 `hard-cases` 前 5 題寫成暫存題庫，回傳路徑。
fn write_subset(source: &Path) -> PathBuf {
    let raw = std::fs::read_to_string(source).unwrap();
    let mut doc: serde_json::Value = serde_json::from_str(&raw).unwrap();
    doc["name"] = serde_json::Value::String("hard-cases-subset".into());
    doc["count"] = serde_json::Value::from(5);
    doc["items"].as_array_mut().unwrap().truncate(5);

    let path = std::env::temp_dir().join(format!("jc-cli-ja-parity-{}.json", std::process::id()));
    std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    path
}

/// 解析 `Report::to_table` 的資料列：`top1`／`top5` 百分比。
fn parse_table_percent(table: &str) -> (usize, f64, f64) {
    // 表格前面有「題庫：…」與欄位標題，資料列是第一個 n 可解析成整數的列。
    for line in table.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 4 {
            continue;
        }
        let Ok(n) = fields[1].parse::<usize>() else {
            continue;
        };
        let top1: f64 = fields[2].trim_end_matches('%').parse().unwrap();
        let top5: f64 = fields[3].trim_end_matches('%').parse().unwrap();
        return (n, top1, top5);
    }
    panic!("表格沒有資料列：\n{table}");
}

/// 兩個入口對同一 pack、同一 5 題題庫的指標必須一致。
#[test]
fn test_ja_eval_and_cli_eval_ja_agree_on_same_pack() {
    let pack = jc_testdata::require_data!("packs/ja-lexicon-mozc-v2.pack");
    let ja_dir = jc_testdata::require_data!("0c-testdata/ja");
    let corpus_path = jc_testdata::require_data!("0c-testdata/ja/hard_cases.json");
    let testdata = testdata_dir(&ja_dir);
    let subset = write_subset(&corpus_path);

    let bin = Command::new(env!("CARGO_BIN_EXE_ja-eval"))
        .arg("--pack")
        .arg(&pack)
        .arg("--corpus")
        .arg(&subset)
        .arg("--top")
        .arg("5")
        .output()
        .expect("執行 ja-eval 失敗");
    assert!(
        bin.status.success(),
        "ja-eval 失敗：{}",
        String::from_utf8_lossy(&bin.stderr)
    );
    let (bin_n, bin_top1, bin_top5) = parse_table_percent(&String::from_utf8_lossy(&bin.stdout));

    let cli = Command::new(env!("CARGO_BIN_EXE_jc-cli"))
        .arg("eval")
        .arg("ja")
        .arg("--testdata")
        .arg(&testdata)
        .arg("--corpus")
        .arg(&subset)
        .arg("--pack")
        .arg(&pack)
        .arg("--json")
        .output()
        .expect("執行 jc-cli 失敗");
    assert!(
        cli.status.success(),
        "jc-cli 失敗：{}",
        String::from_utf8_lossy(&cli.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&cli.stdout).unwrap();
    let metrics = &report["corpora"]["hard-cases-subset"];
    let cli_n = metrics["n"].as_u64().unwrap() as usize;
    let cli_top1 = metrics["top1_rate"].as_f64().unwrap() * 100.0;
    let cli_top5 = metrics["top5_rate"].as_f64().unwrap() * 100.0;

    let _ = std::fs::remove_file(&subset);

    assert_eq!(bin_n, 5);
    assert_eq!(cli_n, bin_n);
    assert!(
        (cli_top1 - bin_top1).abs() < 1e-6,
        "top1 不一致：ja-eval {bin_top1}，jc-cli {cli_top1}"
    );
    assert!(
        (cli_top5 - bin_top5).abs() < 1e-6,
        "top5 不一致：ja-eval {bin_top5}，jc-cli {cli_top5}"
    );
}

/// 沒給 `--pack` 時，預設解析到資料根目錄的 v2 pack。
#[test]
fn test_default_pack_comes_from_data_root() {
    let pack = jc_testdata::require_data!("packs/ja-lexicon-mozc-v2.pack");
    let _ = jc_testdata::require_data!("0c-testdata/ja");
    let root = jc_testdata::root().expect("測試資料根目錄");
    let resolved = jc_cli::data::resolve_ja_pack(&root, None).unwrap();
    assert_eq!(resolved, pack);
}
