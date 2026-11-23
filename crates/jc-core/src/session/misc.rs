//! 其餘 session 行為：待定字面、使用者詞庫標記、本地證據與文件文脈。
//!
//! 對照上游 `core/session/` 的 `pending_literal.go`、`vocab_mark.go`、
//! `local_evidence.go`、`document_context.go` 與 `corrections.go`。這裡只放
//! 不依賴組字游標插入／刪除與解碼器個人化成本的行為；需要 `ZhInput` 逐格
//! 編輯或 `Decoder` 修正成本的部分，依賴關係寫在
//! `docs/port/session-g5-misc.md`。
//!
//! 上游一個 `Session` 的方法分散在多個檔案；本模組比照同一個模式，用
//! `impl Session` 掛在 `session::mod` 的型別上。

use std::collections::BTreeMap;

use super::*;
use crate::decode::Hypothesis;
use crate::lattice::Lexicon;
use crate::personal::Seg;
use crate::zh::syllable::{class_of, key as reading_key, untone, Class, Composer, Syllable};

/// 組字中加詞至少兩個字（同小麥注音、威注音）。
///
/// 單字讀音在設定程式加入，避免誤加單字污染選字。
pub const MIN_COMPOSE_WORD: usize = 2;

// ---------------------------------------------------------------------------
// pending_literal.go：待定字面
// ---------------------------------------------------------------------------

/// 待定字面：一個原樣留在組字區的注音音節。
///
/// `tone` 非零時是「沒有字可讀、由聲調鍵完成」而原樣留下的完整音節：
/// 畫面是注音連同聲調（一聲不標），聲調符號緊接在 [`PendingLiteral::end`]
/// 之後。`tone` 為 0 代表已經退回一般暫存注音，可以改打別的聲調。
///
/// `prefix` 表示音節自己沒有字、只是某個更長詞的開頭（例如「ㄟ」）；下一個
/// 音節能和它成詞時要收回去一起解碼。使用者從候選明確選入的原樣音節不帶
/// 這個標記。`untoned` 是聲調選填時沒打聲調就完成的音節，畫面同一聲。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingLiteral {
    start: usize,
    composer: Composer,
    tone: u8,
    prefix: bool,
    untoned: bool,
}

impl PendingLiteral {
    /// 由按聲調前的組音狀態與完成的音節建立待定字面。
    ///
    /// 沒打聲調的音節（聲調選填）照一聲的樣子留下，之後收回解碼時仍不限
    /// 聲調。
    pub fn from_syllable(start: usize, composer: Composer, syllable: &Syllable) -> Self {
        let untoned = syllable.untoned();
        Self {
            start,
            composer,
            tone: if untoned { 1 } else { syllable.tone() },
            prefix: false,
            untoned,
        }
    }

    /// 標記這個音節只是更長詞的開頭。
    pub fn with_prefix(mut self, prefix: bool) -> Self {
        self.prefix = prefix;
        self
    }

    /// 待定字面在組字區的起點（rune 位置）。
    pub fn start(&self) -> usize {
        self.start
    }

    /// 音節本體結束的位置；聲調符號另外算。
    pub fn end(&self) -> usize {
        self.start + self.composer.string().chars().count()
    }

    /// 畫面上聲調符號占的字數：二、三、四聲與輕聲各一個，一聲不標。
    pub fn mark(&self) -> usize {
        usize::from(self.tone >= 2)
    }

    /// 是否只是更長詞的開頭。
    pub fn prefix(&self) -> bool {
        self.prefix
    }

    /// 目前是否已退回一般暫存注音（沒有聲調）。
    pub fn is_pending(&self) -> bool {
        self.tone == 0
    }

    /// 待定字面代表的音節；聲調還在時含聲調。
    pub fn syllable(&self) -> Syllable {
        let base = Syllable::from_raw(self.composer.string());
        if self.untoned {
            untone(&base)
        } else {
            base.with_tone(self.tone)
        }
    }
}

