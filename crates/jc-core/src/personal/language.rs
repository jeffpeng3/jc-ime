//! 分語言與跨語言學習（對照 `core/personal/language.go`）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{pair, Model, Store};

/// 分語言學習的語言 ID。
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum LanguageId {
    /// 未知。
    #[default]
    Und,
    /// 中文。
    Zh,
    /// 日文。
    Ja,
    /// 英文。
    En,
}

impl LanguageId {
    /// 字串形式。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Und => "",
            Self::Zh => "zh",
            Self::Ja => "ja",
            Self::En => "en",
        }
    }

    /// 從字串解析。
    pub fn parse(value: &str) -> Self {
        match value {
            "zh" => Self::Zh,
            "ja" => Self::Ja,
            "en" => Self::En,
            _ => Self::Und,
        }
    }
}

const MAX_LANGUAGE_SURFACES: usize = 1024;
const MAX_LANGUAGE_READINGS: usize = 1024;
const MAX_LANGUAGE_CONTEXTS: usize = 1024;
const MAX_LANGUAGE_TRANSITIONS: usize = 128;
const MAX_LANGUAGE_UNDO: usize = 32;

/// 分語言學習的一筆有界計數。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LanguageEvidence {
    /// 次數。
    pub count: u32,
    /// 更新時間。
    pub updated: i64,
}

impl LanguageEvidence {
    fn add(&self, at: i64) -> Self {
        Self {
            count: self.count + 1,
            updated: self.updated.max(at),
        }
    }
}

/// 單一語言的學習資料。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LanguageNamespace {
    /// 這個語言被選過的次數。
    pub selections: u32,
    /// 詞面 → 選取次數。
    pub surfaces: BTreeMap<String, LanguageEvidence>,
    /// 「讀音鍵\t詞面」→ 次數。
    pub readings: BTreeMap<String, LanguageEvidence>,
    /// 「左文\t詞面」→ 次數。
    pub contexts: BTreeMap<String, LanguageEvidence>,
    /// 最後學習時間。
    pub updated: i64,
}

/// 跨語言弱偏好。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CrossLanguage {
    /// 語言 → 被選過次數。
    pub primary: BTreeMap<String, u32>,
    /// 「前一個片段語言\t這次語言」→ 次數。
    pub transitions: BTreeMap<String, u32>,
}

impl CrossLanguage {
    /// 清理空鍵。
    pub fn clean(&mut self) {
        self.primary.retain(|key, _| !key.is_empty());
        self.transitions.retain(|key, _| !key.is_empty());
        trim_counts(&mut self.transitions, MAX_LANGUAGE_TRANSITIONS);
    }
}

/// 一次選字觀察。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LanguageSelection {
    /// 詞面。
    pub surface: String,
    /// 讀音。
    pub reading: Vec<String>,
    /// 左側文字。
    pub left: String,
    /// 明確語言標籤。
    pub language: LanguageId,
    /// 左側片段語言。
    pub previous: LanguageId,
    /// 時間。
    pub at: i64,
}

/// 語言候選。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageCandidate {
    /// 詞面。
    pub surface: String,
    /// 語言。
    pub language: LanguageId,
}

/// 分語言學習的反悔紀錄。
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LanguageUndo {
    pub(crate) language: String,
    pub(crate) surface: String,
    pub(crate) surface_before: LanguageEvidence,
    pub(crate) surface_had: bool,
    pub(crate) reading: String,
    pub(crate) reading_before: LanguageEvidence,
    pub(crate) reading_had: bool,
    pub(crate) context: String,
    pub(crate) context_before: LanguageEvidence,
    pub(crate) context_had: bool,
    pub(crate) selections_before: u32,
    pub(crate) updated_before: i64,
    pub(crate) primary_before: u32,
    pub(crate) primary_had: bool,
    pub(crate) transition: String,
    pub(crate) transition_before: u32,
    pub(crate) transition_had: bool,
}

fn strong_script_language(text: &str) -> Option<LanguageId> {
    for ch in text.chars() {
        if ('ㄅ'..='ㄩ').contains(&ch) || ('ˊ'..='˙').contains(&ch) {
            return Some(LanguageId::Zh);
        }
        if ('぀'..='ヿ').contains(&ch) || ch == 'ー' || ch == 'ｰ' {
            return Some(LanguageId::Ja);
        }
    }
    None
}

