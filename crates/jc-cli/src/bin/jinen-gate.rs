//! 統計轉換與 Jinen 的離線門檻實驗。
//!
//! 對每題同時產生統計（Viterbi）與 Jinen 的結果，記錄八個**輸入側**訊號與
//! 各路徑是否命中，再離線掃描「什麼時候改用 Jinen」以及「要不要升級到
//! small」。這裡不接 `Session`，只產生分析用的資料與報表。
//!
//! ```sh
//! cargo run -p jc-cli --release --bin jinen-gate -- collect --corpus modern-500
//! cargo run -p jc-cli --release --bin jinen-gate -- collect --corpus modern-500 \
//!     --gguf2 /path/to/jinen-v2-small-Q5_K_M.gguf
//! cargo run -p jc-cli --release --bin jinen-gate -- sweep
//! ```
//!
//! `collect` 寫出 `target/jinen-gate/<corpus>.json`；`sweep` 讀回三份結果，
//! 只用輸入側訊號掃門檻。門檻在每組題庫的 dev 半邊挑選，holdout 半邊只
//! 回報、不參與選擇；用答案逐題挑最好結果的數字另外標成「上限參考」。

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use jc_core::ja::decode::{segment_candidate_counts, JaContext, JaDecoder, JaResult};
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
    /// 第二個模型（small）的 GGUF；指定時 `collect` 會多跑一次。
    gguf2: Option<PathBuf>,
    tokenizer2: Option<PathBuf>,
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
            gguf2: None,
            tokenizer2: None,
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
    /// 統計 top-1 輸出的漢字比例。題庫讀音是純假名，漢字密度只能從轉換結果
    /// 取；它仍然只看輸入與詞典，不含答案，用來近似「專業詞彙多」。
    kanji_ratio: f64,
    /// top-1 路徑上最長連續未知詞的字元長度。未知詞＝該詞段在同讀音下
    /// 查不到任何詞條。
    max_unknown_run: usize,
    /// top-1 與 top-2 的詞數差（絕對值）；只有一條路徑時取 top-1 的詞數。
    path_len_diff: usize,
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
    /// 第二個模型（small）的輸出；沒有指定時為 `None`。
    #[serde(default)]
    jinen2: Option<String>,
    #[serde(default)]
    jinen2_hit: bool,
    signals: Signals,
    checks: Checks,
    /// 第二個模型輸出的三條檢查；沒有跑模型2 時與 `checks` 相同。
    checks2: Checks,
    stats_ms: f64,
    jinen_ms: f64,
    /// 第二個模型（small）的單題延遲；沒有跑時為 0。
    #[serde(default)]
    jinen2_ms: f64,
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

    // 第二個模型（通常是 small）：沒有指定就不跑。
    let mut model2 = match &args.gguf2 {
        Some(gguf2) => {
            let tokenizer2 = resolve_tokenizer_second(args, gguf2)?;
            let load_started = Instant::now();
            let mut loaded = Jinen::load(gguf2, &tokenizer2)
                .map_err(|err| format!("載入 {} 失敗：{err}", gguf2.display()))?;
            eprintln!(
                "模型2 {}｜tokenizer {}｜載入 {:.1} ms",
                gguf2.display(),
                tokenizer2.display(),
                load_started.elapsed().as_secs_f64() * 1000.0
            );
            loaded
                .generate("テスト", "", args.max_new_tokens)
                .map_err(|err| format!("暖機（模型2）失敗：{err}"))?;
            Some(loaded)
        }
        None => None,
    };

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

        let (jinen2_text, jinen2_hit, jinen2_ms) = match model2.as_mut() {
            Some(model2) => {
                let started = Instant::now();
                let generation2 = model2
                    .generate(&item.reading, "", args.max_new_tokens)
                    .map_err(|err| format!("{} 推論失敗（模型2）：{err}", item.id))?;
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                let hit = accepted_surfaces(item).contains(&generation2.text);
                (Some(generation2.text), hit, elapsed)
            }
            None => (None, false, 0.0),
        };

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
        let checks2 = match &jinen2_text {
            Some(text) => Checks {
                length_ok: !truncated(&stats_output, text),
                echo_ok: !echoes_context("", &item.reading, &stats_output, text),
                emoji_ok: emoji_sequence(&stats_output) == emoji_sequence(text),
            },
            None => checks.clone(),
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
            jinen2: jinen2_text,
            jinen2_hit,
            signals,
            checks,
            checks2,
            stats_ms,
            jinen_ms,
            jinen2_ms,
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
    if model2.is_some() {
        let small_hits = rows.iter().filter(|row| row.jinen2_hit).count();
        println!(
            "{}｜模型2 {:.1}%｜模型2 檢查退回 {}",
            args.corpus,
            100.0 * small_hits as f64 / rows.len() as f64,
            rows.iter().filter(|row| !row.checks2.all()).count()
        );
    }

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
                "gguf2": args.gguf2.as_ref().map(|p| p.display().to_string()),
                "jinen2_hits": rows.iter().filter(|row| row.jinen2_hit).count(),
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

    // 候選數與歧義度與 Session 共用 `segment_candidate_counts`：以 top-1
    // 路徑上的每個詞段為一個詞群，查同讀音詞條數並去重詞面。
    let group_counts: Vec<usize> = first
        .map(|result| segment_candidate_counts(result, decoder.source().as_ref()))
        .unwrap_or_default();
    // top-1 路徑上的未知詞：同讀音查不到任何詞條。最長連續未知詞長度用
    // 字元數計算，跨越相鄰的未知詞段累加。
    let mut max_unknown_run = 0usize;
    let mut current_unknown_run = 0usize;
    let mut top1_words = 0usize;
    if let Some(result) = first {
        for clause in &result.clauses {
            for segment in &clause.segments {
                let seg: Vec<char> = segment.reading.chars().collect();
                top1_words += 1;
                if decoder.source().exact(&seg).is_empty() {
                    current_unknown_run += seg.len();
                    max_unknown_run = max_unknown_run.max(current_unknown_run);
                } else {
                    current_unknown_run = 0;
                }
            }
        }
    }
    let kanji_ratio = if output.is_empty() {
        0.0
    } else {
        let chars: Vec<char> = output.chars().collect();
        let kanji = chars.iter().filter(|c| is_kanji(**c)).count();
        kanji as f64 / chars.len() as f64
    };
    // top-2 的詞數；沒有 top-2 時用 top-1 自己，差值記 0。
    let second_words = results
        .get(1)
        .map(|result| {
            result
                .clauses
                .iter()
                .map(|clause| clause.segments.len())
                .sum::<usize>()
        })
        .unwrap_or(top1_words);
    let path_len_diff = top1_words.abs_diff(second_words);
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
            kanji_ratio,
            max_unknown_run,
            path_len_diff,
        },
    )
}

