//! [`Session`] 的主狀態機：按鍵處理、候選重建與送出。
//!
//! 型別、設定與公開 API 在上層模組；這裡只放實作細節。

use super::*;
use crate::ja::decode::mean_candidates;
use crate::score::BigramLm;

/// 「常用」候選標示的系統詞頻門檻，對照上游 `commonLabelProbability`。
pub(super) const COMMON_LABEL_PROBABILITY: f64 = 1e-4;

/// 「關聯」候選標示所需的鄰詞機率提升倍數，對照上游 `relatedLabelLift`。
pub(super) const RELATED_LABEL_LIFT: f64 = 2.0;

/// 目前 Unix 秒數；時鐘早於 epoch 時回傳 0。
pub(super) fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// 目前 Unix 毫秒數；時鐘早於 epoch 時回傳 0。
///
/// rollback 窗口以毫秒計，秒數的精度不夠。
pub(super) fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

impl Session {
    /// 把送出的文字接到左文尾巴，長度上限對齊前身 `MaxLeftContextUnits`。
    pub(super) fn remember_committed(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.committed.push_str(text);
        let len = self.committed.chars().count();
        if len > MAX_LEFT_CONTEXT_UNITS {
            let skip = len - MAX_LEFT_CONTEXT_UNITS;
            self.committed = self.committed.chars().skip(skip).collect();
        }
    }

    pub(super) fn max_candidates(&self) -> usize {
        self.config.max_candidates.max(1)
    }

    /// 下一次輸入決定上一次提交的學習要留下還是補償。
    ///
    /// Backspace、Esc 與 Ctrl+Z 視為撤銷：rollback 窗口內把學習還原成提交前的
    /// 樣子；其他按鍵或窗口到期則保留學習。對照 Mozkey 的 rollback window。
    pub(super) fn resolve_pending_learning(&mut self, key: &Key) {
        if !self.personal.has_pending_learning() {
            return;
        }
        let undo = matches!(key.code, KeyCode::Backspace | KeyCode::Escape)
            || (key.ctrl && key.code == KeyCode::Char('z'));
        if undo {
            self.personal.rollback_learning(now_millis());
            return;
        }
        self.personal.accept_learning();
    }

    /// Ctrl 組合鍵：目前只處理啟用後的 `Ctrl+,`。
    pub(super) fn press_ctrl(&mut self, key: Key) {
        if !self.config.ctrl_punct {
            return;
        }
        if key.code == KeyCode::Char(',') {
            self.push_literal("，");
        }
    }

    /// 把字面文字插進組字結果；記錄插入位置。
    pub(super) fn push_literal(&mut self, text: &str) {
        if self.literal.is_empty() {
            self.literal_anchor = self.syllables.len();
            self.literal_tail_absorbed = false;
            // 混打／日文沒有音節格可以推導插入點，字面接在目前組字尾端。
            // 固定當下的長度，後續追加的按鍵才不會把字面擠到最前面。
            if !self.in_zh_lane() {
                self.literal_offset = self.base.len();
            }
        }
        self.literal.push_str(text);
    }

    /// Shift＋字母要當成字面，不讓前面的注音被重播成原始按鍵。
    ///
    /// 外殼可能送大寫鍵碼，也可能送小寫鍵碼配 `shift = true`（0c 協定把
    /// `Shift+A` 編成 `a` 加 Shift）；兩種都轉成同一個大寫字面。標點與
    /// 符號不走這裡，Shift＋符號仍由注音／前導表決定。沒有 Shift 的大寫
    /// 輸入仍交給混打規劃，不能一律當字面。
    pub(super) fn shift_literal_text(key: &Key) -> Option<String> {
        if !key.shift || key.ctrl || key.alt {
            return None;
        }
        let KeyCode::Char(code) = key.code else {
            return None;
        };
        code.is_ascii_alphabetic()
            .then(|| code.to_ascii_uppercase().to_string())
    }

    /// 把 Shift＋字母附加成字面，並保持 preedit 長度與游標一致。
    ///
    /// 候選窗開著時先關窗：字面接在整段最後，游標也移到最後。候選窗是選字
    /// 介面，Shift＋字母不是選字鍵，關窗後再把字面接上。
    pub(super) fn append_shift_literal(&mut self, text: &str) -> Snapshot {
        let window = self.candidate_window || self.menu.is_some();
        if window {
            self.menu = None;
            self.candidate_window = false;
            self.candidate_expanded = false;
            self.navigating = false;
            self.keep_candidates = false;
            self.edit = None;
            self.highlighted = 0;
            self.mixed_target = None;
            self.mixed_choices.clear();
            self.rebuild();
        }
        let _ = self.literal(text);
        // 字面固定接在整段最後，游標跟著到最後。
        if self.in_ja_lane() {
            self.ja_cursor = self.base_with_highlight().len();
        } else if self.in_zh_lane() {
            self.cursor = self.syllables.len();
        } else {
            self.mixed_cursor = Self::MIXED_CURSOR_END;
        }
        self.snapshot()
    }

    /// 把混打 lane 中已固定在尾端的字面併回按鍵流，讓後續輸入接在它之後。
    ///
    /// 一個 `literal` 只能記一個插入點；第二段字面若發生在後續按鍵之後，
    /// 先併回按鍵流才會得到 `abcXdYe`，不會兩段都黏在第一個插入點。
    fn flush_literal_to_ascii(&mut self) {
        if !self.literal_tail_absorbed
            || self.literal.is_empty()
            || self.in_zh_lane()
            || self.in_ja_lane()
        {
            return;
        }
        if self.literal_offset != self.base.len() {
            return;
        }
        let start = self.ascii.typed().len();
        let text = std::mem::take(&mut self.literal);
        self.ascii.seed_literal(&text);
        let end = self.ascii.typed().len();
        if start < end {
            self.literal_spans.push((start, end));
        }
        self.literal_anchor = 0;
        self.literal_offset = 0;
        self.literal_tail_absorbed = false;
    }

    /// 把英文／Shift 附加的字面尾巴併回一般組字，讓下一個按鍵接在後面。
    ///
    /// 注音路徑先把未完成的音節收成音節，字面才會落在它之後；所有權仍由
    /// [`Session::composition`] 的既有規則決定，不會因此變成 `User`。
    fn absorb_literal_tail(&mut self) {
        if self.literal_tail.is_empty() {
            return;
        }
        let tail = std::mem::take(&mut self.literal_tail);
        if self.in_zh_lane() {
            // 未完成的音節先收成音節，字面才會落在它之後。
            self.complete_pending();
        }
        self.push_literal(&tail);
        // 只有英文／Shift 字面尾巴可以提前併回按鍵流。
        self.literal_tail_absorbed = true;
    }

    /// 在音節索引 `at` 插入一個完成的音節，游標移到它後面。
    ///
    /// 鎖定、字面插入點與手動片段一起依插入位置平移；被插入點切到的鎖定直接
    /// 解除，因為它原本覆蓋的讀音已經被拆開。對照上游 `shiftLocks(at, +1)`。
    pub(super) fn insert_syllable(&mut self, at: usize, syllable: Syllable) {
        let at = at.min(self.syllables.len());
        self.syllables.insert(at, syllable);
        self.shift_locks(at, 1);
        self.cursor = at + 1;
        self.edit = None;
        self.keep_candidates = false;
        self.zh_window_last = false;
    }

    /// 刪除音節索引 `at` 的音節；游標不動，呼叫端自己決定位置。
    ///
    /// 對照上游 `eraseBeforeCursor` 與 `shiftLocks(at, -1)`。
    pub(super) fn remove_syllable(&mut self, at: usize) -> bool {
        if at >= self.syllables.len() {
            return false;
        }
        self.syllables.remove(at);
        self.shift_locks(at, -1);
        self.cursor = self.cursor.min(self.syllables.len());
        self.edit = None;
        self.keep_candidates = false;
        self.zh_window_last = false;
        true
    }

    /// 在 `at` 插入（`delta = +1`）或刪除（`delta = -1`）音節後平移鎖定。
    ///
    /// 完全在編輯點之前的鎖定不動；完全在之後的平移；被切到的解除。字面的
    /// 插入點也用同一條規則。對照上游 `shiftLocks`。
    pub(super) fn shift_locks(&mut self, at: usize, delta: isize) {
        // 所有權由 `Composition` 統一平移；注音的 `Locks` 只是推導結果。
        self.shift_zh_ownership(at, delta);
        self.sync_zh_locks();
        // 字面字串的插入點是音節索引；編輯點在它之前才平移。
        if at < self.literal_anchor {
            self.literal_anchor = self.literal_anchor.saturating_add_signed(delta);
        }
        // 待定聲調記的是字面內的位元組位置，編輯後不再可靠。
        self.pending_tone = None;
    }

    /// 把尚未完成的注音收成一個音節，插到游標位置。
    ///
    /// `ZhInput` 只留暫存注音；收成音節後立刻清空它的內部清單，避免兩份狀態
    /// 各記一次。回傳是否真的完成了一個音節。
    pub(super) fn complete_pending(&mut self) -> bool {
        if self.zh.pending().is_empty() {
            return false;
        }
        match self.zh.press(Key::new(KeyCode::Space, None, false)) {
            crate::zh::input::ZhInputEvent::SyllableComplete(syllable) => {
                self.zh_pending_keys.clear();
                self.insert_syllable(self.cursor, syllable);
                self.zh.reset();
                true
            }
            _ => false,
        }
    }

    /// 游標格所在的片段（音節起點與長度）。
    ///
    /// 以目前最佳路徑為準；游標在整段結尾時算最後一個片段。沒有路徑時退回
    /// 單一音節。對照上游 `unitStart`／`unitEnd`。
    pub(super) fn cell_segment(&self, cell: usize) -> Option<(usize, usize)> {
        if self.syllables.is_empty() {
            return None;
        }
        if let Some(path) = self.last_hypotheses.first() {
            for segment in &path.segments {
                if segment.start <= cell && cell < segment.end {
                    return Some((segment.start, segment.end - segment.start));
                }
            }
        }
        let at = cell.min(self.syllables.len().saturating_sub(1));
        Some((at, 1))
    }

    /// 接受目前的補全建議；沒有建議時回傳 `false`。
    pub(super) fn accept_suggestion(&mut self) -> bool {
        let Some(text) = self.suggestion.take() else {
            return false;
        };
        self.accepted_completion = Some(text);
        self.candidate_window = false;
        self.navigating = false;
        true
    }

    pub(super) fn press_char(&mut self, ch: char) {
        // 字面尾巴要先併回組字，下一個按鍵才會接在它後面。
        if !self.literal_tail.is_empty() && !self.candidate_window {
            self.absorb_literal_tail();
        }
        // 已固定在尾端的字面先併回按鍵流，第二段字面才有自己的插入點。
        self.flush_literal_to_ascii();
        // 外殼可能把空白送成字元而不是 `KeyCode::Space`；兩條路徑要走同一套
        // 空白處理（注音 lane 的字面空白、混打的字面空格）。注音還沒完成時
        // 空白仍是一聲鍵，照原路徑交給注音狀態機。
        if ch == ' ' && !(self.in_zh_lane() && !self.zh_pending_keys.is_empty()) {
            self.space();
            return;
        }
        // 注音 lane 按過空白：現在才把已完成音節交回混打管線。立刻交回會讓
        // 注音候選清單消失，所以延到還有輸入時處理。空白本身要留著，不能
        // 被當成音節分隔（`ji3 hello` 曾變成 `我he lぉ`）。
        if self.zh_lane && self.zh_space_closed {
            self.zh_space_closed = false;
            if let Some(span) = self.flush_zh_lane_for_mixed() {
                self.literal_spans.push(span);
                self.rebuild();
            }
        }
        // 選取模式中 j／k 換前後一個候選；其他時候 j／k 仍是注音鍵。
        if self.in_zh_lane() && self.candidate_window && self.navigating && self.edit.is_some() {
            match ch {
                'j' => {
                    self.move_highlight(-1);
                    return;
                }
                'k' => {
                    self.move_highlight(1);
                    return;
                }
                _ => {}
            }
        }
        // Mixed 模式下聲調鍵先於選字鍵：未完成的注音前綴遇到數字時先補成音節，
        // 即使外殼在單一數字鍵前補了 Down，也不會選到英文補全候選。
        if self.config.mode == Mode::Mixed
            && !self.in_zh_lane()
            && self.candidate_window
            && self.ascii.tone_completes(ch, &self.lexicon)
        {
            self.ascii.tone(ch, &self.lexicon);
            self.accepted_completion = None;
            self.suggestion = None;
            self.candidate_window = false;
            self.navigating = false;
            self.keep_candidates = false;
            self.highlighted = 0;
            self.mixed_cursor = Self::MIXED_CURSOR_END;
            self.rebuild();
            return;
        }
        // 選字鍵先處理：超出候選數時吃掉按鍵但完全不改狀態，不能退回注音鍵。
        if self.candidate_window {
            if let Some(selected) = self.select_by_digit(ch) {
                if selected {
                    self.candidate_window = false;
                }
                return;
            }
        }
        self.zh_window_last = false;
        self.accepted_completion = None;
        self.suggestion = None;
        self.navigating = false;
        let window = self.candidate_window;
        if self.menu.is_some() {
            let kind = self.menu.map(|menu| menu.kind);
            if ch == '=' && kind == Some(MenuKind::Emoji) {
                self.menu = None;
                self.candidate_window = false;
                self.highlighted = 0;
                self.push_literal("=");
                return;
            }
            if ch == '`' && kind == Some(MenuKind::Symbols) {
                self.menu = None;
                self.candidate_window = false;
                self.highlighted = 0;
                return;
            }
            self.menu = None;
            self.highlighted = 0;
        }
        self.candidate_window = false;
        if !window {
            self.highlighted = 0;
        }
        // 符號與表情選單改由外殼或公開 API 開啟；一般標點與等號照字面。
        // 混打推斷出來的日文片段：羅馬字接不上的按鍵（數字、標點）結束推斷，
        // 把原始羅馬字按鍵交回混打重算，這個鍵也一起送。
        if self.in_ja_lane() {
            let event = self.romaji.press(Key::char(ch, false));
            if event == crate::ja::input::RomajiEvent::Rejected {
                self.leave_ja_run_for_mixed(ch);
            } else {
                // 留下原始按鍵，退回混打時用同一串按鍵重算切分。
                self.ja_run_keys.push(ch);
            }
            return;
        }
        {
            {
                self.mixed_cursor = Self::MIXED_CURSOR_END;
                // 純注音組字走注音狀態機，與舊的純注音模式一致；按鍵打不成
                // 注音時整段交回混打管線。
                if self.zh_lane {
                    if self.mixed_zh_lane_continues(ch) {
                        self.press_zh_char(ch);
                    } else {
                        self.leave_zh_lane_for_mixed(ch);
                    }
                    return;
                }
                if self.mixed_zh_lane_starts(ch) {
                    self.zh_lane = true;
                    self.zh_pending_keys.clear();
                    self.press_zh_char(ch);
                    return;
                }
                // 直接送來的漢字照字面留下，不當成英數按鍵推斷。
                if is_han(ch) {
                    self.push_literal(&ch.to_string());
                    return;
                }
                if !self.ascii.tone(ch, &self.lexicon) {
                    self.ascii.press(ch);
                }
                self.try_japanese_run();
            }
        }
    }

    /// 把一個按鍵交給注音狀態機；純注音與 Mixed 的注音路徑共用。
    fn press_zh_char(&mut self, ch: char) {
        match self.zh.press(Key::char(ch, false)) {
            crate::zh::input::ZhInputEvent::Consumed => self.zh_pending_keys.push(ch),
            crate::zh::input::ZhInputEvent::SyllableComplete(syllable) => {
                // 音節完成後搬進工作階段自己的音節清單，才能在中間插字。
                self.zh_pending_keys.clear();
                self.insert_syllable(self.cursor, syllable);
                self.zh.reset();
            }
            crate::zh::input::ZhInputEvent::Commit(reading) => {
                self.zh_pending_keys.clear();
                // 空白鍵在沒有未完成注音時提交；改用轉換後的結果。
                if let Some(text) = self.convert_reading(&reading) {
                    self.pending_commit = Some(text);
                    self.zh.reset();
                }
            }
            crate::zh::input::ZhInputEvent::Cancel => {
                self.zh_pending_keys.clear();
                self.zh.reset();
            }
            crate::zh::input::ZhInputEvent::Ignored => {
                // 注音路徑接不住的標點照字面留下，不再轉全形。
                self.push_literal(&ch.to_string());
            }
        }
    }

    /// Mixed 從空白開始打注音鍵時，這次組字走注音路徑。
    fn mixed_zh_lane_starts(&self, ch: char) -> bool {
        if self.in_ja_lane() || !self.ascii.is_empty() || !self.syllables.is_empty() {
            return false;
        }
        // 孤立的聲調鍵已在 `segments_press` 當成數字處理，不會走到這裡。
        // 直接送來的注音符號（非鍵盤字母）也走注音路徑。
        if crate::zh::syllable::class_of(ch) != crate::zh::syllable::Class::None {
            return crate::zh::syllable::class_of(ch) != crate::zh::syllable::Class::Tone;
        }
        // 鍵盤上的標點（`, . / ; -`）雖然也是注音鍵，單獨按下時是字面
        // 標點，不自動開始一個注音音節。
        self.zh.keyboard().symbol(ch).is_some_and(|symbol| {
            crate::zh::syllable::class_of(symbol) != crate::zh::syllable::Class::Tone
                && (ch.is_ascii_alphabetic() || ch.is_ascii_digit())
        })
    }

