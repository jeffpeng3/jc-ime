//! 標點模型的 candle 對照工具。
//!
//! 用同一組 token id 跑 candle、ORT fp32 與 ORT int8，比對 logits、argmax
//! 與最終標點輸出，再量單次推論延遲。
//!
//! ```sh
//! ORT_DYLIB_PATH=<...>/libonnxruntime.so.1.32.0 \
//!   cargo run -p jc-cli --release --bin candle-punct-check -- --groups 200
//! ```
//!
//! 預設讀 P2-24A 驗證集的前 200 個 256-token 視窗；`--tokens` 可換檔案。
//! 標點輸出一致率用 `0c-testdata/ja/modern_500.json` 的 surface，移除既有
//! 標點後交給兩個後端。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use jc_model::{Backend, ModelSpec, PunctuationModel};

/// 預設模型根目錄。
const DEFAULT_MODELS: &str = "/mnt/work/jc/jc-ime-data/models";
/// 預設 fp32 對照模型；產品目錄只有 int8。
const DEFAULT_ORT_MODEL: &str =
    "/mnt/work/0c/models/trained/punctuation-public-core/punctuation-public-core.fp32.onnx";
/// 預設 token 檔；P2-24A validation 的第一個 shard。
const DEFAULT_TOKENS: &str =
    "/mnt/work/0c/data/punctuation_public_core_ja/validation/validation-00000.tokens.i32";
/// 預設題庫。
const DEFAULT_TEXTS: &str = "/mnt/work/jc/jc-ime-data/0c-testdata/ja/modern_500.json";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("{USAGE}");
        return;
    }
    let mut out = std::io::stdout();
    if let Err(err) = run(&args, &mut out) {
        eprintln!("candle-punct-check：{err}");
        std::process::exit(1);
    }
}

const USAGE: &str = "\
candle-punct-check [--models <dir>] [--ort-model <fp32.onnx>] [--tokens <i32>] \
[--groups 200] [--seq 256] [--latency 50] [--texts <json>] [--skip-texts]

  --models     模型根目錄（預設 /mnt/work/jc/jc-ime-data/models）
  --ort-model  ORT fp32 對照模型（預設 P2-24A 訓練產物）
  --tokens     int32 token 檔，每 --seq 個 token 一組
  --groups     比對組數（預設 200）
  --seq        每組 token 數（預設 256）
  --latency    量測延遲的組數（預設 50，0 代表不量）
  --texts      標點輸出一致率用的題庫 json（預設 modern_500）
  --skip-texts 跳過標點輸出一致率";

struct Args {
    models: PathBuf,
    ort_model: PathBuf,
    tokens: PathBuf,
    texts: PathBuf,
    groups: usize,
    seq: usize,
    latency: usize,
    skip_texts: bool,
}

impl Args {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut parsed = Self {
            models: PathBuf::from(std::env::var("JC_MODELS").unwrap_or(DEFAULT_MODELS.into())),
            ort_model: PathBuf::from(DEFAULT_ORT_MODEL),
            tokens: PathBuf::from(DEFAULT_TOKENS),
            texts: PathBuf::from(DEFAULT_TEXTS),
            groups: 200,
            seq: 256,
            latency: 50,
            skip_texts: false,
        };
        let mut index = 0;
        while index < args.len() {
            let value = |index: usize| -> Result<&String, String> {
                args.get(index)
                    .ok_or_else(|| format!("{} 需要值", args[index - 1]))
            };
            match args[index].as_str() {
                "--models" => {
                    index += 1;
                    parsed.models = PathBuf::from(value(index)?);
                }
                "--ort-model" => {
                    index += 1;
                    parsed.ort_model = PathBuf::from(value(index)?);
                }
                "--tokens" => {
                    index += 1;
                    parsed.tokens = PathBuf::from(value(index)?);
                }
                "--texts" => {
                    index += 1;
                    parsed.texts = PathBuf::from(value(index)?);
                }
                "--groups" => {
                    index += 1;
                    parsed.groups = number(value(index)?, "--groups")?;
                }
                "--seq" => {
                    index += 1;
                    parsed.seq = number(value(index)?, "--seq")?;
                }
                "--latency" => {
                    index += 1;
                    parsed.latency = number(value(index)?, "--latency")?;
                }
                "--skip-texts" => parsed.skip_texts = true,
                other => return Err(format!("不認得的參數 {other}")),
            }
            index += 1;
        }
        if parsed.groups == 0 || parsed.seq == 0 {
            return Err("--groups 與 --seq 必須大於 0".into());
        }
        Ok(parsed)
    }
}

