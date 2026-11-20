//! 引擎 pipe 名稱的解析規則。
//!
//! 三個使用端共用同一套規則：`jc-tsf`（client）、`jc-engine`（server）與
//! `jc-windows` 自己。優先順序固定為：
//!
//! 1. 環境變數 [`PIPE_ENV`]（除錯與非 Windows 平台用）；
//! 2. `HKCU\Software\jc-ime\EnginePipe`（Windows）；
//! 3. 由目前使用者的 SID 尾段推導的 `jc-ime-<rid>`（Windows）；
//! 4. 取不到 SID 時退回 [`BASE_PIPE_NAME`]，並在 [`ResolvedPipeName::warning`]
//!    回報一次警告。
//!
//! 名稱含使用者識別，避免同一台機器上兩個使用者同時登入時連到同一個引擎。
//!
//! 非 Windows 平台沒有 SID，固定用 [`BASE_PIPE_NAME`] 或 [`PIPE_ENV`]，不會
//! 產生警告。

/// 沒有覆寫、也取不到 SID 時使用的共用 pipe 名稱。
pub const BASE_PIPE_NAME: &str = "jc-ime";

/// 覆寫 pipe 名稱的環境變數；值為空字串時視同未設定。
pub const PIPE_ENV: &str = "JC_IME_PIPE";

/// 註冊表裡指定引擎 pipe 名稱的位置。
#[cfg(windows)]
const REGISTRY_KEY: &str = r"Software\jc-ime";

/// 註冊表值的名稱。
#[cfg(windows)]
const REGISTRY_VALUE: &str = "EnginePipe";

/// pipe 名稱的解析結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPipeName {
    /// pipe 名稱，不含 `\\.\pipe\` 前綴。
    pub name: String,
    /// 需要記錄的警告；只有取不到 SID 而退回共用名稱時才有值。
    ///
    /// 由呼叫端決定用哪個日誌管道輸出，同一個行程應只記一次。
    pub warning: Option<String>,
}

/// 由完整 SID 推導 pipe 名稱，取最後一段（RID）。
///
/// 只接受 `S-…-<數字>` 形式；格式不對回 `None`，交給呼叫端退回共用名稱。
///
/// # 範例
///
/// ```
/// assert_eq!(
///     jc_windows::name_from_sid("S-1-5-21-1-2-3-1001").as_deref(),
///     Some("jc-ime-1001")
/// );
/// assert_eq!(jc_windows::name_from_sid(""), None);
/// ```
pub fn name_from_sid(sid: &str) -> Option<String> {
    let sid = sid.trim();
    if !sid.starts_with("S-") {
        return None;
    }
    let rid = sid.rsplit('-').next()?;
    if rid.is_empty() || !rid.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(format!("{BASE_PIPE_NAME}-{rid}"))
}

/// 依「覆寫優先、其次 SID、最後退回共用名稱」決定 pipe 名稱。
///
/// 這個函式不碰 OS，測試用它鎖住命名規則。
pub fn choose_pipe_name(override_name: Option<&str>, sid: Option<&str>) -> ResolvedPipeName {
    if let Some(name) = override_name.map(str::trim).filter(|name| !name.is_empty()) {
        return ResolvedPipeName {
            name: name.to_string(),
            warning: None,
        };
    }

    if let Some(name) = sid.and_then(name_from_sid) {
        return ResolvedPipeName {
            name,
            warning: None,
        };
    }

    ResolvedPipeName {
        name: BASE_PIPE_NAME.to_string(),
        warning: Some(format!(
            "取不到目前使用者的 SID，引擎 pipe 退回共用名稱 {BASE_PIPE_NAME}；\
             同一台機器上多個使用者會連到同一個引擎"
        )),
    }
}

/// 解析目前使用者的引擎 pipe 名稱。
///
/// 不快取：呼叫端需要自行決定何時記錄 [`ResolvedPipeName::warning`]，通常每個
/// 行程只記一次。非 Windows 平台沒有 SID，回傳固定名稱或 [`PIPE_ENV`] 的覆寫
/// 值，且不帶警告。
pub fn resolve_pipe_name() -> ResolvedPipeName {
    let override_name = env_override().or_else(registry_override);

    #[cfg(windows)]
    {
        choose_pipe_name(override_name.as_deref(), current_user_sid().as_deref())
    }

    #[cfg(not(windows))]
    {
        ResolvedPipeName {
            name: override_name.unwrap_or_else(|| BASE_PIPE_NAME.to_string()),
            warning: None,
        }
    }
}

/// 目前使用者的 SID 字串（`S-1-5-21-…-1001`）；取不到回 `None`。
///
/// 非 Windows 平台一律回 `None`。
#[cfg(windows)]
pub fn current_user_sid() -> Option<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::TOKEN_QUERY;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: 取得目前行程的 token，handle 成功時由這裡負責關閉。
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }
    let sid = token_sid_string(token);
    // SAFETY: token 由上面的 OpenProcessToken 取得，只關一次。
    unsafe { CloseHandle(token) };
    sid
}

/// 非 Windows 平台沒有 SID。
#[cfg(not(windows))]
pub fn current_user_sid() -> Option<String> {
    None
}

