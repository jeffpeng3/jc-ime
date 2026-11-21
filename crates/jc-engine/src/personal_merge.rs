//! 個人化資料的增量合併。
//!
//! engine 只持有一份共用的 [`PersonalStore`]。每條 session 在建立時拿到這份
//! 資料的快照，學習只寫進自己那份。要回寫時，用建立時的快照當基準算出增量，
//! 再加進共用 store，因此同一份種子資料不會被每個 session 重複累加。
//!
//! 計數型欄位用「加法增量」；整筆紀錄（更正、代名詞習慣、英文紀錄）用「相對
//! 基準有變就覆蓋」；集合與順序串列用聯集。`serde(skip)` 的暫時性欄位
//! （rollback、反悔 journal、自訂詞）不進檔，也不合併。

use std::collections::{BTreeMap, BTreeSet, HashMap};

use jc_core::emoji::Stat;
use jc_core::personal::{
    LanguageEvidence, LanguageNamespace, PersonalStore, UsageEvidence, STORE_FORMAT,
};

/// 把 `other` 相對 `base` 的學習增量合併進 `global`。
pub fn merge_learning(global: &mut PersonalStore, base: &PersonalStore, other: &PersonalStore) {
    global.format = STORE_FORMAT;
    global.correction_serial = global.correction_serial.max(other.correction_serial);

    merge_records(
        &mut global.corrections,
        &base.corrections,
        &other.corrections,
    );
    delta_f64_map(&mut global.bi, &base.bi, &other.bi);
    delta_f64_map(&mut global.ctx, &base.ctx, &other.ctx);
    delta_f64_map(&mut global.uni, &base.uni, &other.uni);
    delta_f64(&mut global.total, base.total, other.total);
    delta_f64_map(&mut global.uses, &base.uses, &other.uses);
    delta_usage_map(
        &mut global.evidence_by_context,
        &base.evidence_by_context,
        &other.evidence_by_context,
    );
    delta_usage_map(
        &mut global.evidence_by_word,
        &base.evidence_by_word,
        &other.evidence_by_word,
    );

    global.time = global.time.max(other.time);
    global.time_set |= other.time_set;
    merge_string_lists(&mut global.picked, &base.picked, &other.picked);
    delta_i64_map(
        &mut global.phrase_seen,
        &base.phrase_seen,
        &other.phrase_seen,
    );
    merge_records(&mut global.phrases, &base.phrases, &other.phrases);
    union_vec(
        &mut global.phrase_order,
        &base.phrase_order,
        &other.phrase_order,
    );
    union_set(&mut global.tombstones, &base.tombstones, &other.tombstones);
    union_set(&mut global.readings, &base.readings, &other.readings);
    delta_u32_map(&mut global.rejects, &base.rejects, &other.rejects);
    delta_usage_map(&mut global.confirmed, &base.confirmed, &other.confirmed);
    delta_usage_map(
        &mut global.zhuyin_keys,
        &base.zhuyin_keys,
        &other.zhuyin_keys,
    );
    merge_records(&mut global.pronouns, &base.pronouns, &other.pronouns);
    merge_records(&mut global.english, &base.english, &other.english);
    merge_records(
        &mut global.english_pairs,
        &base.english_pairs,
        &other.english_pairs,
    );

    union_vec(
        &mut global.emoji_recent,
        &base.emoji_recent,
        &other.emoji_recent,
    );
    delta_stat_map(
        &mut global.emoji_stats,
        &base.emoji_stats,
        &other.emoji_stats,
    );
    if other.palette_tab != base.palette_tab {
        global.palette_tab = other.palette_tab.clone();
    }
    union_vec(
        &mut global.symbol_recent,
        &base.symbol_recent,
        &other.symbol_recent,
    );
    delta_stat_map(
        &mut global.symbol_stats,
        &base.symbol_stats,
        &other.symbol_stats,
    );
    merge_languages(&mut global.languages, &base.languages, &other.languages);
    delta_u32_map(
        &mut global.cross_language.primary,
        &base.cross_language.primary,
        &other.cross_language.primary,
    );
    delta_u32_map(
        &mut global.cross_language.transitions,
        &base.cross_language.transitions,
        &other.cross_language.transitions,
    );
    delta_u32_map(&mut global.skips, &base.skips, &other.skips);
}

/// f64 權重只加正增量，避免時鐘或重播造成的倒退被算成新學習。
fn delta_f64(global: &mut f64, base: f64, other: f64) {
    let delta = other - base;
    if delta.is_finite() && delta > 0.0 {
        *global += delta;
    }
}

fn delta_f64_map(
    global: &mut BTreeMap<String, f64>,
    base: &BTreeMap<String, f64>,
    other: &BTreeMap<String, f64>,
) {
    for (key, value) in other {
        let base_value = base.get(key).copied().unwrap_or(0.0);
        delta_f64(global.entry(key.clone()).or_default(), base_value, *value);
    }
}

