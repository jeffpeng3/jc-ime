//! 組字、提交與非同步回應的套用。
//!
//! 這個檔案裡的東西全部只在 TSF 的 UI 執行緒上跑：worker 執行緒只把結果推進
//! [`Queue`]，再用 `PostMessageW` 叫醒這裡的訊息視窗。[`Control::drain`] 在
//! 訊息視窗的 window proc 裡執行，因此可以安全地碰 `ITfContext`。
//!
//! 套用規則是任務書的表格：`State` 有 `preedit` 就開始／更新組字，空的就結束
//! 組字，`Committed` 結束組字後插入文字，錯誤只記錄並收掉組字。

use std::cell::{Cell, RefCell};
use std::mem::ManuallyDrop;
use std::ptr;
use std::rc::Rc;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jc_proto::{CandidateDetail, Key, KeyCode, Request, Response};
use windows::core::{implement, IUnknown, IUnknownImpl, Interface, Ref, Result, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, ERROR_CLASS_ALREADY_EXISTS, E_FAIL, FALSE, HINSTANCE, HMODULE, HWND, LPARAM,
    LRESULT, WPARAM,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::TextServices::{
    ITfComposition, ITfCompositionSink, ITfCompositionSink_Impl, ITfContext, ITfContextComposition,
    ITfEditSession, ITfEditSession_Impl, ITfInsertAtSelection, ITfKeyEventSink, ITfKeystrokeMgr,
    ITfProperty, ITfRange, ITfSource, ITfThreadMgr, ITfThreadMgrEventSink, ITfUIElementMgr,
    GUID_PROP_ATTRIBUTE, INSERT_TEXT_AT_SELECTION_FLAGS, TF_AE_END, TF_ANCHOR_END, TF_ANCHOR_START,
    TF_DEFAULT_SELECTION, TF_ES_READWRITE, TF_ES_SYNC, TF_SELECTION, TF_SELECTIONSTYLE,
    TF_ST_CORRECTION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, RegisterClassW,
    SetWindowLongPtrW, GWLP_USERDATA, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW,
};

use crate::candidates::{CandidateActions, CandidateHost, CandidateItem, CandidateSnapshot};
use crate::display;
use crate::engine::{self, Job, Queue};
use crate::keys;
use crate::logging;

/// `OnKillFocus` 到真正放掉 pipe 的寬限期。
///
/// Windows 同一個文字框內的點擊常先後送出 `OnKillFocus`、`OnSetFocus`。這段
/// 時間內只取消斷線，不重建 worker，避免連線風暴。
const DISCONNECT_GRACE: Duration = Duration::from_millis(120);

/// 私有訊息視窗的 class 名稱。
const WINDOW_CLASS: &str = "jc-tsf-message-window";

/// 上一次套用的引擎狀態。
///
/// 只用來判斷「現在是不是在組字」、需不需要補送 `Commit`（協定 v3 的選字
/// 缺口：Space 選字後引擎回的是空 `State`，文字還在待提交區），以及數字鍵
/// 要不要先送一次 `Down` 進入選取模式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LastState {
    /// 上一次 `preedit` 的 rune 數。
    preedit: usize,
    /// 上一次候選數量。
    candidates: usize,
    /// 上一次是否已在明確的候選選取模式。
    navigating: bool,
}

impl LastState {
    /// 有組字字串或有候選就算組字中。
    fn composing(self) -> bool {
        self.preedit > 0 || self.candidates > 0
    }
}

/// 要套用到 context 的一次編輯。
enum EditAction {
    /// 更新組字字串與游標。
    Update { preedit: String, caret: usize },
    /// 結束組字。
    End,
    /// 結束組字後插入文字。
    Commit { text: String },
}

/// 一個 `Response::State` 的完整內容。
///
/// `drain` 先把欄位搬進來，套用流程就不用一路傳十幾個參數；候選窗也讀同一份。
struct StateData {
    preedit: String,
    cursor: usize,
    cursor_rune: usize,
    candidates: Vec<String>,
    details: Vec<CandidateDetail>,
    highlighted: usize,
    navigating: bool,
    label: Option<String>,
}

/// UI 執行緒看到的 pipe 連線狀態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipeState {
    /// 沒有 worker，也沒有 pipe handle。
    Off,
    /// worker 已啟動並持有（或正在取得）pipe。
    Active,
    /// 已要求斷開，寬限期還沒到期；pipe 仍由 worker 持有。
    Draining,
}

/// [`FocusPipe`] 要求 [`Control`] 做的事。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipeAction {
    /// 狀態沒變，不用動作。
    None,
    /// 建立新的請求通道並啟動 worker。
    Start,
    /// 取消進行中的寬限期，保留原本的 worker。
    Cancel,
    /// 安排寬限期到期後放掉 worker。
    Disconnect,
    /// 立刻放掉 worker（Deactivate）。
    Stop,
}

/// focus 事件到 pipe 動作的狀態機。
///
/// 只描述 UI 執行緒的意圖，不碰 COM 或 pipe。`OnKillFocus` 緊接 `OnSetFocus`
/// 時只回 [`PipeAction::Cancel`]，不重新連整條 pipe。
#[derive(Debug)]
pub struct FocusPipe {
    state: PipeState,
}

impl FocusPipe {
    /// 尚未連線的初始狀態。
    pub fn new() -> Self {
        Self {
            state: PipeState::Off,
        }
    }

    /// 目前的連線狀態。
    pub fn state(&self) -> PipeState {
        self.state
    }

    /// `OnSetFocus`：沒有 worker 就啟動，寬限期內就取消斷線。
    pub fn focus_gained(&mut self) -> PipeAction {
        match self.state {
            PipeState::Off => {
                self.state = PipeState::Active;
                PipeAction::Start
            }
            PipeState::Active => PipeAction::None,
            PipeState::Draining => {
                self.state = PipeState::Active;
                PipeAction::Cancel
            }
        }
    }

    /// `OnKillFocus`：要求寬限期後放掉 pipe；重複呼叫不做事。
    pub fn focus_lost(&mut self) -> PipeAction {
        match self.state {
            PipeState::Off | PipeState::Draining => PipeAction::None,
            PipeState::Active => {
                self.state = PipeState::Draining;
                PipeAction::Disconnect
            }
        }
    }

