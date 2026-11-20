//! 拉丁字串的雙語言切分器（研究原型，預設關閉）。
//!
//! 同一段拉丁字元可能同時含日文羅馬字與注音按鍵，例如
//! `rishireej/ savul3` 應該是 `rishire`（羅馬字）＋`ej/`（注音）＋`sa`
//! （羅馬字）＋`vul3`（注音），最後轉成 `りしれ供さ小`。現有的路由只會
//! 把整段連續英數當成一種語言，因此需要一個在字母層級切分的工具。
//!
//! 演算法是動態規劃：狀態 `(位置, 目前語言)`，轉移是「吃一個完整音節」。
//! 分數由音節本身的合法度、詞典存在性與切換語言的成本組成。空白是硬邊界，
//! 不屬於任何片段。
//!
//! [`LatinSplitter`] 支援逐鍵增量：往前追加字元時，位置 `<= 舊長度` 的最佳
//! 分數與回溯指標都不會改變，因此每次按鍵只計算新尾端那一格。
//!
//! 這個模組目前是研究原型，`mixed::Config::mixed_latin_split` 預設為
//! `false`，關閉時完全不會被呼叫。

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::ja::input::{can_sokuon, default_romaji_table};
use crate::lang::Mode;
use crate::lattice::Lexicon;
use crate::zh::keyboard::Keyboard;
use crate::zh::pinyin::is_legal_base;
use crate::zh::syllable::{class_of, Class};

/// 單一音節在輸入裡的字元數上限。
///
/// 最長的羅馬字按鍵是 `xtsu`／`ltsu`，加上促音前綴也不會超過 5 個字元；
/// 注音音節是聲母＋介音＋韻母＋聲調，最多 4 個。取 8 保留餘裕。
const MAX_SYLLABLE_CHARS: usize = 8;

/// 片段語言。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SplitLang {
    /// 日文羅馬字，讀音是かな。
    Ja,
    /// 注音按鍵，讀音是注音符號字串。
    Zh,
    /// 英文詞，照原樣輸出。
    En,
}

impl SplitLang {
    /// 對應的輸入模式。
    pub fn mode(self) -> Mode {
        match self {
            Self::Ja => Mode::Ja,
            Self::Zh => Mode::Zh,
            Self::En => Mode::En,
        }
    }

    /// 穩定的名稱，可寫進報告與測試資料。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ja => "ja",
            Self::Zh => "zh",
            Self::En => "en",
        }
    }
}

/// 一個切分片段。
///
/// `start`、`end` 是**輸入**的 UTF-8 位元組位移；空白不屬於任何片段，
/// 因此片段之間可能不連續。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    /// 起點位元組位移（含）。
    pub start: usize,
    /// 終點位元組位移（不含）。
    pub end: usize,
    /// 這段是哪一種語言。
    pub lang: SplitLang,
    /// 讀音：日文是かな，注音是注音符號字串（含聲調時帶調號）。
    pub reading: String,
}

impl Piece {
    /// 這段在輸入裡的字面。
    pub fn text<'a>(&self, input: &'a str) -> &'a str {
        input.get(self.start..self.end).unwrap_or_default()
    }
}

/// 一次切分結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Split {
    /// 依輸入順序排列的片段。
    pub pieces: Vec<Piece>,
    /// 整體分數，越大越優先。
    pub score: i32,
}

impl Split {
    /// 片段數。
    pub fn len(&self) -> usize {
        self.pieces.len()
    }

    /// 是否沒有片段。
    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    /// 依序回報每段的語言。
    pub fn languages(&self) -> Vec<SplitLang> {
        self.pieces.iter().map(|p| p.lang).collect()
    }
}

/// 切分評分需要的詞典查詢。
///
/// 全部都有預設值，沒有詞典時只靠文法計分。查詢失敗或沒有資料時回傳
/// `false`，切分器仍可運作。
pub trait SplitLexicon {
    /// 注音讀音是否有完整詞條（有序）。
    fn zh_known(&self, _reading: &str) -> bool {
        false
    }

    /// 注音讀音是否為某個詞條的開頭（無序）。
    fn zh_prefix(&self, _reading: &str) -> bool {
        false
    }

    /// かな讀音是否有詞條。
    fn ja_known(&self, _kana: &str) -> bool {
        false
    }

    /// 是否是英文詞表認得的單字；只有長度 3 以上才會被當成照字面的片段。
    fn english(&self, _text: &str) -> bool {
        false
    }

    /// 英文詞的權重；越大代表越常見。沒有英文詞表時回傳 0。
    fn english_weight(&self, text: &str) -> f64 {
        if self.english(text) {
            1.0
        } else {
            0.0
        }
    }
}

/// 沒有詞典的查詢層。
#[derive(Debug, Clone, Copy, Default)]
pub struct NoLexicon;

impl SplitLexicon for NoLexicon {}

