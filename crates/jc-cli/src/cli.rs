//! 命令列解析與分派。
//!
//! 解析刻意手寫，與 `jc-engine` 的風格一致；錯誤訊息只描述使用者看得到的
//! 參數，不洩漏內部路徑。

use std::io::Write;
use std::path::PathBuf;

use jc_core::ja::decode::{WordGate, DEFAULT_WORD_RERANK_GATE};
use jc_core::Mode;

/// 日文評測的左文來源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LeftMode {
    /// 不給左文；與前身評測相同，基準可重現。
    #[default]
    Off,
    /// 把前一題的第一名接到左文，模擬連續輸入。
    Rolling,
}

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
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// 跑注音 golden 的 45 個案例。
    EvalZhGolden {
        /// `0c-testdata` 目錄；未指定時用 `JC_TESTDATA`。
        testdata: Option<PathBuf>,
        /// 改用注音二進位資料包（`lexicon.bin`、`lm.bin`）跑題庫。
        data: Option<PathBuf>,
        /// 覆寫 bigram 語言模型；未指定時用資料包內的 `lm.bin`。
        lm: Option<PathBuf>,
        /// bigram 內插權重 μ。
        mu: Option<f64>,
        /// 印出 bigram 查詢次數與命中次數。
        stats: bool,
        /// 輸出 JSON 而非表格。
        json: bool,
        /// 啟用注音選字模型重排。
        select_model: bool,
    },
    /// 跑一個日文題庫。
    EvalJa {
        /// `0c-testdata` 目錄。
        testdata: Option<PathBuf>,
        /// `ja/` 目錄裡的題庫名稱，或直接給題庫 `.json` 路徑。
        corpus: String,
        /// 日文 pack；未指定時由 `JC_JA_PACK` 與資料根目錄決定。
        pack: Option<PathBuf>,
        /// 是否接上神經重排；預設關閉，讓統計基準可重現。
        rerank: bool,
        /// 詞群門檻：`off`、成本差上限 `N`，或 `cand:N`（候選數下限）。
        gate: WordGate,
        /// 重排的推論後端：`ort` 或 `candle`。
        backend: jc_model::Backend,
        /// 左文來源：`off` 不給左文，`rolling` 用前一題的輸出當左文。
        left: LeftMode,
        /// 輸出 JSON。
        json: bool,
        /// 輸出含逐題結果的 JSON。
        items: bool,
    },
    /// 跑模糊輸入題庫。
    EvalFuzzy {
        /// `0c-testdata` 目錄。
        testdata: Option<PathBuf>,
        /// 輸出 JSON。
        json: bool,
    },
    /// 跑混合輸入 golden 的 6 個案例。
    EvalMixed {
        /// `0c-testdata` 目錄。
        testdata: Option<PathBuf>,
        /// 注音二進位資料包目錄；未指定時依序找 `<資料根>/build-zh` 與 golden 固定詞庫。
        data: Option<PathBuf>,
        /// 輸出 JSON。
        json: bool,
    },
    /// 單句轉換並列出 N-best。
    Convert {
        /// 讀音（注音音節或かな）。
        reading: String,
        /// `0c-testdata` 目錄。
        testdata: Option<PathBuf>,
        /// 注音二進位資料包目錄（`lexicon.bin`、`lm.bin`、`manifest.json`）。
        data: Option<PathBuf>,
        /// 覆寫 bigram 語言模型；未指定時用資料包內的 `lm.bin`。
        lm: Option<PathBuf>,
        /// bigram 內插權重 μ；未指定時用前身預設 0.7。
        mu: Option<f64>,
        /// 印出 bigram 查詢次數與命中次數。
        stats: bool,
        /// 取幾個候選。
        n: usize,
        /// 輸出 JSON。
        json: bool,
        /// 啟用注音選字模型重排。
        select_model: bool,
        /// 推論後端：`ort` 或 `candle`；未指定時讀 `JC_RERANK_BACKEND`，
        /// 預設 `ort`。
        backend: Option<jc_model::Backend>,
        /// 個人化學習資料目錄；未指定時不載入。
        user_data: Option<PathBuf>,
        /// 指定轉換模式；目前只有 `en` 會走英文候選。
        mode: Option<Mode>,
        /// 日文轉換後要把第幾個候選記進學習（1 起算）；未指定時記首選。
        pick: Option<usize>,
    },
    /// 印出 SentencePiece unigram 與最長前綴匹配的切法對照。
    TokenizeJa {
        /// `0c-testdata` 目錄。
        testdata: Option<PathBuf>,
        /// tokenizer 檔；未指定時用資料根目錄的 SentencePiece 模型。
        vocab: Option<PathBuf>,
        /// 要切分的文字；空的時候從 stdin 逐行讀。
        texts: Vec<String>,
        /// 只印兩種切法不同的句子。
        diff: bool,
        /// 輸出 JSON。
        json: bool,
    },
    /// 對檔案中的每行讀音做轉換並計時。
    Bench {
        /// 每行一個讀音。
        file: PathBuf,
        /// `0c-testdata` 目錄。
        testdata: Option<PathBuf>,
        /// 注音二進位資料包目錄；指定時只跑注音資料包。
        data: Option<PathBuf>,
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
        "tokenize" => parse_tokenize(&args[1..]),
        "bench" => parse_bench(&args[1..]),
        other => Err(CliError::Usage(format!("不認得的子命令 {other}"))),
    }
}

