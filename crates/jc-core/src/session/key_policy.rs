//! 按鍵政策的固定優先序：候選介面 → lane 專屬 → 全域。
//!
//! `press()` 只把按鍵交給這裡一次，不在每個 handler 重新分派 lane。
//! 目前先收 `space`、`segments_press`、`backspace`、`delete_forward`、
//! `escape` 與 `tab`；其他鍵仍走原本的入口，後續再搬。

use super::*;

/// 一次按鍵在固定優先序下要執行的動作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KeyAction {
    FinishComposition,
    Escape,
    ForgetCandidate,
    DeleteForward,
    Backspace,
    ToggleJapaneseScript,
    JapaneseTransform(crate::ja::live::KanaTransform),
    /// 日文「無変換」：拆回讀音。
    JapaneseUnconvert,
    Space,
    Down,
    Up,
    Page(isize),
    CandidateEdge(bool),
    Tab,
    ResizeSegment(isize),
    MoveCursor(isize),
    Char(char),
    /// 組字中沒有對應動作，按鍵由組字區吃掉。
    Ignore,
}

/// `press()` 是否需要重建畫面。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KeyOutcome {
    Changed,
    /// Tab 在沒有補全可用的字面尾巴上是 no-op；不要重建。
    Unchanged,
}

/// 空白要交給哪一條 lane。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpaceLane {
    Zh,
    Ja,
    Mixed,
}

impl Session {
    /// 注音聲調、聲調選填與孤立字面空白的前置處理。
    ///
    /// 這是按鍵政策的 lane 前置步驟：先讓注音狀態機決定這一鍵是不是聲調，
    /// 才交給候選介面或一般按鍵。回傳 `true` 表示這一鍵已被吃掉。
    pub(super) fn key_preprocess(&mut self, key: &Key) -> bool {
        self.segments_press(key)
    }

    /// 依固定優先序處理一鍵：候選介面 → lane 專屬 → 全域。
    ///
    /// `press()` 只呼叫這一個入口；回傳 `Unchanged` 表示這一鍵沒有改動狀態，
    /// 呼叫端要直接回目前快照，不要重建。
    pub(super) fn key_policy(&mut self, key: Key) -> KeyOutcome {
        match self.key_action(&key) {
            KeyAction::FinishComposition => self.finish_composition(),
            KeyAction::Escape => self.escape(),
            KeyAction::ForgetCandidate => self.forget_candidate(),
            KeyAction::DeleteForward => self.delete_forward(),
            KeyAction::Backspace => self.backspace(),
            KeyAction::ToggleJapaneseScript => {
                self.ja_toggle_script();
            }
            KeyAction::JapaneseTransform(kind) => {
                self.ja_transform(kind);
            }
            KeyAction::JapaneseUnconvert => {
                self.ja_unconvert();
            }
            KeyAction::Space => self.space(),
            KeyAction::Down => self.down(),
            KeyAction::Up => self.up(),
            KeyAction::Page(delta) => self.page_step(delta),
            KeyAction::CandidateEdge(end) => self.candidate_edge(end),
            KeyAction::Tab => return self.tab(),
            KeyAction::ResizeSegment(delta) => self.resize_segment(delta),
            KeyAction::MoveCursor(delta) => self.navigate(delta),
            KeyAction::Char(ch) => self.press_char(ch, key.caps),
            KeyAction::Ignore => {}
        }
        KeyOutcome::Changed
    }

    /// 按鍵決策本體；先候選介面，再 lane 專屬，最後全域。
    fn key_action(&self, key: &Key) -> KeyAction {
        if let Some(action) = self.candidate_key(key) {
            return action;
        }
        if let Some(action) = self.lane_key(key) {
            return action;
        }
        self.global_key(key)
    }

