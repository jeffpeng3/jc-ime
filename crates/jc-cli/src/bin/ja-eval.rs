//! 日文統計轉換的獨立評測工具。
//!
//! 讀 pack v2（或 v1）建立 [`JaDecoder`]，接上 Mozc context ID 連接矩陣，
//! 跑 `0c-testdata/ja/*.json` 的凍結題庫，輸出與 `jc-eval` 相同的
//! [`Report`] 表格與逐題結果。
//!
//! ```text
//! cargo run -p jc-cli --release --bin ja-eval -- \
//!   --pack <path> --corpus modern-500 --top 5
//! cargo run -p jc-cli --release --bin ja-eval -- \
//!   --pack <path> --file <path>/ja-lexicon-ambiguous.json --split holdout
//! ```
//!
//! `--corpus` 可以用題庫名稱（掃描 `--corpus-dir`），也可以直接給 `.json`
//! 路徑。`--file` 直接指定題庫檔，會自動辨識 `testdata/ja` 與
//! `eval/ja-lexicon-ambiguous.json` 兩種格式。歧義題庫預設輸出
//! holdout／dev／all 三個切分；`--split` 可以只跑其中一個。`--verbose` 會
//! 逐題印出 id、期望與實際。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use jc_core::ja::decode::{JaContext, JaDecoder, JaResult};
use jc_core::ja::lexicon::JaLexicon;
use jc_eval::{
    evaluate, load_ja_corpus, load_ja_lexicon_ambiguous, Conversion, Converter, EngineInfo,
    EvalCase, JaCorpusEntry, JaLexiconAmbiguous, Report, RunOptions,
};

/// 預設題庫目錄（相對於 `JC_TESTDATA` 或資料根目錄）。
const CORPUS_REL: &str = "0c-testdata/ja";

/// 單語言評測題庫目錄；`--corpus` 在 `ja` 目錄找不到名稱時往這裡找。
const EVAL_CORPUS_REL: &str = "0c-testdata/eval";

/// 命令列參數。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    /// pack 路徑。
    pack: PathBuf,
    /// 題庫名稱或 `.json` 路徑。
    corpus: Option<String>,
    /// 直接指定的題庫檔。
    file: Option<PathBuf>,
    /// 題庫目錄。
    corpus_dir: PathBuf,
    /// 每次轉換取幾個候選。
    top: usize,
    /// 只跑歧義題庫的某個切分。
    split: Option<Split>,
    /// 是否逐題輸出。
    verbose: bool,
}

/// 歧義題庫的切分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Split {
    /// 全部題目。
    All,
    /// 報告用的 holdout。
    Holdout,
    /// 之後校準用的 dev。
    Dev,
}

impl Split {
    /// 報表中的名稱。
    fn name(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Holdout => "holdout",
            Self::Dev => "dev",
        }
    }

    /// 解析 `--split` 的值。
    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "all" => Ok(Self::All),
            "holdout" => Ok(Self::Holdout),
            "dev" => Ok(Self::Dev),
            other => Err(format!("--split {other:?} 無效；可用 all、holdout、dev")),
        }
    }
}

