//! 分段、片段鎖定與候選呈現。
//!
//! 對照上游 `core/session` 的 `segments.go`、`untoned.go`、`tone_mark.go` 與
//! `phonetic_candidates.go`。上游的 `Segment` 帶讀音、語言標籤與 rune 範圍；
//! `jc-core::Segment` 只帶 UTF-8 位元組範圍、模式與書寫系統，因此這一層以
//! 現有型別能表達的部分為準：
//!
//! - 片段本身由 [`Session`] 的轉換結果推導，呼叫端拿到副本。
//! - `ReplaceSegment`／`InsertSegment`／`RemoveSegment` 建立可編輯副本，下一次
//!   按鍵後失效，與上游的 `segmentManual`／`segmentRev` 相同。
//! - 片段鎖定直接寫進解碼器的 [`crate::decode::Locks`]，因此會影響後續轉換。
//!
//! 游標唯一存成顯示字串的 rune 位移：`ZhInput` 的暫存與已完成音節、日文
//! byte 與混打 rune 都只是換算出來的視圖。任意位置插入與刪除仍未接。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use super::*;
use crate::lattice::Lexicon;
use crate::zh::syllable::{class_of, Class, Syllable, TONE1};

/// 游標的 lane 視圖。
///
/// 實際儲存只有 [`Session::cursor_rune`]；這裡是各 lane 對外的語意單位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LaneCursor {
    /// 注音音節格（`0..=syllables.len()`）。
    Cell(usize),
    /// 日文顯示字串的 UTF-8 位元組位移。
    Byte(usize),
    /// 顯示字串的 rune 位移。
    Rune(usize),
}

/// 剛按下、還沒決定要當數字或聲調符號的聲調鍵。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingTone {
    /// 按鍵在 `literal` 中的位元組位置。
    at: usize,
    /// 原本的按鍵字元。
    key: char,
}

/// 沒在組字時，聲調鍵是否先由輸入法留在組字區等空白。
///
/// 對照上游 `ToneMarkKeys`：開了中英混打且使用標準排列時，聲調鍵交給混打
/// 判斷；其他情況先留在組字區，緊接的空白才換成聲調符號。
pub fn tone_mark_keys(mixed_input: bool, standard_layout: bool) -> bool {
    !(mixed_input && standard_layout)
}

impl Session {
    /// 依「打錯注音時提示」安裝聲調閘門（上游 `refuseTone`）。
    ///
    /// 只有開啟設定、注音模式、沒有開聲調選填時才擋。擋下的條件是：加聲調
    /// 後的讀音在詞典裡完全沒有字、也不是任何詞的開頭，模糊音近音也沒有字；
    /// 單一注音符號的讀音不擋（注音文）。同一處、同一讀音、同一聲調連按第二
    /// 次視為使用者故意打注音文，放行一次。
    pub(super) fn install_tone_gate(&mut self) {
        if !self.config.zhuyin_error_hint {
            self.zh.set_tone_gate(crate::zh::input::allow_all_tones);
            return;
        }
        let lexicon = self.lexicon.clone();
        let fuzzy = self.config.fuzzy_pairs.clone();
        let toneless = self.config.toneless;
        let retry = Arc::new(Mutex::new(None::<(String, String)>));
        self.zh.set_tone_gate(move |pending, done| {
            if toneless || done.untoned() || single_symbol(pending) {
                return false;
            }
            let key = (pending.to_string(), done.as_str().to_string());
            {
                let mut last = lock(&retry);
                if last.as_ref() == Some(&key) {
                    *last = None;
                    return false;
                }
            }
            let reachable = reachable(&lexicon, done.as_str())
                || fuzzy
                    .iter()
                    .flat_map(|&(left, right)| {
                        fuzzy_spellings(done.as_str(), left, right).into_iter()
                    })
                    .any(|reading| reachable(&lexicon, &reading));
            if !reachable {
                *lock(&retry) = Some(key);
            }
            !reachable
        });
    }

    /// 目前片段的副本。
    ///
    /// 呼叫端修改回傳值不影響 session。手動編輯過的片段會原樣回傳，直到下一次
    /// 按鍵讓組字狀態重新推導為止。
    pub fn segments(&self) -> Vec<Segment> {
        let mut segments = self
            .segment_manual
            .clone()
            .unwrap_or_else(|| self.segments.clone());
        if !self.literal_tail.is_empty() {
            let end = self.composed_text().len();
            let start = end.saturating_sub(self.literal_tail.len());
            segments.push(Segment::new(start, end, Lang::En, Script::Latin));
        }
        segments
    }

    /// 游標在目前組字字串中的 rune 位置。
    ///
    /// 內部只保存 [`Session::cursor_rune`]；音節格與 byte 都是推導值。游標
    /// 永遠落在 rune 邊界，超出目前字串時夾在結尾。
    pub fn cursor(&self) -> usize {
        match self.to_lane_cursor() {
            LaneCursor::Cell(_) => self.cursor_runes(),
            LaneCursor::Byte(byte) => {
                let text = self.composed_text();
                rune_offset(&text, byte.min(text.len()))
            }
            LaneCursor::Rune(rune) => rune,
        }
    }

    /// 目前 lane 的游標視圖。
    ///
    /// 注音換成音節格、日文換成 byte、混打維持 rune；這三種表示都只是
    /// [`Session::cursor_rune`] 的推導值，不再各自保存。
    pub(super) fn to_lane_cursor(&self) -> LaneCursor {
        if self.in_zh_lane() {
            LaneCursor::Cell(self.zh_cell())
        } else if self.in_ja_lane() {
            LaneCursor::Byte(self.ja_byte())
        } else {
            LaneCursor::Rune(self.mixed_rune())
        }
    }

    /// 把 lane 的游標寫回唯一的 `cursor_rune`。
    ///
    /// 介面名稱沿用文件裡的 `from_lane_cursor`；它是就地更新，不建立新值。
    #[allow(clippy::wrong_self_convention)]
    pub(super) fn from_lane_cursor(&mut self, cursor: LaneCursor) {
        match cursor {
            LaneCursor::Cell(cell) => self.cursor_rune = self.zh_rune_at_cell(cell),
            LaneCursor::Byte(byte) => {
                let text = self.composed_text();
                self.cursor_rune = rune_offset(&text, byte.min(text.len()));
            }
            LaneCursor::Rune(rune) => {
                self.cursor_rune = if rune == Self::MIXED_CURSOR_END {
                    Self::MIXED_CURSOR_END
                } else {
                    rune.min(self.rune_count())
                };
            }
        }
    }

