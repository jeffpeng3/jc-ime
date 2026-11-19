//! 引擎行程：在 Linux 開發期用 stdio 或 unix socket。Windows 之後接 named pipe。
//! 這是唯一的長期執行核心，wrapper 只透過 `jc-proto` 與它溝通。
//!
//! 用法：
//!
//! ```text
//! jc-engine --data <dir> [--mode <zh|ja|mixed|en>] [--socket <path>] [--max-frame <bytes>]
//! ```
//!
//! stdio 模式在所有平台都能跑；`--socket` 目前只有 unix 實作，其他平台
//! 會回報 `named pipe 尚未實作`。

use std::io::{self, BufReader, BufWriter};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use jc_core::Mode;
use jc_engine::loader::load_engine_data;
use jc_engine::{engine_label, serve, Engine};
use jc_proto::{DEFAULT_MAX_FRAME, PROTOCOL_VERSION};

/// 啟動方式。
enum Transport {
    /// 標準輸入／輸出，開發期與整合測試用。
    Stdio,
    /// unix domain socket，一次服務一個連線。非 unix 平台尚未支援。
    Socket(PathBuf),
}

struct Options {
    mode: Transport,
    max_frame: usize,
    data: Option<PathBuf>,
    input: Mode,
}

/// 命令列解析結果。
enum Parsed {
    Run(Options),
    Help,
    Version,
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse_args(&args) {
        Ok(Parsed::Run(options)) => options,
        Ok(Parsed::Help) => {
            print_usage();
            return;
        }
        Ok(Parsed::Version) => {
            println!(
                "jc-engine {}（協定 {}）",
                env!("CARGO_PKG_VERSION"),
                PROTOCOL_VERSION
            );
            return;
        }
        Err(message) => {
            eprintln!("jc-engine: {message}");
            print_usage_to_stderr();
            std::process::exit(2);
        }
    };

    if let Err(err) = run(options) {
        eprintln!("jc-engine: {err:#}");
        std::process::exit(1);
    }
}

fn run(options: Options) -> anyhow::Result<()> {
    log::debug!("啟動 {}（協定 {}）", engine_label(), PROTOCOL_VERSION);
    let dir = resolve_data_dir(options.data)?;
    log::debug!("資料根目錄：{}", dir.display());
    let data =
        load_engine_data(&dir).with_context(|| format!("載入資料失敗：{}", dir.display()))?;
    log::info!(
        "載入 {} 個注音詞條{}，日文解碼器就緒",
        data.zh.lexicon.len(),
        if data.zh.lm.as_ref().is_some_and(|lm| lm.bigrams() > 0) {
            "（含 bigram）"
        } else {
            ""
        }
    );
    let mut engine = Engine::with_data(&data, options.input);
    match options.mode {
        Transport::Stdio => {
            let stdin = io::stdin();
            let stdout = io::stdout();
            let mut input = BufReader::new(stdin.lock());
            let mut output = BufWriter::new(stdout.lock());
            serve(&mut engine, &mut input, &mut output, options.max_frame)
                .context("stdio 迴圈失敗")?;
            log::debug!("stdio 結束");
            Ok(())
        }
        Transport::Socket(path) => serve_socket(&mut engine, &path, options.max_frame),
    }
}

/// 在 `path` 上監聽並序列處理連線，收到 `Shutdown` 才回傳。
#[cfg(unix)]
fn serve_socket(engine: &mut Engine, path: &Path, max_frame: usize) -> anyhow::Result<()> {
    prepare_socket_path(path)?;
    let listener = UnixListener::bind(path)
        .with_context(|| format!("綁定 unix socket 失敗：{}", path.display()))?;
    log::info!("jc-engine 監聽 {}", path.display());

    for connection in listener.incoming() {
        let stream = match connection {
            Ok(stream) => stream,
            Err(err) => {
                log::warn!("接受連線失敗：{err}");
                continue;
            }
        };
        log::debug!("接受連線");
        let reader = stream.try_clone().context("複製連線 handle 失敗")?;
        let mut input = BufReader::new(reader);
        let mut output = BufWriter::new(stream);
        // 單一連線的錯誤不影響下一個連線。
        if let Err(err) = serve(engine, &mut input, &mut output, max_frame) {
            log::warn!("連線結束：{err}");
        }
        if engine.is_shutdown() {
            log::debug!("收到 Shutdown，結束行程");
            break;
        }
        engine.reset_session();
    }

    drop(listener);
    if let Err(err) = std::fs::remove_file(path) {
        log::warn!("移除 socket 檔失敗：{err}");
    }
    Ok(())
}

