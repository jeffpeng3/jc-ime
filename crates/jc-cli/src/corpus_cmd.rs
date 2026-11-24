//! `corpus-diff`：用真人會打的句子量測兩種分詞方式。
//!
//! 與 `mixed-diff` 的差別只在語料：這裡不產生隨機片段，而是從既有題庫、
//! golden 題庫、`ja/modern-500` 與測試／文件裡的真實案例收集輸入。中文句子
//! 一律轉成大千按鍵序列，並另外產生「去掉所有聲調鍵」的版本，因為一聲的
//! 空白鍵同時是分隔鍵，兩條路徑對它的解讀不同。
//!
//! 量測本身沿用 `mixed-diff` 的比對與分類：A 是離線 DP
//! （[`jc_core::mixed_latin::split`]），B 是正式路由（[`Router::convert`] 且
//! `mixed_latin_split: true`）。這個子命令只讀資料，不改任何轉換行為。
//!
//! 分類程式是從 `diff_cmd.rs` 複製的（見
//! `docs/reports/real-corpus-2026-11-21.md`）：那支檔案屬於 `segment-diff`
//! 分支，為了避免兩個 branch 同時改同一個檔案，這裡保留一份等價實作。

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use jc_core::mixed::{English, MixedResult, Router};
use jc_core::mixed_latin::{self, LexiconAdapter, Options as SplitOptions, Split, SplitLang};
use jc_core::zh::keyboard::Keyboard;
use jc_core::zh::pinyin::is_legal_base;
use jc_core::zh::syllable::{class_of, Class, Syllable};
use jc_core::{Mode, Script};

use crate::cli::CliError;
use crate::data;

/// `corpus-diff` 的參數。
#[derive(Debug, Clone)]
pub struct CorpusOptions {
    /// `0c-testdata` 目錄；未指定時用 `JC_TESTDATA`。
    pub source: Option<PathBuf>,
    /// 最多量測幾筆；0 表示全部。
    pub limit: usize,
    /// 每桶輸出的代表案例數。
    pub top: usize,
    /// 輸出完整 JSON。
    pub json: bool,
    /// 把不一致案例寫成與 `mixed-diff` 相容的 JSON。
    pub cases: Option<PathBuf>,
}

impl Default for CorpusOptions {
    fn default() -> Self {
        Self {
            source: None,
            limit: 0,
            top: 5,
            json: false,
            cases: None,
        }
    }
}

/// 語料來源類別。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Origin {
    ZhGolden,
    ZhExpected,
    MixedGolden,
    JaModern500,
    EnglishWords,
    EnglishTests,
    MixedTests,
}

impl Origin {
    const ALL: [Origin; 7] = [
        Origin::ZhGolden,
        Origin::ZhExpected,
        Origin::MixedGolden,
        Origin::JaModern500,
        Origin::EnglishWords,
        Origin::EnglishTests,
        Origin::MixedTests,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::ZhGolden => "zh-golden",
            Self::ZhExpected => "zh-expected",
            Self::MixedGolden => "mixed-golden",
            Self::JaModern500 => "ja-modern-500",
            Self::EnglishWords => "english-words",
            Self::EnglishTests => "english-tests",
            Self::MixedTests => "mixed-tests",
        }
    }
}

/// 輸入版本。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Variant {
    /// 保留聲調鍵；中文的一聲以空白鍵表示。
    Toned,
    /// 去掉所有聲調鍵。
    Toneless,
    /// 日文羅馬字或英文，沒有聲調可去掉。
    Plain,
}

impl Variant {
    fn label(self) -> &'static str {
        match self {
            Self::Toned => "toned",
            Self::Toneless => "toneless",
            Self::Plain => "plain",
        }
    }
}

/// 收集階段的一筆語料。
struct RawEntry {
    origin: Origin,
    source: String,
    full: String,
    toneless: Option<String>,
}

/// 去重後、實際量測的一筆語料。
struct CorpusEntry {
    origin: Origin,
    sources: Vec<String>,
    variant: Variant,
    input: String,
}

/// 一筆量測結果。
struct Measured {
    entry: CorpusEntry,
    case: Case,
}

