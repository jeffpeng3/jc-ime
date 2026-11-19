//! Viterbi 解碼與 N-best。
//!
//! 對照前身 `core/lattice/viterbi.go`：前向 DP 的狀態是「輸入位置＋最後一條
//! 邊的詞性」，因為連接成本讓「到同一個位置但最後詞性不同」的路徑不能互相
//! 比較。每個狀態保留前 n 條路徑，最後再合併排序；沒有改成 beam search。

use std::collections::BTreeMap;

use crate::score::Scorer;

use super::build::Lattice;
use super::candidate::Edge;

/// 路徑起點（BOS）的狀態鍵，不會與任何詞性衝突。
const BOS: u16 = u16::MAX;

/// 一條候選路徑。
///
/// `edges` 已依輸入順序排列，每條邊就是輸出的片段；長詞會以單一條邊覆蓋
/// 多個音節，因此不需要再合併。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    /// 路徑上的邊，依輸入順序。
    pub edges: Vec<Edge>,
    /// 整條路徑的 Viterbi 成本，越低越優先。
    pub cost: i32,
}

impl Path {
    /// 把路徑的詞面依序串接，不插入額外分隔。
    pub fn surface(&self) -> String {
        let mut out = String::new();
        for edge in &self.edges {
            out.push_str(&edge.surface);
        }
        out
    }

    /// 路徑的穩定鍵，供除重與決定性排序使用。
    pub fn key(&self) -> String {
        self.edges
            .iter()
            .map(Edge::key)
            .collect::<Vec<_>>()
            .join("\x1f")
    }

    /// 路徑上的邊數。
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// 路徑是否為空。
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }
}

/// Viterbi 解碼器。
///
/// 成本相同時先偏好邊數較少的路徑（長詞合併優先），再依 [`Path::key`] 排序，
/// 確保結果可重現。
pub struct Viterbi;

impl Viterbi {
    /// 回傳成本最低的路徑；詞格無法完整覆蓋輸入時回傳 `None`。
    pub fn best<S>(lattice: &Lattice, scorer: &S) -> Option<Path>
    where
        S: Scorer + ?Sized,
    {
        Self::nbest_with(lattice, scorer, 1).into_iter().next()
    }

    /// 用詞格建立時的成本設定回傳最多 `n` 條路徑。
    pub fn nbest(lattice: &Lattice, n: usize) -> Vec<Path> {
        Self::nbest_with(lattice, lattice.ctx(), n)
    }

    /// 用指定的評分器回傳最多 `n` 條路徑，依成本由低到高排列。
    ///
    /// `n` 為 0 時視同 1。無法從位置 0 走到輸入結尾時回傳空向量。
    pub fn nbest_with<S>(lattice: &Lattice, scorer: &S, n: usize) -> Vec<Path>
    where
        S: Scorer + ?Sized,
    {
        let n = n.max(1);
        let len = lattice.len();
        if len == 0 {
            return Vec::new();
        }

        // dp[pos][(前一個詞面, 前一個詞性)] 是走到 pos 的前 n 條路徑。
        //
        // 有 bigram 時光看詞性不夠：同一個詞性可能對應不同前詞，之後的機率
        // 也不同。鍵因此同時包含前一個詞面；沒有 bigram 時多出來的鍵只會
        // 重複同樣的成本，結果不變。
        let mut dp: Vec<BTreeMap<(String, u16), Vec<Path>>> = vec![BTreeMap::new(); len + 1];
        dp[0].insert(
            (String::new(), BOS),
            vec![Path {
                edges: Vec::new(),
                cost: 0,
            }],
        );

        for pos in 0..len {
            let states = std::mem::take(&mut dp[pos]);
            if states.is_empty() {
                continue;
            }
            for edge in lattice.edges_from(pos) {
                if edge.end <= pos || edge.end > len {
                    continue;
                }
                for ((left, last), paths) in &states {
                    let left = (!left.is_empty()).then_some(left.as_str());
                    for path in paths {
                        let step = if path.edges.is_empty() {
                            scorer.edge_cost(None, BOS, edge)
                        } else {
                            scorer.edge_cost(left, *last, edge)
                        };
                        let mut next = Path {
                            edges: Vec::with_capacity(path.edges.len() + 1),
                            cost: path.cost.saturating_add(step),
                        };
                        next.edges.extend_from_slice(&path.edges);
                        next.edges.push(edge.clone());
                        insert_top(
                            dp[edge.end]
                                .entry((edge.surface.clone(), edge.pos))
                                .or_default(),
                            next,
                            n,
                        );
                    }
                }
            }
        }

        let mut final_paths: Vec<Path> = dp[len].values().flatten().cloned().collect();
        final_paths.sort_by(path_less);
        final_paths.truncate(n);
        final_paths
    }
}

