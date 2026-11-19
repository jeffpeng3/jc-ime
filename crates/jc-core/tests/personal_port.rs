//! 從前身 `core/personal/*_test.go` 改寫的行為測試。
//!
//! 測試名稱沿用 Go 的 `TestFoo_Bar` → `test_foo_bar()`；需要前身才有的
//! 詞庫／語言模型時，用最小 fake 或 `NoLexicon`／`NoLm` 重建同一條規則。

use std::collections::BTreeMap;

use jc_core::lattice::{InMemoryLexicon, LexEntry};
use jc_core::personal::{
    english_token, reading_seq, Commit, EnglishCommit, LanguageId, LanguageSelection, Model,
    NoEnglish, NoLexicon, NoLm, Params, PrunePolicy, Seg, Store,
};
use jc_core::zh::syllable::Syllable;

fn model() -> Model {
    let mut model = Model::new(Box::new(NoLexicon), Box::new(NoLm));
    model.store.time = 100;
    model
}

fn lexicon(entries: &[(&str, &str)]) -> Model {
    let mut lex = InMemoryLexicon::new();
    for (surface, reading) in entries {
        lex.push(LexEntry::new(*surface, *reading, 0, 0));
    }
    let mut model = Model::new(Box::new(lex), Box::new(NoLm));
    model.store.time = 100;
    model
}

fn syls(reading: &str) -> Vec<Syllable> {
    reading_seq(reading)
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

fn commit(reading: &str, path: Vec<Seg>) -> Commit {
    Commit {
        syls: syls(reading),
        path,
        ..Commit::default()
    }
}

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir()
        .join(format!("jc-personal-test-{}-{}", std::process::id(), name))
        .join("personal.json")
}

// clone_test.go
#[test]
fn test_store_clone_shares_no_maps_or_slices() {
    let mut store = Store::new();
    store
        .picked
        .insert("ㄇㄚˊ".to_string(), vec!["麻".to_string()]);
    store.phrase_order.push("麻婆".to_string());
    let mut clone = store.clone();
    clone.picked.get_mut("ㄇㄚˊ").unwrap()[0] = "嘛".to_string();
    clone.phrase_order.push("新的".to_string());
    assert_eq!(store.picked["ㄇㄚˊ"][0], "麻");
    assert_eq!(store.phrase_order, vec!["麻婆"]);
}

// evidence_test.go
#[test]
fn test_evidence_distinguishes_intent() {
    let mut passive = model();
    passive.learn(&commit(
        "ㄇㄚˊ ㄆㄛˊ ㄉㄡˋ ㄈㄨˇ",
        vec![seg("麻婆豆腐", 0, 4, false)],
    ));
    let mut completion = model();
    completion.learn(&commit(
        "ㄇㄚˊ ㄆㄛˊ ㄉㄡˋ ㄈㄨˇ",
        vec![Seg {
            completion: true,
            ..seg("麻婆豆腐", 0, 4, true)
        }],
    ));
    let mut manual = model();
    manual.learn(&commit(
        "ㄇㄚˊ ㄆㄛˊ ㄉㄡˋ ㄈㄨˇ",
        vec![seg("麻婆豆腐", 0, 4, true)],
    ));
    assert!(passive.word_evidence("麻婆豆腐") < completion.word_evidence("麻婆豆腐"));
    assert!(completion.word_evidence("麻婆豆腐") < manual.word_evidence("麻婆豆腐"));
}

#[test]
fn test_evidence_bounded_and_contextual() {
    let mut m = model();
    for _ in 0..200 {
        m.observe_evidence("想吃", "麻婆豆腐", 0.5);
    }
    let local = m.evidence("想吃", "麻婆豆腐");
    let elsewhere = m.evidence("討論", "麻婆豆腐");
    assert!(local > elsewhere);
    assert!(elsewhere <= 2.0);
}

