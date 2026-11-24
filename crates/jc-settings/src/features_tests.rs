//! 逐項設定行為的測試，從前身 `internal/settings` 的各個 `*_test.go` 搬過來。
//!
//! 前身每一個開關都有獨立的測試檔；這裡合併成 table-driven 測試，斷言的內容
//! 仍是預設值、存檔來回與讀不懂的原文保留。

use std::fs;
use std::path::{Path, PathBuf};

use crate::settings::{
    default_mode_memory, default_toggle_key, load, read, save, Settings, SettingsError,
};
use crate::{keep_toggle_key, ProfileAction};

fn tmpdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("jc-settings-f-{tag}-{}", std::process::id()));
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
fn test_bool_settings_defaults_and_persist() {
    let dir = tmpdir("bool");
    let cases = [
        ("punctuation", "off", "on"),
        ("document-context", "on", "off"),
        ("english-autocomplete", "off", "on"),
        ("esc-clear", "on", "off"),
        ("space-selects", "on", "off"),
        ("advance-after-select", "off", "on"),
        ("emoji-equals-first", "on", "off"),
        ("simplified-hotkey", "off", "on"),
        ("ctrl-punctuation", "on", "off"),
        ("cjk-latin-space", "off", "on"),
        ("zhuyin-error-hint", "on", "off"),
        ("caps-lock-english", "off", "on"),
        ("toneless", "off", "on"),
        // 0.1.0-rc.2 起預設開啟日文自動変換（延遲整句轉換）。
        ("ja-live-conversion", "on", "off"),
        ("ja-symbols", "on", "off"),
        ("ja-learning", "on", "off"),
        ("ja-model", "on", "off"),
        ("allow-placeholders", "off", "on"),
    ];
    for (key, default, other) in cases {
        let path = dir.join(format!("{key}.json"));
        let defaults = Settings::default();
        assert_eq!(defaults.value(key).as_deref(), Some(default), "{key} 預設");
        save(&path, &defaults).expect("存檔預設");
        let (mut settings, error) = load(&path, "linux");
        assert!(error.is_none(), "{key}: {error:?}");
        settings.set(key, other, "linux").expect(key);
        save(&path, &settings).expect(key);
        let (back, error) = load(&path, "linux");
        assert!(error.is_none(), "{key}: {error:?}");
        assert_eq!(back.value(key).as_deref(), Some(other), "{key} 沒有保存");
        assert_eq!(back, settings, "{key} 來回不一致");
    }
}

#[test]
fn test_choice_settings_defaults_and_persist() {
    let dir = tmpdir("choice");
    let cases = [
        ("selection", "neural", "statistical"),
        ("candidate-cursor", "after", "before"),
        ("candidate-layout", "vertical", "grid"),
        ("emoji-shortcut", "equals", "off"),
        ("keyboard-layout", "standard", "hsu"),
        ("selection-keys", "1234567890", "asdfghjkl;"),
        ("toggle-key", "right", "left"),
        ("mode-memory", "native", "shared"),
        ("start-mode", "zhuyin", "english"),
        ("ja-input-method", "romaji", "kana"),
        ("ja-ambiguous", "both", "kana"),
        ("model-mode", "auto", "none"),
        ("primary-language", "zh", "ja"),
        ("shift-symbol-halfwidth", "off", "all"),
    ];
    for (key, default, other) in cases {
        let path = dir.join(format!("{key}.json"));
        let defaults = Settings::default();
        assert_eq!(defaults.value(key).as_deref(), Some(default), "{key} 預設");
        save(&path, &defaults).expect("存檔預設");
        let (mut settings, error) = load(&path, "linux");
        assert!(error.is_none(), "{key}: {error:?}");
        settings.set(key, other, "linux").expect(key);
        save(&path, &settings).expect(key);
        let (back, error) = load(&path, "linux");
        assert!(error.is_none(), "{key}: {error:?}");
        assert_eq!(back.value(key).as_deref(), Some(other), "{key} 沒有保存");
        assert_eq!(back, settings, "{key} 來回不一致");
    }
}

#[test]
fn test_platform_defaults() {
    assert_eq!(default_toggle_key("windows"), "both");
    assert_eq!(default_toggle_key("darwin"), "none");
    assert_eq!(default_toggle_key("linux"), "right");
    assert_eq!(default_mode_memory("darwin"), "shared");
    assert_eq!(default_mode_memory("linux"), "native");

    let linux = Settings::default_for_platform("linux");
    assert_eq!(linux.candidate_cursor, "after");
    assert!(!linux.left_shift);
    let windows = Settings::default_for_platform("windows");
    assert_eq!(windows.toggle_key, "both");
    assert!(windows.left_shift, "Windows 預設左右 Shift");
    let mac = Settings::default_for_platform("darwin");
    assert_eq!(mac.toggle_key, "none");
    assert_eq!(mac.candidate_cursor, "before");
    assert_eq!(mac.mode_memory, "shared");
}

