//! 0c-server 的 named pipe 用戶端：4 位元組小端序長度 + JSON 的訊框（移植自 0c `tsf/src/ipc.rs`，main `8f443af`）。
//!
//! 與 0c 的差異（T1b／T2 範圍）：
//!
//! - **不經過 `interprocess`**：_client_ 自己用 Win32 `CreateFileW` 開啟管道，
//!   讀寫走 `ReadFile`／`WriteFile` + `OVERLAPPED` + 1 秒逾時。監聽端 SDDL
//!   也不流經 `interprocess::SecurityDescriptor::deserialize`（see [`PIPE_SDDL_TEMPLATE`]）。
//! - **spawn 保留 jc 做法**：用 [`std::process::Command::args()`] 而不是 0c 的字串
//!   組法，用 `CREATE_NO_WINDOW` 而不是 `DETACHED_PROCESS`（see [`spawn_engine`]）。
//! - **pipe 名稱用平行建置的新前綴** `jc-ime-next-<RID>`，與舊版 `jc-ime-<RID>` 隔開；
//!   boundary 由 [`server_pipe`] 決定，呼叫端也能用 [`Client::with_pipe`] 覆寫。
//!   資料格式維持 4-byte LE + JSON，回退後舊引擎讀得到同一份語料。
//! - 0c 的 `engine.json`／installer 版本目錄解析（`dll_dir`、`engine_dir`、
//!   `programs_dir`）依重做計畫捨棄，改由 adapter 用 [`Client::set_engine_command`]
//!   注入已解析好的引擎執行檔與 argv。
//!
//! 每次 I/O 都用 overlapped 加逾時：server 卡住時放棄連線並讓外殼放行按鍵，
//! 不得卡住宿主。

use std::env;
use std::ffi::c_void;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::Value;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, ERROR_PIPE_BUSY,
    HANDLE, HLOCAL, WAIT_OBJECT_0,
};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
    FILE_SHARE_MODE, OPEN_EXISTING,
};
use windows::Win32::System::Pipes::{PeekNamedPipe, WaitNamedPipeW};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, OpenProcessToken, WaitForSingleObject,
    CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

/// 記錄呼叫點。T1a 的 logging 尚未接入前先 no-op，保留 0c 的 zlog! 呼叫位置。
macro_rules! zlog {
    ($($arg:tt)*) => {{
        let _ = format_args!($($arg)*);
    }};
}

/// 每次 I/O 的逾時。
pub const IO_TIMEOUT_MS: u32 = 1000;
/// 啟動 server 後等待管道出現的時間。
pub const SPAWN_WAIT_MS: u32 = 5000;
/// 兩次啟動之間至少隔這麼久：server 反覆死掉時不會每鍵都啟動一次、也不會每鍵都等 5 秒。
pub const RESPAWN_INTERVAL: Duration = Duration::from_secs(10);
/// 引擎重啟時等新引擎可以服務的期限（#216），從發現舊引擎關掉連線、或舊引擎回「引擎正在切換」起算，
/// 只設定一次。期限內 `OnKeyDown` 等新引擎，不在 server 根本沒在跑時每鍵都等。
pub const RESTART_WAIT: Duration = Duration::from_secs(5);

/// 單一訊框上限：與 `jc_proto::frame::DEFAULT_MAX_FRAME` 相同（1 MiB）。
pub const MAX_FRAME: u32 = jc_proto::frame::DEFAULT_MAX_FRAME as u32;

/// 新版 DLL 的 pipe 前綴。定義在 `guids.rs`，與新 CLSID／描述同一份來源。
pub const PIPE_PREFIX: &str = crate::guids::PIPE_PREFIX_NEXT;
/// 舊版已發行的前綴；只給 rollback 與對照用，新版 TIP 不連它。
pub const OLD_PIPE_PREFIX: &str = crate::guids::PIPE_PREFIX_OLD;
/// 新版 TIP 專用的覆寫變數；不讀舊版 `JC_IME_PIPE`，避免舊引擎覆寫洩漏到新版。
pub const PIPE_ENV: &str = crate::guids::NEXT_PIPE_ENV;

