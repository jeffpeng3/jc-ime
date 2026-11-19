//! 拼音轉注音與音節表。
//!
//! 音節表是詞庫編譯器的合法性檢查依據：不在表內的音節一律拒收，所以漏列
//! 會以編譯錯誤的形式大聲出現，而不是悄悄放行。

use std::collections::HashSet;
use std::sync::LazyLock;

use crate::zh::syllable::Syllable;
use crate::zh::ZhError;

/// 標準普通話拼音音節（不含聲調），加上台灣注音另有的少數音節。
///
/// 例：崖 `ㄧㄞ`、這 `ㄓㄟ`。
const PINYIN_INVENTORY: &str = "
a o e ai ei ao ou an en ang eng er
yi ya yo ye yai yao you yan yin yang ying yong
wu wa wo wai wei wan wen wang weng
yu yue yuan yun
ba bo bai bei bao ban ben bang beng bi bie biao bian bin bing bu
pa po pai pei pao pou pan pen pang peng pi pie piao pian pin ping pu
ma mo me mai mei mao mou man men mang meng mi mie miao miu mian min ming mu
fa fo fei fou fan fen fang feng fu
da de dai dei dao dou dan den dang deng dong di dia die diao diu dian ding du duo dui duan dun
ta te tai tei tao tou tan tang teng tong ti tie tiao tian ting tu tuo tui tuan tun
na ne nai nei nao nou nan nen nang neng nong ni nie niao niu nian nin niang ning nu nuo nuan nun nü nüe
la le lo lai lei lao lou lan lang leng long li lia lie liao liu lian lin liang ling lu luo luan lun lü lüe
ga ge gai gei gao gou gan gen gang geng gong gu gua guo guai gui guan gun guang
ka ke kai kei kao kou kan ken kang keng kong ku kua kuo kuai kui kuan kun kuang
ha he hai hei hao hou han hen hang heng hong hu hua huo huai hui huan hun huang
ji jia jie jiao jiu jian jin jiang jing jiong ju jue juan jun
qi qia qie qiao qiu qian qin qiang qing qiong qu que quan qun
xi xia xie xiao xiu xian xin xiang xing xiong xu xue xuan xun
zha zhe zhi zhai zhei zhao zhou zhan zhen zhang zheng zhong zhu zhua zhuo zhuai zhui zhuan zhun zhuang
cha che chi chai chao chou chan chen chang cheng chong chu chua chuo chuai chui chuan chun chuang
sha she shi shai shei shao shou shan shen shang sheng shu shua shuo shuai shui shuan shun shuang
re ri rao rou ran ren rang reng rong ru rua ruo rui ruan run
za ze zi zai zei zao zou zan zen zang zeng zong zu zuo zui zuan zun
ca ce ci cai cao cou can cen cang ceng cong cu cuo cui cuan cun
sa se si sai sao sou san sen sang seng song su suo sui suan sun
ê
lüan sei biang pia fiao
";

/// 聲母的拼音寫法；`zh`、`ch`、`sh` 必須在 `z`、`c`、`s` 之前比對。
const PINYIN_INITIALS: &[(&str, &str)] = &[
    ("zh", "ㄓ"),
    ("ch", "ㄔ"),
    ("sh", "ㄕ"),
    ("b", "ㄅ"),
    ("p", "ㄆ"),
    ("m", "ㄇ"),
    ("f", "ㄈ"),
    ("d", "ㄉ"),
    ("t", "ㄊ"),
    ("n", "ㄋ"),
    ("l", "ㄌ"),
    ("g", "ㄍ"),
    ("k", "ㄎ"),
    ("h", "ㄏ"),
    ("j", "ㄐ"),
    ("q", "ㄑ"),
    ("x", "ㄒ"),
    ("r", "ㄖ"),
    ("z", "ㄗ"),
    ("c", "ㄘ"),
    ("s", "ㄙ"),
];

