//! TSF DLL 內的候選窗：HWND + Direct2D + DirectComposition surface。
//!
//! 這個視窗不建立執行緒、不開新程序，也不碰 `jc-ui`。它只活在 TSF 的 UI 執行緒上：
//! `present` 在按鍵路徑同步更新內容，滑鼠點選則把 `WM_SELECT_CANDIDATE` 送回
//! `candidate::CandidateWindow` 的訊息視窗，由原本的 edit session 核對焦點後才選字。
//!
//! 幾個硬性約束：
//! - `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`：點視窗不會搶焦點，也不會出現在工作列。
//! - 一律 `SW_SHOWNOACTIVATE`／`SWP_NOACTIVATE`，絕不呼叫 `SetFocus`。
//! - 呈現用 `IDCompositionSurface`，不是 swap chain：surface 尺寸固定，換尺寸就重建，
//!   每次畫完 `surface.EndDraw()` 再 `comp.Commit()`。繪製放在 `WM_PAINT`。
//! - `WS_EX_NOREDIRECTIONBITMAP` 是 DComp 的前提；沒有它的話系統會準備一張不透明的
//!   重導向點陣圖，把合成器畫的內容蓋掉。
//! - TSF 的按鍵路徑是同步的、訊息迴圈沒在跑，所以更新後要 `InvalidateRect` 再
//!   `UpdateWindow`，`WM_PAINT` 才會真的發生、畫面才會合成。
//! - 初始化失敗只記一行日誌並收起，不在輸入路徑重試；下一次 Activate 再說。

use std::ffi::c_void;

use windows::Win32::Foundation::{
    GetLastError, SetLastError, ERROR_SUCCESS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT,
    WPARAM,
};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap1, ID2D1DeviceContext, ID2D1Factory1, ID2D1SolidColorBrush,
    D2D1_BITMAP_OPTIONS, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET,
    D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_DRAW_TEXT_OPTIONS_NONE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1,
    D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionSurface, IDCompositionTarget,
    IDCompositionVisual,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS,
    DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM,
};
use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGISurface};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetMonitorInfoW, InvalidateRect, MonitorFromPoint, UpdateWindow,
    MONITORINFO, MONITOR_DEFAULTTONEAREST, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetSystemMetrics,
    GetWindowLongPtrW, GetWindowRect, IsWindowVisible, LoadCursorW, PostMessageW, RegisterClassW,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, UnregisterClassW, CS_DROPSHADOW, CS_HREDRAW,
    CS_VREDRAW, GWLP_USERDATA, GWL_EXSTYLE, GWL_STYLE, HWND_TOPMOST, IDC_ARROW, MA_NOACTIVATE,
    SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_SHOWWINDOW, SW_HIDE,
    SW_SHOWNOACTIVATE, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_MOUSEACTIVATE,
    WM_NCDESTROY, WM_PAINT, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows_core::{w, Interface, PCWSTR};

use crate::candidate_layout::{
    self, px_from_dip, Content, DipRect, Layout, Rect, TextStyle, BASE_DPI, CORNER_RADIUS,
};
use crate::log::zlog;
use crate::ui::{UiMessage, WM_SELECT_CANDIDATE};

const CLASS_NAME: PCWSTR = w!("jcCandidateUiWindow");
/// 候選窗與插入點的距離（DIP）。
const GAP_DIP: f32 = 4.0;

// 32-bit 的 `SetWindowLongPtrW`／`GetWindowLongPtrW` 在 windows-rs 收 `i32`，
// 64-bit 收 `isize`；差異只留在這兩個小函式裡。
#[cfg(target_pointer_width = "64")]
fn set_userdata(hwnd: HWND, value: isize) {
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, value) };
}

#[cfg(target_pointer_width = "32")]
fn set_userdata(hwnd: HWND, value: isize) {
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, value as i32) };
}

#[cfg(target_pointer_width = "64")]
fn userdata(hwnd: HWND) -> isize {
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) }
}

#[cfg(target_pointer_width = "32")]
fn userdata(hwnd: HWND) -> isize {
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as isize }
}

/// 淺色／深色主題的一組顏色。
#[derive(Clone, Copy)]
struct Palette {
    background: D2D1_COLOR_F,
    border: D2D1_COLOR_F,
    text: D2D1_COLOR_F,
    dim: D2D1_COLOR_F,
    highlight: D2D1_COLOR_F,
    highlight_text: D2D1_COLOR_F,
}

fn color(r: f32, g: f32, b: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a: 1.0 }
}

fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            background: color(0.145, 0.145, 0.153),
            border: color(0.31, 0.31, 0.33),
            text: color(0.953, 0.953, 0.953),
            dim: color(0.66, 0.66, 0.66),
            highlight: color(0.21, 0.33, 0.45),
            highlight_text: color(1.0, 1.0, 1.0),
        }
    } else {
        Palette {
            background: color(0.996, 0.996, 0.992),
            border: color(0.78, 0.78, 0.78),
            text: color(0.106, 0.106, 0.106),
            dim: color(0.42, 0.42, 0.42),
            highlight: color(0.859, 0.914, 0.973),
            highlight_text: color(0.063, 0.137, 0.227),
        }
    }
}

/// 系統淺色／深色：`AppsUseLightTheme` 為 0 就是深色。讀不到時當淺色。
fn theme_is_dark() -> bool {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let mut value: u32 = 1;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut c_void),
            Some(&mut size),
        )
    };
    status.is_ok() && value == 0
}

/// D3D11 裝置：先試硬體，失敗（例如遠端桌面）再退 WARP。
fn create_device() -> windows_core::Result<ID3D11Device> {
    let levels = [
        D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_1,
        D3D_FEATURE_LEVEL_10_0,
    ];
    for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let mut device: Option<ID3D11Device> = None;
        let created = unsafe {
            D3D11CreateDevice(
                None,
                driver,
                Default::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )
        };
        if let (Ok(()), Some(device)) = (created, device) {
            return Ok(device);
        }
    }
    Err(windows_core::Error::from(windows_core::HRESULT(
        0x8000_4005u32 as i32,
    )))
}

/// 執行一次會回 `HRESULT` 的步驟：失敗時寫出階段名稱與數值，方便實機定位。
fn stage<T>(step: &str, result: windows_core::Result<T>) -> windows_core::Result<T> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            zlog!(
                "ui: 候選窗 {step} 失敗 hr={:#010x} {error}",
                error.code().0 as u32
            );
            Err(error)
        }
    }
}

/// DirectWrite 的量測與文字格式。
///
/// 與呈現資源分開：surface 要用最終尺寸建立，所以得先有字型才能算版面。
struct Text {
    write: IDWriteFactory,
    format_text: IDWriteTextFormat,
    format_key: IDWriteTextFormat,
    format_small: IDWriteTextFormat,
    format_pager: IDWriteTextFormat,
}

impl Text {
    fn new() -> windows_core::Result<Text> {
        unsafe {
            let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let format_text =
                text_format(&write, TextStyle::Candidate, DWRITE_TEXT_ALIGNMENT_LEADING)?;
            let format_key = text_format(&write, TextStyle::Key, DWRITE_TEXT_ALIGNMENT_LEADING)?;
            let format_small =
                text_format(&write, TextStyle::Small, DWRITE_TEXT_ALIGNMENT_LEADING)?;
            let format_pager =
                text_format(&write, TextStyle::Small, DWRITE_TEXT_ALIGNMENT_TRAILING)?;
            Ok(Text {
                write,
                format_text,
                format_key,
                format_small,
                format_pager,
            })
        }
    }

    fn format(&self, style: TextStyle) -> &IDWriteTextFormat {
        match style {
            TextStyle::Candidate => &self.format_text,
            TextStyle::Key => &self.format_key,
            TextStyle::Small => &self.format_small,
        }
    }

    /// 文字寬度（DIP）。量測與繪製用同一個 `IDWriteTextFormat`。
    fn measure(&self, text: &str, style: TextStyle) -> f32 {
        let format = self.format(style);
        let utf16: Vec<u16> = text.encode_utf16().collect();
        let layout = unsafe { self.write.CreateTextLayout(&utf16, format, 4096.0, 64.0) };
        match layout {
            Ok(layout) => {
                let mut metrics = DWRITE_TEXT_METRICS::default();
                if unsafe { layout.GetMetrics(&mut metrics) }.is_err() {
                    return 0.0;
                }
                metrics.width
            }
            Err(_) => 0.0,
        }
    }
}

/// 候選窗的呈現資源：D2D device context + DirectComposition surface。
///
/// surface 是固定尺寸的：換尺寸就重建一條，重新 `SetContent` 再 `Commit`。
/// `visual`／`comp_target` 要留在 struct 裡，不然合成樹會斷。
/// 建立後就一直留到 `LocalWindow` 被銷毀（Deactivate）；繪製失敗時由呼叫端
/// 把整個 `Graphics` 丟掉，下一次顯示重建。
struct Graphics {
    dc: ID2D1DeviceContext,
    surface: IDCompositionSurface,
    comp: IDCompositionDevice,
    /// surface 的內容容器；換 surface 後要重新 `SetContent`。
    visual: IDCompositionVisual,
    /// `CreateTargetForHwnd` 的結果；`SetRoot` 之後要留著。
    /// 只為了持有參照：合成目標要活著，不然合成樹會斷。
    _comp_target: IDCompositionTarget,
    brush_background: ID2D1SolidColorBrush,
    brush_border: ID2D1SolidColorBrush,
    brush_text: ID2D1SolidColorBrush,
    brush_dim: ID2D1SolidColorBrush,
    brush_highlight: ID2D1SolidColorBrush,
    brush_highlight_text: ID2D1SolidColorBrush,
    dark: bool,
    /// 目前 surface 的像素尺寸；改變時重建。
    size: (u32, u32),
    /// 最近一次 `surface.EndDraw` 的 HRESULT；0 代表成功。診斷用。
    present_hr: i32,
    /// 最近一次 `comp.Commit` 的 HRESULT；0 代表成功。診斷用。
    commit_hr: i32,
}

