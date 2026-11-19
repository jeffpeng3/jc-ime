//! 注音選字模型的 `weight`／`margin` 校準工具。
//!
//! dev 集只從 fuzzy 題庫的注音題產生：每題有讀音與正確答案，統計解碼器
//! 給出 N-best 後就有「統計首選」與「正確答案」的對照。詞典同音組只作為
//! 候選來源，答案仍以 fuzzy 的 `surface` 為準。golden 45 題不進入這個
//! 工具，只留給 `eval zh-golden` 當驗收。
//!
//! 用法：
//!
//! ```sh
//! cargo run -p jc-cli --release --features onnx --bin zh-calibrate -- \
//!   --dev /mnt/work/jc/jc-ime-data/0c-testdata/fuzzy/fuzzy-input.json
//! ```

#![cfg_attr(not(feature = "onnx"), allow(dead_code, unused_imports))]

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use jc_cli::cli::CliError;
use serde::Deserialize;
use serde_json::json;

/// 掃描的模型權重。
const WEIGHTS: [f64; 4] = [0.5, 1.0, 2.0, 3.0];
/// 掃描的覆寫門檻，單位是融合後的 nat。
const MARGINS: [f64; 7] = [0.0, 0.25, 0.5, 1.0, 1.5, 2.0, 3.0];
/// 每組最多取幾個統計候選，與模型上限相同。
const DEFAULT_K: usize = 16;

#[derive(Deserialize)]
struct FuzzyFile {
    items: Vec<FuzzyItem>,
}

#[derive(Deserialize)]
struct FuzzyItem {
    id: String,
    language: String,
    reading: String,
    #[serde(default)]
    noisy_reading: Option<String>,
    surface: String,
}

/// 一個 dev 組。
struct DevGroup {
    id: String,
    reading: String,
    syllables: Vec<String>,
    surface: String,
    /// 統計 N-best；第 0 個是統計首選。
    candidates: Vec<Candidate>,
}

/// 一個統計候選與它的模型分數。
struct Candidate {
    text: String,
    /// 統計分數，已換成與模型相同的 nat 尺度。
    log_score: f64,
    /// 送模型評分的合法候選；非法候選保留中性 0 分。
    valid: bool,
    neural: f64,
}

