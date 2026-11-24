//! 混合路由與英文直通。
//!
//! 同一段輸入可以同時含注音、日文羅馬字與英文，使用者不需要切換語言：
//!
//! 1. [`classify`] 與 [`scan`] 判斷英數詞的種類（英文、網址、email、識別字、
//!    全角英數、數字）；
//! 2. [`Router::segments`] 把輸入切成語言片段，`Segment::mode` 是該片段的語言
//!    （[`Lang::Zh`]、[`Lang::Ja`]、[`Lang::En`]），`script` 是字面種類；
//! 3. [`Router::convert`] 把每個片段交給對應的轉換器：注音走
//!    [`crate::decode::Decoder`]、日文走 [`crate::ja::decode::JaDecoder`]、
//!    英文照原樣直出；
//! 4. [`Router::detect`] 回報整段輸入的 [`Lang`]，只有單一語言時就不是
//!    [`Lang::Mixed`]。
//!
//! 判定規則沿用前身 `core/ja/mixed`：大寫、全角、數字、網址、email、識別字與
//! 學過的英文詞一律照字面；小寫且是英文詞典的完整單字也照字面；其餘能完整
//! 組成合法羅馬字的字才走日文。因此 `code`、`hello` 是英文，`sake`、
//! `watashiwa` 是羅馬字，`kubectl`、`github` 打不成合法羅馬字，也照字面。
//!
//! 語言邊界不加分隔符；游標位置以 UTF-16 code unit 計算（見
//! [`MixedResult::cursor_utf16`]）。

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::candidate::{Candidate, CandidateSource};
use crate::decode::{Conversion, Decoder, Locks};
use crate::ja::decode::{JaContext, JaDecoder};
use crate::ja::input::{RomajiEvent, RomajiInput};
use crate::key::Key;
use crate::lang::Lang;
use crate::lattice::{Lattice as ZhLattice, LatticeError, Lexicon};
use crate::score::ScoreContext;
use crate::segment::{detect as detect_script, Script, Segment};
use crate::zh::keyboard::Keyboard;
use crate::zh::pinyin::is_legal_base;
use crate::zh::syllable::{class_of, Class, TONE1};

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

/// 拼字建議每一鍵編輯距離加上的成本，讓建議排在完整前綴補完之後。
const CORRECTION_PENALTY: i32 = 1_000;

/// `words.bin` 的標頭；後面接小端序 `u32` 詞數。
const PACK_MAGIC: &[u8] = b"0cEN\x00\x01";

/// 資料包詞數上限。
const MAX_PACK_WORDS: usize = 200_000;

/// 資料包位元組上限。
const MAX_PACK_BYTES: usize = 16 << 20;

/// `manifest.json` 大小上限。
const MAX_MANIFEST_BYTES: u64 = 64 << 10;

/// `NOTICE.txt` 大小上限。
const MAX_NOTICE_BYTES: u64 = 64 << 10;

/// 沒有環境變數時使用的資料根目錄。
pub const DEFAULT_DATA_ROOT: &str = "/mnt/work/jc/jc-ime-data";

/// 資料根目錄下的英文資料包子目錄。
pub const ENGLISH_PACK_SUBDIR: &str = "english";

// ---------------------------------------------------------------------------
// 英文資料包
// ---------------------------------------------------------------------------

/// 英文資料包裡的一個檔案（`words.bin`、`NOTICE.txt`）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct EnglishPackFile {
    /// 明文內容的 SHA-256，小寫十六進位。
    pub sha256: String,
    /// 檔案位元組數。
    pub size: u64,
}

/// 英文詞表的來源與授權。
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct EnglishSource {
    /// 來源名稱。
    pub name: String,
    /// 來源版本。
    pub version: String,
    /// 來源網址。
    pub url: String,
    /// 上游下載檔的 SHA-256。
    pub sha256: String,
    /// 授權名稱。
    pub license: String,
    /// 姓名標示。
    pub attribution: String,
    /// 上游 commit；舊資料沒有這個欄位。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}

/// `manifest.json`：資料包的詞數、來源與檔案雜湊。
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct EnglishManifest {
    /// 格式版本，目前只有 1。
    pub format: u32,
    /// 詞數，必須與 `words.bin` 一致。
    pub words: usize,
    /// 排序依據的說明。
    pub ranking: String,
    /// 詞表來源。
    pub sources: Vec<EnglishSource>,
    /// 建置輸入檔的 SHA-256。
    #[serde(default)]
    pub inputs: BTreeMap<String, String>,
    /// 資料包內的檔案。
    #[serde(default)]
    pub files: BTreeMap<String, EnglishPackFile>,
}

impl EnglishManifest {
    /// 檢查格式、詞數、來源與檔案雜湊欄位是否完整。
    pub fn validate(&self) -> Result<(), MixedError> {
        if self.format != 1
            || self.words == 0
            || self.words > MAX_PACK_WORDS
            || self.ranking.is_empty()
            || self.sources.is_empty()
            || self.files.len() != 2
        {
            return Err(MixedError::Pack("manifest 格式不合法".to_string()));
        }
        for name in ["words.bin", "NOTICE.txt"] {
            match self.files.get(name) {
                Some(file) if file.size >= 1 && is_sha256_hex(&file.sha256) => {}
                _ => {
                    return Err(MixedError::Pack(format!(
                        "manifest 缺少 {name} 的雜湊或大小"
                    )))
                }
            }
        }
        for source in &self.sources {
            if source.name.is_empty()
                || source.version.is_empty()
                || source.url.is_empty()
                || source.license.is_empty()
                || source.attribution.is_empty()
                || !is_sha256_hex(&source.sha256)
            {
                return Err(MixedError::Pack("資料包缺少來源與授權".to_string()));
            }
        }
        Ok(())
    }
}

/// 回報 `text` 是不是 64 個小寫十六進位字元。
pub fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// 內容的 SHA-256，小寫十六進位。
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(data);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// 解析資料根目錄：`JC_IMEDATA`、`JC_TESTDATA`、[`DEFAULT_DATA_ROOT`]。
///
/// 環境變數指向 `0c-testdata` 時退回它的上層目錄，與 `jc-cli` 的慣例一致。
pub fn data_root() -> Option<PathBuf> {
    let candidate = std::env::var_os("JC_IMEDATA")
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var_os("JC_TESTDATA").filter(|v| !v.is_empty()))
        .map_or_else(|| PathBuf::from(DEFAULT_DATA_ROOT), PathBuf::from);
    data_root_from(&candidate)
}

fn data_root_from(candidate: &Path) -> Option<PathBuf> {
    if !candidate.is_dir() {
        return None;
    }
    if candidate.file_name().map(|n| n == "0c-testdata") == Some(true) {
        return candidate.parent().map(Path::to_path_buf);
    }
    Some(candidate.to_path_buf())
}

/// 英文資料包目錄：`JC_ENGLISH_PACK`，否則資料根目錄下的 `english/`。
pub fn english_pack_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("JC_ENGLISH_PACK").filter(|v| !v.is_empty()) {
        let dir = PathBuf::from(dir);
        return dir.is_dir().then_some(dir);
    }
    let dir = data_root()?.join(ENGLISH_PACK_SUBDIR);
    dir.is_dir().then_some(dir)
}

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

/// R1 的英數 run 可包含的標點；其中 `. , ; / -` 在大千鍵盤也可以當韻母，
/// `:` `!` `?` 永遠是符號。
const SYMBOL_PUNCT: &str = ".,;:/-!?";

/// 切分器接受的輸入：全小寫英數與 run 內標點。
///
/// 大寫、全角與其他符號一律不交給切分器，避免它把 ASCII 大小寫混排當成注音。
fn is_latin_split_input(text: &str) -> bool {
    !text.is_empty()
        && !has_fullwidth_alnum(text)
        && !text.chars().any(|c| c.is_ascii_uppercase())
        && text.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || c == ' ' || SYMBOL_PUNCT.contains(c)
        })
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
    /// 英文資料包驗證失敗。
    #[error("英文資料包不合法：{0}")]
    Pack(String),
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