/// 把注音 [`Lexicon`] 接到切分器的查詢層。
///
/// 沒有聲調的讀音會依序試五個聲調；詞庫的鍵都帶聲調，這樣 `ㄍㄨㄥ` 才能
/// 命中 `供`、`公` 這些詞條。
pub struct ZhLexiconAdapter<'a>(pub &'a dyn Lexicon);

impl SplitLexicon for ZhLexiconAdapter<'_> {
    fn zh_known(&self, reading: &str) -> bool {
        if reading.is_empty() {
            return false;
        }
        if !self.0.lookup(reading).is_empty() {
            return true;
        }
        match reading.chars().last().map(class_of) {
            Some(Class::Tone) | None => false,
            _ => ["", "ˊ", "ˇ", "ˋ", "˙"]
                .into_iter()
                .any(|tone| !self.0.lookup(&format!("{reading}{tone}")).is_empty()),
        }
    }

    fn zh_prefix(&self, reading: &str) -> bool {
        !reading.is_empty() && !self.0.prefix(reading).is_empty()
    }
}

/// 同時提供注音詞典與英文詞表的查詢層。
///
/// 切分器只把英文詞當成「照字面的完整單字」，不會做前綴補完；長度 1–2 的
/// 英文不列入，避免 `s`、`a` 這種單字母把正常的羅馬字切壞。
pub struct LexiconAdapter<'a> {
    /// 注音詞典。
    pub zh: Option<&'a dyn Lexicon>,
    /// 英文查詢層；`None` 時不辨識英文詞。
    pub english: Option<&'a crate::mixed::English>,
}

impl SplitLexicon for LexiconAdapter<'_> {
    fn zh_known(&self, reading: &str) -> bool {
        self.zh
            .is_some_and(|lexicon| ZhLexiconAdapter(lexicon).zh_known(reading))
    }

    fn zh_prefix(&self, reading: &str) -> bool {
        self.zh
            .is_some_and(|lexicon| ZhLexiconAdapter(lexicon).zh_prefix(reading))
    }

    fn english(&self, text: &str) -> bool {
        text.chars().count() >= 3
            && text.chars().all(|ch| ch.is_ascii_lowercase())
            && self.english.is_some_and(|lookup| lookup.has(text))
    }

    fn english_weight(&self, text: &str) -> f64 {
        if !self.english(text) {
            return 0.0;
        }
        self.english
            .and_then(|lookup| lookup.weight(text))
            .unwrap_or(0.0)
    }
}

/// 切分評分參數。
///
/// 分數越大越優先。預設值讓「整段都能完整解析」的切分勝出，其次才用
/// 詞典與語言切換成本微調；分數相同時偏好日文，與現有路由的保守預設一致。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    /// 切換語言的成本（負值）。
    pub switch_cost: i32,
    /// 每個音節的成本（負值），避免把同一段拆得太碎。
    pub piece_cost: i32,
    /// 注音音節帶聲調鍵的加分。
    pub zh_toned_bonus: i32,
    /// 注音音節沒有聲調的扣分。
    pub zh_untoned_cost: i32,
    /// 注音音節是完整詞條的加分。
    pub zh_word_bonus: i32,
    /// 注音音節是詞條開頭的加分。
    pub zh_prefix_bonus: i32,
    /// かな是日文詞條的加分。
    pub ja_word_bonus: i32,
    /// 英文詞照字面的成本（負值）。
    pub en_word_cost: i32,
    /// 英文詞達到這個權重時視為常見英文，再加 `en_common_bonus`。
    pub en_common_weight: f64,
    /// 常見英文詞的加分；讓 `code` 留在英文，`kanji` 這類低頻借詞走日文。
    pub en_common_bonus: i32,
    /// 分數相同時偏好日文。
    pub prefer_ja: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            switch_cost: -3,
            piece_cost: -2,
            zh_toned_bonus: 2,
            zh_untoned_cost: 2,
            zh_word_bonus: 3,
            zh_prefix_bonus: 1,
            ja_word_bonus: 4,
            en_word_cost: -6,
            en_common_weight: 100.0,
            en_common_bonus: 4,
            prefer_ja: true,
        }
    }
}

// ---------------------------------------------------------------------------
// 文法
// ---------------------------------------------------------------------------

/// 羅馬字音節表：完整的按鍵序列與對應かな。
struct JaGrammar {
    entries: HashMap<String, String>,
}

impl JaGrammar {
    fn lookup(&self, keys: &[char]) -> Option<&str> {
        let mut text = String::with_capacity(keys.len());
        text.extend(keys.iter());
        self.entries.get(&text).map(String::as_str)
    }
}