/// 非 unix 平台沒有 unix socket，明確回報錯誤而不是 panic。
#[cfg(not(unix))]
fn serve_socket(_engine: &mut Engine, _path: &Path, _max_frame: usize) -> anyhow::Result<()> {
    bail!("named pipe 尚未實作，這個平台目前只能使用 stdio 模式")
}

/// 清掉上次留下的 socket 檔；有引擎正在監聽時拒絕啟動。
#[cfg(unix)]
fn prepare_socket_path(path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if UnixStream::connect(path).is_ok() {
        bail!("已經有引擎在 {} 監聽", path.display());
    }
    log::debug!("移除殘留的 socket 檔 {}", path.display());
    std::fs::remove_file(path)
        .with_context(|| format!("移除殘留的 socket 檔失敗：{}", path.display()))
}

fn parse_args(args: &[String]) -> Result<Parsed, String> {
    let mut mode = Transport::Stdio;
    let mut socket: Option<PathBuf> = None;
    let mut max_frame = DEFAULT_MAX_FRAME;
    let mut data: Option<PathBuf> = None;
    let mut input = Mode::Zh;
    let mut index = 0;

    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "-h" | "--help" => return Ok(Parsed::Help),
            "-V" | "--version" => return Ok(Parsed::Version),
            "--socket" => {
                let value = value_of(args, &mut index, "--socket")?;
                socket = Some(PathBuf::from(value));
            }
            "--data" => {
                let value = value_of(args, &mut index, "--data")?;
                data = Some(PathBuf::from(value));
            }
            "--mode" => {
                let value = value_of(args, &mut index, "--mode")?;
                input = parse_mode(&value)?;
            }
            "--max-frame" => {
                let value = value_of(args, &mut index, "--max-frame")?;
                let parsed: usize = value
                    .parse()
                    .map_err(|_| format!("--max-frame 需要正整數，收到 {value}"))?;
                if parsed == 0 || parsed > u32::MAX as usize {
                    return Err(format!("--max-frame 必須介於 1 與 {} 之間", u32::MAX));
                }
                max_frame = parsed;
            }
            other => return Err(format!("不認得的參數 {other}")),
        }
        index += 1;
    }

    if let Some(path) = socket {
        mode = Transport::Socket(path);
    }
    Ok(Parsed::Run(Options {
        mode,
        max_frame,
        data,
        input,
    }))
}

/// 解析 `--mode`；只接受四種輸入模式。
fn parse_mode(value: &str) -> Result<Mode, String> {
    match value {
        "zh" => Ok(Mode::Zh),
        "ja" => Ok(Mode::Ja),
        "mixed" => Ok(Mode::Mixed),
        "en" => Ok(Mode::En),
        other => Err(format!("--mode 只接受 zh、ja、mixed、en，收到 {other}")),
    }
}

/// `--data` 未指定時退回 `JC_DATA`；兩者都沒有就拒絕啟動。
fn resolve_data_dir(explicit: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(dir) = explicit {
        return Ok(dir);
    }
    if let Some(value) = std::env::var_os("JC_DATA") {
        if !value.is_empty() {
            return Ok(PathBuf::from(value));
        }
    }
    bail!("沒有指定 --data，且 JC_DATA 未設定；請用 --data <dir> 指向資料根目錄")
}

fn value_of(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{flag} 需要一個值"))
}

/// `--socket` 的說明文字依平台而不同。
#[cfg(unix)]
const SOCKET_HELP: &str = "  --socket <path>     改用 unix domain socket，一次服務一個連線\n";
/// 非 unix 平台沒有 unix socket 實作。
#[cfg(not(unix))]
const SOCKET_HELP: &str = "  --socket <path>     named pipe 尚未實作，目前只支援 stdio\n";