impl Graphics {
    /// 在視窗上建立呈現資源；surface 用最終像素尺寸建立。
    fn new(
        hwnd: HWND,
        dark: bool,
        width: u32,
        height: u32,
        dpi: u32,
    ) -> windows_core::Result<Graphics> {
        let (width, height) = (width.max(1), height.max(1));
        unsafe {
            let device = stage("D3D11CreateDevice", create_device())?;
            let dxgi: IDXGIDevice = stage("IDXGIDevice::cast", device.cast())?;

            let factory: ID2D1Factory1 = stage(
                "D2D1CreateFactory",
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None),
            )?;
            let d2d_device = stage("CreateDevice", factory.CreateDevice(&dxgi))?;
            let dc = stage(
                "CreateDeviceContext",
                d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE),
            )?;

            let comp: IDCompositionDevice =
                stage("DCompositionCreateDevice", DCompositionCreateDevice(&dxgi))?;
            let comp_target = stage("CreateTargetForHwnd", comp.CreateTargetForHwnd(hwnd, true))?;
            let visual = stage("CreateVisual", comp.CreateVisual())?;
            let surface = stage(
                "CreateSurface",
                comp.CreateSurface(
                    width,
                    height,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_ALPHA_MODE_PREMULTIPLIED,
                ),
            )?;
            stage("SetContent", visual.SetContent(&surface))?;
            stage("SetRoot", comp_target.SetRoot(&visual))?;
            stage("Commit", comp.Commit())?;
            zlog!(
                "ui: 候選窗呈現建立 hwnd={:#x} surface={}x{} dpi={}",
                hwnd.0 as usize,
                width,
                height,
                dpi
            );