fn ja_grammar() -> &'static JaGrammar {
    static GRAMMAR: OnceLock<JaGrammar> = OnceLock::new();
    GRAMMAR.get_or_init(|| {
        let mut entries: HashMap<String, String> = HashMap::new();
        for (keys, kana) in default_romaji_table().entries() {
            if keys.is_empty() || !keys.is_ascii() {
                continue;
            }
            entries
                .entry(keys.to_string())
                .or_insert_with(|| kana.to_string());
        }
        // 單獨的 `n` 與 `nn` 都是 `ん`。
        entries
            .entry("n".to_string())
            .or_insert_with(|| "ん".to_string());
        entries
            .entry("nn".to_string())
            .or_insert_with(|| "ん".to_string());
        // 促音：相同子音連打時，前一個子音單獨成為 `っ`。以「促音＋かな」
        // 當成一個完整音節，才不會在 `kka` 中間留下無法解釋的 `k`。
        let base: Vec<(String, String)> = entries
            .iter()
            .map(|(keys, kana)| (keys.clone(), kana.clone()))
            .collect();
        for (keys, kana) in base {
            let Some(first) = keys.chars().next() else {
                continue;
            };
            if keys.chars().count() < 2 || !can_sokuon(first) {
                continue;
            }
            entries
                .entry(format!("{first}{keys}"))
                .or_insert_with(|| format!("っ{kana}"));
        }
        JaGrammar { entries }
    })
}

/// 注音音節表：把實體按鍵序列解析成音節。
struct ZhGrammar {
    keyboard: Keyboard,
}

impl ZhGrammar {
    /// 解析 `keys` 為一個完整注音音節。
    ///
    /// 回傳（讀音, 是否帶聲調）。按鍵必須照聲母、介音、韻母、聲調的順序，
    /// 每一類最多一個；空白不是聲調鍵，切分器把它當成段與段之間的邊界。
    fn parse(&self, keys: &[char]) -> Option<(String, bool)> {
        if keys.is_empty() {
            return None;
        }
        let mut base = String::new();
        let mut tone: Option<char> = None;
        let mut last = Class::None;
        for &key in keys {
            if key == ' ' {
                return None;
            }
            let symbol = self.keyboard.symbol(key)?;
            let class = class_of(symbol);
            if class == Class::None || class <= last {
                return None;
            }
            if class == Class::Tone {
                tone = Some(symbol);
            } else {
                base.push(symbol);
            }
            last = class;
        }
        if base.is_empty() || !is_legal_base(&base) {
            return None;
        }
        let reading = match tone {
            Some(tone) => format!("{base}{tone}"),
            None => base,
        };
        Some((reading, tone.is_some()))
    }
}

fn zh_grammar() -> &'static ZhGrammar {
    static GRAMMAR: OnceLock<ZhGrammar> = OnceLock::new();
    GRAMMAR.get_or_init(|| ZhGrammar {
        keyboard: Keyboard::standard(),
    })
}

// ---------------------------------------------------------------------------
// 動態規劃
// ---------------------------------------------------------------------------

/// 到達某個位置、某種語言的最佳解。
#[derive(Debug, Clone, Copy)]
struct Cell {
    score: i32,
    pieces: u16,
    /// 前一個位置（字元索引）。
    prev_pos: u32,
    /// 前一個語言：0 是日文，1 是注音。
    prev_lang: u8,
}

const JA: usize = 0;
const ZH: usize = 1;
const EN: usize = 2;
const LANGS: usize = 3;
/// 英文詞在一個片段裡的最大字元數。
const MAX_ENGLISH_CHARS: usize = 48;

/// 分數相同時的語言偏好；日文優先，其次注音，最後英文。
fn lang_rank(lang: usize) -> u8 {
    match lang {
        JA => 0,
        ZH => 1,
        _ => 2,
    }
}

/// 依 `Options` 與詞典替一個音節打分。
fn syllable_score(
    lang: usize,
    reading: &str,
    toned: bool,
    opts: &Options,
    lexicon: &dyn SplitLexicon,
) -> i32 {
    let mut score = opts.piece_cost;
    match lang {
        JA => {
            if lexicon.ja_known(reading) {
                score += opts.ja_word_bonus;
            }
        }
        _ => {
            if toned {
                score += opts.zh_toned_bonus;
                if lexicon.zh_known(reading) {
                    score += opts.zh_word_bonus;
                } else if lexicon.zh_prefix(reading) {
                    score += opts.zh_prefix_bonus;
                }
            } else {
                // 沒有聲調的讀音只是猜測，不給詞典加分，避免把一段字母拆成
                // 多個「剛好有字」的無調音節。
                score -= opts.zh_untoned_cost;
            }
        }
    }
    score
}

/// 候選是否比目前最佳解更好。
///
/// 先比總分，再比片段數，最後依 [`Options::prefer_ja`] 決定分數相同時要
/// 選哪一條前驅路徑。
fn better(score: i32, pieces: u16, prev_lang: usize, best: Option<Cell>, opts: &Options) -> bool {
    let Some(best) = best else {
        return true;
    };
    if score != best.score {
        return score > best.score;
    }
    if pieces != best.pieces {
        return pieces < best.pieces;
    }
    let prefer = if opts.prefer_ja { JA } else { ZH };
    prev_lang == prefer && best.prev_lang as usize != prefer
}

