//! 統計轉換與 Jinen 的離線門檻實驗。
//!
//! 對每題同時產生統計（Viterbi）與 Jinen 的結果，記錄五個**輸入側**訊號與
//! 兩邊是否命中，再離線掃描「什麼時候改用 Jinen」。這裡不接 `Session`，
//! 只產生分析用的資料與報表。
//!
//! ```sh
//! cargo run -p jc-cli --release --bin jinen-gate -- collect --corpus modern-500
//! cargo run -p jc-cli --release --bin jinen-gate -- sweep
//! ```
//!
//! `collect` 寫出 `target/jinen-gate/<corpus>.json`；`sweep` 讀回三份結果，
//! 只用輸入側訊號掃門檻。門檻在每組題庫的 dev 半邊挑選，holdout 半邊只
//! 回報、不參與選擇；用答案逐題挑最好結果的數字另外標成「上限參考」。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use jc_core::ja::decode::{JaContext, JaDecoder, JaResult};
use jc_model::candle::jinen::{self, Jinen};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// 題庫目錄。
const DEFAULT_TESTDATA: &str = "/mnt/work/jc/jc-ime-data/0c-testdata/ja";
/// 資料根目錄（`packs/` 所在處）。
const DEFAULT_DATA_ROOT: &str = "/mnt/work/jc/jc-ime-data";
/// 外部模型目錄。
const DEFAULT_MODEL_DATA: &str = "/mnt/work/jc/jc-ime-data/models/external";
/// 預設輸出目錄。
const DEFAULT_OUT_DIR: &str = "target/jinen-gate";

#[derive(Debug, Clone)]
enum Mode {
    Collect,
    Sweep,
}

#[derive(Debug, Clone)]
struct Args {
    mode: Mode,
    corpus: String,
    data_root: PathBuf,
    testdata: PathBuf,
    gguf: Option<PathBuf>,
    tokenizer: Option<PathBuf>,
    limit: Option<usize>,
    max_new_tokens: usize,
    threads: usize,
    dir: PathBuf,
    out: Option<PathBuf>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            mode: Mode::Collect,
            corpus: "ajimee-200".to_string(),
            data_root: PathBuf::from(DEFAULT_DATA_ROOT),
            testdata: PathBuf::from(DEFAULT_TESTDATA),
            gguf: None,
            tokenizer: None,
            limit: None,
            max_new_tokens: jinen::DEFAULT_MAX_NEW_TOKENS,
            threads: 4,
            dir: PathBuf::from(DEFAULT_OUT_DIR),
            out: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct CorpusFile {
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

/// 五個輸入側訊號；全部只看讀音、詞典與統計結果。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Signals {
    /// top-1 與 top-2 的總成本差（Mozc 原始整數）；只有一條路徑時是 `None`。
    gap: Option<i32>,
    /// lattice 中未知詞節點佔全部節點的比例。
    unknown_ratio: f64,
    /// 讀音長度（字元數）。
    reading_len: usize,
    /// top-1 路徑上每個詞群的平均候選數。
    mean_candidates: f64,
    /// top-1 路徑上候選數 > 1 的詞群比例。
    ambiguous_ratio: f64,
    /// 單一詞群的最大候選數。
    max_candidates: usize,
}

/// Mozkey 的三條輸出檢查結果；`true` 表示通過。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Checks {
    /// 沒有明顯變短或首尾截斷。
    length_ok: bool,
    /// 沒有重複左文脈。
    echo_ok: bool,
    /// 沒有增刪絵文字。
    emoji_ok: bool,
}