/// 漢字（含擴充 A 區）判定，用來算輸出的漢字密度。
fn is_kanji(c: char) -> bool {
    matches!(u32::from(c), 0x3400..=0x4dbf | 0x4e00..=0x9fff)
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

/// 第二個模型的 tokenizer：`--tokenizer2` → `JC_JINEN_TOKENIZER2` → 量化目錄
/// 的 xsmall tokenizer（兩者的詞表逐項相同）→ GGUF 同目錄。
fn resolve_tokenizer_second(args: &Args, gguf: &Path) -> Result<PathBuf, String> {
    if let Some(path) = &args.tokenizer2 {
        return Ok(path.clone());
    }
    if let Ok(path) = std::env::var("JC_JINEN_TOKENIZER2") {
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
        "找不到模型2 的 tokenizer.json（試過 {}）；可用 --tokenizer2 指定",
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

/// 驗收要求（沿用 `jinen-gating` 的現況值：三組都不得低於目前門檻組合）。
fn requirement(name: &str) -> f64 {
    match name {
        "modern-500" => 86.0,
        "hard-cases" => 71.4,
        _ => 64.0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Signal {
    Gap,
    Unknown,
    Length,
    MeanCandidates,
    Ambiguous,
    Kanji,
    UnknownRun,
    PathDiff,
}

impl Signal {
    fn name(self) -> &'static str {
        match self {
            Self::Gap => "統計邊界（top1−top2 成本差 < X）",
            Self::Unknown => "未知詞比例 > X",
            Self::Length => "句長 > X",
            Self::MeanCandidates => "平均候選數 > X",
            Self::Ambiguous => "讀音歧義度 > X",
            Self::Kanji => "輸出漢字比例 > X",
            Self::UnknownRun => "最長連續未知詞長度 > X",
            Self::PathDiff => "top1／top2 詞數差 > X",
        }
    }

    fn short(self) -> &'static str {
        match self {
            Self::Gap => "gap",
            Self::Unknown => "unknown",
            Self::Length => "len",
            Self::MeanCandidates => "meancand",
            Self::Ambiguous => "ambig",
            Self::Kanji => "kanji",
            Self::UnknownRun => "unkrun",
            Self::PathDiff => "pathdiff",
        }
    }

    fn value(self, row: &Row) -> Option<f64> {
        match self {
            Self::Gap => row.signals.gap.map(f64::from),
            Self::Unknown => Some(row.signals.unknown_ratio),
            Self::Length => Some(row.signals.reading_len as f64),
            Self::MeanCandidates => Some(row.signals.mean_candidates),
            Self::Ambiguous => Some(row.signals.ambiguous_ratio),
            Self::Kanji => Some(row.signals.kanji_ratio),
            Self::UnknownRun => Some(row.signals.max_unknown_run as f64),
            Self::PathDiff => Some(row.signals.path_len_diff as f64),
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
    /// 其中改用第二個模型（small）的題數。
    small_calls: usize,
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

/// 使用哪個後端。
#[derive(Debug, Clone, Copy, PartialEq)]
enum Backend {
    Stats,
    Jinen,
    Jinen2,
}

/// 完整門檻：`rule` 決定要不要改用 Jinen；`escalate` 再決定是否升級到
/// 第二個模型（small）。兩者都只看輸入側訊號。
#[derive(Debug, Clone)]
struct Gate {
    rule: Rule,
    escalate: Option<(Signal, f64)>,
}

impl Gate {
    fn new(rule: Rule) -> Self {
        Self {
            rule,
            escalate: None,
        }
    }

    fn backend(&self, row: &Row) -> Backend {
        if !self.rule.fires(row) {
            return Backend::Stats;
        }
        match &self.escalate {
            Some((signal, threshold)) if row.jinen2.is_some() && signal.fires(row, *threshold) => {
                Backend::Jinen2
            }
            _ => Backend::Jinen,
        }
    }

    /// 觸發後實際採用的後端是否通過三條檢查。
    fn accepted(&self, row: &Row, backend: Backend) -> bool {
        match backend {
            Backend::Stats => true,
            Backend::Jinen => row.checks.all(),
            Backend::Jinen2 => row.checks2.all(),
        }
    }

    fn label(&self) -> String {
        match &self.escalate {
            Some((signal, threshold)) => format!(
                "{} → 再 {} {:.4} 用 small",
                self.rule.label(),
                signal.short(),
                threshold
            ),
            None => self.rule.label(),
        }
    }
}

/// `subset`：`Some(true)` 只看 dev、`Some(false)` 只看 holdout、`None` 全部。
fn score(gate: &Gate, corpus: &Corpus, subset: Option<bool>) -> Score {
    let mut out = Score::default();
    for (row, is_dev_row) in corpus.items.iter().zip(&corpus.dev) {
        if subset.is_some_and(|want| *is_dev_row != want) {
            continue;
        }
        let backend = gate.backend(row);
        out.add(decide(row, gate, backend), backend != Backend::Stats);
        if backend == Backend::Jinen2 {
            out.small_calls += 1;
        }
    }
    out
}

/// 觸發且檢查通過才用 Jinen，否則回統計結果。
fn decide(row: &Row, gate: &Gate, backend: Backend) -> bool {
    if !gate.accepted(row, backend) {
        return row.stats_hit;
    }
    match backend {
        Backend::Stats => row.stats_hit,
        Backend::Jinen => row.jinen_hit,
        Backend::Jinen2 => row.jinen2_hit,
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

/// 八個輸入側訊號。
const ALL_SIGNALS: [Signal; 8] = [
    Signal::Gap,
    Signal::Unknown,
    Signal::Length,
    Signal::MeanCandidates,
    Signal::Ambiguous,
    Signal::Kanji,
    Signal::UnknownRun,
    Signal::PathDiff,
];

/// 從 dev 半邊取候選門檻：相鄰觀測值的中點；值太多時等距抽樣。
fn thresholds(signal: Signal, corpora: &[Corpus], max_points: usize) -> Vec<f64> {
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
    let mut points: Vec<f64> = values.windows(2).map(|w| (w[0] + w[1]) / 2.0).collect();
    if points.len() > max_points {
        let step = (points.len() - 1) as f64 / (max_points - 1) as f64;
        points = (0..max_points)
            .map(|index| points[(index as f64 * step).round() as usize])
            .collect();
        points.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    }
    points
}

/// 掃描所有單一訊號與「統計邊界 + 另一訊號」的規則，再以其中較好的規則
/// 為底加一層升級條件（改用 small）。全部只用 dev 半邊的資料挑選。
fn search(corpora: &[Corpus], margin: f64) -> Vec<(Gate, Evaluation)> {
    let mut scored: Vec<(Gate, Evaluation)> = Vec::new();
    for signal in ALL_SIGNALS {
        for threshold in thresholds(signal, corpora, 256) {
            let gate = Gate::new(Rule::single(signal, threshold));
            let dev = evaluate(&gate, corpora, Some(true));
            scored.push((gate, dev));
        }
    }

    // 統計邊界當第一條件，其餘七個訊號當第二條件；AND 與 OR 都掃。
    for base in thresholds(Signal::Gap, corpora, 48) {
        for signal in ALL_SIGNALS {
            if signal == Signal::Gap {
                continue;
            }
            for threshold in thresholds(signal, corpora, 48) {
                for and in [true, false] {
                    let rule = Rule {
                        parts: vec![(Signal::Gap, base), (signal, threshold)],
                        and,
                    };
                    let gate = Gate::new(rule);
                    let dev = evaluate(&gate, corpora, Some(true));
                    scored.push((gate, dev));
                }
            }
        }
    }

    // 同一個門檻，但觸發時一律用 small；用來比較「要不要升級模型」。
    let always_small: Vec<(Gate, Evaluation)> = scored
        .iter()
        .filter(|(gate, _)| gate.escalate.is_none())
        .map(|(gate, _)| {
            let upgraded = Gate {
                rule: gate.rule.clone(),
                escalate: Some((Signal::Gap, f64::INFINITY)),
            };
            let evaluated = evaluate(&upgraded, corpora, Some(true));
            (upgraded, evaluated)
        })
        .collect();
    scored.extend(always_small);

    // 兩段式候選：取 dev 上最前面的幾條規則，加上「升級到 small」的第二個
    // 條件。
    let mut ranked: Vec<usize> = (0..scored.len()).collect();
    ranked.sort_by(|a, b| {
        scored[*b]
            .1
            .feasible_with(corpora, margin)
            .cmp(&scored[*a].1.feasible_with(corpora, margin))
            .then_with(|| {
                scored[*b]
                    .1
                    .margin(corpora)
                    .partial_cmp(&scored[*a].1.margin(corpora))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                scored[*a]
                    .1
                    .aggregate()
                    .calls
                    .cmp(&scored[*b].1.aggregate().calls)
            })
    });
    let bases: Vec<Gate> = ranked
        .iter()
        .take(12)
        .map(|index| scored[*index].0.clone())
        .collect();
    for base in bases {
        for signal in ALL_SIGNALS {
            for threshold in thresholds(signal, corpora, 64) {
                let gate = Gate {
                    rule: base.rule.clone(),
                    escalate: Some((signal, threshold)),
                };
                let dev = evaluate(&gate, corpora, Some(true));
                scored.push((gate, dev));
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
            out.small_calls += part.small_calls;
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

fn evaluate(gate: &Gate, corpora: &[Corpus], subset: Option<bool>) -> Evaluation {
    Evaluation {
        per: corpora
            .iter()
            .map(|corpus| score(gate, corpus, subset))
            .collect(),
    }
}

/// 延遲：統計一定會算，觸發時再加上所選後端的時間；`subset` 同 [`score`]。
fn latency_overall(gate: &Gate, corpora: &[Corpus], subset: Option<bool>) -> (f64, f64) {
    let mut values = Vec::new();
    for corpus in corpora {
        for (row, is_dev_row) in corpus.items.iter().zip(&corpus.dev) {
            if subset.is_some_and(|want| *is_dev_row != want) {
                continue;
            }
            let backend = gate.backend(row);
            values.push(row.stats_ms + latency_of(row, backend));
        }
    }
    (percentile(&mut values, 50.0), percentile(&mut values, 95.0))
}

fn latency_of(row: &Row, backend: Backend) -> f64 {
    match backend {
        Backend::Stats => 0.0,
        Backend::Jinen => row.jinen_ms,
        Backend::Jinen2 => row.jinen2_ms,
    }
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
        let small_hits = corpus.items.iter().filter(|row| row.jinen2_hit).count();
        let oracle = corpus
            .items
            .iter()
            .filter(|row| row.stats_hit || row.jinen_hit || row.jinen2_hit)
            .count();
        let only_stats = corpus
            .items
            .iter()
            .filter(|row| row.stats_hit && !row.jinen_hit && !row.jinen2_hit)
            .count();
        let only_jinen = corpus
            .items
            .iter()
            .filter(|row| !row.stats_hit && row.jinen_hit)
            .count();
        let only_small = corpus
            .items
            .iter()
            .filter(|row| !row.stats_hit && !row.jinen_hit && row.jinen2_hit)
            .count();
        let rejected = corpus.items.iter().filter(|row| !row.checks.all()).count();
        println!(
            "{}（n {}）統計 {:.1}%｜xsmall {:.1}%｜small {:.1}%｜逐題上限參考 {:.1}%｜只統計對 {}｜只 xsmall 對 {}｜只 small 對 {}｜檢查退回 {}",
            corpus.name,
            corpus.items.len(),
            100.0 * stats_hits as f64 / n,
            100.0 * jinen_hits as f64 / n,
            100.0 * small_hits as f64 / n,
            100.0 * oracle as f64 / n,
            only_stats,
            only_jinen,
            only_small,
            rejected
        );
        baseline_rows.push(json!({
            "corpus": corpus.name,
            "n": corpus.items.len(),
            "stats_pct": 100.0 * stats_hits as f64 / n,
            "xsmall_pct": 100.0 * jinen_hits as f64 / n,
            "small_pct": 100.0 * small_hits as f64 / n,
            "oracle_upper_pct": 100.0 * oracle as f64 / n,
            "only_stats_correct": only_stats,
            "only_xsmall_correct": only_jinen,
            "only_small_correct": only_small,
            "checks_rejected": rejected,
        }));
    }

    // 延遲基線：三個後端各自的單題延遲（全題庫）。
    for corpus in &corpora {
        let mut stats: Vec<f64> = corpus.items.iter().map(|row| row.stats_ms).collect();
        let mut xsmall: Vec<f64> = corpus.items.iter().map(|row| row.jinen_ms).collect();
        let mut small: Vec<f64> = corpus
            .items
            .iter()
            .map(|row| row.jinen2_ms)
            .filter(|ms| *ms > 0.0)
            .collect();
        println!(
            "{} 延遲｜統計 p50 {:.2}｜xsmall p50 {:.2} p95 {:.2}｜small p50 {:.2} p95 {:.2} ms",
            corpus.name,
            percentile(&mut stats, 50.0),
            percentile(&mut xsmall, 50.0),
            percentile(&mut xsmall, 95.0),
            percentile(&mut small, 50.0),
            percentile(&mut small, 95.0),
        );
    }

    // 2. 只用 dev 挑門檻：可行優先，其次 p50 延遲最低。
    let scored = search(&corpora, 0.0);
    let mut chosen: Vec<(Gate, Evaluation, Evaluation)> = scored
        .into_iter()
        .map(|(gate, dev)| {
            let holdout = evaluate(&gate, &corpora, Some(false));
            (gate, dev, holdout)
        })
        .collect();
    let buffer = [SAFETY_MARGIN, 2.0, 0.0]
        .into_iter()
        .find(|margin| {
            chosen
                .iter()
                .any(|(_, dev, _)| dev.feasible_with(&corpora, *margin))
        })
        .unwrap_or(0.0);
    chosen.sort_by(|a, b| {
        b.1.feasible_with(&corpora, buffer)
            .cmp(&a.1.feasible_with(&corpora, buffer))
            .then_with(|| a.1.aggregate().calls.cmp(&b.1.aggregate().calls))
            .then_with(|| {
                a.1.aggregate()
                    .small_calls
                    .cmp(&b.1.aggregate().small_calls)
            })
            .then_with(|| {
                b.1.margin(&corpora)
                    .partial_cmp(&a.1.margin(&corpora))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    if let Some((gate, dev, holdout)) = chosen.first() {
        println!(
            "\n選擇準則：dev 三組都要 ≥ 要求 + {:.0}pp，取最差一組的超額最大；選中「{}」",
            buffer,
            gate.label()
        );
        let (p50, p95) = latency_overall(gate, &corpora, None);
        let all = evaluate(gate, &corpora, None);
        let cells: Vec<String> = all
            .per
            .iter()
            .zip(&corpora)
            .map(|(part, corpus)| {
                format!(
                    "{} {:.1}%（呼叫 {:.1}%／small {:.1}%）",
                    corpus.name,
                    part.pct(),
                    part.call_rate(),
                    100.0 * part.small_calls as f64 / part.n as f64
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
        "{:<52} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>6}",
        "規則", "dev-mod", "dev-hard", "dev-aji", "ho-mod", "ho-hard", "ho-aji", "呼叫率", "p50"
    );
    for (gate, dev, holdout) in chosen.iter().take(10) {
        print_rule_row(gate, dev, holdout, &corpora);
    }

    // 3. 單一訊號最佳門檻（dev 可行、p50 最低），每一種訊號各一列。
    println!("\n## 單一訊號最佳門檻（dev 可行、p50 最低）");
    let mut signal_best: Vec<(Gate, Evaluation, Evaluation)> = Vec::new();
    for signal in ALL_SIGNALS {
        let best = chosen
            .iter()
            .filter(|(gate, _, _)| gate.rule.parts.len() == 1 && gate.rule.parts[0].0 == signal)
            .min_by(|a, b| {
                b.1.feasible(&corpora)
                    .cmp(&a.1.feasible(&corpora))
                    .then_with(|| a.1.aggregate().calls.cmp(&b.1.aggregate().calls))
                    .then_with(|| {
                        a.1.aggregate()
                            .small_calls
                            .cmp(&b.1.aggregate().small_calls)
                    })
                    .then_with(|| {
                        b.1.margin(&corpora)
                            .partial_cmp(&a.1.margin(&corpora))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            });
        if let Some(row) = best {
            signal_best.push(row.clone());
        }
    }
    for (gate, dev, holdout) in &signal_best {
        print_rule_row(gate, dev, holdout, &corpora);
    }

    println!("\n## 單一訊號 maximin（只有一個門檻的規則；dev 最差一組超額最大）");
    let mut maximin: Vec<&(Gate, Evaluation, Evaluation)> = chosen
        .iter()
        .filter(|(gate, _, _)| gate.rule.parts.len() == 1)
        .collect();
    maximin.sort_by(|a, b| {
        b.1.margin(&corpora)
            .partial_cmp(&a.1.margin(&corpora))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.aggregate().calls.cmp(&b.1.aggregate().calls))
    });
    for (gate, dev, holdout) in maximin.iter().take(6) {
        print_rule_row(gate, dev, holdout, &corpora);
    }
    drop(maximin);

    // 只改用 small、統計邊界單一門檻：dev 達到「要求 + 4pp」後呼叫率最低的。
    println!("\n## small 單一門檻（dev ≥ 要求 + 4pp、呼叫率最低）");
    let mut small_gap: Vec<&(Gate, Evaluation, Evaluation)> = chosen
        .iter()
        .filter(|(gate, _, _)| {
            gate.rule.parts.len() == 1
                && gate.rule.parts[0].0 == Signal::Gap
                && gate.escalate.is_some()
        })
        .filter(|(_, dev, _)| dev.feasible_with(&corpora, SAFETY_MARGIN))
        .collect();
    small_gap.sort_by_key(|a| a.1.aggregate().calls);
    for (gate, dev, holdout) in small_gap.iter().take(5) {
        print_rule_row(gate, dev, holdout, &corpora);
    }

    // 4. 各訊號單維曲線（dev 取門檻，dev／holdout 都列出）。
    println!("\n## 訊號曲線（每格為 dev／holdout 命中率，最後一欄為呼叫率）");
    println!(
        "{:<30} {:>13} {:>13} {:>13} {:>8}",
        "訊號與門檻", "modern-500", "hard-cases", "ajimee-200", "呼叫"
    );
    let mut curve_rows = Vec::new();
    for signal in ALL_SIGNALS {
        for threshold in thresholds(signal, &corpora, 8) {
            let gate = Gate::new(Rule::single(signal, threshold));
            let dev = evaluate(&gate, &corpora, Some(true));
            let holdout = evaluate(&gate, &corpora, Some(false));
            let cells: Vec<String> = dev
                .per
                .iter()
                .zip(holdout.per.iter())
                .map(|(d, h)| format!("{:.1}／{:.1}", d.pct(), h.pct()))
                .collect();
            println!(
                "{:<30} {:>13} {:>13} {:>13} {:>7.1}%",
                format!("{} {:.4}", signal.short(), threshold),
                cells[0],
                cells[1],
                cells[2],
                dev.aggregate().call_rate()
            );
            curve_rows.push(json!({
                "signal": signal.short(),
                "threshold": threshold,
                "dev": eval_json(&dev, &corpora),
                "holdout": eval_json(&holdout, &corpora),
            }));
        }
    }

    // 4b. 同一批門檻，觸發後改用 small。
    println!("\n## 訊號曲線（觸發一律改用 small，其餘同上一節）");
    println!(
        "{:<30} {:>13} {:>13} {:>13} {:>8}",
        "訊號與門檻", "modern-500", "hard-cases", "ajimee-200", "呼叫"
    );
    for signal in ALL_SIGNALS {
        for threshold in thresholds(signal, &corpora, 8) {
            let gate = Gate {
                rule: Rule::single(signal, threshold),
                escalate: Some((Signal::Gap, f64::INFINITY)),
            };
            let dev = evaluate(&gate, &corpora, Some(true));
            let holdout = evaluate(&gate, &corpora, Some(false));
            let cells: Vec<String> = dev
                .per
                .iter()
                .zip(holdout.per.iter())
                .map(|(d, h)| format!("{:.1}／{:.1}", d.pct(), h.pct()))
                .collect();
            println!(
                "{:<30} {:>13} {:>13} {:>13} {:>7.1}%",
                format!("{} {:.4}", signal.short(), threshold),
                cells[0],
                cells[1],
                cells[2],
                dev.aggregate().call_rate()
            );
            curve_rows.push(json!({
                "signal": signal.short(),
                "threshold": threshold,
                "backend": "small",
                "dev": eval_json(&dev, &corpora),
                "holdout": eval_json(&holdout, &corpora),
            }));
        }
    }

    // 5. 統計邊界的平台區（xsmall 與 small 各一輪）。
    println!("\n## 整數門檻掃描（統計邊界）");
    let mut plateau = Vec::new();
    for (backend, escalate) in [
        ("xsmall", None),
        ("small", Some((Signal::Gap, f64::INFINITY))),
    ] {
        for threshold in [
            100.0, 150.0, 200.0, 250.0, 300.0, 350.0, 400.0, 450.0, 500.0, 600.0,
        ] {
            let gate = Gate {
                rule: Rule::single(Signal::Gap, threshold),
                escalate,
            };
            let dev = evaluate(&gate, &corpora, Some(true));
            let holdout = evaluate(&gate, &corpora, Some(false));
            let (p50, p95) = latency_overall(&gate, &corpora, None);
            plateau.push(json!({
                "backend": backend,
                "threshold": threshold,
                "dev": eval_json(&dev, &corpora),
                "holdout": eval_json(&holdout, &corpora),
                "all_p50_ms": p50,
                "all_p95_ms": p95,
            }));
            print_rule_row(&gate, &dev, &holdout, &corpora);
        }
    }

    // 6. 兩段式的升級條件：以第一名的單層規則為底，逐個訊號看門檻。
    println!("\n## 兩段式：xsmall → small 升級條件（底為上一節選中的門檻）");
    let base = chosen.first().map(|(gate, _, _)| gate.rule.clone());
    let mut two_stage = Vec::new();
    if let Some(base) = base {
        println!("底規則：{}", base.label());
        println!(
            "{:<46} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>8} {:>8}",
            "升級條件",
            "dev-mod",
            "dev-hard",
            "dev-aji",
            "ho-mod",
            "ho-hard",
            "ho-aji",
            "small率",
            "p50"
        );
        for signal in [Signal::Length, Signal::Kanji, Signal::PathDiff, Signal::Gap] {
            for threshold in thresholds(signal, &corpora, 6) {
                let gate = Gate {
                    rule: base.clone(),
                    escalate: Some((signal, threshold)),
                };
                let dev = evaluate(&gate, &corpora, Some(true));
                let holdout = evaluate(&gate, &corpora, Some(false));
                let p50 = latency_overall(&gate, &corpora, Some(true)).0;
                let small_rate =
                    100.0 * dev.aggregate().small_calls as f64 / dev.aggregate().n as f64;
                let cells = |eval: &Evaluation| -> Vec<String> {
                    eval.per
                        .iter()
                        .map(|part| format!("{:.1}", part.pct()))
                        .collect()
                };
                let d = cells(&dev);
                let h = cells(&holdout);
                println!(
                    "{:<46} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7.1}% {:>7.2}",
                    format!("{} {:.4}", signal.short(), threshold),
                    d[0],
                    d[1],
                    d[2],
                    h[0],
                    h[1],
                    h[2],
                    small_rate,
                    p50
                );
                two_stage.push(json!({
                    "base": base.label(),
                    "escalate": { "signal": signal.short(), "threshold": threshold },
                    "dev": eval_json(&dev, &corpora),
                    "holdout": eval_json(&holdout, &corpora),
                    "dev_p50_ms": p50,
                }));
            }
        }
    }

    // 7. 具名對照。
    println!("\n## 具名對照（全題庫）");
    let never = Rule {
        parts: Vec::new(),
        and: false,
    };
    let always_small = Gate {
        rule: Rule::single(Signal::Gap, 400.0),
        escalate: Some((Signal::Gap, f64::INFINITY)),
    };
    let named: Vec<(&str, Gate)> = vec![
        ("只用統計", Gate::new(never.clone())),
        (
            "xsmall 全用",
            Gate {
                rule: Rule::single(Signal::Gap, f64::INFINITY),
                escalate: None,
            },
        ),
        (
            "small 全用",
            Gate {
                rule: Rule::single(Signal::Gap, f64::INFINITY),
                escalate: Some((Signal::Gap, f64::INFINITY)),
            },
        ),
        (
            "gap < 400（xsmall）",
            Gate::new(Rule::single(Signal::Gap, 400.0)),
        ),
        ("gap < 400（small）", always_small.clone()),
        (
            "gap < 450（small）",
            Gate {
                rule: Rule::single(Signal::Gap, 450.0),
                escalate: Some((Signal::Gap, f64::INFINITY)),
            },
        ),
        (
            "gap < 460（small）",
            Gate {
                rule: Rule::single(Signal::Gap, 460.0),
                escalate: Some((Signal::Gap, f64::INFINITY)),
            },
        ),
        (
            "gap < 472.5 且 meancand > 6.0（small）",
            Gate {
                rule: Rule {
                    parts: vec![(Signal::Gap, 472.5), (Signal::MeanCandidates, 6.0)],
                    and: true,
                },
                escalate: Some((Signal::Gap, f64::INFINITY)),
            },
        ),
        (
            "gap < 500（small）",
            Gate {
                rule: Rule::single(Signal::Gap, 500.0),
                escalate: Some((Signal::Gap, f64::INFINITY)),
            },
        ),
        (
            "舊 AND：gap < 400 且 meancand > 5.7（xsmall）",
            Gate::new(Rule {
                parts: vec![(Signal::Gap, 400.0), (Signal::MeanCandidates, 5.7)],
                and: true,
            }),
        ),
        (
            "舊 AND（small）",
            Gate {
                rule: Rule {
                    parts: vec![(Signal::Gap, 400.0), (Signal::MeanCandidates, 5.7)],
                    and: true,
                },
                escalate: Some((Signal::Gap, f64::INFINITY)),
            },
        ),
        (
            "gap < 400，讀音 > 20 用 small",
            Gate {
                rule: Rule::single(Signal::Gap, 400.0),
                escalate: Some((Signal::Length, 20.0)),
            },
        ),
        (
            "dev 自動選中",
            chosen
                .first()
                .map(|(gate, _, _)| gate.clone())
                .unwrap_or_else(|| Gate::new(never.clone())),
        ),
    ];
    println!(
        "{:<50} {:>7} {:>7} {:>7} {:>7} {:>7} {:>6} {:>6} {:>4}",
        "規則", "modern", "hard", "ajimee", "呼叫率", "small率", "p50", "p95", "下限"
    );
    let mut named_rows = Vec::new();
    for (label, gate) in &named {
        let all = evaluate(gate, &corpora, None);
        let (p50, p95) = latency_overall(gate, &corpora, None);
        let cells: Vec<String> = all
            .per
            .iter()
            .map(|part| format!("{:.1}", part.pct()))
            .collect();
        println!(
            "{:<50} {:>7} {:>7} {:>7} {:>7.1}% {:>7.1}% {:>6.2} {:>6.2} {:>6}",
            label,
            cells[0],
            cells[1],
            cells[2],
            all.aggregate().call_rate(),
            100.0 * all.aggregate().small_calls as f64 / all.aggregate().n as f64,
            p50,
            p95,
            if all.feasible(&corpora) {
                "過"
            } else {
                "未達"
            }
        );
        named_rows.push(json!({
            "label": label,
            "rule": gate.label(),
            "all": eval_json(&all, &corpora),
            "p50_ms": p50,
            "p95_ms": p95,
        }));
    }

    // 8. 上限參考：用答案在整個題庫（含 holdout）挑單層門檻，只看趨勢。
    let mut upper: Vec<(Gate, Evaluation)> = Vec::new();
    for (gate, _, _) in &chosen {
        if gate.rule.parts.len() != 1 || gate.escalate.is_some() {
            continue;
        }
        upper.push((gate.clone(), evaluate(gate, &corpora, None)));
    }
    upper.sort_by(|a, b| {
        b.1.margin(&corpora)
            .partial_cmp(&a.1.margin(&corpora))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.aggregate().calls.cmp(&b.1.aggregate().calls))
    });
    println!("\n## 上限參考：用答案挑的門檻（不可當出貨值）");
    for (gate, all) in upper.iter().take(5) {
        let cells: Vec<String> = all
            .per
            .iter()
            .map(|part| format!("{:.1}", part.pct()))
            .collect();
        println!(
            "{:<50} {:>7} {:>7} {:>7} {:>7.1}%",
            gate.label(),
            cells[0],
            cells[1],
            cells[2],
            all.aggregate().call_rate()
        );
    }

    // 9. 輸出 JSON。
    let out = args
        .out
        .clone()
        .unwrap_or_else(|| args.dir.join("sweep.json"));
    let detail: Vec<_> = chosen
        .iter()
        .take(10)
        .map(|(gate, dev, holdout)| {
            json!({
                "label": gate.label(),
                "rule": gate.rule.describe(),
                "escalate": gate.escalate.map(|(signal, threshold)| json!({
                    "signal": signal.short(),
                    "threshold": threshold,
                })),
                "dev": eval_json(dev, &corpora),
                "holdout": eval_json(holdout, &corpora),
                "dev_p50_ms": latency_overall(gate, &corpora, Some(true)).0,
                "dev_p95_ms": latency_overall(gate, &corpora, Some(true)).1,
            })
        })
        .collect();
    write_json(
        &out,
        json!({
            "baseline": baseline_rows,
            "selected": detail,
            "signal_best": signal_best.iter().map(|(gate, dev, holdout)| json!({
                "label": gate.label(),
                "dev": eval_json(dev, &corpora),
                "holdout": eval_json(holdout, &corpora),
            })).collect::<Vec<_>>(),
            "curves": curve_rows,
            "plateau": plateau,
            "two_stage": two_stage,
            "named": named_rows,
            "upper_reference": upper.iter().take(5).map(|(gate, all)| json!({
                "label": gate.label(),
                "all": eval_json(all, &corpora),
            })).collect::<Vec<_>>(),
        }),
    )?;
    eprintln!("寫入 {}", out.display());
    Ok(())
}

fn print_rule_row(gate: &Gate, dev: &Evaluation, holdout: &Evaluation, corpora: &[Corpus]) {
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
    let (p50, _) = latency_overall(gate, corpora, Some(true));
    println!(
        "{:<52} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7.1}% {:>6.2}",
        gate.label(),
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
            "small_calls_pct": if part.n == 0 { 0.0 } else { 100.0 * part.small_calls as f64 / part.n as f64 },
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
            "--gguf2" => parsed.gguf2 = Some(PathBuf::from(value("--gguf2")?)),
            "--tokenizer2" => parsed.tokenizer2 = Some(PathBuf::from(value("--tokenizer2")?)),
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
            jinen2: None,
            jinen2_hit: false,
            signals: Signals {
                gap: Some(100),
                unknown_ratio: 0.5,
                reading_len,
                mean_candidates: 2.0,
                ambiguous_ratio: 0.5,
                max_candidates: 3,
                kanji_ratio: 0.5,
                max_unknown_run: 0,
                path_len_diff: 0,
            },
            checks: Checks {
                length_ok: true,
                echo_ok: true,
                emoji_ok: true,
            },
            checks2: Checks {
                length_ok: true,
                echo_ok: true,
                emoji_ok: true,
            },
            stats_ms: 1.0,
            jinen_ms: 20.0,
            jinen2_ms: 0.0,
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

    #[test]
    fn test_is_kanji_covers_cjk_ranges() {
        assert!(is_kanji('漢'));
        assert!(is_kanji('㐂'));
        assert!(!is_kanji('あ'));
        assert!(!is_kanji('。'));
        assert!(!is_kanji('ｱ'));
    }

    #[test]
    fn test_gate_escalates_to_second_model() {
        let mut row = row("a", 25);
        row.jinen2 = Some("亜".into());
        row.jinen2_hit = true;
        row.jinen_hit = false;
        let gate = Gate {
            rule: Rule::single(Signal::Gap, 400.0),
            escalate: Some((Signal::Length, 20.0)),
        };
        assert_eq!(gate.backend(&row), Backend::Jinen2);
        assert!(decide(&row, &gate, gate.backend(&row)));
    }

    #[test]
    fn test_gate_without_second_model_uses_xsmall() {
        let mut row = row("a", 25);
        row.jinen_hit = true;
        let gate = Gate {
            rule: Rule::single(Signal::Gap, 400.0),
            escalate: Some((Signal::Length, 20.0)),
        };
        assert_eq!(gate.backend(&row), Backend::Jinen);
        assert!(decide(&row, &gate, gate.backend(&row)));
    }

    #[test]
    fn test_gate_rejects_triggered_output_failing_checks() {
        let mut row = row("a", 5);
        row.jinen_hit = true;
        row.stats_hit = false;
        row.checks.length_ok = false;
        let gate = Gate::new(Rule::single(Signal::Gap, 400.0));
        assert!(!decide(&row, &gate, gate.backend(&row)));
    }
}
