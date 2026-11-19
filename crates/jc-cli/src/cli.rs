//! 命令列解析與分派。
//!
//! 解析刻意手寫，與 `jc-engine` 的風格一致；錯誤訊息只描述使用者看得到的
//! 參數，不洩漏內部路徑。

use std::io::Write;
use std::path::PathBuf;

/// 命令列錯誤。
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// 參數不合法。
    #[error("{0}")]
    Usage(String),
    /// 檔案或資料集讀取失敗。
    #[error("{0}")]
    Data(String),
    /// 轉換或評測失敗。
    #[error("{0}")]
    Eval(String),
    /// 輸出寫入失敗。
    #[error("寫出結果失敗：{0}")]
    Io(#[from] std::io::Error),
}

impl CliError {
    /// 建立資料錯誤。
    pub fn data(message: impl Into<String>) -> Self {
        Self::Data(message.into())
    }

    /// 建立評測錯誤。
    pub fn eval(message: impl Into<String>) -> Self {
        Self::Eval(message.into())
    }
}

/// 解析後要執行的動作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// 跑注音 golden 的 45 個案例。
    EvalZhGolden {
        /// `0c-testdata` 目錄；未指定時用 `JC_TESTDATA`。
        testdata: Option<PathBuf>,
        /// 輸出 JSON 而非表格。
        json: bool,
    },
    /// 跑一個日文題庫。
    EvalJa {
        /// `0c-testdata` 目錄。
        testdata: Option<PathBuf>,
        /// `rules.json` 裡的題庫名稱，或直接給題庫 `.json` 路徑。
        corpus: String,
        /// 日文 pack；未指定時由 `JC_JA_PACK` 與資料根目錄決定。
        pack: Option<PathBuf>,
        /// 輸出 JSON。
        json: bool,
    },
    /// 跑模糊輸入題庫。
    EvalFuzzy {
        /// `0c-testdata` 目錄。
        testdata: Option<PathBuf>,
        /// 輸出 JSON。
        json: bool,
    },
    /// 單句轉換並列出 N-best。
    Convert {
        /// 讀音（注音音節或かな）。
        reading: String,
        /// `0c-testdata` 目錄。
        testdata: Option<PathBuf>,
        /// 取幾個候選。
        n: usize,
        /// 輸出 JSON。
        json: bool,
    },
    /// 對檔案中的每行讀音做轉換並計時。
    Bench {
        /// 每行一個讀音。
        file: PathBuf,
        /// `0c-testdata` 目錄。
        testdata: Option<PathBuf>,
        /// 每行重複次數。
        repeat: usize,
        /// 輸出 JSON。
        json: bool,
    },
    /// 顯示說明。
    Help,
    /// 顯示版本。
    Version,
}

/// 解析參數。
pub fn parse(args: &[String]) -> Result<Command, CliError> {
    let Some(first) = args.first().map(String::as_str) else {
        return Ok(Command::Help);
    };
    match first {
        "-h" | "--help" | "help" => Ok(Command::Help),
        "-V" | "--version" | "version" => Ok(Command::Version),
        "eval" => parse_eval(&args[1..]),
        "convert" => parse_convert(&args[1..]),
        "bench" => parse_bench(&args[1..]),
        other => Err(CliError::Usage(format!("不認得的子命令 {other}"))),
    }
}

fn parse_eval(args: &[String]) -> Result<Command, CliError> {
    let Some(kind) = args.first().map(String::as_str) else {
        return Err(CliError::Usage(
            "eval 需要 zh-golden、ja 或 fuzzy".to_string(),
        ));
    };
    let rest = &args[1..];
    let mut testdata = None;
    let mut corpus = "modern-500".to_string();
    let mut pack = None;
    let mut json = false;
    let mut index = 0;
    while index < rest.len() {
        match rest[index].as_str() {
            "--testdata" => {
                index += 1;
                testdata = Some(PathBuf::from(value(rest, index, "--testdata")?));
            }
            "--corpus" => {
                index += 1;
                corpus = value(rest, index, "--corpus")?.to_string();
            }
            "--pack" => {
                index += 1;
                pack = Some(PathBuf::from(value(rest, index, "--pack")?));
            }
            "--json" => json = true,
            other => return Err(CliError::Usage(format!("eval 不認得的參數 {other}"))),
        }
        index += 1;
    }
    match kind {
        "zh-golden" => Ok(Command::EvalZhGolden { testdata, json }),
        "ja" => Ok(Command::EvalJa {
            testdata,
            corpus,
            pack,
            json,
        }),
        "fuzzy" => Ok(Command::EvalFuzzy { testdata, json }),
        other => Err(CliError::Usage(format!("eval 不認得的題庫 {other}"))),
    }
}

fn parse_convert(args: &[String]) -> Result<Command, CliError> {
    let mut reading = None;
    let mut testdata = None;
    let mut n = 5usize;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--testdata" => {
                index += 1;
                testdata = Some(PathBuf::from(value(args, index, "--testdata")?));
            }
            "--n" => {
                index += 1;
                n = value(args, index, "--n")?
                    .parse()
                    .map_err(|_| CliError::Usage("--n 需要正整數".to_string()))?;
                if n == 0 {
                    return Err(CliError::Usage("--n 必須大於 0".to_string()));
                }
            }
            "--json" => json = true,
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("convert 不認得的參數 {other}")));
            }
            other => {
                if reading.is_some() {
                    return Err(CliError::Usage("convert 只接受一個讀音".to_string()));
                }
                reading = Some(other.to_string());
            }
        }
        index += 1;
    }
    let reading = reading.ok_or_else(|| CliError::Usage("convert 需要一個讀音".to_string()))?;
    Ok(Command::Convert {
        reading,
        testdata,
        n,
        json,
    })
}