/// 與 0c `transport_windows.go:76` 逐字相同的 SDDL；`{SID}` 由目前使用者的 SID 代入。
///
/// `D:P` 不繼承；本人完全存取；ALL APPLICATION PACKAGES（`S-1-15-2-1`）與
/// ALL RESTRICTED APPLICATION PACKAGES（`S-1-15-2-2`）可讀寫；`S:(ML;;NW;;;LW)`
/// 讓低完整性等級的宿主也能開啟。`interprocess` 在複製含 `G:` 的 SDDL 時有
/// os error 1307，所以監聽端必須直接使用這個字串，不經 interprocess。
pub const PIPE_SDDL_TEMPLATE: &str =
    "D:P(A;;GA;;;{SID})(A;;GRGW;;;S-1-15-2-1)(A;;GRGW;;;S-1-15-2-2)S:(ML;;NW;;;LW)";

/// 帶入目前使用者 SID 的 SDDL（大寫，與 0c `zuin_win_common::pipe_sddl` 一致）。
pub fn pipe_sddl() -> Option<String> {
    Some(PIPE_SDDL_TEMPLATE.replace("{SID}", &user_sid()?.to_uppercase()))
}

/// 以 NUL 結尾的 UTF-16。
pub fn wide(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

/// 訊框：4-byte 小端長度 + JSON，與 `jc_proto::frame`／0c `bridge/ipc` 同格式。
pub fn frame(v: &Value) -> Option<Vec<u8>> {
    let body = serde_json::to_vec(v).ok()?;
    jc_proto::frame::encode(&body).ok()
}

/// 目前使用者的 SID 字串（小寫），與 0c `user_sid` 相同做法。
pub fn user_sid() -> Option<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let mut len = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u8; len as usize];
        let r = GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr() as *mut c_void),
            len,
            &mut len,
        );
        let _ = CloseHandle(token);
        r.ok()?;
        let tu = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = PWSTR::null();
        ConvertSidToStringSidW(tu.User.Sid, &mut s).ok()?;
        let out = s.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(s.0 as *mut c_void)));
        out.map(|x| x.to_lowercase())
    }
}

/// 引擎 pipe 的完整 Win32 路徑：`\\.\pipe\<name>`。
///
/// T9 平行建置用新前綴解析： [`PIPE_ENV`]（`JC_IME_NEXT_PIPE`）覆寫 > SID 最後一段
/// （`jc-ime-next-<RID>`）> `jc-ime-next`。不讀舊版 `JC_IME_PIPE`，所以舊版引擎的
/// 覆寫值不會把新版 TIP 導到舊 pipe。資料格式仍是 4-byte LE + JSON，與舊版共用。
pub fn server_pipe() -> String {
    let name = env::var_os(PIPE_ENV)
        .map(|v| v.to_string_lossy().into_owned())
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            user_sid().and_then(|sid| {
                let rid = sid.rsplit('-').next().unwrap_or_default();
                (!rid.is_empty() && rid.bytes().all(|b| b.is_ascii_digit()))
                    .then(|| crate::guids::next_engine_pipe(rid))
            })
        })
        .unwrap_or_else(|| PIPE_PREFIX.to_string());
    format!(r"\\.\pipe\{name}")
}

/// 舊版管道名稱，只供對照與 rollback 文件使用。
///
/// jc 沒有 0c 的 `zuin-server`，新版 adapter 預設不連舊引擎；若要過渡期連它，
/// 用 [`Client::with_legacy_pipe`] 明確指定這個值。
pub fn legacy_server_pipe() -> String {
    format!(r"\\.\pipe\{OLD_PIPE_PREFIX}-legacy")
}

