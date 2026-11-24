//! `Session` 與語言無關 [`Composition`] 的橋接。
//!
//! 所有權的唯一來源是 [`Session::composition`]：注音路徑與日文路徑都只讀
//! [`Composition::user_marks`] 決定哪一段不能改寫，再把結果推回各語言後端
//! 的原生表示（注音的 [`crate::decode::Locks`]、日文的
//! [`crate::ja::live::UserSegment`]）。兩邊不再各自保存「哪一段鎖定」。
//!
//! 每次轉換後的寫回點只有一個：[`Session::sync_composition_zh`] 與
//! [`Session::sync_composition_ja`] 都走 [`Composition::replace_segments`]，
//! 依讀音單位把 `User` 標記套回新片段；編輯過的片段因為讀音不同而自動退回
//! `Auto`。

use super::*;
use crate::composition::{CompSegment, Composition, Lang, Owner, UserMark};

/// 兩段讀音共同前綴的讀音單位數。
///
/// 日文以假名為單位，所以字元數就是單位數；編輯點之後的 `User` 標記一律
/// 失效。
pub(super) fn common_prefix_units(left: &str, right: &str) -> usize {
    left.chars()
        .zip(right.chars())
        .take_while(|(a, b)| a == b)
        .count()
}

impl Session {
    /// 目前的語言無關組字模型。
    pub fn composition(&self) -> &Composition {
        &self.composition
    }

    /// 目前連續的 `Auto` 區段；外殼可用來觀察哪些讀音會被送去轉換。
    pub fn auto_runs(&self) -> Vec<crate::composition::AutoRun> {
        self.composition.auto_runs()
    }

    /// 使用者固定片段的讀音座標紀錄。
    pub fn user_marks(&self) -> &[UserMark] {
        self.composition.user_marks()
    }

    /// 快照片段對應的所有權。
    ///
    /// 找不到對應片段時視為 `Auto`；外殼只把 `User` 當成不可改寫的提示，
    /// 真正的保護在 [`Session::composition`]。
    pub(super) fn segment_owners(&self, text: &str, segments: &[Segment]) -> Vec<Owner> {
        let composition = self.composition.segments();
        segments
            .iter()
            .map(|segment| {
                let start = crate::output::rune_offset(text, segment.start);
                composition
                    .iter()
                    .find(|candidate| {
                        candidate.span.start == start || candidate.span.contains(&start)
                    })
                    .map(|candidate| candidate.owner)
                    .unwrap_or(Owner::Auto)
            })
            .collect()
    }

    /// 依目前注音轉換結果重建 `Composition`。
    ///
    /// `readings` 是每個音節的讀音，`segs` 是最佳路徑的片段。片段顯示文字
    /// 取自 [`Session::base`]，所以 `segs` 的順序必須與 `base` 一致。
    ///
    /// 位移一律以 rune 累加。`base` 是轉換後的顯示字串，`seg.word` 是解碼
    /// 片段；早退路徑可能讓兩者寬度不同（例如 `base` 退回原始注音），用位元組
    /// 累加會切在多 byte 字元中間。
    pub(super) fn sync_composition_zh(&mut self, readings: &[String], segs: &[crate::decode::Seg]) {
        let base = self.base.clone();
        let base_runes = crate::output::rune_len(&base);
        let mut literal_offset = self.literal_offset.min(base.len());
        while literal_offset > 0 && !base.is_char_boundary(literal_offset) {
            literal_offset -= 1;
        }
        let literal_at = crate::output::rune_len(&base[..literal_offset]);
        let literal_width = crate::output::rune_len(&self.literal);
        let mut segments = Vec::with_capacity(segs.len() + 2);
        let mut at = 0usize;
        for seg in segs {
            let mut start = at.min(base_runes);
            at = at
                .saturating_add(crate::output::rune_len(&seg.word))
                .min(base_runes);
            let mut end = at;
            if !self.literal.is_empty() && start >= literal_at {
                start += literal_width;
                end += literal_width;
            }
            let from = seg.start.min(readings.len());
            let to = seg.end.min(readings.len()).max(from);
            segments.push(CompSegment::new(
                start..end,
                readings[from..to].join(" "),
                to - from,
                Lang::Zh,
                Owner::Auto,
            ));
        }
        if !self.literal.is_empty() {
            segments.push(CompSegment::new(
                literal_at..literal_at + literal_width,
                "",
                0,
                Lang::Literal,
                Owner::Auto,
            ));
        }
        let mut text = base;
        if !self.literal.is_empty() {
            text.insert_str(literal_offset, &self.literal);
        }
        if !self.pending.is_empty() {
            let start = crate::output::rune_len(&text);
            text.push_str(&self.pending);
            segments.push(CompSegment::new(
                start..start + crate::output::rune_len(&self.pending),
                "",
                0,
                Lang::Zh,
                Owner::Auto,
            ));
        }
        segments.sort_by_key(|segment| segment.span.start);
        let cursor = crate::output::rune_offset(&text, self.cursor_byte())
            .min(crate::output::rune_len(&text));
        self.composition.replace_segments(text, cursor, segments);
    }

