//! 混合路由與英文直通。
//!
//! 同一段輸入可以同時含注音、日文羅馬字與英文，使用者不需要切換語言：
//!
//! 1. [`classify`] 與 [`scan`] 判斷英數詞的種類（英文、網址、email、識別字、
//!    全角英數、數字）；
//! 2. [`Router::segments`] 把輸入切成語言片段，`Segment::mode` 是該片段的語言
//!    （[`Mode::Zh`]、[`Mode::Ja`]、[`Mode::En`]），`script` 是字面種類；
//! 3. [`Router::convert`] 把每個片段交給對應的轉換器：注音走
//!    [`crate::decode::Decoder`]、日文走 [`crate::ja::decode::JaDecoder`]、
//!    英文照原樣直出；
//! 4. [`Router::detect`] 回報整段輸入的 [`Mode`]，只有單一語言時就不是
//!    [`Mode::Mixed`]。
//!
//! 判定規則沿用前身 `core/ja/mixed`：大寫、全角、數字、網址、email、識別字與
//! 學過的英文詞一律照字面；小寫且是英文詞典的完整單字也照字面；其餘能完整
//! 組成合法羅馬字的字才走日文。因此 `code`、`hello` 是英文，`sake`、
//! `watashiwa` 是羅馬字，`kubectl`、`github` 打不成合法羅馬字，也照字面。
//!
//! 語言邊界不加分隔符；游標位置以 UTF-16 code unit 計算（見
//! [`MixedResult::cursor_utf16`]）。

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use crate::candidate::{Candidate, CandidateSource};
use crate::decode::{Conversion, Decoder, Locks};
use crate::ja::decode::{JaContext, JaDecoder};
use crate::ja::input::{RomajiEvent, RomajiInput};
use crate::key::Key;
use crate::lang::Mode;
use crate::lattice::{Lattice as ZhLattice, LatticeError, Lexicon};
use crate::score::ScoreContext;
use crate::segment::{detect as detect_script, Script, Segment};
use crate::zh::keyboard::Keyboard;
use crate::zh::syllable::{class_of, Class, Composer, Syllable, TONE1};

/// 英文詞表單一詞的長度上限（位元組）。
pub const MAX_WORD_BYTES: usize = 48;

/// 單次查詢最多回傳的英文候選數。
pub const MAX_SUGGESTIONS: usize = 32;

/// 學習與候選可接受的單一英數詞長度上限。
pub const MAX_LITERAL_BYTES: usize = 96;

/// 詞表權重換算成 [`Candidate::cost`] 的基準。
///
/// [`Candidate::cost`] 越小越優先，權重越大越優先，因此 `cost = 基準 - 權重`。
const WEIGHT_BASE: i32 = 2_000_000;

/// 學習詞的權重，保證排在詞表詞前面。
const LEARNED_WEIGHT: f64 = 1_000_000.0;

// ---------------------------------------------------------------------------
// 英數詞種類
// ---------------------------------------------------------------------------

/// 一段文字在混合輸入裡的種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// 空白、標點或不屬於英數詞的字元。
    Other,
    /// 日文字母（かな、漢字、長音符等）。
    Japanese,
    /// 英文詞表或學習詞認得的英文單字。
    English,
    /// 詞表不認得、但全由拉丁字母組成的字。
    Latin,
    /// 網址。
    Url,
    /// email 位址。
    Email,
    /// 程式識別字、路徑、查詢字串或含英數的編號。
    Identifier,
    /// 全角英數字。
    FullWidthAlnum,
    /// 純數字。
    Number,
}

impl Kind {
    /// 這一類文字是否必須照原樣保留，不交給任何轉換器。
    ///
    /// 日文字母以外都算照字面；[`Kind::Other`] 由呼叫端自行處理標點。
    pub fn is_literal(self) -> bool {
        matches!(
            self,
            Kind::English
                | Kind::Latin
                | Kind::Url
                | Kind::Email
                | Kind::Identifier
                | Kind::FullWidthAlnum
                | Kind::Number
        )
    }

    /// 穩定的英文名稱，可寫進設定與測試資料。
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Other => "other",
            Kind::Japanese => "japanese",
            Kind::English => "english",
            Kind::Latin => "latin",
            Kind::Url => "url",
            Kind::Email => "email",
            Kind::Identifier => "identifier",
            Kind::FullWidthAlnum => "fullwidth_alnum",
            Kind::Number => "number",
        }
    }
}

/// 英文字詞查詢。
///
/// [`Dictionary`] 與 [`English`] 都符合這個介面，可以直接傳給 [`classify`]
/// 與 [`scan`]。
pub trait Lookup {
    /// 回報 `text` 是否為認得的英文詞（不分大小寫）。
    fn has(&self, text: &str) -> bool;
}

/// 掃描結果的一段；`start` 與 `end` 是 `[start, end)` 的 UTF-8 位元組位移。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// 種類。
    pub kind: Kind,
    /// 這段文字。
    pub text: String,
    /// 起點位元組位移（含）。
    pub start: usize,
    /// 終點位元組位移（不含）。
    pub end: usize,
}

/// 判斷一段文字在混合輸入裡的種類；`lookup` 可以是 `None`。
pub fn classify(text: &str, lookup: Option<&dyn Lookup>) -> Kind {
    let text = trim_token_right(text);
    if text.is_empty() {
        return Kind::Other;
    }
    if all_japanese(text) {
        return Kind::Japanese;
    }
    let a = analyze_alnum(text);
    if a.fullwidth && a.alnum {
        return Kind::FullWidthAlnum;
    }
    if is_email(text) {
        return Kind::Email;
    }
    if is_url(text) {
        return Kind::Url;
    }
    if a.ascii && a.letters && !a.digits && all_ascii_letters_or_apostrophe(text) {
        let probe = text.replace('\'', "");
        if lookup.is_some_and(|l| l.has(&probe)) {
            return Kind::English;
        }
        return Kind::Latin;
    }
    if a.digits && !a.letters && a.ascii {
        return Kind::Number;
    }
    if is_identifier(text) {
        return Kind::Identifier;
    }
    Kind::Other
}

/// 把一段文字切成日文、英數詞與其他字元，並標出位元組位移。
///
/// 空白會被跳過；英數詞尾端不屬於詞的標點會切成 [`Kind::Other`]，因此
/// `hello.` 會得到 `hello` 與句點兩段。網址、email 與識別字的內部標點保留
/// 在同一段。
pub fn scan(text: &str, lookup: Option<&dyn Lookup>) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut i = 0usize;
    while i < text.len() {
        let Some(ch) = text[i..].chars().next() else {
            break;
        };
        let size = ch.len_utf8();
        if ch.is_whitespace() {
            i += size;
            continue;
        }
        if is_japanese_letter(ch) {
            let start = i;
            while i < text.len() {
                let Some(r) = text[i..].chars().next() else {
                    break;
                };
                if !is_japanese_letter(r) {
                    break;
                }
                i += r.len_utf8();
            }
            tokens.push(Token {
                kind: Kind::Japanese,
                text: text[start..i].to_string(),
                start,
                end: i,
            });
            continue;
        }
        if can_start_token(ch) {
            let start = i;
            while i < text.len() {
                let Some(r) = text[i..].chars().next() else {
                    break;
                };
                if !is_token_rune(r) {
                    break;
                }
                i += r.len_utf8();
            }
            let (core, rest) = split_token(&text[start..i]);
            let core_end = start + core.len();
            if !core.is_empty() {
                tokens.push(Token {
                    kind: classify(core, lookup),
                    text: core.to_string(),
                    start,
                    end: core_end,
                });
            }
            let mut at = core_end;
            for p in rest.chars() {
                let n = p.len_utf8();
                tokens.push(Token {
                    kind: Kind::Other,
                    text: p.to_string(),
                    start: at,
                    end: at + n,
                });
                at += n;
            }
            continue;
        }
        tokens.push(Token {
            kind: Kind::Other,
            text: ch.to_string(),
            start: i,
            end: i + size,
        });
        i += size;
    }
    tokens
}

/// 回報 `text` 是否為網址。結尾的句點、逗號與右括號不算網址的一部分。
pub fn is_url(text: &str) -> bool {
    let t = trim_token_right(text);
    if t.is_empty()
        || t.chars()
            .any(|r| r.is_whitespace() || is_japanese_letter(r))
    {
        return false;
    }
    let lower = t.to_lowercase();
    if let Some(i) = lower.find("://") {
        if i > 0 && valid_scheme(&t[..i]) {
            return true;
        }
    }
    if t.contains('@') {
        return false;
    }
    if let Some(rest) = lower.strip_prefix("www.") {
        return is_domain_host(rest);
    }
    if let Some(rest) = lower.strip_prefix("//") {
        return is_domain_host(&host_part(rest));
    }
    let host = host_part(&lower);
    if host.is_empty() || !is_domain_host(&host) {
        return false;
    }
    t.contains(['/', '?', '#']) || host.contains('.')
}

/// 回報 `text` 是否為 email 位址。
pub fn is_email(text: &str) -> bool {
    let t = trim_token_right(text);
    if t.matches('@').count() != 1 || t.chars().any(char::is_whitespace) {
        return false;
    }
    let Some(at) = t.find('@') else {
        return false;
    };
    let (local, domain) = (&t[..at], &t[at + 1..]);
    if local.is_empty() || domain.is_empty() || !valid_email_local(local) || !is_domain_host(domain)
    {
        return false;
    }
    let Some(dot) = domain.rfind('.') else {
        return false;
    };
    if dot == 0 || dot == domain.len() - 1 {
        return false;
    }
    let tld = &domain[dot + 1..];
    tld.len() >= 2 && all_ascii_letters(tld)
}

/// 回報 `text` 是否為程式識別字、路徑、查詢字串或含英數的編號。
pub fn is_identifier(text: &str) -> bool {
    let t = trim_token_right(text);
    if t.is_empty()
        || t.chars()
            .any(|r| r.is_whitespace() || is_japanese_letter(r))
    {
        return false;
    }
    let (mut has_letter, mut has_digit, mut has_alnum) = (false, false, false);
    for r in t.chars() {
        if r.is_ascii_alphabetic() {
            has_letter = true;
            has_alnum = true;
        } else if r.is_ascii_digit() {
            has_digit = true;
            has_alnum = true;
        } else if !r.is_ascii() && r.is_alphabetic() {
            has_letter = true;
            has_alnum = true;
        }
    }
    if !has_alnum {
        return false;
    }
    if has_letter && has_digit {
        return true;
    }
    if t.contains([
        '_', '\\', '/', ':', '?', '&', '=', '#', '%', '@', '+', '~', '|',
    ]) {
        return true;
    }
    t.contains('-') && has_letter
}

fn is_domain_host(host: &str) -> bool {
    if host.is_empty() || host.contains(['_', '/', '\\']) {
        return false;
    }
    let mut host = host;
    if let Some(colon) = host.rfind(':') {
        if colon == host.len() - 1 || !all_ascii_digits(&host[colon + 1..]) {
            return false;
        }
        host = &host[..colon];
    }
    if host.is_empty() || !host.contains('.') {
        return false;
    }
    for label in host.split('.') {
        if label.is_empty() || label.len() > 63 || label.starts_with('-') || label.ends_with('-') {
            return false;
        }
        if !label.chars().all(|r| r.is_ascii_alphanumeric() || r == '-') {
            return false;
        }
    }
    true
}

fn host_part(s: &str) -> String {
    let mut s = s;
    if let Some(i) = s.find(['/', '?', '#']) {
        s = &s[..i];
    }
    if let Some(at) = s.rfind('@') {
        s = &s[at + 1..];
    }
    s.to_string()
}

fn valid_scheme(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    for (i, r) in s.chars().enumerate() {
        if i == 0 {
            if !r.is_ascii_alphabetic() {
                return false;
            }
            continue;
        }
        if !(r.is_ascii_alphanumeric() || matches!(r, '+' | '-' | '.')) {
            return false;
        }
    }
    true
}

fn valid_email_local(s: &str) -> bool {
    if s.is_empty() || s.starts_with('.') || s.ends_with('.') {
        return false;
    }
    if !s
        .chars()
        .all(|r| r.is_ascii_alphanumeric() || "._%+-".contains(r))
    {
        return false;
    }
    !s.contains("..")
}

