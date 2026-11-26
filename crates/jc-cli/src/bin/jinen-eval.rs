//! Jinen v2 xsmall／small 的離線題庫量測。
//!
//! 同一套 prompt 與判定跑三組題庫，輸出 accept／NFKC 命中率、延遲分位數與
//! RSS 增量；逐題結果寫到 `target/jinen-eval/`，用來和 Python
//! （llama-cpp-python）版本逐題對帳。
//!
//! ```sh
//! cargo run -p jc-cli --release --bin jinen-eval -- --model xsmall --corpus ajimee-200
//! cargo run -p jc-cli --release --bin jinen-eval -- --model xsmall --corpus modern-500
//! cargo run -p jc-cli --release --bin jinen-eval -- --model xsmall --corpus hard-cases
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use jc_model::candle::jinen::{self, Jinen};
use serde::Deserialize;
use serde_json::json;

/// 題庫目錄。
const DEFAULT_TESTDATA: &str = "/mnt/work/jc/jc-ime-data/0c-testdata/ja";
/// 外部模型目錄。
const DEFAULT_MODEL_DATA: &str = "/mnt/work/jc/jc-ime-data/models/external";
/// 覆寫 tokenizer 路徑的環境變數。
const TOKENIZER_ENV: &str = "JC_JINEN_TOKENIZER";

