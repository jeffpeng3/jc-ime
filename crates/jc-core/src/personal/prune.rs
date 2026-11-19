//! 個人資料清理（對照 `core/personal/prune.go`）。

use std::collections::{BTreeMap, BTreeSet};

use super::evidence::decay;
use super::{cut_pair, pair, Model, UsageEvidence, DAY_SECONDS};

/// 清理門檻。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrunePolicy {
    /// 排序影響比例。
    pub share: f64,
    /// 淘汰時的安全邊界。
    pub margin: f64,
    /// 證據門檻。
    pub evidence: f64,
    /// 詞對上限。
    pub max_pairs: usize,
    /// 證據上限。
    pub max_evidence: usize,
    /// 每詞資料上限。
    pub max_word: usize,
}

impl Default for PrunePolicy {
    fn default() -> Self {
        Self {
            share: 0.003,
            margin: 1.05,
            evidence: 0.05,
            max_pairs: 200_000,
            max_evidence: 60_000,
            max_word: 30_000,
        }
    }
}

/// 一次清理刪掉的筆數。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PruneStats {
    /// 詞對。
    pub bi: usize,
    /// 前文。
    pub ctx: usize,
    /// unigram。
    pub uni: usize,
    /// 使用次數。
    pub uses: usize,
    /// 上下文證據。
    pub evidence_by_context: usize,
    /// 詞證據。
    pub evidence_by_word: usize,
    /// 手選紀錄。
    pub confirmed: usize,
    /// 注音文。
    pub zhuyin_keys: usize,
    /// 暫定片語。
    pub phrase_seen: usize,
}

impl PruneStats {
    /// 合計。
    pub fn total(&self) -> usize {
        self.bi
            + self.ctx
            + self.uni
            + self.uses
            + self.evidence_by_context
            + self.evidence_by_word
            + self.confirmed
            + self.zhuyin_keys
            + self.phrase_seen
    }
}

impl Model {
    /// 以預設門檻清理。
    pub fn prune(&mut self) -> PruneStats {
        self.prune_with(PrunePolicy::default())
    }

    /// 每個 UTC 日至多清理一次。
    pub fn prune_if_due(&mut self) -> (PruneStats, bool) {
        let day = self.store.time / DAY_SECONDS + 1;
        if self.pruned_day == day {
            return (PruneStats::default(), false);
        }
        self.pruned_day = day;
        (self.prune(), true)
    }

    fn protected_words(&self) -> (BTreeSet<String>, BTreeSet<String>) {
        let mut words = BTreeSet::new();
        let mut phrases = BTreeSet::new();
        for phrase in self.store.phrases.keys() {
            words.insert(phrase.clone());
            phrases.insert(phrase.clone());
        }
        for key in &self.store.readings {
            let (word, _) = cut_pair(key);
            words.insert(word.to_string());
        }
        for picks in self.store.picked.values() {
            words.extend(picks.iter().cloned());
        }
        for correction in self.store.corrections.values() {
            words.insert(correction.word.clone());
        }
        (words, phrases)
    }