    /// 注音路徑中的下一個按鍵是否仍打得成注音。
    ///
    /// 標點、非注音鍵與環境鍵留在注音路徑當字面；字母或數字讓目前未完成的
    /// 音節出現同類別重複（`hel` 的兩個聲母）或剛好是英文詞時，代表使用者
    /// 在打英文，整段交回混打管線。
    fn mixed_zh_lane_continues(&self, ch: char) -> bool {
        if self.in_ja_lane() {
            return false;
        }
        if crate::zh::syllable::class_of(ch) != crate::zh::syllable::Class::None {
            return true;
        }
        let Some(symbol) = self.zh.keyboard().symbol(ch) else {
            // 不是注音鍵的標點一律照字面，交回混打管線。
            return false;
        };
        if crate::zh::syllable::class_of(symbol) == crate::zh::syllable::Class::Tone {
            return true;
        }
        // 標點鍵（`, . / ; -`）只有在延續目前未完成的音節時才是韻母。
        if ch.is_ascii_punctuation() {
            return !self.zh.pending().is_empty();
        }
        let mut keys = self.zh_pending_keys.clone();
        keys.push(ch);
        if crate::mixed::prototype().has(&keys) {
            return false;
        }
        if super::mixed::compose_keys(&keys).is_none() {
            return false;
        }
        // 已經有完成的字、而且正在開新音節時，`,`、`.`、`/` 視為標點，
        // 整段交回混打管線，畫面與 Router 一致（`字.` 而不是 `字ㄡ`）。
        self.syllables.is_empty()
            || !self.zh_pending_keys.is_empty()
            || !matches!(ch, ',' | '.' | '/')
    }

    /// 結束這次的注音路徑，把整段按鍵重播進混打管線。
    pub(super) fn leave_zh_lane_for_mixed(&mut self, ch: char) {
        // 已完成的音節還原成原本的按鍵，才能讓混打管線重新判斷前後片段；
        // 直接塞入漢字會讓前面的注音片段失去讀音與候選。
        let keyboard = self.zh.keyboard();
        let mut keys = String::new();
        for syllable in &self.syllables {
            if let Ok(syllable_keys) = keyboard.keys(syllable) {
                keys.push_str(&syllable_keys);
            }
        }
        keys.push_str(&self.zh_pending_keys);
        keys.push(ch);
        keys.push_str(&self.literal);
        keys.push_str(&self.literal_tail);
        self.zh.reset();
        self.zh_lane = false;
        self.zh_space_closed = false;
        self.zh_pending_keys.clear();
        self.syllables.clear();
        self.cursor = 0;
        self.locks.clear();
        self.composition.clear();
        self.edit = None;
        self.keep_candidates = false;
        self.highlighted = 0;
        self.candidate_window = false;
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
        self.mixed_cursor = Self::MIXED_CURSOR_END;
        self.rebuild();
    }

    /// 把未完成的注音凍結成原始按鍵字面，再交回混打管線。
    ///
    /// 游標離開待定音節時用。原始按鍵記在 `literal_spans`，混打規劃不會再
    /// 把它讀成日文或英文；已完成的音節照常重播。回傳凍結前已完成音節的
    /// rune 數，讓呼叫端把游標放到待定尾巴的左緣。
    pub(super) fn freeze_pending_zhuyin_for_mixed(&mut self) -> usize {
        let keyboard = self.zh.keyboard();
        let mut keys = String::new();
        for syllable in &self.syllables {
            if let Ok(syllable_keys) = keyboard.keys(syllable) {
                keys.push_str(&syllable_keys);
            }
        }
        let completed_runes: usize = self
            .syllables
            .iter()
            .map(|syllable| self.decode_syllable(syllable).chars().count())
            .sum();
        let pending_start = keys.len();
        let pending = std::mem::take(&mut self.zh_pending_keys);
        keys.push_str(&pending);
        keys.push_str(&self.literal);
        keys.push_str(&self.literal_tail);
        self.zh.reset();
        self.zh_lane = false;
        self.zh_space_closed = false;
        self.syllables.clear();
        self.cursor = 0;
        self.locks.clear();
        self.composition.clear();
        self.edit = None;
        self.keep_candidates = false;
        self.highlighted = 0;
        self.candidate_window = false;
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
        if !pending.is_empty() {
            let end = pending_start + pending.len();
            if end <= self.ascii.typed().len() {
                self.literal_spans.push((pending_start, end));
            }
        }
        self.mixed_cursor = Self::MIXED_CURSOR_END;
        self.rebuild();
        completed_runes
    }

    /// 把純注音組字整段交回混打管線，不附加額外按鍵。
    ///
    /// 回傳字面段在重播按鍵流裡的位元組範圍；呼叫端可把它記進
    /// `literal_spans`，避免重建時被混打規劃當成片段之間的停頓吃掉。
    pub(super) fn flush_zh_lane_for_mixed(&mut self) -> Option<(usize, usize)> {
        let keyboard = self.zh.keyboard();
        let mut keys = String::new();
        for syllable in &self.syllables {
            if let Ok(syllable_keys) = keyboard.keys(syllable) {
                keys.push_str(&syllable_keys);
            }
        }
        keys.push_str(&self.zh_pending_keys);
        let literal_start = keys.len();
        keys.push_str(&self.literal);
        let literal_end = keys.len();
        keys.push_str(&self.literal_tail);
        self.zh.reset();
        self.zh_lane = false;
        self.zh_space_closed = false;
        self.zh_pending_keys.clear();
        self.syllables.clear();
        self.cursor = 0;
        self.locks.clear();
        self.composition.clear();
        self.edit = None;
        self.keep_candidates = false;
        self.highlighted = 0;
        self.candidate_window = false;
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
        self.mixed_cursor = Self::MIXED_CURSOR_END;
        self.rebuild();
        (literal_start < literal_end && literal_end <= self.ascii.typed().len())
            .then_some((literal_start, literal_end))
    }

    /// 候選視窗開啟時，選字鍵選取目前頁的第幾項。
    ///
    /// 回傳 `None` 代表這個字元不是選字鍵，交回一般按鍵路徑；`Some(true)`
    /// 代表已選取；`Some(false)` 代表是選字鍵但超出候選數，按鍵已被吃掉，
    /// 狀態不變。
    pub(super) fn select_by_digit(&mut self, ch: char) -> Option<bool> {
        if self.candidates.is_empty()
            || !self.candidate_window
            || (self.in_zh_lane() && !self.zh.pending().is_empty() && self.menu.is_none())
        {
            return None;
        }
        // 混合模式只有按過 ↓／↑ 進入選取模式才用數字選字；空白開窗後
        // 直接打數字仍照一般按鍵走，避免吃掉下一個注音或英數。選單開著時
        // 仍然照選單的選字鍵處理。
        if self.config.mode == Mode::Mixed && !self.navigating && self.menu.is_none() {
            return None;
        }
        let row = self.selection_index(ch)?;
        // 數字鍵是「目前直排的列號」：先找高亮所在的頁與欄，再換算成
        // 該欄第 `row` 列的絕對索引。四欄時 1–0 只作用在目前這一欄。
        let layout = self.candidate_layout();
        let page_start = self.current_page() * layout.page_size();
        let column = self.highlighted.saturating_sub(page_start) / layout.rows();
        let index = page_start + column * layout.rows() + row;
        match self.choose_candidate(index) {
            Some(_) => Some(true),
            None => Some(false),
        }
    }

    /// 依絕對索引套用候選，回傳套用後的快照；越界或無法套用時回 `None`。
    ///
    /// 只做三件事：找出這個候選的 [`CandidateAction`]、交給對應 lane 套用、
    /// 關窗收尾。數字鍵選字與候選窗滑鼠點選共用這條路徑。
    pub fn choose_candidate(&mut self, index: usize) -> Option<Snapshot> {
        let action = self.candidate_action(index)?;
        if !self.apply_candidate_action(&action) {
            return None;
        }
        Some(self.finish_choice())
    }

    /// 找出候選的套用動作；所有 lane 的差異都收在這一份資料裡。
    ///
    /// 優先序與原本的 `choose_candidate` 相同：選單、逐字元候選窗、日文
    /// 片段、注音片段／`zh_window_last`、混打游標候選，最後才是直接插入。
    fn candidate_action(&self, index: usize) -> Option<CandidateAction> {
        if index >= self.candidates.len() {
            return None;
        }
        let insert = self.candidates[index].surface.clone();
        if self.menu.is_some() {
            return Some(CandidateAction {
                lane: ActionLane::Menu,
                index,
                insert,
                commit_on_apply: true,
                ..CandidateAction::default()
            });
        }
        if self.candidate_window {
            // 逐字元候選窗有自己的套用動作（拆回原始輸入、換假名等）。
            if let Some(Some(mut action)) = self.mixed_choices.get(index).cloned() {
                action.lane = ActionLane::Mixed;
                action.index = index;
                action.learn = action.syllable_edit.is_some();
                if let Some(target) = &self.mixed_target {
                    action.typed_span = Some(target.typed);
                    action.rune_span = Some((target.rune_start, target.rune_len));
                }
                return Some(action);
            }
            // 日文片段候選換掉游標所在文節；`rune_len > 1` 是整段組字的候選。
            if self.in_ja_lane() {
                let whole = self
                    .mixed_target
                    .as_ref()
                    .is_some_and(|target| target.ja && target.rune_len > 1);
                return Some(CandidateAction {
                    lane: if whole {
                        ActionLane::JaWhole
                    } else {
                        ActionLane::JaClause
                    },
                    index,
                    insert,
                    rune_span: self
                        .mixed_target
                        .as_ref()
                        .map(|target| (target.rune_start, target.rune_len)),
                    learn: true,
                    ..CandidateAction::default()
                });
            }
        }
        // 游標移到中間或調過詞長時，選字只鎖定這一段，不送出整段組字。
        if let Some(edit) = self.edit {
            return Some(self.zh_syllable_action(index, edit));
        }
        // ↓／空白開的純注音候選只涵蓋最後一個音節：選字換那一段，整段留著。
        if self.config.mode == Mode::Mixed && self.in_zh_lane() && self.zh_window_last {
            let start = self.syllables.len().saturating_sub(1);
            return Some(self.zh_syllable_action(index, EditWindow { start, len: 1 }));
        }
        // 混合模式的候選是英文補完、拼字建議或已完成的注音詞；純注音路徑
        // （含 Mixed 的 zh_lane）與選單走上面的動作。
        if self.config.mode == Mode::Mixed && !self.in_zh_lane() && !self.in_ja_lane() {
            return Some(CandidateAction {
                lane: ActionLane::MixedGeneral,
                index,
                insert,
                cursor: ChoiceCursor::AfterInsert,
                learn: true,
                ..CandidateAction::default()
            });
        }
        Some(CandidateAction {
            lane: ActionLane::Insert,
            index,
            insert,
            cursor: ChoiceCursor::AfterInsert,
            learn: true,
            ..CandidateAction::default()
        })
    }

    /// 注音音節替代動作；`edit` 是 `syllables` 裡要換掉的片段。
    fn zh_syllable_action(&self, index: usize, edit: EditWindow) -> CandidateAction {
        let start = edit.start.min(self.syllables.len());
        let end = start.saturating_add(edit.len).min(self.syllables.len());
        let rune_start: usize = self.syllables[..start]
            .iter()
            .map(|syllable| self.decode_syllable(syllable).chars().count())
            .sum();
        let rune_len: usize = self.syllables[start..end]
            .iter()
            .map(|syllable| self.decode_syllable(syllable).chars().count())
            .sum();
        CandidateAction {
            lane: ActionLane::ZhSyllable,
            index,
            rune_span: Some((rune_start, rune_len)),
            insert: self.candidates[index].surface.clone(),
            syllable_edit: Some(edit),
            learn: true,
            ..CandidateAction::default()
        }
    }

    /// 執行一個候選動作；回傳是否真的套用成功。
    ///
    /// 學習由 action 的 `learn` 控制，成功套用後才執行，且每次套用只執行一次。
    fn apply_candidate_action(&mut self, action: &CandidateAction) -> bool {
        // 選單候選代表立即插入；`commit_on_apply` 讓「一次送出」變成資料，
        // 不在每個呼叫端各自判斷選單。
        if action.commit_on_apply {
            return self.apply_menu_action(action);
        }
        match action.lane {
            ActionLane::Menu => self.apply_menu_action(action),
            ActionLane::Mixed => self.apply_mixed_action(action),
            ActionLane::ZhSyllable => self.apply_zh_syllable_action(action),
            ActionLane::JaClause | ActionLane::JaWhole => self.apply_ja_action(action),
            ActionLane::MixedGeneral => self.apply_mixed_general_action(action),
            ActionLane::Insert => self.apply_insert_action(action),
        }
    }

    /// 選單候選：插入字面並一次送出。
    fn apply_menu_action(&mut self, action: &CandidateAction) -> bool {
        self.remember_text(&action.insert);
        let text = self.compose_literal(&action.insert);
        self.pending_commit = Some(self.finalize(text));
        self.clear_composition();
        true
    }

    /// 注音片段或 `zh_window_last`：只換那幾個音節，整段組字留著。
    fn apply_zh_syllable_action(&mut self, action: &CandidateAction) -> bool {
        let Some(edit) = action.syllable_edit else {
            return false;
        };
        let candidate = self.candidates.get(action.index).cloned();
        // `zh_window_last` 只屬於「↓／空白只開最後一格」的視窗；一般片段
        // 編輯不動這個旗標。
        if self.edit.is_none() && self.zh_window_last {
            self.zh_window_last = false;
        }
        self.edit = Some(edit);
        let applied = self.select_window_candidate(action.index);
        self.edit = None;
        if !applied {
            return false;
        }
        self.cursor = (edit.start + 1).min(self.syllables.len());
        if action.learn {
            if let Some(candidate) = candidate {
                self.learn_window_candidate(edit, &candidate);
            }
        }
        true
    }

    /// 日文片段或整段候選；學習用套用前的候選快照，避免重建後找不到這一項。
    fn apply_ja_action(&mut self, action: &CandidateAction) -> bool {
        let candidate = self.candidates.get(action.index).cloned();
        // 明確手選的證據量取決於套用前的候選窗狀態；套用會先關窗。
        let manual = self.candidate_window;
        let applied = match action.lane {
            ActionLane::JaWhole => self.apply_ja_whole_candidate(action.index),
            _ => self.apply_ja_candidate(action.index),
        };
        if applied && action.learn {
            if let Some(candidate) = candidate {
                self.learn_ja_choice(action.index, &candidate, manual);
            }
        }
        applied
    }

    /// 混打游標候選：先後嘗試原始按鍵、英文補完與注音片段，最後才直接插入。
    fn apply_mixed_general_action(&mut self, action: &CandidateAction) -> bool {
        let Some(candidate) = self.candidates.get(action.index).cloned() else {
            return false;
        };
        let raw = self.ascii.pending().to_string();
        if !raw.is_empty() && candidate.surface == raw {
            // 選回原始按鍵：固定成字面，不再讓切分器把它解成羅馬字。
            self.ascii.force_literal();
            if action.learn {
                self.learn_english_choice(&raw, &candidate.surface);
            }
            self.close_mixed_choice_window();
            return true;
        }
        if !raw.is_empty() && self.ascii.accept(&candidate.surface) {
            if action.learn {
                self.learn_english_choice(&raw, &candidate.surface);
            }
            self.close_mixed_choice_window();
            return true;
        }
        if self.select_mixed_zh_candidate(&candidate.surface) {
            self.candidate_window = false;
            self.navigating = false;
            self.highlighted = 0;
            return true;
        }
        self.apply_insert_action(action)
    }

    fn close_mixed_choice_window(&mut self) {
        self.candidate_window = false;
        self.navigating = false;
        self.highlighted = 0;
        self.mixed_cursor = Self::MIXED_CURSOR_END;
    }

    /// 一般插入：字面接到組字結果後送出整段。
    fn apply_insert_action(&mut self, action: &CandidateAction) -> bool {
        self.remember_text(&action.insert);
        if action.learn {
            self.learn_candidate(action.index);
        }
        let text = self.insert_literal(&action.insert);
        self.pending_commit = Some(self.finalize(text));
        self.clear_composition();
        true
    }

    /// 選字完成後與 `press` 相同的收尾：關掉候選窗並重建顯示。
    pub(super) fn finish_choice(&mut self) -> Snapshot {
        self.candidate_window = false;
        self.candidate_expanded = false;
        self.mixed_target = None;
        self.mixed_choices.clear();
        self.rebuild();
        self.snapshot()
    }

    /// 把候選套用到游標所在的片段，其他片段保持原樣。
    ///
    /// 以片段鎖定讓解碼器固定這一段；`keepCandidates` 讓候選窗留著，直到
    /// Esc 或游標離開這個片段。回傳是否真的換了字。
    pub(super) fn select_window_candidate(&mut self, index: usize) -> bool {
        let Some(edit) = self.edit else {
            return false;
        };
        let Some(candidate) = self.candidates.get(index).cloned() else {
            return false;
        };
        if edit.len == 0 || edit.start.saturating_add(edit.len) > self.syllables.len() {
            return false;
        }
        self.mark_zh_user_syllables(edit.start, edit.len, &candidate.surface);
        self.segment_manual = None;
        self.rebuild();
        self.highlighted = 0;
        self.candidate_window = true;
        self.navigating = true;
        self.keep_candidates = true;
        true
    }