/// 組字區插入或刪除 `delta` 個字時，同步移動待定字面的位置。
///
/// 對照上游 `shiftPendingLiterals`：插入點在音節之前時整格往後移；插入點
/// 落在音節本體內時，注音已被編輯，不能再還原舊的符號或組音順序，直接丟
/// 掉；在聲調符號之前插入別的字時，也不再是同一個音節；刪掉聲調則把音節
/// 退回一般暫存注音。
pub fn shift_pending_literals(literals: &mut Vec<PendingLiteral>, at: usize, delta: isize) {
    if literals.is_empty() {
        return;
    }
    let at = at as isize;
    let mut next = Vec::with_capacity(literals.len());
    for mut literal in literals.drain(..) {
        let start = literal.start as isize;
        let end = literal.end() as isize;
        if at < start || delta > 0 && at == start {
            literal.start = (start + delta).max(0) as usize;
        } else if at < end {
            continue;
        } else if literal.mark() > 0 && at == end {
            if delta > 0 {
                continue;
            }
            literal.tone = 0;
        }
        next.push(literal);
    }
    *literals = next;
}

/// 游標剛好在待定字面尾端時，回傳要收回組音的那一格。
///
/// 只有聲調已經消失（`tone == 0`）的待定字面可以還原成未完成注音；帶聲調
/// 的原樣音節要等聲調符號被刪掉。
pub fn restorable_pending_literal(literals: &[PendingLiteral], cursor: usize) -> Option<usize> {
    literals
        .iter()
        .position(|literal| literal.is_pending() && literal.end() == cursor)
}

// ---------------------------------------------------------------------------
// vocab_mark.go：組字中加詞
// ---------------------------------------------------------------------------

/// 組字中加詞的結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordMark {
    /// 是否真的加入或設為優先。
    pub changed: bool,
    /// 給外殼顯示的說明；空字串代表沒有動作。
    pub notice: String,
}

impl WordMark {
    fn refused(notice: impl Into<String>) -> Self {
        Self {
            changed: false,
            notice: notice.into(),
        }
    }
}

/// 範圍能否加入詞庫；不能時保留原因，讓說明與實際加入用同一套判斷。
enum MarkCheck {
    Ready(String, String),
    NoText,
    NoReading,
}

impl Session {
    /// 標示範圍的說明：按 `Enter` 會發生什麼，附讀音；不能加入時說明原因。
    ///
    /// `start`、`end` 是音節索引（半開區間），對照上游 `markHint`。空範圍
    /// 回傳空字串。
    pub fn mark_hint(&self, start: usize, end: usize) -> String {
        if start >= end || end > self.syllables.len() {
            return String::new();
        }
        if end - start < MIN_COMPOSE_WORD {
            return "再多選一個字（單字的讀音請到設定程式「我的詞彙」加入）".to_string();
        }
        if self.literal_inside(start, end) {
            return "範圍裡有英文或符號，不能加入詞庫；這類文字可以放「我的片語」".to_string();
        }
        match self.check_marked(start, end) {
            MarkCheck::Ready(word, reading) => {
                if self.personal.vocab_has_reading(&word, &reading) {
                    format!("「{word}」已在詞庫　Esc 取消")
                } else if self.system_has(&word, &reading) {
                    format!("Enter 設為優先：{word}（{reading}）　Esc 取消")
                } else {
                    format!("Enter 加入詞庫：{word}（{reading}）　Esc 取消")
                }
            }
            MarkCheck::NoText => "這段還沒選好字，先選字再加入".to_string(),
            MarkCheck::NoReading => "這段有沒打聲調、還認不出讀音的字，補上聲調再加入".to_string(),
        }
    }

