//! 候選窗跟隨應用程式主題；高對比模式優先使用使用者指定的系統色。移植自 0c。
use windows::Win32::Graphics::Gdi::{
    GetSysColor, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT,
};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};
use windows_core::w;

/// COLORREF 的 BGR 色值。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub background: u32,
    pub text: u32,
    pub hint: u32,
    pub border: u32,
    pub selection: u32,
    pub selected_text: u32,
    /// 一次性提示的字色：一般主題用強調色，高對比用使用者的文字色。
    pub notice: u32,
    pub color_fonts: bool,
}

impl Palette {
    /// 標準明暗色盤。
    pub fn standard(dark: bool) -> Self {
        if dark {
            Self {
                background: 0x202020,
                text: 0xF2F2F2,
                hint: 0xB3B3B3,
                border: 0x707070,
                selection: 0xFFCF9B,
                selected_text: 0x181818,
                notice: 0xFFCF9B,
                color_fonts: true,
            }
        } else {
            Self {
                background: 0xFFFFFF,
                text: 0x202020,
                hint: 0x606060,
                border: 0xB0B0B0,
                selection: 0xB86600,
                selected_text: 0xFFFFFF,
                notice: 0xB86600,
                color_fonts: true,
            }
        }
    }

    /// 高對比模式：全部用使用者選的系統色。
    fn contrast(background: u32, text: u32, selection: u32, selected_text: u32) -> Self {
        Self {
            background,
            text,
            hint: text,
            border: text,
            selection,
            selected_text,
            notice: text,
            color_fonts: false,
        }
    }
}

/// 讀取系統高對比與明暗設定，回傳對應色盤。
pub fn system_palette() -> Palette {
    unsafe {
        let mut hc = HIGHCONTRASTW {
            cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
            ..Default::default()
        };
        if SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            hc.cbSize,
            Some((&mut hc as *mut HIGHCONTRASTW).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
            && hc.dwFlags.contains(HCF_HIGHCONTRASTON)
        {
            return Palette::contrast(
                GetSysColor(COLOR_WINDOW),
                GetSysColor(COLOR_WINDOWTEXT),
                GetSysColor(COLOR_HIGHLIGHT),
                GetSysColor(COLOR_HIGHLIGHTTEXT),
            );
        }
        let mut light = 1u32;
        let mut size = std::mem::size_of_val(&light) as u32;
        let read = RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut light as *mut u32).cast()),
            Some(&mut size),
        );
        Palette::standard(read.is_ok() && light == 0)
    }
}
