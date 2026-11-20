//! `mixed-diff`：兩種分詞方式的大規模差異盤點。
//!
//! 這個子命令只做量測，不改任何轉換行為。它用固定種子產生混合輸入，對同一個
//! 字串同時跑離線 DP（[`jc_core::mixed_latin::split`]）與正式路由
//! （[`Router::convert`] 且 `mixed_latin_split: true`），把不一致的案例分類後
//! 輸出統計、代表案例與可回歸的 JSON。
//!
//! 資料載入與 `mixed-split` 相同：注音走二進位資料包或 golden 詞庫，日文走
//! 預設 pack，英文走 `English::load_default()`。

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;

use jc_core::mixed::{English, MixedResult, Router};
use jc_core::mixed_latin::{self, LexiconAdapter, Options as SplitOptions, Split, SplitLang};
use jc_core::{Mode, Script};

use crate::cli::CliError;
use crate::data;

/// `mixed-diff` 的參數。
#[derive(Debug, Clone)]
pub struct DiffOptions {
    /// 產生器種子；相同參數會產生完全相同的資料。
    pub seed: u64,
    /// 要產生的輸入數量。
    pub count: usize,
    /// 每桶輸出的代表案例數。
    pub top: usize,
    /// `0c-testdata` 目錄；未指定時用 `JC_TESTDATA`。
    pub testdata: Option<PathBuf>,
    /// 注音二進位資料包目錄。
    pub data: Option<PathBuf>,
    /// 輸出完整 JSON。
    pub json: bool,
    /// 把不一致案例寫成 JSON 檔。
    pub cases: Option<PathBuf>,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            seed: 1,
            count: 20_000,
            top: 3,
            testdata: None,
            data: None,
            json: false,
            cases: None,
        }
    }
}

