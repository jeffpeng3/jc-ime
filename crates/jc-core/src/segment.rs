//! 組字區片段與書寫系統，以及混合字串的分段。
//!
//! 分段器把原始輸入切成注音、かな、漢字、Latin、數字與符號等片段。片段只帶
//! 書寫系統與模式標籤，不保存任何全域語言狀態；同一次輸入可以同時含多個語言。

use unicode_categories::UnicodeCategories;
use unicode_script::{Script as UnicodeScript, UnicodeScript as _};

use crate::lang::Mode;
use crate::zh::han::is_han;
use crate::zh::syllable::{class_of, Class};

/// 片段使用的書寫系統。
///
/// 只用來描述片段本身的字面種類，不單獨決定語言；例如 [`Script::Latin`]
/// 同時可能是英文或日文羅馬字。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Script {
    /// 注音符號。
    Bopomofo,
    /// 平假名、片假名、長音符與踊り字。
    Kana,
    /// 漢字。
    Han,
    /// 拉丁字母。
    Latin,
    /// 數字。
    Digit,
    /// 標點、空白與其他符號。
    Symbol,
    /// 拉丁字母與數字混合的片段，例如 `abc123`。
    Mixed,
    /// 其他無法歸類的字元（不屬於本專案語言範圍的字母與數字）。
    Other,
}

impl Script {
    /// 穩定的英文名稱，可寫進設定與測試資料。
    pub fn as_str(self) -> &'static str {
        match self {
            Script::Bopomofo => "bopomofo",
            Script::Kana => "kana",
            Script::Han => "han",
            Script::Latin => "latin",
            Script::Digit => "digit",
            Script::Symbol => "symbol",
            Script::Mixed => "mixed",
            Script::Other => "other",
        }
    }

    /// 依名稱取得書寫系統。
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "bopomofo" => Script::Bopomofo,
            "kana" => Script::Kana,
            "han" => Script::Han,
            "latin" => Script::Latin,
            "digit" => Script::Digit,
            "symbol" => Script::Symbol,
            "mixed" => Script::Mixed,
            "other" => Script::Other,
            _ => return None,
        })
    }

    /// 回報這個書寫系統是否為語言的強訊號。
    ///
    /// 注音與假名只能來自特定語言，因此是強訊號；其餘都不是。
    pub fn is_strong(self) -> bool {
        matches!(self, Script::Bopomofo | Script::Kana)
    }

    /// 這個書寫系統能確定的輸入模式；不是強訊號時回傳 [`Mode::Mixed`]，
    /// 由呼叫端依候選決定語言。
    pub fn mode(self) -> Mode {
        match self {
            Script::Bopomofo => Mode::Zh,
            Script::Kana => Mode::Ja,
            _ => Mode::Mixed,
        }
    }
}

/// 組字區中的一個片段。
///
/// `start` 與 `end` 是組字字串的 UTF-8 位元組位移，範圍為半開區間
/// `[start, end)`。片段長度一律用 [`Segment::len`] 取得。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// 片段起點（含）。
    pub start: usize,
    /// 片段終點（不含）。
    pub end: usize,
    /// 這個片段所屬的輸入模式。
    pub mode: Mode,
    /// 這個片段的書寫系統。
    pub script: Script,
}

impl Segment {
    /// 建立片段。
    pub fn new(start: usize, end: usize, mode: Mode, script: Script) -> Self {
        Self {
            start,
            end,
            mode,
            script,
        }
    }

    /// 片段長度（位元組）。
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// 片段是否為空。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// 判定單一字元的書寫系統。
///
/// 這是 [`split`] 使用的同一個判定，供測試與診斷使用。
pub fn detect(ch: char) -> Script {
    script_of(ch)
}

/// 把輸入切成依序覆蓋所有位元組的片段。
///
/// 邊界規則：
/// - 相鄰且同書寫系統的字元合併成一個片段；
/// - 相鄰的 Latin 與數字合併成一個混合片段，因為 `abc123` 之類的識別字不能
///   在中間拆成兩種語言候選；
/// - 注音、かな、漢字、Latin、數字與符號之間只要換類就切開；
/// - 空白、標點與其他符號合併成符號片段。
///
/// 空輸入回傳空切片。
pub fn split(input: &str) -> Vec<Segment> {
    Splitter.split(input)
}

/// 預設的腳本分段器。
///
/// 沒有狀態，可以同時給多個 session 使用，也可以重複呼叫；只依輸入字元判定
/// 書寫系統，不讀取主要語言、前文或任何全域模式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Splitter;

impl Splitter {
    /// 建立分段器。
    pub fn new() -> Self {
        Self
    }

