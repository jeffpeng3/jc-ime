//! 注音選字模型的 ORT／candle 對照與延遲量測。
//!
//! 用 fuzzy 題庫的注音題組出 N-best 候選，對同一份 `prepare()` 結果跑三個
//! 後端：ORT fp32、candle fp32、candle Q8_0，比對每個候選的模型分數、排序
//! 一致率與單次推論延遲。
//!
//! ```sh
//! # 第一次：把 ONNX 轉成 candle 的 fp32 safetensors，再匯出 Q8_0
//! cargo run -p jc-cli --release --bin candle-zh-check -- --convert --quant q8_0
//! # 量測（ORT 需要 ORT_DYLIB_PATH）
//! ORT_DYLIB_PATH=/path/libonnxruntime.so \
//!   cargo run -p jc-cli --release --bin candle-zh-check -- --groups 200
//! ```
//!
//! `--only quant` 只載入 Q8_0，用來看單一權重檔的常駐記憶體；三個模型同時
//! 載入時 VmRSS 會疊加，不能拿來比單一後端。
//!
//! 門檻：candle fp32 對 ORT fp32 的 `max |Δ| < 1e-3`、逐對排序一致
//! `>= 98%`、單次推論 p50 `<= 30 ms`。不通過時 exit code 1。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use jc_core::zh::select::{prepare, Request, SelectManifest, Vocabulary};
use jc_model::candle::zh::{convert_onnx, export_gguf, weights_path, ZhInput, ZhSelect};
use jc_model::candle::{QuantDtype, QuantZhSelect};
use jc_model::Backend;
use serde::Deserialize;

/// 預設模型目錄。
const DEFAULT_MODEL_DIR: &str = "/mnt/work/jc/jc-ime-data/models/zh-select";
/// 預設 dev 題庫（fuzzy 注音題）。
const DEFAULT_DEV: &str = "/mnt/work/jc/jc-ime-data/0c-testdata/fuzzy/fuzzy-input.json";
/// 預設注音詞庫。
const DEFAULT_DATA: &str = "/mnt/work/jc/jc-ime-data/build-zh";
/// logits 誤差上限。
const MAX_ABS_DIFF: f64 = 1e-3;
/// 排序一致率下限。
const MIN_RANK_AGREEMENT: f64 = 0.98;
/// 單次推論 p50 上限，毫秒。
const MAX_P50_MS: f64 = 30.0;
/// 每組最多幾個候選，與模型上限相同。
const DEFAULT_K: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Only {
    All,
    Fp32,
    Quant,
}