    /// `Deactivate`：不論目前狀態為何都確保 worker 被放掉。
    pub fn deactivate(&mut self) -> PipeAction {
        let action = if self.state == PipeState::Off {
            PipeAction::None
        } else {
            PipeAction::Stop
        };
        self.state = PipeState::Off;
        action
    }
}

impl Default for FocusPipe {
    fn default() -> Self {
        Self::new()
    }
}

/// 目前 worker 的 sender 與它的代號。
///
/// 代號用來讓延遲斷線的計時器只關掉它對應的那一代 worker；focus 恢復或
/// Deactivate 會把代號往前推，舊計時器因此不會誤關新 worker。
struct ActiveWorker {
    generation: u64,
    sender: Option<Sender<Job>>,
}

/// 文字服務的執行期狀態（UI 執行緒專用）。
pub struct Control {
    thread_mgr: ITfThreadMgr,
    client_id: u32,
    /// 最近一次按鍵所在的 context。
    context: RefCell<Option<ITfContext>>,
    /// 目前的組字物件。
    composition: RefCell<Option<ITfComposition>>,
    last: Cell<LastState>,
    /// 按鍵世代；與候選元素的滑鼠點選共用，過期的 `State` 才會被丟棄。
    generation: Rc<Cell<u64>>,
    /// 已送出但還沒回覆的選取導覽鍵世代；用來避免快速連打時重複補 `Down`。
    navigation_armed: Cell<Option<u64>>,
    hwnd: HWND,
    queue: Arc<Queue>,
    /// 長期的請求通道；候選清單與 `Control` 共用，由 dispatch 執行緒轉送給
    /// 當前的 engine worker。worker 隨 focus 今生今滅，這條通道不跟著換。
    bridge_tx: Sender<Job>,
    /// 目前 worker 的 sender；`None` 代表沒有 worker。
    active: Arc<Mutex<ActiveWorker>>,
    /// focus 事件到 pipe 動作的狀態機。
    pipe: RefCell<FocusPipe>,
    /// 交給 Windows 畫的候選清單；只在 UI 執行緒使用。
    candidate: RefCell<CandidateHost>,
    /// `GUID_DISPLAY_ATTRIBUTE_INPUT` 的 `TfGuidAtom`；`None` 代表底線停用。
    display_atom: Option<u32>,
    sink_state: Arc<SinkState>,
    /// 組字 sink 的 COM 引用；由 TSF 保管，這裡只負責存活。
    sink: ITfCompositionSink,
    key_sink: Option<ITfKeyEventSink>,
    thread_sink: Option<ITfThreadMgrEventSink>,
    keystroke: Option<ITfKeystrokeMgr>,
    source: Option<ITfSource>,
    source_cookie: Cell<u32>,
}

impl Control {
    /// 建立狀態；worker 等到 `OnSetFocus` 才啟動。
    pub fn new(thread_mgr: ITfThreadMgr, client_id: u32) -> Self {
        let queue = Arc::new(Queue::new());
        let active = Arc::new(Mutex::new(ActiveWorker {
            generation: 0,
            sender: None,
        }));

        // 候選清單持有的是一個普通 `Sender<Job>`；讓 dispatch 執行緒當中介，
        // worker 隨 focus 結束時只要清掉 `active`，不必改候選清單的型別。
        let (bridge_tx, bridge_rx) = channel();
        let active_for_dispatch = Arc::clone(&active);
        if std::thread::Builder::new()
            .name("jc-tsf-dispatch".into())
            .spawn(move || {
                for job in bridge_rx {
                    let Ok(active) = active_for_dispatch.lock() else {
                        continue;
                    };
                    let Some(sender) = active.sender.as_ref() else {
                        logging::debug("pipe 未連線，請求被丟棄");
                        continue;
                    };
                    if sender.send(job).is_err() {
                        logging::warn("worker 已結束，請求被丟棄");
                    }
                }
            })
            .is_err()
        {
            logging::warn("啟動 dispatch 執行緒失敗");
        }

        // 候選清單一定要從 thread manager 取得；拿不到就只影響候選窗，
        // 組字與提交照常運作。
        let ui_manager: Option<ITfUIElementMgr> = match thread_mgr.cast() {
            Ok(manager) => Some(manager),
            Err(error) => {
                logging::warn(format!("取得 ITfUIElementMgr 失敗，候選窗停用：{error}"));
                None
            }
        };

        let sink_state = Arc::new(SinkState::new());
        let sink: ITfCompositionSink = CompositionSink {
            state: Arc::clone(&sink_state),
        }
        .into();

        // 底線沒有 atom 就畫不出來；失敗只影響外觀，其他輸入照常。
        let display_atom = display::try_register_guid_atom();

        // 滑鼠點選與一般按鍵共用同一個世代計數器與請求通道。
        let generation = Rc::new(Cell::new(0));
        let actions = Rc::new(CandidateActions::new(
            bridge_tx.clone(),
            Rc::clone(&generation),
            Arc::clone(&queue),
        ));

        Self {
            thread_mgr,
            client_id,
            context: RefCell::new(None),
            composition: RefCell::new(None),
            last: Cell::new(LastState::default()),
            generation,
            navigation_armed: Cell::new(None),
            hwnd: HWND::default(),
            queue,
            bridge_tx,
            active,
            pipe: RefCell::new(FocusPipe::new()),
            candidate: RefCell::new(CandidateHost::new(ui_manager, actions)),
            display_atom,
            sink_state,
            sink,
            key_sink: None,
            thread_sink: None,
            keystroke: None,
            source: None,
            source_cookie: Cell::new(0),
        }
    }

    /// 建立訊息視窗、註冊 sink，完成 `Activate`。
    pub fn start(
        mut control: Box<Self>,
        key_sink: ITfKeyEventSink,
        thread_sink: ITfThreadMgrEventSink,
    ) -> Result<Box<Self>> {
        let pointer = &*control as *const Control;
        logging::debug(format!(
            "Activate：建立訊息視窗（client_id = {}）",
            control.client_id
        ));
        control.hwnd = create_message_window(pointer)?;
        control.queue.attach(control.hwnd.0 as isize);
        control.sink_state.attach(pointer);
        control.key_sink = Some(key_sink.clone());
        control.thread_sink = Some(thread_sink.clone());
        if let Err(error) = control.advise(&key_sink, &thread_sink) {
            control.shutdown();
            return Err(error);
        }
        Ok(control)
    }

