//! 文字服務 DLL 的目標架構，以及從 PE 標頭偵測架構。
//!
//! `jc-tip-register` 是 x64 程式，`LoadLibraryW` 得了 x64 的 `jc-tsf.dll`，
//! 但**載入不了** 32 位元那一份（反之亦然：作業系統直接回
//! `ERROR_BAD_EXE_FORMAT`）。所以註冊與移除都要先知道目標 DLL 是哪個架構：
//!
//! - 同架構：交給 DLL 自己的 `DllRegisterServer`／`DllUnregisterServer`，
//!   註冊邏輯只有一份。
//! - 跨架構：由這支程式直接寫對應檢視的 `InprocServer32`，再呼叫 TSF 介面，
//!   內容與 DLL 內的同名函式相同（`crates/jc-tsf/src/register.rs`）。
//!
//! 架構也可以用 `--arch x64|x86` 指定；檔案不存在或被隔離時仍能清機碼。

use std::fmt;
use std::path::Path;

/// 文字服務 DLL 的目標架構。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    /// `x86_64-pc-windows-msvc`，註冊在 64 位元機碼檢視。
    X64,
    /// `i686-pc-windows-msvc`，註冊在 `Wow6432Node`。
    X86,
}

impl Arch {
    /// 兩個架構，安裝與移除時固定用這個順序（x64 先）。
    pub const ALL: [Arch; 2] = [Arch::X64, Arch::X86];

    /// 命令列與日誌用的名稱：`x64`／`x86`。
    pub fn as_str(self) -> &'static str {
        match self {
            Arch::X64 => "x64",
            Arch::X86 => "x86",
        }
    }

    /// 這個架構是不是目前行程的位元數。
    ///
    /// 只有相同位元數時才載得進 DLL；跨架構一律走直接寫機碼的路徑。
    pub fn is_current_process(self) -> bool {
        match self {
            Arch::X64 => std::mem::size_of::<usize>() == 8,
            Arch::X86 => std::mem::size_of::<usize>() == 4,
        }
    }

    /// 從檔案內容讀出 PE 標頭裡的架構。
    ///
    /// 只認 x86 與 x64；ARM64 等其他架構回 `None`，由呼叫端回報成錯誤。
    pub fn from_pe(image: &[u8]) -> Option<Self> {
        // DOS 標頭：`MZ`，0x3C 起 4 bytes 是 PE 標頭的位移。
        if image.len() < 0x40 || &image[0..2] != b"MZ" {
            return None;
        }
        let offset = u32::from_le_bytes(image[0x3C..0x40].try_into().ok()?) as usize;
        // PE 簽章 4 bytes，接著 IMAGE_FILE_HEADER 的前 2 bytes 是 Machine。
        let header = image.get(offset..offset.checked_add(6)?)?;
        if &header[0..4] != b"PE\0\0" {
            return None;
        }
        match u16::from_le_bytes([header[4], header[5]]) {
            0x014c => Some(Arch::X86),
            0x8664 => Some(Arch::X64),
            _ => None,
        }
    }

    /// 讀檔案判斷架構。
    pub fn detect(path: &Path) -> Result<Self, String> {
        let image =
            std::fs::read(path).map_err(|error| format!("讀不到 {}：{error}", path.display()))?;
        Self::from_pe(&image).ok_or_else(|| {
            format!(
                "{} 不是 x86 或 x64 的 Windows DLL；請用 --arch 指定架構",
                path.display()
            )
        })
    }
}

impl fmt::Display for Arch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// 解析 `--arch` 的值。
pub fn parse_arch(value: &str) -> Result<Arch, String> {
    match value {
        "x64" => Ok(Arch::X64),
        "x86" => Ok(Arch::X86),
        other => Err(format!("--arch 只接受 x64 或 x86，收到 {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 組一個最小的 PE 影像：`MZ`、0x3C 指向 PE 標頭、Machine 欄位。
    fn pe(machine: u16, declared_offset: usize) -> Vec<u8> {
        let mut image = vec![0u8; declared_offset + 6];
        image[0..2].copy_from_slice(b"MZ");
        image[0x3C..0x40].copy_from_slice(&(declared_offset as u32).to_le_bytes());
        image[declared_offset..declared_offset + 4].copy_from_slice(b"PE\0\0");
        image[declared_offset + 4..declared_offset + 6].copy_from_slice(&machine.to_le_bytes());
        image
    }

    #[test]
    fn detects_x64_and_x86_machine_types() {
        assert_eq!(Arch::from_pe(&pe(0x8664, 0x80)), Some(Arch::X64));
        assert_eq!(Arch::from_pe(&pe(0x014c, 0x80)), Some(Arch::X86));
    }

    #[test]
    fn rejects_other_machine_types() {
        // ARM64：本產品沒有這個架構，不能猜成 x64。
        assert_eq!(Arch::from_pe(&pe(0xAA64, 0x80)), None);
    }

    #[test]
    fn rejects_non_pe_images() {
        assert_eq!(Arch::from_pe(b""), None);
        assert_eq!(Arch::from_pe(b"MZ"), None);
        assert_eq!(Arch::from_pe(&[0u8; 64]), None);

        let mut image = pe(0x8664, 0x80);
        // 位移超出檔案長度。
        image[0x3C..0x40].copy_from_slice(&0xFFFF_u32.to_le_bytes());
        assert_eq!(Arch::from_pe(&image), None);

        let mut image = pe(0x8664, 0x80);
        image[0x80..0x84].copy_from_slice(b"NE\0\0");
        assert_eq!(Arch::from_pe(&image), None);
    }

    #[test]
    fn arch_names_and_parsing() {
        assert_eq!(parse_arch("x64"), Ok(Arch::X64));
        assert_eq!(parse_arch("x86"), Ok(Arch::X86));
        assert!(parse_arch("arm64").is_err());
        assert_eq!(Arch::X86.to_string(), "x86");
        assert_eq!(Arch::ALL.len(), 2);
        assert_eq!(Arch::ALL[0], Arch::X64);
    }
}
