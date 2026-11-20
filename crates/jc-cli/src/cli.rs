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

/// `--neural` 的三種模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NeuralMode {
    /// 低呼叫版：`gap < 472.5` 且 `mean_candidates > 5.98`。
    #[default]
    Auto,
    /// 品質優先版：只看 `gap < 460`。
    Quality,
    /// 完全關閉，回到統計路徑。
    Off,
}

impl NeuralMode {
    /// 命令列字串；與 `--neural` 的取值一致。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Quality => "quality",
            Self::Off => "off",
        }
    }
}

/// 逐字変換的觸發設定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveTriggerSpec {
    /// 未轉換假名的長度門檻；0 關閉。
    pub min_units: usize,
    /// 助詞、標點或詞典完整詞觸發。
    pub boundary: bool,
    /// 停頓觸發；由評測在句尾與標點後模擬。
    pub idle: bool,
}

impl Default for LiveTriggerSpec {
    /// 預設每鍵轉換，三個自動觸發都關閉。
    fn default() -> Self {
        Self {
            min_units: 0,
            boundary: false,
            idle: false,
        }
    }
}

impl LiveTriggerSpec {
    /// 是否為每鍵轉換。
    pub fn every_key(self) -> bool {
        self.min_units == 0 && !self.boundary && !self.idle
    }
}

