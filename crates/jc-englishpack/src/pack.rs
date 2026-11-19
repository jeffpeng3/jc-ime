//! 資料包寫出與讀取。
//!
//! 寫出時先建暫存目錄，內容齊全後再改名；輸出目錄必須是新的或空的。
//! 讀取委派給 [`jc_core::mixed::Dictionary::load_pack`]，manifest 與
//! `words.bin`、`NOTICE.txt` 的大小及 SHA-256 都要相符。

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use jc_core::mixed::{
    sha256_hex, Dictionary, EnglishManifest, EnglishPackFile, MixedError, MAX_WORD_BYTES,
};

use crate::error::BuildError;
use crate::{MANIFEST_FILE, NOTICE_FILE, WORDS_FILE};

/// 資料包詞數上限。
pub const MAX_PACK_WORDS: usize = 200_000;

/// 讀取資料包；回傳詞表與 manifest。
pub fn load(dir: &Path) -> Result<(Dictionary, EnglishManifest), MixedError> {
    Dictionary::load_pack(dir)
}

/// 寫出完整資料包。
///
/// `manifest` 的 `format`、`words`、`files` 由這裡覆寫；呼叫端只要提供
/// `ranking`、`sources` 與 `inputs`。詞表會先編碼並解回來驗一次，確認產物
/// 可以載入後才寫檔。
pub fn write(
    dir: &Path,
    dict: &Dictionary,
    mut manifest: EnglishManifest,
    notice: &[u8],
) -> Result<EnglishManifest, BuildError> {
    if String::from_utf8_lossy(notice).trim().is_empty() {
        return Err(BuildError::Source("缺少授權文字".to_string()));
    }
    if dict.is_empty() || dict.len() > MAX_PACK_WORDS {
        return Err(BuildError::Source(format!("詞數 {} 不合法", dict.len())));
    }
    for word in dict.words() {
        if word.text.len() > MAX_WORD_BYTES {
            return Err(BuildError::Source("詞面過長".to_string()));
        }
    }
    let binary = dict.encode();
    if Dictionary::decode(&binary).is_err() {
        return Err(BuildError::Source("編碼後的詞表無法解回".to_string()));
    }
    manifest.format = 1;
    manifest.words = dict.len();
    manifest.files = BTreeMap::from([
        (
            WORDS_FILE.to_string(),
            EnglishPackFile {
                sha256: sha256_hex(&binary),
                size: binary.len() as u64,
            },
        ),
        (
            NOTICE_FILE.to_string(),
            EnglishPackFile {
                sha256: sha256_hex(notice),
                size: notice.len() as u64,
            },
        ),
    ]);
    manifest
        .validate()
        .map_err(|err| BuildError::Pack(err.to_string()))?;

    if let Ok(info) = std::fs::symlink_metadata(dir) {
        if info.file_type().is_symlink() {
            return Err(BuildError::Pack("輸出不接受符號連結".to_string()));
        }
        if info.is_dir() {
            let entries = std::fs::read_dir(dir).map_err(|err| BuildError::io(dir, err))?;
            if entries.into_iter().next().is_some() {
                return Err(BuildError::Pack(format!(
                    "輸出已存在，請使用新目錄：{}",
                    dir.display()
                )));
            }
        } else {
            return Err(BuildError::Pack(format!("輸出不是目錄：{}", dir.display())));
        }
    }
    let parent = dir.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|err| BuildError::io(parent, err))?;

    let staging = unique_staging_dir(parent);
    std::fs::create_dir(&staging).map_err(|err| BuildError::io(&staging, err))?;
    let result = write_files(&staging, &binary, notice, &manifest);
    if let Err(err) = result {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(err);
    }
    if dir.is_dir() {
        std::fs::remove_dir(dir).map_err(|err| BuildError::io(dir, err))?;
    }
    std::fs::rename(&staging, dir).map_err(|err| {
        let _ = std::fs::remove_dir_all(&staging);
        BuildError::io(dir, err)
    })?;
    Ok(manifest)
}

fn write_files(
    dir: &Path,
    binary: &[u8],
    notice: &[u8],
    manifest: &EnglishManifest,
) -> Result<(), BuildError> {
    let mut text = serde_json::to_vec_pretty(manifest)?;
    text.push(b'\n');
    for (name, data) in [
        (WORDS_FILE, binary),
        (NOTICE_FILE, notice),
        (MANIFEST_FILE, text.as_slice()),
    ] {
        let path = dir.join(name);
        let mut file = std::fs::File::create(&path).map_err(|err| BuildError::io(&path, err))?;
        file.write_all(data)
            .map_err(|err| BuildError::io(&path, err))?;
        file.sync_all().map_err(|err| BuildError::io(&path, err))?;
    }
    Ok(())
}

/// 產生同一目錄下不重複的暫存目錄名稱。
fn unique_staging_dir(parent: &Path) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(".0c-english-{}-{serial}", std::process::id()))
}
