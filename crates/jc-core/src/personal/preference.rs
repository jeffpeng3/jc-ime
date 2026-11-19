//! 讀音習慣（對照 `core/personal/preference.go`）。

use std::collections::BTreeMap;

use crate::zh::syllable::Syllable;

use super::{pair, reading_key, Model, Seg, Store, CONFIRM_UNIT};

/// 單字不算習慣。
pub const HABIT_MIN_SYLLABLES: usize = 2;
/// 證據門檻。
const HABIT_THRESHOLD: f64 = 2.4;
/// 全部加分的證據量。
const HABIT_FULL: f64 = 2.6;
/// 使用證據折算比例。
const HABIT_USE_SHARE: f64 = 0.6;
const HABIT_SHARE_LOW: f64 = 0.5;
const HABIT_SHARE_FULL: f64 = 0.8;
const HABIT_LEADER_RATIO: f64 = 3.0;

fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HabitLift {
    reading: String,
    lift: f64,
}

/// 讀音習慣索引。
#[derive(Debug, Clone, Default)]
pub struct HabitIndex {
    pub(crate) store: Option<usize>,
    pub(crate) gen: u64,
    pub(crate) time: i64,
    pub(crate) lifts: BTreeMap<String, Vec<HabitLift>>,
}

impl Model {
    /// word 以 reading 讀時的習慣分數。
    pub fn reading_lift(&mut self, word: &str, syls: &[Syllable]) -> f64 {
        if self.params.habit_lift <= 0.0 || self.store.confirmed.is_empty() {
            return 0.0;
        }
        let reading = reading_key(syls);
        let stale = self.habit.store != Some(&self.store as *const _ as usize)
            || self.habit.gen != self.gen
            || self.habit.time != self.store.time;
        if stale {
            self.build_habit_index();
        }
        self.habit
            .lifts
            .get(word)
            .and_then(|lifts| lifts.iter().find(|item| item.reading == reading))
            .map_or(0.0, |item| item.lift)
    }

    fn build_habit_index(&mut self) {
        let now = self.store.time;
        let mut lifts: BTreeMap<String, Vec<HabitLift>> = BTreeMap::new();
        for (key, evidence) in &self.store.confirmed {
            let (word, reading) = super::cut_pair(key);
            let picked = evidence.at(now).long;
            if picked <= 0.0 {
                continue;
            }
            let used = self
                .store
                .evidence_by_word
                .get(word)
                .map_or(0.0, |item| item.at(now).long);
            let total_evidence = picked + HABIT_USE_SHARE * used;
            if total_evidence < HABIT_THRESHOLD || used <= 0.0 {
                continue;
            }
            let (rival, sys_rival, found) = self.rival_use(word, reading);
            if !found || self.lm.uni(word) > HABIT_LEADER_RATIO * sys_rival {
                continue;
            }
            let share = used / (used + rival);
            let lift = self.params.habit_lift
                * clamp01(total_evidence / HABIT_FULL)
                * clamp01((share - HABIT_SHARE_LOW) / (HABIT_SHARE_FULL - HABIT_SHARE_LOW));
            if lift > 0.0 {
                lifts.entry(word.to_string()).or_default().push(HabitLift {
                    reading: reading.to_string(),
                    lift,
                });
            }
        }
        self.habit = HabitIndex {
            store: Some(&self.store as *const _ as usize),
            gen: self.gen,
            time: self.store.time,
            lifts,
        };
    }

    fn rival_use(&self, word: &str, reading: &str) -> (f64, f64, bool) {
        let now = self.store.time;
        let mut rival = 0.0;
        let mut sys_rival: f64 = 0.0;
        let mut found = false;
        let mut seen = vec![word.to_string()];
        for entry in self.lexicon.exact(&super::reading_seq(reading)) {
            if seen.iter().any(|item| item == &entry.surface) {
                continue;
            }
            seen.push(entry.surface.clone());
            found = true;
            rival += self
                .store
                .evidence_by_word
                .get(&entry.surface)
                .map_or(0.0, |item| item.at(now).long);
            sys_rival = sys_rival.max(self.lm.uni(&entry.surface));
        }
        (rival, sys_rival, found)
    }