impl Args {
    fn usage() -> &'static str {
        "用法：ja-eval --pack <pack> (--corpus <name|path> | --file <path>) \
         [--corpus-dir <dir>] [--top N] [--split all|holdout|dev] [--verbose]"
    }

    /// 解析參數；錯誤回傳給 `main` 印出。
    fn parse(argv: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut pack: Option<PathBuf> = None;
        let mut corpus: Option<String> = None;
        let mut file: Option<PathBuf> = None;
        let mut corpus_dir: Option<PathBuf> = None;
        let mut top = 5usize;
        let mut split = None;
        let mut verbose = false;

        let mut iter = argv.into_iter();
        while let Some(arg) = iter.next() {
            let (flag, inline) = match arg.split_once('=') {
                Some((flag, value)) if flag.starts_with("--") => {
                    (flag.to_string(), Some(value.to_string()))
                }
                _ => (arg, None),
            };
            let mut value = |name: &str| -> Result<String, String> {
                if let Some(value) = inline.clone() {
                    return Ok(value);
                }
                iter.next()
                    .ok_or_else(|| format!("{name} 缺少參數值\n{}", Self::usage()))
            };
            match flag.as_str() {
                "--pack" => pack = Some(PathBuf::from(value("--pack")?)),
                "--corpus" => corpus = Some(value("--corpus")?),
                "--file" => file = Some(PathBuf::from(value("--file")?)),
                "--corpus-dir" => corpus_dir = Some(PathBuf::from(value("--corpus-dir")?)),
                "--top" => {
                    let raw = value("--top")?;
                    let parsed: usize = raw
                        .parse()
                        .map_err(|_| format!("--top {raw:?} 不是正整數"))?;
                    if parsed == 0 {
                        return Err("--top 必須大於 0".into());
                    }
                    top = parsed;
                }
                "--split" => split = Some(Split::parse(&value("--split")?)?),
                "--verbose" => verbose = true,
                "--help" | "-h" => return Err(Self::usage().into()),
                other => return Err(format!("未知參數 {other:?}\n{}", Self::usage())),
            }
        }

        let pack = pack.ok_or_else(|| format!("缺少 --pack\n{}", Self::usage()))?;
        if corpus.is_some() && file.is_some() {
            return Err("--corpus 與 --file 只能給一個".into());
        }
        if corpus.is_none() && file.is_none() {
            return Err(format!("缺少 --corpus 或 --file\n{}", Self::usage()));
        }
        let corpus_dir = corpus_dir.unwrap_or_else(default_corpus_dir);
        Ok(Self {
            pack,
            corpus,
            file,
            corpus_dir,
            top,
            split,
            verbose,
        })
    }
}

/// 題庫目錄的預設值：`JC_TESTDATA` 或 `/mnt/work/jc/jc-ime-data` 下的
/// `0c-testdata/ja`。
fn default_corpus_dir() -> PathBuf {
    match jc_testdata::root() {
        Some(root) => root.join(CORPUS_REL),
        None => PathBuf::from(jc_testdata::DEFAULT_ROOT).join(CORPUS_REL),
    }
}

/// 把 `JaDecoder` 接成 `jc-eval` 的轉換器。
struct StatsConverter {
    decoder: JaDecoder,
}

impl StatsConverter {
    fn results(&self, reading: &str, top_n: usize) -> Vec<JaResult> {
        let units: Vec<char> = reading.chars().collect();
        self.decoder
            .results(&units, &JaContext::default(), top_n.max(1))
    }
}

/// 文節在輸出字串上的 `char` offset；最後一段的結尾不算邊界。
fn clause_boundaries(result: &JaResult) -> Vec<usize> {
    let mut pos = 0usize;
    let mut out = Vec::with_capacity(result.clauses.len().saturating_sub(1));
    for (i, clause) in result.clauses.iter().enumerate() {
        pos += clause.surface.chars().count();
        if i + 1 < result.clauses.len() {
            out.push(pos);
        }
    }
    out
}

impl Converter for StatsConverter {
    fn convert(&self, reading: &str, top_n: usize) -> Vec<String> {
        self.results(reading, top_n)
            .into_iter()
            .map(|r| r.surface())
            .collect()
    }

    fn convert_segmented(&self, reading: &str, top_n: usize) -> Vec<Conversion> {
        self.results(reading, top_n)
            .iter()
            .map(|r| Conversion::new(r.surface(), clause_boundaries(r)))
            .collect()
    }
}

/// 讀取題庫：`--file` 直接載入，`--corpus` 先當路徑再當名稱查詢。
fn load_corpus(args: &Args) -> Result<LoadedCorpus, String> {
    if let Some(file) = &args.file {
        return load_file(file);
    }
    let name = args.corpus.as_deref().unwrap_or_default();
    let candidate = Path::new(name);
    if candidate.is_file() {
        return load_file(candidate);
    }

    let mut names = Vec::new();
    for dir in [args.corpus_dir.clone(), default_eval_dir()] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = std::fs::read(&path) else {
                continue;
            };
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&raw) else {
                continue;
            };
            let Some(found) = value.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            if found == name {
                return load_file(&path);
            }
            names.push(found.to_string());
        }
    }
    names.sort();
    names.dedup();
    Err(format!(
        "在 {} 與 {} 找不到題庫 {:?}；有的是：{}",
        args.corpus_dir.display(),
        default_eval_dir().display(),
        name,
        names.join("、")
    ))
}

