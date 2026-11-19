//! 實體按鍵到注音符號的排列。
//!
//! 標準（大千）與倚天 41 鍵是一鍵一個符號的靜態表格；許氏排列依目前的組音
//! 狀態決定同一鍵代表哪個符號，這輪只預留 [`LayoutKind::Hsu`]，尚未實作。

use crate::zh::syllable::{class_of, Class, Syllable, TONE1};
use crate::zh::ZhError;

/// 鍵盤排列的種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayoutKind {
    /// 標準（大千）排列，預設值。
    Standard,
    /// 倚天 41 鍵排列。
    ETen,
    /// 許氏排列；已預留，尚未實作。
    Hsu,
}

impl LayoutKind {
    /// 設定檔與命令列使用的名稱。
    pub fn name(self) -> &'static str {
        match self {
            LayoutKind::Standard => "standard",
            LayoutKind::ETen => "eten",
            LayoutKind::Hsu => "hsu",
        }
    }

    /// 依名稱取得排列種類。
    pub fn from_name(name: &str) -> Result<Self, ZhError> {
        match name {
            "standard" => Ok(LayoutKind::Standard),
            "eten" => Ok(LayoutKind::ETen),
            "hsu" => Ok(LayoutKind::Hsu),
            other => Err(ZhError::UnknownLayout(other.to_string())),
        }
    }
}

/// 標準（大千）排列的按鍵表。
///
/// 數字鍵與字母鍵打出聲母、介音與韻母，Shift 組合的符號另外由 [`Keyboard::punct`]
/// 處理。
const STANDARD_KEYS: &[(char, char)] = &[
    ('1', 'ㄅ'),
    ('q', 'ㄆ'),
    ('a', 'ㄇ'),
    ('z', 'ㄈ'),
    ('2', 'ㄉ'),
    ('w', 'ㄊ'),
    ('s', 'ㄋ'),
    ('x', 'ㄌ'),
    ('e', 'ㄍ'),
    ('d', 'ㄎ'),
    ('c', 'ㄏ'),
    ('r', 'ㄐ'),
    ('f', 'ㄑ'),
    ('v', 'ㄒ'),
    ('5', 'ㄓ'),
    ('t', 'ㄔ'),
    ('g', 'ㄕ'),
    ('b', 'ㄖ'),
    ('y', 'ㄗ'),
    ('h', 'ㄘ'),
    ('n', 'ㄙ'),
    ('u', 'ㄧ'),
    ('j', 'ㄨ'),
    ('m', 'ㄩ'),
    ('8', 'ㄚ'),
    ('i', 'ㄛ'),
    ('k', 'ㄜ'),
    (',', 'ㄝ'),
    ('9', 'ㄞ'),
    ('o', 'ㄟ'),
    ('l', 'ㄠ'),
    ('.', 'ㄡ'),
    ('0', 'ㄢ'),
    ('p', 'ㄣ'),
    (';', 'ㄤ'),
    ('/', 'ㄥ'),
    ('-', 'ㄦ'),
    (' ', 'ˉ'),
    ('6', 'ˊ'),
    ('3', 'ˇ'),
    ('4', 'ˋ'),
    ('7', '˙'),
];

/// 標準排列的標點。
///
/// 注音用掉了 `, . / ; -` 與數字，標點改由 Shift 與其餘按鍵產生。頓號 `'`
/// 與 `\` 都可以打；`^` 打刪節號、`_` 打破折號。
const STANDARD_PUNCT: &[(char, &str)] = &[
    ('<', "，"),
    ('>', "。"),
    ('?', "？"),
    (':', "："),
    ('"', "；"),
    ('!', "！"),
    ('~', "～"),
    ('[', "「"),
    (']', "」"),
    ('{', "『"),
    ('}', "』"),
    ('\'', "、"),
    ('\\', "、"),
    ('(', "（"),
    (')', "）"),
    ('&', "＆"),
    ('^', "……"),
    ('_', "——"),
];

/// 倚天 41 鍵排列的按鍵表。
///
/// 一鍵一個符號，聲調在 1（輕聲）、2、3、4，一聲是空白；5、6 不是注音鍵。
/// 對照與新酷音（libchewing et.rs）、小麥注音相同。
const ETEN_KEYS: &[(char, char)] = &[
    ('b', 'ㄅ'),
    ('p', 'ㄆ'),
    ('m', 'ㄇ'),
    ('f', 'ㄈ'),
    ('d', 'ㄉ'),
    ('t', 'ㄊ'),
    ('n', 'ㄋ'),
    ('l', 'ㄌ'),
    ('v', 'ㄍ'),
    ('k', 'ㄎ'),
    ('h', 'ㄏ'),
    ('g', 'ㄐ'),
    ('7', 'ㄑ'),
    ('c', 'ㄒ'),
    (',', 'ㄓ'),
    ('.', 'ㄔ'),
    ('/', 'ㄕ'),
    ('j', 'ㄖ'),
    (';', 'ㄗ'),
    ('\'', 'ㄘ'),
    ('s', 'ㄙ'),
    ('e', 'ㄧ'),
    ('x', 'ㄨ'),
    ('u', 'ㄩ'),
    ('a', 'ㄚ'),
    ('o', 'ㄛ'),
    ('r', 'ㄜ'),
    ('w', 'ㄝ'),
    ('i', 'ㄞ'),
    ('q', 'ㄟ'),
    ('z', 'ㄠ'),
    ('y', 'ㄡ'),
    ('8', 'ㄢ'),
    ('9', 'ㄣ'),
    ('0', 'ㄤ'),
    ('-', 'ㄥ'),
    ('=', 'ㄦ'),
    (' ', 'ˉ'),
    ('2', 'ˊ'),
    ('3', 'ˇ'),
    ('4', 'ˋ'),
    ('1', '˙'),
];