            let palette = palette(dark);
            let brush_background = stage(
                "CreateSolidColorBrush",
                dc.CreateSolidColorBrush(&palette.background, None),
            )?;
            let brush_border = stage(
                "CreateSolidColorBrush",
                dc.CreateSolidColorBrush(&palette.border, None),
            )?;
            let brush_text = stage(
                "CreateSolidColorBrush",
                dc.CreateSolidColorBrush(&palette.text, None),
            )?;
            let brush_dim = stage(
                "CreateSolidColorBrush",
                dc.CreateSolidColorBrush(&palette.dim, None),
            )?;
            let brush_highlight = stage(
                "CreateSolidColorBrush",
                dc.CreateSolidColorBrush(&palette.highlight, None),
            )?;
            let brush_highlight_text = stage(
                "CreateSolidColorBrush",
                dc.CreateSolidColorBrush(&palette.highlight_text, None),
            )?;
            Ok(Graphics {
                dc,
                surface,
                comp,
                visual,
                _comp_target: comp_target,
                brush_background,
                brush_border,
                brush_text,
                brush_dim,
                brush_highlight,
                brush_highlight_text,
                dark,
                size: (width, height),
                present_hr: 0,
                commit_hr: 0,
            })
        }
    }

    fn set_theme(&mut self, dark: bool) -> windows_core::Result<()> {
        if self.dark == dark {
            return Ok(());
        }
        let palette = palette(dark);
        self.brush_background =
            unsafe { self.dc.CreateSolidColorBrush(&palette.background, None)? };
        self.brush_border = unsafe { self.dc.CreateSolidColorBrush(&palette.border, None)? };
        self.brush_text = unsafe { self.dc.CreateSolidColorBrush(&palette.text, None)? };
        self.brush_dim = unsafe { self.dc.CreateSolidColorBrush(&palette.dim, None)? };
        self.brush_highlight = unsafe { self.dc.CreateSolidColorBrush(&palette.highlight, None)? };
        self.brush_highlight_text = unsafe {
            self.dc
                .CreateSolidColorBrush(&palette.highlight_text, None)?
        };
        self.dark = dark;
        Ok(())
    }

    /// surface 尺寸變了就重建，並重新綁到 visual。
    ///
    /// DComp 的 surface 尺寸固定；候選窗高度會隨候選數變，所以每次繪製前都要確認。
    fn resize(&mut self, width: u32, height: u32) -> windows_core::Result<()> {
        let (width, height) = (width.max(1), height.max(1));
        if self.size == (width, height) {
            return Ok(());
        }
        unsafe {
            let surface = stage(
                "CreateSurface",
                self.comp.CreateSurface(
                    width,
                    height,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_ALPHA_MODE_PREMULTIPLIED,
                ),
            )?;
            stage("SetContent", self.visual.SetContent(&surface))?;
            stage("Commit", self.comp.Commit())?;
            self.surface = surface;
            self.size = (width, height);
            zlog!("ui: 候選窗 surface 重建 surface={}x{}", width, height);
        }
        Ok(())
    }

    /// 畫一帧並送出。
    ///
    /// `BeginDraw` 回來的貼圖不保證從 (0,0) 開始，所有座標都要加上 `offset`。
    /// D2D 的 bitmap DPI 設成監視器 DPI，所以版面用 DIP 直接畫，offset 先換成 DIP。
    fn paint(&mut self, layout: &Layout, text: &Text, dpi: u32) -> windows_core::Result<()> {
        unsafe {
            let mut offset = POINT::default();
            let target: IDXGISurface = stage(
                "SurfaceBeginDraw",
                self.surface.BeginDraw(None, &mut offset),
            )?;
            let dpi = if dpi == 0 { BASE_DPI } else { dpi };
            let properties = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: dpi as f32,
                dpiY: dpi as f32,
                bitmapOptions: D2D1_BITMAP_OPTIONS(
                    D2D1_BITMAP_OPTIONS_TARGET.0 | D2D1_BITMAP_OPTIONS_CANNOT_DRAW.0,
                ),
                ..Default::default()
            };
            let bitmap: ID2D1Bitmap1 = match stage(
                "CreateBitmapFromDxgiSurface",
                self.dc
                    .CreateBitmapFromDxgiSurface(&target, Some(&properties)),
            ) {
                Ok(bitmap) => bitmap,
                Err(error) => {
                    // BeginDraw 配到的貼圖一定要 EndDraw 釋放，不然下一次會失敗。
                    let _ = self.surface.EndDraw();
                    return Err(error);
                }
            };
            self.dc.SetTarget(&bitmap);
            self.dc.BeginDraw();
            // 每帧清成透明：surface 會沿用上一帧的內容。
            self.dc.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));
            let ox = candidate_layout::dip_from_px(offset.x, dpi);
            let oy = candidate_layout::dip_from_px(offset.y, dpi);
            self.draw_content(layout, text, ox, oy);

            let drawn = self.dc.EndDraw(None, None);
            self.dc.SetTarget(None);
            let presented = self.surface.EndDraw();
            let committed = self.comp.Commit();
            self.present_hr = presented.as_ref().err().map_or(0, |e| e.code().0);
            self.commit_hr = committed.as_ref().err().map_or(0, |e| e.code().0);
            if let Err(error) = &presented {
                zlog!(
                    "ui: 候選窗 SurfaceEndDraw 失敗 hr={:#010x} {error}",
                    error.code().0 as u32
                );
            }
            if let Err(error) = &committed {
                zlog!(
                    "ui: 候選窗 Commit 失敗 hr={:#010x} {error}",
                    error.code().0 as u32
                );
            }
            // EndDraw 失敗（例如 D2DERR_RECREATE_TARGET）往上丟，由呼叫端丟掉整個
            // `Graphics`，下一次顯示重建。
            stage("EndDraw", drawn)?;
            stage("SurfaceEndDraw", presented)?;
            stage("Commit", committed)?;
            Ok(())
        }
    }

    /// 既有的繪製流程；所有座標加上 `offset`（DIP）。
    fn draw_content(&self, layout: &Layout, text: &Text, ox: f32, oy: f32) {
        unsafe {
            let full = D2D_RECT_F {
                left: ox,
                top: oy,
                right: ox + layout.width,
                bottom: oy + layout.height,
            };
            self.dc.FillRectangle(&full, &self.brush_background);
            let border = D2D_RECT_F {
                left: ox + 0.5,
                top: oy + 0.5,
                right: ox + (layout.width - 0.5).max(0.5),
                bottom: oy + (layout.height - 0.5).max(0.5),
            };
            self.dc
                .DrawRectangle(&border, &self.brush_border, 1.0, None);
            if let Some(rect) = layout
                .highlighted
                .and_then(|index| layout.row_highlight(index))
            {
                let rounded = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(rect, ox, oy),
                    radiusX: CORNER_RADIUS,
                    radiusY: CORNER_RADIUS,
                };
                self.dc
                    .FillRoundedRectangle(&rounded, &self.brush_highlight);
            }
            if let Some(title) = &layout.title {
                self.draw_text(
                    &title.text,
                    text.format(TextStyle::Small),
                    title.rect,
                    &self.brush_dim,
                    ox,
                    oy,
                );
            }
            for tab in &layout.tabs {
                let (format, brush) = if tab.selected {
                    let rounded = D2D1_ROUNDED_RECT {
                        rect: d2d_rect(tab.rect, ox, oy),
                        radiusX: CORNER_RADIUS,
                        radiusY: CORNER_RADIUS,
                    };
                    self.dc
                        .FillRoundedRectangle(&rounded, &self.brush_highlight);
                    (text.format(TextStyle::Small), &self.brush_highlight_text)
                } else {
                    (text.format(TextStyle::Small), &self.brush_dim)
                };
                self.draw_text(&tab.text, format, tab.rect, brush, ox, oy);
            }
            for line in &layout.header {
                self.draw_text(
                    &line.text,
                    text.format(TextStyle::Small),
                    line.rect,
                    &self.brush_dim,
                    ox,
                    oy,
                );
            }
            for (index, row) in layout.rows.iter().enumerate() {
                let highlighted = layout.highlighted == Some(index);
                let brush = if highlighted {
                    &self.brush_highlight_text
                } else {
                    &self.brush_text
                };
                let dim = if highlighted {
                    &self.brush_highlight_text
                } else {
                    &self.brush_dim
                };
                if let Some(key) = &row.key {
                    self.draw_text(
                        &key.text,
                        text.format(TextStyle::Key),
                        key.rect,
                        dim,
                        ox,
                        oy,
                    );
                }
                self.draw_text(
                    &row.text,
                    text.format(TextStyle::Candidate),
                    row.text_rect,
                    brush,
                    ox,
                    oy,
                );
                if let Some(rect) = row.note_rect {
                    self.draw_text(&row.note, text.format(TextStyle::Small), rect, dim, ox, oy);
                }
            }
            if let Some(pager) = &layout.pager {
                self.draw_text(
                    &pager.text,
                    &text.format_pager,
                    pager.rect,
                    &self.brush_dim,
                    ox,
                    oy,
                );
            }
        }
    }

    fn draw_text(
        &self,
        text: &str,
        format: &IDWriteTextFormat,
        rect: DipRect,
        brush: &ID2D1SolidColorBrush,
        ox: f32,
        oy: f32,
    ) {
        let utf16: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            self.dc.DrawText(
                &utf16,
                format,
                &d2d_rect(rect, ox, oy),
                brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }
}

