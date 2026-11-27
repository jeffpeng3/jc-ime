//! 候選窗呈現（T12b → D2D 自繪）。
//!
//! 協定型別 [`UiMessage`]／[`UiCandidate`]／[`WM_SELECT_CANDIDATE`] 定義在
//! [`crate::ui_message`]，DLL 內自繪與宿主 UI 元件共用同一份資料。
//!
//! 候選窗本體位於 TSF DLL 內（[`crate::ui_window::LocalWindow`]）：Direct2D 畫、
//! DirectComposition 上屏，不開新程序，也不再送 named pipe。
//! 初始化失敗只收起候選窗並記一行日誌，不影響組字與提交。
//!
//! UI-less 宿主（`TF_TMAE_UIELEMENTENABLEDONLY`，例如遊戲）仍走
//! [`crate::uielement::Elements`]：宿主自己畫時我們也不畫。

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::TextServices::ITfDocumentMgr;

use crate::log::zlog;
use crate::ui_window::LocalWindow;
use crate::uielement::Candidates;

pub use crate::ui_message::{
    notice_expires, UiCandidate, UiMessage, NOTICE_MS, WM_SELECT_CANDIDATE,
};

/// 候選窗。畫面在 DLL 內自繪，沒有外部程序或 pipe。
pub struct UiClient {
    /// UI-less 宿主自行繪製候選的清單元件；Activate 到 Deactivate 之間存在。
    elements: Option<CandidatesHolder>,
    /// 自繪候選窗；第一次真的要顯示時才建立 GPU 資源。
    window: Option<Box<LocalWindow>>,
    /// 這個 Activate 期間初始化失敗：不再重試，Launcher／下一次 Activate 才有機會。
    failed: bool,
    /// 目前是否顯示中；只用來把「顯示／收起」的日誌收成一次。
    shown: bool,
}

/// `uielement::Elements` 的別名，讓本檔不需要重複寫它的生命週期註解。
type CandidatesHolder = crate::uielement::Elements;

impl UiClient {
    // 管道名取自行程環境，Default 沒有額外語意；候選窗一律由文字服務建立。
    #[allow(clippy::new_without_default)]
    pub fn new() -> UiClient {
        UiClient {
            elements: None,
            window: None,
            failed: false,
            shown: false,
        }
    }

    /// Activate 時設定、Deactivate 時清除（清除前結束仍在顯示的元件）。
    pub fn set_elements(&mut self, elements: Option<crate::uielement::Elements>) {
        self.elements = elements;
    }

    /// 呈現候選窗。UI-less 宿主自行畫時只更新候選清單元件，不顯示自己的候選窗（#159）。
    ///
    /// 空候選一律收起：沒有東西可畫就不留一個空框。
    pub fn present(
        &mut self,
        message: &UiMessage,
        _owner: Option<HWND>,
        document: Option<ITfDocumentMgr>,
    ) {
        let draw = self
            .elements
            .as_mut()
            .is_none_or(|elements| elements.present(&Candidates::from(message), document));
        if !draw || !message.show || message.candidates.is_empty() {
            // 排查用：真的有候選卻沒顯示時記一次，分得出是 show=false 還是清單空。
            if !draw {
                crate::log::once("ui: draw=false（UI-less 宿主）");
            } else if !message.show {
                crate::log::once(&format!(
                    "ui: candidates={} 但 show=false",
                    message.candidates.len()
                ));
            }
            self.hide_window();
            return;
        }
        // 只在「顯示／收起」的轉換寫一行，避免每個按鍵都洗版。實機排查時
        // 這一行的有無就分得出「沒被呼叫」與「畫不出來」。
        if !self.shown {
            zlog!(
                "ui: 顯示候選窗 n={} at=({}, {}) {}x{}",
                message.candidates.len(),
                message.x,
                message.y,
                message.w,
                message.h
            );
            self.shown = true;
        }
        self.show_window(message);
    }

    fn show_window(&mut self, message: &UiMessage) {
        if self.failed {
            return;
        }
        if self.window.is_none() {
            match LocalWindow::new() {
                Ok(window) => self.window = Some(window),
                Err(error) => {
                    self.failed = true;
                    zlog!("ui: 候選窗建立失敗，改為不顯示（{error}）");
                    return;
                }
            }
        }
        let Some(window) = self.window.as_mut() else {
            return;
        };
        // `present` 只在 GPU 資源建立失敗時回錯；執行期繪製錯誤它自己收起並重建。
        if let Err(error) = window.present(message) {
            self.failed = true;
            self.window = None;
            zlog!("ui: 候選窗初始化失敗，改為不顯示（{error}）");
        }
    }

    fn hide_window(&mut self) {
        if self.shown {
            zlog!("ui: 收起候選窗");
            self.shown = false;
        }
        if let Some(window) = self.window.as_mut() {
            window.hide();
        }
    }

    /// 候選窗關閉：結束 UI 元件並收起視窗（資源留到 [`Self::shutdown`]）。
    pub fn hide(&mut self) {
        if let Some(elements) = self.elements.as_mut() {
            elements.end();
        }
        self.hide_window();
    }

    /// Deactivate：銷毀視窗與 D2D／D3D／DComp 資源，並清掉失敗旗標。
    ///
    /// `LocalWindow` 的 `Drop` 會釋放所有 COM 參照，`DllCanUnloadNow` 才能回到 0。
    pub fn shutdown(&mut self) {
        self.hide();
        self.window = None;
        self.failed = false;
    }
}
