//! `ITfCandidateListUIElement`：交給 Windows 畫的候選清單。
//!
//! 第一版不自繪候選窗。TSF 的 `ITfUIElementMgr` 由 `ITfThreadMgr` 取得，
//! 每次候選出現時 `BeginUIElement`，資料變更時 `UpdateUIElement`，候選清空
//! 時 `EndUIElement`。Windows 只透過 [`ITfCandidateListUIElement`] 讀取畫面
//! 需要的資料，因此這個檔案只提供「讀得到的快照」，不碰引擎、不改字。
//!
//! **顯示限制**：這個介面每個候選只有一個 `GetString`，沒有讀音或 label
//! 的欄位。`GetDescription` 是整個 UI 元素的說明，不是逐項的第二行。因此
//! 讀音與 label 只留在快照裡供除錯，畫面上只會看到候選字面；要顯示讀音或
//! 標籤必須改成自繪視窗，不能靠這個介面。

use std::cell::{Ref, RefCell, RefMut};
use std::rc::Rc;

use windows::core::{implement, Error, IUnknownImpl, Interface, Result, BOOL, BSTR, GUID};
use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::UI::TextServices::{
    ITfCandidateListUIElement, ITfCandidateListUIElement_Impl, ITfDocumentMgr, ITfUIElement,
    ITfUIElementMgr, ITfUIElement_Impl, TF_CLUIE_COUNT, TF_CLUIE_CURRENTPAGE, TF_CLUIE_DOCUMENTMGR,
    TF_CLUIE_PAGEINDEX, TF_CLUIE_SELECTION, TF_CLUIE_STRING,
};

use crate::guids::GUID_CANDIDATE_UI_ELEMENT;
use crate::logging;

/// 每頁顯示幾個候選。`1`–`9` 對應頁內位置，`0` 留給注音鍵。
pub const PAGE_SIZE: usize = 9;

/// 第一次 `BeginUIElement` 時要宣告全部欄位都是新的。
const ALL_UPDATED: u32 = TF_CLUIE_DOCUMENTMGR
    | TF_CLUIE_COUNT
    | TF_CLUIE_SELECTION
    | TF_CLUIE_STRING
    | TF_CLUIE_PAGEINDEX
    | TF_CLUIE_CURRENTPAGE;

/// 一個候選的顯示資料。
///
/// `reading` 與 `label` 目前畫不出來，保留欄位是為了文件、除錯與之後的自繪
/// 視窗；不要為此改協定。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CandidateItem {
    /// 候選字面。
    pub text: String,
    /// 讀音；`details` 缺漏時是空字串。
    pub reading: String,
    /// 引擎給目前高亮候選的顯示原因；其他候選一律 `None`。
    pub label: Option<String>,
}

/// 一次 `State` 對應的候選快照。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CandidateSnapshot {
    /// 候選，順序即顯示順序。
    pub items: Vec<CandidateItem>,
    /// 引擎目前的高亮索引。
    pub highlighted: usize,
}

/// 分頁後的候選清單。
///
/// 這是純資料，不碰 COM；COM 物件只讀它。頁碼一律由 `highlighted` 反推，
/// 不由 wrapper 自己保存「目前第幾頁」，避免和引擎的狀態分叉。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CandidatePages {
    items: Vec<CandidateItem>,
    highlighted: usize,
    /// 外部透過 `SetPageIndex` 指定的頁首；空的時候用 [`PAGE_SIZE`] 計算。
    custom_starts: Vec<u32>,
}

impl CandidatePages {
    /// 由快照建立；高亮超出範圍時收回最後一項。
    fn new(snapshot: &CandidateSnapshot) -> Self {
        let highlighted = if snapshot.items.is_empty() {
            0
        } else {
            snapshot.highlighted.min(snapshot.items.len() - 1)
        };
        Self {
            items: snapshot.items.clone(),
            highlighted,
            custom_starts: Vec::new(),
        }
    }

    /// 候選總數。
    fn count(&self) -> u32 {
        self.items.len() as u32
    }

    /// 目前高亮索引。
    fn selection(&self) -> u32 {
        self.highlighted as u32
    }

    /// 每一頁的起始索引。
    fn starts(&self) -> Vec<u32> {
        if !self.custom_starts.is_empty() {
            return self.custom_starts.clone();
        }
        (0..self.items.len())
            .step_by(PAGE_SIZE)
            .map(|index| index as u32)
            .collect()
    }

    /// 目前頁碼。
    fn current_page(&self) -> u32 {
        let starts = self.starts();
        let index = self.highlighted as u32;
        starts
            .iter()
            .rposition(|start| *start <= index)
            .unwrap_or(0) as u32
    }

