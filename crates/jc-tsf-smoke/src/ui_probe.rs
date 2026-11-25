//! 檢查 13：UI 探針。
//!
//! 前 12 項只看註冊表、檔案、COM 與 pipe，沒有任何一項證明候選窗真的畫在
//! 螢幕上。這裡建一個 host 視窗加 `EDIT` 子視窗，把本 TIP 設成本行程的作用中
//! 輸入法，用 `SendInput` 打一次真正的注音：
//!
//! 1. `j`、`i`、`3` 組字，再按 `↓` 進入挑選；候選窗只在使用者開始挑選時才開
//!    （`service.rs` 的 `composing && navigating`），只打字不會出現。
//! 2. 每 20ms 用 `FindWindowExW` 找本行程的 `jcCandidateUiWindow`，最多等
//!    2000ms，記錄第一次看到可見視窗的延遲。
//! 3. 量 rect、插入點、虛擬螢幕與插入點所在 monitor 的 work area。
//! 4. 按 `Enter` 提交，讀 EDIT 的文字；必須包含 `ji3` 的第一個候選「我」。
//! 5. 重新打 `j`、`i`、`3`、`↓` 再開一次候選窗，按 `Esc`，最多等 1000ms 確認
//!    消失。Esc 不能在 Enter 之後直接驗，那時視窗已經因為提交而關掉了。
//! 6. 清理：`DestroyWindow`、`UnregisterClassW`，還原 profile 與語言。
//!
//! 判定規則（composition 非空、rect 在虛擬螢幕內、與 work area 相交、與插入點
//! 的距離、Enter 的文字、Esc 後消失）放在 [`crate::ui_probe_rules`]；那裡不碰
//! Win32，Linux 也能跑單元測試。
//!
//! 沒有互動桌面（Session 0、服務工作階段、鎖定中的桌面）時建視窗、取得前景
//! 視窗或送按鍵會失敗；這種情況回 [`Verdict::Headless`]，由呼叫端記成 `[SKIP]`，
//! **不算失敗**。本產品還沒安裝或沒為目前使用者啟用時回
//! [`Verdict::Unavailable`]，同樣是 `[SKIP]`（檢查 10 已經會把這件事判成
//! `[FAIL]`，不要重複紅）。組字空且候選窗完全沒出現時回
//! [`Verdict::Inconclusive`]：TIP 根本沒接到按鍵，這是環境問題，也是 `[SKIP]`。
//! 只要候選窗出現過，Enter 沒上屏、Esc 關不掉或位置不對都是
//! [`Verdict::Fail`]，detail 會帶 cell、所有不成立的地方與
//! `%LOCALAPPDATA%\jc-ime\tsf.log` 的建議。
//!
//! 所有 TSF 狀態都只改本行程：`ActivateProfile` 帶 `TF_IPPMF_FORPROCESS`，
//! 結束時把原本的 profile 與語言還原。

use std::time::{Duration, Instant};