fn parse_eval(args: &[String]) -> Result<Command, CliError> {
    let Some(kind) = args.first().map(String::as_str) else {
        return Err(CliError::Usage(
            "eval 需要 zh-golden、ja、fuzzy 或 mixed".to_string(),
        ));
    };
    let rest = &args[1..];
    let mut testdata = None;
    let mut corpus = "modern-500".to_string();
    let mut pack = None;
    let mut data = None;
    let mut lm = None;
    let mut mu = None;
    let mut stats = false;
    let mut rerank = None;
    let mut gate = None;
    let mut backend = None;
    let mut left = LeftMode::Off;
    let mut json = false;
    let mut items = false;
    let mut select_model = None;
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
            "--select-model" => {
                index += 1;
                select_model = Some(switch(
                    value(rest, index, "--select-model")?,
                    "--select-model",
                )?);
            }
            "--data" => {
                index += 1;
                data = Some(PathBuf::from(value(rest, index, "--data")?));
            }
            "--lm" => {
                index += 1;
                lm = Some(PathBuf::from(value(rest, index, "--lm")?));
            }
            "--mu" => {
                index += 1;
                mu = Some(parse_mu(value(rest, index, "--mu")?)?);
            }
            "--stats" => stats = true,
            "--rerank" => {
                index += 1;
                rerank = Some(match value(rest, index, "--rerank")? {
                    "on" => true,
                    "off" => false,
                    other => {
                        return Err(CliError::Usage(format!(
                            "--rerank 只接受 on 或 off，實際為 {other}"
                        )));
                    }
                });
            }
            "--gate" => {
                index += 1;
                gate = Some(parse_gate(value(rest, index, "--gate")?)?);
            }
            "--backend" => {
                index += 1;
                backend = Some(
                    value(rest, index, "--backend")?
                        .parse::<jc_model::Backend>()
                        .map_err(CliError::Usage)?,
                );
            }
            "--left" => {
                index += 1;
                left = match value(rest, index, "--left")? {
                    "off" => LeftMode::Off,
                    "rolling" => LeftMode::Rolling,
                    other => {
                        return Err(CliError::Usage(format!(
                            "--left 只接受 off 或 rolling，實際為 {other}"
                        )));
                    }
                };
            }
            "--json" => json = true,
            "--items" => items = true,
            other => return Err(CliError::Usage(format!("eval 不認得的參數 {other}"))),
        }
        index += 1;
    }
    match kind {
        "zh-golden" => {
            reject_ja_flags(rerank, gate, left, "eval zh-golden")?;
            reject_backend(backend, "eval zh-golden")?;
            reject_items(items, "eval zh-golden")?;
            Ok(Command::EvalZhGolden {
                testdata,
                data,
                lm,
                mu,
                stats,
                select_model: select_model.unwrap_or(true),
                json,
            })
        }
        "ja" => {
            reject_zh_flags(data, lm, mu, stats, "eval ja")?;
            Ok(Command::EvalJa {
                testdata,
                corpus,
                pack,
                rerank: rerank.unwrap_or(false),
                gate: gate.unwrap_or(DEFAULT_WORD_RERANK_GATE),
                backend: backend.unwrap_or(jc_model::Backend::Ort),
                left,
                json,
                items,
            })
        }
        "fuzzy" => {
            reject_zh_flags(data, lm, mu, stats, "eval fuzzy")?;
            reject_ja_flags(rerank, gate, left, "eval fuzzy")?;
            reject_backend(backend, "eval fuzzy")?;
            reject_items(items, "eval fuzzy")?;
            Ok(Command::EvalFuzzy { testdata, json })
        }
        "mixed" => {
            reject_ja_flags(rerank, gate, left, "eval mixed")?;
            reject_backend(backend, "eval mixed")?;
            reject_items(items, "eval mixed")?;
            Ok(Command::EvalMixed {
                testdata,
                data,
                json,
            })
        }
        other => Err(CliError::Usage(format!("eval 不認得的題庫 {other}"))),
    }
}

