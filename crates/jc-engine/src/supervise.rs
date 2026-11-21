//! 引擎常駐監督者。
//!
//! `jc-engine --supervise` 會用同一支執行檔啟動子行程
//! `jc-engine --socket <name>`，子行程結束後依退避曲線重啟。整個流程是同步的：
//! 主執行緒輪詢子行程狀態與健康檢查，讀取子行程輸出的執行緒只把內容寫進
//! 共用 log。
//!
//! 範圍刻意限制在單一使用者（Windows 具名 mutex `Local\...`），不做 Windows
//! 服務，避免 session 0 與權限問題。

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

/// 監督者的單一實例名稱。
///
/// Windows 用它當 `%LOCALAPPDATA%\jc-ime\` 下的鎖檔名。早期版本把同一個字串
/// 當成 `Local\` 具名 mutex，那是 session 範圍，同一使用者的兩個 RDP session
/// 會各自拿到一把鎖；改成帳號範圍的鎖檔才與 pipe 名稱（帶使用者 SID）一致。
pub const SUPERVISOR_MUTEX_NAME: &str = r"Local\jc-ime-engine-supervisor";

/// log 檔案上限，超過就丟掉最舊的一半。
pub const LOG_LIMIT: u64 = 1 << 20;

/// 退避曲線：第 1、2、3、4 次失敗分別等 1、2、5、10 秒，之後固定 10 秒。
pub const BACKOFF: [Duration; 4] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
];

/// 連續失敗幾次後進入暫停。
pub const FAILURE_LIMIT: u32 = 5;

/// 連續失敗後的暫停時間。
pub const RESTART_PAUSE: Duration = Duration::from_secs(300);

/// 健康檢查間隔。
pub const HEALTH_INTERVAL: Duration = Duration::from_secs(30);

/// 子行程存活超過這個時間就視為穩定，失敗次數歸零。
pub const STABLE_AFTER: Duration = Duration::from_secs(60);

/// 監督者的設定。預設值對應任務書的參數；測試會把時間縮短。
#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    /// 要啟動的執行檔，通常是目前的 `jc-engine`。
    pub program: PathBuf,
    /// 傳給子行程的參數，必須包含 `--socket`。
    pub child_args: Vec<OsString>,
    /// 子行程的 socket 或 pipe，健康檢查也連這個位置。
    pub socket: PathBuf,
    /// 單一實例鎖的名稱。
    pub lock_name: String,
    /// log 檔案路徑。
    pub log_path: PathBuf,
    /// log 檔案大小上限。
    pub log_limit: u64,
    /// 退避曲線，最後一個值會一直重複。
    pub backoff: Vec<Duration>,
    /// 連續失敗後的暫停時間。
    pub restart_pause: Duration,
    /// 連續失敗幾次後暫停。
    pub failure_limit: u32,
    /// 健康檢查間隔。
    pub health_interval: Duration,
    /// 子行程存活多久算穩定。
    pub stable_after: Duration,
    /// 等待子行程狀態的輪詢間隔，也決定停止訊號的反應速度。
    pub poll_interval: Duration,
    /// 收到停止訊號時，等子行程自己結束的寬限時間。
    pub shutdown_grace: Duration,
}

impl SupervisorConfig {
    /// 用執行檔、子行程參數、socket 與 log 路徑建立設定，其餘套用任務書預設值。
    pub fn new(
        program: impl Into<PathBuf>,
        child_args: Vec<OsString>,
        socket: impl Into<PathBuf>,
        log_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            program: program.into(),
            child_args,
            socket: socket.into(),
            lock_name: SUPERVISOR_MUTEX_NAME.to_string(),
            log_path: log_path.into(),
            log_limit: LOG_LIMIT,
            backoff: BACKOFF.to_vec(),
            restart_pause: RESTART_PAUSE,
            failure_limit: FAILURE_LIMIT,
            health_interval: HEALTH_INTERVAL,
            stable_after: STABLE_AFTER,
            poll_interval: Duration::from_millis(100),
            shutdown_grace: Duration::from_secs(5),
        }
    }
}

