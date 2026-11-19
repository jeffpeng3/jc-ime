//! 有界的條件偏好更正（對照 `core/personal/corrections.go`）。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::zh::syllable::Syllable;

use super::evidence::decay;
use super::{cut_pair, pair, reading_key, Model, Seg, DAY_SECONDS};

/// 新更正對不相符文字最多施加的成本。
pub const CORRECTION_COST: f64 = 8.0;
/// 最多保留幾筆更正。
pub const MAX_CORRECTIONS: usize = 2048;
/// 更正最長音節數。
pub const MAX_CORRECTION_LENGTH: usize = 8;

/// 被手動換掉的原區段。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RejectedSeg {
    /// 原詞。
    pub word: String,
    /// 起點音節。
    pub start: usize,
    /// 終點音節（不含）。
    pub end: usize,
    /// 原本是自動修正。
    pub correction: bool,
    /// 自動修正詞條自己的讀音。
    pub reading: Vec<Syllable>,
}

/// 一筆更正。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Correction {
    /// 讀音鍵。
    pub reading: String,
    /// 左鄰音節；空字串是邊界。
    pub left: String,
    /// 右鄰音節。
    pub right: String,
    /// 左鄰詞。
    pub left_word: String,
    /// 右鄰詞。
    pub right_word: String,
    /// 選取的詞。
    pub word: String,
    /// 更新時間。
    pub updated: i64,
    /// 序號。
    pub serial: u64,
    /// 使用證據。
    pub usage: super::UsageEvidence,
}

/// 更正鍵。
pub fn correction_key(reading: &str, left: &str, right: &str) -> String {
    format!("{reading}\t{left}\t{right}")
}

/// 由原始選取區間建立更正。
pub fn correction_for(syls: &[Syllable], seg: &Seg) -> Option<Correction> {
    if !seg.manual
        || seg.completion
        || seg.partial
        || seg.end > syls.len()
        || seg.end <= seg.start
        || seg.end - seg.start > MAX_CORRECTION_LENGTH
        || seg.word.chars().count() != seg.end - seg.start
    {
        return None;
    }
    if !syls[seg.start..seg.end].iter().all(Syllable::legal) {
        return None;
    }
    let mut correction = Correction {
        reading: reading_key(&syls[seg.start..seg.end]),
        word: seg.word.clone(),
        ..Correction::default()
    };
    if seg.start > 0 && syls[seg.start - 1].legal() {
        correction.left = syls[seg.start - 1].as_str().to_string();
    }
    if seg.end < syls.len() && syls[seg.end].legal() {
        correction.right = syls[seg.end].as_str().to_string();
    }
    Some(correction)
}

/// 更正索引。
#[derive(Debug, Clone, Default)]
pub struct CorrectionIndex {
    pub(crate) store: Option<usize>,
    pub(crate) gen: u64,
    pub(crate) serial: u64,
    pub(crate) count: usize,
    pub(crate) by_prefix: BTreeMap<String, Vec<(Correction, Vec<Syllable>)>>,
    pub(crate) by_base: BTreeMap<String, Vec<(Correction, Vec<Syllable>)>>,
}

/// 沒打聲調的音節只比基底；有打的要完全相同。
fn compatible(typed: &Syllable, stored: &Syllable) -> bool {
    if typed.untoned() {
        typed.base() == stored.base()
    } else {
        typed == stored
    }
}

/// 讀音相容比對。
fn reading_matches(typed: &[Syllable], stored: &[Syllable]) -> bool {
    typed.len() == stored.len() && typed.iter().zip(stored).all(|(a, b)| compatible(a, b))
}

/// 左右鄰比對。
fn neighbor_matches(typed: Option<&Syllable>, stored: &str) -> bool {
    match typed {
        Some(syllable) if syllable.legal() || syllable.untoned() => {
            !stored.is_empty() && compatible(syllable, &Syllable::from_raw(stored.to_string()))
        }
        _ => stored.is_empty(),
    }
}

impl Model {
    fn index_corrections(&mut self) {
        let addr = &self.store as *const _ as usize;
        if self.correction_index.store == Some(addr)
            && self.correction_index.gen == self.gen
            && self.correction_index.serial == self.store.correction_serial
            && self.correction_index.count == self.store.corrections.len()
        {
            return;
        }
        let mut index = CorrectionIndex {
            store: Some(addr),
            gen: self.gen,
            serial: self.store.correction_serial,
            count: self.store.corrections.len(),
            by_prefix: BTreeMap::new(),
            by_base: BTreeMap::new(),
        };
        for (key, correction) in &self.store.corrections {
            let _ = key;
            let reading = super::reading_seq(&correction.reading);
            if reading.is_empty() {
                continue;
            }
            let prefix = format!("{}\t{}", correction.left, reading[0].as_str());
            index
                .by_prefix
                .entry(prefix)
                .or_default()
                .push((correction.clone(), reading.clone()));
            index
                .by_base
                .entry(reading[0].base().as_str().to_string())
                .or_default()
                .push((correction.clone(), reading));
        }
        self.correction_index = index;
    }

