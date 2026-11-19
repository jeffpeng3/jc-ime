//! candle 與 ONNX reranker 的逐組對照。
//!
//! 讀 benchmark 的 jsonl（每組一個共用上下文、多個候選），用**同一份**
//! token id 與 attention mask 分別跑 candle 與 ORT，比對 logits 與排序，
//! 再量單組推論延遲。
//!
//! ```text
//! cargo run -p jc-cli --release --bin candle-ja-check -- \
//!   --model-dir /mnt/work/jc/jc-ime-data/models/reranker-extra \
//!   --benchmark /mnt/work/0c/data/benchmarks/homophone-extra-v1/test.jsonl \
//!   --groups 200
//! ```
//!
//! ORT 的 fp32 對照模型不放產品目錄（`model.int8.onnx` 才是出貨檔），
//! 預設指到訓練產物；`--ort-fp32` 可覆寫。ORT 需要 `ORT_DYLIB_PATH` 或
//! `--ort-lib`。
//!
//! logits 誤差上限 1e-3、排序必須完全一致；不通過時 exit code 1。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use jc_model::layout::{pack_batch, PackedBatch};
use jc_model::manifest::ModelSpec;
use jc_model::{Backend, Reranker, Tokenizer};

/// 預設模型目錄。
const DEFAULT_MODEL_DIR: &str = "/mnt/work/jc/jc-ime-data/models/reranker-extra";
/// 預設 fp32 ORT 對照模型。
const DEFAULT_ORT_FP32: &str = "/mnt/work/0c/models/trained/reranker-extra/model.onnx";
/// 預設 int8 ORT 模型。
const DEFAULT_ORT_INT8: &str = "model.int8.onnx";
/// 預設 benchmark。
const DEFAULT_BENCHMARK: &str = "/mnt/work/0c/data/benchmarks/homophone-extra-v1/test.jsonl";
/// logits 誤差上限。
const MAX_ABS_DIFF: f32 = 1e-3;

#[derive(Debug, Clone)]
struct Args {
    model_dir: PathBuf,
    benchmark: PathBuf,
    groups: usize,
    latency_groups: usize,
    ort_fp32: PathBuf,
    ort_int8: PathBuf,
    ort_lib: Option<PathBuf>,
    weights: Option<PathBuf>,
    /// 只保留每側前 N 個 context token；0 代表照原樣。
    shrink: usize,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            model_dir: PathBuf::from(DEFAULT_MODEL_DIR),
            benchmark: PathBuf::from(DEFAULT_BENCHMARK),
            groups: 200,
            latency_groups: 50,
            ort_fp32: PathBuf::from(DEFAULT_ORT_FP32),
            ort_int8: PathBuf::from(DEFAULT_ORT_INT8),
            ort_lib: None,
            weights: None,
            shrink: 0,
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("candle-ja-check：{err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool, String> {
    let args = parse(std::env::args().skip(1).collect())?;
    let spec = ModelSpec::default();
    let tokenizer = Tokenizer::load(args.model_dir.join(spec.vocab_file()))
        .map_err(|err| format!("載入 tokenizer 失敗：{err}"))?;

    if let Some(weights) = &args.weights {
        std::env::set_var(jc_model::candle::WEIGHTS_ENV, weights);
    }
    let candle = Reranker::load_with_backend(&args.model_dir, &spec, None, Backend::Candle)
        .map_err(|err| format!("載入 candle 模型失敗：{err}"))?;
    let fp32_spec = ModelSpec {
        model: Some(args.ort_fp32.display().to_string()),
        ..spec.clone()
    };
    let ort_fp32 = Reranker::load_with_backend(
        &args.model_dir,
        &fp32_spec,
        args.ort_lib.as_deref(),
        Backend::Ort,
    )
    .map_err(|err| format!("載入 ORT fp32 模型失敗：{err}"))?;
    let int8_path = if args.ort_int8.is_absolute() {
        args.ort_int8.clone()
    } else {
        args.model_dir.join(&args.ort_int8)
    };
    let int8_spec = ModelSpec {
        model: Some(int8_path.display().to_string()),
        ..spec.clone()
    };
    let ort_int8 = Reranker::load_with_backend(
        &args.model_dir,
        &int8_spec,
        args.ort_lib.as_deref(),
        Backend::Ort,
    )
    .map_err(|err| format!("載入 ORT int8 模型失敗：{err}"))?;

    let cases = load_cases(&args.benchmark, args.groups)?;
    if cases.is_empty() {
        return Err(format!("{} 沒有可用的組", args.benchmark.display()));
    }
    let mut batches = Vec::with_capacity(cases.len());
    for case in &cases {
        let candidates: Vec<&str> = case.candidates.iter().map(String::as_str).collect();
        let batch = pack_batch(
            &tokenizer,
            &spec,
            Some(&case.left),
            Some(&case.right),
            &candidates,
        )
        .map_err(|err| format!("打包 {} 失敗：{err}", case.id))?;
        batches.push(if args.shrink == 0 {
            batch
        } else {
            shrink_context(&batch, args.shrink)
        });
    }

    let mut candle_all = Vec::with_capacity(batches.len());
    let mut fp32_all = Vec::with_capacity(batches.len());
    let mut int8_all = Vec::with_capacity(batches.len());
    for (case, batch) in cases.iter().zip(&batches) {
        candle_all.push(score(&candle, batch, &case.id)?);
        fp32_all.push(score(&ort_fp32, batch, &case.id)?);
        int8_all.push(score(&ort_int8, batch, &case.id)?);
    }

    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "groups {}，candidates {}，seq {}，slot {}",
        cases.len(),
        batches.iter().map(|b| b.rows).sum::<usize>(),
        spec.seq_len(),
        spec.candidate_slot()
    )
    .map_err(io)?;

