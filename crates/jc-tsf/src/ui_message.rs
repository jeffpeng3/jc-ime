//! 候選窗的呈現訊息型別。
//!
//! 原本由 `jc-ui` crate 提供；候選窗改成 DLL 內 [`crate::ui_window::LocalWindow`]
//! 自繪後，只保留 TSF 實際用到的型別與常數，不再有獨立行程或 named pipe。

/// 候選的一列。`id` 供選字回傳，`note` 是低調附註，`key` 是鍵標。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiCandidate {
    pub id: i64,
    pub text: String,
    /// 低調附註（模糊音近音為「近音 ㄓ」）；沒有附註時為空字串。
    pub note: String,
    /// 鍵標：核心給的選字鍵（1、q、;）；`Some("")` 不畫。
    pub key: Option<String>,
}

/// 要畫在候選窗上的呈現訊息：畫什麼、畫在哪。不含文件文字。
#[derive(Clone, Debug, Default)]
pub struct UiMessage {
    /// false：隱藏候選窗（其餘欄位忽略）。
    pub show: bool,
    /// 游標的螢幕座標矩形（實體像素）。
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub candidates: Vec<UiCandidate>,
    /// 反白的候選在 candidates 裡的索引。
    pub highlight: i64,
    /// 1 起算的頁碼與總頁數。
    pub page: i64,
    pub pages: i64,
    pub columns: usize,
    pub kind: String,
    pub title: String,
    /// 選單種類：`symbols` 或 `emoji`；沒有選單時空字串。
    pub menu_kind: String,
    /// 選單分頁列上方的標題；沒有選單時空字串。
    pub menu_title: String,
    /// 選單分頁名稱，順序即顯示順序；沒有選單時空陣列。
    pub menu_tabs: Vec<String>,
    /// 目前選單分頁索引，從 0 起算。
    pub menu_tab: usize,
    /// Tab 補全提示（一般為後半；同音替換時為「原文 → 完整詞」）。
    pub suggestion: String,
    /// 核心的一次性提示，畫在最上面；幾秒後或下一鍵後消失。
    pub notice: String,
    /// 組字區還在標示要加入詞庫的範圍：這時的提示是標示的說明，不自動收起。
    pub marking: bool,
    /// 讀音提示：中英混打時照字面顯示的數字按空白會組成的注音。
    pub reading: String,
    /// 送出者（宿主行程 ID）；不同宿主的訊息互相取代。
    pub owner: u32,
    /// 同一宿主 TSF 執行緒的訊息視窗；滑鼠選字要把 WM_SELECT_CANDIDATE 送回這裡。
    pub target: u64,
    pub revision: u32,
}

/// UI → TSF：wParam 是畫面版本（revision），lParam 是候選 ID；不傳文字或指標。
pub const WM_SELECT_CANDIDATE: u32 = 0x8000 + 55;

/// 提示顯示多久後由候選窗自己收起。
pub const NOTICE_MS: u64 = 4000;

/// 提示是否要計時自動收起：有提示、且不是標示中的說明。
pub fn notice_expires(m: &UiMessage) -> bool {
    !m.notice.is_empty() && !m.marking
}
