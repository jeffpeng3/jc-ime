//! 引擎連線與 worker 執行緒。
//!
//! UI 執行緒只負責把 [`Job`] 丟進 channel；連線、握手、逾時與重連都在 worker
//! 執行緒完成。回應先推進 [`Queue`]，再用 `PostMessageW` 通知 UI 執行緒，
//! 所以按鍵處理永遠不會卡在 IPC 上。
//!
//! 逾時固定 500 ms：引擎的神經模型硬上限是 180 ms，超過就是連線壞掉，依
//! `jc-windows` 的契約必須重連，不能重送同一個按鍵。

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use jc_proto::Request;
use jc_windows::{Client, Error as ClientError, LocalSocket};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HMODULE};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
    KEY_WOW64_64KEY, REG_VALUE_TYPE,
};
use windows::Win32::System::Threading::{
    CreateProcessW, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS, PROCESS_INFORMATION, STARTUPINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::logging;

/// 單次往返的逾時。
pub const REQUEST_TIMEOUT: Duration = Duration::from_millis(500);

/// 斷線後重試連線的最短間隔。
const RECONNECT_BACKOFF: Duration = Duration::from_secs(2);

/// 按需啟動的冷卻時間；避免連線一直失敗時重複開新行程。
const SPAWN_COOLDOWN: Duration = Duration::from_secs(3);

/// spawn 後等 pipe 出現的重試次數與間隔。
const PIPE_WAIT_ATTEMPTS: usize = 10;
const PIPE_WAIT_INTERVAL: Duration = Duration::from_millis(100);

/// MSI `EngineAutostart` 在 `HKLM\...\Run` 寫的值名稱。
const RUN_VALUE_NAME: &str = "jc-ime-engine";

/// 握手時送出的客戶端名稱。
const CLIENT_NAME: &str = concat!("jc-tsf/", env!("CARGO_PKG_VERSION"));

/// UI 執行緒收到「引擎有回應」的自訂訊息。
pub const WM_ENGINE_RESPONSE: u32 = WM_APP + 1;

/// 送給 worker 的一個請求。
pub struct Job {
    /// 送出時的按鍵世代；只有 `State` 會被這個值淘汰。
    pub generation: u64,
    /// 要送出的協定請求。
    pub request: Request,
}

/// worker 送回 UI 執行緒的結果。
pub struct Incoming {
    /// 對應的按鍵世代。
    pub generation: u64,
    /// 引擎回應或錯誤訊息。
    pub result: std::result::Result<jc_proto::Response, String>,
}

/// UI 與 worker 共用的回應佇列。
///
/// 只有一把鎖、一次處理一個請求，不需要額外的同步機制。
pub struct Queue {
    items: Mutex<VecDeque<Incoming>>,
    /// 訊息視窗的 handle；`0` 代表還沒建立。
    hwnd: AtomicIsize,
    /// 目前是否已連上引擎。
    connected: AtomicBool,
}

impl Queue {
    /// 建立空佇列。
    pub fn new() -> Self {
        Self {
            items: Mutex::new(VecDeque::new()),
            hwnd: AtomicIsize::new(0),
            connected: AtomicBool::new(false),
        }
    }

    /// 綁定訊息視窗，之後 worker 才能喚醒 UI 執行緒。
    pub fn attach(&self, hwnd: isize) {
        self.hwnd.store(hwnd, Ordering::Release);
    }

    /// 引擎是否已連線。
    ///
    /// 沒連線時 `OnTestKeyDown` 直接放行，讓使用者至少能打出原來的字母。
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    /// worker 端：推入一筆結果並喚醒 UI 執行緒。
    fn push(&self, item: Incoming) {
        if let Ok(mut items) = self.items.lock() {
            items.push_back(item);
        }
        let hwnd = self.hwnd.load(Ordering::Acquire);
        if hwnd != 0 {
            // SAFETY: hwnd 由 UI 執行緒在自己的窗口建立後寫入，DestroyWindow
            // 之前會先用 detach 清成 0。
            unsafe {
                let _ = PostMessageW(
                    Some(windows::Win32::Foundation::HWND(
                        hwnd as *mut std::ffi::c_void,
                    )),
                    WM_ENGINE_RESPONSE,
                    Default::default(),
                    Default::default(),
                );
            }
        }
    }

    /// UI 端：取出所有待處理結果。
    pub fn drain(&self) -> Vec<Incoming> {
        match self.items.lock() {
            Ok(mut items) => items.drain(..).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// 標記連線狀態。
    fn set_connected(&self, connected: bool) {
        self.connected.store(connected, Ordering::Release);
    }
}

impl Default for Queue {
    fn default() -> Self {
        Self::new()
    }
}

/// 啟動 worker 執行緒。
///
/// worker 沒有 join handle：`Deactivate` 把 sender 丟掉之後 channel 斷線，worker
/// 自己結束。這條執行緒不碰任何 COM 物件，只讀寫 pipe 與推佇列。
pub fn spawn(jobs: Receiver<Job>, queue: Arc<Queue>) {
    let _ = std::thread::Builder::new()
        .name("jc-tsf-engine".into())
        .spawn(move || run(jobs, queue));
}

/// worker 主迴圈。
fn run(jobs: Receiver<Job>, queue: Arc<Queue>) {
    // Activate 之後立刻試一次，失敗才進入 2 秒一次的退避。
    let mut client = connect(&queue);
    let mut last_attempt = Some(std::time::Instant::now());

    loop {
        // 沒連線時用 recv_timeout 定期重試；連線中就一直等到有工作或斷線。
        let job = if client.is_none() {
            match jobs.recv_timeout(RECONNECT_BACKOFF) {
                Ok(job) => Some(job),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match jobs.recv() {
                Ok(job) => Some(job),
                Err(_) => break,
            }
        };

        if client.is_none() {
            // 最多每 `RECONNECT_BACKOFF` 一次，不能在每個按鍵都阻塞 500 ms 握手。
            try_connect(&mut client, &mut last_attempt, &queue);
        }

        let Some(job) = job else {
            continue;
        };

        let Some(active) = client.as_mut() else {
            queue.push(Incoming {
                generation: job.generation,
                result: Err("引擎未連線".into()),
            });
            continue;
        };

        let result = match active.request(&job.request, REQUEST_TIMEOUT) {
            Ok(response) => Ok(response),
            Err(error) => {
                // 逾時或讀寫失敗都代表這條連線不可信，不能再送任何請求。
                logging::warn(format!("引擎請求失敗：{error}"));
                client = None;
                queue.set_connected(false);
                Err(error.to_string())
            }
        };

        queue.push(Incoming {
            generation: job.generation,
            result,
        });
    }

    queue.set_connected(false);
    logging::debug("引擎 worker 結束");
}

/// 有退避的重連：距離上次嘗試不到 [`RECONNECT_BACKOFF`] 就跳過。
fn try_connect(
    client: &mut Option<Client<LocalSocket>>,
    last_attempt: &mut Option<std::time::Instant>,
    queue: &Queue,
) {
    let due = last_attempt.is_none_or(|at| at.elapsed() >= RECONNECT_BACKOFF);
    if !due {
        return;
    }
    *last_attempt = Some(std::time::Instant::now());
    *client = connect(queue);
}

/// 連上引擎並握手；失敗只記日誌。
///
/// `NotFound`／`PipeBusy` 代表引擎不在或正在重新監聽。這條路徑會 detached spawn
/// 引擎，等 pipe 出現後再連一次。其餘錯誤（版本不符、訊框壞掉）不做這件事，
/// 因為引擎還活著，重開只會被 lockfile 擋下來。
fn connect(queue: &Queue) -> Option<Client<LocalSocket>> {
    let name = pipe_name();
    match Client::connect(&name, CLIENT_NAME, REQUEST_TIMEOUT) {
        Ok(client) => mark_connected(queue, &name, client),
        Err(error) => {
            logging::debug(format!("連線引擎 pipe {name} 失敗：{error}"));
            if connection_warrants_spawn(&error) && spawn_engine_throttled() {
                if let Some(client) = wait_for_pipe(&name) {
                    return mark_connected(queue, &name, client);
                }
            }
            queue.set_connected(false);
            None
        }
    }
}

/// 標記連線成功並回傳 client。
fn mark_connected(
    queue: &Queue,
    name: &str,
    client: Client<LocalSocket>,
) -> Option<Client<LocalSocket>> {
    logging::info(format!("已連上引擎 pipe：{name}"));
    queue.set_connected(true);
    Some(client)
}

/// 哪些連線錯誤值得按需啟動引擎。
///
/// - `ERROR_FILE_NOT_FOUND (2)`：pipe 不存在，引擎被 kill 了。
/// - `ERROR_PIPE_BUSY (231)`：引擎正在起來，舊 instance 也被短暫佔用。
/// - `ERROR_SEM_TIMEOUT (121)`：`WaitNamedPipeW` 等不到 instance，等於 busy。
fn connection_warrants_spawn(error: &ClientError) -> bool {
    match error {
        ClientError::Io(io) => {
            io.kind() == std::io::ErrorKind::NotFound
                || matches!(io.raw_os_error(), Some(2) | Some(121) | Some(231))
        }
        _ => false,
    }
}

/// spawn 後等 pipe 出現再連一次；有界重試，失敗不影響打字。
fn wait_for_pipe(name: &str) -> Option<Client<LocalSocket>> {
    for _ in 0..PIPE_WAIT_ATTEMPTS {
        std::thread::sleep(PIPE_WAIT_INTERVAL);
        if let Ok(client) = Client::connect(name, CLIENT_NAME, REQUEST_TIMEOUT) {
            logging::info(format!("按需啟動後已連上引擎 pipe：{name}"));
            return Some(client);
        }
    }
    logging::warn(format!("按需啟動後等不到引擎 pipe：{name}"));
    None
}

/// 按需啟動的冷卻狀態。
///
/// 行程內一把鎖就夠；跨行程的重複啟動由 supervisor 的 lockfile 擋掉。
struct SpawnCooldown {
    last: Mutex<Option<Instant>>,
}

impl SpawnCooldown {
    fn new() -> Self {
        Self {
            last: Mutex::new(None),
        }
    }

    /// 通過冷卻就記下時間並回 `true`，否則回 `false`。
    fn acquire(&self, now: Instant) -> bool {
        let mut last = self.last.lock().unwrap_or_else(|error| error.into_inner());
        if last.is_some_and(|at| now.saturating_duration_since(at) < SPAWN_COOLDOWN) {
            return false;
        }
        *last = Some(now);
        true
    }
}

static SPAWN_COOLDOWN_STATE: OnceLock<SpawnCooldown> = OnceLock::new();

fn spawn_cooldown() -> &'static SpawnCooldown {
    SPAWN_COOLDOWN_STATE.get_or_init(SpawnCooldown::new)
}

/// 在連線失敗時 detached spawn 引擎；任何失敗只記警告。
fn spawn_engine_throttled() -> bool {
    if !spawn_cooldown().acquire(Instant::now()) {
        logging::debug("引擎啟動冷卻中，略過 detached spawn");
        return false;
    }

    let command = match engine_command_line() {
        Ok(command) => command,
        Err(error) => {
            logging::warn(format!("無法建構引擎啟動指令，略過按需啟動：{error}"));
            return false;
        }
    };

    match launch_detached(&command) {
        Ok(()) => {
            logging::info(format!("已 detached spawn 引擎：{command}"));
            true
        }
        Err(error) => {
            logging::warn(format!("detached spawn 引擎失敗，打字不受影響：{error}"));
            false
        }
    }
}

/// 產生與 MSI `EngineAutostart` 一致的啟動指令。
///
/// 優先直接讀 `HKLM\Software\Microsoft\Windows\CurrentVersion\Run\jc-ime-engine`
/// 並展開環境變數，這保證指令與安裝器完全相同。機碼不存在時退回「TIP 自己的
/// DLL 所在目錄推 INSTALLFOLDER」與系統環境變數推資料／設定檔路徑。
fn engine_command_line() -> Result<String, String> {
    if let Some(command) = command_from_run_key() {
        return Ok(command);
    }

    let exe = module_dir()?.join("jc-engine.exe");
    let program_data =
        std::env::var_os("ProgramData").ok_or_else(|| "讀不到 ProgramData 環境變數".to_string())?;
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| "讀不到 LOCALAPPDATA 環境變數".to_string())?;
    let data = PathBuf::from(program_data).join("jc-ime").join("data");
    let settings = PathBuf::from(local_app_data)
        .join("jc-ime")
        .join("settings.json");
    Ok(build_command_line(&exe, &data, &settings))
}

/// 組合 `--supervise` 啟動指令；抽成純函式方便驗證參數順序與引號。
fn build_command_line(
    exe: &std::path::Path,
    data: &std::path::Path,
    settings: &std::path::Path,
) -> String {
    format!(
        "\"{}\" --supervise --data \"{}\" --settings \"{}\"",
        exe.display(),
        data.display(),
        settings.display()
    )
}

/// 讀取 ExpandEnvironmentStringsW 展開後的 Run 機值；失敗回 `None`。
fn command_from_run_key() -> Option<String> {
    let path = crate::wide(r"Software\Microsoft\Windows\CurrentVersion\Run");
    let name = crate::wide(RUN_VALUE_NAME);
    let mut key = HKEY::default();
    // SAFETY: path 以 NUL 結尾；key 在成功時由 RegOpenKeyExW 寫入。32 位元 TIP
    // 加 KEY_WOW64_64KEY 才會讀到 MSI 寫的 64 位元檢視。
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(path.as_ptr()),
            None,
            KEY_READ | KEY_WOW64_64KEY,
            &mut key,
        )
    };
    if status != ERROR_SUCCESS {
        logging::debug(format!("讀取 Run 機碼失敗，改用模組路徑推導：{}", status.0));
        return None;
    }

    let mut size = 0u32;
    let mut kind = REG_VALUE_TYPE::default();
    // SAFETY: key 由上面成功取得；此呼叫只問資料長度。
    let status = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            None,
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        // SAFETY: key 由 RegOpenKeyExW 取得，這裡是唯一放手的地方。
        unsafe {
            let _ = RegCloseKey(key);
        }
        logging::debug(format!("查詢 Run 值失敗，改用模組路徑推導：{}", status.0));
        return None;
    }

    let mut buffer = vec![0u16; (size as usize).div_ceil(2).max(1)];
    // SAFETY: buffer 的大小由上面的長度查詢決定。
    let status = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            Some(buffer.as_mut_ptr().cast::<u8>()),
            Some(&mut size),
        )
    };
    // SAFETY: key 由 RegOpenKeyExW 取得，這裡是唯一放手的地方。
    unsafe {
        let _ = RegCloseKey(key);
    }
    if status != ERROR_SUCCESS {
        logging::debug(format!(
            "讀取 Run 值內容失敗，改用模組路徑推導：{}",
            status.0
        ));
        return None;
    }

    let raw = String::from_utf16_lossy(&buffer[..(size as usize / 2).min(buffer.len())])
        .trim_end_matches('\0')
        .to_string();
    if raw.is_empty() {
        return None;
    }
    match expand_environment(&raw) {
        Ok(command) => Some(command),
        Err(error) => {
            logging::debug(format!("展開 Run 值失敗，改用模組路徑推導：{error}"));
            None
        }
    }
}