/// 倚天排列的標點；`'` 是 ㄘ，頓號只用 `\`。
const ETEN_PUNCT: &[(char, &str)] = &[
    ('<', "，"),
    ('>', "。"),
    ('?', "？"),
    (':', "："),
    ('"', "；"),
    ('!', "！"),
    ('~', "～"),
    ('[', "「"),
    (']', "」"),
    ('{', "『"),
    ('}', "』"),
    ('\\', "、"),
    ('(', "（"),
    (')', "）"),
    ('&', "＆"),
    ('^', "……"),
    ('_', "——"),
];

/// 一種鍵盤排列。
///
/// 只做字元到符號的對照，不含任何狀態。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Keyboard {
    kind: LayoutKind,
}

impl Keyboard {
    /// 標準（大千）排列。
    pub fn standard() -> Self {
        Self {
            kind: LayoutKind::Standard,
        }
    }

    /// 倚天 41 鍵排列。
    pub fn eten() -> Self {
        Self {
            kind: LayoutKind::ETen,
        }
    }

    /// 依種類建立排列；許氏排列尚未實作。
    pub fn new(kind: LayoutKind) -> Result<Self, ZhError> {
        match kind {
            LayoutKind::Standard | LayoutKind::ETen => Ok(Self { kind }),
            LayoutKind::Hsu => Err(ZhError::UnsupportedLayout(LayoutKind::Hsu.name())),
        }
    }

    /// 依設定檔的名稱建立排列。
    pub fn by_name(name: &str) -> Result<Self, ZhError> {
        Self::new(LayoutKind::from_name(name)?)
    }

    /// 排列種類。
    pub fn kind(self) -> LayoutKind {
        self.kind
    }