    /// 第一層：候選介面。Enter／Esc／Tab、方向鍵、換頁與 Shift＋Delete／
    /// Backspace 都先由候選狀態決定；候選窗沒開時 handler 自己會退回一般行為。
    fn candidate_key(&self, key: &Key) -> Option<KeyAction> {
        match key.code {
            KeyCode::Enter => Some(KeyAction::FinishComposition),
            KeyCode::Escape => Some(KeyAction::Escape),
            KeyCode::Tab => Some(KeyAction::Tab),
            KeyCode::Down => Some(KeyAction::Down),
            KeyCode::Up => Some(KeyAction::Up),
            KeyCode::PageDown => Some(KeyAction::Page(1)),
            KeyCode::PageUp => Some(KeyAction::Page(-1)),
            KeyCode::Home => Some(KeyAction::CandidateEdge(false)),
            KeyCode::End => Some(KeyAction::CandidateEdge(true)),
            // 沒有候選窗時方向鍵只移游標；有窗時由 `navigate` 依 lane 決定
            // 換候選或移游標，決策留在同一條路徑。
            KeyCode::ArrowRight if !key.shift => Some(KeyAction::MoveCursor(1)),
            KeyCode::ArrowLeft if !key.shift => Some(KeyAction::MoveCursor(-1)),
            // 候選窗的反悔鍵；沒有候選窗時 Shift＋Delete／Backspace 仍照
            // 一般刪除處理，所以不在這裡攔。
            KeyCode::Delete if key.shift && self.candidate_window => {
                Some(KeyAction::ForgetCandidate)
            }
            KeyCode::Backspace if key.shift && self.candidate_window => {
                Some(KeyAction::ForgetCandidate)
            }
            _ => None,
        }
    }

    /// 第二層：lane 專屬。注音的聲調已在 `key_preprocess()` 先行處理；這裡
    /// 只分派空白、刪除、日文轉換鍵與一般字元。
    fn lane_key(&self, key: &Key) -> Option<KeyAction> {
        match key.code {
            KeyCode::Delete => Some(KeyAction::DeleteForward),
            KeyCode::Backspace => Some(KeyAction::Backspace),
            KeyCode::F(n) if KeyCode::F(n) == self.config.ja_toggle_key => {
                Some(KeyAction::ToggleJapaneseScript)
            }
            KeyCode::F(6) => Some(KeyAction::JapaneseTransform(
                crate::ja::live::KanaTransform::Hiragana,
            )),
            KeyCode::F(8) => Some(KeyAction::JapaneseTransform(
                crate::ja::live::KanaTransform::HalfwidthKana,
            )),
            KeyCode::F(9) => Some(KeyAction::JapaneseTransform(
                crate::ja::live::KanaTransform::FullwidthLatin,
            )),
            KeyCode::F(10) => Some(KeyAction::JapaneseTransform(
                crate::ja::live::KanaTransform::HalfwidthLatin,
            )),
            // 変換開游標候選；無変換拆回讀音。
            KeyCode::Convert => Some(KeyAction::Down),
            KeyCode::NonConvert => Some(KeyAction::JapaneseUnconvert),
            KeyCode::Space => Some(KeyAction::Space),
            KeyCode::ArrowRight if key.shift => Some(KeyAction::ResizeSegment(1)),
            KeyCode::ArrowLeft if key.shift => Some(KeyAction::ResizeSegment(-1)),
            KeyCode::Char(ch) => Some(KeyAction::Char(ch)),
            _ => None,
        }
    }

    /// 第三層：全域。候選介面與 lane 都沒有對應時，只留下忽略。
    fn global_key(&self, _key: &Key) -> KeyAction {
        KeyAction::Ignore
    }

    /// `Tab`：先看字面尾巴，再看選取模式，最後才是補全／下一個候選。
    fn tab(&mut self) -> KeyOutcome {
        if !self.literal_tail.is_empty() && self.suggestion.is_none() {
            // 英文模式是 raw input，沒有 suggestion：Tab 沒有可補全的東西，
            // 整鍵 no-op，連 rebuild 都不做。
            return KeyOutcome::Unchanged;
        }
        if self.navigating && !self.candidates.is_empty() {
            // 選取模式中的 Tab 換下一個候選；候選選取優先於補全。
            self.move_highlight(1);
            return KeyOutcome::Changed;
        }
        let accepted = self.config.mode == Mode::Mixed
            && !self.in_zh_lane()
            && !self.in_ja_lane()
            && self.config.english_autocomplete
            && self.ascii.accept_suggestion();
        if !accepted && !self.accept_suggestion() {
            self.move_highlight(1);
            if self.config.mode == Mode::Mixed {
                self.navigating = true;
            }
        }
        KeyOutcome::Changed
    }

    /// 空白：三個 lane 共用同一個入口，只差在 `SpaceLane`。
    pub(super) fn space(&mut self) {
        match self.space_lane() {
            SpaceLane::Zh => self.space_zh(),
            SpaceLane::Ja => self.space_ja(),
            SpaceLane::Mixed => self.space_mixed(),
        }
    }

    fn space_lane(&self) -> SpaceLane {
        if self.in_zh_lane() {
            SpaceLane::Zh
        } else if self.in_ja_lane() {
            SpaceLane::Ja
        } else {
            SpaceLane::Mixed
        }
    }