/// 一個拼字建議。
#[derive(Debug, Clone, PartialEq)]
pub struct Correction {
    /// 建議的詞面（照查詢時的大小寫規則）。
    pub text: String,
    /// 與輸入的編輯距離。
    pub distance: usize,
    /// 詞表權重，越大越優先。
    pub weight: f64,
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
        Self::new_filtered(words, std::iter::empty())
    }

    /// 由詞條建立詞表，並把 `blocked` 標成不可推薦。
    ///
    /// 被標記的詞仍然認得（[`Dictionary::has`] 為真），但前綴補完與拼字
    /// 建議不會推薦它們。
    pub fn new_filtered<I, S>(words: I, blocked: S) -> Self
    where
        I: IntoIterator<Item = Word>,
        S: IntoIterator<Item = String>,
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
        let mut no_suggest = vec![false; words.len()];
        for text in blocked {
            let text = text.to_lowercase();
            let i = words.partition_point(|w| w.text < text);
            if words.get(i).map(|w| w.text.as_str()) == Some(text.as_str()) {
                no_suggest[i] = true;
            }
        }
        Self { words, no_suggest }
    }

    /// 讀取 `0cEN` v1 格式的詞表檔。
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MixedError> {
        let data = std::fs::read(path)?;
        Self::decode(&data)
    }

    /// 解析 `0cEN` v1 格式的詞表。
    pub fn decode(data: &[u8]) -> Result<Self, MixedError> {
        if data.len() > MAX_PACK_BYTES {
            return Err(MixedError::Format("資料包過大"));
        }
        let Some(rest) = data.strip_prefix(PACK_MAGIC) else {
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

    /// 編成 `0cEN` v1 詞表位元組。
    ///
    /// 只寫詞面、權重與不可推薦旗標；索引在 [`Dictionary::decode`] 重建。
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(5 + self.words.len() * 16);
        out.extend_from_slice(PACK_MAGIC);
        out.extend_from_slice(&(self.words.len() as u32).to_le_bytes());
        for (i, word) in self.words.iter().enumerate() {
            let len = word.text.len().min(MAX_WORD_BYTES);
            out.push(len as u8);
            out.extend_from_slice(&word.text.as_bytes()[..len]);
            out.extend_from_slice(&word.weight.to_le_bytes());
            out.push(u8::from(self.no_suggest[i]));
        }
        out
    }

    /// 讀取並驗證一個英文資料包目錄。
    ///
    /// 目錄必須含 `manifest.json`、`words.bin` 與非空的 `NOTICE.txt`，
    /// 兩個檔案的內容都要和 manifest 的大小與 SHA-256 一致。
    pub fn load_pack(dir: &Path) -> Result<(Self, EnglishManifest), MixedError> {
        let info = std::fs::symlink_metadata(dir)?;
        if !info.is_dir() || info.file_type().is_symlink() {
            return Err(MixedError::Pack("英文資料包必須是一般目錄".to_string()));
        }
        let manifest_bytes = read_regular_file(&dir.join("manifest.json"), MAX_MANIFEST_BYTES)?;
        let manifest: EnglishManifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|_| MixedError::Pack("manifest 不是合法 JSON".to_string()))?;
        manifest.validate()?;
        let mut files: HashMap<&str, Vec<u8>> = HashMap::new();
        for name in ["words.bin", "NOTICE.txt"] {
            let limit = if name == "words.bin" {
                MAX_PACK_BYTES as u64 + 64
            } else {
                MAX_NOTICE_BYTES
            };
            let data = read_regular_file(&dir.join(name), limit)?;
            let want = &manifest.files[name];
            if want.size != data.len() as u64 || want.sha256 != sha256_hex(&data) {
                return Err(MixedError::Pack(format!("{name} 與 manifest 不符")));
            }
            files.insert(name, data);
        }
        let notice = files.remove("NOTICE.txt").unwrap_or_default();
        if String::from_utf8_lossy(&notice).trim().is_empty() {
            return Err(MixedError::Pack("授權聲明不可空白".to_string()));
        }
        let dict = Self::decode(&files["words.bin"])?;
        if dict.len() != manifest.words {
            return Err(MixedError::Pack("manifest 詞數不符".to_string()));
        }
        Ok((dict, manifest))
    }

    /// 拼字建議：回傳與 `typed` 編輯距離 1（6 個字元以上容許 2）以內的詞。
    ///
    /// 已經在詞表裡的完整單字不會有建議；不可推薦的詞也不會出現。結果先依
    /// 距離、再依權重、最後依詞面排序。
    pub fn corrections(&self, typed: &str, limit: usize) -> Vec<Correction> {
        self.corrections_with_prior(typed, limit, 0.0)
    }

    /// 檢查可建議的編輯距離鄰居裡，有沒有權重達到 `minimum_prior` 的詞。
    ///
    /// 低權重的距離 1 詞不能把常見的距離 2 詞擠出前 k 名，因此這個查詢不看
    /// 候選上限。
    pub fn near_common(&self, typed: &str, minimum_prior: f64) -> bool {
        let valid_prior = minimum_prior.is_finite() && minimum_prior > 0.0;
        valid_prior
            && !self
                .corrections_with_prior(typed, 1, minimum_prior)
                .is_empty()
    }

    fn corrections_with_prior(
        &self,
        typed: &str,
        limit: usize,
        minimum_prior: f64,
    ) -> Vec<Correction> {
        if limit == 0
            || typed.len() > MAX_WORD_BYTES
            || !typed.chars().all(|c| c.is_ascii_alphabetic())
            || self.has(typed)
        {
            return Vec::new();
        }
        let lower = typed.to_lowercase();
        if !is_lower_ascii_word(&lower) || lower.len() < 3 {
            return Vec::new();
        }
        let max_edits = if lower.len() >= 6 { 2 } else { 1 };
        let limit = limit.min(MAX_SUGGESTIONS);
        let all_caps = typed
            .chars()
            .all(|c| !c.is_ascii_alphabetic() || c.is_ascii_uppercase());
        let title = typed.as_bytes().first().is_some_and(u8::is_ascii_uppercase);
        let mut out: Vec<Correction> = Vec::new();
        for (i, word) in self.words.iter().enumerate() {
            if self.no_suggest[i] || word.weight < minimum_prior {
                continue;
            }
            if word.text.len().abs_diff(lower.len()) > max_edits {
                continue;
            }
            let Some(distance) = osa_within(&lower, &word.text, max_edits) else {
                continue;
            };
            let text = if all_caps {
                word.text.to_uppercase()
            } else if title {
                capitalize(&word.text)
            } else {
                word.text.clone()
            };
            out.push(Correction {
                text,
                distance,
                weight: word.weight,
            });
        }
        out.sort_by(|a, b| {
            a.distance
                .cmp(&b.distance)
                .then_with(|| b.weight.total_cmp(&a.weight))
                .then_with(|| a.text.cmp(&b.text))
        });
        out.truncate(limit);
        out
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
uninstall package path url https http foo small large long short name value bar";
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

/// 以「最優字串對齊」距離比較兩個小寫詞；超過 `max_edits` 時回傳 `None`。
///
/// 容許插入、刪除、取代與相鄰互換，並在整列的距離都超過上限時提前放棄。
fn osa_within(left: &str, right: &str, max_edits: usize) -> Option<usize> {
    let a = left.as_bytes();
    let b = right.as_bytes();
    let m = b.len();
    let mut row_prev2 = vec![0usize; m + 1];
    let mut row_prev: Vec<usize> = (0..=m).collect();
    let mut row_cur = vec![0usize; m + 1];
    for i in 1..=a.len() {
        row_cur[0] = i;
        let mut row_min = i;
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (row_cur[j - 1] + 1)
                .min(row_prev[j] + 1)
                .min(row_prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(row_prev2[j - 2] + 1);
            }
            row_cur[j] = best;
            row_min = row_min.min(best);
        }
        if row_min > max_edits {
            return None;
        }
        std::mem::swap(&mut row_prev2, &mut row_prev);
        std::mem::swap(&mut row_prev, &mut row_cur);
    }
    let distance = row_prev[m];
    (distance <= max_edits).then_some(distance)
}

/// 把小寫詞的第一個字母轉大寫。
fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// 讀取一般檔案，拒絕符號連結與超過 `limit` 的內容。
fn read_regular_file(path: &Path, limit: u64) -> Result<Vec<u8>, MixedError> {
    let info = std::fs::symlink_metadata(path)?;
    let to_pack = |reason: &str| MixedError::Pack(format!("{reason}：{}", path.display()));
    if !info.file_type().is_file() {
        return Err(to_pack("不是一般檔案"));
    }
    if info.len() > limit {
        return Err(to_pack("檔案過大"));
    }
    let data = std::fs::read(path)?;
    if data.len() as u64 > limit {
        return Err(to_pack("檔案過大"));
    }
    Ok(data)
}

/// 英文查詢層：唯讀詞表加上個人學習詞。
#[derive(Debug, Clone)]
pub struct English {
    dict: Dictionary,
    learned: Vec<LearnedWord>,
    origin: Option<PathBuf>,
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
            origin: None,
        }
    }

    /// 讀取一個英文資料包目錄。
    pub fn from_pack_dir(dir: impl AsRef<Path>) -> Result<Self, MixedError> {
        let dir = dir.as_ref();
        let (dict, _manifest) = Dictionary::load_pack(dir)?;
        Ok(Self {
            dict,
            learned: Vec::new(),
            origin: Some(dir.join("words.bin")),
        })
    }

    /// 從指定的資料包目錄載入英文詞表。
    ///
    /// 與 [`English::from_pack_dir`] 相同，名稱給引擎明確帶入資料根目錄下的
    /// `english/` 使用；驗證規則仍由 [`Dictionary::load_pack`] 負責。
    pub fn load_from(dir: &Path) -> Result<Self, MixedError> {
        Self::from_pack_dir(dir)
    }

    /// 目錄存在就載入，不存在時回 `None` 並只記一筆 info。
    ///
    /// 目錄存在但驗證失敗時記 warn 並帶上原因，讓「沒安裝」與「裝壞了」
    /// 在 log 裡可以分辨。
    pub fn load_optional(dir: &Path) -> Option<Self> {
        if !dir.is_dir() {
            log::info!(
                "英文資料包不存在（{}），英文使用內建原型詞彙",
                dir.display()
            );
            return None;
        }
        match Self::load_from(dir) {
            Ok(english) => {
                log::info!(
                    "已載入英文詞表：{} 詞（{}）",
                    english.dictionary().len(),
                    dir.display()
                );
                Some(english)
            }
            Err(err) => {
                log::warn!(
                    "英文資料包 {} 載入失敗（{err}），改用內建原型詞彙",
                    dir.display()
                );
                None
            }
        }
    }

    /// 載入系統上的英文詞表；找不到或載入失敗時退回 [`prototype`]。
    ///
    /// 目錄依 [`english_pack_dir`] 的規則解析。找不到目錄只記 info；
    /// 目錄存在但驗證失敗才記警告。
    pub fn load_default() -> Self {
        let Some(dir) = english_pack_dir() else {
            log::info!("找不到英文資料包，英文只使用內建原型詞彙");
            return Self::new(None);
        };
        match Self::from_pack_dir(&dir) {
            Ok(english) => {
                log::info!(
                    "已載入英文詞表：{} 詞（{}）",
                    english.dict.len(),
                    dir.display()
                );
                english
            }
            Err(err) => {
                log::warn!(
                    "英文資料包 {} 載入失敗（{err}），改用內建原型詞彙",
                    dir.display()
                );
                Self::new(None)
            }
        }
    }

    /// 詞表來源檔案；使用內建原型時是 `None`。
    pub fn origin(&self) -> Option<&Path> {
        self.origin.as_deref()
    }

    /// 底層詞表。
    pub fn dictionary(&self) -> &Dictionary {
        &self.dict
    }

    /// 回報 `text` 是否為詞表或學習詞認得的英文詞（不分大小寫）。
    pub fn has(&self, text: &str) -> bool {
        self.learned(text) || self.dict.has(text)
    }

    /// 英文詞的權重：先找學習詞，再找詞表；查無時回傳 `None`。
    pub fn weight(&self, text: &str) -> Option<f64> {
        if let Some(word) = self.find(text) {
            return Some(LEARNED_WEIGHT + f64::from(word.count));
        }
        let lower = text.to_lowercase();
        let words = self.dict.words();
        words
            .binary_search_by(|word| word.text.as_str().cmp(&lower))
            .ok()
            .map(|index| words[index].weight)
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

    /// 只回傳前綴補完，不含拼字建議；成本與 [`English::candidates`] 相同。
    pub fn completion_candidates(&self, prefix: &str, limit: usize) -> Vec<Candidate> {
        self.suggestions(prefix, limit)
            .into_iter()
            .map(|suggestion| {
                let source = if suggestion.learned {
                    CandidateSource::Personal
                } else {
                    CandidateSource::Lexicon
                };
                Candidate::new(
                    suggestion.text.clone(),
                    suggestion.text,
                    cost_of_weight(suggestion.weight),
                    source,
                )
            })
            .collect()
    }

    /// 只回傳拼字建議，不含前綴補完。
    ///
    /// 打對的完整單字、不可推薦的詞與非英文字母都不會有建議；大小寫照查詢
    /// 時的寫法。成本與 [`English::candidates`] 相同，多一層編輯距離懲罰。
    pub fn spelling_candidates(&self, typed: &str, limit: usize) -> Vec<Candidate> {
        if limit == 0 || self.dict.has(typed) {
            return Vec::new();
        }
        self.dict
            .corrections(typed, limit)
            .into_iter()
            .map(|correction| {
                let cost = cost_of_weight(correction.weight)
                    .saturating_add(CORRECTION_PENALTY.saturating_mul(correction.distance as i32));
                Candidate::new(
                    correction.text.clone(),
                    correction.text,
                    cost,
                    CandidateSource::Lexicon,
                )
            })
            .collect()
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

    /// 把補完與拼字建議轉成 [`Candidate`]；原樣輸入的候選由呼叫端自己排在
    /// 最前面。
    ///
    /// 補完候選用完 `limit` 之前，才會補上編輯距離 1（或 2）的拼字建議。
    /// 拼字建議的成本多一層 [`CORRECTION_PENALTY`] 乘以距離，因此一定排在
    /// 完整前綴補完之後。
    pub fn candidates(&self, prefix: &str, limit: usize) -> Vec<Candidate> {
        if prefix.is_empty() || limit == 0 {
            return Vec::new();
        }
        let suggestions = self.suggestions(prefix, limit);
        let mut out: Vec<Candidate> = Vec::with_capacity(suggestions.len());
        let mut seen: Vec<String> = Vec::with_capacity(suggestions.len());
        for s in suggestions {
            let key = s.text.to_lowercase();
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            let source = if s.learned {
                CandidateSource::Personal
            } else {
                CandidateSource::Lexicon
            };
            out.push(Candidate::new(
                s.text.clone(),
                s.text,
                cost_of_weight(s.weight),
                source,
            ));
        }
        let limit = limit.min(MAX_SUGGESTIONS);
        if out.len() < limit
            && prefix.chars().all(|c| c.is_ascii_alphabetic())
            && !self.dict.has(prefix)
        {
            for correction in self.dict.corrections(prefix, limit - out.len()) {
                let key = correction.text.to_lowercase();
                if seen.contains(&key) {
                    continue;
                }
                seen.push(key);
                let cost = cost_of_weight(correction.weight)
                    .saturating_add(CORRECTION_PENALTY.saturating_mul(correction.distance as i32));
                out.push(Candidate::new(
                    correction.text.clone(),
                    correction.text,
                    cost,
                    CandidateSource::Lexicon,
                ));
            }
        }
        out.truncate(limit);
        out
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
        Self::load_default()
    }
}

/// 全行程共用的英文查詢層：優先用系統上的英文資料包，載入不到才退回原型。
///
/// 載入只做一次；之後所有 [`Router`] 與 [`AsciiPiece`] 都共用同一份唯讀詞表。
pub fn default_english() -> &'static English {
    static ENGLISH: OnceLock<English> = OnceLock::new();
    ENGLISH.get_or_init(English::load_default)
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
                None => default_english(),
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

impl AtomKind {
    fn name(self) -> &'static str {
        match self {
            Self::Zhuyin => "zhuyin",
            Self::Kana => "kana",
            Self::Han => "han",
            Self::Word => "word",
            Self::Symbol => "symbol",
        }
    }
}