    let fp32 = compare("candle vs ort-fp32", &candle_all, &fp32_all);
    report(&mut out, &fp32)?;
    let int8 = compare("candle vs ort-int8", &candle_all, &int8_all);
    report(&mut out, &int8)?;

    let timed: Vec<&PackedBatch> = batches.iter().take(args.latency_groups).collect();
    writeln!(out, "延遲（{} 組，每組一次 batch forward）", timed.len()).map_err(io)?;
    for (name, model) in [
        ("candle", &candle),
        ("ort-fp32", &ort_fp32),
        ("ort-int8", &ort_int8),
    ] {
        let samples = latency(model, &timed)?;
        writeln!(out, "  {name:<9} {}", summary(&samples)).map_err(io)?;
    }
    writeln!(
        out,
        "  {}",
        if fp32.passed {
            "PASS：logits 誤差 < 1e-3、排序完全一致"
        } else {
            "FAIL：logits 誤差或排序不符"
        }
    )
    .map_err(io)?;
    Ok(fp32.passed)
}

fn io(err: std::io::Error) -> String {
    err.to_string()
}

/// 單組打分；ORT 的預算放寬到 10 秒，fp32 參考模型才不會被 100 ms 中止。
fn score(model: &Reranker, batch: &PackedBatch, id: &str) -> Result<Vec<f32>, String> {
    model
        .score_packed_with(batch, std::time::Duration::from_secs(10))
        .map_err(|err| format!("{id} 推論失敗：{err}"))
}

struct Comparison {
    name: String,
    max_abs: f32,
    mean_abs: f32,
    rankings_equal: usize,
    groups: usize,
    passed: bool,
}

fn compare(name: &str, left: &[Vec<f32>], right: &[Vec<f32>]) -> Comparison {
    let mut max_abs = 0f32;
    let mut total = 0f64;
    let mut count = 0usize;
    let mut rankings_equal = 0usize;
    for (a, b) in left.iter().zip(right) {
        for (x, y) in a.iter().zip(b) {
            let diff = (x - y).abs();
            max_abs = max_abs.max(diff);
            total += f64::from(diff);
            count += 1;
        }
        if order(a) == order(b) {
            rankings_equal += 1;
        }
    }
    Comparison {
        name: name.to_string(),
        max_abs,
        mean_abs: if count == 0 {
            0.0
        } else {
            (total / count as f64) as f32
        },
        rankings_equal,
        groups: left.len(),
        passed: max_abs < MAX_ABS_DIFF && rankings_equal == left.len(),
    }
}

fn report(out: &mut impl Write, cmp: &Comparison) -> Result<(), String> {
    writeln!(
        out,
        "{}：max |Δ| {:.3e}，mean |Δ| {:.3e}，排序一致 {}/{}",
        cmp.name, cmp.max_abs, cmp.mean_abs, cmp.rankings_equal, cmp.groups
    )
    .map_err(io)
}

/// 分數由高到低；同分保留原本順序。
/// 遮掉距離候選較遠的 context，驗證 candle 只算真 token 的路徑。
fn shrink_context(batch: &PackedBatch, keep: usize) -> PackedBatch {
    let seq = batch.seq;
    let mut ids = batch.ids.clone();
    let mut mask = batch.mask.clone();
    for row in 0..batch.rows {
        let base = row * seq;
        for slot in 1..=60usize {
            if slot + keep <= 60 {
                ids[base + slot] = 0;
                mask[base + slot] = 0;
            }
        }
        for slot in 64 + keep..seq {
            ids[base + slot] = 0;
            mask[base + slot] = 0;
        }
    }
    PackedBatch {
        ids,
        mask,
        seq,
        rows: batch.rows,
    }
}

