//! 日文輸入正規化：かな表記與全角／半角轉換。
//!
//! 這裡只放前身 `core/ja/rewrite` 中屬於輸入正規化的部分：把使用者輸入的
//! かな、半角カナ與英數轉成一致的表示法。候選產生（かな表記候選、數字
//! 表記、記号與顔文字）屬於輸出端改寫，不在這個 task 的範圍。
//!
//! 舊假名 `ゐ`、`ゑ`、`を` 與前身一樣保持原樣，不轉成現代假名。

/// 把かな字串轉成平假名。
///
/// 片假名、半角カナ與濁點／半濁點都會一併正規化；非かな字元原樣保留。
pub fn to_hiragana(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for r in to_katakana_runes(s) {
        match r {
            'ァ'..='ヶ' => out.push(char::from_u32(r as u32 - 0x60).unwrap_or(r)),
            'ヽ' => out.push('ゝ'),
            'ヾ' => out.push('ゞ'),
            _ => out.push(r),
        }
    }
    out
}

/// 把かな字串轉成片假名。
///
/// 平假名與半角カナ都會一併正規化；非かな字元原樣保留。
pub fn to_katakana(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for r in to_katakana_runes(s) {
        match r {
            'ぁ'..='ゖ' => out.push(char::from_u32(r as u32 + 0x60).unwrap_or(r)),
            'ゝ' => out.push('ヽ'),
            'ゞ' => out.push('ヾ'),
            _ => out.push(r),
        }
    }
    out
}

/// 把かな字串轉成半角カナ。
///
/// 濁音與半濁音使用半角的濁點／半濁點表示，例如 `がっこう` →
/// `ｶﾞｯｺｳ`。非かな字元原樣保留。
pub fn to_halfwidth_kana(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for r in to_katakana(s).chars() {
        match halfwidth_kana(r) {
            Some(half) => out.push_str(half),
            None => out.push(r),
        }
    }
    out
}

/// 把 ASCII 英數與標點轉成全角。空白轉成全角空格。
pub fn to_fullwidth_latin(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for r in s.chars() {
        match r {
            ' ' => out.push('\u{3000}'),
            '!'..='~' => out.push(char::from_u32(r as u32 + 0xFEE0).unwrap_or(r)),
            _ => out.push(r),
        }
    }
    out
}

/// 把全角英數與標點轉成 ASCII。全角空格轉成半角空白。
pub fn to_halfwidth_latin(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for r in s.chars() {
        match r {
            '\u{3000}' => out.push(' '),
            '！'..='～' => out.push(char::from_u32(r as u32 - 0xFEE0).unwrap_or(r)),
            _ => out.push(r),
        }
    }
    out
}

/// 先把半角カナ展開成片假名，其他字元不動。
fn to_katakana_runes(s: &str) -> Vec<char> {
    let src: Vec<char> = s.chars().collect();
    let mut out = Vec::with_capacity(src.len());
    for r in src {
        match r {
            'ﾞ' => {
                if let Some(last) = out.last_mut() {
                    if let Some(voiced) = katakana_dakuten(*last) {
                        *last = voiced;
                        continue;
                    }
                }
                out.push('゛');
            }
            'ﾟ' => {
                if let Some(last) = out.last_mut() {
                    if let Some(semi) = katakana_handakuten(*last) {
                        *last = semi;
                        continue;
                    }
                }
                out.push('゜');
            }
            _ => match halfwidth_to_katakana(r) {
                Some(full) => out.push(full),
                None => out.push(r),
            },
        }
    }
    out
}