    /// 從 `Composition` 產生注音解碼器的鎖定。
    ///
    /// `User` 標記的讀音起點就是音節索引；超出目前音節數的舊標記會自然被
    /// 丟掉，不需要另外清理。
    pub(super) fn sync_zh_locks(&mut self) {
        let count = self.syllables.len();
        let mut locks = crate::decode::Locks::new();
        for mark in self.composition.user_marks() {
            if mark.lang != Lang::Zh || mark.units == 0 {
                continue;
            }
            let end = mark.start.saturating_add(mark.units);
            if mark.start >= count || end > count {
                continue;
            }
            locks.insert(
                mark.start,
                crate::decode::Lock {
                    word: mark.surface.clone(),
                    len: mark.units,
                    ..crate::decode::Lock::default()
                },
            );
        }
        self.locks = locks;
    }

    /// 把一段音節標成使用者固定。
    ///
    /// 讀音由目前音節推導；`surface` 是固定在畫面上的文字。
    pub(super) fn mark_zh_user_syllables(
        &mut self,
        start: usize,
        len: usize,
        surface: &str,
    ) -> bool {
        if len == 0 || start.saturating_add(len) > self.syllables.len() {
            return false;
        }
        let reading = self
            .zh_readings(&self.syllables[start..start + len])
            .join(" ");
        self.composition
            .mark_user_units(Lang::Zh, start, len, &reading, surface)
    }

    /// 解除一段音節上的使用者標記。
    pub(super) fn demote_zh_syllables(&mut self, start: usize, len: usize) -> bool {
        self.composition.demote_user_units(Lang::Zh, start, len)
    }

    /// 音節插入或刪除後平移所有權；跨過編輯點的片段退回 `Auto`。
    pub(super) fn shift_zh_ownership(&mut self, at: usize, delta: isize) {
        self.composition.shift_units(Lang::Zh, at, delta);
    }

    /// 依目前日文轉換結果重建 `Composition`。
    ///
    /// 片段的讀音單位是假名字元數；尚未轉換的尾巴也保留成 `Auto`，但因為
    /// 沒有轉換結果，外殼看到的仍是假名。
    pub(super) fn sync_composition_ja(&mut self) {
        let base = self.base.clone();
        let units = self.romaji.units().to_vec();
        let reading: String = units.iter().map(Unit::as_str).collect();
        let mut segments: Vec<CompSegment> = Vec::new();
        let mut at = 0usize;
        if let Some(draft) = self.live.as_ref().and_then(|live| live.draft()) {
            for clause in &draft.clauses {
                let start = crate::output::rune_len(&base[..at.min(base.len())]);
                at = at.saturating_add(clause.surface.len()).min(base.len());
                let end = crate::output::rune_len(&base[..at]);
                let units = clause.reading.chars().count();
                segments.push(CompSegment::new(
                    start..end,
                    clause.reading.clone(),
                    units,
                    Lang::Ja,
                    Owner::Auto,
                ));
            }
        }
        let mut text = base;
        let covered: usize = segments.iter().map(|segment| segment.units).sum();
        if !text.is_empty() && segments.is_empty() {
            let chars = crate::output::rune_len(&text);
            segments.push(CompSegment::new(
                0..chars,
                reading.clone(),
                reading.chars().count(),
                Lang::Ja,
                Owner::Auto,
            ));
        } else if covered < units.len() {
            let tail: String = units[covered..].iter().map(Unit::as_str).collect();
            let start = crate::output::rune_len(&text);
            text.push_str(&tail);
            segments.push(CompSegment::new(
                start..start + crate::output::rune_len(&tail),
                reading.chars().skip(covered).collect::<String>(),
                units.len() - covered,
                Lang::Ja,
                Owner::Auto,
            ));
        }
        if !self.pending.is_empty() {
            let start = crate::output::rune_len(&text);
            text.push_str(&self.pending);
            segments.push(CompSegment::new(
                start..start + crate::output::rune_len(&self.pending),
                "",
                0,
                Lang::Ja,
                Owner::Auto,
            ));
        }
        segments.sort_by_key(|segment| segment.span.start);
        let cursor = crate::output::rune_offset(&text, self.ja_cursor.min(text.len()))
            .min(crate::output::rune_len(&text));
        self.composition.replace_segments(text, cursor, segments);
        self.sync_ja_users();
    }

    /// 把 `Composition` 的日文使用者片段推回逐字変換控制器。
    pub(super) fn sync_ja_users(&mut self) {
        let users: Vec<live::UserSegment> = self
            .composition
            .user_marks()
            .iter()
            .filter(|mark| mark.lang == Lang::Ja && mark.units > 0)
            .map(|mark| live::UserSegment {
                start: mark.start,
                units: mark.units,
                reading: mark.reading.clone(),
                surface: mark.surface.clone(),
            })
            .collect();
        if let Some(live) = self.live.as_mut() {
            live.set_user_segments(users);
        }
    }