#[derive(Debug, Clone)]
struct Args {
    model: String,
    corpus: String,
    testdata: PathBuf,
    gguf: Option<PathBuf>,
    tokenizer: Option<PathBuf>,
    limit: Option<usize>,
    max_new_tokens: usize,
    context: String,
    out: Option<PathBuf>,
    threads: usize,
    trace: Option<String>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            model: "xsmall".to_string(),
            corpus: "ajimee-200".to_string(),
            testdata: PathBuf::from(DEFAULT_TESTDATA),
            gguf: None,
            tokenizer: None,
            limit: None,
            max_new_tokens: jinen::DEFAULT_MAX_NEW_TOKENS,
            context: String::new(),
            out: None,
            threads: 4,
            trace: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Corpus {
    items: Vec<Item>,
}

#[derive(Debug, Deserialize)]
struct Item {
    id: String,
    reading: String,
    surface: String,
    #[serde(default)]
    accept: Vec<String>,
}

#[derive(Debug)]
struct Row {
    id: String,
    reading: String,
    surface: String,
    accept: Vec<String>,
    output: String,
    latency_ms: f64,
    cpu_ms: f64,
    prompt_tokens: usize,
    completion_tokens: usize,
    truncated: bool,
    literal_exact: bool,
    nfkc_exact: bool,
    accept_hit: bool,
    accept_nfkc_hit: bool,
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<(), String> {
    // candle 的 matmul 執行緒數由環境變數決定，必須在任何張量運算之前設定。
    // 預設 4 與 Python（llama.cpp `n_threads=4`）相同，才可比延遲。
    std::env::set_var("CANDLE_NUM_THREADS", args.threads.to_string());
    std::env::set_var("RAYON_NUM_THREADS", args.threads.to_string());

    let corpus_path = args.testdata.join(corpus_file(&args.corpus)?);
    let corpus: Corpus = serde_json::from_reader(
        std::fs::File::open(&corpus_path)
            .map_err(|err| format!("開不了 {}：{err}", corpus_path.display()))?,
    )
    .map_err(|err| format!("{} 不是合法題庫：{err}", corpus_path.display()))?;

    let gguf = match &args.gguf {
        Some(path) => path.clone(),
        None => default_gguf(&args.model)?,
    };
    let tokenizer = resolve_tokenizer(args, &gguf)?;

    let rss_before = rss_mb("VmRSS");
    let load_start = loadavg();
    let started = Instant::now();
    let mut model = Jinen::load(&gguf, &tokenizer)
        .map_err(|err| format!("載入 {} 失敗：{err}", gguf.display()))?;
    let load_ms = started.elapsed().as_secs_f64() * 1000.0;
    let rss_after_load = rss_mb("VmRSS");

    eprintln!(
        "model {}｜corpus {}｜threads {}｜gguf {}｜tokenizer {}｜vocab {}｜context {}",
        args.model,
        args.corpus,
        args.threads,
        gguf.display(),
        tokenizer.display(),
        model.vocab_size(),
        model.context_length()
    );

    // 暖機一次，不計入延遲。
    model
        .generate("テスト", "", args.max_new_tokens)
        .map_err(|err| format!("暖機失敗：{err}"))?;

    let items = match args.limit {
        Some(limit) => &corpus.items[..limit.min(corpus.items.len())],
        None => &corpus.items[..],
    };

    let mut rows = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let cpu_started = process_cpu_ms();
        let started = Instant::now();
        let tracing = args.trace.as_deref() == Some(item.id.as_str());
        let (generation, steps) = if tracing {
            let (generation, steps) = model
                .generate_traced(&item.reading, &args.context, args.max_new_tokens)
                .map_err(|err| format!("{} 推論失敗：{err}", item.id))?;
            (generation, Some(steps))
        } else {
            (
                model
                    .generate(&item.reading, &args.context, args.max_new_tokens)
                    .map_err(|err| format!("{} 推論失敗：{err}", item.id))?,
                None,
            )
        };
        let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
        if let Some(steps) = steps {
            eprintln!("trace {}", item.id);
            for (i, step) in steps.iter().enumerate() {
                let piece = model.decode(&[step.token]).unwrap_or_default();
                eprintln!(
                    "  {i:>2} id {:>5} {piece:?} top {:.4} runner-up {:.4} gap {:.4}",
                    step.token,
                    step.top_logit,
                    step.runner_up_logit,
                    step.top_logit - step.runner_up_logit
                );
            }
        }
        let cpu_ms = process_cpu_ms() - cpu_started;

        let accepted = accepted_surfaces(item);
        let nfkc_accepted: Vec<String> = accepted.iter().map(|s| jinen::nfkc(s)).collect();
        let output_nfkc = jinen::nfkc(&generation.text);
        let accept_hit = accepted.contains(&generation.text);
        let accept_nfkc_hit = nfkc_accepted.contains(&output_nfkc);
        rows.push(Row {
            id: item.id.clone(),
            reading: item.reading.clone(),
            surface: item.surface.clone(),
            accept: item.accept.clone(),
            literal_exact: generation.text == item.surface,
            nfkc_exact: output_nfkc == jinen::nfkc(&item.surface),
            accept_hit,
            accept_nfkc_hit,
            output: generation.text,
            latency_ms,
            cpu_ms,
            prompt_tokens: generation.prompt_tokens,
            completion_tokens: generation.tokens.len(),
            truncated: generation.truncated,
        });
        if (index + 1) % 50 == 0 {
            eprintln!("  {}/{}", index + 1, items.len());
        }
    }

    let summary = summary(&rows);
    let peak_rss = rss_mb("VmHWM");
    let load_end = loadavg();
    println!("corpus {}｜model {}", args.corpus, args.model);
    println!(
        "n {}｜literal exact {}｜NFKC exact {}｜accept {}｜accept+NFKC {}",
        summary.n,
        ratio(summary.literal_exact, summary.n),
        ratio(summary.nfkc_exact, summary.n),
        ratio(summary.accept_hit, summary.n),
        ratio(summary.accept_nfkc_hit, summary.n)
    );
    println!(
        "p50 {:.2} ms｜p95 {:.2} ms｜mean {:.2} ms｜min {:.2} ms｜CPU p50 {:.2} ms",
        summary.p50, summary.p95, summary.mean, summary.min, summary.cpu_p50
    );
    println!(
        "prompt tokens mean {:.1}｜completion tokens mean {:.1}｜截斷 {}｜空輸出 {}",
        summary.prompt_mean, summary.completion_mean, summary.truncated, summary.empty
    );
    println!(
        "載入 {:.1} ms｜RSS {:.1} → {:.1} MB｜峰值 {:.1} MB｜loadavg {:.2} → {:.2}",
        load_ms, rss_before, rss_after_load, peak_rss, load_start, load_end
    );