/// 引擎執行檔與 argv，由 adapter（T1a／session）解析後注入。
#[derive(Clone, Debug, Default)]
pub struct EngineCommand {
    /// `jc-engine.exe` 的完整路徑。
    pub exe: PathBuf,
    /// 傳給引擎的 argv，例如 `--socket ... --data ...`。
    pub args: Vec<String>,
}

pub struct Client {
    pipe: Vec<u16>,           // 以 NUL 結尾
    legacy: Option<Vec<u16>>, // 舊版管道；只在找不到新管道時嘗試
    handle: Option<HANDLE>,
    generation: u32,
    last_spawn: Option<Instant>,
    /// 預期引擎正在重啟、等它到這個時間（RESTART_WAIT）；成功回覆或呼叫端放棄時清掉。
    restart_until: Option<Instant>,
    /// 舊 writer 明確拒絕過（「引擎正在切換」）：等期間不自己啟動引擎。
    closing_writer: bool,
    /// 連不上時可以啟動安裝的引擎；只有日常的管道（`Client::new`）可以，測試用管道不啟動。
    may_start: bool,
    /// spawn 用的執行檔與 argv；`None` 代表只連已在執行的引擎。
    engine: Option<EngineCommand>,
}

impl Drop for Client {
    fn drop(&mut self) {
        self.close();
    }
}

/// 速率限制：距上次啟動不到 RESPAWN_INTERVAL 就不再啟動。
pub fn may_spawn(last: &mut Option<Instant>) -> bool {
    let now = Instant::now();
    if let Some(t) = *last {
        if now.duration_since(t) < RESPAWN_INTERVAL {
            return false;
        }
    }
    *last = Some(now);
    true
}

/// engine.json 的目錄欄位：只接受 `versions/<名稱>`，不含 `..`、反斜線或更深的層級。
///
/// 0c 相容核心更新仍沿用這條檢查；新 jc-tsf-next 不讀 engine.json，保留此純函式
/// 供 adapter 驗證外部提供的引擎目錄。
pub fn valid_engine_directory(dir: &str) -> bool {
    let mut parts = dir.split('/');
    matches!((parts.next(), parts.next(), parts.next()), (Some("versions"), Some(name), None)
        if !name.is_empty() && name != "." && name != ".." && !dir.contains('\\'))
}

impl Client {
    /// 用 jc 引擎的管道建立連線；必要時可啟動引擎。
    pub fn new() -> Client {
        Client::with_pipe(&server_pipe()).with_may_start(true)
    }

    /// 只用指定管道（測試用），不啟動引擎。
    pub fn with_pipe(name: &str) -> Client {
        Client {
            pipe: wide(name),
            legacy: None,
            handle: None,
            generation: 0,
            last_spawn: None,
            restart_until: None,
            closing_writer: false,
            may_start: false,
            engine: None,
        }
    }

    /// 指定升級過渡期要試的舊管道。
    pub fn with_legacy_pipe(mut self, name: &str) -> Self {
        self.legacy = Some(wide(name));
        self
    }

    /// 是否允許在連不上時啟動引擎；`Client::new()` 預設允許。
    pub fn with_may_start(mut self, may_start: bool) -> Self {
        self.may_start = may_start;
        self
    }

    /// 注入引擎執行檔與 argv；沒注入時 [`Client::connect`] 只連已在執行的引擎。
    pub fn set_engine_command(&mut self, command: EngineCommand) {
        self.engine = Some(command);
    }

    /// 每次重新連線後遞增；session 屬於某一代連線，代數變了就要重開並重播。
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// 引擎正在重啟，還在等新引擎的期限內（#216）。
    pub fn restart_pending(&self) -> bool {
        self.restart_until
            .is_some_and(|until| Instant::now() < until)
    }

    /// 等新引擎的期限（過了也照樣回報，直到成功或放棄）；呼叫端以第一次看到的期限為準。
    pub fn restart_deadline(&self) -> Option<Instant> {
        self.restart_until
    }

