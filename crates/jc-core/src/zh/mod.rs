//! 注音輸入前端。
//!
//! 這個模組負責把注音的各種寫法整理成統一的音節字串，交給 lattice 與解碼器：
//! 音節結構與逐符號組音（[`Syllable`]、[`Composer`]）、拼音轉注音
//! （[`pinyin`]）、實體按鍵排列（[`Keyboard`]）、累積按鍵的狀態機
//! （[`ZhInput`]）與模糊音規則（[`fuzzy`]）。
//!
//! 音節的正規字串形式是「聲母 介音 韻母 聲調」，一聲不標、輕聲以 `˙` 放在
//! 音節尾，與詞庫和個人學習檔的既有格式相同。

pub mod fuzzy;
pub mod han;
pub mod input;
pub mod keyboard;
pub mod pinyin;
pub mod select;
pub mod syllable;

pub use han::is_han;
pub use input::{Exact, Frontend, Hyp, Lattice, ZhInput, ZhInputEvent};
pub use keyboard::{Keyboard, LayoutKind};
pub use pinyin::{from_pinyin, inventory, legal_prefix};
pub use syllable::{class_of, Class, Composer, Syllable};

use thiserror::Error;

/// 注音前端的錯誤。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ZhError {
    /// 字串不符合注音音節的結構（各類符號至多一個、順序正確、聲調在尾）。
    #[error("不是合法的注音音節：{0}")]
    InvalidSyllable(String),
    /// 結構合法，但不在國語音節表內。
    #[error("音節不在音節表內：{0}")]
    IllegalSyllable(String),
    /// 拼音字串含有非字母。
    #[error("拼音 {0:?} 含非字母")]
    NonLetter(String),
    /// 拼音的韻母無法辨識。
    #[error("不認得的韻母 {0:?}")]
    UnknownFinal(String),
    /// 有聲母但缺少韻母。
    #[error("拼音 {0:?} 缺少韻母")]
    MissingFinal(String),
    /// 鍵盤排列的名稱無法辨識。
    #[error("未知的鍵盤排列 {0:?}")]
    UnknownLayout(String),
    /// 音節含有這個排列沒有對應按鍵的符號。
    #[error("鍵盤排列沒有 {0:?} 的按鍵")]
    UnmappedSymbol(char),
}