    fn resolve_corrections(
        &mut self,
        syls: &[Syllable],
        start: usize,
        end: usize,
        word: &str,
        force: bool,
    ) -> bool {
        if syls.is_empty() {
            return false;
        }
        let chars: Vec<char> = word.chars().collect();
        let unit = self.params.w_manual - self.params.w_passive;
        let mut contested = false;
        let from = start.saturating_sub(MAX_CORRECTION_LENGTH - 1);
        let mut remove = Vec::new();
        let mut updates = Vec::new();
        let now = self.store.time;
        for i in from..end.min(syls.len()) {
            if !syls[i].legal() {
                continue;
            }
            let left = if i > 0 && syls[i - 1].legal() {
                syls[i - 1].as_str().to_string()
            } else {
                String::new()
            };
            for j in (i + 1)..=syls.len().min(i + MAX_CORRECTION_LENGTH) {
                if !syls[j - 1].legal() {
                    break;
                }
                if j <= start {
                    continue;
                }
                let right = if j < syls.len() && syls[j].legal() {
                    syls[j].as_str().to_string()
                } else {
                    String::new()
                };
                let key = correction_key(&reading_key(&syls[i..j]), &left, &right);
                let Some(old) = self.store.corrections.get(&key).cloned() else {
                    continue;
                };
                if self.store.tombstones.contains(&old.word) {
                    continue;
                }
                let old_chars: Vec<char> = old.word.chars().collect();
                if old_chars.len() != j - i {
                    continue;
                }
                let mut agree = true;
                for p in i.max(start)..j.min(end) {
                    if old_chars[p - i] != chars[p - start] {
                        agree = false;
                        break;
                    }
                }
                if agree {
                    continue;
                }
                let mut usage = old.usage.at(now);
                let elapsed = now - old.updated;
                if force {
                    remove.push(key);
                    contested = true;
                } else if decay(1.0, elapsed, 30 * DAY_SECONDS) < 0.5 {
                    remove.push(key);
                } else if unit > 0.0 && usage.short.max(usage.long) >= 1.5 * unit {
                    usage.short = (usage.short - unit).max(0.0);
                    usage.long = (usage.long - unit).max(0.0);
                    let mut updated = old.clone();
                    updated.usage = usage;
                    updates.push((key, updated));
                    contested = true;
                } else {
                    remove.push(key);
                    contested = true;
                }
            }
        }
        for key in remove {
            self.store.corrections.remove(&key);
        }
        for (key, value) in updates {
            self.store.corrections.insert(key, value);
        }
        contested
    }

    /// 處理與這次手選矛盾的更正；有近期紀錄被反駁時回傳 true。
    pub fn contest_corrections(
        &mut self,
        syls: &[Syllable],
        start: usize,
        end: usize,
        word: &str,
    ) -> bool {
        self.resolve_corrections(syls, start, end, word, false)
    }

    /// 用自訂詞直接取代不一致的舊更正。
    pub fn supersede_corrections(
        &mut self,
        syls: &[Syllable],
        start: usize,
        end: usize,
        word: &str,
    ) -> bool {
        self.resolve_corrections(syls, start, end, word, true)
    }