/// 逐字変換的神經處理範圍。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveScope {
    /// 整段未鎖定尾端。
    Whole,
    /// 只處理最後一個未鎖定文節。
    Clause,
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
        /// 左文來源：`off` 不給左文，`rolling` 用前一題的輸出當左文。
        left: LeftMode,
        /// 是否啟用 Jinen 神經轉換：`auto`（預設）、`quality` 或 `off`。
        neural: NeuralMode,
        /// 是否走逐字変換（ライブ変換）；與 `--neural` 的組合共用同一份門檻。
        live: bool,
        /// 逐字変換的觸發條件；`None` 用預設自動変換。
        live_trigger: Option<LiveTriggerSpec>,
        /// 神經處理範圍；`None` 由觸發條件決定。
        live_scope: Option<LiveScope>,
        /// 停頓後才做神經補正。
        live_deferred: bool,
        /// 強制即時補正，不做停頓等待。
        live_inline: bool,
        /// 延遲整句轉換：打字只顯示假名，停頓後才轉。
        live_sentence: bool,
        /// 停頓毫秒數；只用於延遲整句。
        live_idle_ms: u64,
        /// 前綴快取模式（對照實驗）。
        live_prefix_cache: bool,
        /// 前綴快取保留幾個文節。
        live_cache_keep: usize,
        /// single-flight 非同步推論。
        live_flight: bool,
        /// single-flight 要求 generation 完全相同才套用。
        live_flight_strict: bool,
        /// 補正推論預算（毫秒）。
        live_budget_ms: u64,
        /// 補正的額外嚴格倍率；套用在成本差門檻上。
        live_correction_ratio: f32,
        /// 尾端保留幾個文節不鎖定。
        live_keep_tail: usize,
        /// 印出逐字変換的詳細量測。
        live_report: bool,
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
        /// 顯示候選詞性；不加時輸出與原本相同。
        pos: bool,
        /// 啟用注音選字模型重排。
        select_model: bool,
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
    /// 產生資料根目錄的 `manifest.json`。
    DataManifest {
        /// 資料根目錄。
        root: PathBuf,
        /// 輸出檔；未指定時寫 `<root>/manifest.json`。
        out: Option<PathBuf>,
        /// 資料內容版本標籤；未指定時用 UTC 日期。
        version: Option<String>,
    },
    /// 驗證資料根目錄的 manifest。
    DataVerify {
        /// 資料根目錄。
        root: PathBuf,
        /// manifest 路徑；未指定時讀 `<root>/manifest.json`。
        manifest: Option<PathBuf>,
        /// 輸出 JSON。
        json: bool,
    },
    /// 列出下載項（`shipped: false`）的狀態。
    DataStatus {
        /// 資料根目錄。
        root: PathBuf,
        /// manifest 路徑；未指定時讀 `<root>/manifest.json`。
        manifest: Option<PathBuf>,
        /// 輸出 JSON。
        json: bool,
    },
    /// 下載 manifest 的 `shipped: false` 項目。
    DataDownload {
        /// 資料根目錄。
        root: PathBuf,
        /// manifest 路徑；未指定時讀 `<root>/manifest.json`。
        manifest: Option<PathBuf>,
        /// 不印進度。
        quiet: bool,
    },
    /// 研究用：把一段拉丁輸入切成羅馬字與注音，並量測逐鍵延遲。
    MixedSplit {
        /// 要切分的輸入；`--keys` 可重複指定。
        keys: Vec<String>,
        /// `0c-testdata` 目錄。
        testdata: Option<PathBuf>,
        /// 注音二進位資料包目錄；未指定時依序找 `<資料根>/build-zh` 與 golden 固定詞庫。
        data: Option<PathBuf>,
        /// 逐鍵量測延遲。
        incremental: bool,
        /// 每個前綴重複次數。
        repeat: usize,
        /// 輸出 JSON。
        json: bool,
    },
    /// 研究用：大規模比對離線 DP 與正式路由的分詞差異。
    MixedDiff {
        /// 產生器種子。
        seed: u64,
        /// 要產生的輸入數量。
        count: usize,
        /// 每桶輸出的代表案例數。
        top: usize,
        /// `0c-testdata` 目錄。
        testdata: Option<PathBuf>,
        /// 注音二進位資料包目錄；未指定時依序找 `<資料根>/build-zh` 與 golden 固定詞庫。
        data: Option<PathBuf>,
        /// 輸出 JSON。
        json: bool,
        /// 把不一致案例寫成 JSON 檔。
        cases: Option<PathBuf>,
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
        "data" => parse_data(&args[1..]),
        "mixed-split" => parse_mixed_split(&args[1..]),
        "mixed-diff" => parse_mixed_diff(&args[1..]),
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
    let mut backend = false;
    let mut left = LeftMode::Off;
    let mut neural = None;
    let mut live = false;
    let mut no_live = false;
    let mut live_trigger: Option<LiveTriggerSpec> = None;
    let mut live_scope = None;
    let mut live_deferred = false;
    let mut live_inline = false;
    let mut live_sentence = false;
    let mut live_idle_ms = 300u64;
    let mut live_prefix_cache = false;
    let mut live_cache_keep = 1usize;
    let mut live_flight = false;
    let mut live_flight_strict = false;
    let mut live_budget_ms = 35u64;
    let mut live_correction_ratio = 0.5f32;
    let mut live_keep_tail = 1usize;
    let mut live_report = false;
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
                parse_backend(value(rest, index, "--backend")?)?;
                backend = true;
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
            "--neural" => {
                index += 1;
                neural = Some(parse_neural(value(rest, index, "--neural")?)?);
            }
            "--live" => live = true,
            "--no-live" => no_live = true,
            "--live-trigger" => {
                index += 1;
                live_trigger = Some(parse_live_trigger(value(rest, index, "--live-trigger")?)?);
            }
            "--live-scope" => {
                index += 1;
                live_scope = Some(match value(rest, index, "--live-scope")? {
                    "whole" => LiveScope::Whole,
                    "clause" => LiveScope::Clause,
                    other => {
                        return Err(CliError::Usage(format!(
                            "--live-scope 只接受 whole 或 clause，實際為 {other}"
                        )));
                    }
                });
            }
            "--live-deferred" => live_deferred = true,
            "--live-inline" => live_inline = true,
            "--live-sentence" => live_sentence = true,
            "--live-idle-ms" => {
                index += 1;
                live_idle_ms = value(rest, index, "--live-idle-ms")?
                    .parse()
                    .map_err(|_| CliError::Usage("--live-idle-ms 需要整數毫秒".to_string()))?;
            }
            "--live-prefix-cache" => live_prefix_cache = true,
            "--live-cache-keep" => {
                index += 1;
                live_cache_keep = value(rest, index, "--live-cache-keep")?
                    .parse()
                    .map_err(|_| CliError::Usage("--live-cache-keep 需要整數".to_string()))?;
            }
            "--live-flight" => live_flight = true,
            "--live-flight-strict" => live_flight_strict = true,
            "--live-budget-ms" => {
                index += 1;
                live_budget_ms = value(rest, index, "--live-budget-ms")?
                    .parse()
                    .map_err(|_| CliError::Usage("--live-budget-ms 需要整數毫秒".to_string()))?;
            }
            "--live-correction-ratio" => {
                index += 1;
                live_correction_ratio = value(rest, index, "--live-correction-ratio")?
                    .parse()
                    .map_err(|_| {
                        CliError::Usage("--live-correction-ratio 需要 0 到 1 的數字".to_string())
                    })?;
            }
            "--live-keep-tail" => {
                index += 1;
                live_keep_tail = value(rest, index, "--live-keep-tail")?
                    .parse()
                    .map_err(|_| CliError::Usage("--live-keep-tail 需要正整數".to_string()))?;
            }
            "--live-report" => live_report = true,
            "--json" => json = true,
            "--items" => items = true,
            other => return Err(CliError::Usage(format!("eval 不認得的參數 {other}"))),
        }
        index += 1;
    }
    let live_extra = live_trigger.is_some()
        || live_scope.is_some()
        || live_deferred
        || live_inline
        || live_sentence
        || live_prefix_cache
        || live_flight
        || live_flight_strict
        || live_idle_ms != 300
        || live_cache_keep != 1
        || live_report
        || (live_correction_ratio - 0.5).abs() > f32::EPSILON
        || live_keep_tail != 1;
    if live_extra && no_live {
        return Err(CliError::Usage(
            "--no-live 不能與逐字変換的細部旗標併用".to_string(),
        ));
    }
    if live && no_live {
        return Err(CliError::Usage(
            "--live 與 --no-live 不能同時指定".to_string(),
        ));
    }
    match kind {
        "zh-golden" => {
            reject_ja_flags(
                rerank,
                gate,
                left,
                neural,
                live,
                live_extra,
                "eval zh-golden",
            )?;
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
                left,
                neural: neural.unwrap_or(NeuralMode::Auto),
                live: !no_live,
                live_trigger,
                live_scope,
                live_deferred,
                live_inline,
                live_sentence,
                live_idle_ms,
                live_prefix_cache,
                live_cache_keep,
                live_flight,
                live_flight_strict,
                live_budget_ms,
                live_correction_ratio,
                live_keep_tail,
                live_report,
                json,
                items,
            })
        }
        "fuzzy" => {
            reject_zh_flags(data, lm, mu, stats, "eval fuzzy")?;
            reject_ja_flags(rerank, gate, left, neural, live, live_extra, "eval fuzzy")?;
            reject_backend(backend, "eval fuzzy")?;
            reject_items(items, "eval fuzzy")?;
            Ok(Command::EvalFuzzy { testdata, json })
        }
        "mixed" => {
            reject_ja_flags(rerank, gate, left, neural, live, live_extra, "eval mixed")?;
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

/// `--backend` 只有注音 golden 與日文題庫接受。
fn reject_backend(backend: bool, command: &str) -> Result<(), CliError> {
    if backend {
        return Err(CliError::Usage(format!("{command} 不接受 --backend")));
    }
    Ok(())
}

/// `--backend` 只接受 `candle`；ORT 後端已移除，其他值直接回錯，
/// 不靜默忽略。
fn parse_backend(value: &str) -> Result<(), CliError> {
    if value == jc_model::BACKEND_NAME {
        return Ok(());
    }
    Err(CliError::Usage(format!(
        "--backend 只接受 {}，實際為 {value}",
        jc_model::BACKEND_NAME
    )))
}

/// 只有日文題庫接受 `--rerank`、`--gate`、`--left`、`--neural` 與 `--live`。
fn reject_ja_flags(
    rerank: Option<bool>,
    gate: Option<WordGate>,
    left: LeftMode,
    neural: Option<NeuralMode>,
    live: bool,
    live_extra: bool,
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
    if neural.is_some() {
        return Err(CliError::Usage(format!("{command} 不接受 --neural")));
    }
    if live || live_extra {
        return Err(CliError::Usage(format!("{command} 不接受 --live 系列參數")));
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
    let mut pos = false;
    let mut select_model = true;
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
                parse_backend(value(args, index, "--backend")?)?;
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
            "--pos" => pos = true,
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
        pos,
        select_model,
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

/// 解析 `mixed-split`：研究用的拉丁切分與逐鍵延遲量測。
fn parse_mixed_split(args: &[String]) -> Result<Command, CliError> {
    let mut keys = Vec::new();
    let mut testdata = None;
    let mut data = None;
    let mut incremental = false;
    let mut repeat = 1usize;
    let mut json = false;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--keys" => {
                index += 1;
                keys.push(value(args, index, "--keys")?.to_string());
            }
            "--testdata" => {
                index += 1;
                testdata = Some(PathBuf::from(value(args, index, "--testdata")?));
            }
            "--data" => {
                index += 1;
                data = Some(PathBuf::from(value(args, index, "--data")?));
            }
            "--incremental" => incremental = true,
            "--repeat" => {
                index += 1;
                let text = value(args, index, "--repeat")?;
                repeat = text
                    .parse()
                    .map_err(|_| CliError::Usage(format!("--repeat 必須是整數：{text}")))?;
                if repeat == 0 {
                    return Err(CliError::Usage("--repeat 至少 1".to_string()));
                }
            }
            "--json" => json = true,
            other => {
                return Err(CliError::Usage(format!("mixed-split 不認得的參數 {other}")));
            }
        }
        index += 1;
    }
    if keys.is_empty() {
        return Err(CliError::Usage(
            "mixed-split 需要至少一個 --keys".to_string(),
        ));
    }
    Ok(Command::MixedSplit {
        keys,
        testdata,
        data,
        incremental,
        repeat,
        json,
    })
}

