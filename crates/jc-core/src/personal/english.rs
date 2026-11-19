//! 英文詞的個人化證據（對照 `core/personal/english.go`）。

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::evidence::decay;
use super::{Model, Store, DAY_SECONDS};

/// 最多保留幾筆英文觀察。
pub const MAX_ENGLISH_OBSERVATIONS: usize = 1024;

/// 英文詞的最大位元組數。
pub const MAX_WORD_BYTES: usize = 64;

/// 個人化英文候選。
#[derive(Debug, Clone, PartialEq)]
pub struct EnglishCandidate {
    /// 候選字面。
    pub text: String,
    /// 編輯距離。
    pub distance: usize,
    /// 權重。
    pub weight: f64,
}

/// 一個英文詞的觀察。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EnglishEvidence {
    /// 最近學到的拼法。
    pub text: String,
    /// 確認次數。
    pub confirmed: f64,
    /// 獨立日數。
    pub days: f64,
    /// 使用次數。
    pub usage: f64,
    /// 更新時間。
    pub updated: i64,
    /// 當日。
    pub day: i64,
    /// 當日確認次數。
    pub day_confirmed: i32,
    /// 疑似拼錯。
    pub suspicious: bool,
}

/// 一次英文提交。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnglishCommit {
    /// 字面。
    pub text: String,
    /// 由系統補齊。
    pub assisted: bool,
    /// 明確選取。
    pub explicit: bool,
    /// 疑似拼錯。
    pub suspicious: bool,
}

impl EnglishEvidence {
    fn at(&self, now: i64) -> Self {
        if now <= self.updated {
            return self.clone();
        }
        let mut out = self.clone();
        out.confirmed = decay(out.confirmed, now - out.updated, 90 * DAY_SECONDS);
        out.days = decay(out.days, now - out.updated, 180 * DAY_SECONDS);
        out.usage = decay(out.usage, now - out.updated, 30 * DAY_SECONDS);
        out.updated = out.updated.max(now);
        let day = now / DAY_SECONDS;
        if day > out.day {
            out.day = day;
            out.day_confirmed = 0;
        }
        out
    }

    fn eligible(&self) -> bool {
        if self.suspicious {
            self.confirmed >= 7.5 && self.days >= 3.5
        } else {
            self.confirmed >= 2.5 && self.days >= 1.5
        }
    }
}

/// 是否是合法的英文詞。
pub fn english_token(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() < 3 || bytes.len() > MAX_WORD_BYTES {
        return false;
    }
    for (index, &byte) in bytes.iter().enumerate() {
        if byte.is_ascii_alphabetic() {
            continue;
        }
        if index > 0 && (byte.is_ascii_digit() || byte == b'_') {
            continue;
        }
        return false;
    }
    true
}

/// 英文索引快取。
#[derive(Debug, Clone, Default)]
pub struct EnglishIndex {
    /// 建索引時的版本。
    pub(crate) gen: u64,
    /// 建索引時的 store 位址。
    pub(crate) store: Option<usize>,
    /// 建索引時的詞典位址。
    pub(crate) base: usize,
    /// 索引詞。
    pub(crate) index: Vec<String>,
}

impl Store {
    /// 限制英文觀察筆數。
    pub(crate) fn bound_english(&mut self) {
        if self.english.len() <= MAX_ENGLISH_OBSERVATIONS {
            return;
        }
        let now = self.time;
        let mut entries: Vec<(String, f64)> = self
            .english
            .iter()
            .map(|(key, value)| {
                let value = value.at(now);
                (
                    key.clone(),
                    value.confirmed + value.days + 0.1 * value.usage,
                )
            })
            .collect();
        entries.sort_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        let drop = entries.len() - MAX_ENGLISH_OBSERVATIONS;
        for (key, _) in entries.into_iter().take(drop) {
            self.english.remove(&key);
        }
    }

