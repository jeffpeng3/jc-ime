//! 忘記、移除、恢復與反悔（對照 `core/personal/unlearn.go`）。

use std::collections::{BTreeMap, BTreeSet};

use super::{cut_pair, pair, Model, Store};

/// 一次學習資料操作之前的複本。
///
/// 前身只保存被改到的項目；本版先保存操作前後的整份 Store，復原時再逐項比對。
/// 對外語意相同，代價是操作期間多一份記憶體。
#[derive(Debug, Clone)]
pub struct Undo {
    store: usize,
    store_id: u64,
    used: bool,
    before: Store,
    after: Store,
}

impl Undo {
    fn empty(&self) -> bool {
        self.before == self.after
    }
}

impl Model {
    /// 執行一次可復原的學習資料操作。
    pub fn undoable<F>(&mut self, f: F) -> Option<Undo>
    where
        F: FnOnce(&mut Model),
    {
        let before = self.store.clone();
        let store = &self.store as *const _ as usize;
        let store_id = self.store_id;
        f(self);
        if (&self.store as *const _ as usize) != store || self.store_id != store_id {
            return None;
        }
        let undo = Undo {
            store,
            store_id,
            used: false,
            before,
            after: self.store.clone(),
        };
        if undo.empty() {
            None
        } else {
            Some(undo)
        }
    }

    /// 把複本放回去。
    pub fn restore(&mut self, undo: &mut Undo) -> bool {
        if undo.used
            || undo.store_id != self.store_id
            || undo.store != (&self.store as *const _ as usize)
        {
            return false;
        }
        undo.used = true;
        let before = &undo.before;
        let after = &undo.after;
        merge_float_map(&mut self.store.bi, &before.bi, &after.bi);
        merge_float_map(&mut self.store.ctx, &before.ctx, &after.ctx);
        merge_float_map(&mut self.store.uni, &before.uni, &after.uni);
        merge_float_map(&mut self.store.uses, &before.uses, &after.uses);
        self.store.total = restore_float(self.store.total, before.total, after.total);
        merge_map(
            &mut self.store.corrections,
            &before.corrections,
            &after.corrections,
        );
        merge_map(
            &mut self.store.evidence_by_context,
            &before.evidence_by_context,
            &after.evidence_by_context,
        );
        merge_map(
            &mut self.store.evidence_by_word,
            &before.evidence_by_word,
            &after.evidence_by_word,
        );
        merge_map(&mut self.store.picked, &before.picked, &after.picked);
        merge_int_map(
            &mut self.store.phrase_seen,
            &before.phrase_seen,
            &after.phrase_seen,
        );
        merge_map(&mut self.store.phrases, &before.phrases, &after.phrases);
        merge_set(
            &mut self.store.tombstones,
            &before.tombstones,
            &after.tombstones,
        );
        merge_set(&mut self.store.readings, &before.readings, &after.readings);
        merge_map(
            &mut self.store.confirmed,
            &before.confirmed,
            &after.confirmed,
        );
        merge_map(
            &mut self.store.zhuyin_keys,
            &before.zhuyin_keys,
            &after.zhuyin_keys,
        );
        merge_u32_map(&mut self.store.rejects, &before.rejects, &after.rejects);
        merge_map(&mut self.store.pronouns, &before.pronouns, &after.pronouns);
        merge_map(&mut self.store.english, &before.english, &after.english);
        merge_map(
            &mut self.store.languages,
            &before.languages,
            &after.languages,
        );
        self.store.cross_language = before.cross_language.clone();
        if self.store.phrase_order == after.phrase_order {
            self.store.phrase_order = before.phrase_order.clone();
        } else {
            let mut order: Vec<String> = before
                .phrase_order
                .iter()
                .filter(|phrase| self.store.phrases.contains_key(*phrase))
                .cloned()
                .collect();
            for phrase in &self.store.phrase_order {
                if !order.contains(phrase) {
                    order.push(phrase.clone());
                }
            }
            self.store.phrase_order = order;
        }
        self.store
            .phrase_order
            .retain(|phrase| self.store.phrases.contains_key(phrase));
        self.diversity = super::diversity::DiversityIndex::default();
        self.gen += 1;
        true
    }

    /// 「不再學」：撤銷學習並留下 tombstone。
    pub fn forget(&mut self, word: &str) {
        self.forget_all(&[word.to_string()]);
    }

    /// 一次忘記多個詞。
    pub fn forget_all(&mut self, words: &[String]) {
        self.unlearn_all(words);
        for word in words {
            self.tombstone_english(word);
            self.store.tombstones.insert(word.clone());
        }
    }

    /// 「移除」：撤銷學習但不留 tombstone。
    pub fn unlearn(&mut self, word: &str) {
        self.unlearn_all(&[word.to_string()]);
    }