/// 解析 `mixed-diff`：大規模比對兩種分詞方式。
fn parse_mixed_diff(args: &[String]) -> Result<Command, CliError> {
    let mut seed = 1u64;
    let mut count = 20_000usize;
    let mut top = 3usize;
    let mut testdata = None;
    let mut data = None;
    let mut json = false;
    let mut cases = None;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--seed" => {
                index += 1;
                let text = value(args, index, "--seed")?;
                seed = text
                    .parse()
                    .map_err(|_| CliError::Usage(format!("--seed 必須是整數：{text}")))?;
            }
            "--count" => {
                index += 1;
                let text = value(args, index, "--count")?;
                count = text
                    .parse()
                    .map_err(|_| CliError::Usage(format!("--count 必須是整數：{text}")))?;
                if count == 0 {
                    return Err(CliError::Usage("--count 至少 1".to_string()));
                }
            }
            "--top" => {
                index += 1;
                let text = value(args, index, "--top")?;
                top = text
                    .parse()
                    .map_err(|_| CliError::Usage(format!("--top 必須是整數：{text}")))?;
                if top == 0 {
                    return Err(CliError::Usage("--top 至少 1".to_string()));
                }
            }
            "--testdata" => {
                index += 1;
                testdata = Some(PathBuf::from(value(args, index, "--testdata")?));
            }
            "--data" => {
                index += 1;
                data = Some(PathBuf::from(value(args, index, "--data")?));
            }
            "--json" => json = true,
            "--cases" => {
                index += 1;
                cases = Some(PathBuf::from(value(args, index, "--cases")?));
            }
            other => {
                return Err(CliError::Usage(format!("mixed-diff 不認得的參數 {other}")));
            }
        }
        index += 1;
    }
    Ok(Command::MixedDiff {
        seed,
        count,
        top,
        testdata,
        data,
        json,
        cases,
    })
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