/// 單一字元的半角カナ；濁點由 [`to_katakana_runes`] 組合。
fn halfwidth_to_katakana(r: char) -> Option<char> {
    let full = match r {
        '｡' => '。',
        '｢' => '「',
        '｣' => '」',
        '､' => '、',
        '･' => '・',
        'ｦ' => 'ヲ',
        'ｧ' => 'ァ',
        'ｨ' => 'ィ',
        'ｩ' => 'ゥ',
        'ｪ' => 'ェ',
        'ｫ' => 'ォ',
        'ｬ' => 'ャ',
        'ｭ' => 'ュ',
        'ｮ' => 'ョ',
        'ｯ' => 'ッ',
        'ｰ' => 'ー',
        'ｱ' => 'ア',
        'ｲ' => 'イ',
        'ｳ' => 'ウ',
        'ｴ' => 'エ',
        'ｵ' => 'オ',
        'ｶ' => 'カ',
        'ｷ' => 'キ',
        'ｸ' => 'ク',
        'ｹ' => 'ケ',
        'ｺ' => 'コ',
        'ｻ' => 'サ',
        'ｼ' => 'シ',
        'ｽ' => 'ス',
        'ｾ' => 'セ',
        'ｿ' => 'ソ',
        'ﾀ' => 'タ',
        'ﾁ' => 'チ',
        'ﾂ' => 'ツ',
        'ﾃ' => 'テ',
        'ﾄ' => 'ト',
        'ﾅ' => 'ナ',
        'ﾆ' => 'ニ',
        'ﾇ' => 'ヌ',
        'ﾈ' => 'ネ',
        'ﾉ' => 'ノ',
        'ﾊ' => 'ハ',
        'ﾋ' => 'ヒ',
        'ﾌ' => 'フ',
        'ﾍ' => 'ヘ',
        'ﾎ' => 'ホ',
        'ﾏ' => 'マ',
        'ﾐ' => 'ミ',
        'ﾑ' => 'ム',
        'ﾒ' => 'メ',
        'ﾓ' => 'モ',
        'ﾔ' => 'ヤ',
        'ﾕ' => 'ユ',
        'ﾖ' => 'ヨ',
        'ﾗ' => 'ラ',
        'ﾘ' => 'リ',
        'ﾙ' => 'ル',
        'ﾚ' => 'レ',
        'ﾛ' => 'ロ',
        'ﾜ' => 'ワ',
        'ﾝ' => 'ン',
        _ => return None,
    };
    Some(full)
}

fn katakana_dakuten(r: char) -> Option<char> {
    let voiced = match r {
        'ウ' => 'ヴ',
        'カ' => 'ガ',
        'キ' => 'ギ',
        'ク' => 'グ',
        'ケ' => 'ゲ',
        'コ' => 'ゴ',
        'サ' => 'ザ',
        'シ' => 'ジ',
        'ス' => 'ズ',
        'セ' => 'ゼ',
        'ソ' => 'ゾ',
        'タ' => 'ダ',
        'チ' => 'ヂ',
        'ツ' => 'ヅ',
        'テ' => 'デ',
        'ト' => 'ド',
        'ハ' => 'バ',
        'ヒ' => 'ビ',
        'フ' => 'ブ',
        'ヘ' => 'ベ',
        'ホ' => 'ボ',
        'ワ' => 'ヷ',
        'ヰ' => 'ヸ',
        'ヱ' => 'ヹ',
        'ヲ' => 'ヺ',
        _ => return None,
    };
    Some(voiced)
}

fn katakana_handakuten(r: char) -> Option<char> {
    let semi = match r {
        'ハ' => 'パ',
        'ヒ' => 'ピ',
        'フ' => 'プ',
        'ヘ' => 'ペ',
        'ホ' => 'ポ',
        _ => return None,
    };
    Some(semi)
}