/// 計算位置 `j` 的最佳解；只讀 `j` 以前的格，因此可以逐鍵增量。
fn compute_cell(
    cells: &mut [[Option<Cell>; LANGS]],
    chars: &[char],
    j: usize,
    opts: &Options,
    lexicon: &dyn SplitLexicon,
) {
    let mut best: [Option<Cell>; LANGS] = [None, None, None];
    let low = j.saturating_sub(MAX_SYLLABLE_CHARS);
    for start in low..j {
        let keys = &chars[start..j];
        if let Some(kana) = ja_grammar().lookup(keys) {
            let piece = syllable_score(JA, kana, true, opts, lexicon);
            consider(&mut best, cells, start, JA, piece, opts);
        }
        if let Some((reading, toned)) = zh_grammar().parse(keys) {
            let piece = syllable_score(ZH, &reading, toned, opts, lexicon);
            consider(&mut best, cells, start, ZH, piece, opts);
        }
    }
    // 英文詞：整串照字面。只接受 3 個字元以上、全小寫的詞表單字，
    // 單字母與未知字串不當英文，避免把羅馬字切壞。
    let low_en = j.saturating_sub(MAX_ENGLISH_CHARS);
    for start in (low_en..j).rev() {
        if !chars[start].is_ascii_lowercase() {
            break;
        }
        let word: String = chars[start..j].iter().collect();
        if word.chars().count() < 3 || !lexicon.english(&word) {
            continue;
        }
        let mut piece = opts.en_word_cost;
        if lexicon.english_weight(&word) >= opts.en_common_weight {
            piece += opts.en_common_bonus;
        }
        consider(&mut best, cells, start, EN, piece, opts);
    }
    if chars[j - 1].is_whitespace() {
        // 停頓：空白不產生片段，語言不變。日文段尾的空白就是走這條。
        for lang in [JA, ZH, EN] {
            if let Some(prev) = cells[j - 1][lang] {
                let pieces = prev.pieces;
                let cell = Cell {
                    score: prev.score,
                    pieces,
                    prev_pos: (j - 1) as u32,
                    prev_lang: lang as u8,
                };
                if better(cell.score, pieces, lang, best[lang], opts) {
                    best[lang] = Some(cell);
                }
            }
        }
        // 一聲：注音音節已經完整時，後面的空白屬於該音節，不是分隔符。
        // 連續空白只讓最後一個當一聲。
        let next_is_space = chars.get(j).is_some_and(|ch| ch.is_whitespace());
        if !next_is_space {
            let low = (j - 1).saturating_sub(MAX_SYLLABLE_CHARS);
            for start in low..j - 1 {
                let Some((reading, false)) = zh_grammar().parse(&chars[start..j - 1]) else {
                    continue;
                };
                let piece = syllable_score(ZH, &reading, true, opts, lexicon);
                consider(&mut best, cells, start, ZH, piece, opts);
            }
        }
    }
    cells[j] = best;
}

/// 把「從 `start` 開始、語言 `lang`」的候選併進 `best[lang]`。
fn consider(
    best: &mut [Option<Cell>; LANGS],
    cells: &[[Option<Cell>; LANGS]],
    start: usize,
    lang: usize,
    piece_score: i32,
    opts: &Options,
) {
    for prev_lang in [JA, ZH, EN] {
        let Some(prev) = cells[start][prev_lang] else {
            continue;
        };
        let mut score = prev.score + piece_score;
        if prev_lang != lang {
            score += opts.switch_cost;
        }
        let pieces = prev.pieces.saturating_add(1);
        if better(
            score,
            pieces,
            prev_lang,
            best.get(lang).copied().flatten(),
            opts,
        ) {
            best[lang] = Some(Cell {
                score,
                pieces,
                prev_pos: start as u32,
                prev_lang: prev_lang as u8,
            });
        }
    }
}

/// 一個片段在指定語言下的讀音。
///
/// 注音片段結尾的空白是一聲，解析時要去掉；日文不吃空白。
fn piece_reading(chars: &[char], lang: usize) -> Option<String> {
    match lang {
        JA => ja_grammar().lookup(chars).map(str::to_string),
        EN => chars
            .iter()
            .all(|ch| ch.is_ascii_lowercase())
            .then(|| chars.iter().collect()),
        _ => {
            let keys = match chars.last() {
                Some(ch) if ch.is_whitespace() => &chars[..chars.len() - 1],
                _ => chars,
            };
            zh_grammar().parse(keys).map(|(reading, _)| reading)
        }
    }
}