use windows::core::{w, Interface, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::StationsAndDesktops::{
    GetProcessWindowStation, GetUserObjectInformationW, UOI_NAME,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::HiDpi::{
    GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, SetFocus, HKL, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, VIRTUAL_KEY, VK_3, VK_DOWN, VK_ESCAPE,
    VK_I, VK_J, VK_RETURN,
};
use windows::Win32::UI::TextServices::{
    CLSID_TF_InputProcessorProfiles, CLSID_TF_ThreadMgr, ITfInputProcessorProfileMgr,
    ITfInputProcessorProfiles, ITfThreadMgr, GUID_TFCAT_TIP_KEYBOARD, TF_INPUTPROCESSORPROFILE,
    TF_IPPMF_FORPROCESS, TF_PROFILETYPE_INPUTPROCESSOR,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowExW, GetCaretPos,
    GetClientRect, GetForegroundWindow, GetSystemMetrics, GetWindowRect, GetWindowTextW,
    GetWindowThreadProcessId, IsWindowVisible, LoadCursorW, PeekMessageW, RegisterClassW,
    SetForegroundWindow, ShowWindow, TranslateMessage, UnregisterClassW, CS_HREDRAW, CS_VREDRAW,
    IDC_ARROW, MSG, PM_REMOVE, SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_SHOW, WINDOW_EX_STYLE, WNDCLASSW, WS_BORDER, WS_CHILD,
    WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE,
};

use crate::product;
use crate::report::UiProbeCells;
use crate::ui_probe_rules::{self, Observation, Rect};

/// 探針 host 視窗的類別名稱；只在這個行程內註冊。
const HOST_CLASS: PCWSTR = w!("jcUiProbeHost");
/// `crates/jc-tsf/src/ui_window.rs` 的候選窗類別名稱。
const CANDIDATE_CLASS: PCWSTR = w!("jcCandidateUiWindow");

/// 送出按鍵後等候選窗出現的上限。
const CANDIDATE_WAIT: Duration = Duration::from_millis(2000);
/// 送出 Esc 後等候選窗消失的上限。
const ESCAPE_WAIT: Duration = Duration::from_millis(1000);
/// 輪詢間隔。
const POLL: Duration = Duration::from_millis(20);

/// 檢查 13 的判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// 候選窗出現、位置合理、Enter 送出「我」、Esc 後消失。
    Pass,
    /// 互動桌面上候選窗出現後，任一斷言不成立。
    Fail,
    /// TIP 沒有接到按鍵（組字空且候選窗沒出現）：無法判定，不算失敗。
    Inconclusive,
    /// 沒有互動桌面：不算失敗。
    Headless,
    /// 本產品還沒安裝或沒為目前使用者啟用：不算失敗。
    Unavailable,
}

/// 檢查 13 的結果。
#[derive(Debug)]
pub struct Outcome {
    /// 判定。
    pub verdict: Verdict,
    /// 人類輸出與 JSON `detail` 共用的說明。
    pub detail: String,
    /// JSON 的 `ui_probe` cell。
    pub cells: UiProbeCells,
}

/// 執行 UI 探針。呼叫端必須已經 `CoInitializeEx(APARTMENTTHREADED)`。
pub fn run() -> Outcome {
    enable_per_monitor_dpi();

    // Session 0 的服務工作階段也有桌面與視窗，但不在互動式 window station；
    // 只看 SetForegroundWindow 會誤判，先確認是 WinSta0。
    if !on_interactive_station() {
        return headless("window station 不是 WinSta0");
    }

    let host = match Host::create() {
        Ok(host) => host,
        Err(reason) => return headless(reason),
    };

    // SAFETY: 只查系統度量，不需要額外的前置條件。
    if unsafe { GetSystemMetrics(SM_CXSCREEN) } <= 0 {
        return headless("SM_CXSCREEN=0");
    }

    // 先把視窗畫出來並取得前景，才分得出「沒有互動桌面」與「輸入法壞了」。
    host.show();
    pump_for(Duration::from_millis(60));
    // SAFETY: 兩個都是無副作用的查詢。
    if unsafe { GetForegroundWindow() } != host.hwnd {
        return headless("SetForegroundWindow 沒有生效（前景視窗不是探針視窗）");
    }

    let session = match Session::activate() {
        Ok(session) => session,
        Err(ActivateFailure::NotEnabled(reason)) => return unavailable(reason),
        Err(ActivateFailure::Failed(reason)) => {
            return interactive_fail(
                format!("{reason}；候選窗無法驗證"),
                UiProbeCells::interactive(),
            )
        }
    };

    // msctf 要在 TSF 啟動之後才看得到焦點變化，才會為 EDIT 建立 context。
    // 先放掉焦點再拿回來，確保一定有一次 WM_SETFOCUS 進到 TSF。
    let _ = unsafe { SetFocus(None) };
    host.focus();
    pump_for(Duration::from_millis(200));

    let mut observation = Observation::empty();
    let mut dpi = ui_probe_rules::BASE_DPI;

    // 第一輪：j i 3 ↓，讓候選窗真的開起來。
    match open_candidates() {
        Ok((candidate, latency_ms)) => {
            observation.latency_ms = latency_ms;
            observation.composition = edit_text(host.edit);
            observation.candidate_visible = candidate.is_some();
            observation.screen = virtual_screen();

            if let Some(hwnd) = candidate {
                observation.candidate_rect = window_rect(hwnd);
                observation.caret = caret_screen(host.edit);
                observation.work_area = observation.caret.and_then(monitor_work_area);
                dpi = dpi_for(hwnd);

                // 端到端證明：候選窗開著時 Enter 要真的把「我」送進 EDIT。
                if !send_key(VK_RETURN) {
                    return headless("SendInput(VK_RETURN) 被系統拒絕");
                }
                pump_for(Duration::from_millis(200));
                observation.committed_text = edit_text(host.edit);

                // 第二輪：重新組字開窗再驗 Esc。Enter 之後視窗已經關了，
                // 直接驗「消失」會得到假的 true。
                match open_candidates() {
                    Ok((Some(_), _)) => {
                        observation.escape_verified = true;
                        if !send_key(VK_ESCAPE) {
                            return headless("SendInput(VK_ESCAPE) 被系統拒絕");
                        }
                        observation.escape_hides = wait_hidden(ESCAPE_WAIT);
                    }
                    Ok((None, _)) => {}
                    Err(vk) => return headless(format!("SendInput(VK={:#04X}) 被系統拒絕", vk.0)),
                }
            }
        }
        Err(vk) => return headless(format!("SendInput(VK={:#04X}) 被系統拒絕", vk.0)),
    }

    let assessment = ui_probe_rules::assess(&observation, dpi);
    let cells = ui_probe_rules::cells(&observation, "interactive");

    drop(session);

    match assessment.decision {
        ui_probe_rules::Decision::Pass => Outcome {
            verdict: Verdict::Pass,
            detail: format!(
                "{}；候選窗在插入點附近出現、Enter 送出「{}」、Esc 後消失",
                cells.detail(),
                ui_probe_rules::EXPECTED_COMMIT
            ),
            cells,
        },
        ui_probe_rules::Decision::Fail => Outcome {
            verdict: Verdict::Fail,
            detail: fail_detail(&cells, &assessment.problems),
            cells,
        },
        ui_probe_rules::Decision::Inconclusive => Outcome {
            verdict: Verdict::Inconclusive,
            detail: format!(
                "{}；{}；這是探針環境問題，不能判定產品失敗；設定 JC_TSF_LOG=1 後可看 %LOCALAPPDATA%\\jc-ime\\tsf.log",
                cells.detail(),
                assessment.problems.join("；")
            ),
            cells,
        },
    }
}

/// 失敗說明：cell、所有不成立的地方、tsf.log 的位置。
fn fail_detail(cells: &UiProbeCells, problems: &[String]) -> String {
    format!(
        "{}；{}；詳細原因看 %LOCALAPPDATA%\\jc-ime\\tsf.log（設定 JC_TSF_LOG=1 後重跑）",
        cells.detail(),
        problems.join("；")
    )
}

/// 沒有互動桌面：`[SKIP]`，不算失敗。
fn headless(reason: impl Into<String>) -> Outcome {
    let reason = reason.into();
    Outcome {
        verdict: Verdict::Headless,
        detail: format!(
            "{}；沒有互動桌面（{reason}），跳過候選窗驗證，不算失敗",
            UiProbeCells::headless().detail()
        ),
        cells: UiProbeCells::headless(),
    }
}

/// 互動桌面上的失敗；`reason` 會接在 cell 後面。
fn interactive_fail(reason: String, cells: UiProbeCells) -> Outcome {
    let detail = format!(
        "{}；{reason}；詳細原因看 %LOCALAPPDATA%\\jc-ime\\tsf.log（設定 JC_TSF_LOG=1 後重跑）",
        cells.detail()
    );
    Outcome {
        verdict: Verdict::Fail,
        detail,
        cells,
    }
}

/// 本產品還沒安裝或沒為目前使用者啟用：`[SKIP]`，交給檢查 10 判定。
///
/// 這裡的訊息要和 [`headless`] 分開：CI 必須看得出是「沒有桌面」還是
/// 「輸入法沒裝／沒啟用」，後者不能只印「沒有互動桌面」。
fn unavailable(reason: String) -> Outcome {
    let cells = UiProbeCells::interactive();
    Outcome {
        verdict: Verdict::Unavailable,
        detail: format!(
            "{}；jc 注音未安裝或未為目前使用者啟用（{reason}）；候選窗無法驗證，\
             不算失敗（先以目前使用者執行 jc-tip-register.exe user-install）",
            cells.detail()
        ),
        cells,
    }
}

/// 目前行程是否在互動式 window station（`WinSta0`）。
///
/// Session 0 的服務工作階段是 `Service-0x0-…$`，那裡建視窗會成功但沒有真的
/// 輸入桌面，所以這個判斷要在建立視窗前做。
fn on_interactive_station() -> bool {
    // SAFETY: 只查詢本行程的 window station 名稱，不持有或修改它。
    unsafe {
        let Ok(station) = GetProcessWindowStation() else {
            return false;
        };
        let mut buffer = [0u16; 64];
        let mut needed = 0u32;
        if GetUserObjectInformationW(
            HANDLE(station.0),
            UOI_NAME,
            Some(buffer.as_mut_ptr().cast()),
            std::mem::size_of_val(&buffer) as u32,
            Some(&mut needed),
        )
        .is_err()
        {
            return false;
        }
        let len = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        String::from_utf16_lossy(&buffer[..len]).eq_ignore_ascii_case("WinSta0")
    }
}

/// 設成 per-monitor DPI aware，讓 rect 與插入點都用實體像素。
///
/// 失敗（manifest 已設定、系統太舊）不影響判定：呼叫端與候選窗仍在同一個
/// 座標系。
fn enable_per_monitor_dpi() {
    // SAFETY: 只設定本行程的 DPI awareness，不改系統設定。
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// 插入點的螢幕座標；先試 `GetCaretPos`，失敗才退回 EDIT client rect 的左下角。
fn caret_screen(edit: HWND) -> Option<(i32, i32)> {
    // SAFETY: point 是本機可寫輸出；ClientToScreen 只轉座標。
    unsafe {
        let mut point = POINT::default();
        if GetCaretPos(&mut point).is_ok() && ClientToScreen(edit, &mut point).as_bool() {
            return Some((point.x, point.y));
        }

        let mut rect = RECT::default();
        if GetClientRect(edit, &mut rect).is_ok() {
            let mut point = POINT {
                x: rect.left,
                y: rect.bottom,
            };
            if ClientToScreen(edit, &mut point).as_bool() {
                return Some((point.x, point.y));
            }
        }
    }
    None
}

/// 視窗的螢幕 rect。
fn window_rect(hwnd: HWND) -> Option<Rect> {
    let mut rect = RECT::default();
    // SAFETY: hwnd 是系統回傳的視窗；rect 是本機可寫輸出。
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        return None;
    }
    Some(Rect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    })
}

