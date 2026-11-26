//! 候選窗的版面狀態：列數、欄數與移動規則。
//!
//! 這裡只描述「清單怎麼排」；候選從哪裡來、顯示什麼文字都不在此模組。
//! 外殼拿到同一份版面參數就直接畫，不再依語言決定欄數或換行。
//!
//! 多欄時每一欄是一條直排，先填滿一欄的十列，再換下一欄。因此頁內索引
//! 換算成 `(列, 欄)` 是 `(index % rows, index / rows)`；數字鍵 `1`–`0` 是
//! 列號，只在目前這一欄內選取。

/// 候選窗預設列數。
pub const DEFAULT_ROWS: usize = 10;
/// 直向版面的欄數。
pub const VERTICAL_COLUMNS: usize = 1;
/// 按 `←`／`→` 展開後的欄數。
pub const EXPANDED_COLUMNS: usize = 4;
/// 欄數上限。
pub const MAX_COLUMNS: usize = EXPANDED_COLUMNS;

/// 一次呈現的候選版面。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CandidateLayout {
    rows: usize,
    columns: usize,
}

impl Default for CandidateLayout {
    fn default() -> Self {
        Self::new(DEFAULT_ROWS, VERTICAL_COLUMNS)
    }
}

impl CandidateLayout {
    /// 建立版面；列數至少 1，欄數夾在 1–[`MAX_COLUMNS`]。
    pub fn new(rows: usize, columns: usize) -> Self {
        Self {
            rows: rows.max(1),
            columns: columns.clamp(1, MAX_COLUMNS),
        }
    }

    /// 一頁最多幾列。
    pub fn rows(self) -> usize {
        self.rows
    }

    /// 一頁最多幾欄。
    pub fn columns(self) -> usize {
        self.columns
    }

    /// 一頁最多幾項。
    pub fn page_size(self) -> usize {
        self.rows * self.columns
    }

    /// 是否為橫向的四欄版面。
    pub fn expanded(self) -> bool {
        self.columns > VERTICAL_COLUMNS
    }

    /// 展開成 [`EXPANDED_COLUMNS`] 欄；已經是橫向時不變。
    pub fn expand(&mut self) {
        self.columns = EXPANDED_COLUMNS;
    }

    /// 回到直向版面。
    pub fn collapse(&mut self) {
        self.columns = VERTICAL_COLUMNS;
    }

    /// 這個索引在第幾頁（0 起算）。
    pub fn page(self, index: usize) -> usize {
        index / self.page_size().max(1)
    }

    /// 總頁數；沒有候選時回 0。
    pub fn pages(self, count: usize) -> usize {
        count.div_ceil(self.page_size().max(1))
    }

    /// 這個索引落在頁內第幾列（0 起算）。
    pub fn row(self, index: usize) -> usize {
        index % self.page_size().max(1) % self.rows.max(1)
    }

    /// 這個索引落在頁內第幾欄（0 起算）。
    pub fn column(self, index: usize) -> usize {
        index % self.page_size().max(1) / self.rows.max(1)
    }

    /// 這個索引落在頁內第幾列與第幾欄（都從 0 起算）。
    pub fn cell(self, index: usize) -> (usize, usize) {
        (self.row(index), self.column(index))
    }

    /// 由目前索引在同一個直排往下移一列，欄內頭尾相接。
    pub fn down(self, index: usize, count: usize) -> usize {
        self.move_in_column(index, count, 1)
    }

    /// 由目前索引在同一個直排往上移一列，欄內頭尾相接。
    pub fn up(self, index: usize, count: usize) -> usize {
        self.move_in_column(index, count, -1)
    }

    /// 由目前索引往左換一欄，停在同一個列號。
    pub fn left(self, index: usize, count: usize) -> usize {
        self.move_column(index, count, -1)
    }

    /// 由目前索引往右換一欄，停在同一個列號。
    pub fn right(self, index: usize, count: usize) -> usize {
        self.move_column(index, count, 1)
    }

