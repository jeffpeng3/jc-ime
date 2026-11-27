//! 引擎行程的請求／回應迴圈。
//!
//! 這一層只處理協定與行程行為；組字與轉換全部交給 `jc_core::Session`。
//! 每次請求都回傳完整快照，wrapper 不需要自己保存狀態。
//!
//! 資料載入在 [`loader`]；`jc-cli` 共用同一份載入程式碼。
//!
//! T4 的並行模型：`Session` 本身不是 `Send`（核心的 `ja::live::Config` 與
//! `Controller` 含 `Rc`／`Box<dyn>` 測試注入點），所以 session 留在其所屬
//! worker thread 的 [`LocalState`]，永遠不跨 thread 移動。全行程共享的
//! [`Registry`] 只放 metadata 與一個 `SessionFactory`，兩者都是 `Send + Sync`。
//! 這樣既不需要改 jc-core，也讓 Mutex 臨界區完全避開 `engineDo` 的長計算。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

use jc_core::personal::{PersonalError, PersonalStore};
use jc_core::session::{Session, SessionConfig};
use jc_core::{KeyCode, Mode, Snapshot};
use jc_settings::keybinds::Chord;

pub mod config;
mod config_watch;
pub mod loader;
mod personal_merge;
pub mod supervise;
use jc_proto::message::{read_message, ProtoError};
use jc_proto::{
    write_message, ErrorCode, HealthInfo, Request, Response, MAX_LITERAL_BYTES, PROTOCOL_VERSION,
};

/// 回報給 wrapper 的引擎識別字串。
pub fn engine_label() -> String {
    format!("jc-engine {}", env!("CARGO_PKG_VERSION"))
}

/// 一條連線的識別字。session 歸屬檢查以其為單位。
pub type ConnId = u64;

/// stdio 與直接使用 [`Engine::handle`] 的呼叫端所用的連線 id。
pub const DEFAULT_CONN: ConnId = 0;

/// 個人學習資料的預設檔名前綴；已發行的舊引擎用這個值。
pub const DEFAULT_PERSONAL_PREFIX: &str = "personal";

/// 活動中週期寫回的間隔。避免打字途中崩潰遺失尚未 Close 的學習。
const PERSONAL_FLUSH_INTERVAL: Duration = Duration::from_secs(30);

/// 超過這個耗時就記一行慢請求；正常的按鍵不寫 log，避免洗版。
const SLOW_REQUEST: Duration = Duration::from_millis(200);

/// 最近一次慢請求的快照；health 用它讓 doctor 與 TSF log 不必翻引擎 log。
#[derive(Debug, Clone)]
struct SlowRequest {
    op: String,
    ms: u64,
    input_len: usize,
    at: SystemTime,
}

/// 中斷一條連線的回呼。drain／shutdown 時由 watcher 跨連線廣播。
///
/// Unix 端呼叫 `shutdown(fd, SHUT_RDWR)`，Windows 端呼叫 `CancelIoEx`，
/// 讓阻塞在 read／ConnectNamedPipe 的 worker 結束，不必引入 async runtime。
pub type DisconnectFn = Arc<dyn Fn() + Send + Sync>;

/// 建立 session 的工廠。在 worker thread 內呼叫，回傳的 `Session` 不跨 thread。
pub type SessionFactory = Arc<dyn Fn() -> Session + Send + Sync>;

/// drain watcher 的 polling 間隔。
const DRAIN_POLL: Duration = Duration::from_millis(50);

/// 一條 session 的共享 metadata。真正的 `Session` 在持有它的 worker 裡。
#[derive(Debug, Clone)]
struct SessionMeta {
    owner: ConnId,
    composing: bool,
    last_active: Instant,
    /// 已處理過的請求。連線重開、generation 改變時供上層重播（replay 載體）。
    log: Vec<Request>,
}

/// 全行程共享的引擎狀態。Mutex 臨界區只有 metadata 讀寫與旗標切換，
/// 不含任何 `Session::press()`／`Session::commit()` 長計算。
pub struct Registry {
    next_session: u64,
    next_conn: u64,
    factory: SessionFactory,
    meta: HashMap<u64, SessionMeta>,
    /// 每條連線目前使用的 session id（不帶 id 的 key／toggle 走這裡）。
    current: HashMap<ConnId, u64>,
    /// 每條連線自己持有的 session id set；歸屬檢查用。
    owned: HashMap<ConnId, HashSet<u64>>,
    disconnects: HashMap<ConnId, DisconnectFn>,
    /// accept 迴圈手上空閒 pipe instance 的中斷回呼（Windows 用）。
    listener_disconnect: Option<DisconnectFn>,
    draining: bool,
    closing: bool,
    shutdown: bool,
    drain_done: bool,
    /// ResetPersonal 代次；worker 發現代次變化時重建自己的 session。
    personal_epoch: u64,
    /// 全行程唯一的個人學習資料。跨 session、跨連線共享同一份。
    personal_store: Arc<Mutex<PersonalStore>>,
    /// 啟動時是否成功從前綴對應的檔案讀回資料。Health 據此回報真實情形。
    personal_loaded: bool,
    /// 外殼需要的額外能力旗標（中英切換鍵、簡繁即時切換…）。外殼只認
    /// `info[name] == "true"`，少了這些它會退回保守預設：左 Shift 不切換、
    /// 輸出簡體字按了沒反應。放在 Registry 才讓每條連線的 health 都看得到。
    extra_caps: BTreeMap<String, bool>,
    /// 新 session 的簡體輸出預設值；來自 `settings.json` 與轉換器是否可用。
    default_output_simplified: bool,
    /// 啟動時的開機模式（`english`／`zhuyin`）；外殼讀 health 的 `start_mode`。
    start_mode: Option<String>,
    /// 正在處理的請求數；drain 要等它與 composing 都歸零。
    active: usize,
    last_key: Option<Instant>,
    /// 最近一次超過 [`SLOW_REQUEST`] 的請求；跨連線共享，health 直接回報。
    last_slow: Option<SlowRequest>,
    /// 設定檔路徑；watcher 與 `ApplySettings` 都用它重讀。
    settings_path: Option<PathBuf>,
    /// 設定檔的平台代號。
    settings_goos: String,
    /// CLI 覆寫；熱重載時要一起重算生效設定。
    settings_overrides: config::Overrides,
    /// 最近一次成功套用的生效設定；`None` 表示這個引擎沒有設定來源。
    settings_effective: Option<config::EffectiveSettings>,
    /// 正規化 chord → action。
    settings_keymap: BTreeMap<Chord, &'static str>,
    /// action → 正規化 chord；協定 `bindings` 直接回這個。
    settings_bindings: BTreeMap<String, String>,
    /// 成功套用的次數。
    settings_revision: u64,
    /// 最近一次熱重載失敗的原因。
    settings_error: Option<String>,
    /// watcher 上次看到的設定檔戳記。
    settings_stamp: Option<config_watch::Stamp>,
}

impl Registry {
    fn new(factory: SessionFactory) -> Self {
        Self {
            next_session: 1,
            next_conn: 1,
            factory,
            meta: HashMap::new(),
            current: HashMap::new(),
            owned: HashMap::new(),
            disconnects: HashMap::new(),
            listener_disconnect: None,
            draining: false,
            closing: false,
            shutdown: false,
            drain_done: false,
            personal_epoch: 0,
            personal_store: Arc::new(Mutex::new(PersonalStore::new())),
            personal_loaded: false,
            extra_caps: BTreeMap::new(),
            default_output_simplified: false,
            start_mode: None,
            active: 0,
            last_key: None,
            last_slow: None,
            settings_path: None,
            settings_goos: config::platform_goos().to_string(),
            settings_overrides: config::Overrides::default(),
            settings_effective: None,
            settings_keymap: BTreeMap::new(),
            settings_bindings: BTreeMap::new(),
            settings_revision: 0,
            settings_error: None,
            settings_stamp: None,
        }
    }

    fn broadcast_disconnects(&self) -> Vec<DisconnectFn> {
        let mut out: Vec<DisconnectFn> = self.disconnects.values().cloned().collect();
        if let Some(listener) = &self.listener_disconnect {
            out.push(Arc::clone(listener));
        }
        out
    }
}

/// 保護共享 metadata 的 Mutex lock。中毒時取回內部資料，避免一條 worker
/// panic 就拖垮全行程。
pub(crate) fn lock(registry: &Arc<Mutex<Registry>>) -> MutexGuard<'_, Registry> {
    registry.lock().unwrap_or_else(|err| err.into_inner())
}

/// 請求在處理期間持有；drop 時把 `Registry::active` 減一。
struct ActiveGuard(Arc<Mutex<Registry>>);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        let mut registry = lock(&self.0);
        registry.active = registry.active.saturating_sub(1);
    }
}

/// 只屬於一條 worker thread 的 session。永遠不跨 thread 移動。
struct LocalState {
    sessions: HashMap<u64, Session>,
    /// 各 session 建立時從共用 store 取得的快照。回寫時用來算增量，避免重複累加。
    personal_baseline: HashMap<u64, PersonalStore>,
    epoch: u64,
    /// 這個 worker 已套用的設定 revision。
    settings_revision: u64,
}

/// 可跨 thread 遞送的連線憑證。worker 在自己的 thread 呼叫 [`Connection::into_engine`]
/// 建立引擎視圖，因此 `Session` 從頭到尾都待在同一條 thread。
pub struct Connection {
    shared: Arc<Mutex<Registry>>,
    conn: ConnId,
    personal_dir: Option<PathBuf>,
    /// 個人資料檔名前綴，跟著連線進 worker。
    personal_prefix: String,
}

impl Connection {
    /// 在 worker thread 內建立這條連線的引擎視圖。
    pub fn into_engine(self) -> Engine {
        let epoch = lock(&self.shared).personal_epoch;
        Engine {
            local: LocalState {
                sessions: HashMap::new(),
                personal_baseline: HashMap::new(),
                epoch,
                settings_revision: 0,
            },
            shared: self.shared,
            conn: self.conn,
            hello: false,
            close_connection: false,
            personal_dir: self.personal_dir,
            personal_prefix: self.personal_prefix,
            last_personal_flush: Instant::now(),
        }
    }

    /// 註冊這條連線的中斷回呼；drain／shutdown 時呼叫。
    pub fn set_disconnect(&self, disconnect: DisconnectFn) {
        lock(&self.shared).disconnects.insert(self.conn, disconnect);
    }
}

/// 協定引擎。組字狀態全部在 worker 自己持有的 [`Session`] 裡。
pub struct Engine {
    local: LocalState,
    shared: Arc<Mutex<Registry>>,
    conn: ConnId,
    hello: bool,
    close_connection: bool,
    /// 個人資料存檔目錄；`None`（stdio／測試）時不存檔。
    personal_dir: Option<PathBuf>,
    /// 個人資料檔名前綴；平行建置用不同前綴避免新舊互寫。
    personal_prefix: String,
    /// 上次把 session 增量寫回共用的時間；用來做活動中的週期存檔。
    last_personal_flush: Instant,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// 建立一個尚未握手、沒有額外資料的引擎。
    pub fn new() -> Self {
        Self::with_factory(Arc::new(|| Session::new(SessionConfig::default())), None)
    }

    /// 用 session 工廠建立引擎；工廠會在每個 worker thread 內呼叫。
    pub fn with_factory(factory: SessionFactory, personal_dir: Option<PathBuf>) -> Self {
        Self::with_factory_personal(
            factory,
            personal_dir,
            Arc::new(Mutex::new(PersonalStore::new())),
        )
    }

