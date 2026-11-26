//! 候選窗的版面狀態：列數、欄數與移動規則。
//!
//! 這裡只描述「清單怎麼排」；候選從哪裡來、顯示什麼文字都不在此模組。
//! 外殼拿到同一份版面參數就直接畫，不再依語言決定欄數或換行。

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

    /// 一列最多幾項。
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

    /// `↓` 在視覺上移動一列時的索引位移。
    pub fn row_step(self) -> isize {
        self.columns as isize
    }

    /// 由目前索引往下移一列，頭尾相接。
    pub fn down(self, index: usize, count: usize) -> usize {
        self.step(index, count, self.row_step())
    }

    /// 由目前索引往上移一列，頭尾相接。
    pub fn up(self, index: usize, count: usize) -> usize {
        self.step(index, count, -self.row_step())
    }

    fn step(self, index: usize, count: usize, delta: isize) -> usize {
        if count == 0 {
            return 0;
        }
        let len = count as isize;
        let mut next = (index as isize + delta) % len;
        if next < 0 {
            next += len;
        }
        next as usize
    }

    /// 這個索引在第幾頁（0 起算）。
    pub fn page(self, index: usize) -> usize {
        index / self.page_size().max(1)
    }

    /// 總頁數；沒有候選時回 0。
    pub fn pages(self, count: usize) -> usize {
        count.div_ceil(self.page_size().max(1))
    }

    /// 這個索引落在頁內第幾列與第幾欄（都從 0 起算）。
    pub fn cell(self, index: usize) -> (usize, usize) {
        let within = index % self.page_size().max(1);
        (within / self.columns.max(1), within % self.columns.max(1))
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
        assert_eq!(layout.down(1, 9), 3, "兩欄時 ↓ 只換到下一列同欄");
        assert_eq!(layout.up(3, 9), 1);
    }

    #[test]
    fn test_four_column_down_moves_one_row() {
        let layout = CandidateLayout::new(DEFAULT_ROWS, 4);
        assert_eq!(layout.down(1, 12), 5, "四欄時 ↓ 只換到下一列同欄");
        assert_eq!(layout.up(5, 12), 1);
        assert_eq!(layout.row_step(), 4);
    }

    #[test]
    fn expand_only_changes_columns_and_page_capacity() {
        let mut layout = CandidateLayout::default();
        layout.expand();
        assert_eq!(layout.columns(), 4);
        assert_eq!(layout.rows(), 10);
        assert_eq!(layout.page_size(), 40);
        assert_eq!(layout.cell(13), (3, 1));
        assert_eq!(layout.page(41), 1);
        layout.collapse();
        assert_eq!(layout, CandidateLayout::default());
    }
}
