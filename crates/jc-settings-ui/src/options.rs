//! 設定項目的顯示資料：標題、一行說明、控制項種類與預設值標記。
//!
//! 值本身一律經由 [`jc_settings::Settings::set`] 讀寫，這裡只放 UI 需要的
//! 中文標籤。預設值由 `Settings::default_for_platform` 取得，不另外複製一份。

use jc_settings::{
    Settings, JA_NEURAL_AUTO, JA_NEURAL_OFF, JA_NEURAL_QUALITY, NEURAL, STATISTICAL,
};

/// 下拉選項：設定檔內的值與畫面文字。
#[derive(Clone, Copy)]
pub struct Choice {
    /// 寫進 `settings.json` 的值。
    pub value: &'static str,
    /// 畫面文字。
    pub label: &'static str,
}

/// 控制項種類。
pub enum Kind {
    /// 開／關。
    Switch,
    /// 固定選項。
    Choice(&'static [Choice]),
    /// 文字輸入；值由 `Settings::set` 驗證。
    Text,
}

/// 一個可編輯的設定項目。
pub struct Opt {
    /// `jc-settings` 的設定鍵。
    pub key: &'static str,
    /// 項目標題。
    pub title: &'static str,
    /// 一行說明。
    pub help: &'static str,
    /// 控制項種類。
    pub kind: Kind,
}

const FUZZY_CHOICES: &[Choice] = &[
    Choice {
        value: "off",
        label: "不啟用",
    },
    Choice {
        value: "select",
        label: "只在選字時",
    },
    Choice {
        value: "auto",
        label: "自動",
    },
];

const SELECTION_CHOICES: &[Choice] = &[
    Choice {
        value: NEURAL,
        label: "神經模型",
    },
    Choice {
        value: STATISTICAL,
        label: "統計",
    },
];

const JA_NEURAL_CHOICES: &[Choice] = &[
    Choice {
        value: JA_NEURAL_AUTO,
        label: "自動",
    },
    Choice {
        value: JA_NEURAL_QUALITY,
        label: "品質優先",
    },
    Choice {
        value: JA_NEURAL_OFF,
        label: "關閉",
    },
];

/// 注音分頁：模糊音。
pub const FUZZY: [Opt; 4] = [
    Opt {
        key: "fuzzy-z-zh",
        title: "ㄗ／ㄓ 不分",
        help: "打 ㄗ 也能選到 ㄓ 的字，只在需要時放寬。",
        kind: Kind::Choice(FUZZY_CHOICES),
    },
    Opt {
        key: "fuzzy-c-ch",
        title: "ㄘ／ㄔ 不分",
        help: "打 ㄘ 也能選到 ㄔ 的字。",
        kind: Kind::Choice(FUZZY_CHOICES),
    },
    Opt {
        key: "fuzzy-s-sh",
        title: "ㄙ／ㄕ 不分",
        help: "打 ㄙ 也能選到 ㄕ 的字。",
        kind: Kind::Choice(FUZZY_CHOICES),
    },
    Opt {
        key: "fuzzy-en-eng",
        title: "ㄣ／ㄥ 不分",
        help: "不打韻尾差異也能選字，適合快速輸入。",
        kind: Kind::Choice(FUZZY_CHOICES),
    },
];

/// 注音分頁：學習與選字模型。
pub const ZH: [Opt; 2] = [
    Opt {
        key: "selection",
        title: "選字模型",
        help: "神經模型會參考前後文重排候選；沒下載權重時自動退回統計。",
        kind: Kind::Choice(SELECTION_CHOICES),
    },
    Opt {
        key: "document-context",
        title: "學習",
        help: "依剛提交的文字調整後續選字。",
        kind: Kind::Switch,
    },
];

/// 日文分頁。
///
/// `ja-model`、`ja-rerank`、`ja-input-method` 與 `ja-ambiguous` 已從 UI 移除：
/// 引擎不讀這些值或已標棄用。JSON 欄位保留給舊檔相容，只是不再顯示。
pub const JA: [Opt; 3] = [
    Opt {
        key: "ja-neural-mode",
        title: "神經轉換模式",
        help: "自動：差距小才呼叫模型；品質優先：放寬呼叫門檻；關閉：只走統計。",
        kind: Kind::Choice(JA_NEURAL_CHOICES),
    },
    Opt {
        key: "ja-live-conversion",
        title: "逐字変換",
        help: "邊打邊轉換，不等空白鍵；神經補正由《神經轉換模式》控制。",
        kind: Kind::Switch,
    },
    Opt {
        key: "ja-learning",
        title: "寫入日文個人詞庫",
        help: "選字結果記進個人詞庫，下次優先出現。",
        kind: Kind::Switch,
    },
];

/// 輸入分頁。
///
/// 中英切換固定為單按左／右 Shift，不再提供切換鍵、啟動模式與 Esc 清除開關；
/// 對應的 JSON 欄位保留給舊檔相容。
pub const INPUT: [Opt; 3] = [
    Opt {
        key: "space-selects",
        title: "空白鍵",
        help: "空白只插入空格；候選由 ↓ 開啟。",
        kind: Kind::Switch,
    },
    Opt {
        key: "keybind-symbol-menu",
        title: "符號選單快捷鍵",
        help: "例如 ctrl+alt+,；必須有 Ctrl 或 Alt，儲存後引擎會熱重載。",
        kind: Kind::Text,
    },
    Opt {
        key: "keybind-emoji-palette",
        title: "表情選單快捷鍵",
        help: "例如 ctrl+alt+.；兩個動作不能綁同一個組合。",
        kind: Kind::Text,
    },
];

/// 把預設值翻成畫面文字。
pub fn default_label(opt: &Opt, defaults: &Settings) -> String {
    let value = defaults.value(opt.key).unwrap_or_default();
    match &opt.kind {
        Kind::Switch => switch_label(&value).to_string(),
        Kind::Choice(choices) => choices
            .iter()
            .find(|choice| choice.value == value)
            .map(|choice| choice.label.to_string())
            .unwrap_or(value),
        Kind::Text => value,
    }
}

/// 開／關的畫面文字。
pub fn switch_label(value: &str) -> &'static str {
    if value == "on" {
        "開"
    } else {
        "關"
    }
}

