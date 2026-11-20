//! 引擎行程：stdio、unix socket 或 Windows named pipe。
//! 這是唯一的長期執行核心，wrapper 只透過 `jc-proto` 與它溝通。
//!
//! 用法：
//!
//! ```text
//! jc-engine --data <dir> [--settings <path>] [--mode <zh|ja|mixed|en>]
//!           [--neural <on|off>] [--socket <path>] [--max-frame <bytes>]
//! jc-engine --supervise [--socket <name>] [--supervisor-log <path>]
//! ```
//!
//! 啟動時讀 `settings.json`（`--settings` 或資料根目錄），轉成
//! `SessionConfig`；CLI 參數優先於檔案。設定檔缺漏或壞掉只記警告，不讓引擎
//! 啟動失敗。`--print-settings` 印出生效後的設定後直接結束。
//!
//! `--supervise` 啟動常駐監督者：用同一支執行檔啟動子行程
//! `jc-engine --socket <name>`，子行程結束後依退避曲線重啟，並用
//! `Local\` 具名 mutex 保證每個使用者只有一個監督者（見 [`jc_engine::supervise`]）。
//!
//! `--allow-empty-data` 跳過資料載入，只用空 session，供協定測試與除錯；
//! 正式啟動仍要求 `--data` 或 `JC_DATA`。
//!
//! stdio 模式在所有平台都能跑；`--socket` 在 unix 是 unix domain socket，
//! 在 Windows 是本機 named pipe（見 `pipe_windows`），其他平台回報尚未實作。

use std::ffi::OsString;
use std::io::{self, BufReader, BufWriter};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use jc_core::session::{Session, SessionConfig};
use jc_core::Mode;
use jc_engine::config;
use jc_engine::loader::load_engine_data;
use jc_engine::supervise::{self, SupervisorConfig};
use jc_engine::{engine_label, serve, Engine};
use jc_proto::{DEFAULT_MAX_FRAME, PROTOCOL_VERSION};

#[cfg(windows)]
mod pipe_windows;

/// 啟動方式。
enum Transport {
    /// 標準輸入／輸出，開發期與整合測試用。
    Stdio,
    /// unix domain socket（unix）或 named pipe（Windows），一次服務一個連線。
    Socket(PathBuf),
}