fn all_ascii_letters(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphabetic())
}

fn all_ascii_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

fn all_ascii_letters_or_apostrophe(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphabetic() || c == '\'')
}

fn all_japanese(s: &str) -> bool {
    !s.is_empty() && s.chars().all(is_japanese_letter)
}

#[derive(Debug, Clone, Copy, Default)]
struct AlnumShape {
    ascii: bool,
    fullwidth: bool,
    letters: bool,
    digits: bool,
    alnum: bool,
}

fn analyze_alnum(text: &str) -> AlnumShape {
    let mut s = AlnumShape {
        alnum: true,
        ..AlnumShape::default()
    };
    for r in text.chars() {
        if r.is_ascii_alphabetic() {
            s.ascii = true;
            s.letters = true;
        } else if r.is_ascii_digit() {
            s.ascii = true;
            s.digits = true;
        } else if is_fullwidth_alnum(r) {
            s.fullwidth = true;
            if matches!(r, 'Ａ'..='Ｚ' | 'ａ'..='ｚ') {
                s.letters = true;
            } else {
                s.digits = true;
            }
        } else if r == '\'' {
            // 英文縮寫（don't）留在同一段，但不算英數。
        } else {
            s.alnum = false;
        }
    }
    s
}

fn can_start_token(r: char) -> bool {
    r.is_ascii_alphanumeric() || is_fullwidth_alnum(r) || matches!(r, '@' | '#' | '$')
}

fn is_token_rune(r: char) -> bool {
    if r.is_ascii_alphanumeric() || is_fullwidth_alnum(r) {
        return true;
    }
    "._~:/?#[]@!$&'()*+,;=%\\-|{}".contains(r)
}

/// 去掉詞尾不屬於詞的標點，回傳（本體, 尾端標點）。
fn split_token(s: &str) -> (&str, &str) {
    let mut end = s.len();
    for (i, r) in s.char_indices().rev() {
        if ".,;:!?)]}\"'>".contains(r) {
            end = i;
        } else {
            break;
        }
    }
    (&s[..end], &s[end..])
}

fn trim_token_right(s: &str) -> &str {
    split_token(s).0
}

// ---------------------------------------------------------------------------
// 字寬
// ---------------------------------------------------------------------------

/// 回報 `r` 是否為全角英數字。
pub fn is_fullwidth_alnum(r: char) -> bool {
    matches!(r, '０'..='９' | 'Ａ'..='Ｚ' | 'ａ'..='ｚ')
}

/// 回報 `text` 是否含全角英數字。
pub fn has_fullwidth_alnum(text: &str) -> bool {
    text.chars().any(is_fullwidth_alnum)
}

/// 回報 `text` 是否含 ASCII 英數字。
pub fn has_ascii_alnum(text: &str) -> bool {
    text.chars().any(|r| r.is_ascii_alphanumeric())
}

/// 回報 `text` 是否含拉丁英數字（ASCII 或全角）。
pub fn has_latin_alnum(text: &str) -> bool {
    text.chars()
        .any(|r| r.is_ascii_alphanumeric() || is_fullwidth_alnum(r))
}

/// 把全角英數字轉成半角，其他字元保持原樣。
pub fn to_halfwidth_latin(text: &str) -> String {
    text.chars()
        .map(|r| match r {
            '０'..='９' => char::from(b'0' + (r as u32 - '０' as u32) as u8),
            'Ａ'..='Ｚ' => char::from(b'A' + (r as u32 - 'Ａ' as u32) as u8),
            'ａ'..='ｚ' => char::from(b'a' + (r as u32 - 'ａ' as u32) as u8),
            _ => r,
        })
        .collect()
}

/// 把 ASCII 英數字轉成全角，其他字元保持原樣。
pub fn to_fullwidth_latin(text: &str) -> String {
    text.chars()
        .map(|r| match r {
            '0'..='9' => char::from_u32('０' as u32 + (r as u32 - '0' as u32)).unwrap_or(r),
            'A'..='Z' => char::from_u32('Ａ' as u32 + (r as u32 - 'A' as u32)).unwrap_or(r),
            'a'..='z' => char::from_u32('ａ' as u32 + (r as u32 - 'a' as u32)).unwrap_or(r),
            _ => r,
        })
        .collect()
}

/// 回傳 `text` 的另一種英數字寬度；沒有英數字時回傳 `None`。
///
/// 全角與半角混用時回傳全角（與日文輸入的預設一致），全部全角時回傳半角。
pub fn width_variant(text: &str) -> Option<String> {
    let has_full = has_fullwidth_alnum(text);
    let has_ascii = has_ascii_alnum(text);
    if !has_full && !has_ascii {
        return None;
    }
    if has_full && has_ascii {
        return Some(to_fullwidth_latin(text));
    }
    if has_full {
        return Some(to_halfwidth_latin(text));
    }
    Some(to_fullwidth_latin(text))
}

// ---------------------------------------------------------------------------
// 空白規則
// ---------------------------------------------------------------------------

/// 日文與英文之間是否補空白的設定。
///
/// 日文通常不在漢字、かな與英文之間加空白，和注音的中英混打預設不同，因此
/// 零值 [`Spacing::None`] 就是「不補空白」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Spacing {
    /// 不補空白（預設）。
    #[default]
    None,
    /// 在相鄰的日文字母與拉丁字母之間補一個半形空格。
    Always,
}

impl Spacing {
    /// 穩定的設定名稱。
    pub fn as_str(self) -> &'static str {
        match self {
            Spacing::None => "none",
            Spacing::Always => "always",
        }
    }
}

/// 依設定在日文字母與拉丁字母相連的地方補半形空格。
///
/// 只有直接相鄰的兩個字元才會處理；重複呼叫不會再增加空白。
pub fn space_text(text: &str, spacing: Spacing) -> String {
    if spacing != Spacing::Always || text.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 8);
    let mut previous: Option<char> = None;
    for r in text.chars() {
        if let Some(prev) = previous {
            if needs_space_between(prev, r) {
                out.push(' ');
            }
        }
        out.push(r);
        previous = Some(r);
    }
    out
}

/// 回報 `left` 與 `right` 直接相連時是否需要補空白。
///
/// 規則是「一邊是日文字母、另一邊是拉丁字母」；數字不算英文字。
pub fn needs_space_between(left: char, right: char) -> bool {
    is_japanese_letter(left) && is_latin_letter(right)
        || is_latin_letter(left) && is_japanese_letter(right)
}

/// 回報 `r` 是否為可和英文相鄰的日文字母。
pub fn is_japanese_letter(r: char) -> bool {
    matches!(detect_script(r), Script::Kana | Script::Han)
}

/// 回報 `r` 是否為 ASCII 或全角拉丁字母。
pub fn is_latin_letter(r: char) -> bool {
    r.is_ascii_alphabetic() || matches!(r, 'ａ'..='ｚ' | 'Ａ'..='Ｚ')
}

// ---------------------------------------------------------------------------
// 英文詞表
// ---------------------------------------------------------------------------

/// 一筆英文詞：詞面與排序權重（越大越優先）。
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    /// 小寫詞面。
    pub text: String,
    /// 排序權重，必須是有限正數。
    pub weight: f64,
}

impl Word {
    /// 建立詞條。
    pub fn new(text: impl Into<String>, weight: f64) -> Self {
        Self {
            text: text.into(),
            weight,
        }
    }
}