/// 跑一次真人語料差異盤點。
pub fn run(opts: &CorpusOptions, out: &mut dyn Write) -> Result<(), CliError> {
    let dir = data::testdata_dir(opts.source.clone())?;
    let root = data::data_root(&dir);
    let resources = {
        let bin = root.join("build-zh-bigram");
        if bin.join("manifest.json").is_file() {
            data::load_zh_bin_resources(&bin)?
        } else {
            data::load_zh_resources(&dir.join("golden/zh/data"))?
        }
    };
    let ja = data::load_ja_decoder(&root)
        .ok()
        .map(|decoder| decoder.with_prefer_hiragana(true));
    let english = English::load_default();
    let config = jc_core::mixed::Config::default();
    let mut router = Router::new()
        .with_config(config.clone())
        .with_zh(&resources.lexicon, &resources.ctx)
        .with_english(&english);
    if let Some(decoder) = &ja {
        router = router.with_ja(decoder);
    }

    let adapter = LexiconAdapter {
        zh: Some(&resources.lexicon),
        english: Some(&english),
        config: config.clone(),
    };
    let split_opts = SplitOptions::default();

    let mut raw = Vec::new();
    let mut warnings = Vec::new();
    collect_zh_cases(&dir, &adapter, &split_opts, &mut raw, &mut warnings);
    collect_zh_expected(&dir, &adapter, &split_opts, &mut raw, &mut warnings);
    collect_mixed_golden(&dir, &mut raw, &mut warnings);
    collect_ja(&dir, &mut raw, &mut warnings);
    collect_english(&english, &mut raw, &mut warnings);
    collect_static_cases(&mut raw, &mut warnings);

    let mut entries = expand_and_dedup(raw);
    if opts.limit > 0 {
        entries = round_robin_limit(entries, opts.limit);
    }

    let mut measured = Vec::with_capacity(entries.len());
    for entry in entries {
        let a = mixed_latin::split(&entry.input, &split_opts, &adapter);
        let b = match router.convert(&entry.input) {
            Ok(result) => BOutcome::Ok(result),
            Err(err) => BOutcome::Err(err.to_string()),
        };
        let mut case = Case {
            input: entry.input.clone(),
            a,
            b,
            signals: Vec::new(),
        };
        if !case.agrees() {
            case.signals = classify(&case, &english);
            if case.signals.is_empty() {
                case.signals.push(Bucket::Other);
            }
        }
        measured.push(Measured { entry, case });
    }

    let summary = Summary::new(&measured, opts, &dir, warnings);
    if opts.json {
        let value = summary.json(&measured);
        writeln!(
            out,
            "{}",
            serde_json::to_string(&value)
                .map_err(|err| CliError::Eval(format!("輸出 JSON 失敗：{err}")))?
        )?;
    } else {
        summary.write_human(out)?;
        write_examples(&measured, &summary, out)?;
        summary.write_warnings(out)?;
    }
    if let Some(path) = &opts.cases {
        let value = summary.cases_json(&measured);
        let text = serde_json::to_string(&value)
            .map_err(|err| CliError::Eval(format!("輸出案例 JSON 失敗：{err}")))?;
        std::fs::write(path, text)
            .map_err(|err| CliError::Eval(format!("寫入 {} 失敗：{err}", path.display())))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 語料收集
// ---------------------------------------------------------------------------

/// 收集 `golden/zh/cases.json` 的中文句子。
fn collect_zh_cases(
    dir: &Path,
    adapter: &LexiconAdapter<'_>,
    opts: &SplitOptions,
    raw: &mut Vec<RawEntry>,
    warnings: &mut Vec<String>,
) {
    let path = dir.join("golden/zh/cases.json");
    let Some(value) = read_json(&path, warnings) else {
        return;
    };
    let Some(cases) = value.get("cases").and_then(serde_json::Value::as_array) else {
        warnings.push(format!("{} 缺少 cases 陣列", path.display()));
        return;
    };
    for case in cases {
        let name = case
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unnamed");
        let mut full = String::new();
        let mut toneless = String::new();
        let mut has_toned = false;
        if let Some(actions) = case.get("actions").and_then(serde_json::Value::as_array) {
            for action in actions {
                if let Some(syllables) = action.get("syllables").and_then(serde_json::Value::as_str)
                {
                    match (zh_keys(syllables), zh_toneless_keys(syllables)) {
                        (Some(f), Some(t)) => {
                            full.push_str(&f);
                            toneless.push_str(&t);
                            has_toned = true;
                        }
                        _ => warnings.push(format!(
                            "{}#{name} 有無法解析的注音：{syllables:?}",
                            path.display()
                        )),
                    }
                } else if let Some(bare) = action.get("bare").and_then(serde_json::Value::as_str) {
                    match bare_keys(bare) {
                        Some(keys) => {
                            full.push_str(&keys);
                            toneless.push_str(&keys);
                        }
                        None => warnings.push(format!(
                            "{}#{name} 有無法轉鍵的無調注音：{bare:?}",
                            path.display()
                        )),
                    }
                } else if let Some(keys) = action.get("keys").and_then(serde_json::Value::as_str) {
                    full.push_str(keys);
                    has_toned = true;
                    match toneless_for_keys(keys, adapter, opts) {
                        Some(t) => toneless.push_str(&t),
                        None => toneless.push_str(keys),
                    }
                }
            }
        }
        if full.is_empty() && toneless.is_empty() {
            continue;
        }
        raw.push(RawEntry {
            origin: Origin::ZhGolden,
            source: format!("golden/zh/cases.json#{name}"),
            full: if has_toned { full } else { String::new() },
            toneless: (has_toned || !toneless.is_empty()).then_some(toneless),
        });
    }
}

/// 收集 `golden/zh/expected/*.jsonl` 的按鍵序列。
///
/// 每個檔案的 `# step NN syllables ...` 會組成一句；`# step NN key <` 這類
/// 標點按鍵也照順序接回去。`Ctrl+,`、Enter、方向鍵不是字元輸入，忽略。
fn collect_zh_expected(
    dir: &Path,
    adapter: &LexiconAdapter<'_>,
    opts: &SplitOptions,
    raw: &mut Vec<RawEntry>,
    warnings: &mut Vec<String>,
) {
    let expected = dir.join("golden/zh/expected");
    let Ok(read) = std::fs::read_dir(&expected) else {
        warnings.push(format!("找不到 {}，略過 zh-expected", expected.display()));
        return;
    };
    let mut files: Vec<PathBuf> = read
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .collect();
    files.sort();
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            warnings.push(format!("讀不到 {}", path.display()));
            continue;
        };
        let mut full = String::new();
        let mut toneless = String::new();
        let mut first_line = 0usize;
        for (index, line) in text.lines().enumerate() {
            let Some(rest) = line.strip_prefix("# step ") else {
                continue;
            };
            let Some((_, body)) = rest.split_once(' ') else {
                continue;
            };
            if let Some(syllables) = body.strip_prefix("syllables ") {
                match (zh_keys(syllables), zh_toneless_keys(syllables)) {
                    (Some(f), Some(t)) => {
                        full.push_str(&f);
                        toneless.push_str(&t);
                        if first_line == 0 {
                            first_line = index + 1;
                        }
                    }
                    _ => warnings.push(format!(
                        "{}:{} 有無法解析的注音：{syllables:?}",
                        path.display(),
                        index + 1
                    )),
                }
            } else if let Some(keys) = body.strip_prefix("keys ") {
                full.push_str(keys);
                match toneless_for_keys(keys, adapter, opts) {
                    Some(t) => toneless.push_str(&t),
                    None => toneless.push_str(keys),
                }
                if first_line == 0 {
                    first_line = index + 1;
                }
            } else if let Some(key) = body.strip_prefix("key ") {
                if let Some(text) = text_key(key) {
                    full.push_str(text);
                    toneless.push_str(text);
                    if first_line == 0 {
                        first_line = index + 1;
                    }
                }
            }
        }
        if full.is_empty() && toneless.is_empty() {
            continue;
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown.jsonl".to_string());
        raw.push(RawEntry {
            origin: Origin::ZhExpected,
            source: format!("golden/zh/expected/{name}:{first_line}"),
            full,
            toneless: Some(toneless),
        });
    }
}

/// 收集 `golden/mixed/cases.json` 的中英日混打案例。
fn collect_mixed_golden(dir: &Path, raw: &mut Vec<RawEntry>, warnings: &mut Vec<String>) {
    let path = dir.join("golden/mixed/cases.json");
    let Some(value) = read_json(&path, warnings) else {
        return;
    };
    let Some(cases) = value.get("cases").and_then(serde_json::Value::as_array) else {
        warnings.push(format!("{} 缺少 cases 陣列", path.display()));
        return;
    };
    for case in cases {
        let name = case
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unnamed");
        let Some(input) = case.get("input").and_then(serde_json::Value::as_str) else {
            continue;
        };
        match (convert_mixed(input, false), convert_mixed(input, true)) {
            (Some(full), Some(toneless)) => raw.push(RawEntry {
                origin: Origin::MixedGolden,
                source: format!("golden/mixed/cases.json#{name}"),
                full,
                toneless: Some(toneless),
            }),
            _ => warnings.push(format!(
                "{}#{name} 有無法轉成大千按鍵的內容：{input:?}",
                path.display()
            )),
        }
    }
}

/// 收集 `ja/modern_500.json` 的日文句子，reading 轉成羅馬字。
fn collect_ja(dir: &Path, raw: &mut Vec<RawEntry>, warnings: &mut Vec<String>) {
    let path = dir.join("ja/modern_500.json");
    let Some(value) = read_json(&path, warnings) else {
        return;
    };
    let Some(items) = value.get("items").and_then(serde_json::Value::as_array) else {
        warnings.push(format!("{} 缺少 items 陣列", path.display()));
        return;
    };
    let table = reverse_romaji_table();
    for item in items {
        let id = item
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("modern-???");
        let Some(reading) = item.get("reading").and_then(serde_json::Value::as_str) else {
            continue;
        };
        match kana_to_romaji(reading, table) {
            Some(keys) => raw.push(RawEntry {
                origin: Origin::JaModern500,
                source: format!("ja/modern_500.json#{id}"),
                full: keys,
                toneless: None,
            }),
            None => warnings.push(format!(
                "{}#{id} 有無法轉羅馬字的 reading：{reading:?}",
                path.display()
            )),
        }
    }
}

/// 收集英文詞表與既有測試的英文片語。
fn collect_english(english: &English, raw: &mut Vec<RawEntry>, warnings: &mut Vec<String>) {
    let mut words: Vec<(f64, String)> = english
        .dictionary()
        .words()
        .iter()
        .filter(|word| {
            (1..=12).contains(&word.text.chars().count())
                && word
                    .text
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch == '\'')
        })
        .map(|word| (word.weight, word.text.clone()))
        .collect();
    words.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.1.cmp(&right.1))
    });
    if words.is_empty() {
        warnings.push("英文詞表是空的；english-words 沒有樣本".to_string());
    }
    for (_, word) in words.iter().take(200) {
        raw.push(RawEntry {
            origin: Origin::EnglishWords,
            source: format!("english/words.bin#{word}"),
            full: word.clone(),
            toneless: None,
        });
    }
    for index in 0..100.min(words.len().saturating_sub(1)) {
        let left = &words[index].1;
        let right = &words[index + 1].1;
        if left == right {
            continue;
        }
        raw.push(RawEntry {
            origin: Origin::EnglishWords,
            source: format!("english/words.bin#{left}+{right}"),
            full: format!("{left} {right}"),
            toneless: None,
        });
    }
    for index in 0..50.min(words.len().saturating_sub(2)) {
        let a = &words[index].1;
        let b = &words[index + 1].1;
        let c = &words[index + 2].1;
        if a == b || b == c || a == c {
            continue;
        }
        raw.push(RawEntry {
            origin: Origin::EnglishWords,
            source: format!("english/words.bin#{a}+{b}+{c}"),
            full: format!("{a} {b} {c}"),
            toneless: None,
        });
    }

    // 既有測試與文件裡的英文例句。
    const ENGLISH_TESTS: &[(&str, &str)] = &[
        ("is a good sentence", "crates/jc-core/src/mixed.rs:3738"),
        ("hello", "docs/reports/mixed-latin-2026-11-20.md:123"),
        ("small", "docs/reports/mixed-latin-2026-11-20.md:123"),
        ("code", "docs/reports/mixed-latin-2026-11-20.md:123"),
        ("kanji", "crates/jc-core/src/mixed.rs:3762"),
        ("hello.", "crates/jc-core/src/mixed.rs:3865"),
    ];
    for (text, source) in ENGLISH_TESTS {
        raw.push(RawEntry {
            origin: Origin::EnglishTests,
            source: (*source).to_string(),
            full: (*text).to_string(),
            toneless: None,
        });
    }
}

