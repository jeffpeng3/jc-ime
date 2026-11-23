//! 詞典查詢介面與記憶體實作。
//!
//! 實際二進位詞典的讀取由 `w1-data-zh` 負責；這裡只定 trait 與測試用的
//! [`InMemoryLexicon`]。讀音一律是「音節以單一空格分隔」的字串，例如
//! `"ㄊㄞˊ ㄅㄟˇ"`。
//!
//! 對照前身 `core/lexicon`：`Source.Exact` 對應 [`Lexicon::lookup`]，
//! `Source.Prefixed` 對應 [`Lexicon::prefix`]，`Source.LongestWord` 對應
//! [`Lexicon::longest_word_units`]。TSV 與二進位格式的讀取不在本 task。

use std::collections::HashMap;

/// 未知詞邊使用的詞性標籤。
///
/// 詞性 `u16` 的實際編碼由詞典資料決定；保留最大值代表「沒有詞性」，
/// 連接成本表查不到時會回傳保底值。
pub const UNKNOWN_POS: u16 = u16::MAX;

/// 一筆詞條：（詞面, 讀音）與成本。
///
/// 讀音長度不要求等於詞面長度，但注音資料仍是一字一音節。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexEntry {
    /// 輸出詞面。
    pub surface: String,
    /// 讀音，音節以單一空格分隔。
    pub reading: String,
    /// 單詞成本，越小越優先。
    pub cost: i32,
    /// 詞性標籤，供連接成本查表。
    pub pos: u16,
}

impl LexEntry {
    /// 建立詞條。
    pub fn new(
        surface: impl Into<String>,
        reading: impl Into<String>,
        cost: i32,
        pos: u16,
    ) -> Self {
        Self {
            surface: surface.into(),
            reading: reading.into(),
            cost,
            pos,
        }
    }

    /// 這筆詞條的讀音音節數。
    pub fn units(&self) -> usize {
        unit_count(&self.reading)
    }
}

/// 依讀音查詞條的來源。
///
/// 實作只描述「有哪些詞條」，不決定路徑；成本換算與排序由
/// [`crate::lattice`] 與 [`crate::score`] 負責。
pub trait Lexicon {
    /// 回傳讀音完全相等的詞條；查無資料時回傳空切片。
    fn lookup(&self, reading: &str) -> &[LexEntry];

    /// 回傳讀音以 `reading` 開頭的詞條，包含完全相等者。
    ///
    /// 前綴以音節為界：`"ㄊㄞˊ"` 不會匹配 `"ㄊㄞˊˇ"`。`reading` 是空字串時
    /// 回傳全部詞條。
    fn prefix(&self, reading: &str) -> Vec<&LexEntry>;

    /// 回傳最長詞條的音節數；0 表示實作不知道（由呼叫端套用預設上限）。
    fn longest_word_units(&self) -> usize {
        0
    }
}

/// `Arc<T>` 也是 `Lexicon`：讓呼叫端可以傳 `&self.lexicon`，不必到處寫 `&*`。
///
/// 詞典在實機上是 40～55MB，共用一份才有意義；這一層只做轉發，不多一次配置。
impl<T: Lexicon + ?Sized> Lexicon for std::sync::Arc<T> {
    fn lookup(&self, reading: &str) -> &[LexEntry] {
        (**self).lookup(reading)
    }

    fn prefix(&self, reading: &str) -> Vec<&LexEntry> {
        (**self).prefix(reading)
    }

    fn longest_word_units(&self) -> usize {
        (**self).longest_word_units()
    }
}

/// 音節以單一空格分隔的讀音音節數。
///
/// 空字串與只有空白的讀音算 0 個音節。
pub fn unit_count(reading: &str) -> usize {
    reading.split_whitespace().count()
}

/// 把音節串成詞典查詢用的讀音鍵。
pub(crate) fn join_units(units: &[impl AsRef<str>]) -> String {
    let mut out = String::new();
    for (i, unit) in units.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(unit.as_ref());
    }
    out
}