    /// 把音節範圍 `[start, end)` 目前顯示的字加入自訂詞庫。
    ///
    /// 讀音用使用者打的注音，破音字不會猜錯。範圍內原本的片段鎖定由整個詞
    /// 取代，畫面不變；加入後立刻打得出同一個詞。對照上游 `addMarked`，
    /// 但游標與標示樣式由呼叫端提供（本埠 `Snapshot` 沒有標示樣式）。
    pub fn add_composition_word(&mut self, start: usize, end: usize) -> WordMark {
        if start >= end || end > self.syllables.len() {
            return WordMark {
                changed: false,
                notice: String::new(),
            };
        }
        if self.config.mode != Mode::Zh {
            return WordMark::refused("這裡不加入詞庫");
        }
        if end - start < MIN_COMPOSE_WORD {
            return WordMark::refused("至少選兩個字；單字的讀音請到設定程式「我的詞彙」加入");
        }
        if self.literal_inside(start, end) {
            return WordMark::refused("只能把注音打出的字加入詞庫");
        }
        let (word, reading) = match self.check_marked(start, end) {
            MarkCheck::Ready(word, reading) => (word, reading),
            MarkCheck::NoText => {
                return WordMark::refused("這段還沒有選好字，先選字再加入");
            }
            MarkCheck::NoReading => {
                return WordMark::refused("這段有沒打聲調、還認不出讀音的字，補上聲調再加入");
            }
        };
        if self.personal.vocab_has_reading(&word, &reading) {
            return WordMark::refused(format!("「{word}」已在詞庫"));
        }
        let system = self.system_has(&word, &reading);
        self.personal.add_vocab(&word, &reading);
        self.refresh_lexicon();
        self.mark_zh_user_syllables(start, end - start, &word);
        self.sync_zh_locks();
        let notice = if system {
            format!("已設為優先：{word}")
        } else {
            format!("已加入詞庫：{word}")
        };
        WordMark {
            changed: true,
            notice,
        }
    }

    /// 字面文字是否插在 `[start, end)` 的內部。
    ///
    /// 本埠的字面文字是一整串、記在音節索引 `literal_anchor`；只有插入點落
    /// 在範圍內部才算範圍裡的文字。
    fn literal_inside(&self, start: usize, end: usize) -> bool {
        !self.literal.is_empty() && start < self.literal_anchor && self.literal_anchor < end
    }

    /// 音節範圍目前顯示的字與讀音；任何一格沒選好字就回傳原因。
    fn check_marked(&self, start: usize, end: usize) -> MarkCheck {
        let word = self.text_in(start, end);
        if word.chars().count() != end - start || word.chars().any(|ch| class_of(ch) != Class::None)
        {
            return MarkCheck::NoText;
        }
        match self.typed_reading(start, end) {
            Some(typed) => MarkCheck::Ready(word, reading_key(&typed)),
            None => MarkCheck::NoReading,
        }
    }

    /// 音節 `[start, end)` 目前顯示的字；取不出對應的字時回傳空字串。
    fn text_in(&self, start: usize, end: usize) -> String {
        if let Some(hypothesis) = self.current_hypothesis().filter(|h| !h.segments.is_empty()) {
            let mut out = String::new();
            for seg in &hypothesis.segments {
                if seg.end <= start || seg.start >= end {
                    continue;
                }
                let chars: Vec<char> = seg.word.chars().collect();
                if chars.len() != seg.end - seg.start {
                    // 尾端前綴等字數與音節數不同的分段：取不出對應的字。
                    return String::new();
                }
                let lo = start.saturating_sub(seg.start).min(chars.len());
                let hi = end.saturating_sub(seg.start).min(chars.len());
                out.extend(&chars[lo..hi]);
            }
            return out;
        }
        let mut out = String::new();
        for syllable in &self.syllables[start..end] {
            out.push_str(&self.decode_syllable(syllable));
        }
        out
    }

    /// 使用者打的讀音；沒打聲調的音節用目前解碼選到的聲調。
    fn typed_reading(&self, start: usize, end: usize) -> Option<Vec<Syllable>> {
        let syllables = &self.syllables[start..end];
        let readings = self.zh_readings(syllables);
        if readings.len() != end - start {
            return None;
        }
        readings
            .iter()
            .map(|reading| Syllable::parse(reading).ok())
            .collect()
    }

    /// 系統詞庫有這個詞、這個讀音。
    fn system_has(&self, word: &str, reading: &str) -> bool {
        self.base_lexicon
            .lookup(reading)
            .iter()
            .any(|entry| entry.surface == word)
    }

    /// 目前顯示中的假設：高亮候選對應的路徑，找不到時退回第一個。
    pub(super) fn current_hypothesis(&self) -> Option<&Hypothesis> {
        let surface = self
            .candidates
            .get(self.highlighted)
            .map(|candidate| candidate.surface.as_str());
        surface
            .and_then(|surface| self.last_hypotheses.iter().find(|h| h.text == surface))
            .or_else(|| self.last_hypotheses.first())
    }
}