/// 監督者錯誤。
#[derive(Debug, Error)]
pub enum Error {
    /// 已經有另一個監督者持有鎖。
    #[error("已有另一個監督者在執行（{name}）")]
    AlreadyRunning {
        /// 鎖的名稱。
        name: String,
    },
    /// 建立單一實例鎖失敗。
    #[error("建立單一實例鎖失敗：{0}")]
    Lock(#[source] io::Error),
    /// 建立 log 目錄失敗。
    #[error("建立 log 目錄失敗：{0}")]
    LogDir(#[source] io::Error),
    /// 開啟或寫入 log 失敗。
    #[error("log 檔案 {path} 失敗：{source}")]
    Log {
        /// log 路徑。
        path: PathBuf,
        /// 底層錯誤。
        #[source]
        source: io::Error,
    },
    /// 啟動子行程失敗。
    #[error("啟動子行程失敗：{0}")]
    Spawn(#[source] io::Error),
}

/// 共用 log。主執行緒與讀取子行程輸出的執行緒都往這裡寫。
type SharedLog = Arc<Mutex<LogRing>>;

/// 全域停止旗標，由 Ctrl+C、SIGTERM 或 session 結束事件設定。
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// 要求監督者結束。
pub(crate) fn request_shutdown() {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

/// 是否已經收到停止訊號。
fn shutdown_requested() -> bool {
    SHUTDOWN.load(Ordering::SeqCst)
}

/// 啟動監督者，直到收到停止訊號。
///
/// 同一個 `lock_name` 只允許一個行程。第二次啟動會回
/// [`Error::AlreadyRunning`]，呼叫端可視為正常結束。
pub fn run(config: &SupervisorConfig) -> Result<(), Error> {
    let _lock = InstanceLock::acquire(&config.lock_name)?;
    platform::install_shutdown_handlers();
    #[cfg(windows)]
    platform::watch_session_end();
    run_with(config, &shutdown_requested)
}

/// 監督迴圈。`stop` 是額外的停止條件，測試用它控制何時結束。
fn run_with(config: &SupervisorConfig, stop: &dyn Fn() -> bool) -> Result<(), Error> {
    let log: SharedLog = Arc::new(Mutex::new(LogRing::open(
        &config.log_path,
        config.log_limit,
    )?));
    log_line(
        &log,
        format!(
            "監督者啟動：子行程 {}，socket {}，log 上限 {} bytes",
            config.program.display(),
            config.socket.display(),
            config.log_limit
        ),
    );

    // Windows 用 job object 把子行程綁在監督者上：升級或工作管理員殺掉監督者
    // 時子行程一起結束，pipe 才會釋放。建立失敗只記警告，不擋住引擎啟動。
    let child_group = match platform::create_child_group() {
        Ok(group) => Some(group),
        Err(err) => {
            log_line(
                &log,
                format!("無法建立子行程群組（異常結束時可能殘留引擎行程）：{err}"),
            );
            None
        }
    };

    let mut failures: u32 = 0;
    loop {
        if stop() {
            break;
        }

        let mut child = match spawn_child(config, &log, child_group.as_ref()) {
            Ok(child) => child,
            Err(err) => {
                log_line(&log, format!("{err}"));
                if plan_retry(&mut failures, config, stop, &log) {
                    break;
                }
                continue;
            }
        };
        log_line(&log, format!("子行程啟動 pid={}", child.id()));
        let started = Instant::now();

        let status = match wait_child(&mut child, started, config, stop, &log) {
            WaitResult::Exited(status) => status,
            WaitResult::Stopped => break,
        };

        let uptime = started.elapsed();
        log_line(
            &log,
            format!("子行程結束（{status}），存活 {:.1}s", uptime.as_secs_f64()),
        );

        if status.success() {
            // 正常結束（例如 IME 送出 Shutdown）不算失敗，仍會重啟。
            failures = 0;
            let delay = config
                .backoff
                .first()
                .copied()
                .unwrap_or(Duration::from_secs(1));
            log_line(&log, format!("正常結束，{}ms 後重啟", delay.as_millis()));
            if wait_or_stop(delay, stop) {
                break;
            }
            continue;
        }

        if uptime >= config.stable_after && failures > 0 {
            log_line(&log, "子行程曾穩定存活，失敗次數歸零");
            failures = 0;
        }
        if plan_retry(&mut failures, config, stop, &log) {
            break;
        }
    }

    log_line(&log, "監督者結束");
    Ok(())
}

/// 等待子行程結束，期間做健康檢查並注意停止訊號。
enum WaitResult {
    /// 子行程自己結束。
    Exited(ExitStatus),
    /// 收到停止訊號，子行程已被結束。
    Stopped,
}

fn wait_child(
    child: &mut Child,
    started: Instant,
    config: &SupervisorConfig,
    stop: &dyn Fn() -> bool,
    log: &SharedLog,
) -> WaitResult {
    let mut next_health = started + config.health_interval;
    let mut last_healthy: Option<bool> = None;

    loop {
        if stop() {
            log_line(log, "收到停止訊號，結束子行程");
            terminate(child, config.shutdown_grace, log);
            return WaitResult::Stopped;
        }

        match child.try_wait() {
            Ok(Some(status)) => return WaitResult::Exited(status),
            Ok(None) => {}
            Err(err) => {
                log_line(log, format!("等待子行程失敗：{err}"));
                terminate(child, config.shutdown_grace, log);
                return WaitResult::Stopped;
            }
        }

        let now = Instant::now();
        if now >= next_health {
            let healthy = platform::health_ok(&config.socket);
            if !healthy {
                log_line(
                    log,
                    format!(
                        "健康檢查失敗：連不上 {}，子行程仍在執行 pid={}",
                        config.socket.display(),
                        child.id()
                    ),
                );
            } else if last_healthy == Some(false) {
                log_line(log, "健康檢查恢復正常");
            }
            last_healthy = Some(healthy);
            next_health = now + config.health_interval;
        }

        thread::sleep(config.poll_interval);
    }
}

/// 記錄一次失敗，並依退避曲線等待。回傳 `true` 表示等待期間收到停止訊號。
fn plan_retry(
    failures: &mut u32,
    config: &SupervisorConfig,
    stop: &dyn Fn() -> bool,
    log: &SharedLog,
) -> bool {
    *failures = failures.saturating_add(1);
    if *failures >= config.failure_limit {
        log_line(
            log,
            format!(
                "連續失敗 {} 次，停止重試 {}",
                *failures,
                human(config.restart_pause)
            ),
        );
        if wait_or_stop(config.restart_pause, stop) {
            return true;
        }
        *failures = 0;
        log_line(log, "暫停結束，重新啟動");
        return false;
    }

    let delay = backoff_delay(*failures, &config.backoff);
    log_line(
        log,
        format!("第 {} 次失敗，{}ms 後重啟", *failures, delay.as_millis()),
    );
    wait_or_stop(delay, stop)
}

/// 計算第 `failures` 次連續失敗後的等待時間。
///
/// 曲線用完後固定用最後一個值，所以 1→2→5→10→10…（秒）。
pub fn backoff_delay(failures: u32, curve: &[Duration]) -> Duration {
    match curve.split_last() {
        None => Duration::ZERO,
        Some((last, rest)) => {
            let index = failures.saturating_sub(1) as usize;
            rest.get(index).copied().unwrap_or(*last)
        }
    }
}

/// 把時間寫成人看得懂的字串；不到一秒用毫秒。
fn human(duration: Duration) -> String {
    if duration.as_secs() >= 1 {
        format!("{}s", duration.as_secs())
    } else {
        format!("{}ms", duration.as_millis())
    }
}

/// 等待 `duration`，期間每 50ms 檢查一次停止訊號。回傳 `true` 表示要停止。
fn wait_or_stop(duration: Duration, stop: &dyn Fn() -> bool) -> bool {
    let deadline = Instant::now() + duration;
    loop {
        if stop() {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        thread::sleep((deadline - now).min(Duration::from_millis(50)));
    }
}

/// 啟動子行程，並把 stdout、stderr 接到共用 log。
fn spawn_child(
    config: &SupervisorConfig,
    log: &SharedLog,
    child_group: Option<&platform::ChildGroup>,
) -> Result<Child, Error> {
    let mut command = Command::new(&config.program);
    command
        .args(&config.child_args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 讓子行程自成一組，監督者才能只送 Ctrl+Break 給它。
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP);
    }

    let mut child = command.spawn().map_err(Error::Spawn)?;
    if let Some(group) = child_group {
        if let Err(err) = platform::assign_child(group, child.id()) {
            // 子行程已經啟動，指派失敗不能當成啟動失敗；照樣收輸出並監控。
            log_line(
                log,
                format!("子行程 pid={} 加入群組失敗：{err}", child.id()),
            );
        }
    }
    if let Some(stdout) = child.stdout.take() {
        pump(stdout, Arc::clone(log), "stdout");
    }
    if let Some(stderr) = child.stderr.take() {
        pump(stderr, Arc::clone(log), "stderr");
    }
    Ok(child)
}

/// 把子行程的一個輸出串流逐行寫進 log。
fn pump<R: Read + Send + 'static>(stream: R, log: SharedLog, tag: &'static str) {
    let reader_log = Arc::clone(&log);
    let spawned = thread::Builder::new()
        .name(format!("jc-supervisor-{tag}"))
        .spawn(move || {
            for line in BufReader::new(stream).lines() {
                match line {
                    Ok(line) => log_line(&reader_log, format!("子行程 {tag}: {line}")),
                    Err(err) => {
                        log_line(&reader_log, format!("讀取子行程 {tag} 失敗：{err}"));
                        break;
                    }
                }
            }
        });
    if spawned.is_err() {
        log_line(&log, format!("無法建立讀取子行程 {tag} 的執行緒"));
    }
}

/// 先送溫和的結束訊號，寬限期過了才強制結束。
fn terminate(child: &mut Child, grace: Duration, log: &SharedLog) {
    #[cfg(unix)]
    {
        // SIGTERM 讓引擎有機會收尾；忽略錯誤是因為行程可能已經結束。
        unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
        }
    }
    #[cfg(windows)]
    {
        let ok = unsafe {
            windows_sys::Win32::System::Console::GenerateConsoleCtrlEvent(
                windows_sys::Win32::System::Console::CTRL_BREAK_EVENT,
                child.id(),
            )
        };
        if ok == 0 {
            let _ = child.kill();
        }
    }

    let deadline = Instant::now() + grace;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => {}
            Err(err) => {
                log_line(log, format!("等待子行程收尾失敗：{err}"));
                break;
            }
        }
        if Instant::now() >= deadline {
            log_line(log, "子行程未在寬限期內結束，強制終止");
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    if let Err(err) = child.kill() {
        log_line(log, format!("強制終止子行程失敗：{err}"));
    }
    let _ = child.wait();
}

/// 寫一行 log；log 本身失敗時只印到 stderr，不再遞迴。
fn log_line(log: &SharedLog, message: impl AsRef<str>) {
    let mut ring = match log.lock() {
        Ok(ring) => ring,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Err(err) = ring.write_line(message.as_ref()) {
        eprintln!("jc-engine supervisor: 寫入 log 失敗：{err}");
    }
}

/// 大小有上限的環狀 log。
///
/// 每次寫入都帶上相對於監督者啟動時間的秒數。超過上限時保留檔尾約一半的
/// 內容，從下一個換行開始，並插入一行標記。
pub struct LogRing {
    path: PathBuf,
    limit: u64,
    file: File,
    size: u64,
    started: Instant,
}

impl LogRing {
    /// 開啟（或建立）log；目錄不存在會先建立。既有的檔案太大會先裁掉前半。
    pub fn open(path: impl Into<PathBuf>, limit: u64) -> Result<Self, Error> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(Error::LogDir)?;
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|source| Error::Log {
                path: path.clone(),
                source,
            })?;
        let size = file
            .metadata()
            .map_err(|source| Error::Log {
                path: path.clone(),
                source,
            })?
            .len();
        let mut ring = Self {
            path,
            limit: limit.max(1),
            file,
            size,
            started: Instant::now(),
        };
        if ring.size >= ring.limit {
            ring.trim().map_err(|source| Error::Log {
                path: ring.path.clone(),
                source,
            })?;
        }
        Ok(ring)
    }

    /// log 路徑。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 寫一行，超過上限就先裁掉最舊的內容。
    pub fn write_line(&mut self, message: &str) -> io::Result<()> {
        let message = truncate_chars(message, 4096);
        let line = format!(
            "[{:>10.3}s] {message}\n",
            self.started.elapsed().as_secs_f64()
        );
        if self.size + line.len() as u64 > self.limit {
            self.trim()?;
        }
        self.file.write_all(line.as_bytes())?;
        self.size += line.len() as u64;
        self.file.flush()
    }

    /// 保留檔尾約一半的內容；從換行邊界開始，避免留下半行。
    fn trim(&mut self) -> io::Result<()> {
        let keep = (self.limit / 2).max(64) as usize;
        let mut data = Vec::new();
        File::open(&self.path)?.read_to_end(&mut data)?;
        let mut start = data.len().saturating_sub(keep);
        while start < data.len() && data[start] != b'\n' {
            start += 1;
        }
        if start < data.len() {
            start += 1;
        }
        let tail = data[start..].to_vec();

        let marker = "[log] 已達大小上限，丟棄最舊內容\n";
        let mut fresh = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.path)?;
        fresh.write_all(marker.as_bytes())?;
        fresh.write_all(&tail)?;
        fresh.flush()?;

        self.size = marker.len() as u64 + tail.len() as u64;
        Ok(())
    }
}