/// 收集真機回報與既有測試裡的混打案例。
///
/// 這些字串都直接抄自 `crates/jc-core/src/mixed.rs` 的測試；行號是
/// `f619b46` 時的位置。`docs/port/mixed-e2e.md` 的案例已由
/// `golden/mixed/cases.json` 涵蓋。
fn collect_static_cases(raw: &mut Vec<RawEntry>, warnings: &mut Vec<String>) {
    let table = reverse_romaji_table();
    const CASES: &[(&str, &str, bool)] = &[
        (
            "rishireej/ sasmall",
            "crates/jc-core/src/mixed.rs:3786",
            false,
        ),
        (
            "rishireej/ savul3",
            "crates/jc-core/src/mixed.rs:3814",
            false,
        ),
        ("5k42. g4n0 vul3", "crates/jc-core/src/mixed.rs:3846", false),
        ("kyouhaAPI", "crates/jc-core/src/mixed.rs:3546", false),
        (
            "kyouhafoo@example.com",
            "crates/jc-core/src/mixed.rs:3562",
            false,
        ),
        ("kyouhahello", "crates/jc-core/src/mixed.rs:3604", false),
        ("ㄉㄚˋㄐㄧㄅㄚ", "crates/jc-core/src/mixed.rs:3571", false),
        (
            "ㄇㄚˊㄆㄛˊㄉㄡˋㄈㄨˇ",
            "crates/jc-core/src/mixed.rs:3701",
            false,
        ),
        ("きょうは", "crates/jc-core/src/mixed.rs:3706", false),
        (
            "kubectl pCloud GitHub",
            "crates/jc-core/src/mixed.rs:3714",
            false,
        ),
        (
            "です is a good sentence",
            "crates/jc-core/src/mixed.rs:3738",
            false,
        ),
        ("hello.", "crates/jc-core/src/mixed.rs:3865", false),
    ];
    for (text, source, _allow_toneless) in CASES {
        let full = if text.chars().any(|ch| class_of(ch) != Class::None) {
            match convert_mixed(text, false) {
                Some(full) => full,
                None => {
                    warnings.push(format!("{source} 的注音無法轉鍵：{text:?}"));
                    continue;
                }
            }
        } else if text.chars().any(is_kana) {
            match kana_to_romaji(text, table) {
                Some(keys) => keys,
                None => {
                    warnings.push(format!("{source} 的かな無法轉羅馬字：{text:?}"));
                    continue;
                }
            }
        } else {
            (*text).to_string()
        };
        let toneless = convert_mixed(text, true);
        raw.push(RawEntry {
            origin: Origin::MixedTests,
            source: (*source).to_string(),
            full,
            toneless,
        });
    }
}

/// 把收集到的語料展開成兩種版本並去重。
fn expand_and_dedup(raw: Vec<RawEntry>) -> Vec<CorpusEntry> {
    let mut entries: Vec<CorpusEntry> = Vec::new();
    let mut index: BTreeMap<(Variant, String), usize> = BTreeMap::new();
    for item in raw {
        let sources = vec![item.source];
        let full = item.full;
        let toneless = item.toneless;
        let has_toned = toneless.as_ref().is_some_and(|text| text != &full);
        if !full.is_empty() {
            let variant = if has_toned {
                Variant::Toned
            } else {
                Variant::Plain
            };
            insert_entry(
                &mut entries,
                &mut index,
                item.origin,
                &sources,
                variant,
                full,
            );
            if let Some(text) = toneless {
                if has_toned {
                    insert_entry(
                        &mut entries,
                        &mut index,
                        item.origin,
                        &sources,
                        Variant::Toneless,
                        text,
                    );
                }
            }
        } else if let Some(text) = toneless {
            insert_entry(
                &mut entries,
                &mut index,
                item.origin,
                &sources,
                Variant::Toneless,
                text,
            );
        }
    }
    entries
}

/// 插入一筆語料；重複時只合併來源。
fn insert_entry(
    entries: &mut Vec<CorpusEntry>,
    index: &mut BTreeMap<(Variant, String), usize>,
    origin: Origin,
    sources: &[String],
    variant: Variant,
    input: String,
) {
    if input.is_empty() {
        return;
    }
    let key = (variant, input.clone());
    if let Some(&position) = index.get(&key) {
        for source in sources {
            if !entries[position].sources.contains(source) {
                entries[position].sources.push(source.clone());
            }
        }
        return;
    }
    let position = entries.len();
    entries.push(CorpusEntry {
        origin,
        sources: sources.to_vec(),
        variant,
        input,
    });
    index.insert(key, position);
}

/// 依來源輪流取樣，避免 `--limit` 只留下前面的來源。
fn round_robin_limit(entries: Vec<CorpusEntry>, limit: usize) -> Vec<CorpusEntry> {
    if limit == 0 || entries.len() <= limit {
        return entries;
    }
    let mut buckets: BTreeMap<Origin, Vec<CorpusEntry>> = BTreeMap::new();
    for entry in entries {
        buckets.entry(entry.origin).or_default().push(entry);
    }
    let queues: Vec<Vec<CorpusEntry>> = Origin::ALL
        .iter()
        .filter_map(|origin| buckets.remove(origin))
        .collect();
    let mut positions = vec![0usize; queues.len()];
    let mut out = Vec::with_capacity(limit);
    while out.len() < limit {
        let mut advanced = false;
        for (index, queue) in queues.iter().enumerate() {
            if out.len() >= limit {
                break;
            }
            if let Some(entry) = queue.get(positions[index]) {
                out.push(CorpusEntry {
                    origin: entry.origin,
                    sources: entry.sources.clone(),
                    variant: entry.variant,
                    input: entry.input.clone(),
                });
                positions[index] += 1;
                advanced = true;
            }
        }
        if !advanced {
            break;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 注音／羅馬字轉換
// ---------------------------------------------------------------------------

/// 大千鍵盤的按鍵反查：注音符號 → 按鍵。
fn key_table() -> &'static HashMap<char, char> {
    static TABLE: OnceLock<HashMap<char, char>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let keyboard = Keyboard::standard();
        let keys = "1234567890qwertyuiopasdfghjklzxcvbnm,./;- ";
        keys.chars()
            .filter_map(|key| keyboard.symbol(key).map(|symbol| (symbol, key)))
            .collect()
    })
}

/// 大千鍵盤的正向表：按鍵 → 注音符號。
fn key_to_symbol_table() -> &'static HashMap<char, char> {
    static TABLE: OnceLock<HashMap<char, char>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let keyboard = Keyboard::standard();
        let keys = "1234567890qwertyuiopasdfghjklzxcvbnm,./;- ";
        keys.chars()
            .filter_map(|key| keyboard.symbol(key).map(|symbol| (key, symbol)))
            .collect()
    })
}