/// 判斷 `full` 是否以 `prefix` 開頭，且切在音節邊界上。
fn is_unit_prefix(prefix: &str, full: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    match full.strip_prefix(prefix) {
        // 完全相等也是合法前綴。
        Some(rest) => rest.is_empty() || rest.starts_with(' '),
        None => false,
    }
}

/// 測試與小型情境用的記憶體詞典。
///
/// 保留加入順序，讓 [`Lexicon::prefix`] 的結果可重現。同一讀音可以有多筆
/// 詞條；同一組（詞面、讀音、成本、詞性）加入兩次會保留兩筆，除重由
/// [`crate::lattice::Lattice::build`] 負責。
#[derive(Debug, Clone, Default)]
pub struct InMemoryLexicon {
    exact: HashMap<String, Vec<LexEntry>>,
    all: Vec<LexEntry>,
    longest: usize,
}

impl InMemoryLexicon {
    /// 建立空詞典。
    pub fn new() -> Self {
        Self::default()
    }

    /// 加入一筆詞條。
    pub fn push(&mut self, entry: LexEntry) {
        self.longest = self.longest.max(entry.units());
        self.exact
            .entry(entry.reading.clone())
            .or_default()
            .push(entry.clone());
        self.all.push(entry);
    }

    /// 依序加入多筆詞條。
    pub fn extend<I: IntoIterator<Item = LexEntry>>(&mut self, entries: I) {
        for entry in entries {
            self.push(entry);
        }
    }

    /// 詞條總數。
    pub fn len(&self) -> usize {
        self.all.len()
    }

    /// 是否沒有任何詞條。
    pub fn is_empty(&self) -> bool {
        self.all.is_empty()
    }

    /// 全部詞條，依加入順序。
    pub fn entries(&self) -> &[LexEntry] {
        &self.all
    }
}

impl Lexicon for InMemoryLexicon {
    fn lookup(&self, reading: &str) -> &[LexEntry] {
        self.exact.get(reading).map(Vec::as_slice).unwrap_or(&[])
    }

    fn prefix(&self, reading: &str) -> Vec<&LexEntry> {
        self.all
            .iter()
            .filter(|e| is_unit_prefix(reading, &e.reading))
            .collect()
    }

    fn longest_word_units(&self) -> usize {
        self.longest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(surface: &str, reading: &str, cost: i32) -> LexEntry {
        LexEntry::new(surface, reading, cost, 1)
    }

    #[test]
    fn lookup_returns_exact_reading_only() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("丼", "ㄉㄨㄥˋ", 10));
        lex.push(entry("丼", "ㄐㄧㄥˇ", 20));
        lex.push(entry("應該", "ㄧㄣ ㄍㄞ", 30));