    /// 把 `input` 切成依序覆蓋所有位元組的片段。
    pub fn split(&self, input: &str) -> Vec<Segment> {
        let chars: Vec<(usize, char)> = input.char_indices().collect();
        if chars.is_empty() {
            return Vec::new();
        }
        let mut segments = Vec::with_capacity(chars.len());
        let mut i = 0;
        while i < chars.len() {
            let script = script_of(chars[i].1);
            if matches!(script, Script::Latin | Script::Digit) {
                let (end, latin, digit) = scan_alnum(&chars, i);
                let script = match (latin, digit) {
                    (true, true) => Script::Mixed,
                    (false, true) => Script::Digit,
                    _ => Script::Latin,
                };
                segments.push(segment_at(input, &chars, i, end, script));
                i = end;
                continue;
            }
            let mut end = i + 1;
            while end < chars.len() && script_of(chars[end].1) == script {
                end += 1;
            }
            segments.push(segment_at(input, &chars, i, end, script));
            i = end;
        }
        segments
    }
}

/// 從 `start` 起掃過連續的 Latin 與數字，回報結束位置與出現過的種類。
fn scan_alnum(chars: &[(usize, char)], start: usize) -> (usize, bool, bool) {
    let mut end = start;
    let (mut latin, mut digit) = (false, false);
    while end < chars.len() {
        match script_of(chars[end].1) {
            Script::Latin => latin = true,
            Script::Digit => digit = true,
            _ => break,
        }
        end += 1;
    }
    (end, latin, digit)
}

fn segment_at(
    input: &str,
    chars: &[(usize, char)],
    start: usize,
    end: usize,
    script: Script,
) -> Segment {
    let start_byte = chars[start].0;
    let end_byte = chars.get(end).map_or(input.len(), |(b, _)| *b);
    Segment::new(start_byte, end_byte, script.mode(), script)
}

fn script_of(ch: char) -> Script {
    if is_zhuyin(ch) {
        Script::Bopomofo
    } else if is_kana(ch) {
        Script::Kana
    } else if is_han(ch) || matches!(ch, '々' | '〆') {
        Script::Han
    } else if ch.script() == UnicodeScript::Latin {
        Script::Latin
    } else if ch.is_number_decimal_digit() {
        Script::Digit
    } else if ch.is_alphabetic() || ch.is_numeric() {
        // 不在本專案語言範圍內的字母或數字，保留為未判定，不冒充 Latin
        // 或數字，避免產生錯誤的語言候選。
        Script::Other
    } else {
        Script::Symbol
    }
}

fn is_zhuyin(ch: char) -> bool {
    class_of(ch) != Class::None || matches!(ch as u32, 0x3100..=0x312F | 0x31A0..=0x31BF)
}

fn is_kana(ch: char) -> bool {
    matches!(ch as u32,
        0x3041..=0x309F // ひらがな（含濁點與半濁點）
        | 0x30A0..=0x30FF // カタカナ（含長音符與踊り字）
        | 0x31F0..=0x31FF // カタカナ拡張
        | 0xFF66..=0xFF9D // 半角カナ
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 把 rune 索引轉成位元組位移，讓測試期望值與前身的 rune 索引一致。
    fn at(input: &str, start: usize, end: usize) -> (usize, usize) {
        let byte = |n: usize| input.char_indices().nth(n).map_or(input.len(), |(i, _)| i);
        (byte(start), byte(end))
    }

    fn seg(input: &str, runes: (usize, usize), script: Script) -> Segment {
        let (start, end) = at(input, runes.0, runes.1);
        Segment::new(start, end, script.mode(), script)
    }

    fn assert_covers_input(input: &str, segments: &[Segment]) {
        let mut pos = 0;
        for (i, s) in segments.iter().enumerate() {
            assert_eq!(s.start, pos, "segment[{i}] 起點");
            assert!(!s.is_empty(), "segment[{i}] 是空片段");
            pos = s.end;
        }
        assert_eq!(pos, input.len(), "片段沒有覆蓋完整輸入");
    }

    #[test]
    fn segment_length_is_end_minus_start() {
        let s = Segment::new(2, 5, Mode::Ja, Script::Kana);
        assert_eq!(s.len(), 3);
        assert!(!s.is_empty());
        assert_eq!(s.start, 2);
        assert_eq!(s.end, 5);
    }

    #[test]
    fn inverted_range_has_zero_length() {
        let s = Segment::new(7, 3, Mode::Zh, Script::Bopomofo);
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
    }

    #[test]
    fn only_bopomofo_and_kana_are_strong_scripts() {
        assert!(Script::Bopomofo.is_strong());
        assert!(Script::Kana.is_strong());
        for script in [
            Script::Han,
            Script::Latin,
            Script::Digit,
            Script::Symbol,
            Script::Mixed,
            Script::Other,
        ] {
            assert!(!script.is_strong(), "{} 不是強訊號", script.as_str());
        }
    }

    #[test]
    fn test_split_table() {
        struct Case {
            name: &'static str,
            input: &'static str,
            want: Vec<Segment>,
        }
        let cases: Vec<Case> = vec![
            Case {
                name: "empty",
                input: "",
                want: vec![],
            },
            Case {
                name: "single zhuyin syllable",
                input: "ㄓㄨˋ",
                want: vec![seg("ㄓㄨˋ", (0, 3), Script::Bopomofo)],
            },
            Case {
                name: "zhuyin without tone",
                input: "ㄓㄨㄧㄣ",
                want: vec![seg("ㄓㄨㄧㄣ", (0, 4), Script::Bopomofo)],
            },
            Case {
                name: "two zhuyin syllables",
                input: "ㄓㄨˋㄧㄣ",
                want: vec![seg("ㄓㄨˋㄧㄣ", (0, 5), Script::Bopomofo)],
            },
            Case {
                name: "hiragana",
                input: "かな",
                want: vec![seg("かな", (0, 2), Script::Kana)],
            },
            Case {
                name: "katakana with small kana and long mark",
                input: "キャー",
                want: vec![seg("キャー", (0, 3), Script::Kana)],
            },
            Case {
                name: "iteration marks",
                input: "ゝゞヽヾ",
                want: vec![seg("ゝゞヽヾ", (0, 4), Script::Kana)],
            },
            Case {
                name: "halfwidth katakana",
                input: "ｱｲｳｰ",
                want: vec![seg("ｱｲｳｰ", (0, 4), Script::Kana)],
            },
            Case {
                name: "ascii latin",
                input: "kubectl",
                want: vec![seg("kubectl", (0, 7), Script::Latin)],
            },
            Case {
                name: "fullwidth latin",
                input: "ＡＢＣ",
                want: vec![seg("ＡＢＣ", (0, 3), Script::Latin)],
            },
            Case {
                name: "accented latin",
                input: "café",
                want: vec![seg("café", (0, 4), Script::Latin)],
            },
            Case {
                name: "ascii digits",
                input: "123",
                want: vec![seg("123", (0, 3), Script::Digit)],
            },
            Case {
                name: "fullwidth digits",
                input: "１２３",
                want: vec![seg("１２３", (0, 3), Script::Digit)],
            },
            Case {
                name: "latin and digits are one mixed span",
                input: "abc123",
                want: vec![seg("abc123", (0, 6), Script::Mixed)],
            },
            Case {
                name: "alternating latin and digits stay mixed",
                input: "a1b2",
                want: vec![seg("a1b2", (0, 4), Script::Mixed)],
            },
            Case {
                name: "han",
                input: "漢字",
                want: vec![seg("漢字", (0, 2), Script::Han)],
            },
            Case {
                name: "japanese han and kana split",
                input: "日本語です",
                want: vec![
                    seg("日本語です", (0, 3), Script::Han),
                    seg("日本語です", (3, 5), Script::Kana),
                ],
            },
            Case {
                name: "han iteration mark stays with han",
                input: "人々",
                want: vec![seg("人々", (0, 2), Script::Han)],
            },
            Case {
                name: "punctuation",
                input: "，。！？",
                want: vec![seg("，。！？", (0, 4), Script::Symbol)],
            },
            Case {
                name: "whitespace",
                input: " \t\n",
                want: vec![seg(" \t\n", (0, 3), Script::Symbol)],
            },
            Case {
                name: "emoji",
                input: "😀",
                want: vec![seg("😀", (0, 1), Script::Symbol)],
            },
            Case {
                name: "unsupported letters stay unknown",
                input: "Ω",
                want: vec![seg("Ω", (0, 1), Script::Other)],
            },
            Case {
                name: "mixed input",
                input: "a ㄓㄨ b",
                want: vec![
                    seg("a ㄓㄨ b", (0, 1), Script::Latin),
                    seg("a ㄓㄨ b", (1, 2), Script::Symbol),
                    seg("a ㄓㄨ b", (2, 4), Script::Bopomofo),
                    seg("a ㄓㄨ b", (4, 5), Script::Symbol),
                    seg("a ㄓㄨ b", (5, 6), Script::Latin),
                ],
            },
            Case {
                name: "symbol breaks an alphanumeric token",
                input: "abc,123",
                want: vec![
                    seg("abc,123", (0, 3), Script::Latin),
                    seg("abc,123", (3, 4), Script::Symbol),
                    seg("abc,123", (4, 7), Script::Digit),
                ],
            },
            Case {
                name: "digit next to han splits",
                input: "3月",
                want: vec![
                    seg("3月", (0, 1), Script::Digit),
                    seg("3月", (1, 2), Script::Han),
                ],
            },
        ];

        for case in cases {
            let got = Splitter::new().split(case.input);
            assert_eq!(got, case.want, "Split({:?}) [{}]", case.input, case.name);
            assert_covers_input(case.input, &got);
        }
    }

    #[test]
    fn test_split_names_are_stable() {
        let cases: &[(&str, Script)] = &[
            ("ㄅ", Script::Bopomofo),
            ("あ", Script::Kana),
            ("A", Script::Latin),
            ("1", Script::Digit),
            ("漢", Script::Han),
            ("!", Script::Symbol),
        ];
        for &(input, script) in cases {
            let got = Splitter::new().split(input);
            assert_eq!(got.len(), 1, "Split({input:?}) 應只有一個片段");
            assert_eq!(got[0].script, script, "Split({input:?})");
            assert_eq!(got[0].mode, script.mode());
        }
    }

    #[test]
    fn test_detect() {
        let cases: &[(char, Script)] = &[
            ('ㄓ', Script::Bopomofo),
            ('ˋ', Script::Bopomofo),
            ('ㆠ', Script::Bopomofo),
            ('あ', Script::Kana),
            ('ア', Script::Kana),
            ('ｱ', Script::Kana),
            ('ー', Script::Kana),
            ('ゝ', Script::Kana),
            ('ヽ', Script::Kana),
            ('日', Script::Han),
            ('々', Script::Han),
            ('〆', Script::Han),
            ('a', Script::Latin),
            ('Ａ', Script::Latin),
            ('é', Script::Latin),
            ('1', Script::Digit),
            ('１', Script::Digit),
            (' ', Script::Symbol),
            ('。', Script::Symbol),
            ('😀', Script::Symbol),
            ('Ω', Script::Other),
            ('①', Script::Other),
        ];
        for &(ch, want) in cases {
            assert_eq!(detect(ch), want, "detect({ch:?})");
        }
    }

    #[test]
    fn test_split_constructors_agree() {
        let input = "abc123 ㄓㄨˋ かな";
        let want = Splitter::new().split(input);
        assert_eq!(Splitter.split(input), want);
        assert_eq!(split(input), want);
        assert_eq!(Splitter.split(input), want);
    }

    #[test]
    fn test_split_does_not_keep_state() {
        let splitter = Splitter::new();
        let mut first = splitter.split("a1ㄓ");
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].script, Script::Mixed);
        assert_eq!(first[1].script, Script::Bopomofo);

        let second = splitter.split("かな");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].script, Script::Kana);
        assert_eq!(second[0].mode, Mode::Ja);

        first[0].script = Script::Symbol;
        let third = splitter.split("a1ㄓ");
        assert_eq!(third[0].script, Script::Mixed, "修改回傳片段影響後續結果");
    }

    /// 前身的 FuzzSplit 種子改成固定案例：每個輸入都要被片段完整覆蓋。
    #[test]
    fn split_always_covers_input() {
        for input in [
            "",
            "abc123",
            "ㄓㄨˋㄧㄣ",
            "日本語です",
            "a ㄓㄨ b",
            "😀かなＡＢ１２",
            "3月",
            "ㄅ\u{31A0}Ω①é",
        ] {
            assert_covers_input(input, &Splitter::new().split(input));
        }
    }

    #[test]
    fn test_script_names_and_strength() {
        let cases: &[(Script, &str, bool)] = &[
            (Script::Other, "other", false),
            (Script::Bopomofo, "bopomofo", true),
            (Script::Kana, "kana", true),
            (Script::Latin, "latin", false),
            (Script::Digit, "digit", false),
            (Script::Symbol, "symbol", false),
            (Script::Han, "han", false),
            (Script::Mixed, "mixed", false),
        ];
        for &(script, name, strong) in cases {
            assert_eq!(script.as_str(), name);
            assert_eq!(script.is_strong(), strong, "{name}.is_strong()");
            assert_eq!(Script::from_name(name), Some(script));
        }
        assert_eq!(Script::from_name("emoji"), None);
    }
}
