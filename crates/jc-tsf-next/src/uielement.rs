//! UI-less 模式（#86）：遊戲等自己畫候選的宿主（例如英雄聯盟遊戲內聊天）以
//! `TF_TMAE_UIELEMENTENABLEDONLY` 啟用輸入法，只接受註冊了 UIELEMENTENABLED 的輸入法，
//! 並從 ITfCandidateListUIElement 讀候選清單自己畫。
//!
//! 移植自 0c `platforms/windows/tsf/src/uielement.rs`（main `8f443af`）的生產碼。
//! 0c 的 `zuin_win_common::UiMessage` 在新 crate 不存在，改成本檔的 [`Candidates`]，
//! 並提供 `From<&Snapshot>` 讓 T6 直接從 adapter 的快照更新。
//!
//! 每次顯示候選都先 BeginUIElement：宿主回 pbShow=FALSE 表示它自己畫，外殼不顯示候選窗；
//! 一般應用程式沒有 UI 元件接收者，TSF 回 TRUE，照常畫自己的候選窗。清單只放目前這一頁
//! （核心只送目前頁），翻頁照常用按鍵。
//!
//! 沒有候選、只有補全提示（Tab）、提示訊息或加詞標示說明時，沒有對應的 UI 元件可交給宿主，
//! 外殼也不畫自己的視窗：遊戲在獨佔全螢幕時，任何疊上去的視窗都會讓它失去全螢幕並跳回桌面（#159）。

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{E_INVALIDARG, E_NOTIMPL, E_POINTER};
use windows::Win32::UI::TextServices::{
    ITfCandidateListUIElement, ITfCandidateListUIElement_Impl, ITfDocumentMgr, ITfUIElementMgr,
    ITfUIElement_Impl, TF_CLUIE_COUNT, TF_CLUIE_CURRENTPAGE, TF_CLUIE_DOCUMENTMGR,
    TF_CLUIE_PAGEINDEX, TF_CLUIE_SELECTION, TF_CLUIE_STRING,
};
use windows_core::{implement, ComObject, BOOL, BSTR, GUID};

use crate::session::Snapshot;

/// adapter spike 不帶檔案 I/O：保留呼叫點，不產生輸出。T6 再接到 `logging`。
macro_rules! zlog {
    ($($arg:tt)*) => {{ let _ = format_args!($($arg)*); }};
}

/// COM 物件計數。新 crate 還沒有 DLL 生命週期管理，T6 接到 `lib.rs` 的 LIVE_OBJECTS。
fn object_created() {}
fn object_destroyed() {}

/// COM 邊界不得 unwind；與 0c `lib.rs::guarded` 行為相同。
fn guarded<T, F: FnOnce() -> windows_core::Result<T>>(what: &str, f: F) -> windows_core::Result<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(_) => {
            let _ = what;
            Err(windows::Win32::Foundation::E_FAIL.into())
        }
    }
}

/// 本元件的識別碼（ITfUIElement::GetGUID）；宿主只用來分辨元件，不需要註冊。
const ELEMENT_GUID: GUID = GUID::from_u128(0xda0ca60a_1a05_4b70_980d_a964070868e1);
const ALL_FLAGS: u32 = TF_CLUIE_DOCUMENTMGR
    | TF_CLUIE_COUNT
    | TF_CLUIE_SELECTION
    | TF_CLUIE_STRING
    | TF_CLUIE_PAGEINDEX
    | TF_CLUIE_CURRENTPAGE;

/// 要交給宿主的一頁候選。取代 0c 的 `zuin_win_common::UiMessage`。
#[derive(Clone, Debug, Default)]
pub struct Candidates {
    /// 這一鍵是否要顯示候選（false 表示收起）。
    pub show: bool,
    /// 目前這一頁的候選文字。
    pub candidates: Vec<String>,
    /// 目前反白的候選索引。
    pub highlight: i64,
}

impl<'a> From<&'a Snapshot> for Candidates {
    fn from(snapshot: &'a Snapshot) -> Self {
        Self {
            show: !snapshot.candidates.is_empty(),
            candidates: snapshot
                .candidates
                .iter()
                .map(|(_, text)| text.clone())
                .collect(),
            highlight: snapshot.highlight,
        }
    }
}

#[implement(ITfCandidateListUIElement)]
struct CandidateList {
    items: RefCell<Vec<String>>,
    selection: Cell<u32>,
    document: RefCell<Option<ITfDocumentMgr>>,
    flags: Cell<u32>,
    /// 外殼是否該畫自己的候選窗：BeginUIElement 的 pbShow，之後宿主可用 Show 改變。
    shown: Cell<bool>,
}

impl Drop for CandidateList {
    fn drop(&mut self) {
        object_destroyed();
    }
}

impl ITfUIElement_Impl for CandidateList_Impl {
    fn GetDescription(&self) -> windows_core::Result<BSTR> {
        guarded("UIElement.GetDescription", || Ok(BSTR::from("候選字")))
    }
    fn GetGUID(&self) -> windows_core::Result<GUID> {
        guarded("UIElement.GetGUID", || Ok(ELEMENT_GUID))
    }
    fn Show(&self, show: BOOL) -> windows_core::Result<()> {
        // 下一次更新候選時依此決定是否畫自己的候選窗。
        guarded("UIElement.Show", || {
            self.shown.set(show.as_bool());
            Ok(())
        })
    }
    fn IsShown(&self) -> windows_core::Result<BOOL> {
        guarded("UIElement.IsShown", || Ok(self.shown.get().into()))
    }
}

