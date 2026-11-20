//! `mixed-split`：研究用的拉丁字串雙語言切分與逐鍵延遲量測。
//!
//! 這個子命令只印出切分結果，不改任何設定。真實轉換仍走 [`Router`]，並把
//! `mixed_latin_split` 打開，因此輸出可以對照 `eval mixed` 的預設關閉行為。

use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use jc_core::mixed::Router;
use jc_core::mixed_latin::{self, LexiconAdapter, Options, Piece, SplitLang};
use jc_core::Mode;

use crate::cli::CliError;
use crate::data;

/// 執行 `mixed-split`。
pub fn run(
    keys: &[String],
    testdata: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    incremental: bool,
    repeat: usize,
    json: bool,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    let dir = data::testdata_dir(testdata)?;
    let root = data::data_root(&dir);
    let resources = match data_dir {
        Some(path) => data::load_zh_bin_resources(&path)?,
        None => {
            let bin = root.join("build-zh");
            if bin.join("manifest.json").is_file() {
                data::load_zh_bin_resources(&bin)?
            } else {
                data::load_zh_resources(&dir.join("golden/zh/data"))?
            }
        }
    };
    let ja = data::load_ja_decoder(&root)
        .ok()
        .map(|decoder| decoder.with_prefer_hiragana(true));
    let english = jc_core::mixed::English::load_default();
    let config = jc_core::mixed::Config {
        mixed_latin_split: true,
        ..jc_core::mixed::Config::default()
    };
    let mut router = Router::new()
        .with_config(config)
        .with_zh(&resources.lexicon, &resources.ctx)
        .with_english(&english);
    if let Some(decoder) = &ja {
        router = router.with_ja(decoder);
    }

    let opts = Options::default();
    let mut reports = Vec::new();
    for key in keys {
        let split = mixed_latin::split(
            key,
            &opts,
            &LexiconAdapter {
                zh: Some(&resources.lexicon),
                english: Some(&english),
            },
        );
        let conversion = router
            .convert(key)
            .map_err(|err| CliError::Eval(format!("轉換 {key:?} 失敗：{err}")))?;
        let latency = if incremental {
            Some(measure(key, &resources, &english, &opts, repeat))
        } else {
            None
        };
        reports.push(Report {
            input: key.clone(),
            surface: conversion.surface.clone(),
            split,
            latency,
            segments: conversion
                .segments
                .iter()
                .map(|segment| {
                    (
                        segment.mode,
                        segment.script,
                        key[segment.input_start..segment.input_end].to_string(),
                        segment.surface.clone(),
                    )
                })
                .collect(),
        });
    }

    if json {
        let cases: Vec<serde_json::Value> = reports.iter().map(report_json).collect();
        let value = serde_json::json!({ "suite": "mixed-split", "cases": cases });
        writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&value)
                .map_err(|err| CliError::Eval(format!("輸出 JSON 失敗：{err}")))?
        )?;
        return Ok(());
    }

    for report in &reports {
        writeln!(out, "輸入：{:?}", report.input)?;
        writeln!(out, "輸出：{:?}", report.surface)?;
        match &report.split {
            Some(split) => {
                writeln!(out, "切分（score {}）：", split.score)?;
                for piece in &split.pieces {
                    writeln!(
                        out,
                        "  {:>2} {:>3}..{:<3} {:?} → {:?}",
                        piece.lang.as_str(),
                        piece.start,
                        piece.end,
                        piece.text(&report.input),
                        piece.reading
                    )?;
                }
            }
            None => writeln!(out, "切分：無法完整解析")?,
        }
        writeln!(out, "路由片段：")?;
        for (mode, script, input, surface) in &report.segments {
            writeln!(out, "  {:?} {:?} {:?} → {:?}", mode, script, input, surface)?;
        }
        if let Some(latency) = &report.latency {
            writeln!(
                out,
                "逐鍵延遲（{} 次/鍵）：p50 {:.3} ms、p95 {:.3} ms、max {:.3} ms",
                latency.samples / latency.keys.max(1),
                latency.p50_ms,
                latency.p95_ms,
                latency.max_ms
            )?;
        }
        writeln!(out)?;
    }
    Ok(())
}

