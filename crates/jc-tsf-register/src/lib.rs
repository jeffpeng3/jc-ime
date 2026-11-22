//! 共用的 TSF COM 註冊常式。
//!
//! `jc-tsf`（舊 DLL）與 `jc-tsf-next`（新版 DLL）都呼叫這裡的
//! [`register_with`]／[`unregister_with`]，並各自傳入自己的 [`Registration`]。
//!
//! 這個 crate刻意只產出 rlib，不放 `DllGetClassObject` 等 COM 進入點。
//! 四個 COM 匯出留在各 DLL 自己的 `lib.rs`／`dll.rs`；如此一來
//! `cargo test --workspace` 把兩份 DLL 鏈進同一個執行檔時，不會再有
//! `LNK2005` 重複符號。
//!
//! 非 Windows 平台是空殼，與 `jc-tsf` 一致。

#![cfg(windows)]

mod guids;
mod register;

pub use guids::{
    registry_key, CLSID_JC_TSF, GUID_PROFILE_JC_TSF, LANGID_ZH_TW, PROFILE_DESCRIPTION,
};
pub use register::{register, register_with, unregister, unregister_with, Registration};

/// 把字串轉成以 NUL 結尾的 UTF-16，供 Win32 的 `PCWSTR` 使用。
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
