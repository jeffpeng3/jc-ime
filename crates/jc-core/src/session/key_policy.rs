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

    /// `Tab`：先看字面尾巴，再看選取模式，最後才是英文補全／注音補全。
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
        // 混打的英文補全走候選窗：Tab 只開窗，不直接套用。沒有補全時
        // 交還應用程式（引擎回 handled=false），不能把 Tab 吃掉。
        // 英文補全關閉時也直接交還，不能落進注音的「換下一項」路徑，
        // 否則會開出一個初始高亮在第二項的候選窗。數字與符號仍走一般
        // 候選窗，例如 `123` 的全形變體。
        if !self.in_zh_lane() && !self.in_ja_lane() {
            let pending = self.ascii.pending();
            let english_run =
                !pending.is_empty() && pending.chars().all(|ch| ch.is_ascii_alphabetic());
            if english_run {
                if !self.config.english_autocomplete
                    || self.ascii.suggestion().is_none()
                    || !self.open_mixed_english_completion()
                {
                    return KeyOutcome::Unchanged;
                }
                return KeyOutcome::Changed;
            }
        }
        if !self.accept_suggestion() {
            self.move_highlight(1);
            self.navigating = true;
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

    /// `Backspace`：刪游標左邊一個可見字元。
    pub(super) fn backspace(&mut self) {
        self.delete_visible(false);
    }

    /// `Delete`：刪游標右邊一個可見字元。
    pub(super) fn delete_forward(&mut self) {
        self.delete_visible(true);
    }

    /// 所有 lane 共用的刪除入口：一次只刪一個可見 rune。
    ///
    /// 先關候選窗，再依游標方向取目標 rune。字面尾巴、注音、日文、混打都
    /// 只拿這一個位置去對應自己的狀態；沒有可刪的字元時不動作。
    fn delete_visible(&mut self, forward: bool) {
        self.candidate_window = false;
        self.candidate_expanded = false;
        self.navigating = false;
        self.keep_candidates = false;
        self.edit = None;
        self.highlighted = 0;

        let at = self.cursor_runes();
        let total = self.rune_count();
        if forward {
            if at >= total {
                return;
            }
        } else if at == 0 {
            return;
        }
        let target = if forward { at } else { at - 1 };
        let after = if forward { at } else { at - 1 };

        let deleted = if self.in_zh_lane() {
            self.delete_zh_visible(target)
        } else if self.in_ja_lane() {
            self.delete_ja_visible(target)
        } else {
            self.delete_mixed_visible(target)
        };
        if deleted {
            self.cursor_rune = after.min(self.rune_count());
        }
    }

    /// 目標 rune 落在字面尾巴時，只從尾巴移除那一個字元。
    ///
    /// 字面尾巴不重算組字，否則英文片段會被重新判成注音或かな；呼叫端在
    /// 刪除後直接回快照。回傳 `false` 表示目標不在尾巴內。
    pub(super) fn delete_literal_tail_rune(&mut self, rune: usize) -> bool {
        let runes = self.literal_tail.chars().count();
        if runes == 0 {
            return false;
        }
        let start = self.rune_count().saturating_sub(runes);
        if rune < start {
            return false;
        }
        let mut chars: Vec<char> = self.literal_tail.chars().collect();
        chars.remove(rune - start);
        self.literal_tail = chars.into_iter().collect();
        true
    }

    /// 注音狀態下刪一個可見字元：字面、未完成注音、已完成音節各對應一格。
    fn delete_zh_visible(&mut self, rune: usize) -> bool {
        let pending = self.pending.chars().count();
        if pending > 0 {
            let start = self.zh_pending_start();
            if rune >= start && rune < start + pending {
                return self.delete_zh_pending_at(rune - start);
            }
        }
        if self.delete_literal_rune(rune) {
            return true;
        }
        self.zh_units_at_rune(rune)
            .is_some_and(|(start, count)| self.remove_syllable_range(start, count))
    }

    /// 未完成注音在顯示字串中的起點。
    ///
    /// 游標在音節結尾時注音接在 `base` 後面；游標停在中間時插在游標所在
    /// 音節格，起點就是游標扣掉注音長度。
    fn zh_pending_start(&self) -> usize {
        let pending = self.pending.chars().count();
        if pending == 0 {
            return 0;
        }
        let base = self.base_rune_count();
        if self.cursor_rune == Self::MIXED_CURSOR_END || self.cursor_rune >= base {
            base
        } else {
            self.cursor_runes().saturating_sub(pending)
        }
    }

    /// 從未完成注音刪一個按鍵，再用剩下的按鍵重建注音狀態。
    fn delete_zh_pending_at(&mut self, index: usize) -> bool {
        let mut keys: Vec<char> = self.zh_pending_keys.chars().collect();
        let mut caps: Vec<char> = self.zh_pending_caps.chars().collect();
        if index >= keys.len() {
            return false;
        }
        keys.remove(index);
        if index < caps.len() {
            caps.remove(index);
        }
        self.zh.reset();
        self.zh_pending_keys.clear();
        self.zh_pending_caps.clear();
        for (at, ch) in keys.into_iter().enumerate() {
            let cap = caps.get(at).copied().unwrap_or('0');
            self.zh.press(Key::char(ch, false));
            self.zh_pending_keys.push(ch);
            self.zh_pending_caps.push(cap);
        }
        // 刪除鍵本身就是「下一個按鍵」；重播剩下的按鍵不該讓上一個被擋的
        // 聲調旗標復活。
        self.zh.clear_tone_blocked();
        true
    }

    /// 注音顯示字元對應的全部音節範圍。
    ///
    /// 一般漢字一個字對應一個音節；若一個可見字元涵蓋多個讀音單位，整個
    /// 範圍一起刪除，不能留下隱藏音節。
    fn zh_units_at_rune(&self, rune: usize) -> Option<(usize, usize)> {
        let mut units = 0usize;
        for segment in self.composition.segments() {
            let chars = segment.chars();
            if rune < segment.span.start + chars {
                if segment.lang != crate::composition::Lang::Zh || segment.units == 0 {
                    return None;
                }
                let offset = rune.saturating_sub(segment.span.start);
                if chars >= segment.units {
                    return Some((units + offset.min(segment.units - 1), 1));
                }
                let start = units + offset * segment.units / chars;
                let end = units + (offset + 1) * segment.units / chars;
                return Some((start, (end - start).max(1)));
            }
            units += segment.units;
        }
        None
    }

    /// 刪除一段連續音節；游標位置由呼叫端設定。
    fn remove_syllable_range(&mut self, start: usize, count: usize) -> bool {
        let mut removed = false;
        for _ in 0..count {
            if !self.remove_syllable(start) {
                break;
            }
            removed = true;
        }
        removed
    }

    /// 日文狀態下刪一個可見字元：讀音單位與未完成前綴都各算一格。
    fn delete_ja_visible(&mut self, rune: usize) -> bool {
        let base_runes = self.base.chars().count();
        let surface = (rune < base_runes).then(|| {
            let mut chars: Vec<char> = self.base.chars().collect();
            chars.remove(rune);
            chars.into_iter().collect::<String>()
        });
        let deleted = if rune < base_runes {
            self.ja_units_at_rune(rune)
                .is_some_and(|(start, count)| self.romaji.delete_units(start, count))
        } else {
            self.romaji.delete_pending_at(rune - base_runes)
        };
        if deleted {
            self.ja_delete_surface = surface;
            self.ja_run_keys = self.romaji.raw_keys();
            if let Some(live) = self.live.as_mut() {
                live.invalidate_for_edit();
            }
            self.leave_empty_japanese_run();
            // 先重建一次，逐字変換才不會在游標已定位後又把游標推到結尾。
            self.rebuild();
        }
        deleted
    }

    /// 可見字元對應的全部讀音單位範圍。
    ///
    /// かな組字時字元與單位一對一。轉換後的漢字可能一個字對應多個讀音
    /// 單位（`私` → `わたし`），必須整段刪掉，不能留下隱藏單位。
    fn ja_units_at_rune(&self, rune: usize) -> Option<(usize, usize)> {
        let reading: String = self.romaji.units().iter().map(Unit::as_str).collect();
        let base = self.base.as_str();
        if base == reading {
            return (rune < base.chars().count()).then_some((rune, 1));
        }
        let mut units = 0usize;
        for segment in self.composition.segments() {
            let chars = segment.chars();
            if rune < segment.span.start + chars {
                if segment.lang != crate::composition::Lang::Ja || segment.units == 0 {
                    return None;
                }
                let offset = rune.saturating_sub(segment.span.start);
                if chars >= segment.units {
                    return Some((units + offset.min(segment.units - 1), 1));
                }
                // 字元比讀音單位少時，把該字元涵蓋的單位平均切給它；
                // 一個字元對多個單位時取完整範圍。
                let start = units + offset * segment.units / chars;
                let end = units + (offset + 1) * segment.units / chars;
                return Some((start, (end - start).max(1)));
            }
            units += segment.units;
        }
        None
    }

    /// 混合狀態下刪一個可見字元：日文 unit 先依 provenance 刪，字面再處理，
    /// 其餘從按鍵流移除對應字元。
    fn delete_mixed_visible(&mut self, rune: usize) -> bool {
        if self.delete_ja_unit_visible(rune) {
            return true;
        }
        if self.delete_literal_rune(rune) {
            return true;
        }
        let typed = self.ascii.typed().to_string();
        if typed.is_empty() {
            return false;
        }
        let saved = self.mixed_rune();
        self.set_mixed_rune(rune);
        let target = self.mixed_target_at_cursor();
        self.set_mixed_rune(saved);
        let Some(target) = target else {
            return false;
        };
        let (start, end) = self.mixed_one_rune_span(&target, rune, &typed);
        if start >= end || end > typed.len() || !typed.is_char_boundary(start) {
            return false;
        }
        let mut next = String::with_capacity(typed.len().saturating_sub(end - start));
        next.push_str(&typed[..start]);
        next.push_str(&typed[end..]);
        let spans = self.mixed_literal_spans_after(start, end);
        self.ascii.replay(&next, &self.lexicon);
        self.literal_spans = spans;
        true
    }

    /// 把候選目標的按鍵範圍縮到游標所在的那一個可見 rune。
    fn mixed_one_rune_span(
        &self,
        target: &MixedTarget,
        rune: usize,
        typed: &str,
    ) -> (usize, usize) {
        let (start, end) = target.typed;
        if target.rune_len <= 1 || start >= end || end > typed.len() {
            return (start, end);
        }
        let offset = rune.saturating_sub(target.rune_start);
        let slice = &typed[start..end];
        let at = slice
            .char_indices()
            .nth(offset)
            .map(|(byte, _)| byte)
            .unwrap_or(slice.len());
        let next = slice[at..]
            .chars()
            .next()
            .map(|ch| at + ch.len_utf8())
            .unwrap_or(slice.len());
        (start + at, start + next)
    }

    /// 混合字面片段在刪除後仍要留成字面，避免剩下的字母被重新判成注音。
    pub(super) fn mixed_literal_spans_after(&self, start: usize, end: usize) -> Vec<LiteralSpan> {
        let removed = end.saturating_sub(start);
        let mut spans: Vec<(usize, usize)> = self
            .literal_spans
            .iter()
            .map(|span| (span.start, span.end))
            .collect();
        let segments = self.segments();
        let typed_spans = self.mixed_typed_spans();
        for (index, segment) in segments.iter().enumerate() {
            if segment.mode == Lang::Zh || segment.mode == Lang::Ja {
                continue;
            }
            if let Some(&(from, to)) = typed_spans.get(index) {
                if from < to {
                    spans.push((from, to));
                }
            }
        }
        spans.sort_unstable();
        spans.dedup();
        let mut out = Vec::new();
        for (from, to) in spans {
            let (from, to) = if to <= start {
                (from, to)
            } else if from >= end {
                (from.saturating_sub(removed), to.saturating_sub(removed))
            } else {
                (
                    from.min(start),
                    if to > end {
                        to.saturating_sub(removed)
                    } else {
                        start
                    },
                )
            };
            if from < to {
                out.push(LiteralSpan::new(from, to));
            }
        }
        out
    }

    /// 刪除 `self.literal` 裡第 `rune` 個可見字元；不在字面內時回傳 `false`。
    fn delete_literal_rune(&mut self, rune: usize) -> bool {
        let base = self.displayed_base();
        let at = self.literal_offset.min(base.len());
        let start = crate::output::rune_offset(&base, at);
        let end = start + self.literal.chars().count();
        if rune < start || rune >= end {
            return false;
        }
        let mut chars: Vec<char> = self.literal.chars().collect();
        chars.remove(rune - start);
        self.literal = chars.into_iter().collect();
        true
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
}
