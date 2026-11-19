//! Mozc OSS 詞典匯入與 0CJADIC pack v2 產生器。
//!
//! 輸入是 Mozc `dictionary_oss` 目錄的十份 `dictionaryNN.txt`、`id.def`
//! 與 `connection_single_column.txt`；輸出是
//! [`PACK_FILE`] 與 [`MANIFEST_FILE`]。格式以 pack v2 spec 為準。

pub mod error;
pub mod import;
pub mod mozc;
pub mod pack;
pub mod reading;

pub use error::ImportError;
pub use import::{
    collect_entries, import, sha256_hex, ImportOptions, ImportReport, ImportStats, MANIFEST_FILE,
    PACK_FILE,
};
pub use mozc::{IdInfo, IdTable, RawRow};
pub use pack::{encode, Encoded, PackData, PackEntry};
pub use reading::normalize_reading;