    /// 候選字面；用來判斷 `TF_CLUIE_STRING`。
    fn texts(&self) -> Vec<&str> {
        self.items.iter().map(|item| item.text.as_str()).collect()
    }

    /// 指定外部頁首；`SetPageIndex` 用。
    fn set_starts(&mut self, starts: &[u32]) {
        self.custom_starts = starts.to_vec();
    }

    /// 重設成依 [`PAGE_SIZE`] 計算的預設分頁。
    fn reset_starts(&mut self) {
        self.custom_starts.clear();
    }

    /// 取指定索引的候選。
    fn item(&self, index: u32) -> Option<&CandidateItem> {
        self.items.get(index as usize)
    }
}

/// COM 物件與 [`CandidateHost`] 共用的資料。
#[derive(Debug, Default)]
struct ElementData {
    pages: CandidatePages,
    document: Option<ITfDocumentMgr>,
    shown: bool,
    /// 上一次變更的欄位；初始值是 [`ALL_UPDATED`]。
    flags: u32,
}

impl ElementData {
    /// 換上新快照，回傳這次真正變更的欄位。
    fn apply(&mut self, snapshot: &CandidateSnapshot, document: Option<ITfDocumentMgr>) -> u32 {
        let next = CandidatePages::new(snapshot);
        let mut flags = 0;
        if self.pages.count() != next.count() {
            flags |= TF_CLUIE_COUNT;
        }
        if self.pages.texts() != next.texts() {
            flags |= TF_CLUIE_STRING;
        }
        if self.pages.selection() != next.selection() {
            flags |= TF_CLUIE_SELECTION;
        }
        if self.pages.starts() != next.starts() {
            flags |= TF_CLUIE_PAGEINDEX;
        }
        if self.pages.current_page() != next.current_page() {
            flags |= TF_CLUIE_CURRENTPAGE;
        }
        if !same_document(&self.document, &document) {
            flags |= TF_CLUIE_DOCUMENTMGR;
        }

        self.pages = next;
        self.pages.reset_starts();
        self.document = document;
        self.flags = flags;
        flags
    }
}

/// 兩個文件管理員是否指到同一個 COM 物件。
fn same_document(left: &Option<ITfDocumentMgr>, right: &Option<ITfDocumentMgr>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => Interface::as_raw(left) == Interface::as_raw(right),
        (None, None) => true,
        _ => false,
    }
}

/// 交給 Windows 讀的 UI 元素。
///
/// 資料用 `Rc<RefCell<_>>` 分享：COM 物件要活到 `EndUIElement`，而
/// [`CandidateHost`] 必須能就地更新同一份內容。所有呼叫都在 TSF 的 UI
/// 執行緒；這個元素刻意不是 agile，不提供跨執行緒存取。
#[implement(ITfCandidateListUIElement, Agile = false)]
struct CandidateListElement {
    data: Rc<RefCell<ElementData>>,
}

impl CandidateListElement {
    /// 借用資料。所有呼叫都在同一條 UI 執行緒，不會有跨執行緒競爭。
    fn data(&self) -> Ref<'_, ElementData> {
        self.data.borrow()
    }

    /// 可變借用，供 `Show` 與 `SetPageIndex` 更新狀態。
    fn data_mut(&self) -> RefMut<'_, ElementData> {
        self.data.borrow_mut()
    }
}

impl ITfUIElement_Impl for CandidateListElement_Impl {
    fn GetDescription(&self) -> Result<BSTR> {
        Ok(BSTR::from("jc 注音：候選清單"))
    }

    fn GetGUID(&self) -> Result<GUID> {
        Ok(GUID_CANDIDATE_UI_ELEMENT)
    }

    fn Show(&self, bshow: BOOL) -> Result<()> {
        self.get_impl().data_mut().shown = bshow.as_bool();
        Ok(())
    }

    fn IsShown(&self) -> Result<BOOL> {
        Ok(self.get_impl().data().shown.into())
    }
}

impl ITfCandidateListUIElement_Impl for CandidateListElement_Impl {
    fn GetUpdatedFlags(&self) -> Result<u32> {
        Ok(self.get_impl().data().flags)
    }

    fn GetDocumentMgr(&self) -> Result<ITfDocumentMgr> {
        let data = self.get_impl().data();
        data.document
            .clone()
            .ok_or_else(|| Error::from_hresult(windows::Win32::Foundation::E_FAIL))
    }

    fn GetCount(&self) -> Result<u32> {
        Ok(self.get_impl().data().pages.count())
    }

    fn GetSelection(&self) -> Result<u32> {
        Ok(self.get_impl().data().pages.selection())
    }

