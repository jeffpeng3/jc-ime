//! `ITfCandidateListUIElement`：交給 Windows 畫的候選清單。
//!
//! 第一版不自繪候選窗。TSF 的 `ITfUIElementMgr` 由 `ITfThreadMgr` 取得，
//! 每次候選出現時 `BeginUIElement`，資料變更時 `UpdateUIElement`，候選清空
//! 時 `EndUIElement`。Windows 只透過 [`ITfCandidateListUIElement`] 讀取畫面
//! 需要的資料，因此這個檔案只提供「讀得到的快照」，不碰引擎、不改字。
//!
//! 滑鼠點選由 [`ITfCandidateListUIElementBehavior`] 接住：`SetSelection` 把
//! 點到的頁內索引換成對應的數字鍵（與鍵盤 `1`–`0` 選字同一條路徑），
//! `Finalize` 送 `Space`、`Abort` 送 `Escape`。數字鍵本身就代表「選取並提交」，
//! 所以送出數字鍵後 `Finalize` 不再補鍵。實際提交什麼字仍由引擎決定，wrapper
//! 不從快照自己挑字。
//!
//! **顯示限制**：這個介面每個候選只有一個 `GetString`，沒有讀音或 label
//! 的欄位。`GetDescription` 是整個 UI 元素的說明，不是逐項的第二行。因此
//! 讀音與 label 只留在快照裡供除錯，畫面上只會看到候選字面；要顯示讀音或
//! 標籤必須改成自繪視窗，不能靠這個介面。

use std::cell::{Cell, Ref, RefCell, RefMut};
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::sync::Arc;

use jc_proto::{Key, KeyCode, Request};
use windows::core::{implement, Error, IUnknownImpl, Interface, Result, BOOL, BSTR, GUID};
use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::UI::TextServices::{
    ITfCandidateListUIElement, ITfCandidateListUIElementBehavior,
    ITfCandidateListUIElementBehavior_Impl, ITfCandidateListUIElement_Impl, ITfDocumentMgr,
    ITfUIElement, ITfUIElementMgr, ITfUIElement_Impl, TF_CLUIE_COUNT, TF_CLUIE_CURRENTPAGE,
    TF_CLUIE_DOCUMENTMGR, TF_CLUIE_PAGEINDEX, TF_CLUIE_SELECTION, TF_CLUIE_STRING,
};

use crate::engine::{Job, Queue};
use crate::guids::GUID_CANDIDATE_UI_ELEMENT;
use crate::logging;

use jc_core::output::CANDIDATE_PAGE_SIZE;

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
/// 不由 wrapper 自己保存「目前第幾頁」，避免和引擎的狀態分叉。頁面大小讀
/// 核心的公開常數 [`CANDIDATE_PAGE_SIZE`]，不從協定內容猜。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CandidatePages {
    items: Vec<CandidateItem>,
    highlighted: usize,
    /// 外部透過 `SetPageIndex` 指定的頁首；空的時候用核心頁面大小計算。
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
            .step_by(CANDIDATE_PAGE_SIZE.max(1))
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

    /// 重設成依核心頁面大小計算的預設分頁。
    fn reset_starts(&mut self) {
        self.custom_starts.clear();
    }
    /// 取指定索引的候選。
    fn item(&self, index: u32) -> Option<&CandidateItem> {
        self.items.get(index as usize)
    }
}

/// 把滑鼠點選轉成引擎看得懂的按鍵請求。
///
/// 只送一個數字鍵：核心收到後照 `1`–`0` 選字規則選取並提交，與鍵盤走同一條
/// 路徑。`generation` 與 [`crate::composition::Control`] 共用同一個計數器，
/// 過期的 `State` 才會照既有規則被丟棄。
pub struct CandidateActions {
    jobs: Sender<Job>,
    generation: Rc<Cell<u64>>,
    queue: Arc<Queue>,
}

impl CandidateActions {
    /// 建立與 `Control` 共用的請求通道。
    pub fn new(jobs: Sender<Job>, generation: Rc<Cell<u64>>, queue: Arc<Queue>) -> Self {
        Self {
            jobs,
            generation,
            queue,
        }
    }

    /// 引擎是否已連線；斷線時點選只記 log，不排隊。
    fn connected(&self) -> bool {
        self.queue.is_connected()
    }