    /// 手選後記下讀音習慣，同讀音其他詞扣一次。
    pub fn note_confirmed(&mut self, word: &str, reading: &str) {
        let now = self.store.time;
        let key = pair(word, reading);
        let entry = self.store.confirmed.get(&key).copied().unwrap_or_default();
        self.store
            .confirmed
            .insert(key, entry.add(now, CONFIRM_UNIT));
        let others: Vec<String> = self
            .lexicon
            .exact(&super::reading_seq(reading))
            .into_iter()
            .filter(|item| item.surface != word)
            .map(|item| item.surface)
            .collect();
        for other in others {
            let key = pair(&other, reading);
            if let Some(entry) = self.store.confirmed.get(&key).copied() {
                let mut entry = entry.at(now);
                entry.long = (entry.long - CONFIRM_UNIT).max(0.0);
                self.store.confirmed.insert(key, entry);
            }
        }
    }

    /// 照常打著的詞暫停衰減。
    pub(super) fn keep_confirmed(&mut self, syls: &[Syllable], seg: &Seg) {
        if self.store.confirmed.is_empty()
            || seg.partial
            || seg.end - seg.start < HABIT_MIN_SYLLABLES
            || seg.end > syls.len()
        {
            return;
        }
        let key = pair(&seg.word, &reading_key(&syls[seg.start..seg.end]));
        if let Some(mut entry) = self.store.confirmed.get(&key).copied() {
            if self.store.time > entry.updated {
                entry.updated = self.store.time;
                self.store.confirmed.insert(key, entry);
            }
        }
    }
}

impl Store {
    /// 由舊更正推回手選紀錄。
    pub(crate) fn seed_confirmed(&mut self) {
        let mut keys: Vec<String> = self.corrections.keys().cloned().collect();
        keys.sort_by_key(|key| {
            (
                self.corrections.get(key).map_or(0, |item| item.updated),
                key.clone(),
            )
        });
        for key in keys {
            let correction = self.corrections[&key].clone();
            let syllables = super::reading_seq(&correction.reading).len();
            if syllables >= HABIT_MIN_SYLLABLES && correction.word.chars().count() == syllables {
                let pair_key = pair(&correction.word, &correction.reading);
                let entry = self.confirmed.get(&pair_key).copied().unwrap_or_default();
                self.confirmed
                    .insert(pair_key, entry.add(correction.updated, CONFIRM_UNIT));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::{InMemoryLexicon, LexEntry};
    use crate::personal::{Model, NoLm};
    use crate::personal::{UsageEvidence, DAY_SECONDS};

    #[test]
    fn single_pick_is_not_a_habit() {
        let lex = InMemoryLexicon::new();
        let mut m = Model::new(Box::new(lex), Box::new(NoLm));
        let syls = super::super::reading_seq("ㄕˋ ㄐㄧㄝˊ");
        m.note_confirmed("世傑", "ㄕˋ ㄐㄧㄝˊ");
        assert_eq!(m.reading_lift("世傑", &syls), 0.0);
    }

    #[test]
    fn confirmed_decays_and_can_be_seeded() {
        let mut store = Store::new();
        store.corrections.insert(
            "k".to_string(),
            super::super::Correction {
                reading: "ㄕˋ ㄐㄧㄝˊ".to_string(),
                word: "世傑".to_string(),
                updated: 10,
                ..super::super::Correction::default()
            },
        );
        store.seed_confirmed();
        assert!(store.confirmed.contains_key("世傑\tㄕˋ ㄐㄧㄝˊ"));
    }

    #[test]
    fn habit_lift_is_bounded() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("世傑", "ㄕˋ ㄐㄧㄝˊ", 0, 0));
        lex.push(LexEntry::new("是結", "ㄕˋ ㄐㄧㄝˊ", 0, 0));
        let mut m = Model::new(Box::new(lex), Box::new(NoLm));
        m.store.time = 3 * DAY_SECONDS;
        for _ in 0..5 {
            m.note_confirmed("世傑", "ㄕˋ ㄐㄧㄝˊ");
            m.store
                .evidence_by_word
                .entry("世傑".to_string())
                .and_modify(|e| *e = e.add(m.store.time, 1.0))
                .or_insert_with(|| UsageEvidence::default().add(m.store.time, 1.0));
            m.store
                .evidence_by_word
                .entry("是結".to_string())
                .and_modify(|e| *e = e.add(m.store.time, 1.0))
                .or_insert_with(|| UsageEvidence::default().add(m.store.time, 1.0));
            m.store.time += DAY_SECONDS;
        }
        let syls = super::super::reading_seq("ㄕˋ ㄐㄧㄝˊ");
        let lift = m.reading_lift("世傑", &syls);
        assert!((0.0..=m.params.habit_lift).contains(&lift));
    }
}
