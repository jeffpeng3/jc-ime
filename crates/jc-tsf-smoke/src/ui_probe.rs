//! 檢查 13：UI 探針。
//!
//! 前 12 項只看註冊表、檔案、COM 與 pipe，沒有任何一項證明候選窗真的畫在
//! 螢幕上。這裡建一個 host 視窗加 `EDIT` 子視窗，把本 TIP 設成本行程的作用中
//! 輸入法，用 `SendInput` 打 `j`、`i`、`3`，再用 `FindWindowExW` 輪詢候選窗：
//!
//! 1. 候選窗存在且 `IsWindowVisible` 為真。
//! 2. `GetWindowRect` 非空、在虛擬螢幕範圍內。
//! 3. 位置合理：與 `EDIT` 的插入點比較，`|dx| <= 300`、`dy` 在 `-80..=160`。
//! 4. 送 `Esc` 後 1000ms 內候選窗不再可見（或已銷毀）。
//!
//! 沒有互動桌面（Session 0、服務工作階段、鎖定中的桌面）時建視窗、取得前景
//! 視窗或送按鍵會失敗；這種情況回 [`Verdict::Headless`]，由呼叫端記成 `[SKIP]`，
//! **不算失敗**。本產品還沒安裝或沒為目前使用者啟用時回
//! [`Verdict::Unavailable`]，同樣是 `[SKIP]`（檢查 10 已經會把這件事判成
//! `[FAIL]`，不要重複紅）。其餘失敗一律 [`Verdict::Fail`]，detail 會帶實際
//! rect、延遲與 `%LOCALAPPDATA%\jc-ime\tsf.log` 的建議。
//!
//! 所有 TSF 狀態都只改本行程：`ActivateProfile` 帶 `TF_IPPMF_FORPROCESS`，
//! 結束時把原本的 profile 與語言還原。

use std::time::{Duration, Instant};

use windows::core::{w, Interface, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::StationsAndDesktops::{
    GetProcessWindowStation, GetUserObjectInformationW, UOI_NAME,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, SetFocus, HKL, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, VIRTUAL_KEY, VK_3, VK_ESCAPE, VK_I, VK_J,
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
/// 候選窗左緣與插入點的容許水平差。
const MAX_DX: i32 = 300;
/// 候選窗上緣與插入點的容許垂直差（負值代表在插入點上方）。
const MIN_DY: i32 = -80;
/// 候選窗上緣與插入點的容許垂直差上限。
const MAX_DY: i32 = 160;

/// 檢查 13 的判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// 候選窗出現、位置合理、Esc 後消失。
    Pass,
    /// 互動桌面上任一斷言不成立。
    Fail,
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

    let mut cells = UiProbeCells::interactive();
    let mut problems = Vec::new();

    let mut last_key_at = Instant::now();
    for (index, vk) in [VK_J, VK_I, VK_3].into_iter().enumerate() {
        if !send_key(vk) {
            // 輸入被系統擋掉（鎖定中的桌面、UIPI）不是產品的錯：回 inconclusive。
            return headless(format!("SendInput(VK={:#04X}) 被系統拒絕", vk.0));
        }
        if index == 2 {
            last_key_at = Instant::now();
        }
        pump_for(Duration::from_millis(60));
    }

    let (candidate, latency_ms) = wait_visible(last_key_at, CANDIDATE_WAIT);
    cells.composition = edit_text(host.edit);
    cells.candidate_visible = candidate.is_some();
    cells.latency_ms = latency_ms;

    match candidate {
        Some(hwnd) => {
            let mut rect = RECT::default();
            // SAFETY: hwnd 是剛找到的候選窗；rect 是本機可寫輸出。
            if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() {
                cells.candidate_rect =
                    format!("{},{},{},{}", rect.left, rect.top, rect.right, rect.bottom);
                check_rect(&mut problems, &rect, host.edit, &cells.candidate_rect);
            } else {
                problems.push("GetWindowRect(候選窗) 失敗".to_string());
            }
        }
        None => problems.push(format!(
            "{}ms 內沒有看到本行程可見的 jcCandidateUiWindow",
            latency_ms
        )),
    }

    if send_key(VK_ESCAPE) {
        cells.escape_hides = wait_hidden(ESCAPE_WAIT);
        if !cells.escape_hides {
            problems.push(format!(
                "按 Esc 後 {}ms 內候選窗仍可見",
                ESCAPE_WAIT.as_millis()
            ));
        }
    } else {
        problems.push("SendInput(VK_ESCAPE) 失敗".to_string());
    }

    let detail = if problems.is_empty() {
        format!("{}；候選窗在插入點附近出現，Esc 後消失", cells.detail())
    } else {
        format!(
            "{}；{}；詳細原因看 %LOCALAPPDATA%\\jc-ime\\tsf.log（設定 JC_TSF_LOG=1 後重跑）",
            cells.detail(),
            problems.join("；")
        )
    };

    drop(session);
    let verdict = if problems.is_empty() {
        Verdict::Pass
    } else {
        Verdict::Fail
    };
    Outcome {
        verdict,
        detail,
        cells,
    }
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
fn unavailable(reason: String) -> Outcome {
    let cells = UiProbeCells::interactive();
    Outcome {
        verdict: Verdict::Unavailable,
        detail: format!(
            "{}；{reason}；候選窗無法驗證，不算失敗（先以目前使用者執行 jc-tip-register.exe user-install）",
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

/// 檢查候選窗的 rect：非空、在虛擬螢幕內、與插入點的距離合理。
fn check_rect(problems: &mut Vec<String>, rect: &RECT, edit: HWND, rect_text: &str) {
    if rect.right <= rect.left || rect.bottom <= rect.top {
        problems.push(format!("候選窗 rect 是空的：{rect_text}"));
    }

    // SAFETY: 只查系統度量。
    let (left, top, width, height) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    if rect.left < left || rect.top < top || rect.right > left + width || rect.bottom > top + height
    {
        problems.push(format!(
            "候選窗 rect 不在虛擬螢幕 ({left},{top},{},{}) 內：{rect_text}",
            left + width,
            top + height
        ));
    }

    match caret_screen(edit) {
        Some((x, y)) => {
            let dx = rect.left - x;
            let dy = rect.top - y;
            if dx.abs() > MAX_DX || !(MIN_DY..=MAX_DY).contains(&dy) {
                problems.push(format!(
                    "候選窗位置不合理：rect={rect_text}、插入點=({x},{y})、dx={dx}、dy={dy}"
                ));
            }
        }
        None => {
            problems.push("取不到 EDIT 插入點（GetCaretPos 與 client rect 都失敗）".to_string())
        }
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

/// 讀 EDIT 目前的文字，當成組字字串；取不到就回空字串。
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

/// 本行程的 TSF 狀態：啟動 thread manager、把本 TIP 設成作用中 profile。
struct Session {
    thread_mgr: ITfThreadMgr,
    profiles: ITfInputProcessorProfiles,
    /// 啟動前的鍵盤 profile；沒有作用中的 profile 時是 `None`。
    previous: Option<TF_INPUTPROCESSORPROFILE>,
    /// 啟動前的執行緒語言設定（`GetCurrentLanguage`）。
    language_before: Option<u16>,
}

/// `Session::activate` 的失敗分類。
enum ActivateFailure {
    /// 本產品沒有為目前使用者啟用：`[SKIP]`。
    NotEnabled(String),
    /// 真的建立或激活失敗：`[FAIL]`。
    Failed(String),
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