/// 把以空白分隔的注音音節轉成大千按鍵；一聲會補一個空白鍵。
fn zh_keys(syllables: &str) -> Option<String> {
    let keyboard = Keyboard::standard();
    let mut out = String::new();
    for token in syllables.split_whitespace() {
        let syllable = Syllable::parse(token).ok()?;
        out.push_str(&keyboard.keys(&syllable).ok()?);
    }
    Some(out)
}

/// 把注音音節轉成不含任何聲調鍵的大千按鍵。
fn zh_toneless_keys(syllables: &str) -> Option<String> {
    let table = key_table();
    let mut out = String::new();
    for token in syllables.split_whitespace() {
        let syllable = Syllable::parse(token).ok()?;
        for ch in syllable.base().as_str().chars() {
            out.push(*table.get(&ch)?);
        }
    }
    Some(out)
}

/// 把已經沒有聲調的注音字串（可含 `'` 分隔）轉成按鍵。
fn bare_keys(bare: &str) -> Option<String> {
    let table = key_table();
    let mut out = String::new();
    for ch in bare.chars() {
        if ch == '\'' || ch == ' ' {
            out.push(ch);
        } else {
            out.push(*table.get(&ch)?);
        }
    }
    Some(out)
}

/// 把混合字串裡的注音與かな轉成按鍵，其他字元照原樣保留。
fn convert_mixed(input: &str, toneless: bool) -> Option<String> {
    let table = reverse_romaji_table();
    let mut out = String::new();
    let mut bopomofo = String::new();
    let mut kana = String::new();
    let flush = |out: &mut String, bopomofo: &mut String, kana: &mut String| -> Option<()> {
        if !bopomofo.is_empty() {
            out.push_str(&bopomofo_run_keys(bopomofo, toneless)?);
            bopomofo.clear();
        }
        if !kana.is_empty() {
            out.push_str(&kana_to_romaji(kana, table)?);
            kana.clear();
        }
        Some(())
    };
    for ch in input.chars() {
        if is_bopomofo(ch) {
            // 只在切換到另一種 run 時 flush：聲調記號也屬於注音 run，
            // 若在每個字元前就 flush，調號會被當成獨立音節，toneless 也丟不掉。
            if !kana.is_empty() {
                flush(&mut out, &mut bopomofo, &mut kana)?;
            }
            bopomofo.push(ch);
        } else if is_kana(ch) {
            if !bopomofo.is_empty() {
                flush(&mut out, &mut bopomofo, &mut kana)?;
            }
            kana.push(ch);
        } else {
            flush(&mut out, &mut bopomofo, &mut kana)?;
            out.push(ch);
        }
    }
    flush(&mut out, &mut bopomofo, &mut kana)?;
    Some(out)
}

/// 把一段連續注音切成音節並轉成按鍵。
fn bopomofo_run_keys(run: &str, toneless: bool) -> Option<String> {
    let table = key_table();
    let chars: Vec<char> = run.chars().collect();
    let mut out = String::new();
    let mut index = 0usize;
    while index < chars.len() {
        let mut matched = None;
        for take in (1..=3).rev() {
            if index + take <= chars.len() {
                let candidate: String = chars[index..index + take].iter().collect();
                if is_legal_base(&candidate) {
                    matched = Some((take, candidate));
                    break;
                }
            }
        }
        let (take, mut base) = matched.unwrap_or((1, chars[index].to_string()));
        index += take;
        let tone = if index < chars.len() && is_tone_mark(chars[index]) {
            let mark = chars[index];
            index += 1;
            Some(mark)
        } else {
            None
        };
        if toneless {
            for ch in base.chars() {
                out.push(*table.get(&ch)?);
            }
        } else {
            for ch in base.chars() {
                out.push(*table.get(&ch)?);
            }
            match tone {
                Some('ˉ') => out.push(' '),
                Some(mark) => out.push(*table.get(&mark)?),
                // 連續注音沒有聲調記號時代表沒打聲調鍵，不補空白。
                None => {}
            }
        }
        base.clear();
    }
    Some(out)
}

/// 把完整按鍵序列去掉聲調鍵；只處理 A 判成注音的片段。
///
/// A 解析失败时回傳 `None`，避免把英文裡的數字誤刪。
fn toneless_for_keys(
    input: &str,
    adapter: &LexiconAdapter<'_>,
    opts: &SplitOptions,
) -> Option<String> {
    let split = mixed_latin::split(input, opts, adapter)?;
    let mut out = String::new();
    let mut last = 0usize;
    for piece in &split.pieces {
        out.push_str(input.get(last..piece.start)?);
        if piece.lang == SplitLang::Zh {
            let text = input.get(piece.start..piece.end)?;
            out.push_str(&strip_zh_piece(text));
        } else {
            out.push_str(input.get(piece.start..piece.end)?);
        }
        last = piece.end;
    }
    out.push_str(input.get(last..)?);
    Some(out)
}

/// 去掉一段注音按鍵裡的聲調鍵，包括一聲的空白。
fn strip_zh_piece(piece: &str) -> String {
    let table = key_to_symbol_table();
    let mut out = String::new();
    let mut run = String::new();
    for ch in piece.chars() {
        let Some(symbol) = table.get(&ch).copied() else {
            run.clear();
            out.push(ch);
            continue;
        };
        let is_tone = ch == ' ' || matches!(ch, '6' | '3' | '4' | '7');
        if is_tone {
            if !run.is_empty() && is_legal_base(&run) {
                run.clear();
                continue;
            }
            run.clear();
            out.push(ch);
            continue;
        }
        run.push(symbol);
        out.push(ch);
    }
    out
}

/// 注音符號的範圍判定。
fn is_bopomofo(ch: char) -> bool {
    class_of(ch) != Class::None || matches!(ch as u32, 0x3100..=0x312F | 0x31A0..=0x31BF)
}

/// 聲調記號判定。
fn is_tone_mark(ch: char) -> bool {
    matches!(ch, 'ˉ' | 'ˊ' | 'ˇ' | 'ˋ' | '˙')
}

/// かな判定。
fn is_kana(ch: char) -> bool {
    matches!(ch as u32, 0x3041..=0x309F | 0x30A1..=0x30FF | 0x31F0..=0x31FF)
}

/// 反向羅馬字表：かな → 最短按鍵；同長度取字典序最小。
fn reverse_romaji_table() -> &'static HashMap<String, String> {
    static TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut map: HashMap<String, String> = HashMap::new();
        for (keys, kana) in jc_core::ja::input::default_romaji_table().entries() {
            if keys.is_empty() || !keys.chars().all(|ch| ch.is_ascii_lowercase() || ch == '-') {
                continue;
            }
            match map.get(kana) {
                Some(existing)
                    if (existing.chars().count(), existing.as_str())
                        <= (keys.chars().count(), keys) => {}
                _ => {
                    map.insert(kana.to_string(), keys.to_string());
                }
            }
        }
        // `ん` 由 composer 的 pending `n` 產生，對照表沒有獨立項目；
        // `ー` 的按鍵 `-` 也不是小寫字母，補進來讓反查完整。
        map.insert("ん".to_string(), "n".to_string());
        map.insert("ー".to_string(), "-".to_string());
        map
    })
}

/// 把かな句子轉成羅馬字；未知符號原樣保留。
fn kana_to_romaji(text: &str, table: &HashMap<String, String>) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut index = 0usize;
    while index < chars.len() {
        let mut matched = None;
        for take in (1..=2).rev() {
            if index + take <= chars.len() {
                let candidate: String = chars[index..index + take].iter().collect();
                if let Some(keys) = table.get(&candidate) {
                    matched = Some((take, keys.clone()));
                    break;
                }
            }
        }
        if let Some((take, keys)) = matched {
            out.push_str(&keys);
            index += take;
        } else {
            out.push(chars[index]);
            index += 1;
        }
    }
    Some(out)
}

