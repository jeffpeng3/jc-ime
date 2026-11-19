//! 台灣正體轉簡體：比對規則照 OpenCC 的 `tw2s` 設定。
//!
//! 字典是 OpenCC 上游的文字檔原文，由宿主讀進來交給 [`Converter`]；這裡只有
//! 比對邏輯，不碰檔案。`tw2s` 的步驟：
//!
//! 1. 正規化：CJK 相容表意字換成統一表意字。
//! 2. 分段：以 `TSPhrases` 由左而右取最長詞；比對不到的字連成一段。
//! 3. 每段依序經過兩組字典，各自由左而右取最長的鍵：
//!    先 `TWVariantsRevPhrases → TWVariantsRev`（台灣字形回到 OpenCC 標準字形），
//!    再 `TSPhrases → TSCharacters`（繁轉簡）。一組裡第一本比對得到的字典勝出。
//!
//! 上游標成「可能變豆腐字」的延伸字典 `TSCharactersExt` 不用，同 OpenCC
//! 命令列的預設。
//!
//! 對照前身 `core/zhconv`：資料來源是 OpenCC ver.1.4.2 的官方文字檔，
//! 授權 Apache-2.0，放在資料根目錄的 `opencc/`。

use std::collections::HashMap;
use std::path::Path;

/// 字典解析、建立或讀取失敗。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ZhconvError {
    /// 字典某一行不是「鍵<Tab>值」格式。
    #[error("第 {line} 行不是「鍵<Tab>值」：{line_text:?}")]
    Line {
        /// 行號，從 1 開始。
        line: usize,
        /// 原始內容。
        line_text: String,
    },
    /// `@reverse-prefer` 的欄位數不對。
    #[error("第 {line} 行的 @reverse-prefer 格式不對：{line_text:?}")]
    PreferFormat {
        /// 行號。
        line: usize,
        /// 原始內容。
        line_text: String,
    },
    /// `@reverse-prefer` 指定的值或鍵不存在。
    #[error("@reverse-prefer {value} → {preferred} 沒有對應的詞條")]
    PreferMissing {
        /// 被反轉的值。
        value: String,
        /// 指定的鍵。
        preferred: String,
    },
    /// 字典沒有任何詞條。
    #[error("{name}：沒有詞條")]
    Empty {
        /// 字典名稱。
        name: String,
    },
    /// 讀取字典檔失敗。
    #[error("讀取 {path} 失敗：{message}")]
    Io {
        /// 檔案路徑。
        path: String,
        /// 原始錯誤。
        message: String,
    },
}

/// 一本 OpenCC 字典：鍵 → 第一個值。
///
/// OpenCC 轉換時一律取第一個值；重複的鍵以第一次出現的為準。
#[derive(Debug, Clone, Default)]
pub struct Dict {
    map: HashMap<String, String>,
    /// 最長的鍵，以字元計。
    max: usize,
}

impl Dict {
    fn add(&mut self, key: &str, value: &str) {
        if self.map.contains_key(key) {
            return;
        }
        self.map.insert(key.to_string(), value.to_string());
        self.max = self.max.max(key.chars().count());
    }

    /// 詞條數。
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// 是否沒有任何詞條。
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// 從 `chars[i]` 起最長的鍵；回傳（長度, 值）。
    fn match_at(&self, chars: &[char], i: usize) -> Option<(usize, &str)> {
        let limit = self.max.min(chars.len() - i);
        for n in (1..=limit).rev() {
            let key: String = chars[i..i + n].iter().collect();
            if let Some(value) = self.map.get(&key) {
                return Some((n, value.as_str()));
            }
        }
        None
    }
}

/// 讀 OpenCC 的文字字典：每行「鍵<Tab>值 值…」，`#` 開頭與空行略過。
pub fn parse_dict(text: &str) -> Result<Dict, ZhconvError> {
    let mut dict = Dict::default();
    for (index, raw) in text.split('\n').enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, values)) = line.split_once('\t') else {
            return Err(ZhconvError::Line {
                line: index + 1,
                line_text: line.to_string(),
            });
        };
        if key.is_empty() || values.is_empty() {
            return Err(ZhconvError::Line {
                line: index + 1,
                line_text: line.to_string(),
            });
        }
        let first = values.split(' ').next().unwrap_or(values);
        dict.add(key, first);
    }
    Ok(dict)
}