/// 載入單一題庫檔，自動辨識格式。
fn load_file(path: &Path) -> Result<LoadedCorpus, String> {
    let raw = std::fs::read(path).map_err(|e| format!("讀取題庫 {} 失敗：{e}", path.display()))?;
    let probe: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|e| format!("解析題庫 {} 失敗：{e}", path.display()))?;
    if is_ambiguous_format(&probe) {
        let (corpus, sha256) = load_ja_lexicon_ambiguous(path).map_err(|e| e.to_string())?;
        Ok(LoadedCorpus::Ambiguous { corpus, sha256 })
    } else {
        let entry = load_ja_corpus(path).map_err(|e| e.to_string())?;
        Ok(LoadedCorpus::Ja(entry))
    }
}

/// 歧義題庫有 `reference_rule`，題目有 `candidates`；`testdata/ja` 沒有。
fn is_ambiguous_format(value: &serde_json::Value) -> bool {
    if value.get("reference_rule").is_some() {
        return true;
    }
    value
        .get("items")
        .and_then(|v| v.as_array())
        .and_then(|items| items.first())
        .map(|item| item.get("candidates").is_some())
        .unwrap_or(false)
}

/// 單語言題庫目錄的預設值。
fn default_eval_dir() -> PathBuf {
    match jc_testdata::root() {
        Some(root) => root.join(EVAL_CORPUS_REL),
        None => PathBuf::from(jc_testdata::DEFAULT_ROOT).join(EVAL_CORPUS_REL),
    }
}

/// 載入後的題庫，保留原始格式以便決定切分。
enum LoadedCorpus {
    /// `testdata/ja/*.json`：讀音對詞面。
    Ja(JaCorpusEntry),
    /// `eval/ja-lexicon-ambiguous.json`：一個讀音多個漢字候選。
    Ambiguous {
        corpus: JaLexiconAmbiguous,
        sha256: String,
    },
}

impl LoadedCorpus {
    /// 題庫名稱。
    fn name(&self) -> &str {
        match self {
            Self::Ja(entry) => &entry.corpus.name,
            Self::Ambiguous { corpus, .. } => &corpus.name,
        }
    }

    /// 題庫檔雜湊。
    fn sha256(&self) -> &str {
        match self {
            Self::Ja(entry) => &entry.sha256,
            Self::Ambiguous { sha256, .. } => sha256,
        }
    }

    /// 題庫建置時 pin 的 pack 雜湊；`testdata/ja` 沒有這個欄位。
    fn pinned_pack_sha256(&self) -> Option<&str> {
        match self {
            Self::Ja(_) => None,
            Self::Ambiguous { corpus, .. } => Some(&corpus.pack.sha256),
        }
    }

    /// 要跑的切分與題目。`testdata/ja` 只有一個不分切的列表。
    fn rows(&self, filter: Option<Split>) -> Vec<(String, Vec<EvalCase>)> {
        match self {
            Self::Ja(entry) => vec![(
                entry.corpus.name.clone(),
                entry.corpus.items.iter().map(EvalCase::from).collect(),
            )],
            Self::Ambiguous { corpus, .. } => {
                const ORDER: [Split; 3] = [Split::Holdout, Split::Dev, Split::All];
                ORDER
                    .into_iter()
                    .filter(|split| filter.is_none_or(|wanted| wanted == *split))
                    .map(|split| {
                        let items = corpus.items.iter().filter(|item| match split {
                            Split::All => true,
                            Split::Holdout => item.holdout,
                            Split::Dev => !item.holdout,
                        });
                        (
                            format!("{}/{}", corpus.name, split.name()),
                            items.map(|item| item.eval_case()).collect(),
                        )
                    })
                    .filter(|(_, cases): &(String, Vec<EvalCase>)| !cases.is_empty())
                    .collect()
            }
        }
    }
}

