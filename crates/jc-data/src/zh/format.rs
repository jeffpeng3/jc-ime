//! 注音資料包的底層二進位格式與 manifest 驗證。
//!
//! 對照前身 Go `core/binfmt/binfmt.go`：檔頭是魔術字、`u32` 版本，之後
//! 依序排列區段，檔尾是整份內容（不含檔尾）的 CRC32。全部小端序、不含
//! 指標，因此同一份位元組可以直接 mmap 讀取。
//!
//! 每個區段先寫 `u32` 元素數，再寫元素本身；字串是一段 `u32` 長度加
//! 原始位元組。`Reader` 依寫入順序讀取，寫入端在
//! 前身 `core/lexicon/compact.go`（`ZUINLEX`）與 `core/lm/compact.go`
//! （`ZUINLM`）。

use std::path::Path;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::error::ZhDataError;

/// `lexicon.bin` 的魔術字。
pub const LEXICON_MAGIC: &[u8] = b"ZUINLEX\0";
/// `lexicon.bin` 的格式版本。
pub const LEXICON_VERSION: u32 = 1;
/// `lm.bin` 的魔術字。
pub const LM_MAGIC: &[u8] = b"ZUINLM\0\0";
/// `lm.bin` 的格式版本。
pub const LM_VERSION: u32 = 2;
/// 檔尾 CRC32 的大小。
pub const CRC_SIZE: usize = 4;

/// 依寫入順序讀取二進位區段。
///
/// 所有回傳的切片都借用原始檔案位元組，不解碼、不複製；數值欄位用
/// [`le_u16`]、[`le_u32`]、[`le_f32`] 在存取時讀出。
#[derive(Debug)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// 驗證魔術字、版本與檔尾 CRC32，並定位到第一個區段。
    pub fn new(file: &'a [u8], magic: &[u8], version: u32) -> Result<Self, ZhDataError> {
        let minimum = magic.len() + 8;
        if file.len() < minimum {
            return Err(ZhDataError::TooSmall {
                size: file.len(),
                minimum,
            });
        }
        if &file[..magic.len()] != magic {
            return Err(ZhDataError::BadMagic {
                expected: match magic {
                    LEXICON_MAGIC => "ZUINLEX",
                    LM_MAGIC => "ZUINLM",
                    _ => "未知格式",
                },
                actual: String::from_utf8_lossy(&file[..magic.len()]).into_owned(),
            });
        }
        let (body, trailer) = file.split_at(file.len() - CRC_SIZE);
        let expected = u32::from_le_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
        let actual = crc32fast::hash(body);
        if actual != expected {
            return Err(ZhDataError::CrcMismatch { expected, actual });
        }
        let mut reader = Reader {
            data: &body[magic.len()..],
            pos: 0,
        };
        let found = reader.u32()?;
        if found != version {
            return Err(ZhDataError::UnsupportedVersion {
                actual: found,
                expected: version,
            });
        }
        Ok(reader)
    }

    /// 剩餘未讀的位元組數。
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], ZhDataError> {
        if count > self.remaining() {
            return Err(ZhDataError::Truncated);
        }
        let (head, _tail) = self.data[self.pos..].split_at(count);
        self.pos += count;
        Ok(head)
    }

    /// 讀一個 `u32`。
    pub fn u32(&mut self) -> Result<u32, ZhDataError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// 讀一個帶元素數的 `u16` 區段，回傳原始位元組。
    pub fn u16s(&mut self) -> Result<&'a [u8], ZhDataError> {
        self.array(2)
    }

    /// 讀一個帶元素數的 `u32` 區段，回傳原始位元組。
    pub fn u32s(&mut self) -> Result<&'a [u8], ZhDataError> {
        self.array(4)
    }

    /// 讀一個帶元素數的 `f32` 區段，回傳原始位元組。
    pub fn f32s(&mut self) -> Result<&'a [u8], ZhDataError> {
        self.array(4)
    }

    /// 讀一個帶位元組數的 `u8` 區段。
    pub fn bytes(&mut self) -> Result<&'a [u8], ZhDataError> {
        let count = self.u32()? as usize;
        self.take(count)
    }

    /// 讀一個帶位元組數的 UTF-8 字串。
    pub fn string(&mut self) -> Result<&'a str, ZhDataError> {
        let bytes = self.bytes()?;
        std::str::from_utf8(bytes).map_err(|_| ZhDataError::InvalidUtf8 {
            what: "字串區段"
        })
    }

    fn array(&mut self, size: usize) -> Result<&'a [u8], ZhDataError> {
        let count = self.u32()? as usize;
        let bytes = count.checked_mul(size).ok_or(ZhDataError::Truncated)?;
        self.take(bytes)
    }

    /// 確認所有區段都讀完。
    pub fn done(self) -> Result<(), ZhDataError> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(ZhDataError::TrailingBytes(self.remaining()))
        }
    }
}