    /// 清理不合法英文觀察。
    pub(crate) fn clean_english(&mut self) {
        let tombstones = self.tombstones.clone();
        self.english.retain(|key, value| {
            english_token(&value.text)
                && value.text.to_lowercase() == *key
                && (0..=2).contains(&value.day_confirmed)
                && (0.0..=32.0).contains(&value.confirmed)
                && (0.0..=32.0).contains(&value.days)
                && (0.0..=32.0).contains(&value.usage)
                && value.days <= 16.0
                && value.usage <= 16.0
                && value.confirmed.is_finite()
                && value.days.is_finite()
                && value.usage.is_finite()
                && !tombstones.contains(key)
        });
        self.bound_english();
    }
}

impl Model {
    /// 學習一次提交中的英文詞。
    pub fn learn_english(&mut self, events: &[EnglishCommit]) {
        let now = self.store.time;
        let mut seen = BTreeSet::new();
        for event in events {
            if !english_token(&event.text) {
                continue;
            }
            let key = event.text.to_lowercase();
            if !seen.insert(key.clone()) {
                continue;
            }
            if seen.len() > 64 {
                break;
            }
            if self.store.tombstones.contains(&key) {
                if !event.explicit || event.assisted {
                    continue;
                }
                self.store.tombstones.remove(&key);
            }
            let exists = self.store.english.contains_key(&key);
            if event.assisted && !exists {
                continue;
            }
            let mut evidence =
                self.store
                    .english
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| EnglishEvidence {
                        text: event.text.clone(),
                        updated: now,
                        day: now / DAY_SECONDS,
                        ..EnglishEvidence::default()
                    });
            evidence = evidence.at(now);
            let weight = if event.assisted { 0.25 } else { 1.0 };
            if !event.assisted {
                evidence.text = event.text.clone();
                evidence.suspicious |= event.suspicious;
                if evidence.day_confirmed < 2 {
                    if evidence.day_confirmed == 0 {
                        evidence.days = (evidence.days + 1.0).min(16.0);
                    }
                    evidence.confirmed = (evidence.confirmed + 1.0).min(32.0);
                    evidence.day_confirmed += 1;
                }
            }
            evidence.usage = (evidence.usage + weight).min(16.0);
            self.store.english.insert(key, evidence);
            self.gen += 1;
        }
        self.store.bound_english();
    }

    pub(super) fn forget_english(&mut self, text: &str) {
        if english_token(text) {
            self.store.english.remove(&text.to_lowercase());
        }
    }

    pub(super) fn tombstone_english(&mut self, text: &str) {
        if english_token(text) {
            self.store.tombstones.insert(text.to_lowercase());
        }
    }

    /// 是否已經學到這個英文詞。
    pub fn english_known(&self, text: &str) -> bool {
        if !english_token(text) {
            return false;
        }
        let key = text.to_lowercase();
        self.store.english.get(&key).is_some_and(|evidence| {
            !self.store.tombstones.contains(&key) && evidence.at(self.store.time).eligible()
        })
    }

    /// 是否學到這個拼法（含大小寫）。
    pub fn english_known_as(&self, text: &str) -> bool {
        self.english_known(text)
            && self
                .store
                .english
                .get(&text.to_lowercase())
                .is_some_and(|evidence| evidence.text == text)
    }

    /// 個人化的英文前綴候選。
    pub fn english_prefix(
        &self,
        base: &dyn super::EnglishLexicon,
        prefix: &str,
        limit: usize,
    ) -> Vec<EnglishCandidate> {
        if limit == 0 || prefix.is_empty() || prefix.len() > MAX_WORD_BYTES {
            return Vec::new();
        }
        let mut out = base.prefix(prefix, limit);
        let lower = prefix.to_lowercase();
        for (key, evidence) in &self.store.english {
            let evidence = evidence.at(self.store.time);
            if !evidence.eligible()
                || self.store.tombstones.contains(key)
                || !key.starts_with(&lower)
                || base.has(key) && !base.suggestible(key)
            {
                continue;
            }
            let suffix = evidence.text.get(prefix.len()..).unwrap_or("");
            let suffix = if prefix == prefix.to_uppercase() {
                suffix.to_uppercase()
            } else {
                suffix.to_string()
            };
            out.push(EnglishCandidate {
                text: format!("{prefix}{suffix}"),
                distance: 0,
                weight: 320.0 + evidence.usage,
            });
        }
        english_rank(out, limit)
    }

    /// 個人化的英文拼字更正。
    pub fn english_corrections(
        &mut self,
        base: &dyn super::EnglishLexicon,
        text: &str,
        limit: usize,
    ) -> Vec<EnglishCandidate> {
        if limit == 0 || self.english_known(text) {
            return Vec::new();
        }
        let mut out = base.corrections(text, limit);
        if base.has(text) {
            return out;
        }
        let store_id = &self.store as *const _ as usize;
        let base_id = std::ptr::from_ref(base).cast::<()>() as usize;
        if self.english_index.store != Some(store_id)
            || self.english_index.base != base_id
            || self.english_index.gen != self.gen
        {
            let index = self
                .store
                .english
                .iter()
                .filter(|(key, evidence)| {
                    evidence.at(self.store.time).eligible()
                        && !self.store.tombstones.contains(*key)
                        && !(base.has(key) && !base.suggestible(key))
                })
                .map(|(key, _)| key.clone())
                .collect();
            self.english_index = EnglishIndex {
                gen: self.gen,
                store: Some(store_id),
                base: base_id,
                index,
            };
        }
        for key in &self.english_index.index {
            let Some(evidence) = self.store.english.get(key) else {
                continue;
            };
            let evidence = evidence.at(self.store.time);
            let distance = edit_distance(&text.to_lowercase(), key);
            if distance <= 2 {
                out.push(EnglishCandidate {
                    text: evidence.text.clone(),
                    distance,
                    weight: 320.0 + evidence.usage,
                });
            }
        }
        english_rank(out, limit)
    }
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, &ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, &cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j + 1] + 1)
                .min(cur[j] + 1)
                .min(prev[j] + usize::from(ca != cb));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// 排序、去重並截斷英文候選。
