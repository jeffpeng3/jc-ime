//! Edit session：在 TSF 的寫入權限內，依快照更新組字區、提交文字、放置游標。
//!
//! 移植自 0c `platforms/windows/tsf/src/edit.rs`（main `8f443af`）的生產碼。
//! 提交語意是「所見即所得」：提交時送出的就是畫面上的組字（0c CLAUDE.md）。
//!
//! 與 jc-tsf 舊 `composition.rs` 的關鍵差異（Enter double-commit 根因）：
//!
//! 1. `range.SetText(ec, 0, &commit)` 直接取代 composition range，不是先 EndComposition 再
//!    InsertText；舊做法會把 preedit 留在文件裡，再插入 commit 得到「わたしわ私は」。
//! 2. `holds_commit` 讀回確認：SetText 回錯但已寫入時不重複插入（0c #88）。
//! 3. `no_text()` 指向真的 NUL，避免 host 讀 `&[]` 的懸空指標崩潰（0c #210）。
//! 4. Collapse 到尾端放游標，最後才 `EndComposition`。
//! 5. `committed` 與 `Ending` 兩個旗標防止重入與重寫。

use std::cell::{Cell, RefCell};
use std::mem::ManuallyDrop;
use std::rc::Rc;

use windows::Win32::Foundation::RECT;
use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_I4};
use windows::Win32::UI::TextServices::{
    ITfComposition, ITfCompositionSink, ITfContext, ITfContextComposition, ITfEditSession,
    ITfEditSession_Impl, ITfInsertAtSelection, ITfRange, GUID_PROP_ATTRIBUTE, TF_AE_NONE,
    TF_ANCHOR_END, TF_ANCHOR_START, TF_DEFAULT_SELECTION, TF_ES_ASYNCDONTCARE, TF_ES_READ,
    TF_ES_READWRITE, TF_ES_SYNC, TF_IAS_QUERYONLY, TF_SELECTION, TF_SELECTIONSTYLE,
};
use windows_core::{implement, ComObject, Interface, HRESULT};

use crate::session::Snapshot;

/// adapter spike 不帶檔案 I/O：保留呼叫點，不產生輸出。T6 再接到 `logging`。
macro_rules! zlog {
    ($($arg:tt)*) => {{ let _ = format_args!($($arg)*); }};
}
macro_rules! zdebug {
    ($($arg:tt)*) => {{ let _ = format_args!($($arg)*); }};
}

/// COM 物件計數。新 crate 還沒有 DLL 生命週期管理，T6 接到 `lib.rs` 的 LIVE_OBJECTS。
fn object_created() {}
fn object_destroyed() {}

/// 攔截 panic：COM 邊界不得 unwind。與 0c `lib.rs::guarded` 行為相同。
fn guarded<T, F: FnOnce() -> windows_core::Result<T>>(what: &str, f: F) -> windows_core::Result<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(_) => {
            zlog!("panic 已攔截：{}", what);
            Err(windows::Win32::Foundation::E_FAIL.into())
        }
    }
}

// T7b：顯示屬性的完整定義（Kind／KINDS／StyledRange／style_ranges、GUID 註冊、
// GetGUID／GetDescription／GetAttributeInfo 與 IEnumTfDisplayAttributeInfo）已抽出到
// `crate::displayattr`。T5 內嵌的最小版移除，edit session 的寫入順序保持不變。
use crate::displayattr::{style_ranges, StyledRange, KINDS};

static NUL: [u16; 1] = [0];

/// 傳給 TSF 的「沒有文字」：長度 0、但指標真的指向一個 NUL。Rust 的 `&[]` 是非空的懸空指標（u16 是 0x2），windows-rs 會
/// 原樣傳下去，msctf 再原樣交給宿主的文字儲存；宿主照 C++ 的習慣先讀 *pchText 就當掉。《二之國：交錯世界》（UE4）第一個
/// 組字鍵的 InsertTextAtSelection 就這樣崩潰：畫面凍結、CrashSight 上傳後遊戲被關掉（0c #210）。
fn no_text() -> &'static [u16] {
    &NUL[..0]
}

