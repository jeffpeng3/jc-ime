//! 設定項目的顯示資料：標題、一行說明、控制項種類與預設值標記。
//!
//! 值本身一律經由 [`jc_settings::Settings::set`] 讀寫，這裡只放 UI 需要的
//! 中文標籤。預設值由 `Settings::default_for_platform` 取得，不另外複製一份。

use jc_settings::{
    Settings, JA_AMBIGUOUS_BOTH, JA_AMBIGUOUS_ENGLISH, JA_AMBIGUOUS_KANA, JA_INPUT_KANA,
    JA_INPUT_ROMAJI, NEURAL, START_ENGLISH, START_ZHUYIN, STATISTICAL, TOGGLE_BOTH, TOGGLE_LEFT,
    TOGGLE_NONE, TOGGLE_RIGHT,
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

const TOGGLE_CHOICES: &[Choice] = &[
    Choice {
        value: TOGGLE_RIGHT,
        label: "右 Shift",
    },
    Choice {
        value: TOGGLE_BOTH,
        label: "左右 Shift",
    },
    Choice {
        value: TOGGLE_LEFT,
        label: "左 Shift",
    },
    Choice {
        value: TOGGLE_NONE,
        label: "不使用",
    },
];

const START_CHOICES: &[Choice] = &[
    Choice {
        value: START_ZHUYIN,
        label: "注音",
    },
    Choice {
        value: START_ENGLISH,
        label: "英文",
    },
];

const JA_INPUT_CHOICES: &[Choice] = &[
    Choice {
        value: JA_INPUT_ROMAJI,
        label: "羅馬字",
    },
    Choice {
        value: JA_INPUT_KANA,
        label: "直接かな",
    },
];

const JA_AMBIGUOUS_CHOICES: &[Choice] = &[
    Choice {
        value: JA_AMBIGUOUS_BOTH,
        label: "並列",
    },
    Choice {
        value: JA_AMBIGUOUS_ENGLISH,
        label: "英文優先",
    },
    Choice {
        value: JA_AMBIGUOUS_KANA,
        label: "かな優先",
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
pub const JA: [Opt; 5] = [
    Opt {
        key: "ja-model",
        title: "神經模型（Jinen）",
        help: "轉換時使用神經かな漢字変換；本版引擎以「選字模型」為準，這個值會保留。",
        kind: Kind::Switch,
    },
    Opt {
        key: "ja-live-conversion",
        title: "逐字変換",
        help: "邊打邊轉換，不等空白鍵；本版尚未接上神經模型。",
        kind: Kind::Switch,
    },
    Opt {
        key: "ja-learning",
        title: "寫入日文個人詞庫",
        help: "選字結果記進個人詞庫，下次優先出現。",
        kind: Kind::Switch,
    },
    Opt {
        key: "ja-input-method",
        title: "輸入方式",
        help: "引擎目前只實作羅馬字；選直接かな只會存進設定檔。",
        kind: Kind::Choice(JA_INPUT_CHOICES),
    },
    Opt {
        key: "ja-ambiguous",
        title: "日英同形",
        help: "引擎目前固定日英並列；這個值會保留給之後的排序。",
        kind: Kind::Choice(JA_AMBIGUOUS_CHOICES),
    },
];

/// 輸入分頁。
pub const INPUT: [Opt; 5] = [
    Opt {
        key: "toggle-key",
        title: "中英切換鍵",
        help: "按一下切換中英模式；右 Shift 是 Linux 的預設。",
        kind: Kind::Choice(TOGGLE_CHOICES),
    },
    Opt {
        key: "start-mode",
        title: "啟動模式",
        help: "引擎啟動時先進入哪一種模式。",
        kind: Kind::Choice(START_CHOICES),
    },
    Opt {
        key: "esc-clear",
        title: "Esc 清除",
        help: "按 Esc 清掉整段組字，不用逐字刪。",
        kind: Kind::Switch,
    },
    Opt {
        key: "space-selects",
        title: "空白選字",
        help: "空白鍵選第一個候選；關閉時空白鍵只送出空白。",
        kind: Kind::Switch,
    },
    Opt {
        key: "mixed-input",
        title: "中英混打",
        help: "同一段組字裡直接打英文，不用先切模式。",
        kind: Kind::Switch,
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

/// 說明加上預設值標記。
pub fn description(opt: &Opt, defaults: &Settings) -> String {
    format!("{}　預設值：{}", opt.help, default_label(opt, defaults))
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
    fn test_linux_defaults_are_right_shift_and_zhuyin() {
        let defaults = Settings::default_for_platform("linux");
        assert_eq!(
            defaults.value("toggle-key").as_deref(),
            Some(TOGGLE_RIGHT),
            "Linux 預設用右 Shift 切換"
        );
        assert_eq!(
            defaults.value("start-mode").as_deref(),
            Some(START_ZHUYIN),
            "預設從注音開始"
        );
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
