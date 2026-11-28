//! TextService：TSF 的 text input processor。移植自 0c `service.rs`（main `8f443af`）。
//!
//! 只接事件、送協定、依快照更新組字區；狀態集中在 [`State`]，所有 COM 回呼都包在
//! [`crate::guarded`] 裡，可能重入的位置用 `try_borrow_mut` 回錯而非 panic。
//!
//! 兩條硬要求（verify-su-port 發現，不可違反）：
//!
//! 1. **不補 Down**：[`TextService::handle_key`] 照 0c 的做法把數字直接送給核心。
//!    舊 `jc-tsf` 的 `needs_selection_prefix()` 會在單一數字前插入 `KeyCode::Down`，
//!    讓 `su` + `3` 被當成選字鍵；核心 su 修復已 cherry-pick，補 Down 就會舊病復發。
//! 2. **英文模式文字輸入**：沒有組字時 printable、Tab、方向鍵、Enter、Backspace
//!    與修飾鍵都不送引擎請求、不建組字、不顯示候選窗，直接交還宿主；唯一例外
//!    是已註冊的全域 chord（選單快捷鍵），它會送一個 `Request::Key` 讓 engine
//!    依 keymap 執行 command。組字中（例如切英文前已開始的注音）printable 改送
//!    `Request::Literal`，由 engine 接在既有組字後面，宿主端組字不中斷。
//!
//! 與 jc 現行外殼的另一個關鍵差異：Activate 保留 idempotent guard；Deactivate 只清
//! UI／composition／callback，保留 `session` 與它持有的 IPC client，避免第二次
//! Activate 的 `TF_E_NOLOCK` 又把 worker 拆掉。

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use serde_json::json;
use windows::Win32::Foundation::{E_INVALIDARG, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::System::Ole::CONNECT_E_ADVISELIMIT;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::Variant::{VariantClear, VARIANT, VT_I4};
use windows::Win32::UI::TextServices::{
    ITfCompartment, ITfCompartmentEventSink, ITfCompartmentEventSink_Impl, ITfCompartmentMgr,
    ITfComposition, ITfCompositionSink, ITfCompositionSink_Impl, ITfContext, ITfContextView,
    ITfDisplayAttributeProvider, ITfDisplayAttributeProvider_Impl, ITfDocumentMgr, ITfKeyEventSink,
    ITfKeyEventSink_Impl, ITfKeystrokeMgr, ITfLangBarItem, ITfLangBarItemButton, ITfLangBarItemMgr,
    ITfSource, ITfTextInputProcessor, ITfTextInputProcessorEx, ITfTextInputProcessorEx_Impl,
    ITfTextInputProcessor_Impl, ITfTextLayoutSink, ITfTextLayoutSink_Impl, ITfThreadMgr,
    ITfThreadMgrEventSink, ITfThreadMgrEventSink_Impl, ITfUIElementMgr, TfLayoutCode,
    GUID_COMPARTMENT_KEYBOARD_OPENCLOSE, TF_INVALID_COOKIE, TF_LC_DESTROY, TF_TMAE_COMLESS,
    TF_TMAE_CONSOLE, TF_TMAE_SECUREMODE, TF_TMAE_UIELEMENTENABLEDONLY, TF_TMF_IMMERSIVEMODE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, MB_OK,
};
use windows_core::{implement, ComObject, IUnknownImpl, Interface, Ref, BOOL, GUID};

use crate::candidate;
use crate::candidate_layout;
use crate::caret;
use crate::cuas::{Cuas, Win32Cuas};
use crate::displayattr::{AttributeEnum, InputAttribute, KINDS};
use crate::edit;
use crate::keys;
use crate::langbar;
use crate::log::{self, zdebug, zlog};
use crate::privacy;
use crate::route::{english_route, EnglishRoute};
use crate::session::{Session, Snapshot};
use crate::ui::{self, UiCandidate, UiMessage};

/// 按鍵路徑 debug log 用：vk 轉可讀字元（可列印 ASCII 才轉，其餘顯示 `-`）。
fn vk_label(vk: u32) -> String {
    if (0x20..=0x7e).contains(&vk) {
        ((vk as u8) as char).to_string()
    } else {
        "-".to_owned()
    }
}

/// `AdviseKeyEventSink` 失敗時，把 HRESULT 轉成可執行的原因說明。
///
/// `E_INVALIDARG` 可能來自 client_id 不是已激活 TIP 的 id、client_id=0，或
/// sink 的 `ITfKeyEventSink` QI 失敗；日誌要能直接指出這三種可能。
fn advise_failure_hint(code: windows_core::HRESULT) -> &'static str {
    if code == E_INVALIDARG {
        "msctf 不認得這個 client_id（不是已激活 TIP 的 TIP client id）、client_id=0，或 key sink 的 QI 失敗"
    } else if code == CONNECT_E_ADVISELIMIT {
        "這個 client_id 已經有 key sink；TIP 已由 msctf 激活"
    } else {
        "見 HRESULT"
    }
}

/// 同一個畫面版本最多因 OnLayoutChange 重查幾次。
const MAX_REQUERIES: u32 = 8;
/// 換欄位時，這段時間內已從引擎得知「輸出簡體字」就不再問（一次點擊常連著多次焦點變化）。
const SIMPLIFIED_FRESH: Duration = Duration::from_secs(1);
/// 標記 `edit::apply` 進行中；宿主可能在我們寫入時送 OnLayoutChange，這時 shown 與 revision
/// 還是上一鍵的，不能拿來重送。離開（含 panic）時恢復。
struct Applying<'a>(&'a Cell<u32>);
impl<'a> Applying<'a> {
    fn enter(depth: &'a Cell<u32>, missed: &Cell<bool>) -> Self {
        if depth.get() == 0 {
            missed.set(false);
        }
        depth.set(depth.get() + 1);
        Applying(depth)
    }
}
impl Drop for Applying<'_> {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(1));
    }
}

/// 請系統相容層完成組字的期間（#66）：進入時記下那時的組字，離開（含 panic）時解除。
/// 相容層有沒有終止這個組字是觀察到的：OnCompositionTerminated 對上守衛的組字時記在 `terminated`。
struct Completing<'a>(&'a RefCell<State>);
impl<'a> Completing<'a> {
    fn enter(state: &'a RefCell<State>, composition: Option<ITfComposition>) -> Self {
        state.borrow_mut().completing = Some(CompletingState {
            composition,
            terminated: false,
        });
        Completing(state)
    }
    /// 相容層同步終止了守衛記下的那個組字（OnCompositionTerminated 看到的）。
    fn terminated(&self) -> bool {
        self.0
            .borrow()
            .completing
            .as_ref()
            .is_some_and(|state| state.terminated)
    }
}
impl Drop for Completing<'_> {
    fn drop(&mut self) {
        // 用 try：unwind 中不該再 panic（正常離開時沒有人借用 state）。
        if let Ok(mut state) = self.0.try_borrow_mut() {
            state.completing = None;
        }
    }
}

/// 建立 TIP 的 session。
///
/// Windows 上接真正的 named pipe transport；連不上時它會自己啟動引擎。
/// 其他平台（Linux 的 `cargo test`）只有 `NullTransport`，沒有引擎可連。
fn new_session() -> Session {
    #[cfg(windows)]
    {
        Session::with_client(crate::session::Client::with_transport(Box::new(
            crate::ipc_transport::PipeTransport::engine(),
        )))
    }
    #[cfg(not(windows))]
    {
        Session::new()
    }
}

/// `State.completing`：守衛記下的組字，與相容層是否終止了它。
struct CompletingState {
    composition: Option<ITfComposition>,
    terminated: bool,
}

/// 已經攔下、重排給程式的快捷鍵（#66）：重排的鍵回來之前，同一個 (vk, lParam) 的 TestKeyDown 就是它回來了。
struct PendingRepost {
    vk: u32,
    lparam: isize,
    /// 這一鍵的 OnKeyDown 已經來過（吃下，只吃一次）。
    key_down_seen: bool,
}

/// `complete_via_cuas` 的結果：組字是相容層終止的，還是外殼自己收尾的。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Completed {
    ByCuas,
    BySelf,
}

/// 重送舊畫面時，提示已超過候選窗自己收起的時間就放棄這次重送。
fn drop_expired_notice(message: &mut UiMessage, age: Duration) -> bool {
    if ui::notice_expires(message) && age >= Duration::from_millis(ui::NOTICE_MS) {
        message.notice.clear();
        return false;
    }
    true
}

/// 候選窗最近一次的呈現：OnLayoutChange 時重查游標位置並補送。
struct Shown {
    ctx: ITfContext,
    cursor: usize,
    owner: Option<HWND>,
    document: Option<ITfDocumentMgr>,
    /// 位置來自 GetTextExt（不是退路）。
    exact: bool,
    /// 這個畫面版本已經因 OnLayoutChange 重查幾次。
    requeries: u32,
    since: Instant,
    /// 最近一次送給候選窗的訊息；版面變了要改座標後原樣重送。
    message: UiMessage,
}

/// 執行緒的焦點文件改變時，進行中的組字怎麼辦（移植自 0c `service.rs`）。
#[derive(Debug, PartialEq, Eq)]
enum FocusChange {
    /// 焦點落在組字所在的文件（Excel 的儲存格從「就緒」進入編輯），繼續組字。
    CompositionMoved,
    /// 組字中焦點移到 NULL：可能是宿主正在交接，稍後再確認。
    Undecided,
    /// 換到別的文件，或沒在組字：送出組字。
    Left,
}

fn focus_change<T: PartialEq>(focus: Option<&T>, owner: Option<&T>) -> FocusChange {
    match (focus, owner) {
        (_, None) => FocusChange::Left,
        (Some(focus), Some(owner)) if focus == owner => FocusChange::CompositionMoved,
        (None, Some(_)) => FocusChange::Undecided,
        (Some(_), Some(_)) => FocusChange::Left,
    }
}

/// 焦點變化之後要做什麼。
#[derive(Debug, PartialEq, Eq)]
enum FocusStep<C> {
    Keep,
    Wait(Option<C>),
    Leave(Option<C>),
}

fn focus_step<T: PartialEq, C>(
    focus: Option<&T>,
    owner: Option<&T>,
    previous: Option<C>,
    can_wait: bool,
) -> FocusStep<C> {
    match focus_change(focus, owner) {
        FocusChange::CompositionMoved => FocusStep::Keep,
        FocusChange::Undecided if can_wait => FocusStep::Wait(previous),
        _ => FocusStep::Leave(previous),
    }
}

fn settle_step<T: PartialEq, C>(current: Option<&T>, owner: Option<&T>) -> FocusStep<C> {
    match focus_change(current, owner) {
        FocusChange::CompositionMoved => FocusStep::Keep,
        _ => FocusStep::Leave(None),
    }
}

/// 英文模式這一鍵怎麼處理（`english_key_policy`）。
#[derive(Debug, PartialEq)]
enum EnglishKey {
    /// 不介入，交還宿主。
    Pass,
    /// 直接交給應用程式的字面文字（IMM32 相容層用）。
    Literal(String),
}

impl EnglishKey {
    fn eats(&self) -> bool {
        matches!(self, EnglishKey::Literal(_))
    }
}

/// 打錯注音時的系統預設提示音（照使用者的音效配置）。
fn bell() {
    let ok = unsafe { MessageBeep(MB_OK) }.is_ok();
    zdebug!("bell: MessageBeep ok={ok}");
}

/// `callback` 是候選窗的訊息視窗（Activate 建立、Deactivate 拆除）；它決定焦點 `Undecided`
/// 能不能等非同步確認，也接收 `WM_SELECT_CANDIDATE`。建立失敗時照 0c 立即送出。
#[implement(
    ITfTextInputProcessorEx,
    ITfTextInputProcessor,
    ITfKeyEventSink,
    ITfCompositionSink,
    ITfThreadMgrEventSink,
    ITfDisplayAttributeProvider,
    ITfTextLayoutSink,
    ITfCompartmentEventSink
)]
pub struct TextService {
    state: RefCell<State>,
    /// `edit::apply` 進行中的巢狀深度。
    applying: Cell<u32>,
    /// apply 進行中收到、被略過的版面通知。
    layout_missed: Cell<bool>,
    /// Activate/Deactivate 成對的 idempotent guard（jc 既有做法）。
    activated: Cell<bool>,
    /// panic 之後 `state` 剛好被借用、重設做不完時留下旗標，下一次按鍵先補做。
    needs_reset: Cell<bool>,
}

struct State {
    thread_mgr: Option<ITfThreadMgr>,
    client_id: u32,
    thread_mgr_cookie: u32,
    composition: Option<ITfComposition>,
    ending_composition: Rc<Cell<bool>>,
    /// IPC session 與它與 client 的連線：Deactivate 不清掉，跨重新啟用存活。
    session: Session,
    /// 系統相容層的操作（#66）；測試換成假的。呼叫前先 clone，呼叫期間不持有 state 的借用。
    cuas: Rc<dyn Cuas>,
    /// 正在請相容層完成組字（Completing 守衛，#66）：記著那時的組字。相容層同步終止的正是這個組字時，
    /// OnCompositionTerminated 只清外殼狀態、不送取消。
    completing: Option<CompletingState>,
    /// 攔下重排的快捷鍵，等它從訊息佇列回來（#66）。
    pending_repost: Option<PendingRepost>,
    composing: bool,
    /// 目前顯示的快照是否開著符號／表情選單；按鍵路由用。
    menu_open: bool,
    /// 候選窗或選單開著（`Snapshot::navigating` 或 `menu`）。英文 printable
    /// 在這個狀態要走 Key，數字選字、字母選項才進得了 engine。
    navigating: bool,
    attr_atoms: [u32; KINDS.len()],
    eaten: HashSet<u32>,
    right_shift: keys::ShiftTap,
    left_shift: keys::ShiftTap,
    secure: bool,
    ui_less: bool,
    activate_flags: u32,
    /// 候選窗的訊息視窗（0c 的 CandidateWindow）；收 `WM_SELECT_CANDIDATE` 與延後焦點確認。
    callback: Option<candidate::CandidateWindow>,
    /// 全域快捷鍵的 message-only window（Activate 建立、Deactivate 拆除）。
    hotkey: Option<Box<crate::hotkey::HotkeyWindow>>,
    /// 候選窗訊息用戶端，持有 UI-less 宿主的候選清單元件。
    ui: ui::UiClient,
    revision: u32,
    selection: Option<(ITfContext, Vec<i64>)>,
    langbar: Option<(ITfLangBarItemMgr, ITfLangBarItemButton)>,
    shown: Option<Shown>,
    anchor: Option<RECT>,
    recent: Option<(HWND, RECT)>,
    layout_sink: Option<(ITfContext, u32)>,
    notes: caret::Notes,
    openclose: Option<(ITfCompartment, u32)>,
    openclose_value: i32,
    left_from: Option<ITfContext>,
    hotkey_tested: keys::Tested,
    key_tested: keys::Tested,
}

/// 按鍵判斷的 helper 都放在 `State` 上（與 0c 相同）：呼叫端持有單一 `borrow_mut()`
/// 就能連續問 convert_key／styled_key／vocab_keys，不會對同一個 RefCell 重入借用。
impl State {
    /// 保留風格需要的 Shift＋Space 語意；預判和送鍵共用。
    fn styled_key(&mut self, vk: u32, lparam: isize, vocab: bool, english: bool) -> Option<String> {
        let key = keys::to_key(vk, lparam, vocab, english)?;
        let shift = keys::shift_down();
        let shift_space = key == " " && shift && self.session.style_shift_space();
        let styled = keys::style_key(key, shift, shift_space);
        // Caps Lock 只改最後字面的大小寫，不改變路由：送給引擎的字母一律用
        // 基礎小寫，caps 由 Key 旗標帶過去。數字、標點與具名鍵不受影響。
        if keys::caps_lock_on() && styled.len() == 1 && styled.as_bytes()[0].is_ascii_alphabetic() {
            Some(styled.to_ascii_lowercase())
        } else {
            Some(styled)
        }
    }

    fn vocab_keys(&self) -> bool {
        self.session.vocab_keys()
    }

    /// 組字中的 Shift＋可列印鍵：走字面，不讓前面的注音被重新規劃。
    ///
    /// 空白不在此列；中文的 Shift+Space 仍保留原本的全形切換語意。
    fn shift_literal_key(&self, vk: u32, lparam: isize) -> Option<String> {
        if !self.composing
            || self.session.english()
            || !keys::shift_down()
            || keys::modifier_down()
            || keys::is_space_key(vk)
        {
            return None;
        }
        // 標點與符號交回引擎：反引號前導要看到這一鍵（`` ` `` + `Shift+[` → `｛`）。
        keys::shift_literal(&keys::literal(vk, lparam, false)?)
    }