    /// 同一批按鍵共用一個世代，讓引擎依序處理、畫面只認最後的結果。
    fn send_keys(&self, keys: &[Key]) {
        if keys.is_empty() {
            return;
        }
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        for key in keys {
            let job = Job {
                generation,
                request: Request::Key { key: *key },
            };
            if self.jobs.send(job).is_err() {
                logging::warn("worker 已結束，候選點選請求被丟棄");
                break;
            }
        }
    }
}

/// 頁內位置換成核心的數字選字鍵。
///
/// 核心的 `1`–`9` 對應頁內第 1–9 項；頁面大小是 10 時第 10 項是 `0`。超過
/// 10 項的頁面沒有對應鍵，回 `None`。
fn digit_for(within: usize) -> Option<char> {
    match within + 1 {
        10 => Some('0'),
        ordinal if ordinal < 10 => char::from_digit(ordinal as u32, 10),
        _ => None,
    }
}

/// 候選索引換成要送給核心的數字鍵。
///
/// 頁面大小一律讀核心的公開常數 [`CANDIDATE_PAGE_SIZE`]，不在外殼寫死；索引
/// 先用頁面大小取頁內位置，所以第 2 頁的第 1 項同樣送 `1`。
fn selection_key(index: u32) -> Option<Key> {
    let page = CANDIDATE_PAGE_SIZE.max(1);
    let within = index as usize % page;
    digit_for(within).map(|ch| Key::char(ch, false))
}

/// `Finalize` 的按鍵：空白鍵選取目前高亮項，由引擎決定提交內容。
fn confirm_key() -> Key {
    Key::new(KeyCode::Space, None, false)
}

/// `Abort` 的按鍵：Escape 關掉引擎的候選窗。
fn abort_key() -> Key {
    Key::new(KeyCode::Escape, None, false)
}

/// COM 物件與 [`CandidateHost`] 共用的資料。
#[derive(Default)]
struct ElementData {
    pages: CandidatePages,
    document: Option<ITfDocumentMgr>,
    shown: bool,
    /// 上一次變更的欄位；初始值是 [`ALL_UPDATED`]。
    flags: u32,
    /// 送出點選請求的通道；`None` 代表候選窗停用。
    actions: Option<Rc<CandidateActions>>,
    /// 這次候選是否已經送出點選鍵或 `Finalize`／`Abort`，避免重複送鍵。
    settled: bool,
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
#[implement(
    ITfUIElement,
    ITfCandidateListUIElement,
    ITfCandidateListUIElementBehavior,
    Agile = false
)]
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

impl ITfCandidateListUIElementBehavior_Impl for CandidateListElement_Impl {
    fn SetSelection(&self, nindex: u32) -> Result<()> {
        let data = self.get_impl().data();
        let Some(actions) = data.actions.clone() else {
            return Ok(());
        };
        let count = data.pages.count();
        if nindex >= count {
            // 清單已經變短（例如引擎回了新的 State），這次點選沒有目標。
            logging::debug(format!("候選索引 {nindex} 超出 {count} 項，忽略點選"));
            return Ok(());
        }
        if !actions.connected() {
            logging::debug("引擎未連線，忽略候選點選");
            return Ok(());
        }
        // 數字鍵由核心換算成「核心目前頁」的候選；點到別頁的項目會選錯字，
        // 因此只接受落在目前頁的索引。
        let page = CANDIDATE_PAGE_SIZE.max(1);
        let current = data.pages.selection() as usize / page;
        let clicked = nindex as usize / page;
        if clicked != current {
            logging::debug(format!(
                "候選索引 {nindex} 不在目前頁（第 {current} 頁），忽略點選"
            ));
            return Ok(());
        }
        let Some(key) = selection_key(nindex) else {
            logging::debug(format!("候選索引 {nindex} 沒有對應的數字鍵，忽略點選"));
            return Ok(());
        };
        drop(data);

        // 數字鍵會直接選取並提交，之後的 `Finalize` 不能再補 `Space`。
        self.get_impl().data_mut().settled = true;
        logging::debug(format!("候選點選 index={nindex}，送數字鍵 {:?}", key.code));
        actions.send_keys(&[key]);
        Ok(())
    }

