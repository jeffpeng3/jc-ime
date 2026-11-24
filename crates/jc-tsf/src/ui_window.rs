//! TSF DLL 內的候選窗：HWND + Direct2D + DirectComposition。
//!
//! 這個視窗不建立執行緒、不開新程序，也不碰 `jc-ui`。它只活在 TSF 的 UI 執行緒上：
//! `present` 在按鍵路徑同步更新內容，滑鼠點選則把 `WM_SELECT_CANDIDATE` 送回
//! `candidate::CandidateWindow` 的訊息視窗，由原本的 edit session 核對焦點後才選字。
//!
//! 幾個硬性約束：
//! - `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`：點視窗不會搶焦點，也不會出現在工作列。
//! - 一律 `SW_SHOWNOACTIVATE`／`SWP_NOACTIVATE`，絕不呼叫 `SetFocus`。
//! - 繪製用 D3D11 → DXGI swap chain（for composition）→ Direct2D device context →
//!   DirectComposition visual。`Present(0, …)` 不等垂直同步。
//! - 初始化失敗只記一行日誌並收起，不在輸入路徑重試；下一次 Activate 再說。

use std::ffi::c_void;

use windows::Win32::Foundation::{
    GetLastError, SetLastError, ERROR_SUCCESS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT,
    WPARAM,
};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateDevice, ID2D1Bitmap1, ID2D1DeviceContext, ID2D1Image, ID2D1RenderTarget,
    ID2D1SolidColorBrush, D2D1_BITMAP_OPTIONS, D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
    D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1,
    D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionVisual,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS,
    DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIAdapter, IDXGIDevice, IDXGIFactory2, IDXGISurface, IDXGISwapChain, IDXGISwapChain1,
    DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, ValidateRect, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics, GetWindowLongPtrW,
    GetWindowRect, IsWindowVisible, PostMessageW, RegisterClassW, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, UnregisterClassW, GWLP_USERDATA, HWND_TOPMOST, MA_NOACTIVATE, SM_CXSCREEN,
    SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SW_HIDE, SW_SHOWNOACTIVATE, WM_DESTROY,
    WM_DPICHANGED, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_MOUSEACTIVATE, WM_NCDESTROY, WM_PAINT,
    WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
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
/// `DXGI_STATUS_OCCLUDED`：Present 成功但 DWM 目前沒合成這個視窗。
/// windows-rs 沒有匯出這個 status 常數，值取自 `dxgi.h`。
const DXGI_STATUS_OCCLUDED: i32 = 0x087A_0001u32 as i32;

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

/// 候選窗的 GPU 資源：swap chain、D2D、DirectWrite、DirectComposition。
///
/// 建立後就一直留到 `LocalWindow` 被銷毀（Deactivate）；`resize`／`draw` 失敗時
/// 由呼叫端把整個 `Graphics` 丟掉，下一個按鍵再重建。
struct Graphics {
    /// 只為了持有參照：visual 的內容是這條 swap chain，這裡不能讓它被釋放。
    swap: IDXGISwapChain1,
    swap_base: IDXGISwapChain,
    d2d: ID2D1DeviceContext,
    rt: ID2D1RenderTarget,
    /// 同上：visual／target 由 `comp` 管理，參照要留到視窗銷毀。
    comp: IDCompositionDevice,
    write: IDWriteFactory,
    format_text: IDWriteTextFormat,
    format_key: IDWriteTextFormat,
    format_small: IDWriteTextFormat,
    format_pager: IDWriteTextFormat,
    brush_background: ID2D1SolidColorBrush,
    brush_border: ID2D1SolidColorBrush,
    brush_text: ID2D1SolidColorBrush,
    brush_dim: ID2D1SolidColorBrush,
    brush_highlight: ID2D1SolidColorBrush,
    brush_highlight_text: ID2D1SolidColorBrush,
    target: Option<ID2D1Bitmap1>,
    dark: bool,
    width: u32,
    height: u32,
}

