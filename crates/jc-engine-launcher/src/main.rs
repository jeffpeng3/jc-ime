//! 登入自動啟動用的無視窗啟動器。
//!
//! `jc-engine.exe` 是 console subsystem：由 `HKLM\...\Run` 直接啟動時，Windows
//! 會配一個看得到的終端機視窗，關掉它等於把引擎殺掉。這支啟動器自己是
//! `windows` subsystem，用 `CREATE_NO_WINDOW` 啟動引擎，因此登入時完全不出現
//! 視窗。引擎本體、`--supervise`、單例鎖與 log 路徑都沒有改變。
//!
//! 手動執行 `jc-engine.exe` 仍是 console 程式，`--help`／`--version` 照常輸出。
//! 只有 Run 機碼與安裝器的即時啟動改走這裡。

#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("jc-engine-launcher 只用於 Windows 登入自動啟動");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    if let Err(err) = spawn_engine() {
        log_failure(&err.to_string());
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn spawn_engine() -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    use jc_engine_launcher::{args_after_program, engine_path, quoted_command_line};
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Environment::GetCommandLineW;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, CREATE_NO_WINDOW, PROCESS_INFORMATION, STARTUPINFOW,
    };

    let launcher = std::env::current_exe()?;
    let engine = engine_path(&launcher);
    if !engine.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("找不到引擎：{}", engine.display()),
        ));
    }

    // GetCommandLineW 是整條命令列，包含啟動器自己的路徑。只取 argv[0] 之後的
    // 原始片段，引號與空字串都原樣轉給引擎，不經過第二次組裝。
    let raw = unsafe {
        let pointer = GetCommandLineW();
        let mut len = 0;
        while *pointer.add(len) != 0 {
            len += 1;
        }
        std::slice::from_raw_parts(pointer, len).to_vec()
    };
    let tail = args_after_program(&raw);
    let engine_wide: Vec<u16> = engine.as_os_str().encode_wide().collect();
    let mut command_line = quoted_command_line(&engine_wide, tail);
    command_line.push(0);
    let application: Vec<u16> = engine
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    // bInheritHandles = FALSE：啟動器沒有要交接的 handle。引擎是 console 程式，
    // CREATE_NO_WINDOW 會給它一個沒有視窗的 console，stdout／stderr 仍然有效，
    // 監督者也能照舊用 console 事件結束子行程。
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            CREATE_NO_WINDOW,
            ptr::null(),
            ptr::null(),
            &startup,
            &mut process,
        )
    };
    if created == 0 {
        return Err(std::io::Error::last_os_error());
    }

    // 不等待引擎：啟動器只負責把它帶起來，登入後由監督者常駐。
    unsafe {
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
    }
    Ok(())
}

/// 啟動器在 `windows` subsystem 下沒有 stdout／stderr，失敗時留下檔案線索。
/// 引擎自己的 log 仍寫在 `%LOCALAPPDATA%\jc-ime\supervisor.log`，這裡只記錄
/// 「連引擎都帶不起來」的情況。
#[cfg(windows)]
fn log_failure(message: &str) {
    use std::io::Write;

    let base = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let directory = base.join("jc-ime");
    if std::fs::create_dir_all(&directory).is_err() {
        return;
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("launcher.log"))
    {
        let _ = writeln!(file, "jc-engine-launcher: {message}");
    }
}