/// 讀一本字典並把鍵值對調，用來產生 `TWVariantsRev`。
///
/// 同一個值對到好幾個鍵時取檔案裡最先出現的鍵；`# @reverse-prefer: 值 [鍵]`
/// 指定例外，只寫一個字時鍵與值相同。
pub fn parse_reversed(text: &str) -> Result<Dict, ZhconvError> {
    const PREFER: &str = "# @reverse-prefer:";
    let mut reversed: HashMap<String, Vec<String>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut preferred: HashMap<String, String> = HashMap::new();
    for (index, raw) in text.split('\n').enumerate() {
        let line = raw.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix(PREFER) {
            let fields: Vec<&str> = rest.split_whitespace().collect();
            if fields.len() != 1 && fields.len() != 2 {
                return Err(ZhconvError::PreferFormat {
                    line: index + 1,
                    line_text: line.to_string(),
                });
            }
            preferred.insert(fields[0].to_string(), fields[fields.len() - 1].to_string());
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, values)) = line.split_once('\t') else {
            return Err(ZhconvError::Line {
                line: index + 1,
                line_text: line.to_string(),
            });
        };
        if key.is_empty() || values.is_empty() {
            return Err(ZhconvError::Line {
                line: index + 1,
                line_text: line.to_string(),
            });
        }
        for value in values.split(' ') {
            if !reversed.contains_key(value) {
                order.push(value.to_string());
            }
            reversed
                .entry(value.to_string())
                .or_default()
                .push(key.to_string());
        }
    }
    let mut dict = Dict::default();
    for value in &order {
        let keys = &reversed[value];
        let mut first = keys[0].as_str();
        if let Some(preferred_key) = preferred.get(value) {
            if !keys.iter().any(|key| key == preferred_key) {
                return Err(ZhconvError::PreferMissing {
                    value: value.clone(),
                    preferred: preferred_key.clone(),
                });
            }
            first = preferred_key;
        }
        dict.add(value, first);
    }
    for (value, preferred_key) in &preferred {
        if !reversed.contains_key(value) {
            return Err(ZhconvError::PreferMissing {
                value: value.clone(),
                preferred: preferred_key.clone(),
            });
        }
    }
    Ok(dict)
}

/// OpenCC `short_circuit` 的字典組：依序查，第一本比對得到的字典勝出。
#[derive(Debug, Clone, Default)]
struct Group(Vec<Dict>);

impl Group {
    fn match_at(&self, chars: &[char], i: usize) -> Option<(usize, String)> {
        for dict in &self.0 {
            if let Some((n, value)) = dict.match_at(chars, i) {
                return Some((n, value.to_string()));
            }
        }
        None
    }

    /// 由左而右取最長的鍵換成值，比對不到的字照原樣。
    fn convert(&self, chars: &[char]) -> Vec<char> {
        let mut out = Vec::with_capacity(chars.len());
        let mut i = 0;
        while i < chars.len() {
            match self.match_at(chars, i) {
                Some((n, value)) => {
                    out.extend(value.chars());
                    i += n;
                }
                None => {
                    out.push(chars[i]);
                    i += 1;
                }
            }
        }
        out
    }
}

/// `tw2s` 用到的 OpenCC 上游字典原文。
#[derive(Debug, Clone, Copy, Default)]
pub struct Data<'a> {
    /// `CJK_Compatibility_Ideographs.txt`。
    pub cjk_compatibility: &'a str,
    /// `TWVariants.txt`，會反轉成 `TWVariantsRev`。
    pub tw_variants: &'a str,
    /// `TWVariantsRevPhrases.txt`。
    pub tw_variants_rev_phrases: &'a str,
    /// `TSPhrases.txt`。
    pub ts_phrases: &'a str,
    /// `TSCharacters.txt`。
    pub ts_characters: &'a str,
}

/// 一條 tw2s 轉換鏈；建好後唯讀，可同時給多個 session 使用。
#[derive(Debug, Clone)]
pub struct Converter {
    normalize: Group,
    segment: Dict,
    chain: Vec<Group>,
}

fn parse_named(name: &str, text: &str, reverse: bool) -> Result<Dict, ZhconvError> {
    let dict = if reverse {
        parse_reversed(text)?
    } else {
        parse_dict(text)?
    };
    if dict.is_empty() {
        return Err(ZhconvError::Empty {
            name: name.to_string(),
        });
    }
    Ok(dict)
}

/// 依 OpenCC `tw2s.json` 建立台灣正體轉簡體的轉換器。
pub fn tw2s(data: Data<'_>) -> Result<Converter, ZhconvError> {
    let compat = parse_named(
        "CJK_Compatibility_Ideographs",
        data.cjk_compatibility,
        false,
    )?;
    let variants = parse_named("TWVariants", data.tw_variants, true)?;
    let variant_phrases = parse_named("TWVariantsRevPhrases", data.tw_variants_rev_phrases, false)?;
    let phrases = parse_named("TSPhrases", data.ts_phrases, false)?;
    let chars = parse_named("TSCharacters", data.ts_characters, false)?;
    Ok(Converter {
        normalize: Group(vec![compat]),
        segment: phrases.clone(),
        chain: vec![
            Group(vec![variant_phrases, variants]),
            Group(vec![phrases, chars]),
        ],
    })
}