impl ITfCandidateListUIElement_Impl for CandidateList_Impl {
    fn GetUpdatedFlags(&self) -> windows_core::Result<u32> {
        guarded("UIElement.GetUpdatedFlags", || Ok(self.flags.replace(0)))
    }
    fn GetDocumentMgr(&self) -> windows_core::Result<ITfDocumentMgr> {
        guarded("UIElement.GetDocumentMgr", || {
            self.document
                .borrow()
                .clone()
                .ok_or_else(|| E_NOTIMPL.into())
        })
    }
    fn GetCount(&self) -> windows_core::Result<u32> {
        guarded(
            "UIElement.GetCount",
            || Ok(self.items.borrow().len() as u32),
        )
    }
    fn GetSelection(&self) -> windows_core::Result<u32> {
        guarded("UIElement.GetSelection", || Ok(self.selection.get()))
    }
    fn GetString(&self, index: u32) -> windows_core::Result<BSTR> {
        guarded("UIElement.GetString", || {
            self.items
                .borrow()
                .get(index as usize)
                .map(|text| BSTR::from(text.as_str()))
                .ok_or_else(|| E_INVALIDARG.into())
        })
    }
    fn GetPageIndex(
        &self,
        index: *mut u32,
        size: u32,
        count: *mut u32,
    ) -> windows_core::Result<()> {
        // 只有一頁，從 0 開始。index 為 null 時只回頁數（宿主先問大小）。
        guarded("UIElement.GetPageIndex", || {
            if count.is_null() {
                return Err(E_POINTER.into());
            }
            unsafe {
                count.write(1);
                if !index.is_null() && size >= 1 {
                    index.write(0);
                }
            }
            Ok(())
        })
    }
    fn SetPageIndex(&self, _index: *const u32, _count: u32) -> windows_core::Result<()> {
        guarded("UIElement.SetPageIndex", || Err(E_NOTIMPL.into()))
    }
    fn GetCurrentPage(&self) -> windows_core::Result<u32> {
        guarded("UIElement.GetCurrentPage", || Ok(0))
    }
}

/// 一個 TSF 執行緒的候選清單元件；Activate 時建立，Deactivate 前結束。
pub struct Elements {
    mgr: ITfUIElementMgr,
    element: ComObject<CandidateList>,
    id: Option<u32>,
}

impl Elements {
    pub fn new(mgr: ITfUIElementMgr) -> Elements {
        object_created(); // CandidateList 的 Drop 扣回
        Elements {
            mgr,
            element: ComObject::new(CandidateList {
                items: RefCell::new(Vec::new()),
                selection: Cell::new(0),
                document: RefCell::new(None),
                flags: Cell::new(0),
                shown: Cell::new(true),
            }),
            id: None,
        }
    }

    /// 更新候選清單並回傳外殼是否要畫自己的視窗。只有宿主明確讓外殼畫（pbShow=TRUE）的候選清單才畫；
    /// 沒有候選（只有補全提示、提示訊息或標示說明）時結束元件、不畫（#159）。
    pub fn present(&mut self, view: &Candidates, document: Option<ITfDocumentMgr>) -> bool {
        if !view.show {
            self.end();
            return true; // 收起：讓呼叫端照常送出隱藏訊息
        }
        if view.candidates.is_empty() {
            self.end();
            return false;
        }
        let element = &self.element;
        *element.items.borrow_mut() = view.candidates.clone();
        element
            .selection
            .set(view.highlight.clamp(0, view.candidates.len() as i64 - 1) as u32);
        *element.document.borrow_mut() = document;
        element.flags.set(ALL_FLAGS);
        match self.id {
            Some(id) => unsafe {
                let _ = self.mgr.UpdateUIElement(id);
            },
            None => {
                // ITfUIElement 是基底介面，衍生介面可直接當參數傳。
                let interface: ITfCandidateListUIElement = element.to_interface();
                let mut show = BOOL(1);
                let mut id = 0;
                match unsafe { self.mgr.BeginUIElement(&interface, &mut show, &mut id) } {
                    Ok(()) => {
                        if !show.as_bool() {
                            zlog!("ui: 宿主自行繪製候選（UI-less）");
                        }
                        element.shown.set(show.as_bool());
                        self.id = Some(id);
                    }
                    // 沒有元件管理員可用：照常畫自己的候選窗。
                    Err(_) => return true,
                }
            }
        }
        element.shown.get()
    }

    /// 候選窗關閉：通知宿主結束這個元件。
    pub fn end(&mut self) {
        if let Some(id) = self.id.take() {
            unsafe {
                let _ = self.mgr.EndUIElement(id);
            }
        }
        *self.element.document.borrow_mut() = None;
    }
}

impl Drop for Elements {
    fn drop(&mut self) {
        self.end();
    }
}
