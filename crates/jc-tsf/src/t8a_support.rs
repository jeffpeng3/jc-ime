//! T8a host 測試共用的假宿主：0c `edit.rs` 的假文件／context（T0 的 `Ref<T>` 版）
//! 與不需要 named pipe 的假引擎。只給 T8a 的兩支 host 測試使用。
#![allow(dead_code)]

use super::*;
use std::collections::VecDeque;

use windows::Win32::Foundation::{E_FAIL, E_NOTIMPL, S_OK};
use windows::Win32::System::Com::IDataObject;
use windows::Win32::UI::TextServices::{
    IEnumITfCompositionView, IEnumTfContextViews, IEnumTfProperties, ITfCompositionSink_Impl,
    ITfCompositionView, ITfComposition_Impl, ITfContextComposition, ITfContextComposition_Impl,
    ITfContextView, ITfContextView_Impl, ITfContext_Impl, ITfDocumentMgr, ITfEditSession,
    ITfInsertAtSelection, ITfInsertAtSelection_Impl, ITfProperty, ITfRange, ITfRangeBackup,
    ITfRange_Impl, ITfReadOnlyProperty, TfAnchor, TfGravity, TfShiftDir,
    INSERT_TEXT_AT_SELECTION_FLAGS, TF_ANCHOR_START, TF_CONTEXT_EDIT_CONTEXT_FLAGS, TF_ES_SYNC,
    TF_E_SYNCHRONOUS, TF_HALTCOND, TF_SELECTION, TF_S_ASYNC, TS_E_NOLAYOUT, TS_STATUS,
};
use windows_core::{AsImpl, IUnknown, Ref, BOOL, GUID, HRESULT, PCWSTR, PWSTR};

/// 宿主怎麼處理同步（TF_ES_SYNC）的 edit session。
#[derive(Clone, Copy)]
pub(crate) enum Sync {
    /// 照常執行，回傳 DoEditSession 的結果。
    Run,
    /// 不在按鍵回呼內：不執行，回 TF_E_SYNCHRONOUS。
    Refuse,
    /// 執行了，卻回報失敗（RequestEditSession 本身回錯誤）。
    RunButReport,
}

/// 宿主的版面（GetActiveView／GetTextExt）。
#[derive(Clone, Copy)]
enum Layout {
    /// 沒有 view。
    NoView,
    /// Chromium 剛打的字還沒排版：GetTextExt 回 TS_E_NOLAYOUT。
    NotReady,
    /// GetTextExt 成功但矩形是空的。
    Empty,
    /// 排好了：第 n 個 UTF-16 位置的游標在 x = 100 + 10n。
    Ready,
}

/// 下一次 SetText 怎麼失敗（只作用一次）。
#[derive(Clone, Copy)]
enum SetTextFailure {
    /// 文字寫進去了，卻回報失敗。
    Written,
    /// 沒寫入就回報失敗。
    Dropped,
}

/// 假文件：UTF-16 文字與插入點，以及可注入的失敗。非同步 edit session 先排隊，由測試決定何時執行。
pub(crate) struct Doc {
    text: RefCell<Vec<u16>>,
    caret: Cell<usize>,
    sync: Cell<Sync>,
    fail_end: Cell<bool>,
    fail_select: Cell<bool>,
    fail_set_text: Cell<Option<SetTextFailure>>,
    ended: Cell<u32>,
    /// GetText 被呼叫的次數（讀回確認）。
    reads: Cell<u32>,
    queued: RefCell<VecDeque<ITfEditSession>>,
    layout: Cell<Layout>,
    /// GetTextExt 與 GetSelection 被呼叫的次數（每鍵不做多餘的查詢）。
    extents: Cell<u32>,
    selections: Cell<u32>,
    /// GetStatus 的 dwDynamicFlags（欄位政策看 TF_SD_READONLY）。
    dynamic: Cell<u32>,
    /// 外殼交給宿主的文字指標（InsertTextAtSelection、SetText）；宿主可能真的去讀（#210）。
    text_args: RefCell<Vec<(usize, i32)>>,
    /// GetStatus 的 dwStaticFlags（#260：IMM32 程式經系統相容層的暫時 context 是 TS_SS_TRANSITORY）。
    pub(crate) static_flags: Cell<u32>,
    /// 進行中的組字範圍（StartComposition 到 EndComposition）。
    live: RefCell<Option<Rc<Span>>>,
    /// 指定 view 的視窗（#66）：None 時 GetActiveView 照 `layout`；Some(h) 時一律回一個 view，GetWnd 回 h（可以是 0）。
    pub(crate) view: Cell<Option<windows::Win32::Foundation::HWND>>,
}