fn run(args: Args) -> Result<(), String> {
    let lex = JaLexicon::open(&args.pack).map_err(|e| e.to_string())?;
    let context = lex.connection_cost();
    if !context.has_matrix() {
        eprintln!(
            "警告：{} 沒有 context 連接矩陣（pack v1），將退回 POS 成本",
            args.pack.display()
        );
    }
    let lexicon_sha = lex.pack().file_sha256();
    let decoder = JaDecoder::new(std::sync::Arc::new(jc_core::ja::decode::JaDictionary::new(
        std::sync::Arc::new(lex),
    )))
    .with_context_cost(std::sync::Arc::new(context));

    let engine = EngineInfo {
        name: "ja-stats-context".into(),
        data: args.pack.display().to_string(),
        lexicon_sha256: lexicon_sha.clone(),
        connection_sha256: String::new(),
    };
    let loaded = load_corpus(&args)?;
    if let Some(pin) = loaded.pinned_pack_sha256() {
        if pin != lexicon_sha {
            eprintln!(
                "警告：題庫 pin 的 pack 是 {pin}，這次跑的是 {lexicon_sha}；候選成本可能不同源"
            );
        }
    }

    let converter = StatsConverter { decoder };
    let mut report = Report::new(engine, String::new());
    let mut runs = Vec::new();
    for (name, cases) in loaded.rows(args.split) {
        let evaluation = evaluate(&converter, &cases, &RunOptions::new(args.top, 100));
        report.insert(name.clone(), evaluation.metrics.clone());
        runs.push((name, evaluation));
    }

    println!(
        "題庫：{}  雜湊：{}  pack：{}",
        loaded.name(),
        loaded.sha256(),
        lexicon_sha
    );
    print!("{}", report.to_table());

    if args.verbose {
        for (name, evaluation) in &runs {
            println!("\n[{name}] {:<16} {:<28} {:<28} 命中", "id", "期望", "實際");
            for item in &evaluation.items {
                println!(
                    "{:<16} {:<28} {:<28} {}",
                    item.id,
                    item.reference,
                    item.output,
                    if item.top1 {
                        "top1"
                    } else if item.top5_hit {
                        "top5"
                    } else {
                        "-"
                    }
                );
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match Args::parse(std::env::args().skip(1)).and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("ja-eval：{message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc_eval::{AmbiguousCandidate, JaAmbiguousItem};

    fn parse(argv: &[&str]) -> Result<Args, String> {
        Args::parse(argv.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn test_parse_full_command() {
        let args = parse(&[
            "--pack",
            "/tmp/mozc.pack",
            "--corpus",
            "modern-500",
            "--top",
            "5",
            "--verbose",
        ])
        .unwrap();
        assert_eq!(args.pack, PathBuf::from("/tmp/mozc.pack"));
        assert_eq!(args.corpus.as_deref(), Some("modern-500"));
        assert!(args.file.is_none());
        assert_eq!(args.top, 5);
        assert_eq!(args.split, None);
        assert!(args.verbose);
        assert_eq!(args.corpus_dir, default_corpus_dir());
    }

    #[test]
    fn test_parse_inline_values_and_defaults() {
        let args = parse(&["--pack=/tmp/a.pack", "--corpus=hard-cases"]).unwrap();
        assert_eq!(args.top, 5);
        assert!(!args.verbose);
        assert_eq!(args.corpus.as_deref(), Some("hard-cases"));
    }

    #[test]
    fn test_parse_file_and_split() {
        let args = parse(&[
            "--pack",
            "/tmp/a.pack",
            "--file",
            "/tmp/ja-lexicon-ambiguous.json",
            "--split",
            "holdout",
        ])
        .unwrap();
        assert_eq!(
            args.file,
            Some(PathBuf::from("/tmp/ja-lexicon-ambiguous.json"))
        );
        assert_eq!(args.split, Some(Split::Holdout));
        assert_eq!(Split::parse("dev"), Ok(Split::Dev));
        assert_eq!(Split::parse("all"), Ok(Split::All));
        assert!(Split::parse("test").is_err());
    }

    #[test]
    fn test_parse_rejects_bad_input() {
        assert!(parse(&["--corpus", "x"]).is_err());
        assert!(parse(&["--pack", "a"]).is_err());
        assert!(parse(&["--pack", "a", "--corpus", "x", "--top", "0"]).is_err());
        assert!(parse(&["--pack", "a", "--corpus", "x", "--top", "nope"]).is_err());
        assert!(parse(&["--pack", "a", "--corpus", "x", "--nope", "1"]).is_err());
        assert!(parse(&["--pack", "a", "--corpus", "x", "--split", "half"]).is_err());
        assert!(parse(&["--pack", "a", "--corpus", "x", "--file", "y"]).is_err());
        assert!(parse(&["--pack"]).is_err());
    }

    fn ambiguous_item(id: &str, reading: &str, surface: &str, holdout: bool) -> JaAmbiguousItem {
        JaAmbiguousItem {
            id: id.into(),
            reading: reading.into(),
            reference: surface.into(),
            candidates: vec![
                AmbiguousCandidate {
                    surface: surface.into(),
                    cost: 1,
                },
                AmbiguousCandidate {
                    surface: "別".into(),
                    cost: 2,
                },
            ],
            holdout,
        }
    }

    #[test]
    fn test_ambiguous_rows_follow_split() {
        let corpus = JaLexiconAmbiguous {
            name: "ja-lexicon-ambiguous".into(),
            items: vec![
                ambiguous_item("ja-0001", "あい", "愛", true),
                ambiguous_item("ja-0002", "あお", "青", false),
            ],
            ..Default::default()
        };
        let loaded = LoadedCorpus::Ambiguous {
            corpus,
            sha256: "abc".into(),
        };
        let rows = loaded.rows(None);
        let names: Vec<&str> = rows.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "ja-lexicon-ambiguous/holdout",
                "ja-lexicon-ambiguous/dev",
                "ja-lexicon-ambiguous/all"
            ]
        );
        assert_eq!(rows[0].1.len(), 1);
        assert_eq!(rows[1].1.len(), 1);
        assert_eq!(rows[2].1.len(), 2);
        assert_eq!(rows[2].1[0].reference, "愛");

        let only_all = loaded.rows(Some(Split::All));
        assert_eq!(only_all.len(), 1);
        assert_eq!(only_all[0].0, "ja-lexicon-ambiguous/all");
    }

    #[test]
    fn test_ambiguous_format_detection() {
        let ja = serde_json::json!({
            "name": "modern-500",
            "items": [{"id": "a", "reading": "きょう", "surface": "今日"}]
        });
        assert!(!is_ambiguous_format(&ja));
        let ambiguous = serde_json::json!({
            "name": "ja-lexicon-ambiguous",
            "reference_rule": "lowest cost",
            "items": [{"id": "a", "reading": "あい", "reference": "愛",
                       "candidates": [{"surface": "愛", "cost": 1}]}]
        });
        assert!(is_ambiguous_format(&ambiguous));
        let by_candidates = serde_json::json!({
            "items": [{"reading": "あい", "candidates": []}]
        });
        assert!(is_ambiguous_format(&by_candidates));
        assert!(!is_ambiguous_format(&serde_json::json!({"items": []})));
    }

    #[test]
    fn test_clause_boundaries_use_char_offsets() {
        use jc_core::ja::decode::JaClause;
        let result = JaResult {
            clauses: vec![
                JaClause {
                    reading: "きょう".into(),
                    surface: "今日".into(),
                    segments: Vec::new(),
                    cost: 0,
                },
                JaClause {
                    reading: "は".into(),
                    surface: "は".into(),
                    segments: Vec::new(),
                    cost: 0,
                },
                JaClause {
                    reading: "いい".into(),
                    surface: "いい".into(),
                    segments: Vec::new(),
                    cost: 0,
                },
            ],
            cost: 0.0,
            score: 0.0,
        };
        assert_eq!(clause_boundaries(&result), vec![2, 3]);
    }
}