#[derive(Debug, Clone)]
struct Atom {
    start: usize,
    end: usize,
    script: Script,
    mode: Option<Lang>,
    kind: AtomKind,
    /// 羅馬字已還原的かな；只有整段可完整かな化時才有值。
    kana: Option<String>,
    /// 注音已還原的音節（以空白分隔）；只有切分器產生的片段才有值。
    zh: Option<String>,
    /// 每個注音音節是否已完成：打了聲調鍵，或空白當一聲。
    zh_done: Vec<bool>,
    /// かな照原樣輸出，不送かな漢字変換。
    verbatim: bool,
    /// 全小寫 ASCII 字母（片語合併用）。
    plain_lower: bool,
}

/// 一個輸出片段的規劃。
#[derive(Debug, Clone)]
pub(crate) struct Plan {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) mode: Lang,
    pub(crate) script: Script,
    pub(crate) kana: String,
    pub(crate) kana_ok: bool,
    /// かな照原樣輸出，不送かな漢字変換。
    pub(crate) verbatim: bool,
    /// 切分器已還原的注音音節（以空白分隔）。
    pub(crate) zh: Option<String>,
    /// 每個注音音節是否已完成：打了聲調鍵，或空白當一聲。
    pub(crate) zh_done: Vec<bool>,
}

impl Plan {
    /// 第 `index` 個注音音節是否已完成。
    ///
    /// 沒有完成資訊的片段（直接輸入的注音符號）視為已完成，維持原本行為。
    pub(crate) fn zh_completed(&self, index: usize) -> bool {
        self.zh_done.get(index).copied().unwrap_or(true)
    }
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

    /// 查詢層：指定的英文查詢層，否則用系統詞表（載不到時是內建原型）。
    fn lookup(&self) -> &dyn Lookup {
        match self.english {
            Some(e) => e,
            None => default_english(),
        }
    }

    /// 把輸入切成語言片段；沒有轉換，只判定邊界與語言。
    pub fn segments(&self, input: &str) -> Vec<Segment> {
        self.visible_plans(input)
            .into_iter()
            .map(|p| Segment::new(p.start, p.end, p.mode, p.script))
            .collect()
    }

    /// 回報整段輸入的語言：只有單一語言時就是那個語言，否則
    /// [`Lang::Mixed`]。空輸入視為英文直出。
    pub fn detect(&self, input: &str) -> Lang {
        let segments = self.segments(input);
        let Some(first) = segments.first() else {
            return Lang::En;
        };
        if segments.iter().all(|s| s.mode == first.mode) {
            first.mode
        } else {
            Lang::Mixed
        }
    }

    /// 轉換整段輸入。
    ///
    /// 語言邊界不加分隔符；`segments` 的位移是**輸入**的 UTF-8 位元組位移，
    /// 輸出字面的游標位置另外用 UTF-16 表示。
    pub fn convert(&self, input: &str) -> Result<MixedResult, MixedError> {
        let plans = self.visible_plans(input);
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
        // 注音切分可能把英文前綴判成注音（`hel` → ㄘ ㄍㄠ），補完仍以
        // 拉丁字面為準，否則打到一半的英文詞就不會有候選。
        if last.script != Script::Latin || !text.chars().all(|c| c.is_ascii_alphabetic()) {
            return Vec::new();
        }
        self.english()
            .candidates(text, self.config.normalized().max_candidates)
    }

    /// 取得英文查詢層的參照。
    fn english(&self) -> &English {
        match self.english {
            Some(e) => e,
            None => default_english(),
        }
    }