// ---------------------------------------------------------------------------
// local_evidence.go：本地證據
// ---------------------------------------------------------------------------

/// 本地證據的查詢層。
///
/// 只從目前仍有效的手選鎖定重建，不寫入個人資料；刪除、改選或取消使鎖定
/// 失效時，證據自然消失。對照上游 `localEvidence`，回傳的查詢函式換成
/// [`LocalEvidence::at`]。
#[derive(Debug, Clone, PartialEq)]
pub struct LocalEvidence {
    choices: BTreeMap<Span, BTreeMap<String, f64>>,
    totals: BTreeMap<Span, f64>,
}

/// 證據的作用範圍；`left` 只有鄰詞（弱觀察）會用到。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Span {
    start: usize,
    end: usize,
    left: String,
}

impl LocalEvidence {
    /// 查某個範圍裡某個詞的證據與總量。
    ///
    /// `start`、`end` 是相對於 `base` 的音節索引。總量為 0 時改查帶左鄰詞
    /// 的鍵，對照上游回傳的函式。
    pub fn at(&self, start: usize, end: usize, left: &str, word: &str) -> (f64, f64) {
        let mut span = Span {
            start,
            end,
            left: String::new(),
        };
        if self.totals.get(&span).copied().unwrap_or(0.0) == 0.0 {
            span.left = left.to_string();
        }
        let choice = self
            .choices
            .get(&span)
            .and_then(|words| words.get(word))
            .copied()
            .unwrap_or(0.0);
        (choice, self.totals.get(&span).copied().unwrap_or(0.0))
    }

    /// 是否沒有任何證據。
    pub fn is_empty(&self) -> bool {
        self.choices.is_empty()
    }
}

