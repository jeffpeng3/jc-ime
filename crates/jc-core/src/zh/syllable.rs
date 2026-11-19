//! 注音符號、音節結構與逐符號組音的狀態機。

use std::fmt;

use crate::reading::Unit;
use crate::zh::ZhError;

/// 聲母字元表。
pub const INITIALS: &str = "ㄅㄆㄇㄈㄉㄊㄋㄌㄍㄎㄏㄐㄑㄒㄓㄔㄕㄖㄗㄘㄙ";
/// 介音字元表。
pub const MEDIALS: &str = "ㄧㄨㄩ";
/// 韻母字元表。
pub const FINALS: &str = "ㄚㄛㄜㄝㄞㄟㄠㄡㄢㄣㄤㄥㄦ";
/// 一聲的內部記號；正規字串中不出現。
pub const TONE1: char = 'ˉ';
/// 預留給「沒打聲調」音節的內部記號。
///
/// 它接在注音之後，表示聲調由解碼決定；不進詞庫、學習或任何儲存。
pub const UNTONED_MARK: char = '?';

const TONES: &str = "ˉˊˇˋ˙";

/// 注音符號在音節中的位置類別。
///
/// 數值順序就是正規字串的順序：聲母 < 介音 < 韻母 < 聲調。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    /// 不是注音符號。
    None,
    /// 聲母。
    Initial,
    /// 介音。
    Medial,
    /// 韻母。
    Final,
    /// 聲調。
    Tone,
}

/// 回傳符號的類別；不是注音符號時回傳 [`Class::None`]。
pub fn class_of(ch: char) -> Class {
    if INITIALS.contains(ch) {
        Class::Initial
    } else if MEDIALS.contains(ch) {
        Class::Medial
    } else if FINALS.contains(ch) {
        Class::Final
    } else if TONES.contains(ch) {
        Class::Tone
    } else {
        Class::None
    }
}

/// 一個完整（含聲調）的注音音節。
///
/// 內部保存正規字串形式的音節，例如 `ㄓㄨˋ`、`ㄇㄜ˙`、`ㄔ`。建立時用
/// [`Syllable::parse`] 驗證結構，[`Syllable::from_raw`] 只給內部記號使用。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Syllable(String);

impl Syllable {
    /// 解析並驗證一個注音音節。
    ///
    /// 只檢查結構：各類符號至多一個、順序正確、至少一個發音符號、聲調在尾。
    pub fn parse(s: &str) -> Result<Self, ZhError> {
        let syl = Self(s.to_string());
        if syl.valid() {
            Ok(syl)
        } else {
            Err(ZhError::InvalidSyllable(s.to_string()))
        }
    }