fn parse_bench(args: &[String]) -> Result<Command, CliError> {
    let mut file = None;
    let mut testdata = None;
    let mut repeat = 20usize;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--testdata" => {
                index += 1;
                testdata = Some(PathBuf::from(value(args, index, "--testdata")?));
            }
            "--repeat" => {
                index += 1;
                repeat = value(args, index, "--repeat")?
                    .parse()
                    .map_err(|_| CliError::Usage("--repeat 需要正整數".to_string()))?;
                if repeat == 0 {
                    return Err(CliError::Usage("--repeat 必須大於 0".to_string()));
                }
            }
            "--json" => json = true,
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("bench 不認得的參數 {other}")));
            }
            other => {
                if file.is_some() {
                    return Err(CliError::Usage("bench 只接受一個檔案".to_string()));
                }
                file = Some(PathBuf::from(other));
            }
        }
        index += 1;
    }
    let file = file.ok_or_else(|| CliError::Usage("bench 需要一個檔案".to_string()))?;
    Ok(Command::Bench {
        file,
        testdata,
        repeat,
        json,
    })
}

fn value<'a>(args: &'a [String], index: usize, flag: &str) -> Result<&'a str, CliError> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| CliError::Usage(format!("{flag} 需要一個值")))
}

/// 執行命令並把結果寫到 `out`。
pub fn run(args: &[String], out: &mut dyn Write) -> Result<(), CliError> {
    let command = parse(args)?;
    run_command(&command, out)
}

/// 執行已解析的命令。
pub fn run_command(command: &Command, out: &mut dyn Write) -> Result<(), CliError> {
    match command {
        Command::Help => {
            write!(out, "{}", usage())?;
            Ok(())
        }
        Command::Version => {
            writeln!(out, "jc-cli {}", env!("CARGO_PKG_VERSION"))?;
            Ok(())
        }
        Command::EvalZhGolden { testdata, json } => {
            crate::eval_cmd::zh_golden(testdata.clone(), *json, out)
        }
        Command::EvalJa {
            testdata,
            corpus,
            pack,
            json,
        } => crate::eval_cmd::ja(testdata.clone(), corpus, pack.clone(), *json, out),
        Command::EvalFuzzy { testdata, json } => {
            crate::eval_cmd::fuzzy(testdata.clone(), *json, out)
        }
        Command::Convert {
            reading,
            testdata,
            n,
            json,
        } => crate::converter::convert(testdata.clone(), reading, *n, *json, out),
        Command::Bench {
            file,
            testdata,
            repeat,
            json,
        } => crate::bench::run(testdata.clone(), file, *repeat, *json, out),
    }
}

/// 命令列說明。
pub fn usage() -> String {
    format!(
        "jc-cli {}（開發工具）\n\n\
         用法：\n  \
         jc-cli eval zh-golden --testdata <dir> [--json]\n  \
         jc-cli eval ja --testdata <dir> --corpus modern-500 [--pack <pack>] [--json]\n  \
         jc-cli eval fuzzy --testdata <dir> [--json]\n  \
         jc-cli convert <reading> [--testdata <dir>] [--n 5] [--json]\n  \
         jc-cli bench <file> [--testdata <dir>] [--repeat 20] [--json]\n\n\
         --testdata 預設讀 JC_TESTDATA，再退回 /mnt/work/jc/jc-ime-data/0c-testdata。\n",
        env!("CARGO_PKG_VERSION")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn parses_zh_golden_with_json() {
        assert_eq!(
            parse(&args(&[
                "eval",
                "zh-golden",
                "--testdata",
                "/tmp/t",
                "--json"
            ]))
            .unwrap(),
            Command::EvalZhGolden {
                testdata: Some(PathBuf::from("/tmp/t")),
                json: true,
            }
        );
    }

    #[test]
    fn parses_ja_with_default_corpus() {
        assert_eq!(
            parse(&args(&["eval", "ja"])).unwrap(),
            Command::EvalJa {
                testdata: None,
                corpus: "modern-500".to_string(),
                pack: None,
                json: false,
            }
        );
    }

    #[test]
    fn parses_ja_pack_and_corpus_path() {
        assert_eq!(
            parse(&args(&[
                "eval",
                "ja",
                "--corpus",
                "/tmp/subset.json",
                "--pack",
                "/tmp/ja.pack",
            ]))
            .unwrap(),
            Command::EvalJa {
                testdata: None,
                corpus: "/tmp/subset.json".to_string(),
                pack: Some(PathBuf::from("/tmp/ja.pack")),
                json: false,
            }
        );
    }

    #[test]
    fn parses_convert_reading_and_n() {
        assert_eq!(
            parse(&args(&["convert", "ㄨㄛˇ", "--n", "3"])).unwrap(),
            Command::Convert {
                reading: "ㄨㄛˇ".to_string(),
                testdata: None,
                n: 3,
                json: false,
            }
        );
    }

    #[test]
    fn rejects_missing_values() {
        assert!(parse(&args(&["convert"])).is_err());
        assert!(parse(&args(&["eval", "zh-golden", "--testdata"])).is_err());
        assert!(parse(&args(&["bench"])).is_err());
        assert!(parse(&args(&["eval", "nope"])).is_err());
    }

    #[test]
    fn help_and_version_are_recognized() {
        assert_eq!(parse(&args(&["--help"])).unwrap(), Command::Help);
        assert_eq!(parse(&args(&["-V"])).unwrap(), Command::Version);
        assert_eq!(parse(&[]).unwrap(), Command::Help);
    }
}