impl Session {
    /// 由目前的手選鎖定重建本地證據；沒有可用鎖定時回傳 `None`。
    ///
    /// `base` 是第一個要看的音節索引。同讀音且左或右相鄰音節相同時提供一次
    /// 暫時選取證據；相鄰詞只給 0.25 的弱觀察，且必須是詞庫實際有的詞。
    /// 這一輪只提供查詢，尚未接進解碼器（見 port 文件的 deferred 表）。
    pub fn local_evidence(&self, base: usize) -> Option<LocalEvidence> {
        if !self.config.learning || self.locks.is_empty() {
            return None;
        }
        let syllables = &self.syllables;
        if base >= syllables.len() {
            return None;
        }
        // 上游逐格記錄字面文字，碰到字面就切斷；本埠的字面是單一字串，
        // 沒有逐格位置，因此整段音節都算同一個範圍。
        let limit = syllables.len();
        let path = self
            .current_hypothesis()
            .map(|hypothesis| hypothesis.segments.clone())
            .unwrap_or_default();
        struct Entry {
            start: usize,
            word: String,
            reading: Vec<Syllable>,
            left: Option<Syllable>,
            right: Option<Syllable>,
            next_word: Option<String>,
            next_reading: Vec<Syllable>,
        }
        let mut entries: Vec<Entry> = Vec::new();
        // 依位置排序，避免 map 順序影響同範圍覆蓋。
        for at in base..limit {
            let Some(lock) = self.locks.get(&at) else {
                continue;
            };
            if lock.len == 0
                || lock.completion
                || lock.literal_boundary != 0
                || !lock.matched.is_empty()
                || at + lock.len > limit
                || lock.word.chars().count() != lock.len
                || self.personal.is_forgotten(&lock.word)
            {
                continue;
            }
            let end = at + lock.len;
            let left = at.checked_sub(1).and_then(|i| syllables.get(i).cloned());
            let right = syllables.get(end).cloned();
            let mut entry = Entry {
                start: at,
                word: lock.word.clone(),
                reading: syllables[at..end].to_vec(),
                left,
                right,
                next_word: None,
                next_reading: Vec::new(),
            };
            if entry.right.is_some() {
                if let Some(seg) = path.iter().find(|seg| seg.start == end) {
                    if seg.end <= limit {
                        let reading = reading_key(&syllables[seg.start..seg.end]);
                        if self
                            .lexicon
                            .lookup(&reading)
                            .iter()
                            .any(|item| item.surface == seg.word)
                        {
                            entry.next_word = Some(seg.word.clone());
                            entry.next_reading = syllables[seg.start..seg.end].to_vec();
                        }
                    }
                }
            }
            if entry.left.is_some() || entry.right.is_some() {
                entries.push(entry);
            }
        }
        if entries.is_empty() {
            return None;
        }
        entries.sort_by_key(|entry| entry.start);
        // 先按目標範圍彙整不同選取；重複相同更正不累加，衝突的選取各留一份。
        let mut choices: BTreeMap<Span, BTreeMap<String, f64>> = BTreeMap::new();
        let mut add = |span: Span, word: &str, weight: f64| {
            let words = choices.entry(span).or_default();
            let slot = words.entry(word.to_string()).or_insert(0.0);
            *slot = slot.max(weight);
        };
        for i in base..limit {
            for entry in &entries {
                let n = entry.reading.len();
                if n == 0 || i + n > limit || i < entry.start + n && entry.start < i + n {
                    continue;
                }
                let left = i > base
                    && entry
                        .left
                        .as_ref()
                        .is_some_and(|left| syllables.get(i - 1) == Some(left));
                let right = i + n < limit
                    && entry
                        .right
                        .as_ref()
                        .is_some_and(|right| syllables.get(i + n) == Some(right));
                if entry.right.is_some() && i + n < limit && !right {
                    continue;
                }
                if !left && !right {
                    continue;
                }
                if syllables[i..i + n] != entry.reading[..] {
                    continue;
                }
                add(
                    Span {
                        start: i - base,
                        end: i + n - base,
                        left: String::new(),
                    },
                    &entry.word,
                    1.0,
                );
                if let Some(next) = &entry.next_word {
                    let next_len = entry.next_reading.len();
                    if i + n + next_len <= limit
                        && syllables[i + n..i + n + next_len] == entry.next_reading[..]
                    {
                        add(
                            Span {
                                start: i + n - base,
                                end: i + n + next_len - base,
                                left: entry.word.clone(),
                            },
                            next,
                            0.25,
                        );
                    }
                }
            }
        }
        if choices.is_empty() {
            return None;
        }
        // `BTreeMap` 的走訪順序固定，浮點加總不受 map 順序影響。
        let mut totals: BTreeMap<Span, f64> = BTreeMap::new();
        for (span, words) in &choices {
            let total: f64 = words.values().sum();
            totals.insert(span.clone(), total);
        }
        Some(LocalEvidence { choices, totals })
    }
}

// ---------------------------------------------------------------------------
// corrections.go：修正提示
// ---------------------------------------------------------------------------

impl Session {
    /// 組字範圍內已手選的片段，依位置排序；給修正偏好使用。
    ///
    /// 對照上游 `configureCorrections` 取出 `selected` 的部分：只讀目前仍有效
    /// 的鎖定，跨音手選、字面邊界與被字面切開的暫時鎖定都不算。`local` 為
    /// `false` 時不提供當次覆蓋。實際的 `CorrectionPenalty` 與提交時的
    /// `enclosedSelections` 尚未接上（見 port 文件的 deferred 表）。
    pub fn correction_selections(&self, begin: usize, end: usize, local: bool) -> Vec<Seg> {
        let mut out = Vec::new();
        if !local || begin >= end || end > self.syllables.len() {
            return out;
        }
        for at in begin..end {
            let Some(lock) = self.locks.get(&at) else {
                continue;
            };
            if lock.literal_boundary != 0
                || !lock.matched.is_empty()
                || at + lock.len > end
                || self.literal_inside(at, at + lock.len)
            {
                continue;
            }
            out.push(Seg {
                word: lock.word.clone(),
                start: at - begin,
                end: at + lock.len - begin,
                manual: true,
                completion: lock.completion,
                ..Seg::default()
            });
        }
        out
    }
}

// ---------------------------------------------------------------------------
// document_context.go：文件層文脈
// ---------------------------------------------------------------------------