/// 只有注音題庫接受 `--data`、`--lm` 與 `--mu`。
fn reject_zh_flags(
    data: Option<PathBuf>,
    lm: Option<PathBuf>,
    mu: Option<f64>,
    stats: bool,
    command: &str,
) -> Result<(), CliError> {
    if data.is_some() || lm.is_some() || mu.is_some() || stats {
        return Err(CliError::Usage(format!(
            "{command} 不接受 --data、--lm、--mu 或 --stats"
        )));
    }
    Ok(())
}

/// 只有日文題庫接受 `--backend`。
fn reject_backend(backend: Option<jc_model::Backend>, command: &str) -> Result<(), CliError> {
    if backend.is_some() {
        return Err(CliError::Usage(format!("{command} 不接受 --backend")));
    }
    Ok(())
}

/// 只有日文題庫接受 `--rerank`、`--gate` 與 `--left`。
fn reject_ja_flags(
    rerank: Option<bool>,
    gate: Option<WordGate>,
    left: LeftMode,
    command: &str,
) -> Result<(), CliError> {
    if rerank.is_some() {
        return Err(CliError::Usage(format!("{command} 不接受 --rerank")));
    }
    if gate.is_some() {
        return Err(CliError::Usage(format!("{command} 不接受 --gate")));
    }
    if left != LeftMode::Off {
        return Err(CliError::Usage(format!("{command} 不接受 --left")));
    }
    Ok(())
}

/// `--items` 只有日文題庫輸出逐題結果，其他子命令不接受。
fn reject_items(items: bool, command: &str) -> Result<(), CliError> {
    if items {
        return Err(CliError::Usage(format!("{command} 不接受 --items")));
    }
    Ok(())
}