impl Doc {
    /// 進行中的組字區文字（沒有組字就是空字串）。
    pub(crate) fn composing_text(&self) -> String {
        let text = self.text.borrow();
        match self.live.borrow().as_ref() {
            Some(span) => String::from_utf16(&text[span.start.get()..span.end.get()]).unwrap(),
            None => String::new(),
        }
    }
    /// 有沒有進行中的組字範圍。
    pub(crate) fn composing(&self) -> bool {
        self.live.borrow().is_some()
    }
    /// 把進行中的組字區文字換成別的內容（測試：組字區與引擎要送出的文字不同時，外殼得先換成送出文字，#66）。
    pub(crate) fn overwrite_composing(&self, new: &str) {
        let span = self.live.borrow().clone().expect("要在組字中");
        let new: Vec<u16> = new.encode_utf16().collect();
        let mut text = self.text.borrow_mut();
        let (s, e) = (span.start.get(), span.end.get());
        text.splice(s..e, new.iter().copied());
        span.end.set(s + new.len());
    }
    /// 結束組字、文字留在文件裡成為已提交（系統相容層替程式完成組字，#66）。
    pub(crate) fn end_composition(&self) {
        *self.live.borrow_mut() = None;
    }
    /// 已提交的文字：整份文件扣掉進行中的組字。IMM32 程式只看得到這部分（系統相容層的結果字串）。
    pub(crate) fn committed(&self) -> String {
        let text = self.text.borrow();
        let (s, e) = match self.live.borrow().as_ref() {
            Some(span) => (span.start.get(), span.end.get()),
            None => (text.len(), text.len()),
        };
        String::from_utf16(&text[..s]).unwrap() + &String::from_utf16(&text[e..]).unwrap()
    }
}

struct Span {
    start: Cell<usize>,
    end: Cell<usize>,
}

fn span(start: usize, end: usize) -> Rc<Span> {
    Rc::new(Span {
        start: Cell::new(start),
        end: Cell::new(end),
    })
}

#[implement(ITfRange)]
struct FakeRange {
    doc: Rc<Doc>,
    span: Rc<Span>,
}

fn range(doc: &Rc<Doc>, span: Rc<Span>) -> ITfRange {
    ComObject::new(FakeRange {
        doc: doc.clone(),
        span,
    })
    .into_interface()
}