/// 整個虛擬螢幕的 rect。
fn virtual_screen() -> Option<Rect> {
    // SAFETY: 只查系統度量。
    let (left, top, width, height) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    if width <= 0 || height <= 0 {
        return None;
    }
    Some(Rect {
        left,
        top,
        right: left + width,
        bottom: top + height,
    })
}

/// 插入點所在 monitor 的 work area（不含工作列）。
fn monitor_work_area(caret: (i32, i32)) -> Option<Rect> {
    // SAFETY: 只查插入點最近的 monitor；info 是本機可寫輸出。
    unsafe {
        let monitor = MonitorFromPoint(
            POINT {
                x: caret.0,
                y: caret.1,
            },
            MONITOR_DEFAULTTONEAREST,
        );
        if monitor.is_invalid() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        Some(Rect {
            left: info.rcWork.left,
            top: info.rcWork.top,
            right: info.rcWork.right,
            bottom: info.rcWork.bottom,
        })
    }
}

/// 視窗所在 monitor 的 DPI；取不到時用 96。
fn dpi_for(hwnd: HWND) -> u32 {
    // SAFETY: 純查詢。
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        ui_probe_rules::BASE_DPI
    } else {
        dpi
    }
}

/// 讀 EDIT 目前的文字，當成組字或提交結果；取不到就回空字串。
fn edit_text(edit: HWND) -> String {
    let mut buffer = [0u16; 512];
    // SAFETY: buffer 是本機可寫陣列；回傳長度不會超過 buffer。
    let len = unsafe { GetWindowTextW(edit, &mut buffer) };
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..len as usize])
        .trim()
        .to_string()
}