/// 解析 `--neural` 的 `auto`／`quality`／`off`。
fn parse_neural(value: &str) -> Result<NeuralMode, CliError> {
    match value {
        "auto" => Ok(NeuralMode::Auto),
        "quality" => Ok(NeuralMode::Quality),
        "off" => Ok(NeuralMode::Off),
        other => Err(CliError::Usage(format!(
            "--neural 只接受 auto、quality 或 off，收到 {other:?}"
        ))),
    }
}

/// 解析 `--live-trigger`：逗號分隔的 `length:N`、`boundary`、`idle`。
///
/// 空字串或 `key` 代表每鍵轉換。`length:0` 視為關閉長度觸發。
fn parse_live_trigger(value: &str) -> Result<LiveTriggerSpec, CliError> {
    let mut spec = LiveTriggerSpec::default();
    if value.is_empty() || value == "key" {
        return Ok(spec);
    }
    for part in value.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if part == "boundary" {
            spec.boundary = true;
        } else if part == "idle" {
            spec.idle = true;
        } else if let Some(units) = part.strip_prefix("length:") {
            let units: usize = units
                .parse()
                .map_err(|_| CliError::Usage(format!("--live-trigger 的長度必須是整數：{part}")))?;
            spec.min_units = units;
        } else {
            return Err(CliError::Usage(format!(
                "--live-trigger 只接受 length:N、boundary、idle 或 key，收到 {part:?}"
            )));
        }
    }
    Ok(spec)
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

