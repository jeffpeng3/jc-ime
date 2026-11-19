//! N-best 解碼：保留每個前詞的多條路徑，產生不同的完整文字。
//!
//! 與 [`crate::lattice::Viterbi`] 共用成本語意（成本、段數、穩定鍵），
//! 另外支援鎖定與尾端前綴。Go 的 beam 近似搜尋在成本模型下不需要。

use std::collections::BTreeMap;

use super::repeat::repeat_penalty;
use super::{
    allowed, first_chars, locked_in, path_less, put_best, readings_key, Decoder, Lattice, Locks,
    PathState, Seg, BOS,
};

/// 完整顯示字串及其最低成本路徑。
///
/// 同字串的不同分詞只保留成本最低者。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hypothesis {
    /// 完整顯示字串。
    pub text: String,
    /// 整條路徑的成本，越小越優先。
    pub cost: i32,
    /// 這條路徑的片段。
    pub segments: Vec<Seg>,
    /// 套用了非詞疊字的成本懲罰。
    pub non_word_repeat: bool,
    /// 尾端是只打了一個音節的長詞開頭，而顯示的字不是這個音節自己的邊。
    pub weak_prefix: bool,
}

fn empty_path() -> PathState {
    PathState {
        last: BOS,
        cost: 0,
        segs: Vec::new(),
        capped: false,
    }
}

/// 搜尋統計。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchStats {
    /// 評分過的延伸數。
    pub scored_extensions: usize,
    /// 保留的前綴路徑數。
    pub retained_prefixes: usize,
    /// 被剪掉的上下文數；成本模型沒有 beam，固定 0。
    pub pruned_contexts: usize,
}

/// 目標稽核：以已知文字限制路徑，只用於錯誤分層。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditTarget {
    /// 可由詞（含單字）組成。
    pub surface_segmentable: bool,
    /// 讀音可達（含未裁切的詞庫前綴）。
    pub reading_reachable: bool,
    /// 在現有詞格上的最佳路徑。
    pub best: Option<Hypothesis>,
}