fn number(value: &str, flag: &str) -> Result<usize, String> {
    value
        .parse()
        .map_err(|_| format!("{flag} 需要非負整數，收到 {value:?}"))
}

fn run(args: &[String], out: &mut dyn Write) -> Result<(), String> {
    let args = Args::parse(args)?;
    let dir = args.models.join("punctuation-public-core");
    if !dir.is_dir() {
        return Err(format!("找不到模型目錄 {}", dir.display()));
    }

    // 對照模式把預算放寬，否則 256-token 的 fp32 推論會超過產品 100 ms 上限。
    let spec = ModelSpec {
        timeout_ms: Some(5_000),
        ..ModelSpec::default()
    };
    let candle = PunctuationModel::load_with_backend(&dir, &spec, None, Backend::Candle)
        .map_err(|err| format!("candle 載入失敗：{err}"))?;
    let int8 = PunctuationModel::load_with_backend(&dir, &spec, None, Backend::Ort)
        .map_err(|err| format!("ORT int8 載入失敗：{err}"))?;
    let fp32_spec = ModelSpec {
        model: Some(args.ort_model.to_string_lossy().into_owned()),
        ..spec.clone()
    };
    let fp32 = PunctuationModel::load_with_backend(&dir, &fp32_spec, None, Backend::Ort)
        .map_err(|err| format!("ORT fp32 載入失敗：{err}"))?;
    assert_eq!(candle.backend(), Backend::Candle);

    let groups = read_groups(&args.tokens, args.groups, args.seq)?;
    writeln!(
        out,
        "groups {}，seq {}，tokens {}",
        groups.len(),
        args.seq,
        groups.len() * args.seq
    )
    .map_err(io)?;

    writeln!(out, "\nlogits 對照").map_err(io)?;
    let fp32_stats = compare(&candle, &fp32, &groups, "candle vs ort-fp32", out)?;
    let int8_stats = compare(&candle, &int8, &groups, "candle vs ort-int8", out)?;

    writeln!(out, "\n標點輸出一致率（modern-500，移除既有標點後重跑）").map_err(io)?;
    let (punct_fp32, _punct_int8) = if args.skip_texts {
        writeln!(out, "（--skip-texts）").map_err(io)?;
        (None, None)
    } else {
        let texts = read_texts(&args.texts)?;
        (
            Some(punct_agreement(
                &candle,
                &fp32,
                &texts,
                "candle vs ort-fp32",
                out,
            )?),
            Some(punct_agreement(
                &candle,
                &int8,
                &texts,
                "candle vs ort-int8",
                out,
            )?),
        )
    };

    let mut latency_stats = None;
    if args.latency > 0 {
        let count = args.latency.min(groups.len());
        writeln!(
            out,
            "\n延遲（{count} 組，每次一個 {}-token 視窗）",
            args.seq
        )
        .map_err(io)?;
        let mut latency = |model: &PunctuationModel, label: &str| -> Result<Vec<f64>, String> {
            let samples = measure(model, &groups[..count])?;
            write_latency(out, label, &samples)?;
            Ok(samples)
        };
        let candle_ms = latency(&candle, "candle   ")?;
        let fp32_ms = latency(&fp32, "ort-fp32 ")?;
        let int8_ms = latency(&int8, "ort-int8 ")?;
        let fp32_ratio = ratio(p50(&candle_ms), p50(&fp32_ms));
        let int8_ratio = ratio(p50(&candle_ms), p50(&int8_ms));
        writeln!(
            out,
            "candle／ort-fp32 p50 比 {fp32_ratio:.2}x，candle／ort-int8 p50 比 {int8_ratio:.2}x"
        )
        .map_err(io)?;
        latency_stats = Some((fp32_ratio, int8_ratio));
    }

    writeln!(out, "\n結論").map_err(io)?;
    // 對照基準是同一份 fp32 權重；int8 的差異來自量化，不是後端。
    let logits_ok = fp32_stats.max_diff < 1e-3;
    let order_ok = fp32_stats.agree == fp32_stats.total;
    let output_ok = punct_fp32
        .as_ref()
        .is_none_or(|stats| stats.changed_mismatch == 0 && stats.char_mismatch == 0);
    writeln!(
        out,
        "logits < 1e-3（vs ort-fp32）：{}（max {:.3e}）",
        verdict(logits_ok),
        fp32_stats.max_diff
    )
    .map_err(io)?;
    writeln!(
        out,
        "argmax 一致（vs ort-fp32）：{}（{}/{}）",
        verdict(order_ok),
        fp32_stats.agree,
        fp32_stats.total
    )
    .map_err(io)?;
    writeln!(out, "標點字元一致（vs ort-fp32）：{}", verdict(output_ok)).map_err(io)?;
    writeln!(
        out,
        "附註：candle 與 ort-int8 的 argmax 差 {}/{}，這是 int8 量化誤差；ORT fp32 本身的 logits 與 int8 也差 {:.3e}。",
        int8_stats.total - int8_stats.agree,
        int8_stats.total,
        int8_stats.max_diff
    )
    .map_err(io)?;
    if let Some((fp32_ratio, int8_ratio)) = latency_stats {
        writeln!(
            out,
            "延遲 < 1.2x ORT：{}（vs fp32 {fp32_ratio:.2}x，vs int8 {int8_ratio:.2}x）",
            verdict(fp32_ratio <= 1.2 && int8_ratio <= 1.2)
        )
        .map_err(io)?;
    }
    Ok(())
}

