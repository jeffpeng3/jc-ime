//! `eval` 子命令：注音 golden、日文題庫與模糊輸入。
//!
//! 輸出都是 `jc-eval` 的 [`jc_eval::Report`]：表格給人看，`--json` 給機器
//! 比對。注音 golden 直接驅動 [`jc_core::session::Session`]，因為它的案例是
//! 「按鍵序列」而不是單一讀音。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use jc_core::key::{Key, KeyCode};
use jc_core::lang::Mode;
use jc_core::session::{RerankOptions, Session, SessionConfig};
use jc_core::zh::keyboard::Keyboard;
use jc_core::zh::syllable::class_of;
use jc_eval::{
    evaluate, load_fuzzy_input, load_golden_zh, load_ja_corpus_set, load_rules, percentile, run_ja,
    CaseResult, CorpusMetrics, CorpusRun, EngineInfo, EvalCase, GoldenAction, Report, Run,
    RunOptions,
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
    out: &mut dyn Write,
) -> Result<(), CliError> {
    let dir = data::testdata_dir(testdata)?;
    let golden_dir = dir.join("golden/zh");
    let suite = load_golden_zh(&golden_dir)
        .map_err(|err| CliError::eval(format!("讀取注音 golden 失敗：{err}")))?;
    let resources = data::load_zh_resources(&golden_dir.join("data"))?;

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
        apply_options(&mut config, &case.options, paging);
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
    Ok(())
}

/// 跑單一日文題庫。
pub fn ja(
    testdata: Option<PathBuf>,
    corpus: &str,
    json: bool,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    let dir = data::testdata_dir(testdata)?;
    let rules_path = dir.join("ja/rules.json");
    let rules = load_rules(&rules_path)
        .map_err(|err| CliError::eval(format!("讀取 {} 失敗：{err}", rules_path.display())))?;
    let entries = load_ja_corpus_set(&rules, &rules_path)
        .map_err(|err| CliError::eval(format!("讀取日文題庫失敗：{err}")))?;
    let selected: Vec<_> = entries
        .into_iter()
        .filter(|entry| entry.corpus.name == corpus)
        .collect();
    if selected.is_empty() {
        return Err(CliError::eval(format!("rules.json 沒有題庫 {corpus:?}")));
    }
    let decoder = data::load_ja_decoder(&data::data_root(&dir))?;
    let converter = JaConverter::new(Arc::new(decoder));
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
pub fn fuzzy(testdata: Option<PathBuf>, json: bool, out: &mut dyn Write) -> Result<(), CliError> {
    let dir = data::testdata_dir(testdata)?;
    let path = dir.join("fuzzy/fuzzy-input.json");
    let (corpus, _sha) = load_fuzzy_input(&path)
        .map_err(|err| CliError::eval(format!("讀取 {} 失敗：{err}", path.display())))?;
    let cases: Vec<EvalCase> = corpus.items.iter().map(|item| item.eval_case()).collect();
    let converter = crate::converter::load_auto(&dir)?;
    let evaluation = evaluate(&converter, &cases, &RunOptions::default());
    let mut report = Report::new(engine_info(&dir), "");
    report.insert(corpus.name.clone(), evaluation.metrics.clone());
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
fn apply_options(config: &mut SessionConfig, options: &jc_eval::GoldenOptions, paging: bool) {
    config.toneless = options.toneless;
    config.ctrl_punct = options.ctrl_punct;
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
        zh_golden(Some(dir), false, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("zh-golden"), "{text}");
        assert!(text.contains("通過"), "{text}");
        assert!(
            text.lines().any(|line| line.contains("45")),
            "45 個案例都要跑到：\n{text}"
        );
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