fn d2d_rect(rect: DipRect, ox: f32, oy: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: ox + rect.left,
        top: oy + rect.top,
        right: ox + rect.right(),
        bottom: oy + rect.bottom(),
    }
}

fn text_format(
    write: &IDWriteFactory,
    style: TextStyle,
    alignment: windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT,
) -> windows_core::Result<IDWriteTextFormat> {
    unsafe {
        let format = write.CreateTextFormat(
            w!("Segoe UI"),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            style.size(),
            w!(""),
        )?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        format.SetTextAlignment(alignment)?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        Ok(format)
    }
}

/// 系統工作區（實體像素）；拿不到監視器資訊時退回整個螢幕。
fn work_area(caret: Rect) -> Rect {
    unsafe {
        let point = POINT {
            x: caret.left + caret.width() / 2,
            y: caret.top + caret.height() / 2,
        };
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            let work: RECT = info.rcWork;
            Rect::new(work.left, work.top, work.right, work.bottom)
        } else {
            Rect::new(
                0,
                0,
                GetSystemMetrics(SM_CXSCREEN),
                GetSystemMetrics(SM_CYSCREEN),
            )
        }
    }
}

/// `UiMessage` 轉成版面模組的內容；空字串編號＝不畫。
fn content(message: &UiMessage) -> Content {
    Content {
        candidates: message
            .candidates
            .iter()
            .map(|candidate| candidate_layout::Candidate {
                id: candidate.id,
                text: candidate.text.clone(),
                note: candidate.note.clone(),
                key: candidate.key.clone().filter(|key| !key.is_empty()),
            })
            .collect(),
        highlight: message.highlight,
        page: message.page,
        pages: message.pages,
        reading: message.reading.clone(),
        columns: message.columns,
        title: message.menu_title.clone(),
        tabs: message.menu_tabs.clone(),
        tab: message.menu_tab,
    }
}

/// 插入點矩形（螢幕像素）。高度為 0 時至少留 1 像素，才不會和候選窗重疊。
fn caret_rect(message: &UiMessage) -> Rect {
    Rect::new(
        message.x,
        message.y,
        message.x + message.w.max(1),
        message.y + message.h.max(1),
    )
}

/// 最近一次 `WM_PAINT` 的結果；只用在診斷 log。
#[derive(Clone, Copy)]
enum Paint {
    None,
    Ok { present_hr: i32, commit_hr: i32 },
    Failed(u32),
}

