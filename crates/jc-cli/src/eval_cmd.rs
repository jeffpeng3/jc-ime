//! `eval` 子命令：注音 golden、日文題庫與模糊輸入。
//!
//! 注音 golden 與日文題庫輸出 `jc-eval` 的 [`jc_eval::Report`]，模糊輸入
//! 輸出 [`jc_eval::FuzzyReport`]（語言×擾動的 raw／fallback 指標）。表格
//! 給人看，`--json` 給機器比對。注音 golden 直接驅動
//! [`jc_core::session::Session`]，因為它的案例是「按鍵序列」而不是單一讀音。

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use jc_core::decode::{Decoder, Locks};
use jc_core::ja::decode::JaContext;
use jc_core::ja::input::{RomajiEvent, RomajiInput};
use jc_core::key::{Key, KeyCode};
use jc_core::lang::Mode;
use jc_core::lattice::Lattice;
use jc_core::session::{RerankOptions, Session, SessionConfig};
use jc_core::zh::fuzzy as zh_fuzzy;
use jc_core::zh::fuzzy::{Config as FuzzyConfig, Mode as FuzzyMode};
use jc_core::zh::keyboard::Keyboard;
use jc_core::zh::syllable::class_of;
use jc_eval::{
    fuzzy_corpus_problems, load_fuzzy_input, load_fuzzy_manifest, load_golden_zh, load_ja_corpus,
    load_ja_corpus_set, load_rules, percentile, rank_of, run_ja, CaseResult, CorpusMetrics,
    CorpusRun, EngineInfo, FuzzyCaseResult, FuzzyEngineInfo, FuzzyReport, FuzzyRunConfig,
    GoldenAction, Report, Run, RunOptions,
};

use crate::cli::CliError;
use crate::converter::JaConverter;
use crate::data;

