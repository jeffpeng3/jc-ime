//! `settings.json` → [`SessionConfig`] 的映射。
//!
//! 所有欄位的對照集中在 [`effective`] 與 [`session_config`]；CLI 覆寫由
//! [`Overrides`] 表示，優先於檔案值。
//!
//! 讀檔失敗、JSON 壞掉或單一欄位值不合法都只記警告並退回預設值，不讓引擎
//! 啟動失敗。欄位對照與「本版忽略」的理由見 `docs/port/settings-wire.md`。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use jc_core::inputstyle::Rules;
use jc_core::session::{NeuralConfig, SessionConfig, DEFAULT_MAX_CANDIDATES};
use jc_core::zh::fuzzy;
use jc_core::Mode;
use jc_settings::keybinds::Keybinds;
use jc_settings::settings::{self, FileState, Settings};
use serde::{Serialize, Serializer};

/// 這個平台在 `jc-settings` 使用的 `goos` 代號。
///
/// Rust 的 `macos` 對應 Go 的 `darwin`；其餘平台名稱相同。
pub fn platform_goos() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    }
}

/// 模式轉成命令列與 JSON 使用的代號。
pub fn mode_str(mode: Mode) -> &'static str {
    match mode {
        Mode::Mixed => "mixed",
    }
}

fn serialize_mode<S: Serializer>(mode: &Mode, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(mode_str(*mode))
}

/// CLI 覆寫；有值的欄位優先於設定檔。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Overrides {
    /// `--mode`：啟動時的輸入模式。
    pub mode: Option<Mode>,
    /// `--neural`：是否啟用日文神經轉換。
    pub neural: Option<bool>,
}

/// 設定檔的讀取結果。
///
/// 不論檔案狀態如何，`settings` 都是一份可用的設定；讀不懂的部分已經退回
/// 預設值並記在 `warnings`。
#[derive(Debug)]
pub struct LoadedSettings {
    /// 實際讀取的路徑；沒有資料根目錄也沒有 `--settings` 時為 `None`。
    pub path: Option<PathBuf>,
    /// 設定值。
    pub settings: Settings,
    /// 檔案狀態。
    pub state: FileState,
    /// 退回預設或原樣保留的說明，呼叫端逐條記警告。
    pub warnings: Vec<String>,
    /// `keybinds` 解析失敗的原因；啟動時已退回預設，熱重載據此保留舊設定。
    pub keybind_error: Option<String>,
    /// 讀取設定檔當下的戳記；watcher 用它當基準，避免啟動期間的變更被漏掉。
    pub stamp: Option<(u64, u64)>,
}

/// 讀取設定檔。
///
/// 路徑順序：`explicit`（`--settings`）、`data_root` 的
/// [`settings::settings_path`]。兩者都沒有時只回平台預設值，不讀任何檔案。
pub fn load(explicit: Option<&Path>, data_root: Option<&Path>, goos: &str) -> LoadedSettings {
    let path = explicit
        .map(Path::to_path_buf)
        .or_else(|| data_root.map(jc_settings::paths::settings_path));
    let Some(path) = path else {
        return LoadedSettings {
            path: None,
            settings: Settings::default_for_platform(goos),
            state: FileState::Missing,
            warnings: Vec::new(),
            keybind_error: None,
            stamp: None,
        };
    };
    // 先取戳記再讀檔：讀取期間若檔案又被改，watcher 第一次 tick 會看到差異。
    let stamp = crate::config_watch::stamp(&path);
    let report = settings::read(&path, goos);
    let mut warnings = Vec::new();
    match report.state {
        FileState::Ok => warnings.extend(report.notes.iter().cloned()),
        // 缺檔是正常的第一次啟動，不警告。
        FileState::Missing => {}
        FileState::Corrupt | FileState::Unreadable => {
            let detail = report
                .error
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| report.path.display().to_string());
            warnings.push(format!("{detail}；使用預設值"));
        }
    }
    let mut settings = report.settings;
    // 啟動時快捷鍵解析失敗就整份沿用預設，不讓引擎起不來；錯誤另外留給
    // 熱重載判斷，避免執行中改壞 chord 時被當成成功而清掉使用者的綁定。
    let mut keybind_error = None;
    if let Err(err) = settings.keybinds.chords() {
        warnings.push(format!("keybinds：{err}；整份改用預設綁定"));
        keybind_error = Some(err.to_string());
        settings.keybinds = Keybinds::default();
    }
    LoadedSettings {
        path: Some(report.path),
        settings,
        state: report.state,
        warnings,
        keybind_error,
        stamp,
    }
}