    /// 日文變換鍵（轉換／無轉換、F6–F10）；只有引擎回報能力才送。
    ///
    /// 不再看外殼的日文輸入模式：日文片段由 core 依混打推斷，外殼只問
    /// capability。
    fn convert_key(&mut self, vk: u32) -> Option<&'static str> {
        let key = keys::japanese_named(vk)?;
        (self.session.available() && self.session.convert_keys()).then_some(key)
    }

    fn ctrl_punct_key(&mut self, vk: u32, lparam: isize) -> Option<&'static str> {
        if self.session.english() {
            return None;
        }
        let key = keys::ctrl_punct(vk, lparam)?;
        (self.session.available() && self.session.ctrl_punctuation(key)).then_some(key)
    }

    fn simplified_hotkey(&mut self, vk: u32) -> bool {
        keys::simplified_hotkey(vk) && self.session.available() && self.session.simplified_hotkey()
    }

    fn tap(&mut self, side: keys::Side) -> &mut keys::ShiftTap {
        match side {
            keys::Side::Left => &mut self.left_shift,
            keys::Side::Right => &mut self.right_shift,
        }
    }

    fn shift_toggles(&mut self, side: keys::Side, modified: bool, now: u64) -> bool {
        let mouse = keys::mouse_state();
        let config = self.session.shift_config();
        if !self.tap(side).solo(modified, mouse) {
            return false;
        }
        let enabled = match side {
            keys::Side::Left => config.left,
            keys::Side::Right => config.right,
        };
        enabled && self.tap(side).tap(modified, mouse, now, config.tap_ms)
    }
}

impl TextService {
    /// 建立文字服務物件；還沒有任何 TSF 資源。
    // 文字服務是 COM 物件，由 ClassFactory 建立；Default 會讓「建立物件」看起來像隨手可得。
    #[allow(clippy::new_without_default)]
    pub fn new() -> TextService {
        log::object_created();
        TextService {
            state: RefCell::new(State {
                thread_mgr: None,
                client_id: 0,
                thread_mgr_cookie: TF_INVALID_COOKIE,
                composition: None,
                ending_composition: Rc::new(Cell::new(false)),
                session: new_session(),
                cuas: Rc::new(Win32Cuas),
                completing: None,
                pending_repost: None,
                composing: false,
                menu_open: false,
                navigating: false,
                attr_atoms: [0; KINDS.len()],
                eaten: HashSet::new(),
                right_shift: keys::ShiftTap::default(),
                left_shift: keys::ShiftTap::default(),
                secure: false,
                ui_less: false,
                activate_flags: 0,
                callback: None,
                hotkey: None,
                ui: ui::UiClient::new(),
                revision: 0,
                selection: None,
                langbar: None,
                shown: None,
                anchor: None,
                recent: None,
                layout_sink: None,
                notes: caret::Notes::default(),
                openclose: None,
                openclose_value: 0,
                left_from: None,
                hotkey_tested: keys::Tested::default(),
                key_tested: keys::Tested::default(),
            }),
            applying: Cell::new(0),
            layout_missed: Cell::new(false),
            activated: Cell::new(false),
            needs_reset: Cell::new(false),
        }
    }
}

/// 助手方法放在 `TextService_Impl`：COM 回呼都在這一側，`to_interface()`／`to_object()`
/// 由 windows-core 的 `IUnknownImpl` 提供（T0 的 API 對照：`Ref<'_,T>` → `Ref<T>`）。
#[allow(dead_code)] // 有些方法留給 candidate window 與 host test 接線後才會被呼叫。
impl TextService_Impl {
    fn composition_sink(&self) -> ITfCompositionSink {
        self.to_interface()
    }

    /// 按鍵邊界：panic 時降級成「這個鍵沒處理」（`Ok(FALSE)`），並在降級前重設組字狀態。
    ///
    /// 只有四個按鍵方法用它；其他 COM 方法維持 `crate::guarded` 的 `E_FAIL`。
    fn guarded_key<F>(
        &self,
        what: &str,
        ctx: Option<&ITfContext>,
        f: F,
    ) -> windows_core::Result<BOOL>
    where
        F: FnOnce() -> windows_core::Result<BOOL>,
    {
        self.consume_needs_reset(ctx);
        log::guarded_bool_with(
            what,
            || self.reset_after_panic(ctx),
            || {
                // 熱鍵執行緒可能已經把 chord 排進佇列，但轉送訊息還在等這條
                // UI 執行緒的訊息迴圈；每次按鍵入口先補處理。
                self.drain_hotkeys();
                f()
            },
        )
    }

    /// 上一次 panic 沒能把狀態重設完（`state` 被借用）時，下一次按鍵先補做。
    fn consume_needs_reset(&self, ctx: Option<&ITfContext>) {
        if !self.needs_reset.replace(false) {
            return;
        }
        log::log("panic 後重設：上一次未完成，這次按鍵先重設");
        self.reset_after_panic(ctx);
    }

