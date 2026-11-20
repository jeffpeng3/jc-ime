//! 語言無關的組字模型：`Auto`／`User` 片段與轉換後端介面。
//!
//! 組字區的所有權只有兩種：
//!
//! - [`Owner::Auto`]：系統可以重轉、改寫。
//! - [`Owner::User`]：使用者明確選字、切換假名或拆回讀音後固定，重轉不得
//!   改寫；只有在片段內部編輯時才退回 `Auto`。
//!
//! 語言後端只負責轉換連續的 [`AutoRun`]，不自己判斷哪一段被鎖定。注音與
//! 日文共用同一份 [`Composition`]，因此「哪一段是使用者選的」只有一個答案。
//!
//! 座標約定：
//!
//! - [`CompSegment::span`] 是 [`Composition::text`] 的 rune 半開區間。
//! - [`CompSegment::units`] 是這段對應的讀音單位數（注音音節或日文假名）。
//! - [`UserMark`] 以「同語言讀音單位」座標記錄所有權，重轉後依讀音重新套回
//!   片段，所以顯示文字改變不會把 User 片段洗掉。

use std::collections::HashMap;
use std::ops::Range;

use thiserror::Error;

use crate::lang::Mode;

/// 片段語言。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    /// 注音（台灣繁體中文）。
    Zh,
    /// 日文（假名讀音）。
    Ja,
    /// 英數、標點與其他不轉換的字面文字。
    Literal,
}

impl Lang {
    /// 由輸入模式對應到片段語言；英文與混合模式沒有單一語言。
    pub fn from_mode(mode: Mode) -> Option<Self> {
        match mode {
            Mode::Zh => Some(Self::Zh),
            Mode::Ja => Some(Self::Ja),
            Mode::En | Mode::Mixed => None,
        }
    }

    /// 這個語言是否會被送去轉換。
    pub fn convertible(self) -> bool {
        matches!(self, Self::Zh | Self::Ja)
    }

    /// 把同一段的多個讀音接成後端要的字串。
    ///
    /// 注音以空白分隔音節，日文直接串接假名。
    fn join_readings(self, readings: &[String]) -> String {
        match self {
            Self::Zh => readings.join(" "),
            Self::Ja | Self::Literal => readings.concat(),
        }
    }

    /// 比較所有權時用的讀音鍵：忽略注音音節之間的空白。
    fn reading_key(text: &str) -> String {
        text.chars().filter(|ch| !ch.is_whitespace()).collect()
    }
}

/// 片段所有權。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Owner {
    /// 系統可重轉。
    Auto,
    /// 使用者固定，重轉不得改寫。
    User,
}

impl Owner {
    /// 是否由使用者固定。
    pub fn is_user(self) -> bool {
        self == Self::User
    }
}

/// 組字區的一個片段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompSegment {
    /// 在 [`Composition::text`] 的 rune 範圍。
    pub span: Range<usize>,
    /// 對應讀音（注音音節以空白分隔，日文是假名）。
    pub reading: String,
    /// 讀音單位數（注音音節數或日文假名字元數）。
    pub units: usize,
    /// 語言。
    pub lang: Lang,
    /// 所有權。
    pub owner: Owner,
}

impl CompSegment {
    /// 建立片段。
    pub fn new(
        span: Range<usize>,
        reading: impl Into<String>,
        units: usize,
        lang: Lang,
        owner: Owner,
    ) -> Self {
        Self {
            span,
            reading: reading.into(),
            units,
            lang,
            owner,
        }
    }

    /// 顯示字數。
    pub fn chars(&self) -> usize {
        self.span.end.saturating_sub(self.span.start)
    }
}

/// 使用者固定片段的讀音座標紀錄。
///
/// `start`、`units` 是同一語言讀音單位的位移與長度；重轉後用讀音比對重新
/// 套回 [`CompSegment::owner`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserMark {
    /// 語言。
    pub lang: Lang,
    /// 讀音單位起點。
    pub start: usize,
    /// 讀音單位數。
    pub units: usize,
    /// 固定時的讀音；重轉後用來確認同一段讀音還在。
    pub reading: String,
    /// 固定時的顯示文字。
    pub surface: String,
}

impl UserMark {
    /// 讀音單位範圍。
    pub fn range(&self) -> Range<usize> {
        self.start..self.start.saturating_add(self.units)
    }
}

/// 語言無關的組字狀態。
///
/// 由 [`Composition::replace_segments`] 承接每次轉換的結果，並依讀音座標把
/// [`User`](Owner::User) 片段套回去；語言後端不需要自己保存鎖定。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Composition {
    text: String,
    cursor: usize,
    segments: Vec<CompSegment>,
    users: Vec<UserMark>,
}

impl Composition {
    /// 建立空組字。
    pub fn new() -> Self {
        Self::default()
    }