/// 生效後的設定：檔案值與 CLI 覆寫合併的結果，也是 `--print-settings` 的內容。
///
/// 只放引擎真的會用到的欄位，加上外殼需要知道但引擎本身不處理的欄位
/// （`esc_clear`、`toggle_key` 等）；後者標在文件裡。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EffectiveSettings {
    /// 啟動時的輸入模式。
    #[serde(serialize_with = "serialize_mode")]
    pub mode: Mode,
    /// 混合模式但只走羅馬字；舊設定 `primary_language=ja` 的等價入口。
    pub ja_only: bool,
    /// 一次轉換保留幾個候選。
    pub max_candidates: usize,
    /// 選字方式：`neural` 或 `statistical`（已套用 `--neural` 與
    /// `ja_neural_mode`）。
    pub selection: String,
    /// 是否啟用日文神經轉換；`ja_neural_mode != off`。
    pub neural: bool,
    /// 日文神經轉換模式：`auto`、`quality` 或 `off`。
    pub ja_neural_mode: String,
    /// 日文候選是否使用 reranker 重排。
    pub ja_rerank: bool,
    /// 中英混打；開關已移除、欄位也從 JSON 契約刪除，引擎一律開啟。
    pub mixed_input: bool,
    /// 英文模式自動補字。
    pub english_autocomplete: bool,
    /// 按 Esc 清除組字；由外殼決定是否把 Esc 轉送引擎。
    pub esc_clear: bool,
    /// 空白鍵開候選。
    pub space_selects: bool,
    /// 選字後游標移到所選詞後面；本版由外殼處理。
    pub advance_after_select: bool,
    /// 中英切換鍵；本版由外殼處理。
    pub toggle_key: String,
    /// 舊欄位：左 Shift 是否切換（由 `toggle_key` 推出）。
    pub left_shift: bool,
    /// 開始時的中英模式。
    pub start_mode: String,
    /// 依前文選字。
    pub document_context: bool,
    /// 提交時補標點。
    pub punctuation: bool,
    /// 補上的標點；`punctuation` 關閉時為 `None`。
    pub punct_mark: Option<char>,
    /// 不打聲調也能選字。
    pub toneless: bool,
    /// 打錯注音時提示。
    pub zhuyin_error_hint: bool,
    /// Ctrl＋標點打全形。
    pub ctrl_punctuation: bool,
    /// 漢字與英文字母相連處補半形空格。
    pub cjk_latin_space: bool,
    /// 啟動時是否輸出簡體字；沿用既有 `output_simplified` 欄位。
    pub output_simplified: bool,
    /// 啟用的模糊音配對。
    pub fuzzy_pairs: Vec<(char, char)>,
    /// 日文逐字変換（ライブ変換）。
    pub ja_live: bool,
    /// 個人化學習是否啟用。
    pub learning: bool,
    /// 候選選字鍵。
    pub selection_keys: String,
    /// Emoji 捷徑：`off` 或 `equals`。
    pub emoji_shortcut: String,
    /// Shift＋符號打半形。
    pub shift_symbol_halfwidth: bool,
    /// 快捷鍵綁定；`bindings` 是正規化後的字串表，供協定回報。
    pub keybinds: Keybinds,
    /// action → 正規化 chord；由 `keybinds` 推導。
    pub bindings: BTreeMap<String, String>,
    /// 輸入習慣的鍵位規則；不由 JSON 直接輸出。
    #[serde(skip)]
    pub rules: Rules,
}