/// 解析詞群門檻：`off`、成本差上限，或 `cand:N` 的候選數下限。
fn parse_gate(value: &str) -> Result<WordGate, CliError> {
    if value == "off" {
        return Ok(WordGate::Off);
    }
    if let Some(count) = value.strip_prefix("cand:") {
        let count: usize = count
            .parse()
            .map_err(|_| CliError::Usage("--gate cand:N 需要正整數".to_string()))?;
        if count < 2 {
            return Err(CliError::Usage("--gate cand:N 的 N 至少 2".to_string()));
        }
        return Ok(WordGate::Candidates(count));
    }
    let gap: i32 = value
        .parse()
        .map_err(|_| CliError::Usage("--gate 需要 off、成本差或 cand:N".to_string()))?;
    if gap < 0 {
        return Err(CliError::Usage("--gate 的成本差不能是負數".to_string()));
    }
    Ok(WordGate::Gap(gap))
}

/// 解析 bigram 內插權重，範圍與前身 `Params.Mu` 相同。
fn parse_mu(value: &str) -> Result<f64, CliError> {
    let mu: f64 = value
        .parse()
        .map_err(|_| CliError::Usage("--mu 需要 0 到 1 的數字".to_string()))?;
    if !(0.0..=1.0).contains(&mu) {
        return Err(CliError::Usage("--mu 必須介於 0 與 1".to_string()));
    }
    Ok(mu)
}

fn parse_convert(args: &[String]) -> Result<Command, CliError> {
    let mut reading = None;
    let mut testdata = None;
    let mut data = None;
    let mut lm = None;
    let mut mu = None;
    let mut stats = false;
    let mut n = 5usize;
    let mut json = false;
    let mut select_model = true;
    let mut backend = None;
    let mut user_data = None;
    let mut mode = None;
    let mut pick = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--testdata" => {
                index += 1;
                testdata = Some(PathBuf::from(value(args, index, "--testdata")?));
            }
            "--data" => {
                index += 1;
                data = Some(PathBuf::from(value(args, index, "--data")?));
            }
            "--select-model" => {
                index += 1;
                select_model = switch(value(args, index, "--select-model")?, "--select-model")?;
            }
            "--backend" => {
                index += 1;
                let raw = value(args, index, "--backend")?;
                backend = Some(raw.parse().map_err(CliError::Usage)?);
            }
            "--lm" => {
                index += 1;
                lm = Some(PathBuf::from(value(args, index, "--lm")?));
            }
            "--mu" => {
                index += 1;
                mu = Some(parse_mu(value(args, index, "--mu")?)?);
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
            "--stats" => stats = true,
            "--json" => json = true,
            "--mode" => {
                index += 1;
                mode = Some(parse_convert_mode(value(args, index, "--mode")?)?);
            }
            "--user-data" => {
                index += 1;
                user_data = Some(PathBuf::from(value(args, index, "--user-data")?));
            }
            "--pick" => {
                index += 1;
                let picked: usize = value(args, index, "--pick")?
                    .parse()
                    .map_err(|_| CliError::Usage("--pick 需要正整數".to_string()))?;
                if picked == 0 {
                    return Err(CliError::Usage("--pick 從 1 起算".to_string()));
                }
                pick = Some(picked);
            }
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
        data,
        lm,
        mu,
        stats,
        n,
        json,
        select_model,
        backend,
        user_data,
        mode,
        pick,
    })
}

/// 解析 `convert --mode`；目前只支援英文候選。
fn parse_convert_mode(value: &str) -> Result<Mode, CliError> {
    match value {
        "en" => Ok(Mode::En),
        other => Err(CliError::Usage(format!(
            "convert --mode 只接受 en，收到 {other:?}"
        ))),
    }
}

/// 解析 `on`／`off` 開關。
fn switch(value: &str, flag: &str) -> Result<bool, CliError> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        other => Err(CliError::Usage(format!(
            "{flag} 只接受 on 或 off，收到 {other:?}"
        ))),
    }
}