fn order(scores: &[f32]) -> Vec<usize> {
    let mut index: Vec<usize> = (0..scores.len()).collect();
    index.sort_by(|a, b| {
        scores[*b]
            .partial_cmp(&scores[*a])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cmp(b))
    });
    index
}

fn latency(model: &Reranker, batches: &[&PackedBatch]) -> Result<Vec<f64>, String> {
    for batch in batches.iter().take(3) {
        model
            .score_packed_with(batch, std::time::Duration::from_secs(10))
            .map_err(|err| format!("暖機失敗：{err}"))?;
    }
    let mut samples = Vec::with_capacity(batches.len());
    for batch in batches {
        let started = Instant::now();
        model
            .score_packed_with(batch, std::time::Duration::from_secs(10))
            .map_err(|err| format!("延遲量測失敗：{err}"))?;
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Ok(samples)
}

fn summary(samples: &[f64]) -> String {
    if samples.is_empty() {
        return "沒有樣本".into();
    }
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    format!(
        "mean {:.2} ms，p50 {:.2} ms，p95 {:.2} ms，min {:.2} ms，max {:.2} ms",
        mean,
        percentile(samples, 50.0),
        percentile(samples, 95.0),
        samples[0],
        samples[samples.len() - 1]
    )
}

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (pct / 100.0 * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

struct Case {
    id: String,
    left: String,
    right: String,
    candidates: Vec<String>,
}

fn load_cases(path: &Path, limit: usize) -> Result<Vec<Case>, String> {
    let raw =
        std::fs::read_to_string(path).map_err(|err| format!("讀取 {}：{err}", path.display()))?;
    let mut cases = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if cases.len() >= limit {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line)
            .map_err(|err| format!("{}:{} 不是 JSON：{err}", path.display(), index + 1))?;
        let candidates: Vec<String> = value["candidates"]
            .as_array()
            .ok_or_else(|| format!("{}:{} 沒有 candidates", path.display(), index + 1))?
            .iter()
            .filter_map(|candidate| candidate["surface"].as_str().map(str::to_string))
            .collect();
        if candidates.len() < 2 {
            continue;
        }
        cases.push(Case {
            id: value["id"].as_str().unwrap_or("?").to_string(),
            left: value["left_text"].as_str().unwrap_or_default().to_string(),
            right: value["right_text"].as_str().unwrap_or_default().to_string(),
            candidates,
        });
    }
    Ok(cases)
}

fn parse(args: Vec<String>) -> Result<Args, String> {
    let mut out = Args::default();
    let mut index = 0;
    while index < args.len() {
        let value = |index: usize| {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{} 需要值", args[index]))
        };
        match args[index].as_str() {
            "--model-dir" => {
                out.model_dir = PathBuf::from(value(index)?);
                index += 1;
            }
            "--benchmark" => {
                out.benchmark = PathBuf::from(value(index)?);
                index += 1;
            }
            "--groups" => {
                out.groups = value(index)?
                    .parse()
                    .map_err(|err| format!("--groups：{err}"))?;
                index += 1;
            }
            "--latency-groups" => {
                out.latency_groups = value(index)?
                    .parse()
                    .map_err(|err| format!("--latency-groups：{err}"))?;
                index += 1;
            }
            "--ort-fp32" => {
                out.ort_fp32 = PathBuf::from(value(index)?);
                index += 1;
            }
            "--ort-int8" => {
                out.ort_int8 = PathBuf::from(value(index)?);
                index += 1;
            }
            "--ort-lib" => {
                out.ort_lib = Some(PathBuf::from(value(index)?));
                index += 1;
            }
            "--weights" => {
                out.weights = Some(PathBuf::from(value(index)?));
                index += 1;
            }
            "--shrink" => {
                out.shrink = value(index)?
                    .parse()
                    .map_err(|err| format!("--shrink：{err}"))?;
                index += 1;
            }
            "--help" | "-h" => {
                return Err(
                    "用法：candle-ja-check [--model-dir DIR] [--benchmark JSONL] [--groups N] \
                     [--latency-groups N] [--ort-fp32 ONNX] [--ort-int8 ONNX] [--ort-lib SO] \
                     [--weights SAFETENSORS] [--shrink N]"
                        .into(),
                );
            }
            other => return Err(format!("不認得的參數 {other}")),
        }
        index += 1;
    }
    if out.groups == 0 {
        return Err("--groups 必須大於 0".into());
    }
    Ok(out)
}