    /// 片段候選成功套用後的學習；與「套用」分開，呼叫端只在成功後執行一次。
    pub(super) fn learn_window_candidate(&mut self, edit: EditWindow, candidate: &Candidate) {
        let readings = self.zh_readings(&self.syllables[edit.start..edit.start + edit.len]);
        let reading = readings.join(" ");
        if self.config.learning
            && !self.private
            && !crate::learn_guard::is_sensitive(&candidate.surface)
            && candidate
                .surface
                .chars()
                .next()
                .is_some_and(crate::zh::han::is_han)
        {
            self.personal.begin_rollback(now_millis());
            self.personal
                .note_manual(&candidate.surface, &reading, edit.len);
        } else if self.config.learning && crate::learn_guard::is_sensitive(&candidate.surface) {
            log::debug!("學習守門：片段候選像敏感資料，跳過寫入");
        }
    }

    pub(super) fn space(&mut self) {
        // 注音路徑（純注音或 Mixed 的純注音組字）：未完成的注音先補完；
        // 已完成音節時空白就是字面空格，不再開候選窗或選第一個。
        if self.in_zh_lane() {
            if !self.zh.pending().is_empty() {
                self.complete_pending();
                return;
            }
            self.candidate_window = false;
            self.candidate_expanded = false;
            self.navigating = false;
            self.keep_candidates = false;
            self.zh_window_last = false;
            // 空白是字面空格：留在注音組字區，不開候選、不選字。
            if self.syllables.is_empty() {
                self.ascii.space(&self.lexicon);
            } else {
                self.zh_run_breaks.push(self.syllables.len());
                self.push_literal(" ");
                // 空白是字面字元，同時也是注音 lane 的邊界；等真的有下一個
                // 字元時才把整段交回混打管線（見 `press_char`）。
                self.zh_space_closed = true;
            }
            return;
        }
        if self.in_ja_lane() {
            // 空白只插入字面空格；日文候選改由 ↓ 開啟。
            self.candidate_window = false;
            self.candidate_expanded = false;
            self.navigating = false;
            self.literal_tail.push(' ');
            return;
        }
        // 其餘混打路徑：空白只插入空格。
        self.candidate_window = false;
        self.candidate_expanded = false;
        self.navigating = false;
        self.keep_candidates = false;
        self.mixed_cursor = Self::MIXED_CURSOR_END;
        // 待定按鍵是空的：這個空白是純字面，記進 `literal_spans`，重建時
        // 才不會被 `visible_plans` 當成片段停頓吃掉（`ji3 ji3 ji3`）。
        // 待定按鍵非空時空白可能被當成一聲／音節分隔，照原管線處理。
        let literal_space = self.ascii.pending().is_empty();
        let start = self.ascii.typed().len();
        self.ascii.space(&self.lexicon);
        if literal_space {
            let end = self.ascii.typed().len();
            if start < end {
                self.literal_spans.push((start, end));
            }
        }
    }

    /// Mixed 的純注音組字開候選窗時只列游標所在音節。
    ///
    /// 一般顯示與建議仍用整段解碼（聲調選填、詞組建議要靠前後文）；按下
    /// ↓／空白要選字時才縮到游標那一格，與 Mixed 既有的選字行為一致。
    /// Enter 不選字，仍送出整段，所以不能直接把 `edit` 設起來。
    fn narrow_mixed_zh_candidates(&mut self) {
        if self.config.mode != Mode::Mixed || self.syllables.is_empty() {
            return;
        }
        self.zh_window_last = true;
    }

    /// ↓ 在日文片段上開候選：游標不動，列出整段組字。
    pub(super) fn open_ja_cursor_candidates(&mut self) -> bool {
        let text = self.base.clone();
        if text.is_empty() {
            return false;
        }
        let at = rune_offset(&text, self.ja_cursor.min(text.len()));
        // 日文的 ↓ 只開候選，不移動游標：延遲整句模式列出整段組字的轉換
        // 結果，逐字変換模式仍只列游標所在片段。
        let whole = self
            .live
            .as_ref()
            .is_some_and(LiveController::sentence_mode);
        let opened = if self.live.is_some() {
            if whole {
                self.open_ja_all_candidates()
            } else {
                self.open_ja_candidates()
            }
        } else {
            !self.candidates.is_empty()
        };
        if !opened {
            return false;
        }
        // 原始羅馬字也是一個可選回來的候選；`ja_only` 模式同樣保留按鍵。
        let raw = if !self.ja_run_keys.is_empty() {
            self.ja_run_keys.clone()
        } else {
            self.romaji
                .units()
                .iter()
                .map(Unit::as_str)
                .collect::<String>()
        };
        // 固定順序：目前值、原始羅馬字、其餘轉換。原始羅馬字原本排在最後，
        // 轉成カナ（ス）之後很難選回來；現在永遠是第二項。
        if !raw.is_empty() && self.candidates.first().map(|c| c.surface.as_str()) != Some(&raw) {
            if let Some(at) = self
                .candidates
                .iter()
                .position(|candidate| candidate.surface == raw)
            {
                let candidate = self.candidates.remove(at);
                let at = at.min(1);
                self.candidates.insert(at, candidate);
            } else {
                let at = self.candidates.len().min(1);
                self.candidates.insert(
                    at,
                    Candidate::new(raw.clone(), raw.clone(), i32::MAX, CandidateSource::Literal),
                );
            }
        }
        self.highlighted = 0;
        self.candidate_window = true;
        self.candidate_expanded = false;
        self.navigating = true;
        self.keep_candidates = true;
        self.mixed_target = Some(MixedTarget {
            typed: (0, 0),
            raw,
            raw_toneless: String::new(),
            tone_key: None,
            rune_start: at,
            rune_len: if whole {
                text.chars().count().max(1)
            } else {
                1
            },
            syllable: None,
            reading: None,
            ja: true,
        });
        true
    }

    /// 開啟整段日文組字的候選；日文的 ↓ 用，游標不動。
    ///
    /// [`Session::open_ja_candidates`] 只列游標所在片段；停頓轉換後使用者按 ↓
    /// 想看到整句的替代寫法，所以這裡用整段讀音查一次候選，再把目前詞面排到
    /// 最前面。
    fn open_ja_all_candidates(&mut self) -> bool {
        let Some(live) = self.live.as_ref() else {
            return false;
        };
        let units = self.romaji.units().len();
        if units == 0 {
            return false;
        }
        let Ok(candidates) = live.candidates(crate::ja::decode::JaSpan::new(0, units)) else {
            return false;
        };
        if candidates.is_empty() {
            return false;
        }
        let mut list: Vec<Candidate> = candidates
            .into_iter()
            .map(|candidate| {
                Candidate::new(
                    candidate.surface,
                    candidate.reading,
                    candidate.cost,
                    CandidateSource::Lexicon,
                )
            })
            .collect();
        if let Some(at) = list
            .iter()
            .position(|candidate| candidate.surface == self.base)
        {
            let current = list.remove(at);
            list.insert(0, current);
        }
        self.candidates = list;
        self.highlighted = 0;
        self.candidate_window = true;
        self.navigating = false;
        self.keep_candidates = true;
        true
    }

    /// 開啟日文逐字変換片段的候選。
    ///
    /// 只針對游標所在片段，其他片段保持鎖定。清單由
    /// [`crate::ja::live::Controller::candidates`] 產生，包含字典候選、
    /// 目前詞面與原讀音的平假名／片假名版本。
    pub(super) fn open_ja_candidates(&mut self) -> bool {
        let Some(index) = self.active_segment() else {
            return false;
        };
        let Some(live) = self.live.as_ref() else {
            return false;
        };
        let spans = live.clause_spans();
        let Some(span) = spans.get(index) else {
            return false;
        };
        let Ok(candidates) = live.candidates(crate::ja::decode::JaSpan::new(
            span.start,
            span.end - span.start,
        )) else {
            return false;
        };
        if candidates.is_empty() {
            return false;
        }
        let mut list: Vec<Candidate> = candidates
            .into_iter()
            .map(|candidate| {
                Candidate::new(
                    candidate.surface,
                    candidate.reading,
                    candidate.cost,
                    CandidateSource::Lexicon,
                )
            })
            .collect();
        if let Some(at) = list
            .iter()
            .position(|candidate| candidate.surface == span.surface)
        {
            let current = list.remove(at);
            list.insert(0, current);
        }
        self.candidates = list;
        self.highlighted = 0;
        self.candidate_window = true;
        self.navigating = false;
        self.keep_candidates = true;
        true
    }

    /// 把候選套到游標所在片段，其他片段不變。
    pub(super) fn apply_ja_candidate(&mut self, index: usize) -> bool {
        // ↓ 開的是整段組字的候選時，選字換掉整段，不是只換游標所在文節。
        if self
            .mixed_target
            .as_ref()
            .is_some_and(|target| target.ja && target.rune_len > 1)
        {
            return self.apply_ja_whole_candidate(index);
        }
        let Some(segment) = self.active_segment() else {
            return false;
        };
        let Some(candidate) = self.candidates.get(index).cloned() else {
            return false;
        };
        let segment_start = self
            .segments()
            .get(segment)
            .map(|segment| segment.start)
            .unwrap_or(0);
        let Some(live) = self.live.as_mut() else {
            return false;
        };
        if !live.replace_clause(segment, candidate.surface.clone()) {
            return false;
        }
        // 非延遲模式沒有 user_segments 路徑，要用鎖定前綴讓下一次重建
        // 保留剛選的片段；延遲模式由 `Composition` 的 User 標記處理。
        if !live.sentence_mode() {
            live.lock_through(segment);
        }
        // 學習由 [`CandidateAction::learn`] 在套用成功後執行；這裡只負責套用，
        // 讓「換文節」與「記一次學習」不會因為重畫順序而漏掉或重複。
        self.capture_ja_users();
        self.refresh_ja_live();
        if let Some(segment) = self.segments().get(segment) {
            // 比一個 rune 長的替換（私→わたし）游標放在第一個字元之後。
            let first = candidate
                .surface
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(0);
            self.ja_cursor = if candidate.surface.chars().count() > 1 {
                (segment_start + first).min(segment.end)
            } else {
                segment.end.min(self.base.len())
            };
        }
        self.candidate_window = false;
        self.navigating = false;
        self.keep_candidates = false;
        self.candidate_expanded = false;
        self.mixed_target = None;
        self.mixed_choices.clear();
        true
    }

    /// 整段組字的候選：鎖定整段讀音後重新轉換，換掉整個詞面。
    ///
    /// 只有延遲整句模式會走到這裡（`↓` 用整段候選時）；逐字変換仍走片段路徑。
    fn apply_ja_whole_candidate(&mut self, index: usize) -> bool {
        let Some(candidate) = self.candidates.get(index).cloned() else {
            return false;
        };
        let units = self.romaji.units().to_vec();
        if units.is_empty() {
            return false;
        }
        let reading: String = units.iter().map(Unit::as_str).collect();
        let Some(live) = self.live.as_mut() else {
            return false;
        };
        live.set_user_segments(vec![crate::ja::live::UserSegment {
            start: 0,
            units: units.len(),
            reading: reading.clone(),
            surface: candidate.surface.clone(),
        }]);
        let plan = live.convert_now();
        self.capture_ja_users();
        self.apply_live_plan(&plan, &units, &reading);
        self.candidate_window = false;
        self.navigating = false;
        self.keep_candidates = false;
        self.candidate_expanded = false;
        self.candidates.clear();
        self.highlighted = 0;
        self.mixed_target = None;
        self.mixed_choices.clear();
        self.ja_cursor = self.base.len();
        true
    }

    /// 依控制器目前的結果重畫日文組字字串與片段，不動游標以外的狀態。
    pub(super) fn refresh_ja_live(&mut self) {
        let Some(result) = self.live.as_ref().and_then(|live| live.draft().cloned()) else {
            return;
        };
        // 片段變換後舊候選已不對應目前詞面；清掉才不會用舊候選蓋住組字區。
        self.candidates.clear();
        self.highlighted = 0;
        let units = self.romaji.units().to_vec();
        let mut base = String::new();
        let mut segments = Vec::new();
        for clause in &result.clauses {
            let start = base.len();
            base.push_str(&clause.surface);
            segments.push(Segment::new(start, base.len(), Lang::Ja, Script::Kana));
        }
        let covered: usize = result
            .clauses
            .iter()
            .map(|clause| clause.reading.chars().count())
            .sum();
        if covered < units.len() {
            let start = base.len();
            base.extend(units[covered..].iter().map(Unit::as_str));
            segments.push(Segment::new(start, base.len(), Lang::Ja, Script::Kana));
        }
        self.base = base;
        self.segments = segments;
        self.ja_cursor = self.ja_cursor.min(self.base.len());
        self.sync_composition_ja();
    }

    /// 日文組字游標往左或往右移一個字元。
    fn move_ja_cursor(&mut self, delta: isize) -> bool {
        let text = self.base.clone();
        if text.is_empty() {
            return false;
        }
        let mut at = self.ja_cursor.min(text.len());
        if delta < 0 {
            at = text[..at]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
        } else if delta > 0 && at < text.len() {
            at += text[at..].chars().next().map(char::len_utf8).unwrap_or(0);
        }
        if at == self.ja_cursor {
            return false;
        }
        self.ja_cursor = at;
        true
    }

    pub(super) fn backspace(&mut self) {
        self.candidate_window = false;
        self.navigating = false;
        self.keep_candidates = false;
        self.edit = None;
        self.highlighted = 0;
        // 英文／Shift 字面尾巴以 rune 為單位刪除，不刪整個片段。
        if !self.literal_tail.is_empty() {
            self.literal_tail.pop();
            return;
        }
        if self.in_ja_lane() {
            self.romaji.backspace();
            self.sync_ja_run_keys();
            self.leave_empty_japanese_run();
            return;
        }
        if self.zh_lane {
            self.backspace_zh();
        } else if !self.backspace_mixed() && !self.literal.is_empty() {
            // 孤立聲調鍵先留在字面；沒有混打按鍵可退時退回那一個字面。
            self.literal.pop();
        }
    }

    /// 注音路徑的 Backspace：先退未完成的注音，再退已完成音節與字面。
    fn backspace_zh(&mut self) {
        if !self.zh.pending().is_empty() {
            // 還沒完成的注音先退一格；退完才動已完成的音節。
            self.zh.press(Key::new(KeyCode::Backspace, None, false));
            self.zh_pending_keys.pop();
        } else if self.cursor > 0 {
            self.remove_syllable(self.cursor - 1);
            self.cursor = self.cursor.saturating_sub(1);
        } else if !self.literal.is_empty() {
            self.literal.pop();
        }
    }

    /// Esc 的優先權：先關候選窗，再清未完成的注音，最後才清整段組字。
    ///
    /// 對照上游 `key()` 的 Escape 分支；標示範圍與同音預覽在本埠沒有對應狀態。
    pub(super) fn escape(&mut self) {
        // 英文模式的 Esc 丟掉整段，包含前面的中文緩衝。
        if !self.literal_tail.is_empty() {
            self.clear_composition();
            return;
        }
        if self.candidate_window && self.menu.is_none() {
            self.candidate_window = false;
            self.navigating = false;
            self.keep_candidates = false;
            self.edit = None;
            self.highlighted = 0;
            return;
        }
        if self.in_zh_lane() && !self.zh.pending().is_empty() {
            self.clear_pending_zhuyin();
            return;
        }
        self.clear_composition();
    }

    /// 只清掉尚未完成的注音，保留已完成的音節。
    ///
    /// `ZhInput` 沒有「清暫存」的公開方法，這裡用退格逐格退掉暫存注音；
    /// 暫存清空就停手，不會動到已完成的音節。
    fn clear_pending_zhuyin(&mut self) {
        let mut guard = 0;
        while !self.zh.pending().is_empty() && guard < 16 {
            let before = self.zh.pending();
            self.zh.press(Key::new(KeyCode::Backspace, None, false));
            if self.zh.pending() == before {
                break;
            }
            guard += 1;
        }
        self.pending_tone = None;
        self.zh_pending_keys.clear();
        self.candidate_window = false;
        self.navigating = false;
    }

    /// `Delete`：刪除游標後一個音節；沒有可刪的音節時只關候選窗。
    pub(super) fn delete_forward(&mut self) {
        self.candidate_window = false;
        self.candidate_expanded = false;
        self.navigating = false;
        self.keep_candidates = false;
        self.edit = None;
        self.highlighted = 0;
        if self.config.mode == Mode::Mixed && !self.in_zh_lane() && !self.in_ja_lane() {
            let text = self.base_with_highlight();
            let runes = text.chars().count();
            let at = self.mixed_cursor.min(runes);
            if at < runes {
                let saved = self.mixed_cursor;
                self.mixed_cursor = at;
                let target = self.mixed_target_at_cursor();
                self.mixed_cursor = saved;
                if let Some(target) = target {
                    let typed = self.ascii.typed().to_string();
                    let (start, end) = target.typed;
                    if start < end && end <= typed.len() && typed.is_char_boundary(start) {
                        let mut next =
                            String::with_capacity(typed.len().saturating_sub(end - start));
                        next.push_str(&typed[..start]);
                        next.push_str(&typed[end..]);
                        self.ascii.replay(&next, &self.lexicon);
                        self.literal_spans.clear();
                        let len = self.base_with_highlight().chars().count();
                        self.mixed_cursor = at.min(len);
                        return;
                    }
                }
                let index = self.mixed_segment_index_at_rune(at);
                if let Some(index) = index {
                    self.remove_mixed_segment(index);
                }
            }
            return;
        }
        if self.in_zh_lane() && self.zh.pending().is_empty() && self.remove_syllable(self.cursor) {
            return;
        }
        if self.candidate_window {
            self.candidate_window = false;
            self.navigating = false;
            self.highlighted = 0;
        }
    }