    /// 目前顯示字串。
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 游標位置（rune）。
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// 目前片段。
    pub fn segments(&self) -> &[CompSegment] {
        &self.segments
    }

    /// 使用者固定的讀音座標紀錄。
    pub fn user_marks(&self) -> &[UserMark] {
        &self.users
    }

    /// 是否沒有任何組字內容。
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// 清空組字。
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.segments.clear();
        self.users.clear();
    }

    /// 依序附加一個 `Auto` 片段。
    ///
    /// `text` 是顯示文字，`reading` 是讀音，`units` 是讀音單位數。
    pub fn push_auto(&mut self, text: &str, reading: &str, units: usize, lang: Lang) {
        let start = crate::output::rune_len(&self.text);
        self.text.push_str(text);
        let end = start + crate::output::rune_len(text);
        self.segments.push(CompSegment::new(
            start..end,
            reading,
            units,
            lang,
            Owner::Auto,
        ));
        self.cursor = end;
    }

    /// 在 rune 位置 `at` 插入一個 `Auto` 片段。
    ///
    /// `at` 落在既有片段內部時，該片段被切成兩段：讀音與單位留在右半，
    /// 左半是零單位的文字。呼叫端若在意讀音對應，應在之後用
    /// [`Composition::replace_segments`] 重建。
    pub fn insert(&mut self, at: usize, text: &str, reading: &str, units: usize, lang: Lang) {
        let at = at.min(crate::output::rune_len(&self.text));
        let mut split = None;
        for (index, segment) in self.segments.iter().enumerate() {
            if segment.span.start < at && at < segment.span.end {
                split = Some(index);
                break;
            }
        }
        if let Some(index) = split {
            let original = self.segments[index].clone();
            self.segments[index].span.end = at;
            self.segments[index].reading.clear();
            self.segments[index].units = 0;
            self.segments.insert(
                index + 1,
                CompSegment::new(
                    at..original.span.end,
                    original.reading,
                    original.units,
                    original.lang,
                    original.owner,
                ),
            );
        }
        let added = crate::output::rune_len(text);
        let byte = byte_at_rune(&self.text, at);
        self.text.insert_str(byte, text);
        for segment in &mut self.segments {
            if segment.span.start >= at {
                segment.span.start += added;
                segment.span.end += added;
            }
        }
        let index = self
            .segments
            .iter()
            .position(|segment| segment.span.start >= at + added)
            .unwrap_or(self.segments.len());
        self.segments.insert(
            index,
            CompSegment::new(at..at + added, reading, units, lang, Owner::Auto),
        );
        if self.cursor >= at {
            self.cursor += added;
        }
    }

    /// 刪除 rune 範圍 `range` 的文字與片段，回傳被刪掉的字串。
    ///
    /// 與刪除範圍重疊的使用者片段會退回 `Auto`；呼叫端通常在之後重建片段。
    pub fn delete_range(&mut self, range: Range<usize>) -> String {
        let start = range.start.min(crate::output::rune_len(&self.text));
        let end = range.end.min(crate::output::rune_len(&self.text));
        if start >= end {
            return String::new();
        }
        let start_byte = byte_at_rune(&self.text, start);
        let end_byte = byte_at_rune(&self.text, end);
        let removed = self.text[start_byte..end_byte].to_string();
        self.text.replace_range(start_byte..end_byte, "");
        let width = end - start;
        let mut kept = Vec::with_capacity(self.segments.len());
        for mut segment in std::mem::take(&mut self.segments) {
            if segment.span.end <= start {
                kept.push(segment);
                continue;
            }
            if segment.span.start >= end {
                segment.span.start -= width;
                segment.span.end -= width;
                kept.push(segment);
                continue;
            }
            // 部分重疊：留下沒被刪到的文字，但讀音已不可靠。
            let new_start = segment.span.start.min(start);
            let new_end = segment.span.end.saturating_sub(width).max(new_start);
            if new_start < new_end {
                segment.span = new_start..new_end;
                segment.reading.clear();
                segment.units = 0;
                segment.owner = Owner::Auto;
                kept.push(segment);
            }
        }
        self.segments = kept;
        self.cursor = self
            .cursor
            .saturating_sub(width)
            .min(crate::output::rune_len(&self.text));
        self.prune_marks();
        removed
    }

    /// 移動游標到 rune 位置 `to`。
    pub fn move_cursor(&mut self, to: usize) {
        self.cursor = to.min(crate::output::rune_len(&self.text));
    }

    /// 把 `range` 覆蓋的片段標成 `User`。
    ///
    /// 範圍會往外擴到完整片段，避免所有權只蓋半個詞。回傳實際標記的字數。
    pub fn mark_user(&mut self, range: Range<usize>) -> usize {
        let start = range.start.min(crate::output::rune_len(&self.text));
        let end = range.end.min(crate::output::rune_len(&self.text));
        if start >= end {
            return 0;
        }
        let mut marked = 0;
        let mut groups: Vec<(Lang, Vec<usize>)> = Vec::new();
        for (index, segment) in self.segments.iter().enumerate() {
            if segment.span.end <= start || segment.span.start >= end {
                continue;
            }
            marked += segment.chars();
            match groups.last_mut() {
                Some((lang, indices)) if *lang == segment.lang => indices.push(index),
                _ => groups.push((segment.lang, vec![index])),
            }
        }
        for (lang, indices) in groups {
            let unit_start = self.unit_offset(lang, indices[0]);
            let units: usize = indices
                .iter()
                .map(|&index| self.segments[index].units)
                .sum();
            if units == 0 {
                continue;
            }
            let readings: Vec<String> = indices
                .iter()
                .map(|&index| self.segments[index].reading.clone())
                .collect();
            let reading = lang.join_readings(&readings);
            let surface = self.span_text(
                self.segments[indices[0]].span.start
                    ..self.segments[*indices.last().unwrap()].span.end,
            );
            for &index in &indices {
                self.segments[index].owner = Owner::User;
            }
            self.add_mark(UserMark {
                lang,
                start: unit_start,
                units,
                reading,
                surface,
            });
        }
        marked
    }

    /// 把 `range` 覆蓋的使用者片段退回 `Auto`。
    ///
    /// 回傳是否有片段被解除。
    pub fn demote_user(&mut self, range: Range<usize>) -> bool {
        let start = range.start.min(crate::output::rune_len(&self.text));
        let end = range.end.min(crate::output::rune_len(&self.text));
        let before = self.users.len();
        let mut units = Vec::new();
        for (index, segment) in self.segments.iter().enumerate() {
            if segment.span.end <= start || segment.span.start >= end || segment.units == 0 {
                continue;
            }
            let start = self.unit_offset(segment.lang, index);
            units.push((segment.lang, start, segment.units));
        }
        self.users.retain(|mark| {
            !units.iter().any(|&(lang, start, units)| {
                lang == mark.lang
                    && start < mark.start.saturating_add(mark.units)
                    && mark.start < start.saturating_add(units)
            })
        });
        let changed = self.users.len() != before;
        if changed {
            self.apply_owners();
        }
        changed
    }

    /// 依讀音單位標記使用者片段；語言路徑用這個入口。
    pub fn mark_user_units(
        &mut self,
        lang: Lang,
        start: usize,
        units: usize,
        reading: &str,
        surface: &str,
    ) -> bool {
        if units == 0 || !lang.convertible() {
            return false;
        }
        // 片段可能比這次選取更長（例如詞庫把兩個音節合成一個詞）；先切出
        // 讀音單位邊界，所有權才能精確落在使用者選的範圍。
        self.ensure_unit_boundaries(lang, start, units);
        let mut covered = 0;
        let mut touched = Vec::new();
        for (index, segment) in self.segments.iter().enumerate() {
            if segment.lang != lang {
                continue;
            }
            if covered >= start && covered + segment.units <= start + units {
                touched.push(index);
            }
            covered += segment.units;
        }
        if touched.is_empty() {
            return false;
        }
        for &index in &touched {
            self.segments[index].owner = Owner::User;
        }
        self.add_mark(UserMark {
            lang,
            start,
            units,
            reading: reading.to_string(),
            surface: surface.to_string(),
        });
        true
    }

    /// 解除一段讀音座標上的使用者標記。
    pub fn demote_user_units(&mut self, lang: Lang, start: usize, units: usize) -> bool {
        let end = start.saturating_add(units);
        let before = self.users.len();
        self.users
            .retain(|mark| mark.lang != lang || mark.range().end <= start || mark.start >= end);
        let changed = self.users.len() != before;
        if changed {
            self.apply_owners();
        }
        changed
    }

    /// 編輯一段讀音之後平移使用者標記。
    ///
    /// `at` 是讀音單位位置，`delta` 是插入（正）或刪除（負）的單位數。完全在
    /// 編輯點之前的不動；完全在之後的平移；跨過編輯點的直接退回 `Auto`。
    pub fn shift_units(&mut self, lang: Lang, at: usize, delta: isize) {
        let before = self.users.len();
        let mut changed = false;
        for mark in &mut self.users {
            if mark.lang != lang {
                continue;
            }
            let end = mark.start.saturating_add(mark.units);
            if end <= at {
                continue;
            }
            let after = if delta < 0 {
                mark.start > at
            } else {
                mark.start >= at
            };
            if after {
                mark.start = mark.start.saturating_add_signed(delta);
                changed = true;
            } else {
                mark.units = 0;
            }
        }
        self.users.retain(|mark| mark.units > 0);
        if changed || self.users.len() != before {
            self.apply_owners();
        }
    }

    /// 只保留讀音前綴 `units` 內的使用者標記。
    pub fn retain_units_through(&mut self, lang: Lang, units: usize) {
        let before = self.users.len();
        self.users
            .retain(|mark| mark.lang != lang || mark.start.saturating_add(mark.units) <= units);
        if self.users.len() != before {
            self.apply_owners();
        }
    }

    /// 以新的顯示結果重建片段，並依讀音把使用者標記套回去。
    ///
    /// 這是每次轉換後唯一的寫回點：標記的讀音在新片段裡的同一段單位位置
    /// 還完全相同才保留，否則丟棄（編輯過的片段退回 `Auto`）。
    pub fn replace_segments(&mut self, text: String, cursor: usize, segments: Vec<CompSegment>) {
        self.text = text;
        self.cursor = cursor.min(crate::output::rune_len(&self.text));
        self.segments = segments;
        self.rematch_marks();
    }

    /// 回傳目前所有連續的 `Auto` 區段。
    ///
    /// 只在同一語言、相鄰、而且都有讀音單位時連成一段；`User` 片段、字面
    /// 文字與其他語言的片段都會切開區段。
    pub fn auto_runs(&self) -> Vec<AutoRun> {
        let mut runs: Vec<AutoRun> = Vec::new();
        for segment in &self.segments {
            if segment.owner.is_user()
                || !segment.lang.convertible()
                || segment.units == 0
                || segment.chars() == 0
            {
                continue;
            }
            let continues = runs.last().is_some_and(|run: &AutoRun| {
                run.lang == segment.lang && run.span.end == segment.span.start
            });
            if continues {
                let run = runs.last_mut().expect("已確認有 run");
                run.span.end = segment.span.end;
                run.boundaries.push(run.units);
                run.units += segment.units;
                run.readings.push(segment.reading.clone());
                run.reading = run.lang.join_readings(&run.readings);
            } else {
                runs.push(AutoRun {
                    span: segment.span.clone(),
                    reading: segment.reading.clone(),
                    readings: vec![segment.reading.clone()],
                    units: segment.units,
                    lang: segment.lang,
                    boundaries: Vec::new(),
                });
            }
        }
        runs
    }

    /// 一段 `AutoRun` 的前後固定文字。
    ///
    /// 左文是這段之前的所有組字文字，右文是之後的所有文字；`User` 片段與
    /// 字面文字都自然落在文脈裡。
    pub fn context_for(&self, run: &AutoRun) -> ConvertContext<'_> {
        let start = byte_at_rune(&self.text, run.span.start);
        let end = byte_at_rune(&self.text, run.span.end);
        ConvertContext {
            left: &self.text[..start],
            right: &self.text[end..],
        }
    }

    /// 把後端的轉換結果寫回這段 `Auto`。
    ///
    /// 轉換結果的讀音單位總數必須等於 `run.units`，否則回傳
    /// [`ConvertError::UnitMismatch`]，不改動任何狀態。
    pub fn apply_conversion(
        &mut self,
        run: &AutoRun,
        converted: &[Converted],
    ) -> Result<(), ConvertError> {
        let units: usize = converted.iter().map(|item| item.units).sum();
        if units != run.units {
            return Err(ConvertError::UnitMismatch {
                expected: run.units,
                got: units,
            });
        }
        if converted.is_empty() {
            return Err(ConvertError::Empty);
        }
        self.replace_run(run, converted)
    }

    /// 是否存在任何 `Auto` 區段。
    pub fn has_auto(&self) -> bool {
        self.segments
            .iter()
            .any(|segment| segment.owner == Owner::Auto && segment.units > 0)
    }

    /// 某語言目前的完整讀音字串。
    pub fn reading(&self, lang: Lang) -> String {
        let readings: Vec<String> = self
            .segments
            .iter()
            .filter(|segment| segment.lang == lang)
            .map(|segment| segment.reading.clone())
            .collect();
        lang.join_readings(&readings)
    }

    /// 某語言目前有幾個讀音單位。
    pub fn unit_len(&self, lang: Lang) -> usize {
        self.segments
            .iter()
            .filter(|segment| segment.lang == lang)
            .map(|segment| segment.units)
            .sum()
    }

    /// 片段索引對應的讀音單位起點。
    pub fn unit_offset(&self, lang: Lang, index: usize) -> usize {
        self.segments
            .iter()
            .take(index)
            .filter(|segment| segment.lang == lang)
            .map(|segment| segment.units)
            .sum()
    }

    fn span_text(&self, span: Range<usize>) -> String {
        let start = byte_at_rune(&self.text, span.start);
        let end = byte_at_rune(&self.text, span.end);
        self.text.get(start..end).unwrap_or_default().to_string()
    }

    /// 在讀音單位 `start` 與 `start + units` 切出片段邊界。
    fn ensure_unit_boundaries(&mut self, lang: Lang, start: usize, units: usize) {
        let end = start.saturating_add(units);
        for boundary in [start, end] {
            if boundary == 0 {
                continue;
            }
            let mut covered = 0usize;
            let mut found = None;
            for (index, segment) in self.segments.iter().enumerate() {
                if segment.lang != lang {
                    continue;
                }
                let seg_start = covered;
                covered += segment.units;
                if seg_start < boundary && boundary < covered {
                    found = Some((index, boundary - seg_start));
                    break;
                }
            }
            if let Some((index, left_units)) = found {
                self.split_segment(index, left_units);
            }
        }
    }

    /// 把片段依讀音單位切成兩段，文字範圍按字數比例分。
    fn split_segment(&mut self, index: usize, left_units: usize) {
        let Some(segment) = self.segments.get(index).cloned() else {
            return;
        };
        if left_units == 0 || left_units >= segment.units {
            return;
        }
        let chars = segment.chars();
        let boundary = (left_units * chars + segment.units / 2) / segment.units;
        let boundary = boundary.min(chars);
        let split_at = segment.span.start + boundary;
        let (left_reading, right_reading) =
            split_reading(segment.lang, &segment.reading, segment.units, left_units);
        self.segments[index] = CompSegment::new(
            segment.span.start..split_at,
            left_reading,
            left_units,
            segment.lang,
            segment.owner,
        );
        self.segments.insert(
            index + 1,
            CompSegment::new(
                split_at..segment.span.end,
                right_reading,
                segment.units - left_units,
                segment.lang,
                segment.owner,
            ),
        );
    }

    fn add_mark(&mut self, mark: UserMark) {
        let end = mark.start.saturating_add(mark.units);
        self.users.retain(|existing| {
            !(existing.lang == mark.lang
                && existing.start < end
                && mark.start < existing.start.saturating_add(existing.units))
        });
        self.users.push(mark);
        self.users.sort_by(|a, b| {
            (lang_order(a.lang), a.start, a.units).cmp(&(lang_order(b.lang), b.start, b.units))
        });
        self.apply_owners();
    }

    /// 依目前文字與標記重新套用 `owner`；標記本身不動。
    fn apply_owners(&mut self) {
        for segment in &mut self.segments {
            segment.owner = Owner::Auto;
        }
        let marks = self.users.clone();
        for mark in &marks {
            let mut covered = 0;
            for segment in &mut self.segments {
                if segment.lang != mark.lang {
                    continue;
                }
                let start = covered;
                covered += segment.units;
                let end = start + segment.units;
                if start >= mark.start && end <= mark.start.saturating_add(mark.units) {
                    segment.owner = Owner::User;
                }
            }
        }
    }

    fn rematch_marks(&mut self) {
        let marks = std::mem::take(&mut self.users);
        let axes = self.unit_axes();
        let mut kept = Vec::new();
        for mark in marks {
            let Some(axis) = axes.get(&mark.lang) else {
                continue;
            };
            let Some((reading, surface)) = axis.reading_in(mark.start, mark.units) else {
                continue;
            };
            if Lang::reading_key(&reading) == Lang::reading_key(&mark.reading) {
                kept.push(UserMark {
                    reading,
                    surface,
                    ..mark
                });
            }
        }
        self.users = kept;
        self.apply_owners();
    }

    fn unit_axes(&self) -> HashMap<Lang, UnitAxis<'_>> {
        let mut axes: HashMap<Lang, UnitAxis<'_>> = HashMap::new();
        for segment in &self.segments {
            if !segment.lang.convertible() {
                continue;
            }
            axes.entry(segment.lang)
                .or_insert_with(|| UnitAxis {
                    text: &self.text,
                    segments: Vec::new(),
                })
                .segments
                .push(segment);
        }
        axes
    }

    fn prune_marks(&mut self) {
        let mut axes = self.unit_axes();
        let keep: Vec<bool> = self
            .users
            .iter()
            .map(|mark| {
                axes.get_mut(&mark.lang)
                    .and_then(|axis| axis.reading_in(mark.start, mark.units))
                    .is_some()
            })
            .collect();
        let mut keep = keep.into_iter();
        self.users.retain(|_| keep.next().unwrap_or(false));
        self.apply_owners();
    }

    fn replace_run(&mut self, run: &AutoRun, converted: &[Converted]) -> Result<(), ConvertError> {
        let start_byte = byte_at_rune(&self.text, run.span.start);
        let end_byte = byte_at_rune(&self.text, run.span.end);
        let mut new_text = String::new();
        for item in converted {
            new_text.push_str(&item.text);
        }
        let old_chars = run.span.end - run.span.start;
        let new_chars = crate::output::rune_len(&new_text);
        self.text.replace_range(start_byte..end_byte, &new_text);
        let delta = new_chars as isize - old_chars as isize;
        let mut segments = Vec::with_capacity(self.segments.len());
        for segment in std::mem::take(&mut self.segments) {
            if segment.span.end <= run.span.start {
                segments.push(segment);
            } else if segment.span.start >= run.span.end {
                segments.push(CompSegment {
                    span: shift_range(segment.span, delta),
                    ..segment
                });
            }
        }
        let mut at = run.span.start;
        let mut inserted = Vec::new();
        for item in converted {
            let width = crate::output::rune_len(&item.text);
            inserted.push(CompSegment::new(
                at..at + width,
                item.reading.clone(),
                item.units,
                item.lang,
                Owner::Auto,
            ));
            at += width;
        }
        let insert_at = segments
            .iter()
            .position(|segment| segment.span.start >= run.span.start)
            .unwrap_or(segments.len());
        for (offset, segment) in inserted.into_iter().enumerate() {
            segments.insert(insert_at + offset, segment);
        }
        self.segments = segments;
        if self.cursor >= run.span.end {
            self.cursor = self.cursor.saturating_add_signed(delta);
        } else if self.cursor > run.span.start {
            self.cursor = run.span.start + new_chars;
        }
        self.cursor = self.cursor.min(crate::output::rune_len(&self.text));
        self.rematch_marks();
        Ok(())
    }
}

