//! 人稱代名詞的性別寫法（對照 `core/personal/pronoun.go`）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{Model, Seg};

const PRONOUN_PASSIVE: f64 = 0.25;
const PRONOUN_WINDOW: usize = 6;
const PRONOUN_CUE_REPEATS: f64 = 2.0;
const PRONOUN_CUE_SHARE: f64 = 2.0 / 3.0;
const PRONOUN_GLOBAL_REPEATS: f64 = 5.0;
const PRONOUN_GLOBAL_SHARE: f64 = 0.8;
const PRONOUN_DECAY_AT: f64 = 40.0;
const MAX_PRONOUN_CUES: usize = 256;

fn pair_of(ch: char) -> Option<(char, char, bool)> {
    match ch {
        '他' => Some(('他', '她', false)),
        '她' => Some(('他', '她', true)),
        '你' => Some(('你', '妳', false)),
        '妳' => Some(('你', '妳', true)),
        _ => None,
    }
}

/// 一組代名詞在某條件下的用法次數。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PronounCount {
    /// 變體次數。
    pub variant: f64,
    /// 中性次數。
    pub neutral: f64,
    /// 明確選取變體次數。
    pub manual_variant: f64,
    /// 明確選取中性次數。
    pub manual_neutral: f64,
    /// 最近一次明確選取：1 變體、-1 中性、0 尚無。
    pub last_manual: i32,
    /// 最近更新序號。
    pub updated: u64,
}

impl PronounCount {
    fn add(&mut self, variant: bool, manual: bool) {
        let weight = if manual { 1.0 } else { PRONOUN_PASSIVE };
        if variant {
            self.variant += weight;
            if manual {
                self.manual_variant += 1.0;
                self.last_manual = 1;
            }
        } else {
            self.neutral += weight;
            if manual {
                self.manual_neutral += 1.0;
                self.last_manual = -1;
            }
        }
        if self.variant + self.neutral > PRONOUN_DECAY_AT {
            self.variant /= 2.0;
            self.neutral /= 2.0;
            self.manual_variant /= 2.0;
            self.manual_neutral /= 2.0;
        }
    }

    fn share(&self) -> f64 {
        let total = self.variant + self.neutral;
        if total == 0.0 {
            0.0
        } else {
            self.variant / total
        }
    }
}

/// 一組代名詞的使用者習慣。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PronounHabit {
    /// 整體用法。
    pub all: PronounCount,
    /// 鄰近詞 → 在它附近的用法。
    pub cues: BTreeMap<String, PronounCount>,
    /// 更新序號。
    pub serial: u64,
}

/// 路徑中一個人稱代名詞字。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PronounSite {
    /// 路徑索引。
    pub seg: usize,
    /// 字在詞內的位置。
    pub offset: usize,
    /// 中性字。
    pub neutral: char,
    /// 目前寫成變體。
    pub variant: bool,
    /// 前後幾個音節內的其他詞。
    pub cues: Vec<String>,
}

/// 把 word 第 offset 個字換成同組的另一種寫法。
pub fn swap_pronoun(word: &str, offset: usize) -> String {
    let mut chars: Vec<char> = word.chars().collect();
    if let Some(&ch) = chars.get(offset) {
        if let Some((neutral, variant, is_variant)) = pair_of(ch) {
            chars[offset] = if is_variant { neutral } else { variant };
        }
    }
    chars.into_iter().collect()
}

/// 把「她」「妳」寫成中性字。
pub fn neutral_pronouns(text: &str) -> String {
    text.chars()
        .map(|ch| pair_of(ch).map_or(ch, |(neutral, _, _)| neutral))
        .collect()
}

impl Model {
    /// 找出路徑中的人稱代名詞字。
    pub fn pronoun_sites(&self, path: &[Seg]) -> Vec<PronounSite> {
        let mut sites = Vec::new();
        for (index, seg) in path.iter().enumerate() {
            if seg.partial {
                continue;
            }
            let chars: Vec<char> = seg.word.chars().collect();
            for (offset, &ch) in chars.iter().enumerate() {
                let Some((neutral, _, is_variant)) = pair_of(ch) else {
                    continue;
                };
                if chars.len() > 1 && !self.lexicon.has(&swap_pronoun(&seg.word, offset)) {
                    continue;
                }
                let position = seg.start + offset;
                let mut cues = Vec::new();
                let mut seen = Vec::new();
                for (other_index, other) in path.iter().enumerate() {
                    let cue = neutral_pronouns(&other.word);
                    let cue_len = cue.chars().count();
                    if other_index == index
                        || other.partial
                        || cue_len < 2
                        || seen.contains(&cue)
                        || other.end <= position.saturating_sub(PRONOUN_WINDOW)
                        || other.start > position + PRONOUN_WINDOW
                    {
                        continue;
                    }
                    seen.push(cue.clone());
                    cues.push(cue);
                }
                sites.push(PronounSite {
                    seg: index,
                    offset,
                    neutral,
                    variant: is_variant,
                    cues,
                });
            }
        }
        sites
    }