    /// 下鍵：第一次開候選窗並進入選取模式，之後往下換一項。
    pub(super) fn down(&mut self) {
        // 外殼送來 Down 只是為了避開數字選字，後面的聲調鍵仍要完成原本的音節；
        // 這裡不補完未完成的注音。
        if self.in_ja_lane() {
            if !self.candidate_window {
                self.open_ja_cursor_candidates();
                return;
            }
            self.navigating = true;
            self.move_row(1);
            return;
        }
        if self.config.mode == Mode::Mixed && !self.in_zh_lane() {
            if !self.candidate_window {
                self.open_mixed_cursor_candidates();
                return;
            }
            self.navigating = true;
            self.move_row(1);
            return;
        }
        if self.in_zh_lane() {
            // 還沒打完的注音：↓ 不開窗也不把音節退成字母。外殼會在單一
            // 數字鍵前補 ↓，後面的聲調鍵仍要能完成原本的音節。
            if !self.zh_pending_keys.is_empty() || !self.zh.pending().is_empty() {
                return;
            }
            if !self.candidate_window {
                self.open_zh_cursor_candidates();
                return;
            }
        }
        self.narrow_mixed_zh_candidates();
        if self.candidates.is_empty() {
            return;
        }
        if !self.candidate_window {
            self.candidate_window = true;
            self.navigating = true;
            self.candidate_expanded = false;
            return;
        }
        if !self.navigating {
            self.navigating = true;
            return;
        }
        self.move_row(1);
    }

    /// 上鍵：候選窗開著時往上換一項並進入選取模式；還沒開窗時先開窗。
    pub(super) fn up(&mut self) {
        if self.candidates.is_empty() {
            return;
        }
        if !self.candidate_window {
            self.candidate_window = true;
            self.navigating = true;
            self.candidate_expanded = false;
            return;
        }
        self.navigating = true;
        self.move_row(-1);
    }

    /// `PageDown`／`PageUp`：換頁，頁碼頭尾相接；還沒開窗時先開窗。
    pub(super) fn page_step(&mut self, delta: isize) {
        if self.candidates.is_empty() {
            return;
        }
        if !self.candidate_window {
            self.candidate_window = true;
            self.navigating = true;
            return;
        }
        let size = self.page_size().max(1);
        let pages = self.candidates.len().div_ceil(size).max(1) as isize;
        let page = (self.highlighted / size) as isize;
        let page = (page + delta).rem_euclid(pages);
        self.highlighted = page as usize * size;
        self.navigating = true;
    }

    /// 移動組字游標到指定音節格；回傳是否真的移動。
    ///
    /// 音節打到一半（暫存注音非空）時不動游標，對照上游 `moveTo`。游標移到
    /// 中間時，候選清單改成游標所在片段的候選；回到結尾就恢復整段候選。
    pub(super) fn move_cursor_to(&mut self, pos: usize) -> bool {
        if !self.zh.pending().is_empty() {
            return false;
        }
        let pos = pos.min(self.syllables.len());
        let before = self.cursor;
        self.cursor = pos;
        if pos == before {
            return false;
        }
        self.zh_window_last = false;
        self.keep_candidates = false;
        self.navigating = false;
        self.highlighted = 0;
        self.edit = if pos < self.syllables.len() {
            self.cell_segment(pos)
                .map(|(start, len)| EditWindow { start, len })
        } else {
            None
        };
        // 移動游標不順便開候選窗；候選窗只由 ↓／↑ 開啟。游標移到某個
        // 片段時先記下可調整的片段範圍，供 Shift＋←／→ 與片段選字使用。
        if !self.candidate_window {
            self.edit = None;
        }
        true
    }

    /// 調整游標所在片段的讀音長度，重新解碼。
    ///
    /// `delta = +1` 加長（Shift＋→），`delta = -1` 縮短（Shift＋←）。長度界在
    /// 一個音節到剩餘音節數之間；沒有片段時不動作。
    pub(super) fn resize_segment(&mut self, delta: isize) {
        if !self.in_zh_lane() || !self.zh.pending().is_empty() || self.syllables.is_empty() {
            return;
        }
        let Some((anchor, len)) = self
            .edit
            .map(|edit| (edit.start, edit.len))
            .or_else(|| self.cell_segment(self.cursor))
        else {
            return;
        };
        let max = self.syllables.len().saturating_sub(anchor).max(1);
        let target = (len as isize + delta).clamp(1, max as isize) as usize;
        if target == len && self.edit.is_some() {
            return;
        }
        self.edit = Some(EditWindow {
            start: anchor,
            len: target,
        });
        if self.cursor < anchor || self.cursor >= anchor + target {
            self.cursor = anchor;
        }
        self.candidate_window = true;
        self.navigating = false;
        self.keep_candidates = false;
        self.highlighted = 0;
    }

    /// `Home`／`End`：選取模式中移到候選清單的第一項或最後一項。
    ///
    /// 候選窗沒開或只是顯示時，這兩個鍵移動組字游標到開頭／結尾，對照上游
    /// `moveShownCandidateCursor`；游標不動候選內容，也不會進入選取模式。
    pub(super) fn candidate_edge(&mut self, end: bool) {
        if !self.navigating {
            if self.in_ja_lane() {
                self.ja_cursor = if end {
                    self.base_with_highlight().len()
                } else {
                    0
                };
                return;
            }
            if self.config.mode == Mode::Mixed && !self.in_zh_lane() {
                let runes = self.base_with_highlight().chars().count();
                self.move_mixed_cursor_to(if end { runes } else { 0 });
                return;
            }
            if self.in_zh_lane() {
                let target = if end { self.syllables.len() } else { 0 };
                self.move_cursor_to(target);
            }
            return;
        }
        if self.candidates.is_empty() {
            return;
        }
        self.highlighted = if end { self.candidates.len() - 1 } else { 0 };
        self.navigating = true;
    }

    /// 快照的補全尾綴：注音用路徑補全，混打用英文前綴補全。
    pub(super) fn snapshot_suggestion(&self) -> Option<String> {
        if !self.literal_tail.is_empty() {
            return None;
        }
        if self.menu.is_some() || self.candidate_window || self.accepted_completion.is_some() {
            return None;
        }
        if self.config.mode == Mode::Mixed
            && !self.in_zh_lane()
            && !self.in_ja_lane()
            && self.config.english_autocomplete
        {
            if let Some(text) = self.ascii.suggestion() {
                let rest = text.strip_prefix(self.ascii.pending()).unwrap_or(&text);
                return (!rest.is_empty()).then(|| rest.to_string());
            }
        }
        let text = self.suggestion.as_ref()?;
        let rest = text.strip_prefix(self.base.as_str()).unwrap_or(text);
        (!rest.is_empty()).then(|| rest.to_string())
    }