/// 依讀音單位把一段讀音切成左右兩份。
fn split_reading(lang: Lang, reading: &str, units: usize, left_units: usize) -> (String, String) {
    match lang {
        Lang::Zh => {
            let tokens: Vec<&str> = reading.split_whitespace().collect();
            if tokens.len() == units {
                return (
                    tokens[..left_units].join(" "),
                    tokens[left_units..].join(" "),
                );
            }
        }
        Lang::Ja | Lang::Literal => {
            let chars: Vec<char> = reading.chars().collect();
            if chars.len() == units {
                return (
                    chars[..left_units].iter().collect(),
                    chars[left_units..].iter().collect(),
                );
            }
        }
    }
    // 讀音對不上單位數時放棄精確切分；所有權仍以單位範圍為準。
    (reading.to_string(), String::new())
}

fn lang_order(lang: Lang) -> u8 {
    match lang {
        Lang::Zh => 0,
        Lang::Ja => 1,
        Lang::Literal => 2,
    }
}

fn shift_range(span: Range<usize>, delta: isize) -> Range<usize> {
    let start = span.start.saturating_add_signed(delta);
    let end = span.end.saturating_add_signed(delta);
    start..end
}

fn byte_at_rune(text: &str, rune: usize) -> usize {
    text.char_indices()
        .nth(rune)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len())
}

