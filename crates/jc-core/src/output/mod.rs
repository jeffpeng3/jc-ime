//! 輸出組裝、標點、間距與對外快照。
//!
//! 逐片段組裝：每個片段保有自己的語言與輸出變體，不會把整段文字當成單一
//! 語言重新轉換。繁體與簡體的轉換呼叫點保留 trait 介面 [`Converter`]，
//! 這一輪只有 identity 實作，`zhconv` 之後接上。

use crate::candidate::Candidate;
use crate::lang::Mode;
use crate::segment::Segment;

pub mod punct;

pub use punct::{ModelPunctuator, OnnxPunctuator, PunctLatencyStats, SharedPunctuator};

/// 輸出變體。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Variant {
    /// 台灣繁體中文。
    ZhHant,
    /// 簡體中文。
    ZhHans,
    /// 日文新字體。
    JaShinjitai,
    /// 日文舊字體。
    JaKyujitai,
    /// 英文原文。
    English,
}

impl Variant {
    /// 這個變體屬於哪個語言。
    pub fn language(self) -> Mode {
        match self {
            Variant::ZhHant | Variant::ZhHans => Mode::Zh,
            Variant::JaShinjitai | Variant::JaKyujitai => Mode::Ja,
            Variant::English => Mode::En,
        }
    }
}

/// 語言的預設輸出變體。
pub fn default_variant(mode: Mode) -> Variant {
    match mode {
        Mode::Zh => Variant::ZhHant,
        Mode::Ja => Variant::JaShinjitai,
        _ => Variant::English,
    }
}

/// 組裝時的一個片段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutSegment {
    /// 輸出字面。
    pub surface: String,
    /// 這個片段的語言。
    pub mode: Mode,
    /// 輸出變體；`None` 表示用語言的預設變體。
    pub variant: Option<Variant>,
}

impl OutSegment {
    /// 建立片段。
    pub fn new(surface: impl Into<String>, mode: Mode) -> Self {
        Self {
            surface: surface.into(),
            mode,
            variant: None,
        }
    }

    /// 指定輸出變體。
    pub fn with_variant(mut self, variant: Variant) -> Self {
        self.variant = Some(variant);
        self
    }

    /// 實際使用的變體。
    pub fn effective_variant(&self) -> Variant {
        self.variant.unwrap_or_else(|| default_variant(self.mode))
    }
}

/// 輸出組裝的錯誤。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OutputError {
    /// 片段本身的欄位不一致。
    #[error("output: 片段無效")]
    InvalidSegment,
    /// 變體不在契約內，或第一版尚未實作。
    #[error("output: 不支援的輸出變體：{0:?}")]
    UnsupportedVariant(Variant),
    /// 片段的語言標籤與輸出變體屬於不同語言。
    #[error("output: 語言標籤與輸出變體不符")]
    VariantMismatch,
    /// 要輸出簡體，但沒有提供轉換器。
    #[error("output: 缺少繁體轉換器")]
    MissingConverter,
    /// 第 `index` 個片段失敗。
    #[error("output: 片段 {index}: {source}")]
    Segment {
        /// 片段索引。
        index: usize,
        /// 原始原因。
        source: Box<OutputError>,
    },
}

/// 繁體轉換器。
///
/// `zhconv` 之後直接實作這個介面；轉換器必須唯讀、可同時給多個 session 使用。
pub trait Converter {
    /// 把詞面轉成繁體。
    fn to_traditional(&self, text: &str) -> String;
}

/// 原樣輸出的轉換器。
#[derive(Debug, Clone, Copy, Default)]
pub struct IdentityConverter;

impl Converter for IdentityConverter {
    fn to_traditional(&self, text: &str) -> String {
        text.to_string()
    }
}

/// 繁體轉簡體的轉換器。
///
/// `zhconv::Converter` 直接實作這個介面；轉換器必須唯讀、可同時給多個
/// 片段使用。
pub trait HansConverter {
    /// 把繁體詞面轉成簡體。
    fn to_simplified(&self, text: &str) -> String;
}

impl HansConverter for IdentityConverter {
    fn to_simplified(&self, text: &str) -> String {
        text.to_string()
    }
}

/// 把單一片段轉成輸出文字。
pub trait Renderer {
    /// 轉換一個片段。
    fn render(&self, seg: &OutSegment) -> Result<String, OutputError>;
}