impl Decoder<'_> {
    /// N-best 解碼。
    pub fn nbest(
        &self,
        lat: &Lattice,
        locks: &Locks,
        prefix: bool,
        k: usize,
    ) -> (Vec<Hypothesis>, SearchStats) {
        let mut stats = SearchStats::default();
        if k == 0 || lat.is_empty() {
            return (Vec::new(), stats);
        }
        let n = lat.len();
        let mut states: Vec<BTreeMap<u16, Vec<PathState>>> = vec![BTreeMap::new(); n + 1];
        states[0].insert(BOS, vec![empty_path()]);
        for pos in 0..n {
            let level = states[pos].clone();
            if level.is_empty() {
                continue;
            }
            for edge in lat.edges_from(pos) {
                if !allowed(locks, edge.start, edge.end, &edge.surface) {
                    continue;
                }
                for (last, paths) in &level {
                    for path in paths {
                        stats.scored_extensions += 1;
                        let mut step = self.step_cost(*last, edge);
                        let penalty = repeat_penalty(self.lex, lat, &path.segs, edge);
                        if let Some(p) = penalty {
                            step = step.saturating_add(p);
                        }
                        let mut next = PathState {
                            last: edge.pos,
                            cost: path.cost.saturating_add(step),
                            segs: path.segs.clone(),
                            capped: path.capped || penalty.is_some(),
                        };
                        next.segs.push(Seg::from_edge(edge));
                        insert_top(states[edge.end].entry(edge.pos).or_default(), next, k);
                    }
                }
            }
        }
        let mut result: Vec<Hypothesis> = Vec::new();
        for paths in states[n].values() {
            for p in paths {
                offer_hypothesis(&mut result, hypothesis_of(p, false), k);
            }
        }
        if prefix {
            let top_b = self.top_b();
            let own = self.tail_chars(lat);
            for at in 0..n {
                if locked_in(locks, at, n) || states[at].is_empty() {
                    continue;
                }
                let k_left = n - at;
                let mut level: Vec<PathState> = states[at].values().flatten().cloned().collect();
                level.sort_by(path_less);
                level.truncate(top_b);
                let mut group: BTreeMap<String, PathState> = BTreeMap::new();
                for entry in self.tail_entries(lat, at, k_left) {
                    let display = first_chars(&entry.surface, k_left);
                    for st in &level {
                        stats.scored_extensions += 1;
                        let cost = st
                            .cost
                            .saturating_add(self.step_cost_raw(st.last, entry.pos))
                            .saturating_add(self.ctx.word_cost(entry.cost, entry.units()));
                        let mut segs = st.segs.clone();
                        segs.push(Seg::partial(display.clone(), at, n));
                        let cand = PathState {
                            last: entry.pos,
                            cost,
                            segs,
                            capped: st.capped,
                        };
                        match group.get(&display) {
                            Some(old) if !path_less(&cand, old).is_lt() => {}
                            _ => {
                                group.insert(display.clone(), cand);
                            }
                        }
                    }
                }
                for (display, p) in group {
                    let weak = weak_prefix(&own, at, n, &display);
                    offer_hypothesis(&mut result, hypothesis_of(&p, weak), k);
                }
            }
        }
        for level in &states {
            for paths in level.values() {
                stats.retained_prefixes += paths.len();
            }
        }
        (result, stats)
    }

    /// 以已知文字限制路徑，只用於錯誤分層。
    pub fn audit_target(
        &self,
        lat: &Lattice,
        locks: &Locks,
        prefix: bool,
        text: &str,
    ) -> AuditTarget {
        let n = lat.len();
        let runes: Vec<char> = text.chars().collect();
        let mut out = AuditTarget::default();
        if runes.len() != n || n == 0 {
            return out;
        }
        let mut surface = vec![false; n + 1];
        surface[0] = true;
        for at in 0..n {
            if !surface[at] {
                continue;
            }
            for end in (at + 1)..=n {
                let word: String = runes[at..end].iter().collect();
                if lat
                    .edges_from(at)
                    .iter()
                    .any(|e| e.end == end && e.surface == word)
                {
                    surface[end] = true;
                }
            }
        }
        out.surface_segmentable = surface[n];

        let reading = readings_key(lat.readings());
        let exact: Vec<&crate::lattice::LexEntry> = self
            .lex
            .lookup(&reading)
            .iter()
            .filter(|e| e.surface.chars().take(n).eq(runes.iter().copied()))
            .collect();
        out.reading_reachable = !exact.is_empty()
            || self
                .lex
                .prefix(&reading)
                .iter()
                .any(|e| e.units() > n && e.surface.chars().take(n).eq(runes.iter().copied()));

        let mut states: Vec<BTreeMap<u16, PathState>> = vec![BTreeMap::new(); n + 1];
        states[0].insert(BOS, empty_path());
        for at in 0..n {
            let level = states[at].clone();
            if level.is_empty() {
                continue;
            }
            for edge in lat.edges_from(at) {
                let word: String = runes[at..edge.end].iter().collect();
                if edge.surface != word || !allowed(locks, edge.start, edge.end, &edge.surface) {
                    continue;
                }
                for path in level.values() {
                    let mut next = PathState {
                        last: edge.pos,
                        cost: path.cost.saturating_add(self.step_cost(path.last, edge)),
                        segs: path.segs.clone(),
                        capped: path.capped,
                    };
                    next.segs.push(Seg::from_edge(edge));
                    put_best(&mut states[edge.end], next);
                }
            }
        }
        let _ = prefix;
        let mut best: Option<PathState> = None;
        for p in states[n].values() {
            if best.as_ref().map_or(true, |b| path_less(p, b).is_lt()) {
                best = Some(p.clone());
            }
        }
        out.best = best.map(|p| hypothesis_of(&p, false));
        out
    }

    /// 最後一個位置自己有的字面，供弱前綴判斷。
    fn tail_chars(&self, lat: &Lattice) -> Vec<String> {
        let n = lat.len();
        if n == 0 {
            return Vec::new();
        }
        lat.edges_from(n - 1)
            .iter()
            .filter(|e| e.end == n)
            .map(|e| e.surface.clone())
            .collect()
    }
}

fn weak_prefix(own: &[String], at: usize, n: usize, display: &str) -> bool {
    n - at == 1 && !own.is_empty() && !own.iter().any(|w| w == display)
}

fn hypothesis_of(p: &PathState, weak: bool) -> Hypothesis {
    Hypothesis {
        text: p.segs.iter().map(|s| s.word.as_str()).collect(),
        cost: p.cost,
        segments: p.segs.clone(),
        non_word_repeat: p.capped,
        weak_prefix: weak,
    }
}

/// 依成本保留前 k 個不同文字。
fn offer_hypothesis(rows: &mut Vec<Hypothesis>, h: Hypothesis, k: usize) {
    if let Some(i) = rows.iter().position(|old| old.text == h.text) {
        if rows[i].cost <= h.cost {
            return;
        }
        rows.remove(i);
    }
    let i = rows.partition_point(|r| (r.cost, r.segments.len()) <= (h.cost, h.segments.len()));
    if i >= k {
        return;
    }
    rows.insert(i, h);
    rows.truncate(k);
}