#[derive(Debug, Clone)]
struct Args {
    model_dir: PathBuf,
    dev: PathBuf,
    data: PathBuf,
    groups: usize,
    k: usize,
    convert: bool,
    quant: QuantDtype,
    only: Only,
    ort_lib: Option<PathBuf>,
    latency: usize,
    /// 只載入一個權重檔並跑固定輸入，用來量單一模型的常駐記憶體。
    load_only: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            model_dir: PathBuf::from(DEFAULT_MODEL_DIR),
            dev: PathBuf::from(DEFAULT_DEV),
            data: PathBuf::from(DEFAULT_DATA),
            groups: 200,
            k: DEFAULT_K,
            convert: false,
            quant: QuantDtype::Q8_0,
            only: Only::All,
            ort_lib: None,
            latency: 200,
            load_only: false,
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("candle-zh-check：{err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool, String> {
    let args = parse(std::env::args().skip(1).collect())?;
    let onnx = args.model_dir.join("model.onnx");
    let weights = weights_path(&args.model_dir);

    if args.convert {
        let report = convert_onnx(&onnx, &args.model_dir.join("model.safetensors"))
            .map_err(|err| format!("ONNX 轉檔失敗：{err}"))?;
        println!(
            "轉檔 {} -> {}：{} 個張量、{} 個參數、{:.1} MiB",
            report.source.display(),
            report.path.display(),
            report.tensors,
            report.params,
            report.bytes as f64 / 1024.0 / 1024.0
        );
        let gguf = args.model_dir.join(format!("model.{}.gguf", args.quant));
        let report = export_gguf(&report.path, &gguf, args.quant)
            .map_err(|err| format!("GGUF 匯出失敗：{err}"))?;
        println!(
            "量化 {} -> {}：{} 個張量、{:.1} MiB、{:.3} bits/weight",
            args.quant,
            report.path.display(),
            report.entries.len(),
            report.file_bytes as f64 / 1024.0 / 1024.0,
            report.quantized_bits_per_weight()
        );
        // 轉檔與量測分開跑，避免沒設 ORT_DYLIB_PATH 時多一個誤導的錯誤。
        return Ok(true);
    }
    let weights = weights_path(&args.model_dir).or(weights).ok_or_else(|| {
        format!(
            "{} 沒有 candle 權重；先跑 --convert",
            args.model_dir.display()
        )
    })?;

    if args.load_only {
        return load_only(&args, &weights);
    }

    let groups = build_groups(&args)?;
    if groups.is_empty() {
        return Err("題庫沒有 >=2 音節的注音題".into());
    }
    let candidates: usize = groups.iter().map(|group| group.valid.len()).sum();
    println!(
        "groups {}，有效候選 {}（每組最多 {}）",
        groups.len(),
        candidates,
        args.k
    );

    let vocab = Vocabulary::load(args.model_dir.join("vocab.json"))
        .map_err(|err| format!("讀取 vocab.json 失敗：{err}"))?;
    let manifest = SelectManifest::load(args.model_dir.join("manifest.json"))
        .map_err(|err| format!("讀取 manifest.json 失敗：{err}"))?;

    // 先把每組的 Arrays 準備好；ORT 與 candle 吃完全相同的輸入。
    let mut prepared = Vec::with_capacity(groups.len());
    for group in &groups {
        let texts: Vec<&str> = group
            .valid
            .iter()
            .map(|index| group.candidates[*index].text.as_str())
            .collect();
        let arrays = prepare(
            &vocab,
            &manifest,
            &Request {
                context: "",
                reading: &group.syllables,
                candidates: &texts,
            },
        )
        .map_err(|err| format!("prepare 失敗（{}）：{err}", group.id))?;
        prepared.push(arrays);
    }

    let baseline_rss = vm_rss();
    let mut pass = true;

    let fp32_path = if weights.extension().and_then(|ext| ext.to_str()) == Some("gguf") {
        args.model_dir.join("model.safetensors")
    } else {
        weights.clone()
    };
    let quant_path = {
        let candidate = args.model_dir.join(format!("model.{}.gguf", args.quant));
        if candidate.is_file() {
            candidate
        } else {
            weights.clone()
        }
    };

    let mut candle_fp32: Option<ZhSelect> = None;
    if args.only != Only::Quant {
        let path = fp32_path;
        if path.is_file() {
            let started = Instant::now();
            let model =
                ZhSelect::load(&path).map_err(|err| format!("載入 candle fp32 失敗：{err}"))?;
            println!(
                "candle fp32 {} 載入 {:.1} ms，VmRSS {:.1} MiB（基準 {:.1} MiB）",
                path.display(),
                started.elapsed().as_secs_f64() * 1000.0,
                vm_rss() as f64 / 1024.0,
                baseline_rss as f64 / 1024.0
            );
            candle_fp32 = Some(model);
        }
    }
    let loaded_rss = vm_rss();
    let mut candle_quant: Option<QuantZhSelect> = None;
    if args.only != Only::Fp32 {
        let path = quant_path;
        if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("gguf") {
            let started = Instant::now();
            let model = QuantZhSelect::load(&path)
                .map_err(|err| format!("載入 candle {} 失敗：{err}", args.quant))?;
            println!(
                "candle {} {} 載入 {:.1} ms，VmRSS {:.1} MiB（基準 {:.1} MiB）",
                args.quant,
                path.display(),
                started.elapsed().as_secs_f64() * 1000.0,
                vm_rss() as f64 / 1024.0,
                baseline_rss as f64 / 1024.0
            );
            candle_quant = Some(model);
        }
    }

    let mut ort_fp32 = None;
    if args.only == Only::All {
        jc_model::set_default_backend(Backend::Ort);
        let model =
            jc_core::zh::select::SelectModel::load(&args.model_dir, args.ort_lib.as_deref())
                .map_err(|err| format!("載入 ORT fp32 失敗：{err}"))?;
        ort_fp32 = Some(model);
    }

    // 分數：同一組候選、同一份 Arrays。
    let mut rows: HashMap<&str, Vec<Vec<f64>>> = HashMap::new();
    if let Some(model) = &ort_fp32 {
        let mut scores = Vec::with_capacity(groups.len());
        for (group, arrays) in groups.iter().zip(&prepared) {
            scores.push(run_ort(model, group, arrays)?);
        }
        rows.insert("ort fp32", scores);
    }
    if let Some(model) = &candle_fp32 {
        let mut scores = Vec::with_capacity(groups.len());
        for (group, arrays) in groups.iter().zip(&prepared) {
            scores.push(run_candle_fp32(model, group, arrays)?);
        }
        rows.insert("candle fp32", scores);
    }
    if let Some(model) = &candle_quant {
        let mut scores = Vec::with_capacity(groups.len());
        for (group, arrays) in groups.iter().zip(&prepared) {
            scores.push(run_candle_quant(model, group, arrays)?);
        }
        rows.insert("candle quant", scores);
    }

    let reference = rows.get("ort fp32").cloned();
    if let Some(reference) = &reference {
        for name in ["candle fp32", "candle quant"] {
            let Some(scores) = rows.get(name) else {
                continue;
            };
            let stats = compare(reference, scores);
            println!(
                "{name} vs ort fp32：max |Δ| {:.3e}，mean |Δ| {:.3e}，\
                 逐對排序一致 {}/{}（{:.2}%），top-1 {}/{}",
                stats.max_abs,
                stats.mean_abs,
                stats.pairs_agree,
                stats.pairs,
                stats.rank_agreement() * 100.0,
                stats.top1_agree,
                groups.len()
            );
            if name == "candle fp32" {
                if stats.max_abs >= MAX_ABS_DIFF {
                    println!("FAIL：candle fp32 的 max |Δ| 超過 {MAX_ABS_DIFF:e}");
                    pass = false;
                }
                if stats.rank_agreement() < MIN_RANK_AGREEMENT {
                    println!("FAIL：排序一致率低於 {:.0}%", MIN_RANK_AGREEMENT * 100.0);
                    pass = false;
                }
            }
        }
    } else if let (Some(a), Some(b)) = (rows.get("candle fp32"), rows.get("candle quant")) {
        let stats = compare(a, b);
        println!(
            "candle quant vs candle fp32：max |Δ| {:.3e}，mean |Δ| {:.3e}，\
             逐對排序一致 {}/{}（{:.2}%），top-1 {}/{}",
            stats.max_abs,
            stats.mean_abs,
            stats.pairs_agree,
            stats.pairs,
            stats.rank_agreement() * 100.0,
            stats.top1_agree,
            groups.len()
        );
    }

    // 延遲：每次一個 forward，與 `eval zh-golden` 的統計同尺度。
    let limit = groups.len().min(args.latency);
    println!("延遲（{limit} 次單組推論，毫秒）");
    if let Some(model) = &candle_fp32 {
        let durations = measure(limit, |index| {
            run_candle_fp32(model, &groups[index], &prepared[index]).map(|_| ())
        })?;
        report_latency("candle fp32", &durations);
        if percentile(&durations, 50.0) > MAX_P50_MS {
            println!("FAIL：candle fp32 的 p50 超過 {MAX_P50_MS} ms");
            pass = false;
        }
    }
    if let Some(model) = &candle_quant {
        let durations = measure(limit, |index| {
            run_candle_quant(model, &groups[index], &prepared[index]).map(|_| ())
        })?;
        report_latency(&format!("candle {}", args.quant), &durations);
        if percentile(&durations, 50.0) > MAX_P50_MS {
            println!("FAIL：candle {} 的 p50 超過 {MAX_P50_MS} ms", args.quant);
            pass = false;
        }
    }
    if let Some(model) = &ort_fp32 {
        let durations = measure(limit, |index| {
            run_ort(model, &groups[index], &prepared[index]).map(|_| ())
        })?;
        report_latency("ort fp32", &durations);
    }

    if args.only != Only::All {
        println!(
            "VmRSS 量測：基準 {:.1} MiB，載入後 {:.1} MiB，{} 次推論後 {:.1} MiB",
            baseline_rss as f64 / 1024.0,
            loaded_rss as f64 / 1024.0,
            limit,
            vm_rss() as f64 / 1024.0
        );
    }

    println!(
        "{}：max |Δ| < {MAX_ABS_DIFF:e}、排序一致 >= {:.0}%、p50 <= {MAX_P50_MS} ms",
        if pass { "PASS" } else { "FAIL" },
        MIN_RANK_AGREEMENT * 100.0
    );
    Ok(pass)
}

/// 單一權重檔的常駐記憶體：不載入詞庫，只用固定輸入跑 `--latency` 次。
fn load_only(args: &Args, weights: &Path) -> Result<bool, String> {
    let baseline = vm_rss();
    let input = fixed_input();
    let quant_path = {
        let candidate = args.model_dir.join(format!("model.{}.gguf", args.quant));
        if candidate.is_file() {
            candidate
        } else {
            weights.to_path_buf()
        }
    };
    match args.only {
        Only::Fp32 => {
            let model = ZhSelect::load(weights).map_err(|err| format!("載入失敗：{err}"))?;
            print_rss("candle fp32", baseline, &weights.display().to_string());
            for _ in 0..args.latency {
                model
                    .scores(&input)
                    .map_err(|err| format!("推論失敗：{err}"))?;
            }
        }
        Only::Quant => {
            let model =
                QuantZhSelect::load(&quant_path).map_err(|err| format!("載入失敗：{err}"))?;
            print_rss(
                &format!("candle {}", args.quant),
                baseline,
                &quant_path.display().to_string(),
            );
            for _ in 0..args.latency {
                model
                    .scores(&input)
                    .map_err(|err| format!("推論失敗：{err}"))?;
            }
        }
        Only::All => return Err("--load-only 需要搭配 --only fp32 或 --only quant".into()),
    }
    println!(
        "  {} 次推論後 VmRSS {:.1} MiB（相對基準 +{:.1} MiB）",
        args.latency,
        vm_rss() as f64 / 1024.0,
        (vm_rss().saturating_sub(baseline)) as f64 / 1024.0
    );
    Ok(true)
}

fn print_rss(name: &str, baseline: u64, path: &str) {
    let rss = vm_rss();
    println!(
        "{name} {path}：基準 {:.1} MiB，載入後 {:.1} MiB（+{:.1} MiB）",
        baseline as f64 / 1024.0,
        rss as f64 / 1024.0,
        (rss.saturating_sub(baseline)) as f64 / 1024.0
    );
}

/// 固定輸入（`prepare()` 的 basic 案例），不依賴詞庫。
fn fixed_input() -> ZhInput<'static> {
    static IDS: [i64; 9] = [1, 8, 9, 2, 0, 0, 3, 10, 12];
    static BASES: [i64; 9] = [0, 0, 0, 0, 2, 3, 0, 0, 0];
    static TONES: [i64; 9] = [0, 0, 0, 0, 1, 1, 0, 0, 0];
    static TYPES: [i64; 9] = [0, 0, 0, 0, 1, 1, 0, 2, 2];
    static POSITIONS: [i64; 9] = [0, 1, 2, 3, 4, 5, 6, 7, 7];
    static MASK: [bool; 81] = build_fixed_mask();
    static PROJ: [i64; 3] = [6, 7, 8];
    static NODE: [i64; 4] = [0, 1, 0, 2];
    static TOKENS: [i64; 4] = [4, 5, 6, 12];
    ZhInput {
        seq: 9,
        ids: &IDS,
        bases: &BASES,
        tones: &TONES,
        types: &TYPES,
        positions: &POSITIONS,
        mask: &MASK,
        proj_pos: &PROJ,
        target_node: &NODE,
        target_ids: &TOKENS,
        rows: 2,
        cols: 2,
    }
}