/// 跑一次差異盤點。
pub fn run(opts: &DiffOptions, out: &mut dyn Write) -> Result<(), CliError> {
    let dir = data::testdata_dir(opts.testdata.clone())?;
    let root = data::data_root(&dir);
    let resources = match &opts.data {
        Some(path) => data::load_zh_bin_resources(path)?,
        None => {
            let bin = root.join("build-zh");
            if bin.join("manifest.json").is_file() {
                data::load_zh_bin_resources(&bin)?
            } else {
                data::load_zh_resources(&dir.join("golden/zh/data"))?
            }
        }
    };
    let ja = data::load_ja_decoder(&root)
        .ok()
        .map(|decoder| decoder.with_prefer_hiragana(true));
    let english = English::load_default();
    let config = jc_core::mixed::Config {
        mixed_latin_split: true,
        ..jc_core::mixed::Config::default()
    };
    let mut router = Router::new()
        .with_config(config)
        .with_zh(&resources.lexicon, &resources.ctx)
        .with_english(&english);
    if let Some(decoder) = &ja {
        router = router.with_ja(decoder);
    }

    let pool = Pools::build(&english);
    let inputs = generate(&pool, opts.seed, opts.count);
    let adapter = LexiconAdapter {
        zh: Some(&resources.lexicon),
        english: Some(&english),
    };
    let split_opts = SplitOptions::default();

    let mut cases = Vec::with_capacity(inputs.len());
    let mut a_none = 0usize;
    for input in &inputs {
        let a = mixed_latin::split(input, &split_opts, &adapter);
        if a.is_none() {
            a_none += 1;
        }
        let b = match router.convert(input) {
            Ok(result) => BOutcome::Ok(result),
            Err(err) => BOutcome::Err(err.to_string()),
        };
        let mut case = Case {
            input: input.clone(),
            a,
            b,
            signals: Vec::new(),
        };
        if !case.agrees() {
            case.signals = classify(&case, &english);
            if case.signals.is_empty() {
                case.signals.push(Bucket::Other);
            }
        }
        cases.push(case);
    }

    let summary = Summary::new(&cases, a_none, opts);
    if opts.json {
        let value = summary.json(&cases);
        writeln!(
            out,
            "{}",
            serde_json::to_string(&value)
                .map_err(|err| CliError::Eval(format!("輸出 JSON 失敗：{err}")))?
        )?;
    } else {
        summary.write_human(out)?;
        write_examples(&cases, &summary, out)?;
    }
    if let Some(path) = &opts.cases {
        let value = summary.cases_json(&cases);
        let text = serde_json::to_string(&value)
            .map_err(|err| CliError::Eval(format!("輸出案例 JSON 失敗：{err}")))?;
        std::fs::write(path, text)
            .map_err(|err| CliError::Eval(format!("寫入 {} 失敗：{err}", path.display())))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 片段池與產生器
// ---------------------------------------------------------------------------

/// 產生器使用的片段池。
struct Pools {
    zh: Vec<String>,
    ja: Vec<String>,
    en: Vec<String>,
    short_en: Vec<String>,
    atom: Vec<String>,
    punct_mid: Vec<String>,
    punct_tail: Vec<String>,
}

impl Pools {
    fn build(english: &English) -> Self {
        let zh = zh_fragments();
        let mut ja = ja_fragments();
        let mut en = Vec::new();
        let mut short_en = Vec::new();
        for word in english.dictionary().words() {
            let text = word.text.as_str();
            if text.is_empty() || !text.chars().all(|ch| ch.is_ascii_lowercase()) {
                continue;
            }
            let len = text.chars().count();
            if (1..=2).contains(&len) {
                short_en.push(text.to_string());
            } else if len <= 12 {
                en.push(text.to_string());
            }
        }
        // 詞表可能不含 `a`、`i` 這類單字母；補上固定的短詞讓這一桶一定有樣本。
        for word in ["a", "i", "is", "to", "in", "of", "it", "we", "my", "go"] {
            if !short_en.iter().any(|existing| existing == word) {
                short_en.push(word.to_string());
            }
        }
        ja.sort();
        ja.dedup();
        en.sort();
        en.dedup();
        short_en.sort();
        short_en.dedup();
        Self {
            zh,
            ja,
            en,
            short_en,
            atom: vec![
                "5k42".to_string(),
                "Hello".to_string(),
                "ABC".to_string(),
                "API".to_string(),
                "3d".to_string(),
                "123".to_string(),
                "42".to_string(),
                "2026".to_string(),
                "v2".to_string(),
                "ＡＢＣ".to_string(),
            ],
            punct_mid: vec![
                "hello.world".to_string(),
                "abc,def".to_string(),
                "sa;ku".to_string(),
                "ab/cd".to_string(),
                "ab-cd".to_string(),
                "foo:bar".to_string(),
                "hi!there".to_string(),
                "a?b".to_string(),
                "vul3.abc".to_string(),
                "5k42.g4".to_string(),
                "hello，world".to_string(),
                "abc。def".to_string(),
            ],
            punct_tail: vec![
                "hello.".to_string(),
                "world,".to_string(),
                "abc;".to_string(),
                "foo:".to_string(),
                "ab/".to_string(),
                "cd-".to_string(),
                "hi!".to_string(),
                "no?".to_string(),
                "5k42.".to_string(),
                "vul3.".to_string(),
                "ej/;".to_string(),
                "g4n0,".to_string(),
            ],
        }
    }

    /// 依類別均勻抽一個片段，確保每個類別都會被涵蓋。
    fn pick(&self, rng: &mut Rng) -> &str {
        let buckets: [&[String]; 7] = [
            &self.zh,
            &self.ja,
            &self.en,
            &self.short_en,
            &self.atom,
            &self.punct_mid,
            &self.punct_tail,
        ];
        let bucket = buckets[rng.below(buckets.len())];
        &bucket[rng.below(bucket.len())]
    }
}

/// 大千鍵盤的按鍵反查。
fn key_table() -> HashMap<char, char> {
    let keyboard = jc_core::zh::Keyboard::standard();
    let keys = "1234567890qwertyuiopasdfghjklzxcvbnm,./;- ";
    keys.chars()
        .filter_map(|key| keyboard.symbol(key).map(|symbol| (symbol, key)))
        .collect()
}

/// 注音片段：合法音節的按鍵，含帶調、不帶調與一聲空白。
fn zh_fragments() -> Vec<String> {
    let keys = key_table();
    let mut out = Vec::new();
    for syllable in jc_core::zh::pinyin::inventory() {
        let base = syllable.as_str();
        let Some(keyed) = base
            .chars()
            .map(|symbol| keys.get(&symbol).copied())
            .collect::<Option<String>>()
        else {
            continue;
        };
        out.push(keyed.clone());
        for tone in ['6', '3', '4', '7'] {
            out.push(format!("{keyed}{tone}"));
        }
        // 一聲：一聲鍵是空白，音節後面接空白就是一聲。
        out.push(format!("{keyed} "));
    }
    out.sort();
    out.dedup();
    out
}

/// 羅馬字片段：內建對照表加上促音與 `n`／`nn`。
fn ja_fragments() -> Vec<String> {
    let mut out = Vec::new();
    for (keys, _) in jc_core::ja::input::default_romaji_table().entries() {
        if keys.is_empty() || !keys.chars().all(|ch| ch.is_ascii_lowercase()) {
            continue;
        }
        out.push(keys.to_string());
        let mut chars = keys.chars();
        if let Some(first) = chars.next() {
            if keys.chars().count() >= 2 && can_sokuon(first) {
                out.push(format!("{first}{keys}"));
            }
        }
    }
    for extra in ["n", "nn"] {
        out.push(extra.to_string());
    }
    out
}

/// 重複這個子音時是否形成促音；與 `ja::input` 的規則一致。
fn can_sokuon(ch: char) -> bool {
    matches!(
        ch,
        'b' | 'c'
            | 'd'
            | 'f'
            | 'g'
            | 'h'
            | 'j'
            | 'k'
            | 'm'
            | 'p'
            | 'q'
            | 'r'
            | 's'
            | 't'
            | 'v'
            | 'z'
    )
}

/// SplitMix64；只為產生器提供可重現的隨機序列。
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_add(0x9E37_79B9_7F4A_7C15))
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next_u64() % 100 < percent
    }
}

