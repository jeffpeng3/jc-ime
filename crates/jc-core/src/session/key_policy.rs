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
        // 注音的第一個 Tab 和 ↓ 走同一個開窗路徑：清單由共用 builder
        // 產生，帶目前值、原始輸入與平行動作。開窗後仍照原本 Tab 語意換到
        // 下一項；英文整詞補全與日文 Tab 由下面獨立處理。
        if !self.navigating
            && !self.candidate_window
            && self.suggestion.is_none()
            && self.in_zh_lane()
        {
            self.open_mixed_cursor_candidates();
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

        let deleted = self.delete_unit_visible(target);
        if deleted {
            self.cursor_rune = after.min(self.rune_count());
        }
    }

    /// 刪一個顯示字元；目標 rune 已經由游標決定，lane 只負責套用。
    ///
    /// 注音、日文與混打都在這裡收斂：先看字面尾巴，再交給該狀態的 unit
    /// 刪除。沒有任何「整段／文節／音節」的範圍計算。
    fn delete_unit_visible(&mut self, rune: usize) -> bool {
        if self.delete_literal_tail_rune(rune) {
            return true;
        }
        if self.in_zh_lane() {
            return self.delete_zh_visible(rune);
        }
        if self.in_ja_lane() {
            return self.delete_ja_unit_visible(rune) || self.delete_ja_visible(rune);
        }
        self.delete_mixed_visible(rune)
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
        self.delete_zh_decoded_unit(rune)
    }

    /// 已轉換的注音顯示字元：只刪這一個 unit。
    ///
    /// 按鍵流由 units 的 keys 串接，刪掉 target 的 keys 後重播；一般一音節
    /// 一字時就等於換掉那一格。一音節多字（`台灣/ㄊㄞˊ`）無法用單一音節
    /// 按鍵表示其中一個字，改成直接移除顯示字元並固定剩下的 provenance。
    fn delete_zh_decoded_unit(&mut self, rune: usize) -> bool {
        let (units, spans) = self.compute_units();
        let Some(unit) = units.get(rune) else {
            return false;
        };
        if unit.lang != Lang::Zh
            || matches!(
                unit.meta,
                DisplayUnitMeta::Literal | DisplayUnitMeta::Unknown
            )
        {
            return false;
        }
        if self.zh_segment_is_multi_char(rune) {
            return self.delete_display_unit_direct(rune, &units);
        }
        // 一般一個顯示字元就是一個已完成音節；用 unit 的 rune 找到它在
        // `syllables` 的位置，只移除那一格，保留注音 lane 的後續輸入狀態。
        let syllable = units
            .iter()
            .take(rune)
            .filter(|unit| {
                unit.lang == Lang::Zh
                    && !matches!(
                        unit.meta,
                        DisplayUnitMeta::Literal
                            | DisplayUnitMeta::Unknown
                            | DisplayUnitMeta::Zhuyin
                    )
            })
            .count();
        if syllable < self.syllables.len() && self.remove_syllable(syllable) {
            return true;
        }
        let keys: String = units.iter().map(|unit| unit.keys.as_str()).collect();
        let (start, end) = spans.get(rune).copied().unwrap_or((0, 0));
        if start >= end || end > keys.len() || !keys.is_char_boundary(start) {
            return false;
        }
        let mut next = String::with_capacity(keys.len().saturating_sub(end - start));
        next.push_str(&keys[..start]);
        next.push_str(&keys[end..]);
        self.zh.reset();
        self.zh_lane = false;
        self.zh_space_closed = false;
        self.zh_pending_keys.clear();
        self.zh_pending_caps.clear();
        self.syllables.clear();
        self.zh_syllable_keys.clear();
        self.locks.clear();
        self.composition.clear();
        self.edit = None;
        self.keep_candidates = false;
        self.highlighted = 0;
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
        self.cursor_rune = Self::MIXED_CURSOR_END;
        self.ascii.replay(&next, &self.lexicon);
        self.literal_spans.clear();
        self.rebuild();
        true
    }

    /// 目標 rune 所屬的注音片段是否一個音節對多個顯示字元。
    pub(super) fn zh_segment_is_multi_char(&self, rune: usize) -> bool {
        self.composition.segments().iter().any(|segment| {
            segment.lang == crate::composition::Lang::Zh
                && rune >= segment.span.start
                && rune < segment.span.start + segment.chars()
                && segment.chars() > segment.units
        })
    }

    /// 一音節多字時，按鍵流無法單獨表示其中一個字；直接把剩下的顯示文字
    /// 凍結成字面，其他狀態清乾淨。呼叫端已用 units 選好要刪的 rune，不涉
    /// 音節範圍。
    fn delete_display_unit_direct(&mut self, rune: usize, _units: &[DisplayUnit]) -> bool {
        let mut chars: Vec<char> = self.composed_text().chars().collect();
        if rune >= chars.len() {
            return false;
        }
        chars.remove(rune);
        let remaining: String = chars.into_iter().collect();

        self.zh.reset();
        self.zh_lane = false;
        self.zh_space_closed = false;
        self.zh_pending_keys.clear();
        self.zh_pending_caps.clear();
        self.syllables.clear();
        self.zh_syllable_keys.clear();
        self.locks.clear();
        self.composition.clear();
        self.keep_candidates = false;
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
        self.cursor_rune = Self::MIXED_CURSOR_END;
        self.ascii.replay("", &self.lexicon);
        self.literal_spans.clear();
        self.literal = remaining;
        true
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

    /// 未完成注音的第 `display_index` 個顯示符號是哪一個按鍵打出來的。
    ///
    /// 畫面是正規順序（`ㄗㄜ`），按鍵流是實際順序（`ky`）；不能用同一個
    /// 索引硬對，否則 Backspace 會刪到另一個符號。
    fn zh_pending_key_index(&self, display_index: usize) -> Option<usize> {
        let symbol = self.pending.chars().nth(display_index)?;
        let class = crate::zh::syllable::class_of(symbol);
        if !matches!(
            class,
            crate::zh::syllable::Class::Initial
                | crate::zh::syllable::Class::Medial
                | crate::zh::syllable::Class::Final
        ) {
            return None;
        }
        let keyboard = self.zh.keyboard();
        let mut found = None;
        for (at, key) in self.zh_pending_keys.chars().enumerate() {
            if keyboard
                .symbol(key)
                .map(crate::zh::syllable::class_of)
                .is_some_and(|other| other == class)
            {
                found = Some(at);
            }
        }
        found
    }

    /// 從未完成注音刪一個按鍵，再用剩下的按鍵重建注音狀態。
    fn delete_zh_pending_at(&mut self, index: usize) -> bool {
        let key_index = self.zh_pending_key_index(index).unwrap_or(index);
        let mut keys: Vec<char> = self.zh_pending_keys.chars().collect();
        let mut caps: Vec<char> = self.zh_pending_caps.chars().collect();
        if key_index >= keys.len() {
            return false;
        }
        keys.remove(key_index);
        if key_index < caps.len() {
            caps.remove(key_index);
        }
        self.zh.reset();
        self.zh_pending_keys.clear();
        self.zh_pending_caps.clear();
        for (at, ch) in keys.into_iter().enumerate() {
            let cap = caps.get(at).copied().unwrap_or('0');
            self.zh.press(Key::char(ch, false));
            self.record_zh_pending_key(ch, cap == '1');
        }
        // 刪除鍵本身就是「下一個按鍵」；重播剩下的按鍵不該讓上一個被擋的
        // 聲調旗標復活。
        self.zh.clear_tone_blocked();
        true
    }

    /// 日文未完成前綴刪一個可見字元。
    ///
    /// 已完成的讀音單位由 [`Session::delete_ja_unit_visible`] 依 unit 刪除；
    /// 這裡只處理尾巴還沒轉成かな的羅馬字，不再用 `offset * units / chars`
    /// 把顯示字元比例切給讀音單位。
    fn delete_ja_visible(&mut self, rune: usize) -> bool {
        let base_runes = self.base.chars().count();
        if rune < base_runes {
            return false;
        }
        let deleted = self.romaji.delete_pending_at(rune - base_runes);
        if deleted {
            self.ja_delete_surface = None;
            self.ja_run_keys = self.romaji.raw_keys();
            if let Some(live) = self.live.as_mut() {
                live.invalidate_for_edit();
            }
            self.leave_empty_japanese_run();
            self.rebuild();
        }
        deleted
    }

    /// 可見字元對應的讀音單位範圍。
    ///
    /// 由 `compute_units()` 的 provenance 決定：每個顯示字元帶著自己的
    /// reading（`私` → `わたし`、`今` → `きょ`），把前面的 reading 長度相加
    /// 就是它在 `romaji.units()` 的起點。不再用字數比例 `offset * units /
    /// chars` 猜。
    pub(super) fn ja_units_at_rune(&self, rune: usize) -> Option<(usize, usize)> {
        // 候選窗開著時 `displayed_base()` 會換成高亮候選，不能用它重算
        // units；`self.units` 是開窗時同步好的實際畫面 provenance。
        let computed;
        let units = if self.units.is_empty() {
            computed = self.compute_units().0;
            &computed
        } else {
            &self.units
        };
        let mut at = 0usize;
        for (index, unit) in units.iter().enumerate() {
            let count = unit
                .reading
                .as_deref()
                .map(|reading| reading.chars().count())
                .unwrap_or(0);
            if index == rune {
                return (count > 0).then_some((at, count));
            }
            at = at.saturating_add(count);
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