/// 插入前 n 條路徑；相同鍵只保留成本較低者。
fn insert_top(list: &mut Vec<Path>, path: Path, n: usize) {
    let key = path.key();
    if let Some(index) = list.iter().position(|p| p.key() == key) {
        if path.cost < list[index].cost {
            list[index] = path;
            list.sort_by(path_less);
        }
        return;
    }
    list.push(path);
    list.sort_by(path_less);
    list.truncate(n);
}

/// 路徑排序：成本、邊數、穩定鍵。
fn path_less(a: &Path, b: &Path) -> std::cmp::Ordering {
    (a.cost, a.edges.len(), a.key()).cmp(&(b.cost, b.edges.len(), b.key()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::CandidateSource;
    use crate::lattice::build::Lattice;
    use crate::lattice::lexicon::{InMemoryLexicon, LexEntry};
    use crate::score::{ConnectionMatrix, ScoreContext};

    fn entry(surface: &str, reading: &str, cost: i32, pos: u16) -> LexEntry {
        LexEntry::new(surface, reading, cost, pos)
    }

    /// 測試用設定：關掉長度折扣與連接成本，只留下詞典成本。
    fn plain_ctx() -> ScoreContext {
        ScoreContext {
            length_bonus: 0,
            unknown_cost: 100,
            connection: ConnectionMatrix::with_fallback(0),
            ..Default::default()
        }
    }

    #[test]
    fn test_nbest_orders_paths_and_is_deterministic() {
        // 對應前身 TestNBestOrdersPathsAndIsDeterministic。
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("AB", "a b", 15, 1));
        lex.push(entry("a1", "a", 10, 1));
        lex.push(entry("b1", "b", 10, 1));
        lex.push(entry("a2", "a", 30, 1));
        lex.push(entry("b2", "b", 30, 1));
        let ctx = plain_ctx();
        let readings = ["a", "b"];
        let lat = Lattice::build(&readings, &lex, &ctx).expect("建立詞格");

        assert_eq!(lat.edges_from(0).len(), 4, "AB、a1、a2 與未知詞");

        let paths = Viterbi::nbest(&lat, 3);
        assert_eq!(paths.len(), 3);
        assert_eq!(paths[0].surface(), "AB");
        assert_eq!(paths[1].surface(), "a1b1");
        assert_eq!((paths[0].cost, paths[1].cost), (15, 20));
        assert_eq!(paths[2].cost, 40, "a1b2 與 a2b1 同成本");

        let all = Viterbi::nbest(&lat, 10);
        assert_eq!(all.len(), 10, "含長詞路徑與未知詞組合");

        assert_eq!(paths, Viterbi::nbest(&lat, 3), "同一詞格的結果要穩定");

        let one = Viterbi::nbest(&lat, 0);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].surface(), "AB");

        let best = Viterbi::best(&lat, &ctx).expect("應有最佳路徑");
        assert_eq!(best.surface(), "AB");
        assert_eq!(best.edge_count(), 1);
    }

    #[test]
    fn test_long_word_wins_tie_against_split() {
        // 對應前身 TestLongWordWinsTieAgainstSplit。
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("台", "ㄊㄞˊ", 10, 1));
        lex.push(entry("北", "ㄅㄟˇ", 10, 1));
        lex.push(entry("台北", "ㄊㄞˊ ㄅㄟˇ", 20, 1));
        let ctx = plain_ctx();
        let readings = ["ㄊㄞˊ", "ㄅㄟˇ"];
        let lat = Lattice::build(&readings, &lex, &ctx).expect("建立詞格");
        assert_eq!(lat.ctx().length_bonus, 0, "這個測試只看 tie-break");

        let best = Viterbi::best(&lat, &ctx).expect("應有最佳路徑");
        assert_eq!(best.cost, 20);
        assert_eq!(best.edge_count(), 1, "同成本時偏好長詞");
        assert_eq!(best.surface(), "台北");
    }

    #[test]
    fn test_length_bonus_prefers_long_word_before_full_split() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("台", "ㄊㄞˊ", 10, 1));
        lex.push(entry("北", "ㄅㄟˇ", 10, 1));
        lex.push(entry("台北", "ㄊㄞˊ ㄅㄟˇ", 20, 1));
        let ctx = ScoreContext {
            unknown_cost: 100,
            ..Default::default()
        };
        let readings = ["ㄊㄞˊ", "ㄅㄟˇ"];
        let lat = Lattice::build(&readings, &lex, &ctx).expect("建立詞格");

        let best = Viterbi::best(&lat, &ctx).expect("應有最佳路徑");
        // 長詞 20-1=19，切開是 10+10+連接成本 1=21。
        assert_eq!(best.cost, 19);
        assert_eq!(best.surface(), "台北");
        assert_eq!(best.edge_count(), 1);
    }

    #[test]
    fn test_connection_cost_changes_best_path() {
        // 改寫前身 TestSwitchCostProducesDistinctLanguages：語言切換改成詞性連接成本。
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("甲", "a", 10, 1));
        lex.push(entry("乙", "a", 10, 2));
        lex.push(entry("丙", "b", 10, 1));
        lex.push(entry("丁", "b", 10, 2));

        let mut ctx = plain_ctx();
        ctx.connection.set(1, 1, 0);
        ctx.connection.set(2, 2, 0);
        ctx.connection.set(1, 2, 5);
        ctx.connection.set(2, 1, 5);

        let readings = ["a", "b"];
        let lat = Lattice::build(&readings, &lex, &ctx).expect("建立詞格");
        let paths = Viterbi::nbest(&lat, 4);
        assert_eq!(paths.len(), 4);

        let costs: Vec<i32> = paths.iter().map(|p| p.cost).collect();
        assert_eq!(costs, [20, 20, 25, 25], "同詞性 20，跨詞性 25");
        let surfaces: Vec<String> = paths.iter().map(Path::surface).collect();
        assert_eq!(surfaces, ["乙丁", "甲丙", "乙丙", "甲丁"]);
    }

    #[test]
    fn test_best_uses_supplied_scorer_not_context() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("甲", "a", 10, 1));
        let ctx = plain_ctx();
        let readings = ["a", "a"];
        let lat = Lattice::build(&readings, &lex, &ctx).expect("建立詞格");

        let base = Viterbi::best(&lat, &ctx).expect("應有最佳路徑");
        assert_eq!(base.surface(), "甲甲");
        assert_eq!(base.cost, 20);

        struct PenalizeAll;
        impl Scorer for PenalizeAll {
            fn connection_cost(&self, _prev_pos: u16, _next_pos: u16) -> i32 {
                7
            }
        }

        let best = Viterbi::best(&lat, &PenalizeAll).expect("應有最佳路徑");
        assert_eq!(best.surface(), "甲甲");
        assert_eq!(best.cost, 27, "10 + 10 + 連接 7");
    }

    #[test]
    fn test_connection_cost_can_change_choice() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("甲", "a", 10, 1));
        lex.push(entry("乙", "a", 10, 2));
        let ctx = plain_ctx();
        let readings = ["a", "a"];
        let lat = Lattice::build(&readings, &lex, &ctx).expect("建立詞格");

        // 沒有連接成本時四條路徑同成本，依穩定鍵排序由「乙乙」勝出。
        let base = Viterbi::best(&lat, &ctx).expect("應有最佳路徑");
        assert_eq!(base.surface(), "乙乙");
        assert_eq!(base.cost, 20);

        struct AvoidPrev2;
        impl Scorer for AvoidPrev2 {
            fn connection_cost(&self, prev_pos: u16, _next_pos: u16) -> i32 {
                if prev_pos == 2 {
                    100
                } else {
                    0
                }
            }
        }

        let best = Viterbi::best(&lat, &AvoidPrev2).expect("應有最佳路徑");
        assert_eq!(best.surface(), "甲甲");
        assert_eq!(best.cost, 20);
    }

    #[test]
    fn test_nbest_uses_lattice_context_by_default() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("甲", "a", 10, 1));
        lex.push(entry("乙", "a", 10, 2));
        lex.push(entry("丙", "b", 10, 1));

        let mut ctx = plain_ctx();
        ctx.connection.set(1, 2, 100);

        let readings = ["a", "b"];
        let lat = Lattice::build(&readings, &lex, &ctx).expect("建立詞格");
        let paths = Viterbi::nbest(&lat, 2);
        assert_eq!(paths[0].surface(), "甲丙");
        assert_eq!(paths[0].cost, 20);
        assert_eq!(paths[1].cost, 110, "跨詞性路徑被懲罰");
    }

    #[test]
    fn test_unknown_only_input_still_has_a_path() {
        let lex = InMemoryLexicon::new();
        let ctx = ScoreContext::default();
        let lat = Lattice::build(&["a"], &lex, &ctx).expect("未知詞邊仍可建立");
        let paths = Viterbi::nbest(&lat, 0);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].surface(), "a");
        assert_eq!(paths[0].cost, 8);
    }

    #[test]
    fn test_path_surface_key_and_edge_count() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("台", "ㄊㄞˊ", 10, 1));
        lex.push(entry("北", "ㄅㄟˇ", 10, 1));
        let ctx = plain_ctx();
        let readings = ["ㄊㄞˊ", "ㄅㄟˇ"];
        let lat = Lattice::build(&readings, &lex, &ctx).expect("建立詞格");
        let best = Viterbi::best(&lat, &ctx).expect("應有最佳路徑");

        assert_eq!(best.surface(), "台北");
        assert_eq!(best.edge_count(), 2);
        assert!(!best.is_empty());
        assert!(best.key().contains("|台|"));
        assert!(best.key().contains('\x1f'));
    }

    #[test]
    fn test_scenario_zh_long_word_beats_homophones() {
        // 改寫前身 scenario：注音輸入打「ㄓㄨˋ ㄧㄣ」時，詞典的長詞要勝過單字組合。
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("注", "ㄓㄨˋ", 10, 1));
        lex.push(entry("音", "ㄧㄣ", 10, 1));
        lex.push(entry("注音", "ㄓㄨˋ ㄧㄣ", 15, 1));
        lex.push(entry("注意", "ㄓㄨˋ ㄧˋ", 15, 1));
        let ctx = ScoreContext::default();
        let readings = ["ㄓㄨˋ", "ㄧㄣ"];
        let lat = Lattice::build(&readings, &lex, &ctx).expect("建立詞格");

        let best = Viterbi::best(&lat, &ctx).expect("應有最佳路徑");
        assert_eq!(best.surface(), "注音");
        assert_eq!(best.edge_count(), 1);
        assert_eq!(best.cost, 14);
        assert_eq!(best.edges[0].reading, "ㄓㄨˋ ㄧㄣ");
        assert_eq!(best.edges[0].source, CandidateSource::Lexicon);
    }

    #[test]
    fn test_scenario_unknown_syllable_still_decodes() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("注", "ㄓㄨˋ", 10, 1));
        let ctx = ScoreContext {
            unknown_cost: 100,
            ..Default::default()
        };
        let readings = ["ㄓㄨˋ", "ㄅㄧㄤ"];
        let lat = Lattice::build(&readings, &lex, &ctx).expect("建立詞格");

        let best = Viterbi::best(&lat, &ctx).expect("未知詞仍應有路徑");
        assert_eq!(best.surface(), "注ㄅㄧㄤ");
        assert_eq!(best.cost, 111, "注 10 + 未知詞 100 + 連接 1");
        assert_eq!(best.edges[1].source, CandidateSource::Literal);
    }

    #[test]
    fn test_nbest_truncates_to_available_paths() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("甲", "a", 5, 1));
        let ctx = plain_ctx();
        let readings = ["a"];
        let lat = Lattice::build(&readings, &lex, &ctx).expect("建立詞格");
        let paths = Viterbi::nbest(&lat, 10);
        assert_eq!(paths.len(), 2, "詞典邊與未知詞邊各一條");
        assert_eq!(paths[0].surface(), "甲");
        assert_eq!(paths[0].cost, 5);
        assert_eq!(paths[1].surface(), "a");
        assert_eq!(paths[1].cost, 100);
    }
}
