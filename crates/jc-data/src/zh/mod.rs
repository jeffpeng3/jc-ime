//! 注音資料包：`lexicon.bin`、`lm.bin` 與 `manifest.json`。
//!
//! 對照前身 Go `core/binfmt`、`core/lexicon/compact.go` 與
//! `core/lm/compact.go`。檔案以 `memmap2` 映射，詞條與詞表都直接借用
//! 映射內容；[`Bundle::open`] 會先比對 manifest 的大小與 SHA-256，再驗證
//! 各自格式的魔術字、版本、CRC32 與欄位範圍。

pub mod error;
pub mod format;
pub mod lexicon;
pub mod lm;

#[cfg(test)]
pub(crate) mod testutil;

#[cfg(test)]
mod tests;

pub use error::ZhDataError;
pub use format::{
    FileInfo, Manifest, Reader, LEXICON_MAGIC, LEXICON_VERSION, LM_MAGIC, LM_VERSION,
};
pub use lexicon::{Entries, EntryView, Lexicon, FLAG_CORRECTION, FLAG_RARE};
pub use lm::{Lm, BOS, UNSEEN};

use std::path::Path;

/// 一份完整載入的注音資料包。
pub struct Bundle {
    /// 詞庫。
    pub lexicon: Lexicon,
    /// unigram 與 bigram 語言模型。
    pub lm: Lm,
    /// 驗證過的 manifest。
    pub manifest: Manifest,
}

impl Bundle {
    /// 從資料目錄載入 `manifest.json`、`lexicon.bin` 與 `lm.bin`。
    ///
    /// 兩個二進位檔都必須符合 manifest 的大小與 SHA-256。
    pub fn open(dir: impl AsRef<Path>) -> Result<Bundle, ZhDataError> {
        let dir = dir.as_ref();
        let manifest = Manifest::load(dir)?;
        let lexicon_info = manifest
            .file("lexicon.bin")
            .ok_or_else(|| ZhDataError::Layout("manifest 缺少 lexicon.bin 的記錄".into()))?
            .clone();
        let lm_info = manifest
            .file("lm.bin")
            .ok_or_else(|| ZhDataError::Layout("manifest 缺少 lm.bin 的記錄".into()))?
            .clone();
        let lexicon = Lexicon::open_verified(dir.join("lexicon.bin"), &lexicon_info)?;
        let lm = Lm::open_verified(dir.join("lm.bin"), &lm_info)?;
        Ok(Bundle {
            lexicon,
            lm,
            manifest,
        })
    }
}
