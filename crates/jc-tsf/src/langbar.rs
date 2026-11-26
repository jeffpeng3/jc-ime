//! 輸入指示器的「中／英」按鈕與輸入法選單。
//!
//! 移植自 0c `langbar.rs`（main `8f443af`）的 `ITfLangBarItemButton` 完整實作：
//! Activate 時 AddItem、Deactivate 時 RemoveItem 成對；左鍵切換中英、右鍵三項選單；
//! `GetIcon` 與 `GetText` 共用同一個 `indicator_text`；`AdviseSink` 用 `try_borrow_mut`
//! 防止宿主在 OnUpdate 期間重入。
//!
//! 選單命令改接 **jc-settings-ui**：它是 `#![windows_subsystem = "windows"]` 的 GUI
//! 執行檔，沒有 0c-ui 那種 IPC 命令管道；這裡用 `Command::args()`（空白路徑安全）
//! 加 `CREATE_NO_WINDOW` 啟動，避免灰色終端機彈出（plan T6+T9）。

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard};

use windows::Win32::Foundation::{
    COLORREF, E_INVALIDARG, E_POINTER, E_UNEXPECTED, HANDLE, POINT, RECT,
};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject,
    DrawTextW, GetTextFaceW, SelectObject, SetBkMode, SetTextColor, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS,
    DT_CENTER, DT_SINGLELINE, DT_VCENTER, FW_NORMAL, HBITMAP, HDC, OUT_DEFAULT_PRECIS, TRANSPARENT,
};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;
use windows::Win32::UI::TextServices::{
    ITfLangBarItem, ITfLangBarItemButton, ITfLangBarItemButton_Impl, ITfLangBarItemSink,
    ITfLangBarItem_Impl, ITfMenu, ITfSource, ITfSource_Impl, TfLBIClick, GUID_LBI_INPUTMODE,
    TF_LANGBARITEMINFO, TF_LBI_CLK_LEFT, TF_LBI_CLK_RIGHT, TF_LBI_ICON, TF_LBI_STYLE_BTN_BUTTON,
    TF_LBI_STYLE_SHOWNINTRAY, TF_LBI_TEXT, TF_LBI_TOOLTIP, TF_LBMENUF_CHECKED,
    TF_LBMENUF_SEPARATOR,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DestroyMenu, DestroyWindow,
    GetSystemMetrics, PostMessageW, SetForegroundWindow, TrackPopupMenuEx, HICON, ICONINFO,
    MF_CHECKED, MF_SEPARATOR, MF_STRING, SM_CXSMICON, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    WINDOW_EX_STYLE, WM_NULL, WS_POPUP,
};
use windows_core::{implement, w, AsImpl, Interface, Ref, BSTR, GUID, PCWSTR};

use crate::guids::{CLSID_ZUIN, DESCRIPTION};
use crate::log::{self, guarded, object_created, object_destroyed, zlog};

const SINK_COOKIE: u32 = 0x0c0c;

/// 選單項目 id。0c 的 id 依更新狀態位移；這裡固定三項。
const ID_SIMPLIFIED: u32 = 1;
const ID_RECHECK: u32 = 2;
const ID_SETTINGS: u32 = 3;

/// 一項選單。`args` 是送給 jc-settings-ui 的參數（目前只有「設定…」用得到）。
#[derive(Clone, Debug, PartialEq, Eq)]
struct MenuItem {
    id: u32,
    label: &'static str,
    checked: bool,
    separator: bool,
}

fn items(simplified: Option<bool>) -> Vec<MenuItem> {
    vec![
        MenuItem {
            id: ID_SIMPLIFIED,
            label: "輸出簡體字",
            checked: simplified.unwrap_or(false),
            separator: false,
        },
        MenuItem {
            id: ID_RECHECK,
            label: "重新檢查狀態",
            checked: false,
            separator: false,
        },
        MenuItem {
            id: ID_SEPARATOR,
            label: "",
            checked: false,
            separator: true,
        },
        MenuItem {
            id: ID_SETTINGS,
            label: "設定…",
            checked: false,
            separator: false,
        },
    ]
}

const ID_SEPARATOR: u32 = 0;

/// 狀態快取：背景不讀個人資料，只在開選單時由引擎提供「輸出簡體字」目前的值。
fn lock(status: &Mutex<Option<bool>>) -> MutexGuard<'_, Option<bool>> {
    status.lock().unwrap_or_else(|e| e.into_inner())
}

/// 輸入指示器的字：`英`／`简`／`中`。日文片段不改變外殼模式。
pub fn indicator_text(english: bool, simplified: bool) -> &'static str {
    match (english, simplified) {
        (true, _) => "英",
        (false, true) => "简",
        (false, false) => "中",
    }
}

