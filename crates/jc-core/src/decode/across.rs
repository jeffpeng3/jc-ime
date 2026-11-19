//! 跨分段的補全（#143）。
//!
//! 畫面上 `path[from..]` 的片段合起來，可能是某個更長詞的開頭。這裡以成本
//! 換算的權重比較「合起來的詞群」與「目前路徑在這段」，回傳比例讓呼叫端
//! 決定怎麼用。

use super::{readings_key, Completion, Decoder, Lattice, Seg};
use crate::lattice::LexEntry;

impl Decoder<'_> {
    /// 找從 `path[from]` 起、跨過後面幾個分段的補全。
    ///
    /// 回傳 `(補全, 合起來的詞群 ÷（合起來的詞群 ＋ 目前路徑）, 是否有結果)`。
    pub fn complete_across(
        &self,
        lat: &Lattice,
        path: &[Seg],
        from: usize,
    ) -> (Completion, f64, bool) {
        if path.is_empty() || from + 1 >= path.len() {
            return (Completion::default(), 0.0, false);
        }
        let start = path[from].start;
        let k = lat.len().saturating_sub(start);
        let displayed: String = path[from..].iter().map(|s| s.word.as_str()).collect();
        let entries: Vec<LexEntry> = self
            .tail_entries(lat, start, k)
            .into_iter()
            .filter(|e| e.surface.starts_with(&displayed))
            .collect();
        if entries.is_empty() {
            return (Completion::default(), 0.0, false);
        }
        let mut group = 0.0;
        let mut best = Completion::default();
        let mut best_weight = 0.0;
        let left = if from > 0 { &path[from - 1].word } else { "" };
        let _ = left;
        for e in &entries {
            let w = weight(self.ctx.word_cost(e.cost, e.units()));
            group += w;
            if w > best_weight {
                best_weight = w;
                best = Completion {
                    entry: e.clone(),
                    conf: 0.0,
                };
            }
        }
        // 目前路徑在這段的權重：各片段的邊成本與連接成本。
        let mut current = 0.0;
        let mut prev = super::BOS;
        let mut conn_total: i32 = 0;
        for seg in &path[from..] {
            let edge = lat
                .edges_from(seg.start)
                .iter()
                .find(|e| e.end == seg.end && e.surface == seg.word);
            match edge {
                Some(e) => {
                    conn_total = conn_total.saturating_add(self.step_cost(prev, e));
                    current += weight(conn_total);
                    prev = e.pos;
                }
                None => {
                    current += weight(conn_total.saturating_add(1));
                }
            }
        }
        let total = group + current;
        if total <= 0.0 {
            return (Completion::default(), 0.0, false);
        }
        best.conf = best_weight / group.max(f64::MIN_POSITIVE);
        (best, group / total, true)
    }

    /// 合起來的顯示字串，供診斷。
    pub fn displayed(&self, path: &[Seg], from: usize) -> String {
        readings_key(
            &path[from..]
                .iter()
                .map(|s| s.word.clone())
                .collect::<Vec<_>>(),
        )
    }
}

fn weight(cost: i32) -> f64 {
    (-f64::from(cost)).exp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::InMemoryLexicon;
    use crate::score::{ConnectionMatrix, ScoreContext};

    fn entry(surface: &str, reading: &str, cost: i32) -> LexEntry {
        LexEntry::new(surface, reading, cost, 1)
    }

    fn ctx() -> ScoreContext {
        ScoreContext {
            length_bonus: 0,
            unknown_cost: 100,
            connection: ConnectionMatrix::with_fallback(0),
            ..Default::default()
        }
    }

    #[test]
    fn complete_across_finds_merged_word() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("沒有", "ㄇㄟˊ ㄧㄡˇ", 5));
        lex.push(entry("不", "ㄅㄨˋ", 5));
        lex.push(entry("不同", "ㄅㄨˋ ㄊㄨㄥˊ", 6));
        lex.push(entry("沒有不同", "ㄇㄟˊ ㄧㄡˇ ㄅㄨˋ ㄊㄨㄥˊ", 9));
        lex.push(entry(
            "沒有不透風的牆",
            "ㄇㄟˊ ㄧㄡˇ ㄅㄨˋ ㄊㄡˋ ㄈㄥ ㄉㄜ˙ ㄑㄧㄤˊ",
            12,
        ));
        let ctx = ctx();
        let lat = Lattice::build(&["ㄇㄟˊ", "ㄧㄡˇ", "ㄅㄨˋ"], &lex, &ctx).expect("建立詞格");
        let d = Decoder::new(&lex, &ctx);
        let path = vec![Seg::new("沒有", 0, 2), Seg::partial("不", 2, 3)];
        let (c, ratio, ok) = d.complete_across(&lat, &path, 0);
        assert!(ok);
        assert_eq!(c.entry.surface, "沒有不同");
        assert!(ratio > 0.0 && ratio < 1.0);
        assert_eq!(d.displayed(&path, 0), "沒有 不");

        for from in [1usize, 2] {
            assert!(!d.complete_across(&lat, &path, from).2);
        }
    }
}
