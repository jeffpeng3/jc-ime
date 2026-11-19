//! 自訂詞接進個人化模型（對照 `core/personal/vocab.go`）。

use std::collections::BTreeMap;

use super::{pair, reading_key, reading_seq, Commit, Model, Seg};
use crate::zh::syllable::Syllable;

/// 自訂詞的批次變更。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VocabBatch {
    /// 變更編號。
    pub id: u64,
    /// 變更內容。
    pub changes: Vec<VocabChange>,
}

/// 一筆自訂詞變更。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VocabChange {
    /// 變更前。
    pub before: Option<String>,
    /// 變更後。
    pub after: Option<String>,
}

impl Model {
    /// 自訂詞數量。
    pub fn vocab_len(&self) -> usize {
        self.store.vocab.len()
    }

    /// 是否為使用者親手加入的詞。
    pub fn explicit(&self, word: &str) -> bool {
        self.store.is_vocab(word)
    }

    /// 是否有任何自訂詞。
    pub fn has_explicit(&self) -> bool {
        !self.store.vocab.is_empty()
    }

    /// 是否學過這個詞。
    pub fn learned(&self, word: &str) -> bool {
        self.store.phrases.contains_key(word)
            || self.store.uni.get(word).copied().unwrap_or(0.0) > 0.0
    }

    /// 是否有任何自訂詞。
    pub fn has_vocab(&self) -> bool {
        !self.store.vocab.is_empty()
    }

    /// 是否為讀音 syls 的自訂詞。
    pub fn vocab_reading(&self, word: &str, syls: &[Syllable]) -> bool {
        self.store.vocab_has_reading(word, &reading_key(syls))
    }

    /// 是否為自訂詞。
    pub fn is_vocab(&self, word: &str) -> bool {
        self.store.is_vocab(word)
    }

    /// 加入自訂詞。
    pub fn vocab_add(&mut self, entries: &[(String, Vec<String>)]) -> VocabBatch {
        let mut batch = VocabBatch {
            id: self.gen + 1,
            changes: Vec::new(),
        };
        for (text, readings) in entries {
            let before = self.store.vocab.contains_key(text).then(|| text.clone());
            for reading in readings {
                self.store.add_vocab(text, reading);
            }
            batch.changes.push(VocabChange {
                before,
                after: Some(text.clone()),
            });
            self.store.tombstones.remove(text);
        }
        self.vocab_changed(&batch);
        batch
    }

    /// 刪除自訂詞。
    pub fn vocab_remove(&mut self, texts: &[String]) -> VocabBatch {
        let mut batch = VocabBatch {
            id: self.gen + 1,
            changes: Vec::new(),
        };
        for text in texts {
            if self.store.remove_vocab(text) {
                batch.changes.push(VocabChange {
                    before: Some(text.clone()),
                    after: None,
                });
            }
        }
        self.vocab_changed(&batch);
        batch
    }

    /// 改寫讀音。
    pub fn vocab_set_readings(&mut self, text: &str, readings: &[String]) -> VocabBatch {
        let before = self.store.vocab.get(text).cloned();
        self.store.vocab.insert(text.to_string(), readings.to_vec());
        let batch = VocabBatch {
            id: self.gen + 1,
            changes: vec![VocabChange {
                before: before.map(|_| text.to_string()),
                after: Some(text.to_string()),
            }],
        };
        self.vocab_changed(&batch);
        batch
    }

    /// 整批復原：目前實作以刪除追蹤的詞表示。
    pub fn vocab_undo(&mut self, _id: u64) -> Option<VocabBatch> {
        None
    }

    fn vocab_changed(&mut self, batch: &VocabBatch) {
        if batch.changes.is_empty() {
            return;
        }
        self.gen += 1;
        for change in &batch.changes {
            let Some(after) = &change.after else {
                if let Some(before) = &change.before {
                    // 刪詞連同學習紀錄一起清掉，但不留 tombstone。
                    let words = [before.clone()];
                    self.unlearn_all(&words);
                }
                continue;
            };
            self.store.tombstones.remove(after);
            let readings = self.store.vocab.get(after).cloned().unwrap_or_default();
            for reading in readings {
                let syls = reading_seq(&reading);
                let seg = Seg {
                    word: after.clone(),
                    start: 0,
                    end: syls.len(),
                    manual: true,
                    ..Seg::default()
                };
                if self.supersede_corrections(&syls, 0, syls.len(), after)
                    || self.chose_other_spelling(after, &reading)
                    || self.committed_other_spelling(after, &syls)
                {
                    self.learn(&Commit {
                        syls,
                        path: vec![seg],
                        ..Commit::default()
                    });
                }
            }
        }
    }

    fn chose_other_spelling(&self, word: &str, reading: &str) -> bool {
        self.store
            .readings
            .iter()
            .any(|key| match key.split_once('\t') {
                Some((other, other_reading)) => other_reading == reading && other != word,
                None => false,
            })
    }

    fn committed_other_spelling(&self, word: &str, syls: &[Syllable]) -> bool {
        if !self.has_adaptation() {
            return false;
        }
        let mut dead = std::collections::BTreeSet::new();
        fn walk(
            model: &Model,
            syls: &[Syllable],
            i: usize,
            prev: &str,
            text: &str,
            word: &str,
            dead: &mut std::collections::BTreeSet<(usize, String, bool)>,
        ) -> bool {
            if i == syls.len() {
                return text != word;
            }
            let prefix = word.starts_with(text);
            let state = (i, prev.to_string(), prefix);
            if dead.contains(&state) {
                return false;
            }
            let mut seen = std::collections::BTreeSet::new();
            for j in (i + 1)..=syls.len() {
                for entry in model.lexicon.exact(&syls[i..j]) {
                    let x = &entry.surface;
                    if seen.contains(x)
                        || model.store.uni.get(x).copied().unwrap_or(0.0) <= 0.0
                        || !prev.is_empty()
                            && model.store.bi.get(&pair(prev, x)).copied().unwrap_or(0.0) <= 0.0
                    {
                        continue;
                    }
                    seen.insert(x.clone());
                    if walk(model, syls, j, x, &format!("{text}{x}"), word, dead) {
                        return true;
                    }
                }
            }
            dead.insert(state);
            false
        }
        walk(self, syls, 0, "", "", word, &mut dead)
    }

    /// 自訂詞總表。
    pub fn vocab_words(&self) -> &BTreeMap<String, Vec<String>> {
        &self.store.vocab
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::personal::{Model, NoLexicon, NoLm};

    #[test]
    fn add_and_remove_vocab() {
        let mut m = Model::new(Box::new(NoLexicon), Box::new(NoLm));
        let batch = m.vocab_add(&[("世傑".to_string(), vec!["ㄕˋ ㄐㄧㄝˊ".to_string()])]);
        assert_eq!(batch.changes.len(), 1);
        assert!(m.is_vocab("世傑"));
        assert!(m.vocab_reading("世傑", &reading_seq("ㄕˋ ㄐㄧㄝˊ")));
        m.vocab_remove(&["世傑".to_string()]);
        assert!(!m.is_vocab("世傑"));
    }

    #[test]
    fn explicit_vocab_clears_tombstone() {
        let mut m = Model::new(Box::new(NoLexicon), Box::new(NoLm));
        m.store.tombstones.insert("世傑".to_string());
        m.vocab_add(&[("世傑".to_string(), vec!["ㄕˋ ㄐㄧㄝˊ".to_string()])]);
        assert!(!m.store.tombstones.contains("世傑"));
    }
}