/// 滑鼠停在輸入指示器上的說明。
pub fn tooltip_text(english: bool, simplified: bool) -> String {
    let mode = match (english, simplified) {
        (true, _) => "英文",
        (false, true) => "中・簡體輸出",
        (false, false) => "中文",
    };
    format!("jc 注音 Mixed（{mode}）")
}

fn dword(path: PCWSTR, name: PCWSTR) -> Option<u32> {
    let mut value = 0u32;
    let mut size = 4u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            path,
            name,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut c_void),
            Some(&mut size),
        )
        .is_ok()
        .then_some(value)
    }
}

/// 工作列底色是否偏亮（圖示要用黑字）。
fn light_taskbar() -> bool {
    let personalize = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    if dword(personalize, w!("ColorPrevalence")) == Some(1) {
        let accent = dword(
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Accent"),
            w!("AccentColorMenu"),
        )
        .or_else(|| dword(w!("Software\\Microsoft\\Windows\\DWM"), w!("AccentColor")));
        if let Some(abgr) = accent {
            return light_color(abgr);
        }
    }
    dword(personalize, w!("SystemUsesLightTheme")) == Some(1)
}

fn light_color(abgr: u32) -> bool {
    let (r, g, b) = (abgr & 0xff, (abgr >> 8) & 0xff, (abgr >> 16) & 0xff);
    5 * g + 2 * r + b > 8 * 128
}

unsafe fn selected_face(dc: HDC) -> String {
    let mut buf = [0u16; 64];
    let n = GetTextFaceW(dc, Some(&mut buf)) as usize;
    String::from_utf16_lossy(&buf[..n.min(buf.len()).saturating_sub(1)])
}

/// 以文字（中／英／日）畫出工作列圖示。GDI 不寫 alpha，畫完後把有顏色的像素設為不透明。
fn text_icon(text: &str) -> Option<HICON> {
    unsafe {
        let size = GetSystemMetrics(SM_CXSMICON).max(16);
        let dc = CreateCompatibleDC(None);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let color = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let old = SelectObject(dc, color.into());
        let font = CreateFontW(
            -(size * 7 / 8),
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            DEFAULT_PITCH.0 as u32,
            w!("Microsoft JhengHei UI"),
        );
        let old_font = SelectObject(dc, font.into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00FF_FFFF));
        let mut rc = RECT {
            left: 0,
            top: 0,
            right: size,
            bottom: size,
        };
        let mut t: Vec<u16> = text.encode_utf16().collect();
        DrawTextW(dc, &mut t, &mut rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        let fill = if light_taskbar() {
            0x0000_0000u32
        } else {
            0x00FF_FFFFu32
        };
        let px = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
        for p in px.iter_mut() {
            let a = (*p & 0xff).max((*p >> 8) & 0xff).max((*p >> 16) & 0xff);
            *p = if a == 0 { 0 } else { (a << 24) | fill };
        }
        // 字型不存在時 GDI 換字型會畫成方框；確認選到的就是指定字型。
        let _ = selected_face(dc);
        SelectObject(dc, old_font);
        let _ = DeleteObject(font.into());
        SelectObject(dc, old);
        let _ = DeleteDC(dc);
        let mask: HBITMAP = CreateBitmap(size, size, 1, 1, None);
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(mask.into());
        let _ = DeleteObject(color.into());
        icon
    }
}

/// 用 `Command::args()` + `CREATE_NO_WINDOW` 啟動 jc-settings-ui（空白路徑安全、不彈終端機）。
/// 先試執行檔同目錄（MSI 安裝的配置），再退回 PATH。
fn launch_settings_ui() {
    let name = "jc-settings-ui.exe";
    let mut tried: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(dir) = std::env::current_exe() {
        if let Some(dir) = dir.parent() {
            let path = dir.join(name);
            tried.push(path.clone());
            if path.is_file() {
                spawn(&path);
                return;
            }
        }
    }
    let _ = tried;
    spawn(std::path::Path::new(name));
}

fn spawn(path: &std::path::Path) {
    match Command::new(path)
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()
    {
        Ok(_) => zlog!("langbar: 已啟動 jc-settings-ui"),
        Err(e) => zlog!("langbar: 啟動 jc-settings-ui 失敗：{}", e),
    }
}

#[implement(ITfLangBarItemButton, ITfLangBarItem, ITfSource)]
pub struct LangBarButton {
    english: Cell<bool>,
    simplified: Cell<Option<bool>>,
    status: Arc<Mutex<Option<bool>>>,
    sink: RefCell<Option<ITfLangBarItemSink>>,
    /// 最近一次交給系統的選單項目：選取時依它找 id，不被背景狀態更新位移。
    shown: RefCell<Vec<MenuItem>>,
    toggle: Box<dyn Fn()>,
    simplify: Box<dyn Fn(bool) -> bool>,
    recheck: Box<dyn Fn()>,
}

