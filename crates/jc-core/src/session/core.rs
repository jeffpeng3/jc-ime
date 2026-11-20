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
            self.literal_anchor = self.zh.syllables().len();
        }
        self.literal.push_str(text);
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
        self.accepted_completion = None;
        self.suggestion = None;
        self.navigating = false;
        let window = self.candidate_window;
        if window && self.select_by_digit(ch) {
            self.candidate_window = false;
            return;
        }
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
        match self.config.mode {
            Mode::Zh => {
                if self.zh.pending().is_empty() {
                    if ch == '`' {
                        self.open_menu(MenuKind::Symbols, 0);
                        return;
                    }
                    if ch == '=' {
                        self.open_menu(MenuKind::Emoji, self.palette_tab);
                        return;
                    }
                }
                match self.zh.press(Key::char(ch, false)) {
                    crate::zh::input::ZhInputEvent::Consumed
                    | crate::zh::input::ZhInputEvent::SyllableComplete(_) => {}
                    crate::zh::input::ZhInputEvent::Commit(reading) => {
                        // 空白鍵在沒有未完成注音時提交；改用轉換後的結果。
                        if let Some(text) = self.convert_reading(&reading) {
                            self.pending_commit = Some(text);
                            self.zh.reset();
                        }
                    }
                    crate::zh::input::ZhInputEvent::Cancel => self.zh.reset(),
                    crate::zh::input::ZhInputEvent::Ignored => {
                        let punct = self.zh.keyboard().punct(ch);
                        let text = punct.map(str::to_string).unwrap_or_else(|| ch.to_string());
                        self.push_literal(&text);
                    }
                }
            }
            Mode::Ja => {
                self.romaji.press(Key::char(ch, false));
            }
            Mode::En => self.raw.push(ch),
            Mode::Mixed => {
                if !self.ascii.tone(ch, &self.lexicon) {
                    self.ascii.press(ch);
                }
                self.try_japanese_run();
            }
        }
    }

    /// 數字鍵在候選視窗開啟時選字；成功時把該候選放進待提交區。
    pub(super) fn select_by_digit(&mut self, ch: char) -> bool {
        if self.candidates.is_empty()
            || !self.candidate_window
            || (self.config.mode == Mode::Zh
                && !self.zh.pending().is_empty()
                && self.menu.is_none())
        {
            return false;
        }
        let index = match self.selection_index(ch) {
            Some(index) => index,
            None => return false,
        };
        let Some(candidate) = self.candidates.get(index).cloned() else {
            return false;
        };
        // 混合模式的候選是英文補完或拼字建議，選了就取代待定按鍵。
        if self.config.mode == Mode::Mixed {
            let raw = self.ascii.pending().to_string();
            if self.ascii.accept(&candidate.surface) {
                self.learn_english_choice(&raw, &candidate.surface);
                self.candidate_window = false;
                self.navigating = false;
                self.highlighted = 0;
                return true;
            }
        }
        self.remember_text(&candidate.surface);
        self.learn_candidate(index);
        let text = if self.menu.is_some() {
            self.compose_literal(&candidate.surface)
        } else {
            self.insert_literal(&candidate.surface)
        };
        self.pending_commit = Some(self.finalize(text));
        self.clear_composition();
        true
    }

    pub(super) fn space(&mut self) {
        match self.config.mode {
            Mode::Zh if !self.zh.pending().is_empty() => {
                self.zh.press(Key::new(KeyCode::Space, None, false));
            }
            Mode::Zh
                if !self.candidate_window
                    && !self.candidates.is_empty()
                    && !self.style.space_inserts_space =>
            {
                // 音節打完後空白開候選窗；「空白只插空白」設定保持原行為。
                self.candidate_window = true;
                self.navigating = false;
            }
            _ if self.candidate_window && !self.candidates.is_empty() => {
                self.navigating = false;
                self.remember_selection();
                self.learn_composition();
                let text = self.finalize(self.current_text());
                self.pending_commit = Some(text);
                self.clear_composition();
            }
            Mode::En => self.raw.push(' '),
            Mode::Mixed => self.ascii.space(&self.lexicon),
            _ => {}
        }
    }

    pub(super) fn backspace(&mut self) {
        // 候選窗開著時退格只關窗，不刪已組好的字（上游 #106）。
        if self.candidate_window && self.menu.is_none() {
            self.candidate_window = false;
            self.navigating = false;
            self.highlighted = 0;
            return;
        }
        self.candidate_window = false;
        self.navigating = false;
        match self.config.mode {
            Mode::Zh => {
                if self.zh.pending().is_empty()
                    && self.zh.syllables().is_empty()
                    && !self.literal.is_empty()
                {
                    self.literal.pop();
                } else {
                    self.zh.press(Key::new(KeyCode::Backspace, None, false));
                }
            }
            Mode::Ja => {
                self.romaji.backspace();
                self.leave_empty_japanese_run();
            }
            Mode::En => {
                self.raw.pop();
            }
            Mode::Mixed => {
                self.ascii.backspace();
            }
        }
    }

    /// Esc 的優先權：先關候選窗，再清未完成的注音，最後才清整段組字。
    ///
    /// 對照上游 `key()` 的 Escape 分支；標示範圍與同音預覽在本埠沒有對應狀態。
    pub(super) fn escape(&mut self) {
        if self.candidate_window && self.menu.is_none() {
            self.candidate_window = false;
            self.navigating = false;
            self.highlighted = 0;
            return;
        }
        if self.config.mode == Mode::Zh && !self.zh.pending().is_empty() {
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
        self.candidate_window = false;
        self.navigating = false;
    }

    /// `Delete`：候選窗開著時只關窗；沒有組字游標，其餘情況不動組字。
    pub(super) fn delete_forward(&mut self) {
        if self.candidate_window {
            self.candidate_window = false;
            self.navigating = false;
            self.highlighted = 0;
        }
    }

    /// 下鍵：第一次開候選窗並進入選取模式，之後往下換一項。
    pub(super) fn down(&mut self) {
        if self.candidates.is_empty() {
            return;
        }
        if !self.candidate_window {
            self.candidate_window = true;
            self.navigating = true;
            return;
        }
        if !self.navigating {
            self.navigating = true;
            return;
        }
        self.move_highlight(1);
    }

    /// 上鍵：候選窗開著時往上換一項並進入選取模式；還沒開窗時先開窗。
    pub(super) fn up(&mut self) {
        if self.candidates.is_empty() {
            return;
        }
        if !self.candidate_window {
            self.candidate_window = true;
            self.navigating = true;
            return;
        }
        self.navigating = true;
        self.move_highlight(-1);
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

    /// `Home`／`End`：選取模式中移到候選清單的第一項或最後一項。
    ///
    /// 只是顯示候選時（例如空白開窗、還沒按 ↓）這兩個鍵移動組字游標到
    /// 開頭／結尾，對照上游 `moveShownCandidateCursor`；游標不動候選內容，
    /// 也不會進入選取模式。
    pub(super) fn candidate_edge(&mut self, end: bool) {
        if self.candidates.is_empty() || !self.candidate_window {
            return;
        }
        if !self.navigating {
            if self.config.mode == Mode::Zh {
                let cells = self.zh.cells() as isize;
                self.zh.cursor_move(if end { cells } else { -cells });
            }
            return;
        }
        self.highlighted = if end { self.candidates.len() - 1 } else { 0 };
        self.navigating = true;
    }

    /// 快照的補全尾綴：注音用路徑補全，混打用英文前綴補全。
    pub(super) fn snapshot_suggestion(&self) -> Option<String> {
        if self.menu.is_some() || self.candidate_window || self.accepted_completion.is_some() {
            return None;
        }
        if self.config.mode == Mode::Mixed && self.config.english_autocomplete {
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
        if self.config.mode != Mode::Zh || self.menu.is_some() {
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

    pub(super) fn finish_composition(&mut self) {
        if self.config.mode == Mode::Mixed {
            self.ascii.flush();
        }
        self.remember_selection();
        self.learn_composition();
        let text = match self.config.mode {
            Mode::Mixed => self.mixed_surface(),
            _ => self.current_text(),
        };
        let text = self.finalize(text);
        if !text.is_empty() {
            self.pending_commit = Some(text);
        }
        self.clear_composition();
    }

    /// 送出前套用簡體轉換與中英自動空格。
    pub(super) fn finalize(&self, text: String) -> String {
        let text = self.puncted(text);
        let text = match &self.config.simplified {
            Some(converter) => converter.to_simplified(&text),
            None => text,
        };
        if self.config.cjk_latin_space {
            space_text(&text, Spacing::Always)
        } else {
            text
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
            self.literal_anchor = self.zh.syllables().len();
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
    /// 組字游標；進入選取模式後才在候選間移動。本埠的候選清單固定是整段
    /// 組字，因此換游標不會把清單換成該片段的候選。
    pub(super) fn navigate(&mut self, delta: isize) {
        if self.candidates.is_empty() {
            return;
        }
        if self.candidate_window && !self.navigating && self.config.mode == Mode::Zh {
            self.zh.cursor_move(delta);
            return;
        }
        self.navigating = true;
        if !self.candidate_window {
            self.candidate_window = true;
            return;
        }
        self.move_highlight(delta);
    }

    /// 把這次送出的候選記進個人學習。
    pub(super) fn learn_composition(&mut self) {
        let index = self.highlighted;
        self.learn_candidate(index);
    }

    /// 把指定候選記進個人學習；數字鍵直接選取時用得到。
    ///
    /// 動手前先留一份快照：下一次輸入若是 Backspace／Esc／Ctrl+Z，就把這次
    /// 學習補償掉；否則在 [`Session::press`] 開頭丟掉快照。
    pub(super) fn learn_candidate(&mut self, index: usize) {
        if !self.config.learning || self.menu.is_some() {
            return;
        }
        self.personal.begin_rollback(now_millis());
        match self.config.mode {
            Mode::Zh => {
                self.note_learning_event();
                self.learn_zh_candidate(index);
            }
            Mode::Ja => {
                self.note_learning_event();
                self.learn_ja_candidate(index);
            }
            Mode::En => {
                self.note_learning_event();
                self.learn_en_candidate(index);
            }
            Mode::Mixed => {
                self.note_learning_event();
                self.learn_mixed_languages();
            }
        }
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

    /// 把一次英文候選記進個人學習。
    ///
    /// 接受修正時寫入「打錯 → 修正」的證據；選回原字或其他候選就對學過的
    /// 修正記一次跳過。連續 [`crate::personal::SKIP_LIMIT`] 次後降權，下一次
    /// 手選修正會清掉跳過計數。前綴補完不算拼字修正，不寫入這張表。
    pub(super) fn learn_en_candidate(&mut self, index: usize) {
        if !self.config.learning {
            return;
        }
        let Some(candidate) = self.candidates.get(index).cloned() else {
            return;
        };
        let wrong = self.raw.trim().to_string();
        self.learn_english_choice(&wrong, &candidate.surface);
    }

    /// 記錄一次英文選取：修正或跳過。
    fn learn_english_choice(&mut self, wrong: &str, chosen: &str) {
        if !self.config.learning
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

    pub(super) fn clear_composition(&mut self) {
        if self.ja_run {
            self.config.mode = self.ja_prev_mode;
            self.ja_run = false;
        }
        self.zh.reset();
        self.romaji.reset();
        self.ascii.reset();
        self.literal.clear();
        self.literal_anchor = 0;
        self.literal_offset = 0;
        self.raw.clear();
        self.candidates.clear();
        self.highlighted = 0;
        self.candidate_window = false;
        self.navigating = false;
        self.segments.clear();
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
        if self.config.mode == Mode::Mixed {
            return self.mixed_surface();
        }
        if self.menu.is_some() {
            if let Some(candidate) = self.candidates.get(self.highlighted) {
                return self.compose_literal(&candidate.surface);
            }
        }
        if let Some(text) = &self.accepted_completion {
            return format!("{}{}", self.insert_literal(text), self.pending);
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
        match (
            &self.accepted_completion,
            self.candidates.get(self.highlighted),
        ) {
            (Some(text), _) => text.as_str(),
            // 混合模式的候選只取代待定英數按鍵，`base` 已含前面的中文片段；
            // 直接拿候選當整個組字字串會把中文吃掉。
            (None, _) if self.config.mode == Mode::Mixed => &self.base,
            (None, Some(candidate)) => candidate.surface.as_str(),
            (None, None) => &self.base,
        }
    }

    /// 顯示用字串：高亮候選反映在組字區。
    pub(super) fn base_with_highlight(&self) -> String {
        format!(
            "{}{}",
            self.insert_literal(self.displayed_base()),
            self.pending
        )
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
        if self.menu.is_some() {
            self.rebuild_menu();
            return;
        }
        self.base.clear();
        self.pending.clear();
        self.candidates.clear();
        self.segments.clear();
        self.literal_offset = 0;
        match self.config.mode {
            Mode::Zh => self.rebuild_zh(),
            Mode::Ja => self.rebuild_ja(),
            Mode::En => self.rebuild_en(),
            Mode::Mixed => self.rebuild_mixed(),
        }
        if self.highlighted >= self.candidates.len() {
            self.highlighted = 0;
        }
    }

    pub(super) fn rebuild_zh(&mut self) {
        self.pending = self.zh.pending();
        let syllables = self.zh.syllables().to_vec();
        if syllables.is_empty() {
            return;
        }
        let readings = self.zh_readings(&syllables);
        let units: Vec<&str> = readings.iter().map(String::as_str).collect();
        // 片段鎖定只涵蓋目前還在的音節；退格或送出後留下的過期鎖定直接丟掉。
        let count = syllables.len();
        self.locks
            .retain(|&at, lock| at < count && at.saturating_add(lock.len) <= count);
        let Ok(lattice) = Lattice::build(&units, &self.lexicon, &self.ctx) else {
            self.base = units.join("");
            return;
        };
        let scorer = zh_scorer(&self.ctx, self.zh_lm.as_deref());
        let decoder = zh_decoder(&self.lexicon, &self.ctx, scorer.as_ref());
        let (segs, _) = decoder.best(&lattice, &self.locks, false);
        let conversion = decoder.conversion(&lattice, &segs, Mode::Zh);
        self.base = conversion.surface;
        self.segments = Self::segments_of(&segs, Mode::Zh, Script::Bopomofo);
        self.literal_offset = if self.literal_anchor == 0 {
            0
        } else {
            self.segments
                .get(self.literal_anchor - 1)
                .map(|segment| segment.end)
                .unwrap_or(self.base.len())
        };
        let reading = readings.join(" ");
        let literal = readings.join("");
        let (mut hypotheses, _) =
            decoder.nbest(&lattice, &self.locks, false, self.max_candidates());
        if self.config.learning {
            self.personal.rank_hypotheses(&mut hypotheses, &reading);
        }
        if let Some(options) = &self.config.rerank {
            Self::apply_rerank(&mut hypotheses, options);
        }
        if let Some(best) = hypotheses.first() {
            self.base = best.text.clone();
            self.segments = Self::segments_of(&best.segments, Mode::Zh, Script::Bopomofo);
        }
        self.last_hypotheses = hypotheses.clone();
        let best_segments = hypotheses
            .first()
            .map(|hypothesis| hypothesis.segments.clone())
            .unwrap_or_default();
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
        let phonetic = self.phonetic_candidates(&best_segments);
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
        self.candidates = candidates;
        let (completion, _) = decoder.complete(&lattice, &segs);
        self.suggestion = if completion.found() {
            Some(completion.entry.surface)
        } else {
            None
        };
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
            let scorer = zh_scorer(&self.ctx, self.zh_lm.as_deref());
            let decoder = zh_decoder(&self.lexicon, &self.ctx, scorer.as_ref());
            let (hypotheses, _) =
                decoder.nbest(&lattice, &Locks::new(), false, self.max_candidates());
            for hypothesis in hypotheses {
                if hypothesis.text == variant.join("") || seen.contains(&hypothesis.text) {
                    continue;
                }
                seen.push(hypothesis.text.clone());
                out.push(Candidate::new(
                    hypothesis.text,
                    original.clone(),
                    hypothesis.cost,
                    CandidateSource::Lexicon,
                ));
            }
        }
        out.sort_by_key(|candidate| candidate.cost);
        out
    }

    pub(super) fn rebuild_ja(&mut self) {
        self.pending = self.romaji.pending().to_string();
        let units = self.romaji.units().to_vec();
        let reading: String = units.iter().map(Unit::as_str).collect();
        if reading.is_empty() {
            return;
        }
        if self.live.is_some() {
            self.rebuild_ja_live(&units);
            return;
        }
        let Some(decoder) = self.config.ja.clone() else {
            self.base = reading;
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
        // 路徑；那不是完整轉換，送模型只會每鍵呼叫一次。統計結果等於讀音
        // 時直接跳過，等最後的完整讀音再問模型。
        let decision = if baseline == reading {
            NeuralDecision::Skipped
        } else {
            neural::maybe_convert(
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
            )
        };
        let replaced = decision.accepted_surface().map(str::to_string);
        self.base = replaced.clone().unwrap_or(baseline);
        self.segments = if replaced.is_some() {
            // 神經輸出沒有詞邊界；整段當一個文節，避免片段範圍對不上。
            vec![Segment::new(0, self.base.len(), Mode::Ja, Script::Kana)]
        } else {
            best.conversion().segments
        };
        self.candidates = results
            .iter()
            .map(|result| {
                let conversion = result.conversion();
                Candidate::new(
                    conversion.surface,
                    reading.clone(),
                    conversion.cost,
                    CandidateSource::Lexicon,
                )
            })
            .collect();
        if let (Some(surface), Some(candidate)) = (replaced, self.candidates.first_mut()) {
            // 顯示與提交都走候選清單，第一名必須跟著換。
            candidate.surface = surface;
        }
        self.last_ja_results = results;
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
        let plan = match &mut self.live {
            Some(live) => live.input(units, &JaContext::default()),
            None => return,
        };
        let reading: String = units.iter().map(Unit::as_str).collect();
        self.apply_live_plan(&plan, units, &reading);
    }

    /// 套用逐字変換的決定。
    ///
    /// 有新結果時整段換成新轉換；只有 draft 時顯示前次結果再加上尚未轉換的
    /// かな，讓畫面不會先退回かな。debounce 等待中的新かな因此仍可看見。
    pub(super) fn apply_live_plan(&mut self, plan: &LivePlan, units: &[Unit], reading: &str) {
        self.live_plan = Some(plan.clone());
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
                segments.push(Segment::new(start, base.len(), Mode::Ja, Script::Kana));
            }
            display = base.clone();
        } else {
            base = reading.to_string();
        }
        self.base = base;
        self.segments = segments;

        let mut candidates = Vec::new();
        let mut results = Vec::new();
        if plan.convert {
            if let Some(decoder) = self.config.ja.clone() {
                let chars: Vec<char> = reading.chars().collect();
                results = decoder.results(&chars, &JaContext::default(), self.max_candidates());
                let left = self.committed.clone();
                self.rank_ja_learning(&mut results, &left);
                candidates = results
                    .iter()
                    .map(|result| {
                        let conversion = result.conversion();
                        Candidate::new(
                            conversion.surface,
                            reading.to_string(),
                            conversion.cost,
                            CandidateSource::Lexicon,
                        )
                    })
                    .collect();
            }
        }
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

    pub(super) fn rebuild_en(&mut self) {
        self.base = self.raw.clone();
        if self.raw.is_empty() {
            return;
        }
        let limit = self.max_candidates();
        let mut seen: Vec<String> = Vec::new();
        // 同一個拼法有學過的修正時，修正直接排第一；原字仍在候選裡。
        if let Some(learned) = self.personal.english_correction(&self.raw) {
            let learned = learned.to_string();
            seen.push(learned.to_lowercase());
            self.candidates.push(Candidate::new(
                learned,
                self.raw.clone(),
                0,
                CandidateSource::Personal,
            ));
        }
        seen.push(self.raw.to_lowercase());
        self.candidates.push(Candidate::new(
            self.raw.clone(),
            self.raw.clone(),
            0,
            CandidateSource::Literal,
        ));
        if let Some(full) = symbols::width_variant(&self.raw) {
            if full != self.raw {
                seen.push(full.to_lowercase());
                self.candidates.push(Candidate::new(
                    full,
                    self.raw.clone(),
                    1,
                    CandidateSource::Literal,
                ));
            }
        }
        // 學過的修正先排，內建前綴補完次之，拼字建議接在後面；打對的詞不會
        // 有建議。
        let extra = super::mixed::english_candidates(&self.personal, &self.raw, limit);
        for candidate in extra {
            let key = candidate.surface.to_lowercase();
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            self.candidates.push(candidate);
        }
        self.candidates.truncate(limit);
    }

    /// 把解碼片段轉成輸出字串上的位元組範圍。
    pub(super) fn segments_of(
        segs: &[crate::decode::Seg],
        mode: Mode,
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
        Some(decoder.conversion(&lattice, &segs, Mode::Zh).surface)
    }
}

/// 有 bigram 時建立內插評分器；`lm.bin` 沒有 bigram 時回傳 `None`，
/// 解碼完全走原本的 unigram 成本。
fn zh_scorer<'a>(
    ctx: &'a ScoreContext,
    lm: Option<&'a crate::score::ZhLm>,
) -> Option<crate::score::LmScorer<'a>> {
    lm.filter(|lm| lm.bigrams() > 0)
        .map(|lm| crate::score::LmScorer::new(ctx, lm))
}

/// 建立注音解碼器；有 bigram 時接上評分器並套用 beam 上限。
fn zh_decoder<'a>(
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
        lex
    }

    fn session() -> Session {
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lexicon());
        config.max_candidates = 20;
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
        s.press(Key::new(KeyCode::Space, None, false));
        s.press(Key::new(KeyCode::Down, None, false));
        assert!(s.snapshot().navigating);
        type_syllable(&mut s, "ㄈㄥ");
        let snap = s.snapshot();
        assert!(!snap.navigating, "打字後離開選取模式");
        assert!(snap.preedit.ends_with('風'), "{}", snap.preedit);
    }

    /// 對照 `TestEscLeavesSelectionMode`：Esc 關窗、保留組字。
    #[test]
    fn test_escape_closes_candidate_window_and_keeps_composition() {
        let mut s = session();
        type_syllable(&mut s, "ㄕˋ");
        s.press(Key::new(KeyCode::Space, None, false));
        s.press(Key::new(KeyCode::Down, None, false));
        let snap = s.press(Key::new(KeyCode::Escape, None, false));
        assert!(!snap.navigating, "Esc 離開選取模式");
        assert_eq!(snap.preedit, "甲", "Esc 保留已組好的字");
        // 關窗後數字鍵回到注音路徑，不再選字。
        let typed = s.press(Key::char('2', false));
        assert_eq!(typed.preedit, "甲ㄉ", "關窗後數字是注音鍵");
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

    /// 對照 `TestCandidateWindowSwallowsKeys`：退格只關候選窗、不刪字。
    #[test]
    fn test_backspace_closes_candidate_window_without_deleting() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        s.press(Key::new(KeyCode::Down, None, false));
        let snap = s.press(Key::new(KeyCode::Backspace, None, false));
        assert!(!snap.navigating, "退格關閉候選");
        assert_eq!(snap.preedit, "我", "退格不刪已組好的字");
        let typed = s.press(Key::char('2', false));
        assert_eq!(typed.preedit, "我ㄉ", "關窗後數字是注音鍵");
    }

    /// Delete 沒有組字游標可前向刪除，只關候選窗。
    #[test]
    fn test_delete_only_closes_candidate_window() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        s.press(Key::new(KeyCode::Down, None, false));
        let snap = s.press(Key::new(KeyCode::Delete, None, false));
        assert!(!snap.navigating);
        assert_eq!(snap.preedit, "我", "Delete 不刪組字");
        let typed = s.press(Key::char('2', false));
        assert_eq!(typed.preedit, "我ㄉ", "關窗後數字是注音鍵");
    }

    /// 空白鍵在音節完成後開候選窗，與上游 `openCandidates(0)` 相同。
    #[test]
    fn test_space_opens_candidates_after_complete_syllable() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        let snap = s.press(Key::new(KeyCode::Space, None, false));
        assert!(!snap.candidates.is_empty(), "空白開候選窗");
        assert!(!snap.navigating);
        let selected = s.press(Key::new(KeyCode::Space, None, false));
        assert_eq!(selected.preedit, "", "第二次空白選字並收起組字");
        assert_eq!(s.commit(), "我");
    }

    /// 「空白只插空白」設定關閉空白開候選。
    #[test]
    fn test_space_inserts_space_keeps_literal_behavior() {
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lexicon());
        config.style.space_inserts_space = true;
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄨㄛˇ");
        let snap = s.press(Key::new(KeyCode::Space, None, false));
        assert!(!snap.navigating, "設定關閉時不進入選取模式");
        assert_eq!(snap.preedit, "我", "空白不選字");
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
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lexicon());
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
}