impl Graphics {
    fn new(hwnd: HWND, dark: bool) -> windows_core::Result<Graphics> {
        unsafe {
            let device = create_device()?;
            let dxgi: IDXGIDevice = device.cast()?;
            let adapter: IDXGIAdapter = dxgi.GetAdapter()?;
            let factory: IDXGIFactory2 = adapter.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: 1,
                Height: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                Flags: 0,
            };
            let swap = factory.CreateSwapChainForComposition(&device, &desc, None)?;
            let swap_base: IDXGISwapChain = swap.cast()?;
            let d2d_device = D2D1CreateDevice(&dxgi, None)?;
            let d2d = d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let rt: ID2D1RenderTarget = d2d.cast()?;

            let comp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let target = comp.CreateTargetForHwnd(hwnd, true)?;
            let visual: IDCompositionVisual = comp.CreateVisual()?;
            visual.SetContent(&swap)?;
            target.SetRoot(&visual)?;
            comp.Commit()?;

            let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let format_text =
                text_format(&write, TextStyle::Candidate, DWRITE_TEXT_ALIGNMENT_LEADING)?;
            let format_key = text_format(&write, TextStyle::Key, DWRITE_TEXT_ALIGNMENT_LEADING)?;
            let format_small =
                text_format(&write, TextStyle::Small, DWRITE_TEXT_ALIGNMENT_LEADING)?;
            let format_pager =
                text_format(&write, TextStyle::Small, DWRITE_TEXT_ALIGNMENT_TRAILING)?;

            let palette = palette(dark);
            let brush_background = rt.CreateSolidColorBrush(&palette.background, None)?;
            let brush_border = rt.CreateSolidColorBrush(&palette.border, None)?;
            let brush_text = rt.CreateSolidColorBrush(&palette.text, None)?;
            let brush_dim = rt.CreateSolidColorBrush(&palette.dim, None)?;
            let brush_highlight = rt.CreateSolidColorBrush(&palette.highlight, None)?;
            let brush_highlight_text = rt.CreateSolidColorBrush(&palette.highlight_text, None)?;
            Ok(Graphics {
                swap,
                swap_base,
                d2d,
                rt,
                comp,
                write,
                format_text,
                format_key,
                format_small,
                format_pager,
                brush_background,
                brush_border,
                brush_text,
                brush_dim,
                brush_highlight,
                brush_highlight_text,
                target: None,
                dark,
                width: 0,
                height: 0,
            })
        }
    }

    fn set_theme(&mut self, dark: bool) -> windows_core::Result<()> {
        if self.dark == dark {
            return Ok(());
        }
        let palette = palette(dark);
        self.brush_background =
            unsafe { self.rt.CreateSolidColorBrush(&palette.background, None)? };
        self.brush_border = unsafe { self.rt.CreateSolidColorBrush(&palette.border, None)? };
        self.brush_text = unsafe { self.rt.CreateSolidColorBrush(&palette.text, None)? };
        self.brush_dim = unsafe { self.rt.CreateSolidColorBrush(&palette.dim, None)? };
        self.brush_highlight = unsafe { self.rt.CreateSolidColorBrush(&palette.highlight, None)? };
        self.brush_highlight_text = unsafe {
            self.rt
                .CreateSolidColorBrush(&palette.highlight_text, None)?
        };
        self.dark = dark;
        Ok(())
    }

    /// 文字寬度（DIP）。DirectWrite 的量測與繪製用同一個 `IDWriteTextFormat`。
    fn measure(&self, text: &str, style: TextStyle) -> f32 {
        let format = match style {
            TextStyle::Candidate => &self.format_text,
            TextStyle::Key => &self.format_key,
            TextStyle::Small => &self.format_small,
        };
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

    /// 把目前的 visual 樹送給 DWM。顯示順序改變後要再送一次，不能吞掉錯誤。
    fn commit(&self) -> windows_core::Result<()> {
        unsafe { self.comp.Commit() }
    }

    /// swap chain 目前的緩衝尺寸；診斷用，取不到時回 `None`。
    fn swap_size(&self) -> Option<(u32, u32)> {
        let desc = unsafe { self.swap.GetDesc1() }.ok()?;
        Some((desc.Width, desc.Height))
    }

    /// 把 swap chain 對齊視窗大小；bitmap 的 DPI 設成監視器 DPI，之後畫圖都用 DIP。
    fn resize(&mut self, width: u32, height: u32, dpi: u32) -> windows_core::Result<()> {
        let width = width.max(1);
        let height = height.max(1);
        if self.target.take().is_some() {
            unsafe { self.d2d.SetTarget(None) };
        }
        if self.width != width || self.height != height {
            unsafe {
                self.swap_base.ResizeBuffers(
                    2,
                    width,
                    height,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_SWAP_CHAIN_FLAG(0),
                )?;
            }
            self.width = width;
            self.height = height;
        }
        let surface: IDXGISurface = unsafe { self.swap_base.GetBuffer(0)? };
        let dpi = if dpi == 0 { BASE_DPI } else { dpi };
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: dpi as f32,
            dpiY: dpi as f32,
            bitmapOptions: D2D1_BITMAP_OPTIONS(
                D2D1_BITMAP_OPTIONS_TARGET.0 | D2D1_BITMAP_OPTIONS_CANNOT_DRAW.0,
            ),
            ..Default::default()
        };
        let bitmap = unsafe {
            self.d2d
                .CreateBitmapFromDxgiSurface(&surface, Some(&properties))?
        };
        let image: ID2D1Image = bitmap.cast()?;
        unsafe { self.d2d.SetTarget(&image) };
        self.target = Some(bitmap);
        Ok(())
    }

    /// 畫完後送出一帧，回傳 `Present` 的原始 `HRESULT`。
    ///
    /// `Present` 的成功碼不只 `S_OK`：`DXGI_STATUS_OCCLUDED` 也是成功，代表 DWM
    /// 現在沒合成這個視窗；呼叫端要記錄下來，下一個按鍵再重畫重送。
    fn draw(&mut self, layout: &Layout) -> windows_core::Result<windows_core::HRESULT> {
        unsafe {
            self.d2d.BeginDraw();
            let full = D2D_RECT_F {
                left: 0.0,
                top: 0.0,
                right: layout.width,
                bottom: layout.height,
            };
            self.rt.FillRectangle(&full, &self.brush_background);
            let border = D2D_RECT_F {
                left: 0.5,
                top: 0.5,
                right: (layout.width - 0.5).max(0.5),
                bottom: (layout.height - 0.5).max(0.5),
            };
            self.rt
                .DrawRectangle(&border, &self.brush_border, 1.0, None);
            if let Some(rect) = layout
                .highlighted
                .and_then(|index| layout.row_highlight(index))
            {
                let rounded = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(rect),
                    radiusX: CORNER_RADIUS,
                    radiusY: CORNER_RADIUS,
                };
                self.rt
                    .FillRoundedRectangle(&rounded, &self.brush_highlight);
            }
            for line in &layout.header {
                self.draw_text(&line.text, &self.format_small, line.rect, &self.brush_dim);
            }
            for (index, row) in layout.rows.iter().enumerate() {
                let highlighted = layout.highlighted == Some(index);
                let text = if highlighted {
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
                    self.draw_text(&key.text, &self.format_key, key.rect, dim);
                }
                self.draw_text(&row.text, &self.format_text, row.text_rect, text);
                if let Some(rect) = row.note_rect {
                    self.draw_text(&row.note, &self.format_small, rect, dim);
                }
            }
            if let Some(pager) = &layout.pager {
                self.draw_text(&pager.text, &self.format_pager, pager.rect, &self.brush_dim);
            }
            self.d2d.EndDraw(None, None)?;
        }
        // 不等垂直同步：候選窗晚一帧上屏沒有關係，按鍵路徑不能卡住。
        Ok(unsafe { self.swap_base.Present(0, DXGI_PRESENT(0)) })
    }

    fn draw_text(
        &self,
        text: &str,
        format: &IDWriteTextFormat,
        rect: DipRect,
        brush: &ID2D1SolidColorBrush,
    ) {
        let utf16: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            self.rt.DrawText(
                &utf16,
                format,
                &d2d_rect(rect),
                brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }
}

