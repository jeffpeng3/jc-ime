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
use std::sync::mpsc::Sender;
use std::sync::Arc;

use jc_proto::{CandidateDetail, Key, KeyCode, Request, Response};
use windows::core::{implement, IUnknownImpl, Interface, Ref, Result, PCWSTR};
use windows::Win32::Foundation::{E_FAIL, FALSE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
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
    jobs: Sender<Job>,
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
    /// 建立狀態並啟動 worker 執行緒。
    pub fn new(thread_mgr: ITfThreadMgr, client_id: u32) -> Self {
        let (jobs, receiver) = std::sync::mpsc::channel();
        let queue = Arc::new(Queue::new());
        let queue_for_worker = Arc::clone(&queue);
        engine::spawn(receiver, queue_for_worker);

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
            jobs.clone(),
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
            jobs,
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
    pub fn on_focus_changed(&self) {
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
        // SAFETY: 這兩個 sink 都是在 Activate 時註冊的，只有這裡會反註冊。
        unsafe {
            if let Some(keystroke) = &self.keystroke {
                let _ = keystroke.UnadviseKeyEventSink(self.client_id);
            }
            if let (Some(source), cookie) = (&self.source, self.source_cookie.get()) {
                if cookie != 0 {
                    let _ = source.UnadviseSink(cookie);
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
    fn advise(
        &mut self,
        key_sink: &ITfKeyEventSink,
        thread_sink: &ITfThreadMgrEventSink,
    ) -> Result<()> {
        // SAFETY: sink 的存活期由 Control 持有，反註冊在 shutdown。
        unsafe {
            let keystroke: ITfKeystrokeMgr = self.thread_mgr.cast()?;
            keystroke.AdviseKeyEventSink(self.client_id, key_sink, true)?;
            self.keystroke = Some(keystroke);

            let source: ITfSource = self.thread_mgr.cast()?;
            let unknown: windows::core::IUnknown = thread_sink.cast()?;
            let cookie = source.AdviseSink(&<ITfThreadMgrEventSink as Interface>::IID, &unknown)?;
            self.source = Some(source);
            self.source_cookie.set(cookie);
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

    /// 丟給 worker；worker 已經結束時只記日誌。
    fn send(&self, job: Job) {
        if self.jobs.send(job).is_err() {
            logging::warn("worker 已結束，請求被丟棄");
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

/// 建立訊息視窗。
fn create_message_window(control: *const Control) -> Result<HWND> {
    let class_name = crate::wide(WINDOW_CLASS);
    // SAFETY: 視窗與 class 都只在這個執行緒建立與註冊。
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: windows::Win32::Foundation::HINSTANCE(instance.0),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        // 已經註冊過會回 0，這是正常情況，不用處理。
        let _ = RegisterClassW(&class);

        let hwnd = CreateWindowExW(
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
            Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
            None,
        )?;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, control as isize);
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