/// 同步／非同步 edit session 共用的寫入物件。
#[implement(ITfEditSession)]
pub struct Apply {
    ctx: ITfContext,
    sink: ITfCompositionSink,
    composition: RefCell<Option<ITfComposition>>,
    commit: Vec<u16>,
    preedit: Vec<u16>,
    cursor: usize, // UTF-16 單位
    attr_atoms: [u32; KINDS.len()],
    styles: Vec<StyledRange>,
    caret: RefCell<Located>, // 游標的螢幕矩形（給候選窗定位）與查不到的原因
    /// 沒有組字時也要查游標矩形（這一鍵要顯示候選窗或提示）。
    want_caret: bool,
    ending: Rc<Cell<bool>>,
    /// 進行到哪一步：失敗時記進 tsf.log（只記步驟與 HRESULT，不記文字）。
    step: Cell<&'static str>,
    /// 已經對文件寫過提交文字（SetText 成功，或回報失敗但讀回確認已寫入）。之後不得再寫一次，
    /// 否則宿主會看到同一段字插入兩次（0c #88）。
    committed: Cell<bool>,
    /// DoEditSession 從頭到尾成功跑完：宿主回報失敗也照成功處理。
    done: Cell<bool>,
}

impl Drop for Apply {
    fn drop(&mut self) {
        object_destroyed();
    }
}

/// `apply` 的結果：套用後仍在進行的組字與游標矩形。
pub struct Applied {
    /// 同步 edit session 失敗時：只是更新組字則是呼叫前的組字物件；有提交則為 None（已改排非同步）；
    /// 提交文字已寫入後才失敗則是當時剩下的組字（不重排）。
    pub composition: Option<ITfComposition>,
    /// 游標矩形或查不到的原因。
    pub caret: Located,
}

/// 宿主為什麼給不出游標矩形（記進 tsf.log，不含文字）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Miss {
    /// 沒有跑到查詢：edit session 沒成功，或這一鍵不顯示候選窗、沒有組字，不必查。
    NotRun,
    /// 按鍵沒有 context（沒有可寫入的文件）。
    NoContext,
    /// 沒有 active view。
    NoView(HRESULT),
    /// 組不出游標所在的範圍。
    Range(HRESULT),
    /// GetTextExt 失敗。Chromium（Edge、Electron、WebView2）在版面還沒從 renderer 回來時回
    /// TS_E_NOLAYOUT，之後以 OnLayoutChange 通知。
    Failed(HRESULT),
    /// GetTextExt 成功但矩形是空的。
    Empty(RECT),
}

impl std::fmt::Display for Miss {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Miss::NotRun => write!(f, "edit session 沒有完成"),
            Miss::NoContext => write!(f, "沒有 context"),
            Miss::NoView(hr) => write!(f, "GetActiveView {}", hr),
            Miss::Range(hr) => write!(f, "游標範圍 {}", hr),
            Miss::Failed(hr) if *hr == windows::Win32::UI::TextServices::TS_E_NOLAYOUT => {
                write!(f, "GetTextExt TS_E_NOLAYOUT")
            }
            Miss::Failed(hr) => write!(f, "GetTextExt {}", hr),
            Miss::Empty(r) => write!(
                f,
                "GetTextExt 空矩形 ({},{},{},{})",
                r.left, r.top, r.right, r.bottom
            ),
        }
    }
}

impl Miss {
    /// tsf.log 去重用的類別：不含座標與視窗把手，同一種失敗只記一次（HRESULT 種類有限，保留）。
    pub fn class(&self) -> String {
        match self {
            Miss::NotRun => "not-run".into(),
            Miss::NoContext => "no-context".into(),
            Miss::NoView(hr) => format!("no-view {:#x}", hr.0),
            Miss::Range(hr) => format!("range {:#x}", hr.0),
            Miss::Failed(hr) => format!("failed {:#x}", hr.0),
            Miss::Empty(_) => "empty".into(),
        }
    }
}

/// 游標的螢幕矩形；拿不到時是原因。
pub type Located = Result<RECT, Miss>;