    /// `OnSetFocus`：清掉舊 context，並確保 engine worker 已啟動、pipe 已連線。
    pub fn on_focus_changed(&self) {
        self.clear_focus_state();
        let action = self.pipe.borrow_mut().focus_gained();
        // 寬限期已到期時 worker 的 sender 會是 None；此時 Cancel 沒有對象，
        // 必須改成重新啟動，否則焦點回來後永遠不再連 pipe。
        let action = if action == PipeAction::Cancel && !self.has_active_worker() {
            PipeAction::Start
        } else {
            action
        };
        self.apply_pipe_action(action);
    }

    /// `OnKillFocus`：清掉組字，並安排寬限期後放掉 worker 與 pipe。
    pub fn on_focus_lost(&self) {
        self.clear_focus_state();
        let action = self.pipe.borrow_mut().focus_lost();
        self.apply_pipe_action(action);
    }

    /// 目前是否還有 worker 持有請求通道。
    fn has_active_worker(&self) -> bool {
        self.active
            .lock()
            .map(|active| active.sender.is_some())
            .unwrap_or(false)
    }

    /// 執行 [`FocusPipe`] 的決定。
    fn apply_pipe_action(&self, action: PipeAction) {
        match action {
            PipeAction::None => {}
            PipeAction::Start => self.start_engine_worker(),
            PipeAction::Cancel => self.cancel_disconnect(),
            PipeAction::Disconnect => self.schedule_disconnect(),
            PipeAction::Stop => self.stop_engine_worker(),
        }
    }

    /// 建立新的請求通道並啟動 engine worker；worker 自己會連 pipe。
    fn start_engine_worker(&self) {
        let (sender, receiver) = channel();
        if let Ok(mut active) = self.active.lock() {
            active.generation = active.generation.wrapping_add(1);
            active.sender = Some(sender);
        }
        // 不帶 join handle：worker 結束時自己會把 `Queue` 標成未連線。
        engine::spawn(receiver, Arc::clone(&self.queue));
        logging::debug("OnSetFocus：已啟動引擎 worker");
    }

    /// 取消進行中的寬限期；worker 與 pipe 保持不變。
    fn cancel_disconnect(&self) {
        if let Ok(mut active) = self.active.lock() {
            active.generation = active.generation.wrapping_add(1);
        }
        logging::debug("OnSetFocus：取消前一次的 OnKillFocus，保留引擎 pipe");
    }

    /// 延遲放掉 worker；同一文字框內立刻回來的 focus 會讓計時器失效。
    fn schedule_disconnect(&self) {
        let generation = match self.active.lock() {
            Ok(active) => active.generation,
            Err(error) => {
                logging::warn(format!("讀取 active worker 失敗：{error}"));
                return;
            }
        };
        let active = Arc::clone(&self.active);
        let spawned = std::thread::Builder::new()
            .name("jc-tsf-disconnect".into())
            .spawn(move || {
                std::thread::sleep(DISCONNECT_GRACE);
                if let Ok(mut active) = active.lock() {
                    if active.generation == generation {
                        active.sender = None;
                        logging::debug("OnKillFocus：寬限期到期，已釋放引擎 pipe");
                    } else {
                        logging::debug("OnKillFocus：已取消，保留引擎 pipe");
                    }
                }
            });
        if spawned.is_err() {
            logging::warn("啟動延遲斷線計時器失敗，過期請求不會被清理");
        }
    }

    /// 立刻放掉 worker；`Deactivate` 與 `Drop` 用它確保 pipe 最終會釋放。
    fn stop_engine_worker(&self) {
        if let Ok(mut active) = self.active.lock() {
            active.generation = active.generation.wrapping_add(1);
            active.sender = None;
        }
    }

    /// 是否正在組字（有 `preedit` 或有候選）。
    pub fn composing(&self) -> bool {
        self.last.get().composing()
    }

    /// `OnTestKeyDown`：只在自己要處理時回 `true`。
    pub fn test_key(&self, wparam: usize, lparam: isize) -> bool {
        if !self.queue.is_connected() {
            // 引擎不在就全部放行，讓使用者至少能打出原來的字母。
            return false;
        }
        keys::translate(wparam as u16, scan_code(lparam), self.composing()).is_some()
    }

    /// `OnKeyDown`：把按鍵送給 worker，立即返回。
    pub fn key_down(&self, context: Option<&ITfContext>, wparam: usize, lparam: isize) -> bool {
        if !self.queue.is_connected() {
            return false;
        }
        if let Some(context) = context {
            self.context.replace(Some(context.clone()));
        }

        let Some(mut batch) = keys::translate(wparam as u16, scan_code(lparam), self.composing())
        else {
            return false;
        };

        // 候選清單已顯示但還沒進選取模式時，數字鍵先補一個 `Down`：引擎的
        // 第一次 `Down` 只開候選窗、不動高亮，接著的數字才選得到當頁。
        // 這是送按鍵，不是 wrapper 自己選字。
        let prefix = needs_selection_prefix(
            &batch,
            self.last.get(),
            self.navigation_armed.get().is_some(),
        );
        if prefix {
            batch.insert(0, Key::new(KeyCode::Down, None, false));
        }

        self.generation.set(self.generation.get() + 1);
        let generation = self.generation.get();
        if prefix || batch.iter().any(enters_selection) {
            self.navigation_armed.set(Some(generation));
        }
        for key in batch {
            self.send(Job {
                generation,
                request: Request::Key { key },
            });
        }
        true
    }

    /// 焦點換到別的文件：丟掉舊 context 與組字，並清掉引擎裡殘留的狀態。
    fn clear_focus_state(&self) {
        let was_composing = self.composing();
        self.composition.replace(None);
        self.context.replace(None);
        self.last.set(LastState::default());
        self.navigation_armed.set(None);
        self.candidate.borrow_mut().end();
        if was_composing {
            logging::debug("焦點改變，送出 Escape 清掉引擎組字");
            self.push_key(Key::new(KeyCode::Escape, None, false));
        }
    }