    let out = args.out.clone().unwrap_or_else(|| {
        PathBuf::from("target/jinen-eval")
            .join(format!("jinen-{}-{}.json", args.model, args.corpus))
    });
    write_json(
        &out,
        args,
        &gguf,
        &tokenizer,
        &rows,
        &summary,
        load_ms,
        rss_before,
        rss_after_load,
        peak_rss,
        load_start,
        load_end,
    )?;
    eprintln!("寫入 {}", out.display());
    Ok(())
}

#[derive(Debug)]
struct Summary {
    n: usize,
    literal_exact: usize,
    nfkc_exact: usize,
    accept_hit: usize,
    accept_nfkc_hit: usize,
    truncated: usize,
    empty: usize,
    p50: f64,
    p95: f64,
    mean: f64,
    min: f64,
    cpu_p50: f64,
    cpu_p95: f64,
    prompt_mean: f64,
    completion_mean: f64,
}

fn summary(rows: &[Row]) -> Summary {
    let n = rows.len();
    let mut latencies: Vec<f64> = rows.iter().map(|row| row.latency_ms).collect();
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut cpu: Vec<f64> = rows.iter().map(|row| row.cpu_ms).collect();
    cpu.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let sum: f64 = latencies.iter().sum();
    let prompt_sum: usize = rows.iter().map(|row| row.prompt_tokens).sum();
    let completion_sum: usize = rows.iter().map(|row| row.completion_tokens).sum();
    Summary {
        n,
        literal_exact: rows.iter().filter(|row| row.literal_exact).count(),
        nfkc_exact: rows.iter().filter(|row| row.nfkc_exact).count(),
        accept_hit: rows.iter().filter(|row| row.accept_hit).count(),
        accept_nfkc_hit: rows.iter().filter(|row| row.accept_nfkc_hit).count(),
        truncated: rows.iter().filter(|row| row.truncated).count(),
        empty: rows.iter().filter(|row| row.output.is_empty()).count(),
        p50: percentile(&latencies, 50.0),
        p95: percentile(&latencies, 95.0),
        mean: if n == 0 { 0.0 } else { sum / n as f64 },
        min: latencies.first().copied().unwrap_or(0.0),
        cpu_p50: percentile(&cpu, 50.0),
        cpu_p95: percentile(&cpu, 95.0),
        prompt_mean: if n == 0 {
            0.0
        } else {
            prompt_sum as f64 / n as f64
        },
        completion_mean: if n == 0 {
            0.0
        } else {
            completion_sum as f64 / n as f64
        },
    }
}

/// 最近秩百分位：排序後取第 `ceil(p/100*n)` 個。
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((p / 100.0 * sorted.len() as f64).ceil() as usize).max(1);
    sorted[rank - 1]
}

fn ratio(hits: usize, n: usize) -> String {
    if n == 0 {
        "0.0%".to_string()
    } else {
        format!("{hits}（{:.1}%）", 100.0 * hits as f64 / n as f64)
    }
}

fn accepted_surfaces(item: &Item) -> Vec<String> {
    let mut values = vec![item.surface.clone()];
    for value in &item.accept {
        if !values.contains(value) {
            values.push(value.clone());
        }
    }
    values
}