/// 依固定種子產生輸入；同一組參數會得到完全相同的序列。
fn generate(pool: &Pools, seed: u64, count: usize) -> Vec<String> {
    let mut rng = Rng::new(seed);
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(count);
    let soft_limit = count.saturating_mul(200);
    let mut attempts = 0usize;
    while out.len() < count {
        attempts += 1;
        let fragments = 2 + rng.below(5);
        let mut text = String::new();
        for index in 0..fragments {
            if index > 0 && rng.chance(50) {
                text.push(' ');
            }
            text.push_str(pool.pick(&mut rng));
        }
        let chars = text.chars().count();
        if !(3..=40).contains(&chars) {
            continue;
        }
        if seen.insert(text.clone()) || attempts > soft_limit {
            out.push(text);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 比對
// ---------------------------------------------------------------------------

/// 正規化後的語言標籤。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    Ja,
    Zh,
    En,
    Sym,
}

/// 兩邊共用的片段表示。
#[derive(Debug, Clone)]
struct Span {
    lang: Lang,
    start: usize,
    end: usize,
    /// A 的讀音。
    reading: Option<String>,
    /// B 的輸出字面。
    surface: Option<String>,
}

impl Span {
    fn text<'a>(&self, input: &'a str) -> &'a str {
        input.get(self.start..self.end).unwrap_or_default()
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.start < other.end && other.start < self.end
    }
}

/// B 的結果。
#[derive(Debug, Clone)]
enum BOutcome {
    Ok(MixedResult),
    Err(String),
}

impl BOutcome {
    fn as_ref_result(&self) -> Option<&MixedResult> {
        match self {
            Self::Ok(result) => Some(result),
            Self::Err(_) => None,
        }
    }
}

/// 一個輸入的完整量測結果。
struct Case {
    input: String,
    a: Option<Split>,
    b: BOutcome,
    /// 命中的分類訊號；第一個是主分類，空的時候代表一致。
    signals: Vec<Bucket>,
}

impl Case {
    fn bucket(&self) -> Option<Bucket> {
        self.signals.first().copied()
    }