    /// 依混合模式目前的分段重建 `Composition`。
    ///
    /// 注音片段是一個音節一個單位，英數字面片段不帶讀音；尚未完成的按鍵
    /// 也是零單位，不會被當成 `Auto` 讀音送出去。
    pub(super) fn sync_composition_mixed(&mut self) {
        if self.ascii.is_empty() {
            self.composition.clear();
            return;
        }
        let base = self.base.clone();
        let mut pieces = Vec::new();
        let mut at = 0usize;
        for piece in self.ascii.pieces() {
            let (text, lang, reading, units) = match piece {
                super::mixed::AsciiPiece::Zh(syllable) => {
                    let surface = self.decode_syllable(syllable);
                    (surface, Lang::Zh, syllable.as_str().to_string(), 1usize)
                }
                super::mixed::AsciiPiece::Literal(text) => {
                    (text.clone(), Lang::Literal, String::new(), 0usize)
                }
            };
            let start = crate::output::rune_len(&base[..at.min(base.len())]);
            at = at.saturating_add(text.len()).min(base.len());
            let end = crate::output::rune_len(&base[..at]);
            pieces.push(CompSegment::new(
                start..end,
                reading,
                units,
                lang,
                Owner::Auto,
            ));
        }
        let mut text = base;
        if !self.ascii.pending().is_empty() {
            let pending = self.ascii.display_pending(&self.lexicon);
            let start = crate::output::rune_len(&text);
            text.push_str(&pending);
            pieces.push(CompSegment::new(
                start..start + crate::output::rune_len(&pending),
                "",
                0,
                Lang::Literal,
                Owner::Auto,
            ));
        }
        pieces.sort_by_key(|segment| segment.span.start);
        let cursor = crate::output::rune_len(&text);
        self.composition.replace_segments(text, cursor, pieces);
    }

    /// 把控制器內部剛標記的使用者片段抄回 `Composition`。
    ///
    /// 選字、假名變換與拆回讀音都由後端回報實際的讀音單位範圍，再由
    /// `Composition` 統一保存。
    pub(super) fn capture_ja_users(&mut self) {
        let users = match self.live.as_ref() {
            Some(live) => live.user_segments().to_vec(),
            None => return,
        };
        for user in users {
            self.composition.mark_user_units(
                Lang::Ja,
                user.start,
                user.units,
                &user.reading,
                &user.surface,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::Seg;

    fn session_with_base(base: &str) -> Session {
        let mut session = Session::new(SessionConfig::new(crate::Mode::Zh));
        session.base = base.to_string();
        session
    }

    #[test]
    fn sync_composition_zh_uses_rune_offsets_when_widths_differ() {
        // `base` 退回原始注音（每個音節 6 到 8 bytes），`segs` 仍是轉換後的
        // 單字（每個 3 bytes）。舊碼累加位元組位移會在 9 切進 `ㄇ` 的中間。
        let mut session = session_with_base("ㄕㄣˊㄇㄧˋ的");
        let readings = [
            "ㄕㄣˊ".to_string(),
            "ㄇㄧˋ".to_string(),
            "ㄉㄜ˙".to_string(),
        ];
        let segs = [
            Seg::new("神", 0, 1),
            Seg::new("秘", 1, 2),
            Seg::new("的", 2, 3),
        ];
        session.sync_composition_zh(&readings, &segs);

        let runes = crate::output::rune_len(session.composition().text());
        let segments = session.composition().segments();
        assert_eq!(segments.len(), 3);
        assert!(segments.iter().all(|seg| seg.span.end <= runes));
        assert!(segments
            .windows(2)
            .all(|pair| pair[0].span.end <= pair[1].span.start));
    }

    #[test]
    fn sync_composition_zh_inserts_literal_after_multibyte_segments() {
        let mut session = session_with_base("神秘的");
        session.literal = "，".to_string();
        session.literal_offset = "神秘".len();
        let readings = [
            "ㄕㄣˊ".to_string(),
            "ㄇㄧˋ".to_string(),
            "ㄉㄜ˙".to_string(),
        ];
        let segs = [
            Seg::new("神", 0, 1),
            Seg::new("秘", 1, 2),
            Seg::new("的", 2, 3),
        ];
        session.sync_composition_zh(&readings, &segs);

        assert_eq!(session.composition().text(), "神秘，的");
        let spans: Vec<(usize, usize)> = session
            .composition()
            .segments()
            .iter()
            .map(|segment| (segment.span.start, segment.span.end))
            .collect();
        assert_eq!(spans, [(0, 1), (1, 2), (2, 3), (3, 4)]);
    }

    #[test]
    fn sync_composition_zh_clamps_segments_longer_than_base() {
        let mut session = session_with_base("神");
        let readings = ["ㄕㄣˊ".to_string(), "ㄇㄧˋ".to_string()];
        let segs = [Seg::new("神", 0, 1), Seg::new("秘", 1, 2)];
        session.sync_composition_zh(&readings, &segs);

        let runes = crate::output::rune_len(session.composition().text());
        assert_eq!(runes, 1);
        assert!(session
            .composition()
            .segments()
            .iter()
            .all(|seg| seg.span.end <= runes));
    }
}