    fn convert_plan(&self, plan: &Plan, input: &str) -> Result<(String, i32), MixedError> {
        let literal = &input[plan.start..plan.end];
        match plan.mode {
            Lang::Zh => match &self.zh {
                Some(route) => {
                    if let Some(reading) = &plan.zh {
                        let syllables: Vec<String> =
                            reading.split_whitespace().map(str::to_string).collect();
                        if syllables.is_empty() {
                            return Ok((literal.to_string(), 0));
                        }
                        return convert_zh_syllables(plan, &syllables, route);
                    }
                    let syllables = zhuyin_syllables(literal);
                    if syllables.is_empty() {
                        return Ok((literal.to_string(), 0));
                    }
                    let lattice = ZhLattice::build(&syllables, route.lexicon, route.ctx)?;
                    let decoder = Decoder::new(route.lexicon, route.ctx);
                    let (segs, cost) = decoder.best(&lattice, &Locks::new(), false);
                    let conversion = decoder.conversion(&lattice, &segs, Lang::Zh);
                    Ok((conversion.surface, cost))
                }
                None => Ok((literal.to_string(), 0)),
            },
            Lang::Ja => {
                // すでにかな・漢字で書かれた字面與符號沒有羅馬字可解，照原樣輸出。
                if !plan.kana_ok || plan.kana.is_empty() || plan.script == Script::Symbol {
                    return Ok((literal.to_string(), 0));
                }
                // 切分器產生的かな先照原樣輸出，かな漢字変換留給後續候選。
                if plan.verbatim {
                    return Ok((plan.kana.clone(), 0));
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

    /// 輸出用的片段規劃：注音與日文切分片段之間的空白不輸出。
    fn visible_plans(&self, input: &str) -> Vec<Plan> {
        let plans = self.plan(input);
        visible_plans(plans, input)
    }

    fn plan(&self, input: &str) -> Vec<Plan> {
        let cfg = self.config.normalized();
        let lookup: &dyn Lookup = self.lookup();
        let adapter = crate::mixed_latin::LexiconAdapter {
            zh: self.zh.as_ref().map(|route| route.lexicon),
            english: Some(self.english()),
            config: cfg.clone(),
        };
        let split_lex = &adapter as &dyn crate::mixed_latin::SplitLexicon;
        plan_input(input, lookup, &cfg, Some(split_lex))
    }
}

/// 轉換一個注音片段：未完成的音節照原始注音輸出，已完成的音節才轉成字。
fn convert_zh_syllables(
    plan: &Plan,
    syllables: &[String],
    route: &ZhRoute<'_>,
) -> Result<(String, i32), MixedError> {
    let mut surface = String::new();
    let mut cost = 0i32;
    let mut group: Vec<&str> = Vec::new();
    for (index, syllable) in syllables.iter().enumerate() {
        if plan.zh_completed(index) {
            group.push(syllable);
            continue;
        }
        convert_zh_group(&mut group, &mut surface, &mut cost, route)?;
        surface.push_str(syllable);
    }
    convert_zh_group(&mut group, &mut surface, &mut cost, route)?;
    Ok((surface, cost))
}

/// 轉換一整段已完成的注音音節。
fn convert_zh_group(
    group: &mut Vec<&str>,
    surface: &mut String,
    cost: &mut i32,
    route: &ZhRoute<'_>,
) -> Result<(), MixedError> {
    if group.is_empty() {
        return Ok(());
    }
    let reading = group.join(" ");
    let syllables = crate::mixed_latin::toned_syllables(&reading, route.lexicon);
    let lattice = ZhLattice::build(&syllables, route.lexicon, route.ctx)?;
    let decoder = Decoder::new(route.lexicon, route.ctx);
    let (segs, group_cost) = decoder.best(&lattice, &Locks::new(), false);
    let conversion = decoder.conversion(&lattice, &segs, Lang::Zh);
    surface.push_str(&conversion.surface);
    *cost = cost.saturating_add(group_cost);
    group.clear();
    Ok(())
}

/// 唯一的管線：tokenize（R1、R2、R3）→ 每個 run 的 DP（R4）→ 後處理（R5）。
pub(crate) fn plan_input(
    input: &str,
    lookup: &dyn Lookup,
    cfg: &Config,
    split_lex: Option<&dyn crate::mixed_latin::SplitLexicon>,
) -> Vec<Plan> {
    let mut atoms = build_atoms(input, lookup, cfg, split_lex, None);
    group_words(&mut atoms, input);
    resolve_han(&mut atoms);
    resolve_symbols(&mut atoms);
    merge_atoms(&atoms, input)
}

/// `mixed-split --steps` 用的管線快照。
#[derive(Debug, Clone, Default)]
pub struct Explain {
    /// tokenize 後的 atom。
    pub atoms: Vec<AtomView>,
    /// 每個跑過 DP 的拉丁 run。
    pub runs: Vec<RunView>,
    /// 片語合併後的 atom。
    pub after_group_words: Vec<AtomView>,
    /// 語言判定後的 atom。
    pub after_resolve: Vec<AtomView>,
    /// 合併後的輸出片段。
    pub after_merge: Vec<PlanView>,
}

/// 一個 atom 的快照。
#[derive(Debug, Clone)]
pub struct AtomView {
    /// 輸入起點（位元組）。
    pub start: usize,
    /// 輸入終點（位元組）。
    pub end: usize,
    /// 書寫系統。
    pub script: Script,
    /// 語言；還沒判定時是 `None`。
    pub mode: Option<Lang>,
    /// atom 種類名稱。
    pub kind: &'static str,
    /// 輸入字面。
    pub text: String,
}

impl AtomView {
    fn new(atom: &Atom, input: &str) -> Self {
        Self {
            start: atom.start,
            end: atom.end,
            script: atom.script,
            mode: atom.mode,
            kind: atom.kind.name(),
            text: input
                .get(atom.start..atom.end)
                .unwrap_or_default()
                .to_string(),
        }
    }
}

/// 一個拉丁 run 的 DP 結果。
#[derive(Debug, Clone)]
pub struct RunView {
    /// run 的輸入字面。
    pub text: String,
    /// DP 片段。
    pub pieces: Vec<RunPiece>,
}

/// DP 片段的一格。
#[derive(Debug, Clone)]
pub struct RunPiece {
    /// 起點（run 內的位元組位移）。
    pub start: usize,
    /// 終點（run 內的位元組位移）。
    pub end: usize,
    /// 語言名稱。
    pub lang: &'static str,
    /// 輸入字面。
    pub text: String,
    /// 讀音。
    pub reading: String,
}

impl RunView {
    fn new(text: &str, split: &crate::mixed_latin::Split) -> Self {
        Self {
            text: text.to_string(),
            pieces: split
                .pieces
                .iter()
                .map(|piece| RunPiece {
                    start: piece.start,
                    end: piece.end,
                    lang: piece.lang.as_str(),
                    text: piece.text(text).to_string(),
                    reading: piece.reading.clone(),
                })
                .collect(),
        }
    }
}

/// 輸出片段規劃的快照。
#[derive(Debug, Clone)]
pub struct PlanView {
    /// 輸入起點（位元組）。
    pub start: usize,
    /// 輸入終點（位元組）。
    pub end: usize,
    /// 語言。
    pub mode: Lang,
    /// 書寫系統。
    pub script: Script,
    /// 輸入字面。
    pub text: String,
}

/// 跑一次管線並留下三個階段。
pub fn explain(
    input: &str,
    lookup: &dyn Lookup,
    cfg: &Config,
    split_lex: Option<&dyn crate::mixed_latin::SplitLexicon>,
) -> Explain {
    let mut trace = Explain::default();
    let mut atoms = build_atoms(input, lookup, cfg, split_lex, Some(&mut trace));
    trace.atoms = atoms
        .iter()
        .map(|atom| AtomView::new(atom, input))
        .collect();
    group_words(&mut atoms, input);
    trace.after_group_words = atoms
        .iter()
        .map(|atom| AtomView::new(atom, input))
        .collect();
    resolve_han(&mut atoms);
    resolve_symbols(&mut atoms);
    trace.after_resolve = atoms
        .iter()
        .map(|atom| AtomView::new(atom, input))
        .collect();
    trace.after_merge = merge_atoms(&atoms, input)
        .iter()
        .map(|plan| PlanView {
            start: plan.start,
            end: plan.end,
            mode: plan.mode,
            script: plan.script,
            text: input
                .get(plan.start..plan.end)
                .unwrap_or_default()
                .to_string(),
        })
        .collect();
    trace
}

/// 輸出用的片段規劃：注音與日文切分片段之間的空白不輸出。
///
/// 注音音節結尾的一聲空白已經包含在片段內，不會是獨立片段；這裡處理的是
/// 兩個已切分片段之間的停頓空白。
pub(crate) fn visible_plans(plans: Vec<Plan>, input: &str) -> Vec<Plan> {
    let is_split = |plan: &Plan| {
        plan.zh.is_some()
            || (plan.mode == Lang::Ja && plan.script == Script::Latin && !plan.kana.is_empty())
    };
    let mut keep = vec![true; plans.len()];
    for (index, plan) in plans.iter().enumerate() {
        let text = &input[plan.start..plan.end];
        if text.is_empty() || !text.chars().all(char::is_whitespace) {
            continue;
        }
        let previous = index.checked_sub(1).and_then(|i| plans.get(i));
        let next = plans.get(index + 1);
        let previous_split = previous.is_some_and(is_split);
        let next_split = next.is_some_and(is_split);
        let previous_is_zh = previous.is_some_and(|plan| plan.zh.is_some());
        if (previous_split && next_split) || (previous_is_zh && next.is_none()) {
            keep[index] = false;
        }
    }
    plans
        .into_iter()
        .zip(keep)
        .filter_map(|(plan, keep)| keep.then_some(plan))
        .collect()
}

/// 一個轉換後的片段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MixedSegment {
    /// 輸入上的起點（UTF-8 位元組，含）。
    pub input_start: usize,
    /// 輸入上的終點（UTF-8 位元組，不含）。
    pub input_end: usize,
    /// 這一段的語言。
    pub mode: Lang,
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
    pub fn languages(&self) -> Vec<Lang> {
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

fn build_atoms(
    input: &str,
    lookup: &dyn Lookup,
    cfg: &Config,
    split_lex: Option<&dyn crate::mixed_latin::SplitLexicon>,
    mut trace: Option<&mut Explain>,
) -> Vec<Atom> {
    let mut atoms = Vec::new();
    // R4：前一個拉丁 run 的語言，當下一個 run 的 DP 起始狀態。
    let mut last_lang: Option<crate::mixed_latin::SplitLang> = None;
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
                mode: Some(Lang::Zh),
                kind: AtomKind::Zhuyin,
                kana: None,
                zh: None,
                zh_done: Vec::new(),
                verbatim: false,
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
                        Some(Lang::Ja)
                    } else {
                        None
                    },
                    kind,
                    kana: None,
                    zh: None,
                    zh_done: Vec::new(),
                    verbatim: false,
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
            i = push_token_run(
                &mut atoms,
                &input[start..i],
                start,
                &input[i..],
                lookup,
                cfg,
                split_lex,
                &mut last_lang,
                trace.as_deref_mut(),
            );
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
            zh: None,
            zh_done: Vec::new(),
            verbatim: false,
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

/// 處理一個英數 run，回傳下一個要處理的輸入位移。
///
/// R2 成立時會多吃一個空白，讓空白成為最後一個音節的一聲。
#[allow(clippy::too_many_arguments)]
fn push_token_run(
    atoms: &mut Vec<Atom>,
    run: &str,
    offset: usize,
    next: &str,
    lookup: &dyn Lookup,
    cfg: &Config,
    split_lex: Option<&dyn crate::mixed_latin::SplitLexicon>,
    last_lang: &mut Option<crate::mixed_latin::SplitLang>,
    trace: Option<&mut Explain>,
) -> usize {
    if run.is_empty() {
        return offset;
    }
    if next.starts_with(' ') {
        let text = format!("{run} ");
        let lexicon: &dyn crate::mixed_latin::SplitLexicon =
            split_lex.unwrap_or(&crate::mixed_latin::NoLexicon);
        if crate::mixed_latin::space_is_first_tone(
            &text,
            &split_options(*last_lang, &text),
            lexicon,
        ) {
            push_run_text(
                atoms, &text, offset, lookup, cfg, split_lex, last_lang, trace,
            );
            return offset + text.len();
        }
    }
    push_run_text(atoms, run, offset, lookup, cfg, split_lex, last_lang, trace);
    offset + run.len()
}

/// 切分一段拉丁 run：先整段 DP，失敗才依 R3 把標點拆成符號。
#[allow(clippy::too_many_arguments)]
fn push_run_text(
    atoms: &mut Vec<Atom>,
    text: &str,
    offset: usize,
    lookup: &dyn Lookup,
    cfg: &Config,
    split_lex: Option<&dyn crate::mixed_latin::SplitLexicon>,
    last_lang: &mut Option<crate::mixed_latin::SplitLang>,
    mut trace: Option<&mut Explain>,
) {
    let opts = split_options(*last_lang, text);
    let lexicon: &dyn crate::mixed_latin::SplitLexicon =
        split_lex.unwrap_or(&crate::mixed_latin::NoLexicon);
    if is_latin_split_input(text) {
        if let Some(split) = crate::mixed_latin::split(text, &opts, lexicon) {
            if let Some(trace) = trace.as_deref_mut() {
                trace.runs.push(RunView::new(text, &split));
            }
            let last = split.last_lang();
            push_split_pieces(atoms, text, split.pieces, offset);
            *last_lang = last;
            return;
        }
    }
    // 整段是網址、識別字或「かな前綴 + 英數後綴」時，先維持原本的字面路徑，
    // 不要被 run 內的 `:` `/` `?` 拆開。
    if push_literal_piece(atoms, text, offset, lookup, cfg, last_lang) {
        return;
    }
    if let Some(segments) = split_at_symbol_punct(text) {
        for segment in segments {
            match segment {
                RunSegment::Text(start, end) => {
                    push_latin_segment(
                        atoms,
                        &text[start..end],
                        offset + start,
                        lookup,
                        cfg,
                        split_lex,
                        last_lang,
                        trace.as_deref_mut(),
                    );
                }
                RunSegment::Symbol(start, end) => {
                    atoms.push(symbol_atom(offset + start, offset + end));
                }
            }
        }
        return;
    }
    atoms.push(latin_literal_atom(offset, offset + text.len(), text));
    *last_lang = Some(crate::mixed_latin::SplitLang::En);
}

fn split_options(
    initial: Option<crate::mixed_latin::SplitLang>,
    text: &str,
) -> crate::mixed_latin::Options {
    crate::mixed_latin::Options {
        initial_lang: initial,
        // 尾巴後面還有空白時，最後一個字母通常是下一聲音節的一聲。
        single_letter_en_tail: !text.ends_with(' '),
        ..crate::mixed_latin::Options::default()
    }
}

fn symbol_atom(start: usize, end: usize) -> Atom {
    Atom {
        start,
        end,
        script: Script::Symbol,
        mode: None,
        kind: AtomKind::Symbol,
        kana: None,
        zh: None,
        zh_done: Vec::new(),
        verbatim: false,
        plain_lower: false,
    }
}

/// run 內依 R3 切開的一段。
enum RunSegment {
    /// 交給 DP 或字面路徑的文字。
    Text(usize, usize),
    /// 不當韻母的標點。
    Symbol(usize, usize),
}

/// 把 run 切成拉丁文字與標點符號。
///
/// 只有在真的有標點要拆時回傳 `Some`。`.` `,` `;` `/` `-` 接在缺韻母的
/// 合法音節後面時是韻母，留在文字段內；其他位置一律是符號 atom。
fn split_at_symbol_punct(text: &str) -> Option<Vec<RunSegment>> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut found = false;
    for (i, ch) in text.char_indices() {
        if !SYMBOL_PUNCT.contains(ch) || punct_is_final(text, i) {
            continue;
        }
        if start < i {
            out.push(RunSegment::Text(start, i));
        }
        out.push(RunSegment::Symbol(i, i + ch.len_utf8()));
        start = i + ch.len_utf8();
        found = true;
    }
    if !found {
        return None;
    }
    if start < text.len() {
        out.push(RunSegment::Text(start, text.len()));
    }
    Some(out)
}

/// 回報位置 `at` 的標點是不是前一個注音音節的韻母。
fn punct_is_final(text: &str, at: usize) -> bool {
    let Some(ch) = text[at..].chars().next() else {
        return false;
    };
    if !crate::mixed_latin::is_final_punct(ch) {
        return false;
    }
    let Some(symbol) = Keyboard::standard().symbol(ch) else {
        return false;
    };
    let chars: Vec<char> = text[..at].chars().collect();
    let low = chars.len().saturating_sub(8);
    for start in low..chars.len() {
        let keys: String = chars[start..].iter().collect();
        let Some(base) = zh_key_base(&keys) else {
            continue;
        };
        // 只有缺韻母的音節才需要這個標點。
        if base.chars().last().map(class_of) == Some(Class::Final) {
            continue;
        }
        if is_legal_base(&format!("{base}{symbol}")) {
            return true;
        }
    }
    false
}

/// 把注音按鍵解析成不含聲調的韻母本體。
///
/// 聲調鍵只能出現在結尾；出現非注音鍵或類別順序錯誤時回傳 `None`。
fn zh_key_base(keys: &str) -> Option<String> {
    let keyboard = Keyboard::standard();
    let mut base = String::new();
    let mut last = Class::None;
    let mut chars = keys.chars().peekable();
    while let Some(key) = chars.next() {
        let symbol = keyboard.symbol(key)?;
        let class = class_of(symbol);
        match class {
            Class::Tone => {
                if chars.peek().is_some() {
                    return None;
                }
                break;
            }
            Class::None => return None,
            _ if class <= last => return None,
            _ => {
                base.push(symbol);
                last = class;
            }
        }
    }
    (!base.is_empty()).then_some(base)
}

/// 把 DP 的片段轉成 atom；`text` 是片段所在的原始輸入。
fn push_split_pieces(
    atoms: &mut Vec<Atom>,
    text: &str,
    pieces: Vec<crate::mixed_latin::Piece>,
    offset: usize,
) {
    for piece in pieces {
        let lang = piece.lang;
        atoms.push(Atom {
            start: offset + piece.start,
            end: offset + piece.end,
            script: Script::Latin,
            mode: Some(lang.mode()),
            kind: AtomKind::Word,
            kana: (lang == crate::mixed_latin::SplitLang::Ja).then(|| piece.reading.clone()),
            zh: (lang == crate::mixed_latin::SplitLang::Zh).then(|| piece.reading.clone()),
            zh_done: if lang == crate::mixed_latin::SplitLang::Zh {
                zh_piece_completion(piece.text(text), &piece.reading)
            } else {
                Vec::new()
            },
            verbatim: false,
            plain_lower: lang == crate::mixed_latin::SplitLang::En
                || is_plain_lower(piece.text(text)),
        });
    }
}

/// 一個注音片段內每個音節是否已完成：結尾是聲調鍵，或空白當一聲。
///
/// DP 會把相鄰音節併成一個片段，這裡照大千鍵盤的類別順序切回音節。類別回到
/// 開頭時開始下一個音節；回傳長度與讀音音節數對不上時，交給呼叫端保留原行為。
fn zh_piece_completion(text: &str, reading: &str) -> Vec<bool> {
    let keyboard = Keyboard::standard();
    let mut out = Vec::new();
    let mut last = Class::None;
    let mut start = 0usize;
    for (i, ch) in text.char_indices() {
        if ch.is_whitespace() {
            if start < i {
                out.push(true);
            }
            start = i + ch.len_utf8();
            last = Class::None;
            continue;
        }
        let Some(symbol) = keyboard.symbol(ch) else {
            return vec![true; reading.split_whitespace().count()];
        };
        let class = class_of(symbol);
        if class == Class::Tone {
            if start < i {
                out.push(true);
            }
            start = i + ch.len_utf8();
            last = Class::None;
            continue;
        }
        if class == Class::None {
            return vec![true; reading.split_whitespace().count()];
        }
        if class <= last && start < i {
            out.push(false);
            start = i;
        }
        last = class;
    }
    if start < text.len() {
        out.push(false);
    }
    if out.len() == reading.split_whitespace().count() {
        out
    } else {
        vec![true; reading.split_whitespace().count()]
    }
}

/// 把一段拉丁字串切成字面 atom；DP 不適用或解不出來時走這裡。
#[allow(clippy::too_many_arguments)]
fn push_latin_segment(
    atoms: &mut Vec<Atom>,
    core: &str,
    offset: usize,
    lookup: &dyn Lookup,
    cfg: &Config,
    split_lex: Option<&dyn crate::mixed_latin::SplitLexicon>,
    last_lang: &mut Option<crate::mixed_latin::SplitLang>,
    trace: Option<&mut Explain>,
) {
    let opts = split_options(*last_lang, core);
    let lexicon: &dyn crate::mixed_latin::SplitLexicon =
        split_lex.unwrap_or(&crate::mixed_latin::NoLexicon);
    if is_latin_split_input(core) {
        if let Some(split) = crate::mixed_latin::split(core, &opts, lexicon) {
            if !split.is_empty() {
                if let Some(trace) = trace {
                    trace.runs.push(RunView::new(core, &split));
                }
                let last = split.last_lang();
                push_split_pieces(atoms, core, split.pieces, offset);
                *last_lang = last;
                return;
            }
        }
    }
    if push_literal_piece(atoms, core, offset, lookup, cfg, last_lang) {
        return;
    }
    atoms.push(latin_literal_atom(offset, offset + core.len(), core));
    *last_lang = Some(crate::mixed_latin::SplitLang::En);
}

/// 整段照字面的日文或「かな前綴 + 英數後綴」；不適用時回傳 `false`。
fn push_literal_piece(
    atoms: &mut Vec<Atom>,
    core: &str,
    offset: usize,
    lookup: &dyn Lookup,
    cfg: &Config,
    last_lang: &mut Option<crate::mixed_latin::SplitLang>,
) -> bool {
    if let Some((kana, plain_lower)) = latin_piece(core, lookup, cfg) {
        atoms.push(Atom {
            start: offset,
            end: offset + core.len(),
            script: Script::Latin,
            mode: Some(Lang::Ja),
            kind: AtomKind::Word,
            kana: Some(kana),
            zh: None,
            zh_done: Vec::new(),
            verbatim: false,
            plain_lower,
        });
        *last_lang = Some(crate::mixed_latin::SplitLang::Ja);
        return true;
    }
    if let Some((at, kana)) = split_kana_prefix(core, lookup, cfg) {
        let prefix = &core[..at];
        let suffix = &core[at..];
        atoms.push(Atom {
            start: offset,
            end: offset + prefix.len(),
            script: Script::Latin,
            mode: Some(Lang::Ja),
            kind: AtomKind::Word,
            kana: Some(kana),
            zh: None,
            zh_done: Vec::new(),
            verbatim: false,
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
            mode: Some(Lang::En),
            kind: AtomKind::Word,
            kana: None,
            zh: None,
            zh_done: Vec::new(),
            verbatim: false,
            plain_lower: false,
        });
        *last_lang = Some(crate::mixed_latin::SplitLang::En);
        return true;
    }
    false
}

/// 一段照字面的英數 atom。
fn latin_literal_atom(start: usize, end: usize, text: &str) -> Atom {
    Atom {
        start,
        end,
        script: if has_fullwidth_alnum(text) {
            Script::Mixed
        } else if text.chars().all(|c| c.is_ascii_digit()) {
            Script::Digit
        } else {
            Script::Latin
        },
        mode: Some(Lang::En),
        kind: AtomKind::Word,
        kana: None,
        zh: None,
        zh_done: Vec::new(),
        verbatim: false,
        plain_lower: is_plain_lower(text),
    }
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

/// 英文詞判定：A（切分器的
/// [`crate::mixed_latin::SplitLexicon`]）與 B（路由）共用這一個函式。
pub fn is_english_word(text: &str, lookup: &dyn Lookup, cfg: &Config) -> bool {
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
        while j + 2 < atoms.len()
            && is_single_space(atoms.get(j + 1), input)
            && is_plain_word(atoms, j + 2)
        {
            words.push(j + 2);
            j += 2;
        }
        if words.len() >= 2 {
            // 片語一起判定：單一かな單位的短字不當羅馬字；而且一旦片語裡
            // 出現明確的英文詞，後面的字都屬於同一個英文片語。這樣
            // `desu is a good sentence` 的 `is a good sentence` 不會有個別
            // 字（`sentence`）被還原成かな後送進日文解碼。
            let mut english = false;
            for &w in &words {
                let mode = atoms[w].mode;
                if mode == Some(Lang::En) {
                    english = true;
                } else if english || (mode == Some(Lang::Ja) && kana_units(&atoms[w]) < 2) {
                    atoms[w].mode = Some(Lang::En);
                    atoms[w].kana = None;
                    english = true;
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
    let modes: Vec<Option<Lang>> = atoms.iter().map(|a| a.mode).collect();
    for i in 0..atoms.len() {
        if atoms[i].kind != AtomKind::Han || atoms[i].mode.is_some() {
            continue;
        }
        let previous = modes[..i].iter().rev().find_map(|m| *m);
        let next = modes[i + 1..].iter().find_map(|m| *m);
        let mode = match (previous, next) {
            (Some(Lang::Ja), _) | (_, Some(Lang::Ja)) => Lang::Ja,
            (Some(m), _) => m,
            (None, Some(m)) => m,
            (None, None) => Lang::Zh,
        };
        atoms[i].mode = Some(mode);
    }
}

fn resolve_symbols(atoms: &mut [Atom]) {
    let modes: Vec<Option<Lang>> = atoms.iter().map(|a| a.mode).collect();
    for i in 0..atoms.len() {
        if atoms[i].kind != AtomKind::Symbol || atoms[i].mode.is_some() {
            continue;
        }
        let previous = modes[..i].iter().rev().find_map(|m| *m);
        let next = modes[i + 1..].iter().find_map(|m| *m);
        let mode = previous.or(next).unwrap_or(Lang::En);
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
        let mode = atom.mode.unwrap_or(Lang::En);
        if let Some(last) = plans.last_mut() {
            if last.mode == mode && script_family(last.script) == script_family(atom.script) {
                last.end = atom.end;
                last.verbatim = last.verbatim && atom.verbatim;
                match atom.kind {
                    AtomKind::Kana => last.kana.push_str(&input[atom.start..atom.end]),
                    AtomKind::Word if mode == Lang::Ja => match &atom.kana {
                        Some(kana) => last.kana.push_str(kana),
                        None => last.kana_ok = false,
                    },
                    AtomKind::Word if mode == Lang::Zh => {
                        if let Some(reading) = &atom.zh {
                            match &mut last.zh {
                                Some(zh) => {
                                    zh.push(' ');
                                    zh.push_str(reading);
                                }
                                None => last.zh = Some(reading.clone()),
                            }
                            last.zh_done.extend(atom.zh_done.iter().copied());
                        }
                    }
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
            verbatim: atom.verbatim,
            zh: None,
            zh_done: Vec::new(),
        };
        match atom.kind {
            AtomKind::Kana => plan.kana = input[atom.start..atom.end].to_string(),
            AtomKind::Word if mode == Lang::Ja => match &atom.kana {
                Some(kana) => plan.kana = kana.clone(),
                None => plan.kana_ok = false,
            },
            AtomKind::Word if mode == Lang::Zh => {
                if atom.zh.is_some() {
                    plan.zh = atom.zh.clone();
                    plan.zh_done.extend(atom.zh_done.iter().copied());
                }
            }
            AtomKind::Word | AtomKind::Zhuyin | AtomKind::Symbol => {}
            AtomKind::Han => plan.kana_ok = false,
        }
        plans.push(plan);
    }
    plans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ja::decode::JaWordSource;
    use crate::ja::lexicon::{JaEntry, Split};
    use crate::lattice::{InMemoryLexicon, LexEntry};
    use std::sync::Arc;

    fn english() -> English {
        English::new(Some(prototype().clone()))
    }

    fn mode_of(name: &str) -> Lang {
        match name {
            "zh" => Lang::Zh,
            "ja" => Lang::Ja,
            _ => Lang::En,
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

    #[test]
    fn test_codec_roundtrip_and_malformed_input() {
        let d = Dictionary::new_filtered(
            [Word::new("example", 5.0), Word::new("excluded", 10.0)],
            ["excluded".to_string()],
        );
        let bytes = d.encode();
        let back = Dictionary::decode(&bytes).expect("解碼");
        assert_eq!(bytes, back.encode());
        assert!(back.has("excluded"));
        assert!(back.prefix("excl", 2).is_empty());
        for i in 0..bytes.len() {
            assert!(Dictionary::decode(&bytes[..i]).is_err(), "接受了截斷 {i}");
        }
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(Dictionary::decode(&extra).is_err(), "接受了多餘內容");
        let mut bad = bytes.clone();
        bad[5] = 255;
        bad[6] = 255;
        bad[7] = 255;
        assert!(Dictionary::decode(&bad).is_err(), "接受了過大的詞數");
    }

    #[test]
    fn test_typo_channels_and_valid_word_protection() {
        let d = prototype();
        for (typed, want) in [
            ("recieve", "receive"),
            ("hellp", "hello"),
            ("helo", "hello"),
            ("helllo", "hello"),
            ("teh", "the"),
            ("RECIEVE", "RECEIVE"),
        ] {
            let got = d.corrections(typed, 1);
            assert_eq!(got.len(), 1, "{typed}: {got:?}");
            assert_eq!(got[0].text, want, "{typed}");
        }
        for text in ["from", "the", "api_v2", "https://example.com"] {
            assert!(d.corrections(text, 3).is_empty(), "不該修正 {text}");
        }
    }

    #[test]
    fn test_common_neighbour_is_not_hidden_by_candidate_limit() {
        let mut words = Vec::new();
        for at in 0..6 {
            for c in b'a'..=b'z' {
                if c == b'x' {
                    continue;
                }
                let mut text = *b"xxxxxx";
                text[at] = c;
                words.push(Word::new(
                    String::from_utf8(text.to_vec()).expect("ASCII"),
                    40.0,
                ));
            }
        }
        words.push(Word::new("xxeexx", 100.0));
        let d = Dictionary::new(words);
        assert!(
            !d.corrections("xxxxxx", MAX_SUGGESTIONS)
                .iter()
                .any(|c| c.text == "xxeexx"),
            "距離 2 的常見詞不該擠進前 32 名"
        );
        assert!(d.near_common("xxxxxx", 50.0));
        assert!(!d.near_common("xxxxxx", 101.0));
        assert!(!d.near_common("xxeexx", 50.0));
    }

    #[test]
    fn test_prefix_covers_entire_dictionary() {
        let mut words = Vec::new();
        for i in 0..6000u32 {
            let mut n = i;
            let mut suffix = String::new();
            for _ in 0..4 {
                suffix.push(char::from(b'a' + (n % 26) as u8));
                n /= 26;
            }
            words.push(Word::new(format!("a{suffix}"), 1.0));
        }
        words.push(Word::new("azzzz", 100.0));
        words.push(Word::new("zebra", 90.0));
        let d = Dictionary::new(words);
        let got = d.prefix("a", 1);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].text, "azzzz");
        let got = d.corrections("zebar", 1);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].text, "zebra");
    }

    #[test]
    fn test_no_suggest_words_recognized_and_unicode_identifiers_untouched() {
        let d = Dictionary::new_filtered(
            [
                Word::new("example", 9.0),
                Word::new("examples", 3.0),
                Word::new("excluded", 100.0),
            ],
            ["excluded".to_string()],
        );
        assert!(d.has("excluded"));
        assert!(d.prefix("excl", 3).is_empty());
        assert!(d.corrections("excludde", 3).is_empty());
        let long = "x".repeat(1000);
        for text in ["\u{212a}", "ſearch", "api_v2", "foo123", long.as_str()] {
            assert!(!d.has(text), "has({text:?})");
            assert!(d.prefix(text, 5).is_empty(), "prefix({text:?})");
            assert!(d.corrections(text, 5).is_empty(), "corrections({text:?})");
        }
    }

    #[test]
    fn test_correction_matches_exhaustive_distance() {
        fn generate(prefix: &str, depth: usize, out: &mut Vec<String>) {
            if !prefix.is_empty() {
                out.push(prefix.to_string());
            }
            if depth > 0 {
                for c in ['a', 'b', 'c'] {
                    generate(&format!("{prefix}{c}"), depth - 1, out);
                }
            }
        }
        let mut texts = Vec::new();
        generate("", 4, &mut texts);
        let words: Vec<Word> = texts
            .iter()
            .enumerate()
            .map(|(i, text)| Word::new(text.clone(), (i + 1) as f64))
            .collect();
        let d = Dictionary::new(words);
        for text in &texts {
            for typed in [
                format!("{text}x"),
                format!("x{text}"),
                format!("{text}xx"),
                text.replace('a', "x"),
            ] {
                if typed.len() < 3 || d.has(&typed) {
                    continue;
                }
                let max_edits = if typed.len() >= 6 { 2 } else { 1 };
                let lower = typed.to_lowercase();
                let mut want: Vec<Correction> = d
                    .words()
                    .iter()
                    .filter_map(|candidate| {
                        let distance = osa_distance(&lower, &candidate.text);
                        (distance <= max_edits).then(|| Correction {
                            text: candidate.text.clone(),
                            distance,
                            weight: candidate.weight,
                        })
                    })
                    .collect();
                want.sort_by(|a, b| {
                    a.distance
                        .cmp(&b.distance)
                        .then_with(|| b.weight.total_cmp(&a.weight))
                        .then_with(|| a.text.cmp(&b.text))
                });
                want.truncate(5);
                assert_eq!(d.corrections(&typed, 5), want, "{typed}");
            }
        }
    }

    /// 測試用的最優字串對齊距離；不提前放棄，當作獨立對照。
    fn osa_distance(left: &str, right: &str) -> usize {
        let a = left.as_bytes();
        let b = right.as_bytes();
        let mut rows = vec![vec![0usize; b.len() + 1]; a.len() + 1];
        for (j, cell) in rows[0].iter_mut().enumerate() {
            *cell = j;
        }
        for i in 1..=a.len() {
            rows[i][0] = i;
            for j in 1..=b.len() {
                let cost = usize::from(a[i - 1] != b[j - 1]);
                let mut best = (rows[i][j - 1] + 1)
                    .min(rows[i - 1][j] + 1)
                    .min(rows[i - 1][j - 1] + cost);
                if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                    best = best.min(rows[i - 2][j - 2] + 1);
                }
                rows[i][j] = best;
            }
        }
        rows[a.len()][b.len()]
    }

    #[test]
    fn test_english_candidates_append_corrections() {
        let e = english();
        let cands = e.candidates("recieve", 8);
        assert_eq!(cands.first().map(|c| c.surface.as_str()), Some("receive"));
        assert_eq!(cands[0].source, CandidateSource::Lexicon);
        let exact = e.candidates("hello", 8);
        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0].surface, "hello");
    }

    #[test]
    fn test_data_root_from_handles_testdata_dir() {
        let root = std::env::temp_dir().join(format!("jc-mixed-root-{}", std::process::id()));
        let nested = root.join("0c-testdata");
        std::fs::create_dir_all(&nested).expect("建立測試資料夾");
        assert_eq!(data_root_from(&nested), Some(root.clone()));
        assert_eq!(data_root_from(&root), Some(root.clone()));
        assert_eq!(data_root_from(&root.join("missing")), None);
        std::fs::remove_dir_all(&root).ok();
    }

    /// 在暫存目錄寫一份最小的英文資料包，回傳目錄路徑。
    fn write_minimal_english_pack(words: &[&str]) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir =
            std::env::temp_dir().join(format!("jc-english-pack-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建立英文資料包目錄");

        let dict = Dictionary::new(
            words
                .iter()
                .enumerate()
                .map(|(i, word)| Word::new(*word, 10_000.0 - i as f64)),
        );
        let words_bin = dict.encode();
        let notice = b"minimal english pack for tests\n";
        std::fs::write(dir.join("words.bin"), &words_bin).expect("寫入 words.bin");
        std::fs::write(dir.join("NOTICE.txt"), notice).expect("寫入 NOTICE.txt");

        let manifest = EnglishManifest {
            format: 1,
            words: dict.len(),
            ranking: "test fixture".to_string(),
            sources: vec![EnglishSource {
                name: "test".to_string(),
                version: "1".to_string(),
                url: "https://example.invalid/test".to_string(),
                sha256: sha256_hex(b"test source"),
                license: "CC0-1.0".to_string(),
                attribution: "test fixture".to_string(),
                commit: None,
            }],
            inputs: BTreeMap::new(),
            files: BTreeMap::from([
                (
                    "words.bin".to_string(),
                    EnglishPackFile {
                        sha256: sha256_hex(&words_bin),
                        size: words_bin.len() as u64,
                    },
                ),
                (
                    "NOTICE.txt".to_string(),
                    EnglishPackFile {
                        sha256: sha256_hex(notice),
                        size: notice.len() as u64,
                    },
                ),
            ]),
        };
        let json = serde_json::to_vec_pretty(&manifest).expect("序列化 manifest");
        std::fs::write(dir.join("manifest.json"), json).expect("寫入 manifest.json");
        dir
    }

    #[test]
    fn test_load_from_minimal_pack_routes_hello() {
        let dir = write_minimal_english_pack(&["hello", "configuration", "small"]);
        let english = English::load_from(&dir).expect("載入最小英文資料包");
        assert_eq!(english.dictionary().len(), 3);
        assert!(english.has("hello"));
        assert!(english.has("configuration"), "詞表以外的原型詞不算");

        let router = Router::new().with_english(&english);
        let segments = router.segments("hello");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].mode, Lang::En);
        assert_eq!(router.convert("hello").expect("轉換").surface, "hello");
        // `configuration` 不在內建原型裡，只有載入資料包才會被當英文詞。
        assert_eq!(
            router.convert("configuration").expect("轉換").surface,
            "configuration"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_optional_missing_dir_returns_none() {
        let dir =
            std::env::temp_dir().join(format!("jc-english-pack-missing-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        assert!(English::load_optional(&dir).is_none());
    }

    #[test]
    fn test_real_english_pack_loads() {
        let words = jc_testdata::require_data!("english/words.bin");
        let dir = words.parent().expect("資料包目錄");
        let (dict, manifest) = Dictionary::load_pack(dir).expect("載入英文資料包");
        assert_eq!(manifest.format, 1);
        assert_eq!(dict.len(), manifest.words);
        assert!(dict.len() > 80_000, "詞數 {}", dict.len());
        assert!(dict.has("configuration"));
        assert_eq!(dict.corrections("recieve", 1)[0].text, "receive");
        let prefix = dict.prefix("confi", 1);
        assert_eq!(prefix.len(), 1);
        assert!(prefix[0].text.starts_with("confi"));
    }

    /// 測試用的日文詞條來源。
    struct MemorySource {
        entries: Vec<JaEntry<'static>>,
    }

    impl MemorySource {
        fn new(rows: &[(&str, &str, i32, &str)]) -> Self {
            Self {
                entries: rows
                    .iter()
                    .map(|&(surface, reading, cost, pos)| JaEntry {
                        surface: surface.to_string().into(),
                        reading: reading.to_string().into(),
                        pos: pos.to_string().into(),
                        conj_type: "*".into(),
                        conj_form: "*".into(),
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
                        entry: entry.clone(),
                        left_id: None,
                        right_id: None,
                    });
                }
            }
        }

        fn exact<'a>(&'a self, reading: &[char]) -> Vec<JaEntry<'a>> {
            let key: String = reading.iter().collect();
            self.entries
                .iter()
                .filter(|e| e.reading.as_ref() == key)
                .cloned()
                .collect()
        }

        fn prefixed<'a>(&'a self, reading: &[char]) -> Vec<JaEntry<'a>> {
            let key: String = reading.iter().collect();
            self.entries
                .iter()
                .filter(|e| e.reading.starts_with(&key))
                .cloned()
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
        assert_eq!(r.detect("ㄓㄨˋ"), Lang::Zh);
        assert_eq!(r.detect("かな"), Lang::Ja);
        assert_eq!(r.detect("hello"), Lang::En);
        assert_eq!(r.detect("watashiwaㄉㄚˋdesu"), Lang::Mixed);
        assert_eq!(r.detect(""), Lang::En);
    }

    #[test]
    fn test_watashi_routes_to_japanese() {
        let english = English::new(None);
        let r = Router::new().with_english(&english);
        assert_eq!(r.detect("watashi"), Lang::Ja);
        let segs = r.segments("watashi");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].mode, Lang::Ja);
        assert_eq!(segs[0].script, Script::Latin);
        assert_eq!(r.convert("watashi").expect("轉換").surface, "わたし");

        let ja = JaDecoder::new(Arc::new(MemorySource::new(&[(
            "私",
            "わたし",
            100,
            "名詞,普通名詞,一般,*",
        )])));
        let r = Router::new().with_english(&english).with_ja(&ja);
        assert_eq!(r.convert("watashi").expect("轉換").surface, "私");
    }

    #[test]
    fn test_segments_use_utf8_offsets_and_modes() {
        let r = Router::new();
        let input = "watashiwaㄉㄚˋㄐㄧdesu";
        let segs = r.segments(input);
        assert_eq!(segs.len(), 3);
        assert_eq!(&input[segs[0].start..segs[0].end], "watashiwa");
        assert_eq!(segs[0].mode, Lang::Ja);
        assert_eq!(segs[0].script, Script::Latin);
        assert_eq!(&input[segs[1].start..segs[1].end], "ㄉㄚˋㄐㄧ");
        assert_eq!(segs[1].mode, Lang::Zh);
        assert_eq!(segs[1].script, Script::Bopomofo);
        assert_eq!(&input[segs[2].start..segs[2].end], "desu");
        assert_eq!(segs[2].mode, Lang::Ja);

        // 位元組位移：注音聲母、介音、韻母是 3 bytes，聲調符號 `ˋ` 是 2 bytes，
        // 所以「ㄉㄚˋㄐㄧ」共 14 bytes，接在 9 bytes 後面。
        assert_eq!(segs[1].start, 9);
        assert_eq!(segs[1].end, 9 + "ㄉㄚˋㄐㄧ".len());
        assert_eq!(segs[2].start, segs[1].end);
    }

    #[test]
    fn test_literal_runs_are_not_converted() {
        let e = english();
        let ja = ja_decoder();
        let r = Router::new().with_english(&e).with_ja(&ja);
        let result = r.convert("kyouhaAPI").expect("轉換");
        assert_eq!(result.surface, "今日はAPI");
        assert_eq!(result.languages(), vec![Lang::Ja, Lang::En]);
        assert_eq!(result.segments[1].surface, "API");

        let url = "https://example.com/a?b=1#x";
        let result = r.convert(&format!("kyouha{url}")).expect("轉換");
        assert_eq!(result.surface, format!("今日は{url}"));
        assert_eq!(result.languages(), vec![Lang::Ja, Lang::En]);
    }

    #[test]
    fn test_route_email_and_url_are_literal() {
        let e = english();
        let ja = ja_decoder();
        let r = Router::new().with_english(&e).with_ja(&ja);
        let result = r.convert("kyouhafoo@example.com").expect("轉換");
        assert_eq!(result.surface, "今日はfoo@example.com");
        assert_eq!(result.languages(), vec![Lang::Ja, Lang::En]);
    }

    #[test]
    fn test_route_zh_segment() {
        let (lex, ctx) = zh_fixture();
        let r = Router::new().with_zh(&lex, &ctx);
        let result = r.convert("ㄉㄚˋㄐㄧㄅㄚ").expect("轉換");
        assert_eq!(result.surface, "大雞巴");
        assert_eq!(result.languages(), vec![Lang::Zh]);
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
        assert_eq!(conversion.segments[0].mode, Lang::Zh);
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
        assert_eq!(segs[0].mode, Lang::Ja);
        assert_eq!(segs[1].mode, Lang::En);

        // 打不成合法羅馬字的英文字不拆。
        let segs = r.segments("kubectl");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].mode, Lang::En);
    }

    #[test]
    fn test_golden_mixed_cases() {
        let path = jc_testdata::require_data!("0c-testdata/golden/mixed/cases.json");
        let text = std::fs::read_to_string(&path).expect("讀取 golden");
        let doc: serde_json::Value = serde_json::from_str(&text).expect("解析 golden");
        let cases = doc["cases"].as_array().expect("cases");
        assert!(!cases.is_empty());
        // golden 的英文句子來自完整詞表；測試只需要能認得這些詞。
        let dict = Dictionary::new(
            ["is", "a", "good", "sentence", "kubectl", "pcloud", "github"]
                .iter()
                .map(|word| Word::new(*word, 1.0e9)),
        );
        let english = English::new(Some(dict));
        let router = Router::new().with_english(&english).with_config(Config {
            min_english_len: 1,
            ..Config::default()
        });
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
        assert!(result.languages().iter().all(|&m| m == Lang::En));
    }

    #[test]
    fn test_phrase_grouping_keeps_english_run() {
        let e = english();
        let r = Router::new().with_english(&e);
        let segments = r.segments("desu is a good sentence");
        let texts: Vec<&str> = segments
            .iter()
            .map(|segment| &"desu is a good sentence"[segment.start..segment.end])
            .collect();
        assert_eq!(texts, vec!["desu", " ", "is a good sentence"]);
        assert_eq!(segments[0].mode, Lang::Ja);
        assert_eq!(segments[2].mode, Lang::En);
    }

    #[test]
    fn test_symbol_between_japanese_and_english_stays_literal() {
        let ja = ja_decoder();
        let e = english();
        let r = Router::new().with_ja(&ja).with_english(&e);
        let result = r.convert("です is a good sentence").expect("轉換");
        let surfaces: Vec<&str> = result
            .segments
            .iter()
            .map(|segment| segment.surface.as_str())
            .collect();
        assert_eq!(surfaces, vec!["です", " ", "is a good sentence"]);
        assert_eq!(result.surface, "です is a good sentence");
    }

    #[test]
    fn test_mixed_latin_split_keeps_kanji_japanese() {
        let lex = InMemoryLexicon::new();
        let ctx = ScoreContext::default();
        // `kanji` 也是英文詞表裡的借詞；切分器要比英文更偏好完整羅馬字。
        let dictionary = Dictionary::new(vec![Word::new("kanji", 1.0)]);
        let e = English::new(Some(dictionary));
        let r = Router::new().with_zh(&lex, &ctx).with_english(&e);
        let result = r.convert("kanji").expect("轉換");
        assert_eq!(result.surface, "かんじ");
        assert_eq!(result.languages(), vec![Lang::Ja]);
    }

    #[test]
    fn test_mixed_latin_split_english_tail() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new(
            "供",
            "ㄍㄨㄥ",
            0,
            crate::lattice::UNKNOWN_POS,
        ));
        let ctx = ScoreContext::default();
        let dictionary = Dictionary::new(vec![Word::new("small", 1.0)]);
        let e = English::new(Some(dictionary));
        let r = Router::new().with_zh(&lex, &ctx).with_english(&e);
        let result = r.convert("rishireej/ sasmall").expect("轉換");
        assert_eq!(result.surface, "りしれ供さsmall");
        assert_eq!(
            result.languages(),
            vec![Lang::Ja, Lang::Zh, Lang::Ja, Lang::En]
        );
    }

    #[test]
    fn test_mixed_latin_split_fixture() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new(
            "供",
            "ㄍㄨㄥ",
            0,
            crate::lattice::UNKNOWN_POS,
        ));
        lex.push(LexEntry::new(
            "小",
            "ㄒㄧㄠˇ",
            0,
            crate::lattice::UNKNOWN_POS,
        ));
        let ctx = ScoreContext::default();
        let r = Router::new().with_zh(&lex, &ctx);
        let result = r.convert("rishireej/ savul3").expect("轉換");
        assert_eq!(result.surface, "りしれ供さ小");
        assert_eq!(
            result.languages(),
            vec![Lang::Ja, Lang::Zh, Lang::Ja, Lang::Zh]
        );
        let pieces: Vec<&str> = result
            .segments
            .iter()
            .map(|segment| segment.surface.as_str())
            .collect();
        assert_eq!(pieces, vec!["りしれ", "供", "さ", "小"]);
    }

    #[test]
    fn test_mixed_latin_split_trailing_zhu_yin_final() {
        let mut lex = InMemoryLexicon::new();
        for (word, reading) in [
            ("這", "ㄓㄜˋ"),
            ("都", "ㄉㄡ"),
            ("是", "ㄕˋ"),
            ("三", "ㄙㄢ"),
            ("小", "ㄒㄧㄠˇ"),
        ] {
            lex.push(LexEntry::new(word, reading, 0, crate::lattice::UNKNOWN_POS));
        }
        let ctx = ScoreContext::default();
        let r = Router::new().with_zh(&lex, &ctx);
        // `5k42.` 的句點是 ㄡ，空白是 ㄉㄡ 的一聲。切分器看得到句點才解得出來。
        let result = r.convert("5k42. g4n0 vul3").expect("轉換");
        assert_eq!(result.surface, "這都是三小");
    }

    #[test]
    fn test_mixed_latin_split_trailing_period_stays_literal() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new(
            "小",
            "ㄒㄧㄠˇ",
            0,
            crate::lattice::UNKNOWN_POS,
        ));
        let ctx = ScoreContext::default();
        let r = Router::new().with_zh(&lex, &ctx);
        // 英文句尾的句點不是注音，切分器解不出來時照舊剝掉。
        let result = r.convert("hello.").expect("轉換");
        assert_eq!(result.surface, "hello.");
    }

    #[test]
    fn test_symbol_punctuation_stays_symbol() {
        let r = Router::new();
        // R3：句點接在缺韻母的 ㄉ 後面才是 ㄉㄡ，整段留給注音。
        let segs = r.segments("5k42.");
        assert_eq!(segs.len(), 1);
        assert_eq!((segs[0].start, segs[0].end), (0, 5));
        assert_eq!(segs[0].mode, Lang::Zh);

        // 英文句點不是注音，切成英文詞加符號。
        let segs = r.segments("hello.");
        let texts: Vec<&str> = segs
            .iter()
            .map(|seg| &"hello."[seg.start..seg.end])
            .collect();
        assert_eq!(texts, vec!["hello", "."]);
        assert_eq!(segs[1].script, Script::Symbol);

        // 冒號不是注音鍵。
        let segs = r.segments("hello:");
        let texts: Vec<&str> = segs
            .iter()
            .map(|seg| &"hello:"[seg.start..seg.end])
            .collect();
        assert_eq!(texts, vec!["hello", ":"]);

        // run 中間的逗號，前面不是合法注音音節時是符號。
        let segs = r.segments("abc,def");
        let texts: Vec<&str> = segs
            .iter()
            .map(|seg| &"abc,def"[seg.start..seg.end])
            .collect();
        assert_eq!(texts, vec!["abc", ",", "def"]);
    }

    #[test]
    fn test_mixed_latin_split_is_default() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new(
            "供",
            "ㄍㄨㄥ",
            0,
            crate::lattice::UNKNOWN_POS,
        ));
        lex.push(LexEntry::new(
            "小",
            "ㄒㄧㄠˇ",
            0,
            crate::lattice::UNKNOWN_POS,
        ));
        let ctx = ScoreContext::default();
        let r = Router::new().with_zh(&lex, &ctx);
        let result = r.convert("rishireej/ savul3").expect("轉換");
        assert_eq!(result.surface, "りしれ供さ小");
    }

    #[test]
    fn test_unify_segment_nine_cases() {
        let mut lex = InMemoryLexicon::new();
        for (word, reading) in [
            ("這", "ㄓㄜˋ"),
            ("都", "ㄉㄡ"),
            ("是", "ㄕˋ"),
            ("三", "ㄙㄢ"),
            ("小", "ㄒㄧㄠˇ"),
            ("淤", "ㄩ"),
            ("凹", "ㄠ"),
            ("欸", "ㄟ"),
            ("資", "ㄗ"),
            ("杯", "ㄅㄟ"),
            ("測", "ㄘㄜ"),
        ] {
            lex.push(LexEntry::new(word, reading, 0, crate::lattice::UNKNOWN_POS));
        }
        let ctx = ScoreContext::default();
        let dict = Dictionary::new(
            ["code", "hello", "world"]
                .into_iter()
                .map(|word| Word::new(word, 288.0))
                .collect::<Vec<_>>(),
        );
        let e = English::new(Some(dict));
        let r = Router::new().with_zh(&lex, &ctx).with_english(&e);
        // 任務書的九個案例。大千鍵盤 `m` 是 ㄩ，不是 ㄇ，因此 `m ` 的實際
        // 輸出是 淤；`lo y` 依 R2 是 ㄠ ㄟ ㄗ，不是 ㄠ ㄗ。沒有聲調鍵也沒
        // 接空白的音節不轉字：ㄠ、ㄗ 留在注音，只有空白當一聲的 ㄟ 轉成
        // 「欸」；`1ohk ` 的 ㄅㄟ 也一樣留在注音。
        for (input, want) in [
            ("5k42. g4n0 vul3", "這都是三小"),
            ("hello.", "hello."),
            ("hello world", "hello world"),
            ("abc。def", "abc。def"),
            ("m ", "淤"),
            ("1ohk ", "ㄅㄟ測"),
            ("code", "code"),
            ("ABC", "ABC"),
            ("ＡＢＣ", "ＡＢＣ"),
            ("lo y", "ㄠ欸ㄗ"),
        ] {
            let got = r.convert(input).expect("轉換");
            assert_eq!(got.surface, want, "{input:?}");
        }
    }

    #[test]
    fn test_language_carries_between_latin_runs() {
        let r = Router::new();
        // `xi` 單獨是合法羅馬字；前一個 run 是注音時，R4 讓它留在注音。
        let alone = r.segments("xi");
        assert_eq!(alone.len(), 1);
        assert_eq!(alone[0].mode, Lang::Ja);

        let carried = r.segments("dk4 xi");
        assert_eq!(carried.len(), 2);
        assert_eq!(carried[0].mode, Lang::Zh);
        assert_eq!(carried[1].mode, Lang::Zh);
    }

    #[test]
    fn test_r2_space_follows_last_piece_not_whole_run() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("窩", "ㄨㄛ", 0, crate::lattice::UNKNOWN_POS));
        let ctx = ScoreContext::default();
        let dict = Dictionary::new(vec![Word::new("hello", 288.0)]);
        let e = English::new(Some(dict));
        let r = Router::new().with_zh(&lex, &ctx).with_english(&e);
        // `hello` 是英文、`ji` 是注音；空白只被最後一個片段吃掉。
        assert_eq!(r.convert("helloji ").expect("轉換").surface, "hello窩");
    }

    #[test]
    fn test_single_letter_english_tail_stays_literal() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("我", "ㄨㄛˇ", 0, crate::lattice::UNKNOWN_POS));
        let ctx = ScoreContext::default();
        let dict = Dictionary::new(vec![Word::new("hello", 288.0), Word::new("hell", 288.0)]);
        let e = English::new(Some(dict));
        let r = Router::new().with_zh(&lex, &ctx).with_english(&e);
        assert_eq!(r.convert("ji3hellp").expect("轉換").surface, "我hellp");
    }

    #[test]
    fn test_first_tone_does_not_break_japanese_word() {
        let r = Router::new();
        // `desu ` 是整段羅馬字加分隔符，不是 `で` 加注音 `su `。
        let segs = r.segments("desu ");
        assert_eq!(segs.len(), 2);
        assert_eq!((segs[0].start, segs[0].end), (0, 4));
        assert_eq!(segs[0].mode, Lang::Ja);
        assert_eq!(segs[1].script, Script::Symbol);
        assert_eq!(&"desu "[segs[1].start..segs[1].end], " ");
    }

    #[test]
    fn test_english_tail_without_word_pack() {
        // 沒有詞表（`English::new(None)` 只有原型）時，`hell` 查不到；
        // 結尾單字母仍要留在英文，交給 autocomplete。
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("我", "ㄨㄛˇ", 0, crate::lattice::UNKNOWN_POS));
        let ctx = ScoreContext::default();
        let e = English::new(None);
        let r = Router::new().with_zh(&lex, &ctx).with_english(&e);
        assert_eq!(r.convert("ji3hellp").expect("轉換").surface, "我hellp");

        // 無詞表也不能把合法羅馬字或「かな前綴＋英文詞」整段並成英文。
        let segs = r.segments("rishire");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].mode, Lang::Ja);
        let segs = r.segments("kyouhahello");
        let texts: Vec<&str> = segs
            .iter()
            .map(|seg| &"kyouhahello"[seg.start..seg.end])
            .collect();
        assert_eq!(texts, vec!["kyouha", "hello"]);
    }
}