/// 讀出 `bytes[index]` 的 `u16`（小端序）。
///
/// `index` 以元素為單位，不是位元組位移。
pub fn le_u16(bytes: &[u8], index: usize) -> u16 {
    let at = index * 2;
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

/// 讀出 `bytes[index]` 的 `u32`（小端序）。
pub fn le_u32(bytes: &[u8], index: usize) -> u32 {
    let at = index * 4;
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// 讀出 `bytes[index]` 的 `f32`（小端序）。
pub fn le_f32(bytes: &[u8], index: usize) -> f32 {
    f32::from_bits(le_u32(bytes, index))
}

/// 回傳切片在整個檔案位元組中的位移範圍。
pub(crate) fn span(file: &[u8], slice: &[u8]) -> std::ops::Range<usize> {
    let start = slice.as_ptr() as usize - file.as_ptr() as usize;
    start..start + slice.len()
}

/// 在 `0..len` 上做二分搜尋，回傳第一個不滿足 `pred` 的位置。
///
/// 與 `slice::partition_point` 相同，只是索引由外部提供，資料不必連續。
pub(crate) fn partition_point(len: usize, mut pred: impl FnMut(usize) -> bool) -> usize {
    let mut low = 0;
    let mut high = len;
    while low < high {
        let mid = low + (high - low) / 2;
        if pred(mid) {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    low
}

/// `manifest.json` 的檔案記錄。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FileInfo {
    /// 整檔 SHA-256（小寫 hex）。
    pub sha256: String,
    /// 檔案大小。
    pub size: u64,
}

/// 注音資料包的 `manifest.json`。
///
/// 只解析載入時需要的欄位；其餘欄位保留成 JSON 值，不影響讀取。
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    /// manifest 格式版本；目前是 1。
    pub format: u32,
    /// 檔名 → 雜湊與大小。
    pub files: std::collections::BTreeMap<String, FileInfo>,
    /// 規模統計（詞條數、詞數、bigram 數等）。
    #[serde(default)]
    pub counts: std::collections::BTreeMap<String, u64>,
    /// 編譯選項；保留原始 JSON。
    #[serde(default)]
    pub options: serde_json::Value,
}

impl Manifest {
    /// 從資料目錄載入 `manifest.json`。
    pub fn load(dir: &Path) -> Result<Manifest, ZhDataError> {
        let path = dir.join("manifest.json");
        let text = std::fs::read(&path).map_err(|source| ZhDataError::Io {
            path: path.clone(),
            source,
        })?;
        serde_json::from_slice(&text).map_err(|source| ZhDataError::BadManifest { path, source })
    }

    /// 查某個檔案的記錄。
    pub fn file(&self, name: &str) -> Option<&FileInfo> {
        self.files.get(name)
    }

    /// 查某項統計數字。
    pub fn count(&self, name: &str) -> Option<u64> {
        self.counts.get(name).copied()
    }

    /// 比對檔案大小與 SHA-256。
    pub fn verify(&self, name: &str, bytes: &[u8]) -> Result<(), ZhDataError> {
        let Some(info) = self.file(name) else {
            return Err(ZhDataError::Layout(format!("manifest 缺少 {name} 的記錄")));
        };
        if info.size != bytes.len() as u64 {
            return Err(ZhDataError::ManifestMismatch {
                file: name.to_string(),
                field: "size",
                expected: info.size.to_string(),
                actual: bytes.len().to_string(),
            });
        }
        let actual = sha256_hex(bytes);
        if !info.sha256.eq_ignore_ascii_case(&actual) {
            return Err(ZhDataError::ManifestMismatch {
                file: name.to_string(),
                field: "sha256",
                expected: info.sha256.clone(),
                actual,
            });
        }
        Ok(())
    }
}

/// 整份位元組的 SHA-256（小寫 hex）。
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zh::testutil::Builder;

    #[test]
    fn reader_accepts_valid_payload() {
        let bytes = Builder::new(LEXICON_MAGIC, LEXICON_VERSION)
            .u32(7)
            .string("ㄨㄛˇ")
            .finish();
        let mut reader = Reader::new(&bytes, LEXICON_MAGIC, LEXICON_VERSION).unwrap();
        assert_eq!(reader.u32().unwrap(), 7);
        assert_eq!(reader.string().unwrap(), "ㄨㄛˇ");
        reader.done().unwrap();
    }

    #[test]
    fn reader_rejects_bad_magic() {
        let bytes = Builder::new(LM_MAGIC, LM_VERSION).finish();
        let err = Reader::new(&bytes, LEXICON_MAGIC, LEXICON_VERSION).unwrap_err();
        assert!(matches!(err, ZhDataError::BadMagic { .. }));
    }

    #[test]
    fn reader_rejects_bad_version() {
        let bytes = Builder::new(LEXICON_MAGIC, 9).finish();
        let err = Reader::new(&bytes, LEXICON_MAGIC, LEXICON_VERSION).unwrap_err();
        assert!(matches!(
            err,
            ZhDataError::UnsupportedVersion {
                actual: 9,
                expected: 1
            }
        ));
    }

    #[test]
    fn reader_rejects_crc_mismatch() {
        let mut bytes = Builder::new(LEXICON_MAGIC, LEXICON_VERSION).u32(1).finish();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        let err = Reader::new(&bytes, LEXICON_MAGIC, LEXICON_VERSION).unwrap_err();
        assert!(matches!(err, ZhDataError::CrcMismatch { .. }));
    }

    #[test]
    fn reader_reports_trailing_bytes() {
        let bytes = Builder::new(LEXICON_MAGIC, LEXICON_VERSION).u32(1).finish();
        let reader = Reader::new(&bytes, LEXICON_MAGIC, LEXICON_VERSION).unwrap();
        let err = reader.done().unwrap_err();
        assert!(matches!(err, ZhDataError::TrailingBytes(4)));
    }

    #[test]
    fn manifest_verifies_size_and_hash() {
        let bytes = Builder::new(LEXICON_MAGIC, LEXICON_VERSION).u32(1).finish();
        let manifest: Manifest = serde_json::from_value(serde_json::json!({
            "format": 1,
            "files": {
                "lexicon.bin": {
                    "sha256": sha256_hex(&bytes),
                    "size": bytes.len(),
                }
            },
        }))
        .unwrap();
        manifest.verify("lexicon.bin", &bytes).unwrap();

        let mut tampered = bytes.clone();
        tampered[0] ^= 1;
        let err = manifest.verify("lexicon.bin", &tampered).unwrap_err();
        assert!(matches!(
            err,
            ZhDataError::ManifestMismatch {
                field: "sha256",
                ..
            }
        ));

        let err = manifest.verify("lm.bin", &bytes).unwrap_err();
        assert!(matches!(err, ZhDataError::Layout(_)));
    }
}
