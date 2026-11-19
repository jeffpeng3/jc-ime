//! 日文詞典 pack（`0CJADIC\0` v1／v2）的讀取、驗證與 manifest 配對。
//!
//! 版面與欄位定義見 `jc-ime-data/plan/reference/pack-format.md` 與
//! `pack-v2-spec.md`。本模組只負責讀取端；寫入端不在這個階段。

mod error;
mod manifest;
mod order;
mod pack;

pub use error::PackError;
pub use manifest::{
    Manifest, ManifestCounts, ManifestOutputs, ManifestPack, PackFormat, MANIFEST_SCHEMA,
};
pub use order::{compare_entries, sort_surface_entries};
pub use pack::{
    Capabilities, Entry, Header, Pack, PrefixIter, Split, Stats, ENTRY_SIZE, ENTRY_SIZE_V2,
    FORMAT_VERSION, FORMAT_VERSION_V2, HEADER_SIZE, HEADER_SIZE_V2, INDEX_ENTRY_SIZE, MAGIC,
    MIN_COST, TRAILER_SIZE,
};

#[cfg(test)]
mod tests;