    /// panic 之後把組字狀態歸零：組字／preedit、pending commit、候選窗、edit session 參照。
    ///
    /// 重設本身不得 panic：`state` 借不到就留下 `needs_reset` 給下一次按鍵；途中出錯只記 log，
    /// 同樣留下旗標再試一次。
    fn reset_after_panic(&self, ctx: Option<&ITfContext>) {
        let reset = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.reset_after_panic_inner(ctx)
        }));
        if reset.is_err() {
            log::log("panic 後重設失敗，下一次按鍵再試");
            self.needs_reset.set(true);
        }
    }

    /// [`Self::reset_after_panic`] 的本體；每一段都用 `try_borrow_mut`，借不到就延後。
    fn reset_after_panic_inner(&self, ctx: Option<&ITfContext>) {
        if self.state.try_borrow_mut().is_err() {
            self.needs_reset.set(true);
            return;
        }
        // 先清掉守衛狀態，讓 finish 走正常路徑收掉文件裡的組字（panic 已經 unwind 出去，
        // 相容層的呼叫不會再回來）。
        if let Ok(mut st) = self.state.try_borrow_mut() {
            st.completing = None;
            st.pending_repost = None;
        }
        // 引擎端取消組字：未送出的 commit 丟掉，組字區與底線一併收掉。
        // 沒有 context 或引擎沒回應時只剩下面幾行的外殼狀態重設。
        self.finish(ctx, "cancel");
        // 候選窗與版面通知（edit session 參照）。
        self.forget_candidates();
        let Ok(mut st) = self.state.try_borrow_mut() else {
            self.needs_reset.set(true);
            return;
        };
        st.composition = None;
        st.composing = false;
        st.menu_open = false;
        st.navigating = false;
        st.left_from = None;
        st.selection = None;
        st.eaten.clear();
        // 讓還在飛的非同步重查作廢。
        st.revision = st.revision.wrapping_add(1).max(1);
        // pending commit 與引擎端組字狀態：關掉 session，保留 IPC client。
        st.session.reset();
        // UI-less 宿主的候選清單。
        st.ui.hide();
        if let Some(window) = st.callback.as_ref() {
            window.cancel_settle_focus();
        }
        log::log("panic 後已重設組字狀態");
    }

    /// 焦點內的最上層 context。
    fn focused_context(&self) -> Option<ITfContext> {
        let tm = self.state.borrow().thread_mgr.clone()?;
        unsafe { tm.GetFocus().ok()?.GetTop().ok() }
    }

    /// 把快照套用到 context：更新組字區／提交。回傳是否吞掉按鍵。
    fn present(&self, ctx: Option<&ITfContext>, snapshot: &Snapshot) -> bool {
        let eaten = self.present_in_edit(ctx, snapshot, None);
        self.sync_langbar();
        if snapshot.bell {
            bell();
        }
        eaten
    }

    /// 輸入指示器按鈕的左鍵：切換中英。
    pub(crate) fn toggle_mode(&self) {
        let snapshot = self.state.borrow_mut().session.toggle();
        if snapshot.ok {
            self.sync_langbar();
        }
    }

    /// 選單的「輸出簡體字」：請引擎即時切換，組字不動。
    pub(crate) fn set_output_simplified(&self, on: bool) -> bool {
        let ctx = self.focused_context();
        let (snapshot, live, available) = {
            let Ok(mut st) = self.state.try_borrow_mut() else {
                zlog!("langbar: 輸出簡體 → {on} 失敗：state 被借用");
                return false;
            };
            let available = st.session.available();
            let live = st.session.simplified().is_some();
            let snapshot = if available {
                st.session.output_simplified(on)
            } else {
                Snapshot::default()
            };
            (snapshot, live, available)
        };
        zlog!(
            "langbar: 輸出簡體 → {on} available={available} simplified_live={live} \
             ok={} handled={}",
            snapshot.ok,
            snapshot.handled
        );
        if !snapshot.ok || !snapshot.handled {
            return false;
        }
        self.present(ctx.as_ref(), &snapshot);
        true
    }

    /// 開選單前重新確認「輸出簡體字」。
    pub(crate) fn recheck_simplified(&self) {
        let changed = match self.state.try_borrow_mut() {
            Ok(mut st) => st.session.refresh_simplified(),
            Err(_) => false,
        };
        if changed {
            self.sync_langbar();
        }
    }

    fn keyboard_openclose_changed(&self) {
        let Ok(mut st) = self.state.try_borrow_mut() else {
            return;
        };
        let Some((compartment, _)) = st.openclose.clone() else {
            return;
        };
        let value = compartment_i32(&compartment).unwrap_or(0);
        let previous = std::mem::replace(&mut st.openclose_value, value);
        drop(st);
        if value == previous {
            return;
        }
        let hotkey = keys::ctrl_space_down();
        zdebug!(
            "鍵盤開關 {} -> {}{}",
            previous,
            value,
            if hotkey { "（Ctrl＋Space）" } else { "" }
        );
        if !hotkey {
            return;
        }
        let ctx = self.focused_context();
        let (secure, client, ui_less) = {
            let st = self.state.borrow();
            (st.secure, st.client_id, st.ui_less)
        };
        if !secure && privacy::hotkey_allowed(ctx.as_ref(), client, ui_less) {
            self.toggle_mode();
        }
    }

    /// 第一個按鍵前確定這個程式的中英模式（只問已在執行的引擎）。
    fn learn_mode(&self, vk: u32) {
        // 英文模式已是外殼確定的狀態：不再問引擎，避免每個按鍵都產生 IPC。
        if self.state.borrow().session.english() {
            return;
        }
        if keys::is_modifier(vk) {
            zlog!("learn_mode vk={}({}) 略過：修飾鍵", vk, vk_label(vk));
            return;
        }
        let (available, mode_known_before) = {
            let mut st = self.state.borrow_mut();
            (st.session.available(), st.session.mode_known())
        };
        zlog!(
            "learn_mode 前 vk={}({}) available={} mode_known={}",
            vk,
            vk_label(vk),
            available,
            mode_known_before
        );
        let changed = available && {
            let mut st = self.state.borrow_mut();
            st.session.learn_mode(true)
        };
        let after = {
            let st = self.state.borrow();
            (st.session.mode_known(), st.session.english())
        };
        zlog!(
            "learn_mode 後 vk={}({}) mode_known={} changed={} english={}",
            vk,
            vk_label(vk),
            after.0,
            changed,
            after.1
        );
        if changed {
            self.sync_langbar();
        }
    }

    fn sync_langbar(&self) {
        let st = self.state.borrow();
        if let Some((_, button)) = st.langbar.as_ref() {
            langbar::set_state(button, st.session.english(), st.session.simplified());
        }
    }

    /// 接 T5 的 `edit::apply`：更新組字區／提交、放置游標、回傳是否吞掉按鍵。
    fn present_in_edit(
        &self,
        ctx: Option<&ITfContext>,
        snapshot: &Snapshot,
        cookie: Option<u32>,
    ) -> bool {
        if !snapshot.ok {
            return false;
        }
        let (composition, client_id, atom, ending, ui_less) = {
            let st = self.state.borrow();
            (
                st.composition.clone(),
                st.client_id,
                st.attr_atoms,
                st.ending_composition.clone(),
                st.ui_less,
            )
        };
        let notice = if ui_less {
            String::new()
        } else {
            snapshot.notice()
        };
        let reading = if ui_less {
            String::new()
        } else {
            snapshot.reading_hint.clone()
        };
        // 選單開著時一律顯示，不受 `navigating` 影響；其他情形沿用「組字中且
        // 使用者已進入選取」的規則。每次按鍵都彈一個框會蓋住上下文。
        let show = snapshot.wants_candidate_window()
            && (!snapshot.candidates.is_empty()
                || !snapshot.suggestion.is_empty()
                || !reading.is_empty())
            || !notice.is_empty();
        let applied = match ctx {
            Some(ctx) => {
                let _applying = Applying::enter(&self.applying, &self.layout_missed);
                edit::apply(
                    ctx,
                    client_id,
                    composition,
                    self.composition_sink(),
                    snapshot,
                    atom,
                    cookie,
                    ending,
                    show,
                )
            }
            None => {
                if !snapshot.commit.is_empty() {
                    zlog!("沒有 context 可提交，提交文字遺失");
                }
                edit::Applied {
                    composition: None,
                    caret: Err(edit::Miss::NoContext),
                }
            }
        };
        let view = ctx.and_then(|c| unsafe { c.GetActiveView() }.ok());
        let owner = view.as_ref().and_then(|v| unsafe { v.GetWnd() }.ok());
        if let Ok(rect) = applied.caret {
            let mut st = self.state.borrow_mut();
            st.anchor = Some(rect);
            if let Some(h) = owner.filter(|h| !h.is_invalid()) {
                st.recent = Some((h, rect));
            }
        }
        zdebug!(
            "located {:?}",
            applied.caret.map(|r| (r.left, r.top, r.right, r.bottom))
        );
        let placed = if show {
            self.place(&applied.caret, owner)
        } else {
            None
        };
        // 排查用：引擎已進入選取（navigating）卻沒有顯示，把三個條件印出來。
        // 只在這個異常組合寫一行，正常打字不會洗版。
        if snapshot.navigating && !show {
            zlog!(
                "present: navigating 但 show=false composing={} cands={} suggestion={} reading={}",
                snapshot.composing(),
                snapshot.candidates.len(),
                snapshot.suggestion.is_empty(),
                reading.is_empty()
            );
        }
        let (target, revision) = {
            let mut st = self.state.borrow_mut();
            st.revision = st.revision.wrapping_add(1).max(1);
            st.selection = if show {
                ctx.map(|c| (c.clone(), snapshot.candidates.iter().map(|c| c.0).collect()))
            } else {
                None
            };
            (
                st.callback
                    .as_ref()
                    .map(|window| window.handle())
                    .unwrap_or(0),
                st.revision,
            )
        };
        let caret = placed.map(|(rect, _)| rect);
        // 一頁最多十列；橫向四欄時一頁四十項。反白所在的頁就是顯示頁，
        // 外殼不再自行猜欄數或列數。
        let page = candidate_layout::page_bounds(
            snapshot.candidates.len(),
            snapshot.columns,
            snapshot.rows,
            snapshot.highlight,
        );
        let page_start = page.start;
        let visible: Vec<UiCandidate> = snapshot.candidates[page.clone()]
            .iter()
            .enumerate()
            .map(|(offset, (id, text))| {
                let index = page_start + offset;
                UiCandidate {
                    id: *id,
                    text: text.clone(),
                    note: snapshot
                        .candidate_notes
                        .get(index)
                        .cloned()
                        .unwrap_or_default(),
                    key: Some(
                        snapshot
                            .candidate_keys
                            .get(index)
                            .cloned()
                            .unwrap_or_default(),
                    ),
                }
            })
            .collect();
        let page_size = (snapshot.rows * snapshot.columns).max(1);
        let pages = snapshot.candidates.len().div_ceil(page_size);
        let menu = snapshot.menu.as_ref();
        let message = UiMessage {
            show: show && caret.is_some(),
            x: caret.map(|rect| rect.left).unwrap_or(0),
            y: caret.map(|rect| rect.top).unwrap_or(0),
            w: caret.map(|rect| rect.right - rect.left).unwrap_or(0),
            h: caret.map(|rect| rect.bottom - rect.top).unwrap_or(0),
            candidates: visible,
            highlight: snapshot.highlight - page_start as i64,
            page: (page_start / page_size + 1) as i64,
            pages: pages as i64,
            columns: snapshot.columns,
            kind: menu.map(|menu| menu.kind.clone()).unwrap_or_default(),
            title: snapshot.candidate_title(),
            menu_kind: menu.map(|menu| menu.kind.clone()).unwrap_or_default(),
            menu_title: menu.map(|menu| menu.title.clone()).unwrap_or_default(),
            menu_tabs: menu.map(|menu| menu.tabs.clone()).unwrap_or_default(),
            menu_tab: menu.map(|menu| menu.tab).unwrap_or(0),
            suggestion: snapshot.suggestion.clone(),
            notice,
            marking: snapshot.marking(),
            reading,
            owner: std::process::id(),
            target,
            revision,
        };
        let document = ctx.and_then(|c| unsafe { c.GetDocumentMgr() }.ok());
        {
            let mut st = self.state.borrow_mut();
            // 核心已經是快照的狀態；文件端沒套用成功時保留原組字物件，下一鍵再同步。
            st.composing = snapshot.composing();
            st.menu_open = snapshot.menu.is_some();
            // 候選窗（navigating）與選單都要讓英文 printable 走 Key；兩者
            // 都可能在 composing 之外被打開，不能只看 menu_open。
            st.navigating = snapshot.navigating || snapshot.menu.is_some();
            st.composition = applied.composition;
            st.ui.present(&message, owner, document.clone());
            if !st.composing {
                st.anchor = None;
                st.left_from = None;
            }
            st.shown = ctx.filter(|_| show).map(|ctx| Shown {
                ctx: ctx.clone(),
                cursor: snapshot.cursor_utf16,
                owner,
                document,
                exact: placed.is_some_and(|(_, source)| source == caret::Source::Exact),
                requeries: 0,
                since: Instant::now(),
                message,
            });
        }
        self.watch_layout(if show { ctx } else { None });
        let exact = placed.is_some_and(|(_, source)| source == caret::Source::Exact);
        if self.layout_missed.take() && show && !exact && self.applying.get() == 0 {
            self.requery(ctx);
        }
        snapshot.handled
    }

    /// 候選窗的位置：`GetTextExt` 的結果，拿不到就退到系統插入點、同一次組字上一次的位置、
    /// 這個視窗上一次的位置（0c `caret.rs` 的退化策略）。
    fn place(
        &self,
        located: &edit::Located,
        view_window: Option<HWND>,
    ) -> Option<(RECT, caret::Source)> {
        let (previous, recent) = {
            let st = self.state.borrow();
            let recent = st
                .recent
                .filter(|(h, _)| Some(*h) == view_window)
                .map(|(_, r)| r);
            (st.anchor, recent)
        };
        let placed = caret::choose(*located, caret::system_caret, previous, || {
            caret::around_field(recent)
        });
        let st = self.state.borrow();
        if let Err(miss) = located {
            let to = placed.map(|(_, source)| source.name()).unwrap_or("none");
            st.notes.once(&format!("{}→{}", miss.class(), to), || {
                Some(format!(
                    "{}，改用 {}（view {:#x}）",
                    miss,
                    to,
                    view_window.map(|h| h.0 as usize).unwrap_or(0)
                ))
            });
        }
        placed
    }

    /// 在有候選窗的 context 上註冊 ITfTextLayoutSink；換 context 或沒有候選時取消。
    fn watch_layout(&self, ctx: Option<&ITfContext>) {
        let current = self.state.borrow_mut().layout_sink.take();
        if let Some((watched, cookie)) = current {
            if Some(&watched) == ctx {
                self.state.borrow_mut().layout_sink = Some((watched, cookie));
                return;
            }
            if let Ok(source) = watched.cast::<ITfSource>() {
                let _ = unsafe { source.UnadviseSink(cookie) };
            }
        }
        let Some(ctx) = ctx else { return };
        let sink: ITfTextLayoutSink = self.to_interface();
        match ctx
            .cast::<ITfSource>()
            .and_then(|source| unsafe { source.AdviseSink(&ITfTextLayoutSink::IID, &sink) })
        {
            Ok(cookie) => self.state.borrow_mut().layout_sink = Some((ctx.clone(), cookie)),
            Err(e) => self
                .state
                .borrow_mut()
                .notes
                .once(&format!("advise {:#x}", e.code().0), || {
                    Some(format!("無法註冊 OnLayoutChange：{}", e.code()))
                }),
        }
    }

    /// 收起候選窗並停止收版面通知。
    fn forget_candidates(&self) {
        {
            let mut st = self.state.borrow_mut();
            st.shown = None;
            st.anchor = None;
            // 失焦、Escape、提交都走這裡：候選窗要跟著收，不能留在畫面上。
            st.ui.hide();
        }
        self.watch_layout(None);
    }

    /// 接 T5 的 `edit::request_locate`：版面變了，另開唯讀 edit session 重查游標。
    fn requery(&self, ctx: Option<&ITfContext>) {
        let request = {
            let Ok(mut st) = self.state.try_borrow_mut() else {
                return;
            };
            if st.ui_less {
                return;
            }
            if !st
                .shown
                .as_ref()
                .is_some_and(|shown| ctx == Some(&shown.ctx) && shown.requeries < MAX_REQUERIES)
            {
                return;
            }
            let (client, composition, revision) =
                (st.client_id, st.composition.clone(), st.revision);
            let shown = st.shown.as_mut().expect("shown 已確認存在");
            shown.requeries += 1;
            (
                shown.ctx.clone(),
                client,
                composition,
                shown.cursor,
                revision,
            )
        };
        let (ctx, client, composition, cursor, revision) = request;
        let service = self.to_object();
        let requested = edit::request_locate(
            &ctx,
            client,
            composition,
            cursor,
            Box::new(move |located| service.relocated(revision, located)),
        );
        if let Err(e) = requested {
            self.state
                .borrow_mut()
                .notes
                .once(&format!("requery {:#x}", e.code().0), || {
                    Some(format!("OnLayoutChange 後無法重查位置：{}", e.code()))
                });
        }
    }

    fn relocated(&self, revision: u32, located: edit::Located) {
        if self.applying.get() > 0 {
            self.layout_missed.set(true);
            return;
        }
        let Ok(mut st) = self.state.try_borrow_mut() else {
            return;
        };
        let Ok(rect) = located else { return };
        if st.revision != revision {
            return;
        }
        let Some(shown) = st.shown.as_mut() else {
            return;
        };
        if !drop_expired_notice(&mut shown.message, shown.since.elapsed()) {
            st.shown = None;
            return;
        }
        let moved = (
            shown.message.x,
            shown.message.y,
            shown.message.w,
            shown.message.h,
        ) != (
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
        );
        if shown.exact && shown.message.show && !moved {
            return;
        }
        let first = !shown.exact;
        shown.exact = true;
        shown.message.show = true;
        shown.message.x = rect.left;
        shown.message.y = rect.top;
        shown.message.w = rect.right - rect.left;
        shown.message.h = rect.bottom - rect.top;
        let (message, owner, document) =
            (shown.message.clone(), shown.owner, shown.document.clone());
        if st.composing {
            st.anchor = Some(rect);
        }
        if let Some(handle) = owner.filter(|handle| !handle.is_invalid()) {
            st.recent = Some((handle, rect));
        }
        st.ui.present(&message, owner, document);
        if first {
            st.notes.once("relocated", || {
                Some("版面更新（OnLayoutChange）後取得游標位置".into())
            });
        }
    }

    /// 候選被滑鼠點到時將選擇送給引擎（選字走 engine，不是拿 candidate_keys 攔數字）。
    pub(crate) fn select_candidate(&self, revision: u32, id: i64) {
        let target = {
            let st = self.state.borrow();
            st.selection
                .as_ref()
                .filter(|(_, ids)| revision == st.revision && ids.contains(&id))
                .map(|(ctx, _)| (ctx.clone(), st.client_id))
        };
        if let Some((ctx, client)) = target {
            candidate::request(self, &ctx, client, revision, id);
        }
    }

    /// 候選窗的 edit session 回呼：在點選當下再核對一次焦點、隱私與畫面版本，才送核心。
    pub(crate) fn select_in_edit(&self, ec: u32, revision: u32, id: i64) {
        let (ctx, ui_less) = {
            let st = self.state.borrow();
            let Some((ctx, ids)) = st.selection.as_ref() else {
                return;
            };
            if st.secure || revision != st.revision || !ids.contains(&id) {
                return;
            }
            (ctx.clone(), st.ui_less)
        };
        if self.focused_context().as_ref() != Some(&ctx)
            || !candidate::foreground_belongs_to(std::process::id())
            || privacy::blocked_in_edit(&ctx, ec, ui_less)
        {
            return;
        }
        let snapshot = self
            .state
            .borrow_mut()
            .session
            .handle(json!({"type": "select", "index": id}));
        self.present_in_edit(Some(&ctx), &snapshot, Some(ec));
    }

    /// 硬要求一：直接把按鍵送給核心，絕對不補 Down。
    ///
    /// 0c `service.rs:942-965` 的 `handle_key()` 只做 `session.key(key)` 再 `present`。
    /// 舊 `jc-tsf/src/composition.rs` 的 `needs_selection_prefix()` 會在「單一 1-9 且
    /// candidates > 0」時插入 `KeyCode::Down`，讓注音前綴的數字變成選字鍵；核心 su
    /// 修復後 `su` + `3` 應是聲調鍵，補 Down 會讓它又被 suggestion 取代。
    fn handle_key(&self, ctx: Option<&ITfContext>, key: &str) -> bool {
        let (english, composing) = {
            let st = self.state.borrow();
            (st.session.english(), st.composing)
        };
        // 英文模式且沒有組字才是純 raw；組字中的具名鍵（Enter、Backspace、
        // 方向鍵）仍走原本的 Key 路徑，列印字元由 handle_literal 走 Literal。
        if english && !composing {
            zlog!("handle_key 略過：英文模式沒有組字，不送引擎 key={}", key);
            return false;
        }
        zlog!("handle_key 送引擎 key={}", key);
        let caps = keys::caps_lock_on();
        let shift = keys::shift_down();
        let snapshot = self
            .state
            .borrow_mut()
            .session
            .key_with_flags(key, caps, shift);
        zlog!(
            "handle_key 引擎回 key={} ok={} handled={} composing={} commit_len={} preedit={}",
            key,
            snapshot.ok,
            snapshot.handled,
            snapshot.composing(),
            snapshot.commit.chars().count(),
            snapshot.text()
        );
        zdebug!(
            "snapshot ok={} handled={} composing={} commit_len={}",
            snapshot.ok,
            snapshot.handled,
            snapshot.composing(),
            snapshot.commit.chars().count()
        );
        let eaten = self.present(ctx, &snapshot);
        self.schedule_ja_live_idle(&snapshot);
        eaten
    }

    /// 把命中 bindings 的修飾鍵組合送給引擎；引擎回 handled=false 時放行。
    fn handle_chord(&self, ctx: Option<&ITfContext>, key: &jc_proto::Key) -> bool {
        zlog!("handle_chord 送引擎 key={:?}", key);
        let snapshot = self.state.borrow_mut().session.key_chord(key);
        zlog!(
            "handle_chord 引擎回 ok={} handled={} composing={} preedit={}",
            snapshot.ok,
            snapshot.handled,
            snapshot.composing(),
            snapshot.text()
        );
        // 記下 snapshot 的形狀，實機才能分辨「chord 沒送到」與「送到但沒有
        // 選單狀態」。menu 有值時候選窗由 menu 顯示，不再依賴 navigating。
        zlog!(
            "handle_chord snapshot menu={} navigating={} candidates={} highlight={} title={:?}",
            snapshot.menu.is_some(),
            snapshot.navigating,
            snapshot.candidates.len(),
            snapshot.highlight,
            snapshot.candidate_title()
        );
        if snapshot.menu.is_none()
            && !snapshot.navigating
            && snapshot.composing()
            && !snapshot.candidates.is_empty()
        {
            zlog!("handle_chord：引擎回了候選但 menu=none 且 navigating=false；候選窗不會顯示");
        }
        let eaten = self.present(ctx, &snapshot);
        self.schedule_ja_live_idle(&snapshot);
        eaten
    }

    /// 建立熱鍵執行緒與轉送視窗（已存在就不做事）；失敗只留 key sink 路徑。
    fn create_hotkey_window(&self) -> bool {
        if self.state.borrow().hotkey.is_some() {
            return true;
        }
        match crate::hotkey::HotkeyWindow::new(self) {
            Ok(window) => {
                self.state.borrow_mut().hotkey = Some(window);
                true
            }
            Err(error) => {
                log::log(&format!(
                    "hotkey：建立熱鍵執行緒失敗（{error}），保留 key sink 路徑"
                ));
                false
            }
        }
    }

    /// 這個服務取得前景：建立（必要時）並啟用全域快捷鍵視窗。
    fn enable_hotkeys(&self) {
        if !self.activated.get() {
            return;
        }
        if !self.create_hotkey_window() {
            return;
        }
        if let Some(window) = self.state.borrow().hotkey.as_ref() {
            window.enable();
        }
    }

    /// 失去前景：先反註冊，避免背景行程霸占其他程式的全域快捷鍵。
    fn release_hotkeys(&self) {
        if let Some(window) = self.state.borrow().hotkey.as_ref() {
            window.disable();
        }
    }

    /// 補處理熱鍵執行緒排進佇列的 chord。
    ///
    /// 正常路徑是轉送視窗收到 `WM_FORWARD_HOTKEY` 就呼叫 [`Self::hotkey_pressed`]；
    /// 宿主沒有在 UI 執行緒跑訊息迴圈時，轉送訊息不會派送，這裡在每次按鍵與
    /// 焦點入口補上，讓熱鍵至少在下一個 TSF 事件生效。
    fn drain_hotkeys(&self) {
        let pending = match self.state.try_borrow() {
            Ok(st) => st
                .hotkey
                .as_ref()
                .map(|window| window.take_pending())
                .unwrap_or_default(),
            Err(_) => return,
        };
        for hotkey in pending {
            self.hotkey_pressed(&hotkey);
        }
    }

    /// `WM_HOTKEY`：把已註冊的 chord 當成一次按鍵送給 engine。
    ///
    /// 熱鍵由系統直接送進訊息佇列，不會經過 key sink；隱私與安全模式的判斷
    /// 要在這裡補上。欄位被擋下時組合鍵已經被系統吃掉，只記日誌不送 engine。
    pub(crate) fn hotkey_pressed(&self, hotkey: &crate::hotkey::Hotkey) {
        // 宿主可能在按鍵處理中幫我們跑訊息迴圈；狀態正被借用時不要重入。
        if self.state.try_borrow_mut().is_err() {
            log::log(&format!("hotkey：狀態忙碌，忽略 {}", hotkey.chord));
            return;
        }
        // 熱鍵由系統送進背景行程的訊息佇列；前景不是自己時不能對舊文件開選單。
        if !candidate::foreground_belongs_to(std::process::id()) {
            log::log(&format!("hotkey：前景不屬於本行程，忽略 {}", hotkey.chord));
            return;
        }
        zlog!(
            "hotkey 觸發 {} {} vk={:#04x}",
            hotkey.action,
            hotkey.chord,
            hotkey.vk
        );
        let (secure, client, ui_less) = {
            let st = self.state.borrow();
            (st.secure, st.client_id, st.ui_less)
        };
        if secure {
            log::log(&format!("hotkey：安全模式不處理 {}", hotkey.chord));
            return;
        }
        let Some(ctx) = self.focused_context() else {
            log::log(&format!("hotkey：沒有焦點文件，忽略 {}", hotkey.chord));
            return;
        };
        if !privacy::hotkey_allowed(Some(&ctx), client, ui_less) {
            log::log(&format!("hotkey：目前欄位阻擋 {}", hotkey.chord));
            return;
        }
        let key = hotkey.to_key();
        let eaten = self.handle_chord(Some(&ctx), &key);
        zlog!("hotkey 完成 {} eaten={eaten}", hotkey.chord);
    }

    /// 把字面文字送進引擎的 `literal` op，不重新規劃前面的注音。
    fn handle_literal(&self, ctx: Option<&ITfContext>, text: &str) -> bool {
        let (english, composing) = {
            let st = self.state.borrow();
            (st.session.english(), st.composing)
        };
        if english && !composing {
            zlog!(
                "handle_literal 略過：英文模式沒有組字，不送引擎 literal={:?}",
                text
            );
            return false;
        }
        zlog!("handle_literal 送引擎 literal={:?}", text);
        let snapshot = self.state.borrow_mut().session.literal(text);
        zlog!(
            "handle_literal 引擎回 ok={} handled={} composing={} preedit={}",
            snapshot.ok,
            snapshot.handled,
            snapshot.composing(),
            snapshot.text()
        );
        let eaten = self.present(ctx, &snapshot);
        self.schedule_ja_live_idle(&snapshot);
        if !eaten && english {
            // 英文 printable 只有 Literal 一條路徑：引擎沒回應或回 handled=false
            // 也吃掉這一鍵，不能掉到 handle_key 變成注音鍵，也不能讓宿主再打一次。
            let reason = if snapshot.ok {
                "引擎回 handled=false"
            } else {
                "引擎沒有回應"
            };
            zlog!(
                "handle_literal 失敗：literal={:?} reason={} -> 吃掉這一鍵，不回落 handle_key",
                text,
                reason
            );
            return true;
        }
        eaten
    }

    /// 按鍵處理完後重排ライブ変換的停頓計時器。
    ///
    /// 只在回應仍是組字中時排；同一個 timer id 重設期限，所以連續輸入不會
    /// 每字都多一次 IPC。候選窗開著（`navigating`／`shown`）或組字結束就取消。
    /// 300ms 沿用 `jc_core::ja::live::Config::idle_delay` 的預設值。
    fn schedule_ja_live_idle(&self, snapshot: &Snapshot) {
        let st = self.state.borrow();
        let Some(window) = st.callback.as_ref() else {
            return;
        };
        if snapshot.ok
            && snapshot.composing()
            && !snapshot.navigating
            && snapshot.menu.is_none()
            && st.shown.is_none()
        {
            window.ja_live_idle_later();
        } else {
            window.cancel_ja_live_idle();
        }
    }

    /// 取消排定的ライブ変換停頓；計時器在候選窗的訊息視窗上。
    fn cancel_ja_live_idle(&self) {
        if let Ok(st) = self.state.try_borrow() {
            if let Some(window) = st.callback.as_ref() {
                window.cancel_ja_live_idle();
            }
        }
    }

    /// 停頓計時器到期：送 `idle` 並用與按鍵相同的 `present` 套用結果。
    ///
    /// 候選窗開著、已經離開組字或沒有焦點時不送。連不上時 `Session::idle`
    /// 直接回空快照，不重試；下一次按鍵會重連。
    pub(crate) fn ja_live_idle(&self) {
        let (composing, shown, english) = {
            let st = self.state.borrow();
            (st.composing, st.shown.is_some(), st.session.english())
        };
        if !composing || shown || english {
            zdebug!(
                "ja idle 略過 composing={} shown={} english={}",
                composing,
                shown,
                english
            );
            return;
        }
        let Some(ctx) = self
            .composition_context()
            .or_else(|| self.focused_context())
        else {
            zdebug!("ja idle 略過：沒有組字 context");
            return;
        };
        let snapshot = match self.state.try_borrow_mut() {
            Ok(mut st) => st.session.idle(),
            Err(_) => return,
        };
        zlog!(
            "ja idle 引擎回 ok={} composing={} preedit={}",
            snapshot.ok,
            snapshot.composing(),
            snapshot.text()
        );
        self.present(Some(&ctx), &snapshot);
    }

    /// 結束目前組字：commit（失焦、停用）或 cancel（應用程式自行終止組字）。
    fn finish(&self, ctx: Option<&ITfContext>, kind: &str) {
        // 組字收尾後不再補送ライブ変換。
        self.cancel_ja_live_idle();
        let (composing, completing) = {
            let st = self.state.borrow();
            (st.composing, st.completing.is_some())
        };
        if !composing {
            return;
        }
        if completing {
            // 正在請相容層完成組字（#66）：焦點變化不能在這時送出或改文件，組字由 complete_via_cuas 收尾。
            zdebug!("相容層完成組字中，略過 {}", kind);
            return;
        }
        let snapshot = self
            .state
            .borrow_mut()
            .session
            .handle(json!({"type": kind}));
        self.present(ctx, &snapshot);
    }

    /// IMM32 程式裡把組字經系統相容層送出（#66）：引擎 commit（學習在這裡）→ 組字區換成送出文字、
    /// 不結束 → 進入 Completing 守衛 → `Cuas::complete`（可能同步回呼外殼，呼叫期間不得持有 state 借用）
    /// → 離開守衛 → 還在組字（相容層沒終止）就自己照快照結束。引擎沒回應退回一般送出。
    fn complete_via_cuas(&self, ctx: Option<&ITfContext>, hwnd: HWND) -> Completed {
        let snapshot = self
            .state
            .borrow_mut()
            .session
            .handle(json!({"type": "commit"}));
        if !snapshot.ok || snapshot.commit.is_empty() {
            // 引擎沒回應，或沒有東西可送：引擎已經收過 commit，不再重送一次，只照快照收尾組字區。
            if self.state.borrow().composing {
                self.present(ctx, &snapshot);
            }
            return Completed::BySelf;
        }
        let mut shown = snapshot.clone();
        shown.preedit = vec![crate::session::Segment {
            text: snapshot.commit.clone(),
            style: String::new(),
        }];
        shown.commit.clear();
        shown.cursor_utf16 = snapshot.commit.encode_utf16().count();
        shown.candidates.clear();
        shown.candidate_notes.clear();
        shown.candidate_keys.clear();
        shown.suggestion.clear();
        shown.reading_hint.clear();
        shown.notices.clear();
        self.present(ctx, &shown);
        let (cuas, composition) = {
            let st = self.state.borrow();
            (st.cuas.clone(), st.composition.clone())
        };
        let (result, terminated) = {
            let completing = Completing::enter(&self.state, composition);
            let result = cuas.complete(hwnd);
            (result, completing.terminated())
        };
        zdebug!(
            "經相容層完成組字 view={:#x} focus={:#x} 結果={:?}",
            hwnd.0 as usize,
            cuas.focus().0 as usize,
            result
        );
        if !terminated {
            if self.state.borrow().composing {
                self.present(ctx, &snapshot);
            }
            return Completed::BySelf;
        }
        Completed::ByCuas
    }

    /// 組字一律經相容層送出；條件都成立才把這一鍵重排給程式，回傳有沒有攔下（#66）。
    fn complete_and_repost(&self, ctx: Option<&ITfContext>, vk: u32, lparam: isize) -> bool {
        let (ctrl, alt, win) = keys::ctrl_alt_win();
        let allowed = keys::reposts_after_commit(vk, ctrl, alt, win);
        self.complete_and_repost_if(ctx, vk, lparam, allowed)
    }

    fn complete_and_repost_if(
        &self,
        ctx: Option<&ITfContext>,
        vk: u32,
        lparam: isize,
        allowed: bool,
    ) -> bool {
        let cuas = self.state.borrow().cuas.clone();
        let eligible = allowed && !cuas.ime_hotkey(vk);
        let view = Self::view_window(ctx);
        let target = cuas.focus();
        let queue_had_no_char = eligible && !target.0.is_null() && !cuas.has_char(target);
        let completed = self.complete_via_cuas(ctx, view);
        let by_cuas = completed == Completed::ByCuas;
        let focus_same = (eligible && queue_had_no_char && target == view && by_cuas)
            .then(|| cuas.focus() == target);
        let has_char = (focus_same == Some(true)).then(|| cuas.has_char(target));
        let repost = has_char == Some(true) && cuas.repost(target, vk, lparam);
        if eligible && !repost {
            zdebug!(
                "不重排 vk={} queue_had_no_char={} target==view:{} ByCuas:{} 焦點未變:{:?} has_char:{:?}",
                vk,
                queue_had_no_char,
                target == view,
                by_cuas,
                focus_same,
                has_char
            );
        }
        if repost {
            zdebug!("重排快捷鍵 vk={} 到 {:#x}", vk, target.0 as usize);
            self.state.borrow_mut().pending_repost = Some(PendingRepost {
                vk,
                lparam,
                key_down_seen: false,
            });
        }
        repost
    }

    fn test_simplified_hotkey(&self, vk: u32) -> bool {
        if self.state.borrow().session.english() {
            return false;
        }
        let decision = self.state.borrow_mut().simplified_hotkey(vk);
        self.state.borrow_mut().hotkey_tested.record(vk, decision);
        decision
    }

    fn key_simplified_hotkey(&self, vk: u32, tested: Option<bool>) -> bool {
        tested.unwrap_or_else(|| self.state.borrow_mut().simplified_hotkey(vk))
    }

    fn reset_shift(&self) {
        let mut st = self.state.borrow_mut();
        st.right_shift = keys::ShiftTap::default();
        st.left_shift = keys::ShiftTap::default();
    }

    /// 每次按鍵重新確認欄位；阻擋路徑不記錄鍵碼、不連線／重播引擎。
    /// `callback`／`vk` 只供 probe 失敗的診斷 log 使用。
    fn allowed(&self, ctx: Option<&ITfContext>, callback: &'static str, vk: u32) -> bool {
        let (secure, client, ui_less, flags) = {
            let st = self.state.borrow();
            (st.secure, st.client_id, st.ui_less, st.activate_flags)
        };
        let field = if secure {
            privacy::Field::Blocked
        } else {
            let field = privacy::field_at(
                ctx,
                client,
                ui_less,
                &privacy::ProbeContext {
                    callback,
                    vk: Some(vk),
                    activate_flags: flags,
                },
            );
            privacy::note_context(ctx, flags, field);
            field
        };
        zlog!(
            "allowed vk={}({}) callback={} secure={} field={:?} ctx={} -> {}",
            vk,
            vk_label(vk),
            callback,
            secure,
            field,
            ctx.is_some(),
            field != privacy::Field::Blocked
        );
        if field != privacy::Field::Blocked {
            self.state
                .borrow_mut()
                .session
                .set_private(field == privacy::Field::Private);
            return true;
        }
        let composition = self.state.borrow().composition.clone();
        if let Some(comp) = composition {
            let original = unsafe { comp.GetRange().and_then(|r| r.GetContext()).ok() };
            self.present(
                original.as_ref(),
                &Snapshot {
                    ok: true,
                    ..Default::default()
                },
            );
        }
        self.forget_candidates();
        let mut st = self.state.borrow_mut();
        st.right_shift = keys::ShiftTap::default();
        st.left_shift = keys::ShiftTap::default();
        st.selection = None;
        st.eaten.clear();
        st.composing = false;
        st.menu_open = false;
        st.navigating = false;
        st.composition = None;
        st.left_from = None;
        false
    }

    /// 英文模式這一鍵只做 IMM32 raw passthrough。
    ///
    /// 已註冊的 chord 在呼叫端先處理，會送一個 `Request::Key`；其餘文字輸入
    /// 不送 engine、不建組字、沒有補全或候選窗，也不處理全形空白。一般
    /// printable、Tab、方向鍵與 Backspace 都回 `None`，由 [`Self::wants_key`]
    /// 直接交還宿主；Caps Lock 交給系統與應用程式。組字中的英文回 `None`，
    /// 由 `OnKeyDown` 改走 engine 的 Literal。
    fn english_key_policy(
        &self,
        ctx: Option<&ITfContext>,
        vk: u32,
        lparam: isize,
    ) -> Option<EnglishKey> {
        let result = (|| -> Option<EnglishKey> {
            if keys::is_modifier(vk) && !keys::modifier_down() {
                zlog!(
                    "english_key_policy vk={}({}) 提前回 None：修飾鍵",
                    vk,
                    vk_label(vk)
                );
                return None;
            }
            if !self.state.borrow().session.english() {
                return None;
            }
            // 組字中的英文由 OnKeyDown 走 engine Literal，不交給宿主，也不走
            // IMM32 的直接字面路徑。
            if self.state.borrow().composing {
                return None;
            }
            // Ctrl／Alt／Win 組合鍵直接放行給宿主。
            if keys::modifier_down() {
                return Some(EnglishKey::Pass);
            }
            // IMM32 例外：沒有插入點的相容層程式無法組字，字面直接交給程式
            // （IMM32 例外，保留原路徑，需實機確認）。
            let flags = self.state.borrow().activate_flags;
            if ctx.is_some_and(|c| privacy::english_passthrough(c, flags)) {
                if let Some(text) = keys::literal(vk, lparam, keys::caps_lock_on()) {
                    return Some(EnglishKey::Literal(text));
                }
                log::once("英文模式：IMM32 程式的焦點沒有插入點，按鍵直接交給程式");
                return Some(EnglishKey::Pass);
            }
            None
        })();
        zlog!(
            "english_key_policy vk={}({}) result={:?} eats={}",
            vk,
            vk_label(vk),
            result,
            result.as_ref().is_some_and(|r| r.eats())
        );
        result
    }

    /// 組字中交給宿主的按鍵先送出組字（#106）。
    ///
    /// 回傳 true＝IMM32 程式裡已經把這一鍵攔下、重排給程式（#66）。`may_repost` 只有
    /// OnTestKeyDown 傳 true：OnKeyDown 只送出、不重排，否則原本的鍵會執行兩次。
    fn commit_before_host_key(
        &self,
        ctx: Option<&ITfContext>,
        vk: u32,
        lparam: isize,
        may_repost: bool,
    ) -> bool {
        if !self.state.borrow().composing || self.state.borrow().session.english() {
            return false;
        }
        if self.state.borrow_mut().convert_key(vk).is_some() {
            return false;
        }
        if keys::modifier_down() && self.state.borrow_mut().ctrl_punct_key(vk, lparam).is_some() {
            return false;
        }
        let protocol = keys::to_key(vk, lparam, false, true);
        if keys::commits_before_host(vk, keys::modifier_down(), protocol.as_deref()) {
            let flags = self.state.borrow().activate_flags;
            if ctx.is_some_and(|c| privacy::imm32_context(c, flags)) {
                zdebug!("IMM32 程式：經相容層送出組字");
                if !may_repost {
                    self.complete_via_cuas(ctx, Self::view_window(ctx));
                    return false;
                }
                return self.complete_and_repost(ctx, vk, lparam);
            }
            self.finish(ctx, "commit");
        }
        false
    }

    fn observe_shift(&self, vk: u32, lparam: isize) {
        let repeat = lparam & (1 << 30) != 0;
        let modified = keys::modifier_down();
        let now = keys::now_ms();
        let (right, left) = (keys::right_shift(vk, lparam), keys::left_shift(vk, lparam));
        let mouse = keys::mouse_state();
        let mut st = self.state.borrow_mut();
        st.right_shift.observe(right, repeat, modified, now, mouse);
        st.left_shift.observe(left, repeat, modified, now, mouse);
    }

    fn shift_released(&self, side: keys::Side) -> bool {
        let toggle = {
            let mut st = self.state.borrow_mut();
            let toggle = st.shift_toggles(side, keys::modifier_down(), keys::now_ms());
            *st.tap(side) = keys::ShiftTap::default();
            toggle
        };
        if toggle {
            let snapshot = self.state.borrow_mut().session.toggle();
            if snapshot.ok {
                self.sync_langbar();
            }
        }
        toggle
    }

    /// 這一鍵是否命中目前綁定；先做最多 5 秒一次的 bindings 更新。
    ///
    /// 只回傳 chord，不在這裡送按鍵；送給引擎是 `OnKeyDown` 的事。
    fn bound_chord(&self, vk: u32, lparam: isize) -> Option<String> {
        // 修飾鍵本身沒有 chord。先擋掉再問 bindings：Shift／Ctrl 的按下與放開
        // 不能觸發每 5 秒一次的 health，否則引擎慢一次就把整個 session 判成掛掉。
        if keys::is_modifier(vk) {
            return None;
        }
        self.state.borrow_mut().session.maybe_refresh_bindings();
        let chord = keys::chord(vk, lparam)?;
        let binding = self
            .state
            .borrow()
            .session
            .binding_for(&chord)
            .map(str::to_string);
        zlog!(
            "bound_chord vk={}({}) chord={} binding={:?}",
            vk,
            vk_label(vk),
            chord,
            binding
        );
        binding.map(|_| chord)
    }

    fn wants_key(&self, vk: u32, lparam: isize) -> bool {
        let available_seen: Cell<Option<bool>> = Cell::new(None);
        let wants = (|| -> bool {
            // 全域 chord 不是文字輸入：先判斷已註冊的快捷鍵，英文模式也一樣。
            if self.bound_chord(vk, lparam).is_some() {
                return true;
            }
            // 英文模式沒有組字時是純 raw：一般按鍵一律交還宿主。chord 已在
            // 上面先判斷；這裡不能呼叫 available／idle_key 等會產生 IPC 的方法。
            // 有組字時 printable 要走 Literal，不能在這裡一起擋掉。
            {
                let st = self.state.borrow();
                if english_route(st.session.english(), st.composing, false, st.navigating)
                    == EnglishRoute::Host
                {
                    return false;
                }
            }
            if keys::modifier_down() && self.state.borrow_mut().ctrl_punct_key(vk, lparam).is_some()
            {
                return true;
            }
            if keys::modifier_down() || keys::is_modifier(vk) {
                return false;
            }
            let mut st = self.state.borrow_mut();
            let available = st.session.available();
            available_seen.set(Some(available));
            let english = st.session.english();
            zlog!(
                "wants_key vk={}({}) available={} english={} composing={}",
                vk,
                vk_label(vk),
                available,
                english,
                st.composing
            );
            if !available {
                return false;
            }
            // 英文模式沒有組字時一般按鍵一律交還宿主（chord 已在上方處理）；
            // 組字中的 printable 進引擎走 Literal，具名鍵仍走下面的 Key 路徑。
            if english {
                // 空白與 keypad 字元都算 printable：`literal` 會把 Space 轉成
                // `" "`，keypad 的 ToUnicode 結果也在同一段 ASCII。不能再額外
                // 排除 keypad，否則 OnTestKeyDown 放行、OnKeyDown 卻送 Literal。
                let printable = keys::literal(vk, lparam, false).is_some();
                match english_route(english, st.composing, printable, st.navigating) {
                    EnglishRoute::Literal => return true,
                    EnglishRoute::Host => return false,
                    EnglishRoute::Key => {}
                }
            }
            if st.composing && st.convert_key(vk).is_some() {
                return true;
            }
            let style_key = st.styled_key(vk, lparam, false, english);
            zlog!(
                "wants_key vk={}({}) style_key={:?}",
                vk,
                vk_label(vk),
                style_key
            );
            if style_key.as_deref() == Some("ShiftSpace") {
                return true;
            }
            if style_key
                .as_ref()
                .is_some_and(|k| st.session.english_state.full_width_extra_key(k))
            {
                return true;
            }
            // 選單開著時由選單吃鍵：Tab／Shift+Tab 換分頁、方向鍵移動、
            // 數字選字、Enter 插入、Esc 關閉。
            if st.menu_open {
                return keys::composing_wants(style_key.as_deref());
            }
            if st.composing {
                let vocab = st.vocab_keys();
                return keys::composing_wants(st.styled_key(vk, lparam, vocab, english).as_deref());
            }
            if keys::is_keypad(vk) {
                return style_key
                    .as_deref()
                    .is_some_and(|k| st.session.english_state.full_width_keypad(k));
            }
            if keys::named(vk).is_some() {
                return false;
            }
            // 沒在組字：Caps Lock 時 printable 是大寫字面，路由要用
            // `styled_key` 正規化後的小寫鍵，才對得上 engine 的 idle_keys。
            style_key
                .as_deref()
                .map(|key| st.session.idle_key(key))
                .unwrap_or(false)
        })();
        let (english, composing) = {
            let st = self.state.borrow();
            (st.session.english(), st.composing)
        };
        zlog!(
            "wants_key 最終 vk={}({}) available={:?} english={} composing={} -> wants={}",
            vk,
            vk_label(vk),
            available_seen.get(),
            english,
            composing,
            wants
        );
        wants
    }

    fn composition_document(&self) -> Option<ITfDocumentMgr> {
        unsafe { self.composition_context()?.GetDocumentMgr().ok() }
    }

    fn composition_context(&self) -> Option<ITfContext> {
        let st = self.state.borrow();
        if !st.composing {
            return None;
        }
        let composition = st.composition.as_ref()?;
        unsafe { composition.GetRange().and_then(|r| r.GetContext()).ok() }
    }

    pub(crate) fn settle_focus(&self) {
        let current = self
            .state
            .borrow()
            .thread_mgr
            .as_ref()
            .and_then(|t| unsafe { t.GetFocus() }.ok());
        let owner = self.composition_document();
        let step = settle_step(current.as_ref(), owner.as_ref());
        self.follow_focus(step, current.is_some());
    }

    fn follow_focus(&self, step: FocusStep<ITfContext>, focused: bool) {
        // 借用限制在區塊內：`if let` 的臨時值會撐到整個大括號，後面
        // `borrow_mut` 就會 panic。這裡只呼叫 SetTimer／KillTimer，不會回呼。
        {
            let st = self.state.borrow();
            if let Some(window) = st.callback.as_ref() {
                if matches!(step, FocusStep::Wait(_)) {
                    window.settle_focus_later();
                } else {
                    window.cancel_settle_focus();
                }
            }
        }
        match step {
            FocusStep::Keep => {
                self.state.borrow_mut().left_from = None;
                zdebug!("焦點落在組字所在的文件，繼續組字");
            }
            FocusStep::Wait(previous) => {
                let from = self.composition_context().or(previous);
                let mut st = self.state.borrow_mut();
                if st.left_from.is_none() {
                    st.left_from = from;
                }
                zdebug!("組字中焦點移到 NULL，稍後確認");
            }
            FocusStep::Leave(previous) => {
                let left_from = self.state.borrow_mut().left_from.take();
                let target = self.composition_context().or(previous).or(left_from);
                self.leave_document(target.as_ref(), focused);
            }
        }
    }

    fn leave_document(&self, previous: Option<&ITfContext>, focused: bool) {
        self.finish(previous, "commit");
        self.forget_candidates();
        let changed = {
            let mut st = self.state.borrow_mut();
            st.right_shift = keys::ShiftTap::default();
            st.left_shift = keys::ShiftTap::default();
            st.session.reset();
            st.selection = None;
            let mode = st.session.learn_mode(false);
            let simplified = focused && st.session.refresh_simplified_after(SIMPLIFIED_FRESH);
            mode || simplified
        };
        if changed {
            self.sync_langbar();
        }
    }

    fn view_window(ctx: Option<&ITfContext>) -> HWND {
        ctx.and_then(|c| unsafe { c.GetActiveView() }.ok())
            .and_then(|v| unsafe { v.GetWnd() }.ok())
            .unwrap_or_default()
    }

    /// 不經引擎、直接把文字送進文件。
    ///
    /// IMM32 例外：只有 `english_key_policy` 判定「沒有插入點的相容層程式」
    /// 才走這裡。這條路徑不經 engine Literal，保留原行為，需實機確認。
    fn type_text(&self, ctx: Option<&ITfContext>, vk: u32, text: String) -> bool {
        let literal = Snapshot {
            ok: true,
            handled: true,
            commit: text,
            ..Default::default()
        };
        let eaten = self.present(ctx, &literal);
        if eaten {
            self.state.borrow_mut().eaten.insert(vk);
        }
        eaten
    }

    fn type_literal(&self, ctx: Option<&ITfContext>, vk: u32, lparam: isize) -> bool {
        if keys::modifier_down() {
            return false;
        }
        let Some(text) = keys::printable(vk, lparam, true) else {
            return false;
        };
        zlog!("TestKeyDown 已吃下的鍵引擎沒有處理，以原字元送進文件");
        self.type_text(ctx, vk, text)
    }
}

