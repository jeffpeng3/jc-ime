//! 量化權重的匯出、量測與比對。
//!
//! ```text
//! # 匯出三種等級（原始 safetensors 不動）
//! cargo run -p jc-model --release --bin candle-quant -- export \
//!   --input /mnt/work/jc/jc-ime-data/models/reranker-extra/model.safetensors \
//!   --output /mnt/work/jc/jc-ime-data/models/quant/candle/ja-rerank/q8_0/model.gguf \
//!   --dtype q8_0
//!
//! # 單一權重檔量測（每個權重檔都要重開行程）
//! cargo run -p jc-model --release --bin candle-quant -- bench \
//!   --model-dir /mnt/work/jc/jc-ime-data/models/reranker-extra \
//!   --weights /mnt/work/jc/jc-ime-data/models/quant/candle/ja-rerank/q8_0/model.gguf \
//!   --benchmark /mnt/work/0c/data/benchmarks/homophone-extra-v1/test.jsonl \
//!   --groups 200 --out /tmp/q8_0.json
//!
//! # 與 fp32 的數值比對
//! cargo run -p jc-model --release --bin candle-quant -- compare \
//!   --baseline /tmp/fp32.json --other /tmp/q8_0.json
//! ```
//!
//! `bench` 會印出 RSS（載入後、100 次推論後）與延遲分佈，並把每一組的
//! 分數寫進 `--out`，`compare` 再從兩份 JSON 算誤差與排序統計。分開跑的
//! 原因是 RSS 只有在單一模型載入的乾淨行程裡才有意義。
//!
//! 標點模型用 `punct` 子命令，輸入是固定 256-token 視窗的 int32 檔（P2-24A
//! validation 的第 0 個 shard），輸出每組的 logits：
//!
//! ```text
//! cargo run -p jc-model --release --bin candle-quant -- punct \
//!   --model-dir /mnt/work/jc/jc-ime-data/models/punctuation-public-core \
//!   --weights models/quant/candle/ja-punct/q4k/model.gguf \
//!   --tokens /mnt/work/0c/data/punctuation_public_core_ja/validation/validation-00000.tokens.i32 \
//!   --groups 200 --out /tmp/punct-q4k.json
//! ```

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use jc_model::candle::{
    export_gguf, export_gguf_with_embedding, export_punct_gguf, EmbeddingDtype, QuantDtype,
};
use jc_model::layout::{pack_batch, PackedBatch};
use jc_model::manifest::ModelSpec;
use jc_model::{Backend, PunctuationModel, Reranker, Tokenizer};
use serde_json::{json, Value};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("candle-quant：{err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let command = args.next().ok_or_else(usage)?;
    let rest: Vec<String> = args.collect();
    match command.as_str() {
        "export" => export(&rest),
        "bench" => bench(&rest),
        "punct" => punct_bench(&rest),
        "compare" => compare(&rest),
        "-h" | "--help" | "help" => Err(usage()),
        other => Err(format!("不認得的子命令 {other}\n{}", usage())),
    }
}

fn usage() -> String {
    "用法：candle-quant <export|bench|compare> [選項]\n\
     export  --input SAFETENSORS --output GGUF --dtype q8_0|q5k|q4k \
     [--embed-dtype f32|q8_0|q5k|q4k] [--punct]\n\
     bench   --model-dir DIR --weights FILE --benchmark JSONL [--groups N] \
     [--iters N] [--latency-groups N] [--out JSON]\n\
     punct   --model-dir DIR --weights FILE --tokens I32 [--groups N] \
     [--seq N] [--iters N] [--out JSON]\n\
     compare --baseline JSON --other JSON"
        .into()
}

fn value_of<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

fn required<'a>(args: &'a [String], name: &str) -> Result<&'a str, String> {
    value_of(args, name).ok_or_else(|| format!("{name} 是必填"))
}