/// 依片段自已的輸出變體產生文字。
///
/// 繁體、日文新字體與英文原樣輸出；繁體經 [`Converter`]，簡體經
/// [`HansConverter`]，日文舊字體第一版不支援。
pub struct VariantRenderer {
    converter: Box<dyn Converter>,
    hans: Box<dyn HansConverter>,
}

impl Default for VariantRenderer {
    fn default() -> Self {
        Self {
            converter: Box::new(IdentityConverter),
            hans: Box::new(IdentityConverter),
        }
    }
}

impl VariantRenderer {
    /// 用指定的繁體轉換器建立 renderer。
    pub fn new(converter: impl Converter + 'static) -> Self {
        Self {
            converter: Box::new(converter),
            hans: Box::new(IdentityConverter),
        }
    }

    /// 加上簡體轉換器。
    pub fn with_hans(mut self, converter: impl HansConverter + 'static) -> Self {
        self.hans = Box::new(converter);
        self
    }

    /// 用 identity 轉換器建立 renderer。
    pub fn identity() -> Self {
        Self::default()
    }
}

impl Renderer for VariantRenderer {
    fn render(&self, seg: &OutSegment) -> Result<String, OutputError> {
        let variant = seg.effective_variant();
        if !matches!(seg.mode, Mode::Mixed) && variant.language() != seg.mode {
            return Err(OutputError::VariantMismatch);
        }
        match variant {
            Variant::ZhHant => Ok(self.converter.to_traditional(&seg.surface)),
            Variant::ZhHans => Ok(self.hans.to_simplified(&seg.surface)),
            Variant::JaShinjitai | Variant::English => Ok(seg.surface.clone()),
            Variant::JaKyujitai => Err(OutputError::UnsupportedVariant(Variant::JaKyujitai)),
        }
    }
}

/// 依片段語言補標點。
pub trait Punctuator {
    /// 只插入或替換標點，不改寫既有詞面。
    fn insert(&self, text: &str, mode: Mode) -> String;
}

/// 把半形標點換成該語言的全形標點。
///
/// 只處理緊鄰 CJK 字元的標點，純英文、數字與網址中的標點不動。
#[derive(Debug, Clone, Copy, Default)]
pub struct LanguagePunctuator;

impl Punctuator for LanguagePunctuator {
    fn insert(&self, text: &str, mode: Mode) -> String {
        let table = match mode {
            Mode::Zh => zh_punctuation(),
            Mode::Ja => ja_punctuation(),
            _ => return text.to_string(),
        };
        let mut r: Vec<char> = text.chars().collect();
        let mut changed = false;
        for i in 0..r.len() {
            if let Some(&replacement) = table.get(&r[i]) {
                if punctuation_touches_cjk(&r, i) {
                    r[i] = replacement;
                    changed = true;
                }
            }
        }
        if changed {
            r.into_iter().collect()
        } else {
            text.to_string()
        }
    }
}

fn zh_punctuation() -> std::collections::HashMap<char, char> {
    [
        (',', '，'),
        ('.', '。'),
        ('?', '？'),
        ('!', '！'),
        (':', '：'),
        (';', '；'),
    ]
    .into_iter()
    .collect()
}

fn ja_punctuation() -> std::collections::HashMap<char, char> {
    [
        (',', '、'),
        ('.', '。'),
        ('?', '？'),
        ('!', '！'),
        (':', '：'),
        (';', '；'),
    ]
    .into_iter()
    .collect()
}

fn punctuation_touches_cjk(r: &[char], i: usize) -> bool {
    (i > 0 && is_cjk_word(r[i - 1])) || (i + 1 < r.len() && is_cjk_word(r[i + 1]))
}

/// 字元是否為會與 Latin 相鄰的 CJK 文字。
///
/// 注音符號不是輸出字形，不列入；漢字、假名、長音符與踊り字都會觸發規則。
pub fn is_cjk_word(c: char) -> bool {
    if ('\u{4E00}'..='\u{9FFF}').contains(&c)
        || ('\u{3400}'..='\u{4DBF}').contains(&c)
        || ('\u{F900}'..='\u{FAFF}').contains(&c)
        || ('\u{3040}'..='\u{30FF}').contains(&c)
        || ('\u{20000}'..='\u{2FA1F}').contains(&c)
    {
        return true;
    }
    matches!(c, 'ー' | '々' | '〆' | 'ヶ')
}

/// 決定兩個相鄰片段之間的分隔字串。
pub trait Spacer {
    /// 回傳空字串表示不加空格。
    fn between(&self, left: &OutSegment, right: &OutSegment) -> String;
}