    fn Finalize(&self) -> Result<()> {
        let data = self.get_impl().data();
        // `SetSelection` 已送數字鍵（選取並提交）時不再補鍵；Windows 對
        // 單擊通常是先 `SetSelection` 再 `Finalize`。
        if data.settled || data.pages.count() == 0 {
            return Ok(());
        }
        let Some(actions) = data.actions.clone() else {
            return Ok(());
        };
        drop(data);

        self.get_impl().data_mut().settled = true;
        // 只送「選取目前高亮項」的請求；要提交什麼字由引擎決定。
        logging::debug("候選點選確認，送 Space");
        actions.send_keys(&[confirm_key()]);
        Ok(())
    }

    fn Abort(&self) -> Result<()> {
        let data = self.get_impl().data();
        if data.settled {
            return Ok(());
        }
        let Some(actions) = data.actions.clone() else {
            return Ok(());
        };
        drop(data);

        self.get_impl().data_mut().settled = true;
        logging::debug("候選點選取消，送 Escape");
        actions.send_keys(&[abort_key()]);
        Ok(())
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
    /// 滑鼠點選要用的請求通道。
    actions: Rc<CandidateActions>,
    /// `BeginUIElement` 配到的識別碼。
    id: u32,
    /// 是否已 `BeginUIElement` 且尚未 `EndUIElement`。
    started: bool,
}

impl CandidateHost {
    /// 建立宿主；`manager` 為 `None` 時整個候選窗等於停用。
    pub fn new(manager: Option<ITfUIElementMgr>, actions: Rc<CandidateActions>) -> Self {
        Self {
            manager,
            element: None,
            data: None,
            actions,
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
            actions: Some(Rc::clone(&self.actions)),
            settled: false,
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

    /// 頁面大小來自核心公開常數，不是 wrapper 自己寫死的數字。
    #[test]
    fn pages_hold_nine_candidates() {
        let texts: Vec<String> = (0..20).map(|index| index.to_string()).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let pages = CandidatePages::new(&snapshot(&refs, 0));
        assert_eq!(pages.starts(), vec![0, 9, 18]);
        assert_eq!(pages.count(), 20);
        assert_eq!(pages.current_page(), 0);
        assert_eq!(CANDIDATE_PAGE_SIZE, 9);
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

    /// 點選第 `n` 項只送一個數字鍵，`n + 1` 對應 `1`–`9`；不再送 `Down`。
    #[test]
    fn click_sends_one_digit_key() {
        let digits: Vec<char> = (0..CANDIDATE_PAGE_SIZE as u32)
            .map(
                |index| match selection_key(index).expect("頁內索引有數字鍵").code {
                    KeyCode::Char(ch) => ch,
                    other => panic!("第 {index} 項不是字元鍵：{other:?}"),
                },
            )
            .collect();
        assert_eq!(digits, vec!['1', '2', '3', '4', '5', '6', '7', '8', '9']);
        assert_eq!(CANDIDATE_PAGE_SIZE, 9, "頁面大小來自核心常數");
    }

    /// 數字鍵是單一字元鍵、不帶 Shift／Ctrl，等同鍵盤按 `1`–`9`。
    #[test]
    fn digit_keys_look_like_keyboard_digits() {
        let key = selection_key(2).expect("第 3 項有數字鍵");
        assert_eq!(key.code, KeyCode::Char('3'));
        assert_eq!(key.text, Some('3'));
        assert!(!key.shift);
        assert!(!key.ctrl);
    }

    /// 索引先取頁內位置，所以第 2 頁的第 1 項同樣送 `1`。
    #[test]
    fn click_index_is_page_relative() {
        let page = CANDIDATE_PAGE_SIZE as u32;
        assert_eq!(
            selection_key(page).map(|key| key.code),
            Some(KeyCode::Char('1'))
        );
        assert_eq!(
            selection_key(page + 1).map(|key| key.code),
            Some(KeyCode::Char('2'))
        );
    }

    /// 頁面大小 10 時第 10 項是 `0`；現行頁面大小用不到，但換算不寫死。
    #[test]
    fn tenth_item_maps_to_zero() {
        assert_eq!(digit_for(9), Some('0'));
        assert_eq!(digit_for(8), Some('9'));
        assert_eq!(digit_for(0), Some('1'));
        assert_eq!(digit_for(10), None, "超過 10 項沒有對應鍵");
    }

    /// 確認／取消鍵沒有變：`Finalize` 還是 `Space`，`Abort` 還是 `Escape`。
    #[test]
    fn confirm_and_abort_use_engine_keys() {
        assert_eq!(confirm_key().code, KeyCode::Space);
        assert_eq!(abort_key().code, KeyCode::Escape);
    }
}