/// 從 token 取出使用者 SID 並轉成字串。
#[cfg(windows)]
fn token_sid_string(token: windows_sys::Win32::Foundation::HANDLE) -> Option<String> {
    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_USER};

    let mut needed = 0u32;
    // SAFETY: 第一次呼叫只查詢所需長度，緩衝區為空指標是這個 API 的標準用法。
    unsafe {
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
    }
    if needed == 0 {
        return None;
    }

    // `TOKEN_USER` 內含指標，配置時要對齊 `usize`，不能用 `Vec<u8>`。
    let mut buffer = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
    let mut returned = 0u32;
    // SAFETY: buffer 的位元組長度不小於 needed，長度與指標一致。
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr() as *mut c_void,
            needed,
            &mut returned,
        )
    };
    if ok == 0 {
        return None;
    }

    // SAFETY: API 成功時 buffer 內是完整的 `TOKEN_USER`，對齊已符合。
    let user = unsafe { &*(buffer.as_ptr() as *const TOKEN_USER) };
    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: user.User.Sid 指向 buffer 內的有效 SID；成功時 text 可用 LocalFree 釋放。
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) } == 0 {
        return None;
    }
    // SAFETY: ConvertSidToStringSidW 保證是以 NUL 結尾的 UTF-16。
    let value = unsafe { wide_to_string(text) };
    // SAFETY: text 由 ConvertSidToStringSidW 配置，只釋放一次。
    unsafe { LocalFree(text as *mut c_void) };
    Some(value)
}

/// 讀 [`PIPE_ENV`]；空字串或只有空白視同未設定。
fn env_override() -> Option<String> {
    let value = std::env::var(PIPE_ENV).ok()?;
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// 讀 `HKCU\Software\jc-ime\EnginePipe`；沒有設定回 `None`。
#[cfg(windows)]
fn registry_override() -> Option<String> {
    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};

    let key = wide(REGISTRY_KEY);
    let value = wide(REGISTRY_VALUE);
    let mut buffer = [0u16; 256];
    let mut size = (buffer.len() * std::mem::size_of::<u16>()) as u32;

    // SAFETY: buffer 與 size 的長度一致，兩個字串都以 NUL 結尾。
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr() as *mut c_void,
            &mut size,
        )
    };
    if status != ERROR_SUCCESS || size < 2 {
        return None;
    }

    let units = (size as usize / 2).saturating_sub(1).min(buffer.len());
    let text = String::from_utf16_lossy(&buffer[..units]);
    let text = text.trim_matches('\0').trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// 非 Windows 平台沒有註冊機碼覆寫。
#[cfg(not(windows))]
fn registry_override() -> Option<String> {
    None
}

/// 把字串轉成 NUL 結尾的 UTF-16。
#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 把以 NUL 結尾的 UTF-16 字串複製成 [`String`]。
///
/// # Safety
///
/// `text` 必須指向可讀取、以 NUL 結尾的 UTF-16 序列。
#[cfg(windows)]
unsafe fn wide_to_string(text: *const u16) -> String {
    let mut len = 0usize;
    // SAFETY: 呼叫端保證字串以 NUL 結尾，迴圈不會讀出界。
    while unsafe { *text.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: 前面已確認前 `len` 個元素可讀。
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_pipe_name_is_stable() {
        assert_eq!(BASE_PIPE_NAME, "jc-ime");
    }

    #[test]
    fn name_from_sid_uses_last_segment() {
        assert_eq!(
            name_from_sid("S-1-5-21-1111-2222-3333-1001").as_deref(),
            Some("jc-ime-1001")
        );
        assert_eq!(name_from_sid("S-1-5-18").as_deref(), Some("jc-ime-18"));
    }

    #[test]
    fn name_from_sid_rejects_incomplete_values() {
        assert_eq!(name_from_sid(""), None);
        assert_eq!(name_from_sid("S-1-5-21-"), None);
        assert_eq!(name_from_sid("S-1-5-21-abc"), None);
        assert_eq!(name_from_sid("1001"), None);
    }

    #[test]
    fn same_user_gets_same_name() {
        let sid = "S-1-5-21-1111-2222-3333-1001";
        assert_eq!(
            choose_pipe_name(None, Some(sid)),
            choose_pipe_name(None, Some(sid))
        );
        assert_eq!(choose_pipe_name(None, Some(sid)).name, "jc-ime-1001");
    }

    #[test]
    fn different_users_get_different_names() {
        let first = choose_pipe_name(None, Some("S-1-5-21-1-2-3-1001"));
        let second = choose_pipe_name(None, Some("S-1-5-21-1-2-3-1002"));
        assert_ne!(first.name, second.name);
        assert_eq!(first.name, "jc-ime-1001");
        assert_eq!(second.name, "jc-ime-1002");
    }

    #[test]
    fn override_wins_over_sid() {
        let resolved = choose_pipe_name(Some("  custom-pipe "), Some("S-1-5-21-1-2-3-1001"));
        assert_eq!(resolved.name, "custom-pipe");
        assert!(resolved.warning.is_none());
    }

    #[test]
    fn blank_override_is_ignored() {
        let resolved = choose_pipe_name(Some("   "), Some("S-1-5-21-1-2-3-1001"));
        assert_eq!(resolved.name, "jc-ime-1001");
    }

    #[test]
    fn missing_sid_falls_back_with_one_warning() {
        let resolved = choose_pipe_name(None, None);
        assert_eq!(resolved.name, "jc-ime");
        let warning = resolved.warning.expect("退回共用名稱要帶警告");
        assert!(
            warning.contains("SID") && warning.contains("jc-ime"),
            "警告要說明原因與退回的名稱：{warning}"
        );
    }

    #[test]
    fn unparsable_sid_falls_back() {
        let resolved = choose_pipe_name(None, Some("not-a-sid"));
        assert_eq!(resolved.name, "jc-ime");
        assert!(resolved.warning.is_some());
    }

    /// 非 Windows 平台沒有 SID，解析結果不該帶警告。
    #[cfg(not(windows))]
    #[test]
    fn non_windows_resolution_has_no_warning() {
        let resolved = resolve_pipe_name();
        assert!(resolved.warning.is_none());
        if std::env::var(PIPE_ENV).is_err() {
            assert_eq!(resolved.name, BASE_PIPE_NAME);
        }
    }
}
