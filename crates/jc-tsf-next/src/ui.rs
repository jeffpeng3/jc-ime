//! 候選窗訊息與 pipe 用戶端（T12b）。移植 0c `ui.rs` 的「只寫不讀、短逾時」設計。
//!
//! T12a 尚未合併，這裡照 0c `zuin_win_common` 的形狀定義 [`UiMessage`]／[`UiCandidate`]／
//! [`WM_SELECT_CANDIDATE`] 與 `jc-ime-ui-<SID>` pipe 名稱。**T12a 接合點**：等 `crates/jc-ui`
//! 的 `message.rs` 落地後，把本檔的型別換成 `jc_ui::message::{UiMessage, UiCandidate, ...}` 的
//! re-export（欄位逐字相同，wire 格式不用改），並把 [`UiClient::present`] 裡預留的宿主行程內
//! 繪製（0c 的 `in_process`／`present_local`）接上 jc-ui 的 candidate 模組。
//!
//! 送不出去只是看不到候選窗，不影響組字與提交；pipe 寫入一律 200ms 逾時，不在宿主 UI 執行緒等待。

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_BUSY, HANDLE, HWND};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_GENERIC_WRITE, FILE_SHARE_MODE, OPEN_EXISTING,
};
use windows::Win32::System::Pipes::WaitNamedPipeW;
use windows::Win32::UI::TextServices::ITfDocumentMgr;
use windows_core::PCWSTR;

use crate::log::zlog;
use crate::uielement::Candidates;

/// UI → TSF：wParam 是畫面版本，lParam 是候選 ID；不傳文字或指標。
pub const WM_SELECT_CANDIDATE: u32 = 0x8000 + 55;
/// 提示在候選窗自己收起的時間；重送舊畫面時超過這個時間就不再讓提示跳出來。
pub const NOTICE_MS: u64 = 4000;
/// 與 0c 相同的管線寫入逾時。
const SEND_TIMEOUT_MS: u32 = 200;

/// 候選窗的一列。欄位與 0c `UiCandidate` 逐字相同。
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct UiCandidate {
    pub id: i64,
    pub text: String,
    /// 低調附註（模糊音近音）；沒有時空字串。
    #[serde(default)]
    pub note: String,
    /// 核心給的選字鍵；`Some("")` 不畫。舊 UI 沒收到時依位置畫 1–0。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// DLL 送給候選窗的呈現訊息：畫什麼、畫在哪。不含文件文字，不含組字以外的內容。
/// 欄位與 0c `UiMessage` 逐字相同，T12a 可以直接換成共用型別。
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
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
    #[serde(default)]
    pub columns: usize,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    /// Tab 補全提示。
    pub suggestion: String,
    /// 一次性提示，幾秒後或下一鍵後消失。
    #[serde(default)]
    pub notice: String,
    /// 標示要加入詞庫的範圍時，提示是標示說明，不要自動收起。
    #[serde(default)]
    pub marking: bool,
    /// 讀音提示（中英混打時照字面顯示的數字可組成的注音）。
    #[serde(default)]
    pub reading: String,
    /// 送出者（宿主行程 ID）；不同宿主的訊息互相取代。
    pub owner: u32,
    /// 同一宿主 TSF 執行緒的訊息視窗；候選窗用 `WM_SELECT_CANDIDATE` 回送點選。
    #[serde(default)]
    pub target: u64,
    #[serde(default)]
    pub revision: u32,
}

/// 呈現訊息是否有東西可畫。
pub fn has_content(m: &UiMessage) -> bool {
    !m.candidates.is_empty()
        || !m.suggestion.is_empty()
        || !m.notice.is_empty()
        || !m.reading.is_empty()
}

/// 提示是否要計時自動收起（標示中的說明不收）。
pub fn notice_expires(m: &UiMessage) -> bool {
    !m.notice.is_empty() && !m.marking
}

/// 候選窗 pipe 的完整 Win32 路徑。`jc-ime-ui-` 前綴避免與 0c-ui 衝突（T12a 指定）。
pub fn ui_pipe() -> String {
    match crate::ipc::user_sid() {
        Some(sid) => format!(r"\\.\pipe\jc-ime-ui-{sid}"),
        None => r"\\.\pipe\jc-ime-ui".to_string(),
    }
}