impl Drop for TextService {
    fn drop(&mut self) {
        log::object_destroyed();
    }
}

impl ITfTextInputProcessor_Impl for TextService_Impl {
    fn Activate(&self, ptim: Ref<ITfThreadMgr>, tid: u32) -> windows_core::Result<()> {
        crate::guarded("Activate", || {
            // jc 既有的 idempotent guard：0c 直接覆寫 state 會讓第二次 Activate 回 TF_E_NOLOCK。
            if self.state.borrow().thread_mgr.is_some() || self.activated.get() {
                log::log("重複 Activate，忽略");
                return Ok(());
            }
            // 參數不合法時要在日誌指出是哪一個，不要只留下一個 E_INVALIDARG。
            // 提前回錯也不可以把 activated 留在 true，否則下一次 Activate 會被
            // idempotent guard 當成「已啟用」而假成功。
            if ptim.is_null() {
                zlog!(
                    "Activate 參數不合法：ptim=null（ITfTextInputProcessor::Activate 的 pti 必須是有效的 ITfThreadMgr）"
                );
                return Err(E_INVALIDARG.into());
            }
            if tid == 0 {
                zlog!(
                    "Activate 參數不合法：client_id=0（ITfTextInputProcessor::Activate 的 tid 必須是 msctf 給的 TIP client id）"
                );
                return Err(E_INVALIDARG.into());
            }
            let tm = match ptim.ok() {
                Ok(tm) => tm.clone(),
                Err(error) => {
                    zlog!(
                        "Activate 參數不合法：ptim 無法使用 code={:#010x}",
                        error.code().0 as u32
                    );
                    return Err(error);
                }
            };
            self.activated.set(true);
            zlog!(
                "activate 開始 exe={} client_id={} thread={}",
                log::exe_name(),
                tid,
                unsafe { GetCurrentThreadId() }
            );
            // 候選窗的訊息視窗：收 WM_SELECT_CANDIDATE 與延後的焦點確認。
            let callback = candidate::CandidateWindow::new(self).ok();
            let mut opened = None;
            unsafe {
                let keystrokes: ITfKeystrokeMgr = match tm.cast() {
                    Ok(keystrokes) => keystrokes,
                    Err(error) => {
                        self.activated.set(false);
                        zlog!(
                            "Activate 失敗：ITfThreadMgr 沒有 ITfKeystrokeMgr code={:#010x}",
                            error.code().0 as u32
                        );
                        return Err(error);
                    }
                };
                let key_sink: ITfKeyEventSink = self.to_interface();
                if let Err(error) = keystrokes.AdviseKeyEventSink(tid, &key_sink, true) {
                    self.activated.set(false);
                    zlog!(
                        "AdviseKeyEventSink 失敗 client_id={} thread={} code={:#010x}：{}",
                        tid,
                        GetCurrentThreadId(),
                        error.code().0 as u32,
                        advise_failure_hint(error.code())
                    );
                    return Err(error);
                }
                zlog!("advise key sink 完成 client_id={}", tid);
                let source: ITfSource = match tm.cast() {
                    Ok(source) => source,
                    Err(error) => {
                        self.activated.set(false);
                        zlog!(
                            "Activate 失敗：ITfThreadMgr 沒有 ITfSource code={:#010x}",
                            error.code().0 as u32
                        );
                        return Err(error);
                    }
                };
                let tm_sink: ITfThreadMgrEventSink = self.to_interface();
                let cookie = source
                    .AdviseSink(&ITfThreadMgrEventSink::IID, &tm_sink)
                    .unwrap_or(TF_INVALID_COOKIE);
                // 鍵盤開關（Ctrl＋Space，#117）：拿不到就只是沒有這個熱鍵。
                let openclose = tm
                    .cast::<ITfCompartmentMgr>()
                    .and_then(|mgr| mgr.GetCompartment(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE))
                    .ok()
                    .and_then(|compartment| {
                        opened = Some(open_keyboard(&compartment, tid));
                        let sink: ITfCompartmentEventSink = self.to_interface();
                        let cookie = compartment
                            .cast::<ITfSource>()
                            .and_then(|s| s.AdviseSink(&ITfCompartmentEventSink::IID, &sink))
                            .ok()?;
                        Some((compartment, cookie))
                    });
                let openclose_value = openclose
                    .as_ref()
                    .and_then(|(c, _)| compartment_i32(c))
                    .unwrap_or(0);
                // COM-less 啟用（#157）：T0 的 API 對照要求 windows_core::link! 版本。
                let category = crate::msctf::create_category_mgr().ok();
                let atom = KINDS.map(|kind| {
                    category
                        .as_ref()
                        .and_then(|c| c.RegisterGUID(&kind.guid()).ok())
                        .unwrap_or(0)
                });
                // 只有以 TF_TMAE_UIELEMENTENABLEDONLY 啟用的執行緒（自行繪製候選的遊戲等）把候選清單
                // 交給宿主；一般程式即使宿主回 pbShow=FALSE 也照常顯示候選窗（0c #86）。
                let (ui_less, manager) = {
                    let st = self.state.borrow();
                    (st.ui_less, st.thread_mgr.clone())
                };
                let elements = if ui_less {
                    manager
                        .and_then(|tm| tm.cast::<ITfUIElementMgr>().ok())
                        .map(crate::uielement::Elements::new)
                } else {
                    None
                };
                let mut st = self.state.borrow_mut();
                st.thread_mgr = Some(tm);
                st.client_id = tid;
                st.thread_mgr_cookie = cookie;
                st.attr_atoms = atom;
                st.openclose = openclose;
                st.openclose_value = openclose_value;
                st.callback = callback;
                st.ui.set_elements(elements);
                // 引擎已在執行就先問起始模式，不為此啟動引擎。
                st.session.learn_mode(false);
                let english = st.session.english();
                let simplified = st.session.simplified();
                drop(st);
                // 全域快捷鍵：專用執行緒負責 RegisterHotKey 與訊息迴圈；轉送視窗
                // 留在這條 UI 執行緒，熱鍵執行緒只把 chord 排進共用佇列。
                self.create_hotkey_window();
                // 輸入指示器的中／英按鈕與選單；按鈕持有本服務的參考，Deactivate 時移除以解開循環。
                let tm = self.state.borrow().thread_mgr.clone();
                if let Some(Ok(mgr)) = tm.map(|tm| tm.cast::<ITfLangBarItemMgr>()) {
                    let service = self.to_object();
                    let simplify = self.to_object();
                    let recheck = self.to_object();
                    let button = langbar::LangBarButton::create(
                        english,
                        simplified,
                        Box::new(move || service.toggle_mode()),
                        Box::new(move |on| simplify.set_output_simplified(on)),
                        Box::new(move || recheck.recheck_simplified()),
                    );
                    if let Ok(item) = button.cast::<ITfLangBarItem>() {
                        if mgr.AddItem(&item).is_ok() {
                            self.state.borrow_mut().langbar = Some((mgr, button));
                        }
                    }
                }
            }
            zlog!("activate 完成 exe={} client_id={}", log::exe_name(), tid);
            Ok(())
        })
    }