fn parse_data(args: &[String]) -> Result<Command, CliError> {
    let Some(kind) = args.first().map(String::as_str) else {
        return Err(CliError::Usage(
            "data 需要 manifest、verify、status 或 download".to_string(),
        ));
    };
    let rest = &args[1..];
    let (mut root, mut out, mut version, mut manifest, mut json) = (None, None, None, None, false);
    let mut quiet = false;
    let mut index = 0;
    while index < rest.len() {
        match rest[index].as_str() {
            "--out" => {
                index += 1;
                out = Some(PathBuf::from(value(rest, index, "--out")?));
            }
            "--version" => {
                index += 1;
                version = Some(value(rest, index, "--version")?.to_string());
            }
            "--manifest" => {
                index += 1;
                manifest = Some(PathBuf::from(value(rest, index, "--manifest")?));
            }
            "--json" => json = true,
            "--quiet" => quiet = true,
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("data {kind} 不認得的參數 {other}")));
            }
            other => {
                if root.is_some() {
                    return Err(CliError::Usage("data 只接受一個資料根目錄".to_string()));
                }
                root = Some(PathBuf::from(other));
            }
        }
        index += 1;
    }
    let root = root.ok_or_else(|| CliError::Usage(format!("data {kind} 需要資料根目錄")))?;
    match kind {
        "manifest" => {
            if manifest.is_some() || json || quiet {
                return Err(CliError::Usage(
                    "data manifest 只接受 --out 與 --version".to_string(),
                ));
            }
            Ok(Command::DataManifest { root, out, version })
        }
        "verify" => {
            if out.is_some() || version.is_some() || quiet {
                return Err(CliError::Usage(
                    "data verify 只接受 --manifest 與 --json".to_string(),
                ));
            }
            Ok(Command::DataVerify {
                root,
                manifest,
                json,
            })
        }
        "status" => {
            if out.is_some() || version.is_some() || quiet {
                return Err(CliError::Usage(
                    "data status 只接受 --manifest 與 --json".to_string(),
                ));
            }
            Ok(Command::DataStatus {
                root,
                manifest,
                json,
            })
        }
        "download" => {
            if out.is_some() || version.is_some() || json {
                return Err(CliError::Usage(
                    "data download 只接受 --manifest 與 --quiet".to_string(),
                ));
            }
            Ok(Command::DataDownload {
                root,
                manifest,
                quiet,
            })
        }
        other => Err(CliError::Usage(format!(
            "data 只支援 manifest、verify、status 或 download，收到 {other}"
        ))),
    }
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
        Command::MixedSplit {
            keys,
            testdata,
            data,
            incremental,
            repeat,
            json,
        } => crate::split_cmd::run(
            keys,
            testdata.clone(),
            data.clone(),
            *incremental,
            *repeat,
            *json,
            out,
        ),
        Command::MixedDiff {
            seed,
            count,
            top,
            testdata,
            data,
            json,
            cases,
        } => crate::diff_cmd::run(
            &crate::diff_cmd::DiffOptions {
                seed: *seed,
                count: *count,
                top: *top,
                testdata: testdata.clone(),
                data: data.clone(),
                json: *json,
                cases: cases.clone(),
            },
            out,
        ),
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
            left,
            neural,
            live,
            live_trigger,
            live_scope,
            live_deferred,
            live_inline,
            live_sentence,
            live_idle_ms,
            live_prefix_cache,
            live_cache_keep,
            live_flight,
            live_flight_strict,
            live_budget_ms,
            live_correction_ratio,
            live_keep_tail,
            live_report,
            json,
            items,
        } => crate::eval_cmd::ja(
            crate::eval_cmd::JaOptions {
                testdata: testdata.clone(),
                corpus: corpus.clone(),
                pack: pack.clone(),
                rerank: *rerank,
                gate: rerank.then_some(*gate),
                left: *left,
                neural: *neural,
                live: *live,
                live_trigger: *live_trigger,
                live_scope: *live_scope,
                live_deferred: *live_deferred,
                live_inline: *live_inline,
                live_sentence: *live_sentence,
                live_idle_ms: *live_idle_ms,
                live_prefix_cache: *live_prefix_cache,
                live_cache_keep: *live_cache_keep,
                live_flight: *live_flight,
                live_flight_strict: *live_flight_strict,
                live_budget_ms: *live_budget_ms,
                live_correction_ratio: *live_correction_ratio,
                live_keep_tail: *live_keep_tail,
                live_report: *live_report,
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
            pos,
            select_model,
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
                pos: *pos,
                select_model: *select_model,
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
        Command::DataManifest {
            root,
            out: target,
            version,
        } => crate::manifest_cmd::manifest(root, target.as_deref(), version.as_deref(), out),
        Command::DataVerify {
            root,
            manifest,
            json,
        } => crate::manifest_cmd::verify(root, manifest.as_deref(), *json, out),
        Command::DataStatus {
            root,
            manifest,
            json,
        } => crate::download_cmd::status(root, manifest.as_deref(), *json, out),
        Command::DataDownload {
            root,
            manifest,
            quiet,
        } => crate::download_cmd::download(root, manifest.as_deref(), *quiet, out),
    }
}

