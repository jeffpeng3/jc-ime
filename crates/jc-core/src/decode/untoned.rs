//! 無聲調輸入的候選裁切與讀音還原。
//!
//! 「聲調選填」（#150）把沒打聲調的音節展開成五個聲調；這裡負責裁切展開後
//! 的候選數量，以及提交時把沒打聲調的音節換回詞庫讀音。
//!
//! 音節假設格來自 `z1-zh-front`（[`SyllableLattice`]），讀音查詢透過
//! `z2-zh-mid` 的 [`Lexicon`]（音節以空白相接的字串）。

use super::{Decoder, SyllableLattice};
use crate::lattice::Lexicon;
use crate::reading::Unit;

/// 每組（起點、終點）經過沒打聲調位置的候選上限。
///
/// 對應 Go 的 `maxUntonedArcs`：沒打聲調的單音節展開成五個聲調後常有幾百個字，
/// 機率排在後面的字幾乎不會贏，仍可從候選窗選到（候選另外列舉）。
pub const MAX_UNTONED_ARCS: usize = 64;

/// 音節格第 `i` 個位置是不是沒打聲調展開的。
pub fn untoned_pos(lat: &SyllableLattice, i: usize) -> bool {
    lat.get(i)
        .map_or(false, |row| row.iter().any(|h| h.untoned))
}

/// `[start, end)` 是否碰到沒打聲調的位置。
pub fn untoned_span(lat: &SyllableLattice, start: usize, end: usize) -> bool {
    (start..end).any(|i| untoned_pos(lat, i))
}