/// 送出一輪注音並開候選窗：`j`、`i`、`3`、`↓`。
///
/// 回傳（第一次看到的候選窗, 延遲毫秒）；`SendInput` 被拒時回被拒的鍵。
fn open_candidates() -> Result<(Option<HWND>, u64), VIRTUAL_KEY> {
    let keys = [VK_J, VK_I, VK_3, VK_DOWN];
    let last = keys.len() - 1;
    let mut last_key_at = Instant::now();
    for (index, vk) in keys.into_iter().enumerate() {
        if !send_key(vk) {
            return Err(vk);
        }
        if index == last {
            last_key_at = Instant::now();
        }
        pump_for(Duration::from_millis(60));
    }
    Ok(wait_visible(last_key_at, CANDIDATE_WAIT))
}

/// 送出一個虛擬鍵的按下與放開。
fn send_key(vk: VIRTUAL_KEY) -> bool {
    // SAFETY: MapVirtualKeyW 是純查詢。
    let scan = unsafe { MapVirtualKeyW(u32::from(vk.0), MAPVK_VK_TO_VSC) } as u16;
    let down = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: KEYBD_EVENT_FLAGS(0),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let mut up = down;
    up.Anonymous.ki.dwFlags = KEYEVENTF_KEYUP;
    // SAFETY: 兩個 INPUT 都在本機；長度以 size_of 傳給 API。
    let sent = unsafe { SendInput(&[down, up], std::mem::size_of::<INPUT>() as i32) };
    sent == 2
}

