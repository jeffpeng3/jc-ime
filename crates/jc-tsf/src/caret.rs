//! 游標定位策略與插入點探測（T7a 的過渡 `thread_has_caret` 改接這裡）。
//!
//! 候選窗位置的來源依序是：`GetTextExt` 的精確結果、系統插入點、同一次組字
//! 上一次的位置、這個視窗上一次的位置。每一種「拿不到」只記一次診斷，不記文字。

use std::cell::RefCell;
use std::collections::HashSet;

use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{GetCaretPos, GetGUIThreadInfo, GUITHREADINFO};

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

/// 系統插入點：`GetTextExt` 失敗時的退路（要求 3 的 `GetCaretPos`）。
///
/// 先問呼叫執行緒自己的插入點；TSF 執行緒通常不持有插入點，再問前景執行緒
/// （`GetGUIThreadInfo(0)`）的 `rcCaret`，那是同一組「插入點矩形」。
pub fn system_caret() -> Option<RECT> {
    unsafe {
        let current = GetCurrentThreadId();
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if GetGUIThreadInfo(current, &mut info).is_ok() && !info.hwndCaret.is_invalid() {
            let mut point = POINT::default();
            if GetCaretPos(&mut point).is_ok()
                && ClientToScreen(info.hwndCaret, &mut point).as_bool()
            {
                return Some(caret_at(point, info.rcCaret));
            }
        }
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if GetGUIThreadInfo(0, &mut info).is_ok() && !info.hwndCaret.is_invalid() {
            let mut point = POINT {
                x: info.rcCaret.left,
                y: info.rcCaret.top,
            };
            if ClientToScreen(info.hwndCaret, &mut point).as_bool() {
                return Some(caret_at(point, info.rcCaret));
            }
        }
        None
    }
}

/// 插入點左上角（螢幕座標）與 GUI 執行緒回報的大小；寬高至少 1，空矩形不算。
unsafe fn caret_at(point: POINT, client: RECT) -> RECT {
    let width = (client.right - client.left).max(1);
    let height = (client.bottom - client.top).max(1);
    RECT {
        left: point.x,
        top: point.y,
        right: point.x + width,
        bottom: point.y + height,
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

#[cfg(test)]
mod tests {
    //! 移植自 0c `caret.rs:191-276`。T6 的 service 移植把本模組縮成「退路順序 + Notes 去重」
    //! 四個函式；0c 的 `around_field(field, recent, line)` 幾何、`system_caret`、`field`、
    //! `line_height` 與 `Source::Recent/Field` 已不存在，相關子案例在回報列为 n/a。
    use super::*;
    use windows::Win32::UI::TextServices::TS_E_NOLAYOUT;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn missing_layout_falls_back_instead_of_hiding_candidates() {
        let caret = rect(100, 200, 101, 220);
        let older = rect(80, 200, 81, 220);
        let field_rect = rect(50, 180, 600, 240);
        let missing = Err(crate::edit::Miss::Failed(TS_E_NOLAYOUT));
        // GetTextExt 有結果：其他來源都不查。
        assert_eq!(
            choose(Ok(caret), || unreachable!(), Some(older), || unreachable!()),
            Some((caret, Source::Exact))
        );
        // 有系統插入點的程式照舊用它，即使同一次組字有較舊的位置（#144 之前的行為）。
        assert_eq!(
            choose(missing, || Some(caret), Some(older), || unreachable!()),
            Some((caret, Source::SystemCaret))
        );
        // Chromium 新打的字版面還沒回來（TS_E_NOLAYOUT）、沒有系統插入點：用同一次組字上一次的位置。
        assert_eq!(
            choose(
                Err(crate::edit::Miss::Failed(TS_E_NOLAYOUT)),
                || None,
                Some(caret),
                || unreachable!()
            ),
            Some((caret, Source::Previous))
        );
        // 組字的第一個字：退到輸入框範圍，候選窗不能不出現（#144）。
        assert_eq!(
            choose(
                Err(crate::edit::Miss::Failed(TS_E_NOLAYOUT)),
                || None,
                None,
                || around_field(Some(field_rect))
            ),
            Some((field_rect, Source::AroundField))
        );
        // 沒有任何位置就真的沒有位置（空輸入框範圍不算）。
        assert_eq!(
            choose(
                Err(crate::edit::Miss::Failed(TS_E_NOLAYOUT)),
                || None,
                None,
                || around_field(None)
            ),
            None
        );
    }

    #[test]
    fn notes_are_keyed_by_class_and_details_are_built_once() {
        let notes = Notes::default();
        let mut built = 0;
        for x in 0..100 {
            // 座標每次不同，類別相同：只組一次字串、只記一次，不會把上限吃光。
            notes.once("empty", || {
                built += 1;
                Some(format!("空矩形 ({x},0,0,0)"))
            });
        }
        assert_eq!(built, 1);
        notes.once("foreground", || Some("前景執行緒沒有插入點".into()));
        notes.once("foreground", || unreachable!());
        assert_eq!(notes.seen.borrow().len(), 2);
    }

    #[test]
    fn system_caret_without_window_does_not_panic() {
        // 測試行程沒有插入點；只確認探測無效視窗不會出錯。
        let _ = thread_has_caret();
        assert_eq!(Source::Exact.name(), "GetTextExt");
        assert_eq!(Source::SystemCaret.name(), "系統插入點");
        assert_eq!(Source::Previous.name(), "上一次位置");
        assert_eq!(Source::AroundField.name(), "輸入框範圍");
    }
}