fn halfwidth_kana(r: char) -> Option<&'static str> {
    let half = match r {
        '。' => "｡",
        '「' => "｢",
        '」' => "｣",
        '、' => "､",
        '・' => "･",
        'ヲ' => "ｦ",
        'ァ' => "ｧ",
        'ィ' => "ｨ",
        'ゥ' => "ｩ",
        'ェ' => "ｪ",
        'ォ' => "ｫ",
        'ャ' => "ｬ",
        'ュ' => "ｭ",
        'ョ' => "ｮ",
        'ッ' => "ｯ",
        'ー' => "ｰ",
        'ア' => "ｱ",
        'イ' => "ｲ",
        'ウ' => "ｳ",
        'エ' => "ｴ",
        'オ' => "ｵ",
        'カ' => "ｶ",
        'キ' => "ｷ",
        'ク' => "ｸ",
        'ケ' => "ｹ",
        'コ' => "ｺ",
        'サ' => "ｻ",
        'シ' => "ｼ",
        'ス' => "ｽ",
        'セ' => "ｾ",
        'ソ' => "ｿ",
        'タ' => "ﾀ",
        'チ' => "ﾁ",
        'ツ' => "ﾂ",
        'テ' => "ﾃ",
        'ト' => "ﾄ",
        'ナ' => "ﾅ",
        'ニ' => "ﾆ",
        'ヌ' => "ﾇ",
        'ネ' => "ﾈ",
        'ノ' => "ﾉ",
        'ハ' => "ﾊ",
        'ヒ' => "ﾋ",
        'フ' => "ﾌ",
        'ヘ' => "ﾍ",
        'ホ' => "ﾎ",
        'マ' => "ﾏ",
        'ミ' => "ﾐ",
        'ム' => "ﾑ",
        'メ' => "ﾒ",
        'モ' => "ﾓ",
        'ヤ' => "ﾔ",
        'ユ' => "ﾕ",
        'ヨ' => "ﾖ",
        'ラ' => "ﾗ",
        'リ' => "ﾘ",
        'ル' => "ﾙ",
        'レ' => "ﾚ",
        'ロ' => "ﾛ",
        'ワ' => "ﾜ",
        'ヰ' => "ｲ",
        'ヱ' => "ｴ",
        'ン' => "ﾝ",
        'ガ' => "ｶﾞ",
        'ギ' => "ｷﾞ",
        'グ' => "ｸﾞ",
        'ゲ' => "ｹﾞ",
        'ゴ' => "ｺﾞ",
        'ザ' => "ｻﾞ",
        'ジ' => "ｼﾞ",
        'ズ' => "ｽﾞ",
        'ゼ' => "ｾﾞ",
        'ゾ' => "ｿﾞ",
        'ダ' => "ﾀﾞ",
        'ヂ' => "ﾁﾞ",
        'ヅ' => "ﾂﾞ",
        'デ' => "ﾃﾞ",
        'ド' => "ﾄﾞ",
        'バ' => "ﾊﾞ",
        'ビ' => "ﾋﾞ",
        'ブ' => "ﾌﾞ",
        'ベ' => "ﾍﾞ",
        'ボ' => "ﾎﾞ",
        'パ' => "ﾊﾟ",
        'ピ' => "ﾋﾟ",
        'プ' => "ﾌﾟ",
        'ペ' => "ﾍﾟ",
        'ポ' => "ﾎﾟ",
        'ヴ' => "ｳﾞ",
        'ヷ' => "ﾜﾞ",
        'ヸ' => "ｲﾞ",
        'ヹ' => "ｴﾞ",
        'ヺ' => "ｦﾞ",
        'ヵ' => "ｶ",
        'ヶ' => "ｹ",
        'ヮ' => "ﾜ",
        '゛' => "ﾞ",
        '゜' => "ﾟ",
        '\u{3000}' => " ",
        _ => return None,
    };
    Some(half)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_hiragana_table() {
        for (input, want) in [
            ("きょう", "きょう"),
            ("キョウ", "きょう"),
            ("ｷｮｳ", "きょう"),
            ("ｶﾞｯｺｳ", "がっこう"),
            ("ヴァイオリン", "ゔぁいおりん"),
            ("ABC123", "ABC123"),
            ("ー", "ー"),
            // 舊假名與前身一樣保持原樣。
            ("ゐゑを", "ゐゑを"),
        ] {
            assert_eq!(to_hiragana(input), want, "to_hiragana({input:?})");
        }
    }

    #[test]
    fn test_to_katakana_table() {
        for (input, want) in [
            ("きょう", "キョウ"),
            ("キョウ", "キョウ"),
            ("ｷｮｳ", "キョウ"),
            ("がっこう", "ガッコウ"),
            ("ゔぁいおりん", "ヴァイオリン"),
            ("ABC123", "ABC123"),
        ] {
            assert_eq!(to_katakana(input), want, "to_katakana({input:?})");
        }
    }

    #[test]
    fn test_to_halfwidth_kana_table() {
        for (input, want) in [
            ("きょう", "ｷｮｳ"),
            ("キョウ", "ｷｮｳ"),
            ("がっこう", "ｶﾞｯｺｳ"),
            ("ヴァイオリン", "ｳﾞｧｲｵﾘﾝ"),
            ("ぱぴぷぺぽ", "ﾊﾟﾋﾟﾌﾟﾍﾟﾎﾟ"),
            ("ｶﾞｯｺｳ", "ｶﾞｯｺｳ"),
            ("今日", "今日"),
        ] {
            assert_eq!(
                to_halfwidth_kana(input),
                want,
                "to_halfwidth_kana({input:?})"
            );
        }
    }

    #[test]
    fn test_latin_width_table() {
        for (input, want) in [
            ("abc123", "ａｂｃ１２３"),
            ("ABC-123!", "ＡＢＣ－１２３！"),
            ("a b", "ａ　ｂ"),
            ("あいう", "あいう"),
        ] {
            assert_eq!(
                to_fullwidth_latin(input),
                want,
                "to_fullwidth_latin({input:?})"
            );
        }
        for (input, want) in [
            ("ａｂｃ１２３", "abc123"),
            ("ＡＢＣ－１２３！", "ABC-123!"),
            ("ａ　ｂ", "a b"),
            ("あいう", "あいう"),
        ] {
            assert_eq!(
                to_halfwidth_latin(input),
                want,
                "to_halfwidth_latin({input:?})"
            );
        }
    }
}