/// 注音 golden 的 45 個案例。
///
/// 直接從 `cases.json` 載入，逐題跑按鍵序列，最後比對 `expected/*.jsonl`
/// 的所有 commit 串接結果。任何一題失敗都不影響其他題。
pub fn zh_golden(
    testdata: Option<PathBuf>,
    json: bool,
    select_model: bool,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    let dir = data::testdata_dir(testdata)?;
    #[cfg(feature = "onnx")]
    if select_model {
        data::install_select_model(Some(&dir))?;
    } else {
        data::disable_select_model();
    }
    #[cfg(not(feature = "onnx"))]
    if select_model {
        return Err(CliError::eval(
            "這個建置沒有啟用 onnx feature，不能使用 --select-model on".to_string(),
        ));
    }
    let golden_dir = dir.join("golden/zh");
    let suite = load_golden_zh(&golden_dir)
        .map_err(|err| CliError::eval(format!("讀取注音 golden 失敗：{err}")))?;
    let resources = data::load_zh_resources(&golden_dir.join("data"))?;
    let converter = if suite.cases().iter().any(|case| case.options.simplified) {
        Some(Arc::new(data::load_zhconv(&data::data_root(&dir))?))
    } else {
        None
    };

    let mut items = Vec::with_capacity(suite.cases().len());
    let mut durations = Vec::with_capacity(suite.cases().len());
    let mut passed = 0usize;
    let mut failures = Vec::new();

    for case in suite.cases() {
        let mode = if case.options.mixed_input {
            Mode::Mixed
        } else {
            Mode::Zh
        };
        let mut config = data::zh_session_config(mode, &resources);
        let paging = case.actions.iter().any(|action| {
            action
                .request
                .as_ref()
                .and_then(|request| request.get("type"))
                .and_then(|kind| kind.as_str())
                == Some("page")
        });
        apply_options(&mut config, &case.options, paging, converter.as_ref());
        let mut session = Session::new(config);
        let started = Instant::now();
        let outcome = drive_case(&mut session, &case.actions);
        let duration_ms = started.elapsed().as_secs_f64() * 1000.0;
        let expected = read_expected(
            &golden_dir
                .join("expected")
                .join(format!("{}.jsonl", case.name)),
        );
        let (actual, supported) = match outcome {
            Ok(commits) => (commits.concat(), true),
            Err(message) => {
                failures.push((
                    case.name.clone(),
                    expected.clone(),
                    format!("未支援：{message}"),
                ));
                (String::new(), false)
            }
        };
        let exact = supported && actual == expected;
        if exact {
            passed += 1;
        } else if supported {
            failures.push((case.name.clone(), expected.clone(), actual.clone()));
        }
        durations.push(duration_ms);
        items.push(CaseResult {
            id: case.name.clone(),
            reading: case_reading(&case.actions),
            reference: expected.clone(),
            output: actual,
            top5: Vec::new(),
            top1: exact,
            top5_hit: exact,
            boundary_scored: false,
            boundary_exact: false,
            boundary_tp: 0,
            boundary_fp: 0,
            boundary_fn: 0,
            duration_ms,
            timed_out: false,
        });
    }

    let n = items.len();
    let metrics = CorpusMetrics {
        n,
        top1_rate: ratio(passed, n),
        top5_rate: ratio(passed, n),
        boundary_n: 0,
        boundary_exact_rate: 0.0,
        boundary_f1: 0.0,
        latency_p50_ms: percentile(&durations, 50.0),
        latency_p95_ms: percentile(&durations, 95.0),
        timeout_rate: 0.0,
    };
    let mut report = Report::new(engine_info(&dir), "");
    report.insert("zh-golden", metrics);
    let run = Run {
        report,
        corpora: vec![CorpusRun {
            name: "zh-golden".to_string(),
            license: "CC0-1.0".to_string(),
            source: "0c-testdata/golden/zh".to_string(),
            sha256: String::new(),
            items,
        }],
    };
    if json {
        writeln!(
            out,
            "{}",
            run.to_json_pretty()
                .map_err(|err| CliError::eval(format!("輸出 JSON 失敗：{err}")))?
        )?;
        return Ok(());
    }
    write!(out, "{}", run.to_table())?;
    writeln!(out, "通過 {passed}/{n}")?;
    for (name, expected, actual) in &failures {
        writeln!(out, "  ✗ {name}: 期望 {expected:?}，得到 {actual:?}")?;
    }
    if select_model {
        write_select_stats(out)?;
    }
    Ok(())
}

/// 印出選字模型的推論延遲。
#[cfg(feature = "onnx")]
fn write_select_stats(out: &mut dyn Write) -> Result<(), CliError> {
    let Some(stats) = data::select_model_stats() else {
        writeln!(out, "select-model：沒有推論")?;
        return Ok(());
    };
    writeln!(
        out,
        "select-model：推論 {} 次，p50 {:.2} ms，p95 {:.2} ms",
        stats.calls, stats.p50_ms, stats.p95_ms
    )?;
    Ok(())
}

#[cfg(not(feature = "onnx"))]
fn write_select_stats(_out: &mut dyn Write) -> Result<(), CliError> {
    Ok(())
}

