//! 候選窗的版面狀態：列數、欄數與移動規則。
//!
//! 這裡只描述「清單怎麼排」；候選從哪裡來、顯示什麼文字都不在此模組。
//! 外殼拿到同一份版面參數就直接畫，不再依語言決定欄數或換行。
//!
//! 多欄時每一欄是一條直排，先填滿一欄的十列，再換下一欄。因此頁內索引
//! 換算成 `(列, 欄)` 是 `(index % rows, index / rows)`；數字鍵 `1`–`0` 是
//! 列號，只在目前這一欄內選取。
//!
//! 移動規則：`↑`／`↓` 是整份清單的線性 `-1`／`+1`，跨欄、跨頁自然接續，
//! 到頭尾停住；`←`／`→` 是跳一整欄 `-rows`／`+rows`，不跨頁。直向單欄時
//! 左右不移動。

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

    /// 由目前索引往下移一項，跨欄時自然接到下一欄第一列；本頁最後一項
    /// 接到下一頁第一項。整個清單的最後一項停住，不回捲。
    pub fn down(self, index: usize, count: usize) -> usize {
        if count == 0 {
            return 0;
        }
        index.min(count - 1).saturating_add(1).min(count - 1)
    }

    /// 由目前索引往上移一項，跨欄時自然接到上一欄最後一列；本頁第一項
    /// 接到上一頁最後一項。整個清單的第一項停住，不回捲。
    pub fn up(self, index: usize, count: usize) -> usize {
        if count == 0 {
            return 0;
        }
        index.min(count - 1).saturating_sub(1)
    }

    /// 由目前索引往左跳一欄（十項），停在同一個列號。
    ///
    /// 直向單欄或已在本頁第一欄時不移動。
    pub fn left(self, index: usize, count: usize) -> usize {
        self.jump_column(index, count, -1)
    }

    /// 由目前索引往右跳一欄（十項），停在同一個列號。
    ///
    /// 直向單欄或已在本頁最後一欄時不移動。
    pub fn right(self, index: usize, count: usize) -> usize {
        self.jump_column(index, count, 1)
    }

    /// 目前索引所在頁的起點與終點（終點不含）。
    fn page_span(self, index: usize, count: usize) -> (usize, usize) {
        let page_size = self.page_size().max(1);
        let page_start = index / page_size * page_size;
        (page_start, (page_start + page_size).min(count))
    }

    fn jump_column(self, index: usize, count: usize, delta: isize) -> usize {
        if count == 0 {
            return 0;
        }
        let index = index.min(count - 1);
        if self.columns <= VERTICAL_COLUMNS {
            return index;
        }
        let target = if delta < 0 {
            index.checked_sub(self.rows)
        } else {
            index.checked_add(self.rows)
        };
        let Some(target) = target else {
            return index;
        };
        let (page_start, page_end) = self.page_span(index, count);
        if target < page_start || target >= page_end {
            return index;
        }
        target
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
        assert_eq!(layout.down(1, 25), 2, "↓ 一次一項");
        assert_eq!(layout.up(2, 25), 1);
        assert_eq!(layout.down(9, 25), 10, "第一欄底部接到第二欄第一列");
    }

    #[test]
    fn test_four_column_down_moves_one_row() {
        let layout = CandidateLayout::new(DEFAULT_ROWS, 4);
        assert_eq!(layout.down(1, 45), 2, "↓ 一次一項");
        assert_eq!(layout.up(2, 45), 1);
        assert_eq!(layout.down(11, 45), 12, "第二欄也是線性 ±1");
    }

    #[test]
    fn test_down_moves_into_next_column() {
        let layout = CandidateLayout::new(DEFAULT_ROWS, 4);
        assert_eq!(layout.down(9, 45), 10, "第 1 欄第 10 列 -> 第 2 欄第 1 列");
    }

    #[test]
    fn test_up_moves_into_previous_column() {
        let layout = CandidateLayout::new(DEFAULT_ROWS, 4);
        assert_eq!(layout.up(10, 45), 9, "第 2 欄第 1 列 -> 第 1 欄第 10 列");
    }

    #[test]
    fn test_left_right_jump_ten() {
        let layout = CandidateLayout::new(DEFAULT_ROWS, 4);
        assert_eq!(layout.right(3, 45), 13, "→ 跳一整欄");
        assert_eq!(layout.left(13, 45), 3, "← 跳一整欄");
    }

    #[test]
    fn test_down_crosses_column_boundary() {
        let layout = CandidateLayout::new(DEFAULT_ROWS, EXPANDED_COLUMNS);
        assert_eq!(layout.down(9, 80), 10, "第 1 欄第 10 列 -> 第 2 欄第 1 列");
        assert_eq!(layout.down(19, 80), 20, "第 2 欄 -> 第 3 欄");
        assert_eq!(layout.down(29, 80), 30, "第 3 欄 -> 第 4 欄");
    }

    #[test]
    fn test_up_crosses_column_boundary_backwards() {
        let layout = CandidateLayout::new(DEFAULT_ROWS, EXPANDED_COLUMNS);
        assert_eq!(layout.up(10, 80), 9, "第 2 欄第 1 列 -> 第 1 欄第 10 列");
        assert_eq!(layout.up(20, 80), 19);
        assert_eq!(layout.up(30, 80), 29);
    }

    #[test]
    fn test_down_at_page_end_behavior() {
        // 線性 ±1 跨頁，頭尾停住；不再頁內環繞。
        let vertical = CandidateLayout::default();
        assert_eq!(vertical.down(9, 25), 10, "直向頁尾接到下一頁第一項");
        assert_eq!(vertical.up(10, 25), 9, "下一頁第一項接回上一頁頁尾");
        assert_eq!(vertical.down(24, 25), 24, "清單最後一項停住");
        assert_eq!(vertical.up(0, 25), 0, "清單第一項停住");

        let layout = CandidateLayout::new(DEFAULT_ROWS, EXPANDED_COLUMNS);
        assert_eq!(layout.down(39, 80), 40, "四欄第 40 項接到下一頁");
        assert_eq!(layout.up(40, 80), 39, "下一頁第 1 項接回第 40 項");
        assert_eq!(layout.down(79, 80), 79, "清單最後一項停住");
        assert_eq!(layout.up(0, 80), 0, "清單第一項停住");
    }

    #[test]
    fn test_vertical_mode_left_right_is_noop() {
        let layout = CandidateLayout::default();
        assert_eq!(layout.right(3, 25), 3);
        assert_eq!(layout.left(3, 25), 3);
    }

    #[test]
    fn columns_are_vertical_and_switch_at_the_same_row() {
        let layout = CandidateLayout::new(DEFAULT_ROWS, 4);
        assert_eq!(layout.cell(0), (0, 0));
        assert_eq!(layout.cell(10), (0, 1));
        assert_eq!(layout.cell(13), (3, 1));
        assert_eq!(layout.right(3, 45), 13, "→ 從欄 0 列 3 到欄 1 列 3");
        assert_eq!(layout.left(13, 45), 3, "← 回到欄 0 列 3");
        assert_eq!(layout.right(39, 45), 39, "最後一欄不再往右");
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