/// 英文查詢的錯誤。
#[derive(Debug, thiserror::Error)]
pub enum MixedError {
    /// 讀取檔案失敗。
    #[error("讀取檔案失敗：{0}")]
    Io(#[from] std::io::Error),
    /// 詞表格式不合法。
    #[error("英文詞表格式不合法：{0}")]
    Format(&'static str),
    /// 注音詞格建立失敗。
    #[error("注音詞格建立失敗：{0}")]
    Lattice(#[from] LatticeError),
}

/// 自動補完的一項。
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    /// 補完後的詞面（保留查詢時的大小寫）。
    pub text: String,
    /// 權重，越大越優先。
    pub weight: f64,
    /// 是否來自學習詞。
    pub learned: bool,
}

/// 唯讀英文詞表。
///
/// 詞表依小寫詞面排序，查詢是二分搜尋；建好之後不再修改，可以共用。
#[derive(Debug, Clone, Default)]
pub struct Dictionary {
    words: Vec<Word>,
    no_suggest: Vec<bool>,
}

impl Dictionary {
    /// 由詞條建立詞表。
    ///
    /// 只保留全小寫 ASCII 字母、權重為有限正數且長度不超過
    /// [`MAX_WORD_BYTES`] 的詞；同一詞面重複出現時保留權重最大者。
    pub fn new<I>(words: I) -> Self
    where
        I: IntoIterator<Item = Word>,
    {
        let mut best: HashMap<String, f64> = HashMap::new();
        for w in words {
            let text = w.text.to_lowercase();
            if !is_lower_ascii_word(&text) || w.weight <= 0.0 || !w.weight.is_finite() {
                continue;
            }
            match best.get(&text) {
                Some(&old) if old >= w.weight => {}
                _ => {
                    best.insert(text, w.weight);
                }
            }
        }
        let mut words: Vec<Word> = best
            .into_iter()
            .map(|(text, weight)| Word { text, weight })
            .collect();
        words.sort_by(|a, b| a.text.cmp(&b.text));
        let no_suggest = vec![false; words.len()];
        Self { words, no_suggest }
    }

    /// 讀取 `0cEN` v1 格式的詞表檔。
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MixedError> {
        let data = std::fs::read(path)?;
        Self::decode(&data)
    }

    /// 解析 `0cEN` v1 格式的詞表。
    pub fn decode(data: &[u8]) -> Result<Self, MixedError> {
        const MAGIC: &[u8] = b"0cEN\x00\x01";
        const MAX_PACK_WORDS: usize = 200_000;
        const MAX_PACK_BYTES: usize = 16 << 20;

        if data.len() > MAX_PACK_BYTES {
            return Err(MixedError::Format("資料包過大"));
        }
        let Some(rest) = data.strip_prefix(MAGIC) else {
            return Err(MixedError::Format("標頭不合法"));
        };
        if rest.len() < 4 {
            return Err(MixedError::Format("缺少詞數"));
        }
        let count = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
        if count > MAX_PACK_WORDS {
            return Err(MixedError::Format("詞數不合法"));
        }
        let mut body = &rest[4..];
        let mut words = Vec::with_capacity(count);
        let mut no_suggest = Vec::with_capacity(count);
        let mut previous = String::new();
        for _ in 0..count {
            let Some((&n, tail)) = body.split_first() else {
                return Err(MixedError::Format("詞條截斷"));
            };
            let n = usize::from(n);
            if n == 0 || n > MAX_WORD_BYTES || tail.len() < n + 9 {
                return Err(MixedError::Format("詞條截斷或過長"));
            }
            let (text, tail) = tail.split_at(n);
            let weight = f64::from_le_bytes([
                tail[0], tail[1], tail[2], tail[3], tail[4], tail[5], tail[6], tail[7],
            ]);
            let flag = tail[8];
            body = &tail[9..];
            let text = std::str::from_utf8(text)
                .map_err(|_| MixedError::Format("詞面不是 UTF-8"))?
                .to_string();
            if !is_lower_ascii_word(&text)
                || text <= previous
                || weight <= 0.0
                || !weight.is_finite()
                || flag > 1
            {
                return Err(MixedError::Format("詞條內容或排序不合法"));
            }
            previous = text.clone();
            words.push(Word { text, weight });
            no_suggest.push(flag == 1);
        }
        if !body.is_empty() {
            return Err(MixedError::Format("有多餘內容"));
        }
        Ok(Self { words, no_suggest })
    }

    /// 詞條數。
    pub fn len(&self) -> usize {
        self.words.len()
    }

    /// 是否沒有任何詞條。
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// 詞表內容；依小寫詞面排序。
    pub fn words(&self) -> &[Word] {
        &self.words
    }

    /// 回報 `word` 是否在詞表裡（不分大小寫）。
    pub fn has(&self, word: &str) -> bool {
        self.position(word).is_some()
    }

    /// 回報 `word` 是否在詞表裡，且允許被推薦。
    pub fn suggestible(&self, word: &str) -> bool {
        match self.position(word) {
            Some(i) => !self.no_suggest[i],
            None => false,
        }
    }

    /// 回傳以 `prefix` 開頭的詞，權重由高到低。
    ///
    /// `prefix` 的大小寫決定回傳詞面的大小寫：全大寫查詢回傳全大寫，
    /// 其他查詢保留使用者打的寫法。
    pub fn prefix(&self, prefix: &str, limit: usize) -> Vec<Suggestion> {
        if prefix.is_empty() || limit == 0 || prefix.len() > MAX_WORD_BYTES {
            return Vec::new();
        }
        if !prefix.chars().all(|c| c.is_ascii_alphabetic()) {
            return Vec::new();
        }
        let lower = prefix.to_lowercase();
        if !is_lower_ascii_word(&lower) {
            return Vec::new();
        }
        let all_caps = prefix
            .chars()
            .all(|c| !c.is_ascii_alphabetic() || c.is_ascii_uppercase());
        let limit = limit.min(MAX_SUGGESTIONS);
        let mut out: Vec<Suggestion> = Vec::new();
        for (i, w) in self.words.iter().enumerate().skip(self.lower_bound(&lower)) {
            if !w.text.starts_with(&lower) {
                break;
            }
            if self.no_suggest[i] {
                continue;
            }
            let suffix = &w.text[lower.len()..];
            let text = if all_caps {
                format!("{}{}", prefix, suffix.to_uppercase())
            } else {
                format!("{}{}", prefix, suffix)
            };
            out.push(Suggestion {
                text,
                weight: w.weight,
                learned: false,
            });
        }
        out.sort_by(|a, b| {
            b.weight
                .total_cmp(&a.weight)
                .then_with(|| a.text.cmp(&b.text))
        });
        out.truncate(limit);
        out
    }

    fn position(&self, word: &str) -> Option<usize> {
        if word.len() > MAX_WORD_BYTES || !word.chars().all(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        let lower = word.to_lowercase();
        let i = self.lower_bound(&lower);
        match self.words.get(i) {
            Some(w) if w.text == lower => Some(i),
            _ => None,
        }
    }

    fn lower_bound(&self, lower: &str) -> usize {
        self.words.partition_point(|w| w.text.as_str() < lower)
    }
}

impl Lookup for Dictionary {
    fn has(&self, text: &str) -> bool {
        self.has(text)
    }
}

/// 內建英文原型詞彙。
///
/// 只用來讓第一次混打有東西可查，不含授權詞表的頻率資料。
pub fn prototype() -> &'static Dictionary {
    static DICT: OnceLock<Dictionary> = OnceLock::new();
    DICT.get_or_init(|| {
        const LIST: &str = "the and for with from this that have will hello world meeting \
receive input method english chinese test code file name time today tomorrow thanks thank \
please work project update settings help user users server client computer keyboard window \
windows system application data model language suggestion correct completion autocomplete \
hello meeting message email document write writing text type typing search make build release \
version go api python docker github redis linux macos chatgpt openai kubernetes javascript \
typescript function string boolean integer database network website browser terminal error \
request response cancel accept learning personal dictionary word words download install \
uninstall package path url https http foo bar";
        let words: Vec<&str> = LIST.split_whitespace().collect();
        let n = words.len();
        Dictionary::new(
            words
                .into_iter()
                .enumerate()
                .map(|(i, w)| Word::new(w, (n - i) as f64)),
        )
    })
}

fn is_lower_ascii_word(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_WORD_BYTES && s.chars().all(|c| c.is_ascii_lowercase())
}

/// 英文查詢層：唯讀詞表加上個人學習詞。
#[derive(Debug, Clone)]
pub struct English {
    dict: Dictionary,
    learned: Vec<LearnedWord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LearnedWord {
    text: String,
    count: u32,
}

impl English {
    /// 用指定詞表建立查詢層；`None` 使用 [`prototype`]。
    pub fn new(dict: Option<Dictionary>) -> Self {
        Self {
            dict: dict.unwrap_or_else(|| prototype().clone()),
            learned: Vec::new(),
        }
    }

    /// 底層詞表。
    pub fn dictionary(&self) -> &Dictionary {
        &self.dict
    }

    /// 回報 `text` 是否為詞表或學習詞認得的英文詞（不分大小寫）。
    pub fn has(&self, text: &str) -> bool {
        self.learned(text) || self.dict.has(text)
    }

    /// [`English::has`] 的別名。
    pub fn known(&self, text: &str) -> bool {
        self.has(text)
    }

    /// 回報 `text` 是否為學過的詞（不分大小寫）。
    pub fn learned(&self, text: &str) -> bool {
        !text.is_empty() && self.find(text).is_some()
    }

    /// 記下一次英文詞的使用；空白、空字串與過長的內容會被拒絕。
    ///
    /// 大小寫照第一次學到的寫法保留。
    pub fn learn(&mut self, text: &str) -> bool {
        let trimmed = text.trim();
        if !valid_literal(trimmed) {
            return false;
        }
        match self.find_mut(trimmed) {
            Some(w) => {
                w.count = w.count.saturating_add(1);
                true
            }
            None => {
                self.learned.push(LearnedWord {
                    text: trimmed.to_string(),
                    count: 1,
                });
                true
            }
        }
    }

    /// 移除一個學習詞；有移除時回傳 `true`。
    pub fn forget(&mut self, text: &str) -> bool {
        let key = text.trim().to_lowercase();
        let before = self.learned.len();
        self.learned.retain(|w| w.text.to_lowercase() != key);
        self.learned.len() != before
    }

    /// 學過的詞，依使用次數由高到低。
    pub fn learned_words(&self) -> Vec<String> {
        let mut words = self.learned.clone();
        words.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.text.cmp(&b.text)));
        words.into_iter().map(|w| w.text).collect()
    }

    /// 回傳以 `prefix` 開頭的英文詞，學習詞優先。
    pub fn suggestions(&self, prefix: &str, limit: usize) -> Vec<Suggestion> {
        if prefix.is_empty() || limit == 0 {
            return Vec::new();
        }
        let limit = limit.min(MAX_SUGGESTIONS);
        let lower = prefix.to_lowercase();
        let mut out: Vec<Suggestion> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        let mut learned: Vec<&LearnedWord> = self
            .learned
            .iter()
            .filter(|w| w.text.to_lowercase().starts_with(&lower))
            .collect();
        learned.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.text.cmp(&b.text)));
        for w in learned {
            seen.push(w.text.to_lowercase());
            out.push(Suggestion {
                text: w.text.clone(),
                weight: LEARNED_WEIGHT + f64::from(w.count),
                learned: true,
            });
        }
        if prefix.chars().all(|c| c.is_ascii_alphabetic()) {
            for cand in self.dict.prefix(prefix, limit) {
                let key = cand.text.to_lowercase();
                if seen.contains(&key) {
                    continue;
                }
                seen.push(key);
                out.push(cand);
                if out.len() == limit {
                    break;
                }
            }
        }
        out.truncate(limit);
        out
    }

    /// 回傳 `prefix` 最長或最優先的補完詞；沒有更長的候選時回傳 `None`。
    pub fn complete(&self, prefix: &str) -> Option<String> {
        if prefix.is_empty() {
            return None;
        }
        let count = prefix.chars().count();
        self.suggestions(prefix, MAX_SUGGESTIONS)
            .into_iter()
            .find(|s| s.text.chars().count() > count && !s.text.eq_ignore_ascii_case(prefix))
            .map(|s| s.text)
    }

    /// 把補完轉成 [`Candidate`]；原樣輸入的候選由呼叫端自己排在最前面。
    pub fn candidates(&self, prefix: &str, limit: usize) -> Vec<Candidate> {
        if prefix.is_empty() || limit == 0 {
            return Vec::new();
        }
        self.suggestions(prefix, limit)
            .into_iter()
            .map(|s| {
                let source = if s.learned {
                    CandidateSource::Personal
                } else {
                    CandidateSource::Lexicon
                };
                Candidate::new(s.text.clone(), s.text, cost_of_weight(s.weight), source)
            })
            .collect()
    }

    fn find(&self, text: &str) -> Option<&LearnedWord> {
        let key = text.to_lowercase();
        self.learned.iter().find(|w| w.text.to_lowercase() == key)
    }

    fn find_mut(&mut self, text: &str) -> Option<&mut LearnedWord> {
        let key = text.to_lowercase();
        self.learned
            .iter_mut()
            .find(|w| w.text.to_lowercase() == key)
    }
}

impl Default for English {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Lookup for English {
    fn has(&self, text: &str) -> bool {
        self.has(text)
    }
}

fn cost_of_weight(weight: f64) -> i32 {
    let w = if weight.is_finite() { weight } else { 0.0 };
    let w = w.clamp(0.0, f64::from(WEIGHT_BASE));
    WEIGHT_BASE - w.round() as i32
}

fn valid_literal(text: &str) -> bool {
    if text.is_empty() || text.len() > MAX_LITERAL_BYTES {
        return false;
    }
    !text
        .chars()
        .any(|r| r.is_whitespace() || r == '\u{3000}' || r < ' ')
}

// ---------------------------------------------------------------------------
// 羅馬字判定
// ---------------------------------------------------------------------------

/// 把一段羅馬字還原成（かな, 未完成前綴）；不是合法羅馬字時回傳 `None`。
pub fn replay_romaji(raw: &str) -> Option<(String, String)> {
    if raw.is_empty() {
        return None;
    }
    let mut input = RomajiInput::new();
    for ch in raw.chars() {
        if !('!'..='~').contains(&ch) {
            return None;
        }
        match input.press(Key::char(ch, false)) {
            RomajiEvent::Accepted { .. } => {}
            RomajiEvent::Rejected => return None,
        }
    }
    let kana: String = input.units().iter().map(|u| u.as_str()).collect();
    Some((kana, input.pending().to_string()))
}

/// 回傳 `raw` 組成的かな與尚未完成的羅馬字。
///
/// 不是合法羅馬字，或依規則會照字面處理時回傳 `None`。
pub fn kana_for(raw: &str, cfg: &Config, english: Option<&English>) -> Option<(String, String)> {
    if raw.is_empty() || literal_for(raw, cfg, english) {
        return None;
    }
    replay_romaji(raw)
}

/// 回報一段獨立的 ASCII 按鍵會不會被當成照字面文字。
///
/// 大寫、全角英數、網址、email、識別字、已知英文詞、學過的詞，以及打不成
/// 合法羅馬字的情況都算照字面。
pub fn literal_for(raw: &str, cfg: &Config, english: Option<&English>) -> bool {
    if raw.is_empty() {
        return true;
    }
    let cfg = cfg.normalized();
    let kana_valid = replay_romaji(raw).is_some();
    if has_fullwidth_alnum(raw) || has_ascii_upper(raw) {
        return true;
    }
    match classify(raw, english.map(|e| e as &dyn Lookup)) {
        Kind::Url | Kind::Email | Kind::Identifier | Kind::FullWidthAlnum | Kind::Number => {
            return true;
        }
        Kind::English => {
            let lookup = english.map(|e| e as &dyn Lookup);
            let borrow: &dyn Lookup = match lookup {
                Some(l) => l,
                None => prototype(),
            };
            if raw.chars().count() >= cfg.min_english_len && !cfg.prefer_kana && borrow.has(raw) {
                return true;
            }
        }
        _ => {}
    }
    if english.is_some_and(|e| e.learned(raw)) {
        return true;
    }
    !kana_valid
}

fn has_ascii_upper(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_uppercase())
}

// ---------------------------------------------------------------------------
// 設定
// ---------------------------------------------------------------------------

/// 混合路由的設定。
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// 轉換後日文與英文之間的空白規則。
    pub spacing: Spacing,
    /// 詞表英文詞照字面處理的最短長度；0 代表 3。
    pub min_english_len: usize,
    /// 讓合法羅馬字優先於詞表英文詞。
    pub prefer_kana: bool,
    /// 單次查詢的候選上限；0 代表 8。
    pub max_candidates: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            spacing: Spacing::None,
            min_english_len: 3,
            prefer_kana: false,
            max_candidates: 8,
        }
    }
}