/// 目前可見的候選窗。
///
/// `jc-tsf` 是 in-proc TIP，候選窗一定長在本行程；別的程序（例如 explorer）
/// 也可能有同名視窗，所以用 `GetWindowThreadProcessId` 過濾，避免誤判。
fn visible_candidate() -> Option<HWND> {
    // SAFETY: 只查詢本行程 id。
    let current = unsafe { GetCurrentProcessId() };
    let mut after = HWND::default();
    loop {
        // SAFETY: 依類別列舉同一桌面的 top-level 視窗；after 是上一輪的結果。
        let hwnd =
            unsafe { FindWindowExW(None, Some(after), CANDIDATE_CLASS, PCWSTR::null()) }.ok()?;

        let mut pid = 0u32;
        // SAFETY: hwnd 剛由系統回傳；pid 是本機可寫輸出。
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut u32)) };
        // SAFETY: 純查詢。
        if pid == current && unsafe { IsWindowVisible(hwnd) }.as_bool() {
            return Some(hwnd);
        }
        after = hwnd;
    }
}

/// 從 `start` 起輪詢到候選窗可見為止，回傳（hwnd, 延遲毫秒）。
fn wait_visible(start: Instant, limit: Duration) -> (Option<HWND>, u64) {
    loop {
        pump_messages();
        if let Some(hwnd) = visible_candidate() {
            return (Some(hwnd), elapsed_ms(start));
        }
        if start.elapsed() >= limit {
            return (None, elapsed_ms(start));
        }
        std::thread::sleep(POLL);
    }
}