const fn build_fixed_mask() -> [bool; 81] {
    let mut mask = [false; 81];
    let mut q = 0;
    while q < 5 {
        let mut a = 0;
        while a <= q {
            mask[q * 9 + a] = true;
            a += 1;
        }
        q += 1;
    }
    while q < 9 {
        let mut a = 0;
        while a < 5 {
            mask[q * 9 + a] = true;
            a += 1;
        }
        q += 1;
    }
    mask[5 * 9 + 5] = true;
    mask[6 * 9 + 5] = true;
    mask[6 * 9 + 6] = true;
    mask[7 * 9 + 5] = true;
    mask[7 * 9 + 6] = true;
    mask[7 * 9 + 7] = true;
    mask[8 * 9 + 5] = true;
    mask[8 * 9 + 6] = true;
    mask[8 * 9 + 8] = true;
    mask
}

fn run_ort(
    model: &jc_core::zh::select::SelectModel,
    group: &Group,
    _arrays: &jc_core::zh::select::Arrays,
) -> Result<Vec<f64>, String> {
    let texts: Vec<&str> = group
        .valid
        .iter()
        .map(|index| group.candidates[*index].text.as_str())
        .collect();
    model
        .score("", &group.syllables, &texts)
        .map_err(|err| format!("ORT 推論失敗：{err}"))
}