/// 依字元邊界截短訊息，避免寫入超長的一行。
fn truncate_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((index, _)) => format!("{}…", &text[..index]),
        None => text.to_string(),
    }
}

/// 單一實例鎖。
///
/// Windows 用 `%LOCALAPPDATA%\jc-ime\` 下以獨佔分享模式開啟的鎖檔（行程結束
/// 作業系統會自動關閉 handle）；Linux 用 abstract unix socket；其他 unix 用
/// `$TMPDIR` 下的 socket 檔。鎖在 drop 時釋放，行程被強制結束時由作業系統回收。
///
/// 鎖的範圍必須與 pipe 名稱一致：pipe 帶使用者 SID，帳號範圍的鎖擋得住同一
/// 使用者的兩個 session；session 範圍的 `Local\` mutex 擋不住。
pub struct InstanceLock {
    /// 平台鎖的 handle；只靠 drop 釋放，不直接讀取。
    _inner: platform::Lock,
}

impl InstanceLock {
    /// 取得鎖；名稱已被其他行程持有時回 [`Error::AlreadyRunning`]。
    pub fn acquire(name: &str) -> Result<Self, Error> {
        match platform::acquire(name).map_err(Error::Lock)? {
            Some(inner) => Ok(Self { _inner: inner }),
            None => Err(Error::AlreadyRunning {
                name: name.to_string(),
            }),
        }
    }
}