/// 把沒打聲調位置的候選裁到成本最低的 `max` 個。
///
/// 有打聲調的位置不動；裁切只看成本，成本相同時保留原順序。
pub fn cap_untoned_arcs(lat: &SyllableLattice, max: usize) -> SyllableLattice {
    lat.iter()
        .map(|row| {
            if !row.iter().any(|h| h.untoned) || row.len() <= max {
                return row.clone();
            }
            let mut kept = row.clone();
            kept.sort_by(|a, b| {
                a.cost
                    .partial_cmp(&b.cost)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            kept.truncate(max);
            kept
        })
        .collect()
}

/// 詞 `word` 佔音節格 `[start, end)` 時在詞庫裡的讀音。
///
/// 只看成本 0 的假設（使用者打的音；沒打聲調的音節展開成的五個聲調也是成本 0），
/// 以詞庫前綴剪枝逐一組合。找不到時回傳 `None`。
pub fn reading(
    lex: &dyn Lexicon,
    lat: &SyllableLattice,
    start: usize,
    end: usize,
    word: &str,
) -> Option<Vec<Unit>> {
    if start >= end || end > lat.len() {
        return None;
    }
    fn walk(
        lex: &dyn Lexicon,
        lat: &SyllableLattice,
        end: usize,
        word: &str,
        pos: usize,
        acc: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        if pos == end {
            let key = acc.join(" ");
            if lex.lookup(&key).iter().any(|e| e.surface == word) {
                return Some(acc.clone());
            }
            return None;
        }
        for h in &lat[pos] {
            if h.cost > 0.0 {
                continue;
            }
            acc.push(h.syl.as_str().to_string());
            let key = acc.join(" ");
            // `Lexicon::prefix` 不包含完全相等者，走到最後一個音節時要另外
            // 用 `lookup` 判斷；兩者都沒有才剪枝。
            let viable = !lex.prefix(&key).is_empty() || !lex.lookup(&key).is_empty();
            let found = if viable {
                walk(lex, lat, end, word, pos + 1, acc)
            } else {
                None
            };
            if found.is_some() {
                return found;
            }
            acc.pop();
        }
        None
    }
    walk(lex, lat, end, word, start, &mut Vec::new())
        .map(|units| units.into_iter().map(Unit::new).collect())
}

/// 同 [`reading`]，但找的是自動修正詞條的讀音。
///
/// 成本模型還沒有「自動修正詞條」，行為與 [`reading`] 相同。
pub fn correction_reading(
    lex: &dyn Lexicon,
    lat: &SyllableLattice,
    start: usize,
    end: usize,
    word: &str,
) -> Option<Vec<Unit>> {
    reading(lex, lat, start, end, word)
}

impl Decoder<'_> {
    /// 詞 `word` 佔 `[start, end)` 時的詞庫讀音。
    pub fn reading(
        &self,
        lat: &SyllableLattice,
        start: usize,
        end: usize,
        word: &str,
    ) -> Option<Vec<Unit>> {
        reading(self.lex, lat, start, end, word)
    }

    /// 讀音的鍵（音節以空白相接）。
    pub fn reading_key(&self, reading: &[Unit]) -> String {
        crate::reading::key(reading)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::InMemoryLexicon;
    use crate::zh::input::{Exact, Frontend, Hyp};
    use crate::zh::syllable::{untone, Syllable};

    fn entry(surface: &str, reading: &str) -> crate::lattice::LexEntry {
        crate::lattice::LexEntry::new(surface, reading, 1, 1)
    }

    fn untoned_lat(syl: &str) -> SyllableLattice {
        let s = Syllable::parse(syl).expect("合法音節");
        Exact.expand(&[untone(&s)])
    }

    #[test]
    fn cap_keeps_cheapest_hypotheses() {
        let mut row = Vec::new();
        for k in 0..3 * MAX_UNTONED_ARCS {
            let syl = char::from_u32(0x3105 + (k % 40) as u32)
                .unwrap()
                .to_string();
            row.push(Hyp {
                syl: crate::reading::Unit::new(syl),
                cost: k as f64,
                untoned: true,
            });
        }
        let lat: SyllableLattice = vec![row];
        let capped = cap_untoned_arcs(&lat, MAX_UNTONED_ARCS);
        assert_eq!(capped[0].len(), MAX_UNTONED_ARCS);
        assert_eq!(capped[0][0].cost, 0.0);
        assert_eq!(capped[0][1].cost, 1.0);

        let toned: SyllableLattice = vec![vec![Hyp {
            syl: crate::reading::Unit::new("ㄧ"),
            cost: 0.0,
            untoned: false,
        }]];
        assert_eq!(cap_untoned_arcs(&toned, MAX_UNTONED_ARCS)[0].len(), 1);
    }

    #[test]
    fn reading_resolves_untoned_syllables() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("會議", "ㄏㄨㄟˋ ㄧˋ"));
        lex.push(entry("回憶", "ㄏㄨㄟˊ ㄧˋ"));
        lex.push(entry("行", "ㄒㄧㄥˊ"));
        lex.push(entry("行", "ㄏㄤˊ"));

        let lat: SyllableLattice = vec![
            untoned_lat("ㄏㄨㄟ")[0].clone(),
            untoned_lat("ㄧ")[0].clone(),
        ];
        assert_eq!(
            reading(&lex, &lat, 0, 2, "會議").map(|r| crate::reading::key(&r)),
            Some("ㄏㄨㄟˋ ㄧˋ".to_string())
        );
        assert_eq!(
            reading(&lex, &lat, 0, 2, "回憶").map(|r| crate::reading::key(&r)),
            Some("ㄏㄨㄟˊ ㄧˋ".to_string())
        );
        assert!(reading(&lex, &lat, 0, 2, "灰衣").is_none());

        let mixed: SyllableLattice = vec![
            vec![Hyp {
                syl: crate::reading::Unit::new("ㄏㄨㄟˋ"),
                cost: 0.0,
                untoned: false,
            }],
            untoned_lat("ㄧ")[0].clone(),
        ];
        assert!(reading(&lex, &mixed, 0, 2, "回憶").is_none());

        let xing = untoned_lat("ㄒㄧㄥ");
        assert_eq!(
            reading(&lex, &xing, 0, 1, "行").map(|r| crate::reading::key(&r)),
            Some("ㄒㄧㄥˊ".to_string())
        );
    }

    #[test]
    fn untoned_span_detects_marks() {
        let lat = untoned_lat("ㄧ");
        assert!(untoned_pos(&lat, 0));
        assert!(untoned_span(&lat, 0, 1));
        assert!(!untoned_span(&lat, 0, 0));
    }
}