/// 一列掃描結果。
#[derive(Clone, Copy)]
struct ScanRow {
    weight: f64,
    margin: f64,
    top1: usize,
    stat_top1: usize,
    rescued: usize,
    broken: usize,
    overrides: usize,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("zh-calibrate：{err}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(feature = "onnx"))]
fn run() -> Result<(), CliError> {
    Err(CliError::eval(
        "這個建置沒有啟用 onnx feature，不能讀取選字模型分數".to_string(),
    ))
}

#[cfg(feature = "onnx")]
fn run() -> Result<(), CliError> {
    use jc_core::zh::select::{resolve_model_dir, SelectModel};

    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = Options::parse(&args)?;
    let testdata = resolve_testdata(options.testdata.clone(), &options.dev)?;
    let data_dir = match &options.data {
        Some(dir) => dir.clone(),
        None => jc_cli::data::data_root(&testdata).join("build-zh"),
    };
    if !data_dir.is_dir() {
        return Err(CliError::data(format!(
            "找不到注音資料包：{}（用 --data 指定）",
            data_dir.display()
        )));
    }
    let models_root = match &options.models {
        Some(dir) => dir.clone(),
        None => jc_cli::data::models_root(Some(&testdata)),
    };

    let groups = build_dev(
        &options.dev,
        &data_dir,
        options.limit,
        options.k,
        options.noisy,
        options.exact,
    )?;
    if groups.is_empty() {
        return Err(CliError::eval(
            "dev 集是空的：fuzzy 題庫沒有 >=2 音節的注音題".to_string(),
        ));
    }
    let model_dir = resolve_model_dir(&models_root)
        .map_err(|err| CliError::data(format!("找不到 zh-select 模型：{err}")))?;
    let model = SelectModel::load(&model_dir, None)
        .map_err(|err| CliError::data(format!("載入 zh-select 模型失敗：{err}")))?;
    let groups = score_groups(groups, &model);

    if let Some(path) = &options.out {
        write_groups(path, &groups)?;
    }
    let rows = scan(&groups);
    print_table(&groups, &rows)?;
    Ok(())
}

/// 命令列參數。
#[cfg(feature = "onnx")]
struct Options {
    dev: PathBuf,
    testdata: Option<PathBuf>,
    data: Option<PathBuf>,
    models: Option<PathBuf>,
    out: Option<PathBuf>,
    limit: Option<usize>,
    k: usize,
    /// 用原始讀音或擾動後的讀音建 dev；預設原始讀音。
    noisy: bool,
    /// 只解碼原讀音，不展開近音變體。
    exact: bool,
}

#[cfg(feature = "onnx")]
impl Options {
    fn parse(args: &[String]) -> Result<Self, CliError> {
        let mut options = Self {
            dev: PathBuf::new(),
            testdata: None,
            data: None,
            models: None,
            out: None,
            limit: None,
            k: DEFAULT_K,
            noisy: false,
            exact: false,
        };
        let mut index = 0;
        while index < args.len() {
            let flag = args[index].as_str();
            let value = |name: &str| -> Result<String, CliError> {
                args.get(index + 1)
                    .cloned()
                    .ok_or_else(|| CliError::Usage(format!("{name} 需要一個值")))
            };
            match flag {
                "--dev" => {
                    options.dev = PathBuf::from(value("--dev")?);
                    index += 1;
                }
                "--testdata" => {
                    options.testdata = Some(PathBuf::from(value("--testdata")?));
                    index += 1;
                }
                "--data" => {
                    options.data = Some(PathBuf::from(value("--data")?));
                    index += 1;
                }
                "--models" => {
                    options.models = Some(PathBuf::from(value("--models")?));
                    index += 1;
                }
                "--out" => {
                    options.out = Some(PathBuf::from(value("--out")?));
                    index += 1;
                }
                "--limit" => {
                    let raw = value("--limit")?;
                    options.limit = Some(
                        raw.parse()
                            .map_err(|_| CliError::Usage(format!("--limit 不是整數：{raw}")))?,
                    );
                    index += 1;
                }
                "--k" => {
                    let raw = value("--k")?;
                    options.k = raw
                        .parse()
                        .map_err(|_| CliError::Usage(format!("--k 不是整數：{raw}")))?;
                    index += 1;
                }
                "--reading" => {
                    let raw = value("--reading")?;
                    options.noisy = match raw.as_str() {
                        "original" => false,
                        "noisy" => true,
                        other => {
                            return Err(CliError::Usage(format!(
                                "--reading 只接受 original 或 noisy，實際為 {other}"
                            )));
                        }
                    };
                    index += 1;
                }
                "--exact" => options.exact = true,
                other => {
                    return Err(CliError::Usage(format!(
                        "zh-calibrate 不認得的參數 {other}"
                    )));
                }
            }
            index += 1;
        }
        if options.dev.as_os_str().is_empty() {
            return Err(CliError::Usage("需要 --dev <fuzzy-input.json>".to_string()));
        }
        if options.k == 0 || options.k > DEFAULT_K {
            return Err(CliError::Usage(format!(
                "--k 必須在 1..={DEFAULT_K}，實際為 {}",
                options.k
            )));
        }
        Ok(options)
    }
}

/// 由 `--dev` 路徑回推 `0c-testdata`，找不到時用預設值。
#[cfg(feature = "onnx")]
fn resolve_testdata(explicit: Option<PathBuf>, dev: &Path) -> Result<PathBuf, CliError> {
    if let Some(dir) = explicit {
        return jc_cli::data::testdata_dir(Some(dir));
    }
    for ancestor in dev.ancestors() {
        if ancestor.file_name().map(|name| name == "0c-testdata") == Some(true) {
            return jc_cli::data::testdata_dir(Some(ancestor.to_path_buf()));
        }
    }
    jc_cli::data::testdata_dir(None)
}

/// 從 fuzzy 題庫建立 dev 組。
///
/// 只用注音題、只取 >=2 音節；同一個（讀音、答案）只保留一次，避免同一組
/// 因四種擾動被重複計分。候選來自 `select` 模糊音模式展開的讀音變體，和
/// `eval fuzzy` 的注音候選同一條路徑；詞典同音組自然包含在 N-best 內。
#[cfg(feature = "onnx")]
fn build_dev(
    path: &Path,
    data_dir: &Path,
    limit: Option<usize>,
    k: usize,
    noisy: bool,
    exact: bool,
) -> Result<Vec<DevGroup>, CliError> {
    use jc_core::decode::{Decoder, Locks};
    use jc_core::lattice::Lattice;
    use jc_core::zh::fuzzy::{Config as FuzzyConfig, Mode as FuzzyMode};

    let raw = std::fs::read(path)
        .map_err(|err| CliError::data(format!("讀取 {} 失敗：{err}", path.display())))?;
    let file: FuzzyFile = serde_json::from_slice(&raw)
        .map_err(|err| CliError::data(format!("解析 {} 失敗：{err}", path.display())))?;
    let resources = jc_cli::data::load_zh_bin_resources(data_dir)?;
    let fuzzy = FuzzyConfig::all(FuzzyMode::Select);

    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut groups = Vec::new();
    for item in file.items {
        if item.language != "zh" {
            continue;
        }
        let reading = if noisy {
            item.noisy_reading.unwrap_or(item.reading)
        } else {
            item.reading
        };
        let syllables = jc_core::zh::syllable::parse_reading(&reading);
        if syllables.len() < 2 {
            continue;
        }
        if !seen.insert((reading.clone(), item.surface.clone())) {
            continue;
        }
        let variants = if exact {
            vec![jc_core::zh::fuzzy::Variant {
                syllables: syllables.clone(),
                cost_tenths: 0,
            }]
        } else {
            fuzzy.variants(&syllables)
        };
        let mut best: std::collections::BTreeMap<String, f64> = std::collections::BTreeMap::new();
        for variant in &variants {
            let readings: Vec<String> = variant
                .syllables
                .iter()
                .map(|syllable| syllable.as_str().to_string())
                .collect();
            let Ok(lattice) = Lattice::build(&readings, &resources.lexicon, &resources.ctx) else {
                continue;
            };
            let decoder = Decoder::new(&resources.lexicon, &resources.ctx);
            let (hypotheses, _) = decoder.nbest(&lattice, &Locks::new(), false, k);
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
        ranked.truncate(k);
        if ranked.len() < 2 {
            continue;
        }
        let candidates: Vec<Candidate> = ranked
            .into_iter()
            .map(|(text, cost)| Candidate {
                log_score: -cost / 1000.0,
                text,
                valid: false,
                neural: 0.0,
            })
            .collect();
        let syllables: Vec<String> = syllables
            .iter()
            .map(|syllable| syllable.as_str().to_string())
            .collect();
        groups.push(DevGroup {
            id: item.id,
            reading,
            syllables,
            surface: item.surface,
            candidates,
        });
        if let Some(limit) = limit {
            if groups.len() >= limit {
                break;
            }
        }
    }
    Ok(groups)
}

/// 對每組算模型分數；不合法候選保留中性 0。
#[cfg(feature = "onnx")]
fn score_groups(groups: Vec<DevGroup>, model: &jc_core::zh::select::SelectModel) -> Vec<DevGroup> {
    let vocab = model.vocab();
    groups
        .into_iter()
        .map(|mut group| {
            let valid: Vec<usize> = group
                .candidates
                .iter()
                .enumerate()
                .filter(|(_, candidate)| {
                    let chars: Vec<char> = candidate.text.chars().collect();
                    chars.len() == group.syllables.len()
                        && chars.iter().all(|ch| vocab.char_id(*ch).is_some())
                })
                .map(|(index, _)| index)
                .collect();
            for index in &valid {
                group.candidates[*index].valid = true;
            }
            if valid.len() < 2 || valid.len() > DEFAULT_K {
                return group;
            }
            let texts: Vec<&str> = valid
                .iter()
                .map(|index| group.candidates[*index].text.as_str())
                .collect();
            let Ok(scores) = model.score("", &group.syllables, &texts) else {
                return group;
            };
            for (position, index) in valid.iter().enumerate() {
                group.candidates[*index].neural = scores[position];
            }
            group
        })
        .collect()
}

/// 對固定網格跑 `choose`，統計救援與破壞。
#[cfg(feature = "onnx")]
fn scan(groups: &[DevGroup]) -> Vec<ScanRow> {
    use jc_core::zh::select::choose;

    let mut rows = Vec::with_capacity(WEIGHTS.len() * MARGINS.len());
    for weight in WEIGHTS {
        for margin in MARGINS {
            let mut row = ScanRow {
                weight,
                margin,
                top1: 0,
                stat_top1: 0,
                rescued: 0,
                broken: 0,
                overrides: 0,
            };
            for group in groups {
                let engine: Vec<f64> = group
                    .candidates
                    .iter()
                    .map(|candidate| candidate.log_score)
                    .collect();
                let neural: Vec<f64> = group
                    .candidates
                    .iter()
                    .map(|candidate| candidate.neural)
                    .collect();
                let winner = choose(&engine, &neural, weight, margin);
                let stat_ok = group.candidates[0].text == group.surface;
                let model_ok = group.candidates[winner].text == group.surface;
                row.stat_top1 += usize::from(stat_ok);
                row.top1 += usize::from(model_ok);
                row.overrides += usize::from(winner > 0);
                if !stat_ok && model_ok {
                    row.rescued += 1;
                }
                if stat_ok && !model_ok {
                    row.broken += 1;
                }
            }
            rows.push(row);
        }
    }
    rows
}

/// 依「先不破壞、再最大化救援」排序，並印出表格與選點。
#[cfg(feature = "onnx")]
fn print_table(groups: &[DevGroup], rows: &[ScanRow]) -> Result<(), CliError> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let reachable = groups
        .iter()
        .filter(|group| group.candidates.iter().any(|c| c.text == group.surface))
        .count();
    let stat_top1 = groups
        .iter()
        .filter(|group| group.candidates[0].text == group.surface)
        .count();
    writeln!(
        out,
        "dev 組 {}（答案在候選內 {}、統計首選正確 {}）",
        groups.len(),
        reachable,
        stat_top1
    )
    .map_err(CliError::Io)?;
    writeln!(out, "weight  margin  top1    rescue  broken  override").map_err(CliError::Io)?;
    let mut sorted: Vec<ScanRow> = rows.to_vec();
    sorted.sort_by(|a, b| {
        a.broken
            .cmp(&b.broken)
            .then_with(|| b.rescued.cmp(&a.rescued))
            .then_with(|| b.margin.total_cmp(&a.margin))
            .then_with(|| a.weight.total_cmp(&b.weight))
    });
    for row in &sorted {
        writeln!(
            out,
            "{:<6.2}  {:<6.2}  {:<5.1}%  {:<6}  {:<6}  {}",
            row.weight,
            row.margin,
            100.0 * row.top1 as f64 / groups.len() as f64,
            row.rescued,
            row.broken,
            row.overrides
        )
        .map_err(CliError::Io)?;
    }
    let limit = (groups.len() as f64 * 0.01).floor() as usize;
    let allowed = |row: &ScanRow| row.broken <= limit;
    let best = sorted.iter().find(|row| allowed(row));
    match best {
        Some(row) => {
            writeln!(
                out,
                "選點：weight={} margin={}（rescue {}、broken {}、top1 {}/{}）",
                row.weight,
                row.margin,
                row.rescued,
                row.broken,
                row.top1,
                groups.len()
            )
            .map_err(CliError::Io)?;
        }
        None => {
            writeln!(out, "沒有任何點滿足 broken <= {limit}").map_err(CliError::Io)?;
        }
    }
    Ok(())
}

/// 每組輸出一行 JSON，保留 `(correct, stat_top1_correct, model_score)` 與
/// 完整候選分數，方便回頭檢查選點。
#[cfg(feature = "onnx")]
fn write_groups(path: &Path, groups: &[DevGroup]) -> Result<(), CliError> {
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    for group in groups {
        let stat = &group.candidates[0];
        let candidates: Vec<_> = group
            .candidates
            .iter()
            .map(|candidate| {
                json!({
                    "text": candidate.text,
                    "log_score": candidate.log_score,
                    "valid": candidate.valid,
                    "model_score": candidate.neural,
                })
            })
            .collect();
        let row = json!({
            "id": group.id,
            "reading": group.reading,
            "surface": group.surface,
            "correct": group.candidates.iter().any(|c| c.text == group.surface),
            "stat_top1": stat.text,
            "stat_top1_correct": stat.text == group.surface,
            "model_score": stat.neural,
            "candidates": candidates,
        });
        writeln!(
            out,
            "{}",
            serde_json::to_string(&row)
                .map_err(|err| CliError::eval(format!("輸出 JSON 失敗：{err}")))?
        )?;
    }
    Ok(())
}
