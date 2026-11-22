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
//! 2. **english_pending 只對真正的英文緩衝自動 commit**：
//!    [`TextService::commit_english_pending`] 要求 `snapshot.english && snapshot.english_pending`。
//!    T1a 把所有 jc suggestion（含注音補全）映射成 `english_pending`，原样搬 0c 的
//!    `finish(commit)` 會把注音尾綴當英文單字自動送出。
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
    TF_TMAE_SECUREMODE, TF_TMAE_UIELEMENTENABLEDONLY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, MB_OK,
};
use windows_core::{implement, ComObject, IUnknownImpl, Interface, Ref, BOOL, GUID};

use crate::candidate;
use crate::caret;
use crate::cuas::{Cuas, Win32Cuas};
use crate::displayattr::{AttributeEnum, InputAttribute, KINDS};
use crate::edit;
use crate::keys;
use crate::langbar;
use crate::log::{self, zdebug, zlog};
use crate::privacy;
use crate::session::{Session, Snapshot};
use crate::ui::{self, UiCandidate, UiMessage};

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
    Eat(bool),
    Literal(String),
    WordEnd { key: String, typed: String },
}

impl EnglishKey {
    fn eats(&self) -> bool {
        !matches!(self, EnglishKey::Eat(false))
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
    attr_atoms: [u32; KINDS.len()],
    eaten: HashSet<u32>,
    right_shift: keys::ShiftTap,
    left_shift: keys::ShiftTap,
    secure: bool,
    ui_less: bool,
    activate_flags: u32,
    /// 候選窗的訊息視窗（0c 的 CandidateWindow）；收 `WM_SELECT_CANDIDATE` 與延後焦點確認。
    callback: Option<candidate::CandidateWindow>,
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
/// 就能連續問 japanese_key／styled_key／vocab_keys，不會對同一個 RefCell 重入借用。
impl State {
    /// 保留風格需要的 Shift＋Space 與 Caps 字母語意；預判和送鍵共用。
    fn styled_key(&mut self, vk: u32, lparam: isize, vocab: bool, english: bool) -> Option<String> {
        let key = keys::to_key(vk, lparam, vocab, english)?;
        let shift = keys::shift_down();
        let caps = keys::caps_lock_on();
        let shift_space = key == " " && shift && self.session.style_shift_space();
        let caps_lower = caps && self.session.style_caps_lower();
        Some(keys::style_key(key, shift, caps, shift_space, caps_lower))
    }

    fn vocab_keys(&self) -> bool {
        self.session.vocab_keys() && !self.session.english() && !self.session.japanese()
    }

    fn japanese_key(&mut self, vk: u32) -> Option<&'static str> {
        let key = keys::japanese_named(vk)?;
        (self.session.japanese() && self.session.available() && self.session.convert_keys())
            .then_some(key)
    }

    fn ctrl_punct_key(&mut self, vk: u32, lparam: isize) -> Option<&'static str> {
        if self.session.english() || self.session.japanese() {
            return None;
        }
        let key = keys::ctrl_punct(vk, lparam)?;
        (self.session.available() && self.session.ctrl_punctuation(key)).then_some(key)
    }

