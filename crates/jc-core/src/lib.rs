//! 平台無關的輸入法核心：鍵序列、片段、路徑評分、候選。
//!
//! 這個 crate 不得依賴任何作業系統 API，所有平台相關行為都由
//! `jc-platform` 的 trait 注入，方便在 Linux 上開發與測試。
//!
//! 本檔只負責宣告 module 與轉出公開型別；各 module 的實作由各自的
//! 移植 task 補上，後續不需要再改動這裡。

pub mod candidate;
pub mod candidate_layout;
pub mod composition;
pub mod decode;
pub mod emoji;
pub mod inputstyle;
pub mod ja;
pub mod key;
pub mod lang;
pub mod lattice;
pub mod learn_guard;
pub mod lm;
pub mod mixed;
pub mod mixed_latin;
pub mod output;
pub mod personal;
pub mod punct;
pub mod reading;
pub mod score;
pub mod segment;
pub mod session;
pub mod suggest;
pub mod symbols;
pub mod vocab;
pub mod zh;
pub mod zhconv;

pub use candidate::{Candidate, CandidateSource};
pub use composition::{
    AutoRun, CompSegment, Composition, ConvertBackend, ConvertContext, ConvertError, Converted,
    Lang as CompLang, Owner, UserMark,
};
pub use decode::{Conversion, EngineError};
pub use key::{Key, KeyCode};
pub use lang::{Lang, Mode};
pub use mixed::{MixedResult, MixedSegment, Router};
pub use output::Snapshot;
pub use segment::{Script, Segment};
pub use session::CORE_VERSION;