impl ITfRange_Impl for FakeRange_Impl {
    fn GetText(
        &self,
        _: u32,
        _: u32,
        out: PWSTR,
        cch: u32,
        fetched: *mut u32,
    ) -> windows_core::Result<()> {
        self.doc.reads.set(self.doc.reads.get() + 1);
        let text = self.doc.text.borrow();
        let part = &text[self.span.start.get()..self.span.end.get()];
        let n = part.len().min(cch as usize);
        unsafe {
            std::ptr::copy_nonoverlapping(part.as_ptr(), out.0, n);
            *fetched = n as u32;
        }
        Ok(())
    }
    fn SetText(&self, _: u32, _: u32, text: &PCWSTR, cch: i32) -> windows_core::Result<()> {
        self.doc.text_args.borrow_mut().push((text.0 as usize, cch));
        let failure = self.doc.fail_set_text.take();
        if let Some(SetTextFailure::Dropped) = failure {
            return Err(E_FAIL.into());
        }
        let new: Vec<u16> = if cch > 0 {
            unsafe { std::slice::from_raw_parts(text.0, cch as usize) }.to_vec()
        } else {
            Vec::new()
        };
        let (s, e) = (self.span.start.get(), self.span.end.get());
        self.doc.text.borrow_mut().splice(s..e, new.iter().copied());
        self.span.end.set(s + new.len());
        match failure {
            Some(SetTextFailure::Written) => Err(E_FAIL.into()),
            _ => Ok(()),
        }
    }
    fn GetFormattedText(&self, _: u32) -> windows_core::Result<IDataObject> {
        Err(E_NOTIMPL.into())
    }
    fn GetEmbedded(
        &self,
        _: u32,
        _: *const GUID,
        _: *const GUID,
    ) -> windows_core::Result<IUnknown> {
        Err(E_NOTIMPL.into())
    }
    fn InsertEmbedded(&self, _: u32, _: u32, _: Ref<IDataObject>) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn ShiftStart(
        &self,
        _: u32,
        cch: i32,
        moved: *mut i32,
        _: *const TF_HALTCOND,
    ) -> windows_core::Result<()> {
        let from = self.span.start.get() as i32;
        let to = (from + cch).clamp(0, self.span.end.get() as i32);
        self.span.start.set(to as usize);
        unsafe { moved.write(to - from) };
        Ok(())
    }
    fn ShiftEnd(
        &self,
        _: u32,
        cch: i32,
        moved: *mut i32,
        _: *const TF_HALTCOND,
    ) -> windows_core::Result<()> {
        let from = self.span.end.get() as i32;
        let to = (from + cch).clamp(
            self.span.start.get() as i32,
            self.doc.text.borrow().len() as i32,
        );
        self.span.end.set(to as usize);
        unsafe { moved.write(to - from) };
        Ok(())
    }
    fn ShiftStartToRange(&self, _: u32, _: Ref<ITfRange>, _: TfAnchor) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn ShiftEndToRange(&self, _: u32, _: Ref<ITfRange>, _: TfAnchor) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn ShiftStartRegion(&self, _: u32, _: TfShiftDir) -> windows_core::Result<BOOL> {
        Err(E_NOTIMPL.into())
    }
    fn ShiftEndRegion(&self, _: u32, _: TfShiftDir) -> windows_core::Result<BOOL> {
        Err(E_NOTIMPL.into())
    }
    fn IsEmpty(&self, _: u32) -> windows_core::Result<BOOL> {
        Ok((self.span.start.get() == self.span.end.get()).into())
    }
    fn Collapse(&self, _: u32, anchor: TfAnchor) -> windows_core::Result<()> {
        if anchor == TF_ANCHOR_START {
            self.span.end.set(self.span.start.get());
        } else {
            self.span.start.set(self.span.end.get());
        }
        Ok(())
    }
    fn IsEqualStart(&self, _: u32, _: Ref<ITfRange>, _: TfAnchor) -> windows_core::Result<BOOL> {
        Err(E_NOTIMPL.into())
    }
    fn IsEqualEnd(&self, _: u32, _: Ref<ITfRange>, _: TfAnchor) -> windows_core::Result<BOOL> {
        Err(E_NOTIMPL.into())
    }
    fn CompareStart(&self, _: u32, _: Ref<ITfRange>, _: TfAnchor) -> windows_core::Result<i32> {
        Err(E_NOTIMPL.into())
    }
    fn CompareEnd(&self, _: u32, _: Ref<ITfRange>, _: TfAnchor) -> windows_core::Result<i32> {
        Err(E_NOTIMPL.into())
    }
    fn AdjustForInsert(&self, _: u32, _: u32) -> windows_core::Result<BOOL> {
        Err(E_NOTIMPL.into())
    }
    fn GetGravity(&self, _: *mut TfGravity, _: *mut TfGravity) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn SetGravity(&self, _: u32, _: TfGravity, _: TfGravity) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn Clone(&self) -> windows_core::Result<ITfRange> {
        Ok(range(
            &self.doc,
            span(self.span.start.get(), self.span.end.get()),
        ))
    }
    fn GetContext(&self) -> windows_core::Result<ITfContext> {
        Err(E_NOTIMPL.into())
    }
}