/// 由位置格重建片段；`base` 是這段在原始輸入的位元組位移。
fn reconstruct(
    chars: &[char],
    offsets: &[usize],
    cells: &[[Option<Cell>; LANGS]],
    end: usize,
    lang: usize,
    base: usize,
) -> Option<Vec<Piece>> {
    if end == 0 {
        return Some(Vec::new());
    }
    let mut pieces = Vec::new();
    let mut pos = end;
    let mut current = lang;
    while pos > 0 {
        let cell = cells.get(pos).and_then(|row| row[current])?;
        let start = cell.prev_pos as usize;
        if start >= pos {
            return None;
        }
        // 停頓的空白：不成片段，語言不變。
        if start + 1 == pos && chars[start].is_whitespace() {
            pos = start;
            current = cell.prev_lang as usize;
            continue;
        }
        let reading = piece_reading(&chars[start..pos], current)?;
        pieces.push(Piece {
            start: base + offsets[start],
            end: base + offsets[pos],
            lang: match current {
                JA => SplitLang::Ja,
                ZH => SplitLang::Zh,
                _ => SplitLang::En,
            },
            reading,
        });
        pos = start;
        current = cell.prev_lang as usize;
    }
    if pos != 0 {
        return None;
    }
    pieces.reverse();
    Some(pieces)
}

/// 合併相鄰且同語言的片段，讓輸出對應「一個語言片段」而不是單一音節。
///
/// 只合併輸入上連續的片段；空白隔開的兩段不會被合併。注音讀音以空白
/// 分隔音節，日文讀音直接串接。
fn merge_pieces(pieces: Vec<Piece>) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::with_capacity(pieces.len());
    for piece in pieces {
        match out.last_mut() {
            Some(last) if last.lang == piece.lang && last.end == piece.start => {
                last.end = piece.end;
                if piece.lang == SplitLang::Zh && !last.reading.is_empty() {
                    last.reading.push(' ');
                }
                last.reading.push_str(&piece.reading);
            }
            _ => out.push(piece),
        }
    }
    out
}

/// 把切分器產生的注音讀音補成詞庫查得到的聲調。
///
/// 切分器只看到按鍵，沒有聲調鍵的音節沒有調號。詞庫的鍵都帶聲調，因此
/// 依序試一聲、二聲、三聲、四聲與輕聲，取第一個有詞條的讀音；都沒有時
/// 保留原字串，交給解碼器當未知詞處理。
pub fn toned_syllables(reading: &str, lexicon: &dyn Lexicon) -> Vec<String> {
    reading
        .split_whitespace()
        .map(|syllable| {
            if matches!(syllable.chars().last().map(class_of), Some(Class::Tone)) {
                return syllable.to_string();
            }
            if !lexicon.lookup(syllable).is_empty() {
                return syllable.to_string();
            }
            for tone in ["ˊ", "ˇ", "ˋ", "˙"] {
                let toned = format!("{syllable}{tone}");
                if !lexicon.lookup(&toned).is_empty() {
                    return toned;
                }
            }
            syllable.to_string()
        })
        .collect()
}

/// 整個輸入的動態規劃格。
///
/// 空白不是區段分隔：注音音節後面接空白時，空白是那個音節的一聲；其他
/// 情況空白只當停頓，不產生片段。
#[derive(Debug, Clone)]
struct Dp {
    chars: Vec<char>,
    /// 每個字元在原始輸入的位元組位移，長度是 `chars.len() + 1`。
    offsets: Vec<usize>,
    cells: Vec<[Option<Cell>; LANGS]>,
}

impl Dp {
    fn new() -> Self {
        // 起點：兩種語言都是 0 分，第一段不付切換成本。
        let start = Cell {
            score: 0,
            pieces: 0,
            prev_pos: 0,
            prev_lang: JA as u8,
        };
        Self {
            chars: Vec::new(),
            offsets: vec![0],
            cells: vec![[Some(start), Some(start), Some(start)]],
        }
    }

    fn push(&mut self, ch: char, opts: &Options, lexicon: &dyn SplitLexicon) {
        let byte = *self.offsets.last().unwrap_or(&0);
        self.chars.push(ch);
        self.offsets.push(byte + ch.len_utf8());
        self.cells.push([None, None, None]);
        let j = self.chars.len();
        compute_cell(&mut self.cells, &self.chars, j, opts, lexicon);
    }

    /// 結尾最好的一格；分數相同時依 [`lang_rank`] 決定。
    fn best_end(&self) -> Option<(usize, i32)> {
        let pos = self.chars.len();
        let row = self.cells.get(pos)?;
        let mut best: Option<(usize, i32)> = None;
        for (lang, cell) in row.iter().enumerate() {
            let Some(cell) = cell else {
                continue;
            };
            let better = match best {
                None => true,
                Some((best_lang, best_score)) => {
                    cell.score > best_score
                        || (cell.score == best_score && lang_rank(lang) < lang_rank(best_lang))
                }
            };
            if better {
                best = Some((lang, cell.score));
            }
        }
        best
    }