struct Options {
    mode: Transport,
    max_frame: usize,
    data: Option<PathBuf>,
    /// `--settings` 指定的設定檔；沒給時用資料根目錄的 `settings.json`。
    settings: Option<PathBuf>,
    /// `--mode`；`None` 表示由設定檔決定。
    input: Option<Mode>,
    /// `--neural`；`None` 表示由設定檔的 `selection` 決定。
    neural: Option<bool>,
    /// 只印出生效後的設定後結束。
    print_settings: bool,
    /// 跳過資料載入，只用空 session；僅供測試與協定除錯。
    allow_empty_data: bool,
    /// `--supervise`：啟動常駐監督者，而不是自己服務請求。
    supervise: bool,
    /// `--supervisor-log`：監督者 log 路徑；未指定時用平台預設位置。
    supervisor_log: Option<PathBuf>,
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
    if options.supervise {
        return run_supervisor(&options);
    }
    let goos = config::platform_goos();
    let data_dir = data_dir_from_env(options.data.clone());
    let loaded = config::load(options.settings.as_deref(), data_dir.as_deref(), goos);
    for warning in &loaded.warnings {
        log::warn!("設定：{warning}");
    }
    let overrides = config::Overrides {
        mode: options.input,
        neural: options.neural,
    };
    let effective = config::effective(&loaded.settings, &overrides, goos);
    if options.print_settings {
        println!("{}", loaded.print_json(&effective)?);
        return Ok(());
    }
    log::info!(
        "設定：模式 {}、選字 {}、候選 {}、混打 {}、Esc 清除 {}、空白開候選 {}、前文脈 {}",
        config::mode_str(effective.mode),
        effective.selection,
        effective.max_candidates,
        effective.mixed_input,
        effective.esc_clear,
        effective.space_selects,
        effective.document_context
    );
    let mut engine = if options.allow_empty_data {
        log::info!("--allow-empty-data：不載入資料，使用空 session（僅測試用）");
        let base = SessionConfig::default();
        Engine::with_session(Session::new(config::session_config(base, &effective)))
    } else {
        let dir = data_dir.ok_or_else(missing_data_dir)?;
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
        let base = data.session_config(effective.mode);
        Engine::with_session(Session::new(config::session_config(base, &effective)))
    };
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

/// 啟動監督者，讓引擎常駐並在子行程結束後自動重啟。
fn run_supervisor(options: &Options) -> anyhow::Result<()> {
    if options.print_settings {
        bail!("--supervise 不能與 --print-settings 併用");
    }
    let socket = match &options.mode {
        Transport::Socket(path) => path.clone(),
        Transport::Stdio => default_supervisor_socket(),
    };
    let log_path = options
        .supervisor_log
        .clone()
        .unwrap_or_else(default_supervisor_log);
    let program = std::env::current_exe().context("取得目前的執行檔路徑失敗")?;
    let config = SupervisorConfig::new(program, child_args(options, &socket), socket, log_path);

    match supervise::run(&config) {
        Ok(()) => Ok(()),
        // 第二個實例（例如登入時 Run key 觸發兩次）不算錯誤。
        Err(supervise::Error::AlreadyRunning { name }) => {
            log::info!("監督者已在執行（{name}），這次啟動直接結束");
            Ok(())
        }
        Err(err) => Err(err.into()),
    }
}

/// 把目前的參數原封不動轉給子行程，並強制使用 socket 模式。
fn child_args(options: &Options, socket: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    if let Some(data) = &options.data {
        args.push("--data".into());
        args.push(data.as_os_str().to_os_string());
    }
    if let Some(settings) = &options.settings {
        args.push("--settings".into());
        args.push(settings.as_os_str().to_os_string());
    }
    if let Some(mode) = options.input {
        args.push("--mode".into());
        args.push(config::mode_str(mode).into());
    }
    if let Some(neural) = options.neural {
        args.push("--neural".into());
        args.push(if neural { "on" } else { "off" }.into());
    }
    if options.allow_empty_data {
        args.push("--allow-empty-data".into());
    }
    args.push("--max-frame".into());
    args.push(options.max_frame.to_string().into());
    args.push("--socket".into());
    args.push(socket.as_os_str().to_os_string());
    args
}

/// 監督模式的預設 socket。Windows 用與 TSF 共用的 pipe 名稱規則。
///
/// 名稱由 `jc-windows` 解析：`JC_IME_PIPE` > `HKCU\Software\jc-ime\EnginePipe` >
/// `jc-ime-<SID 尾段>`；取不到 SID 時退回 `jc-ime` 並記一次警告。
#[cfg(windows)]
fn default_supervisor_socket() -> PathBuf {
    let resolved = jc_windows::resolve_pipe_name();
    if let Some(warning) = &resolved.warning {
        log::warn!("{warning}");
    }
    PathBuf::from(resolved.name)
}

/// 監督模式的預設 socket；優先放 `XDG_RUNTIME_DIR`。
#[cfg(unix)]
fn default_supervisor_socket() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    dir.join("jc-ime-engine.sock")
}

/// 其他平台沒有 socket 傳輸，仍回傳一個名稱讓錯誤早點出現。
#[cfg(not(any(unix, windows)))]
fn default_supervisor_socket() -> PathBuf {
    PathBuf::from("jc-ime-engine.sock")
}

/// 監督者 log 的預設位置：`%LOCALAPPDATA%\jc-ime\supervisor.log`，其他平台用
/// `XDG_STATE_HOME` 或 `~/.local/state`。
fn default_supervisor_log() -> PathBuf {
    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(dir).join("jc-ime").join("supervisor.log");
    }
    if let Some(dir) = std::env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(dir).join("jc-ime").join("supervisor.log");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("jc-ime")
            .join("supervisor.log");
    }
    std::env::temp_dir().join("jc-ime").join("supervisor.log")
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

/// Windows：`path` 是 pipe 名稱或路徑，交給 named pipe 分支。
#[cfg(windows)]
fn serve_socket(engine: &mut Engine, path: &Path, max_frame: usize) -> anyhow::Result<()> {
    pipe_windows::serve_pipe(engine, path, max_frame)
}

