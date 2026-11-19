//! 讀音表示法：核心唯一認得的讀音單位。
//!
//! 注音與日文共用同一種單位，[`crate::session`] 與解碼器不直接依賴注音或日文
//! 模組。注音的一個單位是一個完整音節（例：`ㄓㄨˋ`）；日文的一個單位是一個
//! かな字元（例：`き`、`ゃ`、`ー`），小寫かな、長音符與踊り字都各自算一個單位。
//!
//! 單位只描述讀音，不限制詞面長度：一個詞可以有兩個單位，對應三個字的表面。

use crate::zh::syllable::Syllable;
use crate::zh::Class;

/// 一個讀音位置。
///
/// 這是獨立的型別，不是 [`Syllable`] 的別名：注音與日文都使用它，但這個模組
/// 不依賴任何語言的轉換邏輯。[`Unit::valid`] 同時接受合法的注音音節與かな，
/// 讓兩個語言共用同一個讀音契約。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Unit(String);

impl Unit {
    /// 不驗證內容直接建立讀音單位。
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// 讀音單位的字串形式。
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 回報單位是否為一般形式的單一讀音。
    ///
    /// 接受兩種形式：
    /// - 一個結構合法的注音音節，例如 `ㄓㄨˋ`、`ㄇㄜ˙`；
    /// - 一個かな字元，含小寫かな、長音符、踊り字與半角カナ。
    ///
    /// 空字串、含空白或其他字元都不合法。語言特定的檢查（例如日文詞彙是否
    /// 真的這樣讀）仍由該語言的模組負責。
    pub fn valid(&self) -> bool {
        let s = &self.0;
        if s.is_empty() || s.chars().any(char::is_whitespace) {
            return false;
        }
        if s.chars().any(|ch| zhuyin_class(ch) != Class::None) {
            valid_zhuyin(s)
        } else {
            valid_kana(s)
        }
    }
}

/// [`Unit::valid`] 的函式形式。
pub fn valid(u: &Unit) -> bool {
    u.valid()
}

/// 一串讀音單位，依輸入順序排列。
pub type Sequence = Vec<Unit>;

/// 把讀音序列串成可當 map 鍵與個人資料鍵的字串。
///
/// 對注音序列的輸出與 [`crate::zh::syllable::key`] 完全相同，既有個人學習檔
/// 不必轉換。
pub fn key(seq: &[Unit]) -> String {
    seq.iter().map(Unit::as_str).collect::<Vec<_>>().join(" ")
}

/// 把音節序列轉成核心讀音序列。
pub fn from_syllables(syls: &[Syllable]) -> Sequence {
    syls.iter().map(Syllable::reading_unit).collect()
}

/// 把核心讀音序列轉回音節序列。
///
/// 呼叫端負責確認單位確實是注音音節；這裡不做結構檢查。
pub fn to_syllables(seq: &[Unit]) -> Vec<Syllable> {
    seq.iter()
        .map(|u| Syllable::from_raw(u.as_str().to_string()))
        .collect()
}

/// 注音符號的結構檢查。
///
/// 這裡不能呼叫 [`Syllable`] 的檢查：`zh` 需要讀音單位，反向依賴會形成循環。
/// 規則相同：各類符號至多一個、順序正確、至少一個發音符號、聲調在尾。
fn zhuyin_class(ch: char) -> Class {
    crate::zh::syllable::class_of(ch)
}

fn valid_zhuyin(s: &str) -> bool {
    let mut last = Class::None;
    let mut phonetic = 0;
    for ch in s.chars() {
        let class = zhuyin_class(ch);
        if class == Class::None || class <= last || ch == crate::zh::syllable::TONE1 {
            return false;
        }
        if class != Class::Tone {
            phonetic += 1;
        }
        last = class;
    }
    phonetic > 0
}

/// 檢查單位是否為單一かな字元，可帶組合濁點。
fn valid_kana(s: &str) -> bool {
    let mut base = 0;
    for ch in s.chars() {
        if is_kana_mark(ch) {
            continue;
        }
        if !is_kana_rune(ch) {
            return false;
        }
        base += 1;
        if base > 1 {
            return false;
        }
    }
    base == 1
}

/// 接在かな後面的組合濁點與半濁點。
fn is_kana_mark(ch: char) -> bool {
    matches!(ch, '\u{3099}' | '\u{309A}' | '\u{FF9E}' | '\u{FF9F}')
}