    /// 組字被 TSF 外部終止（例如應用程式自己結束組字）。
    pub fn on_composition_terminated(&self) {
        if self.composition.replace(None).is_some() {
            logging::debug("組字被外部終止");
        }
        // 外部終止代表 context 可能已經換掉，候選窗不能再留著。
        self.last.set(LastState::default());
        self.candidate.borrow_mut().end();
    }

    /// 套用所有待處理的引擎回應。
    pub fn drain(&self) {
        for item in self.queue.drain() {
            // 這批按鍵的回應到了，之後可以重新依最新 State 決定要不要補 `Down`。
            if self
                .navigation_armed
                .get()
                .is_some_and(|armed| item.generation >= armed)
            {
                self.navigation_armed.set(None);
            }
            match item.result {
                Ok(Response::State {
                    preedit,
                    cursor,
                    cursor_rune,
                    candidates,
                    details,
                    highlighted,
                    navigating,
                    label,
                    ..
                }) => {
                    if item.generation < self.generation.get() {
                        logging::debug("丟棄過期的 State");
                        continue;
                    }
                    self.apply_state(StateData {
                        preedit,
                        cursor,
                        cursor_rune,
                        candidates,
                        details,
                        highlighted,
                        navigating,
                        label,
                    });
                }
                Ok(Response::Committed { text }) => self.apply_committed(&text),
                Ok(other) => logging::debug(format!("忽略非預期的回應：{other:?}")),
                Err(message) => {
                    logging::warn(format!("引擎回應失敗：{message}"));
                    self.last.set(LastState::default());
                    self.end_composition();
                    self.candidate.borrow_mut().end();
                }
            }
        }
    }

    /// 停止 worker 與 sink；訊息視窗在 [`Drop`] 收掉。
    pub fn shutdown(&mut self) {
        // Deactivate 必須確保 pipe 一定放掉，不論現在是不是在寬限期內。
        logging::debug(format!(
            "Deactivate：pipe 狀態 {:?}",
            self.pipe.borrow().state()
        ));
        let action = self.pipe.borrow_mut().deactivate();
        self.apply_pipe_action(action);

        // SAFETY: 這兩個 sink 都是在 Activate 時註冊的，只有這裡會反註冊。
        // 把反註冊的 HRESULT 也記下來：下一次同 id 重新 Activate 若失敗，
        // 就能從 log 判斷是這裡沒清乾淨，還是 msctf 自己還留著 sink。
        unsafe {
            if let Some(keystroke) = &self.keystroke {
                match keystroke.UnadviseKeyEventSink(self.client_id) {
                    Ok(()) => logging::debug(format!(
                        "shutdown：UnadviseKeyEventSink({}) = 0x00000000",
                        self.client_id
                    )),
                    Err(error) => logging::warn(format!(
                        "shutdown：UnadviseKeyEventSink({}) = {}",
                        self.client_id,
                        logging::hresult(error.code())
                    )),
                }
            }
            if let (Some(source), cookie) = (&self.source, self.source_cookie.get()) {
                if cookie != 0 {
                    match source.UnadviseSink(cookie) {
                        Ok(()) => {
                            logging::debug(format!("shutdown：UnadviseSink({cookie}) = 0x00000000"))
                        }
                        Err(error) => logging::warn(format!(
                            "shutdown：UnadviseSink({cookie}) = {}",
                            logging::hresult(error.code())
                        )),
                    }
                }
            }
        }
        self.keystroke = None;
        self.source = None;
        self.composition.replace(None);
        self.context.replace(None);
        self.last.set(LastState::default());
        self.candidate.borrow_mut().end();
        // 這兩個引用指回自己；不放手的話物件永遠不會被釋放。
        self.key_sink = None;
        self.thread_sink = None;
        self.sink_state.detach();
    }

