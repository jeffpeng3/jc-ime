//! 共用的 TSF COM 註冊常式。
//!
//! `jc-tsf` 呼叫這裡的 [`register_with`]／[`unregister_with`]，傳入自己的
//! [`Registration`]。
//!
//! 這個 crate 刻意只產出 rlib，不放 `DllGetClassObject` 等 COM 進入點。
//! 四個 COM 匯出留在 DLL 自己的 `dll.rs`，避免 rlib 與 DLL 的符號相撞。
//!
//! 非 Windows 平台是空殼，與 `jc-tsf` 一致。

#![cfg(windows)]

mod guids;
mod register;

pub use guids::{
    registry_key, CLSID_JC_TSF, CLSID_JC_TSF_LEGACY, GUID_PROFILE_JC_TSF,
    GUID_PROFILE_JC_TSF_LEGACY, LANGID_ZH_TW, PIPE_PREFIX, PROFILE_DESCRIPTION,
};
pub use register::{register, register_with, unregister, unregister_with, Registration};

/// 把字串轉成以 NUL 結尾的 UTF-16，供 Win32 的 `PCWSTR` 使用。
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