/// 解析 `tokenize ja`。
fn parse_tokenize(args: &[String]) -> Result<Command, CliError> {
    let Some(kind) = args.first().map(String::as_str) else {
        return Err(CliError::Usage("tokenize 需要 ja".to_string()));
    };
    if kind != "ja" {
        return Err(CliError::Usage(format!("tokenize 只支援 ja，收到 {kind}")));
    }
    let rest = &args[1..];
    let mut testdata = None;
    let mut vocab = None;
    let mut texts = Vec::new();
    let mut diff = false;
    let mut json = false;
    let mut index = 0;
    while index < rest.len() {
        match rest[index].as_str() {
            "--testdata" => {
                index += 1;
                testdata = Some(PathBuf::from(value(rest, index, "--testdata")?));
            }
            "--vocab" => {
                index += 1;
                vocab = Some(PathBuf::from(value(rest, index, "--vocab")?));
            }
            "--text" => {
                index += 1;
                texts.push(value(rest, index, "--text")?.to_string());
            }
            "--diff" => diff = true,
            "--json" => json = true,
            other => return Err(CliError::Usage(format!("tokenize 不認得的參數 {other}"))),
        }
        index += 1;
    }
    Ok(Command::TokenizeJa {
        testdata,
        vocab,
        texts,
        diff,
        json,
    })
}

fn parse_bench(args: &[String]) -> Result<Command, CliError> {
    let mut file = None;
    let mut testdata = None;
    let mut data = None;
    let mut repeat = 20usize;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--testdata" => {
                index += 1;
                testdata = Some(PathBuf::from(value(args, index, "--testdata")?));
            }
            "--data" => {
                index += 1;
                data = Some(PathBuf::from(value(args, index, "--data")?));
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
        data,
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
        Command::EvalZhGolden {
            testdata,
            data,
            lm,
            mu,
            stats,
            json,
            select_model,
        } => crate::eval_cmd::zh_golden(
            crate::eval_cmd::GoldenOptions {
                testdata: testdata.clone(),
                data: data.clone(),
                lm: lm.clone(),
                mu: *mu,
                stats: *stats,
                json: *json,
                select_model: *select_model,
            },
            out,
        ),
        Command::EvalJa {
            testdata,
            corpus,
            pack,
            rerank,
            gate,
            backend,
            left,
            json,
            items,
        } => crate::eval_cmd::ja(
            crate::eval_cmd::JaOptions {
                testdata: testdata.clone(),
                corpus: corpus.clone(),
                pack: pack.clone(),
                rerank: *rerank,
                gate: rerank.then_some(*gate),
                backend: *backend,
                left: *left,
                json: *json,
                items: *items,
            },
            out,
        ),
        Command::EvalFuzzy { testdata, json } => {
            crate::eval_cmd::fuzzy(testdata.clone(), *json, out)
        }
        Command::EvalMixed {
            testdata,
            data,
            json,
        } => crate::eval_cmd::mixed(testdata.clone(), data.clone(), *json, out),
        Command::Convert {
            reading,
            testdata,
            data,
            lm,
            mu,
            stats,
            n,
            json,
            select_model,
            backend,
            user_data,
            mode,
            pick,
        } => crate::converter::convert(
            crate::converter::ConvertOptions {
                testdata: testdata.clone(),
                data: data.clone(),
                lm: lm.clone(),
                mu: *mu,
                stats: *stats,
                n: *n,
                json: *json,
                select_model: *select_model,
                backend: *backend,
                user_data: user_data.clone(),
                mode: *mode,
                pick: *pick,
            },
            reading,
            out,
        ),
        Command::TokenizeJa {
            testdata,
            vocab,
            texts,
            diff,
            json,
        } => crate::tokenize::ja(
            crate::tokenize::TokenizeOptions {
                testdata: testdata.clone(),
                vocab: vocab.clone(),
                texts: texts.clone(),
                diff: *diff,
                json: *json,
            },
            out,
        ),
        Command::Bench {
            file,
            testdata,
            data,
            repeat,
            json,
        } => crate::bench::run(testdata.clone(), data.clone(), file, *repeat, *json, out),
    }
}