/// candle 的 `ZhInput` 借用整平的 `target_*`；用這個容器把擁有權留在呼叫端。
struct OwnedTargets {
    target_node: Vec<i64>,
    target_ids: Vec<i64>,
}

impl OwnedTargets {
    fn new(arrays: &jc_core::zh::select::Arrays) -> Self {
        Self {
            target_node: arrays.target_node.iter().flatten().copied().collect(),
            target_ids: arrays.target_ids.iter().flatten().copied().collect(),
        }
    }

    fn view<'a>(&'a self, arrays: &'a jc_core::zh::select::Arrays) -> ZhInput<'a> {
        ZhInput {
            seq: arrays.len(),
            ids: &arrays.ids,
            bases: &arrays.bases,
            tones: &arrays.tones,
            types: &arrays.types,
            positions: &arrays.positions,
            mask: &arrays.mask,
            proj_pos: &arrays.proj_pos,
            target_node: &self.target_node,
            target_ids: &self.target_ids,
            rows: arrays.rows(),
            cols: arrays.target_node.first().map(Vec::len).unwrap_or(0),
        }
    }
}

fn run_candle_fp32(
    model: &ZhSelect,
    group: &Group,
    arrays: &jc_core::zh::select::Arrays,
) -> Result<Vec<f64>, String> {
    let targets = OwnedTargets::new(arrays);
    let scores = model
        .scores(&targets.view(arrays))
        .map_err(|err| format!("candle fp32 推論失敗：{err}"))?;
    Ok(reduce(&scores, group.valid.len()))
}