fn print_usage() {
    println!(
        "jc-engine {}（協定 {}）\n\n\
         用法：\n  jc-engine --data <dir> [--mode <zh|ja|mixed|en>] [--socket <path>] [--max-frame <bytes>]\n\n\
         選項：\n\
         \x20 --data <dir>        資料根目錄；未指定時讀 JC_DATA\n\
         \x20 --mode <mode>       輸入模式：zh（預設）、ja、mixed、en\n\
        {SOCKET_HELP}\
         \x20 --max-frame <bytes> 單一訊框上限，預設 {DEFAULT_MAX_FRAME}\n\
         \x20 -h, --help          顯示說明\n\
         \x20 -V, --version       顯示版本",
        env!("CARGO_PKG_VERSION"),
        PROTOCOL_VERSION
    );
}

fn print_usage_to_stderr() {
    eprintln!(
        "用法：jc-engine --data <dir> [--mode <zh|ja|mixed|en>] [--socket <path>] [--max-frame <bytes>]"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(words: &[&str]) -> Result<Parsed, String> {
        let args: Vec<String> = words.iter().map(|word| word.to_string()).collect();
        parse_args(&args)
    }

    #[test]
    fn defaults_to_stdio_with_one_mib_frames() {
        let Ok(Parsed::Run(options)) = parse(&[]) else {
            panic!("預設應該直接執行");
        };
        assert!(matches!(options.mode, Transport::Stdio));
        assert_eq!(options.max_frame, 1 << 20);
        assert!(options.data.is_none(), "資料目錄由 JC_DATA 或 --data 決定");
        assert_eq!(options.input, Mode::Zh);
    }

    #[test]
    fn data_and_input_mode_are_parsed() {
        let Ok(Parsed::Run(options)) = parse(&["--data", "/data/root", "--mode", "ja"]) else {
            panic!("應該解析成功");
        };
        assert_eq!(options.data, Some(PathBuf::from("/data/root")));
        assert_eq!(options.input, Mode::Ja);
    }

    #[test]
    fn bad_input_mode_fails() {
        let Err(err) = parse(&["--mode", "kana"]) else {
            panic!("不認得的模式應該失敗");
        };
        assert!(
            err.contains("zh、ja、mixed、en"),
            "錯誤應列出可用模式：{err}"
        );
    }

    #[test]
    fn explicit_data_dir_wins_over_environment() {
        let dir = resolve_data_dir(Some(PathBuf::from("/explicit"))).unwrap();
        assert_eq!(dir, PathBuf::from("/explicit"));
    }

    #[test]
    fn socket_and_max_frame_are_parsed() {
        let Ok(Parsed::Run(options)) = parse(&["--max-frame", "4096", "--socket", "/tmp/x.sock"])
        else {
            panic!("應該解析成功");
        };
        assert_eq!(options.max_frame, 4096);
        match options.mode {
            Transport::Socket(path) => assert_eq!(path, PathBuf::from("/tmp/x.sock")),
            Transport::Stdio => panic!("應該是 socket 模式"),
        }
    }

    #[test]
    fn bad_arguments_fail() {
        assert!(parse(&["--max-frame", "0"]).is_err());
        assert!(parse(&["--max-frame"]).is_err());
        assert!(parse(&["--nope"]).is_err());
        assert!(parse(&["--socket"]).is_err());
    }

    /// 非 unix 平台的 `--socket` 必須回傳明確錯誤，不能 panic。
    #[cfg(not(unix))]
    #[test]
    fn socket_mode_reports_named_pipe_unimplemented() {
        let mut engine = Engine::new();
        let err = serve_socket(&mut engine, Path::new("engine.sock"), DEFAULT_MAX_FRAME)
            .expect_err("非 unix 平台不該啟動 socket");
        assert!(
            err.to_string().contains("named pipe 尚未實作"),
            "錯誤訊息應說明 named pipe 尚未實作：{err}"
        );
    }
}
