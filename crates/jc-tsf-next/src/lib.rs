//! `jc-tsf-next`：0c `session.rs` 的移植與 jc State → 0c Snapshot 轉換層。
//!
//! 這是 T1 的 adapter spike。範圍只有三件事：
//!
//! 1. 把 0c 的 `Snapshot`、`Session` 狀態機、generation、log 重播、
//!    `restore_requests`、3 次重試加 20ms 等待原樣搬過來。
//! 2. `open_request` 改成 jc 的 `hello{protocol:3, client}`。
//! 3. 在 0c 的 `parse()` 之前，把 jc `Response::State` 轉成 0c snapshot 的
//!    JSON 形狀，讓移植過來的 `parse()` 不必改。
//!
//! 不碰 TSF、不碰註冊、不碰 jc-engine。IPC 透過 [`Transport`] 抽象；
//! T2 再提供 windows-rs 0.62 的 overlapped I/O 實作。

pub mod session;

// T1b 移植的 Win32 IPC client 與按鍵對照表，以及 T5 移植的 edit session。
// Linux host 上不編這幾塊，讓 `cargo check --workspace` 不被 Windows API 擋住。
#[cfg(windows)]
pub mod edit;
#[cfg(windows)]
pub mod ipc;
#[cfg(windows)]
pub mod keys;

pub use session::{
    response_to_reply, state_to_snapshot_value, to_jc_request, Client, Mode, Segment, Session,
    Snapshot, Transport, PROTOCOL_VERSION,
};