    /// 目前這一欄在頁內的起始索引與實際項數。
    fn column_bounds(self, index: usize, count: usize) -> (usize, usize, usize) {
        let page_size = self.page_size().max(1);
        let page_start = index / page_size * page_size;
        let within = index.saturating_sub(page_start);
        let column = within / self.rows.max(1);
        let start = page_start + column * self.rows.max(1);
        let len = count.saturating_sub(start).min(self.rows.max(1));
        (start, len, within % self.rows.max(1))
    }

    fn move_in_column(self, index: usize, count: usize, delta: isize) -> usize {
        if count == 0 {
            return 0;
        }
        let index = index.min(count - 1);
        let (start, len, row) = self.column_bounds(index, count);
        if len == 0 {
            return index;
        }
        let len = len as isize;
        let mut next = (row as isize + delta) % len;
        if next < 0 {
            next += len;
        }
        start + next as usize
    }

    fn move_column(self, index: usize, count: usize, delta: isize) -> usize {
        if count == 0 {
            return 0;
        }
        let index = index.min(count - 1);
        let page_size = self.page_size().max(1);
        let page_start = index / page_size * page_size;
        let page_len = count.saturating_sub(page_start).min(page_size);
        let used_columns = page_len.div_ceil(self.rows.max(1)).max(1);
        let within = index.saturating_sub(page_start);
        let column = within / self.rows.max(1);
        let row = within % self.rows.max(1);
        let mut target = (column as isize + delta) % used_columns as isize;
        if target < 0 {
            target += used_columns as isize;
        }
        let start = page_start + target as usize * self.rows.max(1);
        let len = count.saturating_sub(start).min(self.rows.max(1));
        if len == 0 {
            return index;
        }
        start + row.min(len - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_candidate_rows_default_is_ten() {
        let layout = CandidateLayout::default();
        assert_eq!(layout.rows(), 10);
        assert_eq!(layout.columns(), 1);
        assert_eq!(layout.page_size(), 10);
    }

    #[test]
    fn test_two_column_down_moves_one_row() {
        let layout = CandidateLayout::new(DEFAULT_ROWS, 2);
        assert_eq!(layout.down(1, 25), 2, "兩欄時 ↓ 在同一欄換到下一列");
        assert_eq!(layout.up(2, 25), 1);
        assert_eq!(layout.down(9, 25), 0, "同一欄底部接回頂端");
    }

    #[test]
    fn test_four_column_down_moves_one_row() {
        let layout = CandidateLayout::new(DEFAULT_ROWS, 4);
        assert_eq!(layout.down(1, 45), 2, "四欄時 ↓ 只換到同欄下一列");
        assert_eq!(layout.up(2, 45), 1);
        assert_eq!(layout.down(11, 45), 12, "第二欄也是同一條直排");
    }

    #[test]
    fn columns_are_vertical_and_switch_at_the_same_row() {
        let layout = CandidateLayout::new(DEFAULT_ROWS, 4);
        assert_eq!(layout.cell(0), (0, 0));
        assert_eq!(layout.cell(10), (0, 1));
        assert_eq!(layout.cell(13), (3, 1));
        assert_eq!(layout.right(3, 45), 13, "→ 從欄 0 列 3 到欄 1 列 3");
        assert_eq!(layout.left(13, 45), 3, "← 回到欄 0 列 3");
    }

    #[test]
    fn expand_only_changes_columns_and_page_capacity() {
        let mut layout = CandidateLayout::default();
        layout.expand();
        assert_eq!(layout.columns(), 4);
        assert_eq!(layout.rows(), 10);
        assert_eq!(layout.page_size(), 40);
        assert_eq!(layout.cell(10), (0, 1));
        assert_eq!(layout.page(41), 1);
        layout.collapse();
        assert_eq!(layout, CandidateLayout::default());
    }
}