/// 跑單一日文題庫。
///
/// `corpus` 可以是 `rules.json` 裡的題庫名稱，也可以是 `.json` 路徑；後者
/// 讓 CLI 與 `ja-eval` 能用同一份小型題庫對照。`pack` 未指定時走
/// [`data::resolve_ja_pack`] 的預設順序。
pub fn ja(
    testdata: Option<PathBuf>,
    corpus: &str,
    pack: Option<PathBuf>,
    json: bool,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    let dir = data::testdata_dir(testdata)?;
    let rules_path = dir.join("ja/rules.json");
    let rules = load_rules(&rules_path)
        .map_err(|err| CliError::eval(format!("讀取 {} 失敗：{err}", rules_path.display())))?;
    let selected = if Path::new(corpus).is_file() {
        vec![load_ja_corpus(Path::new(corpus))
            .map_err(|err| CliError::eval(format!("讀取 {corpus} 失敗：{err}")))?]
    } else {
        let entries = load_ja_corpus_set(&rules, &rules_path)
            .map_err(|err| CliError::eval(format!("讀取日文題庫失敗：{err}")))?;
        let selected: Vec<_> = entries
            .into_iter()
            .filter(|entry| entry.corpus.name == corpus)
            .collect();
        if selected.is_empty() {
            return Err(CliError::eval(format!("rules.json 沒有題庫 {corpus:?}")));
        }
        selected
    };
    let decoder = data::load_ja_decoder_with_pack(&data::data_root(&dir), pack.as_deref())?;
    let converter = JaConverter::new(decoder);
    let options = RunOptions::from_rules(&rules);
    let run = run_ja(&converter, &rules, &selected, engine_info(&dir), &options);
    if json {
        writeln!(
            out,
            "{}",
            run.to_json_pretty()
                .map_err(|err| CliError::eval(format!("輸出 JSON 失敗：{err}")))?
        )?;
    } else {
        write!(out, "{}", run.to_table())?;
    }
    Ok(())
}

/// 跑模糊輸入題庫。
///
/// 注音走產品預設的 `select` 模糊音設定：開候選時合併近音，不改自動顯示；
/// 日文走同一個統計轉換器的 N-best。沒有日文 pack 時日文案例仍留在分母，
/// 候選指標全部算未命中，`no_decoder` 記在降級原因。
pub fn fuzzy(testdata: Option<PathBuf>, json: bool, out: &mut dyn Write) -> Result<(), CliError> {
    let dir = data::testdata_dir(testdata)?;
    let path = dir.join("fuzzy/fuzzy-input.json");
    let manifest_path = dir.join("fuzzy/fuzzy-input.manifest.json");
    let (corpus, sha) = load_fuzzy_input(&path)
        .map_err(|err| CliError::eval(format!("讀取 {} 失敗：{err}", path.display())))?;
    let (manifest, _manifest_sha) = load_fuzzy_manifest(&manifest_path)
        .map_err(|err| CliError::eval(format!("讀取 {} 失敗：{err}", manifest_path.display())))?;
    let problems = fuzzy_corpus_problems(&corpus, &sha, &manifest);
    if !problems.is_empty() {
        return Err(CliError::eval(format!(
            "manifest 與題庫不符：{}",
            problems.join("；")
        )));
    }

    let resources = data::load_zh_resources(&dir.join("golden/zh/data"))?;
    let root = data::data_root(&dir);
    let ja = data::load_ja_decoder(&root).ok().map(Arc::new);
    let pack = data::resolve_ja_pack(&root, None).ok();
    let config = FuzzyConfig::all(FuzzyMode::Select);

    let mut report = FuzzyReport::new(corpus.name.clone(), sha.clone(), corpus.seed);
    report.engine = FuzzyEngineInfo {
        data_dir: dir.display().to_string(),
        lexicon_sha256: file_sha256(&dir.join("golden/zh/data/lexicon.tsv")),
        lm_sha256: file_sha256(&dir.join("golden/zh/data/lm.tsv")),
        japanese_pack: pack.as_ref().map(|path| jc_eval::FuzzyPackInfo {
            path: path.display().to_string(),
            sha256: file_sha256(path),
            entries: 0,
        }),
        models: "none".to_string(),
        models_used: false,
        quality: manifest.quality.clone(),
    };
    report.config = FuzzyRunConfig {
        top_n: FUZZY_TOP_N,
        fuzzy: config.key(),
        ranker: "score.Ranker statistical fallback (no model)".to_string(),
        models: "none".to_string(),
        quality: manifest.quality.clone(),
    };

    for item in &corpus.items {
        let started = Instant::now();
        let mut case = FuzzyCaseResult {
            id: item.id.clone(),
            language: item.language.clone(),
            kind: item.kind.clone(),
            reference: item.surface.clone(),
            input: item.input.clone(),
            noisy_reading: item.noisy_reading.clone(),
            pending: item.pending.clone(),
            ..FuzzyCaseResult::default()
        };
        let mut degraded = Vec::new();
        match item.language.as_str() {
            "zh" => run_fuzzy_zh(item, &resources, config, &mut case, &mut degraded),
            "ja" => run_fuzzy_ja(item, ja.as_ref(), &mut case, &mut degraded),
            other => degraded.push(format!("unknown_language:{other}")),
        }
        case.degraded = !degraded.is_empty();
        case.degraded_reason = degraded.join(",");
        case.duration_ms = started.elapsed().as_secs_f64() * 1000.0;
        report.cases.push(case);
    }
    report.summarize();

    if json {
        writeln!(
            out,
            "{}",
            report
                .to_json_pretty()
                .map_err(|err| CliError::eval(format!("輸出 JSON 失敗：{err}")))?
        )?;
    } else {
        write!(out, "{}", report.to_table())?;
    }
    Ok(())
}