    /// 不驗證結構直接建立音節。
    ///
    /// 供內部記號（沒打聲調的音節）與已驗證過的反序列化使用。
    pub fn from_raw(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// 音節的字串形式。
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 檢查音節結構。
    ///
    /// 這是結構檢查，不是「國語實際存在哪些音節」的清單；後者用
    /// [`Syllable::legal`]。
    pub fn valid(&self) -> bool {
        let mut last = Class::None;
        let mut phonetic = 0;
        for ch in self.0.chars() {
            let class = class_of(ch);
            if class == Class::None || class <= last || ch == TONE1 {
                return false;
            }
            if class != Class::Tone {
                phonetic += 1;
            }
            last = class;
        }
        phonetic > 0
    }

    /// 回報是否為國語實際存在的音節。
    ///
    /// 結構合法但不在音節表內的組合（例如 `ㄅㄩ`）會被拒絕。
    pub fn legal(&self) -> bool {
        self.valid() && crate::zh::pinyin::is_legal_base(self.base().as_str())
    }

    /// 去掉聲調符號的音節；沒打聲調的音節會去掉 [`UNTONED_MARK`]。
    pub fn base(&self) -> Self {
        if self.untoned() {
            let mut s = self.0.clone();
            s.pop();
            return Self(s);
        }
        let mut chars: Vec<char> = self.0.chars().collect();
        if let Some(&last) = chars.last() {
            if class_of(last) == Class::Tone {
                chars.pop();
            }
        }
        Self(chars.into_iter().collect())
    }

    /// 回傳聲調：1 到 4，輕聲為 5；沒打聲調的音節為 0。
    pub fn tone(&self) -> u8 {
        if self.untoned() {
            return 0;
        }
        match self.0.chars().last() {
            Some('ˊ') => 2,
            Some('ˇ') => 3,
            Some('ˋ') => 4,
            Some('˙') => 5,
            _ => 1,
        }
    }

    /// 回傳同一個音節換成聲調 `tone`（1 到 5）。
    ///
    /// 一聲不標，範圍外的值視同換成一聲的基底。
    pub fn with_tone(&self, tone: u8) -> Self {
        let mut s = self.base().0;
        match tone {
            2 => s.push('ˊ'),
            3 => s.push('ˇ'),
            4 => s.push('ˋ'),
            5 => s.push('˙'),
            _ => {}
        }
        Self(s)
    }

    /// 回報是否為「沒打聲調」的音節（結構合法、結尾是 [`UNTONED_MARK`]）。
    pub fn untoned(&self) -> bool {
        let Some(stripped) = self.0.strip_suffix(UNTONED_MARK) else {
            return false;
        };
        let base = Self(stripped.to_string());
        !stripped.is_empty() && base.valid() && base.tone() == 1 && !stripped.contains(TONE1)
    }

    /// 回傳沒打聲調的音節在詞庫裡可能的五個聲調（一、二、三、四、輕聲）。
    ///
    /// 其他音節原樣回傳只含自己的切片。
    pub fn tones(&self) -> Vec<Self> {
        if !self.untoned() {
            return vec![self.clone()];
        }
        let b = self.base();
        vec![
            b.clone(),
            b.with_tone(2),
            b.with_tone(3),
            b.with_tone(4),
            b.with_tone(5),
        ]
    }

    /// 音節在畫面上的樣子：沒打聲調的音節不顯示記號。
    pub fn text(&self) -> &str {
        if self.untoned() {
            &self.0[..self.0.len() - UNTONED_MARK.len_utf8()]
        } else {
            &self.0
        }
    }

    /// 回傳沒打聲調的音節對應的核心讀音單位。
    pub fn reading_unit(&self) -> Unit {
        Unit::new(self.0.clone())
    }
}

impl fmt::Display for Syllable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for Syllable {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// 回傳沒打聲調的音節：去掉 `base` 的聲調再加上 [`UNTONED_MARK`]。
pub fn untone(base: &Syllable) -> Syllable {
    Syllable(format!("{}{}", base.base(), UNTONED_MARK))
}

/// 回報音節序列是否含有沒打聲調的音節。
pub fn has_untoned(syls: &[Syllable]) -> bool {
    syls.iter().any(Syllable::untoned)
}

/// 把以空白分隔的讀音字串切成音節。
///
/// 不做結構檢查，與前身一致：呼叫端手上的字串可能來自半成品輸入。
pub fn parse_reading(s: &str) -> Vec<Syllable> {
    s.split_whitespace()
        .map(|f| Syllable::from_raw(f.to_string()))
        .collect()
}

/// 把音節序列串成可當 map 鍵的字串。
pub fn key(syls: &[Syllable]) -> String {
    syls.iter()
        .map(Syllable::as_str)
        .collect::<Vec<_>>()
        .join(" ")
}

/// 逐符號組出一個音節。
///
/// 同類別的符號後者取代前者，符合一般注音輸入習慣；聲調完成音節並清空。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Composer {
    slot: [Option<char>; 3],
    keys: String,
}

impl Composer {
    /// 建立空的組音器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 回傳打出目前符號的按鍵；沒有記錄時為空字串。
    pub fn keys(&self) -> &str {
        &self.keys
    }

    /// 記下打出目前符號的按鍵。
    pub fn set_keys(&mut self, keys: impl Into<String>) {
        self.keys = keys.into();
    }

    /// 把某類別目前的符號換成 `ch`，輸入順序不變；該類別沒有符號時不動作。
    pub fn replace(&mut self, class: Class, ch: char) {
        if let Some(slot) = slot_mut(&mut self.slot, class) {
            if slot.is_some() {
                *slot = Some(ch);
                self.keys.clear();
            }
        }
    }

    /// 移除某類別的符號。
    pub fn remove(&mut self, class: Class) {
        if let Some(slot) = slot_mut(&mut self.slot, class) {
            if slot.is_some() {
                *slot = None;
                self.keys.clear();
            }
        }
    }

    /// 回報目前是否沒有任何符號。
    pub fn empty(&self) -> bool {
        self.slot.iter().all(Option::is_none)
    }