/// 把設定檔與 CLI 覆寫合併成生效後的設定。
///
/// 不合法的列舉值在讀檔時已退回預設；這裡只負責挑選與投影。
pub fn effective(settings: &Settings, overrides: &Overrides, goos: &str) -> EffectiveSettings {
    let mode = overrides
        .mode
        .unwrap_or_else(|| mode_from_settings(settings));
    // CLI `--neural` 仍優先：on 對應 auto，off 對應 off。
    let ja_neural_mode = match overrides.neural {
        Some(true) => settings::JA_NEURAL_AUTO,
        Some(false) => settings::JA_NEURAL_OFF,
        None => settings.effective_ja_neural_mode(),
    }
    .to_string();
    let neural = ja_neural_mode != settings::JA_NEURAL_OFF;
    let punct_mark = settings.punctuation.then_some('，');
    EffectiveSettings {
        mode,
        ja_only: settings.japanese(),
        max_candidates: DEFAULT_MAX_CANDIDATES,
        selection: if neural {
            settings::NEURAL
        } else {
            settings::STATISTICAL
        }
        .to_string(),
        neural,
        ja_neural_mode,
        ja_rerank: settings.ja_rerank,
        // mixed_input 已從 JSON 契約移除；引擎一律對中文使用者用混打。
        mixed_input: true,
        english_autocomplete: settings.english_autocomplete,
        esc_clear: settings.esc_clear,
        space_selects: settings.space_selects,
        advance_after_select: settings.advance_after_select,
        toggle_key: settings.toggle_key.clone(),
        left_shift: settings.left_shift,
        start_mode: settings.effective_start_mode(goos).to_string(),
        document_context: settings.document_context,
        punctuation: settings.punctuation,
        punct_mark,
        toneless: settings.effective_toneless(),
        zhuyin_error_hint: settings.zhuyin_error_hint,
        ctrl_punctuation: settings.effective_ctrl_punctuation(),
        cjk_latin_space: settings.cjk_latin_space,
        output_simplified: settings.output_simplified(),
        fuzzy_pairs: fuzzy_pairs(settings),
        ja_live: settings.ja_live_enabled(),
        learning: settings.ja_learning,
        selection_keys: settings.selection_keys.clone(),
        emoji_shortcut: settings.emoji_shortcut.clone(),
        shift_symbol_halfwidth: settings.shift_symbol_halfwidth,
        keybinds: settings.keybinds.clone(),
        bindings: settings.keybinds.bindings().unwrap_or_default(),
        rules: settings.profile_rules(goos),
    }
}

/// 設定檔決定的啟動模式。
///
/// 混打開關已移除：日文主要語言進 `Ja`，其餘中文使用者一律進 `Mixed`，
/// 不再有純注音 `Zh` 的啟動模式。CLI `--mode` 一律優先。
fn mode_from_settings(_settings: &Settings) -> Mode {
    Mode::Mixed
}

/// 設定檔裡啟用（非 `off`）的模糊音配對。
fn fuzzy_pairs(settings: &Settings) -> Vec<(char, char)> {
    let config = settings.fuzzy_config();
    [
        (fuzzy::Pair::ZZh, 'ㄗ', 'ㄓ'),
        (fuzzy::Pair::CCh, 'ㄘ', 'ㄔ'),
        (fuzzy::Pair::SSh, 'ㄙ', 'ㄕ'),
        (fuzzy::Pair::EnEng, 'ㄣ', 'ㄥ'),
    ]
    .into_iter()
    .filter(|(pair, _, _)| config.mode(*pair) != fuzzy::Mode::Off)
    .map(|(_, first, second)| (first, second))
    .collect()
}