/// 在 CJK 與 Latin 詞面直接相鄰時插入一個半角空格。
#[derive(Debug, Clone, Copy, Default)]
pub struct CjkLatinSpacer;

impl Spacer for CjkLatinSpacer {
    fn between(&self, left: &OutSegment, right: &OutSegment) -> String {
        let l = left.surface.chars().last();
        let r = right.surface.chars().next();
        match (l, r) {
            (Some(l), Some(r))
                if (is_cjk_word(l) && is_ascii_letter(r))
                    || (is_ascii_letter(l) && is_cjk_word(r)) =>
            {
                " ".to_string()
            }
            _ => String::new(),
        }
    }
}

fn is_ascii_letter(c: char) -> bool {
    c.is_ascii_alphabetic()
}

/// 輸出組裝選項。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// CJK 與 Latin 片段之間插入一個半角空格。
    pub space_cjk_latin: bool,
    /// 依片段語言補標點。
    pub punctuate: bool,
}

/// 產品預設：加空格、補標點。
pub fn default_options() -> Options {
    Options {
        space_cjk_latin: true,
        punctuate: true,
    }
}

/// 把已選定的片段依序組成輸出文字。
pub struct Assembler {
    renderer: Box<dyn Renderer>,
    punctuator: Box<dyn Punctuator>,
    spacer: Box<dyn Spacer>,
}

impl Default for Assembler {
    fn default() -> Self {
        Self {
            renderer: Box::new(VariantRenderer::identity()),
            punctuator: Box::new(LanguagePunctuator),
            spacer: Box::new(CjkLatinSpacer),
        }
    }
}

impl Assembler {
    /// 建立組裝器；`None` 的元件用預設實作。
    pub fn new(
        renderer: Option<Box<dyn Renderer>>,
        punctuator: Option<Box<dyn Punctuator>>,
        spacer: Option<Box<dyn Spacer>>,
    ) -> Self {
        Self {
            renderer: renderer.unwrap_or_else(|| Box::new(VariantRenderer::identity())),
            punctuator: punctuator.unwrap_or_else(|| Box::new(LanguagePunctuator)),
            spacer: spacer.unwrap_or_else(|| Box::new(CjkLatinSpacer)),
        }
    }

    /// 依輸入順序組成輸出文字。
    pub fn assemble(&self, segments: &[OutSegment], opts: Options) -> Result<String, OutputError> {
        if segments.is_empty() {
            return Ok(String::new());
        }
        let mut out = String::new();
        let mut prev: Option<&OutSegment> = None;
        for (i, seg) in segments.iter().enumerate() {
            let mut text = self
                .renderer
                .render(seg)
                .map_err(|e| OutputError::Segment {
                    index: i,
                    source: Box::new(e),
                })?;
            if opts.punctuate && !text.is_empty() {
                text = self.punctuator.insert(&text, seg.mode);
            }
            if text.is_empty() {
                continue;
            }
            if let Some(left) = prev {
                if opts.space_cjk_latin {
                    out.push_str(&self.spacer.between(left, seg));
                }
            }
            out.push_str(&text);
            prev = Some(seg);
        }
        Ok(out)
    }
}

/// 以 UTF-16 code unit 計算的字串長度。
pub fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// 以 Unicode scalar value 計算的字串長度。
pub fn rune_len(s: &str) -> usize {
    s.chars().count()
}

/// UTF-8 位元組位移換成 rune 位移。
///
/// 超出字串長度或落在字元中間時，取該位置之前最近的合法字元邊界。
pub fn rune_offset(s: &str, byte_index: usize) -> usize {
    let mut at = byte_index.min(s.len());
    while at > 0 && !s.is_char_boundary(at) {
        at -= 1;
    }
    rune_len(&s[..at])
}

/// UTF-8 位元組位移換成 UTF-16 code unit 位移。
///
/// 超出字串長度時夾在結尾。
pub fn utf16_offset(s: &str, byte_index: usize) -> usize {
    let mut at = byte_index.min(s.len());
    while at > 0 && !s.is_char_boundary(at) {
        at -= 1;
    }
    utf16_len(&s[..at])
}

