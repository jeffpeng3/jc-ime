//! Windows TSF 文字服務骨架（第一版）。
//!
//! 只做 COM 註冊、按鍵、組字、提交與非同步 IPC。候選窗、設定 UI、安裝器都不在
//! 這個 crate 的範圍；`docs/port/tsf-skeleton.md` 有完整的行為與未驗證清單。
//!
//! 非 Windows 平台上整個 crate 是空的：Linux CI 的 `cargo check --workspace`
//! 會編出一個沒有任何符號的 cdylib，不提供替代實作。

#![cfg(windows)]

mod composition;
mod engine;
mod guids;
mod keys;
mod logging;
mod register;

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use windows::core::{
    implement, ComObjectInterface, IUnknown, IUnknownImpl, Interface, Ref, Result, BOOL, GUID,
    HRESULT,
};
use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_NOINTERFACE, LPARAM, S_FALSE, S_OK, WPARAM,
};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::Win32::UI::TextServices::{
    ITfCompositionSink, ITfCompositionSink_Impl, ITfContext, ITfKeyEventSink, ITfKeyEventSink_Impl,
    ITfTextInputProcessor, ITfTextInputProcessor_Impl, ITfThreadMgr, ITfThreadMgrEventSink,
    ITfThreadMgrEventSink_Impl,
};

use composition::Control;
use guids::CLSID_JC_TSF;

/// 目前仍然活著的文字服務物件數量，供 [`DllCanUnloadNow`] 判斷能否卸載。
static LIVE_OBJECTS: AtomicUsize = AtomicUsize::new(0);

/// 文字服務物件。
///
/// 一個執行緒一個實例，生命週期由 TSF 以 `Activate`／`Deactivate` 控制。真正的
/// 狀態都放在 [`Control`]；這裡只留一個可換掉的欄位，讓 `Deactivate` 能明確
/// 清掉 sink 與視窗。
#[implement(
    ITfTextInputProcessor,
    ITfThreadMgrEventSink,
    ITfKeyEventSink,
    ITfCompositionSink
)]
pub struct TextService {
    /// 已啟用的狀態；`None` 代表尚未 `Activate` 或已經 `Deactivate`。
    control: RefCell<Option<Box<Control>>>,
}

impl TextService {
    /// 建立文字服務物件；`Activate` 之前沒有任何資源。
    fn new() -> Self {
        LIVE_OBJECTS.fetch_add(1, Ordering::Relaxed);
        Self {
            control: RefCell::new(None),
        }
    }
}

impl Drop for TextService {
    fn drop(&mut self) {
        LIVE_OBJECTS.fetch_sub(1, Ordering::Relaxed);
    }
}

impl ITfTextInputProcessor_Impl for TextService_Impl {
    fn Activate(&self, ptim: Ref<ITfThreadMgr>, tid: u32) -> Result<()> {
        let this = self.get_impl();
        if this.control.borrow().is_some() {
            logging::warn("重複 Activate，忽略");
            return Ok(());
        }
        let Some(thread_mgr) = ptim.as_ref() else {
            logging::warn("Activate 收到空的 ITfThreadMgr");
            return Ok(());
        };

        // sink 就是這個物件本身；這兩個引用會在 Deactivate 明確丟掉，否則會
        // 形成自己引用自己的循環，`DllCanUnloadNow` 永遠不會回 S_OK。
        let key_sink: ITfKeyEventSink =
            ComObjectInterface::<ITfKeyEventSink>::as_interface_ref(self).to_owned();
        let thread_sink: ITfThreadMgrEventSink =
            ComObjectInterface::<ITfThreadMgrEventSink>::as_interface_ref(self).to_owned();

        let control = Control::new(thread_mgr.clone(), tid);
        match Control::start(Box::new(control), key_sink, thread_sink) {
            Ok(control) => {
                this.control.replace(Some(control));
                logging::info("文字服務已啟用");
                Ok(())
            }
            Err(error) => {
                // 啟用失敗要讓 TSF 知道，否則它會以為這個 TIP 可以使用。
                logging::warn(format!("啟用失敗：{error}"));
                Err(error)
            }
        }
    }

    fn Deactivate(&self) -> Result<()> {
        let this = self.get_impl();
        // 先取出 Box 再 shutdown：`borrow_mut` 的守衛不能在 shutdown 期間活著，
        // 否則 TSF 若在清理時回呼任何 sink 就會撞到 RefCell。
        let control = this.control.borrow_mut().take();
        if let Some(mut control) = control {
            control.shutdown();
        }
        logging::info("文字服務已停用");
        Ok(())
    }
}

impl ITfThreadMgrEventSink_Impl for TextService_Impl {
    fn OnInitDocumentMgr(
        &self,
        _pdim: Ref<windows::Win32::UI::TextServices::ITfDocumentMgr>,
    ) -> Result<()> {
        Ok(())
    }