/// 範圍在螢幕上的矩形（GetTextExt）。空矩形不算：候選窗會跑到螢幕左上角。
unsafe fn text_ext(ctx: &ITfContext, ec: u32, range: &ITfRange) -> Result<RECT, Miss> {
    let view = ctx.GetActiveView().map_err(|e| Miss::NoView(e.code()))?;
    let mut rc = RECT::default();
    let mut clipped = windows_core::BOOL(0);
    view.GetTextExt(ec, range, &mut rc, &mut clipped)
        .map_err(|e| Miss::Failed(e.code()))?;
    if rc.right > rc.left || rc.bottom > rc.top {
        Ok(rc)
    } else {
        Err(Miss::Empty(rc))
    }
}

/// 組字範圍裡、從起點算 cursor 個 UTF-16 單位的插入點。
unsafe fn composition_caret(
    ec: u32,
    comp: &ITfComposition,
    cursor: usize,
) -> windows_core::Result<ITfRange> {
    // 複製一份再移動，不動到組字本身的範圍。
    let caret = comp.GetRange()?.Clone()?;
    caret.Collapse(ec, TF_ANCHOR_START)?;
    let mut moved = 0i32;
    caret.ShiftEnd(ec, cursor as i32, &mut moved, std::ptr::null())?;
    caret.Collapse(ec, TF_ANCHOR_END)?;
    Ok(caret)
}

/// 目前選取的插入點（空組字也能開 Emoji 選單，這時沒有組字範圍）。
unsafe fn selection_caret(ctx: &ITfContext, ec: u32) -> windows_core::Result<ITfRange> {
    let mut selected = [TF_SELECTION::default()];
    let mut fetched = 0;
    ctx.GetSelection(ec, TF_DEFAULT_SELECTION, &mut selected, &mut fetched)?;
    let range = ManuallyDrop::take(&mut selected[0].range);
    let range =
        range.ok_or_else(|| windows_core::Error::from(windows::Win32::Foundation::E_FAIL))?;
    range.Collapse(ec, TF_ANCHOR_END)?;
    Ok(range)
}

/// 目前選取插入點的螢幕矩形。
unsafe fn selection_rect(ctx: &ITfContext, ec: u32) -> Located {
    let range = selection_caret(ctx, ec).map_err(|e| Miss::Range(e.code()))?;
    text_ext(ctx, ec, &range)
}

/// 游標的螢幕矩形（OnLayoutChange 重查用）：有組字就用組字裡的游標位置；組不出這個範圍（組字已結束）
/// 才改用目前選取的插入點。GetTextExt 失敗不再查選取：組字中選取就是我們放的同一個插入點，結果相同。
pub(crate) unsafe fn locate(
    ctx: &ITfContext,
    ec: u32,
    comp: Option<&ITfComposition>,
    cursor: usize,
) -> Located {
    match comp.map(|comp| composition_caret(ec, comp, cursor)) {
        Some(Ok(range)) => text_ext(ctx, ec, &range),
        _ => selection_rect(ctx, ec),
    }
}

/// 唯讀 edit session：版面晚到（OnLayoutChange）時重查游標矩形。
#[implement(ITfEditSession)]
pub struct Locate {
    ctx: ITfContext,
    composition: Option<ITfComposition>,
    cursor: usize,
    done: Box<dyn Fn(Located)>,
}

impl Drop for Locate {
    fn drop(&mut self) {
        object_destroyed();
    }
}

impl ITfEditSession_Impl for Locate_Impl {
    fn DoEditSession(&self, ec: u32) -> windows_core::Result<()> {
        guarded("Locate", || {
            let located = unsafe { locate(&self.ctx, ec, self.composition.as_ref(), self.cursor) };
            (self.done)(located);
            Ok(())
        })
    }
}

/// 要一個唯讀 edit session 重查游標矩形，查到後呼叫 done（可能同步，也可能稍後才執行）。
// T6 的 service.rs（OnLayoutChange）才會呼叫；先保留移植介面。
#[allow(dead_code)]
pub(crate) fn request_locate(
    ctx: &ITfContext,
    client_id: u32,
    composition: Option<ITfComposition>,
    cursor: usize,
    done: Box<dyn Fn(Located)>,
) -> windows_core::Result<()> {
    object_created();
    let es: ITfEditSession = ComObject::new(Locate {
        ctx: ctx.clone(),
        composition,
        cursor,
        done,
    })
    .into_interface();
    let hr = unsafe { ctx.RequestEditSession(client_id, &es, TF_ES_ASYNCDONTCARE | TF_ES_READ) }?;
    hr.ok()
}

