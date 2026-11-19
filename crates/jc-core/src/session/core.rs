//! [`Session`] 的主狀態機：按鍵處理、候選重建與送出。
//!
//! 型別、設定與公開 API 在上層模組；這裡只放實作細節。

use super::*;

/// 目前 Unix 秒數；時鐘早於 epoch 時回傳 0。
fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
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
        true
    }

    pub(super) fn press_char(&mut self, ch: char) {
        self.accepted_completion = None;
        self.suggestion = None;
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
        let index = match ch {
            '1'..='9' => ch as usize - '1' as usize,
            '0' => 9,
            _ => return false,
        };
        let Some(candidate) = self.candidates.get(index).cloned() else {
            return false;
        };
        // 混合模式的候選是英文補完或拼字建議，選了就取代待定按鍵。
        if self.config.mode == Mode::Mixed && self.ascii.accept(&candidate.surface) {
            self.candidate_window = false;
            self.highlighted = 0;
            return true;
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
            _ if self.candidate_window && !self.candidates.is_empty() => {
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
        self.candidate_window = false;
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
            }
            Mode::En => {
                self.raw.pop();
            }
            Mode::Mixed => {
                self.ascii.backspace();
            }
        }
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

    /// 送出前呼叫標點模型；沒有設定時原樣回傳。
    pub(super) fn model_punctuated(&self, text: String) -> String {
        let Some(punctuator) = &self.config.punct else {
            return text;
        };
        if text.is_empty() {
            return text;
        }
        punctuator.punctuate(&text, self.config.mode)
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

    /// 方向鍵：候選窗還沒開時先開啟並停在目前候選，不立刻移動。
    ///
    /// 前身把「只是顯示」與「明確選取」分開；Down 第一次只進入選取模式，
    /// 第二次才換下一項。這裡用同一個規則簡化兩顆方向鍵。
    pub(super) fn navigate(&mut self, delta: isize) {
        if self.candidates.is_empty() {
            return;
        }
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
    pub(super) fn learn_candidate(&mut self, index: usize) {
        if !self.config.learning || self.menu.is_some() {
            return;
        }
        match self.config.mode {
            Mode::Zh => {
                self.note_learning_event();
                self.learn_zh_candidate(index);
            }
            Mode::Ja => {
                self.note_learning_event();
                self.learn_ja_candidate(index);
            }
            Mode::En | Mode::Mixed => {}
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
            } else {
                self.personal
                    .note_passive(&candidate.surface, &candidate.reading);
            }
        }
    }

    /// 把一次日文提交記進共用個人資料的 `ja` 命名空間。
    ///
    /// 優先使用這一輪 N-best 的文節切分；候選不是這一輪結果時，退化成整筆
    /// 候選一個文節。明確手選（候選窗開著）比被動接受多一倍證據。
    pub(super) fn learn_ja_candidate(&mut self, index: usize) {
        let Some(candidate) = self.candidates.get(index).cloned() else {
            return;
        };
        let weight = if self.candidate_window {
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
        let left = self.committed.clone();
        ja_personal::learn_unified(&mut self.personal, &clauses, &left, weight, now_seconds());
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

    /// 顯示用字串：高亮候選反映在組字區。
    pub(super) fn base_with_highlight(&self) -> String {
        if self.menu.is_some() {
            // 選單開啟時組字區維持原樣，候選只是待選清單。
            return self.preedit_string();
        }
        let base = match (
            &self.accepted_completion,
            self.candidates.get(self.highlighted),
        ) {
            (Some(text), _) => text.as_str(),
            // 混合模式的候選只取代待定英數按鍵，`base` 已含前面的中文片段；
            // 直接拿候選當整個組字字串會把中文吃掉。
            (None, _) if self.config.mode == Mode::Mixed => self.base.as_str(),
            (None, Some(candidate)) => candidate.surface.as_str(),
            (None, None) => self.base.as_str(),
        };
        format!("{}{}", self.insert_literal(base), self.pending)
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
        let mut results = decoder.results(&chars, &ctx, self.max_candidates());
        self.rank_ja_learning(&mut results, &ctx.left);
        let Some(best) = results.first() else {
            self.base = reading;
            return;
        };
        self.base = best.surface();
        self.segments = best.conversion().segments;
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
        self.last_ja_results = results;
    }

    /// 依共用的日文學習資料重排 N-best；學習關閉時維持統計順序。
    pub(super) fn rank_ja_learning(&self, results: &mut [JaResult], left: &str) {
        if !self.config.learning {
            return;
        }
        let learning = ja_personal::UnifiedLearning::new(&self.personal);
        learning.rank_results(results, left);
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
                self.rank_ja_learning(&mut results, &self.committed);
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
        self.candidates.push(Candidate::new(
            self.raw.clone(),
            self.raw.clone(),
            0,
            CandidateSource::Literal,
        ));
        if let Some(full) = symbols::width_variant(&self.raw) {
            if full != self.raw {
                self.candidates.push(Candidate::new(
                    full,
                    self.raw.clone(),
                    1,
                    CandidateSource::Literal,
                ));
            }
        }
        // 前綴補完先排，拼字建議接在後面；打對的詞不會有建議。
        let extra = english_candidates(&self.raw, limit);
        let mut seen: Vec<String> = self
            .candidates
            .iter()
            .map(|candidate| candidate.surface.to_lowercase())
            .collect();
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
            let pending = self.ascii.pending();
            let start = at;
            at += pending.len();
            self.segments
                .push(Segment::new(start, at, Mode::Zh, Script::Bopomofo));
        }
        // 待定按鍵是英文詞時，候選先放原按鍵，再放前綴補完與拼字建議。
        let raw = self.ascii.pending().to_string();
        if raw.is_empty() {
            return;
        }
        let limit = self.max_candidates();
        self.candidates.push(Candidate::new(
            raw.clone(),
            raw.clone(),
            0,
            CandidateSource::Literal,
        ));
        let mut seen = vec![raw.to_lowercase()];
        for candidate in english_candidates(&raw, limit) {
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
        out.push_str(&self.ascii.display_pending());
        out
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

/// 英文片段的候選：前綴補完先排，拼字建議接在後面。
///
/// 對照前身 `core/session/english_autocomplete.go` 的
/// `refreshEnglishCandidates` 與 `core/session/mixed.go` 的
/// `openMixedCandidates`：最多保留 3 個拼字建議，剩下的名額才給前綴補完，
/// 這樣補完不會把拼字建議擠掉。拼字建議直接用已收進來的
/// [`crate::mixed::Dictionary::corrections`]，沒有另寫演算法；打對的完整單字
/// 不會有建議，建議保留查詢時的大小寫。
fn english_candidates(raw: &str, limit: usize) -> Vec<Candidate> {
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
    out.truncate(limit);
    out
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