/// `y`、`w` 開頭的零聲母拼寫還原成介音形式。
fn zero_initial(p: &str) -> Option<&'static str> {
    Some(match p {
        "yi" => "i",
        "ya" => "ia",
        "yo" => "io",
        "ye" => "ie",
        "yai" => "iai",
        "yao" => "iao",
        "you" => "iou",
        "yan" => "ian",
        "yin" => "in",
        "yang" => "iang",
        "ying" => "ing",
        "yong" => "iong",
        "yu" => "ü",
        "yue" => "üe",
        "yuan" => "üan",
        "yun" => "ün",
        "wu" => "u",
        "wa" => "ua",
        "wo" => "uo",
        "wai" => "uai",
        "wei" => "uei",
        "wan" => "uan",
        "wen" => "uen",
        "wang" => "uang",
        "weng" => "ueng",
        _ => return None,
    })
}

/// 韻母的拼音寫法對應的注音。
fn final_of(f: &str) -> Result<&'static str, ZhError> {
    Ok(match f {
        "" => "",
        "a" => "ㄚ",
        "o" => "ㄛ",
        "e" => "ㄜ",
        "ê" => "ㄝ",
        "ai" => "ㄞ",
        "ei" => "ㄟ",
        "ao" => "ㄠ",
        "ou" => "ㄡ",
        "an" => "ㄢ",
        "en" => "ㄣ",
        "ang" => "ㄤ",
        "eng" => "ㄥ",
        "er" => "ㄦ",
        "ong" => "ㄨㄥ",
        "i" => "ㄧ",
        "ia" => "ㄧㄚ",
        "io" => "ㄧㄛ",
        "ie" => "ㄧㄝ",
        "iai" => "ㄧㄞ",
        "iao" => "ㄧㄠ",
        "iou" => "ㄧㄡ",
        "ian" => "ㄧㄢ",
        "in" => "ㄧㄣ",
        "iang" => "ㄧㄤ",
        "ing" => "ㄧㄥ",
        "iong" => "ㄩㄥ",
        "u" => "ㄨ",
        "ua" => "ㄨㄚ",
        "uo" => "ㄨㄛ",
        "uai" => "ㄨㄞ",
        "uei" => "ㄨㄟ",
        "uan" => "ㄨㄢ",
        "uen" => "ㄨㄣ",
        "uang" => "ㄨㄤ",
        "ueng" => "ㄨㄥ",
        "ü" => "ㄩ",
        "üe" => "ㄩㄝ",
        "üan" => "ㄩㄢ",
        "ün" => "ㄩㄣ",
        _ => return Err(ZhError::UnknownFinal(f.to_string())),
    })
}

/// 把不帶聲調的拼音轉成不帶聲調的注音。
fn pinyin_base(p: &str) -> Result<String, ZhError> {
    match p {
        // 空韻：只留聲母。
        "zhi" | "chi" | "shi" | "ri" | "zi" | "ci" | "si" => {
            for (py, zy) in PINYIN_INITIALS {
                if p.starts_with(py) {
                    return Ok((*zy).to_string());
                }
            }
        }
        // `r`：兒化音節（CC-CEDICT 以 r5 表示）。
        "er" | "r" => return Ok("ㄦ".to_string()),
        _ => {}
    }
    if let Some(f) = zero_initial(p) {
        return Ok(final_of(f)?.to_string());
    }
    for (py, zy) in PINYIN_INITIALS {
        let Some(rest) = p.strip_prefix(py) else {
            continue;
        };
        let mut rest = rest.to_string();
        if matches!(*py, "j" | "q" | "x") && rest.starts_with('u') {
            // ju → jü
            rest.replace_range(..1, "ü");
        }
        // 縮寫還原。
        rest = match rest.as_str() {
            "iu" => "iou".to_string(),
            "ui" => "uei".to_string(),
            "un" => "uen".to_string(),
            _ => rest,
        };
        let f = final_of(&rest)?;
        if f.is_empty() {
            return Err(ZhError::MissingFinal(p.to_string()));
        }
        return Ok(format!("{zy}{f}"));
    }
    Ok(final_of(p)?.to_string())
}