    /// 與 [`Engine::with_factory`] 相同，但共用呼叫端提供的個人學習資料。
    ///
    /// 主要給需要事先把初始資料放進共用的嵌入端使用；一般行程用
    /// [`Engine::load_personal`] 從檔案讀回即可。
    pub fn with_factory_personal(
        factory: SessionFactory,
        personal_dir: Option<PathBuf>,
        personal_store: Arc<Mutex<PersonalStore>>,
    ) -> Self {
        let mut registry = Registry::new(factory);
        registry.personal_store = personal_store;
        let shared = Arc::new(Mutex::new(registry));
        shared
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .owned
            .entry(DEFAULT_CONN)
            .or_default();
        Self {
            local: LocalState {
                sessions: HashMap::new(),
                personal_baseline: HashMap::new(),
                epoch: 0,
                settings_revision: 0,
            },
            shared,
            conn: DEFAULT_CONN,
            hello: false,
            close_connection: false,
            personal_dir,
            personal_prefix: DEFAULT_PERSONAL_PREFIX.to_string(),
            last_personal_flush: Instant::now(),
        }
    }

    /// 設定個人資料檔名前綴；不改變資料根目錄，只改檔名。
    ///
    /// 預設是 `"personal"`（`personal.json`），自訂前綴存成 `<prefix>.json`。
    /// 必須在 `accept()` 或 `load_personal()` 之前呼叫。
    pub fn set_personal_prefix(&mut self, prefix: impl Into<String>) {
        self.personal_prefix = prefix.into();
    }

    /// 個人資料的完整檔名；由前綴決定。
    fn personal_path(&self) -> Option<PathBuf> {
        self.personal_dir
            .as_ref()
            .map(|dir| dir.join(format!("{}.json", self.personal_prefix)))
    }

    /// 啟動時讀回共用個人資料。檔案不存在或損壞時使用空白基準，不阻礙啟動。
    ///
    /// 讀檔只看目前前綴對應的單一檔案（預設 `personal.json`），舊版 per-session
    /// 檔（`personal-<id>.json`）不碰。全新安裝，檔案不存在就用空白基準。
    pub fn load_personal(&mut self) {
        let Some(path) = self.personal_path() else {
            return;
        };
        match PersonalStore::load(&path) {
            Ok(store) => {
                let shared = Arc::clone(&self.shared);
                let mut registry = lock(&shared);
                *registry
                    .personal_store
                    .lock()
                    .unwrap_or_else(|err| err.into_inner()) = store;
                registry.personal_loaded = true;
                log::info!("讀回個人資料：{}", path.display());
            }
            Err(PersonalError::Io(err)) if err.kind() == io::ErrorKind::NotFound => {
                log::info!("個人資料檔不存在，使用空白基準：{}", path.display());
            }
            Err(err) => {
                log::warn!("讀取個人資料失敗，使用空白基準：{}：{err}", path.display());
            }
        }
    }

    /// 共用個人資料的快照；診斷與測試用。
    pub fn personal_store(&self) -> PersonalStore {
        let registry = lock(&self.shared);
        let copied = registry
            .personal_store
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        copied
    }

    /// 用指定的工作階段建立引擎；session 裝在預設連線上，id 為 1。
    pub fn with_session(session: Session) -> Self {
        let mut engine = Self::new();
        {
            let mut registry = lock(&engine.shared);
            let id = registry.next_session;
            registry.next_session += 1;
            registry.owned.entry(DEFAULT_CONN).or_default().insert(id);
            registry.current.insert(DEFAULT_CONN, id);
            registry.meta.insert(
                id,
                SessionMeta {
                    owner: DEFAULT_CONN,
                    composing: false,
                    last_active: Instant::now(),
                    log: Vec::new(),
                },
            );
        }
        let baseline = session.personal().clone();
        engine.local.sessions.insert(1, session);
        engine.local.personal_baseline.insert(1, baseline);
        engine
    }

    /// 為一條新 accept 的連線建立憑證，共享同一份 registry。
    pub fn accept(&self) -> Connection {
        let conn = {
            let mut registry = lock(&self.shared);
            let conn = registry.next_conn;
            registry.next_conn += 1;
            registry.owned.insert(conn, HashSet::new());
            conn
        };
        Connection {
            shared: Arc::clone(&self.shared),
            conn,
            personal_dir: self.personal_dir.clone(),
            personal_prefix: self.personal_prefix.clone(),
        }
    }

    /// 註冊 accept 迴圈空閒 pipe instance 的中斷回呼（Windows 用）。
    pub fn set_listener_disconnect(&self, disconnect: DisconnectFn) {
        lock(&self.shared).listener_disconnect = Some(disconnect);
    }

    /// 空閒 pipe 已連上使用者，清掉中斷回呼，避免誤砍 worker 的 I/O。
    pub fn clear_listener_disconnect(&self) {
        lock(&self.shared).listener_disconnect = None;
    }

    /// 連線結束：把這條連線所有 session 的學習合併回共用 store 並存檔。
    pub fn finish_connection(&mut self) {
        let ids: Vec<u64> = {
            let mut registry = lock(&self.shared);
            registry
                .owned
                .remove(&self.conn)
                .map(|set| set.into_iter().collect())
                .unwrap_or_default()
        };
        for id in ids {
            self.flush_session(id);
            self.local.sessions.remove(&id);
            self.local.personal_baseline.remove(&id);
            let mut registry = lock(&self.shared);
            registry.current.remove(&self.conn);
            registry.meta.remove(&id);
        }
        lock(&self.shared).disconnects.remove(&self.conn);
        self.save_global();
        self.close_connection = true;
    }

    /// 目前的組字字串；沒有 session 時回空字串。
    pub fn composition(&self) -> String {
        let id = lock(&self.shared).current.get(&self.conn).copied();
        match id.and_then(|id| self.local.sessions.get(&id)) {
            Some(session) => session.preedit(),
            None => String::new(),
        }
    }

    /// 目前 session 的輸入模式；沒有 session 時回 `Zh`。
    pub fn mode(&self) -> Mode {
        let id = lock(&self.shared).current.get(&self.conn).copied();
        match id.and_then(|id| self.local.sessions.get(&id)) {
            Some(session) => session.mode(),
            None => Mode::Mixed,
        }
    }