    /// 一次移除多個詞。
    pub fn unlearn_all(&mut self, words: &[String]) {
        if words.is_empty() {
            return;
        }
        self.gen += 1;
        let has = contains_any(words);
        let set: BTreeSet<String> = words.iter().cloned().collect();
        for word in words {
            self.forget_english(word);
            self.forget_pronouns(word);
        }
        self.forget_corrections(&has);

        let context_keys: Vec<String> = self
            .store
            .evidence_by_context
            .keys()
            .filter(|key| {
                let (left, word) = cut_pair(key);
                has(left) || has(word)
            })
            .cloned()
            .collect();
        for key in context_keys {
            let (_, word) = cut_pair(&key);
            self.store.evidence_by_context.remove(&key);
            self.store.evidence_by_word.remove(word);
        }
        let word_keys: Vec<String> = self
            .store
            .evidence_by_word
            .keys()
            .filter(|word| has(word))
            .cloned()
            .collect();
        for word in word_keys {
            self.store.evidence_by_word.remove(&word);
        }

        let bi_keys: Vec<String> = self
            .store
            .bi
            .keys()
            .filter(|key| {
                let (left, word) = cut_pair(key);
                has(left) || has(word)
            })
            .cloned()
            .collect();
        for key in bi_keys {
            let (left, word) = cut_pair(&key).to_owned_pair();
            self.drop_bi(&left, &word);
        }

        for word in words {
            let count = self.store.uni.remove(word).unwrap_or(0.0);
            self.store.total -= count;
            self.store.uses.remove(word);
        }
        for picks in self.store.picked.values_mut() {
            picks.retain(|word| !set.contains(word));
        }
        self.store.picked.retain(|_, picks| !picks.is_empty());

        self.store.readings.retain(|key| {
            let (word, _) = cut_pair(key);
            !has(word)
        });
        self.store.confirmed.retain(|key, _| {
            let (word, _) = cut_pair(key);
            !has(word)
        });
        self.store.zhuyin_keys.retain(|key, _| {
            let (left, symbol) = cut_pair(key);
            !has(&format!("{left}{symbol}"))
        });
        let phrases: Vec<String> = self
            .store
            .phrases
            .keys()
            .filter(|phrase| has(phrase))
            .cloned()
            .collect();
        for phrase in phrases {
            self.store.phrases.remove(&phrase);
            let count = self.store.uni.remove(&phrase).unwrap_or(0.0);
            self.store.total -= count;
        }
        self.store
            .phrase_order
            .retain(|phrase| self.store.phrases.contains_key(phrase));
        self.store.phrase_seen.retain(|phrase, _| !has(phrase));
    }

    /// 恢復學習：移除 tombstone。
    pub fn relearn(&mut self, word: &str) -> bool {
        self.store.tombstones.remove(word)
    }

    /// 一次恢復多個詞。
    pub fn relearn_all(&mut self, words: &[String]) -> bool {
        let mut found = false;
        for word in words {
            found |= self.relearn(word);
        }
        found
    }

    fn drop_bi(&mut self, left: &str, word: &str) {
        let key = pair(left, word);
        let Some(before) = self.store.bi.remove(&key) else {
            return;
        };
        if let Some(ctx) = self.store.ctx.get_mut(left) {
            *ctx -= before;
        }
        self.change_bi(left, before, 0.0);
    }

    /// 不再學習的詞。
    pub fn forgotten(&self) -> Vec<String> {
        self.store.tombstones.iter().cloned().collect()
    }

    /// 不再學習的詞數。
    pub fn forgotten_count(&self) -> usize {
        self.store.tombstones.len()
    }
}

fn restore_float(cur: f64, before: f64, after: f64) -> f64 {
    if cur == after {
        before
    } else {
        cur + before - after
    }
}

fn merge_float_map(
    cur: &mut BTreeMap<String, f64>,
    before: &BTreeMap<String, f64>,
    after: &BTreeMap<String, f64>,
) {
    let keys: BTreeSet<String> = before.keys().chain(after.keys()).cloned().collect();
    for key in keys {
        let b = before.get(&key).copied();
        let a = after.get(&key).copied();
        let c = cur.get(&key).copied();
        match (b, a, c) {
            (_, Some(a), Some(c)) if c == a => match b {
                Some(b) => {
                    cur.insert(key, b);
                }
                None => {
                    cur.remove(&key);
                }
            },
            (Some(b), None, None) => {
                cur.insert(key, b);
            }
            (Some(b), Some(a), Some(c)) if c != a => {
                cur.insert(key, c + b - a);
            }
            _ => {}
        }
    }
}