    /// 呼叫端不再等：下一次按鍵照一般情況連線。
    pub fn abandon_restart(&mut self) {
        self.restart_until = None;
        self.closing_writer = false;
    }

    /// 舊引擎關掉了這條連線：從現在起 RESTART_WAIT 內等它。已經在等就不延長。
    fn expect_restart(&mut self) {
        if self.restart_until.is_none() {
            self.restart_until = Some(Instant::now() + RESTART_WAIT);
        }
    }

    /// 剛失敗的把手是不是被另一端關掉（不是逾時）：引擎還在、只是慢，不當成重啟。
    fn peer_closed(&self) -> bool {
        let Some(h) = self.handle else {
            return false;
        };
        unsafe { PeekNamedPipe(h, None, 0, None, None, None) }.is_err()
    }

    /// I/O 失敗後關掉連線；另一端已經關掉時記下引擎正在重啟。
    fn close_after_failure(&mut self) {
        if self.peer_closed() {
            self.expect_restart();
        }
        self.close();
    }

    /// 目前是否持有連線（不代表引擎仍在；下一次呼叫才會發現斷線）。
    pub fn connected(&self) -> bool {
        self.handle.is_some()
    }

    /// 連線是否還在：不送請求、不等待，只看管道另一端關了沒有。斷了，或有不該有的
    /// 殘留資料（訊框會錯位），就關掉連線，下一次呼叫照常重連。
    pub fn alive(&mut self) -> bool {
        let Some(h) = self.handle else {
            return false;
        };
        let mut available = 0u32;
        let ok = unsafe { PeekNamedPipe(h, None, 0, None, Some(&mut available), None) }.is_ok();
        if !ok {
            self.expect_restart(); // 引擎關掉了連線：多半正在重啟（#216）
        }
        if !ok || available != 0 {
            self.close();
            return false;
        }
        true
    }

    /// 只連已經在執行的引擎，不啟動、不等待（Activate 時查開始時的模式，#160）。
    pub fn connect_existing(&mut self) -> bool {
        if self.handle.is_some() {
            return true;
        }
        if self.closing_writer && self.restart_pending() {
            return false;
        }
        self.connect_once().is_ok() || self.connect_legacy()
    }

    /// 放掉連線；下一次呼叫照常重連（代數會變，session 會重開）。
    pub fn release(&mut self) {
        self.close();
    }

    fn close(&mut self) {
        if let Some(h) = self.handle.take() {
            unsafe {
                let _ = CloseHandle(h);
            }
        }
    }

    fn connect_once(&mut self) -> Result<(), u32> {
        let pipe = self.pipe.clone();
        self.connect_to(&pipe)
    }

    /// 升級過渡期：舊版 server 還在執行時接上它，不另起第二個寫入者。
    fn connect_legacy(&mut self) -> bool {
        match self.legacy.clone() {
            Some(pipe) => self.connect_to(&pipe).is_ok(),
            None => false,
        }
    }

    /// 直接用 Win32 `CreateFileW` 開啟 overlapped 管道，完全不經過 `interprocess`。
    fn connect_to(&mut self, pipe: &[u16]) -> Result<(), u32> {
        unsafe {
            match CreateFileW(
                PCWSTR::from_raw(pipe.as_ptr()),
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            ) {
                Ok(h) => {
                    self.handle = Some(h);
                    self.generation = self.generation.wrapping_add(1);
                    Ok(())
                }
                Err(_) => Err(GetLastError().0),
            }
        }
    }

