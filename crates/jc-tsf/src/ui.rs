//! 候選窗呈現（T12b → D2D 自繪）。
//!
//! 協定型別 [`UiMessage`]／[`UiCandidate`]／[`WM_SELECT_CANDIDATE`] 仍由 `crates/jc-ui`
//! 的 `message` 模組提供，DLL 與既有工具共用同一份定義，避免 wire 格式漂移。
//!
//! 候選窗本體改成 TSF DLL 內自繪（[`crate::ui_window::LocalWindow`]）：Direct2D 畫、
//! DirectComposition 上屏，不再送 named pipe 給 `jc-ui.exe`，也不新增任何程序。
//! 初始化失敗只收起候選窗並記一行日誌，不影響組字與提交。
//!
//! UI-less 宿主（`TF_TMAE_UIELEMENTENABLEDONLY`，例如遊戲）仍走
//! [`crate::uielement::Elements`]：宿主自己畫時我們也不畫。

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::TextServices::ITfDocumentMgr;

use crate::log::zlog;
use crate::ui_window::LocalWindow;
use crate::uielement::Candidates;

// 與 jc-ui／舊版 DLL 共用協定型別，不在 DLL 側重複定義。
pub use jc_ui::message::{
    has_content, notice_expires, ui_pipe, UiCandidate, UiMessage, WM_SELECT_CANDIDATE,
};

/// 提示在候選窗自己收起的時間（毫秒）。jc_ui 定義為 u32，這裡轉成 Duration 用的 u64。
pub const NOTICE_MS: u64 = jc_ui::message::NOTICE_MS as u64;

/// 候選窗。畫面在 DLL 內自繪，pipe 只留給既有的外部工具。
pub struct UiClient {
    /// UI-less 宿主自行繪製候選的清單元件；Activate 到 Deactivate 之間存在。
    elements: Option<CandidatesHolder>,
    /// 自繪候選窗；第一次真的要顯示時才建立 GPU 資源。
    window: Option<Box<LocalWindow>>,
    /// 這個 Activate 期間初始化失敗：不再重試，Launcher／下一次 Activate 才有機會。
    failed: bool,
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
            self.hide_window();
            return;
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