    fn GetString(&self, index: u32) -> Result<BSTR> {
        let data = self.get_impl().data();
        let item = data
            .pages
            .item(index)
            .ok_or_else(|| Error::from_hresult(E_INVALIDARG))?;
        Ok(BSTR::from(item.text.as_str()))
    }

    fn GetPageIndex(&self, pindex: *mut u32, size: u32, pupagecnt: *mut u32) -> Result<()> {
        if pupagecnt.is_null() {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
        let starts = self.get_impl().data().pages.starts();
        // SAFETY: 呼叫端保證 pupagecnt 指向可寫的一個 u32；這是 COM 的出參約定。
        unsafe {
            *pupagecnt = starts.len() as u32;
        }
        if pindex.is_null() {
            // 先問頁數的合法呼叫。
            return Ok(());
        }
        let count = (size as usize).min(starts.len());
        for (offset, start) in starts.iter().take(count).enumerate() {
            // SAFETY: pindex 是呼叫端提供、至少 size 個 u32 的緩衝。
            unsafe {
                *pindex.add(offset) = *start;
            }
        }
        Ok(())
    }

    fn SetPageIndex(&self, pindex: *const u32, count: u32) -> Result<()> {
        if count == 0 {
            self.get_impl().data_mut().pages.reset_starts();
            return Ok(());
        }
        if pindex.is_null() {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
        // SAFETY: 呼叫端保證 pindex 指向至少 count 個 u32。
        let starts = unsafe { std::slice::from_raw_parts(pindex, count as usize) };
        self.get_impl().data_mut().pages.set_starts(starts);
        Ok(())
    }

    fn GetCurrentPage(&self) -> Result<u32> {
        Ok(self.get_impl().data().pages.current_page())
    }
}

/// 管理 UI 元素的顯示生命週期。
///
/// `BeginUIElement` 之後 Windows 可能自己畫，也可能要求 TIP 自己畫
/// （`bShow == FALSE`，例如應用程式內嵌候選）。兩種情況都要在資料變更時
/// 呼叫 `UpdateUIElement`，讓對方重新讀取。
pub struct CandidateHost {
    manager: Option<ITfUIElementMgr>,
    element: Option<ITfCandidateListUIElement>,
    data: Option<Rc<RefCell<ElementData>>>,
    /// `BeginUIElement` 配到的識別碼。
    id: u32,
    /// 是否已 `BeginUIElement` 且尚未 `EndUIElement`。
    started: bool,
}

impl CandidateHost {
    /// 建立宿主；`manager` 為 `None` 時整個候選窗等於停用。
    pub fn new(manager: Option<ITfUIElementMgr>) -> Self {
        Self {
            manager,
            element: None,
            data: None,
            id: 0,
            started: false,
        }
    }

    /// 以最新快照更新候選窗；空清單等於關閉。
    pub fn update(&mut self, snapshot: CandidateSnapshot, document: Option<ITfDocumentMgr>) {
        if snapshot.items.is_empty() {
            self.end();
            return;
        }
        if !self.started || self.element.is_none() {
            // 先複製 manager，避免 `begin` 需要 `&mut self` 時還借著欄位。
            if let Some(manager) = self.manager.clone() {
                self.begin(&manager, snapshot, document);
            }
            return;
        }

        let Some(manager) = &self.manager else {
            return;
        };
        let Some(data) = &self.data else {
            return;
        };
        let flags = data.borrow_mut().apply(&snapshot, document);
        if flags == 0 {
            return;
        }
        // SAFETY: 這是 TSF 的 UI 執行緒；id 來自同一條 manager 的 BeginUIElement。
        if let Err(error) = unsafe { manager.UpdateUIElement(self.id) } {
            logging::warn(format!("UpdateUIElement 失敗：{error}"));
        }
    }

    /// 關閉候選窗並放掉 COM 引用。
    pub fn end(&mut self) {
        if self.started {
            if let Some(manager) = &self.manager {
                // SAFETY: id 是這條 manager 配發的，且只 End 一次。
                if let Err(error) = unsafe { manager.EndUIElement(self.id) } {
                    logging::warn(format!("EndUIElement 失敗：{error}"));
                }
            }
        }
        self.started = false;
        self.id = 0;
        self.element = None;
        self.data = None;
    }

    /// 第一次顯示：建立元素、填資料、`BeginUIElement`。
    fn begin(
        &mut self,
        manager: &ITfUIElementMgr,
        snapshot: CandidateSnapshot,
        document: Option<ITfDocumentMgr>,
    ) {
        let data = Rc::new(RefCell::new(ElementData {
            pages: CandidatePages::new(&snapshot),
            document,
            shown: true,
            flags: ALL_UPDATED,
        }));
        let element: ITfCandidateListUIElement = CandidateListElement {
            data: Rc::clone(&data),
        }
        .into();

        let ui: ITfUIElement = match element.cast() {
            Ok(ui) => ui,
            Err(error) => {
                logging::warn(format!("候選元素無法轉成 ITfUIElement：{error}"));
                return;
            }
        };
        let mut show = BOOL::from(true);
        let mut id = 0u32;
        // SAFETY: 元素與 manager 都只在這條 UI 執行緒使用；show／id 是出參。
        if let Err(error) = unsafe { manager.BeginUIElement(&ui, &mut show, &mut id) } {
            logging::warn(format!("BeginUIElement 失敗：{error}"));
            return;
        }

        self.element = Some(element);
        self.data = Some(data);
        self.id = id;
        self.started = true;
        if !show.as_bool() {
            // 應用程式要自己畫；先通知一次讓它讀取剛填好的內容。
            // SAFETY: 同一條 manager、同一個 UI 執行緒。
            if let Err(error) = unsafe { manager.UpdateUIElement(id) } {
                logging::warn(format!("UpdateUIElement 失敗：{error}"));
            }
            logging::debug("候選 UI 由應用程式自行繪製");
        }
        logging::debug(format!("候選窗開啟：{} 項，id={id}", snapshot.items.len()));
    }
}

/// 純分頁邏輯的測試。
#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(texts: &[&str], highlighted: usize) -> CandidateSnapshot {
        CandidateSnapshot {
            items: texts
                .iter()
                .map(|text| CandidateItem {
                    text: (*text).to_string(),
                    reading: String::new(),
                    label: None,
                })
                .collect(),
            highlighted,
        }
    }