/// 同一語言的讀音單位軸，用來把 `UserMark` 對回片段。
struct UnitAxis<'a> {
    text: &'a str,
    segments: Vec<&'a CompSegment>,
}

impl UnitAxis<'_> {
    /// 讀音單位範圍對應的讀音與顯示文字；範圍必須完整落在片段邊界上。
    fn reading_in(&self, start: usize, units: usize) -> Option<(String, String)> {
        if units == 0 {
            return None;
        }
        let end = start.saturating_add(units);
        let mut cursor = 0usize;
        let mut readings = Vec::new();
        let mut surface_start = None;
        let mut surface_end = 0;
        for segment in &self.segments {
            let seg_start = cursor;
            cursor += segment.units;
            if cursor <= start || seg_start >= end {
                continue;
            }
            if seg_start < start || cursor > end {
                return None;
            }
            readings.push(segment.reading.clone());
            surface_start.get_or_insert(segment.span.start);
            surface_end = segment.span.end;
        }
        let lang = self.segments.first()?.lang;
        let start = surface_start?;
        let start_byte = byte_at_rune(self.text, start);
        let end_byte = byte_at_rune(self.text, surface_end);
        let surface = self.text.get(start_byte..end_byte)?.to_string();
        Some((lang.join_readings(&readings), surface))
    }
}