/// 命令列說明。
pub fn usage() -> String {
    format!(
        "jc-cli {}（開發工具）\n\n\
         用法：\n  \
         jc-cli eval zh-golden --testdata <dir> [--data <dir>] [--lm <lm.bin>] [--mu <0..1>] [--stats] [--select-model on|off] [--json]\n  \
         jc-cli eval ja --testdata <dir> --corpus modern-500 [--pack <pack>] \
         [--rerank on|off] [--gate off|<成本差>|cand:<N>] [--backend ort|candle] \
         [--left off|rolling] [--json] [--items]\n  \
         jc-cli eval fuzzy --testdata <dir> [--json]\n  \
         jc-cli eval mixed --testdata <dir> [--data <dir>] [--json]\n  \
         jc-cli convert <reading> [--mode en] [--testdata <dir>] [--data <dir>] [--lm <lm.bin>] [--mu <0..1>] [--stats] [--n 5] [--select-model on|off] [--backend ort|candle] [--user-data <dir>] [--pick <n>] [--json]\n  \
         jc-cli tokenize ja [--testdata <dir>] [--vocab <model>] [--text <字串>]... [--diff] [--json]\n  \
         jc-cli bench <file> [--testdata <dir>] [--data <dir>] [--repeat 20] [--json]\n\n\
         --backend 選擇日文 reranker 的推論後端；預設 ort，candle 讀 model.safetensors。\n\
         --testdata 預設讀 JC_TESTDATA，再退回 /mnt/work/jc/jc-ime-data/0c-testdata。\n\
         --corpus 是 ja/ 目錄裡的題庫名稱；目錄下每份題庫 json 都會被掃到。\n\
         --data 指向注音二進位資料包（lexicon.bin、lm.bin、manifest.json）；\n\
         指定時注音走真實詞庫並自動載入同目錄的 lm.bin。\n\
         --lm 覆寫 bigram 語言模型；--mu 是 bigram 與 unigram 的內插權重（預設 0.7）。\n\
         --select-model on 需要 zh-select 模型與 ONNX Runtime；預設 on，可用 off 關閉。\n\
         --backend 選 reranker 推論後端：ort（預設）或 candle（讀 model.safetensors，不需要 ONNX Runtime）。\n\
         --user-data 指定個人資料目錄；日文學習寫進同一個 personal.json，--pick 記下第幾個候選。\n",
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
                data: None,
                lm: None,
                mu: None,
                stats: false,
                json: true,
                select_model: true,
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
                rerank: false,
                gate: DEFAULT_WORD_RERANK_GATE,
                backend: jc_model::Backend::Ort,
                left: LeftMode::Off,
                json: false,
                items: false,
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
                rerank: false,
                gate: DEFAULT_WORD_RERANK_GATE,
                backend: jc_model::Backend::Ort,
                left: LeftMode::Off,
                json: false,
                items: false,
            }
        );
    }

    #[test]
    fn parses_ja_rerank_switch() {
        assert_eq!(
            parse(&args(&["eval", "ja", "--rerank", "on"])).unwrap(),
            Command::EvalJa {
                testdata: None,
                corpus: "modern-500".to_string(),
                pack: None,
                rerank: true,
                gate: DEFAULT_WORD_RERANK_GATE,
                backend: jc_model::Backend::Ort,
                left: LeftMode::Off,
                json: false,
                items: false,
            }
        );
        assert!(parse(&args(&["eval", "ja", "--rerank", "maybe"])).is_err());
        assert!(parse(&args(&["eval", "fuzzy", "--rerank", "on"])).is_err());
    }

    #[test]
    fn parses_ja_backend_switch() {
        assert_eq!(
            parse(&args(&[
                "eval",
                "ja",
                "--rerank",
                "on",
                "--backend",
                "candle"
            ]))
            .unwrap(),
            Command::EvalJa {
                testdata: None,
                corpus: "modern-500".to_string(),
                pack: None,
                rerank: true,
                gate: DEFAULT_WORD_RERANK_GATE,
                backend: jc_model::Backend::Candle,
                left: LeftMode::Off,
                json: false,
                items: false,
            }
        );
        assert!(parse(&args(&["eval", "ja", "--backend", "tensorrt"])).is_err());
        assert!(parse(&args(&["eval", "fuzzy", "--backend", "ort"])).is_err());
    }

    #[test]
    fn parses_ja_items_output() {
        assert_eq!(
            parse(&args(&["eval", "ja", "--items"])).unwrap(),
            Command::EvalJa {
                testdata: None,
                corpus: "modern-500".to_string(),
                pack: None,
                rerank: false,
                gate: DEFAULT_WORD_RERANK_GATE,
                backend: jc_model::Backend::Ort,
                left: LeftMode::Off,
                json: false,
                items: true,
            }
        );
        assert!(parse(&args(&["eval", "zh-golden", "--items"])).is_err());
        assert!(parse(&args(&["eval", "mixed", "--items"])).is_err());
    }

    #[test]
    fn parses_ja_gate_thresholds() {
        assert_eq!(
            parse(&args(&["eval", "ja", "--rerank", "on", "--gate", "200"])).unwrap(),
            Command::EvalJa {
                testdata: None,
                corpus: "modern-500".to_string(),
                pack: None,
                rerank: true,
                gate: WordGate::Gap(200),
                backend: jc_model::Backend::Ort,
                left: LeftMode::Off,
                json: false,
                items: false,
            }
        );
        assert_eq!(
            parse(&args(&["eval", "ja", "--gate", "cand:3"])).unwrap(),
            Command::EvalJa {
                testdata: None,
                corpus: "modern-500".to_string(),
                pack: None,
                rerank: false,
                gate: WordGate::Candidates(3),
                backend: jc_model::Backend::Ort,
                left: LeftMode::Off,
                json: false,
                items: false,
            }
        );
        assert_eq!(
            parse(&args(&["eval", "ja", "--gate", "off"])).unwrap(),
            Command::EvalJa {
                testdata: None,
                corpus: "modern-500".to_string(),
                pack: None,
                rerank: false,
                gate: WordGate::Off,
                backend: jc_model::Backend::Ort,
                left: LeftMode::Off,
                json: false,
                items: false,
            }
        );
        assert!(parse(&args(&["eval", "ja", "--gate", "-1"])).is_err());
        assert!(parse(&args(&["eval", "ja", "--gate", "cand:1"])).is_err());
        assert!(parse(&args(&["eval", "zh-golden", "--gate", "100"])).is_err());
    }

    #[test]
    fn parses_convert_reading_and_n() {
        assert_eq!(
            parse(&args(&["convert", "ㄨㄛˇ", "--n", "3"])).unwrap(),
            Command::Convert {
                reading: "ㄨㄛˇ".to_string(),
                testdata: None,
                data: None,
                lm: None,
                mu: None,
                stats: false,
                n: 3,
                json: false,
                select_model: true,
                backend: None,
                user_data: None,
                mode: None,
                pick: None,
            }
        );
    }

    #[test]
    fn parses_convert_pick_and_user_data() {
        let parsed = parse(&args(&[
            "convert",
            "きょう",
            "--user-data",
            "/tmp/opencode/ja-user",
            "--pick",
            "2",
        ]))
        .unwrap();
        assert_eq!(
            parsed,
            Command::Convert {
                reading: "きょう".to_string(),
                testdata: None,
                data: None,
                lm: None,
                mu: None,
                stats: false,
                n: 5,
                json: false,
                select_model: true,
                backend: None,
                user_data: Some(PathBuf::from("/tmp/opencode/ja-user")),
                mode: None,
                pick: Some(2),
            }
        );
        assert!(parse(&args(&["convert", "きょう", "--pick", "0"])).is_err());
        assert!(parse(&args(&["convert", "きょう", "--pick", "x"])).is_err());
    }

    #[test]
    fn parses_select_model_switch() {
        assert_eq!(
            parse(&args(&["convert", "ㄨㄛˇ", "--select-model", "on"])).unwrap(),
            Command::Convert {
                reading: "ㄨㄛˇ".to_string(),
                testdata: None,
                data: None,
                lm: None,
                mu: None,
                stats: false,
                n: 5,
                json: false,
                select_model: true,
                backend: None,
                user_data: None,
                mode: None,
                pick: None,
            }
        );
        assert_eq!(
            parse(&args(&["eval", "zh-golden", "--select-model", "off"])).unwrap(),
            Command::EvalZhGolden {
                testdata: None,
                data: None,
                lm: None,
                mu: None,
                stats: false,
                json: false,
                select_model: false,
            }
        );
        assert!(parse(&args(&["convert", "ㄨㄛˇ", "--select-model", "auto"])).is_err());
    }

    #[test]
    fn parses_convert_backend() {
        let parsed = parse(&args(&["convert", "わたし", "--backend", "candle"])).unwrap();
        match parsed {
            Command::Convert { backend, .. } => {
                assert_eq!(backend, Some(jc_model::Backend::Candle));
            }
            other => panic!("應解析成 convert，實際為 {other:?}"),
        }
        assert!(parse(&args(&["convert", "わたし", "--backend", "tensorrt"])).is_err());
    }

    #[test]
    fn parses_bigram_flags() {
        let parsed = parse(&args(&[
            "convert",
            "ㄨㄛˇ",
            "--data",
            "/tmp/pack",
            "--lm",
            "/tmp/lm.bin",
            "--mu",
            "0.5",
        ]))
        .unwrap();
        assert_eq!(
            parsed,
            Command::Convert {
                reading: "ㄨㄛˇ".to_string(),
                testdata: None,
                data: Some(PathBuf::from("/tmp/pack")),
                lm: Some(PathBuf::from("/tmp/lm.bin")),
                mu: Some(0.5),
                stats: false,
                n: 5,
                json: false,
                select_model: true,
                backend: None,
                user_data: None,
                mode: None,
                pick: None,
            }
        );
        assert!(parse(&args(&["convert", "ㄚ", "--mu", "1.5"])).is_err());
        assert!(parse(&args(&["eval", "ja", "--lm", "/tmp/lm.bin"])).is_err());
        assert!(parse(&args(&["eval", "fuzzy", "--data", "/tmp/pack"])).is_err());
    }

    #[test]
    fn rejects_missing_values() {
        assert!(parse(&args(&["convert"])).is_err());
        assert!(parse(&args(&["eval", "zh-golden", "--testdata"])).is_err());
        assert!(parse(&args(&["bench"])).is_err());
        assert!(parse(&args(&["eval", "nope"])).is_err());
    }

    #[test]
    fn parses_tokenize_ja_texts() {
        assert_eq!(
            parse(&args(&[
                "tokenize",
                "ja",
                "--text",
                "東京都",
                "--text",
                "大阪",
                "--diff",
                "--json",
            ]))
            .unwrap(),
            Command::TokenizeJa {
                testdata: None,
                vocab: None,
                texts: vec!["東京都".to_string(), "大阪".to_string()],
                diff: true,
                json: true,
            }
        );
        assert!(parse(&args(&["tokenize"])).is_err());
        assert!(parse(&args(&["tokenize", "zh"])).is_err());
        assert!(parse(&args(&["tokenize", "ja", "--text"])).is_err());
    }

    #[test]
    fn help_and_version_are_recognized() {
        assert_eq!(parse(&args(&["--help"])).unwrap(), Command::Help);
        assert_eq!(parse(&args(&["-V"])).unwrap(), Command::Version);
        assert_eq!(parse(&[]).unwrap(), Command::Help);
    }
}
