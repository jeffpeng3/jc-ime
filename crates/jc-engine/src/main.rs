//! 引擎行程：在 Linux 開發期用 stdio 或 unix socket。Windows 之後接 named pipe。
//! 這是唯一的長期執行核心，wrapper 只透過 `jc-proto` 與它溝通。
//!
//! 用法：
//!
//! ```text
//! jc-engine [--socket <path>] [--max-frame <bytes>]
//! ```

use std::io::{self, BufReader, BufWriter};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use jc_engine::{engine_label, serve, Engine};
use jc_proto::{DEFAULT_MAX_FRAME, PROTOCOL_VERSION};

/// 啟動方式。
enum Mode {
    /// 標準輸入／輸出，開發期與整合測試用。
    Stdio,
    /// unix domain socket，一次服務一個連線。
    Socket(PathBuf),
}

struct Options {
    mode: Mode,
    max_frame: usize,
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
    let mut engine = Engine::new();
    match options.mode {
        Mode::Stdio => {
            let stdin = io::stdin();
            let stdout = io::stdout();
            let mut input = BufReader::new(stdin.lock());
            let mut output = BufWriter::new(stdout.lock());
            serve(&mut engine, &mut input, &mut output, options.max_frame)
                .context("stdio 迴圈失敗")?;
            log::debug!("stdio 結束");
            Ok(())
        }
        Mode::Socket(path) => serve_socket(&mut engine, &path, options.max_frame),
    }
}

/// 在 `path` 上監聽並序列處理連線，收到 `Shutdown` 才回傳。
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

/// 清掉上次留下的 socket 檔；有引擎正在監聽時拒絕啟動。
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
    let mut mode = Mode::Stdio;
    let mut socket: Option<PathBuf> = None;
    let mut max_frame = DEFAULT_MAX_FRAME;
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
        mode = Mode::Socket(path);
    }
    Ok(Parsed::Run(Options { mode, max_frame }))
}

fn value_of(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{flag} 需要一個值"))
}

fn print_usage() {
    println!(
        "jc-engine {}（協定 {}）\n\n\
         用法：\n  jc-engine [--socket <path>] [--max-frame <bytes>]\n\n\
         選項：\n\
         \x20 --socket <path>     改用 unix domain socket，一次服務一個連線\n\
         \x20 --max-frame <bytes> 單一訊框上限，預設 {DEFAULT_MAX_FRAME}\n\
         \x20 -h, --help          顯示說明\n\
         \x20 -V, --version       顯示版本",
        env!("CARGO_PKG_VERSION"),
        PROTOCOL_VERSION
    );
}

fn print_usage_to_stderr() {
    eprintln!("用法：jc-engine [--socket <path>] [--max-frame <bytes>]");
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
        assert!(matches!(options.mode, Mode::Stdio));
        assert_eq!(options.max_frame, 1 << 20);
    }

    #[test]
    fn socket_and_max_frame_are_parsed() {
        let Ok(Parsed::Run(options)) = parse(&["--max-frame", "4096", "--socket", "/tmp/x.sock"])
        else {
            panic!("應該解析成功");
        };
        assert_eq!(options.max_frame, 4096);
        match options.mode {
            Mode::Socket(path) => assert_eq!(path, PathBuf::from("/tmp/x.sock")),
            Mode::Stdio => panic!("應該是 socket 模式"),
        }
    }

    #[test]
    fn bad_arguments_fail() {
        assert!(parse(&["--max-frame", "0"]).is_err());
        assert!(parse(&["--max-frame"]).is_err());
        assert!(parse(&["--nope"]).is_err());
        assert!(parse(&["--socket"]).is_err());
    }
}
