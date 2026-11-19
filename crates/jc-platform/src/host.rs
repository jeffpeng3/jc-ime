//! 核心推給 wrapper 的呼叫，以及 wrapper 回送給核心的事件。
//!
//! 這裡只放語意型別，不含任何 OS handle。視窗、輸入法上下文與執行緒模型
//! 全部留在 wrapper，核心看到的只有文字、游標與候選索引。

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::clock::TimerId;

/// 宿主端回報錯誤的方式。
///
/// 核心不能因為宿主端失敗而中止輸入流程；wrapper 收到錯誤後應自行記錄並
/// 讓核心繼續運作，例如候選窗被系統封鎖時仍可送出 `commit`。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HostError {
    /// 宿主端目前無法接受呼叫，例如視窗已關閉或 IPC 已中斷。
    #[error("宿主端無法接受呼叫：{0}")]
    Unavailable(String),
    /// 宿主端理解呼叫但拒絕執行，例如候選窗被系統封鎖。
    #[error("宿主端拒絕呼叫：{0}")]
    Rejected(String),
    /// 宿主端不支援這個操作，例如精簡版 wrapper 沒有狀態列。
    #[error("宿主端不支援操作：{0}")]
    Unsupported(&'static str),
}

impl HostError {
    /// 建立 [`HostError::Unavailable`]。
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::Unavailable(message.into())
    }

    /// 建立 [`HostError::Rejected`]。
    pub fn rejected(message: impl Into<String>) -> Self {
        Self::Rejected(message.into())
    }
}

/// wrapper 提供給核心的宿主介面。
///
/// 每個方法都可能在正常流程中失敗，因此一律回傳 [`Result`]。核心保證：
///
/// - 只在輸入法啟用時呼叫；焦點遺失後核心會先送 `clear_preedit`。
/// - `update_preedit` 的 `cursor_utf16` 是 `text` 的 UTF-16 位移，範圍是
///   `0..=text.encode_utf16().count()`。
/// - `show_candidates` 的 `highlight` 在 `items` 非空時小於 `items.len()`；
///   `items` 為空時一律是 `0`。
/// - `commit` 之後核心會清掉組字狀態，wrapper 不需要自己再送
///   `clear_preedit`。
/// - `show_status` 是提示而非同步狀態；空字串代表清除提示。
pub trait InputHost {
    /// 更新組字區文字與游標（游標以 UTF-16 位移表示）。
    fn update_preedit(&mut self, text: &str, cursor_utf16: usize) -> Result<(), HostError>;

    /// 清除組字區。焦點遺失、取消組字或 `commit` 後由核心呼叫。
    fn clear_preedit(&mut self) -> Result<(), HostError>;

    /// 顯示候選清單，`highlight` 是目前高亮的索引。
    fn show_candidates(&mut self, items: &[String], highlight: usize) -> Result<(), HostError>;

    /// 送出確定的文字。
    fn commit(&mut self, text: &str) -> Result<(), HostError>;

    /// 顯示狀態提示，例如「學習中」「模型未載入」。
    fn show_status(&mut self, text: &str) -> Result<(), HostError>;
}

/// 一筆推給 wrapper 的呼叫，順序與核心實際呼叫一致。
///
/// 這個型別同時是測試斷言的對象與除錯用的 JSON 格式；欄位命名即契約，
/// 不要因為 wrapper 內部實作而調整。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case")]
pub enum HostCall {
    /// 對應 [`InputHost::update_preedit`]。
    Preedit {
        /// 組字區文字。
        text: String,
        /// 游標在 `text` 中的 UTF-16 位移。
        cursor_utf16: usize,
    },
    /// 對應 [`InputHost::show_candidates`]。
    Candidates {
        /// 候選詞面，已經依顯示順序排好。
        items: Vec<String>,
        /// 高亮索引；`items` 為空時為 `0`。
        highlight: usize,
    },
    /// 對應 [`InputHost::commit`]。
    Commit {
        /// 送出的確定文字。
        text: String,
    },
    /// 對應 [`InputHost::clear_preedit`]。
    ClearPreedit,
    /// 對應 [`InputHost::show_status`]。
    Status {
        /// 提示文字；空字串代表清除。
        text: String,
    },
}

/// wrapper 可能送給核心的事件。
///
/// 這一輪只定型別，核心端如何處理留給 session／engine 的 task。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HostEvent {
    /// 輸入焦點離開；核心應結束組字並丟棄未送出的內容。
    FocusLost,
    /// 輸入焦點回到同一個輸入法上下文。
    FocusGained,
    /// 使用者點了候選清單的第 `index` 項（0 起算，指整個候選清單）。
    CandidateClicked {
        /// 被點選的候選索引。
        index: usize,
    },
    /// 使用者在候選窗上滾動；`delta` 正值代表往下。
    Scroll {
        /// 滾動格數，正負代表方向。
        delta: i32,
    },
    /// 先前用 [`crate::Scheduler::schedule_after`] 排定的計時器到期。
    Timer {
        /// 到期的計時器。
        id: TimerId,
    },
}