/// 一段連續的 `Auto` 讀音。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoRun {
    /// 在 [`Composition::text`] 的 rune 範圍。
    pub span: Range<usize>,
    /// 轉換用的讀音（注音以空白分隔音節，日文直接串接）。
    pub reading: String,
    /// 逐片段的讀音，給需要邊界的後端。
    pub readings: Vec<String>,
    /// 讀音單位總數。
    pub units: usize,
    /// 語言。
    pub lang: Lang,
    /// 片段邊界（讀音單位累計位移，不含 0 與結尾）。
    pub boundaries: Vec<usize>,
}

impl AutoRun {
    /// 建議的片段邊界。
    pub fn boundaries(&self) -> &[usize] {
        &self.boundaries
    }
}

/// 後端轉換所需的前後已固定文字。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConvertContext<'a> {
    /// 這段之前的固定文字。
    pub left: &'a str,
    /// 這段之後的固定文字。
    pub right: &'a str,
}

/// 後端回傳的一段轉換結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converted {
    /// 這段對應的讀音。
    pub reading: String,
    /// 轉換後文字。
    pub text: String,
    /// 讀音單位數。
    pub units: usize,
    /// 語言。
    pub lang: Lang,
}

impl Converted {
    /// 建立一段轉換結果。
    pub fn new(
        reading: impl Into<String>,
        text: impl Into<String>,
        units: usize,
        lang: Lang,
    ) -> Self {
        Self {
            reading: reading.into(),
            text: text.into(),
            units,
            lang,
        }
    }
}