/// 插入前 k 條路徑；相同鍵只保留成本較低者。
fn insert_top(list: &mut Vec<PathState>, path: PathState, k: usize) {
    let key = path.key();
    if let Some(i) = list.iter().position(|p| p.key() == key) {
        if path.cost < list[i].cost {
            list[i] = path;
            list.sort_by(path_less);
        }
        return;
    }
    list.push(path);
    list.sort_by(path_less);
    list.truncate(k);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::Lock;
    use crate::lattice::{InMemoryLexicon, LexEntry};
    use crate::score::{ConnectionMatrix, ScoreContext};

    fn entry(surface: &str, reading: &str, cost: i32) -> LexEntry {
        LexEntry::new(surface, reading, cost, 1)
    }

    fn ctx() -> ScoreContext {
        let mut ctx = ScoreContext::default();
        ctx.length_bonus = 0;
        ctx.unknown_cost = 100;
        ctx.connection = ConnectionMatrix::with_fallback(0);
        ctx
    }

    #[test]
    fn nbest_dedupes_same_text_and_orders_by_cost() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("甲", "ㄐㄧㄚˇ", 5));
        lex.push(entry("假", "ㄐㄧㄚˇ", 6));
        lex.push(entry("乙", "ㄧˇ", 5));
        lex.push(entry("甲乙", "ㄐㄧㄚˇ ㄧˇ", 9));
        let ctx = ctx();
        let lat = Lattice::build(&["ㄐㄧㄚˇ", "ㄧˇ"], &lex, &ctx).expect("建立詞格");
        let d = Decoder::new(&lex, &ctx);
        let (rows, _) = d.nbest(&lat, &Locks::new(), false, 8);
        assert_eq!(rows[0].text, "甲乙");
        assert_eq!(rows[0].cost, 9, "長度折扣已關閉");
        let texts: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
        assert!(texts.contains(&"假乙"));
        assert_eq!(texts.iter().filter(|t| **t == "甲乙").count(), 1);
    }

    #[test]
    fn nbest_respects_locks() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("甲", "ㄚ", 1));
        lex.push(entry("乙", "ㄚ", 2));
        lex.push(entry("丙", "ㄅ", 1));
        let ctx = ctx();
        let lat = Lattice::build(&["ㄚ", "ㄅ"], &lex, &ctx).expect("建立詞格");
        let d = Decoder::new(&lex, &ctx);
        let mut locks = Locks::new();
        locks.insert(
            0,
            Lock {
                word: "乙".to_string(),
                len: 1,
                ..Default::default()
            },
        );
        let (rows, _) = d.nbest(&lat, &locks, false, 8);
        assert_eq!(rows[0].text, "乙丙");
    }

    #[test]
    fn nbest_lists_are_nested_and_empty_input_is_empty() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("甲", "ㄚ", 1));
        lex.push(entry("乙", "ㄚ", 2));
        lex.push(entry("丙", "ㄚ", 3));
        let ctx = ctx();
        let lat = Lattice::build(&["ㄚ"; 3], &lex, &ctx).expect("建立詞格");
        let d = Decoder::new(&lex, &ctx);
        let (full, _) = d.nbest(&lat, &Locks::new(), true, 16);
        assert!(!full.is_empty());
        let (best, _) = d.nbest(&lat, &Locks::new(), true, 1);
        assert_eq!(best[0], full[0]);
        for _ in 0..5 {
            assert_eq!(
                d.nbest(&lat, &Locks::new(), true, 4).0,
                d.nbest(&lat, &Locks::new(), true, 4).0
            );
        }
        assert!(d.nbest(&lat, &Locks::new(), true, 0).0.is_empty());
    }

    #[test]
    fn prefix_extension_is_flagged_as_partial() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("沒", "ㄇㄟˊ", 5));
        lex.push(entry("沒有", "ㄇㄟˊ ㄧㄡˇ", 2));
        let ctx = ctx();
        let lat = Lattice::build(&["ㄇㄟˊ"], &lex, &ctx).expect("建立詞格");
        let d = Decoder::new(&lex, &ctx);
        let (rows, _) = d.nbest(&lat, &Locks::new(), true, 8);
        assert!(rows.iter().any(|r| r.segments.last().unwrap().partial));
    }

    #[test]
    fn audit_target_separates_surface_and_reading_reachability() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("甲乙", "ㄚ ㄅ", 5));
        lex.push(entry("丙丁", "ㄚ ㄅ", 6));
        let ctx = ctx();
        let lat = Lattice::build(&["ㄚ", "ㄅ"], &lex, &ctx).expect("建立詞格");
        let d = Decoder::new(&lex, &ctx);
        let a = d.audit_target(&lat, &Locks::new(), false, "丙丁");
        assert!(a.surface_segmentable);
        assert!(a.best.is_some());

        let b = d.audit_target(&lat, &Locks::new(), false, "未知");
        assert!(!b.surface_segmentable);
        assert!(b.best.is_none());
    }
}