    #[test]
    fn pages_hold_nine_candidates() {
        let texts: Vec<String> = (0..20).map(|index| index.to_string()).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let pages = CandidatePages::new(&snapshot(&refs, 0));
        assert_eq!(pages.starts(), vec![0, 9, 18]);
        assert_eq!(pages.count(), 20);
        assert_eq!(pages.current_page(), 0);
    }

    #[test]
    fn current_page_follows_highlight() {
        let texts: Vec<String> = (0..20).map(|index| index.to_string()).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let pages = CandidatePages::new(&snapshot(&refs, 9));
        assert_eq!(pages.current_page(), 1);
        let pages = CandidatePages::new(&snapshot(&refs, 19));
        assert_eq!(pages.current_page(), 2);
    }

    #[test]
    fn highlight_is_clamped_to_the_list() {
        let pages = CandidatePages::new(&snapshot(&["我", "窩"], 99));
        assert_eq!(pages.selection(), 1);
        let pages = CandidatePages::new(&snapshot(&[], 99));
        assert_eq!(pages.selection(), 0);
        assert_eq!(pages.current_page(), 0);
        assert!(pages.starts().is_empty());
    }

    #[test]
    fn external_page_index_is_kept_until_next_update() {
        let mut pages = CandidatePages::new(&snapshot(&["a", "b", "c"], 2));
        pages.set_starts(&[0, 2]);
        assert_eq!(pages.starts(), vec![0, 2]);
        assert_eq!(pages.current_page(), 1);
        pages.reset_starts();
        assert_eq!(pages.starts(), vec![0]);
    }

    #[test]
    fn item_lookup_rejects_out_of_range() {
        let pages = CandidatePages::new(&snapshot(&["我"], 0));
        assert_eq!(pages.item(0).map(|item| item.text.as_str()), Some("我"));
        assert!(pages.item(1).is_none());
    }

    #[test]
    fn update_flags_only_mark_changed_fields() {
        let mut data = ElementData {
            pages: CandidatePages::new(&snapshot(&["我", "窩"], 0)),
            ..Default::default()
        };
        assert_eq!(data.apply(&snapshot(&["我", "窩"], 0), None), 0);

        let flags = data.apply(&snapshot(&["我", "窩"], 1), None);
        assert_eq!(flags & TF_CLUIE_SELECTION, TF_CLUIE_SELECTION);
        assert_eq!(flags & TF_CLUIE_COUNT, 0);
        assert_eq!(flags & TF_CLUIE_STRING, 0);

        let flags = data.apply(&snapshot(&["我", "我"], 1), None);
        assert_eq!(flags & TF_CLUIE_STRING, TF_CLUIE_STRING);
        assert_eq!(flags & TF_CLUIE_SELECTION, 0);

        let flags = data.apply(&snapshot(&["我", "我", "餓"], 1), None);
        assert_eq!(flags & TF_CLUIE_COUNT, TF_CLUIE_COUNT);
    }
}
