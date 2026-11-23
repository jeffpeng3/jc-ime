//! 把 [`crate::ipc::Client`] 接成 session 的 [`Transport`]。
//!
//! 這條線是 TIP 與引擎之間唯一的正式路徑。少接這段，`Session` 會用
//! `NullTransport`：每個請求都失敗、`available=false`，所有按鍵原樣放行，
//! 使用者看到的就是「切到 jc 注音打不出字」。實機 run 45145104315 的
//! `tsf.log` 就是這個樣子（`learn_mode 結束：connect_existing 失敗`）。
//!
//! 連不上時允許啟動引擎：使用者重開機前、引擎崩掉、或安裝後還沒登入，
//! 第一個按鍵就會把它帶起來，不必等 Run 機碼。

use serde_json::Value;

use crate::ipc::{Client, EngineCommand};
use crate::session::Transport;

/// 正式 TIP 使用的 transport。
pub struct PipeTransport {
    client: Client,
}

impl PipeTransport {
    /// 連 `jc-ime-<rid>`，需要時啟動安裝目錄的 `jc-engine.exe`。
    pub fn engine() -> Self {
        let mut client = Client::new();
        if let Some(command) = engine_command() {
            client.set_engine_command(command);
        } else {
            // 找不到引擎就只連已在執行的：沒有啟動命令時 `connect` 不會等。
            crate::log::log("ipc：找不到 jc-engine.exe 或資料目錄，只連已在執行的引擎");
        }
        Self { client }
    }
}

/// 引擎執行檔與 argv。
///
/// 只帶 `--supervise` 與 `--data`：
/// - pipe 名由 `jc_windows::resolve_pipe_name()` 決定，引擎與 TIP 呼叫同一函式，
///   所以**不要**自己算 `--socket`，那只會多一個出錯點。
/// - 個人資料目錄由引擎解析到 `%LOCALAPPDATA%\jc-ime`（`--personal-dir` 的預設）。
fn engine_command() -> Option<EngineCommand> {
    let dll = jc_tsf_register::module_path().ok()?;
    let exe = std::path::Path::new(&dll).parent()?.join("jc-engine.exe");
    if !exe.is_file() {
        return None;
    }
    let data = data_root()?;
    if !data.is_dir() {
        return None;
    }
    Some(EngineCommand {
        exe,
        args: vec![
            "--supervise".to_string(),
            "--data".to_string(),
            data.display().to_string(),
        ],
    })
}

/// 安裝時的資料根目錄：`%ProgramData%\jc-ime\data`（與 `installer/jc.iss` 相同）。
fn data_root() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("ProgramData")?;
    Some(std::path::PathBuf::from(base).join("jc-ime").join("data"))
}

impl Transport for PipeTransport {
    fn request(&mut self, request: &Value) -> Option<Value> {
        self.client.call(request)
    }

    fn connected(&self) -> bool {
        self.client.connected()
    }

    fn connect(&mut self) -> bool {
        self.client.connect()
    }

    /// Activate 的探測：只連已在執行的引擎，不啟動也不等。
    fn connect_existing(&mut self) -> bool {
        self.client.connect_existing()
    }

    fn close(&mut self) {
        self.client.release();
    }

    fn alive(&mut self) -> bool {
        self.client.alive()
    }
}
