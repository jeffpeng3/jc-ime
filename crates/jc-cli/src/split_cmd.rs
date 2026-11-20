//! `mixed-split`：混合輸入的唯一分詞結果與逐鍵延遲量測。
//!
//! 輸出只有一份結果，來源是 [`Router::convert`]。`--steps` 另外印出
//! tokenize、每個拉丁 run 的 DP 與後處理三個階段，方便對照規則。

use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use jc_core::mixed::{self, Router};
use jc_core::mixed_latin::{self, LexiconAdapter, Options};
use jc_core::Mode;

use crate::cli::CliError;
use crate::data;

/// 執行 `mixed-split`。
#[allow(clippy::too_many_arguments)]
pub fn run(
    keys: &[String],
    testdata: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    incremental: bool,
    repeat: usize,
    steps: bool,
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
    let config = jc_core::mixed::Config::default();
    let mut router = Router::new()
        .with_config(config.clone())
        .with_zh(&resources.lexicon, &resources.ctx)
        .with_english(&english);
    if let Some(decoder) = &ja {
        router = router.with_ja(decoder);
    }
    let adapter = LexiconAdapter {
        zh: Some(&resources.lexicon),
        english: Some(&english),
        config: config.clone(),
    };
    let lookup: &dyn mixed::Lookup = &english;

    let opts = Options::default();
    let mut reports = Vec::new();
    for key in keys {
        let conversion = router
            .convert(key)
            .map_err(|err| CliError::Eval(format!("轉換 {key:?} 失敗：{err}")))?;
        let trace = steps.then(|| mixed::explain(key, lookup, &config, Some(&adapter)));
        let latency = if incremental {
            Some(measure(key, &resources, &english, &config, &opts, repeat))
        } else {
            None
        };
        reports.push(Report {
            input: key.clone(),
            surface: conversion.surface.clone(),
            segments: conversion
                .segments
                .iter()
                .map(|segment| SegmentView {
                    mode: segment.mode,
                    script: segment.script,
                    input: key[segment.input_start..segment.input_end].to_string(),
                    surface: segment.surface.clone(),
                })
                .collect(),
            trace,
            latency,
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
        if let Some(trace) = &report.trace {
            write_steps(out, trace)?;
        }
        writeln!(out, "片段：")?;
        for segment in &report.segments {
            writeln!(
                out,
                "  {} {} {:?} → {:?}",
                mode_name(segment.mode),
                segment.script.as_str(),
                segment.input,
                segment.surface
            )?;
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

fn write_steps(out: &mut dyn Write, trace: &mixed::Explain) -> Result<(), CliError> {
    writeln!(out, "tokenize（{} 個 atom）：", trace.atoms.len())?;
    for atom in &trace.atoms {
        writeln!(
            out,
            "  {:<7} {:<8} {:>3}..{:<3} {:?}",
            atom.kind,
            atom.mode.map(mode_name).unwrap_or("-"),
            atom.start,
            atom.end,
            atom.text
        )?;
    }
    writeln!(out, "DP（{} 個 run）：", trace.runs.len())?;
    for run in &trace.runs {
        writeln!(out, "  run {:?}", run.text)?;
        for piece in &run.pieces {
            writeln!(
                out,
                "    {:>2} {:>3}..{:<3} {:?} → {:?}",
                piece.lang, piece.start, piece.end, piece.text, piece.reading
            )?;
        }
    }
    for (index, title, atoms) in [
        ("1/3", "片語合併", &trace.after_group_words),
        ("2/3", "語言判定", &trace.after_resolve),
    ] {
        writeln!(out, "後處理 {index} {title}：")?;
        for atom in atoms {
            writeln!(
                out,
                "  {:<7} {:<8} {:>3}..{:<3} {:?}",
                atom.kind,
                atom.mode.map(mode_name).unwrap_or("-"),
                atom.start,
                atom.end,
                atom.text
            )?;
        }
    }
    writeln!(out, "後處理 3/3 片段合併：")?;
    for plan in &trace.after_merge {
        writeln!(
            out,
            "  {} {} {:>3}..{:<3} {:?}",
            mode_name(plan.mode),
            plan.script.as_str(),
            plan.start,
            plan.end,
            plan.text
        )?;
    }
    Ok(())
}

/// 一個輸入的報告。
struct Report {
    input: String,
    surface: String,
    segments: Vec<SegmentView>,
    trace: Option<mixed::Explain>,
    latency: Option<Latency>,
}

/// 一個輸出片段。
struct SegmentView {
    mode: Mode,
    script: jc_core::segment::Script,
    input: String,
    surface: String,
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
    let segments: Vec<serde_json::Value> = report
        .segments
        .iter()
        .map(|segment| {
            serde_json::json!({
                "mode": mode_name(segment.mode),
                "script": segment.script.as_str(),
                "input": segment.input,
                "surface": segment.surface,
            })
        })
        .collect();
    let steps = report.trace.as_ref().map(|trace| {
        serde_json::json!({
            "atoms": trace.atoms.iter().map(atom_json).collect::<Vec<_>>(),
            "runs": trace.runs.iter().map(|run| serde_json::json!({
                "text": run.text,
                "pieces": run.pieces.iter().map(|piece| serde_json::json!({
                    "start": piece.start,
                    "end": piece.end,
                    "lang": piece.lang,
                    "text": piece.text,
                    "reading": piece.reading,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "after_group_words": trace.after_group_words.iter().map(atom_json).collect::<Vec<_>>(),
            "after_resolve": trace.after_resolve.iter().map(atom_json).collect::<Vec<_>>(),
            "after_merge": trace.after_merge.iter().map(|plan| serde_json::json!({
                "start": plan.start,
                "end": plan.end,
                "mode": mode_name(plan.mode),
                "script": plan.script.as_str(),
                "text": plan.text,
            })).collect::<Vec<_>>(),
        })
    });
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
        "segments": segments,
        "steps": steps,
        "latency": latency,
    })
}

fn atom_json(atom: &mixed::AtomView) -> serde_json::Value {
    serde_json::json!({
        "start": atom.start,
        "end": atom.end,
        "kind": atom.kind,
        "mode": atom.mode.map(mode_name),
        "script": atom.script.as_str(),
        "text": atom.text,
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
    config: &jc_core::mixed::Config,
    opts: &Options,
    repeat: usize,
) -> Latency {
    let adapter = LexiconAdapter {
        zh: Some(&resources.lexicon),
        english: Some(english),
        config: config.clone(),
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
