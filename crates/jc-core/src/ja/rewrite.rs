//! 日文輸入正規化與轉換後的補助候選。
//!
//! 前半是輸入正規化：把使用者輸入的かな、半角カナ與英數轉成一致的表示法。
//! 後半是輸出端改寫：かな表記、全角／半角英數、數字表記、記号與顔文字
//! 候選，以及把它們接在主要候選之後的 [`append`]。
//!
//! 這個 module 不改轉換演算法，也不重新排序主要 N-best。外殼先取得
//! `ja::decode` 的主要候選，再呼叫 [`Rewriter::candidates`] 產生補助候選，
//! 最後用 [`append`] 接在後面。
//!
//! 舊假名 `ゐ`、`ゑ`、`を` 與前身一樣保持原樣，不轉成現代假名。

use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::candidate::{Candidate, CandidateSource};

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

/// 補助候選的來源字串，對齊前身的 `rewrite.SourceRewrite`。
pub const SOURCE_REWRITE: &str = "rewrite";

/// 記号與顔文字候選的來源字串，對齊前身的 `lang.SourceSymbol`。
pub const SOURCE_SYMBOL: &str = "symbol";

/// 單次 [`Rewriter::candidates`] 最多回傳的補助候選數。
pub const DEFAULT_MAX_CANDIDATES: usize = 40;

/// かな候選的註解。
pub const ANNOTATION_HIRAGANA: &str = "ひらがな";
/// かな候選的註解。
pub const ANNOTATION_KATAKANA: &str = "カタカナ";
/// かな候補的註解。
pub const ANNOTATION_HALFWIDTH_KANA: &str = "半角カタカナ";
/// 全角英數候選的註解。
pub const ANNOTATION_FULLWIDTH_LATIN: &str = "全角英数";
/// 半角英數候選的註解。
pub const ANNOTATION_HALFWIDTH_LATIN: &str = "半角英数";

/// 數字表記候選的註解，字串與 Mozc 的候選描述一致。
pub const ANNOTATION_KANJI: &str = "漢数字";
/// 數字表記候選的註解。
pub const ANNOTATION_DAIJI: &str = "大字";
/// 數字表記候選的註解。
pub const ANNOTATION_CIRCLED: &str = "丸数字";
/// 數字表記候選的註解。
pub const ANNOTATION_ROMAN: &str = "ローマ数字(大文字)";
/// 數字表記候選的註解。
pub const ANNOTATION_ROMAN_SMALL: &str = "ローマ数字(小文字)";
/// 數字表記候選的註解。
pub const ANNOTATION_HEX: &str = "16進数";
/// 數字表記候選的註解。
pub const ANNOTATION_OCT: &str = "8進数";
/// 數字表記候選的註解。
pub const ANNOTATION_BIN: &str = "2進数";

/// 補助候選的來源。
///
/// 核心的 [`CandidateSource`] 沒有補助候選的概念，這裡保留核心的四種來源
/// 再加上 [`Source::Rewrite`] 與 [`Source::Symbol`]，讓 [`append`] 回傳的
/// 清單同時涵蓋主要候選與補助候選。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// 系統詞典。
    Lexicon,
    /// 使用者詞庫或學習結果。
    Personal,
    /// 由前文預測。
    Predicted,
    /// 原樣輸出的字面。
    Literal,
    /// かな、英數寬度與數字表記候選。
    Rewrite,
    /// 記号與顔文字辭典。
    Symbol,
}

impl Source {
    /// 來源的穩定字串形式。
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Lexicon => "lexicon",
            Source::Personal => "personal",
            Source::Predicted => "predicted",
            Source::Literal => "literal",
            Source::Rewrite => SOURCE_REWRITE,
            Source::Symbol => SOURCE_SYMBOL,
        }
    }
}

impl From<CandidateSource> for Source {
    fn from(source: CandidateSource) -> Self {
        match source {
            CandidateSource::Lexicon => Source::Lexicon,
            CandidateSource::Personal => Source::Personal,
            CandidateSource::Predicted => Source::Predicted,
            CandidateSource::Literal => Source::Literal,
        }
    }
}

impl From<Source> for CandidateSource {
    fn from(source: Source) -> Self {
        match source {
            Source::Lexicon => CandidateSource::Lexicon,
            Source::Personal => CandidateSource::Personal,
            Source::Predicted => CandidateSource::Predicted,
            Source::Literal | Source::Rewrite | Source::Symbol => CandidateSource::Literal,
        }
    }
}

