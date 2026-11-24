//! 詞格建構：依讀音序列查詞典，建立節點與邊。
//!
//! 對照前身 `core/lattice/build.go`。前身用 `Input`／`CandidateSource`
//! 接受已經分段的候選；Rust 契約直接吃注音音節序列與 [`Lexicon`]，
//! 因此 `BuildFrom` 的「長詞跨片段」在這裡是自然結果：建邊時就查詢所有
//! 長度的讀音組合。

use std::collections::HashMap;

use crate::candidate::CandidateSource;
use crate::score::{ScoreContext, NON_PRIMARY_READING_PENALTY};

use super::candidate::Edge;
use super::lexicon::{join_units, LexEntry, Lexicon, UNKNOWN_POS};

/// 建構詞格時的錯誤。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LatticeError {
    /// 沒有任何讀音。
    #[error("沒有可建立詞格的讀音")]
    EmptyInput,
    /// 某個音節是空字串。
    #[error("第 {index} 個音節是空的")]
    EmptyReading {
        /// 出問題的音節索引。
        index: usize,
    },
    /// 詞格無法涵蓋整個輸入。
    #[error("第 {index} 個音節沒有可用的邊")]
    Uncovered {
        /// 沒有邊起點的音節索引。
        index: usize,
    },
}

/// 可搜尋的詞格。
///
/// 節點是音節位置 `0..=len`，邊是 [`Edge`]。同一個位置可以有多條邊，
/// 詞典成本越低越優先；查不到詞典詞條的音節會補一條未知詞邊，確保
/// 每個音節都有路徑可用。
#[derive(Debug, Clone, PartialEq)]
pub struct Lattice {
    readings: Vec<String>,
    by_start: Vec<Vec<Edge>>,
    ctx: ScoreContext,
}

impl Lattice {
    /// 依讀音序列建立詞格。
    ///
    /// `readings` 的每個元素是一個注音音節（例如 `["ㄊㄞˊ", "ㄅㄟˇ"]`）。
    /// 長詞會以單一條邊跨越多個音節；每個音節另外補一條未知詞邊。
    /// 單詞最大長度取 `ctx.max_word_units` 與詞典宣告的最長詞較大者，
    /// `ctx.max_word_units` 為 0 表示不限制。
    pub fn build<R, L>(
        readings: &[R],
        lexicon: &L,
        ctx: &ScoreContext,
    ) -> Result<Self, LatticeError>
    where
        R: AsRef<str>,
        L: Lexicon + ?Sized,
    {
        if readings.is_empty() {
            return Err(LatticeError::EmptyInput);
        }
        let mut units = Vec::with_capacity(readings.len());
        for (index, reading) in readings.iter().enumerate() {
            let reading = reading.as_ref();
            if reading.trim().is_empty() {
                return Err(LatticeError::EmptyReading { index });
            }
            units.push(reading.to_string());
        }

        let limit = match ctx.max_word_units {
            0 => units.len(),
            n => n.max(lexicon.longest_word_units()),
        };

        let mut by_start = Vec::with_capacity(units.len());
        for start in 0..units.len() {
            let mut edges = Vec::new();
            let max_len = limit.min(units.len() - start);
            for len in 1..=max_len {
                let reading = join_units(&units[start..start + len]);
                for entry in lexicon.lookup(&reading) {
                    push_lexicon_edge(&mut edges, lexicon, entry, start, len, &reading, ctx);
                }
            }
            edges.push(unknown_edge(&units[start], start, ctx));
            dedupe(&mut edges);
            edges.sort_by_key(|a| (a.end, a.key()));
            by_start.push(edges);
        }

        if let Some(index) = by_start.iter().position(Vec::is_empty) {
            return Err(LatticeError::Uncovered { index });
        }

        Ok(Self {
            readings: units,
            by_start,
            ctx: ctx.clone(),
        })
    }

    /// 建立詞格時的讀音，依輸入順序。
    pub fn readings(&self) -> &[String] {
        &self.readings
    }