/// 以同步的讀寫 edit session 套用快照；回傳套用後仍在進行的組字（沒有就是 None）與游標矩形。
/// 有組字時每一鍵都查游標矩形（同一次組字的位置退路要跟著游標走）；沒有組字時只有 `want_caret`
/// （這一鍵要顯示候選窗或提示，例如空組字的 Emoji 選單、提交後的提示）才查選取的插入點。
/// 同步不被允許時（例如失焦、停用時不在按鍵回呼內）：有提交文字就改排非同步 edit session，
/// 由它結束組字並寫入文字，回傳 composition=None；只是更新組字則保留原本的組字物件，讓外殼狀態一致。
/// 提交文字一旦寫過就不再改排：同一個 Apply 從頭重跑會在新的組字裡再插入一次（0c #88）。
pub fn apply(
    ctx: &ITfContext,
    client_id: u32,
    composition: Option<ITfComposition>,
    sink: ITfCompositionSink,
    snapshot: &Snapshot,
    attr_atoms: [u32; KINDS.len()],
    cookie: Option<u32>,
    ending: Rc<Cell<bool>>,
    want_caret: bool,
) -> Applied {
    object_created();
    let obj = ComObject::new(Apply {
        ctx: ctx.clone(),
        sink,
        composition: RefCell::new(composition),
        commit: snapshot.commit.encode_utf16().collect(),
        preedit: snapshot.text().encode_utf16().collect(),
        cursor: snapshot.cursor_utf16,
        attr_atoms,
        styles: style_ranges(&snapshot.preedit).unwrap_or_default(),
        caret: RefCell::new(Err(Miss::NotRun)),
        want_caret,
        ending,
        step: Cell::new("not_run"),
        committed: Cell::new(false),
        done: Cell::new(false),
    });
    let es: ITfEditSession = obj.to_interface();
    zdebug!("RequestEditSession sync");
    let requested = match cookie {
        Some(ec) => unsafe {
            es.DoEditSession(ec)
                .map(|_| windows::Win32::Foundation::S_OK)
        },
        None => unsafe { ctx.RequestEditSession(client_id, &es, TF_ES_SYNC | TF_ES_READWRITE) },
    };
    let ok = match requested {
        Ok(hr) if hr.is_ok() => true,
        Ok(hr) => {
            zlog!("edit session 回傳 {:?}（步驟 {}）", hr, obj.step.get());
            false
        }
        Err(e) => {
            zlog!(
                "RequestEditSession 失敗：{}（步驟 {}）",
                e.code(),
                obj.step.get()
            );
            false
        }
    };
    // 有些宿主跑完 edit session 仍回報失敗；文件已經是快照的樣子，不能再跑一次。
    let ok = ok || obj.done.get();
    zdebug!("RequestEditSession done ok={}", ok);
    if ok {
        let composition = obj.composition.borrow().clone();
        let caret = *obj.caret.borrow();
        return Applied { composition, caret };
    }
    if obj.committed.get() {
        // 提交文字已寫進文件後才失敗（游標、結束組字或接著的新組字）：寧可少做後續，也不能重排
        // 再插入一次（0c #88）。剩下的組字物件照實回報，下一鍵再同步。
        zlog!(
            "提交後 edit session 失敗（步驟 {}），不再重寫提交文字",
            obj.step.get()
        );
        let composition = obj.composition.borrow().clone();
        return Applied {
            composition,
            caret: Err(Miss::NotRun),
        };
    }
    if snapshot.commit.is_empty() && !snapshot.preedit.is_empty() {
        let composition = obj.composition.borrow().clone();
        return Applied {
            composition,
            caret: Err(Miss::NotRun),
        };
    }
    // 提交不能丟：改排非同步 edit session（Apply 物件自己帶著組字與文字）。走到這裡表示
    // 提交文字還沒寫過：同步要求被拒（TF_E_SYNCHRONOUS）、在寫入前就失敗，或 SetText 失敗且讀回確認沒寫入。
    match unsafe { ctx.RequestEditSession(client_id, &es, TF_ES_ASYNCDONTCARE | TF_ES_READWRITE) } {
        Ok(hr) if hr.is_ok() => zlog!("提交改排非同步 edit session"),
        Ok(hr) => zlog!("非同步 edit session 回傳 {:?}，提交文字可能遺失", hr),
        Err(e) => zlog!("非同步 RequestEditSession 失敗：{}，提交文字可能遺失", e),
    }
    Applied {
        composition: None,
        caret: Err(Miss::NotRun),
    }
}

