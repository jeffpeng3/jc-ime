//! COM DLL 外殼：`DllGetClassObject`／`DllCanUnloadNow`／`DllRegisterServer`／
//! `DllUnregisterServer`。移植自 0c `lib.rs` + `factory.rs`（main `8f443af`）。
//!
//! T0 的 API 對照在這裡的落點：
//!
//! - `windows_link::link!` → `windows_core::link!`：外殼本身沒有 link! 宣告，
//!   既有移植見 `msctf.rs`。
//! - `Error::from_win32()` → `Error::from_thread()`：外殼不直接取 Win32 錯誤，
//!   註冊過程的取法在 `jc-tsf/src/register.rs::module_path()`。
//! - `Ref<'_, T>` → `Ref<T>`：`CreateInstance` 的 `punkouter` 用省略生命週期的
//!   `Ref<IUnknown>`，符合 windows 0.62 的 `IClassFactory_Impl` 簽章。

#![allow(non_snake_case)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_POINTER, S_FALSE, S_OK,
};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows_core::{implement, ComObject, IUnknown, Interface, Ref, BOOL, GUID, HRESULT};

use crate::guids::{
    CLSID_JC_TSF_NEXT, GUID_PROFILE_JC_TSF_NEXT, LANGID, PIPE_PREFIX_NEXT, PROFILE_DESCRIPTION_NEXT,
};
use crate::log::{self, guarded, LIVE_OBJECTS};
use crate::service::TextService;

/// `LockServer(true)` 的次數。與 [`LIVE_OBJECTS`] 分開計：前者是 COM 明確要求
/// DLL 留在記憶體，後者是還活著的 COM 物件。兩者都歸零才可以卸載。
static DLL_LOCKS: AtomicIsize = AtomicIsize::new(0);

/// 建立 [`TextService`] 實例的 class factory。
///
/// 和 0c 一樣，工廠本身也算一個 live object，`Drop` 時扣回。
#[implement(IClassFactory)]
struct ClassFactory;

impl ClassFactory {
    fn new() -> Self {
        log::object_created();
        Self
    }
}

impl Drop for ClassFactory {
    fn drop(&mut self) {
        log::object_destroyed();
    }
}

impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(
        &self,
        punkouter: Ref<IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut c_void,
    ) -> windows_core::Result<()> {
        guarded("CreateInstance", || {
            if ppvobject.is_null() || riid.is_null() {
                return Err(E_POINTER.into());
            }
            // SAFETY: COM 呼叫端保證 ppvobject 可寫。
            unsafe { *ppvobject = std::ptr::null_mut() };
            if !punkouter.is_null() {
                return Err(CLASS_E_NOAGGREGATION.into());
            }
            // T6 的 TextService 自帶 LIVE_OBJECTS 計數與 Activate idempotent guard。
            let unknown: IUnknown = ComObject::new(TextService::new()).into_interface();
            // SAFETY: riid／ppvobject 由 COM 呼叫端提供，已確認非 NULL。
            unsafe { unknown.query(riid, ppvobject).ok() }
        })
    }

    fn LockServer(&self, flock: BOOL) -> windows_core::Result<()> {
        guarded("LockServer", || {
            if flock.as_bool() {
                DLL_LOCKS.fetch_add(1, Ordering::Relaxed);
            } else {
                DLL_LOCKS.fetch_sub(1, Ordering::Relaxed);
            }
            Ok(())
        })
    }
}

/// 新版 DLL 專用的註冊身分；值全部取自 `guids.rs` 的 T9 段。
fn next_registration() -> jc_tsf::register::Registration {
    jc_tsf::register::Registration {
        clsid: CLSID_JC_TSF_NEXT,
        profile: GUID_PROFILE_JC_TSF_NEXT,
        description: PROFILE_DESCRIPTION_NEXT,
        langid: LANGID,
        pipe_prefix: PIPE_PREFIX_NEXT,
    }
}

/// COM 進入點：`rclsid` 必須是新版 CLSID，其他一律回 `CLASS_E_CLASSNOTAVAILABLE`。
#[no_mangle]
pub extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    log::install_panic_hook();
    match guarded("DllGetClassObject", || {
        if rclsid.is_null() || riid.is_null() || ppv.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: 三個指標已確認非 NULL。
        unsafe {
            *ppv = std::ptr::null_mut();
            if *rclsid != CLSID_JC_TSF_NEXT {
                return Err(CLASS_E_CLASSNOTAVAILABLE.into());
            }
        }
        let factory: IUnknown = ComObject::new(ClassFactory::new()).into_interface();
        // SAFETY: riid／ppv 由 COM 呼叫端提供，已確認非 NULL。
        unsafe { factory.query(riid, ppv).ok() }
    }) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

/// COM 進入點：沒物件也沒 DLL lock 才回 `S_OK`。
#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if LIVE_OBJECTS.load(Ordering::Relaxed) == 0 && DLL_LOCKS.load(Ordering::Relaxed) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}

/// COM 進入點：用新版身分註冊，寫入的 `InprocServer32` 就是這份 DLL。
#[no_mangle]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    log::install_panic_hook();
    match guarded("DllRegisterServer", || {
        jc_tsf::register::register_with(&next_registration())
    }) {
        Ok(()) => S_OK,
        Err(error) => {
            log::log(&format!("註冊失敗：{error}"));
            error.code()
        }
    }
}

/// COM 進入點：只反註冊新版身分，舊版已發行的 CLSID/profile 不受影響。
#[no_mangle]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    match guarded("DllUnregisterServer", || {
        jc_tsf::register::unregister_with(&next_registration())
    }) {
        Ok(()) => S_OK,
        Err(error) => {
            log::log(&format!("反註冊失敗：{error}"));
            error.code()
        }
    }
}