fn export(args: &[String]) -> Result<(), String> {
    let input = PathBuf::from(required(args, "--input")?);
    let output = PathBuf::from(required(args, "--output")?);
    let dtype = required(args, "--dtype")?
        .parse::<QuantDtype>()
        .map_err(|err| format!("--dtype：{err}"))?;
    let embedding = match value_of(args, "--embed-dtype") {
        None => None,
        Some("f32") => Some(EmbeddingDtype::F32),
        Some(value) => Some(EmbeddingDtype::Quant(
            value
                .parse::<QuantDtype>()
                .map_err(|err| format!("--embed-dtype：{err}"))?,
        )),
    };
    let report = if args.iter().any(|arg| arg == "--punct") {
        export_punct_gguf(&input, &output, dtype).map_err(|err| err.to_string())?
    } else if let Some(embedding) = embedding {
        export_gguf_with_embedding(&input, &output, dtype, embedding, &ModelSpec::default())
            .map_err(|err| err.to_string())?
    } else {
        export_gguf(&input, &output, dtype, &ModelSpec::default()).map_err(|err| err.to_string())?
    };
    let mut out = std::io::stdout().lock();
    let _ = writeln!(
        out,
        "{}  {:.2} MB  {:.3} bits/weight",
        report.path.display(),
        report.file_bytes as f64 / (1024.0 * 1024.0),
        report.quantized_bits_per_weight()
    );
    for entry in &report.entries {
        let _ = writeln!(
            out,
            "  {:<44} {:>6}  {:>10} 筆  {:>9.2} MB",
            entry.name,
            entry.dtype,
            entry.params,
            entry.bytes as f64 / (1024.0 * 1024.0)
        );
    }
    Ok(())
}

struct BenchArgs {
    model_dir: PathBuf,
    weights: PathBuf,
    benchmark: PathBuf,
    groups: usize,
    iters: usize,
    latency_groups: usize,
    out: Option<PathBuf>,
}

impl BenchArgs {
    fn parse(args: &[String]) -> Result<Self, String> {
        Ok(Self {
            model_dir: PathBuf::from(required(args, "--model-dir")?),
            weights: PathBuf::from(required(args, "--weights")?),
            benchmark: PathBuf::from(required(args, "--benchmark")?),
            groups: value_of(args, "--groups")
                .unwrap_or("200")
                .parse()
                .map_err(|err| format!("--groups：{err}"))?,
            iters: value_of(args, "--iters")
                .unwrap_or("100")
                .parse()
                .map_err(|err| format!("--iters：{err}"))?,
            latency_groups: value_of(args, "--latency-groups")
                .unwrap_or("50")
                .parse()
                .map_err(|err| format!("--latency-groups：{err}"))?,
            out: value_of(args, "--out").map(PathBuf::from),
        })
    }
}

fn bench(args: &[String]) -> Result<(), String> {
    let args = BenchArgs::parse(args)?;
    let spec = ModelSpec::default();
    let tokenizer = Tokenizer::load(args.model_dir.join(spec.vocab_file()))
        .map_err(|err| format!("載入 tokenizer 失敗：{err}"))?;
    let started = Instant::now();
    std::env::set_var(jc_model::candle::WEIGHTS_ENV, &args.weights);
    let model = Reranker::load_with_backend(&args.model_dir, &spec, None, Backend::Candle)
        .map_err(|err| format!("載入模型失敗：{err}"))?;
    let load_ms = started.elapsed().as_secs_f64() * 1000.0;
    let rss_load = rss_bytes().ok_or("讀不到 /proc/self/status")?;
    let file_bytes = std::fs::metadata(&args.weights)
        .map_err(|err| format!("{}：{err}", args.weights.display()))?
        .len();
    let quant = match args.weights.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("gguf") => "gguf",
        _ => "fp32",
    };

    let cases = load_cases(&args.benchmark, args.groups)?;
    if cases.is_empty() {
        return Err(format!("{} 沒有可用的組", args.benchmark.display()));
    }
    let mut batches = Vec::with_capacity(cases.len());
    for case in &cases {
        let candidates: Vec<&str> = case.candidates.iter().map(String::as_str).collect();
        batches.push(
            pack_batch(
                &tokenizer,
                &spec,
                Some(&case.left),
                Some(&case.right),
                &candidates,
            )
            .map_err(|err| format!("打包 {} 失敗：{err}", case.id))?,
        );
    }

    let mut scores = Vec::with_capacity(batches.len());
    for (case, batch) in cases.iter().zip(&batches) {
        scores.push(score(&model, batch, &case.id)?);
    }

    let mut samples = latency(&model, &batches, args.iters, args.latency_groups)?;
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rss_after = rss_bytes().ok_or("讀不到 /proc/self/status")?;

    let payload = json!({
        "label": args.weights.display().to_string(),
        "quant": quant,
        "file_bytes": file_bytes,
        "file_mb": file_bytes as f64 / (1024.0 * 1024.0),
        "rss_load_mb": rss_load as f64 / (1024.0 * 1024.0),
        "rss_after_mb": rss_after as f64 / (1024.0 * 1024.0),
        "rss_delta_mb": (rss_after.saturating_sub(rss_load)) as f64 / (1024.0 * 1024.0),
        "load_ms": load_ms,
        "groups": batches.len(),
        "iters": args.iters,
        "latency_ms": {
            "p50": percentile(&samples, 50.0),
            "p95": percentile(&samples, 95.0),
            "mean": samples.iter().sum::<f64>() / samples.len().max(1) as f64,
            "min": samples.first().copied().unwrap_or(0.0),
            "samples": samples.len(),
        },
        "case_ids": cases.iter().map(|case| case.id.clone()).collect::<Vec<_>>(),
        "scores": scores,
    });
    let text = serde_json::to_string_pretty(&payload).map_err(|err| err.to_string())?;
    if let Some(path) = &args.out {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        std::fs::write(path, &text).map_err(|err| format!("{}：{err}", path.display()))?;
    }
    println!("{text}");
    Ok(())
}