fn io(err: std::io::Error) -> String {
    format!("寫出結果失敗：{err}")
}

fn verdict(ok: bool) -> &'static str {
    if ok {
        "PASS"
    } else {
        "FAIL"
    }
}

fn read_groups(path: &Path, groups: usize, seq: usize) -> Result<Vec<Vec<i64>>, String> {
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
    let available = values.len() / seq;
    let groups = groups.min(available);
    if groups == 0 {
        return Err(format!("{}：不足一個 {seq}-token 視窗", path.display()));
    }
    Ok(values[..groups * seq]
        .chunks_exact(seq)
        .map(|window| window.iter().map(|id| i64::from(*id)).collect())
        .collect())
}

struct DiffStats {
    max_diff: f32,
    agree: usize,
    total: usize,
}

fn compare(
    candle: &PunctuationModel,
    reference: &PunctuationModel,
    groups: &[Vec<i64>],
    label: &str,
    out: &mut dyn Write,
) -> Result<DiffStats, String> {
    let mut max_diff = 0f32;
    let mut sum = 0f64;
    let mut count = 0usize;
    let mut agree = 0usize;
    let mut total = 0usize;
    for (index, ids) in groups.iter().enumerate() {
        let got = candle
            .logits_for_ids(ids)
            .ok_or_else(|| format!("candle 第 {index} 組失敗"))?;
        let want = reference
            .logits_for_ids(ids)
            .ok_or_else(|| format!("{label} 第 {index} 組失敗"))?;
        if got.len() != want.len() {
            return Err(format!(
                "第 {index} 組長度不同：{} vs {}",
                got.len(),
                want.len()
            ));
        }
        for (row_got, row_want) in got.iter().zip(&want) {
            for (a, b) in row_got.iter().zip(row_want) {
                let diff = (a - b).abs();
                max_diff = max_diff.max(diff);
                sum += f64::from(diff);
                count += 1;
            }
            let argmax = |row: &Vec<f32>| -> usize {
                row.iter()
                    .enumerate()
                    .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(index, _)| index)
                    .unwrap_or(0)
            };
            total += 1;
            if argmax(row_got) == argmax(row_want) {
                agree += 1;
            }
        }
    }
    writeln!(
        out,
        "{label}：max |Δ| {max_diff:.3e}，mean |Δ| {:.3e}，argmax 一致 {agree}/{total}",
        sum / count as f64
    )
    .map_err(io)?;
    Ok(DiffStats {
        max_diff,
        agree,
        total,
    })
}