const CJK_COMPATIBILITY: &str = "CJK_Compatibility_Ideographs.txt";
const TW_VARIANTS: &str = "TWVariants.txt";
const TW_VARIANTS_REV_PHRASES: &str = "TWVariantsRevPhrases.txt";
const TS_PHRASES: &str = "TSPhrases.txt";
const TS_CHARACTERS: &str = "TSCharacters.txt";

/// 從目錄讀 OpenCC 的五本字典並建立轉換器。
pub fn tw2s_from_dir(dir: &Path) -> Result<Converter, ZhconvError> {
    let read = |name: &str| -> Result<String, ZhconvError> {
        let path = dir.join(name);
        std::fs::read_to_string(&path).map_err(|source| ZhconvError::Io {
            path: path.display().to_string(),
            message: source.to_string(),
        })
    };
    let compat = read(CJK_COMPATIBILITY)?;
    let variants = read(TW_VARIANTS)?;
    let variant_phrases = read(TW_VARIANTS_REV_PHRASES)?;
    let phrases = read(TS_PHRASES)?;
    let chars = read(TS_CHARACTERS)?;
    tw2s(Data {
        cjk_compatibility: &compat,
        tw_variants: &variants,
        tw_variants_rev_phrases: &variant_phrases,
        ts_phrases: &phrases,
        ts_characters: &chars,
    })
}

impl Converter {
    /// 把台灣正體轉成簡體；英文、數字、標點、Emoji 與注音符號照原樣保留。
    pub fn convert(&self, text: &str) -> String {
        self.convert_pinned(text, &HashMap::new())
    }

    /// 同 [`Converter::convert`]，但 `pins` 指定的位置直接輸出指定的文字。
    ///
    /// 位置以字元計、對應 `text`；指定的位置同時是分段邊界，跨過它的詞不會被
    /// 整詞比對。正規化表若讓字數改變，位置對不上就忽略 `pins`。
    pub fn convert_pinned(&self, text: &str, pins: &HashMap<usize, String>) -> String {
        if text.is_empty() {
            return String::new();
        }
        let original: Vec<char> = text.chars().collect();
        let chars = self.normalize.convert(&original);
        let empty = HashMap::new();
        let pinned = if chars.len() == original.len() {
            pins
        } else {
            &empty
        };
        let mut out = String::with_capacity(text.len());
        let mut start = 0usize;
        let mut i = 0usize;
        while i <= chars.len() {
            let pin = if i < chars.len() {
                pinned.get(&i)
            } else {
                None
            };
            if i < chars.len() && pin.is_none() {
                i += 1;
                continue;
            }
            self.convert_span(&mut out, &chars[start..i]);
            if let Some(pin) = pin {
                if i < chars.len() {
                    out.push_str(pin);
                }
            }
            start = i + 1;
            i += 1;
        }
        out
    }

    /// 分段後每段經過整條轉換鏈。
    fn convert_span(&self, out: &mut String, chars: &[char]) {
        let mut run = 0usize;
        let mut i = 0usize;
        while i < chars.len() {
            let Some((n, _)) = self.segment.match_at(chars, i) else {
                i += 1;
                continue;
            };
            if run < i {
                self.flush(out, &chars[run..i]);
            }
            self.flush(out, &chars[i..i + n]);
            i += n;
            run = i;
        }
        if run < chars.len() {
            self.flush(out, &chars[run..]);
        }
    }

    fn flush(&self, out: &mut String, segment: &[char]) {
        let mut segment = segment.to_vec();
        for group in &self.chain {
            segment = group.convert(&segment);
        }
        out.extend(segment);
    }
}

impl crate::output::HansConverter for Converter {
    fn to_simplified(&self, text: &str) -> String {
        self.convert(text)
    }
}

/// 讀音能決定簡體寫法、而 OpenCC 依詞面常判斷錯的字。
///
/// 「著」讀 ㄓㄨˋ 時簡體仍是「著」；「乾」讀 ㄑㄧㄢˊ 留「乾」、讀 ㄍㄢ 才是
/// 「干」；「瞭」讀 ㄌㄧㄠˋ 留「瞭」、讀 ㄌㄧㄠˇ 是「了」；「沈」讀 ㄔㄣˊ
/// 是「沉」、讀 ㄕㄣˇ（姓）留「沈」。
pub fn pin(c: char, reading: &str) -> Option<&'static str> {
    match (c, reading) {
        ('著', "ㄓㄨˋ") => Some("著"),
        ('乾', "ㄑㄧㄢˊ") => Some("乾"),
        ('乾', "ㄍㄢ") => Some("干"),
        ('瞭', "ㄌㄧㄠˋ") => Some("瞭"),
        ('瞭', "ㄌㄧㄠˇ") => Some("了"),
        ('沈', "ㄔㄣˊ") => Some("沉"),
        ('沈', "ㄕㄣˇ") => Some("沈"),
        _ => None,
    }
}

