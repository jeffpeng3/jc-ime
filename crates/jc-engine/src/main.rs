//! 引擎行程：stdio 或跨平台 local socket（Windows named pipe／Unix domain socket）。
//! 這是唯一的長期執行核心，wrapper 只透過 `jc-proto` 與它溝通。
//!
//! 用法：
//!
//! ```text
//! jc-engine --data <dir> [--settings <path>] [--mode <zh|ja|mixed|en>]
//!           [--neural <on|off>] [--socket <path>] [--max-frame <bytes>]
//!           [--personal-prefix <name>]
//! jc-engine --supervise [--socket <name>] [--supervisor-log <path>]
//!           [--instance <name>]
//! ```
//!
//! Windows 正式啟動與 autostart 都要用 `--supervise`，且**不帶** `--socket`：
//! 直接伺服模式沒給 `--socket` 會走 stdio，完全不開 pipe；監督模式的
//! `run_supervisor()` → `default_supervisor_socket()` 才會呼叫
//! `jc_windows::resolve_pipe_name()`，從 SID 尾段推出 `jc-ime-<rid>`，與
//! `jc-tsf` 一致。顯式 `--socket` 會跳過 SID 推導；要手動指定，必須同時把
//! `JC_IME_PIPE` 或 `HKCU\Software\jc-ime\EnginePipe` 設成同一個值，讓兩端一致。
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
//! stdio 模式在所有平台都能跑。`--socket` 在 Windows 直接走 Win32
//! `CreateNamedPipeW`（見 [`pipe_listener`]），在 unix 走 interprocess 的 unix
//! domain socket；兩端的名稱解析共用 `jc_windows`。其他平台回報尚未實作。

#[cfg(windows)]
mod pipe_listener;

use std::ffi::OsString;
#[cfg(unix)]
use std::io::Write;
use std::io::{self, BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(unix)]
use std::time::Duration;

use anyhow::{bail, Context};
#[cfg(not(windows))]
use interprocess::local_socket::traits::Listener as _;
#[cfg(unix)]
use interprocess::local_socket::ListenerOptions;
#[cfg(unix)]
use interprocess::local_socket::{traits::Stream as _, ListenerNonblockingMode};
use jc_core::session::{Session, SessionConfig};
use jc_core::Mode;
use jc_engine::config;
use jc_engine::loader::load_engine_data;
use jc_engine::supervise::{self, SupervisorConfig, SUPERVISOR_MUTEX_NAME};
use jc_engine::{engine_label, serve, Engine, SessionFactory, DEFAULT_PERSONAL_PREFIX};
use jc_proto::{DEFAULT_MAX_FRAME, PROTOCOL_VERSION};

/// 啟動方式。
enum Transport {
    /// 標準輸入／輸出，開發期與整合測試用。
    Stdio,
    /// local socket：unix domain socket 或 named pipe，一次服務一個連線。
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
    /// `--personal-prefix`：個人學習檔名前綴；平行建置讓新版用不同值。
    personal_prefix: String,
    /// `--personal-dir`：個人學習資料目錄。未指定時用 `--settings` 的目錄，
    /// 再退回使用者資料目錄；**不能**放在資料根目錄，那裡是共用的唯讀資料。
    personal_dir: Option<PathBuf>,
    /// `--instance`：監督者實例名；平行建置讓新版有獨立的鎖與 log。
    instance: String,
}

/// 已發行的預設實例名。非預設值才會改鎖名與 log 檔名。
const DEFAULT_INSTANCE: &str = "main";

/// 監督者的鎖名；預設實例保持已發行值，其他實例加後綴。
fn supervisor_lock_name(instance: &str) -> String {
    if instance == DEFAULT_INSTANCE {
        SUPERVISOR_MUTEX_NAME.to_string()
    } else {
        format!("{SUPERVISOR_MUTEX_NAME}-{instance}")
    }
}