/// 可當成字元輸入的標點按鍵。
fn text_key(key: &str) -> Option<&'static str> {
    const KEYS: &[(&str, &str)] = &[
        ("<", "<"),
        (">", ">"),
        ("?", "?"),
        (":", ":"),
        ("\"", "\""),
        ("!", "!"),
        ("~", "~"),
        ("[", "["),
        ("]", "]"),
        ("{", "{"),
        ("}", "}"),
        ("'", "'"),
        ("\\", "\\"),
        ("(", "("),
        (")", ")"),
        ("&", "&"),
        ("^", "^"),
        ("_", "_"),
    ];
    KEYS.iter()
        .find_map(|(name, text)| (*name == key).then_some(*text))
}

/// 讀取 JSON；失敗時記警告並回傳 `None`。
fn read_json(path: &Path, warnings: &mut Vec<String>) -> Option<serde_json::Value> {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(value) => Some(value),
            Err(err) => {
                warnings.push(format!("{} JSON 解析失敗：{err}", path.display()));
                None
            }
        },
        Err(err) => {
            warnings.push(format!("讀不到 {}：{err}", path.display()));
            None
        }
    }
}
// ---------------------------------------------------------------------------
// 比對
// ---------------------------------------------------------------------------

/// 正規化後的語言標籤。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    Ja,
    Zh,
    En,
    Sym,
}

/// 兩邊共用的片段表示。
#[derive(Debug, Clone)]
struct Span {
    lang: Lang,
    start: usize,
    end: usize,
    /// A 的讀音。
    reading: Option<String>,
    /// B 的輸出字面。
    surface: Option<String>,
}

impl Span {
    fn text<'a>(&self, input: &'a str) -> &'a str {
        input.get(self.start..self.end).unwrap_or_default()
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.start < other.end && other.start < self.end
    }
}

/// B 的結果。
#[derive(Debug, Clone)]
enum BOutcome {
    Ok(MixedResult),
    Err(String),
}

impl BOutcome {
    fn as_ref_result(&self) -> Option<&MixedResult> {
        match self {
            Self::Ok(result) => Some(result),
            Self::Err(_) => None,
        }
    }
}

/// 一個輸入的完整量測結果。
struct Case {
    input: String,
    a: Option<Split>,
    b: BOutcome,
    /// 命中的分類訊號；第一個是主分類，空的時候代表一致。
    signals: Vec<Bucket>,
}

impl Case {
    fn bucket(&self) -> Option<Bucket> {
        self.signals.first().copied()
    }