/// 轉換後端錯誤。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConvertError {
    /// 後端不支援這個語言。
    #[error("轉換後端不支援語言 {0:?}")]
    Unsupported(Lang),
    /// 沒有轉換結果。
    #[error("轉換後端回傳空結果")]
    Empty,
    /// 讀音單位數對不上。
    #[error("轉換單位數不符：預期 {expected}，實際 {got}")]
    UnitMismatch {
        /// 這段 `AutoRun` 的單位數。
        expected: usize,
        /// 後端回傳的單位數。
        got: usize,
    },
    /// 後端自己的失敗。
    #[error("轉換失敗：{0}")]
    Backend(String),
}

/// 語言後端只需要轉換一段連續的 `Auto` 讀音。
///
/// 後端不得改寫 `User` 片段；`run` 保證整段都是 `Auto`，`ctx` 帶前後已固定
/// 的文字。結果的讀音單位總數必須等於 `run.units`，由
/// [`Composition::apply_conversion`] 驗證。
pub trait ConvertBackend {
    /// 轉換一段 `Auto` 讀音。
    fn convert(
        &mut self,
        run: &AutoRun,
        ctx: &ConvertContext<'_>,
    ) -> Result<Vec<Converted>, ConvertError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zh(text: &str, reading: &str, units: usize) -> (String, String, usize, Lang) {
        (text.to_string(), reading.to_string(), units, Lang::Zh)
    }