/// 候選窗 pipe 用戶端。只寫不讀，短逾時；ui 不在就安靜略過。
pub struct UiClient {
    pipe: Vec<u16>,
    handle: Option<HANDLE>,
    /// 目前是否已經讓候選窗顯示，用來省掉每鍵一次隱藏訊息。
    shown: bool,
    /// UI-less 宿主自行繪製候選的清單元件；Activate 到 Deactivate 之間存在。
    elements: Option<CandidatesHolder>,
}

/// `uielement::Elements` 的別名，讓本檔不需要重複寫它的生命週期註解。
type CandidatesHolder = crate::uielement::Elements;

impl Drop for UiClient {
    fn drop(&mut self) {
        self.close();
    }
}

impl UiClient {
    pub fn new() -> UiClient {
        UiClient {
            pipe: crate::ipc::wide(&ui_pipe()),
            handle: None,
            shown: false,
            elements: None,
        }
    }

    /// Activate 時設定、Deactivate 時清除（清除前結束仍在顯示的元件）。
    pub fn set_elements(&mut self, elements: Option<crate::uielement::Elements>) {
        self.elements = elements;
    }

    fn close(&mut self) {
        if let Some(handle) = self.handle.take() {
            unsafe {
                let _ = CloseHandle(handle);
            }
        }
    }

    fn connect_once(&mut self) -> Result<(), u32> {
        unsafe {
            match CreateFileW(
                PCWSTR::from_raw(self.pipe.as_ptr()),
                FILE_GENERIC_WRITE.0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            ) {
                Ok(handle) => {
                    self.handle = Some(handle);
                    Ok(())
                }
                Err(error) => Err(error.code().0 as u32),
            }
        }
    }

    fn connect(&mut self) -> bool {
        if self.handle.is_some() {
            return true;
        }
        match self.connect_once() {
            Ok(()) => true,
            Err(code) if code == ERROR_PIPE_BUSY.0 => {
                unsafe {
                    let _ = WaitNamedPipeW(PCWSTR::from_raw(self.pipe.as_ptr()), SEND_TIMEOUT_MS);
                }
                self.connect_once().is_ok()
            }
            // ui 尚未啟動：T12a 合併後在 warm_up 裡負責 spawn jc-ui.exe；這裡只安靜略過。
            Err(_) => false,
        }
    }

    /// 呈現候選窗。UI-less 宿主自行畫時只更新候選清單元件，不顯示自己的候選窗（#159）。
    ///
    /// T12a 接合點：0c 在沙箱與沉浸式介面（owner 的 window band > 1）於宿主行程內建候選窗；
    /// 新 crate 還沒有 jc-ui 的繪製模組，暫時一樣走 pipe，等 T12a 補 `in_process`／`present_local`。
    pub fn present(
        &mut self,
        message: &UiMessage,
        _owner: Option<HWND>,
        document: Option<ITfDocumentMgr>,
    ) {
        let draw = self.elements.as_mut().map_or(true, |elements| {
            elements.present(&Candidates::from(message), document)
        });
        if !draw {
            self.hide_windows();
            return;
        }
        self.send(message);
    }

    /// 送出呈現訊息給候選窗。隱藏訊息只在目前顯示中才送，省掉每鍵一次寫入。
    fn send(&mut self, message: &UiMessage) {
        if !message.show && !self.shown {
            return;
        }
        let Ok(value) = serde_json::to_value(message) else {
            return;
        };
        let Some(bytes) = crate::ipc::frame(&value) else {
            return;
        };
        // ui 升級交接後手上連線寫入會失敗：要顯示的訊息重連一次再送；隱藏訊息不必重試。
        for retry in [false, true] {
            if !self.connect() {
                return;
            }
            let Some(handle) = self.handle else { return };
            if crate::ipc::write_all(handle, &bytes, SEND_TIMEOUT_MS) {
                self.shown = message.show;
                return;
            }
            zlog!("ui: 寫入失敗，斷線");
            self.close();
            self.shown = false;
            if retry || !message.show {
                return;
            }
        }
    }

    /// 候選窗關閉：結束 UI 元件並送隱藏訊息。
    pub fn hide(&mut self) {
        if let Some(elements) = self.elements.as_mut() {
            elements.end();
        }
        self.hide_windows();
    }

    fn hide_windows(&mut self) {
        self.send(&UiMessage {
            show: false,
            owner: std::process::id(),
            ..Default::default()
        });
    }
}