#[test]
fn test_all_symbols_halfwidth_persists_and_stays_compatible() {
    let dir = tmpdir("symbols");
    let path = dir.join("settings.json");
    write_settings(
        &path,
        r#"{"schema":2,"selection":"statistical","shift_symbol_halfwidth":true,"future":42}"#,
    );
    let (settings, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(
        settings.value("shift-symbol-halfwidth").as_deref(),
        Some("on"),
        "舊設定檔的 true 是 Shift＋符號打半形"
    );
    for value in ["all", "on", "off", "all"] {
        let (mut settings, error) = load(&path, "linux");
        assert!(error.is_none(), "{error:?}");
        settings
            .set("shift-symbol-halfwidth", value, "linux")
            .expect(value);
        save(&path, &settings).expect(value);
        let (back, error) = load(&path, "linux");
        assert!(error.is_none(), "{value}: {error:?}");
        assert_eq!(
            back.value("shift-symbol-halfwidth").as_deref(),
            Some(value),
            "{value}"
        );
        assert_eq!(back.selection, crate::STATISTICAL);
        assert_eq!(back, settings, "{value} 來回不一致");
        let text = read_text(&path);
        let (shift, all) = match value {
            "off" => ("false", "false"),
            "on" => ("true", "false"),
            _ => ("true", "true"),
        };
        assert!(
            text.contains(&format!("\"shift_symbol_halfwidth\": {shift}")),
            "{value}: {text}"
        );
        assert!(
            text.contains(&format!("\"all_symbols_halfwidth\": {all}")),
            "{value}: {text}"
        );
        assert!(text.contains("\"future\": 42"), "{value}: {text}");
    }
    write_settings(
        &path,
        r#"{"schema":2,"shift_symbol_halfwidth":false,"all_symbols_halfwidth":true}"#,
    );
    let (settings, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert!(!settings.all_symbols_halfwidth, "殘值不應打開全部半形");
    assert!(!settings.shift_symbol_halfwidth);
}

#[test]
fn test_all_symbols_halfwidth_fixes_ctrl_and_pauses_toneless() {
    let mut settings = Settings::default();
    for (key, value) in [
        ("ctrl-punctuation", "off"),
        ("toneless", "on"),
        ("shift-symbol-halfwidth", "all"),
    ] {
        settings.set(key, value, "linux").expect(key);
    }
    assert!(settings.effective_ctrl_punctuation());
    assert!(!settings.effective_toneless());
    assert!(!settings.ctrl_punctuation);
    assert!(settings.toneless);
    settings
        .set("shift-symbol-halfwidth", "on", "linux")
        .expect("設定");
    assert!(!settings.effective_ctrl_punctuation(), "換回來照設定檔");
    assert!(settings.effective_toneless());
}

#[test]
fn test_symbol_width_not_locked_by_profiles() {
    for id in ["0c", "microsoft", "chewing", "mcbopomofo", "macos"] {
        let mut settings = crate::profiles::fresh_settings("linux");
        settings
            .profile_action(ProfileAction::Select, id, "", "linux")
            .expect(id);
        settings
            .set("shift-symbol-halfwidth", "all", "linux")
            .unwrap_or_else(|err| panic!("{id} 應可選所有符號鍵打半形：{err}"));
        settings
            .profile_action(ProfileAction::Select, "0c", "", "linux")
            .expect("選用");
        assert_eq!(
            settings.value("shift-symbol-halfwidth").as_deref(),
            Some("all"),
            "{id}：換風格不改標點寬度"
        );
    }
}

#[test]
fn test_shift_symbol_halfwidth_unusable_value_kept() {
    let dir = tmpdir("symbols-bad");
    let path = dir.join("settings.json");
    write_settings(&path, r#"{"schema":1,"shift_symbol_halfwidth":"yes"}"#);
    let (settings, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert!(!settings.shift_symbol_halfwidth);
    save(&path, &settings).expect("存檔");
    assert!(
        read_text(&path).contains("\"shift_symbol_halfwidth\": \"yes\""),
        "{}",
        read_text(&path)
    );
    assert!(Settings::default()
        .set("shift-symbol-halfwidth", "maybe", "linux")
        .is_err());
}

#[test]
fn test_left_shift_defaults_follow_platform_and_persists() {
    let dir = tmpdir("left-shift");
    let path = dir.join("settings.json");
    let linux = Settings::default();
    assert!(!linux.left_shift);
    assert_eq!(linux.value("left-shift").as_deref(), Some("off"));
    write_settings(&path, r#"{"selection":"statistical","future":42}"#);
    for value in ["on", "off", "on"] {
        let (mut settings, error) = load(&path, "linux");
        assert!(error.is_none(), "{error:?}");
        settings.set("left-shift", value, "linux").expect(value);
        save(&path, &settings).expect(value);
        let (back, error) = load(&path, "linux");
        assert!(error.is_none(), "{value}: {error:?}");
        assert_eq!(back.left_shift, value == "on", "{value}");
        assert_eq!(back.selection, crate::STATISTICAL);
        assert!(read_text(&path).contains("\"future\": 42"), "{value}");
    }
}

#[test]
fn test_left_shift_unknown_value_retained() {
    let mut settings =
        Settings::from_json_for(br#"{"left_shift":"both","punctuation":true}"#, "linux")
            .expect("解析");
    assert_eq!(settings.left_shift, Settings::default().left_shift);
    assert!(settings.punctuation);
    settings
        .set("selection", "statistical", "linux")
        .expect("設定");
    let text = serde_json::to_string(&settings).expect("序列化");
    assert!(text.contains("\"left_shift\":\"both\""), "{text}");
    settings.set("left-shift", "off", "linux").expect("設定");
    let text = serde_json::to_string(&settings).expect("序列化");
    assert!(text.contains("\"left_shift\":false"), "{text}");
}

#[test]
fn test_start_mode_default_and_compat() {
    let mut settings =
        Settings::from_json_for(br#"{"mode_memory":"native"}"#, "linux").expect("解析");
    assert_eq!(settings.start_mode, crate::START_ZHUYIN);
    assert!(!settings.starts_english("linux"));

    settings =
        Settings::from_json_for(br#"{"toggle_key":"right","start_mode":"english"}"#, "linux")
            .expect("解析");
    assert!(settings.starts_english("linux"), "english");
    let text = serde_json::to_string(&settings).expect("序列化");
    assert!(text.contains("\"start_mode\":\"english\""), "{text}");

    settings = Settings::from_json_for(br#"{"toggle_key":"right","start_mode":"latin"}"#, "linux")
        .expect("解析");
    assert!(!settings.starts_english("linux"), "讀不懂的值退回注音");
    let text = serde_json::to_string(&settings).expect("序列化");
    assert!(text.contains("\"start_mode\":\"latin\""), "{text}");
    assert!(
        settings
            .notes()
            .iter()
            .any(|note| note.contains("start-mode")),
        "{:?}",
        settings.notes()
    );
    settings
        .set("start-mode", "english", "linux")
        .expect("設定");
    assert!(settings.starts_english("linux"));
    let text = serde_json::to_string(&settings).expect("序列化");
    assert!(!text.contains("\"start_mode\":\"latin\""), "{text}");
    assert!(settings.set("start-mode", "chinese", "linux").is_err());
    assert_eq!(settings.value("start-mode").as_deref(), Some("english"));

    let without_shift =
        Settings::from_json_for(br#"{"toggle_key":"none","start_mode":"english"}"#, "linux")
            .expect("解析");
    assert!(
        !without_shift.starts_english("linux"),
        "不用 Shift 切換時固定從注音開始"
    );
    assert_eq!(
        without_shift.effective_start_mode("linux"),
        crate::START_ZHUYIN
    );
    assert!(without_shift.start_mode_locked("linux"));
    assert!(
        !without_shift.start_mode_locked("windows"),
        "Windows 仍可切回"
    );
}

#[test]
fn test_legacy_mode_migrates_to_mixed_without_touching_other_settings() {
    for legacy in ["zh", "ja", "en"] {
        let body = format!(
            r#"{{"mode":"{legacy}","punctuation":true,"toggle_key":"right","custom":"keep"}}"#
        );
        let settings = Settings::from_json_for(body.as_bytes(), "linux").expect("解析");
        assert_eq!(
            settings.start_mode,
            if legacy == "en" {
                crate::START_ENGLISH
            } else {
                crate::START_ZHUYIN
            },
            "{legacy} 的開機模式"
        );
        assert!(settings.punctuation, "{legacy} 的其他設定要保留");
        assert_eq!(settings.toggle_key, crate::TOGGLE_RIGHT);
        let text = serde_json::to_string(&settings).expect("序列化");
        let raw: serde_json::Value = serde_json::from_str(&text).expect("解析");
        assert!(raw.get("mode").is_none(), "mode 要遷移後移除：{text}");
        assert_eq!(raw["custom"], "keep", "未知欄位要原樣保留：{text}");
        assert!(
            settings.notes().iter().any(|note| note.contains("mode")),
            "{:?}",
            settings.notes()
        );
    }

    // 明確的 start_mode 優先，舊 mode=en 不能覆蓋使用者設定。
    let settings = Settings::from_json_for(
        br#"{"mode":"en","start_mode":"zhuyin","punctuation":false}"#,
        "linux",
    )
    .expect("解析");
    assert_eq!(settings.start_mode, crate::START_ZHUYIN);
    assert!(!settings.punctuation);

    // 讀不懂的 mode 值仍原樣保留，不假裝遷移成功。
    let settings = Settings::from_json_for(br#"{"mode":"klingon"}"#, "linux").expect("解析");
    let text = serde_json::to_string(&settings).expect("序列化");
    assert!(text.contains("\"mode\":\"klingon\""), "{text}");
}

#[test]
fn test_language_migration_and_projection() {
    let settings = Settings::default();
    assert_eq!(settings.primary_language, crate::LANGUAGE_ZH);
    assert_eq!(settings.language(), crate::LANGUAGE_ZH);
    assert!(!settings.japanese());
    assert_eq!(settings.ja_input_method, crate::JA_INPUT_ROMAJI);
    assert!(settings.ja_live_conversion, "日文預設逐字変換");
    assert_eq!(settings.ja_conversion, crate::JA_CONVERSION_LIVE);
    assert!(settings.ja_symbols && settings.ja_learning && settings.ja_model);

    let dir = tmpdir("language");
    let path = dir.join("settings.json");
    write_settings(
        &path,
        r#"{"schema":2,"selection":"statistical","punctuation":true,"keyboard_layout":"hsu","language":"ja","language_memory":"global","language_hotkey":false}"#,
    );
    let report = read(&path, "linux");
    assert!(report.error.is_none(), "{:?}", report.error);
    assert!(!report.read_only);
    let settings = report.settings;
    assert_eq!(settings.primary_language, crate::LANGUAGE_JA);
    assert_eq!(settings.language(), crate::LANGUAGE_JA);
    assert!(settings.japanese());
    assert_eq!(settings.ja_input_method, crate::JA_INPUT_ROMAJI);
    assert_eq!(settings.selection, crate::STATISTICAL);
    assert!(settings.punctuation);
    assert_eq!(settings.keyboard_layout, crate::LAYOUT_HSU);
    save(&path, &settings).expect("存檔");
    let text = read_text(&path);
    for want in [
        "\"primary_language\": \"ja\"",
        "\"ja_input_method\": \"romaji\"",
        "\"ja_model\": true",
        "\"schema\": 2",
    ] {
        assert!(text.contains(want), "存檔應包含 {want}：{text}");
    }
    for gone in ["\"language\":", "language_memory", "language_hotkey"] {
        assert!(!text.contains(gone), "舊語言鍵應移除 {gone}：{text}");
    }
    let (back, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(back, settings);

    let both = Settings::from_json_for(br#"{"language":"zh","primary_language":"ja"}"#, "linux")
        .expect("解析");
    assert_eq!(
        both.primary_language,
        crate::LANGUAGE_JA,
        "兩者都有時以 primary_language 為準"
    );

    let legacy = Settings::from_json_for(br#"{"output_simplified":true}"#, "linux").expect("解析");
    assert_eq!(legacy.output_variant, crate::VARIANT_SIMPLIFIED);
    assert!(legacy.output_simplified());
    let newer = Settings::from_json_for(
        br#"{"output_simplified":true,"output_variant":"traditional"}"#,
        "linux",
    )
    .expect("解析");
    assert_eq!(
        newer.output_variant,
        crate::VARIANT_TRADITIONAL,
        "新欄位優先"
    );
}

#[test]
fn test_unknown_legacy_language_value_falls_back_and_is_removed() {
    let dir = tmpdir("language-bad");
    let path = dir.join("settings.json");
    write_settings(
        &path,
        r#"{"schema":2,"language":"ko","language_memory":"per_app","ja_input_method":"t9","future":true}"#,
    );
    let report = read(&path, "linux");
    assert!(report.error.is_none(), "{:?}", report.error);
    let settings = report.settings;
    assert_eq!(settings.primary_language, crate::LANGUAGE_ZH);
    assert_eq!(settings.ja_input_method, crate::JA_INPUT_ROMAJI);
    assert!(!report.notes.is_empty());
    save(&path, &settings).expect("存檔");
    let text = read_text(&path);
    assert!(!text.contains("\"language\":"), "{text}");
    assert!(!text.contains("language_memory"), "{text}");
    assert!(text.contains("\"ja_input_method\": \"t9\""), "{text}");
    assert!(text.contains("\"future\": true"), "{text}");
}

#[test]
fn test_ja_settings_round_trip() {
    let dir = tmpdir("ja");
    let path = dir.join("settings.json");
    let mut settings = Settings::default();
    for (key, value) in [
        ("primary-language", "ja"),
        ("ja-input-method", "kana"),
        ("ja-live-conversion", "on"),
        ("ja-symbols", "off"),
        ("ja-learning", "off"),
        ("ja-model", "off"),
    ] {
        settings.set(key, value, "linux").expect(key);
    }
    assert!(settings.japanese());
    assert!(settings.ja_live_conversion);
    assert!(!settings.ja_symbols && !settings.ja_learning && !settings.ja_model);
    save(&path, &settings).expect("存檔");
    let (back, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(back, settings);
    assert_eq!(back.ja_input_method, crate::JA_INPUT_KANA);
    assert_eq!(back.value("primary-language").as_deref(), Some("ja"));
    assert_eq!(back.value("language").as_deref(), Some("ja"), "舊鍵是別名");
}

#[test]
fn test_ja_ambiguous_effective() {
    let settings = Settings::from_json_for(br#"{"ja_ambiguous":"later"}"#, "linux").expect("解析");
    assert_eq!(settings.effective_ja_ambiguous(), crate::JA_AMBIGUOUS_BOTH);
    assert!(settings.ja_ambiguous_lists_both());
    assert!(!settings.ja_ambiguous_kana_first());
    let mut settings = settings;
    settings.set("ja-ambiguous", "kana", "linux").expect("設定");
    assert!(settings.ja_ambiguous_kana_first());
    assert_eq!(settings.ja_ambiguous, "kana");
}

#[test]
fn test_preview_retired_key_dropped_on_save() {
    for raw in ["false", "true", "\"beta\""] {
        let dir = tmpdir("preview");
        let path = dir.join("settings.json");
        write_settings(
            &path,
            &format!(
                r#"{{"schema":1,"selection":"statistical","preview_0_2":{raw},"later_field":7}}"#
            ),
        );
        let report = read(&path, "linux");
        assert!(report.error.is_none(), "{raw}: {:?}", report.error);
        assert_eq!(report.settings.selection, crate::STATISTICAL);
        assert!(
            !report.notes.iter().any(|note| note.contains("preview")),
            "{raw}：已拿掉的鍵不當成本版不認得的欄位：{:?}",
            report.notes
        );
        assert_eq!(
            report.settings.value("preview-0-2").as_deref(),
            Some("on"),
            "{raw}"
        );
        save(&path, &report.settings).expect("存檔");
        let text = read_text(&path);
        assert!(!text.contains("preview"), "{raw}: {text}");
        assert!(text.contains("\"later_field\": 7"), "{raw}: {text}");
        assert!(
            text.contains("\"selection\": \"statistical\""),
            "{raw}: {text}"
        );
    }
}

#[test]
fn test_preview_set_is_retired() {
    assert!(!crate::keys().contains_key("preview-0-2"));
    let mut settings = Settings::default();
    match settings.set("nope", "on", "linux") {
        Err(SettingsError::Set(err)) => {
            assert_eq!(err.code(), crate::SetCode::UnknownKey);
            assert!(!err.to_string().contains("preview"), "{err}");
        }
        other => panic!("{other:?}"),
    }
    let before = settings.clone();
    settings
        .set("preview-0-2", "on", "linux")
        .expect("on 照舊成功");
    assert_eq!(settings, before, "on 不改任何設定");
    match settings.set("preview-0-2", "off", "linux") {
        Err(SettingsError::Set(err)) => {
            assert_eq!(err.code(), crate::SetCode::InvalidValue);
            assert!(err.to_string().contains("正式開放"), "{err}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(settings, before, "off 失敗時不改設定");
}

#[test]
fn test_keep_toggle_key() {
    struct Case {
        name: &'static str,
        body: Option<&'static str>,
        wrote: bool,
        toggle: Option<&'static str>,
        member: bool,
    }
    let cases = [
        Case {
            name: "沒有設定檔",
            body: None,
            wrote: true,
            toggle: Some("right"),
            member: true,
        },
        Case {
            name: "空物件",
            body: Some("{}"),
            wrote: true,
            toggle: Some("right"),
            member: true,
        },
        Case {
            name: "其他欄位都在但沒有切換鍵",
            body: Some(
                "{\n  \"schema\": 1,\n  \"punctuation\": true,\n  \"future_key\": [1, 2]\n}\n",
            ),
            wrote: true,
            toggle: Some("right"),
            member: true,
        },
        Case {
            name: "舊檔 left_shift=false",
            body: Some(r#"{"left_shift":false}"#),
            wrote: true,
            toggle: Some("right"),
            member: true,
        },
        Case {
            name: "舊檔 left_shift=true 已推出 both",
            body: Some(r#"{"left_shift":true}"#),
            wrote: false,
            toggle: Some("both"),
            member: false,
        },
        Case {
            name: "使用者選了不用 Shift",
            body: Some(r#"{"toggle_key":"none"}"#),
            wrote: false,
            toggle: Some("none"),
            member: true,
        },
        Case {
            name: "使用者選了左 Shift",
            body: Some(r#"{"toggle_key":"left","left_shift":true}"#),
            wrote: false,
            toggle: Some("left"),
            member: true,
        },
        Case {
            name: "本版讀不懂的切換鍵值",
            body: Some(r#"{"toggle_key":"ctrl"}"#),
            wrote: false,
            toggle: None,
            member: true,
        },
        Case {
            name: "較新版本的設定檔",
            body: Some(r#"{"schema":99}"#),
            wrote: false,
            toggle: None,
            member: false,
        },
    ];
    let dir = tmpdir("keep-toggle");
    for Case {
        name,
        body,
        wrote: want_wrote,
        toggle: want_toggle,
        member: want_member,
    } in cases
    {
        let path = dir.join(format!("{}.json", name.replace(' ', "-")));
        if let Some(body) = body {
            write_settings(&path, body);
        }
        let wrote = keep_toggle_key(&path, "right").unwrap_or_else(|err| panic!("{name}: {err}"));
        assert_eq!(wrote, want_wrote, "{name}");
        if body.is_none() && !wrote {
            assert!(!path.exists(), "{name}：不該建立設定檔");
            continue;
        }
        let text = read_text(&path);
        if !wrote {
            assert_eq!(text, body.expect("有原檔"), "{name}：沒有要改的檔案被動過");
        }
        let members: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&text).unwrap_or_else(|err| panic!("{name}: {err}\n{text}"));
        assert_eq!(
            members.contains_key("toggle_key"),
            want_member,
            "{name}: {text}"
        );
        if let Some(want) = want_toggle {
            let (settings, error) = load(&path, "linux");
            assert!(error.is_none(), "{name}: {error:?}");
            assert_eq!(settings.toggle_key, want, "{name}");
        }
        if body.is_none() && wrote {
            assert_eq!(members.len(), 2, "{name}：只寫 schema 與 toggle_key");
            assert_eq!(
                members.get("schema"),
                Some(&serde_json::Value::from(2)),
                "{name}"
            );
        }
        assert!(
            !keep_toggle_key(&path, "right").expect("第二次"),
            "{name}：第二次不再寫"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_keep_toggle_key_only_once() {
    let dir = tmpdir("keep-once");
    let path = dir.join("settings.json");
    assert!(keep_toggle_key(&path, "right").expect("第一次"));
    let (mut settings, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    settings.set("toggle-key", "none", "linux").expect("設定");
    save(&path, &settings).expect("存檔");
    assert!(!keep_toggle_key(&path, "right").expect("第二次"));
    let (back, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(back.toggle_key, "none", "使用者的選擇不得被蓋掉");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_keep_toggle_key_leaves_corrupt_file() {
    let dir = tmpdir("keep-corrupt");
    for body in ["not json", "[1]", "null"] {
        let path = dir.join("settings.json");
        write_settings(&path, body);
        let err = keep_toggle_key(&path, "right").expect_err("壞檔要回錯誤");
        assert!(
            matches!(err, SettingsError::Corrupt { .. }),
            "{body}: {err:?}"
        );
        assert_eq!(read_text(&path), body, "{body}：壞檔不得被改");
    }
    assert!(keep_toggle_key(&dir.join("other.json"), "ctrl").is_err());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_change_writes_like_updater_set() {
    let dir = tmpdir("change");
    let user = dir.join("user.json");
    let path = crate::settings_path_for_store(&user);
    assert!(!crate::change(&user, &[("output-simplified", "off")], "linux").expect("沒變"));
    assert!(!path.exists(), "值沒變不該建立設定檔");

    write_settings(
        &path,
        r#"{"selection":"statistical","future_key":[1,2],"output_simplified":false}"#,
    );
    assert!(crate::change(&user, &[("output-simplified", "on")], "linux").expect("變更"));
    let text = read_text(&path);
    let value: serde_json::Value = serde_json::from_str(&text).expect("解析");
    assert_eq!(value["output_variant"], "simplified", "{text}");
    assert_eq!(value["selection"], "statistical", "{text}");
    assert!(value.get("future_key").is_some(), "{text}");
    assert!(
        value.get("output_simplified").is_none(),
        "舊欄位應移除：{text}"
    );
    assert!(
        !crate::change(&user, &[("output-simplified", "on")], "linux").expect("值沒變"),
        "值沒變不重寫"
    );

    write_settings(&path, "{壞");
    assert!(matches!(
        crate::change(&user, &[("output-simplified", "off")], "linux"),
        Err(SettingsError::Corrupt { .. })
    ));
    assert_eq!(read_text(&path), "{壞", "損毀的檔案被改了");

    write_settings(&path, r#"{"schema":99,"output_simplified":true}"#);
    assert!(matches!(
        crate::change(&user, &[("output-simplified", "off")], "linux"),
        Err(SettingsError::ReadOnly)
    ));

    write_settings(&path, "{}");
    assert!(crate::change(&user, &[("no-such-key", "on")], "linux").is_err());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_palette_unknown_choices_preserved() {
    let mut settings = Settings::from_json_for(
        br#"{"candidate_layout":"future-layout","emoji_shortcut":"future-key","other":3}"#,
        "linux",
    )
    .expect("解析");
    assert_eq!(settings.candidate_layout, "vertical", "讀不懂的值退回預設");
    assert_eq!(settings.emoji_shortcut, "equals");
    let text = serde_json::to_string(&settings).expect("序列化");
    let raw: serde_json::Value = serde_json::from_str(&text).expect("解析");
    assert_eq!(raw["candidate_layout"], "future-layout", "{text}");
    assert_eq!(raw["emoji_shortcut"], "future-key", "{text}");
    settings
        .set("candidate-layout", "grid", "linux")
        .expect("設定");
    settings
        .set("emoji-shortcut", "equals", "linux")
        .expect("設定");
    let text = serde_json::to_string(&settings).expect("序列化");
    let mut restored: Settings = serde_json::from_str(&text).expect("解析");
    assert_eq!(restored.candidate_layout, "grid");
    assert_eq!(restored.emoji_shortcut, "equals");
    assert!(
        restored.set("emoji-shortcut", "slash", "linux").is_err(),
        "不支援的按鍵應被拒絕"
    );
}

#[test]
fn test_emoji_defaults_and_explicit_off_round_trip() {
    for input in [
        "{}",
        r#"{"emoji_shortcut":"off","emoji_equals_first":false,"future":7}"#,
    ] {
        let settings = Settings::from_json_for(input.as_bytes(), "linux").expect("解析");
        if input == "{}" {
            assert_eq!(settings.emoji_shortcut, "equals");
            assert!(settings.emoji_equals_first);
        } else {
            assert_eq!(settings.emoji_shortcut, "off");
            assert!(!settings.emoji_equals_first);
        }
        let text = serde_json::to_string(&settings).expect("序列化");
        let restored: Settings = serde_json::from_str(&text).expect("解析");
        assert_eq!(restored, settings, "{input}");
    }
    let mut settings =
        Settings::from_json_for(br#"{"emoji_equals_first":"future"}"#, "linux").expect("解析");
    assert!(settings.emoji_equals_first);
    let text = serde_json::to_string(&settings).expect("序列化");
    let raw: serde_json::Value = serde_json::from_str(&text).expect("解析");
    assert_eq!(raw["emoji_equals_first"], "future", "{text}");
    settings
        .set("emoji-equals-first", "off", "linux")
        .expect("設定");
    let text = serde_json::to_string(&settings).expect("序列化");
    let raw: serde_json::Value = serde_json::from_str(&text).expect("解析");
    assert_eq!(raw["emoji_equals_first"], false, "{text}");
}

#[test]
fn test_mixed_profile_migrates_and_removes_english_profile_keys() {
    let dir = tmpdir("mixed-profile");
    let path = dir.join("settings.json");
    write_settings(
        &path,
        r#"{
            "schema": 2,
            "language": "ja",
            "language_memory": "global",
            "language_hotkey": false,
            "english_profile": true,
            "output_simplified": true,
            "start_mode": "english",
            "toggle_key": "both",
            "future": {"keep": true}
        }"#,
    );
    let report = read(&path, "linux");
    assert!(report.error.is_none(), "{:?}", report.error);
    assert!(!report.read_only);
    let settings = report.settings;
    assert_eq!(settings.primary_language, crate::LANGUAGE_JA);
    assert!(settings.japanese());
    assert_eq!(settings.output_variant, crate::VARIANT_SIMPLIFIED);
    assert!(settings.output_simplified());
    assert_eq!(
        settings.start_mode,
        crate::START_ENGLISH,
        "舊的中英模式要保留"
    );
    assert_eq!(settings.toggle_key, crate::TOGGLE_BOTH);
    save(&path, &settings).expect("存檔");
    let text = read_text(&path);
    for gone in [
        "\"language\":",
        "language_memory",
        "language_hotkey",
        "english_profile",
        "\"output_simplified\"",
    ] {
        assert!(!text.contains(gone), "舊鍵應移除 {gone}：{text}");
    }
    for want in [
        "\"primary_language\": \"ja\"",
        "\"output_variant\": \"simplified\"",
        "\"future\": {",
    ] {
        assert!(text.contains(want), "存檔應有 {want}：{text}");
    }
    let (back, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(back, settings);
}

#[test]
fn test_mixed_profile_new_keys_win_over_old_english_settings() {
    let settings = Settings::from_json_for(
        br#"{"schema":2,"language":"ja","primary_language":"zh","output_simplified":true,"output_variant":"shinjitai"}"#,
        "linux",
    )
    .expect("解析");
    assert_eq!(settings.primary_language, crate::LANGUAGE_ZH);
    assert_eq!(settings.output_variant, crate::VARIANT_SHINJITAI);
    assert!(!settings.output_simplified());
}

#[test]
fn test_future_boolean_value_survives_until_explicit_set() {
    let settings = Settings::from_json_for(
        br#"{"schema":2,"english_autocomplete":"later","selection":"statistical"}"#,
        "linux",
    )
    .expect("解析");
    assert!(!settings.english_autocomplete);
    let text = serde_json::to_string(&settings).expect("序列化");
    assert!(
        text.contains("\"english_autocomplete\":\"later\""),
        "{text}"
    );
    let mut settings = settings;
    settings
        .set("english-autocomplete", "on", "linux")
        .expect("設定");
    let text = serde_json::to_string(&settings).expect("序列化");
    assert!(!text.contains("later"), "{text}");
    assert!(text.contains("\"english_autocomplete\":true"), "{text}");
}

/// 舊設定檔沒有 `ja_conversion`：視為逐字変換。
#[test]
fn test_ja_conversion_defaults_to_live_for_old_settings() {
    let dir = tmpdir("ja-conversion-live");
    let path = dir.join("settings.json");
    write_settings(&path, r#"{"ja_neural_mode":"auto"}"#);
    let (settings, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(
        settings.effective_ja_conversion(),
        crate::JA_CONVERSION_LIVE
    );
    assert!(settings.ja_live_enabled());
}

/// 舊設定檔只有 `ja_live_conversion: false`：尊重使用者選整句変換。
#[test]
fn test_ja_conversion_legacy_off_maps_to_sentence() {
    let dir = tmpdir("ja-conversion-sentence");
    let path = dir.join("settings.json");
    write_settings(&path, r#"{"ja_live_conversion":false}"#);
    let (settings, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(
        settings.effective_ja_conversion(),
        crate::JA_CONVERSION_SENTENCE
    );
    assert!(!settings.ja_live_enabled());
}

/// 新欄位優先於舊 bool；存檔後可往返。
#[test]
fn test_ja_conversion_round_trip() {
    let mut settings = Settings::default();
    settings
        .set("ja-conversion", "sentence", "linux")
        .expect("ja-conversion");
    assert_eq!(
        settings.effective_ja_conversion(),
        crate::JA_CONVERSION_SENTENCE
    );
    assert!(!settings.ja_live_conversion);

    let dir = tmpdir("ja-conversion-roundtrip");
    let path = dir.join("settings.json");
    save(&path, &settings).expect("存檔");
    let (back, error) = load(&path, "linux");
    assert!(error.is_none(), "{error:?}");
    assert_eq!(
        back.effective_ja_conversion(),
        crate::JA_CONVERSION_SENTENCE
    );
}