    /// 音節數。
    pub fn len(&self) -> usize {
        self.readings.len()
    }

    /// 是否沒有音節。
    pub fn is_empty(&self) -> bool {
        self.readings.is_empty()
    }

    /// 回傳從 `pos` 開始的所有邊；超出範圍時回傳空切片。
    ///
    /// 同一個起點的邊依（終點, 穩定鍵）排序，結果可重現。
    pub fn edges_from(&self, pos: usize) -> &[Edge] {
        self.by_start.get(pos).map(Vec::as_slice).unwrap_or(&[])
    }

    /// 依起點順序走訪所有邊。
    pub fn edges(&self) -> impl Iterator<Item = &Edge> {
        self.by_start.iter().flat_map(|edges| edges.iter())
    }

    /// 邊的總數。
    pub fn edge_count(&self) -> usize {
        self.by_start.iter().map(Vec::len).sum()
    }

    /// 建立詞格時使用的成本設定。
    pub fn ctx(&self) -> &ScoreContext {
        &self.ctx
    }
}

/// 把詞典詞條轉成一條邊；讀音與查詢不符時忽略。
///
/// 單一字元詞條如果還有成本更低的讀音，加上
/// [`NON_PRIMARY_READING_PENALTY`]；詞典只保留罕用旗標，沒有主要讀音資訊，
/// 這個比較只能靠同一詞面的所有讀音。多字詞有自己的詞條成本，不受影響。
fn push_lexicon_edge<L: Lexicon + ?Sized>(
    edges: &mut Vec<Edge>,
    lexicon: &L,
    entry: &LexEntry,
    start: usize,
    len: usize,
    reading: &str,
    ctx: &ScoreContext,
) {
    if entry.reading != reading {
        return;
    }
    let mut cost = ctx.word_cost(entry.cost, len);
    if len == 1
        && entry.surface.chars().count() == 1
        && lexicon
            .cheapest_reading_cost(&entry.surface)
            .is_some_and(|cheapest| entry.cost > cheapest)
    {
        cost = cost.saturating_add(NON_PRIMARY_READING_PENALTY);
    }
    edges.push(Edge {
        start,
        end: start + len,
        surface: entry.surface.clone(),
        reading: entry.reading.clone(),
        cost,
        pos: entry.pos,
        source: CandidateSource::Lexicon,
    });
}

/// 詞典查不到時保留讀音本身的邊，確保輸入永遠有路徑。
fn unknown_edge(reading: &str, start: usize, ctx: &ScoreContext) -> Edge {
    Edge {
        start,
        end: start + 1,
        surface: reading.to_string(),
        reading: reading.to_string(),
        cost: ctx.unknown_word_cost(1),
        pos: UNKNOWN_POS,
        source: CandidateSource::Literal,
    }
}

/// 相同穩定鍵只保留成本較低者，與前身 `normalize` 的除重語意相同。
fn dedupe(edges: &mut Vec<Edge>) {
    let mut seen: HashMap<String, usize> = HashMap::with_capacity(edges.len());
    let mut out: Vec<Edge> = Vec::with_capacity(edges.len());
    for edge in edges.drain(..) {
        let key = edge.key();
        match seen.get(&key) {
            Some(&index) => {
                if edge.cost < out[index].cost {
                    out[index] = edge;
                }
            }
            None => {
                seen.insert(key, out.len());
                out.push(edge);
            }
        }
    }
    *edges = out;
}

#[cfg(test)]
mod tests {
    use super::super::lexicon::InMemoryLexicon;
    use super::*;
    use crate::score::ScoreContext;

    fn entry(surface: &str, reading: &str, cost: i32, pos: u16) -> LexEntry {
        LexEntry::new(surface, reading, cost, pos)
    }