impl Config {
    fn normalized(&self) -> Config {
        let mut cfg = self.clone();
        if cfg.min_english_len == 0 {
            cfg.min_english_len = 3;
        }
        if cfg.max_candidates == 0 {
            cfg.max_candidates = 8;
        }
        cfg.max_candidates = cfg.max_candidates.min(MAX_SUGGESTIONS);
        cfg
    }
}

// ---------------------------------------------------------------------------
// 分段
// ---------------------------------------------------------------------------

/// 注音轉換需要的詞典與成本設定。
pub struct ZhRoute<'a> {
    /// 注音詞典。
    pub lexicon: &'a dyn Lexicon,
    /// 建詞格與評分用的成本設定。
    pub ctx: &'a ScoreContext,
}

impl std::fmt::Debug for ZhRoute<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZhRoute").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AtomKind {
    Zhuyin,
    Kana,
    Han,
    Word,
    Symbol,
}

#[derive(Debug, Clone)]
struct Atom {
    start: usize,
    end: usize,
    script: Script,
    mode: Option<Mode>,
    kind: AtomKind,
    /// 羅馬字已還原的かな；只有整段可完整かな化時才有值。
    kana: Option<String>,
    /// 全小寫 ASCII 字母（片語合併用）。
    plain_lower: bool,
}

/// 一個輸出片段的規劃。
#[derive(Debug, Clone)]
struct Plan {
    start: usize,
    end: usize,
    mode: Mode,
    script: Script,
    kana: String,
    kana_ok: bool,
}

/// 混合輸入的路由器。
///
/// 三個轉換器都是選配：缺少注音詞典時注音片段照原樣輸出，缺少日文解碼器
/// 時かな片段照原樣輸出。這樣沒有資料也能測試分段與語言判定。
#[derive(Default)]
pub struct Router<'a> {
    /// 設定。
    pub config: Config,
    /// 英文查詢層；`None` 時使用 [`prototype`] 的唯讀查詢。
    pub english: Option<&'a English>,
    /// 注音轉換器。
    pub zh: Option<ZhRoute<'a>>,
    /// 日文解碼器。
    pub ja: Option<&'a JaDecoder>,
}

impl<'a> Router<'a> {
    /// 建立只有英文直通的路由器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 指定設定。
    pub fn with_config(mut self, config: Config) -> Self {
        self.config = config.normalized();
        self
    }

    /// 指定英文查詢層。
    pub fn with_english(mut self, english: &'a English) -> Self {
        self.english = Some(english);
        self
    }

    /// 指定注音詞典與成本設定。
    pub fn with_zh(mut self, lexicon: &'a dyn Lexicon, ctx: &'a ScoreContext) -> Self {
        self.zh = Some(ZhRoute { lexicon, ctx });
        self
    }

    /// 指定日文解碼器。
    pub fn with_ja(mut self, decoder: &'a JaDecoder) -> Self {
        self.ja = Some(decoder);
        self
    }

    /// 查詢層：指定的英文查詢層，否則用內建原型詞彙。
    fn lookup(&self) -> LookupRef<'_> {
        match self.english {
            Some(e) => LookupRef::English(e),
            None => LookupRef::Prototype,
        }
    }

    /// 把輸入切成語言片段；沒有轉換，只判定邊界與語言。
    pub fn segments(&self, input: &str) -> Vec<Segment> {
        self.plan(input)
            .into_iter()
            .map(|p| Segment::new(p.start, p.end, p.mode, p.script))
            .collect()
    }

    /// 回報整段輸入的語言：只有單一語言時就是那個語言，否則
    /// [`Mode::Mixed`]。空輸入視為英文直出。
    pub fn detect(&self, input: &str) -> Mode {
        let segments = self.segments(input);
        let Some(first) = segments.first() else {
            return Mode::En;
        };
        if segments.iter().all(|s| s.mode == first.mode) {
            first.mode
        } else {
            Mode::Mixed
        }
    }

    /// 轉換整段輸入。
    ///
    /// 語言邊界不加分隔符；`segments` 的位移是**輸入**的 UTF-8 位元組位移，
    /// 輸出字面的游標位置另外用 UTF-16 表示。
    pub fn convert(&self, input: &str) -> Result<MixedResult, MixedError> {
        let plans = self.plan(input);
        let mut surface = String::new();
        let mut segments = Vec::with_capacity(plans.len());
        let mut cost = 0i32;
        for plan in &plans {
            let (text, c) = self.convert_plan(plan, input)?;
            cost = cost.saturating_add(c);
            let surface_text = if self.config.spacing == Spacing::Always {
                apply_boundary_space(&surface, &text)
            } else {
                text
            };
            let start_utf16 = surface.encode_utf16().count();
            surface.push_str(&surface_text);
            let end_utf16 = surface.encode_utf16().count();
            segments.push(MixedSegment {
                input_start: plan.start,
                input_end: plan.end,
                mode: plan.mode,
                script: plan.script,
                surface: surface_text,
                start_utf16,
                end_utf16,
            });
        }
        let cursor_utf16 = surface.encode_utf16().count();
        Ok(MixedResult {
            input: input.to_string(),
            surface,
            segments,
            cursor_utf16,
            cost,
        })
    }

    /// 結尾英數段的英文補完候選。
    pub fn prefixes(&self, input: &str) -> Vec<Candidate> {
        let plans = self.plan(input);
        let Some(last) = plans.last() else {
            return Vec::new();
        };
        let text = &input[last.start..last.end];
        if last.mode != Mode::En || !text.chars().all(|c| c.is_ascii_alphabetic()) {
            return Vec::new();
        }
        self.english()
            .candidates(text, self.config.normalized().max_candidates)
    }

    /// 取得英文查詢層的參照。
    fn english(&self) -> EnglishRef<'_> {
        match self.english {
            Some(e) => EnglishRef::Shared(e),
            None => EnglishRef::Prototype,
        }
    }

    fn convert_plan(&self, plan: &Plan, input: &str) -> Result<(String, i32), MixedError> {
        let literal = &input[plan.start..plan.end];
        match plan.mode {
            Mode::Zh => match &self.zh {
                Some(route) => {
                    let syllables = zhuyin_syllables(literal);
                    if syllables.is_empty() {
                        return Ok((literal.to_string(), 0));
                    }
                    let lattice = ZhLattice::build(&syllables, route.lexicon, route.ctx)?;
                    let decoder = Decoder::new(route.lexicon, route.ctx);
                    let (segs, cost) = decoder.best(&lattice, &Locks::new(), false);
                    let conversion = decoder.conversion(&lattice, &segs, Mode::Zh);
                    Ok((conversion.surface, cost))
                }
                None => Ok((literal.to_string(), 0)),
            },
            Mode::Ja => {
                if !plan.kana_ok {
                    return Ok((literal.to_string(), 0));
                }
                match self.ja {
                    Some(decoder) => match decoder.convert(&plan.kana, &JaContext::default()) {
                        Some(conversion) => Ok((conversion.surface, conversion.cost)),
                        None => Ok((plan.kana.clone(), 0)),
                    },
                    None => Ok((plan.kana.clone(), 0)),
                }
            }
            _ => Ok((literal.to_string(), 0)),
        }
    }

    fn plan(&self, input: &str) -> Vec<Plan> {
        let cfg = self.config.normalized();
        let look = self.lookup();
        let lookup: &dyn Lookup = look.as_dyn();
        let mut atoms = build_atoms(input, lookup, &cfg);
        group_words(&mut atoms, input);
        resolve_han(&mut atoms);
        resolve_symbols(&mut atoms);
        merge_atoms(&atoms, input)
    }
}

/// 英文查詢層的借用形式，避免 `Option<&English>` 的生命週期外洩。
enum LookupRef<'a> {
    English(&'a English),
    Prototype,
}

impl<'a> LookupRef<'a> {
    fn as_dyn(&self) -> &'a dyn Lookup {
        match self {
            LookupRef::English(e) => *e,
            LookupRef::Prototype => prototype(),
        }
    }
}

enum EnglishRef<'a> {
    Shared(&'a English),
    Prototype,
}

impl EnglishRef<'_> {
    fn candidates(&self, prefix: &str, limit: usize) -> Vec<Candidate> {
        match self {
            EnglishRef::Shared(e) => e.candidates(prefix, limit),
            EnglishRef::Prototype => {
                let owned = English::new(None);
                owned.candidates(prefix, limit)
            }
        }
    }
}

/// 一個轉換後的片段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MixedSegment {
    /// 輸入上的起點（UTF-8 位元組，含）。
    pub input_start: usize,
    /// 輸入上的終點（UTF-8 位元組，不含）。
    pub input_end: usize,
    /// 這一段的語言。
    pub mode: Mode,
    /// 這一段的字面種類。
    pub script: Script,
    /// 這一段的輸出字面。
    pub surface: String,
    /// 輸出字面在整句中的起點（UTF-16）。
    pub start_utf16: usize,
    /// 輸出字面在整句中的終點（UTF-16）。
    pub end_utf16: usize,
}

/// 一次混合轉換的結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MixedResult {
    /// 原始輸入。
    pub input: String,
    /// 轉換後的整句。
    pub surface: String,
    /// 逐段結果；語言邊界不加分隔符。
    pub segments: Vec<MixedSegment>,
    /// 游標位置（UTF-16，固定在尾端）。
    pub cursor_utf16: usize,
    /// 整體成本，越小越優先。
    pub cost: i32,
}

impl MixedResult {
    /// 轉成 [`Conversion`]，其中的 `Segment` 改用**輸出**位元組位移。
    pub fn conversion(&self) -> Conversion {
        let mut byte = 0usize;
        let mut segments = Vec::with_capacity(self.segments.len());
        for seg in &self.segments {
            let start = byte;
            byte += seg.surface.len();
            segments.push(Segment::new(start, byte, seg.mode, seg.script));
        }
        Conversion {
            surface: self.surface.clone(),
            segments,
            cost: self.cost,
        }
    }

    /// 依序回報每段的語言。
    pub fn languages(&self) -> Vec<Mode> {
        self.segments.iter().map(|s| s.mode).collect()
    }
}

fn apply_boundary_space(surface: &str, text: &str) -> String {
    let left = surface.chars().next_back();
    let right = text.chars().next();
    match (left, right) {
        (Some(l), Some(r)) if needs_space_between(l, r) => format!(" {text}"),
        _ => text.to_string(),
    }
}