    /// 不含未完成注音與字面尾巴的顯示字串 rune 數。
    ///
    /// 直接由來源字串計算，不呼叫 [`Session::composed_text`]；後者在注音
    /// lane 會依游標插入未完成注音，反過來需要游標位置。
    pub(super) fn base_rune_count(&self) -> usize {
        self.insert_literal(&self.displayed_base()).chars().count()
    }

    /// 目前顯示字串的 rune 數。
    pub(super) fn rune_count(&self) -> usize {
        self.base_rune_count() + self.pending.chars().count() + self.literal_tail.chars().count()
    }

    /// 正規化後的游標 rune；結尾哨兵夾到目前長度。
    ///
    /// 注音 lane 的未完成注音插在游標位置，對外游標算在它後面；內部
    /// [`Session::cursor_rune`] 只記穩定字串的邊界，暫存注音增減不會讓游標漂移。
    pub(super) fn cursor_runes(&self) -> usize {
        if self.cursor_rune == Self::MIXED_CURSOR_END {
            return self.rune_count();
        }
        if self.in_zh_lane() {
            let base = self.base_rune_count();
            let raw = self.cursor_rune.min(base);
            if raw >= base {
                // 舊路徑在音節結尾把字面也算進游標，維持同一個落點。
                self.rune_count()
            } else {
                (raw + self.pending.chars().count()).min(self.rune_count())
            }
        } else {
            self.cursor_rune.min(self.rune_count())
        }
    }

    /// 注音 lane 的游標音節格。
    pub(super) fn zh_cell(&self) -> usize {
        if self.in_zh_lane() {
            let cells = self.syllables.len();
            if self.cursor_rune == Self::MIXED_CURSOR_END
                || self.cursor_runes() >= self.rune_count()
            {
                return cells;
            }
            let target = self.cursor_rune.min(self.base_rune_count());
            let mut best = 0;
            for cell in 0..=cells {
                if self.zh_cell_rune(cell) <= target {
                    best = cell;
                } else {
                    break;
                }
            }
            best.min(cells)
        } else {
            self.cursor_runes()
        }
    }

    /// 把注音 lane 的游標設到音節格 `cell`。
    pub(super) fn set_zh_cell(&mut self, cell: usize) {
        self.from_lane_cursor(LaneCursor::Cell(cell));
    }

    pub(super) fn zh_cell_rune(&self, cell: usize) -> usize {
        let display = self.displayed_base();
        let byte = self
            .cell_byte(&display, cell)
            .unwrap_or(display.len())
            .min(display.len());
        let mut rune = rune_offset(&display, byte);
        if !self.literal.is_empty() {
            let mut at_literal = self.literal_offset.min(display.len());
            while at_literal > 0 && !display.is_char_boundary(at_literal) {
                at_literal -= 1;
            }
            if at_literal <= byte {
                rune += self.literal.chars().count();
            }
        }
        rune
    }

    /// 音節格在穩定顯示字串中的 rune 位置；未完成注音另外由游標推導。
    pub(super) fn zh_rune_at_cell(&self, cell: usize) -> usize {
        self.zh_cell_rune(cell.min(self.syllables.len()))
    }

    /// 日文 lane 的游標 byte 位移。
    pub(super) fn ja_byte(&self) -> usize {
        byte_at_rune(&self.composed_text(), self.cursor_runes())
    }

    /// 把日文 lane 的游標設到 byte 位移 `byte`。
    pub(super) fn set_ja_byte(&mut self, byte: usize) {
        self.from_lane_cursor(LaneCursor::Byte(byte));
    }

    /// 混打 lane 的游標 rune 位移。
    pub(super) fn mixed_rune(&self) -> usize {
        self.cursor_runes()
    }

    /// 把混打 lane 的游標設到 rune 位移 `rune`。
    pub(super) fn set_mixed_rune(&mut self, rune: usize) {
        self.from_lane_cursor(LaneCursor::Rune(rune));
    }

    /// 組字游標在顯示字串中的 UTF-8 位元組位置。
    pub(super) fn cursor_byte(&self) -> usize {
        byte_at_rune(&self.composed_text(), self.cursor_runes())
    }

    /// 把音節格換成 `base` 內的位元組位置。
    ///
    /// 只在目前路徑對得上顯示字串時換算；對不上（例如接受的補全）回傳
    /// `None`，由呼叫端把游標放在結尾。
    pub(super) fn cell_byte(&self, base: &str, cell: usize) -> Option<usize> {
        let path = self
            .last_hypotheses
            .iter()
            .find(|hypothesis| hypothesis.text == base)
            .or_else(|| {
                (base == self.base)
                    .then(|| self.last_hypotheses.first())
                    .flatten()
            })?
            .segments
            .as_slice();
        let mut byte = 0;
        for segment in path {
            if cell <= segment.start {
                return Some(byte);
            }
            if cell < segment.end {
                let offset = cell - segment.start;
                return Some(byte + byte_at_rune(&segment.word, offset));
            }
            byte += segment.word.len();
        }
        Some(byte.min(base.len()))
    }

    /// 最近一次按鍵是否因「沒有字」而擋下聲調。
    pub fn tone_blocked(&self) -> bool {
        self.zh.tone_blocked()
    }

    /// 游標所在片段的索引；沒有片段時回傳 `None`。
    ///
    /// 游標在片段結尾時屬於下一個片段（半開區間）；在整段輸入結尾時仍算最後
    /// 一個片段，因為那裡是使用者正在編輯的位置。
    pub fn active_segment(&self) -> Option<usize> {
        let segments = self.segments();
        let text = self.composed_text();
        let byte = byte_at_rune(&text, self.cursor());
        for (index, segment) in segments.iter().enumerate() {
            if segment.start <= byte && byte < segment.end {
                return Some(index);
            }
        }
        let end = segments.iter().map(|segment| segment.end).max()?;
        (byte >= end).then(|| segments.len() - 1)
    }

    /// 回傳包含 rune 位置 `pos` 的片段索引。
    ///
    /// 位置超出組字字串時回傳 `None`；片段結尾屬於下一個片段。
    pub fn segment_at(&self, pos: usize) -> Option<usize> {
        let text = self.composed_text();
        let byte = byte_at_rune(&text, pos);
        self.segments()
            .iter()
            .position(|segment| segment.start <= byte && byte < segment.end)
    }