    fn Deactivate(&self) -> windows_core::Result<()> {
        crate::guarded("Deactivate", || {
            if !self.activated.get() {
                log::log("Deactivate：沒有已啟用的服務");
                // 保險：Activate 中途失敗留下快捷鍵執行緒時也要收掉（join），
                // 不讓 DLL 卸載時還有執行緒。
                self.state.borrow_mut().hotkey = None;
                return Ok(());
            }
            self.activated.set(false);
            // 讓還在飛的非同步 probe 結果作廢，避免 callback 回來更新已離開的欄位。
            privacy::invalidate();
            let ctx = self.focused_context();
            self.finish(ctx.as_ref(), "commit");
            self.cancel_ja_live_idle();
            self.forget_candidates();
            // 只清 UI／composition／callback，**不清 session，也不拆它的 IPC client**。
            // 這樣第二次 Activate 不會從零重建 worker，也不會踩到 TF_E_NOLOCK 後把 worker 拆掉。
            let (tm, tid, cookie) = {
                let mut st = self.state.borrow_mut();
                st.callback = None;
                // 先停熱鍵執行緒（送 WM_QUIT 並 join），再銷毀轉送視窗；
                // Drop 也會做一次，這裡讓卸載前的狀態明確。
                st.hotkey = None;
                st.ui.hide();
                st.ui.set_elements(None);
                // Deactivate 要真的銷毀候選窗與 D2D／D3D／DComp 資源，DllCanUnloadNow 才能回 0。
                st.ui.shutdown();
                st.completing = None;
                st.pending_repost = None;
                st.composition = None;
                st.composing = false;
                st.menu_open = false;
                st.navigating = false;
                st.left_from = None;
                st.eaten.clear();
                st.right_shift = keys::ShiftTap::default();
                st.left_shift = keys::ShiftTap::default();
                st.selection = None;
                st.shown = None;
                (st.thread_mgr.take(), st.client_id, st.thread_mgr_cookie)
            };
            let openclose = self.state.borrow_mut().openclose.take();
            if let Some((compartment, cookie)) = openclose {
                if let Ok(source) = compartment.cast::<ITfSource>() {
                    let _ = unsafe { source.UnadviseSink(cookie) };
                }
            }
            // RemoveItem 解開按鈕持有 service 參考造成的 COM 循環。
            let langbar = self.state.borrow_mut().langbar.take();
            if let Some((mgr, button)) = langbar {
                if let Ok(item) = button.cast::<ITfLangBarItem>() {
                    let _ = unsafe { mgr.RemoveItem(&item) };
                }
            }
            if let Some(tm) = tm {
                unsafe {
                    if let Ok(keystrokes) = tm.cast::<ITfKeystrokeMgr>() {
                        let _ = keystrokes.UnadviseKeyEventSink(tid);
                    }
                    if cookie != TF_INVALID_COOKIE {
                        if let Ok(source) = tm.cast::<ITfSource>() {
                            let _ = source.UnadviseSink(cookie);
                        }
                    }
                }
            }
            zlog!("deactivate");
            Ok(())
        })
    }
}

impl ITfTextInputProcessorEx_Impl for TextService_Impl {
    fn ActivateEx(
        &self,
        ptim: Ref<ITfThreadMgr>,
        tid: u32,
        flags: u32,
    ) -> windows_core::Result<()> {
        crate::guarded("ActivateEx", || {
            {
                let mut st = self.state.borrow_mut();
                st.secure = flags & TF_TMAE_SECUREMODE != 0;
                st.ui_less = flags & TF_TMAE_UIELEMENTENABLEDONLY != 0;
                st.activate_flags = flags;
            }
            zlog!(
                "activate flags={:#x} comless={} console={} immersive={}",
                flags,
                flags & TF_TMAE_COMLESS != 0,
                flags & TF_TMAE_CONSOLE != 0,
                flags & TF_TMF_IMMERSIVEMODE != 0
            );
            self.Activate(ptim, tid)
        })
    }
}

impl ITfKeyEventSink_Impl for TextService_Impl {
    fn OnSetFocus(&self, foreground: BOOL) -> windows_core::Result<()> {
        crate::guarded("OnSetFocus", || {
            if foreground.as_bool() {
                // 只有前景行程保留全域組合；背景行程占住會讓前景的 TSF 收不到。
                self.drain_hotkeys();
                self.enable_hotkeys();
                let _ = self.state.borrow_mut().session.refresh_bindings();
            } else {
                self.release_hotkeys();
                // 先記住舊文件：清掉 thread focus 之後 focused_context 就回 None。
                let previous = self.focused_context();
                if let Some(ctx) = previous.as_ref() {
                    privacy::forget_context(ctx);
                }
                {
                    let mut st = self.state.borrow_mut();
                    st.right_shift = keys::ShiftTap::default();
                    st.left_shift = keys::ShiftTap::default();
                    st.selection = None;
                    st.navigating = false;
                }
                // 失焦時清掉 thread focus：熱鍵與下一次按鍵都拿不到舊文件，
                // 不會對已離開的文件開選單。
                let thread_mgr = self.state.borrow().thread_mgr.clone();
                if let Some(thread_mgr) = thread_mgr {
                    if let Err(error) = unsafe { thread_mgr.SetFocus(None::<&ITfDocumentMgr>) } {
                        zdebug!(
                            "失焦：清除 thread focus 失敗：{error}（HRESULT {:#010x}）",
                            error.code().0 as u32
                        );
                    }
                }
                self.forget_candidates();
            }
            Ok(())
        })
    }