/// 一個輸入的完整報告。
struct Report {
    input: String,
    surface: String,
    split: Option<mixed_latin::Split>,
    latency: Option<Latency>,
    segments: Vec<(Mode, jc_core::segment::Script, String, String)>,
}

impl Report {
    fn languages(&self) -> Vec<&'static str> {
        self.split
            .as_ref()
            .map(|split| {
                split
                    .pieces
                    .iter()
                    .map(|piece| piece.lang.as_str())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// 逐鍵延遲統計，單位是毫秒。
struct Latency {
    keys: usize,
    samples: usize,
    p50_ms: f64,
    p95_ms: f64,
    max_ms: f64,
}

fn report_json(report: &Report) -> serde_json::Value {
    let pieces: Vec<serde_json::Value> = report
        .split
        .as_ref()
        .map(|split| {
            split
                .pieces
                .iter()
                .map(|piece: &Piece| {
                    serde_json::json!({
                        "start": piece.start,
                        "end": piece.end,
                        "lang": piece.lang.as_str(),
                        "reading": piece.reading,
                        "text": piece.text(&report.input),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let segments: Vec<serde_json::Value> = report
        .segments
        .iter()
        .map(|(mode, script, input, surface)| {
            serde_json::json!({
                "mode": mode_name(*mode),
                "script": script.as_str(),
                "input": input,
                "surface": surface,
            })
        })
        .collect();
    let latency = report.latency.as_ref().map(|latency| {
        serde_json::json!({
            "keys": latency.keys,
            "samples": latency.samples,
            "p50_ms": latency.p50_ms,
            "p95_ms": latency.p95_ms,
            "max_ms": latency.max_ms,
        })
    });
    serde_json::json!({
        "input": report.input,
        "surface": report.surface,
        "score": report.split.as_ref().map(|split| split.score),
        "languages": report.languages(),
        "pieces": pieces,
        "segments": segments,
        "latency": latency,
    })
}

fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Zh => "zh",
        Mode::Ja => "ja",
        Mode::En => "en",
        Mode::Mixed => "mixed",
    }
}

/// 量測每個前綴的逐鍵延遲。
///
/// 只量切分器本身（增量 DP），不含注音與日文解碼；這是「每鍵重算尾巴」
/// 的實際成本。每個前綴重複 `repeat` 次，取所有樣本的分位數。
fn measure(
    key: &str,
    resources: &crate::data::ZhResources,
    english: &jc_core::mixed::English,
    opts: &Options,
    repeat: usize,
) -> Latency {
    let adapter = LexiconAdapter {
        zh: Some(&resources.lexicon),
        english: Some(english),
    };
    let mut samples = Vec::new();
    let mut keys = 0usize;
    let mut splitter = mixed_latin::LatinSplitter::new(*opts);
    for (index, _) in key.char_indices().skip(1) {
        let prefix = &key[..index];
        keys += 1;
        for _ in 0..repeat {
            let start = Instant::now();
            splitter.set_text(prefix, &adapter);
            let _ = splitter.preview();
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    if samples.is_empty() {
        return Latency {
            keys,
            samples: 0,
            p50_ms: 0.0,
            p95_ms: 0.0,
            max_ms: 0.0,
        };
    }
    samples.sort_by(f64::total_cmp);
    let percentile = |p: f64| {
        let index = ((samples.len() as f64 - 1.0) * p).round() as usize;
        samples[index.min(samples.len() - 1)]
    };
    Latency {
        keys,
        samples: samples.len(),
        p50_ms: percentile(0.50),
        p95_ms: percentile(0.95),
        max_ms: *samples.last().unwrap_or(&0.0),
    }
}

/// 語言名稱；測試與 JSON 共用。
pub fn lang_name(lang: SplitLang) -> &'static str {
    lang.as_str()
}
