//! 混合輸入的待定按鍵推斷：英數按鍵在聲調或空白時決定是注音還是英文。
//!
//! 對照前身 `core/session/mixed.go` 的 `pendingKeys`、`mixedText`、
//! `finishMixedTone` 與 `mixedBoundary`。

use super::*;
use crate::composition::{CompSegment, Lang as CompLang, Owner};
use crate::lang::Lang;
use crate::lattice::Lexicon;
use crate::mixed::{default_english, prototype, English};
use crate::zh::keyboard::Keyboard;
use crate::zh::syllable::{class_of, Class, Composer, Syllable, TONE1};

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
pub(super) fn compose_keys(raw: &str) -> Option<Composer> {
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

/// 連續小寫英文 run 超過這個長度就只規劃尾巴。
///
/// 英文詞表窗口是 48 個字元，64 已超過窗口，前段必定是字面英文；門檻
/// 取在使用者開始感覺變慢之前，第 65 鍵起就只重算尾巴。
const LONG_LITERAL_RUN: usize = 64;
/// 長 run 每次重新規劃的尾巴長度（rune）。
const PLAN_TAIL_CHARS: usize = 64;
/// 尾巴注音音節最多幾個按鍵。
const ZH_SUFFIX_KEYS: usize = 8;
/// 組字區的硬上限（位元組）：超過就只規劃尾巴，不再重算整串。
///
/// 512 遠大於任何正常中文詞句，也遠大於 TSF 一次組字的常見長度；超過代表
/// 使用者貼上或按住鍵盤打了一大段英數，前段再怎麼重算都不會改變結果。留
/// [`PLAN_TAIL_CHARS`] 給管線是為了讓最後一個注音音節仍能即時切換。
const MAX_COMPOSITION_CHARS: usize = 512;

/// 連續可列印 ASCII（不含空白）；長到超過門檻就整段視為字面。
fn printable_ascii_run(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_graphic())
}

/// 是否需要跳過英文補全與注音前綴的昂貴查詢。
///
/// 純小寫長 run 是原本的快速路徑；英數、標點或大小寫混雜的長 run 同樣不會
/// 因為每鍵重跑 DP 而變慢，也避免 `; ' , .` 被當成注音韻母。
fn long_literal_keys(text: &str) -> bool {
    text.len() > LONG_LITERAL_RUN && printable_ascii_run(text)
}