/// 把注音串重組成音節字串。
///
/// 聲調完成一個音節；沒打聲調時，遇到同類或更前面的符號（例如新的聲母）
/// 就切開，因此 `ㄉㄚˋㄐㄧㄅㄚ` 會得到三個音節。
fn zhuyin_syllables(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut last = Class::None;
    for ch in text.chars() {
        let class = class_of(ch);
        if class == Class::None {
            continue;
        }
        if class == Class::Tone {
            if current.is_empty() {
                continue;
            }
            if ch != TONE1 {
                current.push(ch);
            }
            out.push(std::mem::take(&mut current));
            last = Class::None;
            continue;
        }
        if class <= last && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        current.push(ch);
        last = class;
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn build_atoms(input: &str, lookup: &dyn Lookup, cfg: &Config) -> Vec<Atom> {
    let mut atoms = Vec::new();
    let mut i = 0usize;
    while i < input.len() {
        let Some(ch) = input[i..].chars().next() else {
            break;
        };
        if is_zhuyin(ch) {
            let start = i;
            while i < input.len() {
                let Some(r) = input[i..].chars().next() else {
                    break;
                };
                if !is_zhuyin(r) {
                    break;
                }
                i += r.len_utf8();
            }
            atoms.push(Atom {
                start,
                end: i,
                script: Script::Bopomofo,
                mode: Some(Mode::Zh),
                kind: AtomKind::Zhuyin,
                kana: None,
                plain_lower: false,
            });
            continue;
        }
        if is_japanese_letter(ch) {
            let start = i;
            while i < input.len() {
                let Some(r) = input[i..].chars().next() else {
                    break;
                };
                if !is_japanese_letter(r) {
                    break;
                }
                i += r.len_utf8();
            }
            for (piece_start, piece_end, script) in split_japanese(input, start, i) {
                let kind = if script == Script::Kana {
                    AtomKind::Kana
                } else {
                    AtomKind::Han
                };
                atoms.push(Atom {
                    start: piece_start,
                    end: piece_end,
                    script,
                    mode: if script == Script::Kana {
                        Some(Mode::Ja)
                    } else {
                        None
                    },
                    kind,
                    kana: None,
                    plain_lower: false,
                });
            }
            continue;
        }
        if can_start_token(ch) {
            let start = i;
            while i < input.len() {
                let Some(r) = input[i..].chars().next() else {
                    break;
                };
                if !is_token_rune(r) {
                    break;
                }
                i += r.len_utf8();
            }
            let (core, rest) = split_token(&input[start..i]);
            if !core.is_empty() {
                let offset = start;
                push_latin_atoms(&mut atoms, core, offset, lookup, cfg);
            }
            let mut at = start + core.len();
            for p in rest.chars() {
                let n = p.len_utf8();
                atoms.push(Atom {
                    start: at,
                    end: at + n,
                    script: Script::Symbol,
                    mode: None,
                    kind: AtomKind::Symbol,
                    kana: None,
                    plain_lower: false,
                });
                at += n;
            }
            continue;
        }
        let start = i;
        i += ch.len_utf8();
        while i < input.len() {
            let Some(r) = input[i..].chars().next() else {
                break;
            };
            if is_zhuyin(r) || is_japanese_letter(r) || can_start_token(r) {
                break;
            }
            i += r.len_utf8();
        }
        atoms.push(Atom {
            start,
            end: i,
            script: Script::Symbol,
            mode: None,
            kind: AtomKind::Symbol,
            kana: None,
            plain_lower: false,
        });
    }
    atoms
}

fn is_zhuyin(ch: char) -> bool {
    class_of(ch) != Class::None || matches!(ch as u32, 0x3100..=0x312F | 0x31A0..=0x31BF)
}

/// 把一段日文字母切成かな與漢字的連續片段。
fn split_japanese(input: &str, start: usize, end: usize) -> Vec<(usize, usize, Script)> {
    let mut out = Vec::new();
    let mut piece_start = start;
    let mut current: Option<Script> = None;
    let mut i = start;
    while i < end {
        let Some(r) = input[i..end].chars().next() else {
            break;
        };
        let script = if is_kana(r) {
            Script::Kana
        } else {
            Script::Han
        };
        match current {
            Some(s) if s != script => {
                out.push((piece_start, i, s));
                piece_start = i;
            }
            _ => {}
        }
        current = Some(script);
        i += r.len_utf8();
    }
    if let Some(s) = current {
        out.push((piece_start, end, s));
    }
    out
}

fn is_kana(r: char) -> bool {
    matches!(r as u32,
        0x3041..=0x309F
        | 0x30A0..=0x30FF
        | 0x31F0..=0x31FF
        | 0xFF66..=0xFF9D)
}

/// 把一個英數詞切成（照字面）或（かな前綴 + 照字面後綴）的片段。
fn push_latin_atoms(
    atoms: &mut Vec<Atom>,
    core: &str,
    offset: usize,
    lookup: &dyn Lookup,
    cfg: &Config,
) {
    let whole = latin_piece(core, lookup, cfg);
    if let Some((kana, plain_lower)) = whole {
        atoms.push(Atom {
            start: offset,
            end: offset + core.len(),
            script: Script::Latin,
            mode: Some(Mode::Ja),
            kind: AtomKind::Word,
            kana: Some(kana),
            plain_lower,
        });
        return;
    }
    if let Some((at, kana)) = split_kana_prefix(core, lookup, cfg) {
        let prefix = &core[..at];
        let suffix = &core[at..];
        atoms.push(Atom {
            start: offset,
            end: offset + prefix.len(),
            script: Script::Latin,
            mode: Some(Mode::Ja),
            kind: AtomKind::Word,
            kana: Some(kana),
            plain_lower: is_plain_lower(prefix),
        });
        let suffix_at = offset + prefix.len();
        atoms.push(Atom {
            start: suffix_at,
            end: suffix_at + suffix.len(),
            script: if has_fullwidth_alnum(suffix) {
                Script::Mixed
            } else {
                Script::Latin
            },
            mode: Some(Mode::En),
            kind: AtomKind::Word,
            kana: None,
            plain_lower: false,
        });
        return;
    }
    atoms.push(Atom {
        start: offset,
        end: offset + core.len(),
        script: if has_fullwidth_alnum(core) {
            Script::Mixed
        } else if core.chars().all(|c| c.is_ascii_digit()) {
            Script::Digit
        } else {
            Script::Latin
        },
        mode: Some(Mode::En),
        kind: AtomKind::Word,
        kana: None,
        plain_lower: is_plain_lower(core),
    });
}

/// 整段英數詞能不能當成一個完整的かな讀音。
fn latin_piece(core: &str, lookup: &dyn Lookup, cfg: &Config) -> Option<(String, bool)> {
    if has_fullwidth_alnum(core) || has_ascii_upper(core) || !is_plain_ascii_lower(core) {
        return None;
    }
    let (kana, pending) = replay_romaji(core)?;
    if !pending.is_empty() || kana.is_empty() {
        return None;
    }
    if is_english_word(core, lookup, cfg) {
        return None;
    }
    Some((kana, is_plain_lower(core)))
}

/// 找出「かな前綴 + 英數後綴」的切點與前綴かな。
///
/// 後綴必須從英數字開始，而且本身還不該是完整的かな；多個合法切點取後綴
/// 最像英數段開頭者，分數相同時取較長的かな前綴。
fn split_kana_prefix(core: &str, lookup: &dyn Lookup, cfg: &Config) -> Option<(usize, String)> {
    if has_fullwidth_alnum(core) {
        return None;
    }
    let mut best: Option<(usize, String, i32)> = None;
    for (at, _) in core.char_indices().rev() {
        if at == 0 {
            continue;
        }
        let prefix = &core[..at];
        if !prefix.chars().all(|c| c.is_ascii_lowercase()) {
            continue;
        }
        let Some((kana, pending)) = replay_romaji(prefix) else {
            continue;
        };
        if !pending.is_empty() || kana.chars().count() < 3 {
            continue;
        }
        if is_english_word(prefix, lookup, cfg) {
            continue;
        }
        let suffix = &core[at..];
        let Some(first) = suffix.chars().next() else {
            continue;
        };
        if !(first.is_alphanumeric() || is_fullwidth_alnum(first)) {
            continue;
        }
        if replay_romaji(suffix).is_some_and(|(_, p)| p.is_empty())
            && !is_english_word(suffix, lookup, cfg)
        {
            continue;
        }
        let score = split_score(suffix, lookup, cfg);
        if score < 3 {
            // 只有大寫、數字、全角或已知英文詞開頭的後綴才值得切開；否則
            // `sentence` 這種字會在隨機的かな前綴被誤切。
            continue;
        }
        match &best {
            Some((_, _, old)) if *old >= score => {}
            _ => best = Some((at, kana, score)),
        }
    }
    best.map(|(at, kana, _)| (at, kana))
}

/// 後綴有多像一個英數段的開頭。
fn split_score(suffix: &str, lookup: &dyn Lookup, cfg: &Config) -> i32 {
    let Some(first) = suffix.chars().next() else {
        return 0;
    };
    if first.is_ascii_uppercase()
        || first.is_ascii_digit()
        || is_fullwidth_alnum(first)
        || matches!(first, '@' | '#')
    {
        return 4;
    }
    let leading: String = suffix
        .chars()
        .take_while(|c| c.is_alphanumeric() || is_fullwidth_alnum(*c))
        .collect();
    if leading.is_empty() {
        return 0;
    }
    if is_english_word(&leading, lookup, cfg) {
        return 3;
    }
    if leading
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || is_fullwidth_alnum(c))
    {
        2
    } else {
        1
    }
}

fn is_english_word(text: &str, lookup: &dyn Lookup, cfg: &Config) -> bool {
    let cfg = cfg.clone().normalized();
    if cfg.prefer_kana || text.chars().count() < cfg.min_english_len {
        return false;
    }
    lookup.has(text)
}

fn is_plain_ascii_lower(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase())
}

fn is_plain_lower(s: &str) -> bool {
    is_plain_ascii_lower(s)
}

/// 片語合併：以單一空白相連的全小寫英文字會一起判定，判定相同時併成一段。
///
/// 兩個字以上的英數片語裡，單一かな單位的短字（`a`）不當羅馬字，這樣
/// `desu is a good sentence` 會切成 `desu`（日文）與整句英文片語。只有一個字
/// 時仍依一般規則判定，所以單獨打的 `go` 是かな。
fn group_words(atoms: &mut Vec<Atom>, input: &str) {
    let mut i = 0usize;
    while i < atoms.len() {
        if !is_plain_word(atoms, i) {
            i += 1;
            continue;
        }
        let mut words = vec![i];
        let mut j = i;
        while j + 1 < atoms.len()
            && is_single_space(atoms.get(j), input)
            && is_plain_word(atoms, j + 1)
        {
            words.push(j + 1);
            j += 2;
        }
        if words.len() >= 2 {
            for &w in &words {
                if atoms[w].mode == Some(Mode::Ja) && kana_units(&atoms[w]) < 2 {
                    atoms[w].mode = Some(Mode::En);
                    atoms[w].kana = None;
                }
            }
        }
        i = j + 1;
    }

    let mut merged: Vec<Atom> = Vec::with_capacity(atoms.len());
    let mut i = 0usize;
    while i < atoms.len() {
        if !is_plain_word(atoms, i) {
            merged.push(atoms[i].clone());
            i += 1;
            continue;
        }
        let mode = atoms[i].mode;
        let mut atom = atoms[i].clone();
        let mut j = i + 1;
        while j + 1 < atoms.len()
            && is_single_space(atoms.get(j), input)
            && is_plain_word(atoms, j + 1)
            && atoms[j + 1].mode == mode
        {
            atom.end = atoms[j + 1].end;
            atom.kana = match (&atom.kana, &atoms[j + 1].kana) {
                (Some(a), Some(b)) => Some(format!("{a}{b}")),
                _ => None,
            };
            j += 2;
        }
        merged.push(atom);
        i = j;
    }
    *atoms = merged;
}

fn is_plain_word(atoms: &[Atom], i: usize) -> bool {
    atoms
        .get(i)
        .is_some_and(|a| a.kind == AtomKind::Word && a.plain_lower)
}

fn is_single_space(atom: Option<&Atom>, input: &str) -> bool {
    atom.is_some_and(|a| a.kind == AtomKind::Symbol && &input[a.start..a.end] == " ")
}

fn kana_units(atom: &Atom) -> usize {
    atom.kana.as_deref().map_or(0, |k| k.chars().count())
}

fn resolve_han(atoms: &mut [Atom]) {
    let modes: Vec<Option<Mode>> = atoms.iter().map(|a| a.mode).collect();
    for i in 0..atoms.len() {
        if atoms[i].kind != AtomKind::Han || atoms[i].mode.is_some() {
            continue;
        }
        let previous = modes[..i].iter().rev().find_map(|m| *m);
        let next = modes[i + 1..].iter().find_map(|m| *m);
        let mode = match (previous, next) {
            (Some(Mode::Ja), _) | (_, Some(Mode::Ja)) => Mode::Ja,
            (Some(m), _) => m,
            (None, Some(m)) => m,
            (None, None) => Mode::Zh,
        };
        atoms[i].mode = Some(mode);
    }
}

fn resolve_symbols(atoms: &mut [Atom]) {
    let modes: Vec<Option<Mode>> = atoms.iter().map(|a| a.mode).collect();
    for i in 0..atoms.len() {
        if atoms[i].kind != AtomKind::Symbol || atoms[i].mode.is_some() {
            continue;
        }
        let previous = modes[..i].iter().rev().find_map(|m| *m);
        let next = modes[i + 1..].iter().find_map(|m| *m);
        let mode = previous.or(next).unwrap_or(Mode::En);
        atoms[i].mode = Some(mode);
    }
}

fn script_family(script: Script) -> u8 {
    match script {
        Script::Kana => 0,
        Script::Han => 1,
        Script::Latin | Script::Digit | Script::Mixed | Script::Other => 2,
        Script::Bopomofo => 3,
        Script::Symbol => 4,
    }
}