/// VT_I4 的 VARIANT（TfGuidAtom）。
fn variant_i4(value: i32) -> VARIANT {
    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                vt: VT_I4,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: VARIANT_0_0_0 { lVal: value },
            }),
        },
    }
}

impl Apply {
    /// 目前的組字；沒有就在插入點開始一個。
    unsafe fn ensure(&self, ec: u32) -> windows_core::Result<ITfComposition> {
        let existing = self.composition.borrow().clone();
        if let Some(c) = existing {
            return Ok(c);
        }
        let insert: ITfInsertAtSelection = self.ctx.cast()?;
        let range = insert.InsertTextAtSelection(ec, TF_IAS_QUERYONLY, no_text())?;
        let contexts: ITfContextComposition = self.ctx.cast()?;
        let comp = contexts.StartComposition(ec, &range, &self.sink)?;
        *self.composition.borrow_mut() = Some(comp.clone());
        Ok(comp)
    }

    /// 把插入點放游標。
    unsafe fn select(
        &self,
        ec: u32,
        range: &windows::Win32::UI::TextServices::ITfRange,
    ) -> windows_core::Result<()> {
        let mut sel = TF_SELECTION {
            range: ManuallyDrop::new(Some(range.clone())),
            style: TF_SELECTIONSTYLE {
                ase: TF_AE_NONE,
                fInterimChar: false.into(),
            },
        };
        let r = self.ctx.SetSelection(ec, std::slice::from_ref(&sel));
        ManuallyDrop::drop(&mut sel.range);
        r
    }

    /// 組字範圍的內容是否正好是提交文字（SetText 回報失敗時確認宿主到底寫了沒）。
    /// 緩衝區多留一格，範圍裡還有其他字就讀得出來、判為不符。
    unsafe fn holds_commit(&self, ec: u32, range: &ITfRange) -> windows_core::Result<bool> {
        let mut buf = vec![0u16; self.commit.len() + 1];
        let mut fetched = 0u32;
        range.GetText(ec, 0, &mut buf, &mut fetched)?;
        Ok(buf.get(..fetched as usize) == Some(&self.commit[..]))
    }

    /// 清掉顯示屬性 EndComposition，並清掉持有的組字物件。
    unsafe fn end(&self, ec: u32, comp: &ITfComposition) -> windows_core::Result<()> {
        if let (Ok(prop), Ok(range)) = (self.ctx.GetProperty(&GUID_PROP_ATTRIBUTE), comp.GetRange())
        {
            let _ = prop.Clear(ec, &range);
        }
        let _ending = Ending::new(self.ending.clone());
        let r = comp.EndComposition(ec);
        *self.composition.borrow_mut() = None;
        r
    }
}

// 即使宿主回呼出錯或 panic，也恢復旗標，讓之後真正的外部取消照常處理。
struct Ending(Rc<Cell<bool>>, bool);
impl Ending {
    fn new(flag: Rc<Cell<bool>>) -> Self {
        let previous = flag.replace(true);
        Self(flag, previous)
    }
}
impl Drop for Ending {
    fn drop(&mut self) {
        self.0.set(self.1);
    }
}

impl ITfEditSession_Impl for Apply_Impl {
    fn DoEditSession(&self, ec: u32) -> windows_core::Result<()> {
        guarded("DoEditSession", || {
            let result = unsafe { self.run(ec) };
            match &result {
                Ok(()) => self.done.set(true),
                Err(e) => zlog!("edit session 失敗：步驟 {} {}", self.step.get(), e.code()),
            }
            result
        })
    }
}