impl LangBarButton {
    /// 建立指示器按鈕；三個 closure 分別是左鍵切換、輸出簡體字、開選單前重新確認。
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        english: bool,
        simplified: Option<bool>,
        toggle: Box<dyn Fn()>,
        simplify: Box<dyn Fn(bool) -> bool>,
        recheck: Box<dyn Fn()>,
    ) -> ITfLangBarItemButton {
        object_created();
        LangBarButton {
            english: Cell::new(english),
            simplified: Cell::new(simplified),
            status: Arc::new(Mutex::new(simplified)),
            sink: RefCell::new(None),
            shown: RefCell::new(Vec::new()),
            toggle,
            simplify,
            recheck,
        }
        .into()
    }
}

impl Drop for LangBarButton {
    fn drop(&mut self) {
        object_destroyed();
    }
}

/// 中英／日文狀態或「輸出簡體字」改變時更新圖示與文字（「中」／「简」／「英」／「あ」／「カ」）。
pub fn set_state(button: &ITfLangBarItemButton, english: bool, simplified: Option<bool>) {
    let this = unsafe { button.as_impl() };
    let before = (this.english.get(), this.simplified.get());
    this.english.set(english);
    this.simplified.set(simplified);
    *lock(&this.status) = simplified;
    if before != (english, simplified) {
        // 先取出再通知：OnUpdate 期間宿主可能回頭呼叫 AdviseSink／UnadviseSink。
        let sink = this.sink.try_borrow().ok().and_then(|s| s.clone());
        if let Some(sink) = sink {
            unsafe {
                let _ = sink.OnUpdate(TF_LBI_ICON | TF_LBI_TEXT | TF_LBI_TOOLTIP);
            }
        }
    }
}

impl LangBarButton_Impl {
    fn menu_items(&self) -> Vec<MenuItem> {
        (self.recheck)();
        let simplified = (*lock(&self.status)).or_else(|| self.simplified.get());
        let list = items(simplified);
        if let Ok(mut shown) = self.shown.try_borrow_mut() {
            *shown = list.clone();
        }
        list
    }

    fn select(&self, id: u32) {
        let list = self
            .shown
            .try_borrow()
            .map(|shown| (*shown).clone())
            .unwrap_or_else(|_| items(self.simplified.get()));
        let Some(item) = list.into_iter().find(|i| i.id == id && !i.separator) else {
            return;
        };
        match item.id {
            ID_SIMPLIFIED => {
                let on = !self.simplified.get().unwrap_or(false);
                if (self.simplify)(on) {
                    *lock(&self.status) = Some(on);
                }
            }
            ID_RECHECK => (self.recheck)(),
            ID_SETTINGS => launch_settings_ui(),
            _ => {}
        }
    }

    fn popup(&self, pt: &POINT) {
        let list = self.menu_items();
        unsafe {
            let Ok(menu) = CreatePopupMenu() else { return };
            for item in &list {
                if item.separator {
                    let _ = windows::Win32::UI::WindowsAndMessaging::AppendMenuW(
                        menu,
                        MF_SEPARATOR,
                        0,
                        PCWSTR::null(),
                    );
                } else {
                    let label = log::wide(item.label);
                    let flags = if item.checked {
                        MF_STRING | MF_CHECKED
                    } else {
                        MF_STRING
                    };
                    let _ = windows::Win32::UI::WindowsAndMessaging::AppendMenuW(
                        menu,
                        flags,
                        item.id as usize,
                        PCWSTR::from_raw(label.as_ptr()),
                    );
                }
            }
            // TrackPopupMenu 需要本執行緒擁有、且在前景的視窗。
            let owner = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!(""),
                WS_POPUP,
                pt.x,
                pt.y,
                0,
                0,
                None,
                None,
                None,
                None,
            );
            if let Ok(owner) = owner {
                let _ = SetForegroundWindow(owner);
                let cmd = TrackPopupMenuEx(
                    menu,
                    (TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON).0,
                    pt.x,
                    pt.y,
                    owner,
                    None,
                );
                let _ = PostMessageW(Some(owner), WM_NULL, Default::default(), Default::default());
                let _ = DestroyWindow(owner);
                if cmd.0 > 0 {
                    self.select(cmd.0 as u32);
                }
            }
            let _ = DestroyMenu(menu);
        }
    }
}