fn merge_atoms(atoms: &[Atom], input: &str) -> Vec<Plan> {
    let mut plans: Vec<Plan> = Vec::new();
    for atom in atoms {
        let mode = atom.mode.unwrap_or(Mode::En);
        if let Some(last) = plans.last_mut() {
            if last.mode == mode && script_family(last.script) == script_family(atom.script) {
                last.end = atom.end;
                match atom.kind {
                    AtomKind::Kana => last.kana.push_str(&input[atom.start..atom.end]),
                    AtomKind::Word if mode == Mode::Ja => match &atom.kana {
                        Some(kana) => last.kana.push_str(kana),
                        None => last.kana_ok = false,
                    },
                    AtomKind::Zhuyin | AtomKind::Symbol => {}
                    AtomKind::Han => last.kana_ok = false,
                    AtomKind::Word => {}
                }
                continue;
            }
        }
        let mut plan = Plan {
            start: atom.start,
            end: atom.end,
            mode,
            script: atom.script,
            kana: String::new(),
            kana_ok: true,
        };
        match atom.kind {
            AtomKind::Kana => plan.kana = input[atom.start..atom.end].to_string(),
            AtomKind::Word if mode == Mode::Ja => match &atom.kana {
                Some(kana) => plan.kana = kana.clone(),
                None => plan.kana_ok = false,
            },
            AtomKind::Word | AtomKind::Zhuyin | AtomKind::Symbol => {}
            AtomKind::Han => plan.kana_ok = false,
        }
        plans.push(plan);
    }
    plans
}

// ---------------------------------------------------------------------------
// ASCII 按鍵推斷
// ---------------------------------------------------------------------------

/// 待定按鍵的一種解讀結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsciiPiece {
    /// 用聲調鍵完成的注音音節。
    Zh(Syllable),
    /// 照字面留下的英文、數字或標點。
    Literal(String),
}

/// 查注音讀音是否能用目前的詞典解釋。
///
/// 對照前身 `mixedReachable` 與 `readingContinues`：音節本身有詞條或詞頭，
/// 或接在前一個中文音節後面能成詞。
fn reading_reachable(reading: &str, pieces: &[AsciiPiece], lexicon: &dyn Lexicon) -> bool {
    if !lexicon.lookup(reading).is_empty() || !lexicon.prefix(reading).is_empty() {
        return true;
    }
    let Some(AsciiPiece::Zh(previous)) = pieces.last() else {
        return false;
    };
    let sequence = format!("{} {reading}", previous.as_str());
    !lexicon.lookup(&sequence).is_empty() || !lexicon.prefix(&sequence).is_empty()
}

/// 用實體按鍵組成組音器。
///
/// 與 [`crate::zh::input::ZhInput`] 的差別：這裡是英數混打的待定按鍵，同一
/// 類別重複出現時整串不算注音（`hello` 的 h／e 都是聲母），不做同類別覆蓋。
fn compose_keys(raw: &str) -> Option<Composer> {
    let keyboard = Keyboard::standard();
    let mut composer = Composer::new();
    let mut seen = [false; 3];
    for key in raw.chars() {
        let symbol = keyboard.symbol(key)?;
        let class = class_of(symbol);
        let index = match class {
            Class::Initial => 0,
            Class::Medial => 1,
            Class::Final => 2,
            _ => return None,
        };
        if seen[index] {
            return None;
        }
        seen[index] = true;
        composer.add(symbol);
    }
    if composer.empty() {
        None
    } else {
        Some(composer)
    }
}

/// 後段按鍵是否照聲母、介音、韻母的順序打。
fn keys_in_syllable_order(keys: &str) -> bool {
    let keyboard = Keyboard::standard();
    let mut last = Class::None;
    for key in keys.chars() {
        let Some(symbol) = keyboard.symbol(key) else {
            return false;
        };
        let class = class_of(symbol);
        if class == Class::None || class <= last {
            return false;
        }
        last = class;
    }
    !keys.is_empty()
}

fn ascii_letters(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|ch| ch.is_ascii_alphabetic())
}

/// `raw` 加上 `tone` 是否能組成詞典解釋得了的音節。
fn mixed_reading(
    raw: &str,
    tone: char,
    pieces: &[AsciiPiece],
    lexicon: &dyn Lexicon,
) -> Option<Syllable> {
    let mut composer = compose_keys(raw)?;
    let syllable = composer.add(tone)?;
    reading_reachable(syllable.as_str(), pieces, lexicon).then_some(syllable)
}

/// 英數混打的待定按鍵：累積原始按鍵，聲調或空白才決定是注音還是英文。
///
/// 對應前身 `core/session/mixed.go` 的 `pendingKeys`、`mixedText`、
/// `finishMixedTone` 與 `mixedBoundary` 的核心規則：
///
/// - 整串按鍵先試著組成注音音節，接聲調或空白後能成詞就當中文；
/// - 組不成時從最長的英文前段切開，前段照字面、後段接聲調當中文
///   （`helloji3` → `hello我`、`ji3hellot ` → `我hello吃`）；
/// - 完整英文詞接空白時保留空白（`hello `）；
/// - 都不是時整串照字面留下。
#[derive(Debug, Clone, Default)]
pub struct AsciiInput {
    pending: String,
    pieces: Vec<AsciiPiece>,
}

impl AsciiInput {
    /// 建立空狀態。
    pub fn new() -> Self {
        Self::default()
    }

    /// 目前的原始按鍵。
    pub fn pending(&self) -> &str {
        &self.pending
    }

    /// 已完成的片段。
    pub fn pieces(&self) -> &[AsciiPiece] {
        &self.pieces
    }

    /// 是否沒有任何內容。
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.pieces.is_empty()
    }

    /// 清空。
    pub fn reset(&mut self) {
        self.pending.clear();
        self.pieces.clear();
    }

    /// 已完成的注音音節，依序。
    pub fn syllables(&self) -> Vec<Syllable> {
        self.pieces
            .iter()
            .filter_map(|piece| match piece {
                AsciiPiece::Zh(syllable) => Some(syllable.clone()),
                AsciiPiece::Literal(_) => None,
            })
            .collect()
    }

    /// 加一個按鍵；空白與聲調鍵交給 [`AsciiInput::space`]／
    /// [`AsciiInput::tone`]。
    pub fn press(&mut self, ch: char) {
        self.pending.push(ch);
    }

    /// 刪掉最後一個按鍵；待定按鍵沒有時刪掉最後一個片段的一個字。
    pub fn backspace(&mut self) -> bool {
        if self.pending.pop().is_some() {
            return true;
        }
        match self.pieces.last_mut() {
            Some(AsciiPiece::Literal(text)) => {
                text.pop();
                if text.is_empty() {
                    self.pieces.pop();
                }
                true
            }
            Some(AsciiPiece::Zh(_)) => {
                self.pieces.pop();
                true
            }
            None => false,
        }
    }

    /// 空白：完成待定按鍵。
    ///
    /// 先試一聲讀音，再試英→中切點，最後照字面留下；完整英文詞保留空白。
    pub fn space(&mut self, lexicon: &dyn Lexicon) {
        if self.pending.is_empty() {
            self.push_literal(" ");
            return;
        }
        if let Some(syllable) = mixed_reading(&self.pending, TONE1, &self.pieces, lexicon) {
            self.push_zh(syllable);
            return;
        }
        if let Some((at, syllable)) = self.boundary(TONE1, lexicon) {
            self.split_literal(at);
            self.push_zh(syllable);
            return;
        }
        let raw = std::mem::take(&mut self.pending);
        if prototype().has(&raw) {
            self.push_literal(&format!("{raw} "));
        } else {
            self.push_literal(&raw);
        }
    }

    /// 聲調鍵：完成待定按鍵，或照字面留下。
    ///
    /// 回傳 `true` 表示按鍵已被待定按鍵吸收。
    pub fn tone(&mut self, key: char, lexicon: &dyn Lexicon) -> bool {
        let Some(symbol) = Keyboard::standard().symbol(key) else {
            return false;
        };
        if class_of(symbol) != Class::Tone {
            return false;
        }
        if self.pending.is_empty() {
            return false;
        }
        if let Some(syllable) = mixed_reading(&self.pending, symbol, &self.pieces, lexicon) {
            self.push_zh(syllable);
            return true;
        }
        if let Some((at, syllable)) = self.boundary(symbol, lexicon) {
            self.split_literal(at);
            self.push_zh(syllable);
            return true;
        }
        self.pending.push(key);
        true
    }

    /// 送出前把剩下的待定按鍵照字面留下。
    pub fn flush(&mut self) {
        if !self.pending.is_empty() {
            let raw = std::mem::take(&mut self.pending);
            self.push_literal(&raw);
        }
    }

    /// 英文自動補字建議：前綴補完優先，其次差一鍵的修正。
    pub fn suggestion(&self) -> Option<String> {
        let raw = &self.pending;
        if raw.len() < 2 || !ascii_letters(raw) || prototype().has(raw) {
            return None;
        }
        for suggestion in prototype().prefix(raw, 5) {
            if suggestion.text.chars().count() >= raw.chars().count() + 2 {
                return Some(suggestion.text);
            }
        }
        let lower = raw.to_lowercase();
        let mut best: Option<(f64, &str)> = None;
        for word in prototype().words() {
            if !one_edit_apart(&lower, &word.text) {
                continue;
            }
            if best.is_none_or(|(weight, _)| word.weight > weight) {
                best = Some((word.weight, &word.text));
            }
        }
        best.map(|(_, text)| text.to_string())
    }

    /// 接受英文補字建議。
    pub fn accept_suggestion(&mut self) -> bool {
        match self.suggestion() {
            Some(text) => {
                self.pending = text;
                true
            }
            None => false,
        }
    }

    /// 找英→中切點：回傳（前段長度, 後段音節）。
    fn boundary(&self, tone: char, lexicon: &dyn Lexicon) -> Option<(usize, Syllable)> {
        let raw = &self.pending;
        let chars: Vec<char> = raw.chars().collect();
        for at in (1..chars.len()).rev() {
            let prefix: String = chars[..at].iter().collect();
            let rest: String = chars[at..].iter().collect();
            if !ascii_letters(&prefix) || !prototype().has(&prefix) {
                continue;
            }
            if !keys_in_syllable_order(&rest) {
                continue;
            }
            if let Some(syllable) = mixed_reading(&rest, tone, &self.pieces, lexicon) {
                return Some((prefix.len(), syllable));
            }
        }
        None
    }

    fn split_literal(&mut self, at: usize) {
        let prefix: String = self.pending.chars().take(at).collect();
        self.pending = self.pending.chars().skip(at).collect();
        if !prefix.is_empty() {
            self.push_literal(&prefix);
        }
    }

    fn push_zh(&mut self, syllable: Syllable) {
        self.pending.clear();
        self.pieces.push(AsciiPiece::Zh(syllable));
    }

    fn push_literal(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(AsciiPiece::Literal(last)) = self.pieces.last_mut() {
            last.push_str(text);
        } else {
            self.pieces.push(AsciiPiece::Literal(text.to_string()));
        }
    }

    /// 組字區顯示：已完成的片段加上待定按鍵。
    pub fn display(&self) -> String {
        let mut out = String::new();
        for piece in &self.pieces {
            match piece {
                AsciiPiece::Zh(syllable) => out.push_str(syllable.as_str()),
                AsciiPiece::Literal(text) => out.push_str(text),
            }
        }
        out.push_str(&self.pending_display());
        out
    }

    /// 待定按鍵的顯示字串。
    pub fn display_pending(&self) -> String {
        self.pending_display()
    }

    /// 待定按鍵的顯示：能組成注音時顯示注音，英文或打不成注音時照字面。
    fn pending_display(&self) -> String {
        if self.pending.is_empty() {
            return String::new();
        }
        if prototype().has(&self.pending) {
            return self.pending.clone();
        }
        match compose_keys(&self.pending) {
            Some(composer) => composer.string(),
            None => self.pending.clone(),
        }
    }
}