#[cfg(unix)]
mod platform {
    use std::io;
    use std::os::unix::net::UnixStream;

    /// 平台鎖。
    pub(super) struct Lock {
        _listener: std::os::unix::net::UnixListener,
        #[cfg(not(target_os = "linux"))]
        path: std::path::PathBuf,
    }

    #[cfg(target_os = "linux")]
    pub(super) fn acquire(name: &str) -> io::Result<Option<Lock>> {
        use std::os::linux::net::SocketAddrExt;

        let addr = std::os::unix::net::SocketAddr::from_abstract_name(name.as_bytes())?;
        match std::os::unix::net::UnixListener::bind_addr(&addr) {
            Ok(listener) => Ok(Some(Lock {
                _listener: listener,
            })),
            Err(err) if err.kind() == io::ErrorKind::AddrInUse => Ok(None),
            Err(err) => Err(err),
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub(super) fn acquire(name: &str) -> io::Result<Option<Lock>> {
        let safe: String = name
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                    ch
                } else {
                    '-'
                }
            })
            .collect();
        let path = std::env::temp_dir().join(format!("{safe}.sock"));
        match bind(&path) {
            Ok(listener) => Ok(Some(Lock {
                _listener: listener,
                path,
            })),
            Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
                if UnixStream::connect(&path).is_ok() {
                    return Ok(None);
                }
                // 殘留的 socket 檔：沒有行程在聽，清掉後重試一次。
                let _ = std::fs::remove_file(&path);
                match bind(&path) {
                    Ok(listener) => Ok(Some(Lock {
                        _listener: listener,
                        path,
                    })),
                    Err(err) if err.kind() == io::ErrorKind::AddrInUse => Ok(None),
                    Err(err) => Err(err),
                }
            }
            Err(err) => Err(err),
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn bind(path: &std::path::Path) -> io::Result<std::os::unix::net::UnixListener> {
        std::os::unix::net::UnixListener::bind(path)
    }