fn d2d_rect(rect: DipRect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: rect.left,
        top: rect.top,
        right: rect.right(),
        bottom: rect.bottom(),
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

/// TSF DLL 內的候選窗本體。只有建立它的 TSF 執行緒會碰它。
pub struct LocalWindow {
    hwnd: HWND,
    instance: HINSTANCE,
    graphics: Option<Graphics>,
    layout: Option<Layout>,
    /// 最近一次呈現的訊息：DPI 變化與滑鼠點選都要用。
    last: UiMessage,
    visible: bool,
    dark: bool,
    dpi: u32,
    /// 初始化失敗：本次 Activate 內不再嘗試，避免每個鍵都撞一次 GPU。
    failed: bool,
    /// 上一次 `Present` 回 `DXGI_STATUS_OCCLUDED`：下一個按鍵要照常重畫重送。
    occluded: bool,
    /// `SetWindowPos` 可能同步送來 `WM_DPICHANGED`；重入時直接忽略，避免遞迴。
    updating: bool,
}

impl LocalWindow {
    pub fn new() -> windows_core::Result<Box<LocalWindow>> {
        unsafe {
            let instance: HINSTANCE = GetModuleHandleW(None)?.into();
            // 類別是行程內的，不帶資源；重複註冊（多個 TSF 執行緒）由系統忽略錯誤。
            let _ = RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
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
                layout: None,
                last: UiMessage::default(),
                visible: false,
                dark: false,
                dpi: BASE_DPI,
                failed: false,
                occluded: false,
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
        if self.graphics.is_none() {
            match Graphics::new(self.hwnd, self.dark) {
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

        // 先顯示、再送第一帧。`WS_EX_NOREDIRECTIONBITMAP` 的視窗沒有 GDI 轉向表面，
        // 畫面只來自 DirectComposition；flip-model 的 composition swap chain 在視窗
        // 還藏著時送出的第一個 `Present` 不會被 DWM 合成，之後才 ShowWindow 就只剩
        // 空白。定位完成後立刻顯示，第一帧才會真的上屏。
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
            // 建立 Graphics 時視窗還藏著，那次 Commit 可能沒被 DWM 採用；顯示後補一次。
            if let Some(graphics) = self.graphics.as_ref() {
                if let Err(error) = graphics.commit() {
                    zlog!(
                        "ui: 候選窗 Commit 失敗（顯示後）hr={:#010x}",
                        error.code().0 as u32
                    );
                }
            }
        }

        let dark = self.dark;
        let frame = {
            let graphics = self.graphics.as_mut().expect("Graphics 已建立");
            graphics
                .resize(size.0.max(1) as u32, size.1.max(1) as u32, dpi)
                .and_then(|()| graphics.set_theme(dark))
                .and_then(|()| graphics.draw(&layout))
        };
        let present = match frame {
            Ok(present) => present,
            Err(error) => {
                // 裝置遺失之類的執行期錯誤：收起、丟掉資源，下一個按鍵重建。
                zlog!(
                    "ui: 候選窗繪製失敗，改為不顯示（hr={:#010x} {error}）",
                    error.code().0 as u32
                );
                self.graphics = None;
                self.layout = None;
                self.occluded = false;
                self.hide();
                return Ok(());
            }
        };
        // `DXGI_STATUS_OCCLUDED` 是成功碼，但畫面沒有上屏。記下狀態，下一個按鍵照常
        // 走完整流程重畫重送；不另外排程、不重試、不等待。
        let occluded = present.0 == DXGI_STATUS_OCCLUDED;
        let was_occluded = std::mem::replace(&mut self.occluded, occluded);
        if present.is_err() {
            zlog!(
                "ui: 候選窗 Present 失敗 hr={:#010x}，改為不顯示",
                present.0 as u32
            );
            self.graphics = None;
            self.layout = None;
            self.occluded = false;
            self.hide();
            return Ok(());
        }
        if occluded {
            zlog!(
                "ui: 候選窗 Present 被遮蔽（DXGI_STATUS_OCCLUDED）occluded_prev={was_occluded}，下一個按鍵重送"
            );
        }
        if first_show {
            // 排查用：0 寬或 visible=false 就代表問題在版面尺寸／視窗狀態，
            // 不是「沒有呼叫到這裡」。只在第一次顯示時寫一行。
            let mut rect = RECT::default();
            let got_rect = unsafe { GetWindowRect(self.hwnd, &mut rect) }.is_ok();
            let mut cloaked: u32 = 0;
            let cloaked_result = unsafe {
                DwmGetWindowAttribute(
                    self.hwnd,
                    DWMWA_CLOAKED,
                    &mut cloaked as *mut u32 as *mut c_void,
                    std::mem::size_of::<u32>() as u32,
                )
            };
            let (swap_width, swap_height) = self
                .graphics
                .as_ref()
                .and_then(|graphics| graphics.swap_size())
                .unwrap_or((0, 0));
            zlog!(
                "ui: 候選窗 hwnd={:#x} layout={}x{} size={}x{} at=({}, {}) visible={} rect=({}, {}, {}, {}) cloaked={:#x} cloaked_hr={:#010x} swap={}x{} present={:#010x}",
                self.hwnd.0 as usize,
                layout.width,
                layout.height,
                size.0,
                size.1,
                position.0,
                position.1,
                unsafe { IsWindowVisible(self.hwnd).as_bool() },
                if got_rect { rect.left } else { 0 },
                if got_rect { rect.top } else { 0 },
                if got_rect { rect.right } else { 0 },
                if got_rect { rect.bottom } else { 0 },
                cloaked,
                cloaked_result
                    .map(|()| 0u32)
                    .unwrap_or_else(|error| error.code().0 as u32),
                swap_width,
                swap_height,
                present.0 as u32
            );
        }
        self.layout = Some(layout);
        Ok(())
    }

    /// 定位並調整大小；失敗時記一行 `HRESULT`／`GetLastError` 並回 `false`。
    fn place(&self, size: (i32, i32), position: (i32, i32)) -> bool {
        unsafe { SetLastError(ERROR_SUCCESS) };
        let placed = unsafe {
            SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                position.0,
                position.1,
                size.0,
                size.1,
                SWP_NOACTIVATE | SWP_NOOWNERZORDER,
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
        let graphics = self.graphics.as_ref().expect("Graphics 已建立");
        let layout = candidate_layout::layout(content, |text, style| graphics.measure(text, style));
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
        // 先放掉 swap chain／D2D／DComp，再銷毀視窗。
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
        // 內容由 DirectComposition 上屏，視窗本身不需要擦底色或畫任何東西。
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            }
            LRESULT(0)
        }
        // 這是宿主的訊息迴圈：拒絕 DefWindowProc 對 WM_DESTROY 的預設處理，
        // 免得輔助視窗被銷毀時影響宿主的 WM_QUIT 或視窗鏈。
        WM_DESTROY | WM_NCDESTROY => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