    fn taipei_lexicon() -> InMemoryLexicon {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("台", "ㄊㄞˊ", 1000, 1));
        lex.push(entry("北", "ㄅㄟˇ", 1000, 1));
        lex.push(entry("台北", "ㄊㄞˊ ㄅㄟˇ", 2000, 1));
        lex
    }

    fn edge<'a>(lat: &'a Lattice, start: usize, surface: &str) -> &'a Edge {
        lat.edges_from(start)
            .iter()
            .find(|e| e.surface == surface)
            .unwrap_or_else(|| panic!("找不到 {surface} 的邊"))
    }

    #[test]
    fn test_build_creates_edges_for_each_reading() {
        let lex = taipei_lexicon();
        let ctx = ScoreContext::default();
        let lat = Lattice::build(&["ㄊㄞˊ", "ㄅㄟˇ"], &lex, &ctx).expect("建立詞格");

        assert_eq!(lat.len(), 2);
        assert_eq!(lat.readings(), ["ㄊㄞˊ", "ㄅㄟˇ"]);

        // 起點 0：台、未知詞、台北 三條邊。
        assert_eq!(lat.edges_from(0).len(), 3);
        assert_eq!(edge(&lat, 0, "台").cost, 1000);
        // 兩個音節的詞扣掉一個長度折扣：2000 - 1。
        assert_eq!(edge(&lat, 0, "台北").cost, 1999);
        assert_eq!(edge(&lat, 0, "台北").end, 2);
        // 未知詞以讀音本身輸出，成本 8。
        assert_eq!(edge(&lat, 0, "ㄊㄞˊ").cost, 8);
        assert_eq!(edge(&lat, 0, "ㄊㄞˊ").source, CandidateSource::Literal);

        // 起點 1：北與未知詞。
        assert_eq!(lat.edges_from(1).len(), 2);
        assert_eq!(edge(&lat, 1, "北").end, 2);

        // 最後一個位置沒有邊。
        assert!(lat.edges_from(2).is_empty());
        assert!(lat.edges_from(99).is_empty());
        assert_eq!(lat.edge_count(), 5);
    }

    #[test]
    fn test_build_rejects_empty_input() {
        let lex = InMemoryLexicon::new();
        let ctx = ScoreContext::default();
        let readings: [&str; 0] = [];
        assert_eq!(
            Lattice::build(&readings, &lex, &ctx),
            Err(LatticeError::EmptyInput)
        );
    }

    #[test]
    fn test_build_rejects_empty_reading() {
        let lex = InMemoryLexicon::new();
        let ctx = ScoreContext::default();
        assert_eq!(
            Lattice::build(&["ㄊㄞˊ", "", "ㄅㄟˇ"], &lex, &ctx),
            Err(LatticeError::EmptyReading { index: 1 })
        );
        assert_eq!(
            Lattice::build(&["ㄊㄞˊ", "   "], &lex, &ctx),
            Err(LatticeError::EmptyReading { index: 1 })
        );
    }

    #[test]
    fn test_build_adds_unknown_fallback_for_missing_entries() {
        let lex = InMemoryLexicon::new();
        let ctx = ScoreContext::default();
        let lat = Lattice::build(&["ㄅ"], &lex, &ctx).expect("建立詞格");

        let edges = lat.edges_from(0);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].surface, "ㄅ");
        assert_eq!(edges[0].reading, "ㄅ");
        assert_eq!(edges[0].cost, 8);
        assert_eq!(edges[0].source, CandidateSource::Literal);
        assert_eq!(edges[0].pos, UNKNOWN_POS);
        assert_eq!((edges[0].start, edges[0].end), (0, 1));
    }

    #[test]
    fn test_build_dedupes_same_edge_keeping_lowest_cost() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("台", "ㄊㄞˊ", 5, 1));
        lex.push(entry("台", "ㄊㄞˊ", 3, 1));
        lex.push(entry("台", "ㄊㄞˊ", 9, 1));
        let ctx = ScoreContext::default();
        let lat = Lattice::build(&["ㄊㄞˊ"], &lex, &ctx).expect("建立詞格");

        let edges = lat.edges_from(0);
        assert_eq!(edges.len(), 2, "除重後應只剩詞典邊與未知詞邊");
        assert_eq!(edge(&lat, 0, "台").cost, 3);
    }

    #[test]
    fn test_build_keeps_same_surface_with_different_pos() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("台", "ㄊㄞˊ", 5, 1));
        lex.push(entry("台", "ㄊㄞˊ", 5, 2));
        let ctx = ScoreContext::default();
        let lat = Lattice::build(&["ㄊㄞˊ"], &lex, &ctx).expect("建立詞格");

        assert_eq!(lat.edges_from(0).len(), 3, "不同詞性是不同的邊");
    }

    #[test]
    fn test_build_covers_long_words_across_syllables() {
        let lex = taipei_lexicon();
        let ctx = ScoreContext::default();
        let lat = Lattice::build(&["ㄊㄞˊ", "ㄅㄟˇ"], &lex, &ctx).expect("建立詞格");

        let long = edge(&lat, 0, "台北");
        assert_eq!((long.start, long.end), (0, 2));
        assert_eq!(long.reading, "ㄊㄞˊ ㄅㄟˇ");

        // 所有邊都不會超出輸入範圍。
        assert!(lat.edges().all(|e| e.end <= lat.len() && e.start < e.end));
    }

    #[test]
    fn test_build_limits_word_length_by_context() {
        struct FixedLexicon(Vec<LexEntry>);

        impl Lexicon for FixedLexicon {
            fn lookup(&self, reading: &str) -> &[LexEntry] {
                match self.0.iter().find(|e| e.reading == reading) {
                    Some(entry) => std::slice::from_ref(entry),
                    None => &[],
                }
            }

            fn prefix(&self, reading: &str) -> Vec<&LexEntry> {
                self.0
                    .iter()
                    .filter(|e| e.reading.starts_with(reading))
                    .collect()
            }
        }

        // 沒有覆寫 longest_word_units，詞長上限只由 ctx 決定。
        let lex = FixedLexicon(vec![entry("台北", "ㄊㄞˊ ㄅㄟˇ", 1, 1)]);
        let mut ctx = ScoreContext {
            max_word_units: 1,
            ..Default::default()
        };
        let lat = Lattice::build(&["ㄊㄞˊ", "ㄅㄟˇ"], &lex, &ctx).expect("建立詞格");
        assert_eq!(lat.edges_from(0).len(), 1, "只留未知詞邊");
        assert_eq!(lat.edges_from(0)[0].source, CandidateSource::Literal);

        ctx.max_word_units = 2;
        let lat = Lattice::build(&["ㄊㄞˊ", "ㄅㄟˇ"], &lex, &ctx).expect("建立詞格");
        assert_eq!(lat.edges_from(0).len(), 2);
        assert_eq!(edge(&lat, 0, "台北").cost, 0);
    }

    #[test]
    fn test_build_uses_lexicon_longest_word() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("台北市", "ㄊㄞˊ ㄅㄟˇ ㄕˋ", 3000, 1));
        let ctx = ScoreContext {
            max_word_units: 2,
            ..Default::default()
        };

        let lat = Lattice::build(&["ㄊㄞˊ", "ㄅㄟˇ", "ㄕˋ"], &lex, &ctx).expect("建立詞格");
        assert_eq!(edge(&lat, 0, "台北市").end, 3, "詞庫更長的詞仍要查到");

        let ctx = ScoreContext {
            max_word_units: 0,
            ..Default::default()
        };
        let lat = Lattice::build(&["ㄊㄞˊ", "ㄅㄟˇ", "ㄕˋ"], &lex, &ctx).expect("建立詞格");
        assert_eq!(edge(&lat, 0, "台北市").end, 3);
    }

    #[test]
    fn test_build_ignores_entries_with_mismatched_reading() {
        struct Liar;

        impl Lexicon for Liar {
            fn lookup(&self, _reading: &str) -> &[LexEntry] {
                // 故意回傳一個讀音不符的詞條；呼叫端必須忽略。
                static LEAK: std::sync::OnceLock<Vec<LexEntry>> = std::sync::OnceLock::new();
                LEAK.get_or_init(|| vec![entry("錯", "ㄘㄨㄛˋ", 1, 1)])
            }

            fn prefix(&self, _reading: &str) -> Vec<&LexEntry> {
                Vec::new()
            }
        }

        let ctx = ScoreContext::default();
        let lat = Lattice::build(&["ㄊㄞˊ"], &Liar, &ctx).expect("建立詞格");
        assert_eq!(lat.edges_from(0).len(), 1);
        assert_eq!(lat.edges_from(0)[0].source, CandidateSource::Literal);
    }

    #[test]
    fn test_build_is_deterministic_and_sorted() {
        let lex = taipei_lexicon();
        let ctx = ScoreContext::default();
        let a = Lattice::build(&["ㄊㄞˊ", "ㄅㄟˇ"], &lex, &ctx).expect("建立詞格");
        let b = Lattice::build(&["ㄊㄞˊ", "ㄅㄟˇ"], &lex, &ctx).expect("建立詞格");
        assert_eq!(a, b);

        for pos in 0..a.len() {
            let edges = a.edges_from(pos);
            assert!(edges.windows(2).all(|w| w[0].end <= w[1].end));
        }
    }

    #[test]
    fn test_build_penalizes_non_minimum_reading_of_single_character() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("日", "ㄇㄧˋ", 20, 1));
        lex.push(entry("日", "ㄖˋ", 10, 1));
        lex.push(entry("日常", "ㄖˋ ㄔㄤˊ", 5, 1));
        let ctx = ScoreContext::default();

        let lat = Lattice::build(&["ㄇㄧˋ"], &lex, &ctx).expect("建立詞格");
        assert_eq!(
            edge(&lat, 0, "日").cost,
            20 + NON_PRIMARY_READING_PENALTY,
            "非最低成本讀音要加固定懲罰"
        );

        let lat = Lattice::build(&["ㄖˋ"], &lex, &ctx).expect("建立詞格");
        assert_eq!(edge(&lat, 0, "日").cost, 10, "最低成本讀音不受影響");

        let lat = Lattice::build(&["ㄖˋ", "ㄔㄤˊ"], &lex, &ctx).expect("建立詞格");
        assert_eq!(edge(&lat, 0, "日常").cost, 4, "多字詞不受讀音懲罰");
    }

    #[test]
    fn test_build_does_not_penalize_without_surface_index() {
        struct NoSurfaceIndex(Vec<LexEntry>);

        impl Lexicon for NoSurfaceIndex {
            fn lookup(&self, reading: &str) -> &[LexEntry] {
                match self.0.iter().find(|e| e.reading == reading) {
                    Some(entry) => std::slice::from_ref(entry),
                    None => &[],
                }
            }

            fn prefix(&self, _reading: &str) -> Vec<&LexEntry> {
                Vec::new()
            }
        }

        // 沒有 cheapest_reading_cost 的實作不能假設有詞面索引，維持原成本。
        let lex = NoSurfaceIndex(vec![entry("日", "ㄇㄧˋ", 20, 1)]);
        let ctx = ScoreContext::default();
        let lat = Lattice::build(&["ㄇㄧˋ"], &lex, &ctx).expect("建立詞格");
        assert_eq!(edge(&lat, 0, "日").cost, 20);
    }

    #[test]
    fn test_build_accepts_owned_syllables() {
        let lex = taipei_lexicon();
        let ctx = ScoreContext::default();
        let readings = vec!["ㄊㄞˊ".to_string(), "ㄅㄟˇ".to_string()];
        let lat = Lattice::build(&readings, &lex, &ctx).expect("建立詞格");
        assert_eq!(lat.readings(), readings.as_slice());
    }

    #[test]
    fn test_build_exposes_context() {
        let lex = taipei_lexicon();
        let ctx = ScoreContext {
            unknown_cost: 99,
            ..Default::default()
        };
        let lat = Lattice::build(&["ㄊㄞˊ"], &lex, &ctx).expect("建立詞格");
        assert_eq!(lat.ctx().unknown_cost, 99);
        assert_eq!(edge(&lat, 0, "ㄊㄞˊ").cost, 99);
    }
}