    /// 目前連線持有的 session id；診斷與 replay 用。
    pub fn session_ids(&self) -> Vec<u64> {
        let registry = lock(&self.shared);
        registry
            .owned
            .get(&self.conn)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// 讀取一個 session 的請求 log（replay 載體）；別條連線讀不到。
    pub fn session_log(&self, session: u64) -> Vec<Request> {
        let registry = lock(&self.shared);
        match registry.meta.get(&session) {
            Some(meta) if meta.owner == self.conn => meta.log.clone(),
            _ => Vec::new(),
        }
    }

    /// 把來源 session 的 log 重播到同連線的另一個 session，回傳每個請求的回應。
    ///
    /// 重播是跨連線搬移組字狀態：直接從 registry 拿 log，不走 [`Self::session_log`]
    /// 的連線歸屬檢查。歸屬檢查只擋外部讀取，不擋引擎自己把 log 放進新 session。
    pub fn replay_into(&mut self, from: u64, to: u64) -> Vec<Response> {
        let log = {
            let registry = lock(&self.shared);
            registry
                .meta
                .get(&from)
                .map(|meta| meta.log.clone())
                .unwrap_or_default()
        };
        let keymap = lock(&self.shared).settings_keymap.clone();
        // 重播是重建畫面，不是新的使用者輸入：log 裡的 Enter／選字不能再寫
        // personal 或改統計。先暫停學習，結束後恢復原值。
        let previous = self
            .local
            .sessions
            .get_mut(&to)
            .map(|session| session.set_suppress_learn(true));
        let replies = log
            .into_iter()
            .map(|request| {
                let logged = request.clone();
                self.apply_to_session(to, Some(request), |session| match logged {
                    Request::Key { key } => {
                        if let Some(action) = config_watch::chord_of_key(&key)
                            .and_then(|chord| keymap.get(&chord).copied())
                        {
                            let _ = session.command(action);
                            state(session.snapshot())
                        } else {
                            state(session.press(key))
                        }
                    }
                    Request::Literal { text, .. } => state(session.literal(&text)),
                    Request::Idle { .. } => state(session.ja_live_idle()),
                    Request::Commit => Response::Committed {
                        text: session.commit(),
                    },
                    Request::Snapshot => state(session.snapshot()),
                    Request::Select { index } => match session.choose_candidate(index) {
                        Some(snapshot) => state(snapshot),
                        None => Response::Acknowledged { session: 0 },
                    },
                    Request::OutputSimplified { on } => {
                        session.set_output_simplified(on);
                        state(session.snapshot())
                    }
                    other => {
                        log::debug!("replay 略過非組字請求：{other:?}");
                        Response::Acknowledged { session: 0 }
                    }
                })
            })
            .collect();
        if let (Some(previous), Some(session)) = (previous, self.local.sessions.get_mut(&to)) {
            session.set_suppress_learn(previous);
        }
        replies
    }

    /// 這一條連線是否該收掉（版本不符、`Shutdown`、drain 完成）。
    pub fn is_closing(&self) -> bool {
        self.close_connection || lock(&self.shared).closing
    }

    /// 是否收到 `Shutdown`，整個行程要結束。
    pub fn is_shutdown(&self) -> bool {
        lock(&self.shared).shutdown
    }

    /// drain 是否已完成存檔並斷開所有連線；accept 迴圈據此結束。
    pub fn is_drain_done(&self) -> bool {
        lock(&self.shared).drain_done
    }

    /// 這條連線目前的 session id；還沒握手時為 0。慢請求 log 用。
    pub fn current_session(&self) -> u64 {
        lock(&self.shared)
            .current
            .get(&self.conn)
            .copied()
            .unwrap_or(0)
    }

    /// 記下最近一次慢請求：寫一行 log，並放進共用的 health 快照。
    fn note_slow_request(&self, op: &str, elapsed: Duration, input_len: usize) {
        let slow = SlowRequest {
            op: op.to_string(),
            ms: elapsed.as_millis() as u64,
            input_len,
            at: SystemTime::now(),
        };
        log::warn!(
            "慢請求 op={} session={} 耗時={}ms 輸入長度={} 時間={:?}",
            slow.op,
            self.current_session(),
            slow.ms,
            slow.input_len,
            slow.at
        );
        lock(&self.shared).last_slow = Some(slow);
    }

    /// 清除單一連線的握手狀態並重置它目前的 session。
    ///
    /// `Shutdown` 是全行程旗標，不會被清掉；`closing` 只是「這條連線要收掉」的
    /// 旗標，重置後這條連線重新可握手，所以一併清掉。
    pub fn reset_session(&mut self) {
        self.hello = false;
        lock(&self.shared).closing = false;
        let id = lock(&self.shared).current.get(&self.conn).copied();
        if let Some(id) = id {
            if let Some(session) = self.local.sessions.get_mut(&id) {
                session.reset();
            }
            let mut registry = lock(&self.shared);
            if let Some(meta) = registry.meta.get_mut(&id) {
                meta.log.clear();
                meta.composing = false;
            }
        }
        self.close_connection = false;
    }

    /// 處理一個請求。任何請求都會得到一個回應。
    pub fn handle(&mut self, request: Request) -> Response {
        self.sync_epoch();
        self.sync_settings();
        if self.last_personal_flush.elapsed() >= PERSONAL_FLUSH_INTERVAL {
            self.last_personal_flush = Instant::now();
            self.flush_all();
        }
        match request {
            Request::Hello { protocol, client } => {
                if protocol != PROTOCOL_VERSION {
                    self.close_connection = true;
                    return Response::error(
                        ErrorCode::VersionMismatch,
                        format!("協定版本不符：客戶端 {protocol}，引擎 {PROTOCOL_VERSION}"),
                    );
                }
                self.hello = true;
                self.open_session();
                log::info!("客戶端 {} 完成握手（協定 {protocol}）", client);
                // 把實際配置的 id 回給 adapter；它記住後用在同一連線的 Close。
                let (session, bindings) = {
                    let registry = lock(&self.shared);
                    (
                        registry.current.get(&self.conn).copied().unwrap_or(0),
                        registry.settings_bindings.clone(),
                    )
                };
                Response::Ready {
                    protocol: PROTOCOL_VERSION,
                    engine: engine_label(),
                    session,
                    bindings,
                }
            }
            Request::Key { key } => {
                if !self.hello {
                    return not_handshaked();
                }
                // 快捷鍵先查 keymap：命中就由引擎執行 action，外殼只負責轉發。
                if key.ctrl || key.alt || key.shift {
                    let action = config_watch::chord_of_key(&key)
                        .and_then(|chord| lock(&self.shared).settings_keymap.get(&chord).copied());
                    if let Some(action) = action {
                        lock(&self.shared).last_key = Some(Instant::now());
                        let logged = Some(Request::Key { key });
                        return self.with_current(logged, |session| {
                            let _ = session.command(action);
                            Response::state_with_handled(session.snapshot(), Some(true))
                        });
                    }
                }
                let enters = key.code == KeyCode::Enter;
                let tab = key.code == KeyCode::Tab;
                // Alt 組合鍵是新的快捷鍵空間；沒命中就明確說沒處理，讓外殼放行。
                // 不要先 press：Ctrl+Alt+, 之類的組合若還有 Ctrl 標點副作用，
                // 會變成「引擎沒處理」但字元已插入，與放行語意矛盾。
                if key.alt {
                    lock(&self.shared).last_key = Some(Instant::now());
                    return self.with_current(None, |session| {
                        Response::state_with_handled(session.snapshot(), Some(false))
                    });
                }
                lock(&self.shared).last_key = Some(Instant::now());
                let logged = Some(Request::Key { key });
                self.with_current(logged, |session| {
                    // 這一鍵進來前是否在組字。刪除鍵把組字刪光時 preedit 會變空，
                    // 外殼不能再靠「preedit 非空」推斷這一鍵有沒有被吃掉。候選窗／
                    // 選單開著時 preedit 也是空，所以要和 TSF 的 `composing()` 一樣
                    // 把 candidates 算進來。
                    let before = session.snapshot();
                    let composing = before.composing();
                    let deleting = matches!(key.code, KeyCode::Backspace | KeyCode::Delete);
                    let snapshot = session.press(key);
                    if tab && !session.last_key_consumed() {
                        return Response::state_with_handled(snapshot, Some(false));
                    }
                    if enters {
                        // 候選窗開著時的 Enter 只套用高亮候選、關窗，組字還留著；
                        // 這時 commit 會把整段組字直接送出去，等組字真的結束
                        // （第二次 Enter 或沒有候選窗）才走原本的送出。
                        if snapshot.composing() {
                            return Response::state_with_handled(snapshot, Some(true));
                        }
                        let text = session.commit();
                        if !text.is_empty() {
                            return Response::Committed { text };
                        }
                    }
                    if composing && deleting {
                        return Response::state_with_handled(snapshot, Some(true));
                    }
                    state(snapshot)
                })
            }
            Request::Literal { text, session: _ } => {
                if !self.hello {
                    return not_handshaked();
                }
                if text.len() > MAX_LITERAL_BYTES {
                    return Response::error(
                        ErrorCode::BadFrame,
                        format!("literal 超過 {MAX_LITERAL_BYTES} bytes：{}", text.len()),
                    );
                }
                lock(&self.shared).last_key = Some(Instant::now());
                self.with_current(
                    Some(Request::Literal {
                        text: text.clone(),
                        session: 0,
                    }),
                    |s| state(s.literal(&text)),
                )
            }
            Request::Idle { session } => {
                if !self.hello {
                    return not_handshaked();
                }
                // 外殼的停頓計時器：沒有日文組字時 ja_live_idle 只回目前快照。
                self.with_current(Some(Request::Idle { session }), |session| {
                    state(session.ja_live_idle())
                })
            }
            Request::Snapshot => {
                if !self.hello {
                    return not_handshaked();
                }
                self.with_current(None, |session| state(session.snapshot()))
            }
            Request::Commit => {
                if !self.hello {
                    return not_handshaked();
                }
                self.with_current(Some(Request::Commit), |session| Response::Committed {
                    text: session.commit(),
                })
            }
            Request::Select { index } => {
                if !self.hello {
                    return not_handshaked();
                }
                self.with_current(Some(Request::Select { index }), |session| {
                    match session.choose_candidate(index) {
                        Some(snapshot) => state(snapshot),
                        // 越界是合法的「點到已消失的候選」：保留目前組字，
                        // 但明確標成沒處理，外殼才不會清掉畫面。
                        None => Response::state_with_handled(session.snapshot(), Some(false)),
                    }
                })
            }
            Request::OutputSimplified { on } => {
                if !self.hello {
                    return not_handshaked();
                }
                self.with_current(Some(Request::OutputSimplified { on }), |session| {
                    session.set_output_simplified(on);
                    state(session.snapshot())
                })
            }
            Request::Close { session } => self.close_session(session),
            Request::Ping => Response::Acknowledged { session: 0 },
            Request::Health => Response::Health {
                info: self.health(),
            },
            Request::ApplySettings { settings } => {
                log::info!("收到 {} 項設定；立即從設定檔重讀並套用", settings.len());
                if let Err(err) = self.reload_settings() {
                    log::warn!("套用設定失敗：{err}");
                }
                Response::Health {
                    info: self.health(),
                }
            }
            Request::Drain => {
                self.flush_all();
                self.begin_drain();
                Response::Health {
                    info: self.health(),
                }
            }
            Request::ResetPersonal => {
                {
                    let mut registry = lock(&self.shared);
                    registry.personal_epoch += 1;
                }
                self.sync_epoch();
                Response::Acknowledged { session: 0 }
            }
            Request::Shutdown => {
                self.flush_all();
                self.begin_shutdown();
                self.snapshot()
            }
            Request::Unknown => Response::error(ErrorCode::UnknownOp, "未知的操作"),
            // 0c 的 vocab 等 op 在 T6 接手前回明確的「不支援」，讓 adapter
            // 能區分協定不認識與引擎尚未實作。
            other => Response::error(ErrorCode::Unsupported, format!("jc 尚未支援 {other:?}")),
        }
    }

    /// 目前的完整快照。
    fn snapshot(&mut self) -> Response {
        self.with_current(None, |session| state(session.snapshot()))
    }

    /// 為這條連線開一個 session；已有 current session 時直接沿用。
    ///
    /// id 從 1 開始，並由 [`Request::Hello`] 的 Ready 回給 adapter。第一條連線
    /// 拿到 1（與舊引擎相容），第二條以後拿到 2、3…；adapter 記住自己的 id 後
    /// 用在同一連線的 Close。key／toggle／commit 請求不帶 id，一律走 current。
    fn open_session(&mut self) {
        let mut registry = lock(&self.shared);
        if let Some(id) = registry.current.get(&self.conn).copied() {
            if registry
                .meta
                .get(&id)
                .is_some_and(|meta| meta.owner == self.conn)
            {
                return;
            }
        }
        let id = registry.next_session;
        registry.next_session += 1;
        let seed = registry
            .personal_store
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        let mut session = (registry.factory)();
        session.set_personal_store(seed.clone());
        session.set_output_simplified(registry.default_output_simplified);
        if let Some(effective) = registry.settings_effective.clone() {
            let config = config::session_config(session.config_snapshot(), &effective);
            session.apply_config(config);
        }
        self.local.personal_baseline.insert(id, seed);
        self.local.sessions.insert(id, session);
        registry.owned.entry(self.conn).or_default().insert(id);
        registry.current.insert(self.conn, id);
        registry.meta.insert(
            id,
            SessionMeta {
                owner: self.conn,
                composing: false,
                last_active: Instant::now(),
                log: Vec::new(),
            },
        );
    }

    /// 關閉一個 session；別條連線的 id 回 error（0c `server.go:322-324`）。
    fn close_session(&mut self, session: u64) -> Response {
        {
            let registry = lock(&self.shared);
            let Some(meta) = registry.meta.get(&session) else {
                return ownership_error();
            };
            if meta.owner != self.conn {
                return ownership_error();
            }
        }
        self.flush_session(session);
        self.local.sessions.remove(&session);
        self.local.personal_baseline.remove(&session);
        {
            let mut registry = lock(&self.shared);
            registry.meta.remove(&session);
            if let Some(owned) = registry.owned.get_mut(&self.conn) {
                owned.remove(&session);
            }
            if registry.current.get(&self.conn) == Some(&session) {
                registry.current.remove(&self.conn);
            }
        }
        self.save_global();
        Response::Acknowledged { session }
    }

    /// 對這條連線目前的 session 套用 `f`。不持任何共享 Mutex 計算。
    fn with_current<F>(&mut self, request: Option<Request>, f: F) -> Response
    where
        F: FnOnce(&mut Session) -> Response,
    {
        let id = match lock(&self.shared).current.get(&self.conn).copied() {
            Some(id) => id,
            None => return not_handshaked(),
        };
        self.apply_to_session(id, request, f)
    }

    /// 對指定 session 套用 `f`；shared Mutex 只做歸屬檢查、active 加減與 metadata 更新。
    fn apply_to_session<F>(&mut self, id: u64, request: Option<Request>, f: F) -> Response
    where
        F: FnOnce(&mut Session) -> Response,
    {
        {
            let registry = lock(&self.shared);
            if !registry
                .meta
                .get(&id)
                .is_some_and(|meta| meta.owner == self.conn)
            {
                return ownership_error();
            }
        }
        {
            let mut registry = lock(&self.shared);
            registry.active += 1;
        }
        let _active = ActiveGuard(Arc::clone(&self.shared));
        let Some(session) = self.local.sessions.get_mut(&id) else {
            return ownership_error();
        };
        if let Some(request) = request {
            lock(&self.shared)
                .meta
                .get_mut(&id)
                .expect("歸屬檢查已通過")
                .log
                .push(request);
        }
        let response = f(session);
        let composing = !session.preedit().is_empty();
        {
            let mut registry = lock(&self.shared);
            if let Some(meta) = registry.meta.get_mut(&id) {
                meta.composing = composing;
                meta.last_active = Instant::now();
            }
        }
        response
    }

    /// 發現 personal epoch 改變時，清空共用資料，並用 factory 重建自己的 session。
    fn sync_epoch(&mut self) {
        let (epoch, factory) = {
            let registry = lock(&self.shared);
            (registry.personal_epoch, Arc::clone(&registry.factory))
        };
        if epoch == self.local.epoch {
            return;
        }
        self.local.epoch = epoch;
        {
            let mut registry = lock(&self.shared);
            *registry
                .personal_store
                .lock()
                .unwrap_or_else(|err| err.into_inner()) = PersonalStore::new();
            registry.personal_loaded = false;
        }
        let ids: Vec<u64> = self.local.sessions.keys().copied().collect();
        for id in ids {
            self.local.sessions.insert(id, factory());
            self.local
                .personal_baseline
                .insert(id, PersonalStore::new());
            let mut registry = lock(&self.shared);
            if let Some(meta) = registry.meta.get_mut(&id) {
                meta.log.clear();
                meta.composing = false;
            }
        }
        self.save_global();
    }

    /// 補上外殼需要的能力旗標；`main` 依生效設定呼叫一次。
    ///
    /// 寫進共用的 Registry，所有連線的 Health 都看得到。
    pub fn set_extra_caps(&mut self, caps: BTreeMap<String, bool>) {
        lock(&self.shared).extra_caps = caps;
    }

    /// 安裝啟動時的設定與熱重載來源。
    ///
    /// `base` 保留資料依賴（詞典、模型），熱重載只換由設定檔決定的欄位。
    pub fn set_settings(
        &mut self,
        effective: config::EffectiveSettings,
        path: Option<PathBuf>,
        goos: &str,
        overrides: config::Overrides,
    ) {
        let keymap = effective.keybinds.chords().unwrap_or_default();
        let bindings = effective.bindings.clone();
        let stamp = path.as_deref().and_then(config_watch::stamp);
        {
            let mut registry = lock(&self.shared);
            registry.settings_keymap = keymap;
            registry.settings_bindings = bindings;
            registry.settings_effective = Some(effective);
            registry.settings_path = path;
            registry.settings_goos = goos.to_string();
            registry.settings_overrides = overrides;
            registry.settings_stamp = stamp;
        }
        // 已經有 session 的嵌入端要立刻套用；一般啟動時還沒開 session。
        self.local.settings_revision = u64::MAX;
        self.sync_settings();
    }

    /// 設定 watcher 的起始戳記。
    ///
    /// `main` 在 `config::load` 當下就記下戳記；資料載入可能花十幾秒，若用
    /// `set_settings` 當下的 stat 當基準，啟動期間的變更會被 watcher 漏掉。
    pub fn set_settings_stamp(&self, stamp: Option<(u64, u64)>) {
        lock(&self.shared).settings_stamp = stamp;
    }

    /// 啟動每秒 stat 設定檔的 watcher；沒有設定檔時什麼都不做。
    pub fn watch_settings(&self) {
        config_watch::spawn(Arc::clone(&self.shared));
    }

    /// 立刻從檔案重讀設定並套用到這條 worker 的 session。
    pub fn reload_settings(&mut self) -> Result<u64, String> {
        let revision = config_watch::reload(&self.shared)?;
        self.sync_settings();
        Ok(revision)
    }

    /// 目前成功套用的設定版次；診斷與測試用。
    pub fn settings_revision(&self) -> u64 {
        lock(&self.shared).settings_revision
    }

    /// 把共用 registry 的最新設定套用到這條 worker 的 session。
    ///
    /// `Session` 不跨 thread，所以其他 worker 會在自己的下一次請求進來時套用。
    fn sync_settings(&mut self) {
        let (revision, effective) = {
            let registry = lock(&self.shared);
            (
                registry.settings_revision,
                registry.settings_effective.clone(),
            )
        };
        if revision == self.local.settings_revision {
            return;
        }
        self.local.settings_revision = revision;
        let Some(effective) = effective else {
            return;
        };
        for session in self.local.sessions.values_mut() {
            let config = config::session_config(session.config_snapshot(), &effective);
            session.apply_config(config);
        }
    }

    /// 設定開機模式；外殼從 health 的 `start_mode` 判斷是否直接英文。
    pub fn set_start_mode(&mut self, mode: impl Into<String>) {
        lock(&self.shared).start_mode = Some(mode.into());
    }

    /// 設定新 session 的簡體輸出預設值；`main` 依生效設定與轉換器可用性呼叫一次。
    pub fn set_default_output_simplified(&mut self, on: bool) {
        lock(&self.shared).default_output_simplified = on;
    }

    /// 實際健康數值，對應 0c `Reply.Info`。
    fn health(&self) -> HealthInfo {
        let registry = lock(&self.shared);
        let sessions = registry.meta.len();
        let composing = registry.meta.values().filter(|meta| meta.composing).count();
        let mut capabilities = BTreeMap::new();
        capabilities.insert("personal_shared".to_string(), true);
        capabilities.insert("personal_persist".to_string(), self.personal_dir.is_some());
        capabilities.insert("personal_loaded".to_string(), registry.personal_loaded);
        capabilities.extend(registry.extra_caps.clone());
        let output_simplified = registry
            .current
            .get(&self.conn)
            .and_then(|id| self.local.sessions.get(id))
            .map_or(registry.default_output_simplified, |session| {
                session.output_simplified()
            });
        let english_words = registry
            .current
            .get(&self.conn)
            .and_then(|id| self.local.sessions.get(id))
            .map_or(0, Session::english_words);
        let last_slow = registry.last_slow.as_ref();
        HealthInfo {
            draining: registry.draining || registry.closing,
            // conn 0 是 stdio／測試用，不計入實際 socket 連線。
            connections: registry.owned.len().saturating_sub(1),
            idle_seconds: registry.last_key.map(|at| at.elapsed().as_secs()),
            sessions,
            composing,
            pid: Some(std::process::id()),
            output_simplified,
            english_words,
            // 0c 把 personal_reset 當成「支援清除學習紀錄」的能力旗標；
            // 這份引擎有 reset 指令且會把空白寫回檔案，所以回 true。
            personal_reset: true,
            start_mode: registry.start_mode.clone(),
            capabilities,
            last_slow_op: last_slow.map(|slow| slow.op.clone()),
            last_slow_ms: last_slow.map(|slow| slow.ms),
            last_slow_input_len: last_slow.map(|slow| slow.input_len),
            bindings: registry.settings_bindings.clone(),
            settings_revision: registry.settings_revision,
            settings_error: registry.settings_error.clone(),
        }
    }

    /// drain：標記 draining，接著由背景 watcher 等 composing／active 歸零。
    ///
    /// 成立時先中斷所有連線；worker 結束時各自存檔個人資料。外殼重連時接到
    /// 新版引擎並重播 log。若外殼不重連，accept 迴圈在自己的中斷回呼下醒來。
    fn begin_drain(&self) {
        {
            let mut registry = lock(&self.shared);
            if registry.draining || registry.shutdown {
                return;
            }
            registry.draining = true;
        }
        let shared = Arc::clone(&self.shared);
        let spawned = std::thread::Builder::new()
            .name("jc-engine-drain".into())
            .spawn(move || {
                loop {
                    let ready = {
                        let registry = lock(&shared);
                        registry.shutdown
                            || (registry.active == 0
                                && registry.meta.values().all(|meta| !meta.composing))
                    };
                    if ready {
                        break;
                    }
                    std::thread::sleep(DRAIN_POLL);
                }
                let disconnects = {
                    let mut registry = lock(&shared);
                    registry.closing = true;
                    registry.drain_done = true;
                    registry.broadcast_disconnects()
                };
                for disconnect in disconnects {
                    disconnect();
                }
                log::info!("drain 完成：已中斷所有連線");
            });
        if let Err(err) = spawned {
            log::warn!("啟動 drain watcher 失敗：{err}");
            lock(&self.shared).draining = false;
        }
    }

    /// shutdown：全行程廣播，立刻中斷所有連線；worker 結束時各自存檔。
    fn begin_shutdown(&self) {
        let disconnects = {
            let mut registry = lock(&self.shared);
            if registry.shutdown {
                return;
            }
            registry.shutdown = true;
            registry.closing = true;
            registry.broadcast_disconnects()
        };
        for disconnect in disconnects {
            disconnect();
        }
    }

    /// 把一條 session 相對基準的學習增量合併回共用 store，並前進基準。
    ///
    /// 只在這條 worker thread 內讀 session，Mutex 只包住短暫的合併，沒有
    /// `engineDo` 長計算。基準前進很重要：下一次週期或 Close 不再重複加同一筆。
    fn flush_session(&mut self, id: u64) {
        let Some(session) = self.local.sessions.get(&id) else {
            return;
        };
        let Some(baseline) = self.local.personal_baseline.get(&id).cloned() else {
            return;
        };
        let learned = session.personal().clone();
        let shared = Arc::clone(&self.shared);
        let global = Arc::clone(&lock(&shared).personal_store);
        {
            let mut store = global.lock().unwrap_or_else(|err| err.into_inner());
            personal_merge::merge_learning(&mut store, &baseline, &learned);
        }
        if let Some(current) = self.local.personal_baseline.get_mut(&id) {
            *current = learned;
        }
    }

    /// 把這條連線所有 session 的學習合併回共用 store 並存檔。
    fn flush_all(&mut self) {
        let ids: Vec<u64> = self.local.sessions.keys().copied().collect();
        for id in ids {
            self.flush_session(id);
        }
        self.save_global();
    }

    /// 把共用 store 寫到前綴對應的單一檔案，原子替換由 `Store::save` 處理。
    fn save_global(&self) {
        let Some(path) = self.personal_path() else {
            return;
        };
        let shared = Arc::clone(&self.shared);
        let global = Arc::clone(&lock(&shared).personal_store);
        let store = global.lock().unwrap_or_else(|err| err.into_inner()).clone();
        if let Err(err) = store.save(&path) {
            log::warn!("存檔個人資料 {} 失敗：{err}", path.display());
        }
    }
}

fn not_handshaked() -> Response {
    Response::error(ErrorCode::Internal, "尚未完成握手")
}

fn ownership_error() -> Response {
    Response::error(ErrorCode::Internal, "session 不存在或不屬於這個連線")
}

/// 把核心快照轉成協定回應。
fn state(snapshot: Snapshot) -> Response {
    Response::state(snapshot)
}

/// 請求的 op 名稱，慢請求 log 用。
fn request_op(request: &Request) -> &'static str {
    match request {
        Request::Hello { .. } => "hello",
        Request::Key { .. } => "key",
        Request::Literal { .. } => "literal",
        Request::Idle { .. } => "idle",
        Request::Snapshot => "snapshot",
        Request::Commit => "commit",
        Request::Select { .. } => "select",
        Request::Shutdown => "shutdown",
        Request::Ping => "ping",
        Request::Health => "health",
        Request::ApplySettings { .. } => "apply_settings",
        Request::Drain => "drain",
        Request::Vocab { .. } => "vocab",
        Request::ResetPersonal => "reset_personal",
        Request::Close { .. } => "close",
        Request::OutputSimplified { .. } => "output_simplified",
        Request::Unknown => "unknown",
    }
}