/// 輪詢到候選窗不再可見（或已銷毀）為止。
fn wait_hidden(limit: Duration) -> bool {
    let start = Instant::now();
    loop {
        pump_messages();
        if visible_candidate().is_none() {
            return true;
        }
        if start.elapsed() >= limit {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

/// 在這段時間內持續抽訊息：TSF 的按鍵處理與候選窗更新都靠訊息迴圈推進。
fn pump_for(duration: Duration) {
    let start = Instant::now();
    while start.elapsed() < duration {
        pump_messages();
        std::thread::sleep(POLL);
    }
}

/// 把佇列裡的訊息全部抽完並派送。
fn pump_messages() {
    let mut message = MSG::default();
    // SAFETY: message 是本機可寫緩衝；PM_REMOVE 讓佇列往前走。
    while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
        // SAFETY: message 由 PeekMessageW 填入，交給 Translate／Dispatch 是標準用法。
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn elapsed_ms(start: Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

/// 探針的 host 視窗與 `EDIT` 子視窗。
struct Host {
    hwnd: HWND,
    edit: HWND,
    instance: HINSTANCE,
}

impl Host {
    fn create() -> Result<Self, String> {
        // SAFETY: 建立本行程的視窗類別與兩個視窗；失敗時把已建立的資源收乾淨。
        unsafe {
            let instance: HINSTANCE = GetModuleHandleW(None)
                .map_err(|error| format!("GetModuleHandleW 失敗（{}）", error.code().0 as u32))?
                .into();

            let atom = RegisterClassW(&WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(host_proc),
                hInstance: instance,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: HOST_CLASS,
                ..Default::default()
            });
            if atom == 0 {
                return Err(format!(
                    "RegisterClassW(jcUiProbeHost) 失敗（GetLastError={:#010X}）",
                    GetLastError().0
                ));
            }

            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE(0),
                HOST_CLASS,
                w!("jc-tsf-smoke UI 探針"),
                WS_OVERLAPPEDWINDOW,
                80,
                80,
                520,
                260,
                None,
                None,
                Some(instance),
                None,
            ) {
                Ok(hwnd) => hwnd,
                Err(error) => {
                    let _ = UnregisterClassW(HOST_CLASS, Some(instance));
                    return Err(format!(
                        "CreateWindowExW(jcUiProbeHost) 失敗（HRESULT {:#010X}）",
                        error.code().0 as u32
                    ));
                }
            };

            let edit = match CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                w!(""),
                WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP,
                10,
                10,
                480,
                200,
                Some(hwnd),
                None,
                Some(instance),
                None,
            ) {
                Ok(edit) => edit,
                Err(error) => {
                    let _ = DestroyWindow(hwnd);
                    let _ = UnregisterClassW(HOST_CLASS, Some(instance));
                    return Err(format!(
                        "CreateWindowExW(EDIT) 失敗（HRESULT {:#010X}）",
                        error.code().0 as u32
                    ));
                }
            };

            Ok(Self {
                hwnd,
                edit,
                instance,
            })
        }
    }

    /// 顯示 host、拉到前景，並把焦點給 EDIT。
    fn show(&self) {
        // SAFETY: 三個呼叫都只作用在本行程剛建立的視窗。
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.hwnd);
        }
        self.focus();
    }

    /// 把鍵盤焦點交給 EDIT。
    fn focus(&self) {
        // SAFETY: edit 是本行程的子視窗。
        let _ = unsafe { SetFocus(Some(self.edit)) };
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // SAFETY: 先銷毀視窗再移除類別；兩者失敗都不影響結果。
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            let _ = UnregisterClassW(HOST_CLASS, Some(self.instance));
        }
    }
}

/// host 視窗的訊息處理：全部交給預設處理。
unsafe extern "system" fn host_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: 參數由系統傳入，直接轉給 DefWindowProcW。
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// `Session::activate` 的失敗分類。
enum ActivateFailure {
    /// 本產品沒有為目前使用者啟用：`[SKIP]`。
    NotEnabled(String),
    /// 真的建立或激活失敗：`[FAIL]`。
    Failed(String),
}

/// 本行程的 TSF 狀態：啟動 thread manager、把本 TIP 設成作用中 profile。
struct Session {
    thread_mgr: ITfThreadMgr,
    profiles: ITfInputProcessorProfiles,
    /// 啟動前的鍵盤 profile；沒有作用中的 profile 時是 `None`。
    previous: Option<TF_INPUTPROCESSORPROFILE>,
    /// 啟動前的執行緒語言設定（`GetCurrentLanguage`）。
    language_before: Option<u16>,
}

