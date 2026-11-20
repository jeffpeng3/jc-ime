//! 英文詞的個人化證據（對照 `core/personal/english.go`）。

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::evidence::decay;
use super::{Model, Store, DAY_SECONDS};

/// 最多保留幾筆英文觀察。
pub const MAX_ENGLISH_OBSERVATIONS: usize = 1024;

/// 英文詞的最大位元組數。
pub const MAX_WORD_BYTES: usize = 64;

/// 最多保留幾筆「打錯 → 修正」的學習紀錄。
pub const MAX_ENGLISH_PAIRS: usize = 1024;

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

/// 一次「打錯 → 修正」的學習紀錄。
///
/// 鍵是打錯的拼法（小寫），值是使用者接受過的修正。這份紀錄只用來把學過的
/// 修正排到前面，不會取代內建詞表；內建詞表仍然先查。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EnglishCorrection {
    /// 修正後的拼法，保留接受時的大小寫。
    pub text: String,
    /// 接受次數。
    pub accepted: f64,
    /// 使用次數；接受修正時一起累積，用來和其他學過的拼法排序。
    pub usage: f64,
    /// 更新時間。
    pub updated: i64,
}

impl EnglishCorrection {
    /// 排序權重；次數與使用量都算，數值只在本模組內比較。
    fn weight(&self) -> f64 {
        320.0 + 10.0 * self.accepted + self.usage
    }
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
        self.english_pairs.retain(|wrong, value| {
            english_token(wrong)
                && english_token(&value.text)
                && wrong.to_lowercase() == *wrong
                && value.text.to_lowercase() != *wrong
                && (0.0..=32.0).contains(&value.accepted)
                && (0.0..=32.0).contains(&value.usage)
                && value.accepted.is_finite()
                && value.usage.is_finite()
                && !tombstones.contains(&value.text.to_lowercase())
        });
        self.bound_english();
        self.bound_english_pairs();
    }

    /// 限制英文修正紀錄的筆數。
    fn bound_english_pairs(&mut self) {
        if self.english_pairs.len() <= MAX_ENGLISH_PAIRS {
            return;
        }
        let mut entries: Vec<(String, f64)> = self
            .english_pairs
            .iter()
            .map(|(key, value)| (key.clone(), value.weight()))
            .collect();
        entries.sort_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        let drop = entries.len() - MAX_ENGLISH_PAIRS;
        for (key, _) in entries.into_iter().take(drop) {
            self.english_pairs.remove(&key);
        }
    }

    /// 記一次接受的英文拼字修正。
    ///
    /// `wrong` 是打錯的拼法，`right` 是使用者選定的修正。紀錄不受最短觸發
    /// 長度限制，短字也會留下證據；查詢時才依 [`super::MIN_TRIGGER_LEN`] 決定
    /// 要不要改變排序。大小寫保留在 `right`，鍵一律用小寫。
    ///
    /// 回傳是否真的寫入。
    pub fn note_english_correction(&mut self, wrong: &str, right: &str) -> bool {
        if !english_token(wrong) || !english_token(right) {
            return false;
        }
        let key = wrong.to_lowercase();
        if key == right.to_lowercase() || self.tombstones.contains(&right.to_lowercase()) {
            return false;
        }
        let now = self.time;
        let entry = self
            .english_pairs
            .entry(key)
            .or_insert_with(|| EnglishCorrection {
                text: right.to_string(),
                updated: now,
                ..EnglishCorrection::default()
            });
        entry.text = right.to_string();
        entry.accepted = (entry.accepted + 1.0).min(32.0);
        entry.usage = (entry.usage + 1.0).min(16.0);
        entry.updated = entry.updated.max(now);
        self.bound_english_pairs();
        true
    }

    /// 學過的英文修正；被跳過太多次或字太短時回傳 `None`。
    ///
    /// 這是 session 決定「學習結果要不要排第一」的入口：回傳 `Some` 時，
    /// 同一個打錯的拼法直接以學到的修正為第一候選。連續跳過
    /// [`super::SKIP_LIMIT`] 次之後回傳 `None`，修正仍留在候選清單裡。
    pub fn english_correction(&self, wrong: &str) -> Option<&str> {
        if !english_token(wrong) || wrong.chars().count() < super::MIN_TRIGGER_LEN {
            return None;
        }
        let key = wrong.to_lowercase();
        if self.skip_count(&key) >= super::SKIP_LIMIT {
            return None;
        }
        let entry = self.english_pairs.get(&key)?;
        (!entry.text.is_empty()).then_some(entry.text.as_str())
    }

    /// 學過的英文修正，忽略跳過計數與最短觸發長度。
    ///
    /// session 用這個入口判斷「這次是不是又把學過的修正跳過」，才不會因為
    /// 已經降權就停止累計。
    pub(crate) fn english_correction_target(&self, wrong: &str) -> Option<&str> {
        if !english_token(wrong) || wrong.chars().count() < super::MIN_TRIGGER_LEN {
            return None;
        }
        self.english_pairs
            .get(&wrong.to_lowercase())
            .map(|entry| entry.text.as_str())
    }

    /// 個人化的英文候選，依學習權重排序。
    ///
    /// 打錯的拼法或修正後的拼法以 `prefix` 開頭就算命中。短於
    /// [`super::MIN_TRIGGER_LEN`] 的紀錄不列入，因此只記錄、不改排序。被跳過
    /// 的候選仍在清單裡，只是權重降低。
    pub fn english_candidates(&self, prefix: &str) -> Vec<EnglishCandidate> {
        if prefix.is_empty() {
            return Vec::new();
        }
        let lower = prefix.to_lowercase();
        let mut out: Vec<EnglishCandidate> = self
            .english_pairs
            .iter()
            .filter(|(wrong, entry)| {
                wrong.chars().count() >= super::MIN_TRIGGER_LEN
                    && (wrong.starts_with(&lower) || entry.text.to_lowercase().starts_with(&lower))
            })
            .map(|(wrong, entry)| {
                let skips =
                    i32::try_from(self.skip_count(wrong).min(super::SKIP_LIMIT)).unwrap_or(0);
                EnglishCandidate {
                    text: entry.text.clone(),
                    distance: 0,
                    weight: entry.weight() - f64::from(skips * super::SKIP_PENALTY),
                }
            })
            .collect();
        out.sort_by(|a, b| {
            b.weight
                .partial_cmp(&a.weight)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.text.cmp(&b.text))
        });
        let mut seen = BTreeSet::new();
        out.retain(|candidate| seen.insert(candidate.text.to_lowercase()));
        out
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
            let key = text.to_lowercase();
            self.store.english.remove(&key);
            self.store
                .english_pairs
                .retain(|wrong, entry| *wrong != key && entry.text.to_lowercase() != key);
        }
    }

    pub(super) fn tombstone_english(&mut self, text: &str) {
        if english_token(text) {
            let key = text.to_lowercase();
            self.store.tombstones.insert(key.clone());
            self.store
                .english_pairs
                .retain(|_, entry| entry.text.to_lowercase() != key);
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

    #[test]
    fn english_correction_ranks_first_and_skip_demotes() {
        let mut store = Store::new();
        store.time = 10 * DAY_SECONDS;
        assert!(store.note_english_correction("recieve", "receive"));
        assert_eq!(store.english_correction("recieve"), Some("receive"));
        assert_eq!(store.english_candidates("reci")[0].text, "receive");

        for _ in 0..crate::personal::SKIP_LIMIT {
            store.note_skip("recieve");
        }
        assert_eq!(store.english_correction("recieve"), None);
        assert!(
            store
                .english_candidates("recieve")
                .iter()
                .any(|candidate| candidate.text == "receive"),
            "跳過只降權，不刪除"
        );

        store.clear_skip("recieve");
        assert_eq!(store.english_correction("recieve"), Some("receive"));
    }

    #[test]
    fn english_short_correction_records_but_does_not_rank() {
        let mut store = Store::new();
        store.time = 10 * DAY_SECONDS;
        assert!(store.note_english_correction("hellp", "hello"));
        assert_eq!(store.english_correction("hellp"), None);
        assert!(store.english_candidates("hell").is_empty());
        assert_eq!(store.english_pairs["hellp"].text, "hello", "證據要留");
    }
}