#[allow(clippy::too_many_arguments)]
fn write_json(
    path: &Path,
    args: &Args,
    gguf: &Path,
    tokenizer: &Path,
    rows: &[Row],
    summary: &Summary,
    load_ms: f64,
    rss_before: f64,
    rss_after_load: f64,
    peak_rss: f64,
    load_start: f64,
    load_end: f64,
) -> Result<(), String> {
    let items: Vec<_> = rows
        .iter()
        .map(|row| {
            json!({
                "id": row.id,
                "reading": row.reading,
                "surface": row.surface,
                "accept": row.accept,
                "output": row.output,
                "literal_exact": row.literal_exact,
                "nfkc_exact": row.nfkc_exact,
                "accept_hit": row.accept_hit,
                "accept_nfkc_hit": row.accept_nfkc_hit,
                "latency_ms": row.latency_ms,
                "cpu_ms": row.cpu_ms,
                "prompt_tokens": row.prompt_tokens,
                "completion_tokens": row.completion_tokens,
                "truncated": row.truncated,
            })
        })
        .collect();
    let pct = |hits: usize| {
        if summary.n == 0 {
            0.0
        } else {
            (1000.0 * hits as f64 / summary.n as f64).round() / 10.0
        }
    };
    let payload = json!({
        "meta": {
            "model": args.model,
            "corpus": args.corpus,
            "gguf": gguf.display().to_string(),
            "tokenizer": tokenizer.display().to_string(),
            "max_new_tokens": args.max_new_tokens,
            "context": args.context,
            "threads": args.threads,
            "decoding": "greedy: argmax, stop at eos",
            "prompt_format": "U+EE02<left_context>U+EE00<katakana_reading>U+EE01, NFKC",
            "load_ms": load_ms,
            "rss_before_mb": rss_before,
            "rss_after_load_mb": rss_after_load,
            "peak_rss_mb": peak_rss,
            "loadavg_start": load_start,
            "loadavg_end": load_end,
        },
        "summary": {
            "n": summary.n,
            "literal_exact": summary.literal_exact,
            "literal_exact_pct": pct(summary.literal_exact),
            "nfkc_exact": summary.nfkc_exact,
            "nfkc_exact_pct": pct(summary.nfkc_exact),
            "accept_hit": summary.accept_hit,
            "accept_hit_pct": pct(summary.accept_hit),
            "accept_nfkc_hit": summary.accept_nfkc_hit,
            "accept_nfkc_hit_pct": pct(summary.accept_nfkc_hit),
            "p50_ms": summary.p50,
            "p95_ms": summary.p95,
            "cpu_p50_ms": summary.cpu_p50,
            "cpu_p95_ms": summary.cpu_p95,
            "mean_ms": summary.mean,
            "min_ms": summary.min,
            "prompt_tokens_mean": summary.prompt_mean,
            "completion_tokens_mean": summary.completion_mean,
            "truncated": summary.truncated,
            "empty_output": summary.empty,
        },
        "items": items,
    });
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("建不了 {}：{err}", parent.display()))?;
    }
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&payload).map_err(|e| e.to_string())?,
    )
    .map_err(|err| format!("寫不了 {}：{err}", path.display()))
}

fn default_gguf(model: &str) -> Result<PathBuf, String> {
    let (slug, quant) = match model {
        "xsmall" => ("jinen-v2-xsmall", "Q5_K_M"),
        "small" => ("jinen-v2-small", "Q4_K_M"),
        other => return Err(format!("--model 只接受 xsmall 或 small，實際為 {other}")),
    };
    Ok(PathBuf::from(DEFAULT_MODEL_DATA)
        .join(slug)
        .join(format!("{slug}-{quant}.gguf")))
}

fn resolve_tokenizer(args: &Args, gguf: &Path) -> Result<PathBuf, String> {
    if let Some(path) = &args.tokenizer {
        return Ok(path.clone());
    }
    if let Ok(path) = std::env::var(TOKENIZER_ENV) {
        return Ok(PathBuf::from(path));
    }
    // 依序：GGUF 同目錄、目前目錄的 models/external/<slug>/。
    let mut candidates = Vec::new();
    if let Some(dir) = gguf.parent() {
        candidates.push(dir.join("tokenizer.json"));
    }
    if let Some(slug) = gguf
        .parent()
        .and_then(|dir| dir.file_name())
        .map(|name| name.to_string_lossy().into_owned())
    {
        candidates.push(
            PathBuf::from("models")
                .join("external")
                .join(slug)
                .join("tokenizer.json"),
        );
    }
    for candidate in &candidates {
        if candidate.exists() {
            return Ok(candidate.clone());
        }
    }
    Err(format!(
        "找不到 tokenizer.json，試過 {}；可用 --tokenizer 或 {TOKENIZER_ENV} 指定",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("、")
    ))
}

