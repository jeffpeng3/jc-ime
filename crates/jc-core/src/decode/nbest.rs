//! N-best 解碼：保留每個前詞的多條路徑，產生不同的完整文字。
//!
//! 與 [`crate::lattice::Viterbi`] 共用成本語意（成本、段數、穩定鍵），
//! 另外支援鎖定與尾端前綴。Go 的 beam 近似搜尋在成本模型下不需要。
//!
//! 路徑以共享的前驅節點保存：延伸時只新增一個節點，不複製整條路徑；排序用
//! 預先算好的指紋，只有需要與舊行為逐位一致的地方才重建完整穩定鍵。

use std::collections::BTreeMap;

use super::repeat::repeat_penalty;
use super::{
    allowed, fingerprint_after, first_chars, locked_in, path_less_stable, put_best, readings_key,
    Decoder, Lattice, Locks, PathState, Seg, BOS, ROOT_FINGERPRINT,
};
use crate::candidate::CandidateSource;

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

/// 每個前詞狀態最多保留幾條中間路徑。
///
/// 最終只輸出 `max_candidates` 條不同顯示字串；中間層同一個前詞狀態的第二條
/// 以後幾乎不可能贏過其他前詞的最佳路徑，卻會在每一層被複製、排序。固定留 2
/// 條，讓「同一詞面、不同分詞」仍有機會用較低的成本勝出。最後一層例外，必須
/// 留滿 `k` 條，因為那裡每一條都會成為一個完整候選。
const CONTEXT_PATHS: usize = 2;

/// 空路徑的虛擬根節點。
const ROOT_NODE: u32 = 0;

/// 共享路徑節點：每個節點只保存這一步與父節點。
struct Node {
    parent: u32,
    seg: Seg,
    /// 這一步的詞性。
    last: u16,
    /// 到這個節點為止的段數。
    len: usize,
    cost: i32,
    capped: bool,
    fingerprint: u64,
}

/// 路徑節點池；索引就是路徑身分。
#[derive(Default)]
struct Arena {
    nodes: Vec<Node>,
}

impl Arena {
    fn new() -> Self {
        let root = Node {
            parent: ROOT_NODE,
            seg: Seg::new("", 0, 0),
            last: BOS,
            len: 0,
            cost: 0,
            capped: false,
            fingerprint: ROOT_FINGERPRINT,
        };
        Self { nodes: vec![root] }
    }

    /// 接上一段片段，回傳新路徑。
    fn push(&mut self, parent: u32, seg: Seg, last: u16, cost: i32, capped: bool) -> u32 {
        let previous = &self.nodes[parent as usize];
        let len = previous.len + 1;
        let fingerprint = fingerprint_after(previous.fingerprint, &seg);
        self.nodes.push(Node {
            parent,
            seg,
            last,
            len,
            cost,
            capped,
            fingerprint,
        });
        (self.nodes.len() - 1) as u32
    }

    fn node(&self, id: u32) -> &Node {
        &self.nodes[id as usize]
    }

    fn prev_word(&self, id: u32) -> Option<&str> {
        (id != ROOT_NODE).then(|| self.node(id).seg.word.as_str())
    }

    /// 成本、段數、指紋；插入熱路徑只比這個。
    fn less(&self, a: u32, b: u32) -> std::cmp::Ordering {
        let (a, b) = (self.node(a), self.node(b));
        (a.cost, a.len, a.fingerprint).cmp(&(b.cost, b.len, b.fingerprint))
    }

    /// 與舊 `PathState::key` 相同的完整穩定鍵，只在需要逐位一致時建立。
    fn key(&self, id: u32) -> String {
        let node = self.node(id);
        let mut parts = Vec::with_capacity(node.len);
        let mut current = id;
        while current != ROOT_NODE {
            let node = self.node(current);
            parts.push(node.seg.key());
            current = node.parent;
        }
        parts.reverse();
        parts.join("\x1f")
    }

    /// 成本、段數、穩定鍵；排序結果與舊版一致。
    fn less_stable(&self, a: u32, b: u32) -> std::cmp::Ordering {
        let (na, nb) = (self.node(a), self.node(b));
        (na.cost, na.len)
            .cmp(&(nb.cost, nb.len))
            .then_with(|| self.key(a).cmp(&self.key(b)))
    }

    /// 把路徑還原成片段。
    fn segments(&self, id: u32) -> Vec<Seg> {
        let node = self.node(id);
        let mut out = Vec::with_capacity(node.len);
        let mut current = id;
        while current != ROOT_NODE {
            let node = self.node(current);
            out.push(node.seg.clone());
            current = node.parent;
        }
        out.reverse();
        out
    }