    /// 注音路徑（純注音或 Mixed 的純注音組字）：未完成的注音先補完；
    /// 已完成音節時空白就是字面空格，不再開候選窗或選第一個。
    fn space_zh(&mut self) {
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
    }

    /// 日文路徑：空白只插入字面空格；日文候選改由 ↓ 開啟。
    fn space_ja(&mut self) {
        self.candidate_window = false;
        self.candidate_expanded = false;
        self.navigating = false;
        self.literal_tail.push(' ');
    }

    /// 其餘混打路徑：空白只插入空格。
    fn space_mixed(&mut self) {
        self.candidate_window = false;
        self.candidate_expanded = false;
        self.navigating = false;
        self.keep_candidates = false;
        self.cursor_rune = Self::MIXED_CURSOR_END;
        // 待定按鍵是空的：這個空白是純字面，記進 `literal_spans`，重建時
        // 才不會被 `visible_plans` 當成片段停頓吃掉（`ji3 ji3 ji3`）。
        // 待定按鍵非空時空白可能被當成一聲／音節分隔，照原管線處理。
        let literal_space = self.ascii.pending().is_empty();
        let start = self.ascii.typed().len();
        self.ascii.space(&self.lexicon);
        if literal_space {
            let end = self.ascii.typed().len();
            if start < end {
                self.literal_spans.push(LiteralSpan::new(start, end));
            }
        }
    }

    /// `Backspace`：先判斷字面尾巴，再交給 lane 的刪除單位。
    pub(super) fn backspace(&mut self) {
        self.candidate_window = false;
        self.navigating = false;
        self.keep_candidates = false;
        self.edit = None;
        self.highlighted = 0;
        match self.backspace_lane() {
            BackspaceLane::LiteralTail => {
                self.literal_tail.pop();
            }
            BackspaceLane::Ja => {
                self.romaji.backspace();
                self.sync_ja_run_keys();
                self.leave_empty_japanese_run();
            }
            BackspaceLane::Zh => self.backspace_zh(),
            BackspaceLane::Mixed => {
                // 孤立聲調鍵先留在字面；沒有混打按鍵可退時退回那一個字面。
                if !self.backspace_mixed() && !self.literal.is_empty() {
                    self.literal.pop();
                }
            }
        }
    }

    fn backspace_lane(&self) -> BackspaceLane {
        if !self.literal_tail.is_empty() {
            BackspaceLane::LiteralTail
        } else if self.in_ja_lane() {
            BackspaceLane::Ja
        } else if self.zh_lane {
            BackspaceLane::Zh
        } else {
            BackspaceLane::Mixed
        }
    }

    /// 注音路徑的 Backspace：先退未完成的注音，再退已完成音節與字面。
    fn backspace_zh(&mut self) {
        if !self.zh.pending().is_empty() {
            // 還沒完成的注音先退一格；退完才動已完成的音節。
            self.zh.press(Key::new(KeyCode::Backspace, None, false));
            self.zh_pending_keys.pop();
            self.zh_pending_caps.pop();
        } else if self.zh_cell() > 0 {
            self.remove_syllable(self.zh_cell() - 1);
        } else if !self.literal.is_empty() {
            self.literal.pop();
        }
    }

    /// `Delete`：刪除游標後一個音節；沒有可刪的音節時只關候選窗。
    pub(super) fn delete_forward(&mut self) {
        self.candidate_window = false;
        self.candidate_expanded = false;
        self.navigating = false;
        self.keep_candidates = false;
        self.edit = None;
        self.highlighted = 0;
        match self.delete_lane() {
            DeleteLane::Mixed => self.delete_mixed_forward(),
            DeleteLane::Zh => {
                if self.zh.pending().is_empty() {
                    self.remove_syllable(self.zh_cell());
                }
            }
            // 既有缺口：日文 lane 的 Delete 沒有刪除動作，只關候選窗。
            DeleteLane::Ja => {}
        }
    }

    fn delete_lane(&self) -> DeleteLane {
        if !self.in_zh_lane() && !self.in_ja_lane() {
            DeleteLane::Mixed
        } else if self.in_zh_lane() {
            DeleteLane::Zh
        } else {
            DeleteLane::Ja
        }
    }

    fn delete_mixed_forward(&mut self) {
        let text = self.base_with_highlight();
        let runes = text.chars().count();
        let at = self.mixed_rune().min(runes);
        if at >= runes {
            return;
        }
        let saved = self.mixed_rune();
        self.set_mixed_rune(at);
        let target = self.mixed_target_at_cursor();
        self.set_mixed_rune(saved);
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
                self.set_mixed_rune(at.min(len));
                return;
            }
        }
        if let Some(index) = self.mixed_segment_index_at_rune(at) {
            self.remove_mixed_segment(index);
        }
    }
}

