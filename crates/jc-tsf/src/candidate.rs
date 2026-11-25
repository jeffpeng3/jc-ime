//! 候選窗的回呼視窗與點選edit session（T12b）。
//!
//! 移植自 0c `candidate.rs`（main `8f443af`）的生產碼。候選窗只回送畫面版本與候選 ID，
//! 由原 TSF 執行緒核對焦點後再送核心；`WM_TIMER` 是焦點移到 NULL 後「等訊息佇列空下來再確認」的
//! 三態決策需要的（T6 的 service 保留了 `settle_focus`，但沒有回呼視窗就無法延後）。
//!
//! 與 0c 的差異：新 crate 沒有 `dll_instance()`，視窗類別用目前執行檔的 HINSTANCE 註冊
//! （類別是行程內的，不帶資源）；`foreground_belongs_to` 自己用 `GetGUIThreadInfo` 判斷 WebView2
//! 這類前景屬於別的行程、但鍵盤焦點在本行程的宿主。

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::TextServices::{
    ITfContext, ITfEditSession, ITfEditSession_Impl, ITfKeyEventSink, TF_ES_ASYNCDONTCARE,
    TF_ES_READWRITE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetForegroundWindow, GetGUIThreadInfo,
    GetWindowLongPtrW, GetWindowThreadProcessId, KillTimer, RegisterClassW, SetTimer,
    SetWindowLongPtrW, UnregisterClassW, GUITHREADINFO, GWLP_USERDATA, HWND_MESSAGE,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_TIMER,
};
use windows_core::{implement, w, ComObject, IUnknownImpl};

use crate::log;
use crate::service::TextService_Impl;
use crate::ui::WM_SELECT_CANDIDATE;

const CLASS: windows_core::PCWSTR = w!("jcCandidateCallback");
const SETTLE_FOCUS_TIMER: usize = 0x0c18;
/// WM_TIMER 只在訊息佇列沒有其他訊息時才產生：宿主一連串的焦點變化都處理完才輪到它。
const SETTLE_FOCUS_MS: u32 = 50;
/// ライブ変換的停頓計時器：最後一鍵之後等多久才送 `idle`。
///
/// 300ms 沿用 `jc_core::ja::live::Config::idle_delay` 的預設值；外殼只負責
/// 計時，轉換時機仍由核心的停頓設定決定。
const JA_LIVE_IDLE_TIMER: usize = 0x0c19;
const JA_LIVE_IDLE_MS: u32 = 300;

/// windows-rs 0.62 在 32-bit target 把 `SetWindowLongPtrW` alias 成 `SetWindowLongW`
/// （收 `i32`），64-bit 才是真函式（收 `isize`）。這兩個 helper 把差異封在這裡，
/// 呼叫端不用自己猜指標寬度。
#[cfg(target_pointer_width = "64")]
unsafe fn set_window_userdata(hwnd: HWND, service: *const TextService_Impl) {
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, service as isize);
}

/// 32-bit 版本的 [`set_window_userdata`]：`SetWindowLongPtrW` 在 32-bit 收 `i32`。
#[cfg(target_pointer_width = "32")]
unsafe fn set_window_userdata(hwnd: HWND, service: *const TextService_Impl) {
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, service as i32);
}

/// 讀回 `GWLP_USERDATA`；64-bit 的 `GetWindowLongPtrW` 回 `isize`。
#[cfg(target_pointer_width = "64")]
unsafe fn window_userdata(hwnd: HWND) -> isize {
    GetWindowLongPtrW(hwnd, GWLP_USERDATA)
}

/// 讀回 `GWLP_USERDATA`；32-bit 的 `GetWindowLongPtrW` 回 `i32`。
#[cfg(target_pointer_width = "32")]
unsafe fn window_userdata(hwnd: HWND) -> i32 {
    GetWindowLongPtrW(hwnd, GWLP_USERDATA)
}

/// 回呼視窗。持有 service 的 COM 參考，確保視窗還在時 service 不會被卸載。
pub struct CandidateWindow {
    hwnd: HWND,
    instance: HINSTANCE,
    _keep_alive: ITfKeyEventSink,
}