/// 命令列解析結果。
enum Parsed {
    Run(Box<Options>),
    Help,
    Version,
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse_args(&args) {
        Ok(Parsed::Run(options)) => *options,
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

/// 外殼需要的能力旗標。
///
/// `output_simplified_live` 只在真的載入 OpenCC tw2s 轉換器時出現；缺席時
/// 不寫入 `false`，讓 jc-tsf 維持預設，不顯示一顆按了沒反應的開關。
/// `has_bigram` 一律回報，安裝 smoke 用它確認載入的是 bigram 資料包。
fn build_shell_caps(
    toggle_key: &str,
    has_zhconv: bool,
    has_bigram: bool,
) -> std::collections::BTreeMap<String, bool> {
    let mut caps = std::collections::BTreeMap::new();
    caps.insert(
        "left_shift".to_string(),
        toggle_key == "both" || toggle_key == "left",
    );
    caps.insert(
        "right_shift".to_string(),
        toggle_key == "both" || toggle_key == "right",
    );
    caps.insert("has_bigram".to_string(), has_bigram);
    // 日文能力：資料包一律含日文解碼器。外殼據此送 F6–F10／Convert，
    // 不必先有 legacy 的 `japanese` 輸入模式。
    caps.insert("japanese".to_string(), true);
    caps.insert("convert_keys".to_string(), true);
    caps.insert("clause_display".to_string(), true);
    caps.insert("live_conversion".to_string(), true);
    if has_zhconv {
        caps.insert("output_simplified_live".to_string(), true);
    }
    caps
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
    // T4：每條連線在 worker thread 裡用自己的 factory 建 Session，避免在
    // thread 間移動非 Send 的 Session／SessionConfig。
    // 個人學習資料要寫在使用者自己的目錄。以前這裡直接用資料根目錄
    // （安裝後是 `C:\ProgramData\jc-ime\data`），一般使用者沒有寫入權，
    // 引擎每次載入都回「存取被拒 (os error 5)」，連帶讓請求失敗、連線被關掉。
    let personal_dir = if options.allow_empty_data {
        None
    } else {
        resolve_personal_dir(&options)
    };
    let data_root = if options.allow_empty_data {
        None
    } else {
        Some(data_dir.clone().ok_or_else(missing_data_dir)?)
    };
    // 外殼（jc-tsf）只認 health 的 `info[name] == "true"`。少了這些旗標它會退回
    // 保守預設：左 Shift 不切換（預設 left_shift=false）、狀態列的「輸出簡體字」
    // 按了沒反應（simplified_live=false）。`effective` 之後會被 factory 的
    // closure 移走，所以先在這裡算好。
    let mut shell_caps = build_shell_caps(&effective.toggle_key, false, false);
    let mut default_simplified = false;
    let start_mode = effective.start_mode.clone();
    let factory: SessionFactory = if let Some(dir) = &data_root {
        log::debug!("資料根目錄：{}", dir.display());
        let data = Arc::new(
            load_engine_data(dir).with_context(|| format!("載入資料失敗：{}", dir.display()))?,
        );
        let has_bigram = data.zh.lm.as_ref().is_some_and(|lm| lm.bigrams() > 0);
        // 只有真的載入 tw2s 轉換器才宣告能力；否則狀態列會出現一顆假開關。
        shell_caps = build_shell_caps(&effective.toggle_key, data.zhconv.is_some(), has_bigram);
        if data.zhconv.is_some() {
            default_simplified = effective.output_simplified;
        }
        log::info!(
            "載入 {} 個注音詞條{}，日文解碼器就緒",
            data.zh.lexicon.len(),
            if has_bigram { "（含 bigram）" } else { "" }
        );
        // 資料載入完成、開始接受連線之前先跑一次第一鍵會走的路徑。冷啟動的
        // 66～73ms 來自第一次解碼、第一次 zh-select 推論與 mmap 分頁；在
        // 這裡先付掉，使用者按下的第一個鍵才不會突然變慢。這段必須在
        // listener 建立前完成：外殼的 health／hello 有 1 秒逾時，預熱本身
        // 的量測目標是 300ms 以內，遠低於逾期門檻。
        let base_config = data.session_config(effective.mode);
        let warm_config = config::session_config(base_config.clone(), &effective);
        let prewarm = data.prewarm(&warm_config);
        log::info!("{}", prewarm.log_line());
        // session 建立在 Engine::open_session 裡會從全行程共用的 PersonalStore
        // 灌入快照，因此工廠本身不需要碰個人資料；它只負責建立空白 session。
        // 熱重載後新開的 session 會由 open_session 再套一次最新設定。
        let effective_for_factory = effective.clone();
        let mode = effective.mode;
        Arc::new(move || {
            let base = data.session_config(mode);
            Session::new(config::session_config(base, &effective_for_factory))
        })
    } else {
        log::info!("--allow-empty-data：不載入資料，使用空 session（僅測試用）");
        let effective_for_factory = effective.clone();
        Arc::new(move || {
            Session::new(config::session_config(
                SessionConfig::default(),
                &effective_for_factory,
            ))
        })
    };
    if let Some(path) = &loaded.path {
        log::info!("設定檔：{}", path.display());
    }
    let mut engine = Engine::with_factory(factory, personal_dir);
    engine.set_settings(effective, loaded.path.clone(), goos, overrides);
    engine.set_settings_stamp(loaded.stamp);
    engine.watch_settings();
    engine.set_extra_caps(shell_caps);
    engine.set_start_mode(start_mode);
    engine.set_default_output_simplified(default_simplified);
    if options.personal_prefix != DEFAULT_PERSONAL_PREFIX {
        log::info!("個人資料檔名前綴：{}", options.personal_prefix);
    }
    engine.set_personal_prefix(options.personal_prefix.clone());
    engine.load_personal();
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
        Transport::Socket(path) => serve_socket(&engine, &path, options.max_frame),
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
    let instance = options.instance.clone();
    let log_path = options
        .supervisor_log
        .clone()
        .unwrap_or_else(|| default_supervisor_log_for(&instance));
    let program = std::env::current_exe().context("取得目前的執行檔路徑失敗")?;
    let mut config = SupervisorConfig::new(program, child_args(options, &socket), socket, log_path);
    // 平行建置：新舊引擎的監督者不能搶同一把鎖，否則第二個永遠回 AlreadyRunning。
    config.lock_name = supervisor_lock_name(&instance);

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

/// 個人學習資料目錄：`--personal-dir` > `--settings` 的目錄 > 平台預設。
///
/// 平台預設是 `%LOCALAPPDATA%\jc-ime`（Windows）或 `$XDG_STATE_HOME/jc-ime`
/// （其他）。回 `None` 代表取不到可用目錄，個人化功能關閉而不是寫到共用資料夾。
fn resolve_personal_dir(options: &Options) -> Option<PathBuf> {
    if let Some(dir) = &options.personal_dir {
        return Some(dir.clone());
    }
    if let Some(parent) = options.settings.as_ref().and_then(|path| path.parent()) {
        if !parent.as_os_str().is_empty() {
            return Some(parent.to_path_buf());
        }
    }
    default_personal_dir()
}

/// 平台預設的使用者資料目錄。
fn default_personal_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
        return Some(PathBuf::from(dir).join("jc-ime"));
    }
    if let Some(dir) = std::env::var_os("XDG_STATE_HOME") {
        return Some(PathBuf::from(dir).join("jc-ime"));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state/jc-ime"))
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
    if let Some(dir) = &options.personal_dir {
        args.push("--personal-dir".into());
        args.push(dir.as_os_str().to_os_string());
    }
    // 舊版命令列保持不變；只有新版非預設前綴才多帶一個參數給子行程。
    if options.personal_prefix != DEFAULT_PERSONAL_PREFIX {
        args.push("--personal-prefix".into());
        args.push(options.personal_prefix.as_str().into());
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
/// `XDG_STATE_HOME` 或 `~/.local/state`。非預設實例用 `supervisor-<instance>.log`。
fn default_supervisor_log_for(instance: &str) -> PathBuf {
    let file = if instance == DEFAULT_INSTANCE {
        "supervisor.log".to_string()
    } else {
        format!("supervisor-{instance}.log")
    };
    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(dir).join("jc-ime").join(file);
    }
    if let Some(dir) = std::env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(dir).join("jc-ime").join(file);
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("jc-ime")
            .join(file);
    }
    std::env::temp_dir().join("jc-ime").join(file)
}

/// Windows 在 `path` 對應的 named pipe 上監聽，直接呼叫 `CreateNamedPipeW`。
///
/// SDDL 照 0c 逐字設定，含 AppContainer 的兩組 SID；不流經 `interprocess`，
/// 因此避開它複製含 `G:` 的 SDDL 時回 ERROR_INVALID_OWNER (1307) 的 bug。
/// T4 起每條連線一個 worker thread，共用同一份 engine registry。
#[cfg(windows)]
fn serve_socket(engine: &Engine, path: &Path, max_frame: usize) -> anyhow::Result<()> {
    pipe_listener::serve(engine, path, max_frame)
}

/// 在 `path` 對應的 unix domain socket 上監聽；T4 起每條連線一個 worker
/// thread，共用同一份 registry。listener 設為 nonblocking，每 50ms 輪一次，
/// 讓 drain／Shutdown 在沒有新連線時也能結束 accept 迴圈。
#[cfg(unix)]
fn serve_socket(engine: &Engine, path: &Path, max_frame: usize) -> anyhow::Result<()> {
    use std::thread;

    let name = jc_windows::socket_name(path)?;
    let options = ListenerOptions::new().name(jc_windows::ns_name(&name)?);
    let listener = options
        .create_sync()
        .with_context(|| format!("監聽 local socket 失敗：{name}"))?;
    listener
        .set_nonblocking(ListenerNonblockingMode::Accept)
        .context("設定 nonblocking accept 失敗")?;
    log::info!("jc-engine 監聽 local socket {name}（多連線）");

    loop {
        match listener.accept() {
            Ok(stream) => {
                log::debug!("接受 local socket 連線");
                let connection = engine.accept();
                // interprocess 的 Stream 不暴露 raw fd，改用 receive timeout：
                // worker 每 250ms 醒來一次檢查全域收線旗標，drain／Shutdown
                // 因此能即時收掉所有連線，不必 async runtime。
                stream
                    .set_recv_timeout(Some(Duration::from_millis(250)))
                    .context("設定 socket receive timeout 失敗")?;
                let spawned =
                    thread::Builder::new()
                        .name("jc-engine-conn".into())
                        .spawn(move || {
                            let mut view = connection.into_engine();
                            let mut input = BufReader::new(&stream);
                            let mut output = BufWriter::new(&stream);
                            if let Err(err) = serve(&mut view, &mut input, &mut output, max_frame) {
                                log::warn!("連線結束：{err}");
                            }
                            if let Err(err) = output.flush() {
                                log::warn!("寫出最後一個回應失敗：{err}");
                            }
                            view.finish_connection();
                        });
                if let Err(err) = spawned {
                    log::warn!("衍生連線 worker 失敗：{err}");
                }
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(err) => {
                log::warn!("接受連線失敗：{err}");
                thread::sleep(Duration::from_millis(50));
            }
        }
        if engine.is_shutdown() || engine.is_drain_done() {
            log::debug!("行程結束旗標已設定，離開 accept 迴圈");
            break;
        }
    }

    Ok(())
}

/// 既不是 unix 也不是 Windows 的平台沒有 socket 傳輸，明確回報錯誤而不是 panic。
#[cfg(not(any(unix, windows)))]
fn serve_socket(_engine: &Engine, _path: &Path, _max_frame: usize) -> anyhow::Result<()> {
    bail!("這個平台不支援 --socket，只能使用 stdio 模式")
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
    let mut personal_prefix = DEFAULT_PERSONAL_PREFIX.to_string();
    let mut personal_dir: Option<PathBuf> = None;
    let mut instance = DEFAULT_INSTANCE.to_string();
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
            "--personal-dir" => {
                let value = value_of(args, &mut index, "--personal-dir")?;
                personal_dir = Some(PathBuf::from(value));
            }
            "--personal-prefix" => {
                let value = value_of(args, &mut index, "--personal-prefix")?;
                if value.is_empty() || value.contains('/') || value.contains('\\') {
                    return Err(format!(
                        "--personal-prefix 必須是非空白的檔名前綴，收到 {value}"
                    ));
                }
                personal_prefix = value;
            }
            "--instance" => {
                let value = value_of(args, &mut index, "--instance")?;
                if value.is_empty() || value.contains('/') || value.contains('\\') {
                    return Err(format!("--instance 必須是非空白的實例名，收到 {value}"));
                }
                instance = value;
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
    Ok(Parsed::Run(Box::new(Options {
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
        personal_prefix,
        personal_dir,
        instance,
    })))
}

/// 解析 `--mode`；`en` 併入混打，舊值仍可解析。
fn parse_mode(value: &str) -> Result<Mode, String> {
    match value {
        "zh" => Ok(Mode::Mixed),
        "ja" => Ok(Mode::Mixed),
        "mixed" | "en" => Ok(Mode::Mixed),
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
const SOCKET_HELP: &str =
    "  --socket <path>     改用 local socket（unix domain socket），一次服務一個連線\n";
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
         \x20           [--personal-prefix <name>]\n\
\x20 jc-engine --supervise [--socket <name>] [--supervisor-log <path>]\n\
\x20           [--instance <name>]\n\n\
         選項：\n\
         \x20 --data <dir>        資料根目錄；未指定時讀 JC_DATA\n\
         \x20 --settings <path>   設定檔；未指定時讀資料根目錄的 settings.json\n\
         \x20 --personal-dir <dir> 個人學習資料目錄；預設同 --settings 的目錄\n\
         \x20 --print-settings    印出生效後的設定（JSON）後結束\n\
         \x20 --allow-empty-data  不載入資料，只用空 session（測試／除錯用）\n\
         \x20 --supervise         常駐監督子行程，結束後依退避曲線重啟\n\
         \x20 --supervisor-log <path>  監督者 log 路徑（預設 %LOCALAPPDATA%\\jc-ime\\supervisor.log）\n\
\x20 --personal-prefix <name> 個人資料檔名前綴，預設 personal\n\
\x20 --instance <name>        監督者實例名，預設 main（影響鎖與 log 檔名）\n\
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
        "用法：jc-engine --data <dir> [--settings <path>] [--personal-dir <dir>] [--mode <zh|ja|mixed|en>] [--neural <on|off>] [--socket <path>] [--max-frame <bytes>] [--personal-prefix <name>]\n\
         \x20     jc-engine --supervise [--socket <name>] [--supervisor-log <path>] [--instance <name>]"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(words: &[&str]) -> Result<Parsed, String> {
        let args: Vec<String> = words.iter().map(|word| word.to_string()).collect();
        parse_args(&args)
    }

    /// 只有載入 OpenCC 轉換器時才宣告 `output_simplified_live`。
    #[test]
    fn shell_caps_only_advertise_simplified_with_converter() {
        let without = build_shell_caps("both", false, false);
        assert_eq!(without.get("left_shift"), Some(&true));
        assert_eq!(without.get("right_shift"), Some(&true));
        assert_eq!(without.get("has_bigram"), Some(&false));
        assert!(
            !without.contains_key("output_simplified_live"),
            "沒有轉換器時不得宣告能力旗標"
        );

        let with = build_shell_caps("left", true, true);
        assert_eq!(with.get("left_shift"), Some(&true));
        assert_eq!(with.get("right_shift"), Some(&false));
        assert_eq!(with.get("has_bigram"), Some(&true));
        assert_eq!(with.get("output_simplified_live"), Some(&true));
        // 日文能力只由 capability 宣告；外殼不再需要 legacy `japanese` 模式。
        assert_eq!(with.get("japanese"), Some(&true));
        assert_eq!(with.get("convert_keys"), Some(&true));
        assert_eq!(with.get("clause_display"), Some(&true));
    }

    /// 移除的功能不能再從 health 廣告能力；外殼不該有機會重新打開它。
    #[test]
    fn health_does_not_advertise_removed_caps_lock_english() {
        for toggle in ["both", "left", "right", "none"] {
            let caps = build_shell_caps(toggle, true, true);
            assert!(
                !caps.contains_key("caps_lock_english"),
                "{toggle}: 已移除的 Caps Lock 英文能力不該回報"
            );
        }
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
        assert_eq!(options.personal_prefix, DEFAULT_PERSONAL_PREFIX);
        assert_eq!(options.instance, DEFAULT_INSTANCE);
    }

    /// 個人資料目錄不能落在資料根目錄：安裝後那是 `C:\ProgramData`，一般
    /// 使用者沒有寫入權，引擎每次載入都回「存取被拒 (os error 5)」。
    #[test]
    fn personal_dir_follows_settings_and_override() {
        // 用 `/` 分隔：測試在 Linux 與 Windows 都要跑得過。
        let Ok(Parsed::Run(options)) = parse(&[
            "--data",
            "/srv/jc-ime/data",
            "--settings",
            "/home/jeff/.local/state/jc-ime/settings.json",
        ]) else {
            panic!("應該解析成功");
        };
        assert_eq!(
            resolve_personal_dir(&options),
            Some(PathBuf::from("/home/jeff/.local/state/jc-ime")),
            "沒有 --personal-dir 時要跟 --settings 同一個目錄"
        );

        let Ok(Parsed::Run(options)) = parse(&[
            "--data",
            "/srv/jc-ime/data",
            "--settings",
            "/home/jeff/.local/state/jc-ime/settings.json",
            "--personal-dir",
            "/var/tmp/jc-personal",
        ]) else {
            panic!("應該解析成功");
        };
        assert_eq!(
            resolve_personal_dir(&options),
            Some(PathBuf::from("/var/tmp/jc-personal")),
            "--personal-dir 優先"
        );
    }

    /// 監督者要把 `--personal-dir` 轉給子行程，否則子行程會退回資料根目錄。
    #[test]
    fn supervisor_forwards_personal_dir() {
        let Ok(Parsed::Run(options)) = parse(&[
            "--data",
            r"C:\ProgramData\jc-ime\data",
            "--personal-dir",
            r"D:\jc-personal",
            "--supervise",
        ]) else {
            panic!("應該解析成功");
        };
        let args: Vec<String> = child_args(&options, std::path::Path::new("jc-ime-1001"))
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--personal-dir" && pair[1] == r"D:\jc-personal"),
            "子行程參數要含 --personal-dir：{args:?}"
        );
    }

    /// 自訂個人資料前綴與監督者實例名要照原樣解析，影響 log 與鎖名。
    #[test]
    fn custom_prefix_and_instance_are_parsed() {
        let Ok(Parsed::Run(options)) = parse(&[
            "--personal-prefix",
            "custom-personal",
            "--instance",
            "custom-instance",
            "--supervise",
        ]) else {
            panic!("應該解析成功");
        };
        assert_eq!(options.personal_prefix, "custom-personal");
        assert_eq!(options.instance, "custom-instance");
        assert!(options.supervise);
        assert_eq!(
            default_supervisor_log_for(&options.instance)
                .file_name()
                .unwrap(),
            "supervisor-custom-instance.log"
        );
        assert_eq!(
            supervisor_lock_name(&options.instance),
            format!("{SUPERVISOR_MUTEX_NAME}-custom-instance")
        );
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
        assert_eq!(options.input, Some(Mode::Mixed));
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
                "mixed",
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