fn run_candle_quant(
    model: &QuantZhSelect,
    group: &Group,
    arrays: &jc_core::zh::select::Arrays,
) -> Result<Vec<f64>, String> {
    let targets = OwnedTargets::new(arrays);
    let scores = model
        .scores(&targets.view(arrays))
        .map_err(|err| format!("candle 推論失敗：{err}"))?;
    Ok(reduce(&scores, group.valid.len()))
}

/// 與 `SelectModel::score` 相同的分支合併：同一個候選取最大值。
fn reduce(scores: &[f32], count: usize) -> Vec<f64> {
    let mut out = vec![f64::NEG_INFINITY; count];
    for (row, value) in scores.iter().enumerate() {
        let index = row % count;
        if f64::from(*value) > out[index] {
            out[index] = f64::from(*value);
        }
    }
    out
}

fn measure(
    limit: usize,
    mut run: impl FnMut(usize) -> Result<(), String>,
) -> Result<Vec<f64>, String> {
    let mut durations = Vec::with_capacity(limit);
    for index in 0..limit {
        let started = Instant::now();
        run(index)?;
        durations.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(durations)
}

fn report_latency(name: &str, durations: &[f64]) {
    let mean = durations.iter().sum::<f64>() / durations.len() as f64;
    println!(
        "  {name:<12} mean {mean:.2}，p50 {:.2}，p95 {:.2}，min {:.2}，max {:.2}",
        percentile(durations, 50.0),
        percentile(durations, 95.0),
        durations.iter().cloned().fold(f64::INFINITY, f64::min),
        durations.iter().cloned().fold(0.0, f64::max)
    );
}

fn percentile(values: &[f64], value: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = (value / 100.0 * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

#[derive(Default)]
struct CompareStats {
    max_abs: f64,
    mean_abs: f64,
    pairs: usize,
    pairs_agree: usize,
    top1_agree: usize,
}

impl CompareStats {
    fn rank_agreement(&self) -> f64 {
        if self.pairs == 0 {
            return 1.0;
        }
        self.pairs_agree as f64 / self.pairs as f64
    }
}

fn compare(reference: &[Vec<f64>], other: &[Vec<f64>]) -> CompareStats {
    let mut stats = CompareStats::default();
    let mut total = 0.0;
    let mut count = 0usize;
    for (want, got) in reference.iter().zip(other) {
        for (want, got) in want.iter().zip(got) {
            let diff = (want - got).abs();
            stats.max_abs = stats.max_abs.max(diff);
            total += diff;
            count += 1;
        }
        for i in 0..want.len() {
            for j in (i + 1)..want.len() {
                stats.pairs += 1;
                let order = want[i].partial_cmp(&want[j]);
                let other_order = got[i].partial_cmp(&got[j]);
                if order == other_order {
                    stats.pairs_agree += 1;
                }
            }
        }
        let top = |values: &[f64]| {
            values
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(index, _)| index)
        };
        if top(want) == top(got) {
            stats.top1_agree += 1;
        }
    }
    stats.mean_abs = if count == 0 {
        0.0
    } else {
        total / count as f64
    };
    stats
}

fn vm_rss() -> u64 {
    let Ok(text) = std::fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            if let Some(value) = rest.split_whitespace().next() {
                return value.parse().unwrap_or(0);
            }
        }
    }
    0
}