static LEGAL_BASES: LazyLock<HashSet<String>> = LazyLock::new(|| {
    PINYIN_INVENTORY
        .split_whitespace()
        .map(|p| pinyin_base(p).unwrap_or_else(|e| panic!("音節表內的 {p:?} 無法轉換：{e}")))
        .collect()
});

static LEGAL_PREFIXES: LazyLock<HashSet<String>> = LazyLock::new(|| {
    let mut set = HashSet::new();
    for base in LEGAL_BASES.iter() {
        let chars: Vec<char> = base.chars().collect();
        for i in 1..=chars.len() {
            set.insert(chars[..i].iter().collect::<String>());
        }
    }
    set
});

/// 回報不帶聲調的字串是否為音節表內的音節。
pub fn is_legal_base(base: &str) -> bool {
    LEGAL_BASES.contains(base)
}

/// 音節表內所有不帶聲調的合法音節，依字串排序。
///
/// 供窮舉式測試與資料稽核使用。
pub fn inventory() -> Vec<Syllable> {
    let mut out: Vec<Syllable> = LEGAL_BASES
        .iter()
        .map(|b| Syllable::from_raw(b.clone()))
        .collect();
    out.sort();
    out
}

/// 回報不帶聲調的 `s` 是否為音節表內某個音節的開頭（含完整音節本身）。
///
/// 例如 `ㄒㄧ`、`ㄒㄧㄢ` 是，`ㄉㄧㄤ`、`ㄅㄩ` 不是。不打聲調時用它判斷
/// 下一個注音要接在目前的音節後面還是開新的音節。
pub fn legal_prefix(s: &Syllable) -> bool {
    LEGAL_PREFIXES.contains(s.as_str())
}

/// 拼音單個母音上的聲調符號。
const TONE_MARKS: &[(char, char, u8)] = &[
    ('ā', 'a', 1),
    ('á', 'a', 2),
    ('ǎ', 'a', 3),
    ('à', 'a', 4),
    ('ē', 'e', 1),
    ('é', 'e', 2),
    ('ě', 'e', 3),
    ('è', 'e', 4),
    ('ī', 'i', 1),
    ('í', 'i', 2),
    ('ǐ', 'i', 3),
    ('ì', 'i', 4),
    ('ō', 'o', 1),
    ('ó', 'o', 2),
    ('ǒ', 'o', 3),
    ('ò', 'o', 4),
    ('ū', 'u', 1),
    ('ú', 'u', 2),
    ('ǔ', 'u', 3),
    ('ù', 'u', 4),
    ('ǖ', 'ü', 1),
    ('ǘ', 'ü', 2),
    ('ǚ', 'ü', 3),
    ('ǜ', 'ü', 4),
    ('ế', 'ê', 2),
    ('ề', 'ê', 4),
];

fn tone_mark(ch: char) -> Option<(char, u8)> {
    TONE_MARKS
        .iter()
        .find(|(mark, _, _)| *mark == ch)
        .map(|(_, base, tone)| (*base, *tone))
}

fn combining_tone(ch: char) -> Option<u8> {
    match ch {
        '\u{0304}' => Some(1),
        '\u{0301}' => Some(2),
        '\u{030C}' => Some(3),
        '\u{0300}' => Some(4),
        _ => None,
    }
}

/// 把全形 ASCII 與全形空白轉成半形，並轉小寫、去掉前後空白。
fn normalize(p: &str) -> String {
    p.chars()
        .map(|ch| match ch {
            '\u{3000}' => ' ',
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(ch as u32 - 0xFEE0).unwrap_or(ch),
            _ => ch,
        })
        .collect::<String>()
        .to_lowercase()
        .trim()
        .to_string()
}