impl Session {
    fn activate() -> Result<Self, ActivateFailure> {
        // SAFETY: 呼叫端已 CoInitializeEx；這是標準的 in-proc COM 建立。
        let thread_mgr: ITfThreadMgr = unsafe {
            CoCreateInstance(
                &CLSID_TF_ThreadMgr,
                None::<&windows::core::IUnknown>,
                CLSCTX_INPROC_SERVER,
            )
        }
        .map_err(|error| {
            ActivateFailure::Failed(format!(
                "CoCreateInstance(CLSID_TF_ThreadMgr) 失敗（HRESULT {:#010X}）",
                error.code().0 as u32
            ))
        })?;

        // SAFETY: 同上。
        let profiles: ITfInputProcessorProfiles = unsafe {
            CoCreateInstance(
                &CLSID_TF_InputProcessorProfiles,
                None::<&windows::core::IUnknown>,
                CLSCTX_INPROC_SERVER,
            )
        }
        .map_err(|error| {
            ActivateFailure::Failed(format!(
                "CoCreateInstance(CLSID_TF_InputProcessorProfiles) 失敗（HRESULT {:#010X}）",
                error.code().0 as u32
            ))
        })?;

        // 沒安裝或沒跑 user-install 時不要判失敗：檢查 10 已經會處理，
        // 這裡只需要說「無從驗證」。
        // SAFETY: profiles 剛建立；GUID 都是常數。
        match unsafe {
            profiles.IsEnabledLanguageProfile(
                &product::CLSID_JC_TSF,
                product::LANGID_ZH_TW,
                &product::GUID_PROFILE_JC_TSF,
            )
        } {
            Ok(enabled) if !enabled.as_bool() => {
                return Err(ActivateFailure::NotEnabled(
                    "IsEnabledLanguageProfile=false".to_string(),
                ))
            }
            _ => {}
        }

        // SAFETY: profiles 剛建立；只讀目前執行緒的語言設定。
        let language_before = unsafe { profiles.GetCurrentLanguage() }.ok();
        let previous = profiles
            .cast::<ITfInputProcessorProfileMgr>()
            .ok()
            .and_then(|manager| {
                let mut profile = TF_INPUTPROCESSORPROFILE::default();
                // SAFETY: manager 有效；profile 是本機可寫輸出。
                unsafe { manager.GetActiveProfile(&GUID_TFCAT_TIP_KEYBOARD, &mut profile) }
                    .ok()
                    .map(|()| profile)
            });

        // SAFETY: manager 剛建立，還沒有其他 sink。
        unsafe { thread_mgr.Activate() }.map_err(|error| {
            ActivateFailure::Failed(format!(
                "ITfThreadMgr::Activate 失敗（HRESULT {:#010X}）",
                error.code().0 as u32
            ))
        })?;

        let session = Self {
            thread_mgr,
            profiles,
            previous,
            language_before,
        };
        session.activate_ours().map_err(ActivateFailure::Failed)?;
        Ok(session)
    }

    /// 把本 TIP 設成本行程的作用中 profile；只影響本行程。
    fn activate_ours(&self) -> Result<(), String> {
        let manager: ITfInputProcessorProfileMgr = self.profiles.cast().map_err(|error| {
            format!(
                "ITfInputProcessorProfiles::cast(ProfileMgr) 失敗（HRESULT {:#010X}）",
                error.code().0 as u32
            )
        })?;
        // SAFETY: manager 有效；GUID 都是常數，hkl 傳預設值代表不指定鍵盤配置。
        unsafe {
            manager.ActivateProfile(
                TF_PROFILETYPE_INPUTPROCESSOR,
                product::LANGID_ZH_TW,
                &product::CLSID_JC_TSF,
                &product::GUID_PROFILE_JC_TSF,
                HKL::default(),
                TF_IPPMF_FORPROCESS,
            )
        }
        .map_err(|error| {
            format!(
                "ActivateProfile(TF_IPPMF_FORPROCESS) 失敗（HRESULT {:#010X}）",
                error.code().0 as u32
            )
        })
    }

    /// 還原啟動前的 profile 與語言，再放掉 thread manager。
    fn restore(&self) {
        let Ok(manager) = self.profiles.cast::<ITfInputProcessorProfileMgr>() else {
            return;
        };
        // SAFETY: manager 有效；所有 GUID 都來自先前 API 回傳或本產品常數。
        unsafe {
            match &self.previous {
                Some(previous)
                    if previous.clsid != product::CLSID_JC_TSF
                        || previous.guidProfile != product::GUID_PROFILE_JC_TSF =>
                {
                    let _ = manager.ActivateProfile(
                        TF_PROFILETYPE_INPUTPROCESSOR,
                        previous.langid,
                        &previous.clsid,
                        &previous.guidProfile,
                        HKL::default(),
                        TF_IPPMF_FORPROCESS,
                    );
                }
                _ => {
                    // 原本就是本產品或沒有作用中的 profile：把本產品取消掉。
                    let _ = manager.DeactivateProfile(
                        TF_PROFILETYPE_INPUTPROCESSOR,
                        product::LANGID_ZH_TW,
                        &product::CLSID_JC_TSF,
                        &product::GUID_PROFILE_JC_TSF,
                        HKL::default(),
                        TF_IPPMF_FORPROCESS,
                    );
                }
            }

            if let (Some(language), Some(previous)) = (self.language_before, &self.previous) {
                if language != previous.langid {
                    let _ = self.profiles.ActivateLanguageProfile(
                        &previous.clsid,
                        language,
                        &previous.guidProfile,
                    );
                }
            }

            let _ = self.thread_mgr.Deactivate();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.restore();
    }
}