/// 請求的線路長度（JSON 位元組），慢請求 log 用。只計數，不配置緩衝。
fn request_input_len(request: &Request) -> usize {
    struct Count(usize);
    impl Write for Count {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0 += buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    serde_json::to_writer(&mut count, request).map_or(0, |()| count.0)
}

/// 服務一條連線直到 EOF、`Shutdown`、drain 或協定錯誤。
pub fn serve<R: Read, W: Write>(
    engine: &mut Engine,
    input: &mut R,
    output: &mut W,
    max_frame: usize,
) -> Result<(), ServeError> {
    loop {
        match read_message::<_, Request>(input, max_frame) {
            Ok(None) => return Ok(()),
            Ok(Some(request)) => {
                let op = request_op(&request);
                let input_len = request_input_len(&request);
                let started = Instant::now();
                let response = engine.handle(request);
                let elapsed = started.elapsed();
                if elapsed > SLOW_REQUEST {
                    engine.note_slow_request(op, elapsed, input_len);
                }
                write_message(output, &response, max_frame)?;
                if engine.is_closing() {
                    return Ok(());
                }
            }
            // socket 模式設了 receive timeout，用來取代跨 thread shutdown：
            // 逾時時先看全域收線旗標，還沒收線就繼續等下一個訊框。
            Err(ProtoError::Io(err))
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                if engine.is_closing() {
                    return Ok(());
                }
                continue;
            }
            Err(err) => {
                log::warn!("協定錯誤，結束連線：{err}");
                let response = Response::error(err.code(), err.to_string());
                write_message(output, &response, max_frame)?;
                return Ok(());
            }
        }
    }
}

