//! 同一 surface 內的詞條排序。
//!
//! 讀取端不需要排序，這兩個函式供寫入端與匯出流程沿用 Go 參考實作的行為：
//! 先比成本，再比讀音、POS、活用型、活用形；相同 tuple 保留原順序。

use std::cmp::Ordering;

use super::pack::Entry;

/// 比較兩筆同一 surface 的詞條。
///
/// POS 與活用欄位比的是 table id。table 本身依字串排序，因此 id 順序
/// 與字串順序一致。
pub fn compare_entries(a: &Entry, b: &Entry) -> Ordering {
    a.cost
        .cmp(&b.cost)
        .then_with(|| a.reading.cmp(&b.reading))
        .then_with(|| a.pos_id.cmp(&b.pos_id))
        .then_with(|| a.conj_type.cmp(&b.conj_type))
        .then_with(|| a.conj_form.cmp(&b.conj_form))
}

/// 依 [`compare_entries`] 穩定排序一組同 surface 的詞條。
pub fn sort_surface_entries(entries: &mut [Entry]) {
    entries.sort_by(compare_entries);
}