    /// 回傳尚未完成的符號，依正規順序。
    pub fn string(&self) -> String {
        self.slot.iter().flatten().collect()
    }

    /// 回傳目前在該類別（聲母、介音、韻母）的符號；沒有時回傳 `None`。
    pub fn slot(&self, class: Class) -> Option<char> {
        match class {
            Class::Initial => self.slot[0],
            Class::Medial => self.slot[1],
            Class::Final => self.slot[2],
            _ => None,
        }
    }

    /// 加入一個符號；符號是聲調且已有發音符號時，回傳完成的音節並清空。
    pub fn add(&mut self, ch: char) -> Option<Syllable> {
        match class_of(ch) {
            Class::None => None,
            Class::Tone => {
                if self.empty() {
                    return None;
                }
                let mut s = self.string();
                if ch != TONE1 {
                    s.push(ch);
                }
                self.reset();
                Some(Syllable(s))
            }
            class => {
                self.slot[class as usize - 1] = Some(ch);
                self.keys.clear();
                None
            }
        }
    }

    /// 移除畫面上最後一個符號（韻母 → 介音 → 聲母）；沒有符號時回傳 `false`。
    ///
    /// 不照聲母、介音、韻母的順序打時，退掉的是看得到的最後一個。
    pub fn backspace(&mut self) -> bool {
        for i in (0..self.slot.len()).rev() {
            if self.slot[i].is_some() {
                self.slot[i] = None;
                self.keys.clear();
                return true;
            }
        }
        false
    }