/// 把生效後的設定套進 session 設定。
///
/// `base` 裡的資料（詞典、語言模型、神經 runtime）原樣保留；這裡只改由設定
/// 決定的欄位。`--max-frame` 是協定層參數，不在 [`SessionConfig`] 裡。
pub fn session_config(mut base: SessionConfig, effective: &EffectiveSettings) -> SessionConfig {
    base.mode = effective.mode;
    base.ja_only = effective.ja_only;
    base.max_candidates = effective.max_candidates;
    base.english_autocomplete = effective.english_autocomplete;
    base.cjk_latin_space = effective.cjk_latin_space;
    base.document_context = effective.document_context;
    base.ctrl_punct = effective.ctrl_punctuation;
    base.toneless = effective.toneless;
    base.zhuyin_error_hint = effective.zhuyin_error_hint;
    base.punct_mark = effective.punct_mark;
    base.fuzzy_pairs = effective.fuzzy_pairs.clone();
    base.learning = effective.learning;
    // 門檻由模式決定；模型、逾時、文脈長度與 runtime 沿用載入器的值。
    let runtime = base.ja_neural.runtime.clone();
    let mut ja_neural = match effective.ja_neural_mode.as_str() {
        settings::JA_NEURAL_QUALITY => NeuralConfig::quality(),
        settings::JA_NEURAL_OFF => NeuralConfig::off(),
        _ => NeuralConfig::default(),
    };
    ja_neural.model = base.ja_neural.model;
    ja_neural.timeout_ms = base.ja_neural.timeout_ms;
    ja_neural.max_context_chars = base.ja_neural.max_context_chars;
    ja_neural.runtime = runtime;
    base.ja_neural = ja_neural;
    base.ja_rerank = effective.ja_rerank;
    base.ja_live.enabled = effective.ja_live;
    base.style.mixed_input = effective.mixed_input;
    if effective.space_selects {
        // 舊設定讀到就忽略；空白鍵固定只插入空格。
        jc_core::session::warn_space_selects_ignored();
    }
    base.style.space_inserts_space = true;
    base.style.selection_keys = effective.selection_keys.clone();
    base.style.emoji_shortcut = effective.emoji_shortcut.clone();
    base.style.shift_symbol_halfwidth = effective.shift_symbol_halfwidth;
    base.style.rules = effective.rules.clone();
    base
}

/// `--print-settings` 的完整輸出：來源資訊加上生效後的設定。
#[derive(Debug, Serialize)]
pub struct PrintedSettings<'a> {
    /// 實際讀取的設定檔；沒有可用路徑時為 `null`。
    pub settings_path: Option<&'a Path>,
    /// 設定檔狀態：`ok`、`missing`、`corrupt` 或 `unreadable`。
    pub settings_state: &'a str,
    /// 生效後的設定。
    #[serde(flatten)]
    pub settings: &'a EffectiveSettings,
}