struct PunctStats {
    documents: usize,
    changed_mismatch: usize,
    char_mismatch: usize,
    chars: usize,
    inserted: usize,
}

fn punct_agreement(
    candle: &PunctuationModel,
    reference: &PunctuationModel,
    texts: &[String],
    label: &str,
    out: &mut dyn Write,
) -> Result<PunctStats, String> {
    let mut stats = PunctStats {
        documents: texts.len(),
        changed_mismatch: 0,
        char_mismatch: 0,
        chars: 0,
        inserted: 0,
    };
    for text in texts {
        let reading = strip_punctuation(text);
        let a = candle.punctuate(&reading);
        let b = reference.punctuate(&reading);
        if a != b {
            stats.changed_mismatch += 1;
        }
        if a != reading {
            stats.inserted += 1;
        }
        let a_chars: Vec<char> = a.chars().collect();
        let b_chars: Vec<char> = b.chars().collect();
        stats.chars += a_chars.len().max(b_chars.len());
        stats.char_mismatch += a_chars.iter().zip(&b_chars).filter(|(x, y)| x != y).count();
        stats.char_mismatch += a_chars.len().abs_diff(b_chars.len());
    }
    let same = stats.documents - stats.changed_mismatch;
    writeln!(
        out,
        "{label}：字元一致 {}/{}（{:.2}%），整句相同 {}/{}，candle 插入 {}/{} 篇",
        stats.chars - stats.char_mismatch,
        stats.chars,
        100.0 * (stats.chars - stats.char_mismatch) as f64 / stats.chars.max(1) as f64,
        same,
        stats.documents,
        stats.inserted,
        stats.documents
    )
    .map_err(io)?;
    Ok(stats)
}

/// 移除題庫 surface 的既有標點，讓模型自己決定插入位置。
fn strip_punctuation(text: &str) -> String {
    text.chars()
        .filter(|ch| !matches!(ch, '、' | '。' | '？' | '，' | '！'))
        .collect()
}

fn read_texts(path: &Path) -> Result<Vec<String>, String> {
    let raw = std::fs::read_to_string(path).map_err(|err| format!("{}：{err}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&raw).map_err(|err| format!("{}：{err}", path.display()))?;
    let items = value
        .get("items")
        .and_then(|items| items.as_array())
        .ok_or_else(|| format!("{}：沒有 items", path.display()))?;
    items
        .iter()
        .map(|item| {
            item.get("surface")
                .and_then(|surface| surface.as_str())
                .map(str::to_string)
                .ok_or_else(|| format!("{}：item 沒有 surface", path.display()))
        })
        .collect()
}

fn measure(model: &PunctuationModel, groups: &[Vec<i64>]) -> Result<Vec<f64>, String> {
    let mut samples = Vec::with_capacity(groups.len());
    for (index, ids) in groups.iter().enumerate() {
        let started = Instant::now();
        model
            .logits_for_ids(ids)
            .ok_or_else(|| format!("延遲量測第 {index} 組失敗"))?;
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(samples)
}

fn write_latency(out: &mut dyn Write, label: &str, samples: &[f64]) -> Result<(), String> {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    writeln!(
        out,
        "{label} mean {:.2} ms，p50 {:.2} ms，p95 {:.2} ms，min {:.2} ms",
        sorted.iter().sum::<f64>() / sorted.len() as f64,
        p50(&sorted),
        sorted[((sorted.len() - 1) as f64 * 0.95).round() as usize],
        sorted[0]
    )
    .map_err(io)
}

fn p50(sorted: &[f64]) -> f64 {
    sorted[((sorted.len() - 1) as f64 * 0.5).round() as usize]
}

fn ratio(a: f64, b: f64) -> f64 {
    if b == 0.0 {
        0.0
    } else {
        a / b
    }
}