    fn OnTestKeyDown(
        &self,
        pic: Ref<ITfContext>,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> windows_core::Result<BOOL> {
        self.guarded_key("OnTestKeyDown", pic.as_ref(), || {
            let vk = wparam.0 as u32;
            zlog!(
                "OnTestKeyDown 進入 vk={}({}) lparam={:#x}",
                vk,
                vk_label(vk),
                lparam.0
            );
            {
                let mut st = self.state.borrow_mut();
                st.hotkey_tested.forget();
                st.key_tested.forget();
            }
            // 重排回來的鍵（#66）：同一個 (vk, lParam) 再出現就是它回來了。清掉等待、不吃，交給程式。
            {
                let mut st = self.state.borrow_mut();
                if let Some(pending) = st.pending_repost.take() {
                    if pending.vk == vk && pending.lparam == lparam.0 {
                        zlog!("OnTestKeyDown vk={}({}) 重排回來 -> 不吃", vk, vk_label(vk));
                        zdebug!("OnTestKeyDown vk={} 重排回來 -> 不吃", wparam.0);
                        return Ok(false.into());
                    }
                    zlog!(
                        "OnTestKeyDown vk={}({}) 重排的鍵沒回來，清掉等待 pending_vk={}",
                        vk,
                        vk_label(vk),
                        pending.vk
                    );
                    zdebug!("OnTestKeyDown vk={} 重排的鍵沒回來，清掉等待", pending.vk);
                }
            }
            if !self.allowed(pic.as_ref(), "OnTestKeyDown", vk) {
                zlog!(
                    "OnTestKeyDown vk={}({}) 結束 allowed=false -> 不吃",
                    vk,
                    vk_label(vk)
                );
                return Ok(false.into());
            }
            self.observe_shift(vk, lparam.0);
            if self.test_simplified_hotkey(vk) {
                zlog!(
                    "OnTestKeyDown vk={}({}) 結束：簡體熱鍵 -> 吃",
                    vk,
                    vk_label(vk)
                );
                return Ok(true.into());
            }
            // Shift＋字母／數字在組字中一律走 literal：先於快捷鍵與
            // commit_before_host_key，不能先提交，也不該為了查 bindings 送 IPC。
            if self
                .state
                .borrow()
                .shift_literal_key(vk, lparam.0)
                .is_some()
            {
                zlog!(
                    "OnTestKeyDown vk={}({}) 結束：Shift 字面 -> 吃",
                    vk,
                    vk_label(vk)
                );
                return Ok(true.into());
            }
            // 全域 chord 不是文字輸入：英文模式也要先判斷，命中就送 engine
            // 由 keymap 執行 command。其他按鍵才走英文 raw passthrough。
            if self.bound_chord(vk, lparam.0).is_some() {
                self.state.borrow_mut().key_tested.record(vk, true);
                zlog!(
                    "OnTestKeyDown vk={}({}) 結束：綁定快捷鍵 -> 吃",
                    vk,
                    vk_label(vk)
                );
                return Ok(true.into());
            }
            if self.commit_before_host_key(pic.as_ref(), vk, lparam.0, true) {
                zlog!(
                    "OnTestKeyDown vk={}({}) 結束：攔下重排 -> 吃",
                    vk,
                    vk_label(vk)
                );
                zdebug!("OnTestKeyDown vk={} -> 攔下重排", vk);
                return Ok(true.into());
            }
            self.learn_mode(vk);
            let wants = match self.english_key_policy(pic.as_ref(), vk, lparam.0) {
                // 這幾條路徑都會由輸入法自己處理或直接寫入文件。
                Some(EnglishKey::Literal(_)) => true,
                Some(EnglishKey::Pass) | None => self.wants_key(vk, lparam.0),
            };
            self.state.borrow_mut().key_tested.record(vk, wants);
            zlog!(
                "OnTestKeyDown 結束 vk={}({}) wants={} -> {}",
                vk,
                vk_label(vk),
                wants,
                if wants { "吃" } else { "不吃" }
            );
            Ok(wants.into())
        })
    }

    fn OnTestKeyUp(
        &self,
        pic: Ref<ITfContext>,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> windows_core::Result<BOOL> {
        self.guarded_key("OnTestKeyUp", pic.as_ref(), || {
            {
                let mut st = self.state.borrow_mut();
                st.hotkey_tested.forget();
                st.key_tested.forget();
                // 放開就是這一鍵結束了：重排的鍵沒回來也不再等。
                st.pending_repost = None;
            }
            if !self.allowed(pic.as_ref(), "OnTestKeyUp", wparam.0 as u32) {
                return Ok(false.into());
            }
            let vk = wparam.0 as u32;
            if let Some(side) = keys::shift_side(vk, lparam.0) {
                self.shift_released(side);
                return Ok(false.into());
            }
            Ok(self.state.borrow().eaten.contains(&vk).into())
        })
    }

    fn OnKeyDown(
        &self,
        pic: Ref<ITfContext>,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> windows_core::Result<BOOL> {
        self.guarded_key("OnKeyDown", pic.as_ref(), || {
            let vk = wparam.0 as u32;
            zlog!(
                "OnKeyDown 進入 vk={}({}) lparam={:#x}",
                vk,
                vk_label(vk),
                lparam.0
            );
            // TestKeyDown 已經送出組字並重排：這一次的 OnKeyDown 只吃下，不再判斷。
            {
                let mut st = self.state.borrow_mut();
                if let Some(pending) = st.pending_repost.as_mut() {
                    if pending.vk == vk && !pending.key_down_seen {
                        pending.key_down_seen = true;
                        zlog!(
                            "OnKeyDown vk={}({}) 結束：重排的 OnKeyDown 只吃下",
                            vk,
                            vk_label(vk)
                        );
                        return Ok(true.into());
                    }
                }
            }
            let (tested, key_tested) = {
                let mut st = self.state.borrow_mut();
                (st.hotkey_tested.take(vk), st.key_tested.take(vk))
            };
            zlog!(
                "OnKeyDown vk={}({}) tested={:?} key_tested={:?}",
                vk,
                vk_label(vk),
                tested,
                key_tested
            );
            if !self.allowed(pic.as_ref(), "OnKeyDown", vk) {
                zlog!(
                    "OnKeyDown vk={}({}) 結束 allowed=false -> 不吃",
                    vk,
                    vk_label(vk)
                );
                return Ok(false.into());
            }
            self.observe_shift(vk, lparam.0);
            if self.key_simplified_hotkey(vk, tested) {
                let on = !self.state.borrow().session.simplified().unwrap_or(false);
                let switched = self.set_output_simplified(on);
                zlog!(
                    "OnKeyDown vk={}({}) 結束：切換簡體字 on={} ok={}",
                    vk,
                    vk_label(vk),
                    on,
                    switched
                );
                zdebug!("OnKeyDown vk={} 切換簡體字 on={} ok={}", vk, on, switched);
                self.state.borrow_mut().eaten.insert(vk);
                return Ok(true.into());
            }
            // 英文模式且還有組字：可列印字元送 engine 的 literal（跟中文的 @
            // 一樣接在既有組字後面），不走 Key、也不 type_literal 到宿主。
            // 具名鍵（Enter、Backspace、方向鍵）仍走下面的原本 Key 路徑。
            let (english, composing, navigating) = {
                let st = self.state.borrow();
                (st.session.english(), st.composing, st.navigating)
            };
            if english && composing {
                let text = if keys::modifier_down() || keys::is_modifier(vk) {
                    None
                } else {
                    // 空白（含 Shift+Space）與 keypad 字元都走 Literal；`literal`
                    // 會把 Space 轉成 `" "`。`printable` 只看可見 ASCII，會漏掉空白。
                    keys::literal(vk, lparam.0, false)
                };
                if let Some(text) = text {
                    // 候選窗／選單開著時改走 Key：數字選字、字母選項要進 engine。
                    let route = english_route(english, composing, true, navigating);
                    if route == EnglishRoute::Literal {
                        // handle_literal 失敗時自己吃掉這一鍵；英文 printable
                        // 只有 Literal 一條路徑，不能回落到 handle_key。
                        let eaten = self.handle_literal(pic.as_ref(), &text);
                        if eaten {
                            self.state.borrow_mut().eaten.insert(vk);
                        }
                        zlog!(
                            "OnKeyDown vk={}({}) 結束：英文組字 Literal={:?} eaten={}",
                            vk,
                            vk_label(vk),
                            text,
                            eaten
                        );
                        return Ok(eaten.into());
                    }
                    zlog!(
                        "OnKeyDown vk={}({}) 英文 printable={:?} navigating={} -> Key",
                        vk,
                        vk_label(vk),
                        text,
                        navigating
                    );
                }
            }
            // Shift＋字母／數字先走 literal：先於快捷鍵與 commit_before_host_key，
            // 不能先提交組字，否則組字區會消失、A 變成應用程式直接打進文件。
            // `if let` 會把 borrow 的臨時值延長到整個區塊；`handle_literal`
            // 內要 `borrow_mut`，直接寫在條件裡會 panic（Shift＋`[` 曾因此
            // 炸掉整個 OnKeyDown）。先取值再判斷。
            let shift_literal = self.state.borrow().shift_literal_key(vk, lparam.0);
            if let Some(text) = shift_literal {
                zlog!(
                    "OnKeyDown vk={}({}) 結束：Shift 字面={:?}",
                    vk,
                    vk_label(vk),
                    text
                );
                return Ok(self.handle_literal(pic.as_ref(), &text).into());
            }
            if let Some(chord) = self.bound_chord(vk, lparam.0) {
                if let Some(key) = keys::chord_key(vk, lparam.0) {
                    let eaten = self.handle_chord(pic.as_ref(), &key);
                    if eaten {
                        self.state.borrow_mut().eaten.insert(vk);
                    }
                    zlog!(
                        "OnKeyDown vk={}({}) 結束：綁定 chord={} eaten={}",
                        vk,
                        vk_label(vk),
                        chord,
                        eaten
                    );
                    return Ok(eaten.into());
                }
            }
            self.commit_before_host_key(pic.as_ref(), vk, lparam.0, false);
            self.learn_mode(vk);
            let literal_fallback = key_tested == Some(true) && {
                let st = self.state.borrow();
                !st.composing && !st.session.english()
            };
            let wants = match self.english_key_policy(pic.as_ref(), vk, lparam.0) {
                Some(EnglishKey::Literal(text)) => {
                    zlog!(
                        "OnKeyDown vk={}({}) 結束：英文 Literal={:?}",
                        vk,
                        vk_label(vk),
                        text
                    );
                    return Ok(self.type_text(pic.as_ref(), vk, text).into());
                }
                Some(EnglishKey::Pass) | None => self.wants_key(vk, lparam.0),
            };
            let composing = self.state.borrow().composing;
            zlog!(
                "OnKeyDown vk={}({}) wants={} composing={} literal_fallback={}",
                vk,
                vk_label(vk),
                wants,
                composing,
                literal_fallback
            );
            if !wants {
                if literal_fallback {
                    zlog!(
                        "OnKeyDown vk={}({}) 結束：literal fallback 送出",
                        vk,
                        vk_label(vk)
                    );
                    return Ok(self.type_literal(pic.as_ref(), vk, lparam.0).into());
                }
                zlog!(
                    "OnKeyDown vk={}({}) 結束 wants=false -> 不吃",
                    vk,
                    vk_label(vk)
                );
                return Ok(false.into());
            }
            let (vocab, english) = {
                let st = self.state.borrow();
                (st.vocab_keys(), st.session.english())
            };
            let ctrl = if keys::modifier_down() {
                self.state.borrow_mut().ctrl_punct_key(vk, lparam.0)
            } else {
                None
            };
            let convert = self.state.borrow_mut().convert_key(vk);
            let Some(key) = convert
                .map(str::to_string)
                .or_else(|| ctrl.map(str::to_string))
                .or_else(|| {
                    self.state
                        .borrow_mut()
                        .styled_key(vk, lparam.0, vocab, english)
                })
            else {
                zlog!(
                    "OnKeyDown vk={}({}) 結束：key=None -> 不吃",
                    vk,
                    vk_label(vk)
                );
                return Ok(false.into());
            };
            zlog!("OnKeyDown vk={}({}) 送引擎 key={}", vk, vk_label(vk), key);
            let mut eaten = self.handle_key(pic.as_ref(), &key);
            if !eaten && literal_fallback && !self.state.borrow().composing {
                eaten = self.type_literal(pic.as_ref(), vk, lparam.0);
            }
            if eaten {
                self.state.borrow_mut().eaten.insert(vk);
            }
            zlog!(
                "OnKeyDown 結束 vk={}({}) key={} eaten={}",
                vk,
                vk_label(vk),
                key,
                eaten
            );
            Ok(eaten.into())
        })
    }

    fn OnKeyUp(
        &self,
        pic: Ref<ITfContext>,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> windows_core::Result<BOOL> {
        self.guarded_key("OnKeyUp", pic.as_ref(), || {
            {
                let mut st = self.state.borrow_mut();
                st.hotkey_tested.forget();
                st.key_tested.forget();
            }
            if !self.allowed(pic.as_ref(), "OnKeyUp", wparam.0 as u32) {
                return Ok(false.into());
            }
            if let Some(side) = keys::shift_side(wparam.0 as u32, lparam.0) {
                self.shift_released(side);
                return Ok(false.into());
            }
            // keyup 會消費掉 keydown 記下的「吃掉」：同一個 vk 之後由宿主
            // 正常收到 keydown 時，keyup 不能沿用上一次的記錄。
            let eaten = self.state.borrow_mut().eaten.remove(&(wparam.0 as u32));
            Ok(eaten.into())
        })
    }

    fn OnPreservedKey(
        &self,
        _pic: Ref<ITfContext>,
        _guid: *const GUID,
    ) -> windows_core::Result<BOOL> {
        crate::guarded("OnPreservedKey", || Ok(false.into()))
    }
}

impl ITfCompositionSink_Impl for TextService_Impl {
    fn OnCompositionTerminated(
        &self,
        _ec: u32,
        composition: Ref<ITfComposition>,
    ) -> windows_core::Result<()> {
        crate::guarded("OnCompositionTerminated", || {
            {
                let st = self.state.borrow();
                if st.ending_composition.get()
                    || st.composition.is_none()
                    || st.composition.as_ref() != composition.as_ref()
                {
                    return Ok(());
                }
            }
            // 外殼自己請相容層完成組字（Completing 守衛，#66）而終止的正是那個組字：引擎已經 commit
            // 並學習，只清外殼狀態，不送取消。
            let by_cuas = {
                let st = self.state.borrow();
                matches!(&st.completing, Some(state)
                    if state.composition.as_ref() == composition.as_ref())
            };
            if !by_cuas {
                zlog!("組字被宿主終止 exe={}", log::exe_name());
            }
            {
                let mut st = self.state.borrow_mut();
                st.composition = None;
                st.composing = false;
                st.menu_open = false;
                st.navigating = false;
                st.left_from = None;
                if by_cuas {
                    if let Some(state) = st.completing.as_mut() {
                        state.terminated = true;
                    }
                }
            }
            self.forget_candidates();
            if by_cuas {
                zdebug!("組字由相容層完成，不送取消");
                return Ok(());
            }
            let _ = self
                .state
                .borrow_mut()
                .session
                .handle(json!({"type": "cancel"}));
            Ok(())
        })
    }
}

impl ITfThreadMgrEventSink_Impl for TextService_Impl {
    fn OnInitDocumentMgr(&self, _pdim: Ref<ITfDocumentMgr>) -> windows_core::Result<()> {
        crate::guarded("OnInitDocumentMgr", || Ok(()))
    }
    fn OnUninitDocumentMgr(&self, _pdim: Ref<ITfDocumentMgr>) -> windows_core::Result<()> {
        crate::guarded("OnUninitDocumentMgr", || Ok(()))
    }
    fn OnSetFocus(
        &self,
        focus: Ref<ITfDocumentMgr>,
        previous: Ref<ITfDocumentMgr>,
    ) -> windows_core::Result<()> {
        crate::guarded("OnSetFocus", || {
            let previous = previous.as_ref().and_then(|d| unsafe { d.GetTop().ok() });
            // 焦點離開舊 context：清掉它的 cache，並讓它的非同步 probe 結果過期。
            if let Some(ctx) = previous.as_ref() {
                privacy::forget_context(ctx);
            }
            if focus.as_ref().is_some() {
                let _ = self.state.borrow_mut().session.refresh_bindings();
            }
            let owner = self.composition_document();
            let can_wait = self.state.borrow().callback.is_some();
            let step = focus_step(focus.as_ref(), owner.as_ref(), previous, can_wait);
            self.follow_focus(step, focus.as_ref().is_some());
            Ok(())
        })
    }
    fn OnPushContext(&self, _pic: Ref<ITfContext>) -> windows_core::Result<()> {
        crate::guarded("OnPushContext", || Ok(()))
    }
    fn OnPopContext(&self, _pic: Ref<ITfContext>) -> windows_core::Result<()> {
        crate::guarded("OnPopContext", || Ok(()))
    }
}

impl ITfTextLayoutSink_Impl for TextService_Impl {
    fn OnLayoutChange(
        &self,
        pic: Ref<ITfContext>,
        lcode: TfLayoutCode,
        _view: Ref<ITfContextView>,
    ) -> windows_core::Result<()> {
        crate::guarded("OnLayoutChange", || {
            if lcode == TF_LC_DESTROY {
                return Ok(());
            }
            if self.applying.get() > 0 {
                self.layout_missed.set(true);
                return Ok(());
            }
            self.requery(pic.as_ref());
            Ok(())
        })
    }
}

impl ITfCompartmentEventSink_Impl for TextService_Impl {
    // TSF 契約保證 `guid` 指向有效的 GUID，簽章由 COM 介面決定，不能改成 unsafe fn。
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn OnChange(&self, guid: *const GUID) -> windows_core::Result<()> {
        crate::guarded("OnCompartmentChange", || {
            if !guid.is_null() && unsafe { *guid } == GUID_COMPARTMENT_KEYBOARD_OPENCLOSE {
                self.keyboard_openclose_changed();
            }
            Ok(())
        })
    }
}

impl ITfDisplayAttributeProvider_Impl for TextService_Impl {
    fn EnumDisplayAttributeInfo(
        &self,
    ) -> windows_core::Result<windows::Win32::UI::TextServices::IEnumTfDisplayAttributeInfo> {
        crate::guarded("EnumDisplayAttributeInfo", || {
            Ok(ComObject::new(AttributeEnum::new()).into_interface())
        })
    }
    // TSF 契約保證 `guid` 指向有效的 GUID，簽章由 COM 介面決定，不能改成 unsafe fn。
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn GetDisplayAttributeInfo(
        &self,
        guid: *const GUID,
    ) -> windows_core::Result<windows::Win32::UI::TextServices::ITfDisplayAttributeInfo> {
        crate::guarded("GetDisplayAttributeInfo", || {
            if let Some(kind) = KINDS
                .into_iter()
                .find(|kind| !guid.is_null() && unsafe { *guid } == kind.guid())
            {
                Ok(ComObject::new(InputAttribute::new(kind)).into_interface())
            } else {
                Err(E_INVALIDARG.into())
            }
        })
    }
}

/// 啟用時把執行緒的鍵盤開關設成開（#257），回傳（設定前, 設定後）。
fn open_keyboard(compartment: &ITfCompartment, client: u32) -> (i32, i32) {
    let before = compartment_i32(compartment).unwrap_or(0);
    if before != 0 {
        return (before, before);
    }
    let set = unsafe { compartment.SetValue(client, &VARIANT::from(1i32)) };
    (
        before,
        compartment_i32(compartment).unwrap_or(if set.is_ok() { 1 } else { before }),
    )
}

/// compartment 的 VT_I4 值；沒有值或讀不到回 None。
fn compartment_i32(compartment: &ITfCompartment) -> Option<i32> {
    unsafe {
        let mut value = compartment.GetValue().ok()?;
        let out = (value.Anonymous.Anonymous.vt == VT_I4)
            .then(|| value.Anonymous.Anonymous.Anonymous.lVal);
        let _ = VariantClear(&mut value);
        out
    }
}

/// 前景視窗所屬行程（診斷用，保留 0c 的呼叫點）。
#[allow(dead_code)]
fn foreground_pid() -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid)) };
    pid
}

