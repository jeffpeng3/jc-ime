//! 安裝檔案檢查（檢查 8）。
//!
//! 從 `InprocServer32` 推出安裝目錄，逐一確認四個必要檔案，並輸出每個檔案的
//! SHA-256。PE 檔沒有版本資源只記警告：現行建置不一定會嵌入版本資訊，沒有版本
//! 資源不影響載入。
//!
//! 資料根目錄與 MSI 的 `[DATADIR]` 一致：`%ProgramData%\jc-ime\data`。

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FFI_SIGNATURE,
    VS_FIXEDFILEINFO,
};

use crate::profile;

/// 引擎執行檔名。
pub const ENGINE_EXE: &str = "jc-engine.exe";

/// 資料根目錄的索引檔名。
pub const DATA_MANIFEST: &str = "manifest.json";

/// 一個被檢查的檔案。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileCheck {
    /// 顯示用的名稱。
    pub label: String,
    /// 完整路徑。
    pub path: PathBuf,
    /// 檔案是否存在。
    pub present: bool,
    /// 檔案內容的 SHA-256（小寫十六進位）；讀不到時為 `None`。
    pub sha256: Option<String>,
    /// PE 的檔案版本；沒有版本資源時為 `None`。
    pub version: Option<String>,
}

impl FileCheck {
    /// 檔案存在但沒有版本資源：只警告，不算失敗。
    pub fn missing_version(&self) -> bool {
        self.present && self.version.is_none()
    }

    /// 一行摘要：路徑、SHA-256、版本。
    pub fn detail(&self) -> String {
        if !self.present {
            return format!("{} 不存在", self.path.display());
        }
        let sha = self.sha256.as_deref().unwrap_or("讀取失敗");
        match &self.version {
            Some(version) => format!("{}（SHA-256 {sha}，版本 {version}）", self.path.display()),
            None => format!("{}（SHA-256 {sha}，沒有版本資源）", self.path.display()),
        }
    }

    /// 只含路徑與 SHA-256 的摘要；給不是 PE 的資料檔用。
    pub fn sha_detail(&self) -> String {
        if !self.present {
            return format!("{} 不存在", self.path.display());
        }
        let sha = self.sha256.as_deref().unwrap_or("讀取失敗");
        format!("{}（SHA-256 {sha}）", self.path.display())
    }
}

/// 安裝目錄的探測結果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallProbe {
    /// 安裝目錄（`InprocServer32` 的上一層）。
    pub dir: PathBuf,
    /// `jc-tsf.dll`、`jc-tsf-x86.dll`、`jc-engine.exe`。
    pub files: Vec<FileCheck>,
    /// 資料根目錄的 `manifest.json`。
    pub manifest: FileCheck,
}

/// 從 `InprocServer32` 的路徑建立探測結果；推不出目錄時回 `None`。
pub fn probe(inproc: &str) -> Option<InstallProbe> {
    let dir = profile::install_dir(inproc)?;
    let files = ["jc-tsf.dll", "jc-tsf-x86.dll", ENGINE_EXE]
        .into_iter()
        .map(|name| check_file(dir.join(name), name))
        .collect();
    let manifest_path = data_root().join(DATA_MANIFEST);
    let manifest = check_file(manifest_path, DATA_MANIFEST);
    Some(InstallProbe {
        dir,
        files,
        manifest,
    })
}

/// 資料根目錄：`%ProgramData%\jc-ime\data`。
pub fn data_root() -> PathBuf {
    if let Some(value) = std::env::var_os("ProgramData").filter(|value| !value.is_empty()) {
        return PathBuf::from(value).join("jc-ime").join("data");
    }
    PathBuf::from(r"C:\ProgramData\jc-ime\data")
}

/// 檢查一個檔案：存在性、SHA-256、版本資源。
pub fn check_file(path: PathBuf, label: &str) -> FileCheck {
    let present = path.is_file();
    let sha256 = if present {
        sha256_file(&path).ok()
    } else {
        None
    };
    let version = if present { file_version(&path) } else { None };
    FileCheck {
        label: label.to_string(),
        path,
        present,
        sha256,
        version,
    }
}

/// 計算檔案的 SHA-256（小寫十六進位）。
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// 讀 PE 的 `VS_FIXEDFILEINFO` 版本；沒有版本資源回 `None`。
pub fn file_version(path: &Path) -> Option<String> {
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: wide 以 NUL 結尾；第二個參數允許 null。
    let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None) };
    if size == 0 {
        return None;
    }

    let mut buffer = vec![0u8; size as usize];
    // SAFETY: buffer 的大小就是 GetFileVersionInfoSizeW 回報的位元組數。
    unsafe {
        GetFileVersionInfoW(
            PCWSTR(wide.as_ptr()),
            None,
            size,
            buffer.as_mut_ptr().cast(),
        )
    }
    .ok()?;

    let root: Vec<u16> = r"\".encode_utf16().chain(std::iter::once(0)).collect();
    let mut pointer: *mut c_void = std::ptr::null_mut();
    let mut length = 0u32;
    // SAFETY: buffer 是剛填入的版本資源；root 以 NUL 結尾。
    let ok = unsafe {
        VerQueryValueW(
            buffer.as_ptr().cast(),
            PCWSTR(root.as_ptr()),
            &mut pointer,
            &mut length,
        )
    };
    if !ok.as_bool() || pointer.is_null() {
        return None;
    }
    if (length as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>() {
        return None;
    }
    // SAFETY: VerQueryValueW 回報的長度至少有一個 VS_FIXEDFILEINFO，且指標對齊。
    let info = unsafe { &*(pointer as *const VS_FIXEDFILEINFO) };
    if info.dwSignature != VS_FFI_SIGNATURE as u32 {
        return None;
    }
    Some(format!(
        "{}.{}.{}.{}",
        info.dwFileVersionMS >> 16,
        info.dwFileVersionMS & 0xffff,
        info.dwFileVersionLS >> 16,
        info.dwFileVersionLS & 0xffff
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_vector() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("jc-tsf-smoke-sha-{}.txt", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_file_is_reported_as_absent() {
        let path = std::env::temp_dir().join("jc-tsf-smoke-does-not-exist.dll");
        let check = check_file(path, "不存在.dll");
        assert!(!check.present);
        assert!(check.sha256.is_none());
        assert!(check.detail().contains("不存在"));
    }

    #[test]
    fn data_root_uses_program_data() {
        let root = data_root();
        assert!(root.ends_with(r"jc-ime\data") || root.ends_with("jc-ime/data"));
    }

    #[test]
    fn probe_requires_a_directory_component() {
        assert!(probe("jc-tsf.dll").is_none());
    }
}