/// 把 golden 案例的引擎選項套進 session 設定。
///
/// 只涵蓋 `jc-core` 已實作的選項；尚未接線的（簡繁轉換、混合輸入、
/// 英文補完、補全門檻）留在分類表說明。
fn apply_options(
    config: &mut SessionConfig,
    options: &jc_eval::GoldenOptions,
    paging: bool,
    converter: Option<&Arc<jc_core::zhconv::Converter>>,
) {
    config.toneless = options.toneless;
    config.ctrl_punct = options.ctrl_punct;
    config.english_autocomplete = options.english_autocomplete;
    config.cjk_latin_space = options.cjk_latin_space;
    if options.simplified {
        config.simplified = converter.cloned();
    }
    // 翻頁案例需要第二頁的候選；其餘維持預設 5 個，避免長句 k-best 變慢。
    if paging {
        config.max_candidates = 13;
    }
    config.rerank = options.rerank.as_ref().map(|rerank| RerankOptions {
        weight: rerank.weight,
        prefer: rerank.prefer.clone(),
        prefer_score: rerank.prefer_score,
    });
    config.punct_mark = options
        .punct
        .as_ref()
        .and_then(|punct| punct.mark.chars().next());
    config.fuzzy_pairs = options
        .fuzzy
        .iter()
        .filter_map(|(id, mode)| fuzzy_pair(id, mode))
        .collect();
}

/// 模糊音配對 id 對應到兩個注音符號；只接受 `select` 模式。
fn fuzzy_pair(id: &str, mode: &str) -> Option<(char, char)> {
    if mode != "select" {
        return None;
    }
    match id {
        "z-zh" => Some(('ㄗ', 'ㄓ')),
        "c-ch" => Some(('ㄘ', 'ㄔ')),
        "s-sh" => Some(('ㄙ', 'ㄕ')),
        "en-eng" => Some(('ㄣ', 'ㄥ')),
        _ => None,
    }
}

/// 模糊題庫一次取幾個候選。
const FUZZY_TOP_N: usize = 8;