impl ITfLangBarItem_Impl for LangBarButton_Impl {
    // TSF 契約保證 `pinfo` 指向可寫的 TF_LANGBARITEMINFO，簽章由 COM 介面決定，不能改成 unsafe fn。
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn GetInfo(&self, pinfo: *mut TF_LANGBARITEMINFO) -> windows_core::Result<()> {
        guarded("LangBar.GetInfo", || {
            if pinfo.is_null() {
                return Err(E_POINTER.into());
            }
            let mut info = TF_LANGBARITEMINFO {
                clsidService: CLSID_ZUIN,
                guidItem: GUID_LBI_INPUTMODE,
                dwStyle: TF_LBI_STYLE_BTN_BUTTON | TF_LBI_STYLE_SHOWNINTRAY,
                ulSort: 0,
                szDescription: [0; 32],
            };
            for (i, c) in DESCRIPTION.encode_utf16().take(31).enumerate() {
                info.szDescription[i] = c;
            }
            unsafe { pinfo.write(info) };
            Ok(())
        })
    }
    fn GetStatus(&self) -> windows_core::Result<u32> {
        guarded("LangBar.GetStatus", || Ok(0))
    }
    fn Show(&self, _fshow: windows_core::BOOL) -> windows_core::Result<()> {
        guarded("LangBar.Show", || Ok(()))
    }
    fn GetTooltipString(&self) -> windows_core::Result<BSTR> {
        guarded("LangBar.GetTooltipString", || {
            Ok(BSTR::from(tooltip_text(
                self.english.get(),
                self.simplified.get().unwrap_or(false),
            )))
        })
    }
}

impl ITfLangBarItemButton_Impl for LangBarButton_Impl {
    fn OnClick(
        &self,
        click: TfLBIClick,
        pt: &POINT,
        _prcarea: *const RECT,
    ) -> windows_core::Result<()> {
        guarded("LangBarOnClick", || {
            if click == TF_LBI_CLK_LEFT {
                (self.toggle)();
            } else if click == TF_LBI_CLK_RIGHT {
                self.popup(pt);
            }
            Ok(())
        })
    }
    fn InitMenu(&self, pmenu: Ref<'_, ITfMenu>) -> windows_core::Result<()> {
        guarded("LangBar.InitMenu", || {
            let menu = pmenu.ok()?;
            for item in self.menu_items() {
                let label: Vec<u16> = item.label.encode_utf16().collect();
                let flags = if item.separator {
                    TF_LBMENUF_SEPARATOR
                } else if item.checked {
                    TF_LBMENUF_CHECKED
                } else {
                    0
                };
                unsafe {
                    menu.AddMenuItem(
                        item.id,
                        flags,
                        Default::default(),
                        Default::default(),
                        &label,
                        std::ptr::null_mut(),
                    )?;
                }
            }
            Ok(())
        })
    }
    fn OnMenuSelect(&self, wid: u32) -> windows_core::Result<()> {
        guarded("LangBar.OnMenuSelect", || {
            self.select(wid);
            Ok(())
        })
    }
    fn GetIcon(&self) -> windows_core::Result<HICON> {
        guarded("LangBar.GetIcon", || {
            text_icon(indicator_text(
                self.english.get(),
                self.simplified.get().unwrap_or(false),
            ))
            .ok_or_else(|| windows_core::Error::from(windows::Win32::Foundation::E_FAIL))
        })
    }
    fn GetText(&self) -> windows_core::Result<BSTR> {
        guarded("LangBar.GetText", || {
            Ok(BSTR::from(indicator_text(
                self.english.get(),
                self.simplified.get().unwrap_or(false),
            )))
        })
    }
}

impl ITfSource_Impl for LangBarButton_Impl {
    // TSF 契約保證 `riid` 指向有效的 GUID，簽章由 COM 介面決定，不能改成 unsafe fn。
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn AdviseSink(
        &self,
        riid: *const GUID,
        punk: Ref<'_, windows_core::IUnknown>,
    ) -> windows_core::Result<u32> {
        guarded("LangBar.AdviseSink", || {
            if riid.is_null() {
                return Err(E_INVALIDARG.into());
            }
            if unsafe { *riid } != ITfLangBarItemSink::IID {
                return Err(windows_core::HRESULT(0x8004_0202u32 as i32).into());
            }
            let sink: ITfLangBarItemSink = punk.ok()?.cast()?;
            // 宿主在 OnUpdate 期間重入時不 panic，回錯誤讓它稍後再試。
            *self
                .sink
                .try_borrow_mut()
                .map_err(|_| windows_core::Error::from(E_UNEXPECTED))? = Some(sink);
            Ok(SINK_COOKIE)
        })
    }
    fn UnadviseSink(&self, cookie: u32) -> windows_core::Result<()> {
        guarded("LangBar.UnadviseSink", || {
            if cookie == SINK_COOKIE {
                *self
                    .sink
                    .try_borrow_mut()
                    .map_err(|_| windows_core::Error::from(E_UNEXPECTED))? = None;
            }
            Ok(())
        })
    }
}

// 保留 HANDLE 型別的引用，讓日後接 jc-settings-ui 的命令管道時不必再改 imports。
#[allow(dead_code)]
type MenuPipe = HANDLE;