    fn composition() -> Composition {
        let mut c = Composition::new();
        let (text, reading, units, lang) = zh("台北", "ㄊㄞˊ ㄅㄟˇ", 2);
        c.push_auto(&text, &reading, units, lang);
        let (text, reading, units, lang) = zh("好", "ㄏㄠˇ", 1);
        c.push_auto(&text, &reading, units, lang);
        c
    }

    #[test]
    fn auto_runs_join_adjacent_same_language_segments() {
        let mut c = Composition::new();
        c.push_auto("台", "ㄊㄞˊ", 1, Lang::Zh);
        c.push_auto("北", "ㄅㄟˇ", 1, Lang::Zh);
        c.push_auto("東京", "とうきょう", 5, Lang::Ja);
        let runs = c.auto_runs();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].reading, "ㄊㄞˊ ㄅㄟˇ");
        assert_eq!(runs[0].units, 2);
        assert_eq!(runs[0].boundaries, vec![1]);
        assert_eq!(runs[1].lang, Lang::Ja);
        assert_eq!(runs[1].reading, "とうきょう");
    }

    #[test]
    fn user_segment_splits_runs_and_appears_in_context() {
        let mut c = composition();
        c.mark_user(0..1);
        let runs = c.auto_runs();
        assert_eq!(runs.len(), 1, "User 片段切開前後的 Auto");
        assert_eq!(runs[0].reading, "ㄅㄟˇ ㄏㄠˇ");
        let ctx = c.context_for(&runs[0]);
        assert_eq!(ctx.left, "台");
        assert_eq!(ctx.right, "");
    }

    #[test]
    fn user_segment_survives_replace_segments() {
        let mut c = composition();
        c.mark_user(0..2);
        assert_eq!(c.user_marks().len(), 1);
        let segments = vec![
            CompSegment::new(0..1, "ㄊㄞˊ", 1, Lang::Zh, Owner::Auto),
            CompSegment::new(1..2, "ㄅㄟˇ", 1, Lang::Zh, Owner::Auto),
            CompSegment::new(2..3, "ㄏㄠˋ", 1, Lang::Zh, Owner::Auto),
        ];
        c.replace_segments("台北號".to_string(), 3, segments);
        assert_eq!(c.segments()[0].owner, Owner::User);
        assert_eq!(c.segments()[1].owner, Owner::User);
        assert_eq!(c.segments()[2].owner, Owner::Auto);
        assert_eq!(c.user_marks()[0].surface, "台北");
    }

    #[test]
    fn editing_inside_user_segment_demotes_it() {
        let mut c = composition();
        c.mark_user(0..2);
        // 片段內換了一個音節：同一段讀音單位已經不同。
        let segments = vec![
            CompSegment::new(0..1, "ㄊㄞˊ", 1, Lang::Zh, Owner::Auto),
            CompSegment::new(1..2, "ㄅㄟ", 1, Lang::Zh, Owner::Auto),
        ];
        c.replace_segments("台北".to_string(), 2, segments);
        assert!(c.user_marks().is_empty(), "讀音變了就要退回 Auto");
        assert!(c
            .segments()
            .iter()
            .all(|segment| segment.owner == Owner::Auto));
    }

    #[test]
    fn shift_units_moves_user_mark_after_edit_point() {
        let mut c = composition();
        c.mark_user(0..1);
        c.shift_units(Lang::Zh, 0, 1);
        assert_eq!(c.user_marks()[0].start, 1);
        c.shift_units(Lang::Zh, 1, 1);
        assert_eq!(c.user_marks()[0].start, 2);
        // 跨過編輯點的標記直接解除。
        c.mark_user_units(Lang::Zh, 0, 2, "ㄊㄞˊ ㄅㄟˇ", "台北");
        c.shift_units(Lang::Zh, 1, -1);
        assert!(c.user_marks().is_empty());
    }

    #[test]
    fn demote_user_units_returns_segment_to_auto() {
        let mut c = composition();
        c.mark_user_units(Lang::Zh, 0, 1, "ㄊㄞˊ", "台");
        assert_eq!(c.segments()[0].owner, Owner::User);
        assert!(c.demote_user_units(Lang::Zh, 0, 1));
        assert_eq!(c.segments()[0].owner, Owner::Auto);
        assert!(c.user_marks().is_empty());
    }

    #[test]
    fn apply_conversion_writes_text_and_keeps_user_segments() {
        let mut c = Composition::new();
        c.push_auto("とうきょう", "とうきょう", 5, Lang::Ja);
        c.push_auto("に", "に", 1, Lang::Ja);
        c.mark_user(5..6);
        let runs = c.auto_runs();
        assert_eq!(runs.len(), 1);
        let converted = vec![
            Converted::new("とう", "東", 2, Lang::Ja),
            Converted::new("きょう", "京", 3, Lang::Ja),
        ];
        c.apply_conversion(&runs[0], &converted).expect("單位相符");
        assert_eq!(c.text(), "東京に");
        assert_eq!(c.segments()[0].owner, Owner::Auto);
        assert_eq!(c.segments()[2].owner, Owner::User, "後面的 User 片段不變");
        assert_eq!(c.segments()[2].span, 2..3);
    }

    #[test]
    fn apply_conversion_rejects_unit_mismatch() {
        let mut c = Composition::new();
        c.push_auto("あ", "あ", 1, Lang::Ja);
        let run = c.auto_runs().remove(0);
        let err = c
            .apply_conversion(&run, &[Converted::new("あ", "亜", 2, Lang::Ja)])
            .unwrap_err();
        assert_eq!(
            err,
            ConvertError::UnitMismatch {
                expected: 1,
                got: 2
            }
        );
        assert_eq!(c.text(), "あ", "失敗時不得改動組字");
    }

    #[test]
    fn literal_segment_breaks_auto_run() {
        let mut c = Composition::new();
        c.push_auto("台", "ㄊㄞˊ", 1, Lang::Zh);
        c.push_auto("，", "", 0, Lang::Literal);
        c.push_auto("北", "ㄅㄟˇ", 1, Lang::Zh);
        let runs = c.auto_runs();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].reading, "ㄊㄞˊ");
        assert_eq!(runs[1].reading, "ㄅㄟˇ");
    }

    #[test]
    fn marking_one_syllable_splits_a_longer_segment() {
        let mut c = Composition::new();
        c.push_auto("台北", "ㄊㄞˊ ㄅㄟˇ", 2, Lang::Zh);
        assert!(c.mark_user_units(Lang::Zh, 0, 1, "ㄊㄞˊ", "台"));
        assert_eq!(c.segments().len(), 2);
        assert_eq!(c.segments()[0].span, 0..1);
        assert_eq!(c.segments()[0].owner, Owner::User);
        assert_eq!(c.segments()[1].span, 1..2);
        assert_eq!(c.segments()[1].owner, Owner::Auto);
        assert_eq!(c.segments()[1].reading, "ㄅㄟˇ");
        let runs = c.auto_runs();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].reading, "ㄅㄟˇ");
    }

    #[test]
    fn insert_and_delete_keep_spans_aligned() {
        let mut c = composition();
        c.insert(1, "，", "", 0, Lang::Literal);
        assert_eq!(c.text(), "台，北好");
        assert_eq!(c.segments()[1].span, 1..2);
        assert_eq!(c.segments()[2].span, 2..3);
        c.move_cursor(2);
        c.delete_range(1..2);
        assert_eq!(c.text(), "台北好");
        assert_eq!(c.cursor(), 1);
    }
}