/// 一個補助候選。
///
/// 核心的 [`Candidate`] 沒有註解欄位，補助候選需要顯示「ひらがな」、
/// 「米印」等說明，因此獨立成這個型別。`cost` 沿用核心語意：越小越優先。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteCandidate {
    /// 輸出字面。
    pub surface: String,
    /// 對應讀音。
    pub reading: String,
    /// 路徑成本，越小越優先。
    pub cost: i32,
    /// 候選來源。
    pub source: Source,
    /// 候選視窗顯示的註解。
    pub annotation: String,
}

impl RewriteCandidate {
    /// 建立補助候選。
    pub fn new(
        surface: impl Into<String>,
        reading: impl Into<String>,
        cost: i32,
        source: Source,
        annotation: impl Into<String>,
    ) -> Self {
        Self {
            surface: surface.into(),
            reading: reading.into(),
            cost,
            source,
            annotation: annotation.into(),
        }
    }
}

/// 產生日文轉換的補助候選。
///
/// 零值可用，但不含記号辭典；通常用 [`Rewriter::new`] 或
/// [`Rewriter::with_symbols`] 建立。
#[derive(Debug, Clone, Default)]
pub struct Rewriter {
    /// 記号與顔文字辭典；`None` 表示不產生這類候選。
    pub symbols: Option<Symbols>,
    /// 單次輸出的上限；0 表示 [`DEFAULT_MAX_CANDIDATES`]。
    pub max_candidates: usize,
}

impl Rewriter {
    /// 建立不含記号辭典的 rewriter。
    pub fn new() -> Self {
        Self {
            symbols: None,
            max_candidates: DEFAULT_MAX_CANDIDATES,
        }
    }

    /// 建立含記号與顔文字辭典的 rewriter。
    pub fn with_symbols(symbols: Symbols) -> Self {
        Self {
            symbols: Some(symbols),
            max_candidates: DEFAULT_MAX_CANDIDATES,
        }
    }

    /// 依詞面與讀音產生補助候選。
    ///
    /// かな候選來自讀音，全角／半角英數與數字候選來自詞面，記号與顔文字
    /// 來自 [`Symbols`] 的読み查詢。回傳值不含主要轉換候選；外殼用
    /// [`append`] 把它們接在主要候選之後。相同詞面只保留第一筆。
    pub fn candidates(&self, surface: &str, reading: &str) -> Vec<RewriteCandidate> {
        let mut out: Vec<RewriteCandidate> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut add = |text: String, annotation: &str, source: Source| {
            if text.is_empty() || text == surface {
                return;
            }
            if !seen.insert(text.clone()) {
                return;
            }
            out.push(RewriteCandidate::new(text, reading, 0, source, annotation));
        };

        let kana = reading;
        if has_kana(kana) {
            add(to_hiragana(kana), ANNOTATION_HIRAGANA, Source::Rewrite);
            add(to_katakana(kana), ANNOTATION_KATAKANA, Source::Rewrite);
            add(
                to_halfwidth_kana(kana),
                ANNOTATION_HALFWIDTH_KANA,
                Source::Rewrite,
            );
        }

        if has_ascii_alnum(surface) {
            add(
                to_fullwidth_latin(surface),
                ANNOTATION_FULLWIDTH_LATIN,
                Source::Rewrite,
            );
        }
        if has_fullwidth_alnum(surface) {
            add(
                to_halfwidth_latin(surface),
                ANNOTATION_HALFWIDTH_LATIN,
                Source::Rewrite,
            );
        }

        for (text, annotation) in number_candidates(surface) {
            add(text, annotation, Source::Rewrite);
        }

        if let Some(symbols) = &self.symbols {
            for symbol in symbols.lookup(kana) {
                add(symbol.surface.clone(), &symbol.annotation, Source::Symbol);
            }
        }

        let limit = if self.max_candidates == 0 {
            DEFAULT_MAX_CANDIDATES
        } else {
            self.max_candidates
        };
        if out.len() > limit {
            out.truncate(limit);
        }
        out
    }
}

/// 把補助候選接在主要候選之後，主要候選的順序與成本不變。
///
/// 補助候選若與主要候選同詞面會略過；成本會被設在主要候選最高成本之上，
/// 這樣即使外殼依成本重排，主要 N-best 仍然在前。回傳新清單，不修改輸入。
pub fn append(primary: &[Candidate], extra: &[RewriteCandidate]) -> Vec<RewriteCandidate> {
    let mut out = Vec::with_capacity(primary.len() + extra.len());
    let mut seen: HashSet<&str> = HashSet::new();
    let mut ceiling: i32 = 0;
    let mut have_primary = false;
    for candidate in primary {
        out.push(RewriteCandidate::new(
            candidate.surface.clone(),
            candidate.reading.clone(),
            candidate.cost,
            Source::from(candidate.source),
            "",
        ));
        if !candidate.surface.is_empty() {
            seen.insert(candidate.surface.as_str());
        }
        if !have_primary || candidate.cost > ceiling {
            ceiling = candidate.cost;
            have_primary = true;
        }
    }

    let mut added: i32 = 0;
    for candidate in extra {
        if candidate.surface.is_empty() {
            continue;
        }
        if !seen.insert(candidate.surface.as_str()) {
            continue;
        }
        added += 1;
        let mut candidate = candidate.clone();
        candidate.cost = if have_primary {
            ceiling.saturating_add(added)
        } else {
            added
        };
        out.push(candidate);
    }
    out
}

