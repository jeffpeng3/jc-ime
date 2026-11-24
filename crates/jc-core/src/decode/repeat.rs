//! 非詞疊字的重複片段還原。
//!
//! Go 版用機率比較詞庫詞與非詞疊字（`capRepeat`）；Rust 的成本模型沒有
//! 機率，改用固定成本懲罰：同一段讀音有詞庫詞，但路徑走的是兩個相同單字
//! 或詞庫沒有的疊字時，加上 [`REPEAT_COST`]。詞庫本身有這個疊字（例如
//! 「看看」）時不罰。

use super::{readings_key, Seg};
use crate::lattice::{Edge, Lattice, LexEntry, Lexicon};

/// 非詞疊字的成本懲罰。
///
/// 對應 Go 的 `repeatMargin`（自然對數 1.0）；成本模型的單位較粗，
/// 取足以讓同讀音的詞庫詞勝出的固定值。
pub const REPEAT_COST: i32 = 10;

/// 詞面的重複種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatArc {
    /// 不必處理。
    NoRepeat,
    /// 單字，接在同一個單字後面時才可能是疊字。
    SingleRepeat,
    /// 兩個相同的字。
    PairRepeat,
}

/// 判斷詞面的重複種類。
pub fn repeat_kind(w: &str) -> RepeatArc {
    let mut chars = w.chars();
    match (chars.next(), chars.next(), chars.next()) {
        // 單一字元：長度一定等於它自己的 UTF-8 長度。
        (Some(_), None, _) => RepeatArc::SingleRepeat,
        (Some(a), Some(b), None) if a == b => RepeatArc::PairRepeat,
        _ => RepeatArc::NoRepeat,
    }
}

/// 這一列接上詞面 `w`（起點 `at`）時是否要檢查疊字。
pub fn may_cap(kind: RepeatArc, prev: Option<&Seg>, at: usize, w: &str) -> bool {
    match kind {
        RepeatArc::SingleRepeat => {
            prev.is_some_and(|prev| prev.end == at && prev.word == w && !prev.partial)
        }
        RepeatArc::PairRepeat => true,
        RepeatArc::NoRepeat => false,
    }
}

/// 路徑接上 `edge` 時要加的非詞疊字成本；`None` 表示不受限。
///
/// `prev` 是走到 `edge.start` 為止的最後一段。
pub fn repeat_penalty(
    lex: &dyn Lexicon,
    lat: &Lattice,
    prev: Option<&Seg>,
    edge: &Edge,
) -> Option<i32> {
    let kind = repeat_kind(&edge.surface);
    if !may_cap(kind, prev, edge.start, &edge.surface) {
        return None;
    }
    match kind {
        RepeatArc::PairRepeat => {
            // 疊字本身就是詞庫詞（看看、試試）時不罰。
            if has_surface(lex.lookup(&edge.reading), &edge.surface) {
                None
            } else {
                Some(REPEAT_COST)
            }
        }
        RepeatArc::SingleRepeat => {
            let prev = lat.readings().get(edge.start.checked_sub(1)?)?;
            let pair = format!("{prev} {}", edge.reading);
            if lex.lookup(&pair).is_empty() {
                Some(REPEAT_COST)
            } else {
                None
            }
        }
        RepeatArc::NoRepeat => None,
    }
}

fn has_surface(entries: &[LexEntry], surface: &str) -> bool {
    entries.iter().any(|e| e.surface == surface)
}

/// XX 成段時的讀音鍵，供外部測試與診斷。
pub fn pair_reading(a: &str, b: &str) -> String {
    readings_key(&[a.to_string(), b.to_string()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::CandidateSource;
    use crate::lattice::InMemoryLexicon;

    fn entry(surface: &str, reading: &str, cost: i32) -> LexEntry {
        LexEntry::new(surface, reading, cost, 1)
    }

    fn plain_ctx() -> crate::score::ScoreContext {
        crate::score::ScoreContext {
            unknown_cost: 100,
            ..Default::default()
        }
    }

    fn edge(surface: &str, reading: &str, start: usize, end: usize) -> Edge {
        Edge {
            start,
            end,
            surface: surface.to_string(),
            reading: reading.to_string(),
            cost: 1,
            pos: 1,
            source: CandidateSource::Lexicon,
        }
    }

    #[test]
    fn repeat_kind_classifies_single_pair_and_other() {
        assert_eq!(repeat_kind("恩"), RepeatArc::SingleRepeat);
        assert_eq!(repeat_kind("恩恩"), RepeatArc::PairRepeat);
        assert_eq!(repeat_kind("恩恩恩"), RepeatArc::NoRepeat);
        assert_eq!(repeat_kind("天地"), RepeatArc::NoRepeat);
    }

    #[test]
    fn may_cap_only_matches_adjacent_same_single() {
        let seg = Seg::new("恩", 0, 1);
        assert!(may_cap(RepeatArc::SingleRepeat, Some(&seg), 1, "恩"));
        assert!(!may_cap(RepeatArc::SingleRepeat, Some(&seg), 2, "恩"));
        assert!(may_cap(RepeatArc::PairRepeat, Some(&seg), 1, "恩恩"));
        assert!(!may_cap(RepeatArc::NoRepeat, Some(&seg), 1, "恩"));
        assert!(!may_cap(RepeatArc::SingleRepeat, None, 1, "恩"));
    }

    #[test]
    fn pair_that_is_not_a_lexicon_word_is_penalized() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("嗯嗯", "ㄣ ㄣ", 5));
        // 「恩恩」不是詞庫詞。
        let lat = Lattice::build(&["ㄣ", "ㄣ"], &lex, &plain_ctx()).expect("建立詞格");
        let penalty = repeat_penalty(&lex, &lat, None, &edge("恩恩", "ㄣ ㄣ", 0, 2));
        assert_eq!(penalty, Some(REPEAT_COST));

        // 「嗯嗯」是詞庫詞，不罰。
        assert_eq!(
            repeat_penalty(&lex, &lat, None, &edge("嗯嗯", "ㄣ ㄣ", 0, 2)),
            None
        );
    }

    #[test]
    fn single_repeat_skips_when_reading_is_unknown() {
        let lex = InMemoryLexicon::new();
        let lat = Lattice::build(&["ㄣ", "ㄣ"], &lex, &plain_ctx()).expect("建立詞格");
        let seg = Seg::new("恩", 0, 1);
        assert_eq!(
            repeat_penalty(&lex, &lat, Some(&seg), &edge("恩", "ㄣ", 1, 2)),
            Some(REPEAT_COST)
        );
    }
}