        let got = lex.lookup("ㄉㄨㄥˋ");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].surface, "丼");
        assert_eq!(got[0].cost, 10);

        assert!(lex.lookup("ㄍㄞ").is_empty());
        assert!(lex.lookup("ㄧ").is_empty());
        assert_eq!(lex.lookup("ㄧㄣ ㄍㄞ")[0].surface, "應該");
    }

    #[test]
    fn multiple_surfaces_keep_insertion_order() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("是", "ㄕˋ", 1));
        lex.push(entry("事", "ㄕˋ", 2));
        lex.push(entry("市", "ㄕˋ", 3));

        let surfaces: Vec<&str> = lex
            .lookup("ㄕˋ")
            .iter()
            .map(|e| e.surface.as_str())
            .collect();
        assert_eq!(surfaces, ["是", "事", "市"]);
    }

    #[test]
    fn prefix_stops_at_syllable_boundary() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("應該", "ㄧㄣ ㄍㄞ", 1));
        lex.push(entry("音", "ㄧㄣ", 2));

        let got: Vec<&str> = lex
            .prefix("ㄧㄣ")
            .iter()
            .map(|e| e.surface.as_str())
            .collect();
        assert_eq!(got, ["應該", "音"]);

        // 「ㄧㄣ ㄍ」不是合法音節前綴，不能匹配「ㄧㄣ ㄍㄞ」。
        assert!(lex.prefix("ㄧㄣ ㄍ").is_empty());
        // 讀音不相同的音節也不能匹配。
        assert!(lex.prefix("ㄧ").is_empty());
    }

    #[test]
    fn prefix_includes_exact_and_longer_entries() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("台北市", "ㄊㄞˊ ㄅㄟˇ ㄕˋ", 3));
        lex.push(entry("台", "ㄊㄞˊ", 1));
        lex.push(entry("台北", "ㄊㄞˊ ㄅㄟˇ", 2));
        lex.push(entry("北", "ㄅㄟˇ", 4));

        let got: Vec<&str> = lex
            .prefix("ㄊㄞˊ")
            .iter()
            .map(|e| e.surface.as_str())
            .collect();
        assert_eq!(got, ["台北市", "台", "台北"]);

        let got: Vec<&str> = lex
            .prefix("ㄊㄞˊ ㄅㄟˇ")
            .iter()
            .map(|e| e.surface.as_str())
            .collect();
        assert_eq!(got, ["台北市", "台北"]);
    }

    #[test]
    fn empty_prefix_returns_everything() {
        let mut lex = InMemoryLexicon::new();
        assert!(lex.prefix("").is_empty());
        lex.push(entry("台", "ㄊㄞˊ", 1));
        lex.push(entry("北", "ㄅㄟˇ", 2));
        assert_eq!(lex.prefix("").len(), 2);
    }

    #[test]
    fn longest_word_units_tracks_the_longest_entry() {
        let mut lex = InMemoryLexicon::new();
        assert_eq!(lex.longest_word_units(), 0);

        lex.push(entry("阿姆斯特朗", "ㄚ ㄇㄨˇ ㄙ ㄊㄜˋ ㄌㄤˇ", 1));
        lex.push(entry("喔", "ㄛ", 1));
        assert_eq!(lex.longest_word_units(), 5);

        let reading = "ㄚ ㄇㄨˇ ㄙ ㄊㄜˋ ㄌㄤˇ ㄏㄨㄟˊ ㄒㄩㄢˊ ㄐㄧㄚ ㄙㄨˋ ㄆㄣ ㄑㄧˋ ㄕˋ ㄚ ㄇㄨˇ ㄙ ㄊㄜˋ ㄌㄤˇ ㄆㄠˋ";
        lex.push(entry("阿姆斯特朗回旋加速噴氣式阿姆斯特朗炮", reading, 1));
        assert_eq!(lex.longest_word_units(), 18);
    }

    #[test]
    fn entries_and_len_keep_every_added_entry() {
        let mut lex = InMemoryLexicon::new();
        assert!(lex.is_empty());
        lex.push(entry("台", "ㄊㄞˊ", 1));
        lex.extend([entry("北", "ㄅㄟˇ", 2), entry("台北", "ㄊㄞˊ ㄅㄟˇ", 3)]);
        assert_eq!(lex.len(), 3);
        assert_eq!(lex.entries()[2].surface, "台北");
        assert!(!lex.is_empty());
    }

    #[test]
    fn unit_count_ignores_extra_whitespace() {
        assert_eq!(unit_count(""), 0);
        assert_eq!(unit_count("  "), 0);
        assert_eq!(unit_count("ㄊㄞˊ"), 1);
        assert_eq!(unit_count("ㄊㄞˊ  ㄅㄟˇ"), 2);
    }

    #[test]
    fn entry_units_counts_reading_syllables() {
        let e = LexEntry::new("台北", "ㄊㄞˊ ㄅㄟˇ", 3, 7);
        assert_eq!(e.units(), 2);
        assert_eq!(e.pos, 7);
        assert_eq!(e.surface, "台北");
        assert_eq!(e.cost, 3);
    }

    #[test]
    fn unknown_pos_is_reserved_max() {
        assert_eq!(UNKNOWN_POS, u16::MAX);
    }
}