/// `pin` 用到的字；方便呼叫端先判斷需不需要算讀音。
pub fn pinned_char(c: char) -> bool {
    matches!(c, '著' | '乾' | '瞭' | '沈')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 迷你字典：真正的上游字典與 OpenCC 的對照在資料根目錄的 `opencc/`。
    fn tiny() -> Converter {
        tw2s(Data {
            cjk_compatibility: "# 相容表意字\n\u{F900}\t豈\n",
            tw_variants: "着\t著\n樑\t梁 樑\n# @reverse-prefer: 梁\n梁\t梁\n",
            tw_variants_rev_phrases: "什么\t什麼\n顯著\t顯著\n",
            ts_phrases: "顯著\t显著\n計畫\t计划\n著名\t著名\n",
            ts_characters:
                "顯\t显\n計\t计\n畫\t画 划\n麼\t么 麽\n乾\t干 乾\n梁\t梁\n名\t名\n豈\t岂\n",
        })
        .expect("迷你字典")
    }

    #[test]
    fn test_convert_follows_opencc_matching() {
        let c = tiny();
        for (input, want) in [
            ("顯著", "显著"),
            ("看著", "看着"),
            ("計畫", "计划"),
            ("畫", "画"),
            ("什麼", "什么"),
            ("梁", "梁"),
            ("\u{F900}", "岂"),
            ("abc 123！", "abc 123！"),
            ("", ""),
        ] {
            assert_eq!(c.convert(input), want, "{input:?}");
        }
    }

    #[test]
    fn test_pinned_positions_keep_their_output_and_split_segments() {
        let c = tiny();
        let pins = HashMap::from([(1usize, "著".to_string())]);
        assert_eq!(c.convert_pinned("顯著看著", &pins), "显著看着");
        let pins = HashMap::from([(0usize, "乾".to_string())]);
        assert_eq!(c.convert_pinned("乾乾", &pins), "乾干");
        let pins = HashMap::from([(5usize, "乾".to_string())]);
        assert_eq!(c.convert_pinned("乾", &pins), "干");
    }

    #[test]
    fn test_pin_table() {
        for (c, reading, want) in [
            ('著', "ㄓㄨˋ", Some("著")),
            ('著', "ㄓㄜ˙", None),
            ('乾', "ㄑㄧㄢˊ", Some("乾")),
            ('乾', "ㄍㄢ", Some("干")),
            ('瞭', "ㄌㄧㄠˇ", Some("了")),
            ('瞭', "ㄌㄧㄠˋ", Some("瞭")),
            ('沈', "ㄔㄣˊ", Some("沉")),
            ('沈', "ㄕㄣˇ", Some("沈")),
            ('畫', "ㄏㄨㄚˋ", None),
        ] {
            assert_eq!(pin(c, reading), want, "{c} {reading}");
        }
    }

    #[test]
    fn test_parse_errors() {
        assert!(parse_dict("沒有分隔\n").is_err());
        assert!(parse_reversed("# @reverse-prefer: 梁 樑\n梁\t梁\n").is_err());
        assert!(parse_reversed("# @reverse-prefer: 无\n樑\t梁\n").is_err());
        assert!(tw2s(Data::default()).is_err());
    }

    #[test]
    fn test_reverse_prefer_picks_listed_key() {
        let dict = parse_reversed("樑\t梁 樑\n# @reverse-prefer: 梁 樑\n梁\t梁\n").unwrap();
        assert_eq!(dict.map.get("梁").map(String::as_str), Some("樑"));
    }

    #[test]
    fn test_real_catalog_converts_taiwan_forms() {
        let Some(root) = jc_testdata::root() else {
            eprintln!("缺少測試資料：JC_TESTDATA");
            return;
        };
        let dir = root.join("opencc");
        if !dir.is_dir() {
            eprintln!("缺少測試資料：opencc/");
            return;
        }
        let c = tw2s_from_dir(&dir).expect("讀取 OpenCC 字典");
        assert_eq!(c.convert("龍風學國"), "龙风学国");
        assert_eq!(c.convert("臺灣"), "台湾");
        assert_eq!(c.convert("裡面"), "里面");
        assert_eq!(c.convert("軟體"), "软体");
        // 只有一本字典的詞條數可檢查真的讀到檔案。
        assert!(c.segment.len() > 1000);
    }
}