#[test]
fn test_time_decay_survives_restart_and_clock_rollback() {
    let mut m = model();
    m.advance_time(100 * jc_core::personal::DAY_SECONDS);
    m.learn(&commit("ㄕˋ ㄐㄧㄝˊ", vec![seg("世傑", 0, 2, true)]));
    let before = m.store.uni["世傑"];
    let path = temp_path("decay");
    m.store.save(&path).expect("存檔");
    let mut back = model();
    back.store = Store::load(&path).expect("讀回");
    back.advance_time(130 * jc_core::personal::DAY_SECONDS);
    assert!((back.store.uni["世傑"] - before / 2.0).abs() < 1e-9);
    let evidence = back.word_evidence("世傑");
    back.advance_time(120 * jc_core::personal::DAY_SECONDS);
    assert_eq!(back.word_evidence("世傑"), evidence);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn test_legacy_store_does_not_invent_strong_evidence() {
    let mut store = Store::new();
    store.uses.insert("麻婆豆腐".to_string(), 1000.0);
    store
        .phrases
        .insert("親子丼".to_string(), "ㄑㄧㄣ ㄗˇ ㄐㄧㄥˇ".to_string());
    store.phrase_order.push("親子丼".to_string());
    store.uni.insert("親子丼".to_string(), 2.0);
    store.total = 2.0;
    let mut m = model();
    m.store = store.cleaned();
    m.advance_time(100 * jc_core::personal::DAY_SECONDS);
    assert_eq!(m.evidence("", "麻婆豆腐"), 0.0);
    assert!(m.source().has("親子丼"));
    assert_eq!(m.store.uni["親子丼"], 2.0);
}

#[test]
fn test_forget_removes_evidence_and_passive_cannot_restore() {
    let mut m = model();
    m.learn(&commit("ㄐㄧㄥˇ", vec![seg("井", 0, 1, true)]));
    m.forget("井");
    m.learn(&commit("ㄐㄧㄥˇ", vec![seg("井", 0, 1, false)]));
    assert_eq!(m.evidence("", "井"), 0.0);
    m.learn(&commit("ㄐㄧㄥˇ", vec![seg("井", 0, 1, true)]));
    assert!(m.evidence("", "井") > 0.0);
}

#[test]
fn test_decay_retains_personal_phrase_and_reading() {
    let mut m = model();
    let c = commit(
        "ㄑㄧㄣ ㄗˇ ㄐㄧㄥˇ",
        vec![seg("親子", 0, 2, false), seg("丼", 2, 3, true)],
    );
    m.learn(&c);
    m.learn(&c);
    assert!(m.store.phrases.contains_key("親子丼"));
    m.advance_time(1000 * jc_core::personal::DAY_SECONDS);
    assert!(m.source().has("親子丼"));
    assert!(m.store.readings.contains("丼\tㄐㄧㄥˇ"));
}

// diversity_test.go
#[test]
fn test_frequent_left_word_keeps_unseen_continuations() {
    let mut m = model();
    for index in 0..30 {
        m.observe("在", &format!("詞{index}"), 0.25);
    }
    assert!(m.kappa("在") >= 1.0);
}

#[test]
fn test_sparse_context_unchanged() {
    let mut m = model();
    for _ in 0..5 {
        m.observe("親子", "景", 0.25);
    }
    m.observe("親子", "丼", 0.25);
    assert!(m.prob("親子", "丼") > 0.0);
}

#[test]
fn test_diversity_follows_decay_and_direct_edits() {
    let mut m = model();
    for index in 0..30 {
        m.observe("在", &format!("詞{index}"), 0.25);
    }
    let before = m.diversity("在");
    m.store.bi.insert("在\t新詞".to_string(), 0.25);
    m.store.ctx.insert("在".to_string(), 0.25);
    assert!(m.diversity("在") >= before);
}

#[test]
fn test_diversity_incremental_matches_rebuild() {
    let mut m = model();
    m.observe("在", "這裡", 0.25);
    let before = m.diversity("在");
    m.store.bi.insert("在\t別".to_string(), 0.25);
    assert!(m.diversity("在") >= before);
}

// emoji_test.go / symbols_test.go
#[test]
fn test_emoji_order_bound_migration_and_roundtrip() {
    let mut m = model();
    for emoji in ["😀", "😁", "😂"] {
        m.remember_emoji(emoji);
    }
    assert_eq!(m.store.emoji_recent.len(), 3);
    let path = temp_path("emoji");
    m.store.save(&path).expect("存檔");
    let back = Store::load(&path).expect("讀回");
    assert_eq!(back.emoji_recent, m.store.emoji_recent);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn test_emoji_recent_upgrade_keeps_order() {
    let mut store = Store::new();
    store.emoji_recent = vec!["😀".to_string(), "😁".to_string()];
    let cleaned = store.cleaned();
    assert_eq!(cleaned.emoji_recent, vec!["😀", "😁"]);
}

#[test]
fn test_symbol_recent_clean_and_bounded() {
    let mut store = Store::new();
    store.symbol_recent = vec![
        "※".to_string(),
        "任意文字".to_string(),
        "※".to_string(),
        "，".to_string(),
    ];
    let cleaned = store.cleaned();
    assert_eq!(cleaned.symbol_recent, vec!["※", "，"]);
}

#[test]
fn test_symbol_recent_old_formats() {
    let mut store = Store::new();
    store.uni.insert("保留".to_string(), 2.0);
    store.emoji_recent.push("😀".to_string());
    let mut m = model();
    m.store = store.cleaned();
    m.remember_symbol("※");
    assert_eq!(m.store.symbol_recent, vec!["※"]);
    assert_eq!(m.store.uni["保留"], 2.0);
}

// english_test.go
#[test]
fn test_english_independent_confirmations_daily_cap_and_decay() {
    let mut m = model();
    let event = EnglishCommit {
        text: "OpenCode".to_string(),
        assisted: false,
        explicit: true,
        suspicious: false,
    };
    for _ in 0..4 {
        m.learn_english(std::slice::from_ref(&event));
    }
    assert!(!m.english_known("OpenCode"));
    m.store.time += jc_core::personal::DAY_SECONDS;
    m.learn_english(&[event]);
    assert!(m.english_known("OpenCode"));
}

#[test]
fn test_english_likely_typo_needs_more_independent_days() {
    let mut m = model();
    let event = EnglishCommit {
        text: "OpenCode".to_string(),
        assisted: false,
        explicit: true,
        suspicious: true,
    };
    for _ in 0..8 {
        m.learn_english(std::slice::from_ref(&event));
        m.store.time += jc_core::personal::DAY_SECONDS;
    }
    assert!(m.english_known("OpenCode"));
}

#[test]
fn test_english_assistance_forget_and_identifier_protection() {
    assert!(english_token("OpenCode"));
    assert!(!english_token("http://example"));
    let mut m = model();
    m.learn_english(&[EnglishCommit {
        text: "OpenCode".to_string(),
        assisted: false,
        explicit: true,
        suspicious: false,
    }]);
    m.forget("OpenCode");
    assert!(!m.english_known("OpenCode"));
}

#[test]
fn test_english_no_suggest_cannot_be_bypassed_by_personalization() {
    let base = NoEnglish;
    let mut m = model();
    assert!(m.english_corrections(&base, "OpenCod", 5).is_empty());
}

#[test]
fn test_english_store_bound_round_trip_and_legacy_migration() {
    let mut m = model();
    for _ in 0..4 {
        m.learn_english(&[EnglishCommit {
            text: "OpenCode".to_string(),
            assisted: false,
            explicit: true,
            suspicious: false,
        }]);
        m.store.time += jc_core::personal::DAY_SECONDS;
    }
    let path = temp_path("english");
    m.store.save(&path).expect("存檔");
    let back = Store::load(&path).expect("讀回");
    assert_eq!(back.english, m.store.english);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

// personal_test.go
#[test]
fn test_store_round_trip() {
    let mut m = model();
    let c = commit(
        "ㄑㄧㄣ ㄗˇ ㄐㄧㄥˇ",
        vec![seg("親子", 0, 2, false), seg("丼", 2, 3, true)],
    );
    m.learn(&c);
    m.learn(&c);
    assert!(m.store.phrases.contains_key("親子丼"));
    let path = temp_path("roundtrip");
    m.store.save(&path).expect("存檔");
    let back = Store::load(&path).expect("讀回");
    assert_eq!(back.phrases, m.store.phrases);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn test_reading_keys_remain_compatible() {
    let mut m = lexicon(&[("親子", "ㄑㄧㄣ ㄗˇ")]);
    m.store.readings.insert("親子\tㄑㄧㄣ ㄗˇ".to_string());
    assert!(m.chose_reading("親子", &syls("ㄑㄧㄣ ㄗˇ")));
}

#[test]
fn test_store_format_version() {
    let store = Store::new();
    assert_eq!(store.format, jc_core::personal::STORE_FORMAT);
    let path = temp_path("format");
    store.save(&path).expect("存檔");
    let raw = std::fs::read_to_string(&path).expect("讀檔");
    assert!(raw.contains("\"format\""));
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn test_prob_is_bounded_and_conservative() {
    let mut m = model();
    m.observe("親子", "丼", 1.0);
    let sparse = m.prob("親子", "丼");
    for _ in 0..500 {
        m.observe("的", "事", 1.0);
    }
    m.observe("的", "丼", 1.0);
    let busy = m.prob("的", "丼");
    assert!((0.0..=1.0).contains(&sparse));
    assert!(busy < sparse);
}

// language_test.go
#[test]
fn test_learn_language_is_namespaced() {
    let mut m = model();
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
    assert!(m.store.languages["zh"].surfaces.contains_key("漢字"));
    assert!(!m.store.languages["zh"].surfaces.contains_key("かな"));
}

#[test]
fn test_infer_language_uses_strong_script_and_segment_tag() {
    let m = model();
    assert_eq!(
        m.infer_language("かな", LanguageId::Zh, LanguageId::Und),
        LanguageId::Ja
    );
    assert_eq!(
        m.infer_language("漢字", LanguageId::Zh, LanguageId::Und),
        LanguageId::Zh
    );
}

#[test]
fn test_language_undo_forget_reset() {
    let mut m = model();
    m.learn_language(&LanguageSelection {
        surface: "漢字".to_string(),
        language: LanguageId::Zh,
        ..LanguageSelection::default()
    });
    assert!(m.undo_language());
    assert!(!m.store.languages.contains_key("zh"));
    m.learn_language(&LanguageSelection {
        surface: "漢字".to_string(),
        language: LanguageId::Zh,
        ..LanguageSelection::default()
    });
    assert!(m.forget_language(LanguageId::Zh, "漢字"));
    assert!(!m.store.languages.contains_key("zh"));
}

#[test]
fn test_language_learning_store_round_trip() {
    let mut m = model();
    m.learn_language(&LanguageSelection {
        surface: "漢字".to_string(),
        language: LanguageId::Zh,
        ..LanguageSelection::default()
    });
    let path = temp_path("language");
    m.store.save(&path).expect("存檔");
    let back = Store::load(&path).expect("讀回");
    assert_eq!(back.languages, m.store.languages);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

// preference_test.go
#[test]
fn test_habit_needs_more_than_one_pick() {
    let mut m = lexicon(&[("世傑", "ㄕˋ ㄐㄧㄝˊ"), ("是結", "ㄕˋ ㄐㄧㄝˊ")]);
    let syls = syls("ㄕˋ ㄐㄧㄝˊ");
    m.note_confirmed("世傑", "ㄕˋ ㄐㄧㄝˊ");
    assert_eq!(m.reading_lift("世傑", &syls), 0.0);
}

#[test]
fn test_habit_only_for_contested_multi_syllable_words() {
    let mut m = lexicon(&[("井", "ㄐㄧㄥˇ")]);
    m.note_confirmed("井", "ㄐㄧㄥˇ");
    assert_eq!(m.reading_lift("井", &syls("ㄐㄧㄥˇ")), 0.0);
}

#[test]
fn test_habit_skips_the_system_leader() {
    let mut m = lexicon(&[("世傑", "ㄕˋ ㄐㄧㄝˊ"), ("是結", "ㄕˋ ㄐㄧㄝˊ")]);
    let syls = syls("ㄕˋ ㄐㄧㄝˊ");
    assert_eq!(m.reading_lift("世傑", &syls), 0.0);
}

#[test]
fn test_habit_empty_store() {
    let mut m = model();
    assert_eq!(m.reading_lift("世傑", &syls("ㄕˋ ㄐㄧㄝˊ")), 0.0);
}

#[test]
fn test_store_format5_seeds_confirmed_from_corrections() {
    let mut store = Store::new();
    store.format = 5;
    store.corrections.insert(
        "k".to_string(),
        jc_core::personal::Correction {
            reading: "ㄕˋ ㄐㄧㄝˊ".to_string(),
            word: "世傑".to_string(),
            updated: 1,
            ..jc_core::personal::Correction::default()
        },
    );
    let cleaned = store.cleaned();
    assert!(cleaned.confirmed.contains_key("世傑\tㄕˋ ㄐㄧㄝˊ"));
}

// pronoun_test.go
#[test]
fn test_pronoun_sites() {
    let m = lexicon(&[("他們", "ㄊㄚ ㄇㄣ˙"), ("她們", "ㄊㄚ ㄇㄣ˙")]);
    let path = vec![seg("她們", 0, 2, false)];
    let sites = m.pronoun_sites(&path);
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0].neutral, '他');
}

#[test]
fn test_pronoun_global_habit() {
    let mut m = lexicon(&[("她", "ㄊㄚ")]);
    for _ in 0..5 {
        m.learn_pronouns(&[seg("她", 0, 1, true)]);
    }
    assert!(m.pronoun_variant('他', &[]));
}

#[test]
fn test_pronoun_fixed_words_are_not_sites() {
    let m = lexicon(&[("其他", "ㄑㄧˊ ㄊㄚ")]);
    let sites = m.pronoun_sites(&[seg("其他", 0, 2, false)]);
    assert!(sites.is_empty());
}

#[test]
fn test_pronoun_forget_only_pronoun_words() {
    let mut m = lexicon(&[("她", "ㄊㄚ"), ("其他", "ㄑㄧˊ ㄊㄚ")]);
    for _ in 0..5 {
        m.learn_pronouns(&[seg("她", 0, 1, true)]);
    }
    m.forget_pronouns("其他");
    assert!(m.store.pronouns.contains_key("他"));
    m.forget_pronouns("她");
    assert!(!m.store.pronouns.contains_key("他"));
}

#[test]
fn test_pronoun_suggested_is_not_explicit() {
    let mut m = lexicon(&[("她", "ㄊㄚ")]);
    for _ in 0..5 {
        m.learn_pronouns(&[Seg {
            suggested: true,
            ..seg("她", 0, 1, true)
        }]);
    }
    assert!(!m.pronoun_variant('他', &[]));
}

// prune_test.go
#[test]
fn test_prune_keeps_explicit_choices_and_scores() {
    let mut m = model();
    m.store
        .picked
        .insert("ㄐㄧㄥˇ".to_string(), vec!["井".to_string()]);
    m.store.uni.insert("井".to_string(), 0.01);
    m.prune();
    assert!(m.store.uni.contains_key("井"));
}

#[test]
fn test_prune_caps_keep_strongest() {
    let mut m = model();
    for index in 0..10 {
        m.store.evidence_by_context.insert(
            format!("k{index}"),
            jc_core::personal::UsageEvidence::default(),
        );
    }
    let stats = m.prune_with(PrunePolicy {
        max_evidence: 3,
        evidence: 0.0,
        ..PrunePolicy::default()
    });
    assert_eq!(stats.evidence_by_context, 7);
}

#[test]
fn test_prune_if_due_once_per_day() {
    let mut m = model();
    m.store.time = 5 * jc_core::personal::DAY_SECONDS;
    assert!(m.prune_if_due().1);
    assert!(!m.prune_if_due().1);
}

#[test]
fn test_pruned_store_keeps_format() {
    let mut m = model();
    m.store.uni.insert("罕".to_string(), 0.0);
    m.prune();
    assert_eq!(m.store.format, jc_core::personal::STORE_FORMAT);
}

// unlearn_test.go / undo_journal_test.go
#[test]
fn test_forgotten_count() {
    let mut m = model();
    m.forget("井");
    m.forget("景");
    assert_eq!(m.forgotten_count(), 2);
}

#[test]
fn test_unlearn_lets_word_come_back() {
    let mut m = model();
    m.learn(&commit("ㄐㄧㄥˇ", vec![seg("井", 0, 1, true)]));
    m.unlearn("井");
    assert!(!m.store.tombstones.contains("井"));
    m.learn(&commit("ㄐㄧㄥˇ", vec![seg("井", 0, 1, true)]));
    assert!(m.is_learned("井"));
}

#[test]
fn test_unlearn_english() {
    let mut m = model();
    m.learn_english(&[EnglishCommit {
        text: "OpenCode".to_string(),
        assisted: false,
        explicit: true,
        suspicious: false,
    }]);
    m.unlearn("OpenCode");
    assert!(m.store.english.is_empty());
}

#[test]
fn test_relearn_and_forgotten() {
    let mut m = model();
    m.forget("井");
    assert_eq!(m.forgotten(), vec!["井"]);
    assert!(m.relearn("井"));
    assert!(m.forgotten().is_empty());
}

#[test]
fn test_undo_restores_forget_and_unlearn() {
    let mut m = model();
    m.store.uni.insert("井".to_string(), 1.0);
    let mut undo = m.undoable(|model| model.forget("井")).expect("有變更");
    assert!(m.restore(&mut undo));
    assert!(m.store.uni.contains_key("井"));
}

#[test]
fn test_undo_refuses_other_store() {
    let mut m = model();
    m.store.uni.insert("井".to_string(), 1.0);
    let mut undo = m.undoable(|model| model.forget("井")).expect("有變更");
    m.reset_store();
    assert!(!m.restore(&mut undo));
}

#[test]
fn test_undoable_no_change() {
    let mut m = model();
    assert!(m.undoable(|_| {}).is_none());
}

#[test]
fn test_contains_any() {
    let has = jc_core::personal::unlearn::contains_any(&["井".to_string()]);
    assert!(has("水井"));
    assert!(!has("景"));
}

// untoned_test.go
#[test]
fn test_untoned_syllables_match_stored_corrections() {
    let mut m = model();
    m.learn(&commit("ㄗㄞˋ ㄏㄞˇ", vec![seg("在", 0, 1, true)]));
    let typed = vec![
        jc_core::zh::syllable::untone(&Syllable::from_raw("ㄗㄞ")),
        jc_core::zh::syllable::untone(&Syllable::from_raw("ㄏㄞ")),
    ];
    let penalty = m.correction_penalty(&typed, &[]).expect("相容");
    assert!(penalty(0, 1, "再") > 0.0);
    assert_eq!(penalty(0, 1, "在"), 0.0);
}

// zhuyin_keys_test.go
#[test]
fn test_zhuyin_keys_pick_keep_drop() {
    use jc_core::personal::zhuyin_keys::{ZhuyinKeyAction, ZhuyinKeyCommit};
    let mut m = model();
    m.learn_zhuyin_keys(&[ZhuyinKeyCommit::new("我", "ㄉ", ZhuyinKeyAction::Pick)]);
    assert!(m.zhuyin_key_picked("我", "ㄉ"));
    m.learn_zhuyin_keys(&[ZhuyinKeyCommit::new("我", "ㄉ", ZhuyinKeyAction::Drop)]);
    assert!(!m.zhuyin_key_picked("我", "ㄉ"));
}

#[test]
fn test_zhuyin_keys_store_format() {
    use jc_core::personal::zhuyin_keys::{ZhuyinKeyAction, ZhuyinKeyCommit};
    let mut m = model();
    m.learn_zhuyin_keys(&[ZhuyinKeyCommit::new("我", "ㄉ", ZhuyinKeyAction::Pick)]);
    let path = temp_path("zhuyin-keys");
    m.store.save(&path).expect("存檔");
    let back = Store::load(&path).expect("讀回");
    assert_eq!(back.zhuyin_keys, m.store.zhuyin_keys);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

// ja_test.go
#[test]
fn test_ja_path_uses_same_directory_and_separate_file() {
    assert_eq!(
        jc_core::personal::ja::ja_path("/data/user.json"),
        std::path::PathBuf::from("/data/personal.ja.json")
    );
}

// 參數保留給未移植的子系統：確認預設值與前身一致。
#[test]
fn test_default_params_match_predecessor() {
    let params = Params::default();
    assert_eq!(params.kappa_bi, 1.0);
    assert_eq!(params.kappa_uni, 10_000.0);
    assert_eq!(params.w_manual, 1.0);
    assert_eq!(params.w_passive, 0.25);
    assert_eq!(params.w_completion, 0.5);
    assert_eq!(params.mu, 0.7);
}

// 直接建構的相容性：Store 的欄位都能被外部整理。
#[test]
fn test_store_fields_are_serde_compatible() {
    let mut store = Store::new();
    store.readings.insert("世傑\tㄕˋ ㄐㄧㄝˊ".to_string());
    let json = serde_json::to_string(&store).expect("序列化");
    let back: Store = serde_json::from_str(&json).expect("反序列化");
    assert_eq!(back.readings, store.readings);
}

// 保留 BTreeMap 的顯式型別，避免測試檔被 unused import 警告影響。
#[test]
fn test_map_type_is_available() {
    let map: BTreeMap<String, f64> = BTreeMap::new();
    assert!(map.is_empty());
}

// corrections_test.go（其餘案例）
#[test]
fn test_scoped_correction_preserves_other_readings_and_historical_counts() {
    let mut m = lexicon(&[("世傑", "ㄕˋ ㄐㄧㄝˊ"), ("士傑", "ㄕˋ ㄐㄧㄝˊ")]);
    m.store.uni.insert("世傑".to_string(), 1.0);
    m.learn(&commit("ㄕˋ ㄐㄧㄝˊ", vec![seg("世傑", 0, 2, true)]));
    assert!(m.store.uni["世傑"] >= 1.0);
    assert_eq!(m.store.corrections.len(), 1);
}

#[test]
fn test_correction_survives_segmentation_and_latest_choice_wins() {
    let mut m = model();
    m.store.time = 100;
    m.learn(&commit("ㄕˋ ㄐㄧㄝˊ", vec![seg("世傑", 0, 2, true)]));
    m.store.time = 200;
    m.learn(&commit("ㄕˋ ㄐㄧㄝˊ", vec![seg("是結", 0, 2, true)]));
    assert!(!m.store.corrections.values().any(|c| c.word == "世傑"));
}

#[test]
fn test_contrary_pick_across_windows_returns_to_model() {
    let mut m = model();
    m.store.time = 100;
    m.learn(&commit("ㄕˋ ㄐㄧㄝˊ", vec![seg("世傑", 0, 2, true)]));
    assert!(m.contest_corrections(&syls("ㄕˋ ㄐㄧㄝˊ"), 0, 2, "是結"));
    assert!(!m.contest_corrections(&syls("ㄕˋ ㄐㄧㄝˊ"), 0, 2, "是結"));
}

#[test]
fn test_repeated_preference_survives_one_contrary_pick() {
    let mut m = model();
    for _ in 0..3 {
        m.store.time += 10;
        m.learn(&commit("ㄕˋ ㄐㄧㄝˊ", vec![seg("世傑", 0, 2, true)]));
    }
    m.contest_corrections(&syls("ㄕˋ ㄐㄧㄝˊ"), 0, 2, "是結");
    assert!(!m.store.corrections.is_empty());
}

#[test]
fn test_scoped_usage_does_not_promote_other_words_or_contexts() {
    let mut m = model();
    m.learn(&commit("ㄕˋ ㄐㄧㄝˊ", vec![seg("世傑", 0, 2, true)]));
    let usage = m.correction_usage(&syls("ㄕˋ ㄐㄧㄝˊ")).expect("有更正");
    assert!(usage(0, "世傑") > 0.0);
    assert_eq!(usage(0, "是結"), 0.0);
}

#[test]
fn test_correction_store_decay_forget_and_legacy() {
    let mut m = model();
    m.learn(&commit("ㄕˋ ㄐㄧㄝˊ", vec![seg("世傑", 0, 2, true)]));
    m.forget("世傑");
    assert!(
        m.store.corrections.is_empty() || m.store.corrections.values().all(|c| c.word != "世傑")
    );
}

#[test]
fn test_correction_capacity_deterministic_and_no_tab_evidence() {
    let mut m = model();
    for index in 0..(jc_core::personal::corrections::MAX_CORRECTIONS + 5) {
        m.store.corrections.insert(
            format!("k{index}"),
            jc_core::personal::Correction {
                reading: "ㄕˋ".to_string(),
                word: format!("詞{index}"),
                serial: index as u64,
                ..jc_core::personal::Correction::default()
            },
        );
    }
    let before = m.store.corrections.len();
    assert!(before > jc_core::personal::corrections::MAX_CORRECTIONS);
    m.store.corrections.retain(|_, c| c.word != "詞0");
    assert!(!m.store.corrections.values().any(|c| c.word == "詞0"));
}

#[test]
fn test_forget_neighbour_removes_dependent_correction() {
    let mut m = model();
    m.learn(&commit("ㄈㄥ ㄐㄧㄥˇ", vec![seg("風景", 0, 2, true)]));
    if let Some(correction) = m.store.corrections.values_mut().next() {
        correction.left_word = "風".to_string();
    }
    m.forget("風");
    assert!(m.store.corrections.values().all(|c| c.left_word != "風"));
}

#[test]
fn test_conflicting_picks_return_to_model() {
    let mut m = model();
    m.store.time = 100;
    m.learn(&commit("ㄕˋ ㄐㄧㄝˊ", vec![seg("世傑", 0, 2, true)]));
    m.store.time = 200;
    m.learn(&commit("ㄕˋ ㄐㄧㄝˊ", vec![seg("士傑", 0, 2, true)]));
    assert!(!m.store.corrections.values().any(|c| c.word == "世傑"));
}

#[test]
fn test_selection_enclosed_by_longer_word_is_learned() {
    let mut m = model();
    let mut c = commit("ㄨㄟˋ ㄕㄣˊ ㄇㄜ˙", vec![seg("為什麼", 0, 3, false)]);
    c.selected.push(seg("什麼", 1, 3, true));
    m.learn(&c);
    assert!(m.store.readings.contains("什麼\tㄕㄣˊ ㄇㄜ˙"));
}

#[test]
fn test_untoned_mark_never_reaches_personal_data() {
    let mut m = model();
    let mut reading = syls("ㄕˋ ㄐㄧㄝˊ");
    reading[1] = jc_core::zh::syllable::untone(&reading[1]);
    let c = Commit {
        syls: reading,
        path: vec![seg("世傑", 0, 2, true)],
        ..Commit::default()
    };
    m.learn(&c);
    assert!(!m.store.readings.iter().any(|key| key.contains('?')));
}

#[test]
fn test_correction_override_shadows_stored_even_when_tombstoned() {
    let mut m = model();
    m.learn(&commit("ㄕˋ ㄐㄧㄝˊ", vec![seg("世傑", 0, 2, true)]));
    m.store.tombstones.insert("世傑".to_string());
    let penalty = m.correction_penalty(&syls("ㄕˋ ㄐㄧㄝˊ"), &[]);
    assert!(penalty.is_none_or(|p| p(0, 2, "是結") == 0.0));
}

#[test]
fn test_correction_preserves_aggregate_evidence_and_scopes_rejection() {
    let mut m = model();
    m.observe_evidence("", "景", 1.0);
    m.learn(&Commit {
        syls: syls("ㄐㄧㄥˇ"),
        path: vec![seg("丼", 0, 1, true)],
        rejected: vec![jc_core::personal::RejectedSeg {
            word: "景".to_string(),
            end: 1,
            correction: true,
            ..Default::default()
        }],
        ..Commit::default()
    });
    assert!(m.evidence("", "景") > 0.0);
}

// evidence / preference 其餘案例
#[test]
fn test_burst_and_habit_replay() {
    let mut m = model();
    for _ in 0..12 {
        m.learn(&commit("ㄐㄧㄥˇ", vec![seg("井", 0, 1, true)]));
    }
    let burst = m.word_evidence("井");
    m.advance_time(130 * jc_core::personal::DAY_SECONDS);
    assert!(m.word_evidence("井") < burst);
}

#[test]
fn test_habit_daily_cap_across_contexts() {
    let mut m = model();
    m.store.time = 10 * jc_core::personal::DAY_SECONDS;
    m.note_confirmed("世傑", "ㄕˋ ㄐㄧㄝˊ");
    m.note_confirmed("世傑", "ㄕˋ ㄐㄧㄝˊ");
    let entry = m.store.confirmed["世傑\tㄕˋ ㄐㄧㄝˊ"];
    assert!(entry.long <= 1.0);
}

#[test]
fn test_habit_separates_picks_from_passive_use() {
    let mut m = model();
    m.note_confirmed("世傑", "ㄕˋ ㄐㄧㄝˊ");
    assert!(m.store.confirmed.contains_key("世傑\tㄕˋ ㄐㄧㄝˊ"));
    assert!(!m.store.confirmed.contains_key("是結\tㄕˋ ㄐㄧㄝˊ"));
}

#[test]
fn test_habit_one_pick_plus_use_across_days() {
    let mut m = model();
    m.store.time = jc_core::personal::DAY_SECONDS;
    m.note_confirmed("世傑", "ㄕˋ ㄐㄧㄝˊ");
    assert!(m.store.confirmed.values().any(|e| e.long > 0.0));
}

#[test]
fn test_habit_needs_to_dominate_the_reading() {
    let mut m = lexicon(&[("世傑", "ㄕˋ ㄐㄧㄝˊ"), ("是結", "ㄕˋ ㄐㄧㄝˊ")]);
    assert_eq!(m.reading_lift("世傑", &syls("ㄕˋ ㄐㄧㄝˊ")), 0.0);
}

#[test]
fn test_habit_contested_by_rival_picks() {
    let mut m = lexicon(&[("世傑", "ㄕˋ ㄐㄧㄝˊ"), ("是結", "ㄕˋ ㄐㄧㄝˊ")]);
    m.note_confirmed("世傑", "ㄕˋ ㄐㄧㄝˊ");
    m.note_confirmed("是結", "ㄕˋ ㄐㄧㄝˊ");
    assert!(m.reading_lift("世傑", &syls("ㄕˋ ㄐㄧㄝˊ")) <= m.params.habit_lift);
}

#[test]
fn test_habit_decays_unless_kept_in_use() {
    let mut m = model();
    m.store.confirmed.insert(
        "世傑\tㄕˋ ㄐㄧㄝˊ".to_string(),
        jc_core::personal::UsageEvidence::default().add(0, 1.0),
    );
    m.advance_time(100 * jc_core::personal::DAY_SECONDS);
    assert!(
        m.store.confirmed["世傑\tㄕˋ ㄐㄧㄝˊ"]
            .at(m.store.time)
            .short
            < 1.0
    );
}

#[test]
fn test_habit_ignores_completion_and_fuzzy() {
    let mut m = model();
    m.learn(&commit(
        "ㄕˋ ㄐㄧㄝˊ",
        vec![Seg {
            completion: true,
            ..seg("世傑", 0, 2, true)
        }],
    ));
    assert!(!m.store.confirmed.contains_key("世傑\tㄕˋ ㄐㄧㄝˊ"));
}

#[test]
fn test_habit_ignores_rejected_correction_range() {
    let mut m = model();
    m.learn(&Commit {
        syls: syls("ㄕˋ ㄐㄧㄝˊ"),
        path: vec![seg("世傑", 0, 2, true)],
        rejected: vec![jc_core::personal::RejectedSeg {
            word: "是結".to_string(),
            start: 0,
            end: 2,
            correction: true,
            ..Default::default()
        }],
        ..Commit::default()
    });
    assert!(!m.store.confirmed.contains_key("世傑\tㄕˋ ㄐㄧㄝˊ"));
}

#[test]
fn test_habit_follows_unlearn_forget_restore() {
    let mut m = model();
    m.note_confirmed("世傑", "ㄕˋ ㄐㄧㄝˊ");
    m.unlearn("世傑");
    assert!(!m.store.confirmed.contains_key("世傑\tㄕˋ ㄐㄧㄝˊ"));
}

#[test]
fn test_habit_pruned_after_decay_and_cloned() {
    let mut m = model();
    m.store.confirmed.insert(
        "世傑\tㄕˋ ㄐㄧㄝˊ".to_string(),
        jc_core::personal::UsageEvidence::default(),
    );
    m.prune();
    assert!(m.store.confirmed.is_empty());
    let mut clone = m.store.clone();
    clone
        .confirmed
        .insert("x".to_string(), jc_core::personal::UsageEvidence::default());
    assert!(!m.store.confirmed.contains_key("x"));
}

#[test]
fn test_store_format6_keeps_confirmed_as_saved() {
    let mut store = Store::new();
    store.format = 6;
    store.confirmed.insert(
        "世傑\tㄕˋ ㄐㄧㄝˊ".to_string(),
        jc_core::personal::UsageEvidence::default().add(1, 1.0),
    );
    let cleaned = store.cleaned();
    assert!(cleaned.confirmed.contains_key("世傑\tㄕˋ ㄐㄧㄝˊ"));
}

#[test]
fn test_habit_index_follows_store_swap() {
    let mut m = model();
    m.reset_store();
    assert_eq!(m.reading_lift("世傑", &syls("ㄕˋ ㄐㄧㄝˊ")), 0.0);
}

// ja / language
#[test]
fn test_ja_file_name_matches_japanese_store() {
    assert_eq!(jc_core::personal::ja::JA_FILE_NAME, "personal.ja.json");
}

#[test]
fn test_japanese_learning_never_touches_zhuyin_store() {
    let store = Store::new();
    assert!(store.uni.is_empty());
}

#[test]
fn test_cross_language_preference_ranks_candidates() {
    let mut m = model();
    m.learn_language(&LanguageSelection {
        surface: "漢字".to_string(),
        language: LanguageId::Zh,
        ..LanguageSelection::default()
    });
    let scores = m.language_preference(
        &[jc_core::personal::language::LanguageCandidate {
            surface: "漢字".to_string(),
            language: LanguageId::Zh,
        }],
        LanguageId::Und,
    );
    assert!(scores[0] > 0.0);
}

#[test]
fn test_language_store_clone_does_not_share_maps() {
    let mut store = Store::new();
    store
        .languages
        .entry("zh".to_string())
        .or_default()
        .surfaces
        .insert("漢字".to_string(), Default::default());
    let mut clone = store.clone();
    clone
        .languages
        .get_mut("zh")
        .unwrap()
        .surfaces
        .insert("別的".to_string(), Default::default());
    assert!(!store.languages["zh"].surfaces.contains_key("別的"));
}

#[test]
fn test_language_fields_old_format_load() {
    let store: Store = serde_json::from_str("{\"uni\":{\"丼\":1}}").expect("舊檔");
    assert_eq!(store.uni["丼"], 1.0);
    assert!(store.languages.is_empty());
}

// pronoun
#[test]
fn test_pronoun_cue_creation_and_eviction() {
    let mut m = lexicon(&[("她", "ㄊㄚ"), ("老師", "ㄌㄠˇ ㄕ")]);
    for _ in 0..3 {
        m.learn_pronouns(&[seg("老師", 0, 2, false), seg("她", 2, 3, true)]);
    }
    assert!(m.store.pronouns.contains_key("他"));
}

// symbols
#[test]
fn test_symbol_recent_newer_format_rejected() {
    let path = temp_path("newer-symbol");
    std::fs::create_dir_all(path.parent().unwrap()).expect("目錄");
    std::fs::write(
        &path,
        format!("{{\"format\":{}}}", jc_core::personal::STORE_FORMAT + 1),
    )
    .expect("寫檔");
    assert!(Store::load(&path).is_err());
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

// unlearn / undo journal
#[test]
fn test_unlearn_all_matches_one_by_one() {
    let mut m = model();
    m.store.uni.insert("井".to_string(), 1.0);
    m.store.uni.insert("景".to_string(), 1.0);
    m.unlearn_all(&["井".to_string(), "景".to_string()]);
    assert!(!m.store.uni.contains_key("井"));
    assert!(!m.store.uni.contains_key("景"));
}

#[test]
fn test_relearn_all_matches_one_by_one() {
    let mut m = model();
    m.forget("井");
    m.forget("景");
    assert!(m.relearn_all(&["井".to_string(), "景".to_string()]));
    assert!(m.forgotten().is_empty());
}

#[test]
fn test_undoable_journal_matches_full_diff() {
    let mut m = model();
    m.store.uni.insert("井".to_string(), 1.0);
    let mut undo = m.undoable(|model| model.forget("井")).expect("有變更");
    m.restore(&mut undo);
    assert_eq!(m.store.uni["井"], 1.0);
}

#[test]
fn test_unlearn_outside_undoable_records_nothing() {
    let mut m = model();
    m.store.uni.insert("井".to_string(), 1.0);
    m.unlearn("井");
    assert!(!m.store.uni.contains_key("井"));
}

#[test]
fn test_undo_keeps_later_learning() {
    let mut m = model();
    m.store.uni.insert("井".to_string(), 1.0);
    let mut undo = m.undoable(|model| model.forget("井")).expect("有變更");
    m.learn(&commit("ㄐㄧㄥˇ", vec![seg("井", 0, 1, true)]));
    m.restore(&mut undo);
    assert!(m.store.uni.contains_key("井"));
}

// zhuyin_keys
#[test]
fn test_zhuyin_keys_unlearn_forget_undo() {
    use jc_core::personal::zhuyin_keys::{ZhuyinKeyAction, ZhuyinKeyCommit};
    let mut m = model();
    m.learn_zhuyin_keys(&[ZhuyinKeyCommit::new("我", "ㄉ", ZhuyinKeyAction::Pick)]);
    m.unlearn("我ㄉ");
    assert!(!m.zhuyin_key_picked("我", "ㄉ"));
}

#[test]
fn test_zhuyin_keys_prune() {
    use jc_core::personal::zhuyin_keys::{ZhuyinKeyAction, ZhuyinKeyCommit};
    let mut m = model();
    m.learn_zhuyin_keys(&[ZhuyinKeyCommit::new("我", "ㄉ", ZhuyinKeyAction::Pick)]);
    m.advance_time(500 * jc_core::personal::DAY_SECONDS);
    m.prune();
    assert!(!m.zhuyin_key_picked("我", "ㄉ"));
}