/// 說明加上預設值標記；「預設值：」放在獨立一行。
///
/// `SettingItem::description` 收的是 GPUI `Text`，`Text` 的排版會把 `\n` 當成
/// 換行，因此不需要另外拆成兩個元件。
pub fn description(opt: &Opt, defaults: &Settings) -> String {
    format!("{}\n預設值：{}", opt.help, default_label(opt, defaults))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc_settings::Settings;

    #[test]
    fn test_every_option_exists_in_jc_settings() {
        let keys = jc_settings::keys();
        for opt in FUZZY
            .iter()
            .chain(ZH.iter())
            .chain(JA.iter())
            .chain(INPUT.iter())
        {
            assert!(
                keys.contains_key(opt.key),
                "{} 不是 jc-settings 認得的設定鍵",
                opt.key
            );
        }
    }

    #[test]
    fn test_mixed_input_is_not_exposed_in_input_page() {
        assert!(
            INPUT.iter().all(|opt| opt.key != "mixed-input"),
            "混打已移除開關，輸入頁不能再出現 mixed-input"
        );
    }

    #[test]
    fn test_default_label_is_never_empty() {
        let defaults = Settings::default_for_platform("linux");
        for opt in FUZZY
            .iter()
            .chain(ZH.iter())
            .chain(JA.iter())
            .chain(INPUT.iter())
        {
            assert!(
                !default_label(opt, &defaults).is_empty(),
                "{} 沒有預設值標記",
                opt.key
            );
            assert!(description(opt, &defaults).contains("預設值："));
        }
    }

    #[test]
    fn test_description_puts_default_on_its_own_line() {
        let defaults = Settings::default_for_platform("linux");
        for opt in FUZZY
            .iter()
            .chain(ZH.iter())
            .chain(JA.iter())
            .chain(INPUT.iter())
        {
            let description = description(opt, &defaults);
            let expected = format!("{}\n預設值：{}", opt.help, default_label(opt, &defaults));
            assert_eq!(description, expected, "{} 的說明排版不符合預期", opt.key);
            assert!(
                !description.contains('　'),
                "{} 仍用全形空格接同一行",
                opt.key
            );
        }
    }

    #[test]
    fn test_linux_defaults_are_kept_for_legacy_files() {
        let defaults = Settings::default_for_platform("linux");
        assert_eq!(
            defaults.value("toggle-key").as_deref(),
            Some("right"),
            "舊檔的 toggle-key 預設值仍可查詢"
        );
        assert_eq!(
            defaults.value("start-mode").as_deref(),
            Some("zhuyin"),
            "舊檔的 start-mode 預設值仍可查詢"
        );
    }

    #[test]
    fn test_retired_options_are_not_exposed_in_ui() {
        let retired = [
            "toggle-key",
            "start-mode",
            "esc-clear",
            "ja-model",
            "ja-rerank",
            "ja-input-method",
            "ja-ambiguous",
            "mode-memory",
        ];
        for opt in FUZZY
            .iter()
            .chain(ZH.iter())
            .chain(JA.iter())
            .chain(INPUT.iter())
        {
            assert!(!retired.contains(&opt.key), "{} 已從 UI 移除", opt.key);
        }
    }

    #[test]
    fn test_choice_values_are_accepted_by_settings() {
        let keys = jc_settings::keys();
        for opt in FUZZY
            .iter()
            .chain(ZH.iter())
            .chain(JA.iter())
            .chain(INPUT.iter())
        {
            let Kind::Choice(choices) = &opt.kind else {
                continue;
            };
            let allowed = keys.get(opt.key).cloned().unwrap_or_default();
            for choice in *choices {
                assert!(
                    allowed.split('|').any(|value| value == choice.value),
                    "{} 不接受 {}（可用：{}）",
                    opt.key,
                    choice.value,
                    allowed
                );
            }
        }
    }
}