fn latin_text(text: &str) -> bool {
    let mut saw = false;
    for ch in text.chars() {
        if is_han(ch) || ('ㄅ'..='ㄩ').contains(&ch) || ('぀'..='ヿ').contains(&ch) {
            return false;
        }
        if ch.is_ascii_alphabetic() || ('Ａ'..='ｚ').contains(&ch) {
            saw = true;
        }
    }
    saw
}

fn is_han(ch: char) -> bool {
    matches!(ch as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

/// 只依詞面推斷語言。
pub fn guess_language(surface: &str, previous: LanguageId) -> LanguageId {
    if let Some(id) = strong_script_language(surface) {
        return id;
    }
    if latin_text(surface) {
        return LanguageId::En;
    }
    previous
}

impl Model {
    /// 加上候選標籤與主要語言後的推斷。
    pub fn infer_language(
        &self,
        surface: &str,
        hint: LanguageId,
        previous: LanguageId,
    ) -> LanguageId {
        if let Some(id) = strong_script_language(surface) {
            return id;
        }
        if hint != LanguageId::Und {
            return hint;
        }
        if latin_text(surface) {
            return LanguageId::En;
        }
        if previous != LanguageId::Und {
            return previous;
        }
        let primary = self.primary_language();
        if primary != LanguageId::Und {
            primary
        } else {
            LanguageId::Zh
        }
    }

    fn language_namespace(&mut self, id: LanguageId) -> &mut LanguageNamespace {
        self.store
            .languages
            .entry(id.as_str().to_string())
            .or_default()
    }

    /// 主要語言。
    pub fn primary_language(&self) -> LanguageId {
        self.store
            .cross_language
            .primary
            .iter()
            .max_by_key(|(_, count)| **count)
            .map_or(LanguageId::Und, |(id, _)| LanguageId::parse(id))
    }

    /// 記錄一次選字。
    pub fn learn_language(&mut self, selection: &LanguageSelection) -> LanguageId {
        if selection.surface.is_empty() {
            return LanguageId::Und;
        }
        let mut id =
            self.infer_language(&selection.surface, selection.language, selection.previous);
        if id == LanguageId::Und {
            id = LanguageId::Zh;
        }
        let at = if selection.at == 0 {
            self.now()
        } else {
            selection.at
        };
        let surface = selection.surface.clone();
        let reading_key = if selection.reading.is_empty() {
            String::new()
        } else {
            pair(&selection.reading.join(" "), &surface)
        };
        let context_key = if selection.left.is_empty() {
            String::new()
        } else {
            pair(&selection.left, &surface)
        };
        let transition_key = if selection.previous != LanguageId::Und && selection.previous != id {
            pair(selection.previous.as_str(), id.as_str())
        } else {
            String::new()
        };

        let ns = self.language_namespace(id);
        let undo = LanguageUndo {
            language: id.as_str().to_string(),
            surface: surface.clone(),
            surface_before: ns.surfaces.get(&surface).copied().unwrap_or_default(),
            surface_had: ns.surfaces.contains_key(&surface),
            reading: reading_key.clone(),
            reading_before: ns.readings.get(&reading_key).copied().unwrap_or_default(),
            reading_had: ns.readings.contains_key(&reading_key),
            context: context_key.clone(),
            context_before: ns.contexts.get(&context_key).copied().unwrap_or_default(),
            context_had: ns.contexts.contains_key(&context_key),
            selections_before: ns.selections,
            updated_before: ns.updated,
            primary_before: self
                .store
                .cross_language
                .primary
                .get(id.as_str())
                .copied()
                .unwrap_or(0),
            primary_had: self.store.cross_language.primary.contains_key(id.as_str()),
            transition: transition_key.clone(),
            transition_before: self
                .store
                .cross_language
                .transitions
                .get(&transition_key)
                .copied()
                .unwrap_or(0),
            transition_had: self
                .store
                .cross_language
                .transitions
                .contains_key(&transition_key),
        };
        self.language_journal.push(undo);
        if self.language_journal.len() > MAX_LANGUAGE_UNDO {
            let drop = self.language_journal.len() - MAX_LANGUAGE_UNDO;
            self.language_journal.drain(..drop);
        }

        let ns = self.language_namespace(id);
        let entry = ns.surfaces.get(&surface).copied().unwrap_or_default();
        ns.surfaces.insert(surface.clone(), entry.add(at));
        ns.selections += 1;
        ns.updated = ns.updated.max(at);
        if !reading_key.is_empty() {
            let entry = ns.readings.get(&reading_key).copied().unwrap_or_default();
            ns.readings.insert(reading_key, entry.add(at));
        }
        if !context_key.is_empty() {
            let entry = ns.contexts.get(&context_key).copied().unwrap_or_default();
            ns.contexts.insert(context_key, entry.add(at));
        }
        *self
            .store
            .cross_language
            .primary
            .entry(id.as_str().to_string())
            .or_default() += 1;
        if !transition_key.is_empty() {
            *self
                .store
                .cross_language
                .transitions
                .entry(transition_key)
                .or_default() += 1;
        }
        self.trim_languages();
        self.gen += 1;
        id
    }

    fn trim_languages(&mut self) {
        let mut store = std::mem::take(&mut self.store);
        for ns in store.languages.values_mut() {
            trim_evidence(&mut ns.surfaces, MAX_LANGUAGE_SURFACES);
            trim_evidence(&mut ns.readings, MAX_LANGUAGE_READINGS);
            trim_evidence(&mut ns.contexts, MAX_LANGUAGE_CONTEXTS);
        }
        trim_counts(
            &mut store.cross_language.transitions,
            MAX_LANGUAGE_TRANSITIONS,
        );
        self.store = store;
    }

    /// 復原最近一次分語言學習。
    pub fn undo_language(&mut self) -> bool {
        let Some(undo) = self.language_journal.pop() else {
            return false;
        };
        if undo.language.is_empty() {
            return false;
        }
        let ns = self
            .store
            .languages
            .entry(undo.language.clone())
            .or_default();
        restore_evidence(
            &mut ns.surfaces,
            &undo.surface,
            undo.surface_before,
            undo.surface_had,
        );
        if !undo.reading.is_empty() {
            restore_evidence(
                &mut ns.readings,
                &undo.reading,
                undo.reading_before,
                undo.reading_had,
            );
        }
        if !undo.context.is_empty() {
            restore_evidence(
                &mut ns.contexts,
                &undo.context,
                undo.context_before,
                undo.context_had,
            );
        }
        ns.selections = undo.selections_before;
        ns.updated = undo.updated_before;
        restore_count(
            &mut self.store.cross_language.primary,
            &undo.language,
            undo.primary_before,
            undo.primary_had,
        );
        if !undo.transition.is_empty() {
            restore_count(
                &mut self.store.cross_language.transitions,
                &undo.transition,
                undo.transition_before,
                undo.transition_had,
            );
        }
        if ns.surfaces.is_empty() && ns.selections == 0 {
            self.store.languages.remove(&undo.language);
        }
        self.gen += 1;
        true
    }

    /// 忘記某個語言裡的一個詞面。
    pub fn forget_language(&mut self, id: LanguageId, surface: &str) -> bool {
        if id == LanguageId::Und || surface.is_empty() {
            return false;
        }
        let Some(ns) = self.store.languages.get_mut(id.as_str()) else {
            return false;
        };
        let Some(entry) = ns.surfaces.remove(surface) else {
            return false;
        };
        let suffix = format!("\t{surface}");
        ns.readings.retain(|key, _| !key.ends_with(&suffix));
        ns.contexts.retain(|key, _| !key.ends_with(&suffix));
        ns.selections = ns.selections.saturating_sub(entry.count);
        if let Some(primary) = self.store.cross_language.primary.get_mut(id.as_str()) {
            *primary = primary.saturating_sub(entry.count);
        }
        if ns.surfaces.is_empty() && ns.selections == 0 {
            self.store.languages.remove(id.as_str());
        }
        self.language_journal.clear();
        self.gen += 1;
        true
    }

    /// 清掉一個語言的全部學習。
    pub fn reset_language(&mut self, id: LanguageId) -> bool {
        if id == LanguageId::Und {
            return false;
        }
        let removed = self.store.languages.remove(id.as_str()).is_some();
        self.store.cross_language.primary.remove(id.as_str());
        let prefix = format!("{}\t", id.as_str());
        self.store.cross_language.transitions.retain(|key, _| {
            !key.starts_with(&prefix) && !key.ends_with(&format!("\t{}", id.as_str()))
        });
        if removed {
            self.language_journal.clear();
            self.gen += 1;
        }
        removed
    }

    /// 分語言候選排序的弱偏好。
    pub fn language_preference(
        &self,
        candidates: &[LanguageCandidate],
        previous: LanguageId,
    ) -> Vec<f64> {
        candidates
            .iter()
            .map(|candidate| {
                let Some(ns) = self.store.languages.get(candidate.language.as_str()) else {
                    return 0.0;
                };
                let surface = ns
                    .surfaces
                    .get(&candidate.surface)
                    .map_or(0.0, |entry| entry.count as f64);
                let transition = self
                    .store
                    .cross_language
                    .transitions
                    .get(&pair(previous.as_str(), candidate.language.as_str()))
                    .copied()
                    .unwrap_or(0) as f64;
                surface + transition * 0.25
            })
            .collect()
    }
}

impl Store {
    pub(crate) fn clean_languages(&mut self) {
        self.languages.retain(|id, _| !id.is_empty());
        for ns in self.languages.values_mut() {
            trim_evidence(&mut ns.surfaces, MAX_LANGUAGE_SURFACES);
            trim_evidence(&mut ns.readings, MAX_LANGUAGE_READINGS);
            trim_evidence(&mut ns.contexts, MAX_LANGUAGE_CONTEXTS);
            ns.selections = ns.surfaces.values().map(|entry| entry.count).sum();
        }
        self.cross_language.clean();
    }
}

fn trim_evidence(map: &mut BTreeMap<String, LanguageEvidence>, limit: usize) {
    if map.len() <= limit {
        return;
    }
    let mut keys: Vec<_> = map.keys().cloned().collect();
    keys.sort_by_key(|key| {
        let entry = map.get(key).copied().unwrap_or_default();
        (entry.count, entry.updated, key.clone())
    });
    let drop = keys.len() - limit;
    for key in keys.into_iter().take(drop) {
        map.remove(&key);
    }
}

fn trim_counts(map: &mut BTreeMap<String, u32>, limit: usize) {
    if map.len() <= limit {
        return;
    }
    let mut keys: Vec<_> = map.keys().cloned().collect();
    keys.sort_by_key(|key| (map.get(key).copied().unwrap_or(0), key.clone()));
    let drop = keys.len() - limit;
    for key in keys.into_iter().take(drop) {
        map.remove(&key);
    }
}

fn restore_evidence(
    map: &mut BTreeMap<String, LanguageEvidence>,
    key: &str,
    before: LanguageEvidence,
    had: bool,
) {
    if had {
        map.insert(key.to_string(), before);
    } else {
        map.remove(key);
    }
}

fn restore_count(map: &mut BTreeMap<String, u32>, key: &str, before: u32, had: bool) {
    if had {
        map.insert(key.to_string(), before);
    } else {
        map.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::personal::{Model, NoLexicon, NoLm};

    #[test]
    fn language_is_namespaced() {
        let mut m = Model::new(Box::new(NoLexicon), Box::new(NoLm));
        m.learn_language(&LanguageSelection {
            surface: "漢字".to_string(),
            language: LanguageId::Zh,
            ..LanguageSelection::default()
        });
        m.learn_language(&LanguageSelection {
            surface: "かな".to_string(),
            language: LanguageId::Ja,
            ..LanguageSelection::default()
        });
        assert_eq!(m.store.languages["zh"].surfaces["漢字"].count, 1);
        assert_eq!(m.store.languages["ja"].surfaces["かな"].count, 1);
        assert!(!m.store.languages["zh"].surfaces.contains_key("かな"));
    }

    #[test]
    fn strong_script_beats_hint() {
        let m = Model::new(Box::new(NoLexicon), Box::new(NoLm));
        assert_eq!(
            m.infer_language("かな", LanguageId::Zh, LanguageId::Und),
            LanguageId::Ja
        );
        assert_eq!(
            m.infer_language("ㄅ", LanguageId::Ja, LanguageId::Und),
            LanguageId::Zh
        );
    }

    #[test]
    fn undo_restores_last_selection() {
        let mut m = Model::new(Box::new(NoLexicon), Box::new(NoLm));
        m.learn_language(&LanguageSelection {
            surface: "漢字".to_string(),
            language: LanguageId::Zh,
            ..LanguageSelection::default()
        });
        assert!(m.undo_language());
        assert!(!m.store.languages.contains_key("zh"));
    }
}