    /// 依習慣判斷是否該寫成變體。
    pub fn pronoun_variant(&self, neutral: char, cues: &[String]) -> bool {
        let Some(habit) = self.store.pronouns.get(&neutral.to_string()) else {
            return false;
        };
        let mut variant = habit.all.manual_variant >= PRONOUN_GLOBAL_REPEATS
            && habit.all.share() >= PRONOUN_GLOBAL_SHARE;
        for cue in cues {
            let Some(count) = habit.cues.get(cue) else {
                continue;
            };
            if count.last_manual < 0 {
                return false;
            }
            if count.manual_variant >= PRONOUN_CUE_REPEATS && count.share() >= PRONOUN_CUE_SHARE {
                variant = true;
            }
        }
        variant
    }

    /// 記錄每個代名詞最後寫成哪一種。
    pub fn learn_pronouns(&mut self, path: &[Seg]) {
        for site in self.pronoun_sites(path) {
            let seg = &path[site.seg];
            let manual = seg.manual && !seg.completion && !seg.suggested;
            let key = site.neutral.to_string();
            let mut habit = self.store.pronouns.remove(&key).unwrap_or_default();
            let create = manual && (site.variant || self.pronoun_variant(site.neutral, &[]));
            habit.serial += 1;
            habit.all.add(site.variant, manual);
            for cue in &site.cues {
                if habit.cues.contains_key(cue) || create {
                    let mut count = habit.cues.get(cue).copied().unwrap_or_default();
                    count.add(site.variant, manual);
                    count.updated = habit.serial;
                    habit.cues.insert(cue.clone(), count);
                }
            }
            if habit.cues.len() > MAX_PRONOUN_CUES {
                let mut keys: Vec<_> = habit.cues.keys().cloned().collect();
                keys.sort_by_key(|key| {
                    (
                        habit.cues.get(key).map_or(0, |count| count.updated),
                        key.clone(),
                    )
                });
                let drop = keys.len() - MAX_PRONOUN_CUES;
                for key in keys.into_iter().take(drop) {
                    habit.cues.remove(&key);
                }
            }
            self.store.pronouns.insert(key, habit);
        }
    }

    /// 撤銷「她」「妳」或含它們的代名詞詞。
    pub fn forget_pronouns(&mut self, word: &str) {
        let chars: Vec<char> = word.chars().collect();
        for (offset, &ch) in chars.iter().enumerate() {
            if let Some((neutral, _, is_variant)) = pair_of(ch) {
                if is_variant && (chars.len() == 1 || self.lexicon.has(&swap_pronoun(word, offset)))
                {
                    self.store.pronouns.remove(&neutral.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::{InMemoryLexicon, LexEntry};
    use crate::personal::{Model, NoLm};

    fn model() -> Model {
        let mut lex = InMemoryLexicon::new();
        for word in ["他們", "她們", "其他", "男朋友"] {
            lex.push(LexEntry::new(word, "ㄊㄚ", 0, 0));
        }
        Model::new(Box::new(lex), Box::new(NoLm))
    }

    fn seg(word: &str, start: usize, end: usize, manual: bool) -> Seg {
        Seg {
            word: word.to_string(),
            start,
            end,
            manual,
            ..Seg::default()
        }
    }

    #[test]
    fn sites_skip_fixed_words() {
        let m = model();
        let sites = m.pronoun_sites(&[seg("其他", 0, 1, false), seg("他", 1, 2, false)]);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].neutral, '他');
    }

    #[test]
    fn global_habit_needs_five_manual_variant_picks() {
        let mut m = model();
        for _ in 0..5 {
            m.learn_pronouns(&[seg("她", 0, 1, true)]);
        }
        assert!(m.pronoun_variant('他', &[]));
        for _ in 0..5 {
            m.learn_pronouns(&[seg("他", 0, 1, true)]);
        }
        assert!(!m.pronoun_variant('他', &[]));
    }

    #[test]
    fn swap_and_neutralize() {
        assert_eq!(swap_pronoun("她", 0), "他");
        assert_eq!(swap_pronoun("妳好", 0), "你好");
        assert_eq!(neutral_pronouns("她們"), "他們");
    }
}