/// 既不是 unix 也不是 Windows 的平台沒有 socket 傳輸，明確回報錯誤而不是 panic。
#[cfg(not(any(unix, windows)))]
fn serve_socket(_engine: &mut Engine, _path: &Path, _max_frame: usize) -> anyhow::Result<()> {
    bail!("這個平台不支援 --socket，只能使用 stdio 模式")
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
    let mut settings: Option<PathBuf> = None;
    let mut input: Option<Mode> = None;
    let mut neural: Option<bool> = None;
    let mut print_settings = false;
    let mut allow_empty_data = false;
    let mut supervise = false;
    let mut supervisor_log: Option<PathBuf> = None;
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
            "--settings" => {
                let value = value_of(args, &mut index, "--settings")?;
                settings = Some(PathBuf::from(value));
            }
            "--print-settings" => print_settings = true,
            "--allow-empty-data" => allow_empty_data = true,
            "--supervise" => supervise = true,
            "--supervisor-log" => {
                let value = value_of(args, &mut index, "--supervisor-log")?;
                supervisor_log = Some(PathBuf::from(value));
            }
            "--mode" => {
                let value = value_of(args, &mut index, "--mode")?;
                input = Some(parse_mode(&value)?);
            }
            "--neural" => {
                let value = value_of(args, &mut index, "--neural")?;
                neural = Some(match value.as_str() {
                    "on" => true,
                    "off" => false,
                    other => return Err(format!("--neural 只接受 on 或 off，收到 {other}")),
                });
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
        settings,
        input,
        neural,
        print_settings,
        allow_empty_data,
        supervise,
        supervisor_log,
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

/// `--data` 未指定時退回 `JC_DATA`；兩者都沒有回傳 `None`。
fn data_dir_from_env(explicit: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(dir) = explicit {
        return Some(dir);
    }
    let value = std::env::var_os("JC_DATA")?;
    if value.is_empty() {
        None
    } else {
        Some(PathBuf::from(value))
    }
}

/// 沒有資料根目錄時的啟動錯誤。
fn missing_data_dir() -> anyhow::Error {
    anyhow::anyhow!("沒有指定 --data，且 JC_DATA 未設定；請用 --data <dir> 指向資料根目錄")
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
/// Windows 的 `--socket` 是 named pipe 名稱或路徑。
#[cfg(windows)]
const SOCKET_HELP: &str =
    "  --socket <name>     改用本機 named pipe（\\\\.\\pipe\\<name>），一次服務一個連線\n";
/// 其他平台沒有 socket 傳輸。
#[cfg(not(any(unix, windows)))]
const SOCKET_HELP: &str = "  --socket <path>     這個平台不支援，目前只支援 stdio\n";

fn print_usage() {
    println!(
        "jc-engine {}（協定 {}）\n\n\
         用法：\n  jc-engine --data <dir> [--settings <path>] [--mode <zh|ja|mixed|en>]\n\
         \x20           [--neural <on|off>] [--socket <path>] [--max-frame <bytes>]\n\
         \x20 jc-engine --supervise [--socket <name>] [--supervisor-log <path>]\n\n\
         選項：\n\
         \x20 --data <dir>        資料根目錄；未指定時讀 JC_DATA\n\
         \x20 --settings <path>   設定檔；未指定時讀資料根目錄的 settings.json\n\
         \x20 --print-settings    印出生效後的設定（JSON）後結束\n\
         \x20 --allow-empty-data  不載入資料，只用空 session（測試／除錯用）\n\
         \x20 --supervise         常駐監督子行程，結束後依退避曲線重啟\n\
         \x20 --supervisor-log <path>  監督者 log 路徑（預設 %LOCALAPPDATA%\\jc-ime\\supervisor.log）\n\
         \x20 --mode <mode>       輸入模式：zh、ja、mixed、en；預設由設定檔決定\n\
         \x20 --neural <on|off>   日文神經轉換；預設由設定檔的 selection 決定\n\
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
        "用法：jc-engine --data <dir> [--settings <path>] [--mode <zh|ja|mixed|en>] [--neural <on|off>] [--socket <path>] [--max-frame <bytes>]\n\
         \x20     jc-engine --supervise [--socket <name>] [--supervisor-log <path>]"
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
        assert!(options.settings.is_none(), "設定檔由資料根目錄決定");
        assert_eq!(options.input, None, "模式預設由設定檔決定");
        assert_eq!(options.neural, None, "神經轉換預設由設定檔決定");
        assert!(!options.print_settings);
        assert!(!options.allow_empty_data, "預設不可跳過資料載入");
    }

    #[test]
    fn allow_empty_data_is_parsed() {
        let Ok(Parsed::Run(options)) = parse(&["--allow-empty-data"]) else {
            panic!("應該解析成功");
        };
        assert!(options.allow_empty_data);
    }

    #[test]
    fn data_and_input_mode_are_parsed() {
        let Ok(Parsed::Run(options)) = parse(&["--data", "/data/root", "--mode", "ja"]) else {
            panic!("應該解析成功");
        };
        assert_eq!(options.data, Some(PathBuf::from("/data/root")));
        assert_eq!(options.input, Some(Mode::Ja));
    }

    #[test]
    fn settings_neural_and_print_are_parsed() {
        let Ok(Parsed::Run(options)) = parse(&[
            "--settings",
            "/cfg/settings.json",
            "--neural",
            "off",
            "--print-settings",
            "--mode",
            "mixed",
        ]) else {
            panic!("應該解析成功");
        };
        assert_eq!(options.settings, Some(PathBuf::from("/cfg/settings.json")));
        assert_eq!(options.neural, Some(false));
        assert!(options.print_settings);
        assert_eq!(options.input, Some(Mode::Mixed));
    }

    #[test]
    fn bad_neural_value_fails() {
        let Err(err) = parse(&["--neural", "maybe"]) else {
            panic!("不認得的值應該失敗");
        };
        assert!(err.contains("on 或 off"), "錯誤應列出可用值：{err}");
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
        let dir = data_dir_from_env(Some(PathBuf::from("/explicit"))).unwrap();
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
        assert!(parse(&["--settings"]).is_err());
        assert!(parse(&["--supervisor-log"]).is_err());
    }

    #[test]
    fn supervise_and_log_path_are_parsed() {
        let Ok(Parsed::Run(options)) = parse(&["--supervise", "--supervisor-log", "/tmp/s.log"])
        else {
            panic!("應該解析成功");
        };
        assert!(options.supervise);
        assert_eq!(options.supervisor_log, Some(PathBuf::from("/tmp/s.log")));
    }

    #[test]
    fn supervise_forwards_child_options() {
        let Ok(Parsed::Run(options)) = parse(&[
            "--supervise",
            "--data",
            "/data",
            "--mode",
            "ja",
            "--neural",
            "off",
            "--allow-empty-data",
            "--max-frame",
            "4096",
        ]) else {
            panic!("應該解析成功");
        };
        let args = child_args(&options, Path::new("/tmp/engine.sock"));
        let args: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "--data",
                "/data",
                "--mode",
                "ja",
                "--neural",
                "off",
                "--allow-empty-data",
                "--max-frame",
                "4096",
                "--socket",
                "/tmp/engine.sock",
            ]
        );
    }

    #[test]
    fn supervise_rejects_print_settings() {
        let Ok(Parsed::Run(options)) = parse(&["--supervise", "--print-settings"]) else {
            panic!("應該解析成功");
        };
        let err = run_supervisor(&options).expect_err("不可併用");
        assert!(
            err.to_string().contains("--print-settings"),
            "錯誤應說明衝突：{err}"
        );
    }

    /// 既不是 unix 也不是 Windows 的平台的 `--socket` 必須回傳明確錯誤，不能 panic。
    #[cfg(not(any(unix, windows)))]
    #[test]
    fn socket_mode_reports_unsupported_platform() {
        let mut engine = Engine::new();
        let err = serve_socket(&mut engine, Path::new("engine.sock"), DEFAULT_MAX_FRAME)
            .expect_err("這個平台不該啟動 socket");
        assert!(
            err.to_string().contains("不支援 --socket"),
            "錯誤訊息應說明平台不支援：{err}"
        );
    }
}