    /// 兩條路徑是否是同一個分詞；指紋先篩，只有相同時才建穩定鍵。
    fn same_path(&self, a: u32, b: u32) -> bool {
        a == b
            || (self.node(a).fingerprint == self.node(b).fingerprint && self.key(a) == self.key(b))
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
    /// `beam` 限制下這個位置要展開的狀態鍵。
    ///
    /// 有 bigram 時用 `beam`；沒有 bigram（`beam = 0`）時仍以最終候選數 `k`
    /// 為上限。完全不剪枝會讓同音字極多的讀音把整條 beam 展開，而展開超過
    /// `k` 個前詞狀態本來就不可能全部進到輸出。
    fn frontier_keys<'a>(
        &self,
        level: &BTreeMap<(&'a str, u16), Vec<u32>>,
        arena: &Arena,
        k: usize,
    ) -> Vec<(&'a str, u16)> {
        let beam = if self.beam == 0 { k.max(1) } else { self.beam };
        if level.len() <= beam {
            return level.keys().cloned().collect();
        }
        let mut scored: Vec<(&(&str, u16), &Vec<u32>)> = level.iter().collect();
        scored.sort_by(|a, b| arena.less_stable(a.1[0], b.1[0]));
        scored.truncate(beam);
        scored.into_iter().map(|(key, _)| *key).collect()
    }

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
        let keep = k.min(CONTEXT_PATHS);
        let mut arena = Arena::new();
        let mut states: Vec<BTreeMap<(&str, u16), Vec<u32>>> = vec![BTreeMap::new(); n + 1];
        states[0].insert(("", BOS), vec![ROOT_NODE]);
        for pos in 0..n {
            let level = states[pos].clone();
            if level.is_empty() {
                continue;
            }
            for key in self.frontier_keys(&level, &arena, k) {
                let paths = &level[&key];
                for edge in lat.edges_from(pos) {
                    if !allowed(locks, edge.start, edge.end, &edge.surface) {
                        continue;
                    }
                    for &path in paths {
                        stats.scored_extensions += 1;
                        let cost = arena.node(path).cost;
                        let last = arena.node(path).last;
                        let mut step = self.scorer.edge_cost(arena.prev_word(path), last, edge);
                        let penalty = repeat_penalty(
                            self.lex,
                            lat,
                            (path != ROOT_NODE).then(|| &arena.node(path).seg),
                            edge,
                        );
                        if let Some(p) = penalty {
                            step = step.saturating_add(p);
                        }
                        let seg = Seg::from_edge(edge);
                        let next = arena.push(
                            path,
                            seg,
                            edge.pos,
                            cost.saturating_add(step),
                            arena.node(path).capped || penalty.is_some(),
                        );
                        // 最後一層的所有路徑都會成為完整候選，必須留滿 `k` 條；
                        // 中間層同一前詞只要留幾條，避免同音字把 beam 撐爆。
                        let limit = if edge.end == n { k } else { keep };
                        insert_top(
                            &arena,
                            states[edge.end]
                                .entry((edge.surface.as_str(), edge.pos))
                                .or_default(),
                            next,
                            limit,
                        );
                    }
                }
            }
        }
        let mut result: Vec<Hypothesis> = Vec::new();
        for paths in states[n].values() {
            for &path in paths {
                offer_hypothesis(&mut result, hypothesis_of(&arena, path, false), k);
            }
        }
        if prefix {
            let top_b = self.top_b();
            let own = self.tail_chars(lat);
            for (at, state) in states.iter().enumerate().take(n) {
                if locked_in(locks, at, n) || state.is_empty() {
                    continue;
                }
                let k_left = n - at;
                let mut level: Vec<u32> = state.values().flatten().copied().collect();
                level.sort_by(|a, b| arena.less_stable(*a, *b));
                level.truncate(top_b);
                let mut group: BTreeMap<String, u32> = BTreeMap::new();
                for entry in self.tail_entries(lat, at, k_left) {
                    let display = first_chars(&entry.surface, k_left);
                    for &st in &level {
                        stats.scored_extensions += 1;
                        let cost = arena.node(st).cost.saturating_add(self.scorer.arc_cost(
                            arena.prev_word(st),
                            arena.node(st).last,
                            self.ctx.word_cost(entry.cost, entry.units()),
                            entry.pos,
                            &entry.surface,
                            CandidateSource::Lexicon,
                        ));
                        let seg = Seg::partial(display.clone(), at, n);
                        let cand = arena.push(st, seg, entry.pos, cost, arena.node(st).capped);
                        match group.get(&display) {
                            Some(&old) if !arena.less_stable(cand, old).is_lt() => {}
                            _ => {
                                group.insert(display.clone(), cand);
                            }
                        }
                    }
                }
                for (display, path) in group {
                    let weak = weak_prefix(&own, at, n, &display);
                    offer_hypothesis(&mut result, hypothesis_of(&arena, path, weak), k);
                }
            }
        }
        for level in &states {
            for paths in level.values() {
                stats.retained_prefixes += paths.len();
            }
        }
        // 選字模型只重排首選；沒有安裝模型或不符合模型契約時保持統計順序。
        crate::zh::select::rerank_nbest(&mut result, lat.readings());
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

        let mut states: Vec<BTreeMap<(String, u16), PathState>> = vec![BTreeMap::new(); n + 1];
        states[0].insert(
            (String::new(), BOS),
            PathState {
                last: BOS,
                cost: 0,
                segs: Vec::new(),
                capped: false,
            },
        );
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
                        cost: path.cost.saturating_add(self.step_cost(path, edge)),
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
            if best.as_ref().is_none_or(|b| path_less_stable(p, b).is_lt()) {
                best = Some(p.clone());
            }
        }
        out.best = best.map(|p| hypothesis_of_path(&p, false));
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

fn hypothesis_of(arena: &Arena, path: u32, weak: bool) -> Hypothesis {
    let segments = arena.segments(path);
    let node = arena.node(path);
    Hypothesis {
        text: segments.iter().map(|s| s.word.as_str()).collect(),
        cost: node.cost,
        segments,
        non_word_repeat: node.capped,
        weak_prefix: weak,
    }
}

fn hypothesis_of_path(p: &PathState, weak: bool) -> Hypothesis {
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

/// 插入前 k 條路徑；相同分詞只保留成本較低者。
fn insert_top(arena: &Arena, list: &mut Vec<u32>, path: u32, k: usize) {
    if let Some(i) = list.iter().position(|p| arena.same_path(*p, path)) {
        if arena.node(path).cost < arena.node(list[i]).cost {
            list[i] = path;
            list.sort_by(|a, b| arena.less(*a, *b));
        }
        return;
    }
    list.push(path);
    list.sort_by(|a, b| arena.less(*a, *b));
    list.truncate(k);
}