impl LoadedSettings {
    /// 產生 `--print-settings` 的漂亮 JSON。
    pub fn print_json(&self, effective: &EffectiveSettings) -> serde_json::Result<String> {
        let printed = PrintedSettings {
            settings_path: self.path.as_deref(),
            settings_state: self.state.as_str(),
            settings: effective,
        };
        serde_json::to_string_pretty(&printed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("jc-engine-settings-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn parsed(json: &str) -> Settings {
        Settings::from_json_for(json.as_bytes(), "linux").expect("測試 JSON 必須合法")
    }

    /// 對照 `Settings` 全部會被映射的欄位：完整檔案轉出來的 session 設定。
    #[test]
    fn full_settings_map_every_mapped_field() {
        let settings = parsed(
            r#"{
                "schema": 2,
                "selection": "statistical",
                "punctuation": true,
                "document_context": false,
                "mixed_input": true,
                "english_autocomplete": true,
                "toggle_key": "left",
                "start_mode": "english",
                "esc_clear": false,
                "space_selects": false,
                "advance_after_select": true,
                "candidate_cursor": "before",
                "selection_keys": "asdfghjkl;",
                "emoji_shortcut": "off",
                "shift_symbol_halfwidth": true,
                "cjk_latin_space": true,
                "zhuyin_error_hint": false,
                "toneless": true,
                "ja_live_conversion": true,
                "ja_learning": false,
                "fuzzy": {"z-zh": "auto", "c-ch": "off", "s-sh": "select", "en-eng": "off"}
            }"#,
        );
        let effective = effective(&settings, &Overrides::default(), "linux");
        assert_eq!(effective.mode, Mode::Mixed, "混打優先於開始模式");
        assert_eq!(effective.selection, "statistical");
        assert!(!effective.neural);
        assert_eq!(effective.punct_mark, Some('，'));
        assert!(!effective.document_context);
        assert!(!effective.esc_clear);
        assert!(!effective.space_selects);
        assert!(effective.advance_after_select);
        assert_eq!(effective.toggle_key, "left");
        assert!(effective.left_shift);
        assert_eq!(effective.fuzzy_pairs, vec![('ㄗ', 'ㄓ'), ('ㄙ', 'ㄕ')]);
        assert_eq!(effective.selection_keys, "asdfghjkl;");
        assert_eq!(effective.emoji_shortcut, "off");
        assert!(effective.shift_symbol_halfwidth);
        assert!(effective.ja_live);
        assert!(!effective.learning);
        assert!(!effective.output_simplified, "預設輸出正體");

        let config = session_config(SessionConfig::default(), &effective);
        assert_eq!(config.mode, Mode::Mixed);
        assert_eq!(config.max_candidates, DEFAULT_MAX_CANDIDATES);
        assert_eq!(config.punct_mark, Some('，'));
        assert!(!config.ja_neural.enabled);
        assert!(config.ja_live.enabled);
        assert!(!config.document_context);
        assert!(config.english_autocomplete);
        assert!(config.cjk_latin_space);
        assert!(!config.zhuyin_error_hint);
        assert!(config.toneless);
        assert!(!config.learning);
        assert_eq!(config.fuzzy_pairs, vec![('ㄗ', 'ㄓ'), ('ㄙ', 'ㄕ')]);
        assert_eq!(config.style.selection_keys, "asdfghjkl;");
        assert_eq!(config.style.emoji_shortcut, "off");
        assert_eq!(config.style.rules, effective.rules);
        assert!(config.style.mixed_input);
        assert!(config.style.space_inserts_space);
        assert!(config.style.shift_symbol_halfwidth);
        assert_eq!(effective.ja_neural_mode, "off", "statistical 遷移成 off");
        assert!(!effective.ja_rerank, "rerank 預設關閉");
        assert!(!config.ja_rerank);
    }

    /// 舊 `output_simplified` 欄位沿用為啟動時的簡體輸出預設。
    #[test]
    fn output_simplified_setting_is_the_startup_default() {
        for body in [
            r#"{"output_simplified": true}"#,
            r#"{"output_variant": "simplified"}"#,
        ] {
            let effective = effective(&parsed(body), &Overrides::default(), "linux");
            assert!(effective.output_simplified, "{body}");
        }
        let effective = effective(
            &parsed(r#"{"output_simplified": false}"#),
            &Overrides::default(),
            "linux",
        );
        assert!(!effective.output_simplified);
    }

    /// 三個日文神經模式各自對應到門檻；rerank 獨立映射。
    #[test]
    fn ja_neural_modes_map_to_thresholds_and_rerank() {
        for (mode, enabled, gap, min_mean) in [
            ("auto", true, 472.5, 5.98),
            ("quality", true, 460.0, 0.0),
            ("off", false, 472.5, 5.98),
        ] {
            let settings = parsed(&format!(
                r#"{{"ja_neural_mode": "{mode}", "ja_rerank": true}}"#
            ));
            let effective = effective(&settings, &Overrides::default(), "linux");
            assert_eq!(effective.ja_neural_mode, mode);
            assert_eq!(effective.neural, enabled, "{mode}");
            assert!(effective.ja_rerank, "{mode}");
            let config = session_config(SessionConfig::default(), &effective);
            assert_eq!(config.ja_neural.enabled, enabled, "{mode}");
            assert_eq!(config.ja_neural.gap_threshold, gap, "{mode}");
            assert_eq!(config.ja_neural.min_mean_candidates, min_mean, "{mode}");
            assert!(config.ja_rerank, "{mode}");
        }
    }

    /// `--neural` 仍優先於設定檔的三態。
    #[test]
    fn cli_neural_overrides_three_state_setting() {
        let settings = parsed(r#"{"ja_neural_mode": "quality"}"#);
        let off = effective(
            &settings,
            &Overrides {
                neural: Some(false),
                ..Overrides::default()
            },
            "linux",
        );
        assert_eq!(off.ja_neural_mode, "off");
        assert!(!off.neural);
        let on = effective(
            &settings,
            &Overrides {
                neural: Some(true),
                ..Overrides::default()
            },
            "linux",
        );
        assert_eq!(on.ja_neural_mode, "auto", "--neural on 回到低呼叫門檻");
        assert!(on.neural);
    }

    /// 缺檔：預設值，沒有警告。
    #[test]
    fn missing_file_uses_defaults_without_warning() {
        let dir = temp_dir("missing");
        let loaded = load(None, Some(&dir), "linux");
        assert_eq!(loaded.state, FileState::Missing);
        assert!(loaded.warnings.is_empty(), "缺檔不該警告");
        let default = Settings::default_for_platform("linux");
        assert_eq!(loaded.settings.selection, default.selection);
        let effective = effective(&loaded.settings, &Overrides::default(), "linux");
        assert!(effective.mixed_input, "引擎一律用混打");
        fs::remove_dir_all(&dir).ok();
    }

    /// 壞 JSON：預設值，只回報一次警告。
    #[test]
    fn corrupt_file_falls_back_with_one_warning() {
        let dir = temp_dir("corrupt");
        let path = dir.join(settings::FILE_NAME);
        fs::write(&path, b"{ not json").unwrap();
        let loaded = load(None, Some(&dir), "linux");
        assert_eq!(loaded.state, FileState::Corrupt);
        assert_eq!(loaded.warnings.len(), 1, "{:?}", loaded.warnings);
        assert!(
            loaded.warnings[0].contains("使用預設值"),
            "警告要說明退回預設：{:?}",
            loaded.warnings
        );
        assert_eq!(loaded.settings.selection, settings::NEURAL);
        let effective = effective(&loaded.settings, &Overrides::default(), "linux");
        assert!(effective.mixed_input, "引擎一律用混打");
        fs::remove_dir_all(&dir).ok();
    }

    /// 非法列舉值：該欄位退回預設並警告。
    #[test]
    fn invalid_enum_falls_back_with_warning() {
        let dir = temp_dir("invalid-enum");
        let path = dir.join(settings::FILE_NAME);
        fs::write(&path, br#"{"selection": "foo"}"#).unwrap();
        let loaded = load(None, Some(&dir), "linux");
        assert_eq!(loaded.state, FileState::Ok);
        assert_eq!(loaded.settings.selection, settings::NEURAL);
        assert!(
            loaded
                .warnings
                .iter()
                .any(|note| note.contains("selection")),
            "警告要指出欄位：{:?}",
            loaded.warnings
        );
        fs::remove_dir_all(&dir).ok();
    }

    /// CLI 覆寫優先於檔案。
    #[test]
    fn cli_overrides_win_over_file() {
        let settings = parsed(
            r#"{"selection": "statistical", "start_mode": "english", "mixed_input": false}"#,
        );
        let overrides = Overrides {
            mode: Some(Mode::Mixed),
            neural: Some(true),
        };
        let effective = effective(&settings, &overrides, "linux");
        assert_eq!(effective.mode, Mode::Mixed);
        assert!(effective.neural, "--neural on 蓋過 statistical");
        assert_eq!(effective.selection, "neural");
        assert_eq!(effective.ja_neural_mode, "auto");
        assert_eq!(
            effective.start_mode, "english",
            "start_mode 是檔案值，但不影響 CLI 已指定的模式"
        );
    }

    /// `--print-settings` 的 JSON 可解析，且與實際 session 設定一致。
    #[test]
    fn print_settings_json_matches_session_config() {
        let dir = temp_dir("print");
        let path = dir.join(settings::FILE_NAME);
        fs::write(
            &path,
            br#"{"mixed_input": true, "punctuation": true, "selection": "statistical"}"#,
        )
        .unwrap();
        let loaded = load(None, Some(&dir), "linux");
        let effective = effective(&loaded.settings, &Overrides::default(), "linux");
        let json = loaded.print_json(&effective).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&json).expect("輸出必須是可解析的 JSON");

        let config = session_config(SessionConfig::default(), &effective);
        assert_eq!(value["mode"], "mixed");
        assert_eq!(value["neural"], false);
        assert_eq!(value["selection"], "statistical");
        assert_eq!(value["ja_neural_mode"], "off");
        assert_eq!(value["punct_mark"], "，");
        assert_eq!(value["max_candidates"], config.max_candidates);
        assert_eq!(value["document_context"], config.document_context);
        assert_eq!(value["english_autocomplete"], config.english_autocomplete);
        assert_eq!(value["settings_state"], "ok");
        assert_eq!(
            value["settings_path"],
            path.to_string_lossy().as_ref(),
            "要指出讀的是哪一份檔案"
        );
        fs::remove_dir_all(&dir).ok();
    }

    /// 舊 `mode` 欄位遷移：一律 Mixed，`en` 另記成開機直接英文。
    #[test]
    fn legacy_mode_setting_starts_mixed_and_keeps_english_start() {
        for (legacy, start) in [
            ("zh", settings::START_ZHUYIN),
            ("ja", settings::START_ZHUYIN),
            ("en", settings::START_ENGLISH),
        ] {
            let settings = parsed(&format!(
                r#"{{"mode":"{legacy}","punctuation":true,"toggle_key":"right"}}"#
            ));
            let effective = effective(&settings, &Overrides::default(), "windows");
            assert_eq!(effective.mode, Mode::Mixed, "{legacy}");
            assert_eq!(effective.start_mode, start, "{legacy}");
            assert!(effective.punctuation, "{legacy} 的其他設定要保留");
            assert_eq!(effective.toggle_key, "right");
            let config = session_config(SessionConfig::default(), &effective);
            assert_eq!(config.mode, Mode::Mixed, "{legacy}");
            assert!(config.punct_mark.is_some(), "{legacy}");
        }
        // 明確的 start_mode 優先，不能被舊 mode=en 覆蓋。
        let settings = parsed(r#"{"mode":"en","start_mode":"zhuyin"}"#);
        let effective = effective(&settings, &Overrides::default(), "windows");
        assert_eq!(effective.start_mode, settings::START_ZHUYIN);
    }

    /// 混打開關已移除：中文使用者不論舊檔值或開始模式，都從 Mixed 啟動。
    #[test]
    fn chinese_settings_always_start_in_mixed_mode() {
        for body in [
            r#"{"selection":"statistical"}"#,
            r#"{"selection":"statistical","mixed_input":false}"#,
            r#"{"selection":"statistical","start_mode":"english"}"#,
        ] {
            let settings = parsed(body);
            let effective = effective(&settings, &Overrides::default(), "windows");
            assert_eq!(effective.mode, Mode::Mixed, "{body}");
            assert!(effective.mixed_input, "{body} 的舊欄位不得關閉混打");
            let config = session_config(SessionConfig::default(), &effective);
            assert!(config.style.mixed_input, "{body}");
        }
        let ja = parsed(r#"{"primary_language":"ja","mixed_input":false}"#);
        let ja = effective(&ja, &Overrides::default(), "windows");
        assert_eq!(ja.mode, Mode::Mixed);
        assert!(ja.ja_only, "日文主要語言改成 Mixed 但只走羅馬字");
    }
}