    /// 能完整解析時回傳片段。
    fn split(&self) -> Option<(Vec<Piece>, i32)> {
        let (lang, score) = self.best_end()?;
        let pieces = reconstruct(
            &self.chars,
            &self.offsets,
            &self.cells,
            self.chars.len(),
            lang,
            0,
        )?;
        Some((pieces, score))
    }

    /// 取能解析的最長前綴，剩下的尾巴另行回傳。
    fn preview(&self) -> (Vec<Piece>, i32, String) {
        let mut best: Option<(usize, usize, i32)> = None;
        for pos in (1..=self.chars.len()).rev() {
            let row = &self.cells[pos];
            for lang in [JA, ZH, EN] {
                let Some(cell) = row[lang] else {
                    continue;
                };
                let better = match best {
                    None => true,
                    Some((old_pos, old_lang, old_score)) => {
                        pos > old_pos
                            || (pos == old_pos
                                && (cell.score > old_score
                                    || (cell.score == old_score
                                        && lang_rank(lang) < lang_rank(old_lang))))
                    }
                };
                if better {
                    best = Some((pos, lang, cell.score));
                }
            }
            if best.is_some() {
                break;
            }
        }
        let Some((pos, lang, score)) = best else {
            let tail: String = self.chars.iter().collect();
            return (Vec::new(), 0, tail);
        };
        let pieces =
            reconstruct(&self.chars, &self.offsets, &self.cells, pos, lang, 0).unwrap_or_default();
        let tail: String = self.chars[pos.min(self.chars.len())..].iter().collect();
        (pieces, score, tail)
    }
}

/// 嚴格切分：整段輸入都必須完整解析，否則回傳 `None`。
///
/// 空白在注音音節結尾是一聲，會包含在該片段裡；其他空白是停頓，不產生
/// 片段。回傳的位移是 `text` 的位元組位移。
pub fn split(text: &str, opts: &Options, lexicon: &dyn SplitLexicon) -> Option<Split> {
    let mut splitter = LatinSplitter::new(*opts);
    splitter.set_text(text, lexicon);
    splitter.split()
}

/// 容錯切分：取最長可解析前綴，尾巴不列入片段。
///
/// 給逐鍵顯示用；正式轉換請用 [`split`]，語意才不會把打一半的按鍵當成
/// 已完成的片段。
pub fn split_preview(text: &str, opts: &Options, lexicon: &dyn SplitLexicon) -> Split {
    let mut splitter = LatinSplitter::new(*opts);
    splitter.set_text(text, lexicon);
    splitter.preview().0
}

/// 可逐鍵增量的切分器。
///
/// 追加字元時只計算新尾端的位置格；只有內容被改寫（退格、換字）才重建。
#[derive(Debug, Clone)]
pub struct LatinSplitter {
    text: String,
    dp: Dp,
    opts: Options,
}

impl LatinSplitter {
    /// 建立切分器。
    pub fn new(opts: Options) -> Self {
        Self {
            text: String::new(),
            dp: Dp::new(),
            opts,
        }
    }

    /// 清空。
    pub fn reset(&mut self) {
        self.text.clear();
        self.dp = Dp::new();
    }

    /// 目前的輸入。
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 把內容同步到 `text`；回傳是否有變動。
    ///
    /// 若 `text` 只是目前內容的延伸，只把新增的字元餵進尾端；其餘情況
    /// 重建。這是逐鍵路徑的主要入口。
    pub fn set_text(&mut self, text: &str, lexicon: &dyn SplitLexicon) -> bool {
        if text == self.text {
            return false;
        }
        if let Some(rest) = text.strip_prefix(self.text.as_str()) {
            let mut rest = rest.to_string();
            for ch in rest.drain(..) {
                self.push(ch, lexicon);
            }
            return true;
        }
        self.reset();
        for ch in text.chars() {
            self.push(ch, lexicon);
        }
        true
    }

    /// 追加一個字元。
    pub fn push(&mut self, ch: char, lexicon: &dyn SplitLexicon) {
        self.dp.push(ch, &self.opts, lexicon);
        self.text.push(ch);
    }

    /// 嚴格切分目前內容；整段無法完整解析就回傳 `None`。
    pub fn split(&self) -> Option<Split> {
        let (pieces, score) = self.dp.split()?;
        Some(Split {
            pieces: merge_pieces(pieces),
            score,
        })
    }