/// 一次輸入手勢後的完整狀態。
///
/// 外殼不需要保存狀態，每次都以這份快照重畫。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snapshot {
    /// 目前組字字串。
    pub preedit: String,
    /// 游標位置，以 UTF-16 code unit 計算（配合各平台游標 API）。
    pub cursor_utf16: usize,
    /// 游標位置，以 rune（Unicode scalar value）計算。
    ///
    /// 與 [`Snapshot::cursor_utf16`] 描述同一個位置；需要逐字擺放游標的
    /// 外殼用這個欄位，不必自行把 UTF-8 位元組位移換算成 rune。
    pub cursor_rune: usize,
    /// 組字字串的分段。
    pub segments: Vec<Segment>,
    /// 目前可選的候選。
    pub candidates: Vec<Candidate>,
    /// 目前高亮的候選索引；沒有候選時為 0。
    pub highlighted: usize,
    /// Tab 會接受的補全尾綴；沒有建議時為 `None`。
    ///
    /// 這一輪只提供欄位，補全來源由後續任務接上。
    pub suggestion: Option<String>,
    /// 目前高亮候選的顯示原因（例如 `common`、`related`、`personal`）。
    ///
    /// 只供顯示，不參與排序；沒有標示時為 `None`。
    pub label: Option<String>,
    /// 使用者是否已進入明確的候選選取模式（例如按過上下鍵）。
    ///
    /// 候選窗顯示中但還沒開始選取時是 `false`。
    pub navigating: bool,
}

impl Snapshot {
    /// 建立空快照。
    pub fn empty() -> Self {
        Self::default()
    }

    /// 以組字字串建立快照，游標放在結尾。
    pub fn from_preedit(preedit: impl Into<String>) -> Self {
        let preedit = preedit.into();
        Self {
            cursor_utf16: utf16_len(&preedit),
            cursor_rune: rune_len(&preedit),
            preedit,
            ..Default::default()
        }
    }