fn score(model: &Reranker, batch: &PackedBatch, id: &str) -> Result<Vec<f32>, String> {
    model
        .score_packed_with(batch, Duration::from_secs(10))
        .map_err(|err| format!("{id} 推論失敗：{err}"))
}

/// 前 3 組暖機，之後對每組重複跑，湊到 `iters` 次樣本。
fn latency(
    model: &Reranker,
    batches: &[PackedBatch],
    iters: usize,
    first: usize,
) -> Result<Vec<f64>, String> {
    if batches.is_empty() {
        return Ok(Vec::new());
    }
    for batch in batches.iter().take(3) {
        score(model, batch, "暖機")?;
    }
    let pool = &batches[..batches.len().min(first.max(1))];
    let mut samples = Vec::with_capacity(iters);
    for index in 0..iters {
        let batch = &pool[index % pool.len()];
        let started = Instant::now();
        score(model, batch, "量測")?;
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(samples)
}

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (pct / 100.0 * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

/// `/proc/self/status` 的 VmRSS。
fn rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

/// 標點模型的預設 token 檔；P2-24A validation 的第一個 shard。
const DEFAULT_PUNCT_TOKENS: &str =
    "/mnt/work/0c/data/punctuation_public_core_ja/validation/validation-00000.tokens.i32";

struct PunctArgs {
    model_dir: PathBuf,
    weights: PathBuf,
    tokens: PathBuf,
    groups: usize,
    seq: usize,
    iters: usize,
    out: Option<PathBuf>,
}

impl PunctArgs {
    fn parse(args: &[String]) -> Result<Self, String> {
        Ok(Self {
            model_dir: PathBuf::from(required(args, "--model-dir")?),
            weights: PathBuf::from(required(args, "--weights")?),
            tokens: PathBuf::from(value_of(args, "--tokens").unwrap_or(DEFAULT_PUNCT_TOKENS)),
            groups: value_of(args, "--groups")
                .unwrap_or("200")
                .parse()
                .map_err(|err| format!("--groups：{err}"))?,
            seq: value_of(args, "--seq")
                .unwrap_or("256")
                .parse()
                .map_err(|err| format!("--seq：{err}"))?,
            iters: value_of(args, "--iters")
                .unwrap_or("100")
                .parse()
                .map_err(|err| format!("--iters：{err}"))?,
            out: value_of(args, "--out").map(PathBuf::from),
        })
    }
}

fn punct_bench(args: &[String]) -> Result<(), String> {
    let args = PunctArgs::parse(args)?;
    // 對照模式把預算放寬；產品路徑的 100 ms 上限由呼叫端控制。
    let spec = ModelSpec {
        timeout_ms: Some(10_000),
        ..ModelSpec::default()
    };
    std::env::set_var(jc_model::candle::PUNCT_WEIGHTS_ENV, &args.weights);
    let started = Instant::now();
    let model = PunctuationModel::load_with_backend(&args.model_dir, &spec, None, Backend::Candle)
        .map_err(|err| format!("載入模型失敗：{err}"))?;
    let load_ms = started.elapsed().as_secs_f64() * 1000.0;
    let rss_load = rss_bytes().ok_or("讀不到 /proc/self/status")?;
    let file_bytes = std::fs::metadata(&args.weights)
        .map_err(|err| format!("{}：{err}", args.weights.display()))?
        .len();
    let quant = match args.weights.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("gguf") => "gguf",
        _ => "fp32",
    };
    let windows = read_windows(&args.tokens, args.groups, args.seq)?;
    let mut logits = Vec::with_capacity(windows.len());
    for (index, window) in windows.iter().enumerate() {
        let rows = model
            .logits_for_ids(window)
            .ok_or_else(|| format!("第 {index} 組推論失敗"))?;
        logits.push(rows);
    }
    for window in windows.iter().take(3) {
        model
            .logits_for_ids(window)
            .ok_or_else(|| "暖機失敗".to_string())?;
    }
    let mut samples = Vec::with_capacity(args.iters);
    for index in 0..args.iters {
        let window = &windows[index % windows.len()];
        let started = Instant::now();
        model
            .logits_for_ids(window)
            .ok_or_else(|| "延遲量測失敗".to_string())?;
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rss_after = rss_bytes().ok_or("讀不到 /proc/self/status")?;
    let payload = json!({
        "label": args.weights.display().to_string(),
        "quant": quant,
        "file_bytes": file_bytes,
        "file_mb": file_bytes as f64 / (1024.0 * 1024.0),
        "rss_load_mb": rss_load as f64 / (1024.0 * 1024.0),
        "rss_after_mb": rss_after as f64 / (1024.0 * 1024.0),
        "rss_delta_mb": (rss_after.saturating_sub(rss_load)) as f64 / (1024.0 * 1024.0),
        "load_ms": load_ms,
        "groups": windows.len(),
        "seq": args.seq,
        "iters": args.iters,
        "latency_ms": {
            "p50": percentile(&samples, 50.0),
            "p95": percentile(&samples, 95.0),
            "mean": samples.iter().sum::<f64>() / samples.len().max(1) as f64,
            "min": samples.first().copied().unwrap_or(0.0),
            "samples": samples.len(),
        },
        "logits": logits,
    });
    let text = serde_json::to_string_pretty(&payload).map_err(|err| err.to_string())?;
    if let Some(path) = &args.out {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        std::fs::write(path, &text).map_err(|err| format!("{}：{err}", path.display()))?;
    }
    println!("{text}");
    Ok(())
}

/// 讀 int32 token 檔，每 `seq` 個 token 一組。
fn read_windows(path: &Path, groups: usize, seq: usize) -> Result<Vec<Vec<i64>>, String> {
    let bytes = std::fs::read(path).map_err(|err| format!("{}：{err}", path.display()))?;
    if bytes.len() % 4 != 0 {
        return Err(format!("{}：長度不是 4 的倍數", path.display()));
    }
    let values: Vec<i32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| i32::from_le_bytes(*chunk))
        .collect();
    let available = values.len() / seq.max(1);
    if available == 0 {
        return Err(format!("{}：不足一個視窗", path.display()));
    }
    let take = groups.min(available);
    Ok(values[..take * seq]
        .chunks_exact(seq)
        .map(|window| window.iter().map(|id| i64::from(*id)).collect())
        .collect())
}

fn compare(args: &[String]) -> Result<(), String> {
    let baseline = read_json(&PathBuf::from(required(args, "--baseline")?))?;
    let other = read_json(&PathBuf::from(required(args, "--other")?))?;
    let base_scores = matrix(&baseline)?;
    let other_scores = matrix(&other)?;
    if base_scores.len() != other_scores.len() {
        return Err(format!(
            "組數不同：{} 對 {}",
            base_scores.len(),
            other_scores.len()
        ));
    }
    let mut max_abs = 0f64;
    let mut sum_abs = 0f64;
    let mut count = 0usize;
    let mut top1_same = 0usize;
    let mut order_same = 0usize;
    let mut pairs_same = 0usize;
    let mut pairs_total = 0usize;
    let mut spearman_sum = 0f64;
    for (left, right) in base_scores.iter().zip(&other_scores) {
        if left.len() != right.len() {
            return Err("候選數不同".into());
        }
        for (a, b) in left.iter().zip(right) {
            let diff = (f64::from(*a) - f64::from(*b)).abs();
            max_abs = max_abs.max(diff);
            sum_abs += diff;
            count += 1;
        }
        let left_order = order(left);
        let right_order = order(right);
        if left_order.first() == right_order.first() {
            top1_same += 1;
        }
        if left_order == right_order {
            order_same += 1;
        }
        for a in 0..left.len() {
            for b in (a + 1)..left.len() {
                pairs_total += 1;
                if (left[a] > left[b]) == (right[a] > right[b]) {
                    pairs_same += 1;
                }
            }
        }
        spearman_sum += spearman(left, right);
    }
    let groups = base_scores.len();
    let summary = json!({
        "baseline": baseline["label"],
        "other": other["label"],
        "baseline_file_mb": baseline["file_mb"],
        "other_file_mb": other["file_mb"],
        "baseline_rss_load_mb": baseline["rss_load_mb"],
        "other_rss_load_mb": other["rss_load_mb"],
        "baseline_rss_after_mb": baseline["rss_after_mb"],
        "other_rss_after_mb": other["rss_after_mb"],
        "baseline_p50_ms": baseline["latency_ms"]["p50"],
        "other_p50_ms": other["latency_ms"]["p50"],
        "baseline_p95_ms": baseline["latency_ms"]["p95"],
        "other_p95_ms": other["latency_ms"]["p95"],
        "groups": groups,
        "scored_pairs": count,
        "max_abs_error": max_abs,
        "mean_abs_error": if count == 0 { 0.0 } else { sum_abs / count as f64 },
        "spearman_mean": if groups == 0 { 1.0 } else { spearman_sum / groups as f64 },
        "top1_agreement": if groups == 0 { 1.0 } else { top1_same as f64 / groups as f64 },
        "order_agreement": if groups == 0 { 1.0 } else { order_same as f64 / groups as f64 },
        "pairwise_agreement": if pairs_total == 0 { 1.0 } else { pairs_same as f64 / pairs_total as f64 },
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&summary).map_err(|err| err.to_string())?
    );
    Ok(())
}

fn read_json(path: &Path) -> Result<Value, String> {
    let raw = std::fs::read_to_string(path).map_err(|err| format!("{}：{err}", path.display()))?;
    serde_json::from_str(&raw).map_err(|err| format!("{}：{err}", path.display()))
}

/// 把 `scores`（每組候選）或 `logits`（每組 × token × 類別）攤平成
/// 「一個排序單元一列」；標點模型的一個排序單元就是一個 token。
fn matrix(value: &Value) -> Result<Vec<Vec<f32>>, String> {
    if let Some(groups) = value["logits"].as_array() {
        let mut out = Vec::new();
        for group in groups {
            for row in group.as_array().ok_or("logits 的元素不是陣列")? {
                let list = row.as_array().ok_or("logits 的 token 不是陣列")?;
                out.push(
                    list.iter()
                        .map(|value| value.as_f64().unwrap_or(0.0) as f32)
                        .collect(),
                );
            }
        }
        return Ok(out);
    }
    let groups = value["scores"]
        .as_array()
        .ok_or("JSON 沒有 scores 或 logits 陣列")?;
    let mut out = Vec::with_capacity(groups.len());
    for group in groups {
        let list = group.as_array().ok_or("scores 的元素不是陣列")?;
        let mut row = Vec::with_capacity(list.len());
        for score in list {
            row.push(score.as_f64().ok_or("score 不是數字")? as f32);
        }
        out.push(row);
    }
    Ok(out)
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

/// Spearman：把兩個分數向量各自換成名次後算 Pearson。
fn spearman(left: &[f32], right: &[f32]) -> f64 {
    let n = left.len().min(right.len());
    if n < 2 {
        return 1.0;
    }
    let left_rank = ranks(&left[..n]);
    let right_rank = ranks(&right[..n]);
    let mean = (n as f64 + 1.0) / 2.0;
    let mut num = 0f64;
    let mut left_var = 0f64;
    let mut right_var = 0f64;
    for (a, b) in left_rank.iter().zip(&right_rank) {
        num += (a - mean) * (b - mean);
        left_var += (a - mean).powi(2);
        right_var += (b - mean).powi(2);
    }
    let denom = (left_var * right_var).sqrt();
    if denom == 0.0 {
        1.0
    } else {
        num / denom
    }
}

fn ranks(values: &[f32]) -> Vec<f64> {
    let mut index: Vec<usize> = (0..values.len()).collect();
    index.sort_by(|a, b| {
        values[*b]
            .partial_cmp(&values[*a])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cmp(b))
    });
    let mut ranks = vec![0f64; values.len()];
    for (rank, &position) in index.iter().enumerate() {
        ranks[position] = rank as f64 + 1.0;
    }
    ranks
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
        let value: Value = serde_json::from_str(line)
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
