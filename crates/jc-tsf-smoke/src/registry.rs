//! 註冊表探測（檢查 7）。
//!
//! 只讀 `HKLM\SOFTWARE\Microsoft\CTF\TIP` 底下有沒有本產品的 CLSID。讀取不需要
//! 管理員；沒有這個機碼代表輸入法還沒安裝，屬於正常情況，只回報、不判定失敗。

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
};

use crate::error::SmokeError;
use crate::report;

/// TSF 的 TIP 清單機碼。
pub const TIP_KEY: &str = r"SOFTWARE\Microsoft\CTF\TIP";

/// 探測結果。
pub struct Probe {
    /// `CTF\TIP` 底下總共有幾個子機碼。
    pub total: usize,
    /// 其中是否有本產品的 CLSID。
    pub found: bool,
}

/// 列出 `HKLM\SOFTWARE\Microsoft\CTF\TIP` 的子機碼，找本產品的 CLSID。
///
/// 機碼不存在或讀不到都回 [`SmokeError::Registry`]；呼叫端一律當 `[SKIP]`。
pub fn probe(clsid_text: &str) -> Result<Probe, SmokeError> {
    let path = wide(TIP_KEY);
    let mut key = HKEY::default();
    // SAFETY: path 以 NUL 結尾；key 在成功時由 RegOpenKeyExW 寫入。
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(path.as_ptr()),
            None,
            KEY_READ,
            &mut key,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(SmokeError::Registry(format!(
            r"RegOpenKeyExW(HKLM\{TIP_KEY}) = {}",
            report::hex(status.0)
        )));
    }

    let mut total = 0usize;
    let mut found = false;
    loop {
        let mut name = [0u16; 128];
        let mut length = name.len() as u32;
        // SAFETY: name 是長度正確的可寫緩衝；length 一併傳入長度。
        let status = unsafe {
            RegEnumKeyExW(
                key,
                total as u32,
                Some(PWSTR(name.as_mut_ptr())),
                &mut length,
                None,
                None,
                None,
                None,
            )
        };
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        if status != ERROR_SUCCESS {
            // SAFETY: key 由上面成功的 RegOpenKeyExW 取得。
            unsafe {
                let _ = RegCloseKey(key);
            }
            return Err(SmokeError::Registry(format!(
                "RegEnumKeyExW 第 {total} 項 = {}",
                report::hex(status.0)
            )));
        }
        total += 1;
        let text = String::from_utf16_lossy(&name[..length as usize]);
        if text.eq_ignore_ascii_case(clsid_text) {
            found = true;
        }
    }

    // SAFETY: key 由成功的 RegOpenKeyExW 取得，這裡是唯一放手的地方。
    unsafe {
        let _ = RegCloseKey(key);
    }
    Ok(Probe { total, found })
}

/// 把字串轉成以 NUL 結尾的 UTF-16。
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tip_key_path_is_stable() {
        assert_eq!(TIP_KEY, r"SOFTWARE\Microsoft\CTF\TIP");
    }

    #[test]
    fn clsid_comparison_ignores_case() {
        let expected = "{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}";
        assert!(expected.eq_ignore_ascii_case("{7b7d6e1a-3c1f-4b2e-9a55-6c1d2f3e4a01}"));
    }
}