/// 兩個字串是否只差一鍵（含相鄰互換）。
fn one_edit_apart(left: &str, right: &str) -> bool {
    let a: Vec<char> = left.chars().collect();
    let b: Vec<char> = right.chars().collect();
    if a == b {
        return false;
    }
    let (long, short) = if a.len() >= b.len() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    if long.len() - short.len() > 1 {
        return false;
    }
    let mut i = 0;
    let mut j = 0;
    let mut edits = 0;
    while i < long.len() && j < short.len() {
        if long[i] == short[j] {
            i += 1;
            j += 1;
            continue;
        }
        edits += 1;
        if edits > 1 {
            return false;
        }
        if long.len() == short.len() {
            if i + 1 < long.len()
                && j + 1 < short.len()
                && long[i + 1] == short[j]
                && long[i] == short[j + 1]
            {
                return true;
            }
            i += 1;
            j += 1;
        } else {
            i += 1;
        }
    }
    edits + (long.len() - i) + (short.len() - j) <= 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ja::decode::JaWordSource;
    use crate::ja::lexicon::{JaEntry, Split};
    use crate::lattice::InMemoryLexicon;
    use std::sync::Arc;

    fn english() -> English {
        English::new(Some(prototype().clone()))
    }

    fn mode_of(name: &str) -> Mode {
        match name {
            "zh" => Mode::Zh,
            "ja" => Mode::Ja,
            _ => Mode::En,
        }
    }

    fn script_of(name: &str) -> Script {
        match name {
            "zhuyin" | "bopomofo" => Script::Bopomofo,
            "kana" => Script::Kana,
            "latin" => Script::Latin,
            "han" => Script::Han,
            "symbol" => Script::Symbol,
            other => Script::from_name(other).unwrap_or(Script::Other),
        }
    }

    fn byte_of_rune(input: &str, n: u64) -> usize {
        input
            .char_indices()
            .nth(n as usize)
            .map_or(input.len(), |(i, _)| i)
    }

    #[test]
    fn test_classify() {
        let e = english();
        let cases: &[(&str, Kind)] = &[
            ("hello", Kind::English),
            ("world", Kind::English),
            ("sake", Kind::Latin),
            ("don't", Kind::Latin),
            ("API", Kind::English),
            ("user_name", Kind::Identifier),
            (r"C:\Users", Kind::Identifier),
            ("search?q=1", Kind::Identifier),
            ("abc123", Kind::Identifier),
            ("C++", Kind::Identifier),
            ("3d", Kind::Identifier),
            ("https://example.com/a?b=1", Kind::Url),
            ("http://日本語", Kind::Other),
            ("www.example.com", Kind::Url),
            ("example.com", Kind::Url),
            ("foo@example.com", Kind::Email),
            ("foo@bar", Kind::Identifier),
            ("ＡＢＣ１２３", Kind::FullWidthAlnum),
            ("123", Kind::Number),
            ("今日は", Kind::Japanese),
            ("hello.", Kind::English),
            ("", Kind::Other),
        ];
        for &(text, want) in cases {
            assert_eq!(
                classify(text, Some(&e as &dyn Lookup)),
                want,
                "classify({text:?})"
            );
        }
    }

    #[test]
    fn test_literal_kinds() {
        for kind in [
            Kind::English,
            Kind::Latin,
            Kind::Url,
            Kind::Email,
            Kind::Identifier,
            Kind::FullWidthAlnum,
            Kind::Number,
        ] {
            assert!(kind.is_literal(), "{} 應照字面", kind.as_str());
        }
        for kind in [Kind::Japanese, Kind::Other] {
            assert!(!kind.is_literal(), "{} 不應照字面", kind.as_str());
        }
    }

    #[test]
    fn test_is_url_email_identifier() {
        for u in [
            "https://example.com",
            "http://example.com/a?b=c#d",
            "ftp://files.example.com/x",
            "//example.com/x",
            "www.example.com",
            "example.com",
            "example.com:8080/x",
            "https://example.com.",
        ] {
            assert!(is_url(u), "is_url({u:?}) 應為 true");
        }
        for u in [
            "hello",
            "hello world",
            "foo@example.com",
            "今日はhttps://example.com",
            "example",
        ] {
            assert!(!is_url(u), "is_url({u:?}) 應為 false");
        }

        for m in [
            "foo@example.com",
            "a.b+c@example.co.jp",
            "user_name@example.com.",
        ] {
            assert!(is_email(m), "is_email({m:?}) 應為 true");
        }
        for m in [
            "foo@bar",
            "@example.com",
            "foo@@example.com",
            "foo@example",
            "今日@example.com",
        ] {
            assert!(!is_email(m), "is_email({m:?}) 應為 false");
        }

        for id in [
            "user_name",
            r"C:\Users",
            "search?q=1",
            "abc123",
            "C++",
            "hello-world",
            "foo@bar",
        ] {
            assert!(is_identifier(id), "is_identifier({id:?}) 應為 true");
        }
        for id in ["hello", "今日は", "123", "---"] {
            assert!(!is_identifier(id), "is_identifier({id:?}) 應為 false");
        }
    }

    #[test]
    fn test_scan() {
        let e = english();
        let text = "今日はhello. APIとpython3の話。";
        let tokens = scan(text, Some(&e as &dyn Lookup));
        let mut got = Vec::new();
        for tok in &tokens {
            assert_eq!(&text[tok.start..tok.end], tok.text, "token offset 不一致");
            got.push(format!("{}:{}", tok.kind.as_str(), tok.text));
        }
        let want = vec![
            "japanese:今日は",
            "english:hello",
            "other:.",
            "english:API",
            "japanese:と",
            "identifier:python3",
            "japanese:の話",
            "other:。",
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn test_scan_url_and_email_boundaries() {
        let e = english();
        let text = "URLはhttps://example.com/a?b=1です。mailはfoo@example.com, まで。";
        let tokens = scan(text, Some(&e as &dyn Lookup));
        let kinds: HashMap<&str, Kind> = tokens.iter().map(|t| (t.text.as_str(), t.kind)).collect();
        assert_eq!(kinds.get("https://example.com/a?b=1"), Some(&Kind::Url));
        assert_eq!(kinds.get("foo@example.com"), Some(&Kind::Email));
        assert_eq!(kinds.get(","), Some(&Kind::Other));
    }

    #[test]
    fn test_space_text_default_no_space() {
        for text in ["今日はhello", "hello今日", "API文件", "こんにちはworld"] {
            assert_eq!(space_text(text, Spacing::None), text);
        }
    }

    #[test]
    fn test_space_text_always() {
        let cases: &[(&str, &str)] = &[
            ("今日はhello", "今日は hello"),
            ("hello今日", "hello 今日"),
            ("API文件", "API 文件"),
            ("こんにちはworld", "こんにちは world"),
            ("今日はhello世界", "今日は hello 世界"),
            ("hello world", "hello world"),
            ("今日は hello", "今日は hello"),
            ("今日は、hello", "今日は、hello"),
            ("Python3と3月", "Python3と3月"),
            ("ＡＢＣ文件", "ＡＢＣ 文件"),
            ("今日はhello、world", "今日は hello、world"),
            ("ＡＢＣ１２３", "ＡＢＣ１２３"),
            ("", ""),
        ];
        for &(input, want) in cases {
            let got = space_text(input, Spacing::Always);
            assert_eq!(got, want, "space_text({input:?})");
            assert_eq!(space_text(&got, Spacing::Always), want, "不是冪等");
        }
    }

    #[test]
    fn test_needs_space_between() {
        let cases: &[(char, char, bool)] = &[
            ('日', 'h', true),
            ('h', '日', true),
            ('ぁ', 'A', true),
            ('ー', 'x', true),
            ('Ａ', '日', true),
            ('日', 'Ａ', true),
            ('日', '本', false),
            ('h', 'i', false),
            ('日', '3', false),
            ('3', 'h', false),
            ('、', 'h', false),
            ('h', '、', false),
        ];
        for &(left, right, want) in cases {
            assert_eq!(needs_space_between(left, right), want, "{left:?} {right:?}");
        }
    }

    #[test]
    fn test_spacing_names() {
        assert_eq!(Spacing::default(), Spacing::None);
        assert_eq!(Spacing::None.as_str(), "none");
        assert_eq!(Spacing::Always.as_str(), "always");
    }

    #[test]
    fn test_width_conversion() {
        let cases: &[(&str, &str, &str)] = &[
            ("ＡＢＣ１２３", "ABC123", "ＡＢＣ１２３"),
            ("API123", "API123", "ＡＰＩ１２３"),
            ("ＡＢc１２", "ABc12", "ＡＢｃ１２"),
            ("今日はＡＰＩ", "今日はAPI", "今日はＡＰＩ"),
        ];
        for &(input, half, full) in cases {
            assert_eq!(
                to_halfwidth_latin(input),
                half,
                "to_halfwidth_latin({input:?})"
            );
            assert_eq!(
                to_fullwidth_latin(input),
                full,
                "to_fullwidth_latin({input:?})"
            );
        }
    }

    #[test]
    fn test_width_variant() {
        let cases: &[(&str, Option<&str>)] = &[
            ("API", Some("ＡＰＩ")),
            ("ＡＰＩ", Some("API")),
            ("API１２", Some("ＡＰＩ１２")),
            ("今日は", None),
            ("", None),
        ];
        for &(input, want) in cases {
            assert_eq!(
                width_variant(input).as_deref(),
                want,
                "width_variant({input:?})"
            );
        }
    }

    #[test]
    fn test_width_detectors() {
        assert!(has_ascii_alnum("abc"));
        assert!(!has_ascii_alnum("今日"));
        assert!(has_fullwidth_alnum("ＡＢＣ"));
        assert!(!has_fullwidth_alnum("ABC"));
        for r in "ＡＺａｚ０９".chars() {
            assert!(is_fullwidth_alnum(r), "is_fullwidth_alnum({r:?})");
        }
        assert!(!is_fullwidth_alnum('A'));
        assert!(!is_fullwidth_alnum('日'));
    }

    #[test]
    fn test_english_suggest_and_complete() {
        let e = english();
        assert!(e.has("hello"));
        assert!(!e.has("nope"));
        let suggestions = e.suggestions("hel", 8);
        assert_eq!(suggestions.first().map(|s| s.text.as_str()), Some("hello"));
        assert_eq!(e.complete("hel").as_deref(), Some("hello"));
        assert_eq!(e.complete("hello"), None);
        let caps = e.suggestions("HEL", 8);
        assert_eq!(caps.first().map(|s| s.text.as_str()), Some("HELLO"));
    }

    #[test]
    fn test_english_learn_and_forget() {
        let mut e = English::new(None);
        assert!(!e.learned("kubectl"));
        assert!(e.learn("kubectl"));
        e.learn("kubectl");
        e.learn("kubeadm");
        assert!(e.learned("KUBECTL"));
        assert!(e.has("Kubectl"));
        let suggestions = e.suggestions("kube", 8);
        assert!(suggestions.len() >= 2);
        assert!(suggestions[0].learned);
        assert_eq!(suggestions[0].text, "kubectl");
        let words = e.learned_words();
        assert_eq!(words.len(), 2);
        assert_eq!(words[0], "kubectl");
        assert!(e.forget("KUBECTL"));
        assert!(!e.learned("kubectl"));
        assert!(!e.forget("kubectl"));
        for bad in ["", "   ", "has space", "line\nbreak"] {
            assert!(!e.learn(bad), "learn({bad:?}) 應被拒");
        }
    }

    #[test]
    fn test_english_candidates() {
        let mut e = English::new(None);
        e.learn("kubectl");
        let cands = e.candidates("kub", 8);
        assert_eq!(cands.first().map(|c| c.surface.as_str()), Some("kubectl"));
        assert_eq!(cands[0].source, CandidateSource::Personal);
        assert!(cands.iter().all(|c| matches!(
            c.source,
            CandidateSource::Personal | CandidateSource::Lexicon
        )));
        assert!(cands.iter().any(|c| c.surface == "kubernetes"));
    }

    #[test]
    fn test_dictionary_prefix_and_case() {
        let d = Dictionary::new([
            Word::new("meeting", 10.0),
            Word::new("meet", 1.0),
            Word::new("met", 9.0),
            Word::new("MEETING", 2.0),
        ]);
        assert!(d.has("Meeting"));
        assert!(!d.has("meetng"));
        let got = d.prefix("Me", 2);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].text, "Meeting");
        assert_eq!(got[1].text, "Met");
        assert!(d.prefix("me_", 3).is_empty());
        assert_eq!(d.len(), 3);
    }

    #[test]
    fn test_dictionary_decode_roundtrip() {
        let payload =
            b"0cEN\x00\x01\x02\x00\x00\x00\x04meet\x00\x00\x00\x00\x00\x00\xf0\x3f\x00\x05meeti\
\x00\x00\x00\x00\x00\x00\x00\x40\x00";
        let d = Dictionary::decode(payload).expect("解碼");
        assert_eq!(d.len(), 2);
        assert!(d.has("meet"));
        assert!(d.has("meeti"));
        assert!(!d.has("mee"));
        assert_eq!(
            Dictionary::decode(b"0cEN\x00\x01\x01\x00\x00\x00\x00\x00")
                .unwrap_err()
                .to_string(),
            "英文詞表格式不合法：詞條截斷或過長"
        );
    }

    /// 測試用的日文詞條來源。
    struct MemorySource {
        entries: Vec<JaEntry>,
    }

    impl MemorySource {
        fn new(rows: &[(&str, &str, i32, &str)]) -> Self {
            Self {
                entries: rows
                    .iter()
                    .map(|&(surface, reading, cost, pos)| JaEntry {
                        surface: surface.into(),
                        reading: reading.into(),
                        pos: Arc::from(pos),
                        conj_type: Arc::from("*"),
                        conj_form: Arc::from("*"),
                        cost,
                        split: Split::B,
                    })
                    .collect(),
            }
        }
    }

    impl JaWordSource for MemorySource {
        fn longest_reading_units(&self) -> usize {
            self.entries.iter().map(|e| e.units()).max().unwrap_or(0)
        }

        fn matches<'a>(
            &'a self,
            reading: &[char],
            start: usize,
            out: &mut dyn FnMut(crate::ja::decode::JaWordMatch<'a>),
        ) {
            for entry in &self.entries {
                let units: Vec<char> = entry.reading.chars().collect();
                if reading[start..].starts_with(&units) {
                    out(crate::ja::decode::JaWordMatch {
                        end: start + units.len(),
                        entry,
                        left_id: None,
                        right_id: None,
                    });
                }
            }
        }

        fn exact(&self, reading: &[char]) -> Vec<&JaEntry> {
            let key: String = reading.iter().collect();
            self.entries
                .iter()
                .filter(|e| e.reading.as_ref() == key)
                .collect()
        }

        fn prefixed(&self, reading: &[char]) -> Vec<&JaEntry> {
            let key: String = reading.iter().collect();
            self.entries
                .iter()
                .filter(|e| e.reading.starts_with(&key))
                .collect()
        }
    }

    fn ja_decoder() -> JaDecoder {
        let source = MemorySource::new(&[
            ("今日", "きょう", 100, "名詞,普通名詞,一般,*"),
            ("京", "きょう", 300, "名詞,普通名詞,一般,*"),
            ("は", "は", 100, "助詞,係助詞,*"),
            ("です", "です", 100, "助動詞,*,*"),
        ]);
        JaDecoder::new(Arc::new(source))
    }

    fn zh_fixture() -> (InMemoryLexicon, ScoreContext) {
        let mut lex = InMemoryLexicon::new();
        lex.push(crate::lattice::LexEntry::new("大", "ㄉㄚˋ", 5, 0));
        lex.push(crate::lattice::LexEntry::new("雞", "ㄐㄧ", 5, 0));
        lex.push(crate::lattice::LexEntry::new("巴", "ㄅㄚ", 5, 0));
        lex.push(crate::lattice::LexEntry::new("麻婆", "ㄇㄚˊ ㄆㄛˊ", 2, 0));
        lex.push(crate::lattice::LexEntry::new("豆腐", "ㄉㄡˋ ㄈㄨˇ", 2, 0));
        let ctx = ScoreContext {
            length_bonus: 0,
            max_word_units: 8,
            ..Default::default()
        };
        (lex, ctx)
    }

    #[test]
    fn test_detect_mode() {
        let r = Router::new();
        assert_eq!(r.detect("ㄓㄨˋ"), Mode::Zh);
        assert_eq!(r.detect("かな"), Mode::Ja);
        assert_eq!(r.detect("hello"), Mode::En);
        assert_eq!(r.detect("watashiwaㄉㄚˋdesu"), Mode::Mixed);
        assert_eq!(r.detect(""), Mode::En);
    }

    #[test]
    fn test_segments_use_utf8_offsets_and_modes() {
        let r = Router::new();
        let input = "watashiwaㄉㄚˋㄐㄧdesu";
        let segs = r.segments(input);
        assert_eq!(segs.len(), 3);
        assert_eq!(&input[segs[0].start..segs[0].end], "watashiwa");
        assert_eq!(segs[0].mode, Mode::Ja);
        assert_eq!(segs[0].script, Script::Latin);
        assert_eq!(&input[segs[1].start..segs[1].end], "ㄉㄚˋㄐㄧ");
        assert_eq!(segs[1].mode, Mode::Zh);
        assert_eq!(segs[1].script, Script::Bopomofo);
        assert_eq!(&input[segs[2].start..segs[2].end], "desu");
        assert_eq!(segs[2].mode, Mode::Ja);

        // 位元組位移：注音每個符號 3 bytes，接在 9 bytes 後面。
        assert_eq!(segs[1].start, 9);
        assert_eq!(segs[1].end, 9 + 6 * 3);
        assert_eq!(segs[2].start, segs[1].end);
    }

    #[test]
    fn test_literal_runs_are_not_converted() {
        let e = english();
        let ja = ja_decoder();
        let r = Router::new().with_english(&e).with_ja(&ja);
        let result = r.convert("kyouhaAPI").expect("轉換");
        assert_eq!(result.surface, "今日はAPI");
        assert_eq!(result.languages(), vec![Mode::Ja, Mode::En]);
        assert_eq!(result.segments[1].surface, "API");

        let url = "https://example.com/a?b=1#x";
        let result = r.convert(&format!("kyouha{url}")).expect("轉換");
        assert_eq!(result.surface, format!("今日は{url}"));
        assert_eq!(result.languages(), vec![Mode::Ja, Mode::En]);
    }

    #[test]
    fn test_route_email_and_url_are_literal() {
        let e = english();
        let ja = ja_decoder();
        let r = Router::new().with_english(&e).with_ja(&ja);
        let result = r.convert("kyouhafoo@example.com").expect("轉換");
        assert_eq!(result.surface, "今日はfoo@example.com");
        assert_eq!(result.languages(), vec![Mode::Ja, Mode::En]);
    }

    #[test]
    fn test_route_zh_segment() {
        let (lex, ctx) = zh_fixture();
        let r = Router::new().with_zh(&lex, &ctx);
        let result = r.convert("ㄉㄚˋㄐㄧㄅㄚ").expect("轉換");
        assert_eq!(result.surface, "大雞巴");
        assert_eq!(result.languages(), vec![Mode::Zh]);
        assert_eq!(result.segments[0].input_start, 0);
        assert_eq!(result.segments[0].end_utf16, 3);
        assert_eq!(result.cursor_utf16, 3);
    }

    #[test]
    fn test_route_without_decoders_falls_back_to_reading() {
        let r = Router::new();
        let result = r.convert("kyouhaAPI").expect("轉換");
        assert_eq!(result.surface, "きょうはAPI");
        let result = r.convert("ㄉㄚˋㄐㄧ").expect("轉換");
        assert_eq!(result.surface, "ㄉㄚˋㄐㄧ");
    }

    #[test]
    fn test_route_empty_input() {
        let r = Router::new();
        let result = r.convert("").expect("轉換");
        assert_eq!(result.surface, "");
        assert!(result.segments.is_empty());
        assert_eq!(result.cursor_utf16, 0);
    }

    #[test]
    fn test_spacing_between_segments() {
        let ja = ja_decoder();
        let r = Router::new().with_ja(&ja).with_config(Config {
            spacing: Spacing::Always,
            ..Config::default()
        });
        let result = r.convert("kyouhahello").expect("轉換");
        assert_eq!(result.surface, "今日は hello");
        assert_eq!(result.cursor_utf16, "今日は hello".encode_utf16().count());
    }

    #[test]
    fn test_conversion_uses_output_offsets() {
        let (lex, ctx) = zh_fixture();
        let r = Router::new().with_zh(&lex, &ctx);
        let result = r.convert("ㄉㄚˋㄐㄧㄅㄚ").expect("轉換");
        let conversion = result.conversion();
        assert_eq!(conversion.surface, "大雞巴");
        assert_eq!(conversion.segments[0].start, 0);
        assert_eq!(conversion.segments[0].end, 9);
        assert_eq!(conversion.segments[0].mode, Mode::Zh);
    }

    #[test]
    fn test_english_prefix_completion() {
        let e = english();
        let r = Router::new().with_english(&e);
        let cands = r.prefixes("hel");
        assert_eq!(cands.first().map(|c| c.surface.as_str()), Some("hello"));
        let _ = r.prefixes("ㄓ");
        assert!(r.prefixes("ㄓ").is_empty());
    }

    #[test]
    fn test_kana_for_and_literal_for() {
        let cfg = Config::default();
        let e = english();
        let (kana, pending) = kana_for("kyouha", &cfg, Some(&e)).expect("羅馬字");
        assert_eq!(kana, "きょうは");
        assert_eq!(pending, "");
        assert!(kana_for("hello", &cfg, Some(&e)).is_none());
        assert!(kana_for("kubectl", &cfg, Some(&e)).is_none());
        assert!(literal_for("hello", &cfg, Some(&e)));
        assert!(literal_for("API", &cfg, Some(&e)));
        assert!(!literal_for("kyouha", &cfg, Some(&e)));
        assert!(literal_for("", &cfg, Some(&e)));
    }

    #[test]
    fn test_split_kana_prefix_before_english() {
        let e = english();
        let r = Router::new().with_english(&e);
        let segs = r.segments("kyouhaAPI");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].mode, Mode::Ja);
        assert_eq!(segs[1].mode, Mode::En);

        // 打不成合法羅馬字的英文字不拆。
        let segs = r.segments("kubectl");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].mode, Mode::En);
    }

    #[test]
    fn test_golden_mixed_cases() {
        let path = jc_testdata::require_data!("0c-testdata/golden/mixed/cases.json");
        let text = std::fs::read_to_string(&path).expect("讀取 golden");
        let doc: serde_json::Value = serde_json::from_str(&text).expect("解析 golden");
        let cases = doc["cases"].as_array().expect("cases");
        assert!(!cases.is_empty());
        let router = Router::new();
        for case in cases {
            let name = case["name"].as_str().unwrap_or("?");
            let input = case["input"].as_str().unwrap_or("");
            let want = case["segments"].as_array().expect("segments");
            let got = router.segments(input);
            assert_eq!(got.len(), want.len(), "{name}: 片段數");
            for (i, (seg, want)) in got.iter().zip(want).enumerate() {
                let start = byte_of_rune(input, want["span"]["start"].as_u64().unwrap_or(0));
                let end = byte_of_rune(input, want["span"]["end"].as_u64().unwrap_or(0));
                assert_eq!((seg.start, seg.end), (start, end), "{name}: 片段 {i} 邊界");
                let mode = mode_of(want["language"]["language"].as_str().unwrap_or("en"));
                assert_eq!(seg.mode, mode, "{name}: 片段 {i} 語言");
                let script = script_of(want["script"].as_str().unwrap_or("other"));
                assert_eq!(seg.script, script, "{name}: 片段 {i} 書寫系統");
            }
        }
    }

    #[test]
    fn test_long_words_are_joined_across_boundaries() {
        let (lex, ctx) = zh_fixture();
        let r = Router::new().with_zh(&lex, &ctx);
        let result = r.convert("ㄇㄚˊㄆㄛˊㄉㄡˋㄈㄨˇ").expect("轉換");
        assert_eq!(result.surface, "麻婆豆腐");

        let ja = ja_decoder();
        let r = Router::new().with_ja(&ja);
        let result = r.convert("きょうは").expect("轉換");
        assert_eq!(result.surface, "今日は");
    }

    #[test]
    fn test_primitives_stay_literal() {
        let e = english();
        let r = Router::new().with_english(&e);
        let result = r.convert("kubectl pCloud GitHub").expect("轉換");
        assert_eq!(result.surface, "kubectl pCloud GitHub");
        assert!(result.languages().iter().all(|&m| m == Mode::En));
    }
}