/// TSF DLL 內的候選窗本體。只有建立它的 TSF 執行緒會碰它。
pub struct LocalWindow {
    hwnd: HWND,
    instance: HINSTANCE,
    graphics: Option<Graphics>,
    /// DirectWrite 資源；建立呈現資源之前就要先量版面。
    text: Option<Text>,
    layout: Option<Layout>,
    /// 最近一次呈現的訊息：DPI 變化與滑鼠點選都要用。
    last: UiMessage,
    visible: bool,
    dark: bool,
    dpi: u32,
    /// 初始化失敗：本次 Activate 內不再嘗試，避免每個鍵都撞一次 GPU。
    failed: bool,
    paint: Paint,
    /// `SetWindowPos` 可能同步送來 `WM_DPICHANGED`；重入時直接忽略，避免遞迴。
    updating: bool,
}

impl LocalWindow {
    pub fn new() -> windows_core::Result<Box<LocalWindow>> {
        unsafe {
            let instance: HINSTANCE = GetModuleHandleW(None)?.into();
            // 類別是行程內的，不帶資源；重複註冊（多個 TSF 執行緒）由系統忽略錯誤。
            let _ = RegisterClassW(&WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW | CS_DROPSHADOW,
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: CLASS_NAME,
                ..Default::default()
            });
            let hwnd = CreateWindowExW(
                // 沒有轉向表面：畫面全部由 DirectComposition 提供，避免先閃一格空白。
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOREDIRECTIONBITMAP,
                CLASS_NAME,
                w!(""),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(instance),
                None,
            )?;
            let mut window = Box::new(LocalWindow {
                hwnd,
                instance,
                graphics: None,
                text: None,
                layout: None,
                last: UiMessage::default(),
                visible: false,
                dark: false,
                dpi: BASE_DPI,
                failed: false,
                paint: Paint::None,
                updating: false,
            });
            // Box 的內容不會再搬動，USERDATA 指向穩定的位址。
            set_userdata(hwnd, &mut *window as *mut LocalWindow as isize);
            Ok(window)
        }
    }

    fn dpi(&self) -> u32 {
        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        if dpi == 0 {
            BASE_DPI
        } else {
            dpi
        }
    }

    /// 顯示或更新候選窗。`Ok(())` 代表「處理完了，不論畫不畫得出來」；
    /// 只有建立視窗／GPU 資源失敗才回 `Err`，呼叫端據此停止重試。
    pub fn present(&mut self, message: &UiMessage) -> windows_core::Result<()> {
        if self.updating {
            return Ok(());
        }
        self.updating = true;
        let result = self.present_inner(message);
        self.updating = false;
        result
    }

    fn present_inner(&mut self, message: &UiMessage) -> windows_core::Result<()> {
        if self.failed {
            return Ok(());
        }
        if message.candidates.is_empty() {
            self.hide();
            return Ok(());
        }
        let became_visible = !self.visible;
        if became_visible {
            self.dark = theme_is_dark();
        }
        // 先算版面與尺寸，才決定 surface 要建多大。
        if self.text.is_none() {
            match Text::new() {
                Ok(text) => self.text = Some(text),
                Err(error) => {
                    self.failed = true;
                    zlog!(
                        "ui: 候選窗 DirectWrite 初始化失敗，改為不顯示（hr={:#010x} {error}）",
                        error.code().0 as u32
                    );
                    self.hide();
                    return Ok(());
                }
            }
        }
        self.last = message.clone();
        let content = content(message);
        let caret = caret_rect(message);
        let work = work_area(caret);
        let mut dpi = self.dpi();
        let (mut layout, mut size, mut position) =
            self.build(&content, dpi, caret, work, px_from_dip(GAP_DIP, dpi));

        // 先把視窗移到插入點所在的監視器，再問那個監視器的 DPI；換監視器時版面要重算。
        // 定位一定要在 ShowWindow 之前完成，免得先出現在舊位置。
        if !self.place(size, position) {
            self.hide();
            return Ok(());
        }
        let moved_dpi = self.dpi();
        if moved_dpi != dpi {
            dpi = moved_dpi;
            (layout, size, position) =
                self.build(&content, dpi, caret, work, px_from_dip(GAP_DIP, dpi));
            if !self.place(size, position) {
                self.hide();
                return Ok(());
            }
        }
        self.dpi = dpi;
        let count = layout.rows.len();
        let (layout_width, layout_height) = (layout.width, layout.height);

        // 呈現資源用最終像素尺寸建立；surface 尺寸固定，先建小尺寸沒有意義。
        let width = size.0.max(1) as u32;
        let height = size.1.max(1) as u32;
        if self.graphics.is_none() {
            match Graphics::new(self.hwnd, self.dark, width, height, dpi) {
                Ok(graphics) => self.graphics = Some(graphics),
                Err(error) => {
                    self.failed = true;
                    zlog!(
                        "ui: 候選窗 Direct2D 初始化失敗，改為不顯示（hr={:#010x} {error}）",
                        error.code().0 as u32
                    );
                    self.hide();
                    return Ok(());
                }
            }
        }

        let first_show = !self.visible;
        if first_show {
            unsafe { SetLastError(ERROR_SUCCESS) };
            let previously_visible = unsafe { ShowWindow(self.hwnd, SW_SHOWNOACTIVATE) };
            let last_error = unsafe { GetLastError() };
            if last_error.0 != 0 {
                zlog!(
                    "ui: 候選窗 ShowWindow 失敗 hwnd={:#x} prev_visible={} GetLastError={:#010x}",
                    self.hwnd.0 as usize,
                    previously_visible.as_bool(),
                    last_error.0
                );
            }
            self.visible = true;
        }

        // 內容換過了，要求重畫。`InvalidateRect` 只是排進佇列，TSF 按鍵處理是
        // 同步的、訊息迴圈沒在跑，所以還要 `UpdateWindow` 直接送 `WM_PAINT`。
        self.layout = Some(layout);
        self.paint = Paint::None;
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
            let _ = UpdateWindow(self.hwnd);
        }
        if let Paint::Failed(hr) = self.paint {
            zlog!("ui: 候選窗繪製失敗 hr={:#010x}，改為不顯示", hr);
            self.hide();
            return Ok(());
        }

        // 排查用：surface 尺寸、paint／present／commit 的結果都寫出來。
        let mut rect = RECT::default();
        let got_rect = unsafe { GetWindowRect(self.hwnd, &mut rect) }.is_ok();
        let surface = self
            .graphics
            .as_ref()
            .map(|graphics| format!("{}x{}", graphics.size.0, graphics.size.1))
            .unwrap_or_else(|| "n/a".to_owned());
        let (paint_label, present_label, commit_hr) = match self.paint {
            Paint::Ok {
                present_hr,
                commit_hr,
            } => (
                "ok".to_owned(),
                if present_hr == 0 {
                    "ok".to_owned()
                } else {
                    format!("{:#010x}", present_hr as u32)
                },
                commit_hr,
            ),
            Paint::Failed(hr) => (format!("fail {:#010x}", hr), "n/a".to_owned(), 0),
            Paint::None => ("none".to_owned(), "n/a".to_owned(), 0),
        };
        zlog!(
            "ui: 候選窗 hwnd={:#x} n={} at=({}, {}) layout={}x{} size={}x{} rect=({}, {}, {}, {}) visible={} surface={} dpi={} paint={} present={} commit={:#010x} style={:#010x} exstyle={:#010x}",
            self.hwnd.0 as usize,
            count,
            position.0,
            position.1,
            layout_width,
            layout_height,
            size.0,
            size.1,
            if got_rect { rect.left } else { 0 },
            if got_rect { rect.top } else { 0 },
            if got_rect { rect.right } else { 0 },
            if got_rect { rect.bottom } else { 0 },
            unsafe { IsWindowVisible(self.hwnd).as_bool() },
            surface,
            dpi,
            paint_label,
            present_label,
            commit_hr,
            unsafe { GetWindowLongPtrW(self.hwnd, GWL_STYLE) } as u32,
            unsafe { GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) } as u32
        );
        Ok(())
    }

    /// 定位並調整大小；失敗時記一行 `HRESULT`／`GetLastError` 並回 `false`。
    fn place(&self, size: (i32, i32), position: (i32, i32)) -> bool {
        unsafe { SetLastError(ERROR_SUCCESS) };
        let flags = if self.visible {
            SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW
        } else {
            SWP_NOACTIVATE | SWP_NOOWNERZORDER
        };
        let placed = unsafe {
            SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                position.0,
                position.1,
                size.0,
                size.1,
                flags,
            )
        };
        match placed {
            Ok(()) => true,
            Err(error) => {
                let last_error = unsafe { GetLastError() };
                zlog!(
                    "ui: 候選窗 SetWindowPos 失敗 at=({}, {}) size={}x{} hr={:#010x} GetLastError={:#010x}",
                    position.0,
                    position.1,
                    size.0,
                    size.1,
                    error.code().0 as u32,
                    last_error.0
                );
                false
            }
        }
    }

    fn build(
        &self,
        content: &Content,
        dpi: u32,
        caret: Rect,
        work: Rect,
        gap: i32,
    ) -> (Layout, (i32, i32), (i32, i32)) {
        let text = self.text.as_ref().expect("Text 已建立");
        let layout = candidate_layout::layout(content, |line, style| text.measure(line, style));
        let size = candidate_layout::fit_size(
            (
                px_from_dip(layout.width, dpi),
                px_from_dip(layout.height, dpi),
            ),
            work,
        );
        let position = candidate_layout::place(caret, size, work, gap);
        (layout, size, position)
    }

    pub fn hide(&mut self) {
        if self.visible {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.visible = false;
        }
        self.layout = None;
        self.paint = Paint::None;
    }

    /// `WM_PAINT`：`BeginPaint`／`EndPaint` 包住整帧繪製。
    ///
    /// 失敗就丟掉呈現資源（例如 `D2DERR_RECREATE_TARGET`），下一次顯示重建。
    fn paint(&mut self) {
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(self.hwnd, &mut ps);
            if hdc.is_invalid() {
                let error = GetLastError();
                zlog!("ui: 候選窗 BeginPaint 失敗 GetLastError={:#010x}", error.0);
                self.paint = Paint::Failed(error.0);
                return;
            }
            let result = self.paint_inner();
            let _ = EndPaint(self.hwnd, &ps);
            match result {
                Ok(()) => {
                    let (present_hr, commit_hr) = self
                        .graphics
                        .as_ref()
                        .map(|graphics| (graphics.present_hr, graphics.commit_hr))
                        .unwrap_or((0, 0));
                    self.paint = Paint::Ok {
                        present_hr,
                        commit_hr,
                    };
                }
                Err(error) => {
                    zlog!(
                        "ui: 候選窗繪製失敗 hr={:#010x} {error}，丟掉呈現資源下次重建",
                        error.code().0 as u32
                    );
                    self.paint = Paint::Failed(error.code().0 as u32);
                    self.graphics = None;
                    self.layout = None;
                }
            }
        }
    }

    fn paint_inner(&mut self) -> windows_core::Result<()> {
        let dark = self.dark;
        let dpi = self.dpi();
        let Some(text) = self.text.as_ref() else {
            return Ok(());
        };
        let Some(layout) = self.layout.as_ref() else {
            return Ok(());
        };
        let Some(graphics) = self.graphics.as_mut() else {
            return Ok(());
        };
        let mut rc = RECT::default();
        unsafe { GetClientRect(self.hwnd, &mut rc)? };
        let width = (rc.right - rc.left).max(1) as u32;
        let height = (rc.bottom - rc.top).max(1) as u32;
        graphics.resize(width, height)?;
        graphics.set_theme(dark)?;
        graphics.paint(layout, text, dpi)?;
        Ok(())
    }

    /// 滑鼠左鍵：換成 DIP 後命中候選，把 `WM_SELECT_CANDIDATE` 送回 TSF 的訊息視窗。
    /// 這裡不改任何組字狀態；核對焦點與選字都在原本的 edit session 裡。
    fn click(&mut self, lparam: LPARAM) {
        let x = (lparam.0 & 0xffff) as u16 as i16 as i32;
        let y = ((lparam.0 >> 16) & 0xffff) as u16 as i16 as i32;
        let (x, y) = (
            candidate_layout::dip_from_px(x, self.dpi),
            candidate_layout::dip_from_px(y, self.dpi),
        );
        let Some(id) = self
            .layout
            .as_ref()
            .and_then(|layout| candidate_layout::hit_test(layout, x, y))
        else {
            return;
        };
        let (target, revision) = (self.last.target, self.last.revision);
        if target != 0 {
            let target = HWND(target as usize as *mut c_void);
            unsafe {
                let _ = PostMessageW(
                    Some(target),
                    WM_SELECT_CANDIDATE,
                    WPARAM(revision as usize),
                    LPARAM(id as isize),
                );
            }
        }
        // 選字結果回來前先收起，避免點在同一個位置兩次。
        self.hide();
    }

    /// DPI 或監視器變了：用同一份訊息重算版面。
    fn relayout(&mut self) {
        if !self.visible {
            return;
        }
        let message = self.last.clone();
        if message.candidates.is_empty() {
            return;
        }
        if let Err(error) = self.present(&message) {
            zlog!("ui: 候選窗重排失敗（{error}）");
        }
    }
}