/// 展開 `%LOCALAPPDATA%` 之類的環境變數。
fn expand_environment(text: &str) -> Result<String, String> {
    let source = crate::wide(text);
    // SAFETY: source 以 NUL 結尾；第一次呼叫只問所需大小。
    let size = unsafe { ExpandEnvironmentStringsW(PCWSTR(source.as_ptr()), None) };
    if size == 0 {
        return Err("ExpandEnvironmentStringsW 查詢大小失敗".into());
    }
    let mut buffer = vec![0u16; size as usize];
    // SAFETY: buffer 的大小就是 Query 回來的大小（含結尾 NUL）。
    let written =
        unsafe { ExpandEnvironmentStringsW(PCWSTR(source.as_ptr()), Some(buffer.as_mut_slice())) };
    if written == 0 || written > buffer.len() as u32 {
        return Err("ExpandEnvironmentStringsW 回傳的長度異常".into());
    }
    Ok(String::from_utf16_lossy(&buffer[..(written as usize - 1)]))
}

/// 取得這份 DLL 所在的目錄，也就是 MSI 的 INSTALLFOLDER。
fn module_dir() -> Result<PathBuf, String> {
    let mut module = HMODULE::default();
    // SAFETY: 用本模組內的函式位址反查 DLL；UNCHANGED_REFCOUNT 不增減參考計數。
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(module_dir as *const () as *const u16),
            &mut module,
        )
        .map_err(|error| format!("GetModuleHandleExW 失敗：{error}"))?;
    }

    let mut buffer = vec![0u16; 512];
    loop {
        // SAFETY: buffer 的長度就是傳進去的 nSize；module 是上面取得的 DLL。
        let written = unsafe { GetModuleFileNameW(Some(module), &mut buffer) } as usize;
        if written == 0 {
            return Err("GetModuleFileNameW 失敗".into());
        }
        if written < buffer.len() {
            let path = PathBuf::from(String::from_utf16_lossy(&buffer[..written]));
            return path
                .parent()
                .map(std::path::Path::to_path_buf)
                .ok_or_else(|| "DLL 路徑沒有父目錄".into());
        }
        if buffer.len() >= 32768 {
            return Err("DLL 路徑過長".into());
        }
        buffer.resize(buffer.len() * 2, 0);
    }
}