/// `raw` 加上任何一個聲調是否組得成詞典解釋得了的音節。
fn mixed_reachable(raw: &str, pieces: &[AsciiPiece], lexicon: &dyn Lexicon) -> bool {
    [TONE1, 'ˊ', 'ˇ', 'ˋ', '˙']
        .into_iter()
        .any(|tone| mixed_reading(raw, tone, pieces, lexicon).is_some())
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

/// `text` 結尾最後一段連續可列印 ASCII 的（起點, 終點）；允許尾端空白。
fn last_printable_run(text: &str) -> Option<(usize, usize)> {
    let end = text.trim_end_matches(' ').len();
    let bytes = text.as_bytes();
    let mut start = end;
    while start > 0 && bytes[start - 1].is_ascii_graphic() {
        start -= 1;
    }
    (end > start).then_some((start, end))
}

/// 長小寫 run 的凍結片段：整段照字面輸出英文。
fn literal_en_plan(start: usize, end: usize) -> crate::mixed::Plan {
    crate::mixed::Plan {
        start,
        end,
        mode: Lang::En,
        script: crate::segment::Script::Latin,
        kana: String::new(),
        kana_ok: false,
        verbatim: false,
        zh: None,
        zh_done: Vec::new(),
    }
}

/// 把相鄰的英文字面片段併回一個，維持「一個英文 run 就是一個片段」。
fn merge_adjacent_en_plans(plans: &mut Vec<crate::mixed::Plan>) {
    let mut merged: Vec<crate::mixed::Plan> = Vec::with_capacity(plans.len());
    for plan in plans.drain(..) {
        if let Some(last) = merged.last_mut() {
            if last.mode == Lang::En
                && last.script == crate::segment::Script::Latin
                && plan.mode == Lang::En
                && plan.script == crate::segment::Script::Latin
                && last.end == plan.start
            {
                last.end = plan.end;
                continue;
            }
        }
        merged.push(plan);
    }
    *plans = merged;
}

/// 長小寫 run 尾巴接一個完整注音音節時的切點（前段字元數, 音節）。
///
/// `text` 要含聲調鍵或一聲空白。前段必須是超過門檻的純小寫字面英文，
/// 這樣凍結前段不會把注音、日文或標點鎖成英文；尾巴最多
/// [`ZH_SUFFIX_KEYS`] 個按鍵。
fn long_run_zh_boundary(
    text: &str,
    pieces: &[AsciiPiece],
    lexicon: &dyn Lexicon,
) -> Option<(usize, Syllable)> {
    long_run_zh_boundary_with(text, pieces, lexicon, false)
}

/// 混合英數、標點的長 run 版本：前段只要是可列印 ASCII 就能凍結。
///
/// 尾巴（`body`）仍只接受小寫字母，`; ' , .` 與數字一律不進注音解碼，
/// 隨機長串因此不會冒出 `ㄎㄨㄑㄩ`；`z…zsu3` 這種真的注音尾巴照樣切開。
fn long_run_zh_boundary_mixed(
    text: &str,
    pieces: &[AsciiPiece],
    lexicon: &dyn Lexicon,
) -> Option<(usize, Syllable)> {
    long_run_zh_boundary_with(text, pieces, lexicon, true)
}

fn long_run_zh_boundary_with(
    text: &str,
    pieces: &[AsciiPiece],
    lexicon: &dyn Lexicon,
    allow_mixed_prefix: bool,
) -> Option<(usize, Syllable)> {
    let tone_key = text.chars().next_back()?;
    let (body_end, symbol) = if tone_key.is_whitespace() {
        (text.len() - tone_key.len_utf8(), TONE1)
    } else {
        let symbol = Keyboard::standard().symbol(tone_key)?;
        if class_of(symbol) != Class::Tone {
            return None;
        }
        (text.len() - tone_key.len_utf8(), symbol)
    };
    let low = body_end.saturating_sub(ZH_SUFFIX_KEYS);
    for start in low..body_end {
        let prefix = &text[..start];
        let prefix_ok = if allow_mixed_prefix {
            prefix.len() >= LONG_LITERAL_RUN && printable_ascii_run(prefix)
        } else {
            prefix.len() >= LONG_LITERAL_RUN && prefix.bytes().all(|byte| byte.is_ascii_lowercase())
        };
        if !prefix_ok {
            continue;
        }
        let body = &text[start..body_end];
        if body.is_empty() || !body.bytes().all(|byte| byte.is_ascii_lowercase()) {
            continue;
        }
        if let Some(syllable) = mixed_reading(body, symbol, pieces, lexicon) {
            return Some((start, syllable));
        }
    }
    None
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
    /// 這次組字實際按過的按鍵流（含空白與聲調）。統一管線用它重算片段。
    typed: String,
    /// 這個輸入層使用的英文詞表；`None` 時用行程共用的詞表。
    english: Option<std::sync::Arc<English>>,
    /// 待定按鍵已被使用者固定成字面；統一管線不得再改寫成假名。
    literalized: bool,
}

impl AsciiInput {
    /// 建立空狀態。
    pub fn new() -> Self {
        Self::default()
    }

    /// 指定英文詞表；引擎載入資料根目錄的 `english/` 後用它帶進來。
    pub fn with_english(mut self, english: Option<std::sync::Arc<English>>) -> Self {
        self.english = english;
        self
    }

    /// 實際使用的英文詞表。
    fn english(&self) -> &English {
        match &self.english {
            Some(english) => english,
            None => default_english(),
        }
    }

    /// 目前的原始按鍵。
    pub fn pending(&self) -> &str {
        &self.pending
    }

    /// 這次組字按過的完整按鍵流。
    pub fn typed(&self) -> &str {
        &self.typed
    }

    /// 已完成的片段。
    pub fn pieces(&self) -> &[AsciiPiece] {
        &self.pieces
    }

    /// 是否沒有任何內容。
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.pieces.is_empty() && self.typed.is_empty()
    }

    /// 清空。
    pub fn reset(&mut self) {
        self.literalized = false;
        self.pending.clear();
        self.pieces.clear();
        self.typed.clear();
    }

    /// 已完成的注音音節，依序。
    #[allow(dead_code)] // 對外診斷用；session 目前只讀 pieces。
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
        self.literalized = false;
        self.pending.push(ch);
        self.typed.push(ch);
    }

    /// 把目前的待定按鍵固定成字面。
    ///
    /// 使用者從候選清單選回原始按鍵時，即使那一串剛好也能解成羅馬字，也要
    /// 保留字面，不能再被下一次重建改寫成假名。
    pub fn force_literal(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let raw = std::mem::take(&mut self.pending);
        self.push_literal(&raw);
        self.literalized = true;
    }

    /// 把一段已定案的字面放進按鍵流；不進待定按鍵。
    ///
    /// 混打推斷完日文片段後改走別的語言時，已顯示的日文要先變成固定前綴，
    /// 否則下一次重建會從按鍵流重算而消失。
    pub(super) fn seed_literal(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.push_literal(text);
        self.typed.push_str(text);
    }

    /// 用新的按鍵流重建待定按鍵與片段。
    ///
    /// 刪除整個片段時按鍵流中間會少一段，待定按鍵與片段必須跟著重算；重播
    /// 規則與 [`Session::press_char`] 相同：空白、聲調鍵、其餘按鍵。非 ASCII
    /// 的固定字面直接放回片段，不進待定按鍵。
    pub(super) fn replay(&mut self, keys: &str, lexicon: &dyn Lexicon) {
        self.reset();
        for ch in keys.chars() {
            if !ch.is_ascii() {
                // 非 ASCII 是已定案的字面，會打斷前面的待定按鍵；先收尾，
                // 否則片段順序會變成「字面 + 尚未完成的按鍵」。
                self.flush();
                self.seed_literal(&ch.to_string());
            } else if ch == ' ' {
                self.space(lexicon);
            } else if !self.tone(ch, lexicon) {
                self.press(ch);
            }
        }
    }

    /// 空白：完成待定按鍵。
    ///
    /// 先試一聲讀音，再試英→中切點，最後照字面留下。照字面留下的按鍵後面
    /// 補一個空白（`5pm `、`2026 `）；顯示成注音但組不成字時保留注音，
    /// 空白當一聲吃掉（`c ` → `ㄏ`）。
    pub fn space(&mut self, lexicon: &dyn Lexicon) {
        self.typed.push(' ');
        if self.pending.is_empty() {
            self.push_literal(" ");
            return;
        }
        // 長小寫 run 的空白只有兩種結果：尾巴是完成注音時當一聲，否則整串
        // 照字面留下。兩者都不需要為整串跑完整規劃。
        if long_literal_keys(&self.pending) {
            let with_space = format!("{} ", self.pending);
            if let Some((at, syllable)) = long_run_zh_boundary(&with_space, &self.pieces, lexicon) {
                self.split_literal(at);
                self.push_zh(syllable);
                return;
            }
            let raw = std::mem::take(&mut self.pending);
            self.push_literal(&format!("{raw} "));
            return;
        }
        // R2：前一個英數 run 加上這個空白可以被注音完整解析，而且最後一個
        // 音節原本沒有聲調鍵時，空白是一聲。保留原按鍵，交給統一管線切分。
        let adapter = crate::mixed_latin::LexiconAdapter {
            zh: Some(lexicon),
            english: Some(self.english()),
            ..Default::default()
        };
        let with_space = format!("{} ", self.pending);
        // 切分器把尾巴判成已完成的注音音節時，照它的切點把前段留成字面，
        // 尾巴當注音。`rishireej/ ` 因此是 `rishire`＋`ㄍㄨㄥ`，不是整串
        // 字面；空白是否為一聲完全交給同一條管線決定。
        if let Some((at, syllable)) = self.planned_boundary(&with_space, lexicon) {
            self.split_literal(at);
            self.push_zh(syllable);
            return;
        }
        if crate::mixed_latin::space_is_first_tone(
            &with_space,
            &crate::mixed_latin::Options::default(),
            &adapter,
        ) {
            let raw = std::mem::take(&mut self.pending);
            self.push_literal(&format!("{raw} "));
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
        let display = self.pending_display(lexicon);
        let composer = compose_keys(&self.pending);
        let raw = std::mem::take(&mut self.pending);
        if let Some(mut composer) = composer {
            if display == composer.string() {
                if let Some(syllable) = composer.add(TONE1) {
                    self.push_literal(syllable.as_str());
                    return;
                }
            }
        }
        self.push_literal(&format!("{raw} "));
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
        self.typed.push(key);
        if let Some(syllable) = mixed_reading(&self.pending, symbol, &self.pieces, lexicon) {
            self.push_zh(syllable);
            return true;
        }
        // 批次管線會把英數前段和尾巴的注音音節分開（`savul3` 是 `sa`＋
        // `vul3`），聲調鍵完成的是尾巴那一個音節；照同一份規劃切點拆開，
        // 增量狀態才不會整串留在 pending。
        let with_tone = format!("{}{key}", self.pending);
        // 長字面 run 不跑整串規劃：尾巴的注音音節直接切開，前段留在字面。
        if let Some((at, syllable)) = long_run_zh_boundary(&with_tone, &self.pieces, lexicon) {
            self.split_literal(at);
            self.push_zh(syllable);
            return true;
        }
        // 長小寫 run 沒有可切出的注音音節時，聲調鍵照字面留在 pending，
        // 不必再為整串跑一次完整規劃。
        if long_literal_keys(&self.pending) {
            self.pending.push(key);
            return true;
        }
        if let Some((at, syllable)) = self.planned_boundary(&with_tone, lexicon) {
            self.split_literal(at);
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

    /// 聲調鍵是否會把待定按鍵補成一個注音音節。
    ///
    /// [`AsciiInput::tone`] 在組不出音節時會把聲調鍵照字面留進 pending，那種
    /// 情況仍要交給數字選字鍵處理；這裡只回報真正完成音節的狀況。
    pub(super) fn tone_completes(&self, key: char, lexicon: &dyn Lexicon) -> bool {
        let Some(symbol) = Keyboard::standard().symbol(key) else {
            return false;
        };
        if class_of(symbol) != Class::Tone || self.pending.is_empty() {
            return false;
        }
        mixed_reading(&self.pending, symbol, &self.pieces, lexicon).is_some()
            || self.boundary(symbol, lexicon).is_some()
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
        // 長字面 run 的英文補全沒有意義，前綴查詢只會隨按鍵數變慢。
        if long_literal_keys(raw) {
            return None;
        }
        let english = self.english();
        if raw.len() < 2 || !ascii_letters(raw) || english.has(raw) {
            return None;
        }
        let dict = english.dictionary();
        for suggestion in dict.prefix(raw, 5) {
            if suggestion.text.chars().count() >= raw.chars().count() + 2 {
                return Some(suggestion.text);
            }
        }
        dict.corrections(raw, 1).into_iter().next().map(|c| c.text)
    }

    /// 用候選字面取代待定按鍵；只接受英文詞。
    ///
    /// 混合模式選了前綴補完或拼字建議後，待定按鍵就是那個詞，送出與繼續
    /// 輸入都照一般英文片段處理。
    pub fn accept(&mut self, text: &str) -> bool {
        if text.is_empty() || !ascii_letters(text) {
            return false;
        }
        let keep = self.typed.len().saturating_sub(self.pending.len());
        self.typed.truncate(keep);
        self.typed.push_str(text);
        self.pending = text.to_string();
        true
    }

    /// 接受英文補字建議。
    pub fn accept_suggestion(&mut self) -> bool {
        match self.suggestion() {
            Some(text) => self.accept(&text),
            None => false,
        }
    }

    /// 找統一管線判定的英→中切點：回傳（前段字元數, 尾巴的注音音節）。
    ///
    /// 只接受最後一個輸出片段是注音、且尾巴只剩空白的情況；前段是 `text`
    /// 的位元組位移換算成字元數。這是增量版的 `plan_input`，與批次使用同一套
    /// 詞典與計分，切點不會因為「英文詞前綴」的舊規則而不同。
    fn planned_boundary(&self, text: &str, lexicon: &dyn Lexicon) -> Option<(usize, Syllable)> {
        let english = self.english();
        let adapter = crate::mixed_latin::LexiconAdapter {
            zh: Some(lexicon),
            english: Some(english),
            ..Default::default()
        };
        let cfg = crate::mixed::Config::default();
        let plans = crate::mixed::plan_input(
            text,
            english as &dyn crate::mixed::Lookup,
            &cfg,
            Some(&adapter),
        );
        let plans = crate::mixed::visible_plans(plans, text);
        let plan = plans.last()?;
        if plan.mode != Lang::Zh || plan.start == 0 {
            return None;
        }
        // 前段要是純英文字母的 run；`3.5 ` 這種數字與標點混排仍照字面，
        // 不因為尾巴剛好組得成注音就被拆開。
        let prefix = &text[..plan.start];
        if !ascii_letters(prefix) {
            return None;
        }
        if !text[plan.end..].chars().all(char::is_whitespace) {
            return None;
        }
        let syllable = plan.zh.as_deref()?.split_whitespace().next_back()?;
        Some((
            text[..plan.start].chars().count(),
            Syllable::from_raw(syllable),
        ))
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
    #[allow(dead_code)] // 對外診斷用；session 走 mixed_surface。
    pub fn display(&self, lexicon: &dyn Lexicon) -> String {
        let mut out = String::new();
        for piece in &self.pieces {
            match piece {
                AsciiPiece::Zh(syllable) => out.push_str(syllable.as_str()),
                AsciiPiece::Literal(text) => out.push_str(text),
            }
        }
        out.push_str(&self.pending_display(lexicon));
        out
    }

    /// 待定按鍵除了原按鍵之外的注音候選。
    ///
    /// 對照 `openMixedCandidates`：整串組得成注音時列注音本身；結尾是聲調鍵
    /// 但整串照字面時（`co6`）另列帶聲調的注音（ㄏㄟˊ）。
    pub fn reading_candidate(&self, raw: &str, lexicon: &dyn Lexicon) -> Option<String> {
        if raw.len() > 1 {
            let (last_at, key) = raw.char_indices().next_back()?;
            let body = &raw[..last_at];
            if let Some(symbol) = Keyboard::standard().symbol(key) {
                if class_of(symbol) == Class::Tone {
                    if let Some(mut composer) = compose_keys(body) {
                        if let Some(syllable) = composer.add(symbol) {
                            return Some(syllable.as_str().to_string());
                        }
                    }
                }
            }
        }
        let display = self.pending_display(lexicon);
        if display != raw {
            return Some(display);
        }
        compose_keys(raw).map(|composer| composer.string())
    }

    /// 待定按鍵的顯示字串。
    pub fn display_pending(&self, lexicon: &dyn Lexicon) -> String {
        self.pending_display(lexicon)
    }

    /// 用指定的英文查詢層顯示待定按鍵；讓測試不必依賴系統上的英文資料包。
    #[cfg(test)]
    fn display_pending_with(&self, lexicon: &dyn Lexicon, english: &English) -> String {
        self.pending_display_with(lexicon, english)
    }

    /// 待定按鍵的顯示：能組成注音時顯示注音，英文或打不成注音時照字面。
    ///
    /// 對照 `mixedText`：只有數字與標點、英數反序、或是英文詞／英文詞前綴
    /// 的按鍵照字面，其餘能組音時顯示注音。英文前綴若同時是一聲讀音
    ///（`up` 是 ㄧㄣ），仍顯示注音。
    fn pending_display(&self, lexicon: &dyn Lexicon) -> String {
        self.pending_display_with(lexicon, self.english())
    }

    fn pending_display_with(&self, lexicon: &dyn Lexicon, english: &English) -> String {
        let raw = &self.pending;
        if raw.is_empty() {
            return String::new();
        }
        // 長小寫 run 一律照字面顯示：DP 在同一種 run 上也是字面英文，
        // 不必每次按鍵都重跑 prototype／英文詞表查詢。
        if long_literal_keys(raw) {
            return raw.clone();
        }
        if prototype().has(raw) {
            return raw.clone();
        }
        if !raw.chars().any(char::is_alphabetic) {
            return raw.clone();
        }
        // 數字與字母混在一起、任何聲調都組不成音節（5pm、3d）照字面。
        if raw.chars().any(|ch| ch.is_ascii_digit()) && !mixed_reachable(raw, &self.pieces, lexicon)
        {
            return raw.clone();
        }
        if reversed_alnum(raw) {
            return raw.clone();
        }
        let composer = compose_keys(raw);
        let english_prior =
            raw.chars().count() >= 2 && !english.dictionary().prefix(raw, 1).is_empty();
        if composer.is_some()
            && english_prior
            && mixed_reading(raw, TONE1, &self.pieces, lexicon).is_none()
        {
            return raw.clone();
        }
        match composer {
            Some(composer) => composer.string(),
            None => raw.clone(),
        }
    }
}

/// 字母與數字混打、組得成音節但按鍵順序顛倒（`u2` 的 ㄧㄉ、`o1` 的 ㄟㄅ）。
fn reversed_alnum(raw: &str) -> bool {
    let letters = raw.chars().any(char::is_alphabetic);
    let digits = raw.chars().any(|ch| ch.is_ascii_digit());
    if !letters || !digits || keys_in_syllable_order(raw) || swapped_vowels(raw) {
        return false;
    }
    compose_keys(raw).is_some()
}

/// 快打時一個音節的按鍵只有相鄰兩鍵順序顛倒：介音晚韻母一鍵
///（`2/u` 是 ㄉㄥㄧ）、或聲母晚介音一鍵（`u2/` 是 ㄧㄉㄥ）。
fn swapped_vowels(raw: &str) -> bool {
    let keyboard = Keyboard::standard();
    let mut classes = Vec::new();
    for key in raw.chars() {
        let Some(symbol) = keyboard.symbol(key) else {
            return false;
        };
        classes.push(class_of(symbol));
    }
    matches!(
        classes.as_slice(),
        [Class::Medial, Class::Initial, Class::Final]
    ) || matches!(
        classes.as_slice(),
        [Class::Initial, Class::Final, Class::Medial] | [Class::Final, Class::Medial]
    )
}

impl Session {
    /// 把一次混打送出記進分語言學習（`language_learning.go`）。
    ///
    /// 注音片段記成中文，挑出詞面與讀音；照字面的英數片段記成英文。
    /// 只在送出時呼叫，關閉學習時不寫入。注音片段的讀音短於
    /// [`SessionConfig::min_trigger_len`] 時只記詞面證據。
    pub(super) fn learn_mixed_languages(&mut self) {
        if !self.config.learning || self.private {
            return;
        }
        let at = super::core::now_seconds();
        let left = self.committed.clone();
        let pieces = self.ascii.pieces().to_vec();
        for piece in pieces {
            match piece {
                AsciiPiece::Zh(syllable) => {
                    let surface = self.decode_syllable(&syllable);
                    if surface.is_empty() {
                        continue;
                    }
                    if crate::learn_guard::is_sensitive(&surface) {
                        log::debug!("學習守門：混打注音片段像敏感資料，跳過寫入");
                        continue;
                    }
                    let reading = syllable.as_str();
                    if crate::personal::reading_len(reading) < self.config.min_trigger_len {
                        self.personal.note_language_surface(
                            crate::personal::LanguageId::Zh,
                            &surface,
                            1,
                            at,
                        );
                        continue;
                    }
                    self.personal.note_language(
                        crate::personal::LanguageId::Zh,
                        reading,
                        &surface,
                        &left,
                        at,
                    );
                }
                AsciiPiece::Literal(text) => {
                    for token in text.split_whitespace() {
                        if !crate::personal::english_token(token)
                            || crate::learn_guard::is_sensitive(token)
                        {
                            continue;
                        }
                        self.personal.note_language(
                            crate::personal::LanguageId::En,
                            "",
                            token,
                            &left,
                            at,
                        );
                    }
                }
            }
        }
    }

    /// Mixed 的純注音片段記錄語言證據。
    ///
    /// 混打管線的 `learn_mixed_languages` 讀 `ascii.pieces()`；純注音組字走
    /// 注音狀態機，沒有那條按鍵流，改由已完成的音節解碼出詞面。
    pub(super) fn learn_zh_lane_languages(&mut self) {
        if !self.config.learning || self.private {
            return;
        }
        let at = super::core::now_seconds();
        let left = self.committed.clone();
        let syllables = self.syllables.clone();
        for syllable in syllables {
            let surface = self.decode_syllable(&syllable);
            if surface.is_empty() || crate::learn_guard::is_sensitive(&surface) {
                continue;
            }
            let reading = syllable.as_str();
            if crate::personal::reading_len(reading) < self.config.min_trigger_len {
                self.personal.note_language_surface(
                    crate::personal::LanguageId::Zh,
                    &surface,
                    1,
                    at,
                );
            } else {
                self.personal.note_language(
                    crate::personal::LanguageId::Zh,
                    reading,
                    &surface,
                    &left,
                    at,
                );
            }
        }
    }

    /// 尾巴是否已經是顯示中的英文字面 run。
    ///
    /// `prefix` 是目前待定按鍵扣掉剛按下的字母。長 run 超過詞表窗口後由
    /// 統一管線固定成英文字面；後續字母只能接在同一個字面片段後面。
    fn trailing_literal_english(&self, prefix: &str) -> bool {
        let Some(last) = self.segments.last() else {
            return false;
        };
        if last.mode != Lang::En || last.script != Script::Latin {
            return false;
        }
        let Some(text) = self.base.get(last.start..last.end) else {
            return false;
        };
        !text.is_empty() && text.chars().all(|ch| ch.is_ascii_lowercase()) && prefix.ends_with(text)
    }

    /// 依按鍵推斷日文片段（`ja_mixed.go` 的 `maybeJapanese`）。
    ///
    /// 只在設定有日文解碼器時啟用。待定按鍵必須是兩個以上小寫 ASCII 字母、
    /// 組不成注音、不是英文詞或英文詞前綴，且能完整組成合法羅馬字；符合時
    /// 整段交給日文路徑，送出或取消後回到原本的模式。
    pub(super) fn try_japanese_run(&mut self) -> bool {
        if self.ja_run || self.config.ja.is_none() || !self.ascii.pieces().is_empty() {
            return false;
        }
        let raw = self.ascii.pending().to_string();
        if raw.len() < 2 || !raw.chars().all(|ch| ch.is_ascii_lowercase()) {
            return false;
        }
        if compose_keys(&raw).is_some() {
            return false;
        }
        let english = self.english();
        if !english.dictionary().prefix(&raw, 1).is_empty() {
            return false;
        }
        let config = crate::mixed::Config::default();
        let Some((kana, pending)) = crate::mixed::kana_for(&raw, &config, Some(english)) else {
            return false;
        };
        if kana.is_empty() || !pending.is_empty() {
            return false;
        }
        // 已經字面化的長英文 run 只能續接字面。日文可以從 run 開頭接手，
        // 但不能在使用者看到一整串英文字面後，才把前面幾鍵改寫成假名。
        if raw.len() > crate::mixed_latin::MAX_ENGLISH_CHARS
            && self.trailing_literal_english(&raw[..raw.len() - 1])
        {
            return false;
        }
        self.ascii.reset();
        self.romaji.reset();
        self.ja_run_keys = raw.clone();
        for ch in raw.chars() {
            self.romaji.press(Key::char(ch, false));
        }
        self.ja_run = true;
        true
    }

    /// 日文推斷的片段退格到空時回到原本的模式。
    pub(super) fn leave_empty_japanese_run(&mut self) {
        if !self.ja_run {
            return;
        }
        if !self.romaji.units().is_empty() || !self.romaji.pending().is_empty() {
            return;
        }
        self.ja_run = false;
        self.ja_run_keys.clear();
    }

    /// 日文組字中退格後，把原始按鍵同步回目前的羅馬字狀態。
    ///
    /// `romaji.backspace` 可能一次退掉一整組羅馬字（`kya` 是三個鍵），直接
    /// 砍一個字元會讓退回混打時重播錯的按鍵。用目前狀態反推最長的原始前綴；
    /// 找不到時清空，退回混打時就不會硬塞已刪除的按鍵。
    ///
    /// `ja_only`（日文主語言）也走同一條路徑：原始羅馬字要留著當退化候選。
    pub(super) fn sync_ja_run_keys(&mut self) {
        if !self.in_ja_lane() {
            return;
        }
        let units = self.romaji.units().to_vec();
        let pending = self.romaji.pending().to_string();
        let chars: Vec<char> = self.ja_run_keys.chars().collect();
        let mut probe = self.romaji.clone();
        probe.reset();
        for len in (0..=chars.len()).rev() {
            let mut trial = probe.clone();
            trial.reset();
            let mut accepted = true;
            for ch in &chars[..len] {
                if trial.press(Key::char(*ch, false)) == crate::ja::input::RomajiEvent::Rejected {
                    accepted = false;
                    break;
                }
            }
            if accepted && trial.units() == units.as_slice() && trial.pending() == pending {
                self.ja_run_keys = chars[..len].iter().collect();
                return;
            }
        }
        self.ja_run_keys.clear();
    }

    /// 混合模式的 Backspace：刪掉游標左邊一個字元。
    ///
    /// 游標以 rune 為單位；中文一個字背後的整个音節按鍵一起刪，英文只刪一個
    /// 字母。沒有可刪的目標時才退回舊的整片段路徑。
    pub(super) fn backspace_mixed(&mut self) -> bool {
        let text = self.base_with_highlight();
        let runes = text.chars().count();
        if runes == 0 {
            return false;
        }
        let cursor = self.mixed_cursor.min(runes);
        if cursor == 0 {
            return false;
        }
        let saved = self.mixed_cursor;
        self.mixed_cursor = cursor - 1;
        let target = self.mixed_target_at_cursor();
        self.mixed_cursor = saved;
        if let Some(target) = target {
            let typed = self.ascii.typed().to_string();
            let (start, end) = target.typed;
            if start < end && end <= typed.len() && typed.is_char_boundary(start) {
                let mut next = String::with_capacity(typed.len().saturating_sub(end - start));
                next.push_str(&typed[..start]);
                next.push_str(&typed[end..]);
                self.ascii.replay(&next, &self.lexicon);
                self.literal_spans.clear();
                let len = self.base_with_highlight().chars().count();
                self.mixed_cursor = (cursor - 1).min(len);
                return true;
            }
        }
        let len = self.segments.len();
        let at = self.mixed_cursor.min(len);
        if at == 0 {
            return false;
        }
        if self.remove_mixed_segment(at - 1) {
            return true;
        }
        // 舊路徑（沒有統一管線片段）至少退掉按鍵流最後一個字元。
        let mut typed = self.ascii.typed().to_string();
        if typed.pop().is_none() {
            return false;
        }
        self.ascii.replay(&typed, &self.lexicon);
        self.literal_spans.clear();
        self.mixed_cursor = Self::MIXED_CURSOR_END;
        true
    }

    /// 混合模式的 Delete：刪掉游標後一個片段。
    pub(super) fn remove_mixed_segment(&mut self, index: usize) -> bool {
        let Some((start, end)) = self.mixed_typed_spans().get(index).copied() else {
            return false;
        };
        let typed = self.ascii.typed().to_string();
        if start > end || end > typed.len() || !typed.is_char_boundary(start) {
            return false;
        }
        let mut next = String::with_capacity(typed.len().saturating_sub(end - start));
        next.push_str(&typed[..start]);
        next.push_str(&typed[end..]);
        self.ascii.replay(&next, &self.lexicon);
        // 游標停在被刪片段的位置：後面若還有片段就停在它前面，否則回到結尾。
        self.mixed_cursor = index.min(self.segments.len());
        true
    }

    /// 目前顯示片段對應的按鍵流範圍（位元組）。
    ///
    /// 顯示一律走統一管線，因此用同一份 `plan_input` 重算；回傳長度與
    /// [`Session::segments`] 對不上時表示目前是舊路徑，呼叫端不要依索引刪除。
    fn mixed_typed_spans(&self) -> Vec<(usize, usize)> {
        let Some(text) = self.latin_run_text() else {
            return Vec::new();
        };
        let Some(plans) = self.incremental_latin_split_for(text) else {
            return Vec::new();
        };
        let rendered = self.render_latin_plans(text, &plans);
        plans
            .iter()
            .zip(rendered)
            .filter(|(_, item)| !item.surface.is_empty())
            .map(|(plan, _)| (plan.start, plan.end))
            .collect()
    }

    /// 游標所在注音音節在按鍵流中的範圍；沒有時回傳 `None`。
    ///
    /// 候選與選字都只處理游標所在的那一個音節：游標在結尾時是尾端最後一個
    /// 注音（尾端開候選窗的空白不算）；游標停在片段邊界時，取游標左側片段
    /// 的最後一個注音音節。回傳（起點, 終點, 音節），範圍是 `typed` 的位元組
    /// 位移。
    fn mixed_zh_syllable(&self) -> Option<(usize, usize, Syllable)> {
        let text = self.latin_run_text()?;
        let at = self.mixed_cursor.min(self.segments.len());
        let end = if at < self.segments.len() {
            // 游標在片段邊界：游標左側那個片段的最後一個音節。
            if at == 0 {
                return None;
            }
            self.mixed_typed_spans().get(at - 1)?.1
        } else {
            // 游標在結尾：先試整串按鍵流；尾端只是開候選窗的空白時再省略。
            let mut whole = self.ascii_input();
            whole.replay(text, &self.lexicon);
            let complete = whole.pending().is_empty()
                && matches!(whole.pieces().last(), Some(AsciiPiece::Zh(_)));
            if complete {
                text.len()
            } else {
                text.trim_end().len()
            }
        };
        if end == 0 || end > text.len() || !text.is_char_boundary(end) {
            return None;
        }
        // 重播到目標結尾：最後一個片段必須是注音音節，前面片段是它的前綴。
        let mut before = self.ascii_input();
        before.replay(&text[..end], &self.lexicon);
        if !before.pending().is_empty() {
            return None;
        }
        let pieces = before.pieces();
        let Some(AsciiPiece::Zh(expected)) = pieces.last() else {
            return None;
        };
        let expected = expected.clone();
        let prefix_pieces = &pieces[..pieces.len() - 1];
        // 從尾端往回掃切點：後段重播後只能有這一個注音音節，前段重播要
        // 重現前面的片段，這樣才不會切在音節按鍵中間。
        let starts: Vec<usize> = text[..end].char_indices().map(|(start, _)| start).collect();
        for start in starts.into_iter().rev() {
            let mut suffix = self.ascii_input();
            suffix.replay(&text[start..end], &self.lexicon);
            if !suffix.pending().is_empty() || suffix.pieces().len() != 1 {
                continue;
            }
            match &suffix.pieces()[0] {
                AsciiPiece::Zh(syllable) if syllable == &expected => {}
                _ => continue,
            }
            let mut prefix = self.ascii_input();
            prefix.replay(&text[..start], &self.lexicon);
            if prefix.pending().is_empty() && prefix.pieces() == prefix_pieces {
                return Some((start, end, expected));
            }
        }
        None
    }

    /// 把選到的中文候選換進按鍵流，取代游標所在的那一個注音音節。
    ///
    /// 候選字面不是按鍵，放進按鍵流後由管線當固定字面保留；後續打字不會再
    /// 把它重新解碼掉。其他片段（前面的中文、英文、日文與游標後的內容）原樣
    /// 保留。沒有注音音節或範圍不對時回傳 `false`。
    pub(super) fn select_mixed_zh_candidate(&mut self, surface: &str) -> bool {
        let Some((start, end, _)) = self.mixed_zh_syllable() else {
            return false;
        };
        let typed = self.ascii.typed().to_string();
        if start > end || end > typed.len() || !typed.is_char_boundary(start) {
            return false;
        }
        let mut next = String::with_capacity(typed.len() + surface.len());
        next.push_str(&typed[..start]);
        next.push_str(surface);
        // 開候選窗的尾端空白不算內容；選字後不要一起送出。
        if !typed[end..].chars().all(char::is_whitespace) {
            next.push_str(&typed[end..]);
        }
        self.ascii.replay(&next, &self.lexicon);
        self.mixed_cursor = Self::MIXED_CURSOR_END;
        true
    }

    /// 游標右側 rune 所屬的片段索引。
    pub(super) fn mixed_segment_index_at_rune(&self, rune: usize) -> Option<usize> {
        let text = self.base_with_highlight();
        let byte = Self::rune_byte(&text, rune);
        self.segments()
            .iter()
            .position(|segment| segment.start <= byte && byte < segment.end)
    }

    /// 字串中第 `rune` 個字元的位元組位移；超出長度時夾在結尾。
    fn rune_byte(text: &str, rune: usize) -> usize {
        text.char_indices()
            .nth(rune)
            .map(|(byte, _)| byte)
            .unwrap_or(text.len())
    }

    /// 游標右側那一個字元的候選目標。
    ///
    /// 游標在整段結尾時由呼叫端先左移一格；這裡只處理 `0..rune 數`。
    pub(super) fn mixed_target_at_cursor(&self) -> Option<MixedTarget> {
        let text = self.base_with_highlight();
        let runes = text.chars().count();
        let rune_start = self.mixed_cursor.min(runes);
        if rune_start >= runes {
            return None;
        }
        let byte = Self::rune_byte(&text, rune_start);
        if !text.is_char_boundary(byte) {
            return None;
        }
        let segments = self.segments();
        let spans = self.mixed_typed_spans();
        let typed = self.ascii.typed().to_string();
        let plans = self.plan_latin_text(&typed);
        for (plan_index, (index, segment)) in segments.iter().enumerate().enumerate() {
            let plan = plans.get(plan_index);
            if byte < segment.start || byte >= segment.end {
                continue;
            }
            let span = spans.get(index).copied().unwrap_or((0, 0));
            let raw = typed.get(span.0..span.1).unwrap_or_default().to_string();
            let rune_len = text[segment.start..segment.end].chars().count();
            let inside = text[segment.start..byte].chars().count();
            // 固定成字面的原始按鍵整段都是一個可拆回的單位；其他片段從游標
            // 所在的那一個 rune 開始切，英文才停得進 `hel|lo`。
            let forced = self
                .literal_spans
                .iter()
                .any(|&(start, end)| start <= span.0 && span.1 <= end);
            let whole = forced && inside == 0;
            if whole {
                let (toneless, tone) = split_tone_key(&raw);
                let reading = zh_reading_from_keys(&toneless);
                return Some(MixedTarget {
                    typed: (span.0, span.1),
                    raw,
                    raw_toneless: toneless,
                    tone_key: tone,
                    rune_start,
                    rune_len,
                    syllable: None,
                    reading,
                    ja: false,
                });
            }
            // 注音片段：找到游標所在的那一個音節，候選只作用在它身上。
            if let Some(plan) = plan {
                if plan.mode == Lang::Zh {
                    if let Some(reading) = plan.zh.as_deref() {
                        let readings: Vec<&str> = reading.split_whitespace().collect();
                        let keys = zh_key_syllable_spans(&raw);
                        let at = inside.min(readings.len().saturating_sub(1));
                        if let (Some(reading), Some(&(start, end))) =
                            (readings.get(at), keys.get(at))
                        {
                            let syllable_raw = &raw[start..end];
                            let (toneless, tone) = split_tone_key(syllable_raw);
                            return Some(MixedTarget {
                                typed: (span.0 + start, span.0 + end),
                                raw: syllable_raw.to_string(),
                                raw_toneless: toneless,
                                tone_key: tone,
                                rune_start,
                                rune_len: 1,
                                syllable: None,
                                reading: Some((*reading).to_string()),
                                ja: false,
                            });
                        }
                    }
                }
                if plan.mode == Lang::Ja && inside == 0 {
                    return Some(MixedTarget {
                        typed: (span.0, span.1),
                        raw,
                        raw_toneless: String::new(),
                        tone_key: None,
                        rune_start,
                        rune_len,
                        syllable: None,
                        reading: Some(plan.kana.clone()),
                        ja: true,
                    });
                }
            }
            let (typed_start, typed_end, rune_len) = if raw.chars().count() == rune_len {
                // 字面一鍵一字時，候選只作用在游標那一個字元；連續前導符號
                // 或多字字面不能整段被一個候選取代。
                let at = raw
                    .char_indices()
                    .nth(inside)
                    .map(|(byte, _)| byte)
                    .unwrap_or(raw.len());
                let next = raw[at..]
                    .chars()
                    .next()
                    .map(|ch| at + ch.len_utf8())
                    .unwrap_or(raw.len());
                (span.0 + at, span.0 + next, 1)
            } else {
                (span.0, span.1, rune_len)
            };
            return Some(MixedTarget {
                typed: (typed_start, typed_end),
                raw,
                raw_toneless: String::new(),
                tone_key: None,
                rune_start,
                rune_len,
                syllable: None,
                reading: None,
                ja: false,
            });
        }
        None
    }

    /// 候選窗要作用在游標右側的字元；結尾先左移一格。
    ///
    /// 只有按 ↓ 會走到這裡。回傳是否有可開的候選。
    pub(super) fn open_mixed_cursor_candidates(&mut self) -> bool {
        let text = self.base_with_highlight();
        let runes = text.chars().count();
        if runes == 0 {
            return false;
        }
        // 整段都是英文待定詞時，候選是整串的拼字建議與補全，不是單一字母；
        // 套用時替換整個詞，游標留在結尾。
        let raw = self.ascii.typed().to_string();
        if !self.in_zh_lane()
            && !self.in_ja_lane()
            && self.ascii.pending() == raw
            && self.literal_spans.is_empty()
            && !self.candidates.is_empty()
        {
            let choices: Vec<Option<MixedChoice>> = self
                .candidates
                .iter()
                .map(|candidate| {
                    Some(MixedChoice {
                        insert: candidate.surface.clone(),
                        leftover: String::new(),
                        force_literal: true,
                        cursor: ChoiceCursor::TokenEnd,
                        syllable_edit: None,
                    })
                })
                .collect();
            self.mixed_choices = choices;
            self.mixed_target = Some(MixedTarget {
                typed: (0, raw.len()),
                raw: raw.clone(),
                raw_toneless: raw.clone(),
                tone_key: None,
                rune_start: 0,
                rune_len: runes,
                syllable: None,
                reading: None,
                ja: false,
            });
            self.highlighted = 0;
            self.candidate_window = true;
            self.candidate_expanded = false;
            self.navigating = true;
            self.keep_candidates = true;
            return true;
        }
        if self.mixed_cursor.min(runes) >= runes {
            self.mixed_cursor = runes - 1;
        }
        let Some(target) = self.mixed_target_at_cursor() else {
            return false;
        };
        let (candidates, choices) = self.mixed_candidates_for(&target);
        if candidates.is_empty() {
            return false;
        }
        self.candidates = candidates;
        self.mixed_choices = choices;
        self.mixed_target = Some(target);
        self.highlighted = 0;
        self.candidate_window = true;
        self.candidate_expanded = false;
        self.navigating = true;
        self.keep_candidates = true;
        true
    }

    /// 為一個字元目標產生候選與套用動作。
    fn mixed_candidates_for(
        &self,
        target: &MixedTarget,
    ) -> (Vec<Candidate>, Vec<Option<MixedChoice>>) {
        let text = self.base_with_highlight();
        let start = Self::rune_byte(&text, target.rune_start);
        let end = text[start..]
            .chars()
            .take(target.rune_len)
            .map(char::len_utf8)
            .sum::<usize>()
            + start;
        let surface = text[start..end].to_string();
        let mut out: Vec<(Candidate, MixedChoice)> = Vec::new();
        // 字面片段：只列這一個字元（英文 `hello` 的 `o`），不往左組詞。
        if target.reading.is_none() && !target.ja {
            out.push((
                Candidate::new(
                    surface.clone(),
                    surface.clone(),
                    0,
                    CandidateSource::Literal,
                ),
                MixedChoice {
                    insert: surface.clone(),
                    leftover: String::new(),
                    force_literal: false,
                    cursor: ChoiceCursor::AfterInsert,
                    syllable_edit: None,
                },
            ));
            if let Some(full) = crate::symbols::width_variant(&surface) {
                if full != surface {
                    out.push((
                        Candidate::new(full.clone(), surface.clone(), 1, CandidateSource::Literal),
                        MixedChoice {
                            insert: full,
                            leftover: String::new(),
                            force_literal: false,
                            cursor: ChoiceCursor::AfterInsert,
                            syllable_edit: None,
                        },
                    ));
                }
            }
            // 反引號前導的多變體符號：候選是同一組的其他寫法。
            if let Some(variants) = super::input_style::prefix_variants_for_surface(&surface) {
                for variant in variants {
                    if *variant == surface
                        || out
                            .iter()
                            .any(|(candidate, _)| candidate.surface == *variant)
                    {
                        continue;
                    }
                    out.push((
                        Candidate::new(
                            (*variant).to_string(),
                            surface.clone(),
                            1,
                            CandidateSource::Literal,
                        ),
                        MixedChoice {
                            insert: (*variant).to_string(),
                            leftover: String::new(),
                            force_literal: false,
                            cursor: ChoiceCursor::AfterInsert,
                            syllable_edit: None,
                        },
                    ));
                }
            }
            return split_choices(out);
        }
        // 日文片段：かな漢字変換的漢字假設＋かな＋原始羅馬字。
        if target.ja {
            if let Some(kana) = target.reading.as_deref() {
                if let Some(decoder) = self.config.ja.clone() {
                    let chars: Vec<char> = kana.chars().collect();
                    let results = decoder.results(
                        &chars,
                        &JaContext::default(),
                        self.max_candidates().max(1),
                    );
                    for result in results {
                        let conversion = result.conversion();
                        let surface = conversion.surface.clone();
                        if out
                            .iter()
                            .any(|(candidate, _)| candidate.surface == surface)
                        {
                            continue;
                        }
                        out.push((
                            Candidate::new(
                                surface.clone(),
                                kana.to_string(),
                                conversion.cost,
                                CandidateSource::Lexicon,
                            ),
                            MixedChoice {
                                insert: surface.clone(),
                                leftover: String::new(),
                                force_literal: false,
                                cursor: if surface.chars().count() > 1
                                    && surface.chars().all(is_kana)
                                {
                                    ChoiceCursor::AfterFirst
                                } else {
                                    ChoiceCursor::AfterInsert
                                },
                                syllable_edit: None,
                            },
                        ));
                    }
                }
                for (surface, cursor) in [
                    (kana.to_string(), ChoiceCursor::AfterFirst),
                    (target.raw.clone(), ChoiceCursor::Start),
                ] {
                    if surface.is_empty()
                        || out
                            .iter()
                            .any(|(candidate, _)| candidate.surface == surface)
                    {
                        continue;
                    }
                    out.push((
                        Candidate::new(
                            surface.clone(),
                            kana.to_string(),
                            0,
                            CandidateSource::Literal,
                        ),
                        MixedChoice {
                            insert: surface,
                            leftover: String::new(),
                            force_literal: false,
                            cursor,
                            syllable_edit: None,
                        },
                    ));
                }
            }
            return split_choices(out);
        }
        // 注音音節：詞典候選＋注音本身＋原始按鍵＋無聲調按鍵＋日文解讀。
        if let Some(reading) = target.reading.as_deref() {
            for candidate in self.zh_syllable_candidates(reading) {
                out.push((
                    candidate.clone(),
                    MixedChoice {
                        insert: candidate.surface.clone(),
                        leftover: String::new(),
                        force_literal: false,
                        cursor: ChoiceCursor::AfterInsert,
                        syllable_edit: target.syllable.map(|start| EditWindow { start, len: 1 }),
                    },
                ));
            }
            push_decomposition(
                &mut out,
                reading,
                &target.raw,
                &target.raw_toneless,
                target.tone_key,
            );
        }
        split_choices(out)
    }

    /// 一個注音讀音的詞典候選；不改動工作階段。
    fn zh_syllable_candidates(&self, reading: &str) -> Vec<Candidate> {
        let units = [reading];
        let Ok(lattice) = Lattice::build(&units, &self.lexicon, &self.ctx) else {
            return Vec::new();
        };
        let scorer = super::core::zh_scorer(&self.ctx, self.zh_lm.as_deref());
        let decoder = super::core::zh_decoder(&self.lexicon, &self.ctx, scorer.as_ref());
        let (hypotheses, _) = decoder.nbest(&lattice, &Locks::new(), false, self.max_candidates());
        let hypotheses = self.rank_zh_hypotheses(hypotheses, reading);
        let literal = reading.to_string();
        let mut out: Vec<Candidate> = Vec::new();
        for hypothesis in hypotheses {
            if hypothesis.text == literal {
                continue;
            }
            if out
                .iter()
                .any(|candidate| candidate.surface == hypothesis.text)
            {
                continue;
            }
            out.push(Candidate::new(
                hypothesis.text,
                reading.to_string(),
                hypothesis.cost,
                CandidateSource::Lexicon,
            ));
        }
        out.truncate(self.max_candidates().max(1));
        out
    }

    /// 把按鍵流 `text` 的前 `at` 個位元組換成目前顯示的轉換結果。
    ///
    /// 只處理切點落在照字面輸出的片段內的情況；注音或日文片段中間需要
    /// 音節邊界才切得準，遇到時回傳 `None` 讓呼叫端維持原本的重播路徑。
    /// 回傳字串是 `typed[..at]` 在畫面上的樣子，空白等不顯示的片段會被
    /// 略過，因此前段可以整段凍結成字面，不必再被切分器重新解讀。
    fn rendered_typed_prefix(&self, text: &str, at: usize) -> Option<String> {
        let plans = self.plan_latin_text(text);
        let rendered = self.render_latin_plans(text, &plans);
        let mut out = String::new();
        let mut done = 0usize;
        for (plan, item) in plans.iter().zip(rendered) {
            if plan.end <= at {
                out.push_str(&item.surface);
                done = done.max(plan.end);
                continue;
            }
            if plan.start >= at {
                break;
            }
            let literal = text.get(plan.start..plan.end)?;
            if item.surface != literal {
                return None;
            }
            out.push_str(text.get(plan.start..at)?);
            done = at;
            break;
        }
        // 切點前的按鍵都要有著落；中間只允許被 `visible_plans` 吃掉的不顯示空白。
        (done == at
            || text
                .get(done..at)
                .is_some_and(|rest| rest.chars().all(char::is_whitespace)))
        .then_some(out)
    }

    /// 把候選套用動作寫進按鍵流；`None` 表示這一個候選不存在。
    pub(super) fn apply_mixed_choice(&mut self, index: usize) -> Option<Snapshot> {
        let choice = self.mixed_choices.get(index).cloned().flatten()?;
        let target = self.mixed_target.clone()?;
        if let Some(edit) = choice.syllable_edit {
            self.edit = Some(edit);
            self.mixed_target = None;
            self.mixed_choices.clear();
            let applied = self.select_window_candidate(index);
            self.candidate_window = false;
            self.candidate_expanded = false;
            self.navigating = false;
            self.keep_candidates = false;
            self.edit = None;
            if applied {
                // 選字後游標往右一格；比一個 rune 長的替換放到第一個字元之後。
                self.cursor = (edit.start + 1).min(self.syllables.len());
            }
            return Some(self.finish_choice());
        }
        if let Some(index) = target.syllable {
            self.mixed_target = None;
            self.mixed_choices.clear();
            self.candidate_expanded = false;
            if self.materialize_zh_choice(index, &choice) {
                return Some(self.snapshot());
            }
            return None;
        }
        let typed = self.ascii.typed().to_string();
        let (start, end) = target.typed;
        if start > end || end > typed.len() || !typed.is_char_boundary(start) {
            return None;
        }
        // 字面候選只換掉被取代的那一個字元；前段已經轉換好的注音、日文與
        // 英文照目前顯示留下，不再跟著原始按鍵整串重播（`ji3hello` 選全形
        // `ｏ` 不能讓前面的「我」退回 `ji3`）。
        let prefix = if target.reading.is_none() && !target.ja {
            self.rendered_typed_prefix(&typed, start)
                .unwrap_or_else(|| typed[..start].to_string())
        } else {
            typed[..start].to_string()
        };
        // 前段真的被轉換過才需要凍結；純英文字面照原按鍵重播結果相同。
        let prefix_converted = prefix != typed[..start];
        let mut next =
            String::with_capacity(typed.len() + choice.insert.len() + choice.leftover.len());
        next.push_str(&prefix);
        let insert_start = next.len();
        next.push_str(&choice.insert);
        next.push_str(&choice.leftover);
        next.push_str(&typed[end..]);
        self.ascii.replay(&next, &self.lexicon);
        self.literal_spans.clear();
        if insert_start > 0 && prefix_converted {
            self.literal_spans.push((0, insert_start));
        }
        if choice.force_literal {
            let span = (insert_start, next.len().saturating_sub(typed[end..].len()));
            self.literal_spans.push(span);
        }
        self.mixed_target = None;
        self.mixed_choices.clear();
        self.candidate_window = false;
        self.candidate_expanded = false;
        self.navigating = false;
        self.keep_candidates = false;
        self.highlighted = 0;
        self.rebuild();
        let rune_start = target
            .rune_start
            .min(self.base_with_highlight().chars().count());
        let cursor = match choice.cursor {
            ChoiceCursor::Start => rune_start,
            ChoiceCursor::AfterFirst => rune_start + 1,
            ChoiceCursor::AfterInsert => rune_start + choice.insert.chars().count(),
            ChoiceCursor::TokenEnd => self.base_with_highlight().chars().count(),
        };
        self.mixed_cursor = cursor.min(self.base_with_highlight().chars().count());
        Some(self.snapshot())
    }

    /// ↓ 在純注音組字上開候選：只列游標右側那一個音節。
    pub(super) fn open_zh_cursor_candidates(&mut self) -> bool {
        if !self.zh_pending_keys.is_empty() || !self.zh.pending().is_empty() {
            // 未完成的音節退化成原始字母，交給混打管線；下一次 ↓ 再開。
            if !self.zh_pending_keys.is_empty() {
                let ch = self.zh_pending_keys.chars().next().unwrap_or(' ');
                self.leave_zh_lane_for_mixed(ch);
            }
            return false;
        }
        if self.syllables.is_empty() {
            return false;
        }
        if self.cursor >= self.syllables.len() {
            self.cursor = self.syllables.len() - 1;
        }
        let index = self.cursor.min(self.syllables.len().saturating_sub(1));
        let syllable = self.syllables[index].clone();
        let reading = syllable.as_str().to_string();
        let raw = self
            .zh
            .keyboard()
            .keys(&syllable)
            .unwrap_or_else(|_| reading.clone());
        let (raw_toneless, tone_key) = split_tone_key(&raw);
        let rune_start: usize = self.syllables[..index]
            .iter()
            .map(|syllable| self.decode_syllable(syllable).chars().count())
            .sum();
        let target = MixedTarget {
            typed: (0, 0),
            raw: raw.clone(),
            raw_toneless: raw_toneless.clone(),
            tone_key,
            rune_start,
            rune_len: 1,
            syllable: Some(index),
            reading: Some(reading.clone()),
            ja: false,
        };
        let (mut candidates, mut choices) = {
            let mut out: Vec<(Candidate, MixedChoice)> = Vec::new();
            for candidate in self.zh_syllable_candidates(&reading) {
                out.push((
                    candidate.clone(),
                    MixedChoice {
                        insert: candidate.surface.clone(),
                        leftover: String::new(),
                        force_literal: false,
                        cursor: ChoiceCursor::AfterInsert,
                        syllable_edit: Some(EditWindow {
                            start: index,
                            len: 1,
                        }),
                    },
                ));
            }
            push_decomposition(&mut out, &reading, &raw, &raw_toneless, tone_key);
            split_choices(out)
        };
        if candidates.is_empty() {
            return false;
        }
        // 字典候選走片段選字，拆回字面的選項重播整段按鍵。
        candidates.truncate(self.max_candidates().max(1));
        choices.truncate(candidates.len());
        self.candidates = candidates;
        self.mixed_choices = choices;
        self.mixed_target = Some(target);
        self.edit = Some(EditWindow {
            start: index,
            len: 1,
        });
        self.highlighted = 0;
        self.candidate_window = true;
        self.candidate_expanded = false;
        self.navigating = true;
        self.keep_candidates = true;
        true
    }

    /// 把純注音組字的一個音節換成字面並退出注音路徑。
    fn materialize_zh_choice(&mut self, index: usize, choice: &MixedChoice) -> bool {
        let keyboard = self.zh.keyboard();
        let mut keys = String::new();
        let mut span = (0usize, 0usize);
        for (at, syllable) in self.syllables.iter().enumerate() {
            if at == index {
                span.0 = keys.len();
                keys.push_str(&choice.insert);
                keys.push_str(&choice.leftover);
                span.1 = keys.len();
                continue;
            }
            if let Ok(syllable_keys) = keyboard.keys(syllable) {
                keys.push_str(&syllable_keys);
            }
        }
        keys.push_str(&self.zh_pending_keys);
        keys.push_str(&self.literal);
        keys.push_str(&self.literal_tail);
        let before: String = self
            .syllables
            .iter()
            .take(index)
            .map(|syllable| self.decode_syllable(syllable))
            .collect();
        let rune_start = before.chars().count();
        self.zh.reset();
        self.zh_lane = false;
        self.zh_pending_keys.clear();
        self.syllables.clear();
        self.cursor = 0;
        self.locks.clear();
        self.composition.clear();
        self.edit = None;
        self.keep_candidates = false;
        self.highlighted = 0;
        self.candidate_window = false;
        self.candidate_expanded = false;
        self.navigating = false;
        self.segment_manual = None;
        self.last_hypotheses.clear();
        self.pending.clear();
        self.base.clear();
        self.segments.clear();
        self.literal.clear();
        self.literal_anchor = 0;
        self.literal_offset = 0;
        self.literal_tail.clear();
        self.pending_tone = None;
        self.ascii.replay(&keys, &self.lexicon);
        self.literal_spans.clear();
        if choice.force_literal {
            self.literal_spans.push(span);
        }
        self.mixed_cursor = Self::MIXED_CURSOR_END;
        self.rebuild();
        let len = self.base_with_highlight().chars().count();
        let cursor = match choice.cursor {
            ChoiceCursor::Start => rune_start,
            ChoiceCursor::AfterFirst => rune_start + 1,
            ChoiceCursor::AfterInsert => rune_start + choice.insert.chars().count(),
            ChoiceCursor::TokenEnd => self.base_with_highlight().chars().count(),
        };
        self.mixed_cursor = cursor.min(len);
        true
    }

    /// 候選窗是空白打開的時候，按鍵流尾巴會留一個空白；送出前拿掉。
    ///
    /// 只處理 Mixed 且候選窗開著的情況；英文待定詞真正的結尾空白不會走到
    /// 這裡（那時候沒有中文候選）。
    pub(super) fn trim_mixed_candidate_space(&mut self) {
        if self.config.mode != Mode::Mixed || !self.candidate_window {
            return;
        }
        let typed = self.ascii.typed().to_string();
        let trimmed = typed.trim_end_matches(' ');
        if trimmed.len() == typed.len() {
            return;
        }
        self.ascii.replay(trimmed, &self.lexicon);
    }

    /// 混打推斷的日文片段遇到接不上的按鍵時，結束推斷並改走混打。
    ///
    /// 推斷期間吃掉的原始羅馬字按鍵全部重播回混合模式的按鍵流，再把這個
    /// 按鍵交給混打。直接固定已顯示的かな會讓增量結果和批次規劃不一致：
    /// `rishire` 先推斷成日文、多打一個 `e` 後遇到 `j` 時，若把 `りしれえ`
    /// 整段固定，後面的 `j/` 就只能切成 `ㄨㄥ`；用原按鍵重播則批次會把
    /// `e` 還給 `ej/`，得到 `りしれ`＋`供`。`watashiwaw96j0` 也走同一條路徑，
    /// 前面的 `watashiwa` 仍切成日文，`w96` 接成注音。
    pub(super) fn leave_ja_run_for_mixed(&mut self, ch: char) -> bool {
        if !self.ja_run {
            return false;
        }
        let keys = std::mem::take(&mut self.ja_run_keys);
        self.romaji.reset();
        if let Some(live) = self.live.as_mut() {
            live.reset();
        }
        self.ja_run = false;
        self.ja_cursor = 0;
        self.ascii.replay(&keys, &self.lexicon);
        if !self.ascii.tone(ch, &self.lexicon) {
            self.ascii.press(ch);
        }
        self.mixed_cursor = Self::MIXED_CURSOR_END;
        true
    }

    pub(super) fn rebuild_mixed(&mut self) {
        if self.ascii.is_empty() {
            self.composition.clear();
            return;
        }
        let mut split: Option<(Vec<crate::mixed::Plan>, String)> = None;
        if let Some((plans, text)) = self.incremental_latin_split() {
            self.apply_latin_split(&plans, &text);
            split = Some((plans, text));
        } else {
            self.rebuild_mixed_legacy();
        }
        self.rebuild_mixed_candidates(
            split
                .as_ref()
                .map(|(plans, text)| (plans.as_slice(), text.as_str())),
        );
    }

    /// 沒有啟用拉丁切分時的原路徑：照原本的片段推斷組字。
    fn rebuild_mixed_legacy(&mut self) {
        self.base = self.mixed_surface_legacy();
        let mut at = 0usize;
        self.segments = self
            .ascii
            .pieces()
            .iter()
            .map(|piece| {
                let (text, mode, script) = match piece {
                    AsciiPiece::Zh(syllable) => {
                        (self.decode_syllable(syllable), Lang::Zh, Script::Bopomofo)
                    }
                    AsciiPiece::Literal(text) => (text.clone(), Lang::En, Script::Latin),
                };
                let start = at;
                at += text.len();
                Segment::new(start, at, mode, script)
            })
            .collect();
        if !self.ascii.pending().is_empty() {
            let text = self.ascii.display_pending(&self.lexicon);
            let start = at;
            at += text.len();
            let (mode, script) = if text == self.ascii.pending() {
                (Lang::En, Script::Latin)
            } else {
                (Lang::Zh, Script::Bopomofo)
            };
            self.segments.push(Segment::new(start, at, mode, script));
        }
        self.sync_composition_mixed();
    }

    /// 混合模式的候選：原按鍵、注音本身與英文前綴補完。
    fn rebuild_mixed_candidates(&mut self, split: Option<(&[crate::mixed::Plan], &str)>) {
        // 待定按鍵是英文詞時，候選先放原按鍵，再放注音顯示與英文前綴補完。
        // 同一個拼法有學過的修正時，修正直接排第一，原按鍵留在第二項。
        let raw = self.ascii.pending().to_string();
        if raw.is_empty() {
            // 注音音節打完後，待定按鍵是空的，但已完成的注音片段仍有候選；
            // 少了這段，Mixed 按 ↓ 或空白永遠開不了候選窗。
            self.rebuild_mixed_zh_candidates(split);
            return;
        }
        // 待定按鍵還沒清空，但統一管線已經把尾巴判成完成的注音音節
        // （`savul3` 是 `sa`＋`vul3`）。候選要列那個音節的詞，不能只列
        // 原始按鍵；`cands=1` 的字面候選會讓使用者永遠選不到「小」。
        if let Some(reading) =
            split.and_then(|(plans, text)| Self::planned_trailing_zh_reading(plans, text))
        {
            if self.mixed_cursor.min(self.segments.len()) == self.segments.len() {
                self.fill_zh_candidates(reading);
                if !self
                    .candidates
                    .iter()
                    .any(|candidate| candidate.surface == raw)
                {
                    self.candidates.push(Candidate::new(
                        raw.clone(),
                        raw.clone(),
                        i32::MAX,
                        CandidateSource::Literal,
                    ));
                }
                self.candidates.truncate(self.max_candidates());
                return;
            }
        }
        let limit = self.max_candidates();
        let mut seen = vec![raw.to_lowercase()];
        // 長小寫 run 是字面英文，學過的英文修正與注音候選都不會改變它。
        let long_literal = long_literal_keys(&raw);
        if !long_literal {
            if let Some(learned) = self.personal.english_correction(&raw) {
                let learned = learned.to_string();
                let key = learned.to_lowercase();
                if !seen.contains(&key) {
                    seen.push(key);
                    self.candidates.push(Candidate::new(
                        learned,
                        raw.clone(),
                        0,
                        CandidateSource::Personal,
                    ));
                }
            }
        }
        self.candidates.push(Candidate::new(
            raw.clone(),
            raw.clone(),
            0,
            CandidateSource::Literal,
        ));
        // 半形英數與數字提供全形變體；舊英文直出模式的等價候選路徑。
        if let Some(full) = crate::symbols::width_variant(&raw) {
            if full != raw {
                seen.push(full.to_lowercase());
                self.candidates.push(Candidate::new(
                    full,
                    raw.clone(),
                    1,
                    CandidateSource::Literal,
                ));
            }
        }
        // 整串可以組成注音時，原按鍵之外另列注音本身（`co6` 的 ㄏㄟˊ）。
        if !long_literal {
            if let Some(text) = self.ascii.reading_candidate(&raw, &self.lexicon) {
                if !seen.contains(&text) {
                    seen.push(text.clone());
                    self.candidates.push(Candidate::new(
                        text,
                        raw.clone(),
                        1,
                        CandidateSource::Literal,
                    ));
                }
            }
        }
        // pending 顯示成注音代表它還是未完成的注音前綴（例如 su 是 ㄋㄧ）。
        // 這時英文補全只留在 Snapshot.suggestion 提示，不進候選讓數字鍵選到，
        // 避免組字區被補全取代而鎖死語言。hello、sug 等照字面顯示的前綴
        // 維持原本的英文補全流程。
        let is_zhuyin_prefix = !long_literal && self.ascii.display_pending(&self.lexicon) != raw;
        if !is_zhuyin_prefix && !long_literal {
            for candidate in english_candidates(&self.personal, self.english(), &raw, limit) {
                let key = candidate.surface.to_lowercase();
                if seen.contains(&key) {
                    continue;
                }
                seen.push(key);
                self.candidates.push(candidate);
            }
        }
        self.candidates.truncate(limit);
    }

    /// 游標所在注音音節的候選。
    ///
    /// 顯示仍由統一管線決定；候選只取游標所在的那一個音節，不列整串詞，
    /// 也不跨過英文片段去猜前面中文的詞。沒有注音音節時不產生候選。
    fn rebuild_mixed_zh_candidates(&mut self, split: Option<(&[crate::mixed::Plan], &str)>) {
        // 游標在結尾時直接問統一管線：已完成注音的尾巴就是最後一個注音
        // 片段。這條路不依賴 `AsciiInput` 的 pending 狀態，`ej/ ` 這種
        // 空白當一聲、或前段是 ja 的混合 run 才不會只剩字面候選。
        if self.mixed_cursor.min(self.segments.len()) == self.segments.len() {
            if let Some(reading) =
                split.and_then(|(plans, text)| Self::planned_trailing_zh_reading(plans, text))
            {
                self.fill_zh_candidates(reading);
                return;
            }
        }
        let Some((_, _, syllable)) = self.mixed_zh_syllable() else {
            return;
        };
        self.fill_zh_candidates(syllable.as_str().to_string());
    }

    /// 最後一個輸出片段是完成的注音時，回傳尾巴那個音節的讀音。
    ///
    /// 只認結尾是聲調鍵或空白（一聲）的注音片段；打在音節中間的前綴仍走
    /// 原本的英文／注音前綴候選邏輯。
    fn planned_trailing_zh_reading(plans: &[crate::mixed::Plan], text: &str) -> Option<String> {
        let plan = plans.last()?;
        if plan.mode != Lang::Zh {
            return None;
        }
        let literal = &text[plan.start..plan.end];
        let last = literal.chars().last()?;
        let completed = last.is_whitespace()
            || Keyboard::standard()
                .symbol(last)
                .is_some_and(|symbol| class_of(symbol) == Class::Tone);
        if !completed {
            return None;
        }
        plan.zh
            .as_deref()?
            .split_whitespace()
            .next_back()
            .map(str::to_string)
    }

    /// 用一個注音讀音重建候選：詞格＋解碼器取最佳解，字面也保留。
    fn fill_zh_candidates(&mut self, reading: String) {
        let units = [reading.as_str()];
        let Ok(lattice) = Lattice::build(&units, &self.lexicon, &self.ctx) else {
            return;
        };
        let scorer = super::core::zh_scorer(&self.ctx, self.zh_lm.as_deref());
        let decoder = super::core::zh_decoder(&self.lexicon, &self.ctx, scorer.as_ref());
        let (hypotheses, _) = decoder.nbest(&lattice, &Locks::new(), false, self.max_candidates());
        let hypotheses = self.rank_zh_hypotheses(hypotheses, &reading);
        let literal = reading.clone();
        self.candidates = hypotheses
            .into_iter()
            .map(|hypothesis| {
                let source = if hypothesis.text == literal {
                    CandidateSource::Literal
                } else {
                    CandidateSource::Lexicon
                };
                Candidate::new(hypothesis.text, reading.clone(), hypothesis.cost, source)
            })
            .collect();
        self.candidates.truncate(self.max_candidates());
    }

    /// 已完成的注音音節轉成字面。
    pub(super) fn decode_syllable(&self, syllable: &Syllable) -> String {
        self.decode_reading(syllable.as_str())
    }

    /// 混合模式的完整字面：注音片段解碼，英數照字面，待定按鍵照顯示。
    pub(super) fn mixed_surface(&self) -> String {
        if let Some(text) = self.latin_surface() {
            return text;
        }
        self.mixed_surface_legacy()
    }

    /// 沒有啟用拉丁切分時的字面組合。
    fn mixed_surface_legacy(&self) -> String {
        let mut out = String::new();
        let mut run: Vec<String> = Vec::new();
        let flush = |run: &mut Vec<String>, out: &mut String| {
            if run.is_empty() {
                return;
            }
            let units: Vec<&str> = run.iter().map(String::as_str).collect();
            match Lattice::build(&units, &self.lexicon, &self.ctx) {
                Ok(lattice) => {
                    let decoder = Decoder::new(&self.lexicon, &self.ctx);
                    let (segs, _) = decoder.best(&lattice, &Locks::new(), false);
                    out.push_str(&decoder.conversion(&lattice, &segs, Lang::Zh).surface);
                }
                Err(_) => {
                    for unit in run.iter() {
                        out.push_str(unit);
                    }
                }
            }
            run.clear();
        };
        for piece in self.ascii.pieces() {
            match piece {
                AsciiPiece::Zh(syllable) => run.push(syllable.as_str().to_string()),
                AsciiPiece::Literal(text) => {
                    flush(&mut run, &mut out);
                    out.push_str(text);
                }
            }
        }
        flush(&mut run, &mut out);
        out.push_str(&self.ascii.display_pending(&self.lexicon));
        out
    }

    /// 統一管線的輸入文字：這次組字實際按過的完整按鍵流。
    ///
    /// 含已完成的注音、照字面的英數、空白與聲調；顯示與送出一律用同一條
    /// 管線重算，不再區分「已完成片段」與「待定按鍵」。
    fn latin_run_text(&self) -> Option<&str> {
        if self.ascii.literalized {
            return None;
        }
        let text = self.ascii.typed();
        if text.is_empty() {
            return None;
        }
        Some(text)
    }

    /// 逐鍵增量：用與 [`crate::mixed::Router::convert`] 相同的管線重算片段。
    fn incremental_latin_split(&self) -> Option<(Vec<crate::mixed::Plan>, String)> {
        // 文字必須活過 `apply_latin_split` 的可變借用，這裡只複製一次；
        // 其他讀取路徑都直接借用 `AsciiInput::typed`。
        let text = self.latin_run_text()?.to_string();
        let plans = self.plan_latin_text(&text);
        (!plans.is_empty()).then_some((plans, text))
    }

    /// 沒有增量結果時重算一次；提交路徑會用到。
    fn latin_surface(&self) -> Option<String> {
        let text = self.latin_run_text()?;
        let plans = self.plan_latin_text(text);
        if plans.is_empty() {
            return None;
        }
        let mut out = String::new();
        for piece in self.render_latin_plans(text, &plans) {
            out.push_str(&piece.surface);
        }
        Some(out)
    }

    /// 統一管線的規劃；長字面 run 只規劃尾巴。
    ///
    /// 超過 [`LONG_LITERAL_RUN`] 的純小寫 run 前段已經確定是字面英文，
    /// 凍結成一個英文片段，只把最後 [`PLAN_TAIL_CHARS`] 個字元交給
    /// `plan_input`。尾巴若接完成的注音音節，切點退到音節開頭，聲調鍵
    /// 才能把 `200 個 z + su3` 切成字面英文加「你」。
    ///
    /// 英數與標點混雜的長 run 走 [`Session::mixed_long_run_plans`]：前段同樣
    /// 凍結，`; ' , .` 不再有機會被當成注音韻母。超過
    /// [`MAX_COMPOSITION_CHARS`] 時一律只規劃尾巴，成本不再隨組字長度上升。
    fn plan_latin_text(&self, text: &str) -> Vec<crate::mixed::Plan> {
        let english = self.english();
        let adapter = crate::mixed_latin::LexiconAdapter {
            zh: Some(&self.lexicon),
            english: Some(english),
            ..Default::default()
        };
        let cfg = crate::mixed::Config::default();
        let lookup = english as &dyn crate::mixed::Lookup;
        let split_lex = &adapter as &dyn crate::mixed_latin::SplitLexicon;

        // 純小寫快速路徑保持原樣，先走它，不為其他形狀多掃一次。
        if let Some(tail_start) = self.long_run_tail_start(text) {
            return self.frozen_prefix_plans(text, tail_start, lookup, &cfg, split_lex);
        }
        // 撇號不是任何注音鍵，含撇號的英數 run 一定是字面（`abc;def'ghi`）。
        // 不交給切分器拆，才不會把中間的 `;` `,` `.` 當成韻母。
        if text.is_ascii() && text.contains('\'') {
            return self.frozen_prefix_plans(text, text.len(), lookup, &cfg, split_lex);
        }
        if let Some(plans) = self.mixed_long_run_plans(text, lookup, &cfg, split_lex) {
            return plans;
        }
        // 硬上限：任何超過上限的 ASCII 組字都只算尾巴，即使沒有偵測到長 run。
        if text.len() > MAX_COMPOSITION_CHARS && text.is_ascii() {
            let mut tail_start = text.len().saturating_sub(PLAN_TAIL_CHARS);
            if let Some((at, _)) =
                long_run_zh_boundary_mixed(text, self.ascii.pieces(), &self.lexicon)
            {
                if at > tail_start {
                    tail_start = at;
                }
            }
            return self.frozen_prefix_plans(text, tail_start, lookup, &cfg, split_lex);
        }
        let plans = crate::mixed::plan_input(text, lookup, &cfg, Some(split_lex));
        let plans = crate::mixed::visible_plans(plans, text);
        self.apply_literal_spans(plans)
    }

    /// 前段凍結成字面、只規劃尾巴的計畫。
    fn frozen_prefix_plans(
        &self,
        text: &str,
        tail_start: usize,
        lookup: &dyn crate::mixed::Lookup,
        cfg: &crate::mixed::Config,
        split_lex: &dyn crate::mixed_latin::SplitLexicon,
    ) -> Vec<crate::mixed::Plan> {
        let mut plans = Vec::new();
        plans.push(literal_en_plan(0, tail_start));
        for mut plan in crate::mixed::plan_input(&text[tail_start..], lookup, cfg, Some(split_lex))
        {
            plan.start += tail_start;
            plan.end += tail_start;
            plans.push(plan);
        }
        let mut plans = crate::mixed::visible_plans(plans, text);
        merge_adjacent_en_plans(&mut plans);
        self.apply_literal_spans(plans)
    }

    /// 英數、標點混雜的長 run：前段凍結，只保留完成的注音尾巴。
    ///
    /// 隨機英數加 `; ' , .` 不會被切成注音；`…;su3` 這種尾巴仍是「你」。
    /// 只處理從組字開頭延伸的字尾 run，其他形狀留給一般管線，避免猜錯前段。
    fn mixed_long_run_plans(
        &self,
        text: &str,
        lookup: &dyn crate::mixed::Lookup,
        cfg: &crate::mixed::Config,
        split_lex: &dyn crate::mixed_latin::SplitLexicon,
    ) -> Option<Vec<crate::mixed::Plan>> {
        if !text.is_ascii() {
            return None;
        }
        let (run_start, run_end) = last_printable_run(text)?;
        // 混合 run 從門檻本身（64）就凍結；純小寫仍維持 `>` 的既有行為。
        if run_start != 0 || run_end - run_start < LONG_LITERAL_RUN {
            return None;
        }
        // 純小寫 run 由既有的 `long_run_tail_start` 處理，這裡不重複。
        if text[..run_end]
            .bytes()
            .all(|byte| byte.is_ascii_lowercase())
        {
            return None;
        }
        // 尾巴是完成的注音音節時切成字面＋注音；其餘整段字面。
        if let Some((at, _)) = long_run_zh_boundary_mixed(text, self.ascii.pieces(), &self.lexicon)
        {
            if at > 0 && at < text.len() {
                return Some(self.frozen_prefix_plans(text, at, lookup, cfg, split_lex));
            }
        }
        let plans = vec![literal_en_plan(0, text.len())];
        let mut plans = crate::mixed::visible_plans(plans, text);
        merge_adjacent_en_plans(&mut plans);
        Some(self.apply_literal_spans(plans))
    }

    /// 把使用者固定成字面的按鍵範圍強制轉成英文字面片段。
    ///
    /// 選回原始按鍵（`ji3`）、無聲調按鍵（`ji`），或游標離開未完成的注音
    /// 時，管線不能再把它解成注音或日文。範圍以 `AsciiInput::typed` 的
    /// 位元組位移表示；注音片段若只有尾巴落在字面範圍，會在音節邊界切開，
    /// 前面的已完成音節仍照常轉字。
    fn apply_literal_spans(&self, plans: Vec<crate::mixed::Plan>) -> Vec<crate::mixed::Plan> {
        if self.literal_spans.is_empty() {
            return plans;
        }
        let spans: Vec<(usize, usize)> = self
            .literal_spans
            .iter()
            .copied()
            .filter(|(start, end)| start < end)
            .collect();
        if spans.is_empty() {
            return plans;
        }
        let text = self.ascii.typed();
        let mut out = Vec::with_capacity(plans.len());
        for plan in plans {
            let hits: Vec<(usize, usize)> = spans
                .iter()
                .copied()
                .filter(|(start, end)| *start < plan.end && plan.start < *end)
                .collect();
            if hits.is_empty() {
                out.push(plan);
                continue;
            }
            match self.split_zh_plan_for_literals(&plan, &hits, text) {
                Some(parts) => out.extend(parts),
                None => out.push(Self::literal_plan(plan.start, plan.end)),
            }
        }
        // `visible_plans` 可能把字面空白（或整個字面尾巴）當成片段之間的停頓
        // 吃掉；使用者明確按下的字面仍要輸出，缺的範圍補一個字面計畫。
        let mut inserted = false;
        for &(start, end) in &spans {
            let covered = out.iter().any(|plan| plan.start < end && start < plan.end);
            if covered {
                continue;
            }
            out.push(Self::literal_plan(start, end));
            inserted = true;
        }
        if inserted {
            out.sort_by_key(|plan| plan.start);
        }
        out
    }

    /// 只有尾巴被固定成字面的注音片段：依音節邊界切成「注音＋字面」。
    ///
    /// 切點對不上音節邊界時回傳 `None`，呼叫端維持整段字面的舊行為。
    fn split_zh_plan_for_literals(
        &self,
        plan: &crate::mixed::Plan,
        hits: &[(usize, usize)],
        text: &str,
    ) -> Option<Vec<crate::mixed::Plan>> {
        let literal = text.get(plan.start..plan.end)?;
        let readings: Vec<&str> = plan.zh.as_deref()?.split_whitespace().collect();
        let keys = zh_key_syllable_spans(literal);
        if readings.is_empty() || readings.len() != keys.len() {
            return None;
        }
        let mut cuts = vec![plan.start, plan.end];
        for (start, end) in hits {
            cuts.push((*start).max(plan.start));
            cuts.push((*end).min(plan.end));
        }
        cuts.sort_unstable();
        cuts.dedup();
        let mut out = Vec::new();
        for pair in cuts.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            if start >= end {
                continue;
            }
            let indices: Vec<usize> = keys
                .iter()
                .enumerate()
                .filter(|(_, (ks, ke))| plan.start + ks >= start && plan.start + ke <= end)
                .map(|(index, _)| index)
                .collect();
            let (Some(first), Some(last)) = (indices.first(), indices.last()) else {
                return None;
            };
            if plan.start + keys[*first].0 != start || plan.start + keys[*last].1 != end {
                return None;
            }
            let literal_hit = hits
                .iter()
                .any(|(hit_start, hit_end)| *hit_start <= start && end <= *hit_end);
            if literal_hit {
                out.push(Self::literal_plan(start, end));
            } else {
                out.push(crate::mixed::Plan {
                    start,
                    end,
                    mode: plan.mode,
                    script: plan.script,
                    kana: plan.kana.clone(),
                    kana_ok: plan.kana_ok,
                    verbatim: plan.verbatim,
                    zh: Some(
                        indices
                            .iter()
                            .map(|index| readings[*index])
                            .collect::<Vec<_>>()
                            .join(" "),
                    ),
                    zh_done: indices
                        .iter()
                        .map(|index| plan.zh_completed(*index))
                        .collect(),
                });
            }
        }
        (!out.is_empty()).then_some(out)
    }

    /// 一段照字面輸出的計畫。
    fn literal_plan(start: usize, end: usize) -> crate::mixed::Plan {
        crate::mixed::Plan {
            start,
            end,
            mode: Lang::En,
            script: Script::Latin,
            kana: String::new(),
            kana_ok: false,
            verbatim: true,
            zh: None,
            zh_done: Vec::new(),
        }
    }

    /// 長小寫 run 的凍結切點；沒有可凍結的前段時回傳 `None`。
    fn long_run_tail_start(&self, text: &str) -> Option<usize> {
        // 這條路徑只給純小寫 run；英數與標點混雜的長 run 走
        // [`Session::mixed_long_run_plans`]。
        if text.len() <= LONG_LITERAL_RUN || !text.is_ascii() {
            return None;
        }
        // 找最後一段連續小寫英文 run。
        let bytes = text.as_bytes();
        let mut found: Option<(usize, usize)> = None;
        let mut index = 0;
        while index < bytes.len() {
            if !bytes[index].is_ascii_lowercase() {
                index += 1;
                continue;
            }
            let start = index;
            while index < bytes.len() && bytes[index].is_ascii_lowercase() {
                index += 1;
            }
            if index - start > LONG_LITERAL_RUN {
                found = Some((start, index));
            }
        }
        let (run_start, run_end) = found?;
        let mut tail_start = text.len().saturating_sub(PLAN_TAIL_CHARS);
        if tail_start < run_start || tail_start > run_end {
            return None;
        }
        // 冷凍前段必須整段都是小寫英文；遇到注音、日文或標點就不凍結。
        if !text[..tail_start]
            .bytes()
            .all(|byte| byte.is_ascii_lowercase())
        {
            return None;
        }
        // 尾巴是完成的注音音節時，切在音節開頭，不要把音節留在字面裡。
        if let Some((zh_start, _)) = long_run_zh_boundary(text, &[], &self.lexicon) {
            if zh_start > tail_start && zh_start <= run_end {
                tail_start = zh_start;
            }
        }
        (tail_start > 0).then_some(tail_start)
    }

    /// 不修改狀態的統一管線版本，給診斷路徑用。
    fn incremental_latin_split_for(&self, text: &str) -> Option<Vec<crate::mixed::Plan>> {
        let plans = self.plan_latin_text(text);
        (!plans.is_empty()).then_some(plans)
    }

    /// 把統一管線的片段轉成顯示字面與讀音。
    fn render_latin_plans(&self, text: &str, plans: &[crate::mixed::Plan]) -> Vec<RenderedPiece> {
        plans
            .iter()
            .map(|plan| {
                let literal = &text[plan.start..plan.end];
                // 符號片段照字面輸出，與路由的 convert_plan 一致。
                if plan.script == Script::Symbol {
                    return RenderedPiece {
                        surface: literal.to_string(),
                        mode: plan.mode,
                        reading: String::new(),
                        units: 0,
                    };
                }
                match plan.mode {
                    Lang::Ja => {
                        if !plan.kana_ok || plan.kana.is_empty() {
                            RenderedPiece {
                                surface: literal.to_string(),
                                mode: Lang::Ja,
                                reading: String::new(),
                                units: 0,
                            }
                        } else {
                            RenderedPiece {
                                surface: self.decode_kana(&plan.kana),
                                mode: Lang::Ja,
                                reading: plan.kana.clone(),
                                units: plan.kana.chars().count(),
                            }
                        }
                    }
                    Lang::Zh => {
                        let (reading, surface) = self.zh_plan_surface(plan, literal);
                        let units = reading.split_whitespace().count();
                        RenderedPiece {
                            surface,
                            mode: Lang::Zh,
                            reading,
                            units,
                        }
                    }
                    _ => RenderedPiece {
                        surface: literal.to_string(),
                        mode: Lang::En,
                        reading: String::new(),
                        units: 0,
                    },
                }
            })
            .collect()
    }

    /// 把一個注音片段轉成（讀音, 顯示字面）。
    ///
    /// 沒有聲調鍵、也還沒按空白的音節還沒完成，照原始注音顯示；已完成的
    /// 連續音節照詞典一起轉換，尾巴與中間的未完成音節都不提早變成字。
    fn zh_plan_surface(&self, plan: &crate::mixed::Plan, literal: &str) -> (String, String) {
        let Some(reading) = plan.zh.as_deref() else {
            let reading = literal.to_string();
            let surface = self.decode_reading(&reading);
            return (reading, surface);
        };
        let mut surface = String::new();
        let mut out_reading: Vec<String> = Vec::new();
        let mut group: Vec<&str> = Vec::new();
        for (index, syllable) in reading.split_whitespace().enumerate() {
            if plan.zh_completed(index) {
                group.push(syllable);
                continue;
            }
            self.flush_zh_group(&mut group, &mut surface, &mut out_reading);
            surface.push_str(syllable);
            out_reading.push(syllable.to_string());
        }
        self.flush_zh_group(&mut group, &mut surface, &mut out_reading);
        (out_reading.join(" "), surface)
    }

    /// 轉換一整段已完成的注音音節並寫進顯示字面。
    fn flush_zh_group(
        &self,
        group: &mut Vec<&str>,
        surface: &mut String,
        out_reading: &mut Vec<String>,
    ) {
        if group.is_empty() {
            return;
        }
        let reading = group.join(" ");
        let toned = crate::mixed_latin::toned_syllables(&reading, &self.lexicon);
        surface.push_str(&self.decode_reading(&toned.join(" ")));
        out_reading.extend(toned);
        group.clear();
    }

    /// 把切分結果寫進 `base`、`segments` 與 [`Composition`]。
    ///
    /// 片段是 [`Owner::Auto`]，之後的轉換可以整段重算；日文與注音各自是
    /// 一個連續讀音區段。
    fn apply_latin_split(&mut self, plans: &[crate::mixed::Plan], text_input: &str) {
        let rendered = self.render_latin_plans(text_input, plans);
        let mut text = String::new();
        let mut segments = Vec::new();
        let mut pieces = Vec::new();
        for item in rendered {
            if item.surface.is_empty() {
                continue;
            }
            let start = text.len();
            text.push_str(&item.surface);
            let end = text.len();
            segments.push(Segment::new(start, end, item.mode, Script::Latin));
            let rune_start = crate::output::rune_len(&text[..start]);
            let rune_end = crate::output::rune_len(&text[..end]);
            let lang = match item.mode {
                Lang::Ja => CompLang::Ja,
                Lang::Zh => CompLang::Zh,
                _ => CompLang::Literal,
            };
            pieces.push(CompSegment::new(
                rune_start..rune_end,
                item.reading,
                item.units,
                lang,
                Owner::Auto,
            ));
        }
        let cursor = crate::output::rune_len(&text);
        self.composition
            .replace_segments(text.clone(), cursor, pieces);
        self.base = text;
        self.segments = segments;
    }

    /// 切分器產生的かな照原樣輸出。
    ///
    /// かな漢字変換留給後續候選；自動轉換會把非詞彙的組字（`りしれ`）誤轉成
    /// 單字（`里`），原型先不套用。
    fn decode_kana(&self, kana: &str) -> String {
        kana.to_string()
    }

    /// 已完成的注音讀音轉成字面；`reading` 以空白分隔音節。
    fn decode_reading(&self, reading: &str) -> String {
        let units: Vec<&str> = reading.split_whitespace().collect();
        if units.is_empty() {
            return reading.to_string();
        }
        match Lattice::build(&units, &self.lexicon, &self.ctx) {
            Ok(lattice) => {
                let decoder = Decoder::new(&self.lexicon, &self.ctx);
                let (segs, _) = decoder.best(&lattice, &Locks::new(), false);
                let surface = decoder.conversion(&lattice, &segs, Lang::Zh).surface;
                // 詞典詞的成本可能高於未知詞，最佳路徑會停在讀音字面。
                // 有真字時改顯示那條路徑，`ㄨㄢ` 才會是「彎」。
                let literal: String = units.concat();
                if surface != literal {
                    return surface;
                }
                let (hypotheses, _) =
                    decoder.nbest(&lattice, &Locks::new(), false, self.max_candidates().max(1));
                hypotheses
                    .into_iter()
                    .find(|hypothesis| {
                        hypothesis.text != literal && hypothesis.text.chars().any(is_han)
                    })
                    .map(|hypothesis| hypothesis.text)
                    .unwrap_or(surface)
            }
            Err(_) => reading.to_string(),
        }
    }
}

/// 一個切分片段轉成顯示字面後的結果。
struct RenderedPiece {
    surface: String,
    mode: Lang,
    reading: String,
    units: usize,
}

/// 英文片段的候選：學過的修正先排，前綴補完次之，拼字建議接在後面。
///
/// 對照前身 `core/session/english_autocomplete.go` 的
/// `refreshEnglishCandidates` 與 `core/session/mixed.go` 的
/// `openMixedCandidates`：先用內建詞表算出前綴補完與拼字建議，再問
/// [`crate::personal::Store`] 有沒有學過的修正；學過的排在內建候選前面。
/// 最多保留 3 個拼字建議，剩下的名額才給前綴補完，這樣補完不會把拼字建議
/// 擠掉。打對的完整單字不會有建議，建議保留查詢時的大小寫。
pub(super) fn english_candidates(
    personal: &crate::personal::Store,
    english: &English,
    raw: &str,
    limit: usize,
) -> Vec<Candidate> {
    if limit == 0 || raw.chars().count() < 2 || !raw.chars().all(|c| c.is_ascii_alphabetic()) {
        return Vec::new();
    }
    let mut seen = vec![raw.to_lowercase()];
    let mut corrections: Vec<Candidate> = Vec::new();
    for candidate in english.spelling_candidates(raw, limit.saturating_add(3)) {
        let key = candidate.surface.to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        corrections.push(candidate);
        if corrections.len() == 3 {
            break;
        }
    }
    let prefix_limit = limit.saturating_sub(corrections.len());
    let mut out: Vec<Candidate> = Vec::new();
    for candidate in english.completion_candidates(raw, limit.saturating_add(corrections.len())) {
        if out.len() == prefix_limit {
            break;
        }
        let key = candidate.surface.to_lowercase();
        if seen.contains(&key) || candidate.surface.chars().count() <= raw.chars().count() {
            continue;
        }
        seen.push(key);
        out.push(candidate);
    }
    out.extend(corrections);
    // 學過的修正優先；同一批候選內依權重排序，被跳過的仍然保留。
    let mut learned: Vec<Candidate> = Vec::new();
    for candidate in personal.english_candidates(raw) {
        let key = candidate.text.to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        learned.push(Candidate::new(
            candidate.text.clone(),
            candidate.text,
            0,
            CandidateSource::Personal,
        ));
    }
    learned.extend(out);
    learned.truncate(limit);
    learned
}

/// 把（候選, 動作）清單拆成候選與平行動作；同字面只留第一項。
fn split_choices(out: Vec<(Candidate, MixedChoice)>) -> (Vec<Candidate>, Vec<Option<MixedChoice>>) {
    let mut candidates = Vec::with_capacity(out.len());
    let mut choices = Vec::with_capacity(out.len());
    let mut seen: Vec<String> = Vec::new();
    for (candidate, choice) in out {
        if seen.contains(&candidate.surface) {
            continue;
        }
        seen.push(candidate.surface.clone());
        candidates.push(candidate);
        choices.push(Some(choice));
    }
    (candidates, choices)
}

/// 把注音讀音、原始按鍵、無聲調按鍵與日文解讀排進候選。
fn push_decomposition(
    out: &mut Vec<(Candidate, MixedChoice)>,
    reading: &str,
    raw: &str,
    raw_toneless: &str,
    tone_key: Option<char>,
) {
    let push = |out: &mut Vec<(Candidate, MixedChoice)>, surface: String, choice: MixedChoice| {
        if surface.is_empty() {
            return;
        }
        if out
            .iter()
            .any(|(candidate, _)| candidate.surface == surface)
        {
            return;
        }
        out.push((
            Candidate::new(surface, reading.to_string(), 0, CandidateSource::Literal),
            choice,
        ));
    };
    push(
        out,
        reading.to_string(),
        MixedChoice {
            insert: reading.to_string(),
            leftover: String::new(),
            force_literal: true,
            cursor: ChoiceCursor::AfterInsert,
            syllable_edit: None,
        },
    );
    if !raw.is_empty() {
        push(
            out,
            raw.to_string(),
            MixedChoice {
                insert: raw.to_string(),
                leftover: String::new(),
                force_literal: true,
                cursor: ChoiceCursor::Start,
                syllable_edit: None,
            },
        );
    }
    if !raw_toneless.is_empty() && raw_toneless != raw {
        push(
            out,
            raw_toneless.to_string(),
            MixedChoice {
                insert: raw_toneless.to_string(),
                leftover: tone_key.map(|key| key.to_string()).unwrap_or_default(),
                force_literal: true,
                cursor: ChoiceCursor::AfterInsert,
                syllable_edit: None,
            },
        );
    }
    // 同一串按鍵在日文解讀下成立的かな；聲調鍵原樣留在後面。
    if !raw_toneless.is_empty() {
        let config = crate::mixed::Config::default();
        if let Some((kana, pending)) = crate::mixed::kana_for(raw_toneless, &config, None) {
            if !kana.is_empty() && pending.is_empty() {
                let surface = format!(
                    "{}{}",
                    kana,
                    tone_key.map(|key| key.to_string()).unwrap_or_default()
                );
                push(
                    out,
                    surface.clone(),
                    MixedChoice {
                        insert: surface,
                        leftover: String::new(),
                        force_literal: false,
                        cursor: ChoiceCursor::AfterFirst,
                        syllable_edit: None,
                    },
                );
            }
        }
    }
}

/// 把注音按鍵還原成讀音（例如 `ji3` 的 `ㄐㄧˇ`？否，`ji` 是 ㄨㄛ）。
///
/// 只接受類別遞增的完整音節；不是注音按鍵時回傳 `None`。
fn zh_reading_from_keys(raw: &str) -> Option<String> {
    let keyboard = Keyboard::standard();
    let mut reading = String::new();
    let mut last = Class::None;
    for ch in raw.chars() {
        let symbol = keyboard.symbol(ch)?;
        let class = class_of(symbol);
        if class == Class::None || class <= last {
            return None;
        }
        reading.push(symbol);
        last = class;
    }
    if reading.is_empty() {
        return None;
    }
    Some(reading)
}

/// 把原始按鍵拆成無聲調按鍵與聲調鍵。
fn split_tone_key(raw: &str) -> (String, Option<char>) {
    let Some(last) = raw.chars().last() else {
        return (String::new(), None);
    };
    let is_tone = Keyboard::standard()
        .symbol(last)
        .is_some_and(|symbol| class_of(symbol) == Class::Tone);
    if !is_tone {
        return (raw.to_string(), None);
    }
    let toneless: String = raw.chars().take(raw.chars().count() - 1).collect();
    (toneless, Some(last))
}

/// 一段注音按鍵裡每個音節的位元組範圍。
///
/// 類別回到開頭（或遇到聲調鍵、空白）時開始下一個音節。長度對不上時整段
/// 當成一個音節，呼叫端仍可運作。
fn zh_key_syllable_spans(raw: &str) -> Vec<(usize, usize)> {
    let keyboard = Keyboard::standard();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut last = Class::None;
    let mut tone_seen = false;
    for (index, ch) in raw.char_indices() {
        let next = index + ch.len_utf8();
        if ch.is_whitespace() {
            if start < index {
                out.push((start, index));
            }
            start = next;
            last = Class::None;
            tone_seen = false;
            continue;
        }
        let Some(symbol) = keyboard.symbol(ch) else {
            // 非注音鍵把前一段收掉，自己不算音節。
            if start < index {
                out.push((start, index));
            }
            start = next;
            last = Class::None;
            tone_seen = false;
            continue;
        };
        let class = class_of(symbol);
        if class == Class::Tone {
            if start < next {
                out.push((start, next));
            }
            start = next;
            last = Class::None;
            tone_seen = true;
            continue;
        }
        if class <= last && start < index {
            out.push((start, index));
            start = index;
        }
        last = class;
        tone_seen = false;
    }
    let _ = tone_seen;
    if start < raw.len() {
        out.push((start, raw.len()));
    }
    if out.is_empty() && !raw.is_empty() {
        out.push((0, raw.len()));
    }
    out
}

/// 平假名、片假名與半形片假名。
fn is_kana(ch: char) -> bool {
    matches!(ch as u32, 0x3040..=0x30ff | 0xff66..=0xff9d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::{InMemoryLexicon, LexEntry};
    use crate::mixed::Router;

    fn typed(keys: &str) -> AsciiInput {
        let mut input = AsciiInput::new();
        for ch in keys.chars() {
            input.press(ch);
        }
        input
    }

    /// 測試用英文查詢層：原型詞加上指定詞，不讀系統上的英文資料包。
    fn test_english(extra: &[&str]) -> English {
        let mut words = prototype().words().to_vec();
        words.extend(extra.iter().map(|word| crate::mixed::Word::new(*word, 1.0)));
        English::new(Some(crate::mixed::Dictionary::new(words)))
    }

    /// 對照 `TestMixedTextKeepsLiteralAlnum`：只有數字與標點、英數反序、
    /// 英文詞前綴照字面；一般按鍵顯示注音。
    #[test]
    fn test_mixed_text_display_rules() {
        let lex = InMemoryLexicon::new();
        let english = test_english(&["hi"]);
        for (keys, want) in [
            ("123", "123"),
            ("2.", "2."),
            ("u2", "u2"),
            ("o1", "o1"),
            ("up", "up"),
            ("t", "ㄔ"),
            ("hi", "hi"),
        ] {
            assert_eq!(
                typed(keys).display_pending_with(&lex, &english),
                want,
                "display_pending({keys:?})"
            );
        }
        // 介音晚韻母一鍵是快打，照位置組字，不當英數寫法。
        assert!(swapped_vowels("2/u"));
    }

    struct TestJaSource(Vec<crate::ja::lexicon::JaEntry<'static>>);

    impl crate::ja::decode::JaWordSource for TestJaSource {
        fn longest_reading_units(&self) -> usize {
            self.0.iter().map(|entry| entry.units()).max().unwrap_or(0)
        }

        fn matches<'a>(
            &'a self,
            reading: &[char],
            start: usize,
            out: &mut dyn FnMut(crate::ja::decode::JaWordMatch<'a>),
        ) {
            for entry in &self.0 {
                let units: Vec<char> = entry.reading.chars().collect();
                let end = start + units.len();
                if end > reading.len() || reading[start..end] != units[..] {
                    continue;
                }
                out(crate::ja::decode::JaWordMatch {
                    end,
                    entry: entry.clone(),
                    left_id: None,
                    right_id: None,
                });
            }
        }

        fn exact<'a>(&'a self, reading: &[char]) -> Vec<crate::ja::lexicon::JaEntry<'a>> {
            self.0
                .iter()
                .filter(|entry| entry.reading.chars().eq(reading.iter().copied()))
                .cloned()
                .collect()
        }

        fn prefixed<'a>(&'a self, reading: &[char]) -> Vec<crate::ja::lexicon::JaEntry<'a>> {
            self.0
                .iter()
                .filter(|entry| {
                    entry
                        .reading
                        .chars()
                        .take(reading.len())
                        .eq(reading.iter().copied())
                })
                .cloned()
                .collect()
        }
    }

    /// 對照 `TestJapaneseRomajiComposesKanaWithoutModeSwitch`：設定有日文
    /// 解碼器時，完整羅馬字自動走日文，送出後回到混合模式。
    #[test]
    fn test_japanese_run_infers_from_romaji() {
        use std::sync::Arc;

        let entry = crate::ja::lexicon::JaEntry {
            surface: "今日".into(),
            reading: "きょう".into(),
            pos: "名詞,普通名詞,一般,*".into(),
            conj_type: "*".into(),
            conj_form: "*".into(),
            cost: 0,
            split: crate::ja::lexicon::Split::C,
        };
        let decoder = crate::ja::decode::JaDecoder::new(Arc::new(TestJaSource(vec![entry])));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.ja = Some(Arc::new(decoder));
        let mut s = Session::new(config);

        for ch in "hello".chars() {
            s.press(Key::char(ch, false));
        }
        assert_eq!(s.mode(), Mode::Mixed, "英文詞不轉日文");
        assert_eq!(s.snapshot().preedit, "hello");

        let mut s = Session::new({
            let mut config = SessionConfig::new(Mode::Mixed);
            let entry = crate::ja::lexicon::JaEntry {
                surface: "私".into(),
                reading: "わたし".into(),
                pos: "名詞,普通名詞,一般,*".into(),
                conj_type: "*".into(),
                conj_form: "*".into(),
                cost: 0,
                split: crate::ja::lexicon::Split::C,
            };
            config.ja = Some(Arc::new(crate::ja::decode::JaDecoder::new(Arc::new(
                TestJaSource(vec![entry]),
            ))));
            config
        });
        for ch in "watashi".chars() {
            s.press(Key::char(ch, false));
        }
        // 日文片段沿用延遲整句：停頓後才顯示轉換結果。
        s.ja_live_idle();
        assert_eq!(s.mode(), Mode::Mixed, "日文片段不再切換輸入模式");
        assert_eq!(s.snapshot().preedit, "私");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "私");
        assert_eq!(s.mode(), Mode::Mixed);
    }

    /// `Mode::Mixed` 下 `watashi` 走羅馬字路徑出假名，空白與後續按鍵仍可
    /// 繼續輸入；拉丁鍵被切成注音是混打路由的設計，不是 bug。
    #[test]
    fn test_watashi_continues_in_mixed_mode() {
        let mut s = Session::new(SessionConfig::new(Mode::Mixed));
        for ch in "watashi".chars() {
            s.press(Key::char(ch, false));
        }
        assert_eq!(s.mode(), Mode::Mixed);
        assert_eq!(s.snapshot().preedit, "わたし");

        s.press(Key::new(KeyCode::Space, None, false));
        assert_eq!(s.snapshot().preedit, "わたし ");
        for ch in "desu".chars() {
            s.press(Key::char(ch, false));
        }
        assert_eq!(s.snapshot().preedit, "わたしです");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "わたしです");
    }

    /// 混合模式的按鍵輸入；空白走 `Space`，其餘照字元。
    fn mixed_type(session: &mut Session, keys: &str) {
        for ch in keys.chars() {
            let key = if ch == ' ' {
                Key::new(KeyCode::Space, None, false)
            } else {
                Key::char(ch, false)
            };
            session.press(key);
        }
    }

    /// 注音片段已經轉成詞（`台`）時，Backspace 刪掉整個片段而不是退回
    /// 注音。使用者看到什麼就刪什麼；退回 `ㄊㄞ` 會讓畫面多出沒看過的字。
    #[test]
    fn test_mixed_backspace_deletes_converted_syllable() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("台", "ㄊㄞˊ", 0, 0));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        let mut s = Session::new(config);
        mixed_type(&mut s, "w96");
        assert_eq!(s.snapshot().preedit, "台");

        let snap = s.press(Key::new(KeyCode::Backspace, None, false));
        assert_eq!(snap.preedit, "", "整個已轉換的片段被刪掉");
        assert_eq!(snap.cursor_rune, 0);
    }

    /// 英文字面片段逐 rune 退格，不是整段刪。
    #[test]
    fn test_mixed_backspace_deletes_one_english_rune() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        // 詞表要明確給：預設詞表依機器上有沒有英文資料包而不同，
        // `hell` 不在內建原型裡，靠外部詞表才會過。
        config.english = Some(std::sync::Arc::new(test_english(&["hello", "hell"])));
        let mut s = Session::new(config);
        mixed_type(&mut s, "ji3hello");
        assert_eq!(s.snapshot().preedit, "我hello");

        let snap = s.press(Key::new(KeyCode::Backspace, None, false));
        assert_eq!(snap.preedit, "我hell", "退格只刪游標前一個 rune");
    }

    /// 長串小寫字母不能停在詞表窗口；每個按鍵都要反映在 preedit。
    #[test]
    fn test_mixed_long_lowercase_run_keeps_every_key() {
        for len in [70usize, 200] {
            let mut s = Session::new(SessionConfig::new(Mode::Mixed));
            mixed_type(&mut s, &"a".repeat(len));
            let preedit = s.snapshot().preedit;
            assert_eq!(preedit.chars().count(), len, "{len} 個 a 要完整顯示");
            assert!(
                preedit.chars().all(|ch| ch == 'a'),
                "{len} 個 a：{preedit:?}"
            );
        }

        let mut s = Session::new(SessionConfig::new(Mode::Mixed));
        mixed_type(&mut s, &"z".repeat(70));
        assert_eq!(s.snapshot().preedit, "z".repeat(70), "z 的長 run 也要完整");
    }

    /// 長串英文後面再打字母要繼續變長，不能卡在窗口尾端。
    #[test]
    fn test_mixed_long_lowercase_run_accepts_more_letters() {
        let mut s = Session::new(SessionConfig::new(Mode::Mixed));
        mixed_type(&mut s, &"a".repeat(70));
        mixed_type(&mut s, "b");
        let preedit = s.snapshot().preedit;
        assert_eq!(preedit.chars().count(), 71);
        assert_eq!(preedit, "a".repeat(70) + "b");

        // 使用者原始回報：按住 z，之後打 a、b 都要接在後面。
        let mut s = Session::new(SessionConfig::new(Mode::Mixed));
        mixed_type(&mut s, &"z".repeat(70));
        mixed_type(&mut s, "ab");
        assert_eq!(s.snapshot().preedit.chars().count(), 72);
        assert_eq!(s.snapshot().preedit, "z".repeat(70) + "ab");

        // 標點走符號片段；標點後面的字母仍要能接上。
        for ch in ['z', 'a'] {
            let mut s = Session::new(SessionConfig::new(Mode::Mixed));
            mixed_type(&mut s, &ch.to_string().repeat(70));
            mixed_type(&mut s, ".a");
            assert_eq!(
                s.snapshot().preedit,
                ch.to_string().repeat(70) + ".a",
                "{ch} 長 run 後的標點與字母"
            );
        }
    }

    /// 長串英數裡的 `; ' , .` 照字面，不得被當成注音韻母。
    #[test]
    fn test_long_ascii_run_keeps_punctuation_literal() {
        fn assert_literal(keys: &str) {
            let mut s = Session::new(SessionConfig::new(Mode::Mixed));
            mixed_type(&mut s, keys);
            let preedit = s.snapshot().preedit;
            assert_eq!(preedit, keys, "長串英數要原樣字面：{keys:?}");
            assert!(
                !preedit
                    .chars()
                    .any(|ch| ('\u{3100}'..='\u{312f}').contains(&ch)),
                "不得出現注音字元：{preedit:?}"
            );
        }

        assert_literal("abc;def'ghi,jkl.mno");
        // 超過長 run 門檻後仍要整段字面。
        let long: String = "abc;def'ghi,jkl.mno/".repeat(6);
        assert_literal(&long[..long.len() - 5]);
    }

    /// 長串尾巴退格一次只掉一個字元。
    #[test]
    fn test_long_run_backspace_still_removes_one_rune() {
        let keys = "abc;def'ghi,jkl.mno/".repeat(6);
        let mut s = Session::new(SessionConfig::new(Mode::Mixed));
        mixed_type(&mut s, &keys);
        let snap = s.press(Key::new(KeyCode::Backspace, None, false));
        assert_eq!(snap.preedit, &keys[..keys.len() - 1]);
        assert_eq!(snap.preedit.chars().count(), keys.chars().count() - 1);
    }

    /// 日文解碼器開啟時，已經字面化的長 run 不能因為下一個字母改寫成假名。
    #[test]
    fn test_mixed_long_literal_run_not_reinterpreted_as_japanese() {
        use std::sync::Arc;

        let decoder = crate::ja::decode::JaDecoder::new(Arc::new(TestJaSource(Vec::new())));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.ja = Some(Arc::new(decoder));
        let mut s = Session::new(config);

        mixed_type(&mut s, &"z".repeat(70));
        assert_eq!(s.snapshot().preedit, "z".repeat(70));
        mixed_type(&mut s, "a");
        assert_eq!(s.snapshot().preedit, "z".repeat(70) + "a");
        mixed_type(&mut s, "b");
        assert_eq!(s.snapshot().preedit, "z".repeat(70) + "ab");
    }

    /// 已有混打前綴時，後面的長小寫 run 仍要完整切分，不能吃掉尾巴。
    #[test]
    fn test_mixed_prefix_then_long_run_keeps_every_key() {
        let mut s = Session::new(SessionConfig::new(Mode::Mixed));
        mixed_type(&mut s, "sa");
        mixed_type(&mut s, &"a".repeat(70));
        let preedit = s.snapshot().preedit;
        assert_eq!(preedit.chars().count(), 72);
        assert_eq!(preedit, "sa".to_string() + &"a".repeat(70));
    }

    /// 混合模式打完注音後要有中文候選；↓ 進入選取模式，空白開候選窗。
    #[test]
    fn test_mixed_candidates_open_on_down_and_space() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("台", "ㄊㄞˊ", 0, 0));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        let mut s = Session::new(config);
        mixed_type(&mut s, "w96");
        let snap = s.snapshot();
        assert_eq!(snap.preedit, "台");
        assert!(!snap.candidates.is_empty(), "已完成的注音要有候選");
        assert!(!s.candidate_window);

        let down = s.press(Key::new(KeyCode::Down, None, false));
        assert!(s.candidate_window, "↓ 開候選窗");
        assert!(down.navigating, "↓ 進入選取模式");
        assert_eq!(down.highlighted, 0, "第一下不移動反白");

        let mut s = Session::new({
            let mut config = SessionConfig::new(Mode::Mixed);
            let mut lex = InMemoryLexicon::new();
            lex.push(LexEntry::new("台", "ㄊㄞˊ", 0, 0));
            config.zh_lexicon = Some(std::sync::Arc::new(lex));
            config
        });
        mixed_type(&mut s, "w96");
        let space = s.press(Key::new(KeyCode::Space, None, false));
        assert!(!s.candidate_window, "空白不開候選窗");
        assert!(!space.navigating, "空白不進入選取模式");
        assert_eq!(space.preedit, "台 ", "空白只插入字面空白");
    }

    /// 混合模式選中文候選只換結尾的注音片段，前面已固定的內容保留。
    #[test]
    fn test_mixed_selects_zh_candidate_in_place() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("台", "ㄊㄞˊ", 0, 0));
        lex.push(LexEntry::new("颱", "ㄊㄞˊ", 1, 0));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        let mut s = Session::new(config);
        mixed_type(&mut s, "w96");
        let index = s
            .candidates()
            .iter()
            .position(|candidate| candidate.surface == "颱")
            .expect("要有颱候選");
        assert!(index < PAGE_SIZE, "測試候選要在第一頁");
        s.press(Key::new(KeyCode::Down, None, false));
        let digit = char::from(b'1' + index as u8);
        s.press(Key::char(digit, false));
        let snap = s.snapshot();
        assert_eq!(snap.preedit, "颱", "選字只換結尾的注音片段");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "颱");
    }

    /// 日文片段卡在那裡；`watashiwaw96j0` 後面的注音與空白都要能繼續輸入。
    #[test]
    fn test_mixed_ja_run_falls_back_on_non_romaji_key() {
        use std::sync::Arc;

        let entry = crate::ja::lexicon::JaEntry {
            surface: "今日".into(),
            reading: "きょう".into(),
            pos: "名詞,普通名詞,一般,*".into(),
            conj_type: "*".into(),
            conj_form: "*".into(),
            cost: 0,
            split: crate::ja::lexicon::Split::C,
        };
        let decoder = crate::ja::decode::JaDecoder::new(Arc::new(TestJaSource(vec![entry])));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.ja = Some(Arc::new(decoder));
        let mut s = Session::new(config);
        mixed_type(&mut s, "watashiwaw96j0");
        let snap = s.snapshot();
        assert_eq!(s.mode(), Mode::Mixed, "數字鍵要結束日文推斷");
        assert!(
            snap.preedit.starts_with("わたしわ"),
            "已顯示的日文要保留：{}",
            snap.preedit
        );
        assert!(
            !snap.preedit.ends_with('w'),
            "不能停在未完成的羅馬字：{}",
            snap.preedit
        );
        assert!(
            snap.preedit.contains("ㄊㄞ"),
            "w96 要接成注音：{}",
            snap.preedit
        );

        let before = snap.preedit;
        mixed_type(&mut s, " bp6");
        let after = s.snapshot().preedit;
        assert_ne!(after, before, "後續按鍵仍要能輸入");
        assert!(after.contains("ㄖㄣ"), "bp6 要接成下一個音節：{after}");
    }

    /// 混合模式的方向鍵逐 rune 移動，跨過片段邊界也照樣一格一格走。
    #[test]
    fn test_mixed_arrows_move_between_segments() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        let mut s = Session::new(config);
        mixed_type(&mut s, "ji3hello");
        let typed = s.snapshot();
        assert_eq!(typed.preedit, "我hello");
        assert_eq!(typed.cursor_rune, 6);

        for (want, label) in [
            (5, "← 進到英文片段結尾"),
            (4, "← 逐字母"),
            (3, "← 逐字母"),
            (2, "← 逐字母"),
            (1, "← 移到英文片段開頭"),
            (0, "← 移到注音字"),
        ] {
            let left = s.press(Key::new(KeyCode::ArrowLeft, None, false));
            assert_eq!(left.cursor_rune, want, "{label}");
            assert_eq!(left.preedit, "我hello", "移動游標不改內容");
        }
        for (want, label) in [(1, "→ 回到英文片段開頭"), (2, "→ 逐字母")] {
            let right = s.press(Key::new(KeyCode::ArrowRight, None, false));
            assert_eq!(right.cursor_rune, want, "{label}");
        }
        for _ in 0..4 {
            s.press(Key::new(KeyCode::ArrowRight, None, false));
        }
        assert_eq!(s.snapshot().cursor_rune, 6, "→ 回到整段結尾");
    }

    /// 游標停在英文片段前時，Delete 只刪游標後那一個 rune。
    #[test]
    fn test_mixed_delete_forward_removes_segment_after_cursor() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        // 同 `test_mixed_backspace_deletes_one_english_rune`：詞表要明確給。
        config.english = Some(std::sync::Arc::new(test_english(&["hello", "hell"])));
        let mut s = Session::new(config);
        mixed_type(&mut s, "ji3hello");
        s.press(Key::new(KeyCode::ArrowLeft, None, false));
        assert_eq!(s.snapshot().cursor_rune, 5);

        let snap = s.press(Key::new(KeyCode::Delete, None, false));
        assert_eq!(snap.preedit, "我hell", "Delete 刪掉游標後的 o");
        assert_eq!(snap.cursor_rune, 5);
    }

    /// 對照 `language_learning.go`：混打送出時把注音片段記成中文、
    /// 照字面的英數片段記成英文。
    #[test]
    fn test_mixed_commit_records_language_selection() {
        use crate::personal::LanguageId;

        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        let mut s = Session::new(config);
        for ch in "ji3".chars() {
            s.press(Key::char(ch, false));
        }
        s.press(Key::new(KeyCode::Enter, None, false));
        let text = s.commit();
        assert_eq!(text, "我");
        assert_eq!(s.personal().language_surface_count(LanguageId::Zh, "我"), 1);

        let mut s = Session::new(SessionConfig::new(Mode::Mixed));
        for ch in "hello".chars() {
            s.press(Key::char(ch, false));
        }
        s.press(Key::new(KeyCode::Space, None, false));
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "hello ");
        assert_eq!(
            s.personal().language_surface_count(LanguageId::En, "hello"),
            1
        );
    }

    /// 拉丁切分器開啟時，`rishireej/ savul3` 送出 `りしれ供さ小`。
    #[test]
    fn test_mixed_latin_split_commit() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("供", "ㄍㄨㄥ", 0, 0));
        lex.push(LexEntry::new("小", "ㄒㄧㄠˇ", 0, 0));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        let mut s = Session::new(config);
        for ch in "rishireej/".chars() {
            s.press(Key::char(ch, false));
        }
        s.press(Key::new(KeyCode::Space, None, false));
        for ch in "savul3".chars() {
            s.press(Key::char(ch, false));
        }
        assert_eq!(s.snapshot().preedit, "りしれ供さ小");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "りしれ供さ小");
    }

    /// 未完成的注音不能先轉成字；聲調或空白一聲才轉。
    ///
    /// 真實使用者的回報：`j`、`0` 停在 `ㄨㄢ`，空白補一聲後 preedit 與
    /// commit 都要是「彎」。空白不可以變成字面空格，也不可以只開候選窗。
    /// 詞典成本高於未知詞成本時仍然要選詞典的字，不能退回原始注音。
    #[test]
    fn test_mixed_untoned_zhuyin_converts_only_after_tone() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("彎", "ㄨㄢ", 100_000, 0));
        lex.push(LexEntry::new("灣", "ㄨㄢ", 100_001, 0));
        lex.push(LexEntry::new("台", "ㄊㄞˊ", 0, 0));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        let mut s = Session::new(config);

        mixed_type(&mut s, "j0");
        assert_eq!(s.snapshot().preedit, "ㄨㄢ", "沒有聲調鍵前只能顯示原始注音");
        assert!(!s.snapshot().preedit.contains('彎'));

        let snap = s.press(Key::new(KeyCode::Space, None, false));
        assert_eq!(snap.preedit, "彎", "空白是一聲，要轉成字");
        assert!(!s.candidate_window, "未完成注音補完時不開候選窗");
        assert_eq!(s.commit(), "彎", "commit 也要是轉換後的字");

        // 有聲調鍵的既有行為不變：`w96` 是 ㄊㄞˊ。
        let mut config = SessionConfig::new(Mode::Mixed);
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("台", "ㄊㄞˊ", 0, 0));
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        let mut s = Session::new(config);
        mixed_type(&mut s, "w96");
        assert_eq!(s.snapshot().preedit, "台");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "台");
    }

    /// 有競爭者的詞典：多個 ㄍㄨㄥ 詞、`りしれえ` 這類也合法的日文前綴。
    fn competitive_lexicon() -> InMemoryLexicon {
        let mut lex = InMemoryLexicon::new();
        for (surface, reading, cost) in [
            ("供", "ㄍㄨㄥ", 0),
            ("公", "ㄍㄨㄥ", 1),
            ("工", "ㄍㄨㄥ", 2),
            ("功", "ㄍㄨㄥ", 3),
            ("小", "ㄒㄧㄠˇ", 0),
            ("曉", "ㄒㄧㄠˇ", 1),
            ("翁", "ㄨㄥ", 0),
        ] {
            lex.push(LexEntry::new(surface, reading, cost, 0));
        }
        lex
    }

    /// 日文解碼器：真實機器一定載入；會讓 `rishire` 先走日文推斷，
    /// 再從 `rishiree` 退回混合模式，增量切分才走得到 bug。
    fn test_ja_decoder() -> std::sync::Arc<crate::ja::decode::JaDecoder> {
        let entry = crate::ja::lexicon::JaEntry {
            surface: "今日".into(),
            reading: "きょう".into(),
            pos: "名詞,普通名詞,一般,*".into(),
            conj_type: "*".into(),
            conj_form: "*".into(),
            cost: 0,
            split: crate::ja::lexicon::Split::C,
        };
        std::sync::Arc::new(crate::ja::decode::JaDecoder::new(std::sync::Arc::new(
            TestJaSource(vec![entry]),
        )))
    }

    #[test]
    fn test_mixed_latin_split_commit_with_competitors() {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(competitive_lexicon()));
        config.ja = Some(test_ja_decoder());
        let mut s = Session::new(config);
        for ch in "rishireej/".chars() {
            s.press(Key::char(ch, false));
        }
        assert_eq!(
            s.snapshot().preedit,
            "りしれㄍㄨㄥ",
            "`rishiree` 不能整段留在日文：e 要還給 ej/；沒有聲調鍵的 ㄍㄨㄥ 不提早轉字"
        );
        s.press(Key::new(KeyCode::Space, None, false));
        assert_eq!(s.snapshot().preedit, "りしれ供", "空白是一聲，不是分隔字面");
        let after_space = s.snapshot().candidates;
        assert!(
            after_space
                .iter()
                .any(|candidate| candidate.surface == "供"),
            "空白後ㄍㄨㄥ要有詞典候選：{:?}",
            after_space
                .iter()
                .map(|candidate| candidate.surface.clone())
                .collect::<Vec<_>>()
        );
        for ch in "savul3".chars() {
            s.press(Key::char(ch, false));
        }
        assert_eq!(s.snapshot().preedit, "りしれ供さ小");
        let candidates: Vec<String> = s
            .snapshot()
            .candidates
            .iter()
            .map(|candidate| candidate.surface.clone())
            .collect();
        assert!(candidates.iter().any(|text| text == "小"), "{candidates:?}");
        assert!(
            candidates.iter().any(|text| text == "曉"),
            "同音競爭者也要在候選裡：{candidates:?}"
        );
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "りしれ供さ小");
    }

    /// 第二例：`rishireej/ sasmall` 的逐鍵切分要與批次規劃一致。
    #[test]
    fn test_mixed_latin_split_sasmall_matches_batch() {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(competitive_lexicon()));
        config.ja = Some(test_ja_decoder());
        let mut s = Session::new(config);
        mixed_type(&mut s, "rishireej/ sasmall");
        let snap = s.snapshot();
        assert_eq!(snap.preedit, "りしれ供さsmall");
        let got: Vec<(Lang, String)> = s
            .segments()
            .iter()
            .map(|segment| {
                (
                    segment.mode,
                    snap.preedit[segment.start..segment.end].to_string(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                (Lang::Ja, "りしれ".to_string()),
                (Lang::Zh, "供".to_string()),
                (Lang::Ja, "さ".to_string()),
                (Lang::En, "small".to_string()),
            ],
            "批次規劃是 rishire｜ej/ ｜sa｜small"
        );
    }

    /// 混打路徑的未完成注音不能提早轉字。
    ///
    /// 真實使用者的回報：`saej/` 應該停在 `さㄍㄨㄥ`，不是 `さ供`。尾巴與
    /// 中間的注音音節都一樣，只有聲調鍵或空白一聲才轉成字。
    #[test]
    fn test_mixed_unfinished_zhuyin_stays_bopomofo() {
        fn session() -> Session {
            let mut lex = InMemoryLexicon::new();
            lex.push(LexEntry::new("家", "ㄍㄨ", 0, 0));
            lex.push(LexEntry::new("供", "ㄍㄨㄥ", 0, 0));
            let mut config = SessionConfig::new(Mode::Mixed);
            config.zh_lexicon = Some(std::sync::Arc::new(lex));
            Session::new(config)
        }

        let mut s = session();
        mixed_type(&mut s, "saej");
        assert_eq!(
            s.snapshot().preedit,
            "さㄍㄨ",
            "沒有聲調鍵的 ㄍㄨ 要留在注音"
        );

        let mut s = session();
        mixed_type(&mut s, "saej/");
        assert_eq!(
            s.snapshot().preedit,
            "さㄍㄨㄥ",
            "尾巴的 ㄍㄨㄥ 還沒完成，不能變成供"
        );
        assert_ne!(s.snapshot().preedit, "さ供");
        assert_eq!(s.commit(), "さㄍㄨㄥ", "空白前送出也不能提早轉字");

        let mut s = session();
        mixed_type(&mut s, "saej/");
        s.press(Key::new(KeyCode::Space, None, false));
        assert_eq!(s.snapshot().preedit, "さ供", "空白當一聲後才轉成詞典的字");
    }

    /// 真實引擎會載入日文解碼器：`saej` 可能停在 `さえj`，一樣不能先變漢字。
    ///
    /// 不論 `j` 之後停在原始羅馬字或混打重算的 `さㄍㄨ`，重點是 `/` 之後仍
    /// 是原始注音，只有空白才轉成「供」。
    #[test]
    fn test_mixed_unfinished_zhuyin_with_ja_decoder() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("家", "ㄍㄨ", 0, 0));
        lex.push(LexEntry::new("供", "ㄍㄨㄥ", 0, 0));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        config.ja = Some(test_ja_decoder());
        let mut s = Session::new(config);

        mixed_type(&mut s, "saej");
        let preedit = s.snapshot().preedit;
        assert!(
            preedit == "さえj" || preedit == "さㄍㄨ",
            "j 之後只能停在原始按鍵或注音：{preedit:?}"
        );
        assert!(
            !preedit.contains('家') && !preedit.contains('供'),
            "還沒按聲調或空白就變成漢字：{preedit:?}"
        );

        mixed_type(&mut s, "/");
        assert_eq!(
            s.snapshot().preedit,
            "さㄍㄨㄥ",
            "/ 只是韻母鍵，不能提早轉成供"
        );

        mixed_type(&mut s, " ");
        assert_eq!(s.snapshot().preedit, "さ供", "空白一聲才轉字");
    }

    /// 中間與尾巴的未完成注音都不轉字，已完成的前段仍照常轉換。
    #[test]
    fn test_mixed_unfinished_zhuyin_in_middle_stays_bopomofo() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("台", "ㄊㄞˊ", 0, 0));
        lex.push(LexEntry::new("彎", "ㄨㄢ", 0, 0));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        let mut s = Session::new(config);

        mixed_type(&mut s, "w96j0");
        assert_eq!(s.snapshot().preedit, "台ㄨㄢ", "完成的前段照常轉字");

        let mut s = Session::new({
            let mut lex = InMemoryLexicon::new();
            lex.push(LexEntry::new("台", "ㄊㄞˊ", 0, 0));
            lex.push(LexEntry::new("彎", "ㄨㄢ", 0, 0));
            let mut config = SessionConfig::new(Mode::Mixed);
            config.zh_lexicon = Some(std::sync::Arc::new(lex));
            config
        });
        mixed_type(&mut s, "j0w96");
        assert_eq!(
            s.snapshot().preedit,
            "ㄨㄢ台",
            "中間的 ㄨㄢ 沒有聲調，也不能先轉成彎"
        );
    }

    /// 對照 `openMixedCandidates`：原按鍵之外列注音本身與結尾聲調的注音。
    #[test]
    fn test_mixed_reading_candidates() {
        let lex = InMemoryLexicon::new();
        let co = typed("co");
        assert_eq!(co.reading_candidate("co", &lex).as_deref(), Some("ㄏㄟ"));
        let co6 = typed("co6");
        assert_eq!(co6.reading_candidate("co6", &lex).as_deref(), Some("ㄏㄟˊ"));
        let u2 = typed("u2");
        assert_eq!(
            u2.reading_candidate("u2", &lex),
            Some(compose_keys("u2").unwrap().string())
        );
        let hello = typed("hello");
        assert_eq!(hello.reading_candidate("hello", &lex), None);
    }

    /// 對照 `english_autocomplete.go`：完整英文詞沒有補全，打錯一鍵有更正。
    /// 詞表查詢有長度上限，但輸入本身不能因為上限丟掉按鍵。
    #[test]
    fn test_english_autocomplete_limits() {
        assert_eq!(typed("hellp").suggestion().as_deref(), Some("hello"));
        assert_eq!(typed("hello").suggestion(), None);
        let long = "a".repeat(crate::mixed::MAX_WORD_BYTES + 8);
        let input = typed(&long);
        assert_eq!(
            input.pending().len(),
            long.len(),
            "超過詞表上限的按鍵仍要留在輸入裡"
        );
        assert_eq!(input.suggestion(), None, "超長 run 不做詞表補完");
    }

    /// 對照 `finishMixedTone` 接空白：照字面留下的按鍵補空白，顯示成注音的
    /// 單一按鍵保留注音，空白當一聲吃掉。
    #[test]
    fn test_mixed_space_keeps_literal_or_phonetic() {
        let lex = InMemoryLexicon::new();
        for (keys, want) in [
            ("5pm ", "5pm "),
            ("2026 ", "2026 "),
            ("3.5 ", "3.5 "),
            ("c ", "ㄏ"),
            ("hello ", "hello "),
        ] {
            let mut input = typed(keys.trim_end());
            input.space(&lex);
            assert_eq!(input.display(&lex), want, "space({keys:?})");
        }
    }

    /// 對照 `mixedText` 的一聲保護：`up` 同時是英文詞前綴與 ㄧㄣ 時，
    /// 有中文讀音就顯示注音。
    #[test]
    fn test_mixed_text_prefers_reachable_first_tone() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("因", "ㄧㄣ", 0, 0));
        assert_eq!(typed("up").display_pending(&lex), "ㄧㄣ");
    }

    /// 三個音節的測試詞典：台灣人，另收同音的仁當第二個候選。
    fn taiwan_ren_lexicon() -> InMemoryLexicon {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("台", "ㄊㄞˊ", 0, 0));
        lex.push(LexEntry::new("灣", "ㄨㄢˊ", 0, 0));
        lex.push(LexEntry::new("人", "ㄖㄣˊ", 0, 0));
        lex.push(LexEntry::new("仁", "ㄖㄣˊ", 1, 0));
        lex
    }

    fn taiwan_ren_session(lex: InMemoryLexicon) -> Session {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        Session::new(config)
    }

    /// 游標在結尾按 ↓ 只列最後一個音節的候選，不再列整串詞候選。
    #[test]
    fn mixed_down_lists_only_the_last_syllable() {
        let mut s = taiwan_ren_session(taiwan_ren_lexicon());
        mixed_type(&mut s, "w96j06bp6");
        assert_eq!(s.snapshot().preedit, "台灣人");

        s.press(Key::new(KeyCode::Down, None, false));
        let snap = s.snapshot();
        assert!(s.candidate_window, "↓ 開候選窗");
        assert!(!snap.candidates.is_empty(), "最後一個音節要有候選");
        for candidate in &snap.candidates {
            assert_ne!(candidate.surface, "台灣人", "不再列整串詞候選");
            assert_ne!(candidate.surface, "臺灣人", "不再列整串詞候選");
            assert!(
                !candidate.surface.contains('台') && !candidate.surface.contains('灣'),
                "候選不能包含前面音節的字：{}",
                candidate.surface
            );
            assert_eq!(candidate.reading, "ㄖㄣˊ", "候選讀音是最後一個音節");
        }
        assert!(
            snap.candidates
                .iter()
                .any(|candidate| candidate.surface == "仁"),
            "要有最後一個音節的同音候選"
        );
    }

    /// 選字只換游標所在的那一個音節，前面已組好的字原樣保留。
    #[test]
    fn mixed_select_replaces_only_the_last_syllable() {
        let mut s = taiwan_ren_session(taiwan_ren_lexicon());
        mixed_type(&mut s, "w96j06bp6");
        assert_eq!(s.snapshot().preedit, "台灣人");

        s.press(Key::new(KeyCode::Down, None, false));
        let index = s
            .candidates()
            .iter()
            .position(|candidate| candidate.surface == "仁")
            .expect("要有仁候選");
        assert!(index < PAGE_SIZE, "測試候選要在第一頁");
        let digit = char::from(b'1' + index as u8);
        s.press(Key::char(digit, false));
        let snap = s.snapshot();
        assert_eq!(snap.preedit, "台灣仁", "只換最後一個字");
        assert_eq!(snap.cursor_rune, 3, "選字後游標回到結尾");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "台灣仁");
    }

    /// 游標停在字元邊界時，候選取游標右側那一個字元。
    #[test]
    fn mixed_down_targets_rune_right_of_cursor() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        lex.push(LexEntry::new("台", "ㄊㄞˊ", 0, 0));
        lex.push(LexEntry::new("颱", "ㄊㄞˊ", 1, 0));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        let mut s = Session::new(config);
        mixed_type(&mut s, "ji3w96");
        assert_eq!(s.snapshot().preedit, "我台");

        // ← 停在「台」前面，候選要針對右側的台。
        s.press(Key::new(KeyCode::ArrowLeft, None, false));
        assert_eq!(s.snapshot().cursor_rune, 1);
        s.press(Key::new(KeyCode::Down, None, false));
        let snap = s.snapshot();
        assert!(!snap.candidates.is_empty(), "右側注音字要有候選");
        for candidate in &snap.candidates {
            assert_eq!(candidate.reading, "ㄊㄞˊ", "只取右側那一個字元的讀音");
        }

        let index = s
            .candidates()
            .iter()
            .position(|candidate| candidate.surface == "颱")
            .expect("要有颱候選");
        assert!(index < PAGE_SIZE, "測試候選要在第一頁");
        let digit = char::from(b'1' + index as u8);
        s.press(Key::char(digit, false));
        let snap = s.snapshot();
        assert_eq!(snap.preedit, "我颱", "只換游標右側的字");
        assert_eq!(snap.cursor_rune, 2, "選字後游標往右一格");
    }

    /// 統一管線的一致性：把注音讀音拆回大千按鍵流，逐鍵走 session 的增量
    /// 路徑，結果必須與 `Router::convert` 相同。
    #[test]
    fn test_incremental_matches_router() {
        use crate::score::ScoreContext;
        use crate::zh::keyboard::Keyboard;
        use crate::zh::syllable::Syllable;

        let mut lex = InMemoryLexicon::new();
        let readings = [
            "ㄓㄜˋ",
            "ㄉㄡ",
            "ㄕˋ",
            "ㄙㄢ",
            "ㄒㄧㄠˇ",
            "ㄅㄟ",
            "ㄘㄜ",
            "ㄨㄛˇ",
            "ㄇㄚ",
            "ㄌㄜ",
            "ㄒㄧ",
            "ㄨ",
            "ㄏㄠˇ",
            "ㄐㄧㄝˋ",
            "ㄅㄨˋ",
            "ㄊㄧㄢ",
        ];
        let mut keys = Vec::new();
        for reading in readings {
            let syllable = Syllable::from_raw(reading);
            keys.push(
                Keyboard::standard()
                    .keys(&syllable)
                    .unwrap_or_else(|_| panic!("{reading:?} 無法拆回按鍵")),
            );
            // 詞典收同一個讀音，讓注音轉換有真實輸出。
            lex.push(LexEntry::new("字", reading, 0, 0));
        }
        let ctx = ScoreContext::default();
        let router = Router::new().with_zh(&lex, &ctx);
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex.clone()));

        let mut inputs = Vec::new();
        for left in &keys {
            for right in &keys {
                inputs.push(format!("{left}{right}"));
            }
        }
        // 空白現在是字面空格；含空白的組合仍要與 Router 逐段一致。
        //
        // 例外：`Router::visible_plans` 仍會吃掉「兩個注音切分片段之間」的獨立
        // 空白（舊的空白＝音節分隔規則），`ji3 ji3` 在 router 是 `我我`、在
        // session 是 `我 我`。這組不可能逐段一致，改在空白兩側用英文字面，
        // 並由 `test_space_between_zh_syllables_is_literal` 單獨鎖住 session
        // 保留字面空白的行為。
        for input in [
            "hello world",
            "hello  world",
            "watashi desu",
            "abc def",
            "ABC DEF",
            "hello, world",
        ] {
            inputs.push(input.to_string());
        }
        // 大寫、全角與尾端標點。
        for suffix in [".", "ABC", "ＡＢＣ", ",", "/"] {
            for left in keys.iter().take(4) {
                inputs.push(format!("{left}{suffix}"));
            }
        }

        let mut checked = 0usize;
        for input in &inputs {
            let mut session = Session::new(config.clone());
            for ch in input.chars() {
                let key = if ch == ' ' {
                    Key::new(KeyCode::Space, None, false)
                } else {
                    Key::char(ch, false)
                };
                session.press(key);
            }
            let got_surface = session.snapshot().preedit;
            let want = router.convert(input).expect("轉換");
            assert_eq!(got_surface, want.surface, "{input:?} 字面");

            let got: Vec<(Lang, String)> = session
                .segments()
                .iter()
                .map(|seg| (seg.mode, got_surface[seg.start..seg.end].to_string()))
                .collect();
            let want_segments: Vec<(Lang, String)> = want
                .segments
                .iter()
                .map(|seg| (seg.mode, seg.surface.clone()))
                .collect();
            assert_eq!(got, want_segments, "{input:?} 片段");
            checked += 1;
        }
        // 這個測試的詞典把每個讀音都收成「字」；`w96` 的 ㄊㄞˊ 不在詞典裡，
        // 所以字面是原始注音。真正的「我」／「台」由 `pending_session` 測試鎖住。
        for (input, want) in [
            ("ji3 hello", "字 hello"),
            ("w96 hello", "ㄊㄞˊ hello"),
            ("ji3 hello world", "字 hello world"),
        ] {
            let mut session = Session::new(config.clone());
            for ch in input.chars() {
                let key = if ch == ' ' {
                    Key::new(KeyCode::Space, None, false)
                } else {
                    Key::char(ch, false)
                };
                session.press(key);
            }
            assert_eq!(session.snapshot().preedit, want, "{input:?} 字面");
            assert_eq!(
                router.convert(input).expect("轉換").surface,
                want,
                "{input:?} Router"
            );
        }
        assert!(checked >= 200, "property test 只跑了 {checked} 筆");
    }

    fn cursor_lexicon() -> InMemoryLexicon {
        let mut lex = InMemoryLexicon::new();
        for (surface, reading, cost) in [
            ("我", "ㄨㄛˇ", 0),
            ("窩", "ㄨㄛˇ", 8),
            ("台", "ㄊㄞˊ", 0),
            ("灣", "ㄨㄢˊ", 0),
            ("人", "ㄖㄣˊ", 0),
        ] {
            lex.push(LexEntry::new(surface, reading, cost, 0));
        }
        lex
    }

    fn cursor_session() -> Session {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(cursor_lexicon()));
        Session::new(config)
    }

    /// 真實機器會載入日文解碼器；游標移動時不能讓待定注音被判成日文。
    fn pending_session() -> Session {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(cursor_lexicon()));
        config.ja = Some(test_ja_decoder());
        Session::new(config)
    }

    #[test]
    fn test_pending_syllable_degrades_to_raw_keys_on_cursor_move() {
        let mut s = pending_session();
        mixed_type(&mut s, "ji");
        assert_eq!(s.snapshot().preedit, "ㄨㄛ");
        let snap = s.press(Key::new(KeyCode::ArrowLeft, None, false));
        assert_eq!(snap.preedit, "ji", "待定音節要凍結成原始按鍵");
        assert_eq!(snap.cursor_rune, 0);
        assert!(
            snap.segments.iter().all(|segment| segment.mode != Lang::Ja),
            "游標移動不能生出日文片段：{:?}",
            snap.segments
        );
    }

    #[test]
    fn test_pending_syllable_degrades_for_su() {
        let mut s = pending_session();
        mixed_type(&mut s, "su");
        assert_eq!(s.snapshot().preedit, "ㄋㄧ");
        let snap = s.press(Key::new(KeyCode::ArrowLeft, None, false));
        assert_eq!(snap.preedit, "su", "`su` 要留在原始按鍵");
        assert_eq!(snap.cursor_rune, 0);
        assert!(
            snap.segments.iter().all(|segment| segment.mode != Lang::Ja),
            "`su` 不能變成す：{:?}",
            snap.segments
        );
    }

    #[test]
    fn test_pending_tail_degrades_after_completed_syllable() {
        let mut s = pending_session();
        mixed_type(&mut s, "w96j");
        assert_eq!(s.snapshot().preedit, "台ㄨ");
        let snap = s.press(Key::new(KeyCode::ArrowLeft, None, false));
        assert_eq!(snap.preedit, "台j", "已完成的台保留，尾巴 j 凍結成字面");
        assert_eq!(snap.cursor_rune, 1, "游標停在凍結尾巴的左緣");
    }

    #[test]
    fn test_cursor_move_does_not_create_japanese_reading() {
        for (keys, want) in [("ji", "ji"), ("su", "su")] {
            let mut s = pending_session();
            mixed_type(&mut s, keys);
            let snap = s.press(Key::new(KeyCode::ArrowLeft, None, false));
            assert_eq!(snap.preedit, want, "{keys}");
            assert!(
                !snap.segments.iter().any(|segment| segment.mode == Lang::Ja),
                "{keys} 移動游標後不該有日文片段：{:?}",
                snap.segments
            );
        }
    }

    #[test]
    fn test_cursor_moves_per_rune_for_zhuyin() {
        let mut s = cursor_session();
        mixed_type(&mut s, "w96j06bp6");
        assert_eq!(s.snapshot().preedit, "台灣人");
        assert_eq!(s.cursor(), 3);
        for want in [2, 1, 0] {
            let left = s.press(Key::new(KeyCode::ArrowLeft, None, false));
            assert_eq!(left.cursor_rune, want);
            assert_eq!(s.cursor(), want);
        }
        for want in [1, 2, 3] {
            let right = s.press(Key::new(KeyCode::ArrowRight, None, false));
            assert_eq!(right.cursor_rune, want);
        }
    }

    #[test]
    fn test_cursor_moves_per_kana_and_letter() {
        let mut s = cursor_session();
        mixed_type(&mut s, "watashiwa");
        assert_eq!(s.snapshot().preedit, "わたしわ");
        assert_eq!(s.snapshot().cursor_rune, 4);
        for want in [3, 2, 1, 0] {
            let left = s.press(Key::new(KeyCode::ArrowLeft, None, false));
            assert_eq!(left.cursor_rune, want);
        }
        let mut s = cursor_session();
        mixed_type(&mut s, "hello");
        assert_eq!(s.snapshot().preedit, "hello");
        for want in [4, 3, 2, 1, 0] {
            let left = s.press(Key::new(KeyCode::ArrowLeft, None, false));
            assert_eq!(left.cursor_rune, want);
        }
    }

    #[test]
    fn test_down_lists_decomposition_for_zh_syllable() {
        let mut s = cursor_session();
        mixed_type(&mut s, "ji3");
        let snap = s.press(Key::new(KeyCode::Down, None, false));
        let surfaces: Vec<&str> = snap
            .candidates
            .iter()
            .map(|candidate| candidate.surface.as_str())
            .collect();
        for want in ["我", "ㄨㄛˇ", "ji3"] {
            assert!(surfaces.contains(&want), "{want} 不在 {surfaces:?}");
        }
        assert_eq!(snap.highlighted, 0);
        assert_eq!(snap.candidates[0].surface, "我", "詞典候選仍是預設");
    }

    #[test]
    fn test_select_original_keys_then_japanese_reading() {
        let mut s = cursor_session();
        mixed_type(&mut s, "ji3");
        s.press(Key::new(KeyCode::Down, None, false));
        let index = s
            .candidates()
            .iter()
            .position(|candidate| candidate.surface == "ji3")
            .expect("ji3 候選");
        let snap = s.choose_candidate(index).expect("選回原始按鍵");
        assert_eq!(snap.preedit, "ji3");
        assert_eq!(snap.cursor_rune, 0, "選原始按鍵留在原位");

        let snap = s.press(Key::new(KeyCode::Down, None, false));
        let surfaces: Vec<&str> = snap
            .candidates
            .iter()
            .map(|candidate| candidate.surface.as_str())
            .collect();
        assert!(surfaces.contains(&"じ3"), "日文解讀不在 {surfaces:?}");
        let index = snap
            .candidates
            .iter()
            .position(|candidate| candidate.surface == "じ3")
            .expect("じ3 候選");
        let snap = s.choose_candidate(index).expect("選日文解讀");
        assert_eq!(snap.preedit, "じ3");
        assert_eq!(snap.cursor_rune, 1);
    }

    #[test]
    fn test_down_at_end_targets_last_character() {
        let mut s = cursor_session();
        mixed_type(&mut s, "ji3hello");
        let snap = s.press(Key::new(KeyCode::Down, None, false));
        assert_eq!(snap.preedit, "我hello");
        assert_eq!(snap.cursor_rune, 5, "↓ 先把游標左移到 o");
        assert_eq!(snap.candidates[0].surface, "o", "候選是最後一個字元");
    }

    /// 選英文字面的全形變體時只換那一個字元，前面的注音維持轉換結果。
    #[test]
    fn test_literal_choice_keeps_earlier_zh_conversion() {
        let mut s = cursor_session();
        mixed_type(&mut s, "ji3hello");
        assert_eq!(s.snapshot().preedit, "我hello");

        s.press(Key::new(KeyCode::Down, None, false));
        let snap = s.press(Key::new(KeyCode::Down, None, false));
        assert_eq!(snap.highlighted, 1, "第二項是全形 o");
        assert_eq!(snap.candidates[1].surface, "ｏ");
        let snap = s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(snap.preedit, "我hellｏ", "前面的注音不能退回原始按鍵 ji3");
    }

    /// 多個已完成音節也要整段保留，不能因為尾巴選字面而全部重播成按鍵。
    #[test]
    fn test_literal_choice_keeps_earlier_zh_for_multiple_syllables() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        lex.push(LexEntry::new("你", "ㄋㄧˇ", 0, 0));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        let mut s = Session::new(config);
        mixed_type(&mut s, "ji3su3hello");
        assert_eq!(s.snapshot().preedit, "我你hello");

        s.press(Key::new(KeyCode::Down, None, false));
        let snap = s.press(Key::new(KeyCode::Down, None, false));
        assert_eq!(snap.highlighted, 1, "第二項是全形 o");
        assert_eq!(snap.candidates[1].surface, "ｏ");
        let snap = s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(
            snap.preedit, "我你hellｏ",
            "兩個注音音節都要留著，不能變回 ji3su3"
        );
    }

    /// 選字面候選後再按 Enter，送出的也是轉換後的字。
    #[test]
    fn test_literal_choice_then_commit_keeps_conversion() {
        let mut s = cursor_session();
        mixed_type(&mut s, "ji3hello");
        s.press(Key::new(KeyCode::Down, None, false));
        s.press(Key::new(KeyCode::Down, None, false));
        let snap = s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(snap.preedit, "我hellｏ");

        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "我hellｏ", "送出不能是原始按鍵 ji3hellｏ");
    }

    #[test]
    fn test_down_opens_japanese_kanji_and_kana_candidates() {
        use std::sync::Arc;
        let entry = crate::ja::lexicon::JaEntry {
            surface: "私".into(),
            reading: "わたし".into(),
            pos: "名詞,普通名詞,一般,*".into(),
            conj_type: "*".into(),
            conj_form: "*".into(),
            cost: 0,
            split: crate::ja::lexicon::Split::C,
        };
        let mut config = SessionConfig::new(Mode::Mixed);
        config.ja = Some(Arc::new(crate::ja::decode::JaDecoder::new(Arc::new(
            TestJaSource(vec![entry]),
        ))));
        let mut s = Session::new(config);
        for ch in "watashi".chars() {
            s.press(Key::char(ch, false));
        }
        s.ja_live_idle();
        assert_eq!(s.snapshot().preedit, "私");
        let snap = s.press(Key::new(KeyCode::Down, None, false));
        assert_eq!(snap.highlighted, 0);
        assert_eq!(snap.candidates[0].surface, "私", "漢字是預設");
        assert!(snap
            .candidates
            .iter()
            .any(|candidate| candidate.surface == "わたし"));
        let index = snap
            .candidates
            .iter()
            .position(|candidate| candidate.surface == "わたし")
            .expect("假名候選");
        let snap = s.choose_candidate(index).expect("選假名");
        assert_eq!(snap.preedit, "わたし");
        assert_eq!(snap.cursor_rune, 1, "游標在第一個假名之後");
    }

    #[test]
    fn test_space_only_inserts_space_in_mixed() {
        let mut s = cursor_session();
        mixed_type(&mut s, "ji3");
        let snap = s.press(Key::new(KeyCode::Space, None, false));
        assert!(!snap.navigating, "空白不開候選");
        assert_eq!(snap.preedit, "我 ");
    }

    /// 兩個已完成的注音音節之間按空白，空白留字面，不吞成音節分隔。
    #[test]
    fn test_space_between_zh_syllables_is_literal() {
        let mut s = cursor_session();
        mixed_type(&mut s, "ji3 ji3");
        assert_eq!(s.snapshot().preedit, "我 我");
    }

    /// 三個音節以上時，每個空白都要留下；混打路徑的空白也要記成字面。
    #[test]
    fn test_three_zh_syllables_keep_both_spaces() {
        let mut s = cursor_session();
        mixed_type(&mut s, "ji3 ji3 ji3");
        assert_eq!(s.snapshot().preedit, "我 我 我");
    }

    #[test]
    fn test_four_zh_syllables_keep_all_spaces() {
        let mut s = cursor_session();
        mixed_type(&mut s, "ji3 ji3 ji3 ji3");
        assert_eq!(s.snapshot().preedit, "我 我 我 我");
    }

    /// 英文詞後面的空白照原本管線走，不要硬標成字面。
    #[test]
    fn test_space_after_english_word_not_forced_literal() {
        let mut s = cursor_session();
        mixed_type(&mut s, "hello world");
        assert_eq!(s.snapshot().preedit, "hello world");
    }

    /// 注音 lane 裡按空白要結束注音 lane：後續拉丁字母不能再進注音解碼器。
    ///
    /// 修前 `ji3 hello` 會變成 `我he lぉ`、`ji3 hello world` 變成
    /// `我へㄠㄠ欸world`，提交出去就是這些字。
    #[test]
    fn test_space_after_zh_syllable_ends_zh_lane() {
        let mut s = pending_session();
        mixed_type(&mut s, "ji3 hello");
        let snap = s.snapshot();
        assert_eq!(snap.preedit, "我 hello");
        assert!(
            snap.segments.iter().all(|segment| segment.mode != Lang::Ja),
            "空白後的英文不能生出日文片段：{:?}",
            snap.segments
        );
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "我 hello");
    }

    #[test]
    fn test_space_after_zh_syllable_then_two_words() {
        let mut s = pending_session();
        mixed_type(&mut s, "ji3 hello world");
        assert_eq!(s.snapshot().preedit, "我 hello world");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "我 hello world");
    }

    #[test]
    fn test_space_after_completed_zh_then_english() {
        let mut s = pending_session();
        mixed_type(&mut s, "w96 hello");
        assert_eq!(s.snapshot().preedit, "台 hello");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "台 hello");
    }

    #[test]
    fn test_double_space_then_english() {
        let mut s = pending_session();
        mixed_type(&mut s, "ji3  hello");
        assert_eq!(s.snapshot().preedit, "我  hello");
    }

    /// 外殼把空白送成字元時，要與 `KeyCode::Space` 走同一套處理。
    #[test]
    fn test_char_space_matches_space_key() {
        let mut s = pending_session();
        for ch in "ji3 hello".chars() {
            let key = if ch == ' ' {
                Key::char(' ', false)
            } else {
                Key::char(ch, false)
            };
            s.press(key);
        }
        assert_eq!(s.snapshot().preedit, "我 hello");
    }

    /// 上面幾個情境都要真的打出英文字母，不能留下注音或日文假名。
    #[test]
    fn test_space_then_latin_no_japanese_segments() {
        for (keys, want) in [
            ("ji3 hello", "我 hello"),
            ("w96 hello", "台 hello"),
            ("ji3 xyz", "我 xyz"),
            ("ji3 hello world", "我 hello world"),
        ] {
            let mut s = pending_session();
            mixed_type(&mut s, keys);
            let snap = s.snapshot();
            assert_eq!(snap.preedit, want, "{keys:?}");
            assert!(
                snap.segments.iter().all(|segment| segment.mode != Lang::Ja),
                "{keys:?} 生出日文片段：{:?}",
                snap.segments
            );
            for ch in ['ぉ', 'へ', 'ㄠ', 'ㄟ', 'ㄘ'] {
                assert!(
                    !snap.preedit.contains(ch),
                    "{keys:?} 出現注音或假名字元 {ch}：{:?}",
                    snap.preedit
                );
            }
        }
    }

    fn display_lexicon() -> InMemoryLexicon {
        let mut lex = InMemoryLexicon::new();
        for (surface, reading, cost) in [
            ("我", "ㄨㄛˇ", 0),
            ("婐", "ㄨㄛˇ", 10),
            ("捰", "ㄨㄛˇ", 20),
            ("台", "ㄊㄞˊ", 10),
            ("灣", "ㄨㄢˊ", 10),
            ("台灣", "ㄊㄞˊ ㄨㄢˊ", 0),
        ] {
            lex.push(LexEntry::new(surface, reading, cost, 0));
        }
        lex
    }

    fn display_session(personal: bool) -> Session {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(display_lexicon()));
        if personal {
            config.personal.note_seed("捰", 1.0);
            config.personal.note_seed("台灣", 1.0);
        }
        Session::new(config)
    }

    /// 個人學習把「捰」排第一時，組字區與候選第一項要一起變成「捰」。
    #[test]
    fn test_display_matches_first_candidate_for_zh_syllable() {
        let mut s = display_session(true);
        mixed_type(&mut s, "ji3");
        let snap = s.snapshot();
        assert_eq!(snap.candidates[0].surface, "捰");
        assert_eq!(snap.preedit, "捰");

        let down = s.press(Key::new(KeyCode::Down, None, false));
        assert!(s.candidate_window, "↓ 開候選窗");
        assert_eq!(down.candidates[0].surface, "捰", "候選第一項不變");
        assert_eq!(down.preedit, "捰", "preedit 仍等於第一候選");
    }

    /// 沒有個人資料時，兩邊都回到詞典成本決定的「我」。
    #[test]
    fn test_display_matches_first_candidate_without_personal() {
        let mut s = display_session(false);
        mixed_type(&mut s, "ji3");
        let snap = s.snapshot();
        assert_eq!(snap.candidates[0].surface, "我");
        assert_eq!(snap.preedit, "我");
    }

    /// 多音節詞的顯示與候選第一項也要一致。
    #[test]
    fn test_display_matches_first_candidate_for_multi_syllable() {
        let mut s = display_session(false);
        mixed_type(&mut s, "w96j06");
        let snap = s.snapshot();
        assert_eq!(snap.candidates[0].surface, "台灣");
        assert_eq!(snap.preedit, "台灣");
    }

    /// Enter 在候選窗開著時只套用高亮候選、關窗，不送出整段。
    #[test]
    fn test_enter_in_candidate_window_applies_without_committing() {
        let mut s = cursor_session();
        mixed_type(&mut s, "ji3");
        // UI 用 select 開候選窗、↓ 進入選取模式，再指定第二項；這條路
        // 沒有 mixed_choices 快取，Enter 以前會直接送出整段。
        s.select(0);
        s.press(Key::new(KeyCode::Down, None, false));
        assert!(s.zh_window_last, "↓ 把候選窗縮到游標那一格");
        s.select(1);
        let expected = s.highlighted_text().expect("有高亮候選").to_string();
        assert_eq!(expected, "窩", "第二項是窩");

        let snap = s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(snap.preedit, expected, "只換游標所在音節");
        assert!(!s.candidate_window, "Enter 後關窗");
        assert!(s.pending_commit.is_none(), "第一次 Enter 不能送出整段");
        assert_eq!(snap.cursor_rune, 1, "游標往右一格");
    }

    /// 候選窗關掉後再按 Enter，才送出整段組字。
    #[test]
    fn test_enter_twice_in_candidate_window_commits_second_time() {
        let mut s = cursor_session();
        mixed_type(&mut s, "ji3");
        s.select(0);
        s.press(Key::new(KeyCode::Down, None, false));
        assert!(s.zh_window_last);
        s.select(1);
        let expected = s.highlighted_text().expect("有高亮候選").to_string();
        assert_eq!(expected, "窩");

        s.press(Key::new(KeyCode::Enter, None, false));
        assert!(s.pending_commit.is_none(), "第一次 Enter 不送出");

        let snap = s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(snap.preedit, "", "第二次 Enter 清空組字");
        assert_eq!(s.commit(), expected, "第二次才送出整段");
    }

    /// 沒有候選窗時，Enter 維持直接送出整段。
    #[test]
    fn test_enter_without_candidate_window_commits() {
        let mut s = cursor_session();
        mixed_type(&mut s, "ji3");
        assert!(!s.candidate_window);
        let snap = s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(snap.preedit, "", "直接送出整段");
        assert_eq!(s.commit(), "我");
    }

    #[test]
    fn test_candidate_arrows_do_not_move_cursor() {
        let mut s = cursor_session();
        mixed_type(&mut s, "ji3");
        let opened = s.press(Key::new(KeyCode::Down, None, false));
        let cursor = opened.cursor_rune;
        let right = s.press(Key::new(KeyCode::ArrowRight, None, false));
        assert_eq!(right.cursor_rune, cursor, "第一次 → 只展開");
        let left = s.press(Key::new(KeyCode::ArrowLeft, None, false));
        assert_eq!(left.cursor_rune, cursor, "← 仍在候選介面");
        let closed = s.press(Key::new(KeyCode::Escape, None, false));
        assert_eq!(closed.cursor_rune, cursor);
        assert!(!closed.navigating);
    }
}