impl CandidateWindow {
    pub fn new(service: &TextService_Impl) -> windows_core::Result<CandidateWindow> {
        unsafe {
            let instance: HINSTANCE = GetModuleHandleW(None).map(Into::into).unwrap_or_default();
            let _ = RegisterClassW(&windows::Win32::UI::WindowsAndMessaging::WNDCLASSW {
                lpfnWndProc: Some(callback),
                hInstance: instance,
                lpszClassName: CLASS,
                ..Default::default()
            });
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                CLASS,
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance),
                None,
            )?;
            set_window_userdata(hwnd, service);
            log::object_created();
            Ok(CandidateWindow {
                hwnd,
                instance,
                _keep_alive: service.to_interface(),
            })
        }
    }

    /// 給 UiMessage.target：候選窗用 `WM_SELECT_CANDIDATE` 把點選送回這個視窗。
    pub fn handle(&self) -> u64 {
        self.hwnd.0 as usize as u64
    }

    /// 稍後（訊息佇列空下來時）再確認一次組字所在的文件還有沒有焦點。
    pub fn settle_focus_later(&self) {
        unsafe {
            let _ = SetTimer(Some(self.hwnd), SETTLE_FOCUS_TIMER, SETTLE_FOCUS_MS, None);
        }
    }

    /// 焦點確定落在組字文件（或已經送出）時取消延後確認。
    pub fn cancel_settle_focus(&self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), SETTLE_FOCUS_TIMER);
        }
    }

    /// 重新起算ライブ変換的停頓；同一個 timer id 會覆蓋上一次的期限。
    pub fn ja_live_idle_later(&self) {
        unsafe {
            let _ = SetTimer(Some(self.hwnd), JA_LIVE_IDLE_TIMER, JA_LIVE_IDLE_MS, None);
        }
    }

    /// 取消ライブ変換的停頓：下一個按鍵、組字結束與 Deactivate 都用得到。
    pub fn cancel_ja_live_idle(&self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), JA_LIVE_IDLE_TIMER);
        }
    }
}

impl Drop for CandidateWindow {
    fn drop(&mut self) {
        unsafe {
            set_window_userdata(self.hwnd, std::ptr::null());
            let _ = DestroyWindow(self.hwnd);
            // 同一行程還有多個 service 時會失敗，留給最後一個離開才移除。
            let _ = UnregisterClassW(CLASS, Some(self.instance));
        }
        log::object_destroyed();
    }
}

unsafe extern "system" fn callback(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_SELECT_CANDIDATE {
        let _ = crate::guarded("CandidateClick", || {
            if let Some(service) = service_at(hwnd) {
                service.select_candidate(wp.0 as u32, lp.0 as i64);
            }
            Ok(())
        });
        LRESULT(0)
    } else if msg == WM_TIMER && wp.0 == SETTLE_FOCUS_TIMER {
        let _ = KillTimer(Some(hwnd), SETTLE_FOCUS_TIMER);
        let _ = crate::guarded("SettleFocus", || {
            if let Some(service) = service_at(hwnd) {
                service.settle_focus();
            }
            Ok(())
        });
        LRESULT(0)
    } else if msg == WM_TIMER && wp.0 == JA_LIVE_IDLE_TIMER {
        let _ = KillTimer(Some(hwnd), JA_LIVE_IDLE_TIMER);
        let _ = crate::guarded("JaIdle", || {
            if let Some(service) = service_at(hwnd) {
                service.ja_live_idle();
            }
            Ok(())
        });
        LRESULT(0)
    } else {
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

/// 視窗的 USERDATA 是建立時的 `&TextService_Impl`，只在同一個 UI 執行緒上讀。
unsafe fn service_at(hwnd: HWND) -> Option<&'static TextService_Impl> {
    let ptr = window_userdata(hwnd) as *const TextService_Impl;
    ptr.as_ref()
}

/// 前景視窗屬於本行程，或前景執行緒的鍵盤焦點在本行程（WebView2，#144）。
pub(crate) fn foreground_belongs_to(pid: u32) -> bool {
    unsafe {
        let foreground = GetForegroundWindow();
        let mut owner = 0;
        let thread = GetWindowThreadProcessId(foreground, Some(&mut owner));
        if owner == pid {
            return true;
        }
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if GetGUIThreadInfo(thread, &mut info).is_err() || info.hwndFocus.0.is_null() {
            return false;
        }
        let mut focus_owner = 0;
        GetWindowThreadProcessId(info.hwndFocus, Some(&mut focus_owner));
        focus_owner == pid
    }
}

/// 點選要在宿主給的 edit cookie 內執行；物件同時持有 service 的 COM 參考直到 session 完成。
#[implement(ITfEditSession)]
struct SelectEdit {
    _keep_alive: ITfKeyEventSink,
    service: *const TextService_Impl,
    revision: u32,
    id: i64,
}

impl Drop for SelectEdit {
    fn drop(&mut self) {
        log::object_destroyed();
    }
}

impl ITfEditSession_Impl for SelectEdit_Impl {
    fn DoEditSession(&self, ec: u32) -> windows_core::Result<()> {
        crate::guarded("SelectEdit", || {
            if let Some(service) = unsafe { self.service.as_ref() } {
                service.select_in_edit(ec, self.revision, self.id);
            }
            Ok(())
        })
    }
}

/// 從候選窗回來的點選：在 context 上排一個讀寫 edit session。
pub fn request(service: &TextService_Impl, ctx: &ITfContext, client: u32, revision: u32, id: i64) {
    log::object_created();
    let obj = ComObject::new(SelectEdit {
        _keep_alive: service.to_interface(),
        service,
        revision,
        id,
    });
    let edit: ITfEditSession = obj.into_interface();
    unsafe {
        let _ = ctx.RequestEditSession(client, &edit, TF_ES_ASYNCDONTCARE | TF_ES_READWRITE);
    }
}