#[derive(Debug)]
struct Group {
    id: String,
    syllables: Vec<String>,
    candidates: Vec<Candidate>,
    valid: Vec<usize>,
}

#[derive(Debug)]
struct Candidate {
    text: String,
}

#[derive(Deserialize)]
struct FuzzyFile {
    items: Vec<FuzzyItem>,
}

#[derive(Deserialize)]
struct FuzzyItem {
    id: String,
    language: String,
    reading: String,
    surface: String,
}

/// 與 `zh-calibrate` 相同的 dev 組來源：fuzzy 的注音題展開近音變體後取
/// N-best。候選只保留字數與音節數相同、字元都在詞表內的。
fn build_groups(args: &Args) -> Result<Vec<Group>, String> {
    use jc_core::decode::{Decoder, Locks};
    use jc_core::lattice::Lattice;
    use jc_core::zh::fuzzy::{Config as FuzzyConfig, Mode as FuzzyMode};

    let raw = std::fs::read(&args.dev)
        .map_err(|err| format!("讀取 {} 失敗：{err}", args.dev.display()))?;
    let file: FuzzyFile =
        serde_json::from_slice(&raw).map_err(|err| format!("解析題庫失敗：{err}"))?;
    let resources = jc_cli::data::load_zh_bin_resources(&args.data)
        .map_err(|err| format!("載入注音資料包失敗：{err}"))?;
    let vocab = Vocabulary::load(args.model_dir.join("vocab.json"))
        .map_err(|err| format!("讀取 vocab.json 失敗：{err}"))?;
    let fuzzy = FuzzyConfig::all(FuzzyMode::Select);

    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut groups = Vec::new();
    for item in file.items {
        if item.language != "zh" {
            continue;
        }
        let syllables = jc_core::zh::syllable::parse_reading(&item.reading);
        if syllables.len() < 2 || syllables.len() > 32 {
            continue;
        }
        if !seen.insert((item.reading.clone(), item.surface.clone())) {
            continue;
        }
        let readings: Vec<String> = syllables
            .iter()
            .map(|syllable| syllable.as_str().to_string())
            .collect();
        let mut best: BTreeMap<String, f64> = BTreeMap::new();
        for variant in fuzzy.variants(&syllables) {
            let readings: Vec<String> = variant
                .syllables
                .iter()
                .map(|syllable| syllable.as_str().to_string())
                .collect();
            let Ok(lattice) = Lattice::build(&readings, &resources.lexicon, &resources.ctx) else {
                continue;
            };
            let decoder = Decoder::new(&resources.lexicon, &resources.ctx);
            let (hypotheses, _) = decoder.nbest(&lattice, &Locks::new(), false, args.k);
            for hypothesis in hypotheses {
                let cost =
                    f64::from(hypothesis.cost) + f64::from(variant.cost_tenths as i32) / 10.0;
                best.entry(hypothesis.text)
                    .and_modify(|old| *old = old.min(cost))
                    .or_insert(cost);
            }
        }
        let mut ranked: Vec<(String, f64)> = best.into_iter().collect();
        ranked.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        ranked.truncate(args.k);
        let candidates: Vec<Candidate> = ranked
            .into_iter()
            .map(|(text, _)| Candidate { text })
            .collect();
        let valid: Vec<usize> = candidates
            .iter()
            .enumerate()
            .filter(|(_, candidate)| {
                let chars: Vec<char> = candidate.text.chars().collect();
                chars.len() == readings.len() && chars.iter().all(|ch| vocab.char_id(*ch).is_some())
            })
            .map(|(index, _)| index)
            .collect();
        if valid.len() < 2 || valid.len() > args.k {
            continue;
        }
        groups.push(Group {
            id: item.id,
            syllables: readings,
            candidates,
            valid,
        });
        if groups.len() >= args.groups {
            break;
        }
    }
    Ok(groups)
}