/// 把一個拼音音節轉成注音音節。
///
/// 接受數字聲調（`qi3`、`lu:4`、`r5`）與聲調符號（`qǐ`、`lǜ`）兩種寫法，
/// 也接受全形字母與空白；沒有聲調時視為輕聲。結果必須是音節表內的合法
/// 音節。
pub fn from_pinyin(p: &str) -> Result<Syllable, ZhError> {
    let p = normalize(p)
        .replace("e^", "ê")
        .replace("u:", "ü")
        .replace('v', "ü");

    let mut tone = 5u8;
    let body;
    match p.chars().last() {
        Some(last @ '1'..='5') => {
            tone = last as u8 - b'0';
            body = p[..p.len() - 1].to_string();
        }
        _ => {
            let mut out = String::with_capacity(p.len());
            for ch in p.chars() {
                if let Some((base, t)) = tone_mark(ch) {
                    tone = t;
                    out.push(base);
                } else if let Some(t) = combining_tone(ch) {
                    tone = t;
                } else if ch == '\u{0308}' {
                    // 組合用分音符：u + ¨ → ü。
                    if out.ends_with('u') {
                        out.pop();
                        out.push('ü');
                    } else {
                        out.push(ch);
                    }
                } else {
                    out.push(ch);
                }
            }
            body = out;
        }
    }

    for ch in body.chars() {
        if !ch.is_lowercase() && ch != 'ü' && ch != 'ê' {
            return Err(ZhError::NonLetter(body));
        }
    }
    let base = pinyin_base(&body)?;
    let syl = Syllable::from_raw(base).with_tone(tone);
    if !syl.legal() {
        return Err(ZhError::IllegalSyllable(syl.as_str().to_string()));
    }
    Ok(syl)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zh::syllable::TONE1;

    #[test]
    fn test_from_pinyin() {
        let cases: &[(&str, &str)] = &[
            ("qi3", "ㄑㄧˇ"),
            ("ye4", "ㄧㄝˋ"),
            ("shuo1", "ㄕㄨㄛ"),
            ("shui4", "ㄕㄨㄟˋ"),
            ("dong4", "ㄉㄨㄥˋ"),
            ("jing3", "ㄐㄧㄥˇ"),
            ("zhi1", "ㄓ"),
            ("si4", "ㄙˋ"),
            ("ri4", "ㄖˋ"),
            ("er2", "ㄦˊ"),
            ("r5", "ㄦ˙"),
            ("lu:e4", "ㄌㄩㄝˋ"),
            ("nü3", "ㄋㄩˇ"),
            ("lv4", "ㄌㄩˋ"),
            ("ju2", "ㄐㄩˊ"),
            ("que4", "ㄑㄩㄝˋ"),
            ("xuan1", "ㄒㄩㄢ"),
            ("jun1", "ㄐㄩㄣ"),
            ("qiong2", "ㄑㄩㄥˊ"),
            ("yong3", "ㄩㄥˇ"),
            ("yun2", "ㄩㄣˊ"),
            ("yue4", "ㄩㄝˋ"),
            ("you3", "ㄧㄡˇ"),
            ("liu2", "ㄌㄧㄡˊ"),
            ("gui4", "ㄍㄨㄟˋ"),
            ("dun4", "ㄉㄨㄣˋ"),
            ("weng1", "ㄨㄥ"),
            ("wo3", "ㄨㄛˇ"),
            ("hong2", "ㄏㄨㄥˊ"),
            ("bo2", "ㄅㄛˊ"),
            ("me5", "ㄇㄜ˙"),
            ("ma", "ㄇㄚ˙"),
            ("a1", "ㄚ"),
            ("e4", "ㄜˋ"),
            ("ê2", "ㄝˊ"),
            ("yai2", "ㄧㄞˊ"),
            ("zhei4", "ㄓㄟˋ"),
            ("qǐ", "ㄑㄧˇ"),
            ("lǜ", "ㄌㄩˋ"),
            ("qi\u{030C}", "ㄑㄧˇ"),
            ("lu\u{0308}\u{0300}", "ㄌㄩˋ"),
            ("shuō", "ㄕㄨㄛ"),
            ("nǚ", "ㄋㄩˇ"),
            ("ér", "ㄦˊ"),
        ];
        for (py, want) in cases {
            assert_eq!(
                from_pinyin(py),
                Ok(Syllable::from_raw(*want)),
                "FromPinyin({py:?})"
            );
        }
        for bad in ["bu:3", "xa1", "zhia4", "abc", "q", "hm5", "ng2", "12"] {
            assert!(from_pinyin(bad).is_err(), "{bad:?} 應回報錯誤");
        }
    }

    #[test]
    fn from_pinyin_accepts_fullwidth_and_whitespace() {
        assert_eq!(from_pinyin("  ＱＩ３ "), Ok(Syllable::from_raw("ㄑㄧˇ")));
        assert_eq!(from_pinyin("Zhong1"), Ok(Syllable::from_raw("ㄓㄨㄥ")));
    }

    #[test]
    fn test_legal() {
        for s in [
            "ㄉㄨㄥˋ",
            "ㄐㄧㄥˇ",
            "ㄓ",
            "ㄇㄜ˙",
            "ㄦˊ",
            "ㄩㄥˇ",
            "ㄧㄞˊ",
            "ㄋㄩㄝˋ",
            "ㄌㄩㄢˊ",
            "ㄙㄟ",
            "ㄆㄧㄚ",
        ] {
            assert!(Syllable::from_raw(s).legal(), "{s} 應為合法音節");
        }
        for s in ["ㄅㄩ", "ㄓㄧ", "ㄍㄧ", "ㄈㄧㄥ", "ㄉㄨㄥx", "ㄦㄢ"] {
            assert!(!Syllable::from_raw(s).legal(), "{s} 應不合法");
        }
        let n = LEGAL_BASES.len();
        assert!((400..=430).contains(&n), "音節表有 {n} 個基本音節");
    }

    #[test]
    fn test_tone_helpers() {
        let s = Syllable::from_raw("ㄧ");
        assert_eq!(s.tone(), 1);
        assert_eq!(s.with_tone(2), Syllable::from_raw("ㄧˊ"));
        assert_eq!(
            Syllable::from_raw("ㄧˋ").with_tone(1),
            Syllable::from_raw("ㄧ")
        );
        assert_eq!(Syllable::from_raw("ㄇㄜ˙").tone(), 5);
        assert_eq!(s.with_tone(9), Syllable::from_raw("ㄧ"));
        assert_eq!(s.with_tone(1), s);
    }

    #[test]
    fn inventory_is_sorted_and_matches_legal() {
        let all = inventory();
        assert_eq!(all.len(), LEGAL_BASES.len());
        assert!(
            all.windows(2).all(|w| w[0] < w[1]),
            "inventory 應排序且不重複"
        );
        assert!(all.iter().all(Syllable::legal));
    }

    #[test]
    fn test_legal_prefix() {
        for s in ["ㄒ", "ㄒㄧ", "ㄒㄧㄢ", "ㄓㄨ", "ㄢ", "ㄧ", "ㄧㄢ", "ㄈㄧ"] {
            assert!(legal_prefix(&Syllable::from_raw(s)), "{s} 是音節的開頭");
        }
        for s in ["ㄉㄧㄤ", "ㄅㄩ", "ㄒㄚ", "ㄒㄧˋ", ""] {
            assert!(
                !legal_prefix(&Syllable::from_raw(s)),
                "{s} 不是任何音節的開頭"
            );
        }
    }

    #[test]
    fn tone1_marker_is_not_part_of_legal_syllables() {
        assert_eq!(TONE1, 'ˉ');
        assert!(!Syllable::from_raw("ㄓˉ").valid());
    }
}