/// 英文模式沒有組字時，文字輸入是純 raw：按鍵不進引擎、不建組字，測試替身必須
/// 看到 0 個一般請求；只有已註冊的全域 chord 例外。組字中的英文走 Literal，
/// 由 `english_route_tests` 覆蓋。
#[cfg(all(test, windows))]
mod english_key_policy_tests {
    use super::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_CONTROL, VK_LMENU, VK_LWIN, VK_MENU, VK_RMENU,
        VK_RWIN, VK_SHIFT,
    };
    use windows::Win32::UI::TextServices::ITfKeyEventSink;

    const VK_BACKSPACE: u32 = 0x08;
    const BACKSPACE_LPARAM: isize = 1 | (0x0e << 16);
    const VK_RETURN: u32 = 0x0d;
    const RETURN_LPARAM: isize = 1 | (0x1c << 16);

    /// 清掉 Ctrl／Alt／Win／Shift，只留英文模式。
    fn plain_keyboard() {
        let mut state = [0u8; 256];
        unsafe {
            GetKeyboardState(&mut state).unwrap();
            for vk in [VK_CONTROL, VK_LMENU, VK_RMENU, VK_LWIN, VK_RWIN, VK_SHIFT] {
                state[vk.0 as usize] = 0;
            }
            SetKeyboardState(&state).unwrap();
        }
    }

    /// 按著 Ctrl＋Alt（AltGr 也是這兩個），模擬選單 chord。
    fn hold_ctrl_alt(on: bool) {
        let mut state = [0u8; 256];
        unsafe {
            GetKeyboardState(&mut state).unwrap();
            let value = if on { 0x80 } else { 0 };
            state[VK_CONTROL.0 as usize] = value;
            state[VK_MENU.0 as usize] = value;
            SetKeyboardState(&state).unwrap();
        }
    }

    fn hold_shift(on: bool) {
        let mut state = [0u8; 256];
        unsafe {
            GetKeyboardState(&mut state).unwrap();
            state[VK_SHIFT.0 as usize] = if on { 0x80 } else { 0 };
            SetKeyboardState(&state).unwrap();
        }
    }

    fn english_service() -> ComObject<TextService> {
        let service = ComObject::new(TextService::new());
        let mut st = service.state.borrow_mut();
        st.session = super::support::session();
        assert!(st.session.toggle().english, "外殼自己切英文");
        drop(st);
        service
    }

    fn request_count(service: &TextService) -> usize {
        service.state.borrow().session.sent.len()
    }

    /// 英文模式的 printable、Tab、方向鍵、Backspace、Enter 都不得產生任何請求。
    #[test]
    fn english_mode_keys_send_zero_requests() {
        plain_keyboard();
        let service = english_service();
        let (_doc, ctx) = super::support::host(super::support::Sync::Run);
        let sink: ITfKeyEventSink = service.to_interface();
        for (vk, lparam, label) in [
            (0x41u32, 0x1e, "A"),
            (VK_BACKSPACE, BACKSPACE_LPARAM, "Backspace"),
            (0x09, 0x0f, "Tab"),
            (0x25, 0x14b, "Left"),
            (0x28, 0x150, "Down"),
            (VK_RETURN, RETURN_LPARAM, "Enter"),
            (VK_SHIFT.0 as u32, 0x2a, "Shift"),
            (VK_CONTROL.0 as u32, 0x1d, "Ctrl"),
        ] {
            let (w, l) = (WPARAM(vk as usize), LPARAM(lparam));
            let tested = unsafe { sink.OnTestKeyDown(&ctx, w, l) }.unwrap().as_bool();
            assert!(!tested, "{label} 要交還宿主");
            let eaten = unsafe { sink.OnKeyDown(&ctx, w, l) }.unwrap().as_bool();
            assert!(!eaten, "{label} 不得被吃掉");
            assert_eq!(request_count(&service), 0, "{label} 不得送引擎");
        }
        assert!(!service.state.borrow().composing, "英文模式不建組字");
    }

    /// 組字中切到英文：leftover 組字留著，之後的 printable 走引擎 Literal。
    ///
    /// 這個行為與組字結束分開：切英文本身不 commit、不 cancel，宿主端組字不動。
    #[test]
    fn english_mode_composing_printable_uses_literal_without_commit() {
        plain_keyboard();
        let service = english_service();
        {
            let mut st = service.state.borrow_mut();
            st.composing = true;
            st.session.english_state = Snapshot {
                ok: true,
                handled: true,
                english: true,
                preedit: vec![crate::session::Segment {
                    text: "ㄨ".into(),
                    style: "normal".into(),
                }],
                cursor_rune: 1,
                cursor_utf16: 1,
                ..Default::default()
            };
        }
        let (_doc, ctx) = super::support::host(super::support::Sync::Run);
        let sink: ITfKeyEventSink = service.to_interface();
        let (w, l) = (WPARAM(0x41), LPARAM(0x1e));
        assert!(unsafe { sink.OnTestKeyDown(&ctx, w, l) }.unwrap().as_bool());
        assert!(unsafe { sink.OnKeyDown(&ctx, w, l) }.unwrap().as_bool());
        assert_eq!(
            service.state.borrow().session.sent.clone(),
            vec!["literal".to_string()],
            "組字中的英文要送 Literal，不是 Key"
        );
        let st = service.state.borrow();
        assert!(st.composing, "切英文不得終止組字");
        assert_eq!(st.session.english_state.text(), "a", "引擎收到字面 a");
    }

    /// Shift＋空白在英文模式也不由外殼插全形空白，直接交還宿主。
    #[test]
    fn english_mode_shift_space_is_raw() {
        plain_keyboard();
        hold_shift(true);
        let service = english_service();
        let (_doc, ctx) = super::support::host(super::support::Sync::Run);
        let sink: ITfKeyEventSink = service.to_interface();
        let (w, l) = (WPARAM(0x20), LPARAM(0x39));
        assert!(
            !unsafe { sink.OnTestKeyDown(&ctx, w, l) }.unwrap().as_bool(),
            "Shift＋空白不得被外殼吃掉"
        );
        assert!(
            !unsafe { sink.OnKeyDown(&ctx, w, l) }.unwrap().as_bool(),
            "OnKeyDown 也要放行"
        );
        assert_eq!(request_count(&service), 0, "不得送引擎");
        hold_shift(false);
    }

    /// 已註冊的全域 chord 例外：英文模式仍送一個 Key 給 engine 執行 command。
    #[test]
    fn english_mode_bound_chord_sends_exactly_one_key() {
        plain_keyboard();
        hold_ctrl_alt(true);
        let service = english_service();
        service
            .state
            .borrow_mut()
            .session
            .set_bindings_for_test([("symbol_menu".to_string(), "ctrl+alt+,".to_string())]);
        let (_doc, ctx) = super::support::host(super::support::Sync::Run);
        let sink: ITfKeyEventSink = service.to_interface();
        let before = request_count(&service);
        let (w, l) = (WPARAM(0xBC), LPARAM(0x33));
        assert!(
            unsafe { sink.OnTestKeyDown(&ctx, w, l) }.unwrap().as_bool(),
            "chord 要在 TestKeyDown 被認出來"
        );
        let _ = unsafe { sink.OnKeyDown(&ctx, w, l) }.unwrap();
        let sent = service.state.borrow().session.sent.clone();
        assert_eq!(
            sent,
            vec!["key".to_string()],
            "英文模式只允許 chord 這一個 Key"
        );
        assert_eq!(request_count(&service), before + 1);
        hold_ctrl_alt(false);
    }

    /// Ctrl／Alt／Win 組合鍵直接放行，不送引擎。
    #[test]
    fn english_mode_modifiers_pass_through() {
        plain_keyboard();
        let service = english_service();
        let (_doc, ctx) = super::support::host(super::support::Sync::Run);
        let decision = service.english_key_policy(Some(&ctx), 0x41, 0x1e);
        assert!(
            matches!(decision, None | Some(EnglishKey::Pass)),
            "沒有修飾鍵時政策不介入：{decision:?}"
        );
        assert_eq!(request_count(&service), 0, "政策是純查詢");
    }

    /// F6–F10／Convert 只看 capability，不再需要 legacy 日文輸入模式。
    #[test]
    fn convert_keys_follow_capability_not_language_mode() {
        plain_keyboard();
        let service = ComObject::new(TextService::new());
        let mut st = service.state.borrow_mut();
        st.session = super::support::session();
        st.session.set_japanese_for_test(true);
        assert!(!st.session.english(), "不是英文模式");
        assert_eq!(st.convert_key(0x75), Some("F6"), "VK_F6 要送 core");
        assert_eq!(
            st.convert_key(0x1c),
            Some("Convert"),
            "VK_CONVERT 要送 core"
        );
        st.session.set_japanese_for_test(false);
        assert_eq!(st.convert_key(0x75), None, "沒有 capability 就不送");
    }
}