#[implement(ITfComposition)]
struct FakeComposition {
    doc: Rc<Doc>,
    span: Rc<Span>,
}

impl ITfComposition_Impl for FakeComposition_Impl {
    fn GetRange(&self) -> windows_core::Result<ITfRange> {
        Ok(range(&self.doc, self.span.clone()))
    }
    fn ShiftStart(&self, _: u32, _: Ref<ITfRange>) -> windows_core::Result<()> {
        Ok(())
    }
    fn ShiftEnd(&self, _: u32, _: Ref<ITfRange>) -> windows_core::Result<()> {
        Ok(())
    }
    fn EndComposition(&self, _: u32) -> windows_core::Result<()> {
        if self.doc.fail_end.get() {
            return Err(E_FAIL.into());
        }
        self.doc.ended.set(self.doc.ended.get() + 1);
        let mut live = self.doc.live.borrow_mut();
        if live.as_ref().is_some_and(|l| Rc::ptr_eq(l, &self.span)) {
            *live = None;
        }
        Ok(())
    }
}

#[implement(ITfContext, ITfInsertAtSelection, ITfContextComposition)]
struct FakeContext {
    doc: Rc<Doc>,
}

impl ITfContext_Impl for FakeContext_Impl {
    fn RequestEditSession(
        &self,
        _: u32,
        session: Ref<ITfEditSession>,
        flags: TF_CONTEXT_EDIT_CONTEXT_FLAGS,
    ) -> windows_core::Result<HRESULT> {
        let session = session.ok()?.clone();
        if flags.0 & TF_ES_SYNC.0 == 0 {
            self.doc.queued.borrow_mut().push_back(session);
            return Ok(TF_S_ASYNC);
        }
        match self.doc.sync.get() {
            Sync::Refuse => Ok(TF_E_SYNCHRONOUS),
            Sync::Run => Ok(match unsafe { session.DoEditSession(1) } {
                Ok(()) => S_OK,
                Err(e) => e.code(),
            }),
            Sync::RunButReport => {
                let _ = unsafe { session.DoEditSession(1) };
                Err(E_FAIL.into())
            }
        }
    }
    fn InWriteSession(&self, _: u32) -> windows_core::Result<BOOL> {
        Err(E_NOTIMPL.into())
    }
    fn GetSelection(
        &self,
        _: u32,
        _: u32,
        _: u32,
        _: *mut TF_SELECTION,
        _: *mut u32,
    ) -> windows_core::Result<()> {
        self.doc.selections.set(self.doc.selections.get() + 1);
        Err(E_NOTIMPL.into())
    }
    fn SetSelection(
        &self,
        _: u32,
        count: u32,
        selection: *const TF_SELECTION,
    ) -> windows_core::Result<()> {
        if self.doc.fail_select.get() || count != 1 {
            return Err(E_FAIL.into());
        }
        let r = unsafe { (*selection).range.as_ref() }.ok_or(windows_core::Error::from(E_FAIL))?;
        let r: &FakeRange = unsafe { r.as_impl() };
        self.doc.caret.set(r.span.end.get());
        Ok(())
    }
    fn GetStart(&self, _: u32) -> windows_core::Result<ITfRange> {
        Err(E_NOTIMPL.into())
    }
    fn GetEnd(&self, _: u32) -> windows_core::Result<ITfRange> {
        Err(E_NOTIMPL.into())
    }
    fn GetActiveView(&self) -> windows_core::Result<ITfContextView> {
        match self.doc.layout.get() {
            Layout::NoView if self.doc.view.get().is_none() => Err(E_NOTIMPL.into()),
            _ => Ok(ComObject::new(FakeView {
                doc: self.doc.clone(),
            })
            .into_interface()),
        }
    }
    fn EnumViews(&self) -> windows_core::Result<IEnumTfContextViews> {
        Err(E_NOTIMPL.into())
    }
    fn GetStatus(&self) -> windows_core::Result<TS_STATUS> {
        Ok(TS_STATUS {
            dwDynamicFlags: self.doc.dynamic.get(),
            dwStaticFlags: self.doc.static_flags.get(),
        })
    }
    fn GetProperty(&self, _: *const GUID) -> windows_core::Result<ITfProperty> {
        Err(E_NOTIMPL.into())
    }
    fn GetAppProperty(&self, _: *const GUID) -> windows_core::Result<ITfReadOnlyProperty> {
        Err(E_NOTIMPL.into())
    }
    fn TrackProperties(
        &self,
        _: *const *const GUID,
        _: u32,
        _: *const *const GUID,
        _: u32,
    ) -> windows_core::Result<ITfReadOnlyProperty> {
        Err(E_NOTIMPL.into())
    }
    fn EnumProperties(&self) -> windows_core::Result<IEnumTfProperties> {
        Err(E_NOTIMPL.into())
    }
    fn GetDocumentMgr(&self) -> windows_core::Result<ITfDocumentMgr> {
        Err(E_NOTIMPL.into())
    }
    fn CreateRangeBackup(&self, _: u32, _: Ref<ITfRange>) -> windows_core::Result<ITfRangeBackup> {
        Err(E_NOTIMPL.into())
    }
}