fn delta_u32_map(
    global: &mut BTreeMap<String, u32>,
    base: &BTreeMap<String, u32>,
    other: &BTreeMap<String, u32>,
) {
    for (key, value) in other {
        let base_value = base.get(key).copied().unwrap_or(0);
        let delta = value.saturating_sub(base_value);
        if delta > 0 {
            let entry = global.entry(key.clone()).or_default();
            *entry = entry.saturating_add(delta);
        }
    }
}

fn delta_i64_map(
    global: &mut BTreeMap<String, i64>,
    base: &BTreeMap<String, i64>,
    other: &BTreeMap<String, i64>,
) {
    for (key, value) in other {
        let base_value = base.get(key).copied().unwrap_or(0);
        let delta = value.saturating_sub(base_value);
        if delta > 0 {
            let entry = global.entry(key.clone()).or_default();
            *entry = entry.saturating_add(delta);
        }
    }
}

fn union_set(global: &mut BTreeSet<String>, base: &BTreeSet<String>, other: &BTreeSet<String>) {
    for item in other {
        if !base.contains(item) {
            global.insert(item.clone());
        }
    }
}

fn union_vec(global: &mut Vec<String>, base: &[String], other: &[String]) {
    for item in other {
        if !base.contains(item) && !global.contains(item) {
            global.push(item.clone());
        }
    }
}

fn merge_string_lists(
    global: &mut BTreeMap<String, Vec<String>>,
    base: &BTreeMap<String, Vec<String>>,
    other: &BTreeMap<String, Vec<String>>,
) {
    let empty = Vec::new();
    for (key, values) in other {
        let base_values = base.get(key).unwrap_or(&empty);
        let entry = global.entry(key.clone()).or_default();
        union_vec(entry, base_values, values);
    }
}

/// 整筆紀錄相對基準有變才算新資料，避免把共用種子重複累加。
fn merge_records<T: Clone + PartialEq>(
    global: &mut BTreeMap<String, T>,
    base: &BTreeMap<String, T>,
    other: &BTreeMap<String, T>,
) {
    for (key, value) in other {
        if base.get(key) != Some(value) {
            global.insert(key.clone(), value.clone());
        }
    }
}

fn delta_usage(global: &mut UsageEvidence, base: UsageEvidence, other: UsageEvidence) {
    delta_f64(&mut global.short, base.short, other.short);
    delta_f64(&mut global.long, base.long, other.long);
    delta_f64(&mut global.day_weight, base.day_weight, other.day_weight);
    global.updated = global.updated.max(other.updated);
    global.day = global.day.max(other.day);
}

fn delta_usage_map(
    global: &mut BTreeMap<String, UsageEvidence>,
    base: &BTreeMap<String, UsageEvidence>,
    other: &BTreeMap<String, UsageEvidence>,
) {
    for (key, value) in other {
        let base_value = base.get(key).copied().unwrap_or_default();
        let entry = global.entry(key.clone()).or_default();
        delta_usage(entry, base_value, *value);
    }
}

fn delta_stat(global: &mut Stat, base: Stat, other: Stat) {
    delta_f64(&mut global.score, base.score, other.score);
    global.at = global.at.max(other.at);
}

fn delta_stat_map(
    global: &mut HashMap<String, Stat>,
    base: &HashMap<String, Stat>,
    other: &HashMap<String, Stat>,
) {
    for (key, value) in other {
        let base_value = base.get(key).copied().unwrap_or(Stat { score: 0.0, at: 0 });
        let entry = global
            .entry(key.clone())
            .or_insert(Stat { score: 0.0, at: 0 });
        delta_stat(entry, base_value, *value);
    }
}

fn delta_language_evidence(
    global: &mut LanguageEvidence,
    base: LanguageEvidence,
    other: LanguageEvidence,
) {
    global.count = global
        .count
        .saturating_add(other.count.saturating_sub(base.count));
    global.updated = global.updated.max(other.updated);
}

fn delta_language_map(
    global: &mut BTreeMap<String, LanguageEvidence>,
    base: &BTreeMap<String, LanguageEvidence>,
    other: &BTreeMap<String, LanguageEvidence>,
) {
    for (key, value) in other {
        let base_value = base.get(key).copied().unwrap_or_default();
        let entry = global.entry(key.clone()).or_default();
        delta_language_evidence(entry, base_value, *value);
    }
}

fn merge_language_namespace(
    global: &mut LanguageNamespace,
    base: &LanguageNamespace,
    other: &LanguageNamespace,
) {
    global.selections = global
        .selections
        .saturating_add(other.selections.saturating_sub(base.selections));
    global.updated = global.updated.max(other.updated);
    delta_language_map(&mut global.surfaces, &base.surfaces, &other.surfaces);
    delta_language_map(&mut global.readings, &base.readings, &other.readings);
    delta_language_map(&mut global.contexts, &base.contexts, &other.contexts);
}

fn merge_languages(
    global: &mut BTreeMap<String, LanguageNamespace>,
    base: &BTreeMap<String, LanguageNamespace>,
    other: &BTreeMap<String, LanguageNamespace>,
) {
    let empty = LanguageNamespace::default();
    for (key, value) in other {
        let base_value = base.get(key).unwrap_or(&empty);
        let entry = global.entry(key.clone()).or_default();
        merge_language_namespace(entry, base_value, value);
    }
}