    /// 替換索引 `index` 的片段，保留其他片段。
    ///
    /// 空片段與越界索引回傳 `false`。替換只影響 [`Session::segments`] 的視圖，
    /// 下一次按鍵後由組字狀態重新推導。
    pub fn replace_segment(&mut self, index: usize, segment: Segment) -> bool {
        if segment.is_empty() {
            return false;
        }
        let Some(editable) = self.editable_segments() else {
            return false;
        };
        let Some(slot) = editable.get_mut(index) else {
            return false;
        };
        *slot = segment;
        true
    }

    /// 在索引 `index` 前插入片段；`index` 等於片段數時附加在尾端。
    ///
    /// 空片段與越界索引回傳 `false`。
    pub fn insert_segment(&mut self, index: usize, segment: Segment) -> bool {
        if segment.is_empty() {
            return false;
        }
        let Some(editable) = self.editable_segments() else {
            return false;
        };
        if index > editable.len() {
            return false;
        }
        editable.insert(index, segment);
        true
    }

    /// 移除索引 `index` 的片段；不合併相鄰片段。
    ///
    /// 越界索引回傳 `false`。
    pub fn remove_segment(&mut self, index: usize) -> bool {
        let Some(editable) = self.editable_segments() else {
            return false;
        };
        if index >= editable.len() {
            return false;
        }
        editable.remove(index);
        true
    }

    /// 回報第 `index` 個片段是否被片段鎖定覆蓋。
    pub fn segment_locked(&self, index: usize) -> bool {
        let Some((start, end)) = self.segment_syllables(index) else {
            return false;
        };
        self.locks
            .iter()
            .any(|(&at, lock)| at < end && start < at.saturating_add(lock.len))
    }

    /// 鎖定第 `index` 個片段，詞面就是目前片段顯示的文字。
    ///
    /// 一般選字不走這條路；這是給沒有候選視窗的呼叫端（例如滑鼠選取）用的。
    /// 鎖定會立即重建轉換結果，之後打的新音節不會改掉這一段。越界或空片段
    /// 回傳 `false`。
    pub fn lock_segment(&mut self, index: usize) -> bool {
        let Some((start, end)) = self.segment_syllables(index) else {
            return false;
        };
        if start >= end {
            return false;
        }
        let segments = self.segments();
        let Some(segment) = segments.get(index) else {
            return false;
        };
        let Some(word) = self.base.get(segment.start..segment.end) else {
            return false;
        };
        let word = word.to_string();
        if !self.mark_zh_user_syllables(start, end - start, &word) {
            return false;
        }
        self.segment_manual = None;
        self.rebuild();
        true
    }

    /// 解除第 `index` 個片段上的片段鎖定。
    ///
    /// 沒有可解除的鎖定時回傳 `false`。
    pub fn unlock_segment(&mut self, index: usize) -> bool {
        let Some((start, end)) = self.segment_syllables(index) else {
            return false;
        };
        let changed = self.demote_zh_syllables(start, end - start);
        if changed {
            self.segment_manual = None;
            self.rebuild();
        }
        changed
    }

    /// 下一次按鍵前讓手動片段副本失效。
    pub(super) fn expire_segment_view(&mut self) {
        self.segment_manual = None;
    }