    fn a_spans(&self) -> Vec<Span> {
        self.a
            .as_ref()
            .map(|split| {
                split
                    .pieces
                    .iter()
                    .map(|piece| Span {
                        lang: match piece.lang {
                            SplitLang::Ja => Lang::Ja,
                            SplitLang::Zh => Lang::Zh,
                            SplitLang::En => Lang::En,
                        },
                        start: piece.start,
                        end: piece.end,
                        reading: Some(piece.reading.clone()),
                        surface: None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn b_spans(&self) -> Vec<Span> {
        self.b
            .as_ref_result()
            .map(|result| {
                result
                    .segments
                    .iter()
                    .filter_map(|segment| {
                        let text = self
                            .input
                            .get(segment.input_start..segment.input_end)
                            .unwrap_or_default();
                        if segment.script == Script::Symbol && text.chars().all(char::is_whitespace)
                        {
                            return None;
                        }
                        let lang = if segment.script == Script::Symbol {
                            Lang::Sym
                        } else {
                            match segment.mode {
                                Mode::Ja => Lang::Ja,
                                Mode::Zh => Lang::Zh,
                                Mode::En | Mode::Mixed => Lang::En,
                            }
                        };
                        Some(Span {
                            lang,
                            start: segment.input_start,
                            end: segment.input_end,
                            reading: None,
                            surface: Some(segment.surface.clone()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 兩邊是否一致。
    fn agrees(&self) -> bool {
        let a_spans = self.a_spans();
        let b_spans = self.b_spans();
        match (&self.a, &self.b) {
            (None, BOutcome::Ok(_)) => b_spans.is_empty(),
            (None, BOutcome::Err(_)) => true,
            (Some(_), BOutcome::Err(_)) => false,
            (Some(_), BOutcome::Ok(_)) => {
                if a_spans.len() != b_spans.len() {
                    return false;
                }
                a_spans.iter().zip(&b_spans).all(|(a, b)| {
                    a.lang == b.lang && a.start == b.start && a.end == b.end && literals_agree(a, b)
                })
            }
        }
    }
}

/// 語言與邊界相同時，再比對兩邊實際輸出的字面。
///
/// A 只產生讀音（かな、注音符號、英文照字面），B 會把讀音送進解碼器，因此
/// 只有英文片段的字面可以直接比；日文與注音的差異是轉換層的職責。
fn literals_agree(a: &Span, b: &Span) -> bool {
    if a.lang != Lang::En {
        return true;
    }
    match (&a.reading, &b.surface) {
        (Some(reading), Some(surface)) => reading == surface,
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// 分類
// ---------------------------------------------------------------------------

/// 不一致的主分類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Bucket {
    /// 大寫或全角讓 B 走不同分支。
    AtomUppercaseFullwidth,
    /// A 把 run 尾端的 `.`、`,`、`;` 當韻母，B 當標點。
    TrailingPunctStripped,
    /// A 把 run 開頭的 `.`、`,`、`;`、`/`、`-` 當韻母，B 當標點。
    LeadingPunct,
    /// 空白在一邊是一聲、在另一邊是分隔。
    SpaceToneVsSeparator,
    /// A 用英文詞表判成英文，B 沒有。
    EnWordMissing,
    /// B 用英文詞表判成英文，A 沒有。
    EnWordExtra,
    /// 1–2 個字母的英文。
    ShortEn,
    /// 羅馬字與注音互搶同一段。
    JaVsZh,
    /// 羅馬字與英文互搶同一段。
    JaVsEn,
    /// B 在 run 中間因標點或符號斷開。
    MidRunSymbol,
    /// B 把空白兩側的同語言片段併成一個片段，A 仍分成兩段。
    SpaceMergedSegments,
    /// A 無法完整解析。
    ANone,
    /// A 有結果，B 沒有任何片段。
    BEmpty,
    /// B 轉換失敗。
    BError,
    /// 以上都不是。
    Other,
}

impl Bucket {
    const ALL: [Bucket; 15] = [
        Bucket::AtomUppercaseFullwidth,
        Bucket::TrailingPunctStripped,
        Bucket::LeadingPunct,
        Bucket::SpaceToneVsSeparator,
        Bucket::EnWordMissing,
        Bucket::EnWordExtra,
        Bucket::ShortEn,
        Bucket::JaVsZh,
        Bucket::JaVsEn,
        Bucket::MidRunSymbol,
        Bucket::SpaceMergedSegments,
        Bucket::ANone,
        Bucket::BEmpty,
        Bucket::BError,
        Bucket::Other,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::AtomUppercaseFullwidth => "atom-uppercase-fullwidth",
            Self::TrailingPunctStripped => "trailing-punct-stripped",
            Self::LeadingPunct => "leading-punct",
            Self::SpaceToneVsSeparator => "space-tone-vs-separator",
            Self::EnWordMissing => "en-word-missing",
            Self::EnWordExtra => "en-word-extra",
            Self::ShortEn => "short-en",
            Self::JaVsZh => "ja-vs-zh",
            Self::JaVsEn => "ja-vs-en",
            Self::MidRunSymbol => "mid-run-symbol",
            Self::SpaceMergedSegments => "space-merged-segments",
            Self::ANone => "a-none",
            Self::BEmpty => "b-empty",
            Self::BError => "b-error",
            Self::Other => "other",
        }
    }
}

/// 找出一個不一致案例命中的所有分類，依優先序排列。
fn classify(case: &Case, english: &English) -> Vec<Bucket> {
    let input = case.input.as_str();
    let a_spans = case.a_spans();
    let b_spans = case.b_spans();
    let mut signals = Vec::new();

    if matches!(case.b, BOutcome::Err(_)) {
        signals.push(Bucket::BError);
    }
    if has_upper_or_fullwidth(input) {
        signals.push(Bucket::AtomUppercaseFullwidth);
    }
    if trailing_punct_eaten(input, &a_spans, &b_spans) {
        signals.push(Bucket::TrailingPunctStripped);
    }
    if leading_punct_eaten(input, &a_spans, &b_spans) {
        signals.push(Bucket::LeadingPunct);
    }
    if space_as_tone(input, &a_spans) {
        signals.push(Bucket::SpaceToneVsSeparator);
    }
    if lang_conflict(&a_spans, &b_spans, Lang::Ja, Lang::Zh) {
        signals.push(Bucket::JaVsZh);
    }
    if en_word_missing(&a_spans, &b_spans) {
        signals.push(Bucket::EnWordMissing);
    }
    if en_word_extra(input, english, &a_spans, &b_spans) {
        signals.push(Bucket::EnWordExtra);
    }
    if short_en_mismatch(input, english, &a_spans, &b_spans) {
        signals.push(Bucket::ShortEn);
    }
    if lang_conflict(&a_spans, &b_spans, Lang::Ja, Lang::En) {
        signals.push(Bucket::JaVsEn);
    }
    if has_mid_symbol(input, &b_spans) {
        signals.push(Bucket::MidRunSymbol);
    }
    if case.a.is_none() {
        signals.push(Bucket::ANone);
    }
    if merges_across_space(input, &b_spans) {
        signals.push(Bucket::SpaceMergedSegments);
    }
    if case.a.is_some() && b_spans.is_empty() {
        signals.push(Bucket::BEmpty);
    }
    signals
}

/// 輸入是否含 ASCII 大寫或全角英數字。
fn has_upper_or_fullwidth(input: &str) -> bool {
    input
        .chars()
        .any(|ch| ch.is_ascii_uppercase() || jc_core::mixed::is_fullwidth_alnum(ch))
}

/// 拉丁 run 的尾端標點是否被 A 當成韻母。
fn trailing_punct_eaten(input: &str, a_spans: &[Span], b_spans: &[Span]) -> bool {
    for symbol in b_spans {
        if symbol.lang != Lang::Sym {
            continue;
        }
        let text = symbol.text(input);
        if text.is_empty() || !text.chars().all(|ch| ".,;".contains(ch)) {
            continue;
        }
        let covered = a_spans.iter().any(|span| {
            span.lang == Lang::Zh && span.start <= symbol.start && span.end >= symbol.end
        });
        if !covered {
            continue;
        }
        // 這個標點是不是該 run 的最後一個字元。
        let next_is_token = input
            .get(symbol.end..)
            .and_then(|rest| rest.chars().next())
            .is_some_and(is_token_rune);
        if !next_is_token {
            return true;
        }
    }
    false
}

/// A 是否把空白當成注音的一聲。
///
/// 空白被吃掉後，兩側的注音片段在輸入上連續，`mixed_latin::split` 會把它們
/// 併成一段，所以判斷條件是「注音片段裡面含空白」，不只結尾。
fn space_as_tone(input: &str, a_spans: &[Span]) -> bool {
    a_spans.iter().any(|span| {
        span.lang == Lang::Zh
            && span.end > span.start
            && input
                .get(span.start..span.end)
                .is_some_and(|text| text.chars().any(char::is_whitespace))
    })
}

/// A 把 run 開頭的標點當成韻母。
fn leading_punct_eaten(input: &str, a_spans: &[Span], b_spans: &[Span]) -> bool {
    for symbol in b_spans {
        if symbol.lang != Lang::Sym {
            continue;
        }
        let text = symbol.text(input);
        if text.is_empty() {
            continue;
        }
        let trimmed = text.trim();
        if trimmed.is_empty() || !trimmed.chars().all(|ch| ".,;/-".contains(ch)) {
            continue;
        }
        let start = symbol.start + (text.len() - text.trim_start().len());
        let at_run_head = input
            .get(..start)
            .and_then(|left| left.chars().next_back())
            .is_none_or(|ch| !is_run_char(ch));
        if !at_run_head {
            continue;
        }
        let attached = a_spans.iter().any(|span| {
            span.lang == Lang::Zh && span.start <= symbol.start && span.end > symbol.end
        });
        if attached {
            return true;
        }
    }
    false
}

/// A 判成英文，但 B 沒有覆蓋同一段的英文片段。
fn en_word_missing(a_spans: &[Span], b_spans: &[Span]) -> bool {
    a_spans
        .iter()
        .filter(|span| span.lang == Lang::En)
        .any(|span| {
            !b_spans
                .iter()
                .any(|b| b.lang == Lang::En && b.start <= span.start && b.end >= span.end)
        })
}

/// B 判成英文，但 A 沒有覆蓋同一段的英文片段。
fn en_word_extra(input: &str, english: &English, a_spans: &[Span], b_spans: &[Span]) -> bool {
    if a_spans.is_empty() {
        return false;
    }
    b_spans
        .iter()
        .filter(|span| span.lang == Lang::En)
        .any(|span| {
            let text = span.text(input);
            text.chars().count() >= 3
                && text.chars().all(|ch| ch.is_ascii_lowercase())
                && english.has(text)
                && !a_spans
                    .iter()
                    .any(|a| a.lang == Lang::En && a.start <= span.start && a.end >= span.end)
        })
}

/// B 把 1–2 個字母的英文詞當成英文，A 沒有。
///
/// 只看英文詞表認得的詞，避免把 `ka` 這類羅馬字誤判成短英文。
fn short_en_mismatch(input: &str, english: &English, a_spans: &[Span], b_spans: &[Span]) -> bool {
    if a_spans.is_empty() {
        return false;
    }
    for (start, end) in ascii_word_spans(input) {
        let word = &input[start..end];
        if word.chars().count() > 2 || !english.has(word) {
            continue;
        }
        let b_en = b_spans
            .iter()
            .any(|span| span.lang == Lang::En && span.start <= start && span.end >= end);
        let a_en = a_spans
            .iter()
            .any(|span| span.lang == Lang::En && span.start <= start && span.end >= end);
        if b_en && !a_en {
            return true;
        }
    }
    false
}

/// 掃出連續 ASCII 字母的範圍。
fn ascii_word_spans(input: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    let mut end = 0usize;
    for (index, ch) in input.char_indices() {
        if ch.is_ascii_alphabetic() {
            start.get_or_insert(index);
            end = index + ch.len_utf8();
        } else if let Some(begin) = start.take() {
            out.push((begin, end));
        }
    }
    if let Some(begin) = start {
        out.push((begin, end));
    }
    out
}

/// B 是否有片段跨過空白（把空白兩側的同語言內容併成一段）。
fn merges_across_space(input: &str, b_spans: &[Span]) -> bool {
    b_spans
        .iter()
        .any(|span| span.text(input).chars().any(char::is_whitespace))
}

/// 兩邊是否在同一段上分別判成 `left` 與 `right`。
fn lang_conflict(a_spans: &[Span], b_spans: &[Span], left: Lang, right: Lang) -> bool {
    a_spans.iter().any(|a| {
        b_spans.iter().any(|b| {
            a.overlaps(b)
                && ((a.lang == left && b.lang == right) || (a.lang == right && b.lang == left))
        })
    })
}

/// B 是否在英數 run 中間插入非空白的符號片段。
fn has_mid_symbol(input: &str, b_spans: &[Span]) -> bool {
    b_spans.iter().any(|span| {
        if span.lang != Lang::Sym {
            return false;
        }
        let text = span.text(input);
        if text.is_empty() || text.chars().all(char::is_whitespace) {
            return false;
        }
        // 符號片段可能把後面的空白一起吃掉（例如 `": "`）；判斷 run 邊界時
        // 只看非空白的部分。
        let start = span.start + (text.len() - text.trim_start().len());
        let end = span.end - (text.len() - text.trim_end().len());
        let previous = input
            .get(..start)
            .and_then(|left| left.chars().next_back())
            .map(is_run_char);
        let next = input
            .get(end..)
            .and_then(|right| right.chars().next())
            .map(is_run_char);
        previous == Some(true) && next == Some(true)
    })
}

/// 英數 run 的字元判定，與 `mixed::is_token_rune` 相同。
fn is_token_rune(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || jc_core::mixed::is_fullwidth_alnum(ch)
        || "._~:/?#[]@!$&'()*+,;=%\\-|{}".contains(ch)
}

/// run 內的字母或數字，用來判斷符號是否夾在 run 中間。
fn is_run_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || jc_core::mixed::is_fullwidth_alnum(ch)
}

fn case_json(case: &Case) -> serde_json::Value {
    let a = case.a.as_ref().map(|split| split_json(&case.input, split));
    let b = match &case.b {
        BOutcome::Ok(result) => b_json(&case.input, result),
        BOutcome::Err(err) => serde_json::json!({ "error": err }),
    };
    serde_json::json!({
        "input": case.input,
        "bucket": case.bucket().map(Bucket::name),
        "signals": case.signals.iter().map(|bucket| bucket.name()).collect::<Vec<_>>(),
        "a": a,
        "b": b,
    })
}

fn split_json(input: &str, split: &Split) -> serde_json::Value {
    let pieces: Vec<serde_json::Value> = split
        .pieces
        .iter()
        .map(|piece| {
            serde_json::json!({
                "start": piece.start,
                "end": piece.end,
                "lang": piece.lang.as_str(),
                "reading": piece.reading,
                "text": piece.text(input),
            })
        })
        .collect();
    serde_json::json!({ "score": split.score, "pieces": pieces })
}

fn b_json(input: &str, result: &MixedResult) -> serde_json::Value {
    let segments: Vec<serde_json::Value> = result
        .segments
        .iter()
        .map(|segment| {
            serde_json::json!({
                "mode": mode_name(segment.mode),
                "script": segment.script.as_str(),
                "input_start": segment.input_start,
                "input_end": segment.input_end,
                "input": input.get(segment.input_start..segment.input_end).unwrap_or_default(),
                "surface": segment.surface,
            })
        })
        .collect();
    serde_json::json!({
        "surface": result.surface,
        "cost": result.cost,
        "segments": segments,
    })
}

fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Zh => "zh",
        Mode::Ja => "ja",
        Mode::En => "en",
        Mode::Mixed => "mixed",
    }
}

/// 人類可讀的單行摘要。
fn describe(case: &Case) -> String {
    let a = match &case.a {
        Some(split) => split
            .pieces
            .iter()
            .map(|piece| {
                format!(
                    "{} {}..{} {:?} → {:?}",
                    piece.lang.as_str(),
                    piece.start,
                    piece.end,
                    piece.text(&case.input),
                    piece.reading
                )
            })
            .collect::<Vec<_>>()
            .join("、"),
        None => "無法解析".to_string(),
    };
    let b = match &case.b {
        BOutcome::Ok(result) => result
            .segments
            .iter()
            .map(|segment| {
                format!(
                    "{}/{} {}..{} {:?} → {:?}",
                    mode_name(segment.mode),
                    segment.script.as_str(),
                    segment.input_start,
                    segment.input_end,
                    case.input
                        .get(segment.input_start..segment.input_end)
                        .unwrap_or_default(),
                    segment.surface
                )
            })
            .collect::<Vec<_>>()
            .join("、"),
        BOutcome::Err(err) => format!("錯誤 {err}"),
    };
    format!("A：{a}\n      B：{b}")
}

// ---------------------------------------------------------------------------
// 統計與輸出
// ---------------------------------------------------------------------------

/// 一個來源的統計。
struct OriginStat {
    origin: Origin,
    count: usize,
    a_none: usize,
    agree: usize,
    disagree: usize,
}

/// 一個分類桶的統計。
struct BucketStat {
    bucket: Bucket,
    count: usize,
    a_some: usize,
    examples: Vec<usize>,
}

/// 整次量測的摘要。
struct Summary {
    source_dir: PathBuf,
    count: usize,
    unique: usize,
    agree: usize,
    disagree: usize,
    a_none: usize,
    a_some: usize,
    both_agree: usize,
    b_error: usize,
    origins: Vec<OriginStat>,
    buckets: Vec<BucketStat>,
    signal_counts: BTreeMap<&'static str, usize>,
    warnings: Vec<String>,
}

impl Summary {
    fn new(measured: &[Measured], opts: &CorpusOptions, dir: &Path, warnings: Vec<String>) -> Self {
        let disagree = measured
            .iter()
            .filter(|item| item.case.bucket().is_some())
            .count();
        let agree = measured.len() - disagree;
        let a_none = measured.iter().filter(|item| item.case.a.is_none()).count();
        let a_some = measured.len() - a_none;
        let both_agree = measured
            .iter()
            .filter(|item| item.case.a.is_some() && item.case.bucket().is_none())
            .count();
        let b_error = measured
            .iter()
            .filter(|item| matches!(item.case.b, BOutcome::Err(_)))
            .count();
        let unique = measured
            .iter()
            .map(|item| item.entry.input.as_str())
            .collect::<BTreeSet<_>>()
            .len();

        let mut origins: Vec<OriginStat> = Origin::ALL
            .iter()
            .map(|&origin| {
                let members: Vec<&Measured> = measured
                    .iter()
                    .filter(|item| item.entry.origin == origin)
                    .collect();
                OriginStat {
                    origin,
                    count: members.len(),
                    a_none: members.iter().filter(|item| item.case.a.is_none()).count(),
                    agree: members
                        .iter()
                        .filter(|item| item.case.bucket().is_none())
                        .count(),
                    disagree: members
                        .iter()
                        .filter(|item| item.case.bucket().is_some())
                        .count(),
                }
            })
            .filter(|stat| stat.count > 0)
            .collect();
        origins.sort_by_key(|stat| std::cmp::Reverse(stat.count));

        let mut counts: HashMap<Bucket, usize> = HashMap::new();
        let mut bucket_a_some: HashMap<Bucket, usize> = HashMap::new();
        let mut by_bucket: HashMap<Bucket, Vec<usize>> = HashMap::new();
        let mut signal_counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        for (index, item) in measured.iter().enumerate() {
            for signal in &item.case.signals {
                *signal_counts.entry(signal.name()).or_default() += 1;
            }
            if let Some(bucket) = item.case.bucket() {
                *counts.entry(bucket).or_default() += 1;
                if item.case.a.is_some() {
                    *bucket_a_some.entry(bucket).or_default() += 1;
                }
                by_bucket.entry(bucket).or_default().push(index);
            }
        }
        let mut buckets = Vec::new();
        for bucket in Bucket::ALL {
            let mut members = by_bucket.remove(&bucket).unwrap_or_default();
            if members.is_empty() {
                continue;
            }
            members.sort_by(|&left, &right| {
                measured[left]
                    .entry
                    .input
                    .chars()
                    .count()
                    .cmp(&measured[right].entry.input.chars().count())
                    .then_with(|| measured[left].entry.input.cmp(&measured[right].entry.input))
            });
            members.truncate(opts.top);
            buckets.push(BucketStat {
                bucket,
                count: counts.get(&bucket).copied().unwrap_or(0),
                a_some: bucket_a_some.get(&bucket).copied().unwrap_or(0),
                examples: members,
            });
        }
        buckets.sort_by(|left, right| {
            right
                .count
                .cmp(&left.count)
                .then_with(|| left.bucket.name().cmp(right.bucket.name()))
        });

        Self {
            source_dir: dir.to_path_buf(),
            count: measured.len(),
            unique,
            agree,
            disagree,
            a_none,
            a_some,
            both_agree,
            b_error,
            origins,
            buckets,
            signal_counts,
            warnings,
        }
    }

    fn ratio(&self, count: usize, total: usize) -> f64 {
        if total == 0 {
            0.0
        } else {
            count as f64 / total as f64
        }
    }

    fn write_human(&self, out: &mut dyn Write) -> Result<(), CliError> {
        writeln!(
            out,
            "corpus-diff：來源 {}、量測 {} 筆（唯一輸入 {}）",
            self.source_dir.display(),
            self.count,
            self.unique
        )?;
        writeln!(
            out,
            "一致 {}、不一致 {}（{:.1}%）",
            self.agree,
            self.disagree,
            self.ratio(self.disagree, self.count) * 100.0
        )?;
        writeln!(
            out,
            "A 無法解析 {}（{:.1}%）、A 有結果 {}（其中一致 {}）、B 轉換失敗 {}",
            self.a_none,
            self.ratio(self.a_none, self.count) * 100.0,
            self.a_some,
            self.both_agree,
            self.b_error
        )?;
        writeln!(out, "來源：")?;
        writeln!(
            out,
            "  {:<16} {:>6} {:>10} {:>8} {:>8}",
            "名稱", "句數", "A 無法解析", "一致", "不一致"
        )?;
        for stat in &self.origins {
            writeln!(
                out,
                "  {:<16} {:>6} {:>10} {:>8} {:>8}",
                stat.origin.label(),
                stat.count,
                stat.a_none,
                stat.agree,
                stat.disagree
            )?;
        }
        writeln!(out, "分類（主分類，依真實頻率排序）：")?;
        for stat in &self.buckets {
            writeln!(
                out,
                "  {:<30} {:>6}  {:>6.2}%   A 有結果 {}",
                stat.bucket.name(),
                stat.count,
                self.ratio(stat.count, self.disagree) * 100.0,
                stat.a_some
            )?;
        }
        writeln!(out, "多重訊號（一個案例可能命中多桶）：")?;
        for (name, count) in &self.signal_counts {
            writeln!(out, "  {name:<30} {count:>6}")?;
        }
        Ok(())
    }

    fn json(&self, measured: &[Measured]) -> serde_json::Value {
        let origins: Vec<serde_json::Value> = self
            .origins
            .iter()
            .map(|stat| {
                serde_json::json!({
                    "name": stat.origin.label(),
                    "count": stat.count,
                    "a_none": stat.a_none,
                    "agree": stat.agree,
                    "disagree": stat.disagree,
                })
            })
            .collect();
        let buckets: Vec<serde_json::Value> = self
            .buckets
            .iter()
            .map(|stat| {
                serde_json::json!({
                    "name": stat.bucket.name(),
                    "count": stat.count,
                    "ratio": self.ratio(stat.count, self.disagree),
                    "a_some": stat.a_some,
                    "examples": stat
                        .examples
                        .iter()
                        .map(|&index| corpus_case_json(&measured[index]))
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        let cases: Vec<serde_json::Value> = measured
            .iter()
            .filter(|item| item.case.bucket().is_some())
            .map(corpus_case_json)
            .collect();
        serde_json::json!({
            "suite": "corpus-diff",
            "source": self.source_dir.display().to_string(),
            "count": self.count,
            "unique": self.unique,
            "agree": self.agree,
            "disagree": self.disagree,
            "a_none": self.a_none,
            "a_some": self.a_some,
            "both_agree": self.both_agree,
            "both_disagree": self.a_some - self.both_agree,
            "b_error": self.b_error,
            "sources": origins,
            "buckets": buckets,
            "signals": self.signal_counts,
            "warnings": self.warnings,
            "cases": cases,
        })
    }

    fn cases_json(&self, measured: &[Measured]) -> serde_json::Value {
        let cases: Vec<serde_json::Value> = measured
            .iter()
            .filter(|item| item.case.bucket().is_some())
            .map(corpus_case_json)
            .collect();
        serde_json::json!({
            "suite": "corpus-diff-cases",
            "source": self.source_dir.display().to_string(),
            "count": self.count,
            "unique": self.unique,
            "disagree": self.disagree,
            "sources": self
                .origins
                .iter()
                .map(|stat| serde_json::json!({
                    "name": stat.origin.label(),
                    "count": stat.count,
                }))
                .collect::<Vec<_>>(),
            "cases": cases,
        })
    }

    fn write_warnings(&self, out: &mut dyn Write) -> Result<(), CliError> {
        if self.warnings.is_empty() {
            return Ok(());
        }
        writeln!(out, "警告：")?;
        for warning in &self.warnings {
            writeln!(out, "  {warning}")?;
        }
        Ok(())
    }
}

/// 與 `mixed-diff` 相同的案例 JSON，另加語料來源。
fn corpus_case_json(item: &Measured) -> serde_json::Value {
    let mut value = case_json(&item.case);
    value["source"] = serde_json::json!(item.entry.sources.join("; "));
    value["origin"] = serde_json::json!(item.entry.origin.label());
    value["variant"] = serde_json::json!(item.entry.variant.label());
    value
}

/// 把代表案例寫成多行輸出。
fn write_examples(
    measured: &[Measured],
    summary: &Summary,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    for stat in &summary.buckets {
        if stat.examples.is_empty() {
            continue;
        }
        writeln!(out, "{}：", stat.bucket.name())?;
        for &index in &stat.examples {
            let item = &measured[index];
            writeln!(
                out,
                "  {:?}  [{} {}]  {}",
                item.entry.input,
                item.entry.origin.label(),
                item.entry.variant.label(),
                item.entry.sources.join(" ; ")
            )?;
            writeln!(out, "      {}", describe(&item.case))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zh_keys_keeps_first_tone_space() {
        assert_eq!(zh_keys("ㄨㄛˇ").as_deref(), Some("ji3"));
        assert_eq!(zh_keys("ㄐㄧㄣ").as_deref(), Some("rup "));
        assert_eq!(zh_keys("ㄇㄣ˙").as_deref(), Some("ap7"));
    }

    #[test]
    fn test_zh_toneless_keys_removes_tones() {
        assert_eq!(zh_toneless_keys("ㄨㄛˇ ㄒㄧㄤˇ").as_deref(), Some("jivu;"));
        assert_eq!(zh_toneless_keys("ㄐㄧㄣ").as_deref(), Some("rup"));
        assert_eq!(zh_toneless_keys("ㄇㄣ˙").as_deref(), Some("ap"));
    }

    #[test]
    fn test_bare_keys_keeps_separator() {
        assert_eq!(bare_keys("ㄨㄛㄒㄧㄤ").as_deref(), Some("jivu;"));
        assert_eq!(bare_keys("ㄒㄧ'ㄢ").as_deref(), Some("vu'0"));
    }

    #[test]
    fn test_strip_zh_piece_drops_all_tones() {
        assert_eq!(strip_zh_piece("ji3"), "ji");
        assert_eq!(strip_zh_piece("t "), "t");
        assert_eq!(strip_zh_piece("5k42. g4n0 vul3"), "5k2.gn0vul");
    }

    #[test]
    fn test_convert_mixed_replaces_bopomofo() {
        assert_eq!(
            convert_mixed("watashiwaㄉㄚˋㄐㄧㄅㄚdesu", false).as_deref(),
            Some("watashiwa284ru18desu")
        );
        assert_eq!(
            convert_mixed("watashiwaㄉㄚˋㄐㄧㄅㄚdesu", true).as_deref(),
            Some("watashiwa28ru18desu")
        );
    }

    #[test]
    fn test_kana_to_romaji_uses_shortest_table_entry() {
        let table = reverse_romaji_table();
        assert_eq!(
            kana_to_romaji("わたしは", table).as_deref(),
            Some("wataciha")
        );
        assert_eq!(kana_to_romaji("きょうは", table).as_deref(), Some("kyouha"));
    }

    #[test]
    fn test_bucket_names_are_stable() {
        assert_eq!(
            Bucket::TrailingPunctStripped.name(),
            "trailing-punct-stripped"
        );
        assert_eq!(
            Bucket::SpaceToneVsSeparator.name(),
            "space-tone-vs-separator"
        );
        assert_eq!(Bucket::ANone.name(), "a-none");
    }
}