pub fn english_rank(mut candidates: Vec<EnglishCandidate>, limit: usize) -> Vec<EnglishCandidate> {
    candidates.sort_by(|a, b| {
        a.distance
            .cmp(&b.distance)
            .then_with(|| {
                b.weight
                    .partial_cmp(&a.weight)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.text.cmp(&b.text))
    });
    let mut seen = BTreeSet::new();
    candidates.retain(|candidate| seen.insert(candidate.text.to_lowercase()));
    candidates.truncate(limit);
    candidates
}

/// 英文詞典的空白實作。
#[derive(Debug, Clone, Copy, Default)]
pub struct NoEnglish;

impl super::EnglishLexicon for NoEnglish {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::personal::{Model, NoLexicon, NoLm};

    fn model() -> Model {
        let mut model = Model::new(Box::new(NoLexicon), Box::new(NoLm));
        model.store.time = 10 * DAY_SECONDS;
        model
    }

    fn commit(text: &str) -> EnglishCommit {
        EnglishCommit {
            text: text.to_string(),
            assisted: false,
            explicit: true,
            suspicious: false,
        }
    }

    #[test]
    fn token_rules() {
        assert!(english_token("OpenCode"));
        assert!(english_token("a_b1"));
        assert!(!english_token("ab"));
        assert!(!english_token("http://x"));
        assert!(!english_token("1abc"));
    }

    #[test]
    fn needs_three_confirmations_over_two_days() {
        let mut m = model();
        for _ in 0..3 {
            m.learn_english(&[commit("OpenCode")]);
        }
        assert!(!m.english_known("OpenCode"));
        m.store.time += DAY_SECONDS;
        m.learn_english(&[commit("OpenCode")]);
        assert!(m.english_known("OpenCode"));
    }

    #[test]
    fn suspicious_needs_more_days() {
        let mut m = model();
        let mut event = commit("OpenCode");
        event.suspicious = true;
        for _ in 0..8 {
            m.learn_english(&[event.clone()]);
            m.store.time += DAY_SECONDS;
        }
        assert!(m.english_known("OpenCode"));
    }

    #[test]
    fn forget_blocks_learning() {
        let mut m = model();
        for _ in 0..4 {
            m.learn_english(&[commit("OpenCode")]);
            m.store.time += DAY_SECONDS;
        }
        assert!(m.english_known("OpenCode"));
        m.tombstone_english("OpenCode");
        assert!(!m.english_known("OpenCode"));
    }
}