impl ITfInsertAtSelection_Impl for FakeContext_Impl {
    fn InsertTextAtSelection(
        &self,
        _: u32,
        _: INSERT_TEXT_AT_SELECTION_FLAGS,
        text: &PCWSTR,
        cch: i32,
    ) -> windows_core::Result<ITfRange> {
        self.doc.text_args.borrow_mut().push((text.0 as usize, cch));
        let at = self.doc.caret.get();
        Ok(range(&self.doc, span(at, at)))
    }
    fn InsertEmbeddedAtSelection(
        &self,
        _: u32,
        _: u32,
        _: Ref<IDataObject>,
    ) -> windows_core::Result<ITfRange> {
        Err(E_NOTIMPL.into())
    }
}

impl ITfContextComposition_Impl for FakeContext_Impl {
    fn StartComposition(
        &self,
        _: u32,
        at: Ref<ITfRange>,
        _: Ref<ITfCompositionSink>,
    ) -> windows_core::Result<ITfComposition> {
        let at: &FakeRange = unsafe { at.ok()?.as_impl() };
        let live = span(at.span.start.get(), at.span.end.get());
        *self.doc.live.borrow_mut() = Some(live.clone());
        Ok(ComObject::new(FakeComposition {
            doc: self.doc.clone(),
            span: live,
        })
        .into_interface())
    }
    fn EnumCompositions(&self) -> windows_core::Result<IEnumITfCompositionView> {
        Err(E_NOTIMPL.into())
    }
    fn FindComposition(
        &self,
        _: u32,
        _: Ref<ITfRange>,
    ) -> windows_core::Result<IEnumITfCompositionView> {
        Err(E_NOTIMPL.into())
    }
    fn TakeOwnership(
        &self,
        _: u32,
        _: Ref<ITfCompositionView>,
        _: Ref<ITfCompositionSink>,
    ) -> windows_core::Result<ITfComposition> {
        Err(E_NOTIMPL.into())
    }
}

#[implement(ITfContextView)]
struct FakeView {
    doc: Rc<Doc>,
}

fn caret_at(pos: usize) -> RECT {
    let left = 100 + 10 * pos as i32;
    RECT {
        left,
        top: 200,
        right: left + 1,
        bottom: 220,
    }
}