    /// 容錯切分目前內容；尾巴照字面留在回傳字串，不進片段。
    pub fn preview(&self) -> (Split, String) {
        let (pieces, score, tail) = self.dp.preview();
        (
            Split {
                pieces: merge_pieces(pieces),
                score,
            },
            tail,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::InMemoryLexicon;

    fn opts() -> Options {
        Options::default()
    }

    fn check_with(input: &str, want: &[(&str, &str, &str)], lexicon: &dyn SplitLexicon) {
        let split = split(input, &opts(), lexicon).unwrap_or_else(|| panic!("{input:?} 應能切分"));
        let got: Vec<(&str, &str, String)> = split
            .pieces
            .iter()
            .map(|piece| {
                (
                    piece.lang.as_str(),
                    piece.text(input),
                    piece.reading.clone(),
                )
            })
            .collect();
        let want: Vec<(&str, &str, String)> = want
            .iter()
            .map(|(lang, text, reading)| (*lang, *text, reading.to_string()))
            .collect();
        assert_eq!(got, want, "{input:?}");
    }

    fn check(input: &str, want: &[(&str, &str, &str)]) {
        check_with(input, want, &NoLexicon);
    }

    fn english_words(words: &[&str]) -> crate::mixed::English {
        let dictionary = crate::mixed::Dictionary::new(
            words.iter().map(|word| crate::mixed::Word::new(*word, 1.0)),
        );
        crate::mixed::English::new(Some(dictionary))
    }

    #[test]
    fn test_english_word_stays_literal() {
        // `small` 以 `s` 開頭，但 `sm` 不是合法羅馬字，只能靠英文詞表判定
        // 為照字面的片段。
        let dict = english_words(&["small"]);
        let lexicon = LexiconAdapter {
            zh: None,
            english: Some(&dict),
        };
        check_with(
            "rishireej/ sasmall",
            &[
                ("ja", "rishire", "りしれ"),
                ("zh", "ej/ ", "ㄍㄨㄥ"),
                ("ja", "sa", "さ"),
                ("en", "small", "small"),
            ],
            &lexicon,
        );
        // `small` 不能被拆成 `s`＋`mall`，也不能變成羅馬字。
        let split = split("sasmall", &opts(), &lexicon).expect("切分");
        assert_eq!(split.len(), 2);
        assert_eq!(split.pieces[0].lang, SplitLang::Ja);
        assert_eq!(split.pieces[1].lang, SplitLang::En);
        assert_eq!(split.pieces[1].text("sasmall"), "small");
    }

    #[test]
    fn test_common_english_beats_romaji() {
        // 高頻英文詞留在英文：`code` 權重 288。
        let common = crate::mixed::English::new(Some(crate::mixed::Dictionary::new(vec![
            crate::mixed::Word::new("code", 288.0),
        ])));
        let lexicon = LexiconAdapter {
            zh: None,
            english: Some(&common),
        };
        let code = split("code", &opts(), &lexicon).expect("切分");
        assert_eq!(code.pieces[0].lang, SplitLang::En);

        // 低頻借詞走完整羅馬字：`kanji` 權重 40。
        let rare = crate::mixed::English::new(Some(crate::mixed::Dictionary::new(vec![
            crate::mixed::Word::new("kanji", 40.0),
        ])));
        let lexicon = LexiconAdapter {
            zh: None,
            english: Some(&rare),
        };
        let kanji = split("kanji", &opts(), &lexicon).expect("切分");
        assert_eq!(kanji.pieces[0].lang, SplitLang::Ja);
        assert_eq!(kanji.pieces[0].reading, "かんじ");
    }

    #[test]
    fn test_fixture_rishireej_savul3() {
        check(
            "rishireej/ savul3",
            &[
                ("ja", "rishire", "りしれ"),
                ("zh", "ej/ ", "ㄍㄨㄥ"),
                ("ja", "sa", "さ"),
                ("zh", "vul3", "ㄒㄧㄠˇ"),
            ],
        );
    }

    #[test]
    fn test_pure_romaji() {
        check("sakura", &[("ja", "sakura", "さくら")]);
        check("shinjuku", &[("ja", "shinjuku", "しんじゅく")]);
        check("kitte", &[("ja", "kitte", "きって")]);
        check("kanji", &[("ja", "kanji", "かんじ")]);
    }

    #[test]
    fn test_pure_zhuyin() {
        check("vul3", &[("zh", "vul3", "ㄒㄧㄠˇ")]);
        check("ru/3", &[("zh", "ru/3", "ㄐㄧㄥˇ")]);
        check("2j/4", &[("zh", "2j/4", "ㄉㄨㄥˋ")]);
        check("y3", &[("zh", "y3", "ㄗˇ")]);
    }

    #[test]
    fn test_mixed_with_spaces() {
        check(
            "konnichiha ej/ desu",
            &[
                ("ja", "konnichiha", "こんにちは"),
                ("zh", "ej/ ", "ㄍㄨㄥ"),
                ("ja", "desu", "です"),
            ],
        );
    }

    #[test]
    fn test_space_is_first_tone_for_zhuyin() {
        // 注音音節後面的空白是一聲，屬於該音節，不是分隔符。
        let with_space = split("ej/ ", &opts(), &NoLexicon).expect("切分");
        assert_eq!(with_space.len(), 1);
        assert_eq!(with_space.pieces[0].lang, SplitLang::Zh);
        assert_eq!(with_space.pieces[0].reading, "ㄍㄨㄥ");
        assert_eq!(with_space.pieces[0].text("ej/ "), "ej/ ");
        let without = split("ej/", &opts(), &NoLexicon).expect("切分");
        assert!(
            with_space.score > without.score,
            "一聲要比沒有聲調高分：{} vs {}",
            with_space.score,
            without.score
        );

        // 已經有聲調的音節，後面的空白只是停頓。
        let toned = split("vul3 ", &opts(), &NoLexicon).expect("切分");
        assert_eq!(toned.pieces[0].reading, "ㄒㄧㄠˇ");
        assert_eq!(toned.pieces[0].text("vul3 "), "vul3");
    }

    #[test]
    fn test_space_after_kana_is_pause() {
        let split = split("sa ", &opts(), &NoLexicon).expect("切分");
        assert_eq!(split.len(), 1);
        assert_eq!(split.pieces[0].text("sa "), "sa");
        assert_eq!(split.pieces[0].reading, "さ");
    }

    #[test]
    fn test_space_tone_then_japanese() {
        check("ej/ sa", &[("zh", "ej/ ", "ㄍㄨㄥ"), ("ja", "sa", "さ")]);
    }

    #[test]
    fn test_tone_keys_mark_zhuyin() {
        // `ji` 兩邊都合法，沒有聲調時偏好日文。
        check("ji", &[("ja", "ji", "じ")]);
        // 聲調鍵固定注音段。
        check("ji3", &[("zh", "ji3", "ㄐㄧˇ")]);
        check("savul3", &[("ja", "sa", "さ"), ("zh", "vul3", "ㄒㄧㄠˇ")]);
    }

    #[test]
    fn test_unparsable_returns_none() {
        assert!(split("qqq", &opts(), &NoLexicon).is_none());
        assert!(split("123", &opts(), &NoLexicon).is_none());
        assert!(split("3d", &opts(), &NoLexicon).is_none());
        assert!(split("1q", &opts(), &NoLexicon).is_none());
    }

    #[test]
    fn test_preview_keeps_tail_literal() {
        let mut splitter = LatinSplitter::new(opts());
        splitter.set_text("rishireq", &NoLexicon);
        let (split, tail) = splitter.preview();
        assert_eq!(split.pieces[0].reading, "りしれ");
        assert_eq!(tail, "q");
    }

    #[test]
    fn test_incremental_matches_full() {
        let input = "rishireej/ savul3";
        let mut splitter = LatinSplitter::new(opts());
        for (i, ch) in input.chars().enumerate() {
            splitter.push(ch, &NoLexicon);
            let _ = i;
        }
        assert_eq!(splitter.split(), split(input, &opts(), &NoLexicon));
    }

    #[test]
    fn test_set_text_appends_only() {
        let mut splitter = LatinSplitter::new(opts());
        assert!(splitter.set_text("rishi", &NoLexicon));
        assert!(splitter.split().is_some());
        assert!(splitter.set_text("rishire", &NoLexicon));
        assert_eq!(splitter.text(), "rishire", "追加不應重建");
        assert!(!splitter.set_text("rishire", &NoLexicon));
        assert!(splitter.set_text("ri", &NoLexicon), "縮短要重建");
        assert_eq!(splitter.split(), split("ri", &opts(), &NoLexicon));
    }

    #[test]
    fn test_lexicon_prefers_known_word() {
        // 有聲調的注音音節才吃詞典加分；`ji3` 是 ㄨㄛˇ（無詞典時也能切，
        // 但詞典命中時分數更高）。沒有聲調的 `ji` 維持日文 じ。
        let mut lexicon = InMemoryLexicon::new();
        lexicon.push(crate::lattice::LexEntry::new(
            "我",
            "ㄨㄛˇ",
            100,
            crate::lattice::UNKNOWN_POS,
        ));
        let with_word = split("ji3", &opts(), &ZhLexiconAdapter(&lexicon)).expect("切分");
        let plain = split("ji3", &opts(), &NoLexicon).expect("切分");
        assert_eq!(with_word.pieces[0].lang, SplitLang::Zh);
        assert_eq!(with_word.pieces[0].reading, "ㄨㄛˇ");
        assert!(with_word.score > plain.score);
        let ambiguous = split("ji", &opts(), &ZhLexiconAdapter(&lexicon)).expect("切分");
        assert_eq!(ambiguous.pieces[0].lang, SplitLang::Ja);
    }

    #[test]
    fn test_space_pauses_between_segments() {
        let split = split("sa vul3", &opts(), &NoLexicon).expect("切分");
        assert_eq!(split.len(), 2);
        assert_eq!(split.pieces[0].text("sa vul3"), "sa");
        assert_eq!(split.pieces[1].text("sa vul3"), "vul3");
        assert_eq!(split.pieces[1].start, 3);
    }
}