    /// 清空。
    pub fn reset(&mut self) {
        self.slot = [None; 3];
        self.keys.clear();
    }
}

fn slot_mut(slot: &mut [Option<char>; 3], class: Class) -> Option<&mut Option<char>> {
    match class {
        Class::Initial => Some(&mut slot[0]),
        Class::Medial => Some(&mut slot[1]),
        Class::Final => Some(&mut slot[2]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid() {
        for s in ["ㄓㄨˋ", "ㄇㄜ˙", "ㄔ", "ㄧ", "ㄩㄢˊ", "ㄦˋ", "ㄐㄧㄥˇ"] {
            assert!(Syllable::from_raw(s).valid(), "{s} 應為合法");
        }
        for s in ["", "ˋ", "ㄨㄓ", "ㄓㄓ", "ㄓˋㄨ", "ㄓabc", "ㄓˉ"] {
            assert!(!Syllable::from_raw(s).valid(), "{s} 應為不合法");
        }
        assert!(Syllable::parse("ㄓㄨˋ").is_ok());
        assert_eq!(
            Syllable::parse("ㄓabc"),
            Err(ZhError::InvalidSyllable("ㄓabc".to_string()))
        );
    }

    #[test]
    fn test_composer() {
        let mut c = Composer::new();
        for ch in "ㄐㄧㄥ".chars() {
            assert_eq!(c.add(ch), None, "尚未輸入聲調不應完成");
        }
        assert_eq!(
            c.add('ˇ'),
            Some(Syllable::from_raw("ㄐㄧㄥˇ")),
            "聲調應完成音節"
        );
        assert!(c.empty(), "完成後應清空");

        // 同類別取代、順序無關、一聲不標。
        for ch in "ㄧㄑㄐㄣ".chars() {
            c.add(ch);
        }
        assert_eq!(c.string(), "ㄐㄧㄣ");
        assert_eq!(c.add(TONE1), Some(Syllable::from_raw("ㄐㄧㄣ")));

        // 退格依畫面順序：照順序打時就是最後按的那個。
        c.add('ㄉ');
        c.add('ㄨ');
        assert!(c.backspace());
        assert_eq!(c.string(), "ㄉ");
        assert_eq!(c.add('ˋ'), Some(Syllable::from_raw("ㄉˋ")));
        assert_eq!(c.add('ˋ'), None, "空的組音器不應接受聲調");
    }

    #[test]
    fn test_composer_edits_for_dynamic_layouts() {
        let mut c = Composer::new();
        c.add('ㄓ');
        c.add('ㄧ');
        c.set_keys("je");
        let mut d = c.clone();
        d.replace(Class::Initial, 'ㄐ');
        assert_eq!(d.string(), "ㄐㄧ");
        assert_eq!(c.string(), "ㄓㄧ");
        assert_eq!(d.keys(), "");
        assert_eq!(c.keys(), "je");

        d.backspace();
        assert_eq!(d.string(), "ㄐ", "退格後應剩 ㄐ");
        assert_eq!(c.string(), "ㄓㄧ", "原本不受影響");

        let mut e = c.clone();
        e.remove(Class::Initial);
        e.add('ㄢ');
        assert_eq!(e.string(), "ㄧㄢ");
        assert_eq!(c.string(), "ㄓㄧ");

        e.backspace();
        assert_eq!(e.string(), "ㄧ");
        e.replace(Class::Final, 'ㄣ');
        assert_eq!(e.string(), "ㄧ", "Replace 不該補上沒有的符號");
    }

    #[test]
    fn test_composer_backspace_follows_display_order() {
        let mut c = Composer::new();
        for ch in "ㄨㄢㄓ".chars() {
            c.add(ch);
        }
        assert_eq!(c.string(), "ㄓㄨㄢ");
        for want in ["ㄓㄨ", "ㄓ", ""] {
            assert!(c.backspace());
            assert_eq!(c.string(), want);
        }
        assert!(c.empty());
        assert!(!c.backspace(), "空組音器沒有東西可退");

        for ch in "ㄢㄉ".chars() {
            c.add(ch);
        }
        c.backspace();
        c.add('ㄤ');
        assert_eq!(c.string(), "ㄉㄤ");
    }

    #[test]
    fn test_parse_reading_and_key() {
        let syls = parse_reading("ㄓㄨˋ  ㄧ ㄉㄧㄢˇ ");
        assert_eq!(syls.len(), 3);
        assert_eq!(syls[0], Syllable::from_raw("ㄓㄨˋ"));
        assert_eq!(key(&syls), "ㄓㄨˋ ㄧ ㄉㄧㄢˇ");
        assert_eq!(key(&[]), "");
    }

    #[test]
    fn test_untone() {
        let y = untone(&Syllable::from_raw("ㄋㄧ"));
        assert!(y.untoned());
        assert!(!y.valid());
        assert!(!y.legal());
        assert_eq!(y.tone(), 0);
        assert_eq!(y.base(), Syllable::from_raw("ㄋㄧ"));
        assert_eq!(y.text(), "ㄋㄧ");
        assert_eq!(untone(&Syllable::from_raw("ㄋㄧˇ")), y);

        let want = ["ㄋㄧ", "ㄋㄧˊ", "ㄋㄧˇ", "ㄋㄧˋ", "ㄋㄧ˙"].map(Syllable::from_raw);
        assert_eq!(y.tones(), want.to_vec());

        for s in ["ㄋㄧ", "ㄋㄧˇ", "?", "ˇ?", "ㄋㄧˇ?", ""] {
            let syl = Syllable::from_raw(s);
            assert!(!syl.untoned(), "{s:?} 不是沒打聲調的音節");
            if !s.is_empty() {
                assert_eq!(syl.tones(), vec![syl.clone()], "{s:?} 的 Tones 只有自己");
            }
        }
        assert_eq!(Syllable::from_raw("?").base(), Syllable::from_raw("?"));
        assert_eq!(Syllable::from_raw("?").text(), "?");

        assert!(has_untoned(&[Syllable::from_raw("ㄨㄛˇ"), y.clone()]));
        assert!(!has_untoned(&[
            Syllable::from_raw("ㄨㄛˇ"),
            Syllable::from_raw("ㄋㄧ")
        ]));
    }

    #[test]
    fn test_display_and_class() {
        assert_eq!(Syllable::from_raw("ㄓㄨˋ").to_string(), "ㄓㄨˋ");
        assert_eq!(class_of('ㄅ'), Class::Initial);
        assert_eq!(class_of('ㄧ'), Class::Medial);
        assert_eq!(class_of('ㄚ'), Class::Final);
        assert_eq!(class_of('ˋ'), Class::Tone);
        assert_eq!(class_of('x'), Class::None);
        let mut c = Composer::new();
        c.add('ㄓ');
        c.add('ㄨ');
        c.add('ㄚ');
        assert_eq!(c.slot(Class::Initial), Some('ㄓ'));
        assert_eq!(c.slot(Class::Medial), Some('ㄨ'));
        assert_eq!(c.slot(Class::Final), Some('ㄚ'));
        assert_eq!(c.slot(Class::Tone), None);
    }
}
