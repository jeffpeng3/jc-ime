//! 游標定位策略與插入點探測（T7a 的過渡 `thread_has_caret` 改接這裡）。
//!
//! 候選窗位置的來源依序是：`GetTextExt` 的精確結果、系統插入點、同一次組字
//! 上一次的位置、這個視窗上一次的位置。每一種「拿不到」只記一次診斷，不記文字。

use std::cell::RefCell;
use std::collections::HashSet;

use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{GetGUIThreadInfo, GUITHREADINFO};

/// 候選窗位置是從哪裡來的（只進 tsf.log，不進 UI）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// GetTextExt 的精確結果。
    Exact,
    /// 系統插入點。
    SystemCaret,
    /// 同一次組字上一次的位置。
    Previous,
    /// 同一個視窗上一次的位置或輸入框範圍。
    AroundField,
}

impl Source {
    /// 記錄用的名字。
    pub fn name(self) -> &'static str {
        match self {
            Source::Exact => "GetTextExt",
            Source::SystemCaret => "系統插入點",
            Source::Previous => "上一次位置",
            Source::AroundField => "輸入框範圍",
        }
    }
}

/// 同一種診斷只記一次（做到 0c `caret::Notes` 的去重語意）。
#[derive(Default)]
pub struct Notes {
    seen: RefCell<HashSet<String>>,
}

impl Notes {
    /// 第一次看到 `key` 時呼叫 `message` 並記下來。
    pub fn once(&self, key: &str, message: impl FnOnce() -> Option<String>) {
        if self.seen.borrow().contains(key) {
            return;
        }
        if let Some(msg) = message() {
            self.seen.borrow_mut().insert(key.to_owned());
            crate::log::log(&msg);
        }
    }
}

/// 呼叫端執行緒（宿主 UI 執行緒）有沒有系統插入點。唯讀、不送按鍵。
pub fn thread_has_caret() -> bool {
    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        if GetGUIThreadInfo(GetCurrentThreadId(), &mut info).is_err() || info.hwndCaret.is_invalid()
        {
            return false;
        }
        let mut point = POINT {
            x: info.rcCaret.left,
            y: info.rcCaret.top,
        };
        ClientToScreen(info.hwndCaret, &mut point).as_bool()
    }
}

/// 同一次組字的上一次位置或這個視窗上一次的位置退路。
/// 沒有輸入框範圍的探測時退回 `recent`（0c 的 `around_field` 在拿不到 view 時也是這個退路）。
pub fn around_field(recent: Option<RECT>) -> Option<(RECT, Source)> {
    recent.map(|r| (r, Source::AroundField))
}

/// 依序挑選候選窗位置：`located` → `system` → `previous` → `around`。
pub fn choose(
    located: Result<RECT, crate::edit::Miss>,
    system: impl FnOnce() -> Option<RECT>,
    previous: Option<RECT>,
    around: impl FnOnce() -> Option<(RECT, Source)>,
) -> Option<(RECT, Source)> {
    if let Ok(r) = located {
        return Some((r, Source::Exact));
    }
    if let Some(r) = system() {
        return Some((r, Source::SystemCaret));
    }
    if let Some(r) = previous {
        return Some((r, Source::Previous));
    }
    around()
}