    /// 註冊 key sink 與 thread manager 事件 sink。
    ///
    /// 每一步都單獨記一次 HRESULT：`Activate` 失敗時 `0x80070057` 可能來自這裡
    /// 任一呼叫，也可能是 `CreateWindowExW` 的 `GetLastError`，日誌要能分辨。
    fn advise(
        &mut self,
        key_sink: &ITfKeyEventSink,
        thread_sink: &ITfThreadMgrEventSink,
    ) -> Result<()> {
        // SAFETY: GetCurrentThreadId 沒有參數，也不碰呼叫端的記憶體。
        let thread_id = unsafe { GetCurrentThreadId() };
        // SAFETY: sink 的存活期由 Control 持有，反註冊在 shutdown。
        unsafe {
            let keystroke: ITfKeystrokeMgr = match self.thread_mgr.cast() {
                Ok(keystroke) => {
                    logging::debug("advise：cast ITfKeystrokeMgr = 0x00000000");
                    keystroke
                }
                Err(error) => {
                    logging::warn(format!(
                        "advise：cast ITfKeystrokeMgr = {}",
                        logging::hresult(error.code())
                    ));
                    return Err(error);
                }
            };

            if let Err(error) = keystroke.AdviseKeyEventSink(self.client_id, key_sink, true) {
                logging::warn(format!(
                    "advise：AdviseKeyEventSink = {}（client_id = {}，thread = {}）",
                    logging::hresult(error.code()),
                    self.client_id,
                    thread_id
                ));
                return Err(error);
            }
            logging::debug(format!(
                "advise：AdviseKeyEventSink = 0x00000000（client_id = {}，thread = {}）",
                self.client_id, thread_id
            ));
            self.keystroke = Some(keystroke);

            let source: ITfSource = match self.thread_mgr.cast() {
                Ok(source) => {
                    logging::debug("advise：cast ITfSource = 0x00000000");
                    source
                }
                Err(error) => {
                    logging::warn(format!(
                        "advise：cast ITfSource = {}",
                        logging::hresult(error.code())
                    ));
                    return Err(error);
                }
            };

            let unknown: IUnknown = match thread_sink.cast() {
                Ok(unknown) => unknown,
                Err(error) => {
                    logging::warn(format!(
                        "advise：cast IUnknown = {}",
                        logging::hresult(error.code())
                    ));
                    return Err(error);
                }
            };
            // riid 必須是 sink 自己的 IID（`IID_ITfThreadMgrEventSink`）；傳
            // `IID_IUnknown` 之類的泛型值會被 msctf 當成「不支援的 sink 類型」拒絕。
            match source.AdviseSink(&<ITfThreadMgrEventSink as Interface>::IID, &unknown) {
                Ok(cookie) => {
                    logging::debug(format!(
                        "advise：AdviseSink = 0x00000000（IID_ITfThreadMgrEventSink，cookie = {cookie}）"
                    ));
                    self.source = Some(source);
                    self.source_cookie.set(cookie);
                }
                Err(error) => {
                    logging::warn(format!(
                        "advise：AdviseSink = {}（IID_ITfThreadMgrEventSink）",
                        logging::hresult(error.code())
                    ));
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// 套用一個 `State`。
    fn apply_state(&self, state: StateData) {
        let previous = self.last.get();
        let current = LastState {
            preedit: state.preedit.chars().count(),
            candidates: state.candidates.len(),
            navigating: state.navigating,
        };
        self.last.set(current);

        self.update_candidates(&state);

        if state.preedit.is_empty() {
            self.end_composition();
        } else {
            // `cursor_rune` 是權威值，但 0 會被省略成「沒有欄位」；核心目前
            // 游標都在結尾，所以 0 時退回 `cursor`。
            let caret = if state.cursor_rune == 0 {
                state.cursor
            } else {
                state.cursor_rune
            };
            self.request(EditAction::Update {
                preedit: state.preedit,
                caret,
            });
        }

        if previous.composing() && !current.composing() {
            // Space／數字選字後引擎回空 State，文字留在待提交區，見
            // `docs/wrapper.md` §7.2。沒有候選窗也要補這一次 Commit，否則選到
            // 的字會消失。
            logging::debug("組字結束，補送 Commit 取回待提交文字");
            self.push_request(Request::Commit);
        }
    }

    /// 把候選清單交給 Windows；空清單就關窗。
    ///
    /// `preedit` 空但候選非空是合法狀態（符號／表情選單），所以關窗條件看
    /// 候選而不是看組字字串。
    fn update_candidates(&self, state: &StateData) {
        let document = match self.context.borrow().as_ref() {
            // SAFETY: context 由 TSF 在呼叫期間保持有效。
            Some(context) => unsafe { context.GetDocumentMgr().ok() },
            None => None,
        };
        let items = state
            .candidates
            .iter()
            .enumerate()
            .map(|(index, text)| CandidateItem {
                text: text.clone(),
                reading: state
                    .details
                    .get(index)
                    .map(|detail| detail.reading.clone())
                    .unwrap_or_default(),
                label: if index == state.highlighted {
                    state.label.clone()
                } else {
                    None
                },
            })
            .collect();
        self.candidate.borrow_mut().update(
            CandidateSnapshot {
                items,
                highlighted: state.highlighted,
            },
            document,
        );
    }

    /// 套用一次提交。
    fn apply_committed(&self, text: &str) {
        self.last.set(LastState::default());
        self.candidate.borrow_mut().end();
        if text.is_empty() {
            self.end_composition();
            return;
        }
        self.request(EditAction::Commit {
            text: text.to_string(),
        });
    }

    /// 送一個按鍵（自己產生的，例如焦點切換時的 Escape）。
    fn push_key(&self, key: Key) {
        self.generation.set(self.generation.get() + 1);
        let generation = self.generation.get();
        self.send(Job {
            generation,
            request: Request::Key { key },
        });
    }

    /// 送一個非按鍵請求。
    fn push_request(&self, request: Request) {
        self.generation.set(self.generation.get() + 1);
        let generation = self.generation.get();
        self.send(Job {
            generation,
            request,
        });
    }

    /// 丟給 dispatch 執行緒；它會轉送給當前的 worker。
    fn send(&self, job: Job) {
        if self.bridge_tx.send(job).is_err() {
            logging::warn("dispatch 已結束，請求被丟棄");
        }
    }

    /// 在 edit session 裡結束組字。
    fn end_composition(&self) {
        if self.context.borrow().is_none() {
            // 沒有 context 就沒有東西可以結束，清掉殘留的參照即可。
            self.composition.replace(None);
            return;
        }
        // 先取出來再判斷，避免 `borrow` 的暫時值活到 `request` 內部。
        let has_composition = self.composition.borrow().is_some();
        if has_composition {
            self.request(EditAction::End);
        }
    }

    /// 以 `TF_ES_SYNC` 送出一次編輯；context 失效時清掉它。
    fn request(&self, action: EditAction) {
        let Some(context) = self.context.borrow().clone() else {
            logging::debug("沒有 context，忽略這次套用");
            return;
        };
        let session: ITfEditSession = ApplySession {
            control: self as *const Control,
            context: context.clone(),
            action,
        }
        .into();

        // SAFETY: session 在同步呼叫期間保持存活；Control 由 TextService 持有。
        match unsafe {
            context.RequestEditSession(self.client_id, &session, TF_ES_SYNC | TF_ES_READWRITE)
        } {
            Ok(result) if result.is_ok() => {}
            Ok(result) => logging::warn(format!("edit session 失敗：{result:?}")),
            Err(error) => {
                logging::warn(format!("RequestEditSession 失敗：{error}"));
                self.context.replace(None);
            }
        }
    }

    /// 在 edit session 內真正做事；呼叫前已重新確認 context。
    fn run_edit(&self, ec: u32, expected: &ITfContext, action: &EditAction) -> Result<()> {
        let Some(context) = self.context.borrow().clone() else {
            return Ok(());
        };
        if Interface::as_raw(&context) != Interface::as_raw(expected) {
            logging::debug("context 已改變，丟棄這次套用");
            return Ok(());
        }

        match action {
            EditAction::Update { preedit, caret } => {
                self.set_preedit(ec, &context, preedit, *caret)
            }
            EditAction::End => {
                self.close_composition(ec, &context);
                Ok(())
            }
            EditAction::Commit { text } => {
                self.close_composition(ec, &context);
                self.insert_text(ec, &context, text)
            }
        }
    }

    /// 更新或建立組字。
    fn set_preedit(
        &self,
        ec: u32,
        context: &ITfContext,
        preedit: &str,
        caret: usize,
    ) -> Result<()> {
        let wide: Vec<u16> = preedit.encode_utf16().collect();

        // 先把既有的組字取出來；`borrow` 的暫時值不能活到後面的 `replace`。
        let existing = self.composition.borrow().clone();
        let (composition, fresh) = match existing {
            Some(composition) => (composition, false),
            None => {
                let anchor = self.selection_range(ec, context)?;
                let factory: ITfContextComposition = context.cast()?;
                // SAFETY: sink 由 Control 持有；StartComposition 只在這個 edit session 內使用。
                let composition = unsafe { factory.StartComposition(ec, &anchor, &self.sink) }?;
                self.composition.replace(Some(composition.clone()));
                (composition, true)
            }
        };

        // SAFETY: 所有範圍都在同一個 edit session 內使用。
        let range = unsafe { composition.GetRange()? };
        unsafe {
            // 第一次寫入用 0；之後都是修正，讓 TSF 不要把整段當成新輸入。
            let flags = if fresh { 0 } else { TF_ST_CORRECTION };
            range.SetText(ec, flags, &wide)?;

            // TSF 的 ITfRange::Clone 會複製 anchors；直接用同一個範圍去 Collapse
            // 會把組字本身縮成空字串。
            let caret_range = range.Clone()?;
            caret_range.Collapse(ec, TF_ANCHOR_START)?;
            let mut shifted = 0i32;
            caret_range.ShiftEnd(ec, utf16_offset(preedit, caret), &mut shifted, ptr::null())?;
            caret_range.Collapse(ec, TF_ANCHOR_END)?;
            select(context, ec, caret_range)?;
        }

        // 底線套在整個組字範圍上；每次更新都重設，字數變動也會跟著調整。
        self.set_display_attribute(ec, context, &range);
        Ok(())
    }

    /// 結束組字；先取出來再呼叫，避免 `OnCompositionTerminated` 重入時撞到借用。
    ///
    /// 底線屬性必須在 `EndComposition` 之前清掉，否則範圍消失後就找不到
    /// `ITfProperty` 的值可以移除。
    fn close_composition(&self, ec: u32, context: &ITfContext) {
        let Some(composition) = self.composition.replace(None) else {
            return;
        };
        self.clear_display_attribute(ec, context, &composition);
        // SAFETY: 這個 edit session 有讀寫權。
        if let Err(error) = unsafe { composition.EndComposition(ec) } {
            logging::warn(format!("EndComposition 失敗：{error}"));
        }
    }

    /// 把顯示屬性 atom 套到組字範圍。
    ///
    /// 失敗只記錄 debug log：底線是裝飾，不能因此讓組字或提交失敗。
    fn set_display_attribute(&self, ec: u32, context: &ITfContext, range: &ITfRange) {
        let Some(atom) = self.display_atom else {
            return;
        };
        let result = (|| -> Result<()> {
            let property: ITfProperty = unsafe { context.GetProperty(&GUID_PROP_ATTRIBUTE)? };
            let mut value = VARIANT::default();
            // SAFETY: vt 設成 VT_I4 之後，lVal 是有效的 union 分支。
            unsafe {
                (*value.Anonymous.Anonymous).vt = VT_I4;
                (*value.Anonymous.Anonymous).Anonymous.lVal = atom as i32;
            }
            // SAFETY: property 與 range 都屬於這個 edit session。
            unsafe { property.SetValue(ec, range, &value) }
        })();
        if let Err(error) = result {
            logging::debug(format!("設定組字底線失敗：{error}"));
        }
    }

    /// 清掉組字範圍上的顯示屬性；失敗同樣只記錄。
    fn clear_display_attribute(&self, ec: u32, context: &ITfContext, composition: &ITfComposition) {
        if self.display_atom.is_none() {
            return;
        }
        let result = (|| -> Result<()> {
            // SAFETY: composition 仍在這個 edit session 內有效。
            let range = unsafe { composition.GetRange()? };
            let property: ITfProperty = unsafe { context.GetProperty(&GUID_PROP_ATTRIBUTE)? };
            // SAFETY: property 與 range 都屬於這個 edit session。
            unsafe { property.Clear(ec, &range) }
        })();
        if let Err(error) = result {
            logging::debug(format!("清除組字底線失敗：{error}"));
        }
    }

    /// 在目前選取範圍插入文字。
    fn insert_text(&self, ec: u32, context: &ITfContext, text: &str) -> Result<()> {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let insert: ITfInsertAtSelection = context.cast()?;
        // SAFETY: 在 edit session 內插入；flags 0 代表真的寫入並回傳插入範圍。
        unsafe {
            let range =
                insert.InsertTextAtSelection(ec, INSERT_TEXT_AT_SELECTION_FLAGS(0), &wide)?;
            let caret = range.Clone()?;
            caret.Collapse(ec, TF_ANCHOR_END)?;
            select(context, ec, caret)?;
        }
        Ok(())
    }

    /// 取得選取範圍，用來當組字的附著點。
    ///
    /// `StartComposition` 要求一個**空**範圍；選取範圍本身就是空的游標位置，
    /// 但所有權屬於 TSF，所以先做一次 TSF 的 `Clone`，避免之後改動到選取。
    fn selection_range(&self, ec: u32, context: &ITfContext) -> Result<ITfRange> {
        let mut selection = [TF_SELECTION::default()];
        let mut fetched = 0u32;
        // SAFETY: 一個 selection 的緩衝；TSF 會填入實際數量。
        unsafe {
            context.GetSelection(ec, TF_DEFAULT_SELECTION, &mut selection, &mut fetched)?;
        }
        if fetched == 0 {
            return Err(windows::core::Error::from_hresult(E_FAIL));
        }
        // SAFETY: selection 不再使用，可以安全取出 ManuallyDrop 的內容。
        let range = unsafe { ManuallyDrop::take(&mut selection[0].range) };
        let Some(range) = range else {
            return Err(windows::core::Error::from_hresult(E_FAIL));
        };
        // SAFETY: 在 edit session 內複製範圍。
        let anchor = match unsafe { range.Clone() } {
            Ok(copy) => copy,
            Err(_) => range,
        };
        // 有選取範圍時 `StartComposition` 會拒絕非空範圍，先收成游標。
        // SAFETY: 同一個 edit session。
        unsafe { anchor.Collapse(ec, TF_ANCHOR_START)? };
        Ok(anchor)
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        // 萬一沒走 Deactivate 就要被釋放，也要先把 worker 的 sender 清掉。
        self.stop_engine_worker();
        self.sink_state.detach();
        self.queue.attach(0);
        self.candidate.borrow_mut().end();
        if !self.hwnd.is_invalid() {
            // SAFETY: 視窗由這個執行緒建立，也在這個執行緒摧毀。
            unsafe {
                SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
                let _ = DestroyWindow(self.hwnd);
            }
            self.hwnd = HWND::default();
        }
    }
}

/// 把游標設到指定範圍的結尾。
fn select(context: &ITfContext, ec: u32, range: ITfRange) -> Result<()> {
    let selection = TF_SELECTION {
        range: ManuallyDrop::new(Some(range)),
        style: TF_SELECTIONSTYLE {
            ase: TF_AE_END,
            fInterimChar: FALSE,
        },
    };
    // SAFETY: 一個有效的 selection 陣列。
    unsafe { context.SetSelection(ec, &[selection]) }
}

/// 前 `rune_index` 個 rune 佔幾個 UTF-16 code unit。
fn utf16_offset(text: &str, rune_index: usize) -> i32 {
    let mut units = 0usize;
    for (index, ch) in text.chars().enumerate() {
        if index >= rune_index {
            break;
        }
        units += ch.len_utf16();
    }
    units as i32
}

/// 從 `lparam` 取掃描碼。
fn scan_code(lparam: isize) -> u32 {
    ((lparam >> 16) & 0xff) as u32
}

/// 這個 `KeyCode` 是否會讓引擎進入候選選取模式。
fn enters_selection(key: &Key) -> bool {
    matches!(
        key.code,
        KeyCode::Down | KeyCode::Up | KeyCode::PageUp | KeyCode::PageDown
    )
}

/// 是否要在這批按鍵前面補一個 `Down`。
///
/// 條件：候選清單已顯示（`candidates > 0`）、還沒進明確選取模式、沒有已經
/// 送出但還沒回覆的導覽鍵，且按的是單一 `1`–`9`。引擎的第一次 `Down` 只開
/// 候選窗、不動高亮，所以補這一下不會改變使用者看到的選取項目。
///
/// `armed` 是為了快速連打：使用者先按 `Down` 再立刻按 `1` 時，`Down` 的
/// `State` 可能還沒回來；此時不能再補一次 `Down`，否則引擎會多移一格。
fn needs_selection_prefix(batch: &[Key], last: LastState, armed: bool) -> bool {
    if last.candidates == 0 || last.navigating || armed || batch.len() != 1 {
        return false;
    }
    match batch[0] {
        Key {
            code: KeyCode::Char(ch),
            shift: false,
            ctrl: false,
            ..
        } => ch.is_ascii_digit() && ch != '0',
        _ => false,
    }
}

/// 組字 sink 共用的狀態；`Deactivate` 之後指標會被清成 null。
struct SinkState {
    control: AtomicPtr<Control>,
}

impl SinkState {
    fn new() -> Self {
        Self {
            control: AtomicPtr::new(ptr::null_mut()),
        }
    }

    fn attach(&self, control: *const Control) {
        self.control
            .store(control as *mut Control, Ordering::Release);
    }

    fn detach(&self) {
        self.control.store(ptr::null_mut(), Ordering::Release);
    }

    fn notify(&self) {
        let pointer = self.control.load(Ordering::Acquire);
        if pointer.is_null() {
            return;
        }
        // SAFETY: 指標只在 Control 存活期間非 null。
        unsafe { (*pointer).on_composition_terminated() }
    }
}

/// `ITfCompositionSink` 的實作物件。
///
/// 獨立成一個物件而不是讓 [`crate::TextService`] 自己兼任，是為了避免「物件
/// 持有自己的介面引用」造成 refcount 永遠不歸零。
#[implement(ITfCompositionSink)]
struct CompositionSink {
    state: Arc<SinkState>,
}

impl ITfCompositionSink_Impl for CompositionSink_Impl {
    fn OnCompositionTerminated(
        &self,
        _ecwrite: u32,
        _pcomposition: Ref<ITfComposition>,
    ) -> Result<()> {
        self.get_impl().state.notify();
        Ok(())
    }
}

/// 單次編輯的 edit session。
#[implement(ITfEditSession)]
struct ApplySession {
    control: *const Control,
    context: ITfContext,
    action: EditAction,
}

impl ITfEditSession_Impl for ApplySession_Impl {
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        let session = self.get_impl();
        // SAFETY: Control 由 TextService 持有；TF_ES_SYNC 期間不會被釋放。
        let control = unsafe { &*session.control };
        control.run_edit(ec, &session.context, &session.action)
    }
}

/// 取得**這份 DLL** 的 `HINSTANCE`。
///
/// `GetModuleHandleW(None)` 拿到的是宿主行程的 exe：TSF 會把 TIP 載進任意程式，
/// 用 exe 的 handle 註冊視窗 class，class 就會掛在錯的模組上（`GCLP_HMODULE`、
/// 之後的 `UnregisterClassW` 都對不上）。`GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS`
/// 用本模組內的函式位址反查，才是真正裝著這段程式碼的 DLL。
fn module_instance() -> Result<HINSTANCE> {
    let mut module = HMODULE::default();
    // SAFETY: anchor 是本模組內的函式位址；UNCHANGED_REFCOUNT 不增加參考計數，
    // 所以不需要（也不能）對 module 呼叫 FreeLibrary。
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(module_instance as *const () as *const u16),
            &mut module,
        )?;
    }
    Ok(HINSTANCE(module.0))
}

/// 建立訊息視窗。
///
/// 每一步都留日誌：這個函式失敗時 `Activate` 只會看到
/// `HRESULT_FROM_WIN32(GetLastError)`，事後猜不出是哪一步。
fn create_message_window(control: *const Control) -> Result<HWND> {
    let class_name = crate::wide(WINDOW_CLASS);
    let instance = module_instance()?;
    // SAFETY: 視窗與 class 都只在這個執行緒建立與註冊。
    unsafe {
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        // 回 0 代表「這次沒註冊成功」，兩種情形：class 已存在（同一個行程重複
        // Activate，正常）或真的失敗。成功時 `GetLastError` 沒有意義，照樣印出
        // 實際值，方便比對。
        let registered = RegisterClassW(&class);
        let register_error = GetLastError();
        if registered == 0 && register_error != ERROR_CLASS_ALREADY_EXISTS {
            logging::warn(format!(
                "create_message_window：RegisterClassW = 0x{registered:04X}，GetLastError = {}",
                register_error.0
            ));
        } else {
            logging::debug(format!(
                "create_message_window：RegisterClassW = 0x{registered:04X}，GetLastError = {}",
                register_error.0
            ));
        }

        // 訊息視窗不需要任何樣式（`WINDOW_STYLE(0)` 就是 `WS_OVERLAPPED`）；
        // 加 `WS_POPUP` 或 `WS_CHILD` 都不會讓它更正確，`WS_CHILD` 反而會讓
        // `CreateWindowExW` 直接失敗。
        let result = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(class_name.as_ptr()),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance),
            None,
        );
        let hwnd = match result {
            Ok(hwnd) => hwnd,
            Err(error) => {
                logging::warn(format!(
                    "create_message_window：CreateWindowExW 失敗，GetLastError = {}（HRESULT {}）",
                    GetLastError().0,
                    logging::hresult(error.code())
                ));
                return Err(error);
            }
        };
        logging::debug(format!(
            "create_message_window：CreateWindowExW = {:?}",
            hwnd.0
        ));
        // 32 位元 Windows 的 `SetWindowLongPtrW` 就是 `SetWindowLongW`，第三個
        // 參數是 `i32`；64 位元才是 `isize`。指標只當不透明值傳遞，不是真的截斷
        // （32 位元的指標本來就是 32 位元）。
        #[cfg(target_pointer_width = "64")]
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, control as isize);
        #[cfg(target_pointer_width = "32")]
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, control as i32);
        Ok(hwnd)
    }
}