impl Checks {
    fn all(&self) -> bool {
        self.length_ok && self.echo_ok && self.emoji_ok
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Row {
    id: String,
    reading: String,
    surface: String,
    accept: Vec<String>,
    stats: String,
    stats_hit: bool,
    jinen: String,
    jinen_hit: bool,
    jinen_nfkc_hit: bool,
    signals: Signals,
    checks: Checks,
    stats_ms: f64,
    jinen_ms: f64,
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let result = match args.mode {
        Mode::Collect => collect(&args),
        Mode::Sweep => sweep(&args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

fn collect(args: &Args) -> Result<(), String> {
    // candle 的 matmul 執行緒數必須在任何張量運算之前設定。
    std::env::set_var("CANDLE_NUM_THREADS", args.threads.to_string());
    std::env::set_var("RAYON_NUM_THREADS", args.threads.to_string());

    let corpus_path = args.testdata.join(corpus_file(&args.corpus)?);
    let corpus: CorpusFile = serde_json::from_reader(
        std::fs::File::open(&corpus_path)
            .map_err(|err| format!("開不了 {}：{err}", corpus_path.display()))?,
    )
    .map_err(|err| format!("{} 不是合法題庫：{err}", corpus_path.display()))?;

    let decoder = jc_engine::loader::load_ja_decoder_with_pack(&args.data_root, None)
        .map_err(|err| format!("載入日文 pack 失敗：{err}"))?;
    let ctx = JaContext::default();

    let gguf = match &args.gguf {
        Some(path) => path.clone(),
        None => default_gguf()?,
    };
    let tokenizer = resolve_tokenizer(args, &gguf)?;
    let load_started = Instant::now();
    let mut model = Jinen::load(&gguf, &tokenizer)
        .map_err(|err| format!("載入 {} 失敗：{err}", gguf.display()))?;
    eprintln!(
        "模型 {}｜tokenizer {}｜載入 {:.1} ms｜corpus {}",
        gguf.display(),
        tokenizer.display(),
        load_started.elapsed().as_secs_f64() * 1000.0,
        args.corpus
    );
    model
        .generate("テスト", "", args.max_new_tokens)
        .map_err(|err| format!("暖機失敗：{err}"))?;

    let items = match args.limit {
        Some(limit) => &corpus.items[..limit.min(corpus.items.len())],
        None => &corpus.items[..],
    };

    let mut rows = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let stats_started = Instant::now();
        let (stats_output, signals) = stats_side(&decoder, &item.reading, &ctx);
        let stats_ms = stats_started.elapsed().as_secs_f64() * 1000.0;

        let jinen_started = Instant::now();
        let generation = model
            .generate(&item.reading, "", args.max_new_tokens)
            .map_err(|err| format!("{} 推論失敗：{err}", item.id))?;
        let jinen_ms = jinen_started.elapsed().as_secs_f64() * 1000.0;

        let accepted = accepted_surfaces(item);
        let stats_hit = accepted.contains(&stats_output);
        let jinen_hit = accepted.contains(&generation.text);
        let jinen_nfkc_hit = accepted
            .iter()
            .any(|value| jinen::nfkc(value) == jinen::nfkc(&generation.text));
        let checks = Checks {
            length_ok: !truncated(&stats_output, &generation.text),
            echo_ok: !echoes_context("", &item.reading, &stats_output, &generation.text),
            emoji_ok: emoji_sequence(&stats_output) == emoji_sequence(&generation.text),
        };
        rows.push(Row {
            id: item.id.clone(),
            reading: item.reading.clone(),
            surface: item.surface.clone(),
            accept: item.accept.clone(),
            stats: stats_output,
            stats_hit,
            jinen: generation.text,
            jinen_hit,
            jinen_nfkc_hit,
            signals,
            checks,
            stats_ms,
            jinen_ms,
        });
        if (index + 1) % 50 == 0 {
            eprintln!("  {}/{}", index + 1, items.len());
        }
    }

    let stats_hits = rows.iter().filter(|row| row.stats_hit).count();
    let jinen_hits = rows.iter().filter(|row| row.jinen_hit).count();
    let both = rows
        .iter()
        .filter(|row| row.stats_hit && row.jinen_hit)
        .count();
    let neither = rows
        .iter()
        .filter(|row| !row.stats_hit && !row.jinen_hit)
        .count();
    let rejected = rows.iter().filter(|row| !row.checks.all()).count();
    println!(
        "{}｜n {}｜統計 {:.1}%｜Jinen {:.1}%｜兩邊都對 {}｜都不對 {}｜檢查退回 {}",
        args.corpus,
        rows.len(),
        100.0 * stats_hits as f64 / rows.len() as f64,
        100.0 * jinen_hits as f64 / rows.len() as f64,
        both,
        neither,
        rejected
    );

    let out = args.out.clone().unwrap_or_else(|| {
        args.dir
            .join(format!("{}.json", args.corpus.replace('/', "-")))
    });
    write_json(
        &out,
        json!({
            "meta": {
                "corpus": args.corpus,
                "gguf": gguf.display().to_string(),
                "tokenizer": tokenizer.display().to_string(),
                "pack": "ja-lexicon-mozc-v2.pack",
                "decoder": "Viterbi + context 矩陣，無 reranker、無 LM、無 particle rewrite",
                "context": "",
                "threads": args.threads,
                "max_new_tokens": args.max_new_tokens,
                "stats_hits": stats_hits,
                "jinen_hits": jinen_hits,
                "both_hits": both,
                "neither": neither,
                "checks_rejected": rejected,
            },
            "items": rows,
        }),
    )?;
    eprintln!("寫入 {}", out.display());
    Ok(())
}

/// 統計轉換的 top-1 與五個訊號。
///
/// 候選數與歧義度以 top-1 路徑上的每個詞段為一個詞群，查同讀音詞條數。
/// 未知詞比例用 lattice 的節點數推算：解碼器在每個讀音位置固定補一個未知
/// 詞節點，其餘節點就是詞典命中數。
fn stats_side(decoder: &JaDecoder, reading: &str, ctx: &JaContext) -> (String, Signals) {
    let units: Vec<char> = reading.chars().collect();
    let results = decoder.results(&units, ctx, 2);
    let first = results.first();
    let output = first.map(JaResult::surface).unwrap_or_default();
    let gap = match (results.first(), results.get(1)) {
        (Some(a), Some(b)) => Some((((b.cost - a.cost) * 1000.0).round()) as i32),
        _ => None,
    };

    // lattice 節點數：每個位置一個未知詞節點，加上詞典命中的詞弧。
    let max_units = 32usize;
    let mut nodes = 0usize;
    for start in 0..units.len() {
        let mut hits = 0usize;
        decoder.source().matches(&units, start, &mut |m| {
            if m.end.saturating_sub(start) <= max_units {
                hits += 1;
            }
        });
        nodes += hits + 1;
    }
    let unknown_ratio = if nodes == 0 {
        0.0
    } else {
        units.len() as f64 / nodes as f64
    };

    let mut group_counts: Vec<usize> = Vec::new();
    if let Some(result) = first {
        for clause in &result.clauses {
            for segment in &clause.segments {
                let seg: Vec<char> = segment.reading.chars().collect();
                let mut surfaces: Vec<&str> = decoder
                    .source()
                    .exact(&seg)
                    .iter()
                    .map(|entry| entry.surface.as_ref())
                    .collect();
                surfaces.sort_unstable();
                surfaces.dedup();
                group_counts.push(surfaces.len().max(1));
            }
        }
    }
    let mean_candidates = if group_counts.is_empty() {
        1.0
    } else {
        group_counts.iter().sum::<usize>() as f64 / group_counts.len() as f64
    };
    let ambiguous_ratio = if group_counts.is_empty() {
        0.0
    } else {
        group_counts.iter().filter(|count| **count > 1).count() as f64 / group_counts.len() as f64
    };
    let max_candidates = group_counts.iter().copied().max().unwrap_or(1);

    (
        output,
        Signals {
            gap,
            unknown_ratio,
            reading_len: units.len(),
            mean_candidates,
            ambiguous_ratio,
            max_candidates,
        },
    )
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

/// 首尾截斷：Jinen 比統計結果短，且是它的前綴或後綴；或長度不到一半。
fn truncated(baseline: &str, candidate: &str) -> bool {
    let base_len = baseline.chars().count();
    let cand_len = candidate.chars().count();
    if cand_len == 0 || base_len == 0 {
        return cand_len == 0;
    }
    if cand_len < base_len && (baseline.starts_with(candidate) || baseline.ends_with(candidate)) {
        return true;
    }
    (cand_len as f64) * 2.0 < base_len as f64
}

/// 左文脈 echo：輸出開頭重複了左文脈尾端，且長度不足以解釋成正常轉換。
fn echoes_context(context: &str, reading: &str, baseline: &str, candidate: &str) -> bool {
    if context.is_empty() {
        return false;
    }
    let ctx: Vec<char> = context.chars().collect();
    let out: Vec<char> = candidate.chars().collect();
    let base_len = baseline.chars().count();
    let key_len = reading.chars().count();
    let limit = ctx.len().min(out.len());
    let mut overlap = 0usize;
    for len in (1..=limit).rev() {
        if ctx[ctx.len() - len..] == out[..len] {
            overlap = len;
            break;
        }
    }
    if overlap < 4 {
        return false;
    }
    let span = base_len.max(key_len);
    overlap > span * 2 || out.len() >= overlap + base_len
}

/// 抽出有序的絵文字 cluster 序列；ZWJ、膚色、variation selector、keycap 都
/// 算同一個 cluster。
fn emoji_sequence(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        if !is_emoji_base(chars[index]) {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        loop {
            if index >= chars.len() {
                break;
            }
            let c = chars[index];
            if c == '\u{200d}' {
                index += 1;
                if index < chars.len() && is_emoji_base(chars[index]) {
                    index += 1;
                }
                continue;
            }
            if is_emoji_modifier(c) {
                index += 1;
                continue;
            }
            break;
        }
        out.push(chars[start..index].iter().collect());
    }
    out
}

fn is_emoji_base(c: char) -> bool {
    matches!(
        u32::from(c),
        0x1f000..=0x1faff | 0x2600..=0x26ff | 0x2700..=0x27bf | 0x2b00..=0x2bff
    )
}

fn is_emoji_modifier(c: char) -> bool {
    matches!(
        u32::from(c),
        0xfe0e | 0xfe0f | 0x200d | 0x20e3 | 0x1f3fb..=0x1f3ff
    )
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

fn default_gguf() -> Result<PathBuf, String> {
    let path = PathBuf::from(DEFAULT_MODEL_DATA)
        .join("jinen-v2-xsmall")
        .join("jinen-v2-xsmall-Q5_K_M.gguf");
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("找不到 {}", path.display()))
    }
}

fn resolve_tokenizer(args: &Args, gguf: &Path) -> Result<PathBuf, String> {
    if let Some(path) = &args.tokenizer {
        return Ok(path.clone());
    }
    if let Ok(path) = std::env::var("JC_JINEN_TOKENIZER") {
        return Ok(PathBuf::from(path));
    }
    let quant =
        PathBuf::from(DEFAULT_DATA_ROOT).join("models/quant/jinen-v2-xsmall/tokenizer.json");
    if quant.is_file() {
        return Ok(quant);
    }
    let sibling = gguf.with_file_name("tokenizer.json");
    if sibling.is_file() {
        return Ok(sibling);
    }
    Err(format!(
        "找不到 tokenizer.json（試過 {}）；可用 --tokenizer 或 JC_JINEN_TOKENIZER 指定",
        quant.display()
    ))
}

// ---------------------------------------------------------------------------
// 門檻掃描
// ---------------------------------------------------------------------------

/// 題庫名稱，固定三組。
const CORPORA: [&str; 3] = ["modern-500", "hard-cases", "ajimee-200"];

/// dev 挑門檻時要求的安全邊際（pp）。dev 只有半個題庫，hard-cases 更是
/// 18 題（1 題＝5.6pp），沒有邊際的門檻在 holdout 上很容易掉回基線。
const SAFETY_MARGIN: f64 = 4.0;

/// 驗收要求。
fn requirement(name: &str) -> f64 {
    match name {
        "modern-500" => 86.0,
        "hard-cases" => 60.0,
        _ => 60.0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Signal {
    Gap,
    Unknown,
    Length,
    MeanCandidates,
    Ambiguous,
}

impl Signal {
    fn name(self) -> &'static str {
        match self {
            Self::Gap => "統計邊界（top1−top2 成本差 < X）",
            Self::Unknown => "未知詞比例 > X",
            Self::Length => "句長 > X",
            Self::MeanCandidates => "平均候選數 > X",
            Self::Ambiguous => "讀音歧義度 > X",
        }
    }

    fn short(self) -> &'static str {
        match self {
            Self::Gap => "gap",
            Self::Unknown => "unknown",
            Self::Length => "len",
            Self::MeanCandidates => "meancand",
            Self::Ambiguous => "ambig",
        }
    }

    fn value(self, row: &Row) -> Option<f64> {
        match self {
            Self::Gap => row.signals.gap.map(f64::from),
            Self::Unknown => Some(row.signals.unknown_ratio),
            Self::Length => Some(row.signals.reading_len as f64),
            Self::MeanCandidates => Some(row.signals.mean_candidates),
            Self::Ambiguous => Some(row.signals.ambiguous_ratio),
        }
    }

    /// 門檻是否觸發「改用 Jinen」。
    fn fires(self, row: &Row, threshold: f64) -> bool {
        match self.value(row) {
            None => true, // 只有一條路徑＝統計沒有備案，交給 Jinen。
            Some(value) => match self {
                Self::Gap => value < threshold,
                _ => value > threshold,
            },
        }
    }
}

/// 一條規則：單一訊號門檻，或兩個訊號門檻的 AND／OR。
#[derive(Debug, Clone)]
struct Rule {
    parts: Vec<(Signal, f64)>,
    /// `true` 是 AND（兩個條件都要成立），`false` 是 OR。
    and: bool,
}

impl Rule {
    fn single(signal: Signal, threshold: f64) -> Self {
        Self {
            parts: vec![(signal, threshold)],
            and: false,
        }
    }

    fn fires(&self, row: &Row) -> bool {
        let test = |(signal, threshold): &(Signal, f64)| signal.fires(row, *threshold);
        if self.and {
            self.parts.iter().all(test)
        } else {
            self.parts.iter().any(test)
        }
    }

    fn label(&self) -> String {
        let sep = if self.and { " 且 " } else { " 或 " };
        self.parts
            .iter()
            .map(|(signal, threshold)| format!("{} {:.4}", signal.short(), threshold))
            .collect::<Vec<_>>()
            .join(sep)
    }

    fn describe(&self) -> Vec<rule_json::Part> {
        self.parts
            .iter()
            .map(|(signal, threshold)| rule_json::Part {
                signal: signal.name(),
                short: signal.short(),
                threshold: *threshold,
            })
            .collect()
    }
}

mod rule_json {
    use serde::Serialize;

    #[derive(Debug, Clone, Serialize)]
    pub struct Part {
        pub signal: &'static str,
        pub short: &'static str,
        pub threshold: f64,
    }
}

/// 一組題庫的逐題資料。
struct Corpus {
    name: &'static str,
    items: Vec<Row>,
    dev: Vec<bool>,
}

impl Corpus {
    fn load(dir: &Path, name: &'static str) -> Result<Self, String> {
        let path = dir.join(format!("{name}.json"));
        let raw: serde_json::Value = serde_json::from_reader(
            std::fs::File::open(&path)
                .map_err(|err| format!("開不了 {}：{err}（先跑 collect）", path.display()))?,
        )
        .map_err(|err| format!("{} 不是合法 JSON：{err}", path.display()))?;
        let items: Vec<Row> = serde_json::from_value(
            raw.get("items")
                .cloned()
                .ok_or_else(|| format!("{} 沒有 items", path.display()))?,
        )
        .map_err(|err| format!("{} 的 items 格式不符：{err}", path.display()))?;
        let dev = items.iter().map(|row| is_dev(&row.id)).collect();
        Ok(Self { name, items, dev })
    }
}

/// 以題號雜湊做穩定的 50/50 切分：dev 用來挑門檻，holdout 只回報。
fn is_dev(id: &str) -> bool {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in id.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash.is_multiple_of(2)
}

#[derive(Debug, Clone, Copy, Default)]
struct Score {
    hits: usize,
    n: usize,
    calls: usize,
}

impl Score {
    fn pct(self) -> f64 {
        if self.n == 0 {
            0.0
        } else {
            100.0 * self.hits as f64 / self.n as f64
        }
    }

    fn call_rate(self) -> f64 {
        if self.n == 0 {
            0.0
        } else {
            100.0 * self.calls as f64 / self.n as f64
        }
    }

    fn add(&mut self, hit: bool, called: bool) {
        self.n += 1;
        self.hits += usize::from(hit);
        self.calls += usize::from(called);
    }
}

/// `subset`：`Some(true)` 只看 dev、`Some(false)` 只看 holdout、`None` 全部。
fn score(rule: &Rule, corpus: &Corpus, subset: Option<bool>) -> Score {
    let mut out = Score::default();
    for (row, is_dev_row) in corpus.items.iter().zip(&corpus.dev) {
        if subset.is_some_and(|want| *is_dev_row != want) {
            continue;
        }
        let fire = rule.fires(row);
        out.add(decide(row, fire), fire);
    }
    out
}

/// 觸發且檢查通過才用 Jinen，否則回統計結果。
fn decide(row: &Row, fire: bool) -> bool {
    if fire && row.checks.all() {
        row.jinen_hit
    } else {
        row.stats_hit
    }
}

fn percentile(values: &mut [f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rank = ((p / 100.0 * values.len() as f64).ceil() as usize).max(1);
    values[rank - 1]
}

/// 給定一組門檻候選，回傳所有單一訊號規則，加上 dev 上可行的兩個訊號 OR。
fn search(corpora: &[Corpus]) -> Vec<(Rule, Evaluation)> {
    let signals = [
        Signal::Gap,
        Signal::Unknown,
        Signal::Length,
        Signal::MeanCandidates,
        Signal::Ambiguous,
    ];
    let mut candidates: BTreeMap<&'static str, Vec<f64>> = BTreeMap::new();
    for signal in signals {
        let mut values: Vec<f64> = corpora
            .iter()
            .flat_map(|corpus| {
                corpus
                    .items
                    .iter()
                    .zip(&corpus.dev)
                    .filter(|(_, dev)| **dev)
                    .filter_map(|(row, _)| signal.value(row))
            })
            .collect();
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        values.dedup();
        candidates.insert(signal.short(), values);
    }

    let mut scored: Vec<(Rule, Evaluation)> = Vec::new();
    for signal in signals {
        let values = &candidates[signal.short()];
        // 每個相鄰值之間取中點，讓門檻落在兩個觀測值之間。
        for window in values.windows(2) {
            let rule = Rule::single(signal, (window[0] + window[1]) / 2.0);
            let dev = evaluate(&rule, corpora, Some(true));
            scored.push((rule, dev));
        }
    }

    // 兩個訊號的組合。只有統計邊界能單獨達標，所以拿它當第一條件，其餘
    // 四個訊號當第二條件；AND 與 OR 都掃。
    let mut bases: Vec<(f64, Score)> = scored
        .iter()
        .filter(|(rule, dev)| {
            rule.parts[0].0 == Signal::Gap && dev.feasible_with(corpora, SAFETY_MARGIN)
        })
        .map(|(rule, dev)| (rule.parts[0].1, dev.aggregate()))
        .collect();
    bases.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    if bases.len() > 8 {
        // 平均取 8 個，涵蓋整個可行區間，不偏向某一端。
        let step = (bases.len() - 1) as f64 / 7.0;
        bases = (0..8)
            .map(|index| bases[(index as f64 * step).round() as usize])
            .collect();
    }
    for (base, _) in &bases {
        for signal in [
            Signal::Unknown,
            Signal::Length,
            Signal::MeanCandidates,
            Signal::Ambiguous,
        ] {
            for window in candidates[signal.short()].windows(2) {
                let threshold = (window[0] + window[1]) / 2.0;
                for and in [true, false] {
                    let rule = Rule {
                        parts: vec![(Signal::Gap, *base), (signal, threshold)],
                        and,
                    };
                    let dev = evaluate(&rule, corpora, Some(true));
                    scored.push((rule, dev));
                }
            }
        }
    }
    scored
}

/// 一組規則在 dev 或 holdout 上的三組分數。
#[derive(Clone)]
struct Evaluation {
    per: Vec<Score>,
}

impl Evaluation {
    fn aggregate(&self) -> Score {
        let mut out = Score::default();
        for part in &self.per {
            out.hits += part.hits;
            out.n += part.n;
            out.calls += part.calls;
        }
        out
    }

    fn feasible(&self, corpora: &[Corpus]) -> bool {
        self.feasible_with(corpora, 0.0)
    }

    /// 三組都要達到「要求 + 邊際」。
    fn feasible_with(&self, corpora: &[Corpus], margin: f64) -> bool {
        self.per
            .iter()
            .zip(corpora)
            .all(|(part, corpus)| part.pct() + 1e-9 >= requirement(corpus.name) + margin)
    }

    /// 可行時回傳最小的超額（以 pp 計）當作安全邊際；不可行時回傳最差缺口。
    fn margin(&self, corpora: &[Corpus]) -> f64 {
        self.per
            .iter()
            .zip(corpora)
            .map(|(part, corpus)| part.pct() - requirement(corpus.name))
            .fold(f64::INFINITY, f64::min)
    }
}

fn evaluate(rule: &Rule, corpora: &[Corpus], subset: Option<bool>) -> Evaluation {
    Evaluation {
        per: corpora
            .iter()
            .map(|corpus| score(rule, corpus, subset))
            .collect(),
    }
}

/// 延遲：統計一定會算，觸發時再等 Jinen；`subset` 同 [`score`]。
fn latency_overall(rule: &Rule, corpora: &[Corpus], subset: Option<bool>) -> (f64, f64) {
    let mut values = Vec::new();
    for corpus in corpora {
        for (row, is_dev_row) in corpus.items.iter().zip(&corpus.dev) {
            if subset.is_some_and(|want| *is_dev_row != want) {
                continue;
            }
            let fire = rule.fires(row);
            values.push(row.stats_ms + if fire { row.jinen_ms } else { 0.0 });
        }
    }
    (percentile(&mut values, 50.0), percentile(&mut values, 95.0))
}

fn sweep(args: &Args) -> Result<(), String> {
    let corpora: Vec<Corpus> = CORPORA
        .iter()
        .map(|name| Corpus::load(&args.dir, name))
        .collect::<Result<_, _>>()?;
    for corpus in &corpora {
        eprintln!(
            "{}｜n {}｜dev {}｜holdout {}",
            corpus.name,
            corpus.items.len(),
            corpus.dev.iter().filter(|dev| **dev).count(),
            corpus.dev.iter().filter(|dev| !**dev).count()
        );
    }

    // 1. 基線（全題庫）與逐題上限。
    println!("## 基線（全題庫）");
    let mut baseline_rows = Vec::new();
    for corpus in &corpora {
        let n = corpus.items.len() as f64;
        let stats_hits = corpus.items.iter().filter(|row| row.stats_hit).count();
        let jinen_hits = corpus.items.iter().filter(|row| row.jinen_hit).count();
        let oracle = corpus
            .items
            .iter()
            .filter(|row| row.stats_hit || row.jinen_hit)
            .count();
        let only_stats = corpus
            .items
            .iter()
            .filter(|row| row.stats_hit && !row.jinen_hit)
            .count();
        let only_jinen = corpus
            .items
            .iter()
            .filter(|row| !row.stats_hit && row.jinen_hit)
            .count();
        let rejected = corpus.items.iter().filter(|row| !row.checks.all()).count();
        println!(
            "{}（n {}）統計 {:.1}%｜Jinen {:.1}%｜逐題上限參考 {:.1}%｜只統計對 {}｜只 Jinen 對 {}｜檢查退回 {}",
            corpus.name,
            corpus.items.len(),
            100.0 * stats_hits as f64 / n,
            100.0 * jinen_hits as f64 / n,
            100.0 * oracle as f64 / n,
            only_stats,
            only_jinen,
            rejected
        );
        baseline_rows.push(json!({
            "corpus": corpus.name,
            "n": corpus.items.len(),
            "stats_pct": 100.0 * stats_hits as f64 / n,
            "jinen_pct": 100.0 * jinen_hits as f64 / n,
            "oracle_upper_pct": 100.0 * oracle as f64 / n,
            "only_stats_correct": only_stats,
            "only_jinen_correct": only_jinen,
            "checks_rejected": rejected,
            "both": corpus.items.iter().filter(|row| row.stats_hit && row.jinen_hit).count(),
        }));
    }

    // 2. 只用 dev 挑門檻：可行優先，其次呼叫率最低，再其次安全邊際。
    let scored = search(&corpora);
    let mut chosen: Vec<(Rule, Evaluation, Evaluation)> = scored
        .into_iter()
        .map(|(rule, dev)| {
            let holdout = evaluate(&rule, &corpora, Some(false));
            (rule, dev, holdout)
        })
        .collect();
    let use_buffer = chosen
        .iter()
        .any(|(_, dev, _)| dev.feasible_with(&corpora, SAFETY_MARGIN));
    let buffer = if use_buffer { SAFETY_MARGIN } else { 0.0 };
    chosen.sort_by(|a, b| {
        b.1.feasible_with(&corpora, buffer)
            .cmp(&a.1.feasible_with(&corpora, buffer))
            .then_with(|| a.1.aggregate().calls.cmp(&b.1.aggregate().calls))
            .then_with(|| {
                b.1.margin(&corpora)
                    .partial_cmp(&a.1.margin(&corpora))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    if let Some((rule, dev, holdout)) = chosen.first() {
        println!(
            "\n選擇準則：dev 三組都要 ≥ 要求 + {:.0}pp，取呼叫率最低；選中「{}」",
            buffer,
            rule.label()
        );
        let (p50, p95) = latency_overall(rule, &corpora, None);
        let all = evaluate(rule, &corpora, None);
        let cells: Vec<String> = all
            .per
            .iter()
            .zip(&corpora)
            .map(|(part, corpus)| {
                format!(
                    "{} {:.1}%（呼叫 {:.1}%）",
                    corpus.name,
                    part.pct(),
                    part.call_rate()
                )
            })
            .collect();
        println!(
            "  全題庫 {}｜p50 {p50:.2} ms｜p95 {p95:.2} ms｜dev 可行 {}｜holdout 可行 {}",
            cells.join("｜"),
            dev.feasible(&corpora),
            holdout.feasible(&corpora)
        );
    }

    println!("\n## dev 挑選（前 10 名；holdout 只回報、未參與選擇）");
    println!(
        "{:<44} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>6}",
        "規則", "dev-mod", "dev-hard", "dev-aji", "ho-mod", "ho-hard", "ho-aji", "呼叫率", "p50"
    );
    for (rule, dev, holdout) in chosen.iter().take(10) {
        print_rule_row(rule, dev, holdout, &corpora);
    }

    // 3. 單一訊號各自的最佳門檻（dev 可行、呼叫率最低），方便看哪個訊號有用。
    println!("\n## 單一訊號最佳門檻（dev 可行、呼叫率最低）");
    let mut per_signal: Vec<(Rule, Evaluation, Evaluation)> = chosen
        .iter()
        .filter(|(rule, _, _)| rule.parts.len() == 1)
        .map(|(rule, _dev, holdout)| {
            (
                rule.clone(),
                evaluate(rule, &corpora, Some(true)),
                holdout.clone(),
            )
        })
        .collect();
    per_signal.sort_by(|a, b| {
        b.1.feasible(&corpora)
            .cmp(&a.1.feasible(&corpora))
            .then_with(|| a.1.aggregate().calls.cmp(&b.1.aggregate().calls))
    });
    for (rule, dev, holdout) in per_signal.iter().take(6) {
        print_rule_row(rule, dev, holdout, &corpora);
    }

    // 4. 各訊號單獨使用的表現（全題庫），用來說明為什麼只有統計邊界能用。
    println!("\n## 各訊號單獨使用（全題庫）");
    let probes: [(Signal, &[f64]); 5] = [
        (Signal::Gap, &[150.0, 250.0, 400.0]),
        (Signal::Unknown, &[0.028, 0.032, 0.04]),
        (Signal::Length, &[15.0, 25.0, 40.0]),
        (Signal::MeanCandidates, &[6.0, 8.0, 10.0]),
        (Signal::Ambiguous, &[0.7, 0.8, 0.9]),
    ];
    let mut signal_rows = Vec::new();
    for (signal, thresholds) in probes {
        for threshold in thresholds {
            let rule = Rule::single(signal, *threshold);
            let all = evaluate(&rule, &corpora, None);
            let cells: Vec<String> = all
                .per
                .iter()
                .map(|part| format!("{:.1}", part.pct()))
                .collect();
            println!(
                "{:<28} {:>7} {:>7} {:>7} {:>7.1}%",
                format!("{} {:.4}", signal.short(), threshold),
                cells[0],
                cells[1],
                cells[2],
                all.aggregate().call_rate()
            );
            signal_rows.push(json!({
                "signal": signal.short(),
                "threshold": threshold,
                "all": eval_json(&all, &corpora),
            }));
        }
    }

    // 5. 整數門檻的平台區：看同一條規則在不同門檻下穩不穩。
    println!("\n## 整數門檻掃描（統計邊界）");
    let mut plateau = Vec::new();
    for threshold in [
        100.0, 150.0, 200.0, 250.0, 300.0, 350.0, 400.0, 450.0, 500.0, 600.0,
    ] {
        let rule = Rule::single(Signal::Gap, threshold);
        let dev = evaluate(&rule, &corpora, Some(true));
        let holdout = evaluate(&rule, &corpora, Some(false));
        let (p50, p95) = latency_overall(&rule, &corpora, None);
        plateau.push(json!({
            "threshold": threshold,
            "dev": eval_json(&dev, &corpora),
            "holdout": eval_json(&holdout, &corpora),
            "all_p50_ms": p50,
            "all_p95_ms": p95,
        }));
        print_rule_row(&rule, &dev, &holdout, &corpora);
    }

    // 6. 上限參考：用答案在整個題庫挑單一門檻（含 holdout），只看趨勢。
    let mut upper: Vec<(Rule, Evaluation)> = Vec::new();
    for (rule, _, _) in &chosen {
        if rule.parts.len() != 1 {
            continue;
        }
        upper.push((rule.clone(), evaluate(rule, &corpora, None)));
    }
    upper.sort_by(|a, b| {
        b.1.margin(&corpora)
            .partial_cmp(&a.1.margin(&corpora))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.aggregate().calls.cmp(&b.1.aggregate().calls))
    });
    println!("\n## 上限參考：用答案挑的門檻（不可當出貨值）");
    for (rule, all) in upper.iter().take(5) {
        let calls = all.aggregate().call_rate();
        let cells: Vec<String> = all
            .per
            .iter()
            .map(|part| format!("{:.1}", part.pct()))
            .collect();
        println!(
            "{:<44} {:>7} {:>7} {:>7} {:>7.1}%",
            rule.label(),
            cells[0],
            cells[1],
            cells[2],
            calls
        );
    }

    // 5. 輸出 JSON。
    let out = args
        .out
        .clone()
        .unwrap_or_else(|| args.dir.join("sweep.json"));
    let detail: Vec<_> = chosen
        .iter()
        .take(10)
        .map(|(rule, dev, holdout)| {
            json!({
                "rule": rule.describe(),
                "label": rule.label(),
                "dev": eval_json(dev, &corpora),
                "holdout": eval_json(holdout, &corpora),
                "dev_p50_ms": latency_overall(rule, &corpora, Some(true)).0,
                "dev_p95_ms": latency_overall(rule, &corpora, Some(true)).1,
            })
        })
        .collect();
    write_json(
        &out,
        json!({
            "baseline": baseline_rows,
            "selected": detail,
            "signal_scan": signal_rows,
            "plateau": plateau,
            "upper_reference": upper.iter().take(5).map(|(rule, all)| json!({
                "rule": rule.describe(),
                "label": rule.label(),
                "all": eval_json(all, &corpora),
            })).collect::<Vec<_>>(),
        }),
    )?;
    eprintln!("寫入 {}", out.display());
    Ok(())
}

fn print_rule_row(rule: &Rule, dev: &Evaluation, holdout: &Evaluation, corpora: &[Corpus]) {
    let calls = dev.aggregate().call_rate();
    let cell = |eval: &Evaluation| -> Vec<String> {
        eval.per
            .iter()
            .zip(corpora)
            .map(|(part, _)| format!("{:.1}", part.pct()))
            .collect()
    };
    let d = cell(dev);
    let h = cell(holdout);
    let (p50, _) = latency_overall(rule, corpora, Some(true));
    println!(
        "{:<44} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7.1}% {:>6.2}",
        rule.label(),
        d[0],
        d[1],
        d[2],
        h[0],
        h[1],
        h[2],
        calls,
        p50
    );
}

fn eval_json(eval: &Evaluation, corpora: &[Corpus]) -> serde_json::Value {
    json!({
        "per_corpus": eval.per.iter().zip(corpora).map(|(part, corpus)| json!({
            "corpus": corpus.name,
            "n": part.n,
            "pct": part.pct(),
            "calls_pct": part.call_rate(),
        })).collect::<Vec<_>>(),
        "overall_calls_pct": eval.aggregate().call_rate(),
        "feasible": eval.feasible(corpora),
    })
}

// ---------------------------------------------------------------------------
// 參數與輸出
// ---------------------------------------------------------------------------

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut parsed = Args::default();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} 需要一個值"));
        match arg.as_str() {
            "collect" => parsed.mode = Mode::Collect,
            "sweep" => parsed.mode = Mode::Sweep,
            "--corpus" => parsed.corpus = value("--corpus")?,
            "--data-root" => parsed.data_root = PathBuf::from(value("--data-root")?),
            "--testdata" => parsed.testdata = PathBuf::from(value("--testdata")?),
            "--gguf" => parsed.gguf = Some(PathBuf::from(value("--gguf")?)),
            "--tokenizer" => parsed.tokenizer = Some(PathBuf::from(value("--tokenizer")?)),
            "--out" => parsed.out = Some(PathBuf::from(value("--out")?)),
            "--dir" => parsed.dir = PathBuf::from(value("--dir")?),
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
            "--threads" => {
                parsed.threads = value("--threads")?
                    .parse()
                    .map_err(|err| format!("--threads 不是整數：{err}"))?
            }
            "-h" | "--help" => {
                println!(
                    "用法：jinen-gate collect --corpus ajimee-200|modern-500|hard-cases [--limit N]\n\
                     \x20     jinen-gate sweep [--dir target/jinen-gate] [--out FILE]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("未知參數：{other}")),
        }
    }
    Ok(parsed)
}

fn write_json(path: &Path, payload: serde_json::Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("建不了 {}：{err}", parent.display()))?;
    }
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&payload).map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("寫不了 {}：{err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, reading_len: usize) -> Row {
        Row {
            id: id.into(),
            reading: "あ".repeat(reading_len),
            surface: "亜".into(),
            accept: Vec::new(),
            stats: "亜".into(),
            stats_hit: true,
            jinen: "亜".into(),
            jinen_hit: true,
            jinen_nfkc_hit: true,
            signals: Signals {
                gap: Some(100),
                unknown_ratio: 0.5,
                reading_len,
                mean_candidates: 2.0,
                ambiguous_ratio: 0.5,
                max_candidates: 3,
            },
            checks: Checks {
                length_ok: true,
                echo_ok: true,
                emoji_ok: true,
            },
            stats_ms: 1.0,
            jinen_ms: 20.0,
        }
    }

    #[test]
    fn test_truncated_detects_prefix_and_suffix() {
        assert!(truncated("私は本を読む。", "本を読む。"));
        assert!(truncated("私は本を読む。", "私は本を"));
        assert!(!truncated("私は本を読む。", "私は書を読む。"));
        assert!(truncated("ああああああ", "ああ"));
    }

    #[test]
    fn test_echoes_context_needs_overlap_of_four() {
        assert!(!echoes_context("", "よむ", "読む", "読む"));
        assert!(!echoes_context("きょうは", "よむ", "読む", "読む"));
        assert!(echoes_context(
            "きょうはいいてんきですね",
            "よむ",
            "読む",
            "きょうはいいてんきですね読む"
        ));
    }

    #[test]
    fn test_emoji_sequence_keeps_zwj_clusters() {
        assert_eq!(emoji_sequence("家族👨‍👩‍👧‍👦です").len(), 1);
        assert_eq!(emoji_sequence("👍🏽").first().unwrap(), "👍🏽");
        assert!(emoji_sequence("漢字").is_empty());
    }

    #[test]
    fn test_rule_gap_fires_below_threshold() {
        let row = row("a", 5);
        assert!(Rule::single(Signal::Gap, 150.0).fires(&row));
        assert!(!Rule::single(Signal::Gap, 50.0).fires(&row));
    }

    #[test]
    fn test_rule_or_fires_on_either_signal() {
        let row = row("a", 30);
        let rule = Rule {
            parts: vec![(Signal::Gap, 10.0), (Signal::Length, 20.0)],
            and: false,
        };
        assert!(rule.fires(&row));
    }

    #[test]
    fn test_is_dev_is_stable_and_split() {
        let dev = (0..100)
            .filter(|index| is_dev(&format!("modern-{index:03}")))
            .count();
        assert!((30..=70).contains(&dev), "dev 比例失衡：{dev}");
        assert_eq!(is_dev("modern-007"), is_dev("modern-007"));
    }
}
