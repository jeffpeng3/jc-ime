//! 不接 UI 的宿主實作，供核心與測試在沒有視窗的情況下跑完整流程。

use serde::{Deserialize, Serialize};

use crate::host::{HostCall, HostError, InputHost};

/// 什麼都不做的宿主，只回報成功。
///
/// 它通常不是終點，而是 [`RecordingHost`] 的底層；直接使用時等於丟棄
/// 所有畫面更新。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NullHost;

impl InputHost for NullHost {
    fn update_preedit(&mut self, _text: &str, _cursor_utf16: usize) -> Result<(), HostError> {
        Ok(())
    }

    fn clear_preedit(&mut self) -> Result<(), HostError> {
        Ok(())
    }

    fn show_candidates(&mut self, _items: &[String], _highlight: usize) -> Result<(), HostError> {
        Ok(())
    }

    fn commit(&mut self, _text: &str) -> Result<(), HostError> {
        Ok(())
    }

    fn show_status(&mut self, _text: &str) -> Result<(), HostError> {
        Ok(())
    }
}

/// 把呼叫依序記錄下來，再轉給內層宿主的包裝層。
///
/// 記錄的是核心送出的呼叫，不代表內層宿主執行成功；內層回傳錯誤時仍然
/// 保留該筆記錄，除錯時才看得到失敗的嘗試。整份記錄可以用
/// [`RecordingHost::to_json`] 輸出。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RecordingHost<H = NullHost> {
    inner: H,
    calls: Vec<HostCall>,
}

/// 沒有任何 UI 的宿主：呼叫只會被記錄，不會顯示。
///
/// 等於 [`RecordingHost<NullHost>`]，測試可直接斷言 [`HeadlessHost::calls`]。
pub type HeadlessHost = RecordingHost<NullHost>;

impl RecordingHost<NullHost> {
    /// 建立一個不接 UI、從空記錄開始的宿主。
    pub fn new() -> Self {
        Self::with_inner(NullHost)
    }
}

impl<H> RecordingHost<H> {
    /// 用指定的內層宿主建立包裝。
    pub fn with_inner(inner: H) -> Self {
        Self {
            inner,
            calls: Vec::new(),
        }
    }

    /// 內層宿主。
    pub fn inner(&self) -> &H {
        &self.inner
    }

    /// 內層宿主（可變）。
    pub fn inner_mut(&mut self) -> &mut H {
        &mut self.inner
    }

    /// 取出內層宿主，記錄一併丟棄。
    pub fn into_inner(self) -> H {
        self.inner
    }

    /// 目前為止的呼叫記錄，順序與核心呼叫一致。
    pub fn calls(&self) -> &[HostCall] {
        &self.calls
    }

    /// 取出呼叫記錄。
    pub fn into_calls(self) -> Vec<HostCall> {
        self.calls
    }

    /// 清空記錄，內層宿主保留。
    pub fn clear_log(&mut self) {
        self.calls.clear();
    }

    /// 複製一份記錄，方便序列化或比對。
    pub fn log(&self) -> RecordingLog {
        RecordingLog {
            calls: self.calls.clone(),
        }
    }

    /// 把記錄序列化成單行 JSON。
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.log())
    }

    /// 把記錄序列化成縮排 JSON，方便人工閱讀。
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&self.log())
    }
}

impl<H: InputHost> InputHost for RecordingHost<H> {
    fn update_preedit(&mut self, text: &str, cursor_utf16: usize) -> Result<(), HostError> {
        self.calls.push(HostCall::Preedit {
            text: text.to_owned(),
            cursor_utf16,
        });
        self.inner.update_preedit(text, cursor_utf16)
    }

    fn clear_preedit(&mut self) -> Result<(), HostError> {
        self.calls.push(HostCall::ClearPreedit);
        self.inner.clear_preedit()
    }

    fn show_candidates(&mut self, items: &[String], highlight: usize) -> Result<(), HostError> {
        self.calls.push(HostCall::Candidates {
            items: items.to_vec(),
            highlight,
        });
        self.inner.show_candidates(items, highlight)
    }

    fn commit(&mut self, text: &str) -> Result<(), HostError> {
        self.calls.push(HostCall::Commit {
            text: text.to_owned(),
        });
        self.inner.commit(text)
    }

    fn show_status(&mut self, text: &str) -> Result<(), HostError> {
        self.calls.push(HostCall::Status {
            text: text.to_owned(),
        });
        self.inner.show_status(text)
    }
}

/// 一份可序列化的呼叫記錄。
///
/// JSON 形狀固定為 `{"calls":[...]}`，每筆的 `call` 欄位對應
/// [`HostCall`] 的變體。
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingLog {
    /// 依發生順序排列的呼叫。
    pub calls: Vec<HostCall>,
}