    #[cfg(not(target_os = "linux"))]
    impl Drop for Lock {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// 非 Windows 沒有 job object；子行程由 `terminate` 收尾。
    pub(super) struct ChildGroup;

    pub(super) fn create_child_group() -> io::Result<ChildGroup> {
        Ok(ChildGroup)
    }

    pub(super) fn assign_child(_group: &ChildGroup, _pid: u32) -> io::Result<()> {
        Ok(())
    }

    /// 安裝 SIGINT、SIGTERM 的處理器；只設停止旗標，實際收尾在主迴圈。
    pub(super) fn install_shutdown_handlers() {
        unsafe {
            libc::signal(
                libc::SIGINT,
                handle_signal as *const () as libc::sighandler_t,
            );
            libc::signal(
                libc::SIGTERM,
                handle_signal as *const () as libc::sighandler_t,
            );
        }
    }

    extern "C" fn handle_signal(_signal: libc::c_int) {
        super::request_shutdown();
    }

    /// 連一次 socket 當健康檢查。連上就立刻關閉，引擎會看到一次空的連線。
    pub(super) fn health_ok(socket: &std::path::Path) -> bool {
        UnixStream::connect(socket).is_ok()
    }
}

#[cfg(windows)]
mod platform {
    use std::io;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::path::PathBuf;
    use std::ptr;

