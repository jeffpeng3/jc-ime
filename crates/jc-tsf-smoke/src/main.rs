//! `jc-tsf.dll` 的免管理員冒煙測試。
//!
//! **這支程式只能在 Windows 上執行。** TSF 的 `msctf.dll`、COM 與 named pipe
//! 都是 Windows 專屬；Linux 上建置出來的 `main` 只是印一行訊息後以 2 結束，
//! 供 `cargo check --workspace --all-targets` 驗證型別。
//!
//! 七項檢查與每個失敗項目的可能原因見 `docs/port/tsf-smoke.md`。

use std::process::ExitCode;

#[cfg_attr(not(windows), allow(dead_code))]
mod report;

#[cfg(windows)]
mod checks;
#[cfg(windows)]
mod dll;
#[cfg(windows)]
mod engine;
#[cfg(windows)]
mod error;
#[cfg(windows)]
mod product;
#[cfg(windows)]
mod registry;

#[cfg(windows)]
fn main() -> ExitCode {
    checks::run(std::env::args().skip(1).collect())
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("jc-tsf-smoke 只能在 Windows 上執行；其他平台只支援 cargo check。");
    ExitCode::from(2)
}
