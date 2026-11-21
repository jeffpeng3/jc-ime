//! `jc-tsf-next`：照 0c 架構完整重做 TSF 的外殼。
//!
//! 目前階段（T6）已具備：T1a 的 `session` 與 State→Snapshot 轉換層、T1b 的
//! `ipc`／`keys`、T5 的 `edit`、T7a 的 `privacy`，以及本階段的 `service`（TIP
//! 狀態機、Present、兩條 su 硬要求）與 `langbar`（完整 ITfLangBarItemButton）。
//!
//! 非 Windows 平台只編 `session` 與 `log`，讓 Linux CI 的 `cargo check --workspace`
//! 不被 Windows API 擋住。真正的 DLL 註冊與 COM 工廠仍在舊 `jc-tsf` crate。

pub mod session;

/// 診斷紀錄、COM 邊界 `guarded`、物件計數。T7a 的過渡實作在 T6 改接這裡。
pub mod log;

// 讓移植自 0c 的模組維持 `crate::guarded()` / `crate::object_created()` 的寫法。
#[cfg(windows)]
#[allow(unused_imports)]
pub(crate) use log::{guarded, object_created, object_destroyed};

#[cfg(windows)]
pub mod caret;
#[cfg(windows)]
pub mod context;
#[cfg(windows)]
pub mod displayattr;
#[cfg(windows)]
pub mod dll;
#[cfg(windows)]
pub mod edit;
#[cfg(windows)]
pub mod guids;
#[cfg(windows)]
pub mod ipc;
#[cfg(windows)]
pub mod keys;
#[cfg(windows)]
pub mod langbar;
#[cfg(windows)]
pub mod msctf;
#[cfg(windows)]
pub mod privacy;
#[cfg(windows)]
pub mod service;

pub use session::{
    response_to_reply, state_to_snapshot_value, to_jc_request, Client, Mode, Segment, Session,
    Snapshot, Transport, PROTOCOL_VERSION,
};

// T7b：UIElement（候選窗）與 IMM32 相容層。
#[cfg(windows)]
pub mod cuas;
#[cfg(windows)]
pub mod uielement;
