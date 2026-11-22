//! msctf.dll 直接匯出的建立函式（COM-less 啟用，#157）。
//!
//! 以 `TF_TMAE_COMLESS` 啟用本輸入法的宿主可能沒有初始化 COM，`Activate` 不能呼叫
//! `CoCreateInstance`。改用 msctf 的 `TF_CreateCategoryMgr` 建立類別管理員來註冊
//! 顯示屬性的 GUID atom。0c 用 `windows_link::link!`，T0 的 API 對照要求改成
//! windows-core 0.62 的 `windows_core::link!`（raw-dylib，x86 為未修飾名稱）。

use windows::Win32::UI::TextServices::ITfCategoryMgr;
use windows_core::{Interface, HRESULT};

mod ffi {
    windows_core::link!(
        "msctf.dll" "system" fn TF_CreateCategoryMgr(ppcat: *mut *mut core::ffi::c_void) -> windows_core::HRESULT
    );
}

/// 不經 COM 建立類別管理員。回傳的物件由 msctf 持有，用完照常 Release（Drop）即可。
pub fn create_category_mgr() -> windows_core::Result<ITfCategoryMgr> {
    let mut raw: *mut core::ffi::c_void = std::ptr::null_mut();
    let hr: HRESULT = unsafe { ffi::TF_CreateCategoryMgr(&mut raw) };
    hr.ok()?;
    if raw.is_null() {
        return Err(windows::Win32::Foundation::E_POINTER.into());
    }
    Ok(unsafe { ITfCategoryMgr::from_raw(raw) })
}