/// 是否含有かな字元。
fn has_kana(s: &str) -> bool {
    s.chars().any(|r| {
        matches!(r,
            'ぁ'..='ゖ'
            | 'ァ'..='ヺ'
            | 'ｦ'..='ﾟ'
            | 'ー' | '゛' | '゜' | 'ゝ' | 'ゞ' | 'ヽ' | 'ヾ'
        )
    })
}

/// 是否含有 ASCII 英數字。
fn has_ascii_alnum(s: &str) -> bool {
    s.chars()
        .any(|r| r.is_ascii_digit() || r.is_ascii_uppercase() || r.is_ascii_lowercase())
}

/// 是否含有全角英數字。
fn has_fullwidth_alnum(s: &str) -> bool {
    s.chars().any(|r| {
        matches!(r,
            '０'..='９' | 'Ａ'..='Ｚ' | 'ａ'..='ｚ'
        )
    })
}

const KANJI_DIGITS: [&str; 10] = ["〇", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
const DAIJI_DIGITS: [&str; 10] = ["", "壱", "弐", "参", "四", "五", "六", "七", "八", "九"];
const KANJI_BIG_UNITS: [&str; 5] = ["", "万", "億", "兆", "京"];
const DAIJI_BIG_UNITS: [&str; 5] = ["", "萬", "億", "兆", "京"];
const ROMAN_CAPITAL: [&str; 13] = [
    "", "Ⅰ", "Ⅱ", "Ⅲ", "Ⅳ", "Ⅴ", "Ⅵ", "Ⅶ", "Ⅷ", "Ⅸ", "Ⅹ", "Ⅺ", "Ⅻ",
];
const ROMAN_SMALL: [&str; 13] = [
    "", "ⅰ", "ⅱ", "ⅲ", "ⅳ", "ⅴ", "ⅵ", "ⅶ", "ⅷ", "ⅸ", "ⅹ", "ⅺ", "ⅻ",
];
const CIRCLED_NUMBERS: [&str; 51] = [
    "⓪", "①", "②", "③", "④", "⑤", "⑥", "⑦", "⑧", "⑨", "⑩", "⑪", "⑫", "⑬", "⑭", "⑮", "⑯", "⑰", "⑱",
    "⑲", "⑳", "㉑", "㉒", "㉓", "㉔", "㉕", "㉖", "㉗", "㉘", "㉙", "㉚", "㉛", "㉜", "㉝", "㉞",
    "㉟", "㊱", "㊲", "㊳", "㊴", "㊵", "㊶", "㊷", "㊸", "㊹", "㊺", "㊻", "㊼", "㊽", "㊾", "㊿",
];

/// 回傳詞面的數字表記候選；詞面不是純阿拉伯數字時回傳空清單。
///
/// 支援半角與全角阿拉伯數字。輸出順序與 Mozc 的 NumberRewriter 相近：
/// 漢数字、大字、丸数字、羅馬數字、16／8／2 進位。
fn number_candidates(surface: &str) -> Vec<(String, &'static str)> {
    let Some(n) = parse_arabic(surface) else {
        return Vec::new();
    };
    let mut out: Vec<(String, &'static str)> = Vec::new();
    let mut add = |text: String, annotation: &'static str| out.push((text, annotation));

    add(kanji_number(n, false), ANNOTATION_KANJI);
    add(kanji_number(n, true), ANNOTATION_DAIJI);

    if (n as usize) < CIRCLED_NUMBERS.len() {
        add(CIRCLED_NUMBERS[n as usize].to_string(), ANNOTATION_CIRCLED);
    }
    if n > 0 && (n as usize) < ROMAN_CAPITAL.len() {
        add(ROMAN_CAPITAL[n as usize].to_string(), ANNOTATION_ROMAN);
        add(ROMAN_SMALL[n as usize].to_string(), ANNOTATION_ROMAN_SMALL);
    }

    // Mozc 只在數字大到需要另一種進位才有意義時才列進位候選。
    if n > 9 {
        add(format!("0x{n:x}"), ANNOTATION_HEX);
    }
    if n > 7 {
        add(format!("0{n:o}"), ANNOTATION_OCT);
    }
    if n > 1 {
        add(format!("0b{n:b}"), ANNOTATION_BIN);
    }
    out
}

/// 解析純阿拉伯數字；全角數字先轉成半角。溢位或含其他字元回傳 `None`。
fn parse_arabic(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let mut digits = String::with_capacity(s.len());
    for r in s.chars() {
        match r {
            '0'..='9' => digits.push(r),
            '０'..='９' => digits.push(char::from_u32(r as u32 - '０' as u32 + '0' as u32)?),
            _ => return None,
        }
    }
    digits.parse().ok()
}

/// 把整數轉成漢數字或大字。
///
/// 一般漢數字省略十、百、千前面的「一」；大字則保留「壱」，並使用阡、萬
/// 等舊字，例如 123 → 壱百弐拾参。
fn kanji_number(mut n: u64, daiji: bool) -> String {
    if n == 0 {
        return "零".to_string();
    }
    let mut groups = [0u64; 5];
    for group in &mut groups {
        *group = n % 10000;
        n /= 10000;
    }

    let mut out = String::new();
    for i in (0..groups.len()).rev() {
        if groups[i] == 0 {
            continue;
        }
        out.push_str(&format_group(groups[i], daiji));
        out.push_str(if daiji {
            DAIJI_BIG_UNITS[i]
        } else {
            KANJI_BIG_UNITS[i]
        });
    }
    out
}

/// 格式化一萬以下的四位數。
fn format_group(mut n: u64, daiji: bool) -> String {
    const UNITS: [(u64, &str, &str); 4] = [
        (1000, "千", "阡"),
        (100, "百", "百"),
        (10, "十", "拾"),
        (1, "", ""),
    ];
    let mut out = String::new();
    for (value, rank, old) in UNITS {
        let digit = n / value;
        n -= digit * value;
        if digit == 0 {
            continue;
        }
        if daiji {
            out.push_str(DAIJI_DIGITS[digit as usize]);
            out.push_str(old);
            continue;
        }
        if digit != 1 || value == 1 {
            out.push_str(KANJI_DIGITS[digit as usize]);
        }
        out.push_str(rank);
    }
    out
}

/// 記号或顔文字辭典的一筆。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// 查詢用的読み，例如「きごう」「かおもじ」。
    pub reading: String,
    /// 候選詞面。
    pub surface: String,
    /// 候選視窗顯示的註解，例如「矢印」「顔文字」。
    pub annotation: String,
    /// 辭典分類，只用於整理資料，不進入候選。
    pub category: String,
}

/// 讀取記号辭典時的錯誤。
#[derive(Debug, thiserror::Error)]
pub enum SymbolsError {
    /// 欄位數不是四欄。
    #[error("symbols: 第 {line} 行有 {got} 欄，需要 4 欄")]
    Fields {
        /// 行號，從 1 開始。
        line: usize,
        /// 實際欄位數。
        got: usize,
    },
    /// 讀音或詞面為空。
    #[error("symbols: 第 {line} 行的讀音或詞面為空")]
    EmptyField {
        /// 行號，從 1 開始。
        line: usize,
    },
    /// 註解為空。
    #[error("symbols: 第 {line} 行的註解為空")]
    EmptyAnnotation {
        /// 行號，從 1 開始。
        line: usize,
    },
    /// 讀取檔案失敗。
    #[error("symbols: {path}: {source}")]
    Io {
        /// 檔案路徑。
        path: String,
        /// 原始錯誤。
        source: std::io::Error,
    },
    /// 目錄下沒有 `.tsv`。
    #[error("symbols: {dir} 沒有 .tsv 檔")]
    NoFiles {
        /// 目錄路徑。
        dir: String,
    },
}

/// 依読み查詢的記号與顔文字辭典。
///
/// 建好後不再修改，可以安全共用。零值等於空辭典。
#[derive(Debug, Clone, Default)]
pub struct Symbols {
    by_reading: std::collections::HashMap<String, Vec<Symbol>>,
    count: usize,
}

impl Symbols {
    /// 建立辭典。空白読み或空白詞面的項目會被忽略。
    pub fn new(entries: impl IntoIterator<Item = Symbol>) -> Self {
        let mut symbols = Self::default();
        symbols.add(entries);
        symbols
    }

    /// 追加項目；方便外殼載入使用者自訂辭典。
    pub fn add(&mut self, entries: impl IntoIterator<Item = Symbol>) {
        for mut entry in entries {
            entry.reading = normalize_symbol_reading(&entry.reading);
            if entry.reading.is_empty() || entry.surface.is_empty() {
                continue;
            }
            self.by_reading
                .entry(entry.reading.clone())
                .or_default()
                .push(entry);
            self.count += 1;
        }
    }

    /// 辭典項目數。
    pub fn len(&self) -> usize {
        self.count
    }

    /// 辭典是否為空。
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// 回傳読み對應的項目；読み會先轉成平假名並移除空白。
    pub fn lookup(&self, reading: &str) -> &[Symbol] {
        self.by_reading
            .get(&normalize_symbol_reading(reading))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

/// 正規化辭典的読み：移除空白與 TAB，再轉成平假名。
fn normalize_symbol_reading(reading: &str) -> String {
    let compact: String = reading
        .chars()
        .filter(|r| *r != ' ' && *r != '\t')
        .collect();
    to_hiragana(&compact)
}

/// 從 TSV 讀取辭典。
///
/// 每行四欄：読み、詞面、註解、分類，以 TAB 分隔；空行與 `#` 開頭的行
/// 忽略。同一読み可以有多筆。回傳的項目保持檔案順序。
pub fn load_symbols(reader: impl BufRead) -> Result<Vec<Symbol>, SymbolsError> {
    let mut entries = Vec::new();
    for (index, line) in reader.lines().enumerate() {
        let line_no = index + 1;
        let line = line.map_err(|source| SymbolsError::Io {
            path: format!("第 {line_no} 行"),
            source,
        })?;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 4 {
            return Err(SymbolsError::Fields {
                line: line_no,
                got: fields.len(),
            });
        }
        let entry = Symbol {
            reading: fields[0].trim().to_string(),
            surface: fields[1].to_string(),
            annotation: fields[2].trim().to_string(),
            category: fields[3].trim().to_string(),
        };
        if entry.reading.is_empty() || entry.surface.is_empty() {
            return Err(SymbolsError::EmptyField { line: line_no });
        }
        if entry.annotation.is_empty() {
            return Err(SymbolsError::EmptyAnnotation { line: line_no });
        }
        entries.push(entry);
    }
    Ok(entries)
}

/// 讀取單一 TSV 檔。
pub fn load_symbols_file(path: impl AsRef<Path>) -> Result<Vec<Symbol>, SymbolsError> {
    let path = path.as_ref();
    let file = std::fs::File::open(path).map_err(|source| SymbolsError::Io {
        path: path.display().to_string(),
        source,
    })?;
    load_symbols(BufReader::new(file)).map_err(|source| match source {
        SymbolsError::Io { source, .. } => SymbolsError::Io {
            path: path.display().to_string(),
            source,
        },
        other => other,
    })
}

/// 讀取目錄下所有 `.tsv` 檔並合併成一份辭典。
///
/// 檔名排序後依序載入，因此同一読み的候選順序可重現。核心不假設目錄結構，
/// 外殼或測試自行提供資料目錄。
pub fn load_symbols_dir(dir: impl AsRef<Path>) -> Result<Symbols, SymbolsError> {
    let dir = dir.as_ref();
    let read = std::fs::read_dir(dir).map_err(|source| SymbolsError::Io {
        path: dir.display().to_string(),
        source,
    })?;
    let mut names = Vec::new();
    for entry in read {
        let entry = entry.map_err(|source| SymbolsError::Io {
            path: dir.display().to_string(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("tsv") {
            names.push(path);
        }
    }
    names.sort();
    if names.is_empty() {
        return Err(SymbolsError::NoFiles {
            dir: dir.display().to_string(),
        });
    }
    let mut entries = Vec::new();
    for name in names {
        entries.extend(load_symbols_file(name)?);
    }
    Ok(Symbols::new(entries))
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

    fn surfaces(cands: &[RewriteCandidate]) -> Vec<&str> {
        cands.iter().map(|c| c.surface.as_str()).collect()
    }

    fn annotations_by_surface(cands: &[RewriteCandidate]) -> std::collections::HashMap<&str, &str> {
        cands
            .iter()
            .map(|c| (c.surface.as_str(), c.annotation.as_str()))
            .collect()
    }

    #[test]
    fn test_append_keeps_primary_first() {
        let primary = vec![
            Candidate::new("今日", "", 100, CandidateSource::Lexicon),
            Candidate::new("きょう", "", 20, CandidateSource::Lexicon),
        ];
        let extra = vec![
            RewriteCandidate::new("キョウ", "", 999, Source::Rewrite, ANNOTATION_KATAKANA),
            RewriteCandidate::new("今日", "", 999, Source::Rewrite, "duplicate"),
            RewriteCandidate::new("ｷｮｳ", "", 999, Source::Rewrite, ANNOTATION_HALFWIDTH_KANA),
        ];

        let got = append(&primary, &extra);
        assert_eq!(got.len(), 4);
        assert_eq!(got[0].surface, "今日");
        assert_eq!(got[0].cost, 100);
        assert_eq!(got[0].source, Source::Lexicon);
        assert_eq!(got[1].surface, "きょう");
        assert_eq!(got[1].cost, 20);
        assert_eq!(got[2].surface, "キョウ");
        assert!(got[2].cost > 20, "補助候選成本要在主要候選之後");
        assert_eq!(got[3].surface, "ｷｮｳ");
        assert!(got[3].cost > got[2].cost);
        // 不修改輸入。
        assert_eq!(primary[0].cost, 100);
        assert_eq!(primary[1].cost, 20);
    }

    #[test]
    fn test_append_without_primary() {
        let extra = vec![
            RewriteCandidate::new("ア", "", 0, Source::Rewrite, ""),
            RewriteCandidate::new("", "", 0, Source::Rewrite, ""),
            RewriteCandidate::new("イ", "", 0, Source::Rewrite, ""),
        ];
        let got = append(&[], &extra);
        assert_eq!(surfaces(&got), vec!["ア", "イ"]);
        assert!(got[0].cost > 0 && got[1].cost > got[0].cost);
    }

    #[test]
    fn test_candidates_order_and_source() {
        let rewriter = Rewriter::new();
        let cands = rewriter.candidates("abc123", "えーびーしー");
        let got = surfaces(&cands);
        assert!(got.len() >= 3, "候選太少：{got:?}");
        assert_eq!(&got[..3], &["えーびーしー", "エービーシー", "ｴｰﾋﾞｰｼｰ"]);
        for candidate in &cands[..3] {
            assert_eq!(candidate.source, Source::Rewrite);
        }
        let fullwidth = cands
            .iter()
            .position(|c| c.annotation == ANNOTATION_FULLWIDTH_LATIN)
            .expect("缺少全角英數候選");
        assert_eq!(fullwidth, 3);
    }

    #[test]
    fn test_candidates_reads_reading_and_copies() {
        let cands = Rewriter::new().candidates("今日", "きょう");
        assert!(!cands.is_empty());
        for candidate in &cands {
            assert_eq!(candidate.reading, "きょう");
        }
    }

    #[test]
    fn test_max_candidates() {
        let mut rewriter = Rewriter {
            max_candidates: 2,
            ..Rewriter::default()
        };
        let cands = rewriter.candidates("abc123", "えーびーしー");
        assert_eq!(cands.len(), 2, "{:?}", surfaces(&cands));

        rewriter.max_candidates = 0;
        let cands = rewriter.candidates("abc123", "えーびーしー");
        assert!(cands.len() > 2);
    }

    #[test]
    fn test_nil_symbols() {
        let rewriter = Rewriter::default();
        let cands = rewriter.candidates("記号", "きごう");
        assert_eq!(cands.len(), 3, "{:?}", surfaces(&cands));
    }

    #[test]
    fn test_number_candidates_table() {
        struct Case {
            input: &'static str,
            want: &'static [(&'static str, &'static str)],
            absent: &'static [&'static str],
        }
        let cases = [
            Case {
                input: "123",
                want: &[
                    ("百二十三", ANNOTATION_KANJI),
                    ("壱百弐拾参", ANNOTATION_DAIJI),
                    ("0x7b", ANNOTATION_HEX),
                    ("0173", ANNOTATION_OCT),
                    ("0b1111011", ANNOTATION_BIN),
                ],
                absent: &[ANNOTATION_CIRCLED, ANNOTATION_ROMAN],
            },
            Case {
                input: "12",
                want: &[
                    ("十二", ANNOTATION_KANJI),
                    ("壱拾弐", ANNOTATION_DAIJI),
                    ("⑫", ANNOTATION_CIRCLED),
                    ("Ⅻ", ANNOTATION_ROMAN),
                    ("ⅻ", ANNOTATION_ROMAN_SMALL),
                    ("0xc", ANNOTATION_HEX),
                    ("014", ANNOTATION_OCT),
                    ("0b1100", ANNOTATION_BIN),
                ],
                absent: &[],
            },
            Case {
                input: "0",
                want: &[("零", ANNOTATION_KANJI), ("⓪", ANNOTATION_CIRCLED)],
                absent: &[
                    ANNOTATION_DAIJI,
                    ANNOTATION_ROMAN,
                    ANNOTATION_HEX,
                    ANNOTATION_OCT,
                    ANNOTATION_BIN,
                ],
            },
            Case {
                input: "50",
                want: &[
                    ("五十", ANNOTATION_KANJI),
                    ("五拾", ANNOTATION_DAIJI),
                    ("㊿", ANNOTATION_CIRCLED),
                    ("0x32", ANNOTATION_HEX),
                    ("062", ANNOTATION_OCT),
                    ("0b110010", ANNOTATION_BIN),
                ],
                absent: &[ANNOTATION_ROMAN],
            },
            Case {
                input: "51",
                want: &[
                    ("五十一", ANNOTATION_KANJI),
                    ("五拾壱", ANNOTATION_DAIJI),
                    ("0x33", ANNOTATION_HEX),
                    ("063", ANNOTATION_OCT),
                    ("0b110011", ANNOTATION_BIN),
                ],
                absent: &[ANNOTATION_CIRCLED, ANNOTATION_ROMAN],
            },
            Case {
                input: "1000",
                want: &[("千", ANNOTATION_KANJI), ("壱阡", ANNOTATION_DAIJI)],
                absent: &[],
            },
            Case {
                input: "10000",
                want: &[("一万", ANNOTATION_KANJI), ("壱萬", ANNOTATION_DAIJI)],
                absent: &[],
            },
            Case {
                input: "123456789",
                want: &[
                    ("一億二千三百四十五万六千七百八十九", ANNOTATION_KANJI),
                    ("壱億弐阡参百四拾五萬六阡七百八拾九", ANNOTATION_DAIJI),
                    ("0x75bcd15", ANNOTATION_HEX),
                    ("0726746425", ANNOTATION_OCT),
                    ("0b111010110111100110100010101", ANNOTATION_BIN),
                ],
                absent: &[ANNOTATION_CIRCLED, ANNOTATION_ROMAN],
            },
            Case {
                input: "１２０",
                want: &[
                    ("百二十", ANNOTATION_KANJI),
                    ("壱百弐拾", ANNOTATION_DAIJI),
                    ("0x78", ANNOTATION_HEX),
                    ("0170", ANNOTATION_OCT),
                    ("0b1111000", ANNOTATION_BIN),
                ],
                absent: &[],
            },
            Case {
                input: "abc123",
                want: &[],
                absent: &[
                    ANNOTATION_KANJI,
                    ANNOTATION_DAIJI,
                    ANNOTATION_CIRCLED,
                    ANNOTATION_ROMAN,
                    ANNOTATION_HEX,
                    ANNOTATION_OCT,
                    ANNOTATION_BIN,
                ],
            },
            Case {
                // uint64 溢位
                input: "18446744073709551616",
                want: &[],
                absent: &[
                    ANNOTATION_KANJI,
                    ANNOTATION_DAIJI,
                    ANNOTATION_CIRCLED,
                    ANNOTATION_ROMAN,
                    ANNOTATION_HEX,
                ],
            },
        ];

        for case in cases {
            let cands = Rewriter::new().candidates(case.input, "");
            let got = annotations_by_surface(&cands);
            for (surface, annotation) in case.want {
                assert_eq!(
                    got.get(surface).copied(),
                    Some(*annotation),
                    "{}: 候選 {surface:?} 的註解不對，全部：{got:?}",
                    case.input
                );
            }
            for annotation in case.absent {
                for (surface, got_annotation) in &got {
                    assert_ne!(
                        got_annotation, annotation,
                        "{}: 不該出現 {annotation} 候選 {surface:?}",
                        case.input
                    );
                }
            }
        }
    }

    #[test]
    fn test_number_candidates_order() {
        let cands = Rewriter::new().candidates("12", "");
        let want = [
            "１２",
            "十二",
            "壱拾弐",
            "⑫",
            "Ⅻ",
            "ⅻ",
            "0xc",
            "014",
            "0b1100",
        ];
        assert_eq!(surfaces(&cands), want);
    }

    #[test]
    fn test_mozc_comparison_cases() {
        let cases = [
            (
                "123",
                vec!["百二十三", "壱百弐拾参", "0x7b", "0173", "0b1111011"],
            ),
            (
                "12",
                vec![
                    "１２",
                    "十二",
                    "壱拾弐",
                    "⑫",
                    "Ⅻ",
                    "ⅻ",
                    "0xc",
                    "014",
                    "0b1100",
                ],
            ),
            ("1000", vec!["千", "壱阡"]),
            ("10000", vec!["一万", "壱萬"]),
            (
                "123456789",
                vec![
                    "一億二千三百四十五万六千七百八十九",
                    "壱億弐阡参百四拾五萬六阡七百八拾九",
                    "0x75bcd15",
                    "0726746425",
                    "0b111010110111100110100010101",
                ],
            ),
        ];
        for (input, want) in cases {
            let cands = Rewriter::new().candidates(input, "");
            let got = surfaces(&cands);
            for surface in want {
                assert!(
                    got.contains(&surface),
                    "{input}: 缺少 {surface:?}，全部：{got:?}"
                );
            }
        }
    }

    #[test]
    fn test_load_symbols_fs() {
        let dir = jc_testdata::require_data!("ja-src/symbols");
        let symbols = load_symbols_dir(dir).expect("載入記号辭典");
        assert!(symbols.len() >= 150, "只有 {} 筆", symbols.len());

        for (reading, surface, annotation) in [
            ("きごう", "※", "米印"),
            ("やじるし", "→", "右矢印"),
            ("すうがく", "∞", "無限大"),
            ("ずけい", "★", "黒星"),
            ("つうか", "￥", "円"),
            ("たんい", "℃", "摂氏"),
            ("かおもじ", "(^^)", "笑顔の顔文字"),
            ("キゴウ", "※", "米印"),
        ] {
            let found = symbols
                .lookup(reading)
                .iter()
                .any(|entry| entry.surface == surface && entry.annotation == annotation);
            assert!(
                found,
                "Lookup({reading:?}) 缺少 {surface:?}（{annotation:?}）"
            );
        }

        let kaomoji = symbols.lookup("かおもじ");
        assert!(kaomoji.len() >= 40, "只有 {} 筆顔文字", kaomoji.len());
        for entry in kaomoji {
            assert_eq!(entry.category, "顔文字");
            assert!(!entry.annotation.is_empty());
        }
    }

    #[test]
    fn test_load_symbols_table() {
        let valid = "# comment\n\nきごう\t※\t米印\t記号\nやじるし\t→\t右矢印\t記号\n";
        let got = load_symbols(std::io::Cursor::new(valid)).expect("解析成功");
        assert_eq!(
            got,
            vec![
                Symbol {
                    reading: "きごう".to_string(),
                    surface: "※".to_string(),
                    annotation: "米印".to_string(),
                    category: "記号".to_string(),
                },
                Symbol {
                    reading: "やじるし".to_string(),
                    surface: "→".to_string(),
                    annotation: "右矢印".to_string(),
                    category: "記号".to_string(),
                },
            ]
        );

        let err = load_symbols(std::io::Cursor::new("きごう\t※\t米印\n")).expect_err("欄位太少");
        assert!(matches!(err, SymbolsError::Fields { line: 1, got: 3 }));

        let err = load_symbols(std::io::Cursor::new("きごう\t※\t\t記号\n")).expect_err("註解為空");
        assert!(matches!(err, SymbolsError::EmptyAnnotation { line: 1 }));

        let err = load_symbols(std::io::Cursor::new("\t※\t米印\t記号\n")).expect_err("讀音為空");
        assert!(matches!(err, SymbolsError::EmptyField { line: 1 }));
    }

    #[test]
    fn test_symbol_candidates() {
        let dir = jc_testdata::require_data!("ja-src/symbols");
        let symbols = load_symbols_dir(dir).expect("載入記号辭典");
        let rewriter = Rewriter::with_symbols(symbols);

        let cands = rewriter.candidates("記号", "きごう");
        let got = annotations_by_surface(&cands);
        assert_eq!(got.get("※").copied(), Some("米印"));
        for candidate in &cands {
            assert!(!candidate.annotation.is_empty());
            assert!(matches!(candidate.source, Source::Rewrite | Source::Symbol));
        }
        let symbol = cands.iter().find(|c| c.surface == "※").expect("缺少※");
        assert_eq!(symbol.source, Source::Symbol);
        let kana = cands
            .iter()
            .find(|c| c.surface == "きごう")
            .expect("缺少きごう");
        assert_eq!(kana.source, Source::Rewrite);

        let kaomoji = rewriter.candidates("顔文字", "かおもじ");
        let got = annotations_by_surface(&kaomoji);
        assert_eq!(got.get("(^^)").copied(), Some("笑顔の顔文字"));
    }

    #[test]
    fn test_candidates_skipped_when_unchanged() {
        let cands = Rewriter::new().candidates("きょう", "きょう");
        for surface in annotations_by_surface(&cands).keys() {
            assert_ne!(
                *surface,
                "きょう",
                "同形候選不該出現：{:?}",
                surfaces(&cands)
            );
        }
    }
}