/// 訊息視窗的 window proc：只處理「引擎有回應」。
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == engine::WM_ENGINE_RESPONSE {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Control;
        if !pointer.is_null() {
            (*pointer).drain();
        }
        return LRESULT(0);
    }
    // SAFETY: 其他訊息一律交回預設處理。
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Activate 不連；OnSetFocus 連；OnKillFocus 後立刻回來只取消，不重連；
    /// Deactivate 確保放掉。
    #[test]
    fn focus_pipe_connects_once_and_releases_on_deactivate() {
        let mut pipe = FocusPipe::new();
        assert_eq!(pipe.state(), PipeState::Off);

        // Activate 本身沒有任何 pipe 動作。
        assert_eq!(pipe.focus_gained(), PipeAction::Start);
        assert_eq!(pipe.state(), PipeState::Active);
        // 同一個文字框再收到一次 set focus，不重複連線。
        assert_eq!(pipe.focus_gained(), PipeAction::None);
        assert_eq!(pipe.state(), PipeState::Active);

        assert_eq!(pipe.focus_lost(), PipeAction::Disconnect);
        assert_eq!(pipe.state(), PipeState::Draining);
        assert_eq!(pipe.focus_lost(), PipeAction::None);
        assert_eq!(pipe.state(), PipeState::Draining);

        // 寬限期內的 OnSetFocus 只取消斷線。
        assert_eq!(pipe.focus_gained(), PipeAction::Cancel);
        assert_eq!(pipe.state(), PipeState::Active);

        assert_eq!(pipe.deactivate(), PipeAction::Stop);
        assert_eq!(pipe.state(), PipeState::Off);
        assert_eq!(pipe.deactivate(), PipeAction::None);
    }

    /// 從 Draining 直接 Deactivate 也要收斂到 Off，之後仍能重新啟動。
    #[test]
    fn focus_pipe_deactivate_from_draining_stops() {
        let mut pipe = FocusPipe::new();
        assert_eq!(pipe.focus_gained(), PipeAction::Start);
        assert_eq!(pipe.focus_lost(), PipeAction::Disconnect);
        assert_eq!(pipe.state(), PipeState::Draining);
        assert_eq!(pipe.deactivate(), PipeAction::Stop);
        assert_eq!(pipe.state(), PipeState::Off);
        assert_eq!(pipe.focus_gained(), PipeAction::Start);
    }

    #[test]
    fn composing_needs_preedit_or_candidates() {
        assert!(!LastState::default().composing());
        assert!(LastState {
            preedit: 1,
            candidates: 0,
            navigating: false,
        }
        .composing());
        assert!(LastState {
            preedit: 0,
            candidates: 2,
            navigating: false,
        }
        .composing());
    }

    #[test]
    fn digit_gets_a_down_prefix_only_before_selection_mode() {
        let digit = [Key::char('3', false)];
        let visible = LastState {
            preedit: 1,
            candidates: 2,
            navigating: false,
        };
        assert!(needs_selection_prefix(&digit, visible, false));
        assert!(!needs_selection_prefix(&digit, visible, true));
        assert!(!needs_selection_prefix(
            &digit,
            LastState {
                navigating: true,
                ..visible
            },
            false
        ));
        assert!(!needs_selection_prefix(&digit, LastState::default(), false));
        assert!(!needs_selection_prefix(
            &[Key::char('0', false)],
            visible,
            false
        ));
        assert!(!needs_selection_prefix(
            &[Key::new(KeyCode::Down, None, false)],
            visible,
            false
        ));
        assert!(!needs_selection_prefix(
            &[Key::char('ㄅ', false)],
            visible,
            false
        ));
        assert!(!needs_selection_prefix(
            &[Key::char('3', true)],
            visible,
            false
        ));
        assert!(enters_selection(&Key::new(KeyCode::PageDown, None, false)));
        assert!(!enters_selection(&Key::new(KeyCode::Tab, None, false)));
    }

    #[test]
    fn utf16_offset_counts_surrogate_pairs() {
        assert_eq!(utf16_offset("abc", 0), 0);
        assert_eq!(utf16_offset("abc", 2), 2);
        assert_eq!(utf16_offset("a😀b", 2), 3);
        assert_eq!(utf16_offset("a😀b", 99), 4);
    }

    #[test]
    fn scan_code_comes_from_the_high_word() {
        assert_eq!(scan_code(0x003a_0001), 0x3a);
        assert_eq!(scan_code(0), 0);
    }
}