    /// 排列名稱。
    pub fn name(self) -> &'static str {
        self.kind.name()
    }

    fn table(self) -> &'static [(char, char)] {
        match self.kind {
            LayoutKind::ETen => ETEN_KEYS,
            _ => STANDARD_KEYS,
        }
    }

    fn punct_table(self) -> &'static [(char, &'static str)] {
        match self.kind {
            LayoutKind::ETen => ETEN_PUNCT,
            _ => STANDARD_PUNCT,
        }
    }

    /// 回傳按鍵代表的注音符號（聲調含一聲記號）。
    pub fn symbol(self, key: char) -> Option<char> {
        self.table()
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, sym)| *sym)
    }

    /// 回傳按鍵對應的全形標點。
    pub fn punct(self, key: char) -> Option<&'static str> {
        self.punct_table()
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, p)| *p)
    }

    /// 回傳打出某音節所需的按鍵序列。
    ///
    /// 沒有聲調的音節結尾補一聲鍵（空白），與前身一致。
    pub fn keys(self, syl: &Syllable) -> Result<String, ZhError> {
        let mut out = String::with_capacity(5);
        let mut has_tone = false;
        for ch in syl.as_str().chars() {
            has_tone |= class_of(ch) == Class::Tone;
            out.push(self.key_of(ch)?);
        }
        if !has_tone {
            out.push(self.key_of(TONE1)?);
        }
        Ok(out)
    }

    fn key_of(self, sym: char) -> Result<char, ZhError> {
        self.table()
            .iter()
            .find(|(_, s)| *s == sym)
            .map(|(k, _)| *k)
            .ok_or(ZhError::UnmappedSymbol(sym))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zh::syllable::Composer;

    fn compose(keys: &str, kb: Keyboard) -> Syllable {
        let mut c = Composer::new();
        let mut done = None;
        for k in keys.chars() {
            let sym = kb.symbol(k).unwrap_or_else(|| panic!("按鍵 {k:?} 無對應"));
            if let Some(syl) = c.add(sym) {
                done = Some(syl);
            }
        }
        done.expect("按鍵序列應完成一個音節")
    }

    #[test]
    fn test_standard_round_trip() {
        let cases: &[(&str, &str)] = &[
            ("ㄑㄧㄣ", "fup "),
            ("ㄗˇ", "y3"),
            ("ㄐㄧㄥˇ", "ru/3"),
            ("ㄉㄨㄥˋ", "2j/4"),
            ("ㄇㄚ˙", "a87"),
            ("ㄔ", "t "),
        ];
        for &(syl, keys) in cases {
            let syl = Syllable::from_raw(syl);
            assert_eq!(
                Keyboard::standard().keys(&syl),
                Ok(keys.to_string()),
                "Keys({syl})"
            );
            assert_eq!(compose(keys, Keyboard::standard()), syl);
        }
    }

    #[test]
    fn test_standard_punct() {
        let cases: &[(char, &str)] = &[
            ('<', "，"),
            ('>', "。"),
            ('?', "？"),
            ('\'', "、"),
            ('\\', "、"),
            ('&', "＆"),
            ('^', "……"),
            ('_', "——"),
        ];
        for &(key, want) in cases {
            assert_eq!(Keyboard::standard().punct(key), Some(want));
        }
        assert_eq!(Keyboard::standard().punct('@'), None);
    }

    /// 回報者提供的倚天對照表。
    const ETEN_TABLE: &str = "bㄅpㄆmㄇfㄈdㄉtㄊnㄋlㄌvㄍkㄎhㄏgㄐ7ㄑcㄒ,ㄓ.ㄔ/ㄕjㄖ;ㄗ'ㄘsㄙeㄧxㄨuㄩaㄚoㄛrㄜwㄝiㄞqㄟzㄠyㄡ8ㄢ9ㄣ0ㄤ-ㄥ=ㄦ ˉ2ˊ3ˇ4ˋ1˙";

    #[test]
    fn test_eten_table() {
        let pairs: Vec<char> = ETEN_TABLE.chars().collect();
        assert_eq!(pairs.len(), 84, "對照表應有 42 筆");
        let mut seen = std::collections::HashSet::new();
        for pair in pairs.chunks(2) {
            assert_eq!(
                Keyboard::eten().symbol(pair[0]),
                Some(pair[1]),
                "倚天 Symbol({:?})",
                pair[0]
            );
            seen.insert(pair[1]);
        }
        assert_eq!(seen.len(), 42, "應涵蓋 37 個注音與 5 個聲調");
        for key in "56`[]\\".chars() {
            assert_eq!(Keyboard::eten().symbol(key), None, "{key:?} 不是注音鍵");
        }
    }

    #[test]
    fn test_eten_round_trip() {
        let cases: &[(&str, &str)] = &[
            ("ㄑㄧㄣ", "7e9 "),
            ("ㄗˇ", ";3"),
            ("ㄐㄧㄥˇ", "ge-3"),
            ("ㄉㄨㄥˋ", "dx-4"),
            ("ㄇㄚ˙", "ma1"),
            ("ㄔ", ". "),
            ("ㄘˋ", "'4"),
            ("ㄦˊ", "=2"),
            ("ㄕˋ", "/4"),
            ("ㄓ", ", "),
            ("ㄨㄛˇ", "xo3"),
        ];
        for &(syl, keys) in cases {
            let syl = Syllable::from_raw(syl);
            assert_eq!(Keyboard::eten().keys(&syl), Ok(keys.to_string()));
            assert_eq!(compose(keys, Keyboard::eten()), syl);
        }
    }

    #[test]
    fn test_eten_punct() {
        let cases: &[(char, &str)] = &[
            ('<', "，"),
            ('>', "。"),
            ('?', "？"),
            (':', "："),
            ('"', "；"),
            ('\\', "、"),
            ('[', "「"),
            (']', "」"),
            ('&', "＆"),
            ('^', "……"),
            ('_', "——"),
        ];
        for &(key, want) in cases {
            assert_eq!(Keyboard::eten().punct(key), Some(want));
        }
        assert_eq!(Keyboard::eten().punct('\''), None, "倚天的 ' 是 ㄘ");

        // 標點不可與注音鍵重疊，否則那個標點永遠打不出來。
        for kb in [Keyboard::standard(), Keyboard::eten()] {
            for key in "<>?:\"!~[]{}\\()&'^_".chars() {
                if kb.punct(key).is_some() {
                    assert_eq!(kb.symbol(key), None, "{} 的 {key:?}", kb.name());
                }
            }
        }
    }

    #[test]
    fn test_by_name() {
        assert_eq!(Keyboard::by_name("standard"), Ok(Keyboard::standard()));
        assert_eq!(Keyboard::by_name("eten"), Ok(Keyboard::eten()));
        assert_eq!(
            Keyboard::by_name("hsu"),
            Err(ZhError::UnsupportedLayout("hsu"))
        );
        assert_eq!(
            Keyboard::by_name("eten26"),
            Err(ZhError::UnknownLayout("eten26".to_string()))
        );
        assert_eq!(LayoutKind::from_name("standard"), Ok(LayoutKind::Standard));
    }
}