    /// 是否沒有任何組字內容。
    pub fn is_empty(&self) -> bool {
        self.preedit.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::CandidateSource;

    #[test]
    fn empty_has_zero_cursor_and_no_items() {
        let s = Snapshot::empty();
        assert_eq!(s.preedit, "");
        assert_eq!(s.cursor_utf16, 0);
        assert_eq!(s.cursor_rune, 0);
        assert!(s.segments.is_empty());
        assert!(s.candidates.is_empty());
        assert_eq!(s.highlighted, 0);
        assert_eq!(s.suggestion, None);
        assert_eq!(s.label, None);
        assert!(!s.navigating);
        assert!(s.is_empty());
    }

    #[test]
    fn populated_snapshot_keeps_highlighted_index() {
        let s = Snapshot {
            preedit: "ㄨㄛˇ".to_string(),
            cursor_utf16: 3,
            cursor_rune: 3,
            segments: vec![Segment::new(
                0,
                9,
                Mode::Zh,
                crate::segment::Script::Bopomofo,
            )],
            candidates: vec![Candidate::new("我", "ㄨㄛˇ", 1, CandidateSource::Lexicon)],
            highlighted: 0,
            ..Default::default()
        };
        assert_eq!(s.cursor_utf16, 3);
        assert_eq!(s.cursor_rune, 3);
        assert_eq!(s.segments[0].len(), 9);
        assert_eq!(s.candidates[0].surface, "我");
        assert!(!s.is_empty());
    }

    #[test]
    fn cursor_uses_utf16_code_units() {
        assert_eq!(utf16_len("abc"), 3);
        assert_eq!(utf16_len("我"), 1);
        assert_eq!(utf16_len("😀"), 2);
        assert_eq!(utf16_offset("a我", 4), 2);
        assert_eq!(utf16_offset("a我", 99), 2);
        let s = Snapshot::from_preedit("我😀");
        assert_eq!(s.cursor_utf16, 3);
        assert_eq!(s.cursor_rune, 2);
    }

    #[test]
    fn cursor_rune_converts_byte_offsets_on_char_boundary() {
        assert_eq!(rune_len("abc"), 3);
        assert_eq!(rune_len("我😀"), 2);
        assert_eq!(rune_offset("abc", 2), 2);
        assert_eq!(rune_offset("a我", 1), 1);
        assert_eq!(rune_offset("a我", 2), 1, "落在字元中間取前一個邊界");
        assert_eq!(rune_offset("a我", 4), 2);
        assert_eq!(rune_offset("a我", 99), 2, "超出長度夾在結尾");
        assert_eq!(rune_offset("", 0), 0);
    }

    #[test]
    fn suggestion_label_and_navigating_default_to_unset() {
        let s = Snapshot::from_preedit("ㄇㄚ");
        assert_eq!(s.suggestion, None);
        assert_eq!(s.label, None);
        assert!(!s.navigating);
        let filled = Snapshot {
            suggestion: Some("ㄚ".to_string()),
            label: Some("common".to_string()),
            navigating: true,
            ..s
        };
        assert_eq!(filled.suggestion.as_deref(), Some("ㄚ"));
        assert_eq!(filled.label.as_deref(), Some("common"));
        assert!(filled.navigating);
    }

    #[test]
    fn default_variant_follows_mode() {
        assert_eq!(default_variant(Mode::Zh), Variant::ZhHant);
        assert_eq!(default_variant(Mode::Ja), Variant::JaShinjitai);
        assert_eq!(default_variant(Mode::En), Variant::English);
        assert_eq!(Variant::ZhHans.language(), Mode::Zh);
        assert_eq!(Variant::JaKyujitai.language(), Mode::Ja);
    }

    #[test]
    fn variant_renderer_per_variant() {
        let r = VariantRenderer::identity();
        let cases = [
            (
                OutSegment::new("臺灣", Mode::Zh).with_variant(Variant::ZhHant),
                Ok("臺灣".to_string()),
            ),
            (
                OutSegment::new("日本語です", Mode::Ja).with_variant(Variant::JaShinjitai),
                Ok("日本語です".to_string()),
            ),
            (
                OutSegment::new("Hello, world! 123", Mode::En).with_variant(Variant::English),
                Ok("Hello, world! 123".to_string()),
            ),
            (OutSegment::new("臺灣", Mode::Zh), Ok("臺灣".to_string())),
            (
                OutSegment::new("國", Mode::Ja).with_variant(Variant::JaKyujitai),
                Err(OutputError::UnsupportedVariant(Variant::JaKyujitai)),
            ),
            (
                OutSegment::new("臺灣", Mode::Zh).with_variant(Variant::ZhHans),
                Ok("臺灣".to_string()),
            ),
            (
                OutSegment::new("臺灣", Mode::En).with_variant(Variant::ZhHant),
                Err(OutputError::VariantMismatch),
            ),
        ];
        for (seg, want) in cases {
            assert_eq!(r.render(&seg), want, "{seg:?}");
        }
    }

    #[test]
    fn converter_is_used_only_for_traditional() {
        use std::cell::Cell;
        use std::rc::Rc;
        struct Counting(Rc<Cell<usize>>);
        impl Converter for Counting {
            fn to_traditional(&self, text: &str) -> String {
                self.0.set(self.0.get() + 1);
                text.to_string()
            }
        }
        let calls = Rc::new(Cell::new(0));
        let r = VariantRenderer::new(Counting(calls.clone()));
        assert_eq!(
            r.render(&OutSegment::new("繁", Mode::Zh).with_variant(Variant::ZhHant)),
            Ok("繁".to_string())
        );
        assert_eq!(calls.get(), 1);
        assert_eq!(
            r.render(&OutSegment::new("繁", Mode::Zh).with_variant(Variant::ZhHans)),
            Ok("繁".to_string())
        );
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn hans_converter_is_used_only_for_simplified() {
        use std::cell::Cell;
        use std::rc::Rc;
        struct Counting(Rc<Cell<usize>>);
        impl HansConverter for Counting {
            fn to_simplified(&self, text: &str) -> String {
                self.0.set(self.0.get() + 1);
                text.replace('繁', "简")
            }
        }
        let calls = Rc::new(Cell::new(0));
        let r = VariantRenderer::identity().with_hans(Counting(calls.clone()));
        assert_eq!(
            r.render(&OutSegment::new("繁", Mode::Zh).with_variant(Variant::ZhHans)),
            Ok("简".to_string())
        );
        assert_eq!(calls.get(), 1);
        assert_eq!(
            r.render(&OutSegment::new("繁", Mode::Zh).with_variant(Variant::ZhHant)),
            Ok("繁".to_string())
        );
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn language_punctuator_follows_cjk_neighbours() {
        let p = LanguagePunctuator;
        assert_eq!(p.insert("你好,世界", Mode::Zh), "你好，世界");
        assert_eq!(p.insert("完成了!", Mode::Zh), "完成了！");
        assert_eq!(p.insert("這是什麼?:好", Mode::Zh), "這是什麼？：好");
        assert_eq!(p.insert("こんにちは,世界", Mode::Ja), "こんにちは、世界");
        assert_eq!(p.insert("です.", Mode::Ja), "です。");
        assert_eq!(p.insert("Hello, world!", Mode::En), "Hello, world!");
        assert_eq!(p.insert("版本3.14", Mode::Zh), "版本3.14");
        assert_eq!(p.insert("檔案a.txt", Mode::Zh), "檔案a.txt");
        assert_eq!(p.insert("hello,world", Mode::Zh), "hello,world");
        assert_eq!(p.insert("你好，世界。", Mode::Zh), "你好，世界。");
        assert_eq!(p.insert("OK,你好", Mode::Zh), "OK，你好");
    }

    #[test]
    fn cjk_latin_spacer_only_between_cjk_and_letters() {
        let sp = CjkLatinSpacer;
        let seg = |mode, s: &str| OutSegment::new(s, mode);
        assert_eq!(sp.between(&seg(Mode::Ja, "私"), &seg(Mode::En, "is")), " ");
        assert_eq!(
            sp.between(&seg(Mode::En, "is"), &seg(Mode::Zh, "學生")),
            " "
        );
        assert_eq!(
            sp.between(&seg(Mode::Ja, "です"), &seg(Mode::En, "OK")),
            " "
        );
        assert_eq!(
            sp.between(&seg(Mode::Ja, "コンピューター"), &seg(Mode::En, "ABC")),
            " "
        );
        assert_eq!(sp.between(&seg(Mode::Zh, "私"), &seg(Mode::En, "123")), "");
        assert_eq!(sp.between(&seg(Mode::En, "abc"), &seg(Mode::En, "def")), "");
        assert_eq!(sp.between(&seg(Mode::Zh, "私 "), &seg(Mode::En, "is")), "");
        assert_eq!(sp.between(&seg(Mode::Zh, ""), &seg(Mode::En, "is")), "");
        assert_eq!(
            sp.between(&seg(Mode::En, "Python"), &seg(Mode::Zh, "文件")),
            " "
        );
        assert_eq!(sp.between(&seg(Mode::Zh, "「"), &seg(Mode::En, "a")), "");
        assert_eq!(sp.between(&seg(Mode::Ja, "々"), &seg(Mode::En, "a")), " ");
    }

    #[test]
    fn assembler_keeps_per_segment_variants() {
        let a = Assembler::default();
        let segs = vec![
            OutSegment::new("私", Mode::Ja).with_variant(Variant::JaShinjitai),
            OutSegment::new("use", Mode::En).with_variant(Variant::English),
            OutSegment::new("臺灣", Mode::Zh).with_variant(Variant::ZhHant),
            OutSegment::new("and", Mode::En).with_variant(Variant::English),
        ];
        let got = a
            .assemble(
                &segs,
                Options {
                    space_cjk_latin: true,
                    punctuate: false,
                },
            )
            .expect("assemble");
        assert_eq!(got, "私 use 臺灣 and");
    }

    #[test]
    fn assembler_punctuation_follows_segment_language() {
        let a = Assembler::default();
        let segs = vec![
            OutSegment::new("你好,世界", Mode::Zh),
            OutSegment::new("Hello, world!", Mode::En),
            OutSegment::new("これは,ペン", Mode::Ja),
        ];
        let got = a.assemble(&segs, default_options()).expect("assemble");
        assert_eq!(got, "你好，世界 Hello, world!これは、ペン");

        let raw = a
            .assemble(
                &segs,
                Options {
                    space_cjk_latin: true,
                    punctuate: false,
                },
            )
            .expect("assemble");
        assert_eq!(raw, "你好,世界 Hello, world!これは,ペン");

        let no_space = a
            .assemble(
                &segs,
                Options {
                    space_cjk_latin: false,
                    punctuate: true,
                },
            )
            .expect("assemble");
        assert_eq!(no_space, "你好，世界Hello, world!これは、ペン");
    }

    #[test]
    fn assembler_reports_segment_index() {
        let a = Assembler::default();
        let segs = vec![
            OutSegment::new("ok", Mode::En).with_variant(Variant::English),
            OutSegment::new("國", Mode::Ja).with_variant(Variant::JaKyujitai),
        ];
        let err = a
            .assemble(&segs, default_options())
            .expect_err("should fail");
        assert!(matches!(err, OutputError::Segment { index: 1, .. }));
        assert!(err.to_string().contains("片段 1"));
    }

    #[test]
    fn assembler_skips_empty_segments_and_handles_empty_input() {
        let a = Assembler::default();
        let segs = vec![
            OutSegment::new("私", Mode::Ja),
            OutSegment::new("", Mode::En),
            OutSegment::new("is", Mode::En),
        ];
        assert_eq!(
            a.assemble(&segs, default_options()).expect("assemble"),
            "私 is"
        );
        assert_eq!(a.assemble(&[], default_options()).expect("assemble"), "");
    }
}