    /// 用 [`std::process::Command::args()`] + [`CREATE_NO_WINDOW`] 啟動引擎後立刻放手。
    ///
    /// `Command` 在 Windows 上會把每個 argv 正確跳脫後再呼叫 `CreateProcessW`，
    /// 不用自己處理 `lpCommandLine` 的引號規則（0c 用 `wide(&format!(...))` 字串組法，
    /// 空白路徑會出錯）。用 `CREATE_NO_WINDOW` 而非 `DETACHED_PROCESS`：後者只切斷父行程
    /// 的 console，Windows 仍會配置新 console 因而彈出終端機；前者保留 console handle
    /// 但不建立視窗。兩者互斥，只能選一個。
    fn spawn_engine(command: &EngineCommand) -> Option<u32> {
        let child = Command::new(&command.exe)
            .args(&command.args)
            .creation_flags((CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP).0)
            .spawn()
            .ok()?;
        // 不 wait、不用 job object 的 KILL_ON_JOB_CLOSE；drop child 只關我們這端的 handle。
        Some(child.id())
    }

    /// 啟動引擎；回傳是否真的啟動了（有速率限制）。
    /// 沙箱應用程式裡的 DLL 不能啟動程式，只能連上已在執行的引擎。
    fn spawn_server(&mut self) -> bool {
        if !self.may_start || !may_spawn(&mut self.last_spawn) {
            return false;
        }
        let Some(command) = self.engine.clone() else {
            zlog!("spawn: 未設定 EngineCommand，只連已在執行的引擎");
            return false;
        };
        match Self::spawn_engine(&command) {
            Some(pid) => {
                zlog!("spawn: pid={pid}");
                true
            }
            None => {
                zlog!("spawn: 失敗");
                false
            }
        }
    }