    /// 聲調鍵、聲調選填與孤立字面空白的前置處理。
    ///
    /// 回傳 `true` 表示這一鍵已經處理完；`false` 表示仍要交給一般按鍵路徑。
    pub(super) fn segments_press(&mut self, key: &Key) -> bool {
        // 日文 lane 的數字不是注音聲調：直接交給 `press_char`，由羅馬字
        // 狀態機判斷接不上而結束推斷、把按鍵交回混打。
        if self.in_ja_lane() {
            self.pending_tone = None;
            return false;
        }
        // 混打已經有英數待定按鍵時，聲調鍵是那個詞的一部分，不是孤立聲調。
        if !self.ascii.is_empty() {
            self.pending_tone = None;
            return false;
        }
        match key.code {
            KeyCode::Char(ch) => {
                if ch == '\'' && !self.zh.pending().is_empty() && self.config.toneless {
                    // 對照 `untonedSeparator`：組字中按 ' 只切音節，不送出頓號。
                    self.complete_pending();
                    self.pending_tone = None;
                    return true;
                }
                self.untoned_break(ch);
                if self.tone_key(*key) {
                    return true;
                }
                self.pending_tone = None;
                false
            }
            KeyCode::Space => {
                if self.finish_tone_key() {
                    return true;
                }
                self.pending_tone = None;
                self.literal_space()
            }
            KeyCode::Enter | KeyCode::Tab | KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                if self.finish_untoned() {
                    // 先讓新完成的音節進轉換結果，Enter／Tab 才看得到它。
                    self.rebuild();
                }
                self.pending_tone = None;
                false
            }
            _ => {
                self.pending_tone = None;
                false
            }
        }
    }

    /// 沒在組字時按下聲調鍵：先照字面留下數字，等下一鍵決定。
    fn tone_key(&mut self, key: Key) -> bool {
        if !self.zh.pending().is_empty() || self.candidate_window || self.menu.is_some() {
            return false;
        }
        let KeyCode::Char(ch) = key.code else {
            return false;
        };
        let Some(symbol) = self.zh.keyboard().symbol(ch) else {
            return false;
        };
        if class_of(symbol) != Class::Tone || symbol == TONE1 {
            return false;
        }
        // 前面緊接著數字時是數字的一部分，不當成孤立聲調鍵。
        let lone = !self
            .literal
            .chars()
            .last()
            .is_some_and(|last| last.is_ascii_digit());
        self.push_literal(&ch.to_string());
        if lone {
            self.pending_tone = Some(PendingTone {
                at: self.literal.len().saturating_sub(1),
                key: ch,
            });
        } else {
            self.pending_tone = None;
        }
        true
    }

    /// 把緊接在空白前的孤立聲調鍵換成聲調符號。
    fn finish_tone_key(&mut self) -> bool {
        let Some(pending) = self.pending_tone else {
            return false;
        };
        self.pending_tone = None;
        if !self.zh.pending().is_empty()
            || self.literal.len() != pending.at.saturating_add(1)
            || !self.literal.ends_with(pending.key)
        {
            return false;
        }
        let Some(symbol) = self.zh.keyboard().symbol(pending.key) else {
            return false;
        };
        if class_of(symbol) != Class::Tone || symbol == TONE1 {
            return false;
        }
        self.literal.pop();
        self.literal.push(symbol);
        true
    }

    /// 沒有注音組字、只有字面內容時，空白照字面留下。
    fn literal_space(&mut self) -> bool {
        if !self.zh.pending().is_empty()
            || !self.syllables.is_empty()
            || self.literal.is_empty()
            || self.candidate_window
            || self.menu.is_some()
        {
            return false;
        }
        self.push_literal(" ");
        true
    }

    /// 對照 `untonedBreak`：聲調選填時，接不上的注音先完成目前的音節。
    ///
    /// 回傳 `true` 表示已把目前的音節收進組字，後續按鍵從新音節開始。現有
    /// 模型把一聲音節當成可展開聲調的音節，因此這裡用一聲完成，效果與上游的
    /// `UntonedMark` 相同。
    fn untoned_break(&mut self, ch: char) -> bool {
        if !self.config.toneless || self.zh.pending().is_empty() {
            return false;
        }
        let Some(symbol) = self.zh.keyboard().symbol(ch) else {
            return false;
        };
        let class = class_of(symbol);
        if !matches!(class, Class::Initial | Class::Medial | Class::Final) {
            return false;
        }
        let pending = self.zh.pending();
        let current = Syllable::from_raw(pending.clone());
        if !current.legal() {
            return false;
        }
        let has_class = pending.chars().any(|existing| class_of(existing) == class);
        let last_class = pending.chars().map(class_of).max().unwrap_or(Class::None);
        let extended = Syllable::from_raw(format!("{pending}{symbol}"));
        if !has_class && last_class < class && crate::zh::pinyin::legal_prefix(&extended) {
            return false;
        }
        self.complete_pending();
        true
    }

    /// 聲調選填時，在會用到整段組字的按鍵前完成未打完的音節。
    fn finish_untoned(&mut self) -> bool {
        if !self.config.toneless || self.zh.pending().is_empty() {
            return false;
        }
        if !Syllable::from_raw(self.zh.pending()).legal() {
            return false;
        }
        self.complete_pending();
        true
    }

    /// 尾端差一個注音符號的完整詞候選。
    ///
    /// 對照上游 `phoneticWindow`：從每個路徑片段的起點看到整段尾巴，長度 4 到
    /// 8 個音節，只換一個聲母／介音／韻母，聲調與音節邊界固定。這是明確列出
    /// 的候選，不會自己改字。
    pub(super) fn phonetic_candidates(&self, path: &[crate::decode::Seg]) -> Vec<Candidate> {
        let syllables = &self.syllables;
        if syllables.len() < 4 {
            return Vec::new();
        }
        let mut out: Vec<Candidate> = Vec::new();
        for segment in path {
            let start = segment.start;
            if start >= syllables.len() {
                continue;
            }
            let length = syllables.len() - start;
            if !(4..=crate::decode::MAX_WORD_LEN).contains(&length) {
                continue;
            }
            let typed = &syllables[start..];
            if typed.iter().any(|syllable| !syllable.legal()) {
                continue;
            }
            let replace: String = path
                .iter()
                .filter(|seg| seg.start >= start)
                .map(|seg| seg.word.as_str())
                .collect();
            let typed_key = typed
                .iter()
                .map(Syllable::as_str)
                .collect::<Vec<_>>()
                .join(" ");
            if self
                .lexicon
                .lookup(&typed_key)
                .iter()
                .any(|entry| entry.surface == replace)
            {
                continue;
            }
            for (at, syllable) in typed.iter().enumerate() {
                for neighbour in phonetic_neighbours(syllable) {
                    let mut reading: Vec<String> =
                        typed.iter().map(|item| item.as_str().to_string()).collect();
                    reading[at] = neighbour.with_tone(syllable.tone()).as_str().to_string();
                    let key = reading.join(" ");
                    for entry in self.lexicon.lookup(&key) {
                        if entry.surface.chars().count() != length
                            || entry.surface == replace
                            || out
                                .iter()
                                .any(|candidate| candidate.surface == entry.surface)
                        {
                            continue;
                        }
                        out.push(Candidate::new(
                            entry.surface.clone(),
                            key.clone(),
                            entry.cost,
                            CandidateSource::Lexicon,
                        ));
                    }
                }
            }
        }
        out
    }

    fn editable_segments(&mut self) -> Option<&mut Vec<Segment>> {
        if self.segment_manual.is_none() {
            self.segment_manual = Some(self.segments.clone());
        }
        self.segment_manual.as_mut()
    }

    /// 找片段對應的音節範圍，供片段鎖定使用。
    fn segment_syllables(&self, index: usize) -> Option<(usize, usize)> {
        let segments = self.segments();
        let target = segments.get(index)?;
        if let Some(path) = self.last_hypotheses.first() {
            let mut at: usize = 0;
            let mut start = None;
            let mut end = None;
            for segment in &path.segments {
                let length = segment.word.len();
                if target.start < at.saturating_add(length) && at < target.end {
                    start.get_or_insert(segment.start);
                    end = Some(segment.end);
                }
                at = at.saturating_add(length);
            }
            if let (Some(start), Some(end)) = (start, end) {
                if end > start {
                    return Some((start, end));
                }
            }
        }
        // 沒有路徑時用每個音節的顯示長度佔一段，與上游的保底映射相同。
        let mut at: usize = 0;
        let mut start = None;
        let mut end = None;
        for (position, syllable) in self.syllables.iter().enumerate() {
            let length = syllable.text().len();
            if target.start < at.saturating_add(length) && at < target.end {
                start.get_or_insert(position);
                end = Some(position + 1);
            }
            at = at.saturating_add(length);
        }
        match (start, end) {
            (Some(start), Some(end)) if end > start => Some((start, end)),
            _ => None,
        }
    }
}

/// 把 rune 位置換成 UTF-8 位元組位置；超出長度時回傳字串長度。
fn byte_at_rune(text: &str, pos: usize) -> usize {
    text.char_indices()
        .nth(pos)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len())
}

/// 取得互斥鎖；被毒化時仍取回內容，聲調判斷不需要 panic。
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// 讀音在詞典裡有字，或是某個詞的開頭。
fn reachable<L: Lexicon + ?Sized>(lexicon: &L, reading: &str) -> bool {
    !lexicon.lookup(reading).is_empty() || !lexicon.prefix(reading).is_empty()
}

/// 暫存注音只有一個符號（聲母、介音、韻母其中一類）。
fn single_symbol(pending: &str) -> bool {
    let mut seen = [false; 3];
    for ch in pending.chars() {
        match class_of(ch) {
            Class::Initial => seen[0] = true,
            Class::Medial => seen[1] = true,
            Class::Final => seen[2] = true,
            _ => {}
        }
    }
    seen.iter().filter(|present| **present).count() == 1
}