impl Session {
    /// 文件層文脈的左文：目前提交過的尾巴；沒有可用文脈時回傳 `None`。
    ///
    /// 對照上游 `contextLeft(0)`：只在句首、非私人／暫停時使用已確認的末詞。
    /// 本埠沒有私人與暫停狀態，長度上限沿用 `MAX_LEFT_CONTEXT_UNITS`。
    /// 這一輪只提供查詢，尚未混進解碼評分（見 port 文件的 deferred 表）。
    pub fn document_left(&self) -> Option<&str> {
        if !self.config.document_context || self.committed.is_empty() {
            return None;
        }
        Some(self.committed.as_str())
    }

    /// 自己最近提交的完整末詞；沒有組字、尾端不是完整詞時回傳 `None`。
    ///
    /// 對照上游 `committedTail`：只認詞庫實際有的詞，尾端前綴與字面文字都
    /// 不算。外殼確認文件更新後才可當下一段的前詞。
    pub fn committed_tail(&self) -> Option<&str> {
        if !self.config.document_context
            || !self.zh.pending().is_empty()
            || self.syllables.is_empty()
        {
            return None;
        }
        let syllables = &self.syllables;
        let hypothesis = self.current_hypothesis()?;
        let last = hypothesis.segments.last()?;
        if last.partial || last.end != syllables.len() || last.start >= last.end {
            return None;
        }
        let reading = reading_key(&syllables[last.start..last.end]);
        self.lexicon
            .lookup(&reading)
            .iter()
            .find(|entry| entry.surface == last.word)
            .map(|entry| entry.surface.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{Key, KeyCode};
    use crate::lattice::{InMemoryLexicon, LexEntry};

    fn lexicon() -> InMemoryLexicon {
        let mut lex = InMemoryLexicon::new();
        for (surface, reading) in [
            ("我", "ㄨㄛˇ"),
            ("想", "ㄒㄧㄤˇ"),
            ("你", "ㄋㄧˇ"),
            ("風", "ㄈㄥ"),
            ("風景", "ㄈㄥ ㄐㄧㄥˇ"),
        ] {
            lex.push(LexEntry::new(surface, reading, 0, 0));
        }
        lex
    }

    fn session() -> Session {
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
        Session::new(config)
    }

    fn type_syllable(session: &mut Session, reading: &str) {
        let keyboard = crate::zh::keyboard::Keyboard::standard();
        for syllable in crate::zh::syllable::parse_reading(reading) {
            let keys = keyboard.keys(&syllable).expect("音節應該有對應按鍵");
            for ch in keys.chars() {
                session.press(Key::char(ch, false));
            }
        }
    }

    fn composer(reading: &str) -> Composer {
        let mut composer = Composer::new();
        for ch in reading.chars() {
            composer.add(ch);
        }
        composer
    }

    fn literal(start: usize, reading: &str, tone: &str) -> PendingLiteral {
        let syllable = Syllable::parse(&format!("{reading}{tone}")).unwrap();
        PendingLiteral::from_syllable(start, composer(reading), &syllable)
    }

    #[test]
    fn test_pending_literal_positions_and_syllable() {
        let pending = literal(2, "ㄍㄨㄜ", "ˊ");
        assert_eq!(pending.start(), 2);
        assert_eq!(pending.end(), 5);
        assert_eq!(pending.mark(), 1);
        assert_eq!(pending.syllable().as_str(), "ㄍㄨㄜˊ");
        assert!(!pending.is_pending());
        assert!(!pending.prefix());

        let first_tone = literal(0, "ㄏㄟ", "");
        assert_eq!(first_tone.mark(), 0);
        assert_eq!(first_tone.syllable().as_str(), "ㄏㄟ");

        // 沒打聲調（聲調選填）的音節：畫面上不標聲調，收回解碼時不限聲調。
        let untoned =
            PendingLiteral::from_syllable(1, composer("ㄋㄧ"), &Syllable::from_raw("ㄋㄧ?"));
        assert_eq!(untoned.mark(), 0);
        assert!(untoned.syllable().untoned());
        assert_eq!(untoned.syllable().text(), "ㄋㄧ");
    }

    #[test]
    fn test_shift_pending_literal_insert_before_start() {
        let mut literals = vec![literal(2, "ㄍㄨㄜ", "ˊ")];
        shift_pending_literals(&mut literals, 1, 1);
        assert_eq!(literals[0].start(), 3);
        assert_eq!(literals[0].mark(), 1);
    }

    #[test]
    fn test_shift_pending_literal_insert_at_start() {
        let mut literals = vec![literal(2, "ㄍㄨㄜ", "ˊ")];
        shift_pending_literals(&mut literals, 2, 1);
        assert_eq!(literals[0].start(), 3);
    }

    #[test]
    fn test_shift_pending_literal_edit_inside_drops_literal() {
        let mut literals = vec![literal(2, "ㄍㄨㄜ", "ˊ")];
        shift_pending_literals(&mut literals, 4, 1);
        assert!(literals.is_empty());
    }

    #[test]
    fn test_shift_pending_literal_insert_before_tone_drops_literal() {
        let mut literals = vec![literal(2, "ㄍㄨㄜ", "ˊ")];
        shift_pending_literals(&mut literals, 5, 1);
        assert!(literals.is_empty());
    }

    #[test]
    fn test_shift_pending_literal_delete_tone_returns_to_pending() {
        let mut literals = vec![literal(2, "ㄍㄨㄜ", "ˊ")];
        shift_pending_literals(&mut literals, 5, -1);
        assert_eq!(literals.len(), 1);
        assert!(literals[0].is_pending());
        assert_eq!(literals[0].mark(), 0);
        assert_eq!(literals[0].syllable().as_str(), "ㄍㄨㄜ");
    }

    #[test]
    fn test_shift_pending_literal_after_end_keeps_literal() {
        let mut literals = vec![literal(2, "ㄍㄨㄜ", "ˊ")];
        shift_pending_literals(&mut literals, 7, -1);
        assert_eq!(literals[0].start(), 2);
        assert_eq!(literals[0].mark(), 1);
    }

    #[test]
    fn test_restorable_pending_literal_needs_cursor_and_no_tone() {
        let mut literals = vec![literal(2, "ㄍㄨㄜ", "ˊ")];
        assert_eq!(restorable_pending_literal(&literals, 5), None);
        shift_pending_literals(&mut literals, 5, -1);
        assert_eq!(restorable_pending_literal(&literals, 5), Some(0));
        assert_eq!(restorable_pending_literal(&literals, 4), None);
    }

    #[test]
    fn test_vocab_mark_hint_and_add_word() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_syllable(&mut s, "ㄒㄧㄤˇ");
        assert_eq!(
            s.mark_hint(0, 2),
            "Enter 加入詞庫：我想（ㄨㄛˇ ㄒㄧㄤˇ）　Esc 取消"
        );
        let mark = s.add_composition_word(0, 2);
        assert!(mark.changed);
        assert_eq!(mark.notice, "已加入詞庫：我想");
        assert!(s.personal.vocab_has_reading("我想", "ㄨㄛˇ ㄒㄧㄤˇ"));
        assert_eq!(s.snapshot().preedit, "我想");

        // 加入後立刻打得出同一個詞。
        s.press(Key::new(KeyCode::Escape, None, false));
        type_syllable(&mut s, "ㄨㄛˇ");
        type_syllable(&mut s, "ㄒㄧㄤˇ");
        assert_eq!(s.snapshot().preedit, "我想");
    }

    #[test]
    fn test_vocab_mark_system_word_is_priority() {
        let mut s = session();
        type_syllable(&mut s, "ㄈㄥ");
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        assert_eq!(
            s.mark_hint(0, 2),
            "Enter 設為優先：風景（ㄈㄥ ㄐㄧㄥˇ）　Esc 取消"
        );
        let mark = s.add_composition_word(0, 2);
        assert!(mark.changed);
        assert_eq!(mark.notice, "已設為優先：風景");
        // 第二次加入：已在詞庫。
        assert_eq!(s.mark_hint(0, 2), "「風景」已在詞庫　Esc 取消");
        assert_eq!(s.add_composition_word(0, 2).notice, "「風景」已在詞庫");
    }

    #[test]
    fn test_vocab_mark_rejects_single_char_and_empty_range() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        assert_eq!(
            s.mark_hint(0, 1),
            "再多選一個字（單字的讀音請到設定程式「我的詞彙」加入）"
        );
        assert_eq!(
            s.add_composition_word(0, 1).notice,
            "至少選兩個字；單字的讀音請到設定程式「我的詞彙」加入"
        );
        assert_eq!(s.mark_hint(1, 1), "");
        assert!(!s.add_composition_word(1, 1).changed);
    }