fn corpus_file(corpus: &str) -> Result<&'static str, String> {
    match corpus {
        "ajimee-200" => Ok("ajimee_200.json"),
        "modern-500" => Ok("modern_500.json"),
        "hard-cases" => Ok("hard_cases.json"),
        other => Err(format!(
            "--corpus 只接受 ajimee-200、modern-500、hard-cases，實際為 {other}"
        )),
    }
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut parsed = Args::default();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} 需要一個值"));
        match arg.as_str() {
            "--model" => parsed.model = value("--model")?,
            "--corpus" => parsed.corpus = value("--corpus")?,
            "--testdata" => parsed.testdata = PathBuf::from(value("--testdata")?),
            "--gguf" => parsed.gguf = Some(PathBuf::from(value("--gguf")?)),
            "--tokenizer" => parsed.tokenizer = Some(PathBuf::from(value("--tokenizer")?)),
            "--out" => parsed.out = Some(PathBuf::from(value("--out")?)),
            "--context" => parsed.context = value("--context")?,
            "--limit" => {
                parsed.limit = Some(
                    value("--limit")?
                        .parse()
                        .map_err(|err| format!("--limit 不是整數：{err}"))?,
                )
            }
            "--max-new-tokens" => {
                parsed.max_new_tokens = value("--max-new-tokens")?
                    .parse()
                    .map_err(|err| format!("--max-new-tokens 不是整數：{err}"))?
            }
            "--trace" => parsed.trace = Some(value("--trace")?),
            "--threads" => {
                parsed.threads = value("--threads")?
                    .parse()
                    .map_err(|err| format!("--threads 不是整數：{err}"))?
            }
            "-h" | "--help" => {
                println!(
                    "用法：jinen-eval [--model xsmall|small] [--corpus ajimee-200|modern-500|hard-cases]\n\
                     \x20       [--testdata DIR] [--gguf FILE] [--tokenizer FILE] [--limit N]\n\
                     \x20       [--max-new-tokens N] [--context TEXT] [--threads N] [--trace ID] [--out FILE]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("未知參數：{other}")),
        }
    }
    Ok(parsed)
}

fn process_cpu_ms() -> f64 {
    // /proc/self/stat 的 utime、stime 以 clock tick 計；Linux aarch64 是 100 Hz。
    let Ok(stat) = std::fs::read_to_string("/proc/self/stat") else {
        return 0.0;
    };
    // comm 可能含空白，從最後一個 ')' 之後開始切。
    let Some(rest) = stat.rsplit_once(')').map(|(_, rest)| rest) else {
        return 0.0;
    };
    let fields: Vec<&str> = rest.split_whitespace().collect();
    if fields.len() < 13 {
        return 0.0;
    }
    let utime: f64 = fields[11].parse().unwrap_or(0.0);
    let stime: f64 = fields[12].parse().unwrap_or(0.0);
    (utime + stime) * 10.0
}

fn loadavg() -> f64 {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|text| text.split_whitespace().next()?.parse().ok())
        .unwrap_or(0.0)
}

fn rss_mb(key: &str) -> f64 {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return 0.0;
    };
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix(&format!("{key}:")) {
            if let Some(kb) = rest.split_whitespace().next() {
                if let Ok(kb) = kb.parse::<f64>() {
                    return kb / 1024.0;
                }
            }
        }
    }
    0.0
}