/// 一組模糊音配對的所有換法；沒有出現的配對回傳空清單。
fn fuzzy_spellings(reading: &str, left: char, right: char) -> Vec<String> {
    let mut out = Vec::new();
    for (from, to) in [(left, right), (right, left)] {
        if reading.contains(from) {
            let swapped: String = reading
                .chars()
                .map(|ch| if ch == from { to } else { ch })
                .collect();
            if swapped != reading {
                out.push(swapped);
            }
        }
    }
    out
}

/// 只比注音符號：聲調與音節邊界固定，相同讀音不算。
///
/// 對照上游 `oneSymbolApart`。
fn one_symbol_apart(a: &Syllable, b: &Syllable) -> bool {
    if a.tone() != b.tone() || a == b {
        return false;
    }
    let mut x: Vec<char> = a.base().as_str().chars().collect();
    let mut y: Vec<char> = b.base().as_str().chars().collect();
    if x.len() > y.len() {
        std::mem::swap(&mut x, &mut y);
    }
    if y.len() - x.len() > 1 {
        return false;
    }
    let (mut i, mut j, mut edits) = (0, 0, 0);
    while i < x.len() && j < y.len() {
        if x[i] == y[j] {
            i += 1;
            j += 1;
            continue;
        }
        edits += 1;
        if edits > 1 {
            return false;
        }
        if x.len() == y.len() {
            i += 1;
        }
        j += 1;
    }
    edits + y.len() - j == 1
}