impl ITfContextView_Impl for FakeView_Impl {
    fn GetRangeFromPoint(
        &self,
        _: u32,
        _: *const windows::Win32::Foundation::POINT,
        _: u32,
    ) -> windows_core::Result<ITfRange> {
        Err(E_NOTIMPL.into())
    }
    fn GetTextExt(
        &self,
        _: u32,
        range: Ref<ITfRange>,
        rc: *mut RECT,
        clipped: *mut BOOL,
    ) -> windows_core::Result<()> {
        let range: &FakeRange = unsafe { range.ok()?.as_impl() };
        self.doc.extents.set(self.doc.extents.get() + 1);
        let rect = match self.doc.layout.get() {
            Layout::NotReady => return Err(TS_E_NOLAYOUT.into()),
            Layout::Empty | Layout::NoView => RECT::default(),
            Layout::Ready => caret_at(range.span.end.get()),
        };
        unsafe {
            rc.write(rect);
            clipped.write(BOOL(0));
        }
        Ok(())
    }
    fn GetScreenExt(&self) -> windows_core::Result<RECT> {
        Ok(RECT {
            left: 50,
            top: 180,
            right: 600,
            bottom: 240,
        })
    }
    fn GetWnd(&self) -> windows_core::Result<windows::Win32::Foundation::HWND> {
        Ok(self.doc.view.get().unwrap_or_default())
    }
}

#[implement(ITfCompositionSink)]
struct NullSink;
impl ITfCompositionSink_Impl for NullSink_Impl {
    fn OnCompositionTerminated(&self, _: u32, _: Ref<ITfComposition>) -> windows_core::Result<()> {
        Ok(())
    }
}

/// 文件原本是「前」、插入點在它後面。
pub(crate) fn host(sync: Sync) -> (Rc<Doc>, ITfContext) {
    let doc = Rc::new(Doc {
        text: RefCell::new("前".encode_utf16().collect()),
        caret: Cell::new(1),
        sync: Cell::new(sync),
        fail_end: Cell::new(false),
        fail_select: Cell::new(false),
        fail_set_text: Cell::new(None),
        ended: Cell::new(0),
        reads: Cell::new(0),
        queued: RefCell::new(VecDeque::new()),
        layout: Cell::new(Layout::NoView),
        extents: Cell::new(0),
        selections: Cell::new(0),
        dynamic: Cell::new(0),
        text_args: RefCell::new(Vec::new()),
        static_flags: Cell::new(0),
        live: RefCell::new(None),
        view: Cell::new(None),
    });
    let ctx = ComObject::new(FakeContext { doc: doc.clone() }).into_interface();
    (doc, ctx)
}

/// 執行排隊的非同步 edit session（宿主稍後才給寫入權限）。
fn drain(doc: &Doc) {
    loop {
        let next = doc.queued.borrow_mut().pop_front();
        let Some(session) = next else { break };
        let _ = unsafe { session.DoEditSession(2) };
    }
}

fn text(doc: &Doc) -> String {
    String::from_utf16(&doc.text.borrow()).unwrap()
}

// ---------------------------------------------------------------------------
// T8a 的假引擎：記憶體 Transport，不需要 ZUIN_TEST_* 管道就能跑 host 測試。
// ---------------------------------------------------------------------------

use serde_json::{json, Value};

use crate::session::{Client, Transport};

/// 測試引擎目前累積的按鍵與模式。
pub(crate) struct Scripted {
    connected: bool,
    keys: Vec<String>,
    /// `literal` op 接在組字後面的字面尾巴。
    literal: String,
    english: bool,
    /// 上次要求的「輸出簡體字」設定。
    pub(crate) simplified: bool,
}

impl Scripted {
    fn new() -> Self {
        Scripted {
            connected: false,
            keys: Vec::new(),
            literal: String::new(),
            english: false,
            simplified: false,
        }
    }

    /// 目前注音／英文緩衝區的文字。
    fn preedit(&self) -> String {
        let joined = self.keys.concat();
        let base = if self.english {
            joined
        } else {
            match joined.as_str() {
                // 大千排列：s 是 ㄋ、u 是 ㄧ；拼音的 s／ou 對應不適用於注音。
                "s" => "ㄋ".into(),
                "su" => "ㄋㄧ".into(),
                "su3" => "你".into(),
                "su3cl3" => "你好".into(),
                "w96j0" => "台灣".into(),
                "j" => "ㄨ".into(),
                "ji" => "ㄨㄛ".into(),
                "ji3" => "我".into(),
                "c" => "ㄏ".into(),
                "ji3c" => "我ㄏ".into(),
                _ => joined,
            }
        };
        format!("{base}{}", self.literal)
    }

