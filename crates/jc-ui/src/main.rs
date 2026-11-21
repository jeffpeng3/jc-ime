//! jc-ui：候選窗與補全提示。跑在 jc 自己的行程，接收 TSF DLL 經 named pipe 送來的
//! UiMessage（候選、反白、頁碼、建議、游標矩形），在游標旁畫一個不搶焦點的視窗。
//! 移植自 0c 的 ui/src/main.rs；選單與更新交接（0c-menu／0c-update）不在本 crate 範圍。
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod imp {
    use jc_ui::candidate;
    use jc_ui::message::{pipe_sddl, ui_pipe, wide, UiMessage, MAX_FRAME};
    use jc_ui::retire;

    use std::cell::Cell;
    use std::ffi::c_void;
    use std::time::Instant;

    use windows::Win32::Foundation::{
        CloseHandle, GetLastError, LocalFree, ERROR_ALREADY_EXISTS, HANDLE, HLOCAL, HWND, LPARAM,
        LRESULT, WPARAM,
    };
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows::Win32::Storage::FileSystem::{ReadFile, PIPE_ACCESS_INBOUND};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
        PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, IsWindowVisible, KillTimer,
        PostMessageW, PostQuitMessage, SetTimer, TranslateMessage, MSG, WM_APP, WM_CLOSE,
        WM_DESTROY, WM_ENDSESSION, WM_TIMER,
    };
    use windows_core::{w, PCWSTR};

    const WM_JC_PRESENT: u32 = WM_APP + 1;
    /// 延後結束的計時器（candidate 的提示計時器是 0x0c01）。
    const RETIRE_TIMER: usize = 0x0c02;

    thread_local! {
        /// 最後一則呈現訊息的時間（視窗執行緒）。
        static LAST_PRESENT: Cell<Option<Instant>> = const { Cell::new(None) };
    }

    /// 結束要求（WM_CLOSE）：閒下來就關視窗結束，否則稍後再看。
    unsafe fn retire(hwnd: HWND) {
        let since = LAST_PRESENT.with(|t| t.get()).map(|t| t.elapsed());
        if retire::ready_to_exit(IsWindowVisible(hwnd).as_bool(), since) {
            let _ = KillTimer(Some(hwnd), RETIRE_TIMER);
            let _ = DestroyWindow(hwnd);
        } else {
            SetTimer(Some(hwnd), RETIRE_TIMER, retire::RETIRE_POLL_MS, None);
        }
    }

    /// 程式進入點：單一實例、註冊視窗類別、起 pipe server，然後跑訊息迴圈。
    pub fn run() {
        unsafe {
            // 單一實例：第二個直接退出（DLL 在多個宿主同時啟動時會發生）。
            // 名稱與 0c 的 Local\0c-ui 分開，避免平行建置共用一把 mutex。
            let mutex = CreateMutexW(None, false, w!(r"Local\jc-ui"));
            if mutex.is_err() || GetLastError() == ERROR_ALREADY_EXISTS {
                return;
            }
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let instance = GetModuleHandleW(None).unwrap_or_default();
            let class = w!("JcCandidateWindow");
            candidate::register_class(instance.into(), class, Some(wndproc));
            let Ok(hwnd) = candidate::create(instance.into(), class, None) else {
                return;
            };
            let hwnd_value = hwnd.0 as isize;
            std::thread::spawn(move || pipe_server(&ui_pipe(), HWND(hwnd_value as *mut c_void)));
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    /// 管道伺服器：每個連線一條執行緒；每個訊框解成 UiMessage 後 PostMessage 給視窗執行緒。
    fn pipe_server(pipe: &str, hwnd: HWND) {
        let name = wide(pipe);
        let sddl = wide(&pipe_sddl());
        loop {
            unsafe {
                let mut sd = PSECURITY_DESCRIPTOR::default();
                if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    PCWSTR::from_raw(sddl.as_ptr()),
                    SDDL_REVISION_1,
                    &mut sd,
                    None,
                )
                .is_err()
                {
                    return;
                }
                let sa = SECURITY_ATTRIBUTES {
                    nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: sd.0,
                    bInheritHandle: false.into(),
                };
                let pipe = CreateNamedPipeW(
                    PCWSTR::from_raw(name.as_ptr()),
                    PIPE_ACCESS_INBOUND,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                    PIPE_UNLIMITED_INSTANCES,
                    1 << 16,
                    1 << 16,
                    0,
                    Some(&sa),
                );
                let _ = LocalFree(Some(HLOCAL(sd.0)));
                if pipe.is_invalid() {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    continue;
                }
                if ConnectNamedPipe(pipe, None).is_err() {
                    let _ = CloseHandle(pipe);
                    continue;
                }
                let h = pipe.0 as isize;
                let hw = hwnd.0 as isize;
                std::thread::spawn(move || {
                    serve(HANDLE(h as *mut c_void), HWND(hw as *mut c_void))
                });
            }
        }
    }

    fn read_exact(h: HANDLE, buf: &mut [u8]) -> bool {
        let mut off = 0usize;
        while off < buf.len() {
            let mut n = 0u32;
            let r = unsafe { ReadFile(h, Some(&mut buf[off..]), Some(&mut n), None) };
            if r.is_err() || n == 0 {
                return false;
            }
            off += n as usize;
        }
        true
    }

    fn serve(h: HANDLE, hwnd: HWND) {
        loop {
            let mut hdr = [0u8; 4];
            if !read_exact(h, &mut hdr) {
                break;
            }
            let n = u32::from_le_bytes(hdr);
            if n > MAX_FRAME {
                break;
            }
            let mut body = vec![0u8; n as usize];
            if !read_exact(h, &mut body) {
                break;
            }
            if let Ok(m) = serde_json::from_slice::<UiMessage>(&body) {
                let boxed = Box::into_raw(Box::new(m));
                unsafe {
                    if PostMessageW(Some(hwnd), WM_JC_PRESENT, WPARAM(0), LPARAM(boxed as isize))
                        .is_err()
                    {
                        drop(Box::from_raw(boxed));
                    }
                }
            }
        }
        unsafe {
            let _ = DisconnectNamedPipe(h);
            let _ = CloseHandle(h);
        }
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_JC_PRESENT => {
                let m = Box::from_raw(lparam.0 as *mut UiMessage);
                LAST_PRESENT.with(|t| t.set(Some(Instant::now())));
                candidate::present(hwnd, *m);
                LRESULT(0)
            }
            WM_CLOSE => {
                retire(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == RETIRE_TIMER => {
                retire(hwnd);
                LRESULT(0)
            }
            WM_ENDSESSION | WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => candidate::handle(hwnd, msg, wparam, lparam)
                .unwrap_or_else(|| DefWindowProcW(hwnd, msg, wparam, lparam)),
        }
    }
}

#[cfg(windows)]
fn main() {
    imp::run();
}

/// Linux 上只編訊息格式與退休判斷，bin target 保留一個空的 main 讓 `--all-targets` 編得過。
#[cfg(not(windows))]
fn main() {}