    /// 確保已連上；必要時啟動引擎。失敗回傳 false。
    pub fn connect(&mut self) -> bool {
        if self.handle.is_some() {
            return true;
        }
        if self.restart_pending() {
            // 引擎正在重啟（#216）：只試一次、不在這裡等，由 Session::handle 在期限內反覆試。
            // 舊 writer 拒絕過時接管者由更新器啟動：不要連回仍在關閉的 writer，也不自己啟動。
            return match self.connect_once() {
                Ok(()) => true,
                Err(code) if code == ERROR_FILE_NOT_FOUND.0 => {
                    if self.connect_legacy() {
                        return true;
                    }
                    if !self.closing_writer && self.spawn_server() {
                        zlog!("connect: 引擎重啟中，管道還不在，已啟動引擎");
                    }
                    false
                }
                Err(_) => self.connect_legacy(),
            };
        }
        if self.restart_until.is_some() {
            self.abandon_restart(); // 期限已過：這一次 server 不在就不等
        }
        for attempt in 0..2 {
            match self.connect_once() {
                Ok(()) => return true,
                Err(code) if code == ERROR_PIPE_BUSY.0 => {
                    unsafe {
                        let _ = WaitNamedPipeW(PCWSTR::from_raw(self.pipe.as_ptr()), IO_TIMEOUT_MS);
                    }
                    if self.connect_once().is_ok() {
                        return true;
                    }
                }
                Err(code) if code == ERROR_FILE_NOT_FOUND.0 && attempt == 0 => {
                    if self.connect_legacy() {
                        return true;
                    }
                    if !self.spawn_server() {
                        return false; // 沒有啟動就不等：server 不在時每鍵立刻放行
                    }
                    // 等 server 建好管道；沒等到則之後的呼叫在同一個期限內繼續等，不再多等一輪。
                    let deadline = Instant::now() + Duration::from_millis(SPAWN_WAIT_MS as u64);
                    self.restart_until = Some(deadline);
                    while Instant::now() < deadline {
                        if self.connect_once().is_ok() {
                            return true;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }
                Err(code) => {
                    zlog!("connect: 失敗 code={code}");
                    return false;
                }
            }
        }
        false
    }

    /// 送出一個 Envelope 並讀回 Reply。連線失敗回傳 None，並關閉連線（下次呼叫會重連）。
    pub fn call(&mut self, envelope: &Value) -> Option<Value> {
        if !self.connect() {
            return None;
        }
        let bytes = frame(envelope)?;
        let h = self.handle?;
        if !write_all(h, &bytes, IO_TIMEOUT_MS) {
            zlog!("call: 寫入失敗，斷線");
            self.close_after_failure();
            return None;
        }
        let mut hdr = [0u8; 4];
        if !read_all(h, &mut hdr, IO_TIMEOUT_MS) {
            zlog!("call: 讀取失敗，斷線");
            self.close_after_failure();
            return None;
        }
        let n = u32::from_le_bytes(hdr);
        if n > MAX_FRAME {
            self.close();
            return None;
        }
        let mut buf = vec![0u8; n as usize];
        if !read_all(h, &mut buf, IO_TIMEOUT_MS) {
            self.close_after_failure();
            return None;
        }
        match serde_json::from_slice::<Value>(&buf) {
            Ok(v) => {
                if v["ok"].as_bool() == Some(false) && v["error"].as_str() == Some("引擎正在切換")
                {
                    // 此回覆表示請求完全未執行，重試不會重複提交。只設定一次期限。
                    self.expect_restart();
                    self.closing_writer = true;
                    self.close();
                } else if v["ok"].as_bool() == Some(true) {
                    self.abandon_restart(); // 新引擎接上了
                }
                Some(v)
            }
            Err(_) => {
                self.close();
                None
            }
        }
    }
}

/// 一次 overlapped I/O：回傳完成的位元組數；逾時或錯誤回傳 None。
fn overlapped(
    h: HANDLE,
    timeout_ms: u32,
    op: impl FnOnce(*mut OVERLAPPED) -> windows::core::Result<()>,
) -> Option<u32> {
    unsafe {
        let event = CreateEventW(None, true, false, PCWSTR::null()).ok()?;
        let mut ov = OVERLAPPED {
            hEvent: event,
            ..Default::default()
        };
        let started = op(&mut ov);
        let mut done = 0u32;
        let result = match started {
            Ok(()) => GetOverlappedResult(h, &ov, &mut done, false)
                .ok()
                .map(|_| done),
            Err(_) if GetLastError() == ERROR_IO_PENDING => {
                if WaitForSingleObject(event, timeout_ms) == WAIT_OBJECT_0 {
                    GetOverlappedResult(h, &ov, &mut done, false)
                        .ok()
                        .map(|_| done)
                } else {
                    let _ = CancelIoEx(h, Some(&ov));
                    let _ = GetOverlappedResult(h, &ov, &mut done, true);
                    None
                }
            }
            Err(_) => None,
        };
        let _ = CloseHandle(event);
        result
    }
}

/// 把整個緩衝寫完，每次 overlapped 都有逾時。
pub fn write_all(h: HANDLE, mut data: &[u8], timeout_ms: u32) -> bool {
    while !data.is_empty() {
        let chunk = data;
        let n = overlapped(h, timeout_ms, |ov| unsafe {
            WriteFile(h, Some(chunk), None, Some(ov))
        });
        match n {
            Some(n) if n > 0 => data = &data[n as usize..],
            _ => return false,
        }
    }
    true
}

fn read_all(h: HANDLE, mut buf: &mut [u8], timeout_ms: u32) -> bool {
    while !buf.is_empty() {
        let n = {
            let chunk: &mut [u8] = buf;
            overlapped(h, timeout_ms, |ov| unsafe {
                ReadFile(h, Some(chunk), None, Some(ov))
            })
        };
        match n {
            Some(n) if n > 0 => buf = &mut buf[n as usize..],
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod engine_pointer_tests {
    use super::valid_engine_directory;

    #[test]
    fn only_direct_version_directories_are_accepted() {
        assert!(valid_engine_directory("versions/0.1.0-dev.18-abc"));
        for bad in [
            "../x",
            "versions/../x",
            "versions/a/b",
            "other/x",
            "versions\\x",
            "versions/",
            "versions/.",
            "versions/..",
            "versions",
        ] {
            assert!(!valid_engine_directory(bad), "{bad}");
        }
    }
}