/// 注音案例：展開近音與無聲調讀音，合併各變體的 N-best。
fn run_fuzzy_zh(
    item: &jc_eval::FuzzyItem,
    resources: &data::ZhResources,
    config: FuzzyConfig,
    case: &mut FuzzyCaseResult,
    degraded: &mut Vec<String>,
) {
    if item.noisy_reading.is_empty() {
        degraded.push("no_reading".to_string());
        set_fuzzy_empty(case);
        case.miss_class = "noise".to_string();
        return;
    }
    let syllables = jc_core::zh::syllable::parse_reading(&item.noisy_reading);
    if syllables.is_empty() {
        degraded.push("no_reading".to_string());
        set_fuzzy_empty(case);
        case.miss_class = "noise".to_string();
        return;
    }
    let variants = config.variants(&syllables);
    let mut best: BTreeMap<String, i32> = BTreeMap::new();
    for variant in &variants {
        let readings: Vec<String> = variant
            .syllables
            .iter()
            .map(|syl| syl.as_str().to_string())
            .collect();
        let Ok(lattice) = Lattice::build(&readings, &resources.lexicon, &resources.ctx) else {
            continue;
        };
        let decoder = Decoder::new(&resources.lexicon, &resources.ctx);
        let (hypotheses, _) = decoder.nbest(&lattice, &Locks::new(), false, FUZZY_TOP_N);
        for hypothesis in hypotheses {
            let cost = hypothesis
                .cost
                .saturating_mul(10)
                .saturating_add(variant.cost_tenths as i32);
            best.entry(hypothesis.text)
                .and_modify(|old| *old = (*old).min(cost))
                .or_insert(cost);
        }
    }
    let mut ranked: Vec<(String, i32)> = best.into_iter().collect();
    ranked.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let surfaces: Vec<String> = ranked.into_iter().map(|(surface, _)| surface).collect();
    if surfaces.is_empty() {
        degraded.push("no_candidates".to_string());
    }
    case.rank = rank_of(&surfaces, &item.surface);
    case.raw_top5 = surfaces.iter().take(5).cloned().collect();
    case.output = surfaces.first().cloned().unwrap_or_default();
    case.intended = case.output == item.surface;
    if case.output.is_empty() {
        degraded.push("no_commit".to_string());
    }
    case.fallback_rank = case.rank;
    case.fallback_top5 = case.raw_top5.clone();
    case.fallback_intended = case.fallback_rank == 1;
    case.fallback_outcome = if surfaces.is_empty() {
        "none".to_string()
    } else {
        "fallback".to_string()
    };
    if case.rank == 0 {
        case.miss_class = classify_zh_miss(item, resources, &variants);
    }
}

