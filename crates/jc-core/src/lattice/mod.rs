//! 候選詞格（lattice）建構與 Viterbi 解碼。
//!
//! 讀音進來後依詞典建立節點與邊，再用成本模型找出最短路徑。詞典的實際
//! 二進位讀取由 `w1-data-zh` 負責，這裡只定 [`Lexicon`] 介面與演算法。

pub mod build;
pub mod candidate;
pub mod lexicon;
pub mod viterbi;

pub use build::{Lattice, LatticeError};
pub use candidate::Edge;
pub use lexicon::{InMemoryLexicon, LexEntry, Lexicon, UNKNOWN_POS};
pub use viterbi::{Path, Viterbi};
