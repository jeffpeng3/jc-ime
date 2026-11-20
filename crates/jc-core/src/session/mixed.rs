//! 混合輸入的待定按鍵推斷：英數按鍵在聲調或空白時決定是注音還是英文。
//!
//! 對照前身 `core/session/mixed.go` 的 `pendingKeys`、`mixedText`、
//! `finishMixedTone` 與 `mixedBoundary`。

use super::*;
use crate::lattice::Lexicon;
use crate::mixed::{default_english, prototype, English, MAX_WORD_BYTES};
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
        // 英文詞上限與前身 `english.MaxWordBytes` 相同；超過的字母不進待定按鍵。
        if ch.is_ascii_alphabetic() && self.pending.len() >= MAX_WORD_BYTES {
            return;
        }
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
    /// 先試一聲讀音，再試英→中切點，最後照字面留下。照字面留下的按鍵後面
    /// 補一個空白（`5pm `、`2026 `）；顯示成注音但組不成字時保留注音，
    /// 空白當一聲吃掉（`c ` → `ㄏ`）。
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
        let english = default_english();
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
        self.pending = text.to_string();
        true
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
            let body = &raw[..raw.len() - 1];
            let key = raw.chars().last()?;
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
        self.pending_display_with(lexicon, default_english())
    }

    fn pending_display_with(&self, lexicon: &dyn Lexicon, english: &English) -> String {
        let raw = &self.pending;
        if raw.is_empty() {
            return String::new();
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
        if !self.config.learning {
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
                        if !crate::personal::english_token(token) {
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

    /// 依按鍵推斷日文片段（`ja_mixed.go` 的 `maybeJapanese`）。
    ///
    /// 只在設定有日文解碼器時啟用。待定按鍵必須是兩個以上小寫 ASCII 字母、
    /// 組不成注音、不是英文詞或英文詞前綴，且能完整組成合法羅馬字；符合時
    /// 整段交給日文路徑，送出或取消後回到原本的模式。
    pub(super) fn try_japanese_run(&mut self) -> bool {
        if self.ja_run
            || self.config.mode != Mode::Mixed
            || self.config.ja.is_none()
            || !self.ascii.pieces().is_empty()
        {
            return false;
        }
        let raw = self.ascii.pending().to_string();
        if raw.len() < 2 || !raw.chars().all(|ch| ch.is_ascii_lowercase()) {
            return false;
        }
        if compose_keys(&raw).is_some() {
            return false;
        }
        let english = crate::mixed::default_english();
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
        self.ascii.reset();
        self.romaji.reset();
        for ch in raw.chars() {
            self.romaji.press(Key::char(ch, false));
        }
        self.ja_prev_mode = self.config.mode;
        self.config.mode = Mode::Ja;
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
        self.config.mode = self.ja_prev_mode;
        self.ja_run = false;
    }

    pub(super) fn rebuild_mixed(&mut self) {
        if self.ascii.is_empty() {
            return;
        }
        self.base = self.mixed_surface();
        let mut at = 0usize;
        self.segments = self
            .ascii
            .pieces()
            .iter()
            .map(|piece| {
                let (text, mode, script) = match piece {
                    AsciiPiece::Zh(syllable) => {
                        (self.decode_syllable(syllable), Mode::Zh, Script::Bopomofo)
                    }
                    AsciiPiece::Literal(text) => (text.clone(), Mode::En, Script::Latin),
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
                (Mode::En, Script::Latin)
            } else {
                (Mode::Zh, Script::Bopomofo)
            };
            self.segments.push(Segment::new(start, at, mode, script));
        }
        // 待定按鍵是英文詞時，候選先放原按鍵，再放注音顯示與英文前綴補完。
        // 同一個拼法有學過的修正時，修正直接排第一，原按鍵留在第二項。
        let raw = self.ascii.pending().to_string();
        if raw.is_empty() {
            return;
        }
        let limit = self.max_candidates();
        let mut seen = vec![raw.to_lowercase()];
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
        self.candidates.push(Candidate::new(
            raw.clone(),
            raw.clone(),
            0,
            CandidateSource::Literal,
        ));
        // 整串可以組成注音時，原按鍵之外另列注音本身（`co6` 的 ㄏㄟˊ）。
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
        for candidate in english_candidates(&self.personal, &raw, limit) {
            let key = candidate.surface.to_lowercase();
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            self.candidates.push(candidate);
        }
        self.candidates.truncate(limit);
    }

    /// 已完成的注音音節轉成字面。
    pub(super) fn decode_syllable(&self, syllable: &Syllable) -> String {
        let reading = syllable.as_str();
        let units = [reading];
        match Lattice::build(&units, &self.lexicon, &self.ctx) {
            Ok(lattice) => {
                let decoder = Decoder::new(&self.lexicon, &self.ctx);
                let (segs, _) = decoder.best(&lattice, &Locks::new(), false);
                decoder.conversion(&lattice, &segs, Mode::Zh).surface
            }
            Err(_) => reading.to_string(),
        }
    }

    /// 混合模式的完整字面：注音片段解碼，英數照字面，待定按鍵照顯示。
    pub(super) fn mixed_surface(&self) -> String {
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
                    out.push_str(&decoder.conversion(&lattice, &segs, Mode::Zh).surface);
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
    raw: &str,
    limit: usize,
) -> Vec<Candidate> {
    if limit == 0 || raw.chars().count() < 2 || !raw.chars().all(|c| c.is_ascii_alphabetic()) {
        return Vec::new();
    }
    let english = crate::mixed::default_english();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::{InMemoryLexicon, LexEntry};

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

    struct TestJaSource(Vec<crate::ja::lexicon::JaEntry>);

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
                    entry,
                    left_id: None,
                    right_id: None,
                });
            }
        }

        fn exact(&self, reading: &[char]) -> Vec<&crate::ja::lexicon::JaEntry> {
            self.0
                .iter()
                .filter(|entry| entry.reading.chars().eq(reading.iter().copied()))
                .collect()
        }

        fn prefixed(&self, reading: &[char]) -> Vec<&crate::ja::lexicon::JaEntry> {
            self.0
                .iter()
                .filter(|entry| {
                    entry
                        .reading
                        .chars()
                        .take(reading.len())
                        .eq(reading.iter().copied())
                })
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
            pos: Arc::from("名詞,普通名詞,一般,*"),
            conj_type: Arc::from("*"),
            conj_form: Arc::from("*"),
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
                surface: "今日".into(),
                reading: "きょう".into(),
                pos: Arc::from("名詞,普通名詞,一般,*"),
                conj_type: Arc::from("*"),
                conj_form: Arc::from("*"),
                cost: 0,
                split: crate::ja::lexicon::Split::C,
            };
            config.ja = Some(Arc::new(crate::ja::decode::JaDecoder::new(Arc::new(
                TestJaSource(vec![entry]),
            ))));
            config
        });
        for ch in "kyou".chars() {
            s.press(Key::char(ch, false));
        }
        assert_eq!(s.mode(), Mode::Ja);
        assert_eq!(s.snapshot().preedit, "今日");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "今日");
        assert_eq!(s.mode(), Mode::Mixed);
    }

    /// 對照 `language_learning.go`：混打送出時把注音片段記成中文、
    /// 照字面的英數片段記成英文。
    #[test]
    fn test_mixed_commit_records_language_selection() {
        use crate::personal::LanguageId;

        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(lex);
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

    /// 對照 `english_autocomplete.go`：完整英文詞沒有補全，打錯一鍵有更正，
    /// 超過 `MAX_WORD_BYTES` 的字母不再收進待定按鍵。
    #[test]
    fn test_english_autocomplete_limits() {
        assert_eq!(typed("hellp").suggestion().as_deref(), Some("hello"));
        assert_eq!(typed("hello").suggestion(), None);
        let long = "a".repeat(MAX_WORD_BYTES + 8);
        let input = typed(&long);
        assert_eq!(input.pending().len(), MAX_WORD_BYTES);
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
}