/// 分類注音失誤：規則沒展開、詞典沒有、或候選排序把答案擠出前五。
fn classify_zh_miss(
    item: &jc_eval::FuzzyItem,
    resources: &data::ZhResources,
    variants: &[zh_fuzzy::Variant],
) -> String {
    use jc_core::lattice::Lexicon;
    let mut original_reachable = false;
    let mut surface_reachable = false;
    for variant in variants {
        let reading = variant
            .syllables
            .iter()
            .map(|syl| syl.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if reading == item.reading {
            original_reachable = true;
        }
        if resources
            .lexicon
            .lookup(&reading)
            .iter()
            .any(|entry| entry.surface == item.surface)
        {
            surface_reachable = true;
            break;
        }
    }
    if surface_reachable {
        "ranking".to_string()
    } else if original_reachable {
        "dictionary".to_string()
    } else if item.kind == "near-sound" {
        "rule".to_string()
    } else {
        "noise".to_string()
    }
}

/// 日文案例：統計轉換器的 N-best，加上羅馬字工作階段的實際提交。
fn run_fuzzy_ja(
    item: &jc_eval::FuzzyItem,
    decoder: Option<&Arc<jc_core::ja::decode::JaDecoder>>,
    case: &mut FuzzyCaseResult,
    degraded: &mut Vec<String>,
) {
    let mut romaji = RomajiInput::new();
    let mut unhandled = 0;
    for ch in item.input.chars() {
        if romaji.press(Key::char(ch, false)) == RomajiEvent::Rejected {
            unhandled += 1;
        }
    }
    if unhandled > 0 || !item.pending.is_empty() {
        degraded.push("unhandled_keys".to_string());
    }
    if item.noisy_reading.is_empty() {
        degraded.push("no_reading".to_string());
        set_fuzzy_empty(case);
        case.miss_class = "dictionary".to_string();
        return;
    }
    let Some(decoder) = decoder else {
        degraded.push("no_decoder".to_string());
        set_fuzzy_empty(case);
        case.miss_class = "dictionary".to_string();
        return;
    };
    let results = decoder.nbest(&item.noisy_reading, &JaContext::default(), FUZZY_TOP_N);
    let surfaces: Vec<String> = results
        .iter()
        .map(|result| result.surface.clone())
        .filter(|surface| !surface.is_empty())
        .collect();
    if surfaces.is_empty() {
        degraded.push("no_candidates".to_string());
    }
    case.rank = rank_of(&surfaces, &item.surface);
    case.raw_top5 = surfaces.iter().take(5).cloned().collect();
    case.fallback_rank = case.rank;
    case.fallback_top5 = case.raw_top5.clone();
    case.fallback_intended = case.fallback_rank == 1;
    case.fallback_outcome = if surfaces.is_empty() {
        "none".to_string()
    } else {
        "fallback".to_string()
    };

    let mut session = Session::new(jc_core::session::SessionConfig {
        mode: Mode::Ja,
        max_candidates: FUZZY_TOP_N,
        ja: Some(Arc::clone(decoder)),
        ..jc_core::session::SessionConfig::default()
    });
    for ch in item.input.chars() {
        session.press(Key::char(ch, false));
    }
    session.press(Key::new(KeyCode::Enter, None, false));
    case.output = session.commit();
    case.intended = case.output == item.surface;
    if case.output.is_empty() {
        degraded.push("no_commit".to_string());
    }
    if case.rank == 0 {
        case.miss_class = "dictionary".to_string();
    }
}

fn set_fuzzy_empty(case: &mut FuzzyCaseResult) {
    case.rank = 0;
    case.raw_top5.clear();
    case.output.clear();
    case.intended = false;
    case.fallback_rank = 0;
    case.fallback_top5.clear();
    case.fallback_intended = false;
    case.fallback_outcome = "none".to_string();
}

fn file_sha256(path: &Path) -> String {
    let Ok(raw) = std::fs::read(path) else {
        return String::new();
    };
    jc_eval::sha256_hex(&raw)
}

/// 跑一個 golden 案例，回傳依序的 commit 字串。
fn drive_case(session: &mut Session, actions: &[GoldenAction]) -> Result<Vec<String>, String> {
    let mut commits = Vec::new();
    for action in actions {
        if !action.syllables.is_empty() {
            type_syllables(session, &action.syllables)?;
        } else if !action.bare.is_empty() {
            type_bare(session, &action.bare)?;
        } else if !action.keys.is_empty() {
            for ch in action.keys.chars() {
                session.press(Key::char(ch, false));
            }
        } else if !action.key.is_empty() {
            if let Some(key) = named_key(&action.key)? {
                session.press(key);
                if key.code == KeyCode::Enter {
                    let text = session.commit();
                    if !text.is_empty() {
                        commits.push(text);
                    }
                }
            }
        } else if let Some(request) = &action.request {
            apply_request(session, request)?;
        }
    }
    Ok(commits)
}

fn type_syllables(session: &mut Session, reading: &str) -> Result<(), String> {
    let keyboard = Keyboard::standard();
    for syllable in jc_core::zh::syllable::parse_reading(reading) {
        let keys = keyboard
            .keys(&syllable)
            .map_err(|err| format!("音節 {} 沒有按鍵：{err}", syllable.as_str()))?;
        for ch in keys.chars() {
            session.press(Key::char(ch, false));
        }
    }
    Ok(())
}

/// 直接打注音符號：把符號換成標準排列的按鍵。
///
/// `bare` 沒有聲調，遇到類別順序回頭或 `'` 分隔符就視為下一個音節，補一聲
/// 完成後再繼續。
fn type_bare(session: &mut Session, bare: &str) -> Result<(), String> {
    let mut current: Vec<char> = Vec::new();
    let mut last = jc_core::zh::syllable::Class::None;
    let flush = |session: &mut Session, current: &mut Vec<char>| -> Result<(), String> {
        if current.is_empty() {
            return Ok(());
        }
        for &symbol in current.iter() {
            let key =
                reverse_key(symbol).ok_or_else(|| format!("注音符號 {symbol:?} 沒有對應按鍵"))?;
            session.press(Key::char(key, false));
        }
        session.press(Key::new(KeyCode::Space, None, false));
        current.clear();
        Ok(())
    };
    for ch in bare.chars() {
        if ch == '\'' {
            flush(session, &mut current)?;
            last = jc_core::zh::syllable::Class::None;
            continue;
        }
        let class = class_of(ch);
        if class == jc_core::zh::syllable::Class::None {
            continue;
        }
        if !current.is_empty() && class <= last {
            flush(session, &mut current)?;
        }
        current.push(ch);
        last = class;
    }
    flush(session, &mut current)?;
    Ok(())
}

fn reverse_key(symbol: char) -> Option<char> {
    ('!'..='~').find(|key| Keyboard::standard().symbol(*key) == Some(symbol))
}

/// 具名鍵對照；回傳 `Ok(None)` 代表不需要送鍵（目前沒有這種鍵）。
fn named_key(name: &str) -> Result<Option<Key>, String> {
    let code = match name {
        "Enter" => KeyCode::Enter,
        "Escape" => KeyCode::Escape,
        "Backspace" => KeyCode::Backspace,
        "Delete" => KeyCode::Delete,
        "Tab" => KeyCode::Tab,
        "Space" => KeyCode::Space,
        "Left" | "Up" => KeyCode::ArrowLeft,
        "Right" | "Down" => KeyCode::ArrowRight,
        "Ctrl+," => {
            return Ok(Some(
                Key::new(KeyCode::Char(','), Some(','), false).with_ctrl(),
            ));
        }
        other => {
            let mut chars = other.chars();
            if let (Some(ch), None) = (chars.next(), chars.next()) {
                return Ok(Some(Key::char(ch, false)));
            }
            return Err(format!("按鍵 {other:?}"));
        }
    };
    Ok(Some(Key::new(code, None, false)))
}

fn apply_request(session: &mut Session, request: &serde_json::Value) -> Result<(), String> {
    let kind = request
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    match kind {
        "select" => {
            let index = request
                .get("index")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as usize;
            session.select(index);
            Ok(())
        }
        "page" => {
            let index = request
                .get("index")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as usize;
            session.page(index);
            Ok(())
        }
        "toggle" => {
            session.toggle_english();
            Ok(())
        }
        "forget" => Ok(()),
        other => Err(format!("請求 {other:?}")),
    }
}

/// 讀 `expected/*.jsonl` 的所有 commit，依序串接。
fn read_expected(path: &Path) -> String {
    let Ok(text) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let mut out = String::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(commit) = value.get("commit").and_then(|value| value.as_str()) {
                out.push_str(commit);
            }
        }
    }
    out
}