fn parse(args: Vec<String>) -> Result<Args, String> {
    let mut options = Args::default();
    let mut index = 0;
    let value = |index: usize, flag: &str| -> Result<String, String> {
        args.get(index + 1)
            .cloned()
            .ok_or_else(|| format!("{flag} 需要一個值"))
    };
    while index < args.len() {
        match args[index].as_str() {
            "--model-dir" => {
                options.model_dir = PathBuf::from(value(index, "--model-dir")?);
                index += 1;
            }
            "--dev" => {
                options.dev = PathBuf::from(value(index, "--dev")?);
                index += 1;
            }
            "--data" => {
                options.data = PathBuf::from(value(index, "--data")?);
                index += 1;
            }
            "--groups" => {
                options.groups = value(index, "--groups")?
                    .parse()
                    .map_err(|_| "--groups 需要正整數".to_string())?;
                index += 1;
            }
            "--k" => {
                options.k = value(index, "--k")?
                    .parse()
                    .map_err(|_| "--k 需要正整數".to_string())?;
                index += 1;
            }
            "--latency" => {
                options.latency = value(index, "--latency")?
                    .parse()
                    .map_err(|_| "--latency 需要正整數".to_string())?;
                index += 1;
            }
            "--convert" => options.convert = true,
            "--load-only" => options.load_only = true,
            "--quant" => {
                options.quant = value(index, "--quant")?
                    .parse()
                    .map_err(|err: String| err)?;
                index += 1;
            }
            "--only" => {
                options.only = match value(index, "--only")?.as_str() {
                    "all" => Only::All,
                    "fp32" => Only::Fp32,
                    "quant" => Only::Quant,
                    other => return Err(format!("--only 只接受 all、fp32、quant，實際為 {other}")),
                };
                index += 1;
            }
            "--ort-lib" => {
                options.ort_lib = Some(PathBuf::from(value(index, "--ort-lib")?));
                index += 1;
            }
            other => return Err(format!("不認得的參數 {other}")),
        }
        index += 1;
    }
    Ok(options)
}