fn merge_int_map(
    cur: &mut BTreeMap<String, i64>,
    before: &BTreeMap<String, i64>,
    after: &BTreeMap<String, i64>,
) {
    let keys: BTreeSet<String> = before.keys().chain(after.keys()).cloned().collect();
    for key in keys {
        let b = before.get(&key).copied();
        let a = after.get(&key).copied();
        let c = cur.get(&key).copied();
        match (b, a, c) {
            (_, Some(a), Some(c)) if c == a => match b {
                Some(b) => {
                    cur.insert(key, b);
                }
                None => {
                    cur.remove(&key);
                }
            },
            (Some(b), None, None) => {
                cur.insert(key, b);
            }
            (Some(b), Some(a), Some(c)) if c != a => {
                cur.insert(key, b + c);
            }
            _ => {}
        }
    }
}

fn merge_u32_map(
    cur: &mut BTreeMap<String, u32>,
    before: &BTreeMap<String, u32>,
    after: &BTreeMap<String, u32>,
) {
    let keys: BTreeSet<String> = before.keys().chain(after.keys()).cloned().collect();
    for key in keys {
        let b = before.get(&key).copied();
        let a = after.get(&key).copied();
        let c = cur.get(&key).copied();
        match (b, a, c) {
            (_, Some(a), Some(c)) if c == a => match b {
                Some(b) => {
                    cur.insert(key, b);
                }
                None => {
                    cur.remove(&key);
                }
            },
            (Some(b), None, None) => {
                cur.insert(key, b);
            }
            (Some(b), Some(a), Some(c)) if c != a => {
                cur.insert(key, b + c);
            }
            _ => {}
        }
    }
}

fn merge_map<V: Clone + PartialEq>(
    cur: &mut BTreeMap<String, V>,
    before: &BTreeMap<String, V>,
    after: &BTreeMap<String, V>,
) {
    let keys: BTreeSet<String> = before.keys().chain(after.keys()).cloned().collect();
    for key in keys {
        let b = before.get(&key);
        let a = after.get(&key);
        let c = cur.get(&key);
        match (b, a, c) {
            (_, Some(a), Some(c)) if c == a => match b {
                Some(b) => {
                    cur.insert(key, b.clone());
                }
                None => {
                    cur.remove(&key);
                }
            },
            (Some(b), None, None) => {
                cur.insert(key, b.clone());
            }
            _ => {}
        }
    }
}

fn merge_set(cur: &mut BTreeSet<String>, before: &BTreeSet<String>, after: &BTreeSet<String>) {
    let keys: BTreeSet<String> = before.union(after).cloned().collect();
    for key in keys {
        let b = before.contains(&key);
        let a = after.contains(&key);
        let c = cur.contains(&key);
        if c == a {
            if b {
                cur.insert(key);
            } else {
                cur.remove(&key);
            }
        } else if !c && b {
            cur.insert(key);
        }
    }
}

/// 建一個「字串是否含任一詞」的判斷器。
pub fn contains_any(words: &[String]) -> impl Fn(&str) -> bool {
    let words = words.to_vec();
    move |text| words.iter().any(|word| text.contains(word))
}

trait PairOwned {
    fn to_owned_pair(&self) -> (String, String);
}

impl PairOwned for (&str, &str) {
    fn to_owned_pair(&self) -> (String, String) {
        (self.0.to_string(), self.1.to_string())
    }
}

#[cfg(test)]
mod tests {
    use crate::personal::{Model, NoLexicon, NoLm};

    #[test]
    fn forget_leaves_tombstone_and_unlearn_does_not() {
        let mut m = Model::new(Box::new(NoLexicon), Box::new(NoLm));
        m.store.uni.insert("井".to_string(), 1.0);
        m.forget("井");
        assert!(m.store.tombstones.contains("井"));
        assert!(!m.store.uni.contains_key("井"));
        m.store.uni.insert("景".to_string(), 1.0);
        m.unlearn("景");
        assert!(!m.store.tombstones.contains("景"));
        assert!(!m.store.uni.contains_key("景"));
    }

    #[test]
    fn undo_restores_forget() {
        let mut m = Model::new(Box::new(NoLexicon), Box::new(NoLm));
        m.store.uni.insert("井".to_string(), 1.0);
        let mut undo = m.undoable(|model| model.forget("井")).expect("有變更");
        assert!(!m.store.uni.contains_key("井"));
        assert!(m.restore(&mut undo));
        assert!(m.store.uni.contains_key("井"));
        assert!(!m.store.tombstones.contains("井"));
    }

    #[test]
    fn relearn_removes_tombstone() {
        let mut m = Model::new(Box::new(NoLexicon), Box::new(NoLm));
        m.forget("井");
        assert!(m.relearn("井"));
        assert!(!m.store.tombstones.contains("井"));
    }
}