/// 與指定音節只差一個注音符號的音節表；第一次使用時由音節表建立。
fn phonetic_neighbours(syllable: &Syllable) -> Vec<Syllable> {
    static TABLE: OnceLock<HashMap<String, Vec<Syllable>>> = OnceLock::new();
    let base = syllable.base();
    TABLE
        .get_or_init(|| {
            let inventory = crate::zh::pinyin::inventory();
            let mut table: HashMap<String, Vec<Syllable>> = HashMap::new();
            for left in &inventory {
                for right in &inventory {
                    if one_symbol_apart(left, right) {
                        table
                            .entry(left.as_str().to_string())
                            .or_default()
                            .push(right.clone());
                    }
                }
            }
            table
        })
        .get(base.as_str())
        .cloned()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::LexEntry;

    fn lexicon() -> InMemoryLexicon {
        let mut lexicon = InMemoryLexicon::new();
        for (surface, reading) in [
            ("我", "ㄨㄛˇ"),
            ("想", "ㄒㄧㄤˇ"),
            ("吃", "ㄔ"),
            ("風", "ㄈㄥ"),
            ("景", "ㄐㄧㄥˇ"),
            ("風景", "ㄈㄥ ㄐㄧㄥˇ"),
            ("個", "ㄍㄜˋ"),
            ("你", "ㄋㄧˇ"),
            ("好", "ㄏㄠˇ"),
            ("你好", "ㄋㄧˇ ㄏㄠˇ"),
            ("西", "ㄒㄧ"),
            ("安", "ㄢ"),
            ("西安", "ㄒㄧ ㄢ"),
            ("仙", "ㄒㄧㄢ"),
            ("書", "ㄕㄨ"),
            ("麻婆豆腐", "ㄇㄚˊ ㄆㄛˊ ㄉㄡˋ ㄈㄨˇ"),
        ] {
            lexicon.push(LexEntry::new(surface, reading, 0, 0));
        }
        lexicon
    }

    fn session() -> Session {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
        Session::new(config)
    }

    fn toneless_session() -> Session {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
        config.toneless = true;
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

    fn type_keys(session: &mut Session, keys: &str) {
        for ch in keys.chars() {
            let key = if ch == ' ' {
                Key::new(KeyCode::Space, None, false)
            } else {
                Key::char(ch, false)
            };
            session.press(key);
        }
    }

    /// 換算介面的不變量：每個合法位置都回到同一個 lane 游標。
    #[test]
    fn test_lane_cursor_round_trip_for_every_position() {
        // 注音：音節格 ↔ rune。
        let mut zh = session();
        type_syllable(&mut zh, "ㄨㄛˇ");
        type_syllable(&mut zh, "ㄒㄧㄤˇ");
        type_syllable(&mut zh, "ㄔ");
        for cell in 0..=zh.syllables.len() {
            zh.from_lane_cursor(LaneCursor::Cell(cell));
            assert_eq!(
                zh.to_lane_cursor(),
                LaneCursor::Cell(cell),
                "注音音節格 {cell} 換算後要回到同一格"
            );
            assert!(
                zh.cursor_byte() <= zh.composed_text().len(),
                "游標 byte 不能超過顯示字串"
            );
            assert!(
                zh.composed_text().is_char_boundary(zh.cursor_byte()),
                "游標要落在 UTF-8 邊界"
            );
        }

        // 字面插在音節前面（`，我想`）時，每個音節格仍要換算回同一格。
        let mut front = session();
        type_syllable(&mut front, "ㄨㄛˇ");
        type_syllable(&mut front, "ㄒㄧㄤˇ");
        front.literal_anchor = 0;
        front.literal = "，".to_string();
        front.rebuild();
        assert_eq!(front.snapshot().preedit, "，我想");
        for cell in 0..=front.syllables.len() {
            front.from_lane_cursor(LaneCursor::Cell(cell));
            assert_eq!(
                front.to_lane_cursor(),
                LaneCursor::Cell(cell),
                "前導字面下音節格 {cell} 換算後要回到同一格"
            );
        }

        // 日文：byte ↔ rune。
        let mut ja = session();
        ja.config.ja_only = true;
        ja.base = "わたし".to_string();
        ja.cursor_rune = 0;
        let bytes: Vec<usize> = ja
            .base
            .char_indices()
            .map(|(byte, _)| byte)
            .chain([ja.base.len()])
            .collect();
        for byte in bytes {
            ja.from_lane_cursor(LaneCursor::Byte(byte));
            assert_eq!(
                ja.to_lane_cursor(),
                LaneCursor::Byte(byte),
                "日文 byte {byte} 換算後要回到同一個位移"
            );
            assert!(ja.base.is_char_boundary(ja.cursor_byte()));
        }

        // 混打：rune ↔ rune。
        let mut mixed = session();
        mixed.zh_lane = false;
        mixed.base = "a我b".to_string();
        for rune in 0..=mixed.base.chars().count() {
            mixed.from_lane_cursor(LaneCursor::Rune(rune));
            assert_eq!(mixed.to_lane_cursor(), LaneCursor::Rune(rune));
            assert!(mixed.base.is_char_boundary(mixed.cursor_byte()));
        }
    }

    /// 重建不能把中間的游標推到整段結尾。
    #[test]
    fn test_cursor_stays_on_rune_boundary_after_rebuild() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_syllable(&mut s, "ㄒㄧㄤˇ");
        type_syllable(&mut s, "ㄔ");
        assert!(s.move_cursor_to(1), "游標移到第二個音節");
        assert_eq!(s.snapshot().cursor_rune, 1);
        assert_eq!(s.active_segment(), Some(1));
        s.rebuild();
        let snap = s.snapshot();
        assert_eq!(snap.cursor_rune, 1, "重建後游標留在原本的 rune 邊界");
        assert_eq!(s.active_segment(), Some(1), "片段切換後仍在同一段");
        assert!(snap.preedit.is_char_boundary(
            snap.preedit
                .char_indices()
                .nth(snap.cursor_rune)
                .map(|(byte, _)| byte)
                .unwrap_or(snap.preedit.len())
        ));
    }

    /// 對照 `TestSessionSegmentsFollowZhuyinPathAndCursor`。
    #[test]
    fn test_session_segments_follow_zhuyin_path_and_cursor() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_syllable(&mut s, "ㄒㄧㄤˇ");
        type_syllable(&mut s, "ㄔ");
        let segments = s.segments();
        assert_eq!(segments.len(), 3);
        assert_eq!(s.cursor(), 3);
        assert_eq!(s.active_segment(), Some(2));
        for (index, (segment, want)) in segments.iter().zip(["我", "想", "吃"]).enumerate() {
            assert_eq!(&s.base[segment.start..segment.end], want);
            assert_eq!(segment.mode, Lang::Zh);
            assert_eq!(segment.script, Script::Bopomofo);
            assert_eq!(s.segment_at(index), Some(index));
        }
        assert_eq!(s.segment_at(3), None, "片段結尾屬於下一個片段");
    }

    /// 對照 `TestSessionSegmentsKeepLongWordMerged`。
    #[test]
    fn test_session_segments_keep_long_word_merged() {
        let mut s = session();
        type_syllable(&mut s, "ㄈㄥ");
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        let segments = s.segments();
        assert_eq!(segments.len(), 1);
        assert_eq!(&s.base[segments[0].start..segments[0].end], "風景");
        assert_eq!(segments[0].len(), "風景".len());
    }

    /// 對照 `TestSessionSegmentsReturnCopiesAndEditLanguages` 的形狀。
    #[test]
    fn test_session_segments_return_copies_and_edit() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_syllable(&mut s, "ㄒㄧㄤˇ");

        let mut copy = s.segments();
        copy[0].script = Script::Kana;
        assert_eq!(
            s.segments()[0].script,
            Script::Bopomofo,
            "副本不影響 session"
        );

        assert!(s.replace_segment(0, Segment::new(0, 3, Lang::Ja, Script::Kana)));
        assert_eq!(s.segments()[0].mode, Lang::Ja);
        assert!(s.insert_segment(2, Segment::new(6, 9, Lang::Zh, Script::Han)));
        assert_eq!(s.segments().len(), 3);
        assert_eq!(s.segments()[1].mode, Lang::Zh, "插入不影響其他片段");
        assert!(s.remove_segment(2));
        assert_eq!(s.segments().len(), 2);
        assert!(s.remove_segment(0));
        assert_eq!(s.segments()[0].mode, Lang::Zh);

        assert!(!s.remove_segment(9));
        assert!(!s.replace_segment(9, Segment::new(0, 1, Lang::Zh, Script::Han)));
        assert!(!s.insert_segment(9, Segment::new(0, 1, Lang::Zh, Script::Han)));
        assert!(!s.replace_segment(0, Segment::new(1, 1, Lang::Zh, Script::Han)));
    }

    /// 手動片段只存活到下一次按鍵。
    ///
    /// `x` 是注音的 ㄌ；新按鍵會留下未完成注音，片段因此變成兩段。這裡要
    /// 確認的是手動標記失效，片段由目前的組字狀態重新推導，而不是片段數不變。
    #[test]
    fn test_manual_segments_expire_on_next_key() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        assert!(s.replace_segment(0, Segment::new(0, 3, Lang::Ja, Script::Kana)));
        assert_eq!(s.segments()[0].mode, Lang::Ja);
        s.press(Key::char('x', false));
        let preedit = s.preedit();
        let segments = s.segments();
        assert_eq!(segments.len(), 2, "未完成注音自成一段");
        assert_eq!(&preedit[segments[0].start..segments[0].end], "我");
        assert_eq!(&preedit[segments[1].start..segments[1].end], "ㄌ");
        assert!(
            segments.iter().all(|segment| segment.mode == Lang::Zh),
            "下一次按鍵重新推導"
        );
    }

    /// 對照 `TestSessionSegmentSelectionLocksOnlyThatSegment` 的狀態行為。
    #[test]
    fn test_session_segment_locks_only_that_segment() {
        let mut s = session();
        type_syllable(&mut s, "ㄈㄥ");
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        assert_eq!(s.segments().len(), 1);
        assert!(s.lock_segment(0));
        assert!(s.segment_locked(0));
        type_syllable(&mut s, "ㄨㄛˇ");
        assert_eq!(s.snapshot().preedit, "風景我");
        assert_eq!(s.segments().len(), 2);
        assert!(s.segment_locked(0));
        assert!(!s.segment_locked(1));
        assert!(s.unlock_segment(0));
        assert!(!s.segment_locked(0));
        assert!(!s.lock_segment(9));
        assert!(!s.unlock_segment(9));
    }

    /// 對照 `TestLoneToneKeyPlusSpaceTypesToneMark`。
    #[test]
    fn test_lone_tone_key_plus_space_types_tone_mark() {
        for (key, want) in [('3', 'ˇ'), ('4', 'ˋ'), ('6', 'ˊ'), ('7', '˙')] {
            let mut s = session();
            let snap = s.press(Key::char(key, false));
            assert_eq!(snap.preedit, key.to_string());
            assert!(snap.candidates.is_empty());
            let snap = s.press(Key::new(KeyCode::Space, None, false));
            assert_eq!(snap.preedit, want.to_string());
            assert_eq!(snap.cursor_utf16, 1);
            assert!(snap.candidates.is_empty());
            s.press(Key::new(KeyCode::Enter, None, false));
            assert_eq!(s.commit(), want.to_string());
        }

        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_keys(&mut s, "3 ");
        assert_eq!(s.snapshot().preedit, "我ˇ");
        type_keys(&mut s, "4 ");
        assert_eq!(s.snapshot().preedit, "我ˇˋ");
        type_syllable(&mut s, "ㄍㄜˋ");
        assert_eq!(s.snapshot().preedit, "我ˇˋ個");
    }

    /// 對照 `TestToneKeyStaysDigitUnlessSpaceFollows` 與
    /// `TestToneKeyRunStaysNumber`。
    #[test]
    fn test_tone_key_stays_digit_unless_space_follows() {
        let mut s = session();
        type_keys(&mut s, "3");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "3");

        let mut s = session();
        type_keys(&mut s, "3");
        type_syllable(&mut s, "ㄍㄜˋ");
        assert_eq!(s.snapshot().preedit, "3個");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "3個");

        let mut s = session();
        type_keys(&mut s, "3");
        s.press(Key::new(KeyCode::Backspace, None, false));
        assert_eq!(s.snapshot().preedit, "");

        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_keys(&mut s, "3");
        s.press(Key::char('\\', false));
        let snap = s.press(Key::new(KeyCode::Space, None, false));
        assert!(
            !snap.preedit.contains('ˇ'),
            "中間隔了別的操作，空白不換聲調符號：{}",
            snap.preedit
        );

        for run in ["34", "67", "3467", "33"] {
            let mut s = session();
            type_keys(&mut s, run);
            let snap = s.press(Key::new(KeyCode::Space, None, false));
            assert_eq!(snap.preedit, format!("{run} "));
            s.press(Key::new(KeyCode::Enter, None, false));
            assert_eq!(s.commit(), format!("{run} "));
        }
    }

    #[test]
    fn test_tone_mark_keys() {
        assert!(!tone_mark_keys(true, true));
        assert!(tone_mark_keys(true, false));
        assert!(tone_mark_keys(false, true));
        assert!(tone_mark_keys(false, false));
    }

    /// 對照 `untonedSeparator`：組字中按 ' 只切音節。
    #[test]
    fn test_toneless_separator_splits_syllables() {
        let mut s = toneless_session();
        type_keys(&mut s, "vu");
        s.press(Key::char('\'', false));
        type_keys(&mut s, "0");
        s.press(Key::new(KeyCode::Space, None, false));
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "西安");

        let mut s = toneless_session();
        type_keys(&mut s, "vu0");
        s.press(Key::new(KeyCode::Space, None, false));
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "仙", "沒有分隔鍵時 ㄒㄧ＋ㄢ 是一個音節");
    }

    /// 對照 `untonedBreak`：接不上的注音先完成目前的音節。
    #[test]
    fn test_toneless_break_starts_new_syllable() {
        let mut s = toneless_session();
        type_keys(&mut s, "su");
        type_keys(&mut s, "cl");
        assert_eq!(s.snapshot().preedit, "你ㄏㄠ");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "你好");
    }

    /// 對照 `phoneticWindow` 的明確候選：尾端差一個符號的完整詞。
    #[test]
    fn test_phonetic_candidates_find_one_symbol_apart_word() {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
        config.max_candidates = 10;
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄇㄚˊ");
        type_syllable(&mut s, "ㄆㄛˊ");
        type_syllable(&mut s, "ㄉㄡˋ");
        type_syllable(&mut s, "ㄅㄨˇ");
        let surfaces: Vec<&str> = s
            .candidates()
            .iter()
            .map(|candidate| candidate.surface.as_str())
            .collect();
        assert!(
            surfaces.contains(&"麻婆豆腐"),
            "差一個聲母的完整詞應列為候選：{surfaces:?}"
        );
    }

    #[test]
    fn test_one_symbol_apart() {
        let syl = Syllable::from_raw;
        assert!(one_symbol_apart(&syl("ㄓ"), &syl("ㄗ")));
        assert!(one_symbol_apart(&syl("ㄓㄨ"), &syl("ㄗㄨ")));
        assert!(one_symbol_apart(&syl("ㄗ"), &syl("ㄗㄨ")));
        assert!(!one_symbol_apart(&syl("ㄓ"), &syl("ㄓ")));
        assert!(!one_symbol_apart(&syl("ㄓ"), &syl("ㄗˇ")));
        assert!(!one_symbol_apart(&syl("ㄓ"), &syl("ㄗㄨㄚ")));
    }
    fn error_hint_session() -> Session {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
        config.zhuyin_error_hint = true;
        Session::new(config)
    }

    /// 上鍵在候選窗還沒開時先開窗並進入選取模式；PageDown 已交給宿主。
    #[test]
    fn test_up_opens_candidates_but_page_down_does_not() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        let snap = s.press(Key::new(KeyCode::PageDown, None, false));
        assert!(!snap.navigating, "PageDown 不開候選窗也不進選取模式");

        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        let snap = s.press(Key::new(KeyCode::Up, None, false));
        assert!(!snap.candidates.is_empty(), "上鍵開候選窗");
        assert!(snap.navigating);
        assert_eq!(snap.highlighted, 0, "剛開窗不移動反白");
    }

    /// 對照 `moveShownCandidateCursor`：只是顯示候選時 Home／End 移動組字游標。
    #[test]
    fn test_home_end_move_composition_cursor_when_shown() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_syllable(&mut s, "ㄒㄧㄤˇ");
        type_syllable(&mut s, "ㄔ");
        let shown = s.snapshot();
        assert!(!shown.navigating, "候選只是顯示");
        assert_eq!(shown.preedit, "我想吃");
        assert_eq!(shown.cursor_rune, 3, "游標本來在結尾");

        let home = s.press(Key::new(KeyCode::Home, None, false));
        assert_eq!(home.cursor_rune, 0, "Home 移到組字開頭");
        assert_eq!(home.cursor_utf16, 0);
        assert_eq!(home.preedit, "我想吃", "移動游標不改組字");
        assert!(!home.navigating, "移動游標不進入選取模式");
        assert_eq!(home.highlighted, 0);

        let end = s.press(Key::new(KeyCode::End, None, false));
        assert_eq!(end.cursor_rune, 3, "End 回到結尾");
        assert_eq!(end.cursor_utf16, 3);
        assert_eq!(end.preedit, "我想吃");
    }

    /// 只是顯示候選時左右鍵移動組字游標，不換候選、不進入選取模式。
    #[test]
    fn test_arrows_move_composition_cursor_when_shown() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_syllable(&mut s, "ㄒㄧㄤˇ");
        type_syllable(&mut s, "ㄔ");
        let shown = s.snapshot();
        assert!(!shown.navigating);
        assert_eq!(shown.cursor_rune, 3);

        let left = s.press(Key::new(KeyCode::ArrowLeft, None, false));
        assert_eq!(left.cursor_rune, 2, "← 移動組字游標");
        assert!(!left.navigating, "只是顯示時左右鍵不進入選取模式");
        assert!(!left.candidates.is_empty(), "候選窗保留");
        assert_eq!(left.highlighted, 0, "換游標不換反白");

        let back = s.press(Key::new(KeyCode::ArrowLeft, None, false));
        assert_eq!(back.cursor_rune, 1);
        let forward = s.press(Key::new(KeyCode::ArrowRight, None, false));
        assert_eq!(forward.cursor_rune, 2);
        assert_eq!(forward.preedit, "我想吃");
    }

    /// 多音節詞的游標也要落在顯示字串的正確 rune 邊界。
    #[test]
    fn test_home_end_cursor_inside_multi_syllable_word() {
        let mut s = session();
        type_syllable(&mut s, "ㄈㄥ");
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        assert_eq!(s.snapshot().preedit, "風景");
        let home = s.press(Key::new(KeyCode::Home, None, false));
        assert_eq!(home.cursor_rune, 0);
        let end = s.press(Key::new(KeyCode::End, None, false));
        assert_eq!(end.cursor_rune, 2, "兩個字的詞結尾是 rune 2");
        assert_eq!(end.cursor_utf16, 2);
    }

    /// 打錯注音時聲調被擋下：注音留在暫存、不完成音節。
    #[test]
    fn test_tone_blocked_keeps_pending_when_reading_has_no_word() {
        let mut s = error_hint_session();
        let snap = type_syllable_snapshot(&mut s, "ㄘㄨㄛˋ");
        assert_eq!(snap.preedit, "ㄘㄨㄛ", "聲調被擋下，注音留在暫存");
        assert_eq!(snap.cursor_rune, 3);
        assert!(s.tone_blocked(), "ZhInput 記下擋聲調");
        assert!(snap.candidates.is_empty());
    }

    /// 擋下後下一個按鍵清掉旗標，暫存注音照常可退格。
    #[test]
    fn test_tone_blocked_flag_clears_on_next_key() {
        let mut s = error_hint_session();
        type_syllable(&mut s, "ㄘㄨㄛˋ");
        assert!(s.tone_blocked());
        let snap = s.press(Key::new(KeyCode::Backspace, None, false));
        assert!(!s.tone_blocked(), "下一個按鍵清掉旗標");
        assert_eq!(snap.preedit, "ㄘㄨ", "退格從暫存尾端退一格");
    }

    /// 讀音有字時聲調照常收下。
    #[test]
    fn test_tone_allowed_when_reading_has_word() {
        let mut s = error_hint_session();
        let snap = type_syllable_snapshot(&mut s, "ㄨㄛˇ");
        assert!(!s.tone_blocked());
        assert_eq!(snap.preedit, "我");
    }

    /// 單一注音符號的讀音不擋，照舊讓注音文留下。
    #[test]
    fn test_single_symbol_reading_is_not_blocked() {
        let mut s = error_hint_session();
        let snap = type_syllable_snapshot(&mut s, "ㄟˋ");
        assert!(!s.tone_blocked());
        assert_eq!(snap.preedit, "ㄟˋ");
    }

    /// 模糊音近音有字時不擋。
    #[test]
    fn test_fuzzy_near_reading_is_not_blocked() {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
        config.zhuyin_error_hint = true;
        config.fuzzy_pairs = vec![('ㄙ', 'ㄕ')];
        let mut s = Session::new(config);
        let snap = type_syllable_snapshot(&mut s, "ㄙㄨ");
        assert!(!s.tone_blocked(), "近音 ㄕㄨ 有字，不擋");
        assert_eq!(snap.preedit, "書");

        let mut strict = SessionConfig::new(Mode::Mixed);
        strict.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
        strict.zhuyin_error_hint = true;
        strict.fuzzy_pairs = Vec::new();
        let mut s = Session::new(strict);
        let snap = type_syllable_snapshot(&mut s, "ㄙㄨ");
        assert!(s.tone_blocked(), "沒有模糊音時同一個讀音被擋");
        assert_eq!(snap.preedit, "ㄙㄨ");
    }

    /// 同一處連按同一個聲調第二次視為故意打注音文，放行一次。
    #[test]
    fn test_tone_retry_allows_deliberate_bopomofo() {
        let mut s = error_hint_session();
        for ch in "hji".chars() {
            s.press(Key::char(ch, false));
        }
        assert_eq!(s.snapshot().preedit, "ㄘㄨㄛ", "hji 是 ㄘㄨㄛ");
        let blocked = s.press(Key::char('4', false));
        assert_eq!(blocked.preedit, "ㄘㄨㄛ");
        assert!(s.tone_blocked());
        let retried = s.press(Key::char('4', false));
        assert!(!s.tone_blocked(), "重試放行");
        assert_eq!(retried.preedit, "ㄘㄨㄛˋ");
    }

    /// 沒開設定時聲調照舊收下，行為與原本相同。
    #[test]
    fn test_tone_gate_is_off_by_default() {
        let mut s = session();
        let snap = type_syllable_snapshot(&mut s, "ㄘㄨㄛˋ");
        assert!(!s.tone_blocked());
        assert_eq!(snap.preedit, "ㄘㄨㄛˋ");
    }

    /// 聲調選填不套用擋聲調規則。
    #[test]
    fn test_tone_gate_is_off_when_toneless() {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
        config.zhuyin_error_hint = true;
        config.toneless = true;
        let mut s = Session::new(config);
        let snap = type_syllable_snapshot(&mut s, "ㄘㄨㄛˋ");
        assert!(!s.tone_blocked());
        assert_eq!(snap.preedit, "ㄘㄨㄛˋ");
    }

    fn type_syllable_snapshot(session: &mut Session, reading: &str) -> Snapshot {
        let keyboard = crate::zh::keyboard::Keyboard::standard();
        let mut snap = session.snapshot();
        for syllable in crate::zh::syllable::parse_reading(reading) {
            let keys = keyboard.keys(&syllable).expect("音節應該有對應按鍵");
            for ch in keys.chars() {
                snap = session.press(Key::char(ch, false));
            }
        }
        snap
    }
}