fn case_reading(actions: &[GoldenAction]) -> String {
    for action in actions {
        if !action.syllables.is_empty() {
            return action.syllables.clone();
        }
        if !action.bare.is_empty() {
            return action.bare.clone();
        }
        if !action.keys.is_empty() {
            return action.keys.clone();
        }
    }
    String::new()
}

fn ratio(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

fn engine_info(dir: &Path) -> EngineInfo {
    EngineInfo {
        name: format!("jc-cli {}", env!("CARGO_PKG_VERSION")),
        data: dir.display().to_string(),
        ..EngineInfo::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys_follow_protocol_names() {
        assert_eq!(
            named_key("Enter").unwrap().map(|key| key.code),
            Some(KeyCode::Enter)
        );
        assert_eq!(
            named_key("Down").unwrap().map(|key| key.code),
            Some(KeyCode::ArrowRight)
        );
        assert_eq!(
            named_key("<").unwrap().map(|key| key.code),
            Some(KeyCode::Char('<'))
        );
        assert!(named_key("Ctrl+,").is_err());
    }

    #[test]
    fn zh_golden_runs_all_forty_five_cases() {
        let Some(dir) = jc_testdata::path("0c-testdata") else {
            eprintln!("缺少測試資料：0c-testdata");
            return;
        };
        let mut out = Vec::new();
        zh_golden(Some(dir), false, false, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("zh-golden"), "{text}");
        assert!(text.contains("通過"), "{text}");
        assert!(
            text.lines().any(|line| line.contains("45")),
            "45 個案例都要跑到：\n{text}"
        );
    }

    #[test]
    fn fuzzy_reports_language_and_kind_groups() {
        let dir = jc_testdata::require_data!("0c-testdata/fuzzy/fuzzy-input.json");
        let testdata = dir.parent().and_then(Path::parent).unwrap().to_path_buf();
        let mut out = Vec::new();
        fuzzy(Some(testdata), true, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let report: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(report["corpus"], "fuzzy-input");
        assert_eq!(report["cases"].as_array().unwrap().len(), 919);
        let groups = report["groups"].as_array().unwrap();
        assert!(groups.iter().any(|group| {
            group["language"] == "zh" && group["kind"] == "near-sound" && group["n"] == 43
        }));
        assert!(groups.iter().any(|group| {
            group["language"] == "ja" && group["kind"] == "wrong-key" && group["n"] == 200
        }));
        let all = groups
            .iter()
            .find(|group| group["language"].is_null() && group["kind"].is_null())
            .expect("缺少全體合計");
        assert_eq!(all["n"], 919);
        assert_eq!(all["raw"]["n"], 919);
        assert_eq!(all["fallback"]["n"], 919);
    }

    #[test]
    fn fuzzy_near_sound_reaches_reference() {
        let dir = jc_testdata::require_data!("0c-testdata/fuzzy/fuzzy-input.json");
        let testdata = dir.parent().and_then(Path::parent).unwrap().to_path_buf();
        let mut out = Vec::new();
        fuzzy(Some(testdata), true, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let report: serde_json::Value = serde_json::from_str(&text).unwrap();
        let group = report["groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|group| group["language"] == "zh" && group["kind"] == "near-sound")
            .expect("缺少 zh/near-sound");
        assert_eq!(group["n"], 43);
        assert_eq!(group["raw"]["top1"], 43);
        assert_eq!(group["raw"]["top5"], 43);
    }

    #[test]
    fn fuzzy_rejects_manifest_mismatch() {
        let dir = jc_testdata::require_data!("0c-testdata/fuzzy/fuzzy-input.json");
        let source = dir.parent().and_then(Path::parent).unwrap();
        let work = std::env::temp_dir().join(format!("jc-cli-fuzzy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(work.join("fuzzy")).unwrap();
        std::fs::copy(&dir, work.join("fuzzy/fuzzy-input.json")).unwrap();
        let manifest = source.join("fuzzy/fuzzy-input.manifest.json");
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(manifest).unwrap()).unwrap();
        value["sha256"] = serde_json::Value::String("0".repeat(64));
        std::fs::write(
            work.join("fuzzy/fuzzy-input.manifest.json"),
            serde_json::to_string(&value).unwrap(),
        )
        .unwrap();
        let mut out = Vec::new();
        let err = fuzzy(Some(work.clone()), false, &mut out).unwrap_err();
        assert!(err.to_string().contains("manifest"), "{err}");
        std::fs::remove_dir_all(&work).unwrap();
    }

    #[test]
    fn expected_commits_concatenate() {
        let dir = std::env::temp_dir().join(format!("jc-cli-expected-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x.jsonl");
        std::fs::write(
            &path,
            "{\"commit\":\"我\"}\n{\"handled\":true}\n{\"commit\":\"想\"}\n",
        )
        .unwrap();
        assert_eq!(read_expected(&path), "我想");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