    /// 高亮候選的顯示原因；只標個人詞彙、關聯與常用詞，不影響排序。
    ///
    /// 對照上游 `candidateLabels`：個人標示看使用者詞彙與使用證據，關聯標示
    /// 看已送出左文最後一個詞的 bigram 提升，常用標示看系統 unigram 機率。
    /// 每列只給一個標示，優先序是個人、關聯、常用。
    pub(super) fn highlighted_label(&self) -> Option<&'static str> {
        if !self.in_zh_lane() || self.menu.is_some() {
            return None;
        }
        let candidate = self.candidates.get(self.highlighted)?;
        if candidate.source == CandidateSource::Literal {
            return None;
        }
        if self.personal.is_vocab(&candidate.surface) {
            return Some("personal");
        }
        if let Some(reading) = candidate.reading.split_whitespace().next() {
            if self.personal.is_picked(reading, &candidate.surface) {
                return Some("personal");
            }
        }
        let lm = self.zh_lm.as_ref()?;
        let word = candidate.surface.as_str();
        let base = lm.uni(word);
        if let Some(left) = self.committed_context_word() {
            let context =
                self.ctx.bigram_mu * lm.bi(left, word) + (1.0 - self.ctx.bigram_mu) * base;
            if base > 0.0 && context >= RELATED_LABEL_LIFT * base {
                return Some("related");
            }
        }
        (base >= COMMON_LABEL_PROBABILITY).then_some("common")
    }

    /// 已送出左文尾巴的連續漢字（最多四個字）；當作鄰詞機率的左文。
    fn committed_context_word(&self) -> Option<&str> {
        let start = self
            .committed
            .char_indices()
            .rev()
            .take_while(|(_, ch)| is_han(*ch))
            .take(4)
            .last()
            .map(|(index, _)| index)?;
        Some(&self.committed[start..])
    }

    /// Enter：候選窗有動作就先套用並保留組字，否則走 lane 的送出流程。
    ///
    /// 兩段式、選單一次送出與前導變體例外都由 [`CandidateAction`] 決定，
    /// 不再為每個 lane 各寫一份。
    pub(super) fn finish_composition(&mut self) {
        self.prepare_ja_conversion();
        if let Some(action) = self.enter_action() {
            if self.apply_candidate_action(&action) {
                self.finish_choice();
                return;
            }
            // `zh_window_last` 套用失敗時回到整段候選，再照一般流程送出；
            // 與舊行為一致，下一次 Enter 不會重試同一個壞掉的清單。
            if self.zh_window_last {
                self.zh_window_last = false;
                self.rebuild();
            }
        }
        self.finalize_lane();
    }

    /// 沒有要先套用的候選時，把整段組字交給 lane 送出。
    fn finalize_lane(&mut self) {
        if self.config.mode == Mode::Mixed && !self.in_zh_lane() && !self.in_ja_lane() {
            self.ascii.flush();
            self.trim_mixed_candidate_space();
        }
        self.remember_selection();
        self.learn_composition();
        let text = self.current_text();
        let text = self.finalize(text);
        if !text.is_empty() {
            self.pending_commit = Some(text);
        }
        self.clear_composition();
    }

    /// 這次 Enter 要套用的候選動作；沒有候選窗或前導變體的高亮即現值時回 `None`。
    fn enter_action(&self) -> Option<CandidateAction> {
        if !self.candidate_window || self.candidates.is_empty() {
            return None;
        }
        let action = self.candidate_action(self.highlighted)?;
        // 反引號前導的多變體符號：高亮項就是組字已顯示的字面，套用等於
        // 什麼都沒換，Enter 直接送出整段，不要讓使用者多按一次。
        if matches!(action.lane, ActionLane::Mixed | ActionLane::Insert)
            && self.highlighted_choice_is_current_literal()
        {
            return None;
        }
        Some(action)
    }

    /// 延遲整句模式：還在假名就同步轉換一次再提交，不默默送出假名。
    fn prepare_ja_conversion(&mut self) {
        if !self.in_ja_lane() {
            return;
        }
        let pending = self
            .live_plan
            .as_ref()
            .is_none_or(|plan| plan.result.is_none());
        let has_units = !self.romaji.units().is_empty();
        if pending && has_units {
            if let Some(live) = self.live.as_mut() {
                if live.sentence_mode() {
                    let plan = live.convert_now();
                    let units = self.romaji.units().to_vec();
                    let reading: String = units.iter().map(Unit::as_str).collect();
                    self.apply_live_plan(&plan, &units, &reading);
                }
            }
        }
    }

    /// 目前高亮的候選是否就是組字裡已經顯示的字面。
    ///
    /// 反引號前導的多變體符號會先插入預設值，再開候選窗列出同類寫法；游標
    /// 停在剛插入的符號上、高亮第一項時，套用候選不會換掉任何字。Enter
    /// 應該直接送出整段，只有真的選到不同字時才先套用。
    fn highlighted_choice_is_current_literal(&self) -> bool {
        if !self.candidate_window || self.edit.is_some() || !self.zh.pending().is_empty() {
            return false;
        }
        let Some(Some(choice)) = self.mixed_choices.get(self.highlighted) else {
            return false;
        };
        if choice.syllable_edit.is_some() || !choice.leftover.is_empty() || choice.force_literal {
            return false;
        }
        let Some(target) = self.mixed_target.as_ref() else {
            return false;
        };
        if target.reading.is_some() || target.ja {
            return false;
        }
        let text = self.base_with_highlight();
        let surface: String = text
            .chars()
            .skip(target.rune_start)
            .take(target.rune_len)
            .collect();
        surface == choice.insert
    }

    /// 送出前套用中英自動空格；簡體轉換留到 [`Session::commit`]。
    pub(super) fn finalize(&self, text: String) -> String {
        let text = self.puncted(text);
        if self.config.cjk_latin_space {
            space_text(&text, Spacing::Always)
        } else {
            text
        }
    }

    /// 依執行期開關把送出文字轉成簡體。
    ///
    /// 只在 [`Session::commit`] 呼叫，組字中的預覽與候選保持繁體。
    pub(super) fn output_text(&self, text: String) -> String {
        if !self.output_simplified {
            return text;
        }
        match &self.config.simplified {
            Some(converter) => converter.convert(&text),
            None => text,
        }
    }

    /// 依設定在最後一個漢字前插入標點；規則同 [`punct::apply`]。
    pub(super) fn puncted(&self, text: String) -> String {
        let Some(mark) = self.config.punct_mark else {
            return text;
        };
        let chars: Vec<char> = text.chars().collect();
        let Some(offset) = chars.iter().rposition(|ch| is_han(*ch)) else {
            return text;
        };
        let edits = [Edit::new(offset, mark.to_string())];
        match punct::apply(&text, &edits) {
            (out, true) => out,
            (_, false) => text,
        }
    }

    /// 開啟選單並重建候選；選取時插入到目前組字尾端。
    pub(super) fn open_menu(&mut self, kind: MenuKind, tab: usize) -> Snapshot {
        if self.literal.is_empty() {
            self.literal_anchor = self.syllables.len();
        }
        self.literal_offset = self
            .segments
            .last()
            .map(|segment| segment.end)
            .unwrap_or(self.base.len());
        let names = self.menu_tab_names(kind);
        let tab = tab.min(names.len().saturating_sub(1));
        self.menu = Some(Menu { kind, tab });
        self.highlighted = 0;
        self.candidate_window = true;
        self.rebuild();
        self.snapshot()
    }

    /// 選單的分頁名稱。
    pub(super) fn menu_tab_names(&self, kind: MenuKind) -> Vec<&'static str> {
        match kind {
            MenuKind::Symbols => {
                let mut names = Vec::new();
                if !self.symbol_recent.is_empty() {
                    names.push(symbols::RECENT);
                }
                names.extend(symbols::TABS.iter().map(|tab| tab.name));
                names
            }
            MenuKind::Emoji => vec!["emoji", "kaomoji"],
        }
    }

    /// 目前分頁的候選字面。
    pub(super) fn menu_items(&self, menu: Menu) -> Vec<String> {
        match menu.kind {
            MenuKind::Symbols => {
                let names = self.menu_tab_names(menu.kind);
                let Some(name) = names.get(menu.tab) else {
                    return Vec::new();
                };
                if *name == symbols::RECENT {
                    symbols::clean_recent(&self.symbol_recent)
                        .into_iter()
                        .take(PAGE_SIZE)
                        .collect()
                } else {
                    symbols::tab_items(name)
                        .map(|items| items.iter().map(|item| (*item).to_string()).collect())
                        .unwrap_or_default()
                }
            }
            MenuKind::Emoji => {
                let kind = if menu.tab == 0 { "emoji" } else { "kaomoji" };
                let mut items = Vec::new();
                if kind == "emoji" {
                    items.push("=".to_string());
                }
                items.extend(emoji::palette(kind, &self.emoji_recent));
                items
            }
        }
    }

    pub(super) fn rebuild_menu(&mut self) {
        let Some(menu) = self.menu else {
            return;
        };
        let items = self.menu_items(menu);
        self.candidates = items
            .into_iter()
            .map(|text| Candidate::new(text, "", 0, CandidateSource::Literal))
            .collect();
        self.candidate_window = true;
        if self.highlighted >= self.candidates.len() {
            self.highlighted = 0;
        }
    }

    /// 選單開啟時的 Tab／Shift＋Tab 與退格；其他按鍵交回一般處理。
    pub(super) fn menu_key(&mut self, key: &Key) -> bool {
        let Some(menu) = self.menu else {
            return false;
        };
        match key.code {
            KeyCode::Tab => {
                let names = self.menu_tab_names(menu.kind);
                if names.is_empty() {
                    return false;
                }
                let step = if key.shift { -1 } else { 1 };
                let len = names.len() as isize;
                let tab = (menu.tab as isize + step).rem_euclid(len) as usize;
                self.menu = Some(Menu {
                    kind: menu.kind,
                    tab,
                });
                self.highlighted = 0;
                true
            }
            KeyCode::Backspace | KeyCode::Escape | KeyCode::Delete => {
                // 只關選單，不刪除已組好的字。
                self.menu = None;
                self.candidate_window = false;
                true
            }
            _ => false,
        }
    }

    /// 記下目前高亮候選的字面；只有明確開啟選單時才算。
    pub(super) fn remember_selection(&mut self) {
        let Some(text) = self
            .candidates
            .get(self.highlighted)
            .map(|candidate| candidate.surface.clone())
        else {
            return;
        };
        self.remember_text(&text);
    }

    /// 記下指定字面的選取；數字鍵直接選取時要高亮以外的項目。
    pub(super) fn remember_text(&mut self, text: &str) {
        let Some(menu) = self.menu else {
            return;
        };
        let text = text.to_string();
        match menu.kind {
            MenuKind::Symbols => {
                if symbols::contains(&text) {
                    let order = symbols::clean_recent(&self.symbol_recent);
                    let (order, stats) =
                        emoji::rank(&order, &self.symbol_stats, &text, 0, symbols::MAX_RECENT);
                    self.symbol_recent = order;
                    self.symbol_stats = stats;
                }
            }
            MenuKind::Emoji => {
                if text != "=" {
                    let (order, stats) = emoji::rank(
                        &self.emoji_recent,
                        &self.emoji_stats,
                        &text,
                        0,
                        emoji::MAX_RECENT,
                    );
                    self.emoji_recent = order;
                    self.emoji_stats = stats;
                    self.palette_tab = menu.tab;
                }
            }
        }
    }

    /// 方向鍵：只是顯示候選時移動組字游標，選取模式中換項；還沒開窗時先開窗。
    ///
    /// 對照上游 `shownCandidateKey`：候選窗顯示中但還沒按 ↓ 時，左右鍵移動
    /// 組字游標；進入選取模式後才在候選間移動。游標移動後，候選清單改成
    /// 游標所在片段的候選。
    pub(super) fn navigate(&mut self, delta: isize) {
        // 候選窗開著時四方向鍵都屬於候選介面：游標不能移動。第一次按 →／←
        // 先展開成四欄，之後才在候選間移動；欄數與語言無關。
        if self.candidate_window && self.navigating && !self.candidates.is_empty() {
            if !self.candidate_expanded {
                self.candidate_expanded = true;
                return;
            }
            self.move_column(delta);
            return;
        }
        if self.in_ja_lane() && self.live.is_some() {
            if self.move_ja_cursor(delta) {
                return;
            }
            // 游標已經在組字頭尾：方向鍵不開候選窗，也不留下選取狀態。
            // 舊路徑會落到下面的 `navigating = true`，在結尾按 → 時跳出一個
            // 只有目前詞面的候選窗。
            self.candidate_window = false;
            self.candidate_expanded = false;
            self.navigating = false;
            self.keep_candidates = false;
            return;
        }
        if self.config.mode == Mode::Mixed && !self.in_zh_lane() && !self.in_ja_lane() {
            self.navigate_mixed(delta);
            return;
        }
        if self.in_zh_lane() && (!self.zh_pending_keys.is_empty() || !self.zh.pending().is_empty())
        {
            // 注音還沒打完就移動游標：待定音節凍結成原始按鍵字面，不能被
            // 混打管線重新讀成日文或英文。游標停在凍結尾巴的左緣。
            let completed = self.freeze_pending_zhuyin_for_mixed();
            let len = self.base_with_highlight().chars().count();
            self.move_mixed_cursor_to(if delta < 0 { completed } else { len });
            return;
        }
        if self.in_zh_lane() && self.candidate_window && !self.navigating {
            let target = self.cursor.saturating_add_signed(delta);
            if self.move_cursor_to(target) {
                return;
            }
            // 已經在組字頭尾：左右鍵改成換候選，與舊行為一致。
            if !self.candidates.is_empty() {
                self.move_highlight(delta);
            }
            return;
        }
        if self.in_zh_lane() && !self.candidate_window {
            // 候選窗沒開時方向鍵只移動游標，不順便開窗。
            self.move_cursor_to(self.cursor.saturating_add_signed(delta));
            return;
        }
        if self.candidates.is_empty() {
            return;
        }
        self.navigating = true;
        if !self.candidate_window {
            self.candidate_window = true;
            return;
        }
        self.move_highlight(delta);
    }

    /// 把混合模式的游標移到指定 rune；會關掉候選窗。
    pub(super) fn move_mixed_cursor_to(&mut self, pos: usize) {
        let len = self.base_with_highlight().chars().count();
        let target = pos.min(len);
        if target == self.mixed_cursor.min(len) {
            return;
        }
        self.mixed_cursor = target;
        self.candidate_window = false;
        self.candidate_expanded = false;
        self.navigating = false;
        self.keep_candidates = false;
        self.edit = None;
        self.highlighted = 0;
        self.mixed_target = None;
        self.mixed_choices.clear();
    }

    /// 混合模式的方向鍵：逐 rune 移動游標。
    ///
    /// 顯示字串可能是注音詞、かな或英文；游標一律停在 rune 邊界，跨越片段
    /// 邊界時直接進到相鄰片段的第一／最後一個 rune。
    pub(super) fn navigate_mixed(&mut self, delta: isize) {
        let text = self.base_with_highlight();
        let len = text.chars().count();
        if len == 0 {
            return;
        }
        let at = self.mixed_cursor.min(len);
        let target = at.saturating_add_signed(delta).min(len);
        self.move_mixed_cursor_to(target);
    }

    /// 把這次送出的候選記進個人學習。
    pub(super) fn learn_composition(&mut self) {
        if self.private {
            log::debug!("私密欄位：跳過這次提交的學習");
            return;
        }
        let index = self.highlighted;
        self.learn_candidate(index);
    }

    /// 把指定候選記進個人學習；數字鍵直接選取時用得到。
    ///
    /// 動手前先留一份快照：下一次輸入若是 Backspace／Esc／Ctrl+Z，就把這次
    /// 學習補償掉；否則在 [`Session::press`] 開頭丟掉快照。
    pub(super) fn learn_candidate(&mut self, index: usize) {
        if self.private {
            log::debug!("私密欄位：跳過候選學習");
            return;
        }
        if !self.config.learning || self.menu.is_some() {
            return;
        }
        self.personal.begin_rollback(now_millis());
        if self.in_zh_lane() {
            self.note_learning_event();
            self.learn_zh_candidate(index);
            if self.config.mode == Mode::Mixed {
                self.learn_zh_lane_languages();
            }
            return;
        }
        if self.in_ja_lane() {
            self.note_learning_event();
            self.learn_ja_candidate(index);
            return;
        }
        self.note_learning_event();
        self.learn_mixed_languages();
    }

    /// 推進時間並定期清理個人資料；上限與衰減沿用 `personal::prune`。
    pub(super) fn note_learning_event(&mut self) {
        self.learn_events += 1;
        self.personal.advance_time(now_seconds());
        if self.learn_events.is_multiple_of(LEARN_PRUNE_INTERVAL) {
            let policy = crate::personal::PrunePolicy::default();
            self.personal.prune_capacity(policy);
            self.personal.prune_languages(policy);
        }
    }

    /// 把一次注音提交記進個人學習。
    ///
    /// 被動接受且讀音短於 [`SessionConfig::min_trigger_len`] 時只留證據，不改
    /// 排序；明確手選不受門檻限制。
    pub(super) fn learn_zh_candidate(&mut self, index: usize) {
        let manual = self.candidate_window;
        let Some(candidate) = self.candidates.get(index).cloned() else {
            return;
        };
        let units: Vec<&str> = candidate
            .reading
            .split_whitespace()
            .filter(|unit| !unit.is_empty())
            .collect();
        if units.is_empty() {
            return;
        }
        self.note_candidate_skips(index, &candidate.surface);
        let short = crate::personal::reading_len(&candidate.reading) < self.config.min_trigger_len;
        let hypothesis = self
            .last_hypotheses
            .iter()
            .find(|hypothesis| hypothesis.text == candidate.surface);
        if let Some(hypothesis) = hypothesis {
            for segment in &hypothesis.segments {
                if segment.partial
                    || segment.start >= segment.end
                    || segment.end > units.len()
                    || !segment.word.chars().next().is_some_and(is_han)
                    || crate::learn_guard::is_sensitive(&segment.word)
                {
                    continue;
                }
                let reading = units[segment.start..segment.end].join(" ");
                let syllables = segment.end - segment.start;
                if manual {
                    self.personal
                        .note_manual(&segment.word, &reading, syllables);
                } else if short {
                    self.personal
                        .note_passive_evidence(&segment.word, self.personal.passive_weight());
                } else {
                    self.personal.note_passive(&segment.word, &reading);
                }
            }
        } else if candidate.surface.chars().next().is_some_and(is_han) {
            if crate::learn_guard::is_sensitive(&candidate.surface) {
                log::debug!("學習守門：注音候選像敏感資料，跳過寫入");
                return;
            }
            // 模糊音或自訂詞候選沒有對應路徑，整串當一個單位學習。
            let syllables = units.len();
            if manual {
                self.personal
                    .note_manual(&candidate.surface, &candidate.reading, syllables);
            } else if short {
                self.personal
                    .note_passive_evidence(&candidate.surface, self.personal.passive_weight());
            } else {
                self.personal
                    .note_passive(&candidate.surface, &candidate.reading);
            }
        }
    }

    /// 記錄一次英文選取：修正或跳過。
    fn learn_english_choice(&mut self, wrong: &str, chosen: &str) {
        if !self.config.learning
            || self.private
            || crate::learn_guard::is_sensitive(wrong)
            || crate::learn_guard::is_sensitive(chosen)
            || wrong.is_empty()
            || !wrong.chars().all(|ch| ch.is_ascii_alphabetic())
            || !chosen.chars().all(|ch| ch.is_ascii_alphabetic())
        {
            return;
        }
        if let Some(target) = self.personal.english_correction_target(wrong) {
            let target = target.to_string();
            let key = wrong.to_lowercase();
            if chosen.eq_ignore_ascii_case(&target) {
                self.personal.clear_skip(&key);
            } else {
                self.personal.note_skip(&key);
            }
        }
        // 修正不是原字的前綴；前綴補完只是把字補長，不當拼字修正學習。
        if !chosen.eq_ignore_ascii_case(wrong)
            && !chosen.to_lowercase().starts_with(&wrong.to_lowercase())
        {
            self.personal.note_english_correction(wrong, chosen);
        }
    }

    /// 把一次日文提交記進共用個人資料的 `ja` 命名空間。
    ///
    /// 優先使用這一輪 N-best 的文節切分；候選不是這一輪結果時，退化成整筆
    /// 候選一個文節。明確手選（候選窗開著）比被動接受多一倍證據。被動接受
    /// 且讀音短於 [`SessionConfig::min_trigger_len`] 時只記詞面證據，不改排序。
    pub(super) fn learn_ja_candidate(&mut self, index: usize) {
        let Some(candidate) = self.candidates.get(index).cloned() else {
            return;
        };
        let manual = self.candidate_window;
        self.learn_ja_candidate_data(index, &candidate, manual);
    }

    /// 日文候選套用成功後的學習；候選由套用前先留下，重建後仍記得到這一項。
    fn learn_ja_choice(&mut self, index: usize, candidate: &Candidate, manual: bool) {
        if self.private {
            log::debug!("私密欄位：跳過候選學習");
            return;
        }
        if !self.config.learning || self.menu.is_some() {
            return;
        }
        self.personal.begin_rollback(now_millis());
        self.note_learning_event();
        self.learn_ja_candidate_data(index, candidate, manual);
    }

    fn learn_ja_candidate_data(&mut self, index: usize, candidate: &Candidate, manual: bool) {
        if crate::learn_guard::is_sensitive(&candidate.surface) {
            log::debug!("學習守門：日文候選像敏感資料，跳過寫入");
            return;
        }
        let weight = if manual {
            ja_personal::UNIFIED_MANUAL_WEIGHT
        } else {
            ja_personal::UNIFIED_PASSIVE_WEIGHT
        };
        let clauses = match self
            .last_ja_results
            .iter()
            .find(|result| result.surface() == candidate.surface)
        {
            Some(result) => result.clauses.clone(),
            None => vec![JaClause {
                reading: candidate.reading.clone(),
                surface: candidate.surface.clone(),
                segments: Vec::new(),
                cost: candidate.cost,
            }],
        };
        if clauses
            .iter()
            .any(|clause| crate::learn_guard::is_sensitive(&clause.surface))
        {
            log::debug!("學習守門：日文文節像敏感資料，跳過寫入");
            return;
        }
        self.note_ja_skips(index, &candidate.surface);
        let at = now_seconds();
        let short = crate::personal::reading_len(&candidate.reading) < self.config.min_trigger_len;
        if short && !manual {
            for clause in &clauses {
                self.personal.note_language_surface(
                    crate::personal::LanguageId::Ja,
                    &clause.surface,
                    weight,
                    at,
                );
            }
            return;
        }
        let left = self.committed.clone();
        ja_personal::learn_unified(&mut self.personal, &clauses, &left, weight, at);
    }

    /// 日文的弱負向訊號：選了非學習候選時，對前面被跳過的結果記一次跳過。
    pub(super) fn note_ja_skips(&mut self, index: usize, chosen: &str) {
        self.personal.clear_skip(chosen);
        if self
            .personal
            .language_surface_count(crate::personal::LanguageId::Ja, chosen)
            > 0
        {
            return;
        }
        let skipped: Vec<String> = self
            .last_ja_results
            .iter()
            .take(index)
            .map(|result| result.surface().to_string())
            .filter(|surface| {
                self.personal
                    .language_surface_count(crate::personal::LanguageId::Ja, surface)
                    > 0
            })
            .collect();
        for surface in skipped {
            self.personal.note_skip(&surface);
        }
    }

    /// 候選窗的反悔鍵：自訂詞刪除、學到的詞忘記。
    pub(super) fn forget_candidate(&mut self) {
        let Some(candidate) = self.candidates.get(self.highlighted).cloned() else {
            return;
        };
        if self.personal.is_vocab(&candidate.surface) {
            self.personal.remove_vocab(&candidate.surface);
            self.refresh_lexicon();
        } else if self.personal.is_learned(&candidate.surface) {
            self.personal.forget(&candidate.surface);
        }
        self.highlighted = 0;
    }

    /// 自訂詞變動後重建詞典。
    pub(super) fn refresh_lexicon(&mut self) {
        self.lexicon = self.personal.augment_lexicon(&self.base_lexicon);
        self.install_tone_gate();
    }

    pub(super) fn move_highlight(&mut self, delta: isize) {
        let len = self.candidates.len();
        if len == 0 {
            return;
        }
        self.candidate_window = true;
        if len == 1 {
            return;
        }
        let len = len as isize;
        let mut index = (self.highlighted as isize + delta) % len;
        if index < 0 {
            index += len;
        }
        self.highlighted = index as usize;
    }

    /// `↑`／`↓`：整份候選線性移動一項，跨欄、跨頁時自然接續。
    pub(super) fn move_row(&mut self, delta: isize) {
        let len = self.candidates.len();
        if len == 0 {
            return;
        }
        self.candidate_window = true;
        let layout = self.candidate_layout();
        self.highlighted = if delta < 0 {
            layout.up(self.highlighted, len)
        } else {
            layout.down(self.highlighted, len)
        };
    }

    /// `←`／`→`：跳一整欄（十項），停在同一個列號；單欄時不移動。
    pub(super) fn move_column(&mut self, delta: isize) {
        let len = self.candidates.len();
        if len == 0 {
            return;
        }
        self.candidate_window = true;
        let layout = self.candidate_layout();
        self.highlighted = if delta < 0 {
            layout.left(self.highlighted, len)
        } else {
            layout.right(self.highlighted, len)
        };
    }

    pub(super) fn clear_composition(&mut self) {
        self.ja_run = false;
        self.ja_run_keys.clear();
        self.zh.reset();
        self.zh_lane = false;
        self.zh_space_closed = false;
        self.zh_pending_keys.clear();
        self.zh_window_last = false;
        self.zh_run_breaks.clear();
        self.syllables.clear();
        self.cursor = 0;
        self.ja_cursor = 0;
        self.mixed_cursor = Self::MIXED_CURSOR_END;
        self.edit = None;
        self.keep_candidates = false;
        self.romaji.reset();
        self.ascii.reset();
        self.literal.clear();
        self.literal_anchor = 0;
        self.literal_offset = 0;
        self.literal_tail.clear();
        self.candidates.clear();
        self.highlighted = 0;
        self.candidate_window = false;
        self.navigating = false;
        self.segments.clear();
        self.composition.clear();
        self.locks.clear();
        self.segment_manual = None;
        self.pending_tone = None;
        self.base.clear();
        self.pending.clear();
        self.last_hypotheses.clear();
        self.last_ja_results.clear();
        self.accepted_completion = None;
        self.suggestion = None;
        self.menu = None;
        if let Some(live) = &mut self.live {
            live.reset();
        }
        self.live_plan = None;
    }

    /// 目前要提交的文字：接受的補全優先，其次高亮候選，最後轉換結果。
    pub(super) fn current_text(&self) -> String {
        let mut text = self.current_text_without_literal_tail();
        text.push_str(&self.literal_tail);
        text
    }

    fn current_text_without_literal_tail(&self) -> String {
        // 選單開著時送出高亮候選，字面按鍵插在原本的位置。
        if self.menu.is_some() {
            if let Some(candidate) = self.candidates.get(self.highlighted) {
                return self.compose_literal(&candidate.surface);
            }
        }
        if self.config.mode == Mode::Mixed && !self.in_zh_lane() && !self.in_ja_lane() {
            // 混打的字面標點、選單插入與前導符號記在 `literal`，要一起送出。
            return self.insert_literal(&self.mixed_surface());
        }
        if let Some(text) = &self.accepted_completion {
            return format!("{}{}", self.insert_literal(text), self.pending);
        }
        // 片段編輯中的候選只換那一段；送出時用已套用鎖定的整段結果。
        if self.edit.is_some() {
            return self.preedit_string();
        }
        // 注音路徑的組字區就是 `base`（轉換後的中文），候選窗沒開時 Enter
        // 要送出看到的字，不能拿候選清單第一項的字面讀音。詞典成本高於未知
        // 詞時，第一候選可能還是 `ㄨㄢ`，送出的字就會和畫面上不一致。
        if self.in_zh_lane() && !self.candidate_window {
            return self.preedit_string();
        }
        if let Some(candidate) = self.candidates.get(self.highlighted) {
            return format!(
                "{}{}",
                self.insert_literal(&candidate.surface),
                self.pending
            );
        }
        self.preedit_string()
    }

    /// 高亮候選或轉換結果加上尚未轉換的尾巴。
    pub(super) fn preedit_string(&self) -> String {
        format!("{}{}", self.insert_literal(&self.base), self.pending)
    }

    /// 顯示用的轉換結果：接受的補全或高亮候選，混合模式維持中文片段。
    pub(super) fn displayed_base(&self) -> &str {
        if self.menu.is_some() {
            // 選單開啟時組字區維持原樣，候選只是待選清單。
            return &self.base;
        }
        // 片段編輯時候選只涵蓋游標那一段，不能拿來取代整個組字字串。
        if self.edit.is_some() {
            return &self.base;
        }
        // 純注音組字的 Mixed 顯示整段最佳路徑；候選只供選字，不取代組字區。
        if self.config.mode == Mode::Mixed && self.zh_lane {
            return &self.base;
        }
        match (
            &self.accepted_completion,
            self.candidates.get(self.highlighted),
        ) {
            (Some(text), _) => text.as_str(),
            // 混合模式的候選只取代待定英數按鍵，`base` 已含前面的中文片段；
            // 直接拿候選當整個組字字串會把中文吃掉。純注音路徑照注音處理。
            (None, _)
                if self.config.mode == Mode::Mixed && !self.in_zh_lane() && !self.in_ja_lane() =>
            {
                &self.base
            }
            (None, Some(candidate)) => candidate.surface.as_str(),
            (None, None) => &self.base,
        }
    }

    /// 顯示用字串：高亮候選反映在組字區，未完成的注音插在游標位置。
    pub(super) fn base_with_highlight(&self) -> String {
        let base = self.insert_literal(self.displayed_base());
        let mut out = if self.pending.is_empty() {
            base
        } else if !self.in_zh_lane() || self.cursor >= self.syllables.len() {
            format!("{}{}", base, self.pending)
        } else {
            let mut at = self.cell_byte(&base, self.cursor).unwrap_or(base.len());
            while at > 0 && !base.is_char_boundary(at) {
                at -= 1;
            }
            let mut out = String::with_capacity(base.len() + self.pending.len());
            out.push_str(&base[..at]);
            out.push_str(&self.pending);
            out.push_str(&base[at..]);
            out
        };
        out.push_str(&self.literal_tail);
        out
    }

    /// 把字面按鍵插回轉換結果的對應位置。
    pub(super) fn insert_literal(&self, base: &str) -> String {
        if self.literal.is_empty() {
            return base.to_string();
        }
        let mut offset = self.literal_offset.min(base.len());
        while offset > 0 && !base.is_char_boundary(offset) {
            offset -= 1;
        }
        let mut out = String::with_capacity(base.len() + self.literal.len());
        out.push_str(&base[..offset]);
        out.push_str(&self.literal);
        out.push_str(&base[offset..]);
        out
    }

    /// 把選單選取的字面插回目前組字結果的對應位置。
    pub(super) fn compose_literal(&self, text: &str) -> String {
        let mut offset = self.literal_offset.min(self.base.len());
        while offset > 0 && !self.base.is_char_boundary(offset) {
            offset -= 1;
        }
        let mut out = String::with_capacity(self.base.len() + self.literal.len() + text.len());
        out.push_str(&self.base[..offset]);
        out.push_str(&self.literal);
        out.push_str(text);
        out.push_str(&self.base[offset..]);
        out
    }

    pub(super) fn rebuild(&mut self) {
        if !self.candidate_window {
            self.mixed_target = None;
            self.mixed_choices.clear();
            self.candidate_expanded = false;
        }
        // 逐字元候選窗開著時，重建會把剛產生的清單換掉；游標與組字內容
        // 都沒變，直接保留。
        if self.candidate_window && self.mixed_target.is_some() {
            return;
        }
        if self.menu.is_some() {
            self.rebuild_menu();
            return;
        }
        self.base.clear();
        self.pending.clear();
        // 日文逐字変換的純重畫（游標移動、開候選窗）讀音沒變，保留候選
        // 清單；讀音一變就清掉，交給 `apply_live_plan` 重建。
        // 真正轉換時候選會整份換掉。
        let keep_ja_candidates = self.in_ja_lane() && self.live.is_some() && {
            let reading: String = self.romaji.units().iter().map(Unit::as_str).collect();
            reading == self.composition.reading(crate::composition::Lang::Ja)
        };
        if !keep_ja_candidates {
            self.candidates.clear();
        }
        self.segments.clear();
        // 注音路徑由 `rebuild_zh` 依字面插入點重算；混打與日文保留
        // `push_literal` 當下固定的尾端位置。
        if self.zh_lane {
            self.literal_offset = 0;
        }
        if self.in_ja_lane() {
            self.rebuild_ja();
        } else if self.zh_lane {
            self.rebuild_zh();
        } else {
            self.rebuild_mixed();
        }
        if self.highlighted >= self.candidates.len() {
            self.highlighted = 0;
        }
    }

    /// 注音顯示與候選共用的排序：nbest → 個人學習 → rerank。
    ///
    /// 兩邊各自排序會分叉（`ji3` 顯示「我」、候選第一項「捰」），所以只在
    /// 這裡排一次，呼叫端都拿同一份結果。
    pub(super) fn rank_zh_hypotheses(
        &self,
        mut hypotheses: Vec<Hypothesis>,
        reading: &str,
    ) -> Vec<Hypothesis> {
        if self.config.learning {
            self.personal.rank_hypotheses(&mut hypotheses, reading);
        }
        if let Some(options) = &self.config.rerank {
            Self::apply_rerank(&mut hypotheses, options);
        }
        hypotheses
    }

    /// 有詞典字時不讓字面讀音排第一，並讓顯示與候選第一項指向同一條路徑。
    ///
    /// 優先取整條路徑都是詞典字的一條；`nbest` 會對非詞疊字加成本，
    /// `decoder.best` 不會，所以第一條可能是「字面讀音＋詞」而不是整串詞。
    fn prefer_dictionary_hypothesis(hypotheses: &mut Vec<Hypothesis>, literal: &str) {
        let dictionary_path = |hypothesis: &Hypothesis| {
            !hypothesis.segments.is_empty()
                && hypothesis
                    .segments
                    .iter()
                    .all(|segment| segment.word.chars().any(is_han))
        };
        let index = hypotheses
            .iter()
            .position(|hypothesis| hypothesis.text != literal && dictionary_path(hypothesis))
            .or_else(|| {
                hypotheses.iter().position(|hypothesis| {
                    hypothesis.text != literal && hypothesis.text.chars().any(is_han)
                })
            });
        let Some(index) = index else {
            return;
        };
        if index > 0 {
            let display = hypotheses.remove(index);
            hypotheses.insert(0, display);
        }
    }

    pub(super) fn rebuild_zh(&mut self) {
        self.pending = self.zh.pending();
        let syllables = self.syllables.to_vec();
        if syllables.is_empty() {
            self.sync_composition_zh(&[], &[]);
            return;
        }
        let readings = self.zh_readings(&syllables);
        let units: Vec<&str> = readings.iter().map(String::as_str).collect();
        // 片段鎖定只涵蓋目前還在的音節；由 `Composition` 的 User 標記推導，
        // 過期的標記在推導時自然被丟掉。
        self.sync_zh_locks();
        let Ok(lattice) = Lattice::build(&units, &self.lexicon, &self.ctx) else {
            self.base = units.join("");
            self.sync_composition_zh(&readings, &[]);
            return;
        };
        let scorer = zh_scorer(&self.ctx, self.zh_lm.as_deref());
        let decoder = zh_decoder(&self.lexicon, &self.ctx, scorer.as_ref());
        let reading = readings.join(" ");
        let literal = readings.join("");
        // 顯示與候選共用同一份排序：nbest → 個人學習 → rerank。兩邊各自
        // 再算一次會分叉（`ji3` 顯示「我」、清單第一項卻是「捰」）。
        let (hypotheses, _) = decoder.nbest(&lattice, &self.locks, false, self.max_candidates());
        let mut hypotheses = self.rank_zh_hypotheses(hypotheses, &reading);
        Self::prefer_dictionary_hypothesis(&mut hypotheses, &literal);
        let display_segments = if let Some(best) = hypotheses.first() {
            self.base = best.text.clone();
            self.segments = Self::segments_of(&best.segments, Lang::Zh, Script::Bopomofo);
            best.segments.clone()
        } else {
            // nbest 沒有結果時退回解碼器的最佳路徑，至少維持可顯示與可送出。
            let (segs, _) = decoder.best(&lattice, &self.locks, false);
            let conversion = decoder.conversion(&lattice, &segs, Lang::Zh);
            self.base = conversion.surface;
            self.segments = Self::segments_of(&segs, Lang::Zh, Script::Bopomofo);
            segs
        };
        self.literal_offset = if self.literal_anchor == 0 {
            0
        } else {
            self.segments
                .get(self.literal_anchor - 1)
                .map(|segment| segment.end)
                .unwrap_or(self.base.len())
        };
        self.last_hypotheses = hypotheses.clone();
        let mixed_lane = self.config.mode == Mode::Mixed && self.zh_lane;
        let mut candidates: Vec<Candidate> = hypotheses
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
        let near = self.fuzzy_candidates(&readings);
        if !near.is_empty() {
            let at = candidates
                .iter()
                .position(|candidate| candidate.surface == literal)
                .unwrap_or(candidates.len());
            for (offset, candidate) in near.into_iter().enumerate() {
                let index = (at + offset).min(candidates.len());
                candidates.insert(index, candidate);
            }
            let mut seen = Vec::new();
            candidates.retain(|candidate| {
                if seen.contains(&candidate.surface) {
                    false
                } else {
                    seen.push(candidate.surface.clone());
                    true
                }
            });
        }
        // 尾端差一個符號的完整詞：一般候選只是顯示，選了才改字。
        let phonetic = self.phonetic_candidates(&display_segments);
        if !phonetic.is_empty() {
            let mut seen: Vec<String> = candidates
                .iter()
                .map(|candidate| candidate.surface.clone())
                .collect();
            for candidate in phonetic {
                if seen.contains(&candidate.surface) {
                    continue;
                }
                seen.push(candidate.surface.clone());
                candidates.push(candidate);
            }
        }
        candidates.truncate(self.max_candidates());
        // Mixed 的純注音顯示沿用混打管線：整段沒有詞可解時，若模糊音候選
        // 有真字就顯示那個字，而不是留一串注音。
        if mixed_lane && self.base == literal {
            if let Some(first) = candidates.first() {
                if first.surface.chars().next().is_some_and(is_han) {
                    self.base = first.surface.clone();
                    self.segments =
                        vec![Segment::new(0, self.base.len(), Lang::Zh, Script::Bopomofo)];
                }
            }
        }
        // 顯示字必須就是第一候選；模糊音或字面候選插在前面時把顯示字移回
        // 第一項，兩條路徑才不會再各說各話。
        if candidates
            .first()
            .map(|candidate| candidate.surface.as_str())
            != Some(self.base.as_str())
        {
            if let Some(at) = candidates
                .iter()
                .position(|candidate| candidate.surface == self.base)
            {
                let display = candidates.remove(at);
                candidates.insert(0, display);
            }
        }
        self.candidates = candidates;
        // 混打管線把一段連續注音當一個片段（`字字`），純注音模式則照詞切。
        // Mixed 的組字字串沿用前者：相鄰同字面、中間沒有空白打斷的片段合併。
        if mixed_lane && self.segments.len() == display_segments.len() {
            let mut merged: Vec<Segment> = Vec::with_capacity(self.segments.len());
            for (index, segment) in std::mem::take(&mut self.segments).into_iter().enumerate() {
                let boundary = display_segments[index].start;
                let joinable = self.zh_run_breaks.binary_search(&boundary).is_err();
                if let Some(last) = merged.last_mut() {
                    if joinable
                        && last.mode == segment.mode
                        && last.script == segment.script
                        && self.base.get(last.start..last.end)
                            == self.base.get(segment.start..segment.end)
                    {
                        last.end = segment.end;
                        continue;
                    }
                }
                merged.push(segment);
            }
            self.segments = merged;
        }
        let (completion, _) = decoder.complete(&lattice, &display_segments);
        self.suggestion = if completion.found() {
            Some(completion.entry.surface)
        } else {
            None
        };
        if let Some(edit) = self.edit {
            if edit.len > 0 && edit.start.saturating_add(edit.len) <= syllables.len() {
                self.candidates = self.window_candidates(edit);
            }
        } else if self.zh_window_last
            && self.config.mode == Mode::Mixed
            && self.candidate_window
            && !syllables.is_empty()
        {
            // ↓／空白開窗：只列游標所在音節；顯示仍是整段最佳路徑。
            let last = syllables.len() - 1;
            self.candidates = self.window_candidates(EditWindow {
                start: last,
                len: 1,
            });
        }
        // 尚未完成的注音也是組字的一部分；片段要覆蓋整個 preedit。
        if !self.pending.is_empty() {
            let start = self.base.len();
            self.segments.push(Segment::new(
                start,
                start + self.pending.len(),
                Lang::Zh,
                Script::Bopomofo,
            ));
        }
        // 所有權寫回點：語言無關模型只在這一步承接新的顯示片段。
        self.sync_composition_zh(&readings, &display_segments);
    }

    /// 游標所在片段的候選；整段解碼只負責顯示，選字由這份清單決定。
    ///
    /// 只解碼 `edit.start..edit.start + edit.len` 的讀音，因此換詞長會直接換掉
    /// 候選。這一段已有鎖定時把鎖定的詞排第一，讓 `keepCandidates` 看得到剛
    /// 換上的字。
    pub(super) fn window_candidates(&self, edit: EditWindow) -> Vec<Candidate> {
        let end = edit
            .start
            .saturating_add(edit.len)
            .min(self.syllables.len());
        if edit.start >= end {
            return Vec::new();
        }
        let readings = self.zh_readings(&self.syllables[edit.start..end]);
        let reading = readings.join(" ");
        // 鎖定的詞就是目前顯示值；沒鎖定時用這一段自己的最佳解。
        let current = match self.locks.get(&edit.start) {
            Some(lock) if lock.len == edit.len => lock.word.clone(),
            _ => self.decode_reading(&reading),
        };
        // 原始輸入用讀音傳進去，由共用排序還原成按鍵（多音節時就是整段
        // 按鍵）；第一項固定是目前值，第二項是原始輸入。
        let (candidates, _) =
            self.zh_choice_candidates(&reading, &readings.join(""), &current, Some(edit));
        candidates
    }

    /// 目前音節要交給詞格的讀音。
    ///
    /// 聲調選填開啟時，沒有聲調的音節展開成五個聲調，從組合中挑成本最低的
    /// 一條；組合數超過上限（長句）時維持原樣，避免指數爆炸。
    pub(super) fn zh_readings(&self, syllables: &[Syllable]) -> Vec<String> {
        let exact: Vec<String> = syllables
            .iter()
            .map(|syllable| syllable.as_str().to_string())
            .collect();
        if !self.config.toneless {
            return exact;
        }
        let rows: Vec<Vec<String>> = syllables
            .iter()
            .map(|syllable| {
                if syllable.tone() != 1 {
                    return vec![syllable.as_str().to_string()];
                }
                (1..=5)
                    .map(|tone| syllable.with_tone(tone).as_str().to_string())
                    .collect()
            })
            .collect();
        let combinations: usize = rows.iter().map(Vec::len).product();
        if combinations == 0 || combinations > TONELESS_COMBINATIONS {
            return exact;
        }
        let mut best: Option<(i32, Vec<String>)> = None;
        let mut index = vec![0usize; rows.len()];
        loop {
            let readings: Vec<String> = rows
                .iter()
                .zip(index.iter())
                .map(|(row, &i)| row[i].clone())
                .collect();
            let units: Vec<&str> = readings.iter().map(String::as_str).collect();
            if let Ok(lattice) = Lattice::build(&units, &self.lexicon, &self.ctx) {
                let scorer = zh_scorer(&self.ctx, self.zh_lm.as_deref());
                let decoder = zh_decoder(&self.lexicon, &self.ctx, scorer.as_ref());
                let (_, cost) = decoder.best(&lattice, &Locks::new(), false);
                if best.as_ref().is_none_or(|(best_cost, _)| cost < *best_cost) {
                    best = Some((cost, readings));
                }
            }
            let mut at = index.len();
            loop {
                if at == 0 {
                    return best.map(|(_, readings)| readings).unwrap_or(exact);
                }
                at -= 1;
                index[at] += 1;
                if index[at] < rows[at].len() {
                    break;
                }
                index[at] = 0;
            }
        }
    }

    /// 依固定替身設定重排：偏好字串命中時扣掉換算後的成本。
    pub(super) fn apply_rerank(
        hypotheses: &mut [crate::decode::Hypothesis],
        options: &RerankOptions,
    ) {
        let score = if options.prefer_score > 0.0 {
            options.prefer_score
        } else {
            10.0
        };
        let bonus = (options.weight * score * 1000.0).round() as i32;
        hypotheses.sort_by_key(|hypothesis| {
            let preferred = options
                .prefer
                .iter()
                .any(|text| hypothesis.text.contains(text));
            if preferred {
                hypothesis.cost.saturating_sub(bonus)
            } else {
                hypothesis.cost
            }
        });
    }

    /// 模糊音候選：把已完成的音節換成近音讀音，取候選補進清單。
    pub(super) fn fuzzy_candidates(&self, readings: &[String]) -> Vec<Candidate> {
        if self.config.fuzzy_pairs.is_empty() {
            return Vec::new();
        }
        let mut variants: Vec<Vec<String>> = vec![Vec::new()];
        for reading in readings {
            let mut alternatives = vec![reading.clone()];
            for &(left, right) in &self.config.fuzzy_pairs {
                for (from, to) in [(left, right), (right, left)] {
                    if reading.contains(from) {
                        let swapped: String = reading
                            .chars()
                            .map(|ch| if ch == from { to } else { ch })
                            .collect();
                        if !alternatives.contains(&swapped) {
                            alternatives.push(swapped);
                        }
                    }
                }
            }
            let mut next = Vec::new();
            for prefix in &variants {
                for alternative in &alternatives {
                    let mut variant = prefix.clone();
                    variant.push(alternative.clone());
                    next.push(variant);
                }
            }
            variants = next;
            if variants.len() > FUZZY_COMBINATIONS {
                variants.truncate(FUZZY_COMBINATIONS);
            }
        }
        let original = readings.join(" ");
        let scorer = zh_scorer(&self.ctx, self.zh_lm.as_deref());
        let decoder = zh_decoder(&self.lexicon, &self.ctx, scorer.as_ref());
        let mut out: Vec<Candidate> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for variant in variants {
            if variant.as_slice() == readings {
                continue;
            }
            let units: Vec<&str> = variant.iter().map(String::as_str).collect();
            let Ok(lattice) = Lattice::build(&units, &self.lexicon, &self.ctx) else {
                continue;
            };
            // 模糊音只是補充候選；每個變體各跑一次完整 n-best 會把同音字最多
            // 的讀音成本乘上變體數。這裡只取最佳路徑，真正的候選量由上面的
            // 主 n-best 決定。
            let (segs, cost) = decoder.best(&lattice, &Locks::new(), false);
            let text: String = segs.iter().map(|seg| seg.word.as_str()).collect();
            if text.is_empty() || text == variant.join("") || seen.contains(&text) {
                continue;
            }
            seen.push(text.clone());
            out.push(Candidate::new(
                text,
                original.clone(),
                cost,
                CandidateSource::Lexicon,
            ));
        }
        out.sort_by_key(|candidate| candidate.cost);
        out
    }

    pub(super) fn rebuild_ja(&mut self) {
        self.pending = self.romaji.pending().to_string();
        let units = self.romaji.units().to_vec();
        let reading: String = units.iter().map(Unit::as_str).collect();
        if reading.is_empty() {
            self.composition.clear();
            return;
        }
        if self.live.is_some() {
            self.rebuild_ja_live(&units);
            return;
        }
        let Some(decoder) = self.config.ja.clone() else {
            self.base = reading;
            self.sync_composition_ja();
            return;
        };
        let ctx = JaContext {
            left: self.committed.clone(),
            right: String::new(),
        };
        let chars: Vec<char> = reading.chars().collect();
        let (mut results, gap) = decoder.results_with_gap(&chars, &ctx, self.max_candidates());
        // 門檻的 top-1 取學習排序前的統計結果，與 gating 實驗一致。
        let gate = results.first().map(|first| {
            (
                first.conversion().cost,
                mean_candidates(first, decoder.source().as_ref()),
            )
        });
        self.rank_ja_learning(&mut results, &ctx.left);
        let Some(best) = results.first() else {
            self.base = reading;
            self.sync_composition_ja();
            return;
        };
        let baseline = best.surface();
        // 門檻沿用 gating 實驗的訊號：只看最前面兩條原始路徑，詞面相同就
        // 視為沒有備案、一定觸發。這與候選清單的 N-best 分開算。
        // `mean_candidates` 從 top-1 統計結果算，不在等候模型的路徑上補算。
        let needs_neural = self.config.ja_neural.enabled && self.config.ja_neural.runtime.is_some();
        let (statistical_cost, runner_up_cost, mean_candidates) = if needs_neural {
            let (cost, mean) = gate.unwrap_or((best.conversion().cost, 1.0));
            (cost, gap.map(|gap| cost.saturating_add(gap)), mean)
        } else {
            (
                best.conversion().cost,
                results.get(1).map(|result| result.conversion().cost),
                1.0,
            )
        };
        // 統計結果先算好；只有門檻命中且通過驗證才換成神經輸出。
        //
        // 組字中的未知詞 fallback 會讓半截讀音（例如「きょ」）也產生一條
        // 路徑；那不是完整轉換，送模型只會每鍵呼叫一次。半截讀音的判定
        // 放在 `neural::maybe_convert` 內，與逐字変換共用同一條規則。
        let decision = neural::maybe_convert(
            &self.config.ja_neural,
            &NeuralInput {
                reading: &reading,
                left: &ctx.left,
                right: &ctx.right,
                statistical: &baseline,
                statistical_cost,
                runner_up_cost,
                mean_candidates,
            },
        );
        let replaced = decision.accepted_surface().map(str::to_string);
        self.base = replaced.clone().unwrap_or(baseline);
        self.segments = if replaced.is_some() {
            // 神經輸出沒有詞邊界；整段當一個文節，避免片段範圍對不上。
            vec![Segment::new(0, self.base.len(), Lang::Ja, Script::Kana)]
        } else {
            best.conversion().segments
        };
        let pos_table = decoder.pos_table();
        self.candidates = results
            .iter()
            .map(|result| {
                let conversion = result.conversion();
                let mut candidate = Candidate::new(
                    conversion.surface,
                    reading.clone(),
                    conversion.cost,
                    CandidateSource::Lexicon,
                );
                if let (Some(id), Some(table)) = (result.pos_id(), pos_table.clone()) {
                    candidate = candidate.with_pos(id, table);
                }
                candidate
            })
            .collect();
        if let (Some(surface), Some(candidate)) = (replaced, self.candidates.first_mut()) {
            // 顯示與提交都走候選清單，第一名必須跟著換；字面已不是原本的
            // 詞條，詞性一併清掉，避免顯示上一筆的詞性。
            candidate.surface = surface;
            candidate.clear_pos();
        }
        self.last_ja_results = results;
        self.sync_composition_ja();
    }

    /// 弱負向訊號：選了非學習候選時，對前面被跳過的學習詞記一次跳過。
    ///
    /// 選到的詞若是學過的，等同使用者把它選回來，清掉跳過計數（自我修復）。
    /// 跳過只降權，不刪除；累積到 [`crate::personal::SKIP_LIMIT`] 次才不再
    /// 自動排第一。
    pub(super) fn note_candidate_skips(&mut self, index: usize, chosen: &str) {
        self.personal.clear_skip(chosen);
        if self.personal.is_learned(chosen) {
            return;
        }
        let skipped: Vec<String> = self
            .candidates
            .iter()
            .take(index)
            .filter(|candidate| self.personal.is_learned(&candidate.surface))
            .map(|candidate| candidate.surface.clone())
            .collect();
        for word in skipped {
            self.personal.note_skip(&word);
        }
    }

    /// 依共用的日文學習資料重排 N-best；學習關閉時維持統計順序。
    ///
    /// 排序之後套用弱負向訊號：被反覆跳過的整句不再自動排第一，但仍在候選
    /// 清單裡。加上 [`crate::personal::SKIP_LIMIT`] 的判定在個人資料層，這裡
    /// 只負責把統計排序的第一名往後移一格。
    pub(super) fn rank_ja_learning(&mut self, results: &mut [JaResult], left: &str) {
        if !self.config.learning {
            return;
        }
        let learning = ja_personal::UnifiedLearning::new(&self.personal);
        learning.rank_results(results, left);
        if results.len() > 1 {
            let top = results[0].surface().to_string();
            if self.personal.skip_count(&top) >= crate::personal::SKIP_LIMIT {
                results.swap(0, 1);
            }
        }
    }

    /// 逐字変換開啟時的日文重建：把讀音送進控制器，依決定畫出結果。
    pub(super) fn rebuild_ja_live(&mut self, units: &[Unit]) {
        let reading: String = units.iter().map(Unit::as_str).collect();
        // 先讓 `Composition` 判定這次編輯保留了哪些 User 片段，再推給控制器；
        // 控制器的清單只是快取，不自己決定所有權。
        let previous = self.composition.reading(crate::composition::Lang::Ja);
        let common = super::composition::common_prefix_units(&previous, &reading);
        self.composition
            .retain_units_through(crate::composition::Lang::Ja, common);
        self.sync_ja_users();
        let plan = match &mut self.live {
            Some(live) => live.input(units, &JaContext::default()),
            None => return,
        };
        let reading_changed = previous != reading;
        self.apply_live_plan(&plan, units, &reading);
        if reading_changed {
            self.ja_cursor = self.base.len();
        }
    }

    /// 套用逐字変換的決定。
    ///
    /// 有新結果時整段換成新轉換；只有 draft 時顯示前次結果再加上尚未轉換的
    /// かな，讓畫面不會先退回かな。debounce 等待中的新かな因此仍可看見。
    pub(super) fn apply_live_plan(&mut self, plan: &LivePlan, units: &[Unit], reading: &str) {
        self.live_plan = Some(plan.clone());
        // 只有讀音真的改變（打字、倒退）才把游標移到結尾；單純左右移動觸發
        // 的重畫要保留游標，否則片段選字永遠選到最後一段。
        let previous_cursor = self.ja_cursor;
        let mut base = String::new();
        let mut segments = Vec::new();
        let mut display = reading.to_string();
        if let Some(result) = &plan.result {
            base.push_str(&result.surface());
            segments = result.conversion().segments;
            let covered: usize = result
                .clauses
                .iter()
                .map(|clause| clause.reading.chars().count())
                .sum();
            if covered < units.len() {
                let tail: String = units[covered..].iter().map(Unit::as_str).collect();
                let start = base.len();
                base.push_str(&tail);
                segments.push(Segment::new(start, base.len(), Lang::Ja, Script::Kana));
            }
            display = base.clone();
        } else {
            base = reading.to_string();
        }
        self.base = base;
        self.segments = segments;
        self.ja_cursor = previous_cursor.min(self.base.len());

        let mut candidates = Vec::new();
        let mut results = Vec::new();
        if plan.convert {
            if let Some(decoder) = self.config.ja.clone() {
                let chars: Vec<char> = reading.chars().collect();
                results = decoder.results(&chars, &JaContext::default(), self.max_candidates());
                let left = self.committed.clone();
                let before = results.first().map(|result| result.surface());
                self.rank_ja_learning(&mut results, &left);
                // 學習真的翻掉第一名時，改用學習後的首選；否則保留這次
                // 逐字／神經轉換的結果，不要被統計順序蓋掉。
                if results.first().map(|result| result.surface()) != before {
                    if let Some(best) = results.first() {
                        display = best.surface();
                    }
                }
                let pos_table = decoder.pos_table();
                candidates = results
                    .iter()
                    .map(|result| {
                        let conversion = result.conversion();
                        let mut candidate = Candidate::new(
                            conversion.surface,
                            reading.to_string(),
                            conversion.cost,
                            CandidateSource::Lexicon,
                        );
                        if let (Some(id), Some(table)) = (result.pos_id(), pos_table.clone()) {
                            candidate = candidate.with_pos(id, table);
                        }
                        candidate
                    })
                    .collect();
            }
        }
        // 單純重畫（例如左右移動游標）沒有重新轉換，保留上一次的候選清單；
        // 否則片段候選會在移動游標時被縮成一項，選不到第二個候選。
        // 剛用 Space 開好片段候選窗時也一樣，不能讓整句候選蓋掉片段候選。
        let keep_open_candidates = self.candidate_window && !self.candidates.is_empty();
        if (plan.convert || self.candidates.is_empty()) && !keep_open_candidates {
            self.last_ja_results = results;
            // 畫面上顯示的是高亮候選；第一個候選必須與逐字変換的結果一致。
            if candidates
                .first()
                .map(|candidate| candidate.surface.as_str())
                != Some(display.as_str())
            {
                candidates.insert(
                    0,
                    Candidate::new(display, reading.to_string(), 0, CandidateSource::Lexicon),
                );
            }
            self.candidates = candidates;
        }
        self.sync_composition_ja();
    }

    /// 把解碼片段轉成輸出字串上的位元組範圍。
    pub(super) fn segments_of(
        segs: &[crate::decode::Seg],
        mode: Lang,
        script: Script,
    ) -> Vec<Segment> {
        let mut at = 0usize;
        segs.iter()
            .map(|seg| {
                let start = at;
                at += seg.word.len();
                Segment::new(start, at, mode, script)
            })
            .collect()
    }

    /// 把一整串讀音直接轉成最佳字面；給空白鍵提交與外部驅動使用。
    pub(super) fn convert_reading(&self, reading: &str) -> Option<String> {
        let syllables = crate::zh::syllable::parse_reading(reading);
        if syllables.is_empty() {
            return None;
        }
        let units: Vec<&str> = syllables.iter().map(|s| s.as_str()).collect();
        let lattice = Lattice::build(&units, &self.lexicon, &self.ctx).ok()?;
        let scorer = zh_scorer(&self.ctx, self.zh_lm.as_deref());
        let decoder = zh_decoder(&self.lexicon, &self.ctx, scorer.as_ref());
        let (segs, _) = decoder.best(&lattice, &Locks::new(), false);
        Some(decoder.conversion(&lattice, &segs, Lang::Zh).surface)
    }
}

