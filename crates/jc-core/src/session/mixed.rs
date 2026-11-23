//! 混合輸入的待定按鍵推斷：英數按鍵在聲調或空白時決定是注音還是英文。
//!
//! 對照前身 `core/session/mixed.go` 的 `pendingKeys`、`mixedText`、
//! `finishMixedTone` 與 `mixedBoundary`。

use super::*;
use crate::composition::{CompSegment, Lang, Owner};
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
    /// 這次組字實際按過的按鍵流（含空白與聲調）。統一管線用它重算片段。
    typed: String,
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
        // 英文詞上限與前身 `english.MaxWordBytes` 相同；超過的字母不進待定按鍵。
        if ch.is_ascii_alphabetic() && self.pending.len() >= MAX_WORD_BYTES {
            return;
        }
        self.pending.push(ch);
        self.typed.push(ch);
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
        // R2：前一個英數 run 加上這個空白可以被注音完整解析，而且最後一個
        // 音節原本沒有聲調鍵時，空白是一聲。保留原按鍵，交給統一管線切分。
        let adapter = crate::mixed_latin::LexiconAdapter {
            zh: Some(lexicon),
            english: Some(default_english()),
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
        let english = default_english();
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
        if plan.mode != Mode::Zh || plan.start == 0 {
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
        self.ja_run_keys = raw.clone();
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
        self.ja_run_keys.clear();
    }

    /// 日文推斷中退格後，把原始按鍵同步回目前的羅馬字狀態。
    ///
    /// `romaji.backspace` 可能一次退掉一整組羅馬字（`kya` 是三個鍵），直接
    /// 砍一個字元會讓退回混打時重播錯的按鍵。用目前狀態反推最長的原始前綴；
    /// 找不到時清空，退回混打時就不會硬塞已刪除的按鍵。
    pub(super) fn sync_ja_run_keys(&mut self) {
        if !self.ja_run {
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

    /// 混合模式的 Backspace：刪掉游標前一個片段。
    /// 片段已經是使用者看到的字（`台`、`hello`），不再退回注音或逐字母刪。
    /// 游標在結尾時就是刪最右邊那一段。沒有游標前的片段時回傳 `false`，
    /// 交給呼叫端的舊路徑處理。
    pub(super) fn backspace_mixed(&mut self) -> bool {
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
        let Some((plans, _)) = self.incremental_latin_split_for(&text) else {
            return Vec::new();
        };
        let rendered = self.render_latin_plans(&text, &plans);
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
            let mut whole = AsciiInput::new();
            whole.replay(&text, &self.lexicon);
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
        let mut before = AsciiInput::new();
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
            let mut suffix = AsciiInput::new();
            suffix.replay(&text[start..end], &self.lexicon);
            if !suffix.pending().is_empty() || suffix.pieces().len() != 1 {
                continue;
            }
            match &suffix.pieces()[0] {
                AsciiPiece::Zh(syllable) if syllable == &expected => {}
                _ => continue,
            }
            let mut prefix = AsciiInput::new();
            prefix.replay(&text[..start], &self.lexicon);
            if prefix.pending().is_empty() && prefix.pieces() == prefix_pieces {
                return Some((start, end, expected));
            }
        }
        None
    }

    /// 尾巴連續注音片段在按鍵流中的範圍；沒有時回傳 `None`。
    ///
    /// 跟 [`Session::mixed_typed_spans`] 共用同一份切分結果，只用來把選到的
    /// 中文候選換回按鍵流，不會跨過英文或記號片段。
    pub(super) fn mixed_trailing_zh_span(&self) -> Option<(usize, usize)> {
        let text = self.latin_run_text()?;
        let (plans, _) = self.incremental_latin_split_for(&text)?;
        let rendered = self.render_latin_plans(&text, &plans);
        // 先去掉尾端空白（開候選窗時留下的按鍵），再從尾端往回收注音片段。
        let last = plans.iter().zip(&rendered).rposition(|(_, item)| {
            !item.surface.is_empty() && !item.surface.chars().all(char::is_whitespace)
        })?;
        let mut span: Option<(usize, usize)> = None;
        for (plan, item) in plans.iter().zip(rendered).take(last + 1) {
            if item.surface.is_empty() {
                continue;
            }
            if plan.zh.is_some() {
                span = Some(match span {
                    Some((start, _)) => (start, plan.end),
                    None => (plan.start, plan.end),
                });
            } else {
                span = None;
            }
        }
        span
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
        if !self.ja_run || self.config.mode != Mode::Ja {
            return false;
        }
        let keys = std::mem::take(&mut self.ja_run_keys);
        self.romaji.reset();
        if let Some(live) = self.live.as_mut() {
            live.reset();
        }
        self.ja_run = false;
        self.config.mode = self.ja_prev_mode;
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
        let mut split_used = false;
        if let Some((plans, text)) = self.incremental_latin_split() {
            self.apply_latin_split(&plans, &text);
            split_used = true;
        }
        if !split_used {
            self.rebuild_mixed_legacy();
        }
        self.rebuild_mixed_candidates();
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
        self.sync_composition_mixed();
    }

    /// 混合模式的候選：原按鍵、注音本身與英文前綴補完。
    fn rebuild_mixed_candidates(&mut self) {
        // 待定按鍵是英文詞時，候選先放原按鍵，再放注音顯示與英文前綴補完。
        // 同一個拼法有學過的修正時，修正直接排第一，原按鍵留在第二項。
        let raw = self.ascii.pending().to_string();
        if raw.is_empty() {
            // 注音音節打完後，待定按鍵是空的，但已完成的注音片段仍有候選；
            // 少了這段，Mixed 按 ↓ 或空白永遠開不了候選窗。
            self.rebuild_mixed_zh_candidates();
            return;
        }
        // 待定按鍵還沒清空，但統一管線已經把尾巴判成完成的注音音節
        // （`savul3` 是 `sa`＋`vul3`）。候選要列那個音節的詞，不能只列
        // 原始按鍵；`cands=1` 的字面候選會讓使用者永遠選不到「小」。
        if let Some(reading) = self.planned_trailing_zh_reading() {
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
        // pending 顯示成注音代表它還是未完成的注音前綴（例如 su 是 ㄋㄧ）。
        // 這時英文補全只留在 Snapshot.suggestion 提示，不進候選讓數字鍵選到，
        // 避免組字區被補全取代而鎖死語言。hello、sug 等照字面顯示的前綴
        // 維持原本的英文補全流程。
        let is_zhuyin_prefix = self.ascii.display_pending(&self.lexicon) != raw;
        if !is_zhuyin_prefix {
            for candidate in english_candidates(&self.personal, &raw, limit) {
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
    fn rebuild_mixed_zh_candidates(&mut self) {
        // 游標在結尾時直接問統一管線：已完成注音的尾巴就是最後一個注音
        // 片段。這條路不依賴 `AsciiInput` 的 pending 狀態，`ej/ ` 這種
        // 空白當一聲、或前段是 ja 的混合 run 才不會只剩字面候選。
        if self.mixed_cursor.min(self.segments.len()) == self.segments.len() {
            if let Some(reading) = self.planned_trailing_zh_reading() {
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
    fn planned_trailing_zh_reading(&self) -> Option<String> {
        let text = self.latin_run_text()?;
        let (plans, _) = self.incremental_latin_split_for(&text)?;
        let plan = plans.last()?;
        if plan.mode != Mode::Zh {
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
        let (mut hypotheses, _) =
            decoder.nbest(&lattice, &Locks::new(), false, self.max_candidates());
        let literal = reading.clone();
        if self.config.learning {
            self.personal.rank_hypotheses(&mut hypotheses, &reading);
        }
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

    /// 統一管線的輸入文字：這次組字實際按過的完整按鍵流。
    ///
    /// 含已完成的注音、照字面的英數、空白與聲調；顯示與送出一律用同一條
    /// 管線重算，不再區分「已完成片段」與「待定按鍵」。
    fn latin_run_text(&self) -> Option<String> {
        let text = self.ascii.typed();
        if text.is_empty() {
            return None;
        }
        Some(text.to_string())
    }

    /// 逐鍵增量：用與 [`crate::mixed::Router::convert`] 相同的管線重算片段。
    fn incremental_latin_split(&mut self) -> Option<(Vec<crate::mixed::Plan>, String)> {
        let text = self.latin_run_text()?;
        let english = default_english();
        let adapter = crate::mixed_latin::LexiconAdapter {
            zh: Some(&self.lexicon),
            english: Some(english),
            ..Default::default()
        };
        let cfg = crate::mixed::Config::default();
        let plans = crate::mixed::plan_input(
            &text,
            english as &dyn crate::mixed::Lookup,
            &cfg,
            Some(&adapter),
        );
        let plans = crate::mixed::visible_plans(plans, &text);
        if plans.is_empty() {
            return None;
        }
        Some((plans, text))
    }

    /// 沒有增量結果時重算一次；提交路徑會用到。
    fn latin_surface(&self) -> Option<String> {
        let (plans, text) = self.incremental_latin_split_for(&self.latin_run_text()?)?;
        let mut out = String::new();
        for piece in self.render_latin_plans(&text, &plans) {
            out.push_str(&piece.surface);
        }
        Some(out)
    }

    /// 不修改狀態的統一管線版本，給 [`Session::latin_surface`] 用。
    fn incremental_latin_split_for(&self, text: &str) -> Option<(Vec<crate::mixed::Plan>, String)> {
        let english = default_english();
        let adapter = crate::mixed_latin::LexiconAdapter {
            zh: Some(&self.lexicon),
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
        (!plans.is_empty()).then_some((plans, text.to_string()))
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
                    Mode::Ja => {
                        if !plan.kana_ok || plan.kana.is_empty() {
                            RenderedPiece {
                                surface: literal.to_string(),
                                mode: Mode::Ja,
                                reading: String::new(),
                                units: 0,
                            }
                        } else {
                            RenderedPiece {
                                surface: self.decode_kana(&plan.kana),
                                mode: Mode::Ja,
                                reading: plan.kana.clone(),
                                units: plan.kana.chars().count(),
                            }
                        }
                    }
                    Mode::Zh => {
                        let reading = match &plan.zh {
                            Some(reading) => {
                                crate::mixed_latin::toned_syllables(reading, &self.lexicon)
                                    .join(" ")
                            }
                            None => literal.to_string(),
                        };
                        let units = reading.split_whitespace().count();
                        RenderedPiece {
                            surface: self.decode_reading(&reading),
                            mode: Mode::Zh,
                            reading,
                            units,
                        }
                    }
                    _ => RenderedPiece {
                        surface: literal.to_string(),
                        mode: Mode::En,
                        reading: String::new(),
                        units: 0,
                    },
                }
            })
            .collect()
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
                Mode::Ja => Lang::Ja,
                Mode::Zh => Lang::Zh,
                _ => Lang::Literal,
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
                let surface = decoder.conversion(&lattice, &segs, Mode::Zh).surface;
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
    mode: Mode,
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
                surface: "今日".into(),
                reading: "きょう".into(),
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
        for ch in "kyou".chars() {
            s.press(Key::char(ch, false));
        }
        // 日文片段沿用延遲整句：停頓後才顯示轉換結果。
        s.ja_live_idle();
        assert_eq!(s.mode(), Mode::Ja);
        assert_eq!(s.snapshot().preedit, "今日");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "今日");
        assert_eq!(s.mode(), Mode::Mixed);
    }

    /// `Mode::Mixed` 下 `watashi` 走羅馬字路徑出假名，空白與後續按鍵仍可
    /// 繼續輸入；注音模式（`Mode::Zh`）把同一串拉丁鍵解成注音是設計，不是
    /// 路由 bug。
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

    /// 英文字面片段也是整段刪，不是逐字母退。
    #[test]
    fn test_mixed_backspace_deletes_whole_english_segment() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        let mut s = Session::new(config);
        mixed_type(&mut s, "ji3hello");
        assert_eq!(s.snapshot().preedit, "我hello");

        let snap = s.press(Key::new(KeyCode::Backspace, None, false));
        assert_eq!(snap.preedit, "我", "英文片段整段刪掉");
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
        assert!(s.candidate_window, "空白開候選窗");
        assert!(!space.navigating, "空白只顯示候選");
        assert_eq!(space.preedit, "台", "空白不插入字面空白");
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

    /// 混合模式的方向鍵在片段邊界之間移動，英文片段也算一格。
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

        let left = s.press(Key::new(KeyCode::ArrowLeft, None, false));
        assert_eq!(left.cursor_rune, 1, "← 移到英文片段開頭");
        assert_eq!(left.preedit, "我hello", "移動游標不改內容");
        let left = s.press(Key::new(KeyCode::ArrowLeft, None, false));
        assert_eq!(left.cursor_rune, 0, "← 再移到注音片段開頭");
        let right = s.press(Key::new(KeyCode::ArrowRight, None, false));
        assert_eq!(right.cursor_rune, 1, "→ 回到英文片段開頭");
        let right = s.press(Key::new(KeyCode::ArrowRight, None, false));
        assert_eq!(right.cursor_rune, 6, "→ 回到整段結尾");
    }

    /// 游標停在英文片段前時，Delete 刪掉游標後那一段。
    #[test]
    fn test_mixed_delete_forward_removes_segment_after_cursor() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        let mut s = Session::new(config);
        mixed_type(&mut s, "ji3hello");
        s.press(Key::new(KeyCode::ArrowLeft, None, false));
        assert_eq!(s.snapshot().cursor_rune, 1);

        let snap = s.press(Key::new(KeyCode::Delete, None, false));
        assert_eq!(snap.preedit, "我", "Delete 刪掉游標後的英文片段");
        assert_eq!(snap.cursor_rune, 1);
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
            "りしれ供",
            "`rishiree` 不能整段留在日文：e 要還給 ej/"
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
        let got: Vec<(Mode, String)> = s
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
                (Mode::Ja, "りしれ".to_string()),
                (Mode::Zh, "供".to_string()),
                (Mode::Ja, "さ".to_string()),
                (Mode::En, "small".to_string()),
            ],
            "批次規劃是 rishire｜ej/ ｜sa｜small"
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

    /// 游標停在片段邊界時，候選取游標左側片段的最後一個注音音節。
    #[test]
    fn mixed_down_uses_syllable_left_of_cursor() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        lex.push(LexEntry::new("婐", "ㄨㄛˇ", 1, 0));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        let mut s = Session::new(config);
        mixed_type(&mut s, "ji3hello ");
        assert_eq!(s.snapshot().preedit, "我hello ");

        // ← 停在注音與英文片段的邊界（尾端空白自成一段）。
        s.press(Key::new(KeyCode::ArrowLeft, None, false));
        s.press(Key::new(KeyCode::ArrowLeft, None, false));
        assert_eq!(s.snapshot().cursor_rune, 1);
        s.press(Key::new(KeyCode::Down, None, false));
        let snap = s.snapshot();
        assert!(!snap.candidates.is_empty(), "左側注音片段要有候選");
        for candidate in &snap.candidates {
            assert_eq!(candidate.reading, "ㄨㄛˇ", "只取左側片段的那一個音節");
        }

        let index = s
            .candidates()
            .iter()
            .position(|candidate| candidate.surface == "婐")
            .expect("要有婐候選");
        assert!(index < PAGE_SIZE, "測試候選要在第一頁");
        let digit = char::from(b'1' + index as u8);
        s.press(Key::char(digit, false));
        let snap = s.snapshot();
        assert_eq!(snap.preedit, "婐hello ", "只換游標所在的字，後面英文保留");
        assert_eq!(snap.cursor_rune, 7, "選字後游標回到結尾");
    }

    /// 統一管線的一致性：把注音讀音拆回大千按鍵流，逐鍵走 session 的增量
    /// 路徑，結果必須與 `Router::convert` 相同。
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
                for sep in ["", " "] {
                    inputs.push(format!("{left}{sep}{right}"));
                }
            }
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

            let got: Vec<(Mode, String)> = session
                .segments()
                .iter()
                .map(|seg| (seg.mode, got_surface[seg.start..seg.end].to_string()))
                .collect();
            let want_segments: Vec<(Mode, String)> = want
                .segments
                .iter()
                .map(|seg| (seg.mode, seg.surface.clone()))
                .collect();
            assert_eq!(got, want_segments, "{input:?} 片段");
            checked += 1;
        }
        assert!(checked >= 200, "property test 只跑了 {checked} 筆");
    }
}