    #[test]
    fn test_local_evidence_from_neighbouring_lock() {
        let mut s = Session::new({
            let mut config = SessionConfig::new(Mode::Zh);
            let mut lex = InMemoryLexicon::new();
            lex.push(LexEntry::new("新", "ㄒㄧㄣ", 0, 0));
            lex.push(LexEntry::new("圖", "ㄊㄨˊ", 0, 0));
            config.zh_lexicon = Some(std::sync::Arc::new(lex));
            config
        });
        type_syllable(&mut s, "ㄒㄧㄣ");
        type_syllable(&mut s, "ㄊㄨˊ");
        type_syllable(&mut s, "ㄒㄧㄣ");
        type_syllable(&mut s, "ㄊㄨˊ");
        assert!(s.lock_segment(0));
        let evidence = s.local_evidence(0).expect("鎖定應提供證據");
        // 手選的第一次「新圖」本身是硬鎖定，不重複累加證據。
        assert_eq!(evidence.at(0, 1, "", "新"), (0.0, 0.0));
        // 第二次同樣的讀音、同樣的右鄰：提供一份暫時選取證據。
        assert_eq!(evidence.at(2, 3, "", "新"), (1.0, 1.0));
        // 後面的「圖」是詞庫實際有的詞：弱觀察 0.25。
        assert_eq!(evidence.at(3, 4, "新", "圖"), (0.25, 0.25));
    }

