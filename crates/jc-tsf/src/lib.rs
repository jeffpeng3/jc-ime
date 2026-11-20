//! Windows TSF 文字服務骨架（第一版）。
//!
//! 只做 COM 註冊、按鍵、組字、提交與非同步 IPC。候選窗、設定 UI、安裝器都不在
//! 這個 crate 的範圍；`docs/port/tsf-skeleton.md` 有完整的行為與未驗證清單。
//!
//! 同一個 crate 建兩份：`x86_64-pc-windows-msvc` 出 `jc-tsf.dll`（x64），
//! `i686-pc-windows-msvc` 出 `jc-tsf-x86.dll`。兩份的差別只有 `guids.rs` 依
//! `target_pointer_width` 選到的 CLSID、profile GUID 與顯示名稱；TSF 依宿主行程
//! 的位元數載入對應的那一份，理由與註冊流程見 `docs/port/x86-tip.md`。
//!
//! 非 Windows 平台上整個 crate 是空的：Linux CI 的 `cargo check --workspace`
//! 會編出一個沒有任何符號的 cdylib，不提供替代實作。

#![cfg(windows)]

mod candidates;
mod composition;
mod display;
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
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_INVALIDARG, E_NOINTERFACE, LPARAM, S_FALSE,
    S_OK, WPARAM,
};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::Win32::UI::TextServices::{
    IEnumTfDisplayAttributeInfo, ITfCompositionSink, ITfCompositionSink_Impl, ITfContext,
    ITfDisplayAttributeInfo, ITfDisplayAttributeProvider, ITfDisplayAttributeProvider_Impl,
    ITfKeyEventSink, ITfKeyEventSink_Impl, ITfTextInputProcessor, ITfTextInputProcessor_Impl,
    ITfThreadMgr, ITfThreadMgrEventSink, ITfThreadMgrEventSink_Impl, TF_E_NOPROVIDER,
};

use composition::Control;
use display::{DisplayAttributeEnumerator, DisplayAttributeInfo};
use guids::{registry_key, CLSID_JC_TSF, GUID_DISPLAY_ATTRIBUTE_INPUT};

/// 目前仍然活著的文字服務物件數量，供 [`DllCanUnloadNow`] 判斷能否卸載。
static LIVE_OBJECTS: AtomicUsize = AtomicUsize::new(0);

/// 日誌用：這份 DLL 的架構與 CLSID。
fn build_identity() -> String {
    format!(
        "{}，{}",
        std::env::consts::ARCH,
        registry_key(&CLSID_JC_TSF)
    )
}

/// 文字服務物件。
///
/// 一個執行緒一個實例，生命週期由 TSF 以 `Activate`／`Deactivate` 控制。真正的
/// 狀態都放在 [`Control`]；這裡只留一個可換掉的欄位，讓 `Deactivate` 能明確
/// 清掉 sink 與視窗。
#[implement(
    ITfTextInputProcessor,
    ITfThreadMgrEventSink,
    ITfKeyEventSink,
    ITfCompositionSink,
    ITfDisplayAttributeProvider
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

impl ITfDisplayAttributeProvider_Impl for TextService_Impl {
    fn EnumDisplayAttributeInfo(&self) -> Result<IEnumTfDisplayAttributeInfo> {
        let enumerator: IEnumTfDisplayAttributeInfo = DisplayAttributeEnumerator::new().into();
        Ok(enumerator)
    }

    // COM 的出參指標由呼叫端保證有效；trait 方法本身不能標成 unsafe。
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn GetDisplayAttributeInfo(&self, guid: *const GUID) -> Result<ITfDisplayAttributeInfo> {
        if guid.is_null() {
            return Err(E_INVALIDARG.into());
        }
        // SAFETY: 呼叫端保證 guid 指向一個有效的 GUID。
        if unsafe { *guid } != GUID_DISPLAY_ATTRIBUTE_INPUT {
            return Err(TF_E_NOPROVIDER.into());
        }
        let info: ITfDisplayAttributeInfo = DisplayAttributeInfo.into();
        Ok(info)
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
            logging::info(format!("註冊完成（{}）", build_identity()));
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
            logging::info(format!("反註冊完成（{}）", build_identity()));
            S_OK
        }
        Err(error) => {
            logging::warn(format!("反註冊失敗：{error}"));
            error.code()
        }
    }
}

/// 冒煙測試用的按鍵直驅匯出：虛擬鍵 → 引擎 → 回應碼。
///
/// 不經過 TSF 的 context 與組字套用，只驗證「按鍵 → 引擎」這一段；回傳的是
/// 引擎有沒有正常回應，不代表畫面上出現什麼。`jc-tsf-smoke.exe` 用它量測
/// DLL 與引擎的實際連線，不需要管理員、也不需要安裝輸入法。
///
/// 回傳值：
///
/// - `0`：按鍵已送出，引擎也回應了
/// - `1`：這個鍵不歸輸入法處理，或 `vk` 不是合法的虛擬鍵碼
/// - `2`：連不上引擎 pipe
/// - `3`：引擎往返失敗（逾時、協定錯誤、連線已關閉）
///
/// 只在 `debug_assertions` 或 feature `test-export` 下編譯，出貨版不會帶這個
/// 符號。
#[cfg(any(debug_assertions, feature = "test-export"))]
#[no_mangle]
pub extern "system" fn jc_test_key(vk: u32) -> i32 {
    use std::time::Duration;

    use jc_proto::Request;
    use jc_windows::Client;
    use windows::Win32::UI::Input::KeyboardAndMouse::{MapVirtualKeyW, MAPVK_VK_TO_VSC};

    /// 這個鍵不歸輸入法處理。
    const NOT_HANDLED: i32 = 1;
    /// 連不上引擎。
    const CONNECT_FAILED: i32 = 2;
    /// 引擎往返失敗。
    const ENGINE_FAILED: i32 = 3;

    let Ok(vk) = u16::try_from(vk) else {
        return NOT_HANDLED;
    };
    // SAFETY: MapVirtualKeyW 只讀兩個純量參數。
    let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) };
    let Some(batch) = keys::translate(vk, scan, false) else {
        return NOT_HANDLED;
    };

    // 這條連線與 TSF 的 worker 無關：冒煙程式是在自己的行程裡直接驅動 DLL。
    let name = engine::pipe_name();
    let client_name = concat!("jc-tsf-smoke/", env!("CARGO_PKG_VERSION"));
    let mut client = match Client::connect_with_retry(
        &name,
        client_name,
        engine::REQUEST_TIMEOUT,
        2,
        Duration::from_millis(100),
    ) {
        Ok(client) => client,
        Err(error) => {
            logging::debug(format!("jc_test_key 連線失敗：{error}"));
            return CONNECT_FAILED;
        }
    };

    for key in batch {
        if let Err(error) = client.request(&Request::Key { key }, engine::REQUEST_TIMEOUT) {
            logging::debug(format!("jc_test_key 引擎往返失敗：{error}"));
            return ENGINE_FAILED;
        }
    }
    0
}

/// 把字串轉成以 NUL 結尾的 UTF-16，供 Win32 的 `PCWSTR` 使用。
pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