impl Session {
    /// `Escape` 的固定層級：字面尾巴 → 候選窗 → 未完成注音 → 整段組字。
    pub(super) fn escape(&mut self) {
        match self.escape_layer() {
            EscapeLayer::LiteralTail => self.clear_composition(),
            EscapeLayer::CandidateWindow => {
                self.candidate_window = false;
                self.navigating = false;
                self.keep_candidates = false;
                self.edit = None;
                self.highlighted = 0;
            }
            EscapeLayer::PendingZhuyin => self.clear_pending_zhuyin(),
            EscapeLayer::Composition => self.clear_composition(),
        }
    }

    fn escape_layer(&self) -> EscapeLayer {
        // 英文模式的 Esc 丟掉整段，包含前面的中文緩衝。
        if !self.literal_tail.is_empty() {
            EscapeLayer::LiteralTail
        } else if self.candidate_window && self.menu.is_none() {
            EscapeLayer::CandidateWindow
        } else if self.in_zh_lane() && !self.zh.pending().is_empty() {
            EscapeLayer::PendingZhuyin
        } else {
            EscapeLayer::Composition
        }
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
        self.zh_pending_caps.clear();
        self.candidate_window = false;
        self.navigating = false;
    }
}

/// `Escape` 要處理的層級。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EscapeLayer {
    LiteralTail,
    CandidateWindow,
    PendingZhuyin,
    Composition,
}

/// `Backspace` 的刪除單位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackspaceLane {
    LiteralTail,
    Ja,
    Zh,
    Mixed,
}

/// `Delete` 的刪除單位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeleteLane {
    Mixed,
    Zh,
    Ja,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session::new(SessionConfig::new(Mode::Mixed))
    }

    fn key(code: KeyCode) -> Key {
        Key::new(code, None, false)
    }

    /// 候選介面優先：Enter／Esc／Tab 與方向鍵都在 lane 之前決定。
    #[test]
    fn key_action_candidate_interface_comes_first() {
        let s = session();
        assert_eq!(
            s.key_action(&key(KeyCode::Enter)),
            KeyAction::FinishComposition
        );
        assert_eq!(s.key_action(&key(KeyCode::Escape)), KeyAction::Escape);
        assert_eq!(s.key_action(&key(KeyCode::Tab)), KeyAction::Tab);
        assert_eq!(
            s.key_action(&key(KeyCode::ArrowRight)),
            KeyAction::MoveCursor(1)
        );
        assert_eq!(s.key_action(&key(KeyCode::Space)), KeyAction::Space);
        assert_eq!(s.key_action(&key(KeyCode::Backspace)), KeyAction::Backspace);
        assert_eq!(
            s.key_action(&key(KeyCode::Delete)),
            KeyAction::DeleteForward
        );
    }

    /// Shift＋方向鍵仍屬 lane 的片段調整，不落到候選換頁。
    #[test]
    fn key_action_shift_arrows_resize_segment() {
        let s = session();
        let mut right = key(KeyCode::ArrowRight);
        right.shift = true;
        assert_eq!(s.key_action(&right), KeyAction::ResizeSegment(1));
        let mut left = key(KeyCode::ArrowLeft);
        left.shift = true;
        assert_eq!(s.key_action(&left), KeyAction::ResizeSegment(-1));
    }

    /// Esc 層級：字面尾巴 → 候選窗 → 注音 pending → 整段。
    #[test]
    fn escape_layer_follows_fixed_priority() {
        let mut s = session();
        assert_eq!(s.escape_layer(), EscapeLayer::Composition);
        s.candidate_window = true;
        assert_eq!(s.escape_layer(), EscapeLayer::CandidateWindow);
        s.literal_tail.push('a');
        assert_eq!(s.escape_layer(), EscapeLayer::LiteralTail);
    }

    /// 刪除單位由 lane 決定；日文目前沒有 Delete 動作（既有缺口）。
    #[test]
    fn delete_lane_keeps_japanese_gap() {
        let mut s = session();
        assert_eq!(s.backspace_lane(), BackspaceLane::Mixed);
        assert_eq!(s.delete_lane(), DeleteLane::Mixed);
        s.literal_tail.push('a');
        assert_eq!(s.backspace_lane(), BackspaceLane::LiteralTail);
        s.literal_tail.clear();
        s.config.ja_only = true;
        assert_eq!(s.backspace_lane(), BackspaceLane::Ja);
        assert_eq!(s.delete_lane(), DeleteLane::Ja);
    }
}