    #[test]
    fn test_local_evidence_requires_lock_and_learning() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_syllable(&mut s, "ㄒㄧㄤˇ");
        assert!(s.local_evidence(0).is_none());
        assert!(s.lock_segment(0));
        s.config.learning = false;
        assert!(s.local_evidence(0).is_none());
    }

    #[test]
    fn test_correction_selections_reflect_locks() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_syllable(&mut s, "ㄒㄧㄤˇ");
        assert!(s.correction_selections(0, 2, true).is_empty());
        assert!(s.lock_segment(0));
        let selected = s.correction_selections(0, 2, true);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].word, "我");
        assert_eq!((selected[0].start, selected[0].end), (0, 1));
        assert!(selected[0].manual);
        assert!(!selected[0].completion);
        // 範圍外的鎖定不算；local 關閉時不提供當次覆蓋。
        assert!(s.correction_selections(1, 2, true).is_empty());
        assert!(s.correction_selections(0, 2, false).is_empty());
    }

    #[test]
    fn test_document_context_uses_committed_tail() {
        let mut s = session();
        assert_eq!(s.document_left(), None);
        assert_eq!(s.committed_tail(), None);
        type_syllable(&mut s, "ㄈㄥ");
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        assert_eq!(s.committed_tail(), Some("風景"));
        assert_eq!(s.commit(), "風景");
        assert_eq!(s.document_left(), Some("風景"));
        assert_eq!(s.committed_tail(), None);

        // 尾端不是詞庫實際有的完整詞時不算末詞。
        let mut config = SessionConfig::new(Mode::Zh);
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("風景", "ㄈㄥ ㄐㄧㄥˇ", 0, 0));
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄈㄥ");
        assert_eq!(s.committed_tail(), None);

        // 設定關閉時不提供文脈。
        let mut s = session();
        type_syllable(&mut s, "ㄈㄥ");
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        assert_eq!(s.committed_tail(), Some("風景"));
        assert_eq!(s.commit(), "風景");
        assert_eq!(s.document_left(), Some("風景"));
        s.config.document_context = false;
        assert_eq!(s.document_left(), None);
        assert_eq!(s.committed_tail(), None);
    }
}