    use windows_sys::Win32::Foundation::{
        GetLastError, ERROR_SEM_TIMEOUT, ERROR_SHARING_VIOLATION, GENERIC_READ, GENERIC_WRITE,
        INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_ALWAYS,
    };
    use windows_sys::Win32::System::Console::{
        SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_C_EVENT, CTRL_LOGOFF_EVENT,
        CTRL_SHUTDOWN_EVENT,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Pipes::WaitNamedPipeW;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    /// 鎖檔的 handle；drop 時關閉，另一個監督者才能開啟。
    pub(super) struct Lock {
        _handle: OwnedHandle,
    }

    /// 鎖檔路徑：`%LOCALAPPDATA%\jc-ime\<safe-name>.lock`。
    ///
    /// `LOCALAPPDATA` 是帳號範圍，跨 RDP session 相同；`Local\` mutex 則是
    /// session 範圍，同一帳號兩個 session 會拿到兩把鎖。名稱只當檔名用，
    /// 非法字元換成 `-`，避免路徑穿越。
    fn lock_path(name: &str) -> PathBuf {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let safe: String = name
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                    ch
                } else {
                    '-'
                }
            })
            .collect();
        base.join("jc-ime").join(format!("{safe}.lock"))
    }

    pub(super) fn acquire(name: &str) -> io::Result<Option<Lock>> {
        let path = lock_path(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let wide = super::wide(&path.to_string_lossy());
        // share mode 0：任何人（包括同一行程的第二次呼叫）都開不了第二個 handle。
        // 行程結束時作業系統關閉 handle，鎖自動釋放，檔案留下也沒關係。
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                ptr::null(),
                OPEN_ALWAYS,
                FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(ERROR_SHARING_VIOLATION as i32) {
                return Ok(None);
            }
            return Err(err);
        }
        // SAFETY: handle 剛由 CreateFileW 建立，所有權交給 OwnedHandle。
        Ok(Some(Lock {
            _handle: unsafe { OwnedHandle::from_raw_handle(handle) },
        }))
    }

    /// 子行程群組：Windows job object 設 `KILL_ON_JOB_CLOSE`，監督者被殺時
    /// 子行程一起結束，不會留下孤兒引擎佔著 pipe。
    pub(super) struct ChildGroup {
        job: OwnedHandle,
    }

    pub(super) fn create_child_group() -> io::Result<ChildGroup> {
        let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: handle 剛由 CreateJobObjectW 建立。
        let job = unsafe { OwnedHandle::from_raw_handle(job) };
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = unsafe {
            SetInformationJobObject(
                job.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of_val(&info) as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(ChildGroup { job })
    }

    pub(super) fn assign_child(group: &ChildGroup, pid: u32) -> io::Result<()> {
        // AssignProcessToJobObject 需要 PROCESS_SET_QUOTA；一併要 TERMINATE 才能收尾。
        let process = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid) };
        if process.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: handle 剛由 OpenProcess 建立。
        let process = unsafe { OwnedHandle::from_raw_handle(process) };
        let ok = unsafe {
            AssignProcessToJobObject(
                group.job.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE,
                process.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// 安裝 console 控制處理器：Ctrl+C、關機、登出都只設停止旗標。
    pub(super) fn install_shutdown_handlers() {
        unsafe {
            SetConsoleCtrlHandler(Some(handle_ctrl), 1);
        }
    }

    unsafe extern "system" fn handle_ctrl(ctrl_type: u32) -> i32 {
        match ctrl_type {
            CTRL_C_EVENT | CTRL_BREAK_EVENT | CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT
            | CTRL_SHUTDOWN_EVENT => {
                super::request_shutdown();
                1
            }
            _ => 0,
        }
    }

    /// 用訊息視窗接收 `WM_ENDSESSION`。console 程式在 session 結束時通常收到
    /// `CTRL_LOGOFF_EVENT`／`CTRL_SHUTDOWN_EVENT`，這裡補上視窗訊息版本。
    pub(super) fn watch_session_end() {
        let spawned = std::thread::Builder::new()
            .name("jc-supervisor-session".to_string())
            .spawn(|| unsafe { session_window() });
        if spawned.is_err() {
            log::warn!("無法建立監聽 session 結束事件的執行緒");
        }
    }

    #[allow(clippy::undocumented_unsafe_blocks)]
    unsafe fn session_window() {
        use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
        use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
            TranslateMessage, MSG, WM_ENDSESSION, WNDCLASSW,
        };

        const CLASS: &str = "jc-ime-supervisor";

        unsafe extern "system" fn wndproc(
            hwnd: HWND,
            message: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            if message == WM_ENDSESSION && wparam != 0 {
                super::request_shutdown();
                return 0;
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }

        let instance = GetModuleHandleW(ptr::null());
        let class = super::wide(CLASS);
        let window_class = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: ptr::null_mut(),
            hCursor: ptr::null_mut(),
            hbrBackground: ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: class.as_ptr(),
        };
        if RegisterClassW(&window_class) == 0 {
            log::warn!("註冊監督者視窗類別失敗，session 結束事件改用 console 事件");
            return;
        }
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null_mut(),
        );
        if hwnd.is_null() {
            log::warn!("建立監督者訊息視窗失敗，session 結束事件改用 console 事件");
            return;
        }

        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    /// 健康檢查：pipe 存不存在，不建立連線。
    ///
    /// 唯一的 instance 被 IME 佔用時 `WaitNamedPipeW` 會逾時並回
    /// `ERROR_SEM_TIMEOUT`；那代表引擎活著，算健康。只有找不到 pipe 才失敗。
    pub(super) fn health_ok(socket: &std::path::Path) -> bool {
        let name = super::pipe_path(socket);
        let wide = super::wide(&name);
        if unsafe { WaitNamedPipeW(wide.as_ptr(), 1) } != 0 {
            return true;
        }
        (unsafe { GetLastError() }) == ERROR_SEM_TIMEOUT
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use std::io;

    /// 其他平台沒有單一實例機制，保留型別讓程式能編譯。
    pub(super) struct Lock;

    pub(super) fn acquire(_name: &str) -> io::Result<Option<Lock>> {
        Ok(Some(Lock))
    }

    pub(super) struct ChildGroup;

    pub(super) fn create_child_group() -> io::Result<ChildGroup> {
        Ok(ChildGroup)
    }

    pub(super) fn assign_child(_group: &ChildGroup, _pid: u32) -> io::Result<()> {
        Ok(())
    }

    pub(super) fn install_shutdown_handlers() {}

    pub(super) fn health_ok(_socket: &std::path::Path) -> bool {
        false
    }
}

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 把 socket 設定轉成完整 pipe 名稱；與 `pipe_windows` 的規則一致。
#[cfg(windows)]
fn pipe_path(socket: &Path) -> String {
    let raw = socket.as_os_str().to_string_lossy();
    let base = raw.rsplit(['\\', '/']).next().unwrap_or("");
    format!(r"\\.\pipe\{base}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("jc-supervisor-test-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建立測試目錄");
        dir
    }

    #[cfg(unix)]
    fn count_starts(path: &Path) -> u32 {
        std::fs::read_to_string(path)
            .map(|text| text.lines().count() as u32)
            .unwrap_or(0)
    }

    #[cfg(unix)]
    fn read_log(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    #[test]
    fn backoff_curve_matches_spec() {
        let curve = BACKOFF;
        assert_eq!(backoff_delay(1, &curve), Duration::from_secs(1));
        assert_eq!(backoff_delay(2, &curve), Duration::from_secs(2));
        assert_eq!(backoff_delay(3, &curve), Duration::from_secs(5));
        assert_eq!(backoff_delay(4, &curve), Duration::from_secs(10));
        assert_eq!(backoff_delay(9, &curve), Duration::from_secs(10));
        assert_eq!(backoff_delay(1, &[]), Duration::ZERO);
    }

    #[test]
    fn supervisor_mutex_is_per_user() {
        assert_eq!(SUPERVISOR_MUTEX_NAME, r"Local\jc-ime-engine-supervisor");
    }

    #[test]
    fn log_ring_keeps_newest_lines_under_limit() {
        let dir = temp_dir("ring");
        let path = dir.join("supervisor.log");
        let mut ring = LogRing::open(&path, 512).expect("開啟 log");
        for index in 0..200 {
            ring.write_line(&format!("第 {index} 行")).expect("寫入");
        }
        let text = std::fs::read_to_string(&path).expect("讀取 log");
        assert!(
            text.len() as u64 <= 512,
            "log 不應超過上限，實際 {} bytes",
            text.len()
        );
        assert!(text.contains("第 199 行"), "應保留最新內容");
        assert!(!text.contains("第 0 行"), "最舊內容應被丟棄");
        assert!(text.contains("已達大小上限"), "應留下裁切標記");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn second_supervisor_is_rejected_by_lock() {
        let name = format!("jc-ime-test-lock-{}", std::process::id());
        let first = InstanceLock::acquire(&name).expect("第一個鎖應成功");
        let second = InstanceLock::acquire(&name);
        assert!(
            matches!(second, Err(Error::AlreadyRunning { .. })),
            "第二個鎖應被拒絕"
        );
        drop(first);
        let third = InstanceLock::acquire(&name).expect("釋放後應能再取得");
        drop(third);
    }

    /// Windows 也必須擋住第二個監督者；鎖檔走帳號範圍，不受 session 影響。
    #[cfg(windows)]
    #[test]
    fn second_supervisor_is_rejected_by_lock() {
        let name = format!(r"Local\jc-ime-test-lock-{}", std::process::id());
        let first = InstanceLock::acquire(&name).expect("第一個鎖應成功");
        let second = InstanceLock::acquire(&name);
        assert!(
            matches!(second, Err(Error::AlreadyRunning { .. })),
            "第二個鎖應被拒絕"
        );
        drop(first);
        let third = InstanceLock::acquire(&name).expect("釋放後應能再取得");
        drop(third);
    }

    #[cfg(unix)]
    #[test]
    fn child_is_restarted_until_stopped() {
        let dir = temp_dir("restart");
        let counter = dir.join("starts.log");
        let log_path = dir.join("supervisor.log");

        let mut config = SupervisorConfig::new(
            "/bin/sh",
            vec![
                "-c".into(),
                format!("echo start >> {}; exit 7", counter.display()).into(),
            ],
            dir.join("engine.sock"),
            &log_path,
        );
        config.backoff = vec![Duration::from_millis(10)];
        config.poll_interval = Duration::from_millis(5);
        config.health_interval = Duration::from_millis(20);
        config.failure_limit = 100;

        run_with(&config, &|| count_starts(&counter) >= 3).expect("監督迴圈應正常結束");

        assert_eq!(count_starts(&counter), 3, "應啟動三次");
        let text = read_log(&log_path);
        assert!(text.contains("子行程結束"), "應記錄子行程結束");
        assert!(text.contains("第 1 次失敗"), "應記錄退避");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn five_failures_pause_before_retry() {
        let dir = temp_dir("pause");
        let counter = dir.join("starts.log");
        let log_path = dir.join("supervisor.log");

        let mut config = SupervisorConfig::new(
            "/bin/sh",
            vec![
                "-c".into(),
                format!("echo start >> {}; exit 1", counter.display()).into(),
            ],
            dir.join("engine.sock"),
            &log_path,
        );
        config.backoff = vec![Duration::from_millis(5)];
        config.poll_interval = Duration::from_millis(5);
        config.health_interval = Duration::from_millis(10);
        config.failure_limit = 5;
        config.restart_pause = Duration::from_secs(30);

        let stop_counter = counter.clone();
        let stop_log = log_path.clone();
        let stop =
            move || count_starts(&stop_counter) >= 5 && read_log(&stop_log).contains("停止重試");
        run_with(&config, &stop).expect("監督迴圈應正常結束");

        assert_eq!(count_starts(&counter), 5, "暫停前應只啟動五次");
        assert!(
            read_log(&log_path).contains("連續失敗 5 次，停止重試 30s"),
            "應記錄暫停與時間"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn health_check_warns_when_socket_missing() {
        let dir = temp_dir("health");
        let log_path = dir.join("supervisor.log");

        let mut config = SupervisorConfig::new(
            "/bin/sh",
            vec!["-c".into(), "sleep 5".into()],
            dir.join("engine.sock"),
            &log_path,
        );
        config.poll_interval = Duration::from_millis(5);
        config.health_interval = Duration::from_millis(20);
        config.shutdown_grace = Duration::from_millis(200);

        let stop_log = log_path.clone();
        let stop = move || read_log(&stop_log).contains("健康檢查失敗");
        run_with(&config, &stop).expect("監督迴圈應正常結束");

        assert!(
            read_log(&log_path).contains("健康檢查失敗"),
            "子行程活著但連不上 socket 應警告"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
