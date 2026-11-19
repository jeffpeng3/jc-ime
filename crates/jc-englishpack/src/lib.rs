//! 英文詞表建置：ESDB／SCOWL 正規化 TSV 加產品詞表，輸出 `0cEN` v1 資料包。
//!
//! 這個 crate 只做檔案轉換與驗證，不連網、不執行上游程式。輸入的
//! `source.json` 固定每個檔案的 SHA-256，任何不符都在寫出產物前拒絕。
//!
//! 執行期查詢在 `jc_core::mixed`：`words.bin` 解碼成 [`jc_core::mixed::Dictionary`]。

pub mod compile;
pub mod error;
mod fsutil;
pub mod import;
pub mod pack;

pub use compile::{commit_from_version, compile, Export, SourceMetadata};
pub use error::BuildError;
pub use import::import_lists;
pub use pack::{load, write};

/// 資料包裡放詞表的檔名。
pub const WORDS_FILE: &str = "words.bin";

/// 資料包裡放授權聲明的檔名。
pub const NOTICE_FILE: &str = "NOTICE.txt";

/// 資料包裡放 manifest 的檔名。
pub const MANIFEST_FILE: &str = "manifest.json";