/// 英文路由（printable／Key／Literal）與 Caps Lock 路由的 host 測試。
///
/// 只驗外殼的決策與送出的請求；引擎用 `t8a_support` 的記憶體替身，
/// 不需要真的 pipe。Caps Lock 的實體鍵盤狀態只改本執行緒。
#[cfg(all(test, windows))]
mod english_route_tests {
    use super::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, MapVirtualKeyW, SetKeyboardState, MAPVK_VK_TO_VSC, VK_CAPITAL,
        VK_CONTROL, VK_LMENU, VK_LWIN, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
    };
    use windows::Win32::UI::TextServices::ITfKeyEventSink;

    const VK_BACKSPACE: u32 = 0x08;
    const VK_RETURN: u32 = 0x0d;

    fn keyboard_state() -> [u8; 256] {
        let mut state = [0u8; 256];
        unsafe { GetKeyboardState(&mut state).unwrap() };
        state
    }

    fn set_keyboard_state(state: &[u8; 256]) {
        unsafe { SetKeyboardState(state).unwrap() };
    }

    /// 清掉 Caps Lock 與所有修飾鍵。
    fn plain_keyboard() {
        let mut state = keyboard_state();
        state[VK_CAPITAL.0 as usize] = 0;
        for vk in [VK_CONTROL, VK_LMENU, VK_RMENU, VK_LWIN, VK_RWIN, VK_SHIFT] {
            state[vk.0 as usize] = 0;
        }
        set_keyboard_state(&state);
    }

    fn hold_shift(on: bool) {
        let mut state = keyboard_state();
        let value = if on { 0x80 } else { 0 };
        state[VK_SHIFT.0 as usize] = value;
        set_keyboard_state(&state);
    }

    fn hold_caps(on: bool) {
        let mut state = keyboard_state();
        state[VK_CAPITAL.0 as usize] = if on { 1 } else { 0 };
        set_keyboard_state(&state);
    }

    struct Rig {
        service: ComObject<TextService>,
        sink: ITfKeyEventSink,
        ctx: ITfContext,
        doc: Rc<support::Doc>,
        host: String,
    }

    impl Rig {
        fn new(english: bool) -> Rig {
            Self::with_session(support::session(), english)
        }

        fn with_session(session: crate::session::Session, english: bool) -> Rig {
            let (doc, ctx) = support::host(support::Sync::Run);
            let service = ComObject::new(TextService::new());
            {
                let mut st = service.state.borrow_mut();
                st.session = session;
                if english {
                    assert!(st.session.toggle().english, "切到英文模式");
                }
            }
            let sink: ITfKeyEventSink = service.to_interface();
            Rig {
                service,
                sink,
                ctx,
                doc,
                host: String::new(),
            }
        }

        /// 模擬引擎回報選單／候選窗開著；下一次 present 會再依快照覆寫。
        fn set_navigation(&self, menu_open: bool, navigating: bool) {
            let mut st = self.service.state.borrow_mut();
            st.menu_open = menu_open;
            st.navigating = navigating;
        }

        fn sent(&self) -> Vec<String> {
            self.service.state.borrow().session.sent.clone()
        }

        fn english(&self) -> bool {
            self.service.state.borrow().session.english()
        }

        fn composing(&self) -> bool {
            self.service.state.borrow().composing
        }

        /// 一鍵：TestKeyDown 說吃才送 OnKeyDown；不吃就當成宿主收到 `host_char`。
        /// 回傳 TestKeyDown 的答案。
        fn press_vk(&mut self, vk: u32, scan: isize, host_char: Option<char>) -> bool {
            let (w, l) = (WPARAM(vk as usize), LPARAM(1 | scan << 16));
            let tested = unsafe { self.sink.OnTestKeyDown(&self.ctx, w, l) }
                .unwrap()
                .as_bool();
            if tested {
                let eaten = unsafe { self.sink.OnKeyDown(&self.ctx, w, l) }
                    .unwrap()
                    .as_bool();
                assert!(eaten, "TestKeyDown 說吃，OnKeyDown 也要吃 vk={vk:#x}");
            } else if let Some(ch) = host_char {
                self.host.push(ch);
            }
            tested
        }

        fn press_char(&mut self, ch: char) -> bool {
            let vk = ch.to_ascii_uppercase() as u32;
            let scan = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) } as isize;
            self.press_vk(vk, scan, Some(ch))
        }

        fn type_chars(&mut self, text: &str) {
            for ch in text.chars() {
                assert!(self.press_char(ch), "{ch:?} 要進引擎");
            }
        }

        /// 單按 Caps Lock：外殼只讓系統處理，不得碰目前的組字。
        fn tap_caps_lock(&mut self) {
            let (w, l) = (WPARAM(VK_CAPITAL.0 as usize), LPARAM(1 | (0x3a << 16)));
            let tested = unsafe { self.sink.OnTestKeyDown(&self.ctx, w, l) }
                .unwrap()
                .as_bool();
            if tested {
                let _ = unsafe { self.sink.OnKeyDown(&self.ctx, w, l) }.unwrap();
            }
            let up = LPARAM(1 | (0x3a << 16) | 1 << 30 | 1 << 31);
            let _ = unsafe { self.sink.OnKeyUp(&self.ctx, w, up) }.unwrap();
        }

        /// 單按右 Shift：keydown 只讓外殼觀察，keyup 觸發模式切換。
        fn tap_right_shift(&mut self) {
            let (w, l) = (WPARAM(VK_RSHIFT.0 as usize), LPARAM(1 | (0x36 << 16)));
            let _ = unsafe { self.sink.OnTestKeyDown(&self.ctx, w, l) }.unwrap();
            let (w, l) = (
                WPARAM(VK_RSHIFT.0 as usize),
                LPARAM(1 | (0x36 << 16) | 1 << 30 | 1 << 31),
            );
            let _ = unsafe { self.sink.OnTestKeyUp(&self.ctx, w, l) }.unwrap();
        }

        fn preedit(&self) -> String {
            self.doc.composing_text()
        }
    }

    /// 1. 無組字、中文、`a` → engine Key "a"。
    #[test]
    fn chinese_idle_a_sends_key() {
        plain_keyboard();
        let mut rig = Rig::new(false);
        assert!(rig.press_char('a'));
        assert_eq!(rig.sent(), vec!["key".to_string()]);
        assert_eq!(rig.preedit(), "a");
    }

    /// 2. 無組字、英文、`a` → 0 請求，宿主收到 "a"。
    #[test]
    fn english_idle_a_passes_to_host() {
        plain_keyboard();
        let mut rig = Rig::new(true);
        assert!(!rig.press_char('a'), "英文沒有組字要交還宿主");
        assert_eq!(rig.host, "a");
        assert!(rig.sent().is_empty(), "不得送引擎");
        assert!(!rig.composing(), "不建組字");
    }

    /// 3. 無組字、英文、`@` → 0 請求，宿主收到 "@"。
    #[test]
    fn english_idle_at_passes_to_host() {
        plain_keyboard();
        let mut rig = Rig::new(true);
        hold_shift(true);
        assert!(
            !rig.press_vk(0x32, 0x03, Some('@')),
            "英文沒有組字要交還宿主"
        );
        hold_shift(false);
        assert_eq!(rig.host, "@");
        assert!(rig.sent().is_empty(), "不得送引擎");
    }

    /// 4. 組字 `hellp`、中文、`@` → 引擎接成字面，preedit `hellp@`。
    #[test]
    fn chinese_composing_at_is_appended_as_literal() {
        plain_keyboard();
        let mut rig = Rig::new(false);
        rig.type_chars("hellp");
        hold_shift(true);
        assert!(
            rig.press_vk(0x32, 0x03, Some('@')),
            "中文組字中的 @ 要進引擎"
        );
        hold_shift(false);
        assert_eq!(rig.preedit(), "hellp@");
        assert_eq!(rig.sent().last().map(String::as_str), Some("key"));
    }

    /// 5. 組字 `hellp`、英文、`a` → Literal "a"，preedit `hellpa`。
    #[test]
    fn english_composing_a_sends_literal() {
        plain_keyboard();
        let mut rig = Rig::new(false);
        rig.type_chars("hellp");
        rig.tap_right_shift();
        assert!(rig.english(), "Shift 切到英文");
        let before = rig.sent().len();
        assert!(rig.press_char('a'));
        assert_eq!(rig.sent().last().map(String::as_str), Some("literal"));
        assert_eq!(rig.sent().len(), before + 1, "只送一個 Literal");
        assert_eq!(rig.preedit(), "hellpa");
        assert!(rig.composing(), "切英文不得終止組字");
    }

    /// 6. 組字 `hellp`、英文、Shift+A → Literal "A"，preedit `hellpA`。
    #[test]
    fn english_composing_shift_a_keeps_uppercase_literal() {
        plain_keyboard();
        let mut rig = Rig::new(false);
        rig.type_chars("hellp");
        rig.tap_right_shift();
        hold_shift(true);
        assert!(rig.press_char('A'));
        hold_shift(false);
        assert_eq!(rig.sent().last().map(String::as_str), Some("literal"));
        assert_eq!(rig.preedit(), "hellpA");
    }

    /// 7. 組字 `ㄨㄛ`、英文、`a` → Literal "a"，preedit `ㄨㄛa`。
    #[test]
    fn english_composing_after_zhuyin_appends_literal() {
        plain_keyboard();
        let mut rig = Rig::new(false);
        rig.type_chars("ji");
        assert_eq!(rig.preedit(), "ㄨㄛ");
        rig.tap_right_shift();
        assert!(rig.press_char('a'));
        assert_eq!(rig.sent().last().map(String::as_str), Some("literal"));
        assert_eq!(rig.preedit(), "ㄨㄛa");
    }

    /// 組字 `hellp`、英文、Space → Literal " "，preedit 追加空白。
    #[test]
    fn english_composing_space_sends_literal_space() {
        plain_keyboard();
        let mut rig = Rig::new(false);
        rig.type_chars("hellp");
        rig.tap_right_shift();
        assert!(rig.english());
        let before = rig.sent().len();
        assert!(rig.press_vk(0x20, 0x39, Some(' ')), "空白要進引擎");
        assert_eq!(rig.sent().last().map(String::as_str), Some("literal"));
        assert_eq!(rig.sent().len(), before + 1, "只送一個 Literal");
        assert_eq!(rig.preedit(), "hellp ", "preedit 要追加空白");
    }

    /// 組字 `hellp`、英文、Shift+Space → 同一個 Literal " "。
    #[test]
    fn english_composing_shift_space_sends_literal_space() {
        plain_keyboard();
        let mut rig = Rig::new(false);
        rig.type_chars("hellp");
        rig.tap_right_shift();
        hold_shift(true);
        assert!(rig.press_vk(0x20, 0x39, Some(' ')), "Shift+Space 要進引擎");
        hold_shift(false);
        assert_eq!(rig.sent().last().map(String::as_str), Some("literal"));
        assert_eq!(rig.preedit(), "hellp ", "Shift+Space 也是字面空白");
    }

    /// 英文沒有組字：Space 也要交還宿主，0 請求。
    #[test]
    fn english_idle_space_passes_to_host() {
        plain_keyboard();
        let mut rig = Rig::new(true);
        assert!(!rig.press_vk(0x20, 0x39, Some(' ')), "沒有組字要交還宿主");
        assert_eq!(rig.host, " ");
        assert!(rig.sent().is_empty(), "不得送引擎");
    }

    /// Literal 回 handled=false：只送一筆 literal，不再送 Key，且吃掉這一鍵。
    #[test]
    fn english_composing_literal_failure_has_no_second_key() {
        plain_keyboard();
        let mut rig = Rig::with_session(support::failing_literal_session(), false);
        rig.type_chars("hellp");
        rig.tap_right_shift();
        let before = rig.sent().len();
        assert!(rig.press_char('a'), "Literal 失敗也要吃掉這一鍵");
        assert_eq!(rig.sent().len(), before + 1, "不能再送第二筆 Key");
        assert_eq!(rig.sent().last().map(String::as_str), Some("literal"));
    }

    /// 選單開著按 `1`：走 Key 讓 engine 選第一項，不是字面 `1`。
    #[test]
    fn english_menu_open_digit_selects_via_key() {
        plain_keyboard();
        let mut rig = Rig::new(false);
        rig.type_chars("hellp");
        rig.tap_right_shift();
        rig.set_navigation(true, true);
        let before = rig.sent().len();
        assert!(rig.press_char('1'), "選單選字要進引擎");
        assert_eq!(rig.sent().last().map(String::as_str), Some("key"));
        assert_eq!(rig.sent().len(), before + 1, "只送一個 Key");
    }

    /// 候選窗開著（navigating）按 `1`：同樣走 Key 選字。
    #[test]
    fn english_candidate_window_digit_selects_via_key() {
        plain_keyboard();
        let mut rig = Rig::new(false);
        rig.type_chars("hellp");
        rig.tap_right_shift();
        rig.set_navigation(false, true);
        let before = rig.sent().len();
        assert!(rig.press_char('1'), "候選窗選字要進引擎");
        assert_eq!(rig.sent().last().map(String::as_str), Some("key"));
        assert_eq!(rig.sent().len(), before + 1, "只送一個 Key");
    }

    /// 8. 組字中 Shift → english=true，preedit／游標不動，0 請求。
    #[test]
    fn shift_while_composing_only_flips_english() {
        plain_keyboard();
        let mut rig = Rig::new(false);
        rig.type_chars("hellp");
        let before = rig.sent().len();
        let (text, cursor) = {
            let st = rig.service.state.borrow();
            (
                st.session.english_state.text(),
                st.session.english_state.cursor_utf16,
            )
        };
        rig.tap_right_shift();
        assert!(rig.english(), "Shift 只切 english");
        assert_eq!(rig.sent().len(), before, "Shift 不得送引擎");
        assert_eq!(rig.preedit(), "hellp", "preedit 不動");
        let st = rig.service.state.borrow();
        assert_eq!(st.session.english_state.text(), text, "組字內容不動");
        assert_eq!(st.session.english_state.cursor_utf16, cursor, "游標不動");
        assert!(st.composing, "組字仍在");
    }

    /// 9. 承 8，Enter → 走原本 Key 路徑 commit。
    #[test]
    fn enter_after_switch_uses_key_commit() {
        plain_keyboard();
        let mut rig = Rig::new(false);
        rig.type_chars("hellp");
        rig.tap_right_shift();
        assert!(rig.press_vk(VK_RETURN, 0x1c, None), "Enter 要進引擎");
        assert_eq!(rig.sent().last().map(String::as_str), Some("key"));
        assert!(!rig.composing(), "Enter 要結束組字");
        assert_eq!(rig.doc.committed(), "前hellp");
    }

    /// 10. 承 8，Backspace → 原本刪除路徑。
    #[test]
    fn backspace_after_switch_uses_key_delete() {
        plain_keyboard();
        let mut rig = Rig::new(false);
        rig.type_chars("hellp");
        rig.tap_right_shift();
        assert!(rig.press_vk(VK_BACKSPACE, 0x0e, None), "Backspace 要進引擎");
        assert_eq!(rig.sent().last().map(String::as_str), Some("key"));
        assert!(rig.composing(), "Backspace 不結束組字");
        assert_eq!(rig.preedit(), "hell");
    }

    /// 11. 承 8，切回中文 → english=false，preedit 不動。
    #[test]
    fn switch_back_to_chinese_keeps_composition() {
        plain_keyboard();
        let mut rig = Rig::new(false);
        rig.type_chars("hellp");
        rig.tap_right_shift();
        assert!(rig.english());
        let before = rig.sent().len();
        rig.tap_right_shift();
        assert!(!rig.english(), "再按一次 Shift 切回中文");
        assert_eq!(rig.sent().len(), before, "切換不得送引擎");
        assert_eq!(rig.preedit(), "hellp", "preedit 不動");
        assert!(rig.composing(), "組字仍在");
    }

    /// 12. 無組字、Shift 後打 `hello` → 5 鍵 0 請求。
    #[test]
    fn english_after_shift_types_hello_without_requests() {
        plain_keyboard();
        let mut rig = Rig::new(false);
        rig.tap_right_shift();
        assert!(rig.english());
        for ch in "hello".chars() {
            assert!(!rig.press_char(ch), "{ch:?} 要交還宿主");
        }
        assert_eq!(rig.host, "hello");
        assert!(rig.sent().is_empty(), "5 鍵都不得送引擎");
        assert!(!rig.composing(), "英文不建組字");
    }

    /// 組字中按 Caps Lock 只由系統翻轉鎖定狀態，不能送出 present／commit／
    /// notice，也不能改動 preedit 或結束組字。
    #[test]
    fn caps_lock_key_does_not_touch_composition_or_send_requests() {
        plain_keyboard();
        let mut rig = Rig::new(false);
        rig.type_chars("hellp");
        let sent = rig.sent();
        let preedit = rig.preedit();
        let host = rig.host.clone();

        hold_caps(true);
        rig.tap_caps_lock();
        hold_caps(false);
        plain_keyboard();

        assert_eq!(rig.sent(), sent, "Caps Lock 不得送 present／commit／notice");
        assert_eq!(rig.preedit(), preedit, "Caps Lock 不得改動 preedit");
        assert_eq!(rig.host, host, "Caps Lock 不是字面輸入");
        assert!(rig.composing(), "Caps Lock 不得結束組字");
    }

    /// Caps Lock 開著時，字母路由用正規化後的小寫鍵，引擎才會開始組字。
    #[test]
    fn caps_lock_chinese_letters_route_as_lowercase() {
        plain_keyboard();
        hold_caps(true);
        let mut rig = Rig::new(false);
        rig.type_chars("su3cl3");
        hold_caps(false);
        plain_keyboard();
        assert_eq!(rig.preedit(), "你好");
        assert_eq!(rig.sent(), vec!["key".to_string(); 6]);
    }

    #[test]
    fn caps_lock_chinese_w96j0_routes_as_taiwan() {
        plain_keyboard();
        hold_caps(true);
        let mut rig = Rig::new(false);
        rig.type_chars("w96j0");
        hold_caps(false);
        plain_keyboard();
        assert_eq!(rig.preedit(), "台灣");
    }

    /// Caps 只改字母字面；Caps＋Shift 是小寫，數字與標點不受影響。
    #[test]
    fn caps_lock_changes_only_letter_literal_case() {
        plain_keyboard();
        hold_caps(true);
        let service = ComObject::new(TextService::new());
        {
            let mut st = service.state.borrow_mut();
            st.session = support::session();
            let a_scan = unsafe { MapVirtualKeyW(0x41, MAPVK_VK_TO_VSC) } as isize;
            assert_eq!(keys::printable(0x41, a_scan, false).as_deref(), Some("A"));
            assert_eq!(
                st.styled_key(0x41, a_scan, false, false).as_deref(),
                Some("a"),
                "Caps 的字母路由要用小寫"
            );
            hold_shift(true);
            assert_eq!(
                keys::printable(0x41, a_scan, false).as_deref(),
                Some("a"),
                "Caps＋Shift 是小寫"
            );
            assert_eq!(
                st.styled_key(0x41, a_scan, false, false).as_deref(),
                Some("a")
            );
            hold_shift(false);
            let one_scan = unsafe { MapVirtualKeyW(0x31, MAPVK_VK_TO_VSC) } as isize;
            assert_eq!(keys::printable(0x31, one_scan, false).as_deref(), Some("1"));
            assert_eq!(
                st.styled_key(0x31, one_scan, false, false).as_deref(),
                Some("1")
            );
            let comma_scan = unsafe { MapVirtualKeyW(0xBC, MAPVK_VK_TO_VSC) } as isize;
            assert_eq!(
                keys::printable(0xBC, comma_scan, false).as_deref(),
                Some(",")
            );
            assert_eq!(
                st.styled_key(0xBC, comma_scan, false, false).as_deref(),
                Some(",")
            );
        }
        hold_caps(false);
        plain_keyboard();
    }
}

/// panic 降級與重設的單元測試；不需要宿主，也不碰引擎。
#[cfg(all(test, windows))]
mod panic_guard_tests {
    use super::*;

    /// 按鍵 panic 要回 `Ok(FALSE)`，而且組字狀態要歸零。
    #[test]
    fn key_panic_returns_false_and_resets() {
        let service = ComObject::new(TextService::new());
        {
            let mut st = service.state.borrow_mut();
            st.composing = true;
            st.eaten.insert(0x41);
            st.pending_repost = Some(PendingRepost {
                vk: 0x41,
                lparam: 0,
                key_down_seen: false,
            });
        }
        let r = service.guarded_key("OnKeyDown", None, || panic!("裝的"));
        assert!(!r.unwrap().as_bool(), "panic 要回 Ok(FALSE)");
        let st = service.state.borrow();
        assert!(!st.composing, "重設後不能再組字");
        assert!(st.composition.is_none(), "組字物件要清掉");
        assert!(st.pending_repost.is_none(), "重排等待要清掉");
        assert!(st.eaten.is_empty(), "吃掉記錄要清掉");
    }

    /// 沒 panic 時按鍵邊界原樣回傳，不會誤觸重設。
    #[test]
    fn key_without_panic_passes_value_through() {
        let service = ComObject::new(TextService::new());
        service.state.borrow_mut().composing = true;
        let r = service.guarded_key("OnKeyDown", None, || Ok(true.into()));
        assert!(r.unwrap().as_bool());
        assert!(service.state.borrow().composing, "沒 panic 不能清掉組字");
    }

    /// `state` 被借用時重設只能留下旗標，由下一次按鍵消費。
    #[test]
    fn needs_reset_consumed_by_next_key() {
        let service = ComObject::new(TextService::new());
        let held = service.state.borrow_mut();
        service.reset_after_panic(None);
        assert!(service.needs_reset.get(), "借不到 state 要留下旗標");
        drop(held);
        service.consume_needs_reset(None);
        assert!(!service.needs_reset.get(), "下一次按鍵要消費掉旗標");
    }
}

// T8a host 測試：要注入 TextService 的私有 State 與假文件，必須是 service 的子模組。
#[cfg(all(test, windows))]
#[path = "t8a_support.rs"]
mod support;

#[cfg(all(test, windows))]
#[path = "focus_host_tests.rs"]
mod focus_host_tests;

#[cfg(all(test, windows))]
#[path = "imm32_order_tests.rs"]
mod imm32_order_tests;