/// 有 bigram 時建立內插評分器；`lm.bin` 沒有 bigram 時回傳 `None`，
/// 解碼完全走原本的 unigram 成本。
pub(super) fn zh_scorer<'a>(
    ctx: &'a ScoreContext,
    lm: Option<&'a crate::score::ZhLm>,
) -> Option<crate::score::LmScorer<'a>> {
    lm.filter(|lm| lm.bigrams() > 0)
        .map(|lm| crate::score::LmScorer::new(ctx, lm))
}

/// 建立注音解碼器；有 bigram 時接上評分器並套用 beam 上限。
pub(super) fn zh_decoder<'a>(
    lex: &'a InMemoryLexicon,
    ctx: &'a ScoreContext,
    scorer: Option<&'a crate::score::LmScorer<'a>>,
) -> Decoder<'a> {
    let mut decoder = match scorer {
        Some(scorer) => Decoder::new(lex, ctx).with_scorer(scorer),
        None => Decoder::new(lex, ctx),
    };
    if scorer.is_some() {
        decoder.beam = crate::decode::DEFAULT_BEAM;
    }
    decoder
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::LexEntry;

    /// 基礎詞庫；另外補 12 個同讀音單字用來翻頁。
    fn lexicon() -> InMemoryLexicon {
        let mut lex = InMemoryLexicon::new();
        for (surface, reading) in [
            ("我", "ㄨㄛˇ"),
            ("想", "ㄒㄧㄤˇ"),
            ("吃", "ㄔ"),
            ("風", "ㄈㄥ"),
            ("景", "ㄐㄧㄥˇ"),
            ("風景", "ㄈㄥ ㄐㄧㄥˇ"),
            ("麻", "ㄇㄚˊ"),
            ("麻煩", "ㄇㄚˊ ㄈㄢˊ"),
            ("台", "ㄊㄞˊ"),
            ("北", "ㄅㄟˇ"),
            ("台北", "ㄊㄞˊ ㄅㄟˇ"),
        ] {
            lex.push(LexEntry::new(surface, reading, 0, 0));
        }
        for (index, surface) in [
            "甲", "乙", "丙", "丁", "戊", "己", "庚", "辛", "壬", "癸", "子", "丑",
        ]
        .iter()
        .enumerate()
        {
            lex.push(LexEntry::new(*surface, "ㄕˋ", index as i32, 0));
        }
        // 換字測試需要成本較高的同音字，排序才會固定在「想」後面。
        lex.push(LexEntry::new("像", "ㄒㄧㄤˇ", 20, 0));
        lex
    }

    fn session() -> Session {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
        config.max_candidates = 20;
        Session::new(config)
    }

    /// 按 k 把反白移到指定字面；候選順序改變時測試不必硬寫索引。
    fn highlight_surface(session: &mut Session, surface: &str) -> Snapshot {
        let target = session
            .candidates()
            .iter()
            .position(|candidate| candidate.surface == surface)
            .unwrap_or_else(|| panic!("找不到候選 {surface}"));
        while session.snapshot().highlighted < target {
            session.press(Key::char('k', false));
        }
        session.snapshot()
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

    /// 對照 `TestDownEntersSelectionModeThenArrowsPage` 的第一段：
    /// 空白只顯示候選，下鍵才進入選取模式，第一下不移動反白。
    #[test]
    fn test_down_enters_selection_mode_without_moving() {
        let mut s = session();
        type_syllable(&mut s, "ㄕˋ");
        let shown = s.press(Key::new(KeyCode::Space, None, false));
        assert!(!shown.candidates.is_empty(), "空白應開候選窗");
        assert!(!shown.navigating, "空白只顯示候選，不進入選取模式");
        assert_eq!(shown.highlighted, 0);

        let down = s.press(Key::new(KeyCode::Down, None, false));
        assert!(down.navigating, "下鍵進入選取模式");
        assert_eq!(down.highlighted, 0, "第一下不移動反白");
        assert!(!down.candidates.is_empty(), "候選窗仍顯示");

        let next = s.press(Key::new(KeyCode::Down, None, false));
        assert_eq!(next.highlighted, 1, "第二下換下一項");
        let back = s.press(Key::new(KeyCode::Up, None, false));
        assert_eq!(back.highlighted, 0, "上鍵換回上一項");
    }

    /// 對照 `TestDownEntersSelectionModeThenArrowsPage` 的翻頁段落。
    #[test]
    fn test_page_keys_switch_pages_and_wrap() {
        let mut s = session();
        type_syllable(&mut s, "ㄕˋ");
        s.press(Key::new(KeyCode::Space, None, false));
        s.press(Key::new(KeyCode::Down, None, false));

        let down = s.press(Key::new(KeyCode::PageDown, None, false));
        assert_eq!(down.highlighted, PAGE_SIZE, "PageDown 到第二頁第一項");
        assert!(down.navigating);

        let wrap = s.press(Key::new(KeyCode::PageDown, None, false));
        assert_eq!(wrap.highlighted, 0, "最後一頁再往後回到第一頁");

        let back = s.press(Key::new(KeyCode::PageUp, None, false));
        assert_eq!(back.highlighted, PAGE_SIZE, "PageUp 從第一頁往前到最後一頁");
    }

    /// 對照 `TestHomeEndDependOnSelectionMode`：選取模式的 Home／End 是
    /// 第一個／最後一個候選；只是顯示時不動作。
    #[test]
    fn test_home_end_move_only_in_selection_mode() {
        let mut s = session();
        type_syllable(&mut s, "ㄕˋ");
        s.press(Key::new(KeyCode::Space, None, false));

        let home = s.press(Key::new(KeyCode::Home, None, false));
        assert!(!home.navigating, "只是顯示候選時 Home 不進入選取模式");
        assert_eq!(home.highlighted, 0);

        s.press(Key::new(KeyCode::Down, None, false));
        let end = s.press(Key::new(KeyCode::End, None, false));
        assert_eq!(
            end.highlighted,
            end.candidates.len() - 1,
            "End 到清單最後一項"
        );
        let home = s.press(Key::new(KeyCode::Home, None, false));
        assert_eq!(home.highlighted, 0, "Home 回到第一項");
    }

    /// 對照 `TestDownEntersSelectionMode...` 的「打字離開選取模式」。
    #[test]
    fn test_typing_leaves_selection_mode() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        s.press(Key::new(KeyCode::Down, None, false));
        assert!(s.snapshot().navigating);
        type_syllable(&mut s, "ㄈㄥ");
        let snap = s.snapshot();
        assert!(!snap.navigating, "打字後離開選取模式");
        assert_eq!(snap.preedit, "風我");
    }

    /// 對照 `TestEscLeavesSelectionMode`：Esc 關窗、保留組字。
    #[test]
    fn test_escape_closes_candidate_window_and_keeps_composition() {
        let mut s = session();
        type_syllable(&mut s, "ㄕˋ");
        s.press(Key::new(KeyCode::Down, None, false));
        let snap = s.press(Key::new(KeyCode::Escape, None, false));
        assert!(!snap.navigating, "Esc 離開選取模式");
        assert_eq!(snap.preedit, "甲", "Esc 保留已組好的字");
        // 關窗後數字鍵回到注音路徑，不再選字；游標已在字前。
        let typed = s.press(Key::char('2', false));
        assert_eq!(typed.preedit, "ㄉ甲", "關窗後數字是注音鍵");
    }

    /// Esc 先清未完成的注音，已完成的音節留著；再按一次才整段清掉。
    #[test]
    fn test_escape_clears_pending_syllable_before_composition() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        for ch in "vu".chars() {
            s.press(Key::char(ch, false));
        }
        assert_eq!(s.snapshot().preedit, "我ㄒㄧ");
        let first = s.press(Key::new(KeyCode::Escape, None, false));
        assert_eq!(first.preedit, "我", "第一次 Esc 只清未完成的注音");
        let second = s.press(Key::new(KeyCode::Escape, None, false));
        assert_eq!(second.preedit, "", "第二次 Esc 清掉整段組字");
    }

    /// 候選窗開著時退格刪游標前一個字；空組字才只關窗。
    #[test]
    fn test_backspace_deletes_before_cursor_and_closes_candidates() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_syllable(&mut s, "ㄔ");
        s.press(Key::new(KeyCode::ArrowLeft, None, false));
        s.press(Key::new(KeyCode::Down, None, false));
        let snap = s.press(Key::new(KeyCode::Backspace, None, false));
        assert!(!snap.navigating, "退格關閉候選");
        assert_eq!(snap.preedit, "吃", "退格刪掉游標前的我");
        let typed = s.press(Key::char('2', false));
        assert_eq!(typed.preedit, "ㄉ吃", "關窗後數字是注音鍵");
    }

    /// Delete 刪游標後一個字；游標在結尾時不動作。
    #[test]
    fn test_delete_removes_after_cursor_or_closes_candidates() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_syllable(&mut s, "ㄔ");
        s.press(Key::new(KeyCode::ArrowLeft, None, false));
        s.press(Key::new(KeyCode::Down, None, false));
        let snap = s.press(Key::new(KeyCode::Delete, None, false));
        assert!(!snap.navigating);
        assert_eq!(snap.preedit, "我", "Delete 刪掉游標後面的吃");
        let typed = s.press(Key::char('2', false));
        assert_eq!(typed.preedit, "我ㄉ", "關窗後數字是注音鍵");

        // 游標在結尾時 Delete 不刪字。
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        let snap = s.press(Key::new(KeyCode::Delete, None, false));
        assert_eq!(snap.preedit, "我");
    }

    /// 空白鍵只插入空格，不再開候選窗或選第一個。
    #[test]
    fn test_space_opens_candidates_after_complete_syllable() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        let snap = s.press(Key::new(KeyCode::Space, None, false));
        assert!(!snap.navigating, "空白不進入選取模式");
        assert_eq!(snap.preedit, "我 ", "空白只插入空格");
        let snap = s.press(Key::new(KeyCode::Space, None, false));
        assert_eq!(snap.preedit, "我  ", "再按一次也是空格");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "我  ");
    }

    /// 「空白只插空白」是唯一行為；舊設定讀到也一樣。
    #[test]
    fn test_space_inserts_space_keeps_literal_behavior() {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
        config.style.space_inserts_space = true;
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄨㄛˇ");
        let snap = s.press(Key::new(KeyCode::Space, None, false));
        assert!(!snap.navigating, "不進入選取模式");
        assert_eq!(snap.preedit, "我 ", "空白插入空格");
    }

    /// 快照的 suggestion 是 Tab 會補上的尾綴。
    #[test]
    fn test_snapshot_suggestion_is_completion_tail() {
        let mut s = session();
        type_syllable(&mut s, "ㄇㄚˊ");
        let snap = s.snapshot();
        assert_eq!(snap.suggestion.as_deref(), Some("煩"), "{:?}", snap.preedit);
    }

    /// 個人詞彙的候選標上 `personal`，不影響排序。
    #[test]
    fn test_snapshot_label_marks_personal_vocab() {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
        config.personal.add_vocab("窩", "ㄨㄛˇ");
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄨㄛˇ");
        let at = s
            .candidates()
            .iter()
            .position(|candidate| candidate.surface == "窩")
            .expect("自訂詞應在候選中");
        let snap = s.select(at);
        assert_eq!(snap.label.as_deref(), Some("personal"));
    }

    /// 沒有標示來源的一般候選不捏造 label。
    #[test]
    fn test_snapshot_label_is_none_without_evidence() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        assert_eq!(s.snapshot().label, None);
    }

    /// 關聯標示的左文只取尾巴的連續漢字；沒有 bigram 模型時不捏造 related。
    #[test]
    fn test_label_context_uses_trailing_han_only() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "我");
        assert_eq!(s.committed_context_word(), Some("我"));

        s.remember_committed("，");
        assert_eq!(s.committed_context_word(), None, "標點不是鄰詞");

        s.remember_committed("一二三四五六");
        assert_eq!(
            s.committed_context_word(),
            Some("三四五六"),
            "只取最後四個漢字"
        );

        type_syllable(&mut s, "ㄒㄧㄤˇ");
        assert_ne!(s.snapshot().label.as_deref(), Some("related"));
    }

    /// 片段必須覆蓋整個 preedit，不能留下缺口。
    fn assert_segments_cover(snap: &Snapshot) {
        let joined: String = snap
            .segments
            .iter()
            .map(|segment| &snap.preedit[segment.start..segment.end])
            .collect();
        assert_eq!(joined, snap.preedit, "片段要覆蓋整個組字字串");
    }

    /// 游標移到中間插入新音節，結果與直接打完整字串相同。
    #[test]
    fn test_insert_syllable_at_cursor_matches_direct_typing() {
        let mut direct = session();
        type_syllable(&mut direct, "ㄊㄞˊㄨㄛˇㄅㄟˇ");
        let want = direct.snapshot().preedit;
        assert_eq!(want, "台我北");

        let mut edited = session();
        type_syllable(&mut edited, "ㄊㄞˊㄅㄟˇ");
        assert_eq!(edited.snapshot().preedit, "台北");
        let moved = edited.press(Key::new(KeyCode::ArrowLeft, None, false));
        assert_eq!(moved.cursor_rune, 1, "游標移到台與北之間");

        type_syllable(&mut edited, "ㄨㄛˇ");
        let snap = edited.snapshot();
        assert_eq!(snap.preedit, want, "插入後與直接打相同");
        assert_eq!(snap.cursor_rune, 2, "游標留在插入的音節後面");
        assert_segments_cover(&snap);

        edited.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(edited.commit(), want);
    }

    /// 中間 Backspace 與 Delete 各自刪對位置，重建後片段仍連續。
    #[test]
    fn test_backspace_and_delete_in_middle_keep_segments_whole() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_syllable(&mut s, "ㄒㄧㄤˇ");
        type_syllable(&mut s, "ㄔ");
        s.press(Key::new(KeyCode::ArrowLeft, None, false));
        let snap = s.press(Key::new(KeyCode::ArrowLeft, None, false));
        assert_eq!(snap.cursor_rune, 1, "游標停在想之前");
        assert_eq!(snap.preedit, "我想吃");

        let snap = s.press(Key::new(KeyCode::Backspace, None, false));
        assert_eq!(snap.preedit, "想吃", "Backspace 刪游標前的我");
        assert_segments_cover(&snap);

        let snap = s.press(Key::new(KeyCode::Delete, None, false));
        assert_eq!(snap.preedit, "吃", "Delete 刪游標後的想");
        assert_segments_cover(&snap);
    }

    /// 調整詞長後候選清單改變，片段仍覆蓋整個 preedit。
    #[test]
    fn test_resize_segment_changes_candidates_and_keeps_segments() {
        let mut s = session();
        type_syllable(&mut s, "ㄊㄞˊ");
        type_syllable(&mut s, "ㄅㄟˇ");
        s.press(Key::new(KeyCode::Home, None, false));
        let whole = s.snapshot();
        assert_eq!(whole.preedit, "台北");
        assert_eq!(whole.segments.len(), 1, "兩音節合成一個片段");

        let shorter = s.press(Key::new(KeyCode::ArrowLeft, None, true));
        assert_eq!(shorter.preedit, "台北", "換詞長只換候選，不改整段顯示");
        assert_segments_cover(&shorter);
        let short_surfaces: Vec<String> = shorter
            .candidates
            .iter()
            .map(|candidate| candidate.surface.clone())
            .collect();
        assert!(short_surfaces.contains(&"台".to_string()));

        let longer = s.press(Key::new(KeyCode::ArrowRight, None, true));
        assert_segments_cover(&longer);
        let long_surfaces: Vec<String> = longer
            .candidates
            .iter()
            .map(|candidate| candidate.surface.clone())
            .collect();
        assert_ne!(short_surfaces, long_surfaces, "詞長改變候選清單");
        assert!(long_surfaces.contains(&"台北".to_string()));
    }

    /// j／k 只換游標所在片段，其他片段保持原樣。
    #[test]
    fn test_jk_changes_only_cursor_segment() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_syllable(&mut s, "ㄒㄧㄤˇ");
        type_syllable(&mut s, "ㄔ");
        s.press(Key::new(KeyCode::ArrowLeft, None, false));
        s.press(Key::new(KeyCode::ArrowLeft, None, false));
        assert_eq!(s.snapshot().cursor_rune, 1);

        let opened = s.press(Key::new(KeyCode::Down, None, false));
        let target = opened
            .candidates
            .iter()
            .position(|candidate| candidate.surface == "像")
            .expect("像 應在候選中");
        let mut moved = opened;
        for _ in 0..target {
            moved = s.press(Key::char('k', false));
        }
        assert_eq!(moved.highlighted, target, "k 換下一個候選");
        assert_eq!(moved.candidates[target].surface, "像");

        let applied = s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(applied.preedit, "我像吃", "只替換游標所在片段");
        assert_eq!(applied.preedit.chars().next(), Some('我'));
        assert_eq!(applied.preedit.chars().last(), Some('吃'));
    }

    /// Enter 換字後直接關窗；組字保留，下一次 Enter 才送出整段。
    #[test]
    fn test_keep_candidates_after_segment_change_until_escape() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_syllable(&mut s, "ㄒㄧㄤˇ");
        type_syllable(&mut s, "ㄔ");
        s.press(Key::new(KeyCode::ArrowLeft, None, false));
        s.press(Key::new(KeyCode::ArrowLeft, None, false));
        s.press(Key::new(KeyCode::Down, None, false));
        highlight_surface(&mut s, "像");
        let changed = s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(changed.preedit, "我像吃");
        assert!(!changed.navigating, "換字後候選窗關閉");

        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "我像吃");
    }

    /// 手選片段後繼續輸入，片段保持；在片段內刪字才退回 Auto。
    #[test]
    fn test_zh_user_segment_survives_more_input_and_demotes_on_edit() {
        use crate::composition::Owner;

        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_syllable(&mut s, "ㄒㄧㄤˇ");
        // 游標移到「想」的片段，換成同音字「像」。
        s.press(Key::new(KeyCode::ArrowLeft, None, false));
        s.press(Key::new(KeyCode::Down, None, false));
        highlight_surface(&mut s, "像");
        let changed = s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(changed.preedit, "我像");
        assert_eq!(s.user_marks().len(), 1, "選字要留下 User 標記");
        assert_eq!(s.user_marks()[0].lang, crate::composition::Lang::Zh);
        assert!(
            changed.owners.contains(&Owner::User),
            "快照要標出 User 片段"
        );

        // Enter 已關掉候選窗，游標停在剛選的字後面；再插一個音節，片段保持。
        type_syllable(&mut s, "ㄈㄢˊ");
        let kept = s.snapshot();
        assert!(
            kept.preedit.contains('像'),
            "重轉洗掉手選片段：{}",
            kept.preedit
        );
        assert_eq!(s.user_marks().len(), 1, "插入在後不影響 User 標記");
        assert_eq!(s.user_marks()[0].start, 1, "User 標記不因後面插入而移動");

        // 刪掉 User 片段本身的音節：該片段退回 Auto。
        s.press(Key::new(KeyCode::ArrowLeft, None, false));
        s.press(Key::new(KeyCode::ArrowLeft, None, false));
        let demoted = s.press(Key::new(KeyCode::Delete, None, false));
        assert!(!demoted.preedit.contains('像'), "{:?}", demoted.preedit);
        assert!(s.user_marks().is_empty(), "片段內編輯要解除 User 標記");
    }

    /// `auto_runs` 在 User 片段前後切開，並帶完整的左右文脈。
    #[test]
    fn test_auto_runs_split_around_user_segment_with_context() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_syllable(&mut s, "ㄒㄧㄤˇ");
        type_syllable(&mut s, "ㄔ");
        assert!(s.lock_segment(0));
        let runs = s.auto_runs();
        assert_eq!(runs.len(), 1, "User 片段把前後 Auto 切開");
        assert_eq!(runs[0].reading, "ㄒㄧㄤˇ ㄔ");
        let ctx = s.composition().context_for(&runs[0]);
        assert_eq!(ctx.left, "我");
        assert_eq!(ctx.right, "");

        // 轉換只會送 Auto 區段，User 片段不在任何 run 裡。
        let sent = s.convert_auto_runs();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].1.is_ok(), "{:?}", sent[0].1);
    }

    /// 提交後再輸入，已提交的文字不變。
    #[test]
    fn test_committed_text_stays_after_more_input() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "我");
        type_syllable(&mut s, "ㄒㄧㄤˇ");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "想");
        assert_eq!(s.committed(), "我想");
        assert!(s.user_marks().is_empty(), "提交後不得留下所有權狀態");
    }
}