/// 用 `CreateProcessW` + `DETACHED_PROCESS` 啟動引擎後立刻關 handle。
///
/// 引擎是獨立行程：TIP 結束不影響它，supervisor 的 lockfile 會擋掉重複啟動。
fn launch_detached(command: &str) -> Result<(), String> {
    let mut command_line = crate::wide(command);
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut process = PROCESS_INFORMATION::default();

    // SAFETY: command_line 以 NUL 結尾且可寫（CreateProcessW 會修改它）；
    // startup 的 cb 已設好，process 由 API 填入。
    unsafe {
        CreateProcessW(
            PCWSTR::null(),
            Some(PWSTR(command_line.as_mut_ptr())),
            None,
            None,
            false,
            DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP,
            None,
            PCWSTR::null(),
            &startup,
            &mut process,
        )
        .map_err(|error| format!("CreateProcessW 失敗：{error}"))?;

        // 放手：不等引擎、不用 job object 的 KILL_ON_JOB_CLOSE。
        let _ = CloseHandle(process.hProcess);
        let _ = CloseHandle(process.hThread);
    }
    Ok(())
}

/// 解析引擎 pipe 名稱，同一個行程只解析一次。
///
/// 規則與 `jc-engine` 共用 [`jc_windows::resolve_pipe_name`]：`JC_IME_PIPE` >
/// `HKCU\Software\jc-ime\EnginePipe` > `jc-ime-<SID 尾段>`，取不到 SID 才退回
/// [`jc_windows::BASE_PIPE_NAME`] 並記一次警告。
pub fn pipe_name() -> String {
    static RESOLVED: OnceLock<String> = OnceLock::new();

    RESOLVED
        .get_or_init(|| {
            let resolved = jc_windows::resolve_pipe_name();
            if let Some(warning) = &resolved.warning {
                logging::warn(warning.clone());
            }
            resolved.name
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// 冷卻時間內不重複 spawn，期滿才放行下一次。
    #[test]
    fn spawn_cooldown_allows_one_spawn_per_window() {
        let cooldown = SpawnCooldown::new();
        let start = Instant::now();
        assert!(cooldown.acquire(start));
        assert!(!cooldown.acquire(start + Duration::from_secs(2)));
        assert!(!cooldown.acquire(start + Duration::from_millis(2_999)));
        assert!(cooldown.acquire(start + Duration::from_secs(3)));
    }

    /// 指令格式要和 MSI 的 EngineAutostart 一致：supervise + data + settings。
    #[test]
    fn command_line_matches_msi_autostart() {
        let command = build_command_line(
            Path::new(r"C:\Program Files\jc-ime\jc-engine.exe"),
            Path::new(r"C:\ProgramData\jc-ime\data"),
            Path::new(r"C:\Users\jeff\AppData\Local\jc-ime\settings.json"),
        );
        assert_eq!(
            command,
            r#""C:\Program Files\jc-ime\jc-engine.exe" --supervise --data "C:\ProgramData\jc-ime\data" --settings "C:\Users\jeff\AppData\Local\jc-ime\settings.json""#
        );
    }

    /// 只有 pipe 不存在或忙碌才按需啟動；其他錯誤不重開引擎。
    #[test]
    fn only_missing_or_busy_pipe_triggers_spawn() {
        let missing = ClientError::Io(std::io::Error::from_raw_os_error(2));
        let busy = ClientError::Io(std::io::Error::from_raw_os_error(231));
        let wait_timeout = ClientError::Io(std::io::Error::from_raw_os_error(121));
        let denied = ClientError::Io(std::io::Error::from_raw_os_error(5));
        assert!(connection_warrants_spawn(&missing));
        assert!(connection_warrants_spawn(&busy));
        assert!(connection_warrants_spawn(&wait_timeout));
        assert!(!connection_warrants_spawn(&denied));
        assert!(!connection_warrants_spawn(&ClientError::Timeout));
    }

    /// 同一個行程內重複解析要拿到同一個名稱；解析結果會快取。
    #[test]
    fn pipe_name_is_stable_within_process() {
        let first = pipe_name();
        assert_eq!(first, pipe_name());
        assert!(!first.is_empty());
    }

    #[test]
    fn queue_marks_disconnected_by_default() {
        let queue = Queue::new();
        assert!(!queue.is_connected());
        queue.set_connected(true);
        assert!(queue.is_connected());
        queue.set_connected(false);
        assert!(!queue.is_connected());
    }
}
