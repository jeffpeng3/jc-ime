//! 讀音正規化。
//!
//! Mozc 的讀音是片假名；pack v1 的讀音欄位只接受平假名、`ー` 與踊り字。
//! 轉換規則與 Go 參考實作 `jaimport.NormalizeReading` 一致：半形空白忽略，
//! 其他空白與非かな字元視為不可用讀音。

use std::fmt;

/// 讀音無法轉成 pack 可用的平假名。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadingError {
    /// 空讀音。
    Empty,
    /// 半形空白以外的空白字元。
    Whitespace(char),
    /// 不是かな的字元。
    NonKana(char),
}

impl fmt::Display for ReadingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadingError::Empty => write!(f, "讀音不可為空"),
            ReadingError::Whitespace(ch) => {
                write!(f, "讀音含有不合法的空白字元 {ch:?}")
            }
            ReadingError::NonKana(ch) => {
                write!(f, "讀音含有非かな字元 {ch:?}")
            }
        }
    }
}

impl std::error::Error for ReadingError {}

/// 把片假名讀音轉成平假名，並驗證只含 pack 可接受的かな。
pub fn normalize_reading(raw: &str) -> Result<String, ReadingError> {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_whitespace() {
            if ch == ' ' {
                continue;
            }
            return Err(ReadingError::Whitespace(ch));
        }
        let ch = to_hiragana(ch);
        if !is_pack_kana(ch) {
            return Err(ReadingError::NonKana(ch));
        }
        out.push(ch);
    }
    if out.is_empty() {
        return Err(ReadingError::Empty);
    }
    Ok(out)
}

fn to_hiragana(ch: char) -> char {
    let code = ch as u32;
    // ァ..ヶ 與 ヽ..ヾ 平移 0x60 就是對應的平假名。
    if (0x30A1..=0x30F6).contains(&code) || (0x30FD..=0x30FE).contains(&code) {
        if let Some(converted) = char::from_u32(code - 0x60) {
            return converted;
        }
    }
    ch
}

/// pack v1 讀取端接受的かな：平假名本體、踊り字與長音符。
pub fn is_pack_kana(ch: char) -> bool {
    matches!(ch as u32, 0x3041..=0x3096 | 0x309D..=0x309F) || ch == 'ー'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn katakana_becomes_hiragana() {
        assert_eq!(normalize_reading("カタカナ"), Ok("かたかな".to_owned()));
        assert_eq!(
            normalize_reading("ヴァイオリン"),
            Ok("ゔぁいおりん".to_owned())
        );
        assert_eq!(normalize_reading("ヶしょ"), Ok("ゖしょ".to_owned()));
        assert_eq!(normalize_reading("コーヒー"), Ok("こーひー".to_owned()));
    }

    #[test]
    fn halfwidth_space_is_ignored_and_other_whitespace_rejected() {
        assert_eq!(normalize_reading("あ い"), Ok("あい".to_owned()));
        assert_eq!(
            normalize_reading("あ　い"),
            Err(ReadingError::Whitespace('　'))
        );
    }

    #[test]
    fn non_kana_is_rejected() {
        assert_eq!(normalize_reading("abc"), Err(ReadingError::NonKana('a')));
        assert_eq!(
            normalize_reading("あ・い"),
            Err(ReadingError::NonKana('・'))
        );
        assert_eq!(normalize_reading(""), Err(ReadingError::Empty));
        assert_eq!(normalize_reading(" "), Err(ReadingError::Empty));
    }
}