impl Drop for LocalWindow {
    fn drop(&mut self) {
        // 先放掉呈現資源（它持有這個 hwnd 的合成目標），再銷毀視窗。
        self.graphics = None;
        set_userdata(self.hwnd, 0);
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            // 同一行程還有多個 service 時會失敗，留給最後一個離開才移除。
            let _ = UnregisterClassW(CLASS_NAME, Some(self.instance));
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let window = || unsafe {
        let pointer = userdata(hwnd) as *mut LocalWindow;
        pointer.as_mut()
    };
    match message {
        // 點候選窗不啟動、不搶焦點；TSF 的焦點文件因此不會被換掉。
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_LBUTTONDOWN => {
            if let Some(window) = window() {
                window.click(lparam);
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            if let Some(window) = window() {
                window.relayout();
            }
            LRESULT(0)
        }
        // 背景由 D2D 自己畫，讓系統擦一次是白費工還會閃。
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            if let Some(window) = window() {
                window.paint();
            }
            LRESULT(0)
        }
        // 這是宿主的訊息迴圈：拒絕 DefWindowProc 對 WM_DESTROY 的預設處理，
        // 免得輔助視窗被銷毀時影響宿主的 WM_QUIT 或視窗鏈。
        WM_DESTROY | WM_NCDESTROY => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
