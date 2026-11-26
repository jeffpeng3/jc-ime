//! 按鍵政策的固定優先序：候選介面 → lane 專屬 → 全域。
//!
//! `press()` 只把按鍵交給這裡一次，不在每個 handler 重新分派 lane。
//! 目前先收 `space`、`segments_press`、`backspace`、`delete_forward`、
//! `escape` 與 `tab`；其他鍵仍走原本的入口，後續再搬。

use super::*;

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
        } else if self.cursor > 0 {
            self.remove_syllable(self.cursor - 1);
            self.cursor = self.cursor.saturating_sub(1);
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
                    self.remove_syllable(self.cursor);
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
        let at = self.mixed_cursor.min(runes);
        if at >= runes {
            return;
        }
        let saved = self.mixed_cursor;
        self.mixed_cursor = at;
        let target = self.mixed_target_at_cursor();
        self.mixed_cursor = saved;
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
                self.mixed_cursor = at.min(len);
                return;
            }
        }
        if let Some(index) = self.mixed_segment_index_at_rune(at) {
            self.remove_mixed_segment(index);
        }
    }
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
