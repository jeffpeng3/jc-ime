//! 輸入習慣的規則項目：鍵、允許值與讀寫。
//!
//! 對照前身 `internal/settings/profile_rules.go`。規則本身是
//! [`jc_core::inputstyle::Rules`]，這裡只補上設定檔用的鍵與顯示名稱。

use jc_core::inputstyle::Rules;

use crate::settings::{SetError, Settings, SettingsError};

/// 一條規則項目的定義。
#[derive(Clone, Copy)]
pub(crate) struct RuleSpec {
    /// 設定鍵。
    pub(crate) key: &'static str,
    /// 允許值。
    pub(crate) values: &'static [&'static str],
    /// 值的顯示名稱；空清單表示直接用值。
    pub(crate) labels: &'static [(&'static str, &'static str)],
    /// 讀取規則目前的值。
    pub(crate) get: fn(&Rules) -> String,
    /// 寫入規則。
    pub(crate) set: fn(&mut Rules, &str),
}

pub(crate) fn rule_specs() -> Vec<RuleSpec> {
    vec![
        RuleSpec {
            key: "style-shift-space",
            values: &["disabled", "fullwidth", "space", "single"],
            labels: &[
                ("disabled", "不使用"),
                ("fullwidth", "切換全半形"),
                ("space", "插入空白"),
                ("single", "開單字候選"),
            ],
            get: |rules| rules.shift_space.clone(),
            set: |rules, value| rules.shift_space = value.to_string(),
        },
        RuleSpec {
            key: "style-tab",
            values: &["0c", "cycle", "segment"],
            labels: &[
                ("0c", "靈犀原有操作"),
                ("cycle", "輪換字詞"),
                ("segment", "調整斷詞"),
            ],
            get: |rules| rules.tab.clone(),
            set: |rules, value| rules.tab = value.to_string(),
        },
        RuleSpec {
            key: "style-up",
            values: &["0c", "cycle"],
            labels: &[("0c", "靈犀原有操作"), ("cycle", "輪換字詞")],
            get: |rules| rules.up.clone(),
            set: |rules, value| rules.up = value.to_string(),
        },
        RuleSpec {
            key: "style-candidate-tab",
            values: &["length", "grid", "next"],
            labels: &[
                ("length", "切換字詞長度"),
                ("grid", "展開多排候選"),
                ("next", "移到下一個候選"),
            ],
            get: |rules| rules.candidate_tab.clone(),
            set: |rules, value| rules.candidate_tab = value.to_string(),
        },
        RuleSpec {
            key: "style-candidate-space",
            values: &["page", "select"],
            labels: &[("page", "下一頁"), ("select", "選取候選字")],
            get: |rules| rules.candidate_space.clone(),
            set: |rules, value| rules.candidate_space = value.to_string(),
        },
        RuleSpec {
            key: "style-candidate-enter",
            values: &["select", "commit"],
            labels: &[("select", "選字並保留組字"), ("commit", "選字並送出")],
            get: |rules| rules.candidate_enter.clone(),
            set: |rules, value| rules.candidate_enter = value.to_string(),
        },
        RuleSpec {
            key: "style-page-size",
            values: &["9", "10"],
            labels: &[],
            get: |rules| rules.candidate_page_size.to_string(),
            set: |rules, value| {
                rules.candidate_page_size = value.parse().unwrap_or(rules.candidate_page_size);
            },
        },
        RuleSpec {
            key: "style-punctuation",
            values: &["0c", "microsoft", "standard", "chewing"],
            labels: &[
                ("0c", "靈犀"),
                ("microsoft", "微軟參考"),
                ("standard", "小麥參考"),
                ("chewing", "新酷音參考"),
            ],
            get: |rules| rules.punctuation.clone(),
            set: |rules, value| rules.punctuation = value.to_string(),
        },
        RuleSpec {
            key: "style-shift-letter",
            values: &["inline", "commit", "lower"],
            labels: &[
                ("inline", "留在組字內"),
                ("commit", "送出組字並輸入英文"),
                ("lower", "打小寫字母"),
            ],
            get: |rules| rules.shift_letter.clone(),
            set: |rules, value| rules.shift_letter = value.to_string(),
        },
        RuleSpec {
            key: "style-shift-delete",
            values: &["0c", "tone"],
            labels: &[("0c", "靈犀原有操作"), ("tone", "退回聲調")],
            get: |rules| rules.shift_delete.clone(),
            set: |rules, value| rules.shift_delete = value.to_string(),
        },
        RuleSpec {
            key: "style-caps-letter",
            values: &["platform", "lower"],
            labels: &[
                ("platform", "依平台大小寫"),
                ("lower", "預設小寫，Shift 大寫"),
            ],
            get: |rules| rules.caps_letter.clone(),
            set: |rules, value| rules.caps_letter = value.to_string(),
        },
        RuleSpec {
            key: "style-candidate-navigation",
            values: &["0c", "horizontal"],
            labels: &[("0c", "依候選排列"), ("horizontal", "水平操作")],
            get: |rules| rules.candidate_navigation.clone(),
            set: |rules, value| rules.candidate_navigation = value.to_string(),
        },
    ]
}

pub(crate) fn lookup_rule(key: &str) -> Option<RuleSpec> {
    rule_specs().into_iter().find(|spec| spec.key == key)
}

/// 規則值的顯示名稱；沒有對照表時回傳原值。
pub(crate) fn rule_value_label(key: &str, value: &str) -> String {
    let Some(spec) = lookup_rule(key) else {
        return value.to_string();
    };
    spec.labels
        .iter()
        .find(|(candidate, _)| *candidate == value)
        .map(|(_, label)| (*label).to_string())
        .unwrap_or_else(|| value.to_string())
}

/// 設定項目的顯示名稱，用於衝突訊息。
pub(crate) fn feature_label(key: &str) -> &'static str {
    match key {
        "english-autocomplete" => "英文模式自動補字",
        "toneless" => "不打聲調也能選字",
        _ => "設定",
    }
}

impl Settings {
    /// 修改目前輸入習慣的一條規則；不是規則鍵時回傳 `false`。
    pub(crate) fn set_rule(&mut self, key: &str, value: &str) -> Result<bool, SettingsError> {
        let Some(spec) = lookup_rule(key) else {
            return Ok(false);
        };
        let mut store = self.profile_store();
        for profile in &mut store.items {
            if profile.id == store.active {
                (spec.set)(&mut profile.rules, value);
                profile
                    .rules
                    .validate()
                    .map_err(|err| SetError::Message(err.to_string()))?;
            }
        }
        self.put_profile_store(store);
        Ok(true)
    }
}
