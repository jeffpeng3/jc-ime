//! 日文統計轉換的獨立評測工具。
//!
//! 讀 pack v2（或 v1）建立 [`JaDecoder`]，接上 Mozc context ID 連接矩陣，
//! 跑 `0c-testdata/ja/*.json` 的凍結題庫，輸出與 `jc-eval` 相同的
//! [`Report`] 表格與逐題結果。
//!
//! ```text
//! cargo run -p jc-cli --release --bin ja-eval -- \
//!   --pack <path> --corpus modern-500 --top 5
//! ```
//!
//! `--corpus` 可以用題庫名稱（掃描 `--corpus-dir`），也可以直接給 `.json`
//! 路徑。`--verbose` 會逐題印出 id、期望與實際。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use jc_core::ja::decode::{JaContext, JaDecoder, JaResult};
use jc_core::ja::lexicon::JaLexicon;
use jc_eval::{
    evaluate, load_ja_corpus, Conversion, Converter, EngineInfo, EvalCase, JaCorpusEntry, Report,
    RunOptions,
};

/// 預設題庫目錄（相對於 `JC_TESTDATA` 或資料根目錄）。
const CORPUS_REL: &str = "0c-testdata/ja";

/// 命令列參數。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    /// pack 路徑。
    pack: PathBuf,
    /// 題庫名稱或 `.json` 路徑。
    corpus: String,
    /// 題庫目錄。
    corpus_dir: PathBuf,
    /// 每次轉換取幾個候選。
    top: usize,
    /// 是否逐題輸出。
    verbose: bool,
}

impl Args {
    fn usage() -> &'static str {
        "用法：ja-eval --pack <pack> --corpus <name|path> [--corpus-dir <dir>] [--top N] [--verbose]"
    }

    /// 解析參數；錯誤回傳給 `main` 印出。
    fn parse(argv: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut pack: Option<PathBuf> = None;
        let mut corpus: Option<String> = None;
        let mut corpus_dir: Option<PathBuf> = None;
        let mut top = 5usize;
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
                "--verbose" => verbose = true,
                "--help" | "-h" => return Err(Self::usage().into()),
                other => return Err(format!("未知參數 {other:?}\n{}", Self::usage())),
            }
        }

        let pack = pack.ok_or_else(|| format!("缺少 --pack\n{}", Self::usage()))?;
        let corpus = corpus.ok_or_else(|| format!("缺少 --corpus\n{}", Self::usage()))?;
        let corpus_dir = corpus_dir.unwrap_or_else(default_corpus_dir);
        Ok(Self {
            pack,
            corpus,
            corpus_dir,
            top,
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

/// 讀取題庫：名稱會在 `corpus_dir` 內比對，路徑則直接載入。
fn load_corpus(args: &Args) -> Result<JaCorpusEntry, String> {
    let candidate = Path::new(&args.corpus);
    if candidate.is_file() {
        return load_ja_corpus(candidate).map_err(|e| e.to_string());
    }

    let mut names = Vec::new();
    let entries = std::fs::read_dir(&args.corpus_dir).map_err(|e| {
        format!(
            "讀取題庫目錄 {} 失敗：{e}；可用 --corpus-dir 指定",
            args.corpus_dir.display()
        )
    })?;
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let loaded = load_ja_corpus(&path).map_err(|e| e.to_string())?;
        if loaded.corpus.name == args.corpus {
            return Ok(loaded);
        }
        names.push(loaded.corpus.name);
    }
    names.sort();
    Err(format!(
        "在 {} 找不到題庫 {:?}；有的是：{}",
        args.corpus_dir.display(),
        args.corpus,
        names.join("、")
    ))
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
        lexicon_sha256: lexicon_sha,
        connection_sha256: String::new(),
    };
    let entry = load_corpus(&args)?;
    let cases: Vec<EvalCase> = entry.corpus.items.iter().map(EvalCase::from).collect();
    let evaluation = evaluate(
        &StatsConverter { decoder },
        &cases,
        &RunOptions::new(args.top, 100),
    );

    let mut report = Report::new(engine, String::new());
    report.insert(entry.corpus.name.clone(), evaluation.metrics);
    print!("{}", report.to_table());

    if args.verbose {
        println!("\n{:<16} {:<28} {:<28} 命中", "id", "期望", "實際");
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
        assert_eq!(args.corpus, "modern-500");
        assert_eq!(args.top, 5);
        assert!(args.verbose);
        assert_eq!(args.corpus_dir, default_corpus_dir());
    }

    #[test]
    fn test_parse_inline_values_and_defaults() {
        let args = parse(&["--pack=/tmp/a.pack", "--corpus=hard-cases"]).unwrap();
        assert_eq!(args.top, 5);
        assert!(!args.verbose);
        assert_eq!(args.corpus, "hard-cases");
    }

    #[test]
    fn test_parse_rejects_bad_input() {
        assert!(parse(&["--corpus", "x"]).is_err());
        assert!(parse(&["--pack", "a"]).is_err());
        assert!(parse(&["--pack", "a", "--corpus", "x", "--top", "0"]).is_err());
        assert!(parse(&["--pack", "a", "--corpus", "x", "--top", "nope"]).is_err());
        assert!(parse(&["--pack", "a", "--corpus", "x", "--nope", "1"]).is_err());
        assert!(parse(&["--pack"]).is_err());
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