    fn a_spans(&self) -> Vec<Span> {
        self.a
            .as_ref()
            .map(|split| {
                split
                    .pieces
                    .iter()
                    .map(|piece| Span {
                        lang: match piece.lang {
                            SplitLang::Ja => Lang::Ja,
                            SplitLang::Zh => Lang::Zh,
                            SplitLang::En => Lang::En,
                        },
                        start: piece.start,
                        end: piece.end,
                        reading: Some(piece.reading.clone()),
                        surface: None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn b_spans(&self) -> Vec<Span> {
        self.b
            .as_ref_result()
            .map(|result| {
                result
                    .segments
                    .iter()
                    .filter_map(|segment| {
                        let text = self
                            .input
                            .get(segment.input_start..segment.input_end)
                            .unwrap_or_default();
                        if segment.script == Script::Symbol && text.chars().all(char::is_whitespace)
                        {
                            return None;
                        }
                        let lang = if segment.script == Script::Symbol {
                            Lang::Sym
                        } else {
                            match segment.mode {
                                Mode::Ja => Lang::Ja,
                                Mode::Zh => Lang::Zh,
                                Mode::En | Mode::Mixed => Lang::En,
                            }
                        };
                        Some(Span {
                            lang,
                            start: segment.input_start,
                            end: segment.input_end,
                            reading: None,
                            surface: Some(segment.surface.clone()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 兩邊是否一致。
    fn agrees(&self) -> bool {
        let a_spans = self.a_spans();
        let b_spans = self.b_spans();
        match (&self.a, &self.b) {
            (None, BOutcome::Ok(_)) => b_spans.is_empty(),
            (None, BOutcome::Err(_)) => true,
            (Some(_), BOutcome::Err(_)) => false,
            (Some(_), BOutcome::Ok(_)) => {
                if a_spans.len() != b_spans.len() {
                    return false;
                }
                a_spans.iter().zip(&b_spans).all(|(a, b)| {
                    a.lang == b.lang && a.start == b.start && a.end == b.end && literals_agree(a, b)
                })
            }
        }
    }
}

/// 語言與邊界相同時，再比對兩邊實際輸出的字面。
///
/// A 只產生讀音（かな、注音符號、英文照字面），B 會把讀音送進解碼器，因此
/// 只有英文片段的字面可以直接比；日文與注音的差異是轉換層的職責。
fn literals_agree(a: &Span, b: &Span) -> bool {
    if a.lang != Lang::En {
        return true;
    }
    match (&a.reading, &b.surface) {
        (Some(reading), Some(surface)) => reading == surface,
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// 分類
// ---------------------------------------------------------------------------

/// 不一致的主分類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Bucket {
    /// 大寫或全角讓 B 走不同分支。
    AtomUppercaseFullwidth,
    /// A 把 run 尾端的 `.`、`,`、`;` 當韻母，B 當標點。
    TrailingPunctStripped,
    /// A 把 run 開頭的 `.`、`,`、`;`、`/`、`-` 當韻母，B 當標點。
    LeadingPunct,
    /// 空白在一邊是一聲、在另一邊是分隔。
    SpaceToneVsSeparator,
    /// A 用英文詞表判成英文，B 沒有。
    EnWordMissing,
    /// B 用英文詞表判成英文，A 沒有。
    EnWordExtra,
    /// 1–2 個字母的英文。
    ShortEn,
    /// 羅馬字與注音互搶同一段。
    JaVsZh,
    /// 羅馬字與英文互搶同一段。
    JaVsEn,
    /// B 在 run 中間因標點或符號斷開。
    MidRunSymbol,
    /// B 把空白兩側的同語言片段併成一個片段，A 仍分成兩段。
    SpaceMergedSegments,
    /// A 無法完整解析。
    ANone,
    /// A 有結果，B 沒有任何片段。
    BEmpty,
    /// B 轉換失敗。
    BError,
    /// 以上都不是。
    Other,
}

impl Bucket {
    const ALL: [Bucket; 15] = [
        Bucket::AtomUppercaseFullwidth,
        Bucket::TrailingPunctStripped,
        Bucket::LeadingPunct,
        Bucket::SpaceToneVsSeparator,
        Bucket::EnWordMissing,
        Bucket::EnWordExtra,
        Bucket::ShortEn,
        Bucket::JaVsZh,
        Bucket::JaVsEn,
        Bucket::MidRunSymbol,
        Bucket::SpaceMergedSegments,
        Bucket::ANone,
        Bucket::BEmpty,
        Bucket::BError,
        Bucket::Other,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::AtomUppercaseFullwidth => "atom-uppercase-fullwidth",
            Self::TrailingPunctStripped => "trailing-punct-stripped",
            Self::LeadingPunct => "leading-punct",
            Self::SpaceToneVsSeparator => "space-tone-vs-separator",
            Self::EnWordMissing => "en-word-missing",
            Self::EnWordExtra => "en-word-extra",
            Self::ShortEn => "short-en",
            Self::JaVsZh => "ja-vs-zh",
            Self::JaVsEn => "ja-vs-en",
            Self::MidRunSymbol => "mid-run-symbol",
            Self::SpaceMergedSegments => "space-merged-segments",
            Self::ANone => "a-none",
            Self::BEmpty => "b-empty",
            Self::BError => "b-error",
            Self::Other => "other",
        }
    }
}

/// 找出一個不一致案例命中的所有分類，依優先序排列。
fn classify(case: &Case, english: &English) -> Vec<Bucket> {
    let input = case.input.as_str();
    let a_spans = case.a_spans();
    let b_spans = case.b_spans();
    let mut signals = Vec::new();

    if matches!(case.b, BOutcome::Err(_)) {
        signals.push(Bucket::BError);
    }
    if has_upper_or_fullwidth(input) {
        signals.push(Bucket::AtomUppercaseFullwidth);
    }
    if trailing_punct_eaten(input, &a_spans, &b_spans) {
        signals.push(Bucket::TrailingPunctStripped);
    }
    if leading_punct_eaten(input, &a_spans, &b_spans) {
        signals.push(Bucket::LeadingPunct);
    }
    if space_as_tone(input, &a_spans) {
        signals.push(Bucket::SpaceToneVsSeparator);
    }
    if lang_conflict(&a_spans, &b_spans, Lang::Ja, Lang::Zh) {
        signals.push(Bucket::JaVsZh);
    }
    if en_word_missing(&a_spans, &b_spans) {
        signals.push(Bucket::EnWordMissing);
    }
    if en_word_extra(input, english, &a_spans, &b_spans) {
        signals.push(Bucket::EnWordExtra);
    }
    if short_en_mismatch(input, english, &a_spans, &b_spans) {
        signals.push(Bucket::ShortEn);
    }
    if lang_conflict(&a_spans, &b_spans, Lang::Ja, Lang::En) {
        signals.push(Bucket::JaVsEn);
    }
    if has_mid_symbol(input, &b_spans) {
        signals.push(Bucket::MidRunSymbol);
    }
    if case.a.is_none() {
        signals.push(Bucket::ANone);
    }
    if merges_across_space(input, &b_spans) {
        signals.push(Bucket::SpaceMergedSegments);
    }
    if case.a.is_some() && b_spans.is_empty() {
        signals.push(Bucket::BEmpty);
    }
    signals
}

/// 輸入是否含 ASCII 大寫或全角英數字。
fn has_upper_or_fullwidth(input: &str) -> bool {
    input
        .chars()
        .any(|ch| ch.is_ascii_uppercase() || jc_core::mixed::is_fullwidth_alnum(ch))
}

/// 拉丁 run 的尾端標點是否被 A 當成韻母。
fn trailing_punct_eaten(input: &str, a_spans: &[Span], b_spans: &[Span]) -> bool {
    for symbol in b_spans {
        if symbol.lang != Lang::Sym {
            continue;
        }
        let text = symbol.text(input);
        if text.is_empty() || !text.chars().all(|ch| ".,;".contains(ch)) {
            continue;
        }
        let covered = a_spans.iter().any(|span| {
            span.lang == Lang::Zh && span.start <= symbol.start && span.end >= symbol.end
        });
        if !covered {
            continue;
        }
        // 這個標點是不是該 run 的最後一個字元。
        let next_is_token = input
            .get(symbol.end..)
            .and_then(|rest| rest.chars().next())
            .is_some_and(is_token_rune);
        if !next_is_token {
            return true;
        }
    }
    false
}

/// A 是否把空白當成注音的一聲。
///
/// 空白被吃掉後，兩側的注音片段在輸入上連續，`mixed_latin::split` 會把它們
/// 併成一段，所以判斷條件是「注音片段裡面含空白」，不只結尾。
fn space_as_tone(input: &str, a_spans: &[Span]) -> bool {
    a_spans.iter().any(|span| {
        span.lang == Lang::Zh
            && span.end > span.start
            && input
                .get(span.start..span.end)
                .is_some_and(|text| text.chars().any(char::is_whitespace))
    })
}

/// A 把 run 開頭的標點當成韻母。
fn leading_punct_eaten(input: &str, a_spans: &[Span], b_spans: &[Span]) -> bool {
    for symbol in b_spans {
        if symbol.lang != Lang::Sym {
            continue;
        }
        let text = symbol.text(input);
        if text.is_empty() {
            continue;
        }
        let trimmed = text.trim();
        if trimmed.is_empty() || !trimmed.chars().all(|ch| ".,;/-".contains(ch)) {
            continue;
        }
        let start = symbol.start + (text.len() - text.trim_start().len());
        let at_run_head = input
            .get(..start)
            .and_then(|left| left.chars().next_back())
            .is_none_or(|ch| !is_run_char(ch));
        if !at_run_head {
            continue;
        }
        let attached = a_spans.iter().any(|span| {
            span.lang == Lang::Zh && span.start <= symbol.start && span.end > symbol.end
        });
        if attached {
            return true;
        }
    }
    false
}

/// A 判成英文，但 B 沒有覆蓋同一段的英文片段。
fn en_word_missing(a_spans: &[Span], b_spans: &[Span]) -> bool {
    a_spans
        .iter()
        .filter(|span| span.lang == Lang::En)
        .any(|span| {
            !b_spans
                .iter()
                .any(|b| b.lang == Lang::En && b.start <= span.start && b.end >= span.end)
        })
}

/// B 判成英文，但 A 沒有覆蓋同一段的英文片段。
fn en_word_extra(input: &str, english: &English, a_spans: &[Span], b_spans: &[Span]) -> bool {
    if a_spans.is_empty() {
        return false;
    }
    b_spans
        .iter()
        .filter(|span| span.lang == Lang::En)
        .any(|span| {
            let text = span.text(input);
            text.chars().count() >= 3
                && text.chars().all(|ch| ch.is_ascii_lowercase())
                && english.has(text)
                && !a_spans
                    .iter()
                    .any(|a| a.lang == Lang::En && a.start <= span.start && a.end >= span.end)
        })
}

/// B 把 1–2 個字母的英文詞當成英文，A 沒有。
///
/// 只看英文詞表認得的詞，避免把 `ka` 這類羅馬字誤判成短英文。
fn short_en_mismatch(input: &str, english: &English, a_spans: &[Span], b_spans: &[Span]) -> bool {
    if a_spans.is_empty() {
        return false;
    }
    for (start, end) in ascii_word_spans(input) {
        let word = &input[start..end];
        if word.chars().count() > 2 || !english.has(word) {
            continue;
        }
        let b_en = b_spans
            .iter()
            .any(|span| span.lang == Lang::En && span.start <= start && span.end >= end);
        let a_en = a_spans
            .iter()
            .any(|span| span.lang == Lang::En && span.start <= start && span.end >= end);
        if b_en && !a_en {
            return true;
        }
    }
    false
}

/// 掃出連續 ASCII 字母的範圍。
fn ascii_word_spans(input: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    let mut end = 0usize;
    for (index, ch) in input.char_indices() {
        if ch.is_ascii_alphabetic() {
            start.get_or_insert(index);
            end = index + ch.len_utf8();
        } else if let Some(begin) = start.take() {
            out.push((begin, end));
        }
    }
    if let Some(begin) = start {
        out.push((begin, end));
    }
    out
}

/// B 是否有片段跨過空白（把空白兩側的同語言內容併成一段）。
fn merges_across_space(input: &str, b_spans: &[Span]) -> bool {
    b_spans
        .iter()
        .any(|span| span.text(input).chars().any(char::is_whitespace))
}

/// 兩邊是否在同一段上分別判成 `left` 與 `right`。
fn lang_conflict(a_spans: &[Span], b_spans: &[Span], left: Lang, right: Lang) -> bool {
    a_spans.iter().any(|a| {
        b_spans.iter().any(|b| {
            a.overlaps(b)
                && ((a.lang == left && b.lang == right) || (a.lang == right && b.lang == left))
        })
    })
}

/// B 是否在英數 run 中間插入非空白的符號片段。
fn has_mid_symbol(input: &str, b_spans: &[Span]) -> bool {
    b_spans.iter().any(|span| {
        if span.lang != Lang::Sym {
            return false;
        }
        let text = span.text(input);
        if text.is_empty() || text.chars().all(char::is_whitespace) {
            return false;
        }
        // 符號片段可能把後面的空白一起吃掉（例如 `": "`）；判斷 run 邊界時
        // 只看非空白的部分。
        let start = span.start + (text.len() - text.trim_start().len());
        let end = span.end - (text.len() - text.trim_end().len());
        let previous = input
            .get(..start)
            .and_then(|left| left.chars().next_back())
            .map(is_run_char);
        let next = input
            .get(end..)
            .and_then(|right| right.chars().next())
            .map(is_run_char);
        previous == Some(true) && next == Some(true)
    })
}

/// 英數 run 的字元判定，與 `mixed::is_token_rune` 相同。
fn is_token_rune(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || jc_core::mixed::is_fullwidth_alnum(ch)
        || "._~:/?#[]@!$&'()*+,;=%\\-|{}".contains(ch)
}

/// run 內的字母或數字，用來判斷符號是否夾在 run 中間。
fn is_run_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || jc_core::mixed::is_fullwidth_alnum(ch)
}

// ---------------------------------------------------------------------------
// 統計與輸出
// ---------------------------------------------------------------------------

/// 一桶的統計。
struct BucketStat {
    bucket: Bucket,
    count: usize,
    examples: Vec<usize>,
}

/// 整次量測的摘要。
struct Summary {
    seed: u64,
    requested: usize,
    generated: usize,
    unique: usize,
    agree: usize,
    disagree: usize,
    a_none: usize,
    a_some: usize,
    both_agree: usize,
    b_error: usize,
    buckets: Vec<BucketStat>,
}

impl Summary {
    fn new(cases: &[Case], a_none: usize, opts: &DiffOptions) -> Self {
        let disagree = cases.iter().filter(|case| case.bucket().is_some()).count();
        let agree = cases.len() - disagree;
        let b_error = cases
            .iter()
            .filter(|case| matches!(case.b, BOutcome::Err(_)))
            .count();
        let unique = cases
            .iter()
            .map(|case| case.input.as_str())
            .collect::<HashSet<_>>()
            .len();
        let a_some = cases.iter().filter(|case| case.a.is_some()).count();
        let both_agree = cases
            .iter()
            .filter(|case| case.a.is_some() && case.bucket().is_none())
            .count();
        let mut counts: HashMap<Bucket, usize> = HashMap::new();
        let mut by_bucket: HashMap<Bucket, Vec<usize>> = HashMap::new();
        for (index, case) in cases.iter().enumerate() {
            if let Some(bucket) = case.bucket() {
                *counts.entry(bucket).or_default() += 1;
                by_bucket.entry(bucket).or_default().push(index);
            }
        }
        let mut buckets = Vec::new();
        for bucket in Bucket::ALL {
            let mut members = by_bucket.remove(&bucket).unwrap_or_default();
            members.sort_by(|&left, &right| {
                let left_case = &cases[left];
                let right_case = &cases[right];
                left_case
                    .input
                    .chars()
                    .count()
                    .cmp(&right_case.input.chars().count())
                    .then_with(|| left_case.input.cmp(&right_case.input))
            });
            members.truncate(opts.top);
            buckets.push(BucketStat {
                bucket,
                count: counts.get(&bucket).copied().unwrap_or(0),
                examples: members,
            });
        }
        Self {
            seed: opts.seed,
            requested: opts.count,
            generated: cases.len(),
            unique,
            agree,
            disagree,
            a_none,
            a_some,
            both_agree,
            b_error,
            buckets,
        }
    }

    fn ratio(&self, count: usize) -> f64 {
        if self.disagree == 0 {
            0.0
        } else {
            count as f64 / self.disagree as f64
        }
    }

    fn write_human(&self, out: &mut dyn Write) -> Result<(), CliError> {
        writeln!(
            out,
            "mixed-diff：seed {}、要求 {}、實際 {}（唯一 {}）",
            self.seed, self.requested, self.generated, self.unique
        )?;
        writeln!(
            out,
            "一致 {}、不一致 {}（{:.1}%）",
            self.agree,
            self.disagree,
            self.ratio(self.disagree) * 100.0
        )?;
        writeln!(
            out,
            "A 無法解析 {}、A 有結果 {}（其中一致 {}、不一致 {}）、B 轉換失敗 {}",
            self.a_none,
            self.a_some,
            self.both_agree,
            self.a_some - self.both_agree,
            self.b_error
        )?;
        writeln!(out, "分類（主分類，依優先序）：")?;
        for stat in &self.buckets {
            writeln!(
                out,
                "  {:<28} {:>7}  {:>6.2}%",
                stat.bucket.name(),
                stat.count,
                self.ratio(stat.count) * 100.0
            )?;
        }
        Ok(())
    }

    fn json(&self, cases: &[Case]) -> serde_json::Value {
        let buckets: Vec<serde_json::Value> = self
            .buckets
            .iter()
            .map(|stat| {
                serde_json::json!({
                    "name": stat.bucket.name(),
                    "count": stat.count,
                    "ratio": self.ratio(stat.count),
                    "examples": stat
                        .examples
                        .iter()
                        .map(|&index| case_json(&cases[index]))
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        let mismatch: Vec<serde_json::Value> = cases
            .iter()
            .filter(|case| case.bucket().is_some())
            .map(case_json)
            .collect();
        serde_json::json!({
            "suite": "mixed-diff",
            "seed": self.seed,
            "count": self.requested,
            "generated": self.generated,
            "unique": self.unique,
            "agree": self.agree,
            "disagree": self.disagree,
            "a_none": self.a_none,
            "a_some": self.a_some,
            "both_agree": self.both_agree,
            "both_disagree": self.a_some - self.both_agree,
            "b_error": self.b_error,
            "buckets": buckets,
            "cases": mismatch,
        })
    }

    fn cases_json(&self, cases: &[Case]) -> serde_json::Value {
        let mismatch: Vec<serde_json::Value> = cases
            .iter()
            .filter(|case| case.bucket().is_some())
            .map(case_json)
            .collect();
        serde_json::json!({
            "suite": "mixed-diff-cases",
            "seed": self.seed,
            "count": self.requested,
            "generated": self.generated,
            "unique": self.unique,
            "disagree": self.disagree,
            "cases": mismatch,
        })
    }
}

fn case_json(case: &Case) -> serde_json::Value {
    let a = case.a.as_ref().map(|split| split_json(&case.input, split));
    let b = match &case.b {
        BOutcome::Ok(result) => b_json(&case.input, result),
        BOutcome::Err(err) => serde_json::json!({ "error": err }),
    };
    serde_json::json!({
        "input": case.input,
        "bucket": case.bucket().map(Bucket::name),
        "signals": case.signals.iter().map(|bucket| bucket.name()).collect::<Vec<_>>(),
        "a": a,
        "b": b,
    })
}

fn split_json(input: &str, split: &Split) -> serde_json::Value {
    let pieces: Vec<serde_json::Value> = split
        .pieces
        .iter()
        .map(|piece| {
            serde_json::json!({
                "start": piece.start,
                "end": piece.end,
                "lang": piece.lang.as_str(),
                "reading": piece.reading,
                "text": piece.text(input),
            })
        })
        .collect();
    serde_json::json!({ "score": split.score, "pieces": pieces })
}

fn b_json(input: &str, result: &MixedResult) -> serde_json::Value {
    let segments: Vec<serde_json::Value> = result
        .segments
        .iter()
        .map(|segment| {
            serde_json::json!({
                "mode": mode_name(segment.mode),
                "script": segment.script.as_str(),
                "input_start": segment.input_start,
                "input_end": segment.input_end,
                "input": input.get(segment.input_start..segment.input_end).unwrap_or_default(),
                "surface": segment.surface,
            })
        })
        .collect();
    serde_json::json!({
        "surface": result.surface,
        "cost": result.cost,
        "segments": segments,
    })
}

fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Zh => "zh",
        Mode::Ja => "ja",
        Mode::En => "en",
        Mode::Mixed => "mixed",
    }
}

/// 人類可讀的單行摘要。
fn describe(case: &Case) -> String {
    let a = match &case.a {
        Some(split) => split
            .pieces
            .iter()
            .map(|piece| {
                format!(
                    "{} {}..{} {:?} → {:?}",
                    piece.lang.as_str(),
                    piece.start,
                    piece.end,
                    piece.text(&case.input),
                    piece.reading
                )
            })
            .collect::<Vec<_>>()
            .join("、"),
        None => "無法解析".to_string(),
    };
    let b = match &case.b {
        BOutcome::Ok(result) => result
            .segments
            .iter()
            .map(|segment| {
                format!(
                    "{}/{} {}..{} {:?} → {:?}",
                    mode_name(segment.mode),
                    segment.script.as_str(),
                    segment.input_start,
                    segment.input_end,
                    case.input
                        .get(segment.input_start..segment.input_end)
                        .unwrap_or_default(),
                    segment.surface
                )
            })
            .collect::<Vec<_>>()
            .join("、"),
        BOutcome::Err(err) => format!("錯誤 {err}"),
    };
    format!("A：{a}\n      B：{b}")
}

/// 把代表案例寫成多行輸出；供報告與人工檢視使用。
fn write_examples(cases: &[Case], summary: &Summary, out: &mut dyn Write) -> Result<(), CliError> {
    for stat in &summary.buckets {
        if stat.examples.is_empty() {
            continue;
        }
        writeln!(out, "{}：", stat.bucket.name())?;
        for &index in &stat.examples {
            let case = &cases[index];
            writeln!(out, "  {:?}", case.input)?;
            writeln!(out, "      {}", describe(case))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_pools() -> Pools {
        Pools {
            zh: vec!["vul3".to_string(), "ej/ ".to_string()],
            ja: vec!["sa".to_string(), "kka".to_string()],
            en: vec!["small".to_string()],
            short_en: vec!["a".to_string()],
            atom: vec!["5k42".to_string()],
            punct_mid: vec!["hi!there".to_string()],
            punct_tail: vec!["hello.".to_string()],
        }
    }

    #[test]
    fn test_generator_is_deterministic() {
        let pool = tiny_pools();
        let first = generate(&pool, 7, 50);
        let second = generate(&pool, 7, 50);
        assert_eq!(first, second);
        assert_eq!(first.len(), 50);
        assert!(first
            .iter()
            .all(|input| (3..=40).contains(&input.chars().count())));
    }

    #[test]
    fn test_different_seed_changes_sequence() {
        let pool = tiny_pools();
        assert_ne!(generate(&pool, 1, 20), generate(&pool, 2, 20));
    }

    #[test]
    fn test_zh_fragments_cover_trailing_finals() {
        let fragments = zh_fragments();
        for suffix in [".", ",", ";", "/", "-"] {
            assert!(
                fragments.iter().any(|fragment| fragment.ends_with(suffix)),
                "缺少以 {suffix:?} 結尾的注音片段"
            );
        }
        assert!(fragments.iter().any(|fragment| fragment.ends_with(' ')));
    }

    #[test]
    fn test_ja_fragments_cover_sokuon_and_n() {
        let fragments = ja_fragments();
        assert!(fragments.iter().any(|fragment| fragment == "kka"));
        assert!(fragments.iter().any(|fragment| fragment == "n"));
        assert!(fragments.iter().any(|fragment| fragment == "nn"));
    }

    #[test]
    fn test_bucket_names_are_stable() {
        assert_eq!(
            Bucket::TrailingPunctStripped.name(),
            "trailing-punct-stripped"
        );
        assert_eq!(
            Bucket::SpaceToneVsSeparator.name(),
            "space-tone-vs-separator"
        );
        assert_eq!(Bucket::ANone.name(), "a-none");
    }
}
