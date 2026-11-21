//! 候選窗訊息與 pipe 用戶端（T12b）。移植 0c `ui.rs` 的「只寫不讀、短逾時」設計。
//!
//! 協定型別 [`UiMessage`]／[`UiCandidate`]／[`WM_SELECT_CANDIDATE`]／pipe 名稱都由
//! `crates/jc-ui` 的 `message` 模組提供（T12a），DLL 與候選窗程式共用同一份定義，
//! 避免 wire 格式漂移。本檔只保留 DLL 側的 [`UiClient`]。
//!
//! 送不出去只是看不到候選窗，不影響組字與提交；pipe 寫入一律 200ms 逾時，不在宿主 UI 執行緒等待。

use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_BUSY, HANDLE, HWND};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_GENERIC_WRITE, FILE_SHARE_MODE, OPEN_EXISTING,
};
use windows::Win32::System::Pipes::WaitNamedPipeW;
use windows::Win32::UI::TextServices::ITfDocumentMgr;
use windows_core::PCWSTR;

use crate::log::zlog;
use crate::uielement::Candidates;

// 與候選窗程式共用 jc_ui 的協定型別，不在DLL 側重複定義。
pub use jc_ui::message::{
    has_content, notice_expires, ui_pipe, UiCandidate, UiMessage, WM_SELECT_CANDIDATE,
};

/// 提示在候選窗自己收起的時間（毫秒）。jc_ui 定義為 u32，這裡轉成 Duration 用的 u64。
pub const NOTICE_MS: u64 = jc_ui::message::NOTICE_MS as u64;
/// 與 0c 相同的管線寫入逾時。
const SEND_TIMEOUT_MS: u32 = 200;

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
            pipe: jc_ui::message::wide(&ui_pipe()),
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
        let Some(bytes) = jc_ui::message::frame(message) else {
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