    /// 學習提交中的更正。
    pub fn learn_corrections(&mut self, commit: &super::Commit) {
        let mut items: Vec<Seg> = commit.path.clone();
        items.extend(commit.selected.iter().cloned());
        for seg in items {
            if seg.fuzzy {
                continue;
            }
            let Some(mut correction) = correction_for(&commit.syls, &seg) else {
                self.keep_confirmed(&commit.syls, &seg);
                continue;
            };
            let blocked = commit.rejected.iter().any(|rejected| {
                rejected.correction && rejected.start < seg.end && seg.start < rejected.end
            });
            if blocked {
                continue;
            }
            for neighbor in &commit.path {
                if neighbor.end == seg.start
                    || neighbor.start < seg.start && seg.start < neighbor.end
                {
                    correction.left_word = neighbor.word.clone();
                }
                if neighbor.start == seg.end || neighbor.start < seg.end && seg.end < neighbor.end {
                    correction.right_word = neighbor.word.clone();
                }
            }
            if seg.end - seg.start >= super::preference::HABIT_MIN_SYLLABLES {
                self.note_confirmed(&correction.word, &correction.reading);
            }
            if self.contest_corrections(&commit.syls, seg.start, seg.end, &correction.word) {
                continue;
            }
            let key = correction_key(&correction.reading, &correction.left, &correction.right);
            if let Some(old) = self.store.corrections.get(&key) {
                if old.word == correction.word {
                    correction.usage = old.usage;
                }
            }
            self.store.correction_serial += 1;
            correction.serial = self.store.correction_serial;
            correction.updated = self.store.time;
            correction.usage = correction.usage.add(
                self.store.time,
                (self.params.w_manual - self.params.w_passive).max(0.0),
            );
            self.store.corrections.insert(key, correction);
        }
        if self.store.corrections.len() > MAX_CORRECTIONS {
            let mut keys: Vec<String> = self.store.corrections.keys().cloned().collect();
            keys.sort_by_key(|key| {
                (
                    self.store
                        .corrections
                        .get(key)
                        .map_or(0, |item| item.serial),
                    key.clone(),
                )
            });
            let drop = keys.len() - MAX_CORRECTIONS;
            for key in keys.into_iter().take(drop) {
                self.store.corrections.remove(&key);
            }
        }
    }

