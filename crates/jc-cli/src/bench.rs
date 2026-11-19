//! `bench` 子命令：對檔案中每行讀音重複轉換並統計延遲。
//!
//! 只量轉換本身，不含資料載入；每次都取 5-best，與 `convert` 相同。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use jc_eval::{percentile, Converter};
use serde_json::json;

use crate::cli::CliError;
use crate::data;

/// 執行 bench。
pub fn run(
    testdata: Option<PathBuf>,
    file: &Path,
    repeat: usize,
    json: bool,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    let dir = data::testdata_dir(testdata)?;
    let text = std::fs::read_to_string(file)
        .map_err(|err| CliError::data(format!("讀取 {} 失敗：{err}", file.display())))?;
    let readings: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    if readings.is_empty() {
        return Err(CliError::data(format!("{} 沒有讀音", file.display())));
    }
    let converter = crate::converter::load_auto(&dir)?;

    let mut durations = Vec::with_capacity(readings.len() * repeat);
    let mut rows = Vec::with_capacity(readings.len());
    for reading in &readings {
        let started = Instant::now();
        let mut best = String::new();
        for _ in 0..repeat.max(1) {
            let candidates = converter.convert_segmented(reading, 5);
            if let Some(first) = candidates.first() {
                best = first.surface.clone();
            }
        }
        let elapsed = started.elapsed().as_secs_f64() * 1000.0 / repeat.max(1) as f64;
        durations.push(elapsed);
        rows.push((*reading, best, elapsed));
    }

    let p50 = percentile(&durations, 50.0);
    let p95 = percentile(&durations, 95.0);
    let mean = durations.iter().sum::<f64>() / durations.len() as f64;
    if json {
        let value = json!({
            "file": file.display().to_string(),
            "lines": readings.len(),
            "repeat": repeat,
            "mean_ms": mean,
            "p50_ms": p50,
            "p95_ms": p95,
            "rows": rows
                .iter()
                .map(|(reading, surface, ms)| json!({
                    "reading": reading,
                    "surface": surface,
                    "ms": ms,
                }))
                .collect::<Vec<_>>(),
        });
        writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&value)
                .map_err(|err| CliError::eval(format!("輸出 JSON 失敗：{err}")))?
        )?;
        return Ok(());
    }
    writeln!(out, "{:<24} {:>10}  surface", "reading", "ms")?;
    for (reading, surface, ms) in &rows {
        writeln!(out, "{reading:<24} {ms:>10.3}  {surface}")?;
    }
    writeln!(
        out,
        "{} 行，每行 {} 次：mean {mean:.3} ms，p50 {p50:.3} ms，p95 {p95:.3} ms",
        readings.len(),
        repeat
    )?;
    Ok(())
}