/// 單一かな字元可能出現的字元範圍。
fn is_kana_rune(ch: char) -> bool {
    matches!(ch,
        '\u{3041}'..='\u{3096}' // ひらがな
        | '\u{309D}'..='\u{309F}' // ゝゞゟ
        | '\u{30A1}'..='\u{30FA}' // カタカナ
        | '\u{30FC}'..='\u{30FF}' // ーヽヾヿ
        | '\u{31F0}'..='\u{31FF}' // カタカナ拡張
        | '\u{FF66}'..='\u{FF9D}' // 半角カナ
        | '\u{3005}' | '\u{3006}' | '\u{303B}' // 々〆〻（踊り字）
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key() {
        let cases: &[(Vec<&str>, &str)] = &[
            (vec![], ""),
            (vec!["ㄓㄨˋ"], "ㄓㄨˋ"),
            (vec!["ㄓㄨˋ", "ㄧ", "ㄉㄧㄢˇ"], "ㄓㄨˋ ㄧ ㄉㄧㄢˇ"),
            (vec!["き", "ょ", "う"], "き ょ う"),
            // 可変長：兩個字的詞可以對上四個單位。
            (vec!["よ", "う", "せ", "い"], "よ う せ い"),
        ];
        for (seq, want) in cases {
            let seq: Sequence = seq.iter().map(|s| Unit::new(*s)).collect();
            assert_eq!(key(&seq), *want);
        }
    }

    #[test]
    fn test_unit_valid_kana() {
        for u in [
            "き",
            "ゃ",
            "ー",
            "々",
            "ゝ",
            "ゔ",
            "カ",
            "ヿ",
            "ｷ",
            "ｶﾞ",
            "き\u{3099}",
        ] {
            let unit = Unit::new(u);
            assert!(unit.valid(), "{u:?} 應為合法かな單位");
            assert_eq!(valid(&unit), unit.valid());
        }
    }

    #[test]
    fn test_unit_valid_zhuyin() {
        for u in ["ㄓㄨˋ", "ㄧ", "ㄇㄜ˙", "ㄔ", "ㄐㄩㄥˇ"] {
            assert!(Unit::new(u).valid(), "{u:?} 應為合法注音音節");
        }
    }

    #[test]
    fn test_unit_invalid() {
        for u in [
            "",
            "き ょ",
            "き\tょ",
            "き\n",
            "a",
            "1",
            "。",
            "きょ",
            "ㄉㄨㄥx",
            "ㄅㄆ",
            "ˉ",
        ] {
            assert!(!Unit::new(u).valid(), "{u:?} 應為不合法");
        }
    }

    #[test]
    fn test_reading_key_matches_zhuyin_key() {
        for s in ["", "ㄓㄨˋ", "ㄓㄨˋ ㄧ ㄉㄧㄢˇ", "ㄔ ㄇㄜ˙", "ㄐㄧㄥˇ ㄍㄞˋ"]
        {
            let syls = crate::zh::syllable::parse_reading(s);
            assert_eq!(
                key(&from_syllables(&syls)),
                crate::zh::syllable::key(&syls),
                "{s:?}"
            );
        }
    }

    #[test]
    fn test_reading_units_round_trip() {
        let syls = crate::zh::syllable::parse_reading("ㄓㄨˋ ㄧ ㄉㄧㄢˇ");
        let seq = from_syllables(&syls);
        assert_eq!(seq.len(), syls.len());
        for (unit, syl) in seq.iter().zip(&syls) {
            assert!(unit.valid(), "unit {unit:?} 應為合法");
            assert_eq!(&to_syllables(std::slice::from_ref(unit))[0], syl);
        }
        assert_eq!(to_syllables(&seq), syls);
    }

    #[test]
    fn test_reading_unit_validity_matches_zhuyin() {
        for s in [
            "ㄓㄨˋ",
            "ㄇㄜ˙",
            "ㄔ",
            "ㄐㄩㄥˇ",
            "ㄧ",
            "ㄦˊ",
            "ㄅㄆ",
            "ㄚㄅ",
            "ˉ",
            "ㄓˋˊ",
            "ㄓx",
            "",
            "ㄓ ㄨˋ",
        ] {
            assert_eq!(Unit::new(s).valid(), Syllable::from_raw(s).valid(), "{s:?}");
        }
    }
}