    fn snapshot(&self, commit: String) -> Value {
        let preedit = if commit.is_empty() {
            self.preedit()
        } else {
            String::new()
        };
        let segments = if preedit.is_empty() {
            Value::Array(Vec::new())
        } else {
            json!([{"text": preedit, "style": "normal"}])
        };
        json!({
            "ok": true,
            "snapshot": {
                "handled": !preedit.is_empty() || !commit.is_empty(),
                "preedit": segments,
                "commit": commit,
                "cursor": {"rune": preedit.chars().count(), "utf16": preedit.encode_utf16().count()},
                "output_simplified": self.simplified,
            }
        })
    }
}

impl Transport for Scripted {
    fn request(&mut self, request: &Value) -> Option<Value> {
        match request["op"].as_str().unwrap_or("") {
            "hello" => Some(json!({"ok": true, "session": 1, "info": {}})),
            "health" => Some(json!({"ok": true, "info": {
                "simplified_hotkey": "true",
                "output_simplified_live": "true",
                "output_simplified": if self.simplified { "true" } else { "false" },
                "ctrl_punctuation": "true",
                "ctrl_punctuation_keys": ",.';?!/",
            }})),
            "output_simplified" => {
                self.simplified = request["on"].as_bool().unwrap_or(false);
                Some(self.snapshot(String::new()))
            }
            "toggle" => {
                self.english = !self.english;
                self.keys.clear();
                self.literal.clear();
                Some(self.snapshot(String::new()))
            }
            "key" => {
                // 按鍵現在一律是 jc 的結構化 Key（`to_jc_request` 轉的）；這裡是
                // 假引擎，要把它還原成 0c 鍵名才對得上下面的邊界判斷。
                let key = match request["key"].as_str() {
                    Some(name) => name.to_string(),
                    None => {
                        let name = serde_json::from_value::<jc_proto::Key>(request["key"].clone())
                            .ok()
                            .and_then(|key| crate::keys::to_protocol(&key))
                            .unwrap_or_default();
                        // `to_protocol` 把 Tab 與 ShiftTab 都寫成 "BackTab"。
                        if name == "BackTab" {
                            "Tab".to_string()
                        } else {
                            name
                        }
                    }
                };
                if key == "Backspace" {
                    // 字面尾巴只刪一個字元；沒有尾巴時刪最後一個按鍵。
                    if self.literal.pop().is_none() {
                        self.keys.pop();
                    }
                    return Some(self.snapshot(String::new()));
                }
                let boundary = matches!(
                    key.as_str(),
                    " " | "Left" | "Enter" | "Delete" | "Tab" | "."
                );
                let commit = if boundary {
                    self.preedit()
                } else {
                    String::new()
                };
                if boundary {
                    self.keys.clear();
                    self.literal.clear();
                    Some(self.snapshot(commit))
                } else {
                    self.keys.push(key);
                    Some(self.snapshot(String::new()))
                }
            }
            "literal" => {
                self.literal
                    .push_str(request["text"].as_str().unwrap_or(""));
                Some(self.snapshot(String::new()))
            }
            "commit" => {
                let commit = self.preedit();
                self.keys.clear();
                self.literal.clear();
                Some(self.snapshot(commit))
            }
            "cancel" | "reset" => {
                self.keys.clear();
                self.literal.clear();
                Some(self.snapshot(String::new()))
            }
            "snapshot" => Some(self.snapshot(String::new())),
            _ => Some(json!({"ok": true})),
        }
    }

    fn connected(&self) -> bool {
        self.connected
    }

    fn connect(&mut self) -> bool {
        self.connected = true;
        true
    }

    fn close(&mut self) {
        self.connected = false;
    }

    fn alive(&mut self) -> bool {
        self.connected
    }
}

/// 注入假引擎的 Session；注音模式、緩衝區是空的。
pub(crate) fn session() -> Session {
    Session::with_client(Client::with_transport(Box::new(Scripted::new())))
}

/// 沒有引擎的 client：模擬「引擎沒有回應」（#66 的 dead engine 案例）。
pub(crate) fn dead_client() -> Client {
    Client::new()
}