/// 迴圈層級的錯誤。
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// 回應寫不出去。
    #[error("寫出回應失敗：{0}")]
    Write(#[from] ProtoError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc_core::Key;
    use std::io::Cursor;

    fn hello(engine: &mut Engine) {
        let response = engine.handle(Request::Hello {
            protocol: PROTOCOL_VERSION,
            client: "test".into(),
        });
        assert!(matches!(response, Response::Ready { .. }));
    }

    /// 用標準鍵盤把一個注音讀音送進引擎。
    fn type_syllable(engine: &mut Engine, reading: &str) {
        let keyboard = jc_core::zh::keyboard::Keyboard::standard();
        for syllable in jc_core::zh::syllable::parse_reading(reading) {
            let keys = keyboard.keys(&syllable).expect("音節應該有對應按鍵");
            for ch in keys.chars() {
                engine.handle(Request::Key {
                    key: Key::char(ch, false),
                });
            }
        }
    }

    fn state_of(preedit: &str) -> Response {
        Response::state(Snapshot {
            rows: jc_core::candidate_layout::DEFAULT_ROWS,
            columns: jc_core::candidate_layout::VERTICAL_COLUMNS,
            ..Snapshot::from_preedit(preedit)
        })
    }

    #[test]
    fn version_mismatch_closes_after_error() {
        let mut engine = Engine::new();
        let response = engine.handle(Request::Hello {
            protocol: PROTOCOL_VERSION + 1,
            client: "old".into(),
        });
        assert_eq!(
            response,
            Response::error(
                ErrorCode::VersionMismatch,
                format!(
                    "協定版本不符：客戶端 {}，引擎 {}",
                    PROTOCOL_VERSION + 1,
                    PROTOCOL_VERSION
                )
            )
        );
        assert!(engine.is_closing());
        assert!(!engine.is_shutdown());
    }

    #[test]
    fn key_edits_composition_and_commit_takes_it() {
        let mut engine = Engine::new();
        hello(&mut engine);
        engine.handle(Request::Key {
            key: Key::char('ㄇ', false),
        });
        engine.handle(Request::Key {
            key: Key::char('ㄚ', false),
        });
        assert_eq!(engine.handle(Request::Snapshot), state_of("ㄇㄚ"));
        assert_eq!(
            engine.handle(Request::Commit),
            Response::Committed {
                text: "ㄇㄚ".into()
            }
        );
        assert_eq!(engine.composition(), "");
        assert_eq!(engine.handle(Request::Snapshot), state_of(""));
    }

    /// 停頓觸發在沒有日文組字時只回目前快照，不改狀態。
    #[test]
    fn idle_without_japanese_composition_is_a_noop() {
        let mut engine = Engine::new();
        hello(&mut engine);
        engine.handle(Request::Key {
            key: Key::char('ㄇ', false),
        });
        let before = engine.handle(Request::Snapshot);
        assert_eq!(engine.handle(Request::Idle { session: 0 }), before);
    }

    #[test]
    fn literal_appends_without_candidates_and_rejects_oversize() {
        let mut engine = Engine::new();
        hello(&mut engine);
        engine.handle(Request::Key {
            key: Key::char('ㄇ', false),
        });
        engine.handle(Request::Key {
            key: Key::char('ㄚ', false),
        });
        let response = engine.handle(Request::Literal {
            text: "hello".into(),
            session: 0,
        });
        let Response::State {
            preedit,
            candidates,
            suggestion,
            ..
        } = response
        else {
            panic!("literal 要回 State");
        };
        assert_eq!(preedit, "ㄇㄚhello");
        assert!(candidates.is_empty());
        assert!(suggestion.is_none());

        let too_long = "a".repeat(jc_proto::MAX_LITERAL_BYTES + 1);
        let response = engine.handle(Request::Literal {
            text: too_long,
            session: 0,
        });
        let Response::Error { code, .. } = response else {
            panic!("超長 literal 要回錯誤");
        };
        assert_eq!(code, ErrorCode::BadFrame);
        assert_eq!(engine.composition(), "ㄇㄚhello");
    }

    #[test]
    fn backspace_removes_one_character() {
        let mut engine = Engine::new();
        hello(&mut engine);
        for ch in "注音".chars() {
            engine.handle(Request::Key {
                key: Key::char(ch, false),
            });
        }
        engine.handle(Request::Key {
            key: Key::new(KeyCode::Backspace, None, false),
        });
        assert_eq!(engine.composition(), "注");
    }

    #[test]
    fn test_backspace_emptying_composition_is_still_handled() {
        let mut engine = Engine::new();
        hello(&mut engine);
        engine.handle(Request::Literal {
            text: "ab".into(),
            session: 0,
        });
        assert_eq!(engine.composition(), "ab");

        let Response::State {
            preedit, handled, ..
        } = engine.handle(Request::Key {
            key: Key::new(KeyCode::Backspace, None, false),
        })
        else {
            panic!("組字中 Backspace 應回 State");
        };
        assert_eq!(preedit, "a");
        assert_eq!(handled, Some(true), "組字中按 Backspace 要明確回 handled");

        let Response::State {
            preedit, handled, ..
        } = engine.handle(Request::Key {
            key: Key::new(KeyCode::Backspace, None, false),
        })
        else {
            panic!("組字中 Backspace 應回 State");
        };
        assert_eq!(preedit, "");
        assert_eq!(handled, Some(true), "刪掉最後一個字元的那一鍵仍是 handled");
        assert_eq!(engine.composition(), "");

        let Response::State { handled, .. } = engine.handle(Request::Key {
            key: Key::new(KeyCode::Backspace, None, false),
        }) else {
            panic!("沒有組字時 Backspace 應回 State");
        };
        assert_ne!(handled, Some(true), "沒有組字時不能搶應用程式的刪除鍵");
    }

    #[test]
    fn test_delete_emptying_composition_is_still_handled() {
        let mut engine = Engine::new();
        hello(&mut engine);
        type_syllable(&mut engine, "ㄨㄛˇ");
        type_syllable(&mut engine, "ㄊㄞˊ");
        assert_eq!(engine.composition(), "ㄨㄛˇㄊㄞˊ");

        // Delete 刪游標後的字；先把游標移到兩個音節之間。
        engine.handle(Request::Key {
            key: Key::new(KeyCode::ArrowLeft, None, false),
        });
        let Response::State {
            preedit, handled, ..
        } = engine.handle(Request::Key {
            key: Key::new(KeyCode::Delete, None, false),
        })
        else {
            panic!("組字中 Delete 應回 State");
        };
        assert_eq!(preedit, "ㄨㄛˇ");
        assert_eq!(handled, Some(true), "組字中按 Delete 要明確回 handled");

        engine.handle(Request::Key {
            key: Key::new(KeyCode::ArrowLeft, None, false),
        });
        let Response::State {
            preedit, handled, ..
        } = engine.handle(Request::Key {
            key: Key::new(KeyCode::Delete, None, false),
        })
        else {
            panic!("組字中 Delete 應回 State");
        };
        assert_eq!(preedit, "");
        assert_eq!(handled, Some(true), "刪掉最後一個字元的那一鍵仍是 handled");
        assert_eq!(engine.composition(), "");

        let Response::State { handled, .. } = engine.handle(Request::Key {
            key: Key::new(KeyCode::Delete, None, false),
        }) else {
            panic!("沒有組字時 Delete 應回 State");
        };
        assert_ne!(handled, Some(true), "沒有組字時不能搶應用程式的刪除鍵");
    }

    #[test]
    fn test_backspace_closes_menu_without_deleting_document_text() {
        let mut engine = engine_with_settings(None);
        hello(&mut engine);
        let response = engine.handle(Request::Key {
            key: Key::char(',', false).with_ctrl().with_alt(),
        });
        assert_eq!(handled(&response), Some(true));
        let Response::State {
            preedit,
            candidates,
            ..
        } = &response
        else {
            panic!("開選單應回 State");
        };
        assert_eq!(preedit, "", "選單開著時 preedit 是空的");
        assert!(!candidates.is_empty(), "選單要有候選");
        let id = engine.current_session();
        assert!(
            engine.local.sessions[&id].menu_tab().is_some(),
            "引擎要真的開選單"
        );

        let response = engine.handle(Request::Key {
            key: Key::new(KeyCode::Backspace, None, false),
        });
        assert_eq!(
            handled(&response),
            Some(true),
            "選單開著時的 Backspace 是輸入法吃掉的一鍵"
        );
        let Response::State {
            preedit,
            candidates,
            ..
        } = response
        else {
            panic!("關選單應回 State");
        };
        assert_eq!(preedit, "", "關選單不能改動 preedit");
        assert!(candidates.is_empty(), "Backspace 應關掉選單");
        assert!(
            engine.local.sessions[&id].menu_tab().is_none(),
            "Backspace 後選單要關閉"
        );
    }

    #[test]
    fn test_delete_closes_menu_without_forward_delete() {
        let mut engine = engine_with_settings(None);
        hello(&mut engine);
        let response = engine.handle(Request::Key {
            key: Key::char(',', false).with_ctrl().with_alt(),
        });
        assert_eq!(handled(&response), Some(true));
        let Response::State {
            preedit,
            candidates,
            ..
        } = &response
        else {
            panic!("開選單應回 State");
        };
        assert_eq!(preedit, "");
        assert!(!candidates.is_empty());
        let id = engine.current_session();
        assert!(engine.local.sessions[&id].menu_tab().is_some());

        let response = engine.handle(Request::Key {
            key: Key::new(KeyCode::Delete, None, false),
        });
        assert_eq!(
            handled(&response),
            Some(true),
            "選單開著時的 Delete 是輸入法吃掉的一鍵"
        );
        let Response::State {
            preedit,
            candidates,
            ..
        } = response
        else {
            panic!("關選單應回 State");
        };
        assert_eq!(preedit, "", "關選單不能改動 preedit");
        assert!(candidates.is_empty(), "Delete 應關掉選單");
        assert!(
            engine.local.sessions[&id].menu_tab().is_none(),
            "Delete 後選單要關閉"
        );
    }

    #[test]
    fn test_backspace_without_composition_or_menu_is_not_handled() {
        let mut engine = engine_with_settings(None);
        hello(&mut engine);
        assert_eq!(engine.composition(), "");
        let response = engine.handle(Request::Key {
            key: Key::new(KeyCode::Backspace, None, false),
        });
        assert_ne!(
            handled(&response),
            Some(true),
            "沒有組字也沒有選單時不能搶應用程式的刪除鍵"
        );
    }

    #[test]
    fn mixed_tab_without_english_completion_is_not_handled() {
        let dir = temp_data_dir("tab-no-completion");
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"english_autocomplete":true}"#).unwrap();
        let mut engine = engine_with_settings(Some(path));
        hello(&mut engine);
        for ch in "zzzz".chars() {
            engine.handle(Request::Key {
                key: Key::char(ch, false),
            });
        }
        let response = engine.handle(Request::Key {
            key: Key::new(KeyCode::Tab, None, false),
        });
        assert_eq!(
            handled(&response),
            Some(false),
            "混合英文沒有補全時 Tab 要交還應用程式"
        );
    }

    #[test]
    fn requests_before_hello_are_rejected() {
        let mut engine = Engine::new();
        assert_eq!(
            engine.handle(Request::Commit),
            Response::error(ErrorCode::Internal, "尚未完成握手")
        );
        assert_eq!(engine.composition(), "");
    }

    #[test]
    fn key_produces_converted_candidates() {
        use jc_core::lattice::{InMemoryLexicon, LexEntry};

        let mut lexicon = InMemoryLexicon::new();
        lexicon.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        let config = SessionConfig {
            zh_lexicon: Some(std::sync::Arc::new(lexicon)),
            ..Default::default()
        };
        let mut engine = Engine::with_session(Session::new(config));
        hello(&mut engine);

        for ch in "ji3".chars() {
            engine.handle(Request::Key {
                key: Key::char(ch, false),
            });
        }
        let snapshot = engine.handle(Request::Snapshot);
        let Response::State {
            preedit,
            cursor,
            cursor_rune,
            candidates,
            ..
        } = snapshot
        else {
            panic!("快照應是 State");
        };
        assert_eq!(preedit, "我");
        assert_eq!(cursor, 1);
        assert_eq!(cursor_rune, 1);
        assert_eq!(candidates.first().map(String::as_str), Some("我"));
        assert_eq!(
            engine.handle(Request::Commit),
            Response::Committed { text: "我".into() }
        );
    }

    /// 建一個只認得「我」的引擎，讓 `ji3` 打出單一候選。
    fn engine_with_wo_lexicon() -> Engine {
        use jc_core::lattice::{InMemoryLexicon, LexEntry};

        let mut lexicon = InMemoryLexicon::new();
        lexicon.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        let config = SessionConfig {
            zh_lexicon: Some(Arc::new(lexicon)),
            ..Default::default()
        };
        Engine::with_session(Session::new(config))
    }

    fn type_wo(engine: &mut Engine) {
        for ch in "ji3".chars() {
            engine.handle(Request::Key {
                key: Key::char(ch, false),
            });
        }
    }

    fn press_enter(engine: &mut Engine) -> Response {
        engine.handle(Request::Key {
            key: Key::new(KeyCode::Enter, None, false),
        })
    }

    fn press_down(engine: &mut Engine) -> Response {
        engine.handle(Request::Key {
            key: Key::new(KeyCode::Down, None, false),
        })
    }

    #[test]
    fn test_enter_in_candidate_window_does_not_commit() {
        let mut engine = engine_with_wo_lexicon();
        hello(&mut engine);
        type_wo(&mut engine);
        press_down(&mut engine);
        let Response::State {
            preedit,
            candidates,
            navigating,
            ..
        } = engine.handle(Request::Snapshot)
        else {
            panic!("快照應是 State");
        };
        assert_eq!(preedit, "我");
        assert!(!candidates.is_empty(), "↓ 要開候選窗");
        assert!(navigating, "↓ 要進入選取模式");

        let response = press_enter(&mut engine);
        let Response::State {
            preedit, handled, ..
        } = response
        else {
            panic!("候選窗的第一次 Enter 應回 State，得到 {response:?}");
        };
        assert_eq!(handled, Some(true), "候選窗的 Enter 是輸入法吃掉的一鍵");
        assert_eq!(preedit, "我", "Enter 只套用高亮候選，不能送出整段");
        assert_eq!(engine.composition(), "我", "組字要留著");
    }

    #[test]
    fn test_second_enter_commits_after_candidate_applied() {
        let mut engine = engine_with_wo_lexicon();
        hello(&mut engine);
        type_wo(&mut engine);
        press_down(&mut engine);
        let first = press_enter(&mut engine);
        assert!(matches!(first, Response::State { .. }));
        let applied = engine.composition();
        assert_eq!(applied, "我", "第一次 Enter 要套用高亮候選");

        let response = press_enter(&mut engine);
        let Response::Committed { text } = response else {
            panic!("候選套用後再按 Enter 應送出，得到 {response:?}");
        };
        assert_eq!(text, applied, "第二次 Enter 要送出套用後的字");
        assert_eq!(engine.composition(), "");
    }

    #[test]
    fn test_enter_without_candidate_window_commits() {
        let mut engine = engine_with_wo_lexicon();
        hello(&mut engine);
        type_wo(&mut engine);
        assert_eq!(engine.composition(), "我");

        let response = press_enter(&mut engine);
        let Response::Committed { text } = response else {
            panic!("沒有候選窗時 Enter 應直接送出，得到 {response:?}");
        };
        assert_eq!(text, "我");
        assert_eq!(engine.composition(), "");
    }

    #[test]
    fn prefix_symbol_commits_on_first_enter() {
        let mut engine = engine_with_wo_lexicon();
        hello(&mut engine);
        for ch in "`;`[".chars() {
            engine.handle(Request::Key {
                key: Key::char(ch, false),
            });
        }
        assert_eq!(engine.composition(), "；［");

        let response = press_enter(&mut engine);
        let Response::Committed { text } = response else {
            panic!("字面組字的 Enter 應直接送出，得到 {response:?}");
        };
        assert_eq!(text, "；［", "前導符號不能吃掉第一次 Enter");
        assert_eq!(engine.composition(), "");
    }

    #[test]
    fn health_reports_the_last_slow_request() {
        let mut engine = Engine::new();
        let Response::Health { info } = engine.handle(Request::Health) else {
            panic!("health 應回 Health");
        };
        assert_eq!(info.last_slow_op, None);
        assert_eq!(info.last_slow_ms, None);
        assert_eq!(info.last_slow_input_len, None);

        engine.note_slow_request("key", Duration::from_millis(412), 37);
        let Response::Health { info } = engine.handle(Request::Health) else {
            panic!("health 應回 Health");
        };
        assert_eq!(info.last_slow_op.as_deref(), Some("key"));
        assert_eq!(info.last_slow_ms, Some(412));
        assert_eq!(info.last_slow_input_len, Some(37));
    }

    #[test]
    fn output_simplified_switches_commit_and_health() {
        use jc_core::lattice::{InMemoryLexicon, LexEntry};

        let path = jc_testdata::require_data!("opencc");
        let converter = jc_core::zhconv::tw2s_from_dir(&path).expect("讀取 OpenCC 字典");
        let mut lexicon = InMemoryLexicon::new();
        lexicon.push(LexEntry::new("台", "ㄊㄞˊ", 0, 0));
        lexicon.push(LexEntry::new("灣", "ㄨㄢ", 0, 0));
        let config = SessionConfig {
            zh_lexicon: Some(std::sync::Arc::new(lexicon)),
            simplified: Some(std::sync::Arc::new(converter)),
            ..Default::default()
        };
        let mut engine = Engine::with_session(Session::new(config));
        hello(&mut engine);

        // 預設關閉：health 不帶 output_simplified。
        let Response::Health { info } = engine.handle(Request::Health) else {
            panic!("health 應回 Health");
        };
        assert!(!info.as_info().contains_key("output_simplified"));

        let response = engine.handle(Request::OutputSimplified { on: true });
        assert!(matches!(response, Response::State { .. }));
        let Response::Health { info } = engine.handle(Request::Health) else {
            panic!("health 應回 Health");
        };
        assert_eq!(
            info.as_info().get("output_simplified").map(String::as_str),
            Some("true")
        );

        for reading in ["ㄊㄞˊ", "ㄨㄢ"] {
            type_syllable(&mut engine, reading);
        }
        assert_eq!(
            engine.handle(Request::Commit),
            Response::Committed {
                text: "台湾".into()
            }
        );

        engine.handle(Request::OutputSimplified { on: false });
        for reading in ["ㄊㄞˊ", "ㄨㄢ"] {
            type_syllable(&mut engine, reading);
        }
        assert_eq!(
            engine.handle(Request::Commit),
            Response::Committed {
                text: "台灣".into()
            }
        );
    }

    #[test]
    fn output_simplified_before_hello_is_rejected() {
        let mut engine = Engine::new();
        assert_eq!(
            engine.handle(Request::OutputSimplified { on: true }),
            Response::error(ErrorCode::Internal, "尚未完成握手")
        );
    }

    #[test]
    fn snapshot_keeps_core_in_mixed_and_preserves_composition() {
        let config = SessionConfig::new(jc_core::Mode::Mixed);
        let mut engine = Engine::with_session(Session::new(config));
        hello(&mut engine);
        type_syllable(&mut engine, "ㄨㄛˇ");
        let before = engine.handle(Request::Snapshot);
        assert!(matches!(before, Response::State { .. }));
        assert_eq!(engine.mode(), jc_core::Mode::Mixed);
        assert!(matches!(
            engine.handle(Request::Snapshot),
            Response::State { .. }
        ));
        assert_eq!(engine.mode(), jc_core::Mode::Mixed, "切英文由外殼處理");
        assert_eq!(
            engine.handle(Request::Commit),
            Response::Committed {
                text: "ㄨㄛˇ".into()
            }
        );
    }

    #[test]
    fn snapshot_before_hello_is_rejected() {
        let mut engine = Engine::new();
        assert_eq!(
            engine.handle(Request::Snapshot),
            Response::error(ErrorCode::Internal, "尚未完成握手")
        );
    }

    #[test]
    fn unknown_op_reports_unknown_op() {
        let mut engine = Engine::new();
        let request: Request = serde_json::from_str(r#"{"op":"nope"}"#).unwrap();
        assert_eq!(
            engine.handle(request),
            Response::error(ErrorCode::UnknownOp, "未知的操作")
        );
    }

    #[test]
    fn select_applies_candidate_and_commits_pending_text() {
        use jc_core::lattice::{InMemoryLexicon, LexEntry};

        let mut lexicon = InMemoryLexicon::new();
        lexicon.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        lexicon.push(LexEntry::new("窩", "ㄨㄛˇ", 20, 0));
        let config = SessionConfig {
            zh_lexicon: Some(std::sync::Arc::new(lexicon)),
            ..Default::default()
        };
        let mut engine = Engine::with_session(Session::new(config));
        hello(&mut engine);
        type_syllable(&mut engine, "ㄨㄛˇ");

        let Response::State { candidates, .. } = engine.handle(Request::Snapshot) else {
            panic!("快照應是 State");
        };
        let index = candidates
            .iter()
            .position(|candidate| candidate == "窩")
            .expect("候選要有窩");

        let response = engine.handle(Request::Select { index });
        let Response::State { handled, .. } = response else {
            panic!("select 應回 State，得到 {response:?}");
        };
        assert_eq!(handled, None, "成功套用時沿用內容推斷");
        assert_eq!(
            engine.handle(Request::Commit),
            Response::Committed { text: "窩".into() },
            "select 要真的換字，不能只移動高亮"
        );
        assert!(!engine.is_closing());
    }

    #[test]
    fn select_out_of_range_keeps_composition_and_reports_not_handled() {
        use jc_core::lattice::{InMemoryLexicon, LexEntry};

        let mut lexicon = InMemoryLexicon::new();
        lexicon.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        let config = SessionConfig {
            zh_lexicon: Some(std::sync::Arc::new(lexicon)),
            ..Default::default()
        };
        let mut engine = Engine::with_session(Session::new(config));
        hello(&mut engine);
        type_syllable(&mut engine, "ㄨㄛˇ");

        let before = engine.handle(Request::Snapshot);
        let response = engine.handle(Request::Select { index: 99 });
        let Response::State {
            preedit,
            candidates,
            handled,
            ..
        } = response
        else {
            panic!("越界應回 State，得到 {response:?}");
        };
        assert_eq!(handled, Some(false), "越界要明確回 handled=false");
        assert_eq!(preedit, "我", "越界不得清掉組字");
        assert!(!candidates.is_empty(), "越界不得清掉候選");
        assert_eq!(
            engine.handle(Request::Snapshot),
            before,
            "越界不得改變組字狀態"
        );
        assert!(!engine.is_closing(), "越界不得斷線");
    }

    #[test]
    fn select_before_hello_is_rejected() {
        let mut engine = Engine::new();
        assert_eq!(
            engine.handle(Request::Select { index: 0 }),
            Response::error(ErrorCode::Internal, "尚未完成握手")
        );
    }

    #[test]
    fn shutdown_marks_process_exit() {
        let mut engine = Engine::new();
        hello(&mut engine);
        engine.handle(Request::Shutdown);
        assert!(engine.is_shutdown());
        assert!(engine.is_closing());
        engine.reset_session();
        assert!(engine.is_shutdown(), "reset_session 不該清掉 Shutdown");
        assert!(!engine.is_closing());
    }

    #[test]
    fn session_ownership_is_checked_across_connections() {
        let engine = Engine::new();
        let mut first = engine.accept().into_engine();
        hello(&mut first);
        let id = first.session_ids()[0];
        let mut second = engine.accept().into_engine();
        hello(&mut second);
        assert_eq!(
            second.handle(Request::Close { session: id }),
            ownership_error()
        );
        assert!(first.session_ids().contains(&id), "別條連線不能關掉這個 id");
    }

    #[test]
    fn health_reports_session_english_words() {
        let dict = jc_core::mixed::Dictionary::new([
            jc_core::mixed::Word::new("hello", 3.0),
            jc_core::mixed::Word::new("configuration", 2.0),
            jc_core::mixed::Word::new("small", 1.0),
        ]);
        let config = SessionConfig {
            english: Some(Arc::new(jc_core::mixed::English::new(Some(dict)))),
            ..SessionConfig::default()
        };
        let mut engine = Engine::with_session(Session::new(config));
        hello(&mut engine);
        let Response::Health { info } = engine.handle(Request::Health) else {
            panic!("health 應回 Health");
        };
        assert_eq!(info.english_words, 3);
        assert_eq!(
            info.as_info().get("english_words").map(String::as_str),
            Some("3")
        );
    }

    #[test]
    fn health_reports_registry_numbers() {
        let engine = Engine::new();
        let mut conn = engine.accept().into_engine();
        conn.handle(Request::Ping);
        let Response::Health { info } = conn.handle(Request::Health) else {
            panic!("health 應回 Health");
        };
        assert_eq!(info.sessions, 0);
        assert_eq!(info.composing, 0);
        assert!(!info.draining);
        assert!(info.pid.is_some());
        assert!(info.personal_reset);
        hello(&mut conn);
        let Response::Health { info } = conn.handle(Request::Health) else {
            panic!("health 應回 Health");
        };
        assert_eq!(info.sessions, 1);
    }

    /// health 要把外殼需要的能力旗標帶出去。
    ///
    /// 少了 `left_shift`，jc-tsf 的 `ShiftConfig::from_info` 會退回
    /// `left_shift=false`，左 Shift 就完全不切中英。
    #[test]
    fn health_reports_shell_capabilities() {
        let mut engine = Engine::new();
        engine.set_extra_caps(
            [
                ("left_shift".to_string(), true),
                ("right_shift".to_string(), true),
            ]
            .into_iter()
            .collect(),
        );
        let mut conn = engine.accept().into_engine();
        hello(&mut conn);
        let Response::Health { info } = conn.handle(Request::Health) else {
            panic!("health 應回 Health");
        };
        let map = info.as_info();
        assert_eq!(map.get("left_shift").map(String::as_str), Some("true"));
        assert_eq!(map.get("right_shift").map(String::as_str), Some("true"));
    }

    #[test]
    fn replay_log_is_carried_in_registry() {
        let mut engine = Engine::new();
        hello(&mut engine);
        engine.handle(Request::Key {
            key: Key::char('ㄇ', false),
        });
        engine.handle(Request::Key {
            key: Key::char('ㄚ', false),
        });
        let from = engine.session_ids()[0];
        let log = engine.session_log(from);
        assert_eq!(log.len(), 2);
        let mut conn = engine.accept().into_engine();
        hello(&mut conn);
        let to = conn.session_ids()[0];
        let replies = conn.replay_into(from, to);
        assert_eq!(replies.len(), 2);
        // 第一條連線的 log 對第二條不可見（歸屬檢查）。
        assert!(conn.session_log(from).is_empty());
    }

    /// 建一個 replay 測試用的引擎；所有連線共用同一份注音詞典。
    fn engine_for_replay() -> Engine {
        use jc_core::lattice::{InMemoryLexicon, LexEntry};

        let mut lexicon = InMemoryLexicon::new();
        lexicon.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        let lexicon = Arc::new(lexicon);
        Engine::with_factory(
            Arc::new(move || {
                let config = SessionConfig {
                    zh_lexicon: Some(Arc::clone(&lexicon)),
                    english_autocomplete: true,
                    ..Default::default()
                };
                Session::new(config)
            }),
            None,
        )
    }

    /// 把來源 log 重播一次與兩次，兩邊都不該寫 personal，且結果相同。
    fn assert_replay_learns_nothing(engine: &Engine, from: u64, case: &str) {
        let (once, once_id, baseline) = {
            let mut conn = engine.accept().into_engine();
            hello(&mut conn);
            let id = conn.session_ids()[0];
            let baseline = conn.local.sessions[&id].personal().clone();
            conn.replay_into(from, id);
            (conn, id, baseline)
        };
        let (twice, twice_id) = {
            let mut conn = engine.accept().into_engine();
            hello(&mut conn);
            let id = conn.session_ids()[0];
            conn.replay_into(from, id);
            conn.replay_into(from, id);
            (conn, id)
        };

        for (label, session) in [
            ("once", &once.local.sessions[&once_id]),
            ("twice", &twice.local.sessions[&twice_id]),
        ] {
            assert_eq!(
                session.personal(),
                &baseline,
                "{case} {label} replay 不得寫 personal"
            );
            assert_eq!(session.learn_events(), 0, "{case} {label} 不得累加學習統計");
        }
        assert_eq!(
            once.local.sessions[&once_id].personal(),
            twice.local.sessions[&twice_id].personal(),
            "{case} 重播一次與兩次的統計要相同"
        );
    }

    #[test]
    fn replaying_a_select_request_does_not_learn_again() {
        let source = engine_for_replay();
        let mut conn = source.accept().into_engine();
        hello(&mut conn);
        type_syllable(&mut conn, "ㄨㄛˇ");
        conn.handle(Request::Key {
            key: Key::new(KeyCode::Down, None, false),
        });
        let from = conn.session_ids()[0];
        let before = conn.local.sessions[&from].personal().clone();
        conn.handle(Request::Select { index: 0 });
        assert_ne!(
            conn.local.sessions[&from].personal(),
            &before,
            "來源的 Select 要先真的學習"
        );
        assert_replay_learns_nothing(&source, from, "select");
    }

    #[test]
    fn replaying_a_digit_window_choice_does_not_learn_again() {
        let source = engine_for_replay();
        let mut conn = source.accept().into_engine();
        hello(&mut conn);
        type_syllable(&mut conn, "ㄨㄛˇ");
        conn.handle(Request::Key {
            key: Key::new(KeyCode::Down, None, false),
        });
        let from = conn.session_ids()[0];
        let before = conn.local.sessions[&from].personal().clone();
        conn.handle(Request::Key {
            key: Key::char('1', false),
        });
        assert_ne!(
            conn.local.sessions[&from].personal(),
            &before,
            "來源的數字鍵要先真的學習"
        );
        assert_replay_learns_nothing(&source, from, "digit");
    }

    #[test]
    fn replaying_an_english_correction_does_not_learn_again() {
        let source = engine_for_replay();
        let mut conn = source.accept().into_engine();
        hello(&mut conn);
        for ch in "recieve".chars() {
            conn.handle(Request::Key {
                key: Key::char(ch, false),
            });
        }
        let Response::State { candidates, .. } = conn.handle(Request::Snapshot) else {
            panic!("快照應是 State");
        };
        let index = candidates
            .iter()
            .position(|candidate| candidate == "receive")
            .expect("receive 候選");
        let from = conn.session_ids()[0];
        let before = conn.local.sessions[&from].personal().clone();
        conn.handle(Request::Select { index });
        assert_ne!(
            conn.local.sessions[&from].personal(),
            &before,
            "來源的英文修正要先真的學習"
        );
        assert_replay_learns_nothing(&source, from, "english");
    }

    #[test]
    fn serve_returns_after_clean_eof() {
        let mut engine = Engine::new();
        let mut input = Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        serve(
            &mut engine,
            &mut input,
            &mut output,
            jc_proto::DEFAULT_MAX_FRAME,
        )
        .unwrap();
        assert!(output.is_empty(), "沒有請求就不該有回應");
    }

    #[test]
    fn serve_answers_version_mismatch_then_stops() {
        let mut engine = Engine::new();
        let mut input = Vec::new();
        write_message(
            &mut input,
            &Request::Hello {
                protocol: PROTOCOL_VERSION + 1,
                client: "test".into(),
            },
            jc_proto::DEFAULT_MAX_FRAME,
        )
        .unwrap();
        write_message(&mut input, &Request::Shutdown, jc_proto::DEFAULT_MAX_FRAME).unwrap();

        let mut output = Vec::new();
        serve(
            &mut engine,
            &mut Cursor::new(input),
            &mut output,
            jc_proto::DEFAULT_MAX_FRAME,
        )
        .unwrap();

        let mut replies = Cursor::new(output);
        let reply: Response = read_message(&mut replies, jc_proto::DEFAULT_MAX_FRAME)
            .unwrap()
            .unwrap();
        assert!(matches!(
            reply,
            Response::Error {
                code: ErrorCode::VersionMismatch,
                ..
            }
        ));
        assert!(
            read_message::<_, Response>(&mut replies, jc_proto::DEFAULT_MAX_FRAME)
                .unwrap()
                .is_none(),
            "只能有一個回應"
        );
    }

    #[test]
    fn serve_reports_bad_frame_and_closes() {
        let mut engine = Engine::new();
        hello(&mut engine);
        let mut input = Vec::new();
        write_message(&mut input, &Request::Snapshot, jc_proto::DEFAULT_MAX_FRAME).unwrap();
        input.extend_from_slice(&5u32.to_le_bytes());
        input.extend_from_slice(b"nope!");

        let mut output = Vec::new();
        serve(
            &mut engine,
            &mut Cursor::new(input),
            &mut output,
            jc_proto::DEFAULT_MAX_FRAME,
        )
        .unwrap();

        let mut replies = Cursor::new(output);
        let first: Response = read_message(&mut replies, jc_proto::DEFAULT_MAX_FRAME)
            .unwrap()
            .unwrap();
        assert!(matches!(first, Response::State { .. }));
        let second: Response = read_message(&mut replies, jc_proto::DEFAULT_MAX_FRAME)
            .unwrap()
            .unwrap();
        match second {
            Response::Error { code, .. } => assert_eq!(code, ErrorCode::BadFrame),
            other => panic!("壞訊框應回 BadFrame，收到 {other:?}"),
        }
    }

    fn temp_data_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jc-t12c-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_store(word: &str) -> PersonalStore {
        let mut store = PersonalStore::new();
        store.note_seed(word, 1.0);
        store
    }

    #[test]
    fn personal_loads_only_the_prefixed_file() {
        let dir = temp_data_dir("isolate");
        make_store("舊詞").save(&dir.join("personal.json")).unwrap();
        make_store("新詞")
            .save(&dir.join("custom-personal.json"))
            .unwrap();

        let mut engine = Engine::with_factory(
            Arc::new(|| Session::new(SessionConfig::default())),
            Some(dir.clone()),
        );
        engine.set_personal_prefix("custom-personal");
        engine.load_personal();
        assert!(engine.personal_store().is_learned("新詞"));
        assert!(!engine.personal_store().is_learned("舊詞"));

        let mut old_engine = Engine::with_factory(
            Arc::new(|| Session::new(SessionConfig::default())),
            Some(dir),
        );
        old_engine.set_personal_prefix("personal");
        old_engine.load_personal();
        assert!(old_engine.personal_store().is_learned("舊詞"));
        assert!(!old_engine.personal_store().is_learned("新詞"));
    }

    #[test]
    fn personal_seed_applies_to_all_connections() {
        let dir = temp_data_dir("seed");
        make_store("全域詞")
            .save(&dir.join("personal.json"))
            .unwrap();
        let mut engine = Engine::with_factory(
            Arc::new(|| Session::new(SessionConfig::default())),
            Some(dir),
        );
        engine.load_personal();

        let mut first = engine.accept().into_engine();
        let mut second = engine.accept().into_engine();
        hello(&mut first);
        hello(&mut second);
        for conn in [&first, &second] {
            let id = conn.session_ids()[0];
            assert!(conn.local.sessions[&id].personal().is_learned("全域詞"));
        }
    }

    #[test]
    fn personal_merge_adds_only_the_delta() {
        let mut global = make_store("基準詞");
        let baseline = global.clone();
        let mut session = baseline.clone();
        session.note_seed("基準詞", 0.5);
        session.note_seed("新詞", 1.0);

        personal_merge::merge_learning(&mut global, &baseline, &session);
        assert_eq!(global.uni.get("基準詞").copied(), Some(1.5));
        assert_eq!(global.uni.get("新詞").copied(), Some(1.0));

        // 基準前進後再合一次，同一筆學習不該重複累加。
        let advanced = session.clone();
        personal_merge::merge_learning(&mut global, &advanced, &session);
        assert_eq!(global.uni.get("基準詞").copied(), Some(1.5));
        assert_eq!(global.uni.get("新詞").copied(), Some(1.0));
    }

    #[test]
    fn personal_flush_writes_prefixed_file_on_close() {
        let dir = temp_data_dir("close");
        let mut engine = Engine::with_factory(
            Arc::new(|| Session::new(SessionConfig::default())),
            Some(dir.clone()),
        );
        engine.set_personal_prefix("custom-personal");
        engine.load_personal();
        hello(&mut engine);
        let id = engine.session_ids()[0];
        {
            let session = engine.local.sessions.get_mut(&id).unwrap();
            let mut store = session.personal().clone();
            store.note_seed("關閉詞", 2.0);
            session.set_personal_store(store);
        }
        engine.handle(Request::Close { session: id });

        let saved = PersonalStore::load(&dir.join("custom-personal.json")).unwrap();
        assert_eq!(saved.uni.get("關閉詞").copied(), Some(2.0));
    }

    #[test]
    fn health_reports_real_personal_state() {
        let dir = temp_data_dir("health");
        make_store("讀回詞")
            .save(&dir.join("personal.json"))
            .unwrap();
        let mut engine = Engine::with_factory(
            Arc::new(|| Session::new(SessionConfig::default())),
            Some(dir),
        );
        engine.load_personal();
        let Response::Health { info } = engine.handle(Request::Health) else {
            panic!("health 應回 Health");
        };
        assert!(info.personal_reset);
        assert_eq!(info.capabilities.get("personal_shared"), Some(&true));
        assert_eq!(info.capabilities.get("personal_persist"), Some(&true));
        assert_eq!(info.capabilities.get("personal_loaded"), Some(&true));
    }

    fn engine_with_settings(path: Option<PathBuf>) -> Engine {
        let goos = "linux";
        let loaded = config::load(path.as_deref(), None, goos);
        let effective = config::effective(&loaded.settings, &config::Overrides::default(), goos);
        let mut engine = Engine::new();
        engine.set_settings(effective, path, goos, config::Overrides::default());
        engine
    }

    fn write_keybinds(path: &PathBuf, symbol: &str, emoji: &str) {
        let body =
            format!(r#"{{"keybinds":{{"symbol_menu":"{symbol}","emoji_palette":"{emoji}"}}}}"#);
        std::fs::write(path, body).unwrap();
    }

    fn handled(response: &Response) -> Option<bool> {
        match response {
            Response::State { handled, .. } => *handled,
            other => panic!("應該回 State，收到 {other:?}"),
        }
    }

    #[test]
    fn ctrl_alt_comma_opens_symbol_menu_through_keymap() {
        let mut engine = engine_with_settings(None);
        hello(&mut engine);
        let response = engine.handle(Request::Key {
            key: Key::char(',', false).with_ctrl().with_alt(),
        });
        assert_eq!(handled(&response), Some(true));
        let id = engine.current_session();
        assert!(
            engine.local.sessions[&id].menu_tab().is_some(),
            "引擎要真的開符號選單"
        );
    }

    #[test]
    fn unbound_alt_chord_is_not_handled() {
        let mut engine = engine_with_settings(None);
        hello(&mut engine);
        type_syllable(&mut engine, "ㄇㄚ");
        assert_eq!(engine.composition(), "ㄇㄚ");
        let response = engine.handle(Request::Key {
            key: Key::char('x', false).with_ctrl().with_alt(),
        });
        assert_eq!(handled(&response), Some(false));
        assert_eq!(
            engine.composition(),
            "ㄇㄚ",
            "未命中的 Alt 組合鍵不得改動組字"
        );
    }

    #[test]
    fn settings_reload_updates_revision_keymap_and_keeps_composition() {
        let dir = temp_data_dir("reload");
        let path = dir.join("settings.json");
        write_keybinds(&path, "ctrl+alt+,", "ctrl+alt+.");
        let mut engine = engine_with_settings(Some(path.clone()));
        hello(&mut engine);
        type_syllable(&mut engine, "ㄇㄚ");
        assert_eq!(engine.composition(), "ㄇㄚ");

        write_keybinds(&path, "ctrl+alt+s", "ctrl+alt+.");
        let revision = engine.reload_settings().expect("重載設定");
        assert_eq!(revision, 1);
        assert_eq!(engine.composition(), "ㄇㄚ", "熱重載不得清掉組字");

        let old = engine.handle(Request::Key {
            key: Key::char(',', false).with_ctrl().with_alt(),
        });
        assert_ne!(handled(&old), Some(true), "舊綁定不該再命中");
        let new = engine.handle(Request::Key {
            key: Key::char('s', false).with_ctrl().with_alt(),
        });
        assert_eq!(handled(&new), Some(true), "新綁定要立刻生效");

        let Response::Health { info } = engine.handle(Request::Health) else {
            panic!("health 應回 Health");
        };
        assert_eq!(info.settings_revision, 1);
        assert_eq!(info.bindings["symbol_menu"], "ctrl+alt+s");
        assert!(info.settings_error.is_none());
    }

    #[test]
    fn apply_settings_reloads_immediately() {
        let dir = temp_data_dir("apply");
        let path = dir.join("settings.json");
        write_keybinds(&path, "ctrl+alt+,", "ctrl+alt+.");
        let mut engine = engine_with_settings(Some(path.clone()));
        hello(&mut engine);

        write_keybinds(&path, "ctrl+alt+p", "ctrl+alt+.");
        let Response::Health { info } = engine.handle(Request::ApplySettings {
            settings: BTreeMap::new(),
        }) else {
            panic!("apply_settings 應回 Health");
        };
        assert_eq!(info.settings_revision, 1);
        assert_eq!(info.bindings["symbol_menu"], "ctrl+alt+p");

        let response = engine.handle(Request::Key {
            key: Key::char('p', false).with_ctrl().with_alt(),
        });
        assert_eq!(handled(&response), Some(true));
    }

    #[test]
    fn corrupt_reload_keeps_old_settings_and_reports_error() {
        let dir = temp_data_dir("reload-error");
        let path = dir.join("settings.json");
        write_keybinds(&path, "ctrl+alt+,", "ctrl+alt+.");
        let mut engine = engine_with_settings(Some(path.clone()));
        hello(&mut engine);
        engine.reload_settings().expect("第一次重載");

        std::fs::write(&path, b"{ not json").unwrap();
        assert!(engine.reload_settings().is_err());
        let Response::Health { info } = engine.handle(Request::Health) else {
            panic!("health 應回 Health");
        };
        assert_eq!(info.settings_revision, 1, "失敗不得 +1");
        assert_eq!(info.bindings["symbol_menu"], "ctrl+alt+,");
        assert!(info.settings_error.is_some());
    }

    #[test]
    fn bad_chord_reload_keeps_old_bindings_and_reports_error() {
        let dir = temp_data_dir("reload-bad-chord");
        let path = dir.join("settings.json");
        write_keybinds(&path, "ctrl+alt+s", "ctrl+alt+.");
        let mut engine = engine_with_settings(Some(path.clone()));
        hello(&mut engine);
        assert_eq!(engine.reload_settings().expect("第一次重載"), 1);

        // 合法 JSON，但 chord 沒有 ctrl／alt；熱重載必須保留舊表。
        write_keybinds(&path, "a", "shift+b");
        assert!(engine.reload_settings().is_err());

        let Response::Health { info } = engine.handle(Request::Health) else {
            panic!("health 應回 Health");
        };
        assert_eq!(info.settings_revision, 1, "失敗不得 +1");
        assert_eq!(info.bindings["symbol_menu"], "ctrl+alt+s");
        assert!(info.settings_error.is_some(), "要回報壞 chord");

        let old = engine.handle(Request::Key {
            key: Key::char('s', false).with_ctrl().with_alt(),
        });
        assert_eq!(handled(&old), Some(true), "舊 chord 仍要可用");
        let bad = engine.handle(Request::Key {
            key: Key::char('a', false),
        });
        assert_ne!(handled(&bad), Some(true), "壞 chord 不能被啟用");
    }

    #[test]
    fn watcher_picks_up_change_made_during_startup_load() {
        let dir = temp_data_dir("startup-change");
        let path = dir.join("settings.json");
        write_keybinds(&path, "ctrl+alt+,", "ctrl+alt+.");
        // 模擬 main：設定檔在資料載入前先讀，戳記也那時就記下。
        let loaded = config::load(Some(&path), None, "linux");
        let effective = config::effective(&loaded.settings, &config::Overrides::default(), "linux");
        let mut engine = Engine::new();
        engine.set_settings(
            effective,
            Some(path.clone()),
            "linux",
            config::Overrides::default(),
        );
        engine.set_settings_stamp(loaded.stamp);

        // 資料載入的十幾秒內使用者改檔；watcher 起跑後要補上這一次。
        write_keybinds(&path, "ctrl+alt+shift+s", "ctrl+alt+.");
        engine.watch_settings();
        let deadline = Instant::now() + Duration::from_secs(3);
        while engine.settings_revision() == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(engine.settings_revision(), 1, "啟動期間的變更不得漏掉");
        engine.handle(Request::Shutdown);
    }

    #[test]
    fn bound_chord_during_composition_opens_menu_without_committing() {
        let mut engine = engine_with_settings(None);
        hello(&mut engine);
        type_syllable(&mut engine, "ㄇㄚ");
        assert_eq!(engine.composition(), "ㄇㄚ");

        let response = engine.handle(Request::Key {
            key: Key::char(',', false).with_ctrl().with_alt(),
        });
        assert_eq!(handled(&response), Some(true));
        assert_eq!(engine.composition(), "ㄇㄚ", "快捷鍵不能先提交組字");
        let id = engine.current_session();
        assert!(engine.local.sessions[&id].menu_tab().is_some());
    }
}