impl Apply {
    unsafe fn run(&self, ec: u32) -> windows_core::Result<()> {
        if !self.commit.is_empty() && !self.committed.get() {
            self.step.set("commit.start");
            let comp = self.ensure(ec)?;
            self.step.set("commit.range");
            let range = comp.GetRange()?;
            self.step.set("commit.text");
            if let Err(e) = range.SetText(ec, 0, &self.commit) {
                // 有些宿主寫入後仍回報失敗：用同一個 cookie 讀回組字範圍，內容就是提交文字才算
                // 已寫入（不能插兩次）；讀不回或不相符就照失敗處理，讓 apply 改排非同步重寫，
                // 提交文字不能遺失（0c #88）。
                match self.holds_commit(ec, &range) {
                    Ok(true) => zlog!("提交 SetText 回報失敗：{}，讀回確認已寫入", e.code()),
                    Ok(false) => {
                        zlog!("提交 SetText 失敗：{}，讀回內容不符，未寫入", e.code());
                        return Err(e);
                    }
                    Err(read) => {
                        zlog!(
                            "提交 SetText 失敗：{}，讀回失敗：{}，當作未寫入",
                            e.code(),
                            read.code()
                        );
                        return Err(e);
                    }
                }
            }
            self.committed.set(true);
            // 文字已在文件裡：放游標或結束組字失敗都只記錄，不讓整個 session 被當成失敗重來。
            self.step.set("commit.caret");
            let placed = range.Clone().and_then(|caret| {
                caret.Collapse(ec, TF_ANCHOR_END)?;
                self.select(ec, &caret)
            });
            if let Err(e) = placed {
                zlog!("提交後放置游標失敗：{}", e.code());
            }
            self.step.set("commit.end");
            if let Err(e) = self.end(ec, &comp) {
                zlog!("提交後結束組字失敗：{}", e.code());
            }
        }
        self.step.set("preedit");
        if !self.preedit.is_empty() {
            let comp = self.ensure(ec)?;
            let range = comp.GetRange()?;
            range.SetText(ec, 0, &self.preedit)?;
            if let Ok(prop) = self.ctx.GetProperty(&GUID_PROP_ATTRIBUTE) {
                // 先清掉上次組字的屬性，避免縮短或換樣式後殘留粗線。
                let _ = prop.Clear(ec, &range);
                for style in &self.styles {
                    let atom = self.attr_atoms[style.kind.index()];
                    if atom == 0 {
                        continue;
                    }
                    let part = range.Clone()?;
                    part.Collapse(ec, TF_ANCHOR_START)?;
                    let mut moved = 0;
                    part.ShiftEnd(ec, style.end, &mut moved, std::ptr::null())?;
                    if moved != style.end {
                        continue;
                    }
                    part.ShiftStart(ec, style.start, &mut moved, std::ptr::null())?;
                    if moved != style.start {
                        continue;
                    }
                    if let Err(e) = prop.SetValue(ec, &part, &variant_i4(atom as i32)) {
                        zlog!("顯示屬性設定失敗：{}", e);
                    }
                }
            }
            let caret = composition_caret(ec, &comp, self.cursor)?;
            self.select(ec, &caret)?;
            // 游標的螢幕座標：候選窗定位用，直接用剛放好的插入點。拿不到就記下原因，由外殼退回其他來源
            // 或等 OnLayoutChange。
            *self.caret.borrow_mut() = text_ext(&self.ctx, ec, &caret);
        } else {
            self.step.set("clear");
            let existing = self.composition.borrow().clone();
            if let Some(comp) = existing {
                let range = comp.GetRange()?;
                range.SetText(ec, 0, no_text())?;
                self.end(ec, &comp)?;
            }
            // 空組字也能開 Emoji 選單、提交後也可能有提示：沒有組字範圍，從目前選取的插入點取得位置。
            if self.want_caret {
                *self.caret.borrow_mut() = selection_rect(&self.ctx, ec);
            }
        }
        Ok(())
    }
}