    /// 依已輸入音節套用更正成本。
    pub fn correction_penalty<'a>(
        &'a mut self,
        syls: &[Syllable],
        overrides: &[Seg],
    ) -> Option<impl Fn(usize, usize, &str) -> f64 + 'a> {
        if self.store.corrections.is_empty() && overrides.is_empty() {
            return None;
        }
        let mut local = BTreeMap::new();
        for seg in overrides {
            if let Some(mut correction) = correction_for(syls, seg) {
                correction.serial = u64::MAX;
                correction.updated = self.store.time;
                local.insert(
                    correction_key(&correction.reading, &correction.left, &correction.right),
                    correction,
                );
            }
        }
        #[derive(Clone, Default)]
        struct Choice {
            ch: char,
            cost: f64,
            serial: u64,
            key: String,
        }
        let mut chosen = vec![Choice::default(); syls.len()];
        let mut have = false;
        let now = self.store.time;
        let mut apply = |i: usize, key: &str, correction: &Correction| {
            if self.store.tombstones.contains(&correction.word) {
                return;
            }
            let chars: Vec<char> = correction.word.chars().collect();
            if chars.is_empty() {
                return;
            }
            let cost = CORRECTION_COST * decay(1.0, now - correction.updated, 30 * DAY_SECONDS)
                / chars.len() as f64;
            for (at, &ch) in chars.iter().enumerate() {
                let Some(slot) = chosen.get_mut(i + at) else {
                    continue;
                };
                if slot.cost == 0.0
                    || correction.serial > slot.serial
                    || correction.serial == slot.serial && key > slot.key.as_str()
                {
                    *slot = Choice {
                        ch,
                        cost,
                        serial: correction.serial,
                        key: key.to_string(),
                    };
                    have = true;
                }
            }
        };
        let records: Vec<(String, Correction)> = self
            .store
            .corrections
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        for (key, correction) in &records {
            let reading = super::reading_seq(&correction.reading);
            for i in 0..syls.len() {
                if i + reading.len() > syls.len() || reading.is_empty() {
                    continue;
                }
                let left = i.checked_sub(1).and_then(|at| syls.get(at));
                let right = syls.get(i + reading.len());
                if reading_matches(&syls[i..i + reading.len()], &reading)
                    && neighbor_matches(left, &correction.left)
                    && neighbor_matches(right, &correction.right)
                {
                    apply(i, key, correction);
                }
            }
        }
        for (key, correction) in &local {
            let reading = super::reading_seq(&correction.reading);
            for i in 0..syls.len() {
                if i + reading.len() > syls.len() || reading.is_empty() {
                    continue;
                }
                let left = i.checked_sub(1).and_then(|at| syls.get(at));
                let right = syls.get(i + reading.len());
                if reading_matches(&syls[i..i + reading.len()], &reading)
                    && neighbor_matches(left, &correction.left)
                    && neighbor_matches(right, &correction.right)
                {
                    apply(i, key, correction);
                }
            }
        }
        if !have {
            return None;
        }
        Some(move |start: usize, end: usize, word: &str| {
            if end > chosen.len() || end <= start {
                return 0.0;
            }
            let chars: Vec<char> = word.chars().collect();
            if chars.len() < end - start {
                return 0.0;
            }
            let mut total = 0.0;
            for i in start..end {
                if chosen[i].cost > 0.0 && chars[i - start] != chosen[i].ch {
                    total += chosen[i].cost;
                }
            }
            total
        })
    }

    /// 補全使用更正證據。
    pub fn correction_usage(
        &mut self,
        syls: &[Syllable],
    ) -> Option<impl Fn(usize, &str) -> f64 + '_> {
        if self.store.corrections.is_empty() {
            return None;
        }
        self.index_corrections();
        let mut by_start: BTreeMap<usize, BTreeMap<String, f64>> = BTreeMap::new();
        let records: Vec<(Correction, Vec<Syllable>)> = self
            .store
            .corrections
            .values()
            .map(|correction| (correction.clone(), super::reading_seq(&correction.reading)))
            .collect();
        for (correction, reading) in records {
            if self.store.tombstones.contains(&correction.word) {
                continue;
            }
            for i in 0..syls.len() {
                if i + reading.len() > syls.len() || reading.is_empty() {
                    continue;
                }
                if !reading_matches(&syls[i..i + reading.len()], &reading) {
                    continue;
                }
                let left = i.checked_sub(1).and_then(|at| syls.get(at));
                let right = syls.get(i + reading.len());
                if !neighbor_matches(left, &correction.left)
                    || !neighbor_matches(right, &correction.right)
                {
                    continue;
                }
                let usage = correction.usage.at(self.store.time);
                let entry = by_start.entry(i).or_default();
                let value = entry.entry(correction.word.clone()).or_insert(0.0);
                *value = value.max(usage.short + usage.long);
            }
        }
        Some(move |start: usize, word: &str| {
            by_start
                .get(&start)
                .and_then(|map| map.get(word).copied())
                .unwrap_or(0.0)
        })
    }

    pub(super) fn forget_corrections(&mut self, has: &dyn Fn(&str) -> bool) {
        self.store.corrections.retain(|_, correction| {
            !(has(&correction.word) || has(&correction.left_word) || has(&correction.right_word))
        });
    }

    /// 更正涉及的詞。
    pub fn correction_words(&self) -> BTreeSet<String> {
        self.store
            .corrections
            .values()
            .map(|correction| correction.word.clone())
            .collect()
    }

    /// 把「左詞\t詞」切成兩半；供直接查詢。
    pub fn split_pair(key: &str) -> (&str, &str) {
        cut_pair(key)
    }

    /// 以更正鍵查詢。
    pub fn correction(&self, reading: &str, left: &str, right: &str) -> Option<&Correction> {
        self.store
            .corrections
            .get(&correction_key(reading, left, right))
    }

    /// 更正鍵輔助。
    pub fn correction_map_key(word: &str, reading: &str) -> String {
        pair(word, reading)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::personal::{Commit, Model, NoLexicon, NoLm};

    fn model() -> Model {
        Model::new(Box::new(NoLexicon), Box::new(NoLm))
    }

    fn syls(reading: &str) -> Vec<Syllable> {
        super::super::reading_seq(reading)
    }

    #[test]
    fn manual_selection_creates_correction() {
        let mut m = model();
        m.store.time = 100;
        let syls = syls("ㄕˋ ㄐㄧㄝˊ");
        let seg = Seg {
            word: "世傑".to_string(),
            start: 0,
            end: 2,
            manual: true,
            ..Seg::default()
        };
        m.learn_corrections(&Commit {
            syls: syls.clone(),
            path: vec![seg.clone()],
            ..Commit::default()
        });
        assert_eq!(m.store.corrections.len(), 1);
        let correction = m.store.corrections.values().next().unwrap();
        assert_eq!(correction.word, "世傑");
        assert_eq!(correction.reading, "ㄕˋ ㄐㄧㄝˊ");
    }

    #[test]
    fn correction_penalty_charges_different_word() {
        let mut m = model();
        m.store.time = 100;
        let syls = syls("ㄕˋ ㄐㄧㄝˊ");
        let seg = Seg {
            word: "世傑".to_string(),
            start: 0,
            end: 2,
            manual: true,
            ..Seg::default()
        };
        m.learn_corrections(&Commit {
            syls: syls.clone(),
            path: vec![seg],
            ..Commit::default()
        });
        let penalty = m.correction_penalty(&syls, &[]).expect("有更正");
        assert!(penalty(0, 2, "是結") > 0.0);
        assert_eq!(penalty(0, 2, "世傑"), 0.0);
    }
}