    fn simplified_hotkey(&mut self, vk: u32) -> bool {
        keys::simplified_hotkey(vk)
            && !self.session.japanese()
            && self.session.available()
            && self.session.simplified_hotkey()
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
    pub fn new() -> TextService {
        log::object_created();
        TextService {
            state: RefCell::new(State {
                thread_mgr: None,
                client_id: 0,
                thread_mgr_cookie: TF_INVALID_COOKIE,
                composition: None,
                ending_composition: Rc::new(Cell::new(false)),
                session: Session::new(),
                cuas: Rc::new(Win32Cuas),
                completing: None,
                pending_repost: None,
                composing: false,
                attr_atoms: [0; KINDS.len()],
                eaten: HashSet::new(),
                right_shift: keys::ShiftTap::default(),
                left_shift: keys::ShiftTap::default(),
                secure: false,
                ui_less: false,
                activate_flags: 0,
                callback: None,
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
        let ctx = self.focused_context();
        let snapshot = self.state.borrow_mut().session.toggle();
        self.present(ctx.as_ref(), &snapshot);
    }

    /// 選單的「輸出簡體字」：請引擎即時切換，組字不動。
    pub(crate) fn set_output_simplified(&self, on: bool) -> bool {
        let ctx = self.focused_context();
        let snapshot = {
            let Ok(mut st) = self.state.try_borrow_mut() else {
                return false;
            };
            if !st.session.available() {
                return false;
            }
            st.session.output_simplified(on)
        };
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
        if keys::is_modifier(vk) {
            return;
        }
        let changed = {
            let mut st = self.state.borrow_mut();
            st.session.available() && st.session.learn_mode(true)
        };
        if changed {
            self.sync_langbar();
        }
    }

    fn sync_caps(&self, ctx: Option<&ITfContext>, vk: u32) {
        let on = keys::caps_lock_on();
        let snapshot = {
            let mut st = self.state.borrow_mut();
            if vk == keys::VK_CAPS_LOCK {
                st.session.refresh_caps_lock_english();
            }
            if !st.session.caps_needs_sync(on) || !st.session.available() {
                return;
            }
            st.session.caps_lock(on)
        };
        zdebug!("Caps Lock {} -> english={}", on, snapshot.english);
        self.present(ctx, &snapshot);
    }

    fn sync_langbar(&self) {
        let st = self.state.borrow();
        if let Some((_, button)) = st.langbar.as_ref() {
            langbar::set_state(
                button,
                st.session.english(),
                st.session.simplified(),
                st.session.japanese(),
                st.session.input_method(),
            );
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
        let show = snapshot.composing()
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
        let message = UiMessage {
            show: show && caret.is_some(),
            x: caret.map(|rect| rect.left).unwrap_or(0),
            y: caret.map(|rect| rect.top).unwrap_or(0),
            w: caret.map(|rect| rect.right - rect.left).unwrap_or(0),
            h: caret.map(|rect| rect.bottom - rect.top).unwrap_or(0),
            candidates: snapshot
                .candidates
                .iter()
                .enumerate()
                .map(|(index, (id, text))| UiCandidate {
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
                })
                .collect(),
            highlight: snapshot.highlight,
            page: snapshot.page + 1,
            pages: snapshot.pages,
            columns: snapshot.columns,
            kind: snapshot.candidate_kind.clone(),
            title: snapshot.candidate_title(),
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
            st.composition = applied.composition;
            st.ui.present(&message, owner, document.clone());
            if !st.composing {
                st.anchor = None;
                st.left_from = None;
            }
            st.shown = match ctx.filter(|_| show) {
                Some(ctx) => Some(Shown {
                    ctx: ctx.clone(),
                    cursor: snapshot.cursor_utf16,
                    owner,
                    document,
                    exact: placed.is_some_and(|(_, source)| source == caret::Source::Exact),
                    requeries: 0,
                    since: Instant::now(),
                    message,
                }),
                None => None,
            };
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
        let placed = caret::choose(
            located.clone(),
            || None, // 本 crate 不查系統插入點：拿不到精確位置就退到上一次組字位置。
            previous,
            || caret::around_field(recent),
        );
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

    /// 新組字的第一鍵之前：游標仍緊接著剛提交的文字，才讓引擎以它作為前文。
    fn confirm_document_context(&self, ctx: Option<&ITfContext>) {
        let (composing, client) = {
            let st = self.state.borrow();
            (st.composing, st.client_id)
        };
        if composing {
            return;
        }
        let Some((id, commit)) = self.state.borrow_mut().session.take_pending_context() else {
            return;
        };
        let expected: Vec<u16> = commit.encode_utf16().collect();
        let confirmed = ctx.is_some_and(|c| crate::context::preceded_by(c, client, &expected));
        zdebug!("document context confirmed={}", confirmed);
        self.state
            .borrow_mut()
            .session
            .confirm_context(id, confirmed);
    }

    /// 硬要求一：直接把按鍵送給核心，絕對不補 Down。
    ///
    /// 0c `service.rs:942-965` 的 `handle_key()` 只做 `session.key(key)` 再 `present`。
    /// 舊 `jc-tsf/src/composition.rs` 的 `needs_selection_prefix()` 會在「單一 1-9 且
    /// candidates > 0」時插入 `KeyCode::Down`，讓注音前綴的數字變成選字鍵；核心 su
    /// 修復後 `su` + `3` 應是聲調鍵，補 Down 會讓它又被 suggestion 取代。
    fn handle_key(&self, ctx: Option<&ITfContext>, key: &str) -> bool {
        let snapshot = self.state.borrow_mut().session.key(key);
        zdebug!(
            "snapshot ok={} handled={} composing={} commit_len={}",
            snapshot.ok,
            snapshot.handled,
            snapshot.composing(),
            snapshot.commit.chars().count()
        );
        self.present(ctx, &snapshot)
    }

    /// 結束目前組字：commit（失焦、停用）或 cancel（應用程式自行終止組字）。
    fn finish(&self, ctx: Option<&ITfContext>, kind: &str) {
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
        let cuas = self.state.borrow().cuas.clone();
        let (ctrl, alt, win) = keys::ctrl_alt_win();
        let eligible = keys::reposts_after_commit(vk, ctrl, alt, win) && !cuas.ime_hotkey(vk);
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

    /// 硬要求二：只有真正的英文緩衝才自動 commit。
    ///
    /// 0c 在 `english_key_policy` 的 1243-1245 與 1287-1288 遇到 `english_pending`
    /// 就 `finish(commit)`；那時 pending 代表真正的英文單字緩衝。T1a 把任何 jc
    /// suggestion（含注音補全）都映射成 `english_pending`，所以這裡加上
    /// `snapshot.english` 的條件，注音／日文路徑不會自動把尾綴送出。
    fn commit_english_pending(&self, ctx: Option<&ITfContext>, snap: &Snapshot) {
        if snap.english && snap.english_pending {
            self.finish(ctx, "commit");
        }
    }

    fn test_simplified_hotkey(&self, vk: u32) -> bool {
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
        st.composition = None;
        st.left_from = None;
        false
    }

    fn english_key_policy(
        &self,
        ctx: Option<&ITfContext>,
        vk: u32,
        lparam: isize,
    ) -> Option<EnglishKey> {
        if keys::is_modifier(vk) && !keys::modifier_down() {
            return None;
        }
        let snap = {
            let mut st = self.state.borrow_mut();
            if !st.session.english() || !st.session.available() {
                return None;
            }
            if !st.composing {
                st.session.handle(json!({"type": "display", "columns": 0}))
            } else {
                st.session.english_state.clone()
            }
        };
        let caps_lower = snap.caps_lock && self.state.borrow_mut().session.style_caps_lower();
        if !snap.ok
            || (!snap.english_autocomplete && !snap.full_width && !caps_lower)
            || snap.composing() && !snap.english_pending
        {
            return None;
        }
        if keys::modifier_down() {
            self.commit_english_pending(ctx, &snap);
            return Some(EnglishKey::Eat(false));
        }
        let key = self.state.borrow_mut().styled_key(vk, lparam, false, true);
        if key.as_deref() == Some("ShiftSpace") {
            return Some(EnglishKey::Eat(true));
        }
        let flags = self.state.borrow().activate_flags;
        if !snap.english_pending && ctx.is_some_and(|c| privacy::english_passthrough(c, flags)) {
            if let Some(text) = keys::caps_lower_literal(caps_lower, key.as_deref()) {
                return Some(EnglishKey::Literal(text));
            }
            log::once("英文模式：IMM32 程式的焦點沒有插入點，不開始英文補字，按鍵直接交給程式");
            return Some(EnglishKey::Eat(false));
        }
        if key.as_ref().is_some_and(|k| {
            snap.english_wants_key(k)
                || caps_lower && k.len() == 1 && k.as_bytes()[0].is_ascii_alphabetic()
        }) {
            return Some(EnglishKey::Eat(true));
        }
        if snap.english_pending && ctx.is_some_and(|c| privacy::imm32_context(c, flags)) {
            if let Some(key) = key.as_deref() {
                if let Some(typed) = keys::imm32_word_end(key) {
                    return Some(EnglishKey::WordEnd {
                        key: key.to_string(),
                        typed,
                    });
                }
            }
        }
        if let Some(key) = key {
            let next = self.state.borrow_mut().session.key(&key);
            if next.ok && (!next.commit.is_empty() || snap.english_pending) {
                self.present(ctx, &next);
            }
            return Some(EnglishKey::Eat(next.handled));
        }
        self.commit_english_pending(ctx, &snap);
        Some(EnglishKey::Eat(false))
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
        if !self.state.borrow().composing {
            return false;
        }
        if self.state.borrow_mut().japanese_key(vk).is_some() {
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
        if !right && !left {
            st.session.bypassed();
        }
    }

    fn shift_released(&self, ctx: Option<&ITfContext>, side: keys::Side) -> bool {
        let toggle = {
            let mut st = self.state.borrow_mut();
            let toggle = st.shift_toggles(side, keys::modifier_down(), keys::now_ms());
            *st.tap(side) = keys::ShiftTap::default();
            toggle
        };
        if toggle {
            let snapshot = self.state.borrow_mut().session.toggle();
            self.present(ctx, &snapshot);
        }
        toggle
    }

    fn wants_key(&self, vk: u32, lparam: isize) -> bool {
        if keys::modifier_down() && self.state.borrow_mut().ctrl_punct_key(vk, lparam).is_some() {
            return true;
        }
        if keys::modifier_down() || keys::is_modifier(vk) {
            return false;
        }
        let mut st = self.state.borrow_mut();
        if !st.session.available() {
            return false;
        }
        if st.japanese_key(vk).is_some() {
            return true;
        }
        let english = st.session.english();
        let japanese = st.session.japanese();
        let style_key = st.styled_key(vk, lparam, false, english);
        if style_key.as_deref() == Some("ShiftSpace") {
            return true;
        }
        if style_key
            .as_ref()
            .is_some_and(|k| st.session.english_state.full_width_extra_key(k))
        {
            return true;
        }
        if st.composing {
            let vocab = st.vocab_keys();
            return keys::composing_wants(
                english,
                st.styled_key(vk, lparam, vocab, english).as_deref(),
            );
        }
        if english {
            return false;
        }
        if japanese {
            if keys::is_keypad(vk) || keys::named(vk).is_some() {
                return false;
            }
            return keys::printable(vk, lparam, false).is_some();
        }
        if keys::is_keypad(vk) {
            return style_key
                .as_deref()
                .is_some_and(|k| st.session.english_state.full_width_keypad(k));
        }
        if keys::named(vk).is_some() {
            return false;
        }
        keys::printable(vk, lparam, false)
            .map(|key| st.session.idle_key(&key))
            .unwrap_or(false)
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
        if let Some(window) = self.state.borrow().callback.as_ref() {
            if matches!(step, FocusStep::Wait(_)) {
                window.settle_focus_later();
            } else {
                window.cancel_settle_focus();
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

    fn end_word_with(&self, ctx: Option<&ITfContext>, vk: u32, key: &str, typed: String) -> bool {
        let shown = self.state.borrow().session.english_state.text();
        let mut snapshot = self.state.borrow_mut().session.key(key);
        if !snapshot.ok {
            return self.type_text(ctx, vk, shown + &typed);
        }
        if !snapshot.handled {
            snapshot.commit.push_str(&typed);
            snapshot.handled = true;
        }
        let eaten = self.present(ctx, &snapshot);
        if eaten {
            self.state.borrow_mut().eaten.insert(vk);
        }
        eaten
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
            self.activated.set(true);
            zlog!("activate 開始 exe={} tid={}", log::exe_name(), tid);
            let tm = ptim.ok()?.clone();
            // 候選窗的訊息視窗：收 WM_SELECT_CANDIDATE 與延後的焦點確認。
            let callback = candidate::CandidateWindow::new(self).ok();
            let mut opened = None;
            unsafe {
                let keystrokes: ITfKeystrokeMgr = tm.cast()?;
                let key_sink: ITfKeyEventSink = self.to_interface();
                keystrokes.AdviseKeyEventSink(tid, &key_sink, true)?;
                let source: ITfSource = tm.cast()?;
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
                let japanese = st.session.japanese();
                let input_method = st.session.input_method().to_string();
                drop(st);
                // 輸入指示器的中／英按鈕與選單；按鈕持有本服務的參考，Deactivate 時移除以解開循環。
                let tm = self.state.borrow().thread_mgr.clone();
                if let Some(Ok(mgr)) = tm.map(|tm| tm.cast::<ITfLangBarItemMgr>()) {
                    let service = self.to_object();
                    let simplify = self.to_object();
                    let recheck = self.to_object();
                    let button = langbar::LangBarButton::create(
                        english,
                        simplified,
                        japanese,
                        input_method,
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
            zlog!("activate 完成 exe={} tid={}", log::exe_name(), tid);
            Ok(())
        })
    }

    fn Deactivate(&self) -> windows_core::Result<()> {
        crate::guarded("Deactivate", || {
            if !self.activated.get() {
                log::log("Deactivate：沒有已啟用的服務");
                return Ok(());
            }
            self.activated.set(false);
            let ctx = self.focused_context();
            self.finish(ctx.as_ref(), "commit");
            self.forget_candidates();
            // 只清 UI／composition／callback，**不清 session，也不拆它的 IPC client**。
            // 這樣第二次 Activate 不會從零重建 worker，也不會踩到 TF_E_NOLOCK 後把 worker 拆掉。
            let (tm, tid, cookie) = {
                let mut st = self.state.borrow_mut();
                st.callback = None;
                st.ui.hide();
                st.ui.set_elements(None);
                st.completing = None;
                st.pending_repost = None;
                st.composition = None;
                st.composing = false;
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
                "activate flags={:#x} comless={}",
                flags,
                flags & TF_TMAE_COMLESS != 0
            );
            self.Activate(ptim, tid)
        })
    }
}

impl ITfKeyEventSink_Impl for TextService_Impl {
    fn OnSetFocus(&self, foreground: BOOL) -> windows_core::Result<()> {
        crate::guarded("OnSetFocus", || {
            if !foreground.as_bool() {
                {
                    let mut st = self.state.borrow_mut();
                    st.right_shift = keys::ShiftTap::default();
                    st.left_shift = keys::ShiftTap::default();
                    st.selection = None;
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
        crate::guarded("OnTestKeyDown", || {
            {
                let mut st = self.state.borrow_mut();
                st.hotkey_tested.forget();
                st.key_tested.forget();
            }
            // 重排回來的鍵（#66）：同一個 (vk, lParam) 再出現就是它回來了。清掉等待、不吃，交給程式。
            {
                let mut st = self.state.borrow_mut();
                if let Some(pending) = st.pending_repost.take() {
                    if pending.vk == wparam.0 as u32 && pending.lparam == lparam.0 {
                        zdebug!("OnTestKeyDown vk={} 重排回來 -> 不吃", wparam.0);
                        return Ok(false.into());
                    }
                    zdebug!("OnTestKeyDown vk={} 重排的鍵沒回來，清掉等待", pending.vk);
                }
            }
            if !self.allowed(pic.as_ref(), "OnTestKeyDown", wparam.0 as u32) {
                return Ok(false.into());
            }
            let vk = wparam.0 as u32;
            self.observe_shift(vk, lparam.0);
            if self.test_simplified_hotkey(vk) {
                return Ok(true.into());
            }
            if self.commit_before_host_key(pic.as_ref(), vk, lparam.0, true) {
                zdebug!("OnTestKeyDown vk={} -> 攔下重排", vk);
                return Ok(true.into());
            }
            self.learn_mode(vk);
            self.sync_caps(pic.as_ref(), vk);
            let wants = match self.english_key_policy(pic.as_ref(), vk, lparam.0) {
                Some(policy) => policy.eats(),
                None => self.wants_key(vk, lparam.0),
            };
            self.state.borrow_mut().key_tested.record(vk, wants);
            Ok(wants.into())
        })
    }

    fn OnTestKeyUp(
        &self,
        pic: Ref<ITfContext>,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> windows_core::Result<BOOL> {
        crate::guarded("OnTestKeyUp", || {
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
            if vk == keys::VK_CAPS_LOCK {
                self.sync_caps(pic.as_ref(), 0);
            }
            if let Some(side) = keys::shift_side(vk, lparam.0) {
                self.shift_released(pic.as_ref(), side);
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
        crate::guarded("OnKeyDown", || {
            let vk = wparam.0 as u32;
            // TestKeyDown 已經送出組字並重排：這一次的 OnKeyDown 只吃下，不再判斷。
            {
                let mut st = self.state.borrow_mut();
                if let Some(pending) = st.pending_repost.as_mut() {
                    if pending.vk == vk && !pending.key_down_seen {
                        pending.key_down_seen = true;
                        return Ok(true.into());
                    }
                }
            }
            let (tested, key_tested) = {
                let mut st = self.state.borrow_mut();
                (st.hotkey_tested.take(vk), st.key_tested.take(vk))
            };
            if !self.allowed(pic.as_ref(), "OnKeyDown", vk) {
                return Ok(false.into());
            }
            self.observe_shift(vk, lparam.0);
            if self.key_simplified_hotkey(vk, tested) {
                let on = !self.state.borrow().session.simplified().unwrap_or(false);
                let switched = self.set_output_simplified(on);
                zdebug!("OnKeyDown vk={} 切換簡體字 on={} ok={}", vk, on, switched);
                self.state.borrow_mut().eaten.insert(vk);
                return Ok(true.into());
            }
            self.commit_before_host_key(pic.as_ref(), vk, lparam.0, false);
            self.learn_mode(vk);
            self.sync_caps(pic.as_ref(), vk);
            let literal_fallback = key_tested == Some(true) && {
                let st = self.state.borrow();
                !st.composing && !st.session.english()
            };
            let wants = match self.english_key_policy(pic.as_ref(), vk, lparam.0) {
                Some(EnglishKey::Literal(text)) => {
                    return Ok(self.type_text(pic.as_ref(), vk, text).into())
                }
                Some(EnglishKey::WordEnd { key, typed }) => {
                    self.confirm_document_context(pic.as_ref());
                    return Ok(self.end_word_with(pic.as_ref(), vk, &key, typed).into());
                }
                Some(policy) => policy.eats(),
                None => self.wants_key(vk, lparam.0),
            };
            if !wants {
                if literal_fallback {
                    return Ok(self.type_literal(pic.as_ref(), vk, lparam.0).into());
                }
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
            let japanese = self.state.borrow_mut().japanese_key(vk);
            let Some(key) = japanese
                .map(str::to_string)
                .or_else(|| ctrl.map(str::to_string))
                .or_else(|| {
                    self.state
                        .borrow_mut()
                        .styled_key(vk, lparam.0, vocab, english)
                })
            else {
                return Ok(false.into());
            };
            self.confirm_document_context(pic.as_ref());
            let mut eaten = self.handle_key(pic.as_ref(), &key);
            if !eaten && literal_fallback && !self.state.borrow().composing {
                eaten = self.type_literal(pic.as_ref(), vk, lparam.0);
            }
            if eaten {
                self.state.borrow_mut().eaten.insert(vk);
            }
            Ok(eaten.into())
        })
    }

    fn OnKeyUp(
        &self,
        pic: Ref<ITfContext>,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> windows_core::Result<BOOL> {
        crate::guarded("OnKeyUp", || {
            {
                let mut st = self.state.borrow_mut();
                st.hotkey_tested.forget();
                st.key_tested.forget();
            }
            if !self.allowed(pic.as_ref(), "OnKeyUp", wparam.0 as u32) {
                return Ok(false.into());
            }
            if let Some(side) = keys::shift_side(wparam.0 as u32, lparam.0) {
                self.shift_released(pic.as_ref(), side);
                return Ok(false.into());
            }
            Ok(self
                .state
                .borrow_mut()
                .eaten
                .remove(&(wparam.0 as u32))
                .into())
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