/// 命令列說明。
pub fn usage() -> String {
    format!(
        "jc-cli {}（開發工具）\n\n\
         用法：\n  \
         jc-cli eval zh-golden --testdata <dir> [--data <dir>] [--lm <lm.bin>] [--mu <0..1>] [--stats] [--select-model on|off] [--backend candle] [--json]\n  \
         jc-cli eval ja --testdata <dir> --corpus modern-500 [--pack <pack>] \
         [--rerank on|off] [--gate off|<成本差>|cand:<N>] [--backend candle] \
         [--left off|rolling] [--neural auto|quality|off] [--no-live] [--live] [--live-trigger <spec>] \
         [--live-scope whole|clause] [--live-deferred] [--live-budget-ms <ms>] \
         [--live-correction-ratio <0..1>] [--live-keep-tail <n>] [--live-report] [--json] [--items]\n  \
         jc-cli eval fuzzy --testdata <dir> [--json]\n  \
         jc-cli eval mixed --testdata <dir> [--data <dir>] [--json]\n  \
         jc-cli mixed-split --keys <字串>... [--testdata <dir>] [--data <dir>] [--incremental] [--repeat 1] [--json]\n  \
         jc-cli mixed-diff [--seed 1] [--count 20000] [--top 3] [--testdata <dir>] [--data <dir>] [--json] [--cases <file>]\n  \
         jc-cli convert <reading> [--mode en] [--testdata <dir>] [--data <dir>] [--lm <lm.bin>] [--mu <0..1>] [--stats] [--n 5] [--select-model on|off] [--backend candle] [--user-data <dir>] [--pick <n>] [--json]\n  \
         jc-cli tokenize ja [--testdata <dir>] [--vocab <model>] [--text <字串>]... [--diff] [--json]\n  \
         jc-cli bench <file> [--testdata <dir>] [--data <dir>] [--repeat 20] [--json]\n\n\
         jc-cli data manifest <root> [--out <manifest.json>] [--version <標籤>]\n  \
         jc-cli data verify <root> [--manifest <manifest.json>] [--json]\n  \
         jc-cli data status <root> [--manifest <manifest.json>] [--json]\n  \
         jc-cli data download <root> [--manifest <manifest.json>] [--quiet]\n\n\
         --backend 只接受 candle；ORT 已移除，其他值會直接報錯。\n\
         --testdata 預設讀 JC_TESTDATA，再退回 /mnt/work/jc/jc-ime-data/0c-testdata。\n\
         --corpus 是 ja/ 目錄裡的題庫名稱；目錄下每份題庫 json 都會被掃到。\n\
         --data 指向注音二進位資料包（lexicon.bin、lm.bin、manifest.json）；\n\
         指定時注音走真實詞庫並自動載入同目錄的 lm.bin。\n\
         --lm 覆寫 bigram 語言模型；--mu 是 bigram 與 unigram 的內插權重（預設 0.7）。\n\
         --select-model on 需要 zh-select 的 candle 權重；預設 on，可用 off 關閉。\n\
         --neural 選 Jinen 神經轉換：auto（預設，gap < 472.5 且平均候選數 > 5.98）、quality（只看 gap < 460）、off。\n\
         --no-live 回到整句変換；日文預設走逐字変換（ライブ変換），可與 --neural 併用。\n\
         --live-trigger 以逗號組合 length:N、boundary、idle；省略等於每鍵轉換。\n\
         --live-sentence 延遲整句轉換；--live-idle-ms 是停頓門檻（預設 300）。\n\
         --live-deferred 停頓後才補正尾巴；--live-inline 強制即時補正；--live-budget-ms 是預算。\n\
         --live-prefix-cache 用前綴快取（對照）；--live-flight 用 single-flight 非同步推論。\n\
         --live-scope clause 只讓模型補最後一個片段；--live-keep-tail 保留幾個片段不鎖定。\n\
         --live-report 印出每鍵延遲、呼叫次數、補正率與已顯示片段變動率。\n\
         --user-data 指定個人資料目錄；日文學習寫進同一個 personal.json，--pick 記下第幾個候選。\n\
         data manifest 會實際算大小與 SHA-256；下載項不存在就略過，隨附檔缺少則失敗。\n\
         data verify 逐檔檢查存在、大小與 SHA-256；缺少隨附檔或內容不符會回傳非零。\n\
         data status 只查下載項（shipped: false），列出已存在、缺少或雜湊不符。\n\
         data download 從 manifest 的 url 下載缺少的下載項，支援 .part 續傳；\n\
         進度寫 stderr，--quiet 關閉。缺少下載項不影響引擎啟動，只會降級。\n",
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
                left: LeftMode::Off,
                neural: NeuralMode::Auto,
                live: true,
                live_trigger: None,
                live_scope: None,
                live_deferred: false,
                live_inline: false,
                live_sentence: false,
                live_idle_ms: 300,
                live_prefix_cache: false,
                live_cache_keep: 1,
                live_flight: false,
                live_flight_strict: false,
                live_budget_ms: 35,
                live_correction_ratio: 0.5,
                live_keep_tail: 1,
                live_report: false,
                json: false,
                items: false,
            }
        );
    }

    #[test]
    fn parses_ja_neural_switch() {
        for (value, want) in [
            ("auto", NeuralMode::Auto),
            ("quality", NeuralMode::Quality),
            ("off", NeuralMode::Off),
        ] {
            let Command::EvalJa { neural, .. } =
                parse(&args(&["eval", "ja", "--neural", value])).unwrap()
            else {
                panic!("要解析成 eval ja");
            };
            assert_eq!(neural, want, "--neural {value}");
        }
        let Command::EvalJa { neural, .. } = parse(&args(&["eval", "ja"])).unwrap() else {
            panic!("要解析成 eval ja");
        };
        assert_eq!(neural, NeuralMode::Auto, "預設是低呼叫版");
        assert!(parse(&args(&["eval", "ja", "--neural", "maybe"])).is_err());
        assert!(parse(&args(&["eval", "fuzzy", "--neural", "off"])).is_err());
    }

    /// `--live` 只給日文題庫，且可與 `--neural off` 併用。
    #[test]
    fn parses_ja_live_switch() {
        let Command::EvalJa {
            live, neural, json, ..
        } = parse(&args(&["eval", "ja", "--live"])).unwrap()
        else {
            panic!("要解析成 eval ja");
        };
        assert!(live);
        assert_eq!(neural, NeuralMode::Auto, "沒給 --neural 時是低呼叫版");
        assert!(!json);

        let Command::EvalJa { live, neural, .. } =
            parse(&args(&["eval", "ja", "--live", "--neural", "off"])).unwrap()
        else {
            panic!("要解析成 eval ja");
        };
        assert!(live);
        assert_eq!(neural, NeuralMode::Off);

        assert!(parse(&args(&["eval", "fuzzy", "--live"])).is_err());
        assert!(parse(&args(&["eval", "zh-golden", "--live"])).is_err());
        assert!(parse(&args(&["eval", "mixed", "--live"])).is_err());
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
                left: LeftMode::Off,
                neural: NeuralMode::Auto,
                live: true,
                live_trigger: None,
                live_scope: None,
                live_deferred: false,
                live_inline: false,
                live_sentence: false,
                live_idle_ms: 300,
                live_prefix_cache: false,
                live_cache_keep: 1,
                live_flight: false,
                live_flight_strict: false,
                live_budget_ms: 35,
                live_correction_ratio: 0.5,
                live_keep_tail: 1,
                live_report: false,
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
                left: LeftMode::Off,
                neural: NeuralMode::Auto,
                live: true,
                live_trigger: None,
                live_scope: None,
                live_deferred: false,
                live_inline: false,
                live_sentence: false,
                live_idle_ms: 300,
                live_prefix_cache: false,
                live_cache_keep: 1,
                live_flight: false,
                live_flight_strict: false,
                live_budget_ms: 35,
                live_correction_ratio: 0.5,
                live_keep_tail: 1,
                live_report: false,
                json: false,
                items: false,
            }
        );
        assert!(parse(&args(&["eval", "ja", "--rerank", "maybe"])).is_err());
        assert!(parse(&args(&["eval", "fuzzy", "--rerank", "on"])).is_err());
    }

    #[test]
    fn parses_ja_backend_switch() {
        // `--backend candle` 是唯一合法的值；ORT 已移除。
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
                left: LeftMode::Off,
                neural: NeuralMode::Auto,
                live: true,
                live_trigger: None,
                live_scope: None,
                live_deferred: false,
                live_inline: false,
                live_sentence: false,
                live_idle_ms: 300,
                live_prefix_cache: false,
                live_cache_keep: 1,
                live_flight: false,
                live_flight_strict: false,
                live_budget_ms: 35,
                live_correction_ratio: 0.5,
                live_keep_tail: 1,
                live_report: false,
                json: false,
                items: false,
            }
        );
        for value in ["ort", "tensorrt"] {
            let err = parse(&args(&["eval", "ja", "--backend", value])).unwrap_err();
            assert_eq!(
                err.to_string(),
                format!("--backend 只接受 candle，實際為 {value}")
            );
        }
        assert!(parse(&args(&["eval", "fuzzy", "--backend", "candle"])).is_err());
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
                left: LeftMode::Off,
                neural: NeuralMode::Auto,
                live: true,
                live_trigger: None,
                live_scope: None,
                live_deferred: false,
                live_inline: false,
                live_sentence: false,
                live_idle_ms: 300,
                live_prefix_cache: false,
                live_cache_keep: 1,
                live_flight: false,
                live_flight_strict: false,
                live_budget_ms: 35,
                live_correction_ratio: 0.5,
                live_keep_tail: 1,
                live_report: false,
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
                left: LeftMode::Off,
                neural: NeuralMode::Auto,
                live: true,
                live_trigger: None,
                live_scope: None,
                live_deferred: false,
                live_inline: false,
                live_sentence: false,
                live_idle_ms: 300,
                live_prefix_cache: false,
                live_cache_keep: 1,
                live_flight: false,
                live_flight_strict: false,
                live_budget_ms: 35,
                live_correction_ratio: 0.5,
                live_keep_tail: 1,
                live_report: false,
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
                left: LeftMode::Off,
                neural: NeuralMode::Auto,
                live: true,
                live_trigger: None,
                live_scope: None,
                live_deferred: false,
                live_inline: false,
                live_sentence: false,
                live_idle_ms: 300,
                live_prefix_cache: false,
                live_cache_keep: 1,
                live_flight: false,
                live_flight_strict: false,
                live_budget_ms: 35,
                live_correction_ratio: 0.5,
                live_keep_tail: 1,
                live_report: false,
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
                left: LeftMode::Off,
                neural: NeuralMode::Auto,
                live: true,
                live_trigger: None,
                live_scope: None,
                live_deferred: false,
                live_inline: false,
                live_sentence: false,
                live_idle_ms: 300,
                live_prefix_cache: false,
                live_cache_keep: 1,
                live_flight: false,
                live_flight_strict: false,
                live_budget_ms: 35,
                live_correction_ratio: 0.5,
                live_keep_tail: 1,
                live_report: false,
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
                pos: false,
                select_model: true,
                user_data: None,
                mode: None,
                pick: None,
            }
        );
    }

    #[test]
    fn parses_convert_pos_flag() {
        assert_eq!(
            parse(&args(&["convert", "きょう", "--pos"])).unwrap(),
            Command::Convert {
                reading: "きょう".to_string(),
                testdata: None,
                data: None,
                lm: None,
                mu: None,
                stats: false,
                n: 5,
                json: false,
                pos: true,
                select_model: true,
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
                pos: false,
                select_model: true,
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
                pos: false,
                select_model: true,
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
        assert!(parse(&args(&["convert", "わたし", "--backend", "candle"])).is_ok());
        let err = parse(&args(&["convert", "わたし", "--backend", "ort"])).unwrap_err();
        assert_eq!(err.to_string(), "--backend 只接受 candle，實際為 ort");
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
                pos: false,
                select_model: true,
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
    fn parses_data_manifest_and_verify() {
        assert_eq!(
            parse(&args(&[
                "data",
                "manifest",
                "/data/root",
                "--out",
                "/tmp/manifest.json",
                "--version",
                "2026-11-20",
            ]))
            .unwrap(),
            Command::DataManifest {
                root: PathBuf::from("/data/root"),
                out: Some(PathBuf::from("/tmp/manifest.json")),
                version: Some("2026-11-20".to_string()),
            }
        );
        assert_eq!(
            parse(&args(&[
                "data",
                "verify",
                "/data/root",
                "--manifest",
                "/tmp/manifest.json",
                "--json",
            ]))
            .unwrap(),
            Command::DataVerify {
                root: PathBuf::from("/data/root"),
                manifest: Some(PathBuf::from("/tmp/manifest.json")),
                json: true,
            }
        );
        assert!(parse(&args(&["data"])).is_err());
        assert!(parse(&args(&["data", "manifest"])).is_err());
        assert!(parse(&args(&["data", "manifest", "/data", "--json"])).is_err());
        assert!(parse(&args(&["data", "verify", "/data", "--out", "/tmp/x"])).is_err());
    }

    #[test]
    fn parses_data_status_and_download() {
        assert_eq!(
            parse(&args(&[
                "data",
                "status",
                "/data/root",
                "--manifest",
                "/tmp/manifest.json",
                "--json",
            ]))
            .unwrap(),
            Command::DataStatus {
                root: PathBuf::from("/data/root"),
                manifest: Some(PathBuf::from("/tmp/manifest.json")),
                json: true,
            }
        );
        assert_eq!(
            parse(&args(&["data", "download", "/data/root", "--quiet"])).unwrap(),
            Command::DataDownload {
                root: PathBuf::from("/data/root"),
                manifest: None,
                quiet: true,
            }
        );
        assert_eq!(
            parse(&args(&["data", "download", "/data/root"])).unwrap(),
            Command::DataDownload {
                root: PathBuf::from("/data/root"),
                manifest: None,
                quiet: false,
            }
        );
        // 旗標只在對應的子命令有效。
        assert!(parse(&args(&["data", "status", "/data", "--quiet"])).is_err());
        assert!(parse(&args(&["data", "download", "/data", "--json"])).is_err());
        assert!(parse(&args(&["data", "download", "/data", "--out", "/tmp/x"])).is_err());
        assert!(parse(&args(&["data", "nope", "/data"])).is_err());
    }

    #[test]
    fn help_and_version_are_recognized() {
        assert_eq!(parse(&args(&["--help"])).unwrap(), Command::Help);
        assert_eq!(parse(&args(&["-V"])).unwrap(), Command::Version);
        assert_eq!(parse(&[]).unwrap(), Command::Help);
    }
}