    fn OnUninitDocumentMgr(
        &self,
        _pdim: Ref<windows::Win32::UI::TextServices::ITfDocumentMgr>,
    ) -> Result<()> {
        Ok(())
    }

    fn OnSetFocus(
        &self,
        _pdimfocus: Ref<windows::Win32::UI::TextServices::ITfDocumentMgr>,
        _pdimprevfocus: Ref<windows::Win32::UI::TextServices::ITfDocumentMgr>,
    ) -> Result<()> {
        if let Some(control) = self.get_impl().control.borrow().as_deref() {
            control.on_focus_changed();
        }
        Ok(())
    }

    fn OnPushContext(&self, _pic: Ref<ITfContext>) -> Result<()> {
        Ok(())
    }

    fn OnPopContext(&self, _pic: Ref<ITfContext>) -> Result<()> {
        Ok(())
    }
}

impl ITfKeyEventSink_Impl for TextService_Impl {
    fn OnSetFocus(&self, _fforeground: BOOL) -> Result<()> {
        Ok(())
    }

    fn OnTestKeyDown(&self, _pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        let borrowed = self.get_impl().control.borrow();
        let Some(control) = borrowed.as_deref() else {
            return Ok(BOOL(0));
        };
        Ok(BOOL(control.test_key(wparam.0, lparam.0) as i32))
    }

    fn OnTestKeyUp(&self, _pic: Ref<ITfContext>, _wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        Ok(BOOL(0))
    }

    fn OnKeyDown(&self, pic: Ref<ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        let borrowed = self.get_impl().control.borrow();
        let Some(control) = borrowed.as_deref() else {
            return Ok(BOOL(0));
        };
        Ok(BOOL(
            control.key_down(pic.as_ref(), wparam.0, lparam.0) as i32
        ))
    }

    fn OnKeyUp(&self, _pic: Ref<ITfContext>, _wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        Ok(BOOL(0))
    }

    fn OnPreservedKey(&self, _pic: Ref<ITfContext>, _rguid: *const GUID) -> Result<BOOL> {
        Ok(BOOL(0))
    }
}

impl ITfCompositionSink_Impl for TextService_Impl {
    fn OnCompositionTerminated(
        &self,
        _ecwrite: u32,
        _pcomposition: Ref<windows::Win32::UI::TextServices::ITfComposition>,
    ) -> Result<()> {
        // 正常的結束路徑（我們自己 EndComposition）會先把 `composition` 清掉，
        // 所以這裡只在外部終止時才真的有事情要做。
        if let Some(control) = self.get_impl().control.borrow().as_deref() {
            control.on_composition_terminated();
        }
        Ok(())
    }
}

/// 建立文字服務實例的 class factory。
#[implement(IClassFactory)]
struct ClassFactory;

impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(
        &self,
        punkouter: Ref<IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut c_void,
    ) -> Result<()> {
        if !punkouter.is_null() {
            logging::warn("不支援 aggregation");
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let service: ITfTextInputProcessor = TextService::new().into();
        let unknown: IUnknown = service.cast()?;
        let hr = unsafe { unknown.query(riid, ppvobject) };
        hr.ok()
    }

    fn LockServer(&self, _flock: BOOL) -> Result<()> {
        Ok(())
    }
}

/// COM 進入點：建立 class factory。
///
/// # Safety
///
/// 由 COM 呼叫，`rclsid`／`riid`／`ppv` 必須符合 COM 的指標約定。
#[no_mangle]
#[allow(non_snake_case)]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if rclsid.is_null() || riid.is_null() || ppv.is_null() {
        return E_NOINTERFACE;
    }
    // SAFETY: 呼叫端保證三個指標都有效。
    let clsid = unsafe { *rclsid };
    if clsid != CLSID_JC_TSF {
        return CLASS_E_CLASSNOTAVAILABLE;
    }

    let factory: IClassFactory = ClassFactory.into();
    match factory.cast::<IUnknown>() {
        Ok(unknown) => unsafe { unknown.query(riid, ppv) },
        Err(error) => error.code(),
    }
}

/// COM 進入點：還有物件或 lock 時回 `S_FALSE`。
#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if LIVE_OBJECTS.load(Ordering::Relaxed) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}

/// COM 進入點：註冊文字服務。
///
/// 需要系統管理員權限（寫 `HKCR` 與 TSF 的 profile 資料庫）。
#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    match register::register() {
        Ok(()) => {
            logging::info("註冊完成");
            S_OK
        }
        Err(error) => {
            logging::warn(format!("註冊失敗：{error}"));
            error.code()
        }
    }
}

/// COM 進入點：反註冊文字服務。
#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    match register::unregister() {
        Ok(()) => {
            logging::info("反註冊完成");
            S_OK
        }
        Err(error) => {
            logging::warn(format!("反註冊失敗：{error}"));
            error.code()
        }
    }
}

/// 把字串轉成以 NUL 結尾的 UTF-16，供 Win32 的 `PCWSTR` 使用。
pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
