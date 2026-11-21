//! 設定、輸入習慣與路徑的測試，從前身 Go 測試搬過來。

use std::fs;
use std::path::{Path, PathBuf};

use jc_core::zh::fuzzy;

use crate::profiles::fresh_settings;
use crate::settings::{
    hidden_specs, language_specs, model_specs, read, save, specs, FileState, SetCode, SetError,
    Settings, SettingsError, SCHEMA_VERSION,
};
use crate::{load, quarantine, ProfileAction, MODEL_RESIDENCY_DUAL_MB, MODEL_RESIDENCY_SINGLE_MB};

fn tmpdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("jc-settings-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("建立測試目錄");
    dir
}

fn write_settings(path: &Path, body: &str) {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).expect("建立目錄");
    }
    fs::write(path, body).expect("寫入設定檔");
}

fn read_text(path: &Path) -> String {
    fs::read_to_string(path).expect("讀回設定檔")
}

#[test]
fn test_defaults_missing_fields_and_round_trip() {
    let dir = tmpdir("defaults");
    let path = dir.join("settings.json");
    let (settings, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(settings, fresh_settings("linux"));
    assert_eq!(settings.selection, crate::NEURAL);
    assert!(!settings.punctuation);
    assert!(settings.document_context);

    write_settings(&path, r#"{"selection":"statistical"}"#);
    let (settings, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(settings.selection, crate::STATISTICAL);
    assert!(!settings.punctuation);
    assert!(settings.document_context);

    let mut settings = settings;
    settings.set("punctuation", "on", "linux").expect("設定");
    save(&path, &settings).expect("存檔");
    let (back, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(back, settings);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_legacy_selection_migrates_to_neural_mode() {
    let dir = tmpdir("legacy-selection");
    let path = dir.join("settings.json");

    write_settings(&path, r#"{"selection":"statistical"}"#);
    let report = read(&path, "linux");
    assert_eq!(report.settings.ja_neural_mode, crate::JA_NEURAL_OFF);
    assert_eq!(report.settings.selection, crate::STATISTICAL);
    assert!(report.notes.is_empty(), "{:?}", report.notes);

    write_settings(&path, r#"{"selection":"neural"}"#);
    let report = read(&path, "linux");
    assert_eq!(report.settings.ja_neural_mode, crate::JA_NEURAL_AUTO);
    assert_eq!(report.settings.selection, crate::NEURAL);

    // 設定舊鍵時也要推出新欄位。
    let mut settings = Settings::default();
    settings
        .set("selection", crate::STATISTICAL, "linux")
        .expect("設定");
    assert_eq!(settings.ja_neural_mode, crate::JA_NEURAL_OFF);
    settings
        .set("selection", crate::NEURAL, "linux")
        .expect("設定");
    assert_eq!(settings.ja_neural_mode, crate::JA_NEURAL_AUTO);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_selection_and_mode_use_mode_with_one_warning() {
    let dir = tmpdir("selection-mode");
    let path = dir.join("settings.json");

    write_settings(
        &path,
        r#"{"selection":"statistical","ja_neural_mode":"quality","punctuation":true}"#,
    );
    let report = read(&path, "linux");
    assert_eq!(
        report.settings.ja_neural_mode,
        crate::JA_NEURAL_QUALITY,
        "兩者都在時以 ja_neural_mode 為準"
    );
    assert_eq!(report.settings.selection, crate::NEURAL);
    assert_eq!(report.notes.len(), 1, "{:?}", report.notes);
    assert!(
        report.notes[0].contains("ja-neural-mode"),
        "{:?}",
        report.notes
    );
    assert!(report.notes[0].contains("selection"), "{:?}", report.notes);

    // 存檔後只留一致的投影，下次讀取不再警告。
    save(&path, &report.settings).expect("存檔");
    let back = read(&path, "linux");
    assert_eq!(back.settings.ja_neural_mode, crate::JA_NEURAL_QUALITY);
    assert_eq!(back.settings.selection, crate::NEURAL);
    assert!(back.notes.is_empty(), "{:?}", back.notes);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_ja_neural_mode_and_rerank_persist() {
    let dir = tmpdir("ja-neural");
    let path = dir.join("settings.json");
    let mut settings = Settings::default();
    assert_eq!(settings.value("ja-neural-mode").as_deref(), Some("auto"));
    assert_eq!(settings.value("ja-rerank").as_deref(), Some("off"));
    settings
        .set("ja-neural-mode", crate::JA_NEURAL_QUALITY, "linux")
        .expect("設定");
    settings.set("ja-rerank", "on", "linux").expect("設定");
    save(&path, &settings).expect("存檔");
    let text = read_text(&path);
    assert!(text.contains("\"ja_neural_mode\": \"quality\""), "{text}");
    assert!(text.contains("\"ja_rerank\": true"), "{text}");
    let (back, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(back.ja_neural_mode, crate::JA_NEURAL_QUALITY);
    assert!(back.ja_rerank);
    assert_eq!(back, settings, "來回不一致");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_punctuation_keeps_explicit_preference() {
    for (name, body, enabled) in [
        ("off", r#"{"schema":1,"punctuation":false}"#, false),
        ("on", r#"{"schema":1,"punctuation":true}"#, true),
    ] {
        let dir = tmpdir(&format!("punctuation-{name}"));
        let path = dir.join("settings.json");
        write_settings(&path, body);
        let (mut settings, error) = load(&path, "linux");
        assert!(error.is_none(), "{name}: {error:?}");
        assert_eq!(settings.punctuation, enabled, "{name}");
        assert_eq!(read_text(&path), body, "{name}：讀取不得改寫既有設定");
        settings
            .set("selection", crate::STATISTICAL, "linux")
            .expect("設定");
        save(&path, &settings).expect("存檔");
        let (back, error) = load(&path, "linux");
        assert!(error.is_none(), "{name}: {error:?}");
        assert_eq!(back.punctuation, enabled, "{name}");
        assert_eq!(back.selection, crate::STATISTICAL, "{name}");
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn test_unknown_selection_only_resets_that_field() {
    let dir = tmpdir("unknown-selection");
    let path = dir.join("settings.json");
    write_settings(
        &path,
        r#"{"selection":"remote","punctuation":false,"document_context":false}"#,
    );
    let report = read(&path, "linux");
    assert_eq!(report.state, FileState::Ok);
    assert!(report.error.is_none());
    assert_eq!(report.settings.selection, crate::NEURAL);
    assert!(!report.settings.document_context);
    assert_eq!(report.notes.len(), 1);
    assert!(report.notes[0].contains("selection"), "{:?}", report.notes);
    assert!(report.notes[0].contains("\"remote\""), "{:?}", report.notes);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_rejects_unknown_values() {
    let mut settings = Settings::default();
    for (key, value) in [("selection", "x"), ("punctuation", "yes"), ("nope", "on")] {
        let err = settings.set(key, value, "linux").expect_err("應拒絕");
        assert!(matches!(err, SettingsError::Set(_)), "{err:?}");
    }
    assert_eq!(settings, Settings::default());
}

#[test]
fn test_set_errors_carry_machine_readable_codes() {
    let mut settings = Settings::default();
    for (key, value, code) in [
        ("nope", "on", SetCode::UnknownKey),
        ("fuzzy-", "select", SetCode::UnknownKey),
        ("selection", "x", SetCode::InvalidValue),
        ("punctuation", "yes", SetCode::InvalidValue),
        ("fuzzy-z-zh", "learn", SetCode::InvalidValue),
    ] {
        let err = settings.set(key, value, "linux").expect_err("應拒絕");
        let SettingsError::Set(inner) = err else {
            panic!("應是設定錯誤：{key}={value}");
        };
        assert_eq!(inner.code(), code, "{key}={value}");
        assert_eq!(inner.key(), key, "{key}={value}");
        assert!(matches!(
            inner,
            SetError::UnknownKey { .. } | SetError::InvalidValue { .. }
        ));
    }
    assert_eq!(settings, Settings::default());
}

#[test]
fn test_fuzzy_defaults_select_and_leaves_file_untouched() {
    let dir = tmpdir("fuzzy-defaults");
    let path = dir.join("settings.json");
    write_settings(
        &path,
        r#"{"selection":"neural","punctuation":true,"document_context":true}"#,
    );
    let (settings, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(
        settings.fuzzy_config().key(),
        "z-zh=select,c-ch=select,s-sh=select,en-eng=select"
    );
    assert_eq!(Settings::default().fuzzy_config(), settings.fuzzy_config());
    save(&path, &settings).expect("存檔");
    assert!(
        !read_text(&path).contains("fuzzy"),
        "維持預設時不應寫入欄位：{}",
        read_text(&path)
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_fuzzy_set_load_and_round_trip() {
    let dir = tmpdir("fuzzy-round-trip");
    let path = dir.join("settings.json");
    let mut settings = Settings::default();
    for pair in fuzzy::pairs() {
        settings
            .set(&format!("fuzzy-{}", pair.id()), "select", "linux")
            .expect("設定");
    }
    settings.set("fuzzy-s-sh", "off", "linux").expect("設定");
    save(&path, &settings).expect("存檔");
    let (back, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(back, settings);
    let config = back.fuzzy_config();
    assert_eq!(config.mode(fuzzy::Pair::ZZh), fuzzy::Mode::Select);
    assert_eq!(config.mode(fuzzy::Pair::CCh), fuzzy::Mode::Select);
    assert_eq!(config.mode(fuzzy::Pair::SSh), fuzzy::Mode::Off);
    assert_eq!(config.mode(fuzzy::Pair::EnEng), fuzzy::Mode::Select);
    let text = read_text(&path);
    assert!(text.contains("\"s-sh\": \"off\""), "{text}");
    assert!(!text.contains("z-zh"), "{text}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_fuzzy_unknown_value_only_disables_that_pair() {
    let dir = tmpdir("fuzzy-unknown");
    let path = dir.join("settings.json");
    write_settings(
        &path,
        r#"{"selection":"statistical","fuzzy":{"z-zh":"learn","c-ch":"select"}}"#,
    );
    let (mut settings, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(settings.selection, crate::STATISTICAL);
    let config = settings.fuzzy_config();
    assert_eq!(
        config.mode(fuzzy::Pair::ZZh),
        fuzzy::Mode::Off,
        "{}",
        config.key()
    );
    assert_eq!(config.mode(fuzzy::Pair::CCh), fuzzy::Mode::Select);
    settings.set("punctuation", "off", "linux").expect("設定");
    save(&path, &settings).expect("存檔");
    assert!(
        read_text(&path).contains("\"learn\""),
        "{}",
        read_text(&path)
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_fuzzy_tolerant_of_non_string_values() {
    let dir = tmpdir("fuzzy-non-string");
    let path = dir.join("settings.json");
    write_settings(
        &path,
        r#"{"selection":"statistical","punctuation":false,"fuzzy":{"z-zh":{"mode":"auto"},"s-sh":true,"c-ch":"select"}}"#,
    );
    let (settings, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(settings.selection, crate::STATISTICAL);
    assert_eq!(settings.fuzzy_config().key(), "c-ch=select,en-eng=select");
    save(&path, &settings).expect("存檔");
    let text = read_text(&path);
    assert!(
        text.contains("\"mode\": \"auto\"") || text.contains("\"mode\":\"auto\""),
        "{text}"
    );
    assert!(text.contains("true"), "{text}");

    write_settings(&path, r#"{"selection":"statistical","fuzzy":"later"}"#);
    let (mut settings, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(settings.selection, crate::STATISTICAL);
    assert!(!settings.fuzzy_config().enabled());
    settings.set("punctuation", "off", "linux").expect("設定");
    save(&path, &settings).expect("存檔");
    assert!(
        read_text(&path).contains("\"fuzzy\": \"later\""),
        "{}",
        read_text(&path)
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_fuzzy_keeps_unknown_pairs_and_edits_only_the_named_one() {
    let dir = tmpdir("fuzzy-unknown-pair");
    let path = dir.join("settings.json");
    write_settings(
        &path,
        r#"{"fuzzy":{"an-ang":"select","z-zh":{"mode":"auto"}}}"#,
    );
    let (mut settings, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(
        settings.fuzzy_config().key(),
        "c-ch=select,s-sh=select,en-eng=select"
    );
    settings.set("fuzzy-c-ch", "auto", "linux").expect("設定");
    save(&path, &settings).expect("存檔");
    let text = read_text(&path);
    for want in [
        "\"an-ang\": \"select\"",
        "\"mode\": \"auto\"",
        "\"c-ch\": \"auto\"",
    ] {
        assert!(text.contains(want), "存檔應包含 {want}：{text}");
    }
    let (mut back, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    back.set("fuzzy-z-zh", "off", "linux").expect("設定");
    save(&path, &back).expect("存檔");
    let text = read_text(&path);
    assert!(!text.contains("\"mode\""), "只該丟掉它自己的舊原文：{text}");
    assert!(text.contains("\"an-ang\""), "{text}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_fuzzy_odd_string_value_round_trips() {
    let dir = tmpdir("fuzzy-odd");
    let path = dir.join("settings.json");
    write_settings(&path, r#"{"fuzzy":{"z-zh":"\u0000raw:{bad"}}"#);
    let (settings, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(
        settings.fuzzy_config().mode(fuzzy::Pair::ZZh),
        fuzzy::Mode::Off
    );
    save(&path, &settings).expect("存檔不得失敗");
    let (back, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(back, settings);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_fuzzy_rejects_bad_settings() {
    let mut settings = Settings::default();
    for (key, value) in [
        ("fuzzy-z-zh", "learn"),
        ("fuzzy-z-zh", "on"),
        ("fuzzy-f-h", "select"),
        ("fuzzy-", "select"),
    ] {
        assert!(
            settings.set(key, value, "linux").is_err(),
            "應拒絕 {key}={value}"
        );
    }
    assert_eq!(settings, Settings::default());
    assert!(crate::keys().contains_key("fuzzy-en-eng"));
}

#[test]
fn test_fuzzy_auto_round_trips() {
    let dir = tmpdir("fuzzy-auto");
    let path = dir.join("settings.json");
    let mut settings = Settings::default();
    settings.set("fuzzy-z-zh", "auto", "linux").expect("設定");
    settings.set("fuzzy-c-ch", "select", "linux").expect("設定");
    save(&path, &settings).expect("存檔");
    let (mut back, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    let config = back.fuzzy_config();
    assert_eq!(config.mode(fuzzy::Pair::ZZh), fuzzy::Mode::Auto);
    assert_eq!(config.mode(fuzzy::Pair::CCh), fuzzy::Mode::Select);
    assert!(config.auto_enabled());
    back.set("fuzzy-z-zh", "off", "linux").expect("設定");
    save(&path, &back).expect("存檔");
    let text = read_text(&path);
    assert!(!text.contains("\"z-zh\": \"auto\""), "{text}");
    assert!(text.contains("\"z-zh\": \"off\""), "{text}");
    back.set("fuzzy-z-zh", "select", "linux").expect("設定");
    save(&path, &back).expect("存檔");
    assert!(!read_text(&path).contains("z-zh"), "{}", read_text(&path));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_future_fields_and_values_survive_save() {
    let dir = tmpdir("future-fields");
    let path = dir.join("settings.json");
    write_settings(
        &path,
        r#"{"schema":1,"selection":"remote","punctuation":false,"sync":{"enabled":true},"zeta":[1,2]}"#,
    );
    let (mut settings, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(settings.selection, crate::NEURAL);
    settings
        .set("document-context", "off", "linux")
        .expect("設定");
    save(&path, &settings).expect("存檔");
    let text = read_text(&path);
    for want in [
        "\"selection\": \"remote\"",
        "\"punctuation\": false",
        "\"document_context\": false",
        "\"sync\"",
        "\"enabled\": true",
        "\"zeta\"",
        "\"schema\": 2",
    ] {
        assert!(text.contains(want), "存檔應包含 {want}：{text}");
    }
    let (back, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(back, settings);
    settings
        .set("selection", crate::STATISTICAL, "linux")
        .expect("設定");
    save(&path, &settings).expect("存檔");
    let text = read_text(&path);
    assert!(!text.contains("remote"), "{text}");
    assert!(text.contains("\"selection\": \"statistical\""), "{text}");
    assert!(text.contains("\"sync\""), "{text}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_unusable_bool_value_is_kept_until_set() {
    let dir = tmpdir("bad-bool");
    let path = dir.join("settings.json");
    write_settings(
        &path,
        r#"{"punctuation":"maybe","selection":"statistical"}"#,
    );
    let (mut settings, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert!(!settings.punctuation);
    assert_eq!(settings.selection, crate::STATISTICAL);
    save(&path, &settings).expect("存檔");
    assert!(read_text(&path).contains("\"punctuation\": \"maybe\""));
    settings.set("punctuation", "off", "linux").expect("設定");
    save(&path, &settings).expect("存檔");
    let text = read_text(&path);
    assert!(!text.contains("maybe"), "{text}");
    assert!(text.contains("\"punctuation\": false"), "{text}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_newer_schema_is_read_only() {
    for body in [
        r#"{"schema":3,"selection":"statistical","future":"x"}"#,
        r#"{"schema":"v2","selection":"statistical"}"#,
    ] {
        let dir = tmpdir("newer-schema");
        let path = dir.join("settings.json");
        write_settings(&path, body);
        let report = read(&path, "linux");
        assert!(report.error.is_none(), "{body}");
        assert!(report.read_only, "{body}");
        assert!(report.settings.read_only(), "{body}");
        assert_eq!(report.settings.selection, crate::STATISTICAL, "{body}");
        let mut settings = report.settings;
        assert!(matches!(
            settings.set("punctuation", "off", "linux"),
            Err(SettingsError::ReadOnly)
        ));
        assert!(matches!(
            save(&path, &settings),
            Err(SettingsError::ReadOnly)
        ));
        assert_eq!(read_text(&path), body);
        let _ = fs::remove_dir_all(&dir);
    }
    for body in [r#"{"schema":1}"#, r#"{"schema":0}"#, "{}"] {
        let dir = tmpdir("writable-schema");
        let path = dir.join("settings.json");
        write_settings(&path, body);
        let report = read(&path, "linux");
        assert!(report.error.is_none(), "{body}");
        assert!(!report.read_only, "{body}");
        save(&path, &report.settings).expect(body);
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn test_save_writes_schema_first_in_stable_order() {
    let dir = tmpdir("save-order");
    let path = dir.join("settings.json");
    save(&path, &Settings::default()).expect("存檔");
    let text = read_text(&path);
    let mut last = None;
    for key in [
        "\"schema\": 2",
        "\"selection\"",
        "\"punctuation\"",
        "\"document_context\"",
    ] {
        let at = text
            .find(key)
            .unwrap_or_else(|| panic!("缺少 {key}：{text}"));
        assert!(last.is_none_or(|last| at > last), "{key} 順序不對：{text}");
        last = Some(at);
    }
    let report = read(&path, "linux");
    assert!(report.error.is_none());
    assert_eq!(report.schema, SCHEMA_VERSION);
    assert!(report.notes.is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_corrupt_file_is_reported_and_can_be_quarantined() {
    let dir = tmpdir("corrupt");
    for body in ["not json", r#"["selection"]"#, r#"{"selection":"#] {
        let path = dir.join("settings.json");
        write_settings(&path, body);
        let report = read(&path, "linux");
        assert!(
            matches!(report.error, Some(SettingsError::Corrupt { .. })),
            "{body}"
        );
        assert_eq!(report.state, FileState::Corrupt);
        assert_eq!(report.settings, Settings::default());
        let (settings, error) = load(&path, "linux");
        assert!(error.is_some());
        assert_eq!(settings, Settings::default());
        let moved = quarantine(&path, "20261006T010203Z").expect("改名");
        assert!(moved
            .to_string_lossy()
            .ends_with(".corrupt-20261006T010203Z"));
        assert_eq!(read_text(&moved), body);
        assert_eq!(read(&path, "linux").state, FileState::Missing);
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_keys_list_every_value_with_default() {
    assert_eq!(specs()[0].key, "selection");
    let settings = Settings::default();
    for spec in specs()
        .into_iter()
        .chain(language_specs())
        .chain(model_specs())
        .chain(hidden_specs())
    {
        let value = settings
            .value(&spec.key)
            .unwrap_or_else(|| panic!("{} 沒有值", spec.key));
        assert!(!value.is_empty(), "{} 的值是空的", spec.key);
        assert!(
            spec.values.contains(&value.as_str()),
            "{}={value} 不在允許值 {:?}",
            spec.key,
            spec.values
        );
    }
    assert!(settings.value("nope").is_none());
}

#[test]
fn test_model_settings_defaults() {
    let settings = Settings::default();
    assert_eq!(settings.model_mode, crate::MODEL_MODE_AUTO);
    assert_eq!(settings.primary_language, crate::LANGUAGE_ZH);
    assert_eq!(settings.model_residency_mb, crate::MODEL_RESIDENCY_AUTO_MB);
    assert!(!settings.allow_placeholders);
    assert_eq!(settings.model_budget_mb(false), MODEL_RESIDENCY_SINGLE_MB);
    assert_eq!(settings.model_budget_mb(true), MODEL_RESIDENCY_DUAL_MB);
    assert_eq!(
        settings.model_budget_bytes(false),
        MODEL_RESIDENCY_SINGLE_MB << 20
    );
    assert_eq!(settings.model_budget_source(false), "phase2-single");
    assert_eq!(settings.model_budget_source(true), "phase2-dual");
}

#[test]
fn test_old_settings_file_migrates_model_fields() {
    let dir = tmpdir("model-migration");
    let path = dir.join("settings.json");
    write_settings(
        &path,
        r#"{"schema":2,"selection":"statistical","language":"ja","future":"x"}"#,
    );
    let report = read(&path, "linux");
    assert!(report.error.is_none(), "{:?}", report.error);
    assert!(!report.read_only);
    let settings = report.settings;
    assert_eq!(settings.model_mode, crate::MODEL_MODE_AUTO);
    assert_eq!(settings.model_residency_mb, 0);
    assert!(!settings.allow_placeholders);
    assert_eq!(settings.primary_language, crate::LANGUAGE_JA);
    save(&path, &settings).expect("存檔");
    let text = read_text(&path);
    for want in [
        "\"model_mode\": \"auto\"",
        "\"primary_language\": \"ja\"",
        "\"model_residency_mb\": 0",
        "\"allow_placeholders\": false",
        "\"future\": \"x\"",
    ] {
        assert!(text.contains(want), "存檔應包含 {want}：{text}");
    }
    assert!(
        !text.contains("\"language\":"),
        "舊的 language 應移除：{text}"
    );
    assert_eq!(report.schema, SCHEMA_VERSION);
    let (back, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(back, settings);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_model_settings_persist_through_set_and_save() {
    let dir = tmpdir("model-persist");
    let path = dir.join("settings.json");
    let mut settings = Settings::default();
    for (key, value) in [
        ("model-mode", crate::MODEL_MODE_PRIMARY),
        ("primary-language", crate::LANGUAGE_JA),
        ("model-residency-mb", "400"),
        ("allow-placeholders", "on"),
    ] {
        settings.set(key, value, "linux").expect(key);
    }
    save(&path, &settings).expect("存檔");
    let (mut back, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(back.model_mode, crate::MODEL_MODE_PRIMARY);
    assert_eq!(back.primary_language, crate::LANGUAGE_JA);
    assert_eq!(back.model_residency_mb, 400);
    assert!(back.allow_placeholders);
    assert_eq!(back.model_budget_mb(false), 400);
    assert_eq!(back.model_budget_source(false), "configured");
    assert_eq!(back.value("model-residency-mb").as_deref(), Some("400"));
    back.set("model-residency-mb", "auto", "linux")
        .expect("設定");
    assert_eq!(back.model_residency_mb, 0);
    assert_eq!(back.model_budget_mb(true), MODEL_RESIDENCY_DUAL_MB);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_unusable_model_residency_value_is_kept_until_set() {
    for raw in ["9999", "-1", "\"many\""] {
        let dir = tmpdir("model-residency-raw");
        let path = dir.join("settings.json");
        let body = format!(
            r#"{{"schema":2,"model_residency_mb":{raw},"model_mode":"sometimes","primary_language":"ko","allow_placeholders":"maybe","model_set":"private"}}"#
        );
        write_settings(&path, &body);
        let report = read(&path, "linux");
        assert!(report.error.is_none(), "{raw}: {:?}", report.error);
        let mut settings = report.settings;
        assert_eq!(settings.model_residency_mb, 0, "{raw}");
        assert_eq!(settings.model_mode, crate::MODEL_MODE_AUTO, "{raw}");
        assert_eq!(settings.primary_language, crate::LANGUAGE_ZH, "{raw}");
        assert!(!settings.allow_placeholders, "{raw}");
        assert!(!report.notes.is_empty(), "{raw}");
        save(&path, &settings).expect("存檔");
        let text = read_text(&path);
        for want in [
            format!("\"model_residency_mb\": {raw}"),
            "\"model_mode\": \"sometimes\"".to_string(),
            "\"primary_language\": \"ko\"".to_string(),
            "\"allow_placeholders\": \"maybe\"".to_string(),
            "\"model_set\": \"private\"".to_string(),
        ] {
            assert!(text.contains(&want), "{raw}：應保留 {want}：{text}");
        }
        settings
            .set("model-residency-mb", "250", "linux")
            .expect("設定");
        save(&path, &settings).expect("存檔");
        let text = read_text(&path);
        assert!(
            !text.contains(&format!("\"model_residency_mb\": {raw}")),
            "{raw}: {text}"
        );
        assert!(
            text.contains("\"model_residency_mb\": 250"),
            "{raw}: {text}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn test_fresh_windows_settings_enable_mixed_input() {
    let settings = fresh_settings("windows");
    assert!(settings.mixed_input, "全新安裝的 Windows 設定要開混打");
    let active = settings.active_profile("windows");
    assert_eq!(
        active.values.get("mixed-input").map(String::as_str),
        Some("on")
    );
    assert_eq!(settings.value("mixed-input").as_deref(), Some("on"));

    let dir = tmpdir("windows-mixed");
    let path = dir.join("settings.json");
    save(&path, &settings).expect("存檔");
    let report = read(&path, "windows");
    assert!(
        report.settings.mixed_input,
        "重新讀取不能被風格快照蓋回 off"
    );
    assert_eq!(report.settings, settings);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_profiles_migration_and_complete_round_trip() {
    let dir = tmpdir("profile-migration");
    let path = dir.join("settings.json");
    let original = r#"{"schema":1,"toggle_key":"left","mixed_input":true,"punctuation":true,"keyboard_layout":"standard","mode_memory":"shared","future_field":{"x":1}}"#;
    write_settings(&path, original);
    let report = read(&path, "linux");
    assert!(report.error.is_none(), "{:?}", report.error);
    let mut settings = report.settings;
    assert!(
        !settings.active_profile("linux").builtin,
        "舊設定應凍結成 legacy"
    );
    assert!(settings.mixed_input);
    assert!(settings.punctuation);
    assert_eq!(settings.toggle_key, "left");
    assert_eq!(read_text(&path), original, "讀取不得改寫設定");

    for id in ["microsoft", "chewing", "mcbopomofo", "macos", "0c"] {
        settings
            .profile_action(ProfileAction::Select, id, "", "linux")
            .unwrap_or_else(|err| panic!("{id}: {err}"));
        assert!(settings.active_profile("linux").builtin, "{id}");
        assert!(settings.punctuation, "{id}");
        assert_eq!(settings.mode_memory, "shared", "{id}");
        assert_eq!(settings.keyboard_layout, "standard", "{id}");
        assert!(
            settings.set("toggle-key", "right", "linux").is_err(),
            "{id}：內建風格不應可改"
        );
        save(&path, &settings).expect("存檔");
        let (back, error) = load(&path, "linux");
        assert!(error.is_none(), "{id}: {error:?}");
        assert_eq!(back, settings, "{id} 來回不一致");
    }
    settings
        .profile_action(ProfileAction::Select, "legacy", "", "linux")
        .expect("選回舊設定");
    assert!(settings.mixed_input);
    assert_eq!(settings.toggle_key, "left");
    assert!(settings.punctuation);
    save(&path, &settings).expect("存檔");
    assert!(read_text(&path).contains("future_field"), "未知欄位要保留");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_profile_copies_rules_and_keeps_custom_changes() {
    let mut settings = fresh_settings("linux");
    assert_eq!(settings.profiles("linux").len(), 5, "全新安裝只有五個內建");
    settings
        .profile_action(ProfileAction::Copy, "microsoft", " 我的習慣 ", "linux")
        .expect("複製");
    let id = settings.active_profile("linux").id;
    assert_eq!(settings.active_profile("linux").name, "我的習慣");
    assert_eq!(settings.profile_rules("linux").shift_space, "fullwidth");
    settings
        .set("style-shift-space", "space", "linux")
        .expect("設定");
    settings.set("esc-clear", "off", "linux").expect("設定");
    let fingerprint = settings.profile_fingerprint("linux");
    settings
        .profile_action(ProfileAction::Select, "mcbopomofo", "", "linux")
        .expect("選用");
    settings
        .profile_action(ProfileAction::Select, &id, "", "linux")
        .expect("選回自訂");
    assert_eq!(settings.profile_rules("linux").shift_space, "space");
    assert!(!settings.esc_clear);
    assert_eq!(settings.profile_fingerprint("linux"), fingerprint);
    settings
        .profile_action(ProfileAction::Rename, &id, "工作用", "linux")
        .expect("改名");
    assert_eq!(settings.profile_fingerprint("linux"), fingerprint);

    let before = settings.clone();
    assert!(settings
        .profile_action(ProfileAction::Delete, &id, "", "linux")
        .is_err());
    assert_eq!(settings, before, "使用中的風格不得刪除");
    assert!(settings
        .profile_action(ProfileAction::Rename, &id, "bad\nname", "linux")
        .is_err());
    assert_eq!(settings, before, "不合法的名稱不得改動快照");

    settings
        .profile_action(ProfileAction::Select, "0c", "", "linux")
        .expect("選用內建");
    settings
        .profile_action(ProfileAction::Delete, &id, "", "linux")
        .expect("刪除自訂");
    assert!(!settings.profiles("linux").iter().any(|p| p.id == id));
}

#[test]
fn test_profile_layout_guard_cannot_be_bypassed_by_copy() {
    let mut settings = fresh_settings("linux");
    settings
        .profile_action(ProfileAction::Select, "microsoft", "", "linux")
        .expect("選用");
    let before = settings.clone();
    assert!(settings.set("keyboard-layout", "hsu", "linux").is_err());
    assert_eq!(settings, before, "不相容的排列不得被接受");
    settings
        .profile_action(ProfileAction::Copy, "microsoft", "copy", "linux")
        .expect("複製");
    let before = settings.clone();
    assert!(settings.set("keyboard-layout", "eten", "linux").is_err());
    assert_eq!(settings, before, "複製不得繞過排列限制");
    settings
        .profile_action(ProfileAction::Select, "0c", "", "linux")
        .expect("選用");
    settings
        .set("keyboard-layout", "hsu", "linux")
        .expect("設定");
    let before = settings.clone();
    assert!(settings
        .profile_action(ProfileAction::Select, "microsoft", "", "linux")
        .is_err());
    assert_eq!(settings, before, "選用不得默默改掉排列");
}

#[test]
fn test_unknown_profile_metadata_read_only_and_safe_rules() {
    let settings = fresh_settings("linux");
    let raw = serde_json::to_string(&settings).expect("序列化");
    for (from, to) in [
        (r#""version":1"#, r#""version":99"#),
        (r#""shift_space":"disabled""#, r#""shift_space":"future""#),
        (r#""rules":{"#, r#""rules":{"future":true,"#),
        (r#""builtin":true"#, r#""builtin":false"#),
        (r#""id":"0c""#, r#""id":"unknown""#),
    ] {
        let body = raw.replacen(from, to, 1);
        let mut got: Settings = serde_json::from_str(&body).expect("仍可讀");
        assert!(got.read_only(), "較新的輸入習慣格式應只讀：{to}");
        assert!(
            got.profile_rules("linux").validate().is_ok(),
            "退回的規則必須合法：{to}"
        );
        assert_eq!(got.profile_rules("linux").shift_space, "disabled", "{to}");
        let dir = tmpdir("profile-future");
        let path = dir.join("settings.json");
        assert!(matches!(save(&path, &got), Err(SettingsError::ReadOnly)));
        assert!(got.set("esc-clear", "off", "linux").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn test_profile_locks_all_managed_entry_points() {
    let dir = tmpdir("profile-lock");
    let user = dir.join("user.json");
    let mut settings = fresh_settings("linux");
    for spec in specs().into_iter().chain(model_specs()) {
        let value = settings.value(&spec.key).expect("有值");
        if value.is_empty() {
            continue;
        }
        let before = settings.clone();
        if let Err(SettingsError::Set(err)) = settings.set(&spec.key, &value, "linux") {
            if err.code() == SetCode::LockedByProfile {
                assert_eq!(settings, before, "{} 不得改動", spec.key);
            }
        }
    }
    save(&crate::settings_path_for_store(&user), &settings).expect("存檔");
    assert!(
        crate::change(&user, &[("left-shift", "on")], "linux").is_err(),
        "舊鍵不得繞過風格鎖"
    );
    assert!(
        crate::change(&user, &[("output-simplified", "on")], "linux").expect("舊鍵應可用"),
        "輸出簡體應寫入"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_custom_rules_cannot_bypass_compatibility() {
    let mut settings = Settings::default();
    settings
        .profile_action(ProfileAction::Copy, "0c", "custom", "linux")
        .expect("複製");
    settings
        .set("style-punctuation", "microsoft", "linux")
        .expect("設定");
    for (key, value) in [("keyboard-layout", "hsu"), ("toneless", "on")] {
        let before = settings.clone();
        assert!(
            settings.set(key, value, "linux").is_err(),
            "不相容的組合不得被接受：{key}={value}"
        );
        assert_eq!(settings, before, "{key}");
    }
    let mut settings = Settings::default();
    settings.set("mixed-input", "on", "linux").expect("設定");
    let before = settings.clone();
    assert!(settings
        .set("style-shift-space", "fullwidth", "linux")
        .is_err());
    assert_eq!(settings, before, "反向操作也要擋");
}

#[test]
fn test_linux_settings_fallback_then_save_to_new_path() {
    let dir = tmpdir("linux-fallback");
    let share = dir.join(".local/share");
    let path = share.join("0c/settings.json");
    let old = share.join("zuin/settings.json");
    let user = share.join("zuin/user.json");
    assert_eq!(crate::settings_path_for_store(&user), path);

    let body = r#"{"schema":1,"selection":"statistical","punctuation":true,"future":{"x":1},"candidate_layout":"future-layout"}"#;
    write_settings(&old, body);
    let report = read(&path, "linux");
    assert!(report.error.is_none(), "{:?}", report.error);
    assert_eq!(report.state, FileState::Ok);
    assert_eq!(report.path, old);
    assert_eq!(report.settings.selection, crate::STATISTICAL);
    assert!(report.settings.punctuation);
    assert!(!share.join("0c").exists(), "讀取不得建立新目錄");

    let mut settings = report.settings;
    settings
        .set("document-context", "off", "linux")
        .expect("設定");
    save(&path, &settings).expect("存檔");
    assert_eq!(read_text(&old), body, "舊檔不得被改寫");
    let text = read_text(&path);
    assert!(text.contains("future-layout"), "{text}");
    assert!(text.contains("\"x\": 1"), "{text}");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path).expect("檔案").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "設定檔權限");
        let mode = fs::metadata(path.parent().expect("目錄"))
            .expect("目錄")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700, "目錄權限");
    }

    write_settings(
        &old,
        r#"{"selection":"neural","document_context":true,"punctuation":false}"#,
    );
    let report = read(&path, "linux");
    assert_eq!(report.path, path, "新檔要優先");
    assert_eq!(report.settings.selection, crate::STATISTICAL);
    assert!(!report.settings.document_context);
    assert!(report.settings.punctuation);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_linux_settings_fresh_install_and_custom_paths() {
    let dir = tmpdir("linux-fresh");
    let path = dir.join("settings.json");
    let report = read(&path, "linux");
    assert!(report.error.is_none());
    assert_eq!(report.path, path);
    assert_eq!(report.state, FileState::Missing);
    assert_eq!(report.settings, fresh_settings("linux"));
    save(&path, &report.settings).expect("存檔");
    assert_eq!(read(&path, "linux").state, FileState::Ok);

    for custom in [dir.join("a"), dir.join("b/zuin"), dir.join("c/share/zuin")] {
        let want = custom.join("settings.json");
        assert_eq!(
            crate::settings_path_for_store(&custom.join("user.json")),
            want
        );
    }
    let custom = path.with_file_name("custom.json");
    write_settings(&dir.join("zuin/custom.json"), "{}");
    let report = read(&custom, "linux");
    assert_eq!(report.state, FileState::Missing, "自訂檔名不得退回舊路徑");
    assert_eq!(report.path, custom);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_linux_settings_existing_new_file_never_falls_back() {
    let dir = tmpdir("linux-existing");
    let share = dir.join(".local/share");
    let path = share.join("0c/settings.json");
    let old = share.join("zuin/settings.json");
    for kind in ["corrupt", "unreadable", "newer", "valid"] {
        let _ = fs::remove_dir_all(&share);
        write_settings(&old, r#"{"selection":"statistical","punctuation":true}"#);
        let mut expected = FileState::Ok;
        match kind {
            "unreadable" => {
                fs::create_dir_all(&path).expect("建立目錄");
                expected = FileState::Unreadable;
            }
            "corrupt" => {
                write_settings(&path, "{broken");
                expected = FileState::Corrupt;
            }
            "newer" => write_settings(&path, r#"{"schema":9,"selection":"neural"}"#),
            _ => write_settings(&path, "{}"),
        }
        let report = read(&path, "linux");
        assert_eq!(report.path, path, "{kind}");
        assert_eq!(report.state, expected, "{kind}");
        assert_eq!(report.settings.selection, crate::NEURAL, "{kind}");
        assert!(!report.settings.punctuation, "{kind}");
        let failed = report.error.is_some();
        assert_eq!(failed, kind == "corrupt" || kind == "unreadable", "{kind}");
        assert_eq!(report.read_only, kind == "newer", "{kind}");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_linux_settings_legacy_errors_and_write_failures_preserve_files() {
    let dir = tmpdir("linux-legacy-errors");
    let share = dir.join(".local/share");
    let path = share.join("0c/settings.json");
    let old = share.join("zuin/settings.json");
    for kind in ["corrupt", "unreadable", "newer", "write-failure"] {
        let _ = fs::remove_dir_all(&share);
        let mut body = r#"{"selection":"statistical","punctuation":true}"#.to_string();
        match kind {
            "unreadable" => fs::create_dir_all(&old).expect("建立目錄"),
            "corrupt" => {
                body = "{broken".to_string();
                write_settings(&old, &body);
            }
            "newer" => {
                body = r#"{"schema":9,"selection":"statistical"}"#.to_string();
                write_settings(&old, &body);
            }
            _ => write_settings(&old, &body),
        }
        let report = read(&path, "linux");
        assert_eq!(report.path, old, "{kind}：要回報舊檔來源");
        match kind {
            "corrupt" => {
                assert!(matches!(report.error, Some(SettingsError::Corrupt { .. })));
                assert_eq!(report.state, FileState::Corrupt);
            }
            "unreadable" => {
                assert!(report.error.is_some());
                assert_eq!(report.state, FileState::Unreadable);
            }
            "newer" => {
                assert!(report.error.is_none());
                assert!(report.read_only);
                assert!(matches!(
                    save(&path, &report.settings),
                    Err(SettingsError::ReadOnly)
                ));
            }
            _ => {
                assert!(report.error.is_none());
                write_settings(path.parent().expect("資料根目錄"), "blocks mkdir");
                assert!(save(&path, &report.settings).is_err(), "目的被擋住時要失敗");
            }
        }
        if kind != "unreadable" {
            assert_eq!(read_text(&old), body, "{kind}：舊檔不得被改寫");
        }
    }
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn test_linux_settings_dangling_symlink_is_unreadable() {
    use std::os::unix::fs::symlink;

    let dir = tmpdir("linux-symlink");
    let share = dir.join(".local/share");
    let path = share.join("0c/settings.json");
    let old = share.join("zuin/settings.json");
    for broken_new in [true, false] {
        let _ = fs::remove_dir_all(&share);
        let broken = if broken_new {
            write_settings(&old, r#"{"selection":"statistical","punctuation":true}"#);
            path.clone()
        } else {
            old.clone()
        };
        fs::create_dir_all(broken.parent().expect("目錄")).expect("建立目錄");
        symlink("missing-settings.json", &broken).expect("建立符號連結");
        let report = read(&path, "linux");
        assert!(report.error.is_some(), "目標遺失的符號連結要回報讀不到");
        assert_eq!(report.state, FileState::Unreadable);
        assert_eq!(report.path, broken);
        assert_eq!(report.settings, Settings::default());
        assert_eq!(
            fs::read_link(&broken).expect("連結仍在").to_string_lossy(),
            "missing-settings.json"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_adopt_both_shift_rewrites_right_to_both() {
    let dir = tmpdir("both-shift");
    let path = dir.join("settings.json");
    write_settings(
        &path,
        r#"{"schema":1,"toggle_key":"right","left_shift":false,"selection":"statistical"}"#,
    );
    let user = dir.join("user.json");
    assert!(crate::both_shift::adopt_both_shift(&user).expect("遷移"));
    let text = read_text(&path);
    assert!(text.contains("\"toggle_key\": \"both\""), "{text}");
    assert!(text.contains("\"left_shift\": true"), "{text}");
    assert!(text.contains("\"selection\": \"statistical\""), "{text}");
    assert!(crate::both_shift::both_shift_marker(&path).is_file());
    assert!(!crate::both_shift::adopt_both_shift(&user).expect("再跑一次"));
    let _ = fs::remove_dir_all(&dir);
}