    /// 依指定門檻清理。
    pub fn prune_with(&mut self, policy: PrunePolicy) -> PruneStats {
        let mut stats = PruneStats::default();
        let (words, phrases) = self.protected_words();
        let mut store = std::mem::take(&mut self.store);
        let kappas: BTreeMap<String, f64> = store
            .ctx
            .keys()
            .map(|left| (left.clone(), self.kappa(left)))
            .collect();
        let now = store.time;

        // 排序計數。
        let mut drop_bi = BTreeSet::new();
        let mut keep_bi = Vec::new();
        for (key, &count) in &store.bi {
            let (left, word) = cut_pair(key);
            if phrases.contains(word) {
                continue;
            }
            let kappa = kappas.get(left).copied().unwrap_or(self.params.kappa_bi);
            let prior = kappa
                * (self.params.mu * self.lm.bi(left, word)
                    + (1.0 - self.params.mu)
                        * ((store.uni.get(word).copied().unwrap_or(0.0)
                            + self.params.kappa_uni * self.lm.uni(word))
                            / (store.total + self.params.kappa_uni)));
            if count <= policy.share * prior {
                drop_bi.insert(key.clone());
            } else {
                keep_bi.push((key.clone(), count));
            }
        }
        keep_bi.sort_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        if policy.max_pairs > 0 && store.bi.len().saturating_sub(drop_bi.len()) > policy.max_pairs {
            let over = store.bi.len() - drop_bi.len() - policy.max_pairs;
            for (key, _) in keep_bi.iter().take(over) {
                drop_bi.insert(key.clone());
            }
        }
        for key in &drop_bi {
            let (left, _) = cut_pair(key);
            let count = store.bi.remove(key).unwrap_or(0.0);
            *store.ctx.entry(left.to_string()).or_default() -= count;
            stats.bi += 1;
        }
        let lefts: BTreeSet<String> = store
            .bi
            .keys()
            .map(|key| cut_pair(key).0.to_string())
            .collect();
        let empty: Vec<String> = store
            .ctx
            .keys()
            .filter(|left| !lefts.contains(*left))
            .cloned()
            .collect();
        for left in empty {
            store.ctx.remove(&left);
            stats.ctx += 1;
        }

        // unigram。
        let mut drop_uni = BTreeSet::new();
        let mut keep_uni = Vec::new();
        for (word, &count) in &store.uni {
            if words.contains(word) {
                continue;
            }
            if count <= policy.share * self.params.kappa_uni * self.lm.uni(word) {
                drop_uni.insert(word.clone());
            } else {
                keep_uni.push((word.clone(), count));
            }
        }
        keep_uni.sort_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        if policy.max_word > 0 && store.uni.len().saturating_sub(drop_uni.len()) > policy.max_word {
            let over = store.uni.len() - drop_uni.len() - policy.max_word;
            for (word, _) in keep_uni.iter().take(over) {
                drop_uni.insert(word.clone());
            }
        }
        for word in &drop_uni {
            let count = store.uni.remove(word).unwrap_or(0.0);
            store.total -= count;
            stats.uni += 1;
        }
        if store.total < 0.0 || store.uni.is_empty() {
            store.total = 0.0;
        }
        let used: Vec<String> = store
            .uses
            .keys()
            .filter(|word| !store.uni.contains_key(*word) && !words.contains(*word))
            .cloned()
            .collect();
        for word in used {
            store.uses.remove(&word);
            stats.uses += 1;
        }

        // 補全證據。
        stats.evidence_by_context = prune_evidence(
            &mut store.evidence_by_context,
            now,
            policy.evidence,
            policy.max_evidence,
            |key| phrases.contains(cut_pair(key).1),
        );
        stats.evidence_by_word = prune_evidence(
            &mut store.evidence_by_word,
            now,
            policy.evidence,
            policy.max_word,
            |word| words.contains(word),
        );
        stats.confirmed = prune_evidence(
            &mut store.confirmed,
            now,
            policy.evidence,
            policy.max_word,
            |_| false,
        );
        stats.zhuyin_keys = prune_evidence(
            &mut store.zhuyin_keys,
            now,
            super::zhuyin_keys::ZHUYIN_KEY_FLOOR,
            policy.max_word,
            |_| false,
        );

        // 暫定片語。
        let mut keep_seen = Vec::new();
        let seen_keys: Vec<String> = store.phrase_seen.keys().cloned().collect();
        for phrase in seen_keys {
            if phrases.contains(&phrase) {
                continue;
            }
            let Some(evidence) = store.evidence_by_word.get(&phrase) else {
                store.phrase_seen.remove(&phrase);
                stats.phrase_seen += 1;
                continue;
            };
            keep_seen.push((phrase, evidence.at(now).short + evidence.at(now).long));
        }
        keep_seen.sort_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        if policy.max_word > 0 && keep_seen.len() > policy.max_word {
            let over = keep_seen.len() - policy.max_word;
            for (phrase, _) in keep_seen.into_iter().take(over) {
                store.phrase_seen.remove(&phrase);
                stats.phrase_seen += 1;
            }
        }

        self.store = store;
        if stats.total() > 0 {
            self.gen += 1;
        }
        stats
    }
}

fn prune_evidence(
    evidence: &mut BTreeMap<String, UsageEvidence>,
    now: i64,
    floor: f64,
    limit: usize,
    protected: impl Fn(&str) -> bool,
) -> usize {
    let mut removed = 0;
    let mut keep = Vec::new();
    let keys: Vec<String> = evidence.keys().cloned().collect();
    for key in keys {
        if protected(&key) {
            continue;
        }
        let entry = evidence.get(&key).copied().unwrap_or_default().at(now);
        let strength = entry.short + entry.long;
        if strength < floor {
            evidence.remove(&key);
            removed += 1;
        } else {
            keep.push((key, strength));
        }
    }
    keep.sort_by(|a, b| {
        a.1.partial_cmp(&b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    if limit > 0 && evidence.len() > limit {
        let over = evidence.len() - limit;
        for (key, _) in keep.into_iter().take(over) {
            evidence.remove(&key);
            removed += 1;
        }
    }
    removed
}

/// 保留給 prune 的介面：時間差衰減。
pub(crate) fn decay_since(now: i64, updated: i64) -> f64 {
    decay(1.0, now - updated, 30 * DAY_SECONDS)
}

/// 保留給 prune 的介面：排序鍵。
pub(crate) fn pair_key(a: &str, b: &str) -> String {
    pair(a, b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::personal::{Model, NoLexicon, NoLm};

    #[test]
    fn prune_keeps_explicit_choices() {
        let mut m = Model::new(Box::new(NoLexicon), Box::new(NoLm));
        m.store.time = 1000;
        m.store
            .picked
            .insert("ㄐㄧㄥˇ".to_string(), vec!["井".to_string()]);
        m.store.uni.insert("井".to_string(), 0.01);
        m.store
            .evidence_by_context
            .insert("k".to_string(), UsageEvidence::default().add(0, 0.001));
        let stats = m.prune();
        assert!(stats.total() > 0);
        assert!(m.store.uni.contains_key("井"));
    }

    #[test]
    fn prune_caps_evidence() {
        let mut m = Model::new(Box::new(NoLexicon), Box::new(NoLm));
        for i in 0..10 {
            m.store
                .evidence_by_context
                .insert(format!("k{i}"), UsageEvidence::default().add(0, 1.0));
        }
        let stats = m.prune_with(PrunePolicy {
            max_evidence: 3,
            evidence: 0.0,
            ..PrunePolicy::default()
        });
        assert_eq!(stats.evidence_by_context, 7);
        assert_eq!(m.store.evidence_by_context.len(), 3);
    }
}
