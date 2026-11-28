//! 版本化、可序列化的輸入習慣。
//!
//! 對照前身 Go `core/inputstyle`。這個 module 只管資料與驗證，不含任何平台
//! I/O；外殼與核心共用同一份不可變的預設目錄。

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// `Rules` 的格式版本。
pub const VERSION: i32 = 1;

/// 輸入風格規則的驗證錯誤。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InputStyleError {
    /// 候選頁面大小不是 9 或 10。
    #[error("invalid candidate page size {0}")]
    CandidatePageSize(i32),
    /// 規則版本不是 [`VERSION`]。
    #[error("unsupported input style rules version {0}")]
    Version(i32),
    /// 某個欄位的值不在允許清單內。
    #[error("invalid input style {key}: \"{value}\"")]
    Value {
        /// 欄位名稱。
        key: &'static str,
        /// 實際值。
        value: String,
    },
}

/// 舊設定無法表達的行為。
///
/// 全零的 `Rules` 保留舊呼叫端既有的引擎行為。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rules {
    /// 候選清單的移動方式。
    #[serde(default)]
    pub candidate_navigation: String,
    /// Shift+Delete 的行為。
    #[serde(default)]
    pub shift_delete: String,
    /// 上鍵的行為。
    #[serde(default)]
    pub up: String,
    /// 候選清單的 Tab 行為。
    #[serde(default)]
    pub candidate_tab: String,
    /// 候選頁面大小。
    #[serde(default)]
    pub candidate_page_size: i32,
    /// 規則版本。
    #[serde(default)]
    pub version: i32,
    /// 標點寬度風格。
    #[serde(default)]
    pub punctuation: String,
    /// Shift+Space 的行為。
    #[serde(default)]
    pub shift_space: String,
    /// Tab 的行為。
    #[serde(default)]
    pub tab: String,
    /// 候選清單的空白鍵行為。
    #[serde(default)]
    pub candidate_space: String,
    /// 候選清單的 Enter 行為。
    #[serde(default)]
    pub candidate_enter: String,
    /// Shift+字母的行為。
    #[serde(default)]
    pub shift_letter: String,
}

impl Rules {
    /// 驗證規則；全零的規則視為合法。
    pub fn validate(&self) -> Result<(), InputStyleError> {
        if *self == Rules::default() {
            return Ok(());
        }
        if self.candidate_page_size != 9 && self.candidate_page_size != 10 {
            return Err(InputStyleError::CandidatePageSize(self.candidate_page_size));
        }
        if self.version != VERSION {
            return Err(InputStyleError::Version(self.version));
        }
        let checks: [(&'static str, &str, &[&str]); 10] = [
            (
                "candidate_navigation",
                &self.candidate_navigation,
                &["0c", "horizontal"],
            ),
            ("shift_delete", &self.shift_delete, &["0c", "tone"]),
            ("up", &self.up, &["0c", "cycle"]),
            (
                "candidate_tab",
                &self.candidate_tab,
                &["length", "grid", "next"],
            ),
            (
                "punctuation",
                &self.punctuation,
                &["0c", "microsoft", "standard", "chewing"],
            ),
            (
                "shift_space",
                &self.shift_space,
                &["disabled", "fullwidth", "space", "single"],
            ),
            ("tab", &self.tab, &["0c", "cycle", "segment"]),
            (
                "candidate_space",
                &self.candidate_space,
                &["page", "select"],
            ),
            (
                "candidate_enter",
                &self.candidate_enter,
                &["select", "commit"],
            ),
            (
                "shift_letter",
                &self.shift_letter,
                &["inline", "commit", "lower"],
            ),
        ];
        for (key, value, allowed) in checks {
            if !allowed.contains(&value) {
                return Err(InputStyleError::Value {
                    key,
                    value: value.to_string(),
                });
            }
        }
        Ok(())
    }
}

/// 由風格管理的舊設定完整快照。
///
/// 保留純量值，讓設定快照可以直接比較。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Behavior {
    /// 切換中英文的按鍵。
    #[serde(default)]
    pub toggle_key: String,
    /// 模式記憶範圍。
    #[serde(default)]
    pub mode_memory: String,
    /// 啟動時的模式。
    #[serde(default)]
    pub start_mode: String,
    /// Esc 是否清空組字。
    #[serde(default)]
    pub esc_clear: bool,
    /// 空白鍵是否選取候選。
    #[serde(default)]
    pub space_selects: bool,
    /// 選取後是否前進。
    #[serde(default)]
    pub advance_after_select: bool,
    /// 候選游標的位置。
    #[serde(default)]
    pub candidate_cursor: String,
    /// 候選清單的排列。
    #[serde(default)]
    pub candidate_layout: String,
    /// 選取候選的按鍵。
    #[serde(default)]
    pub selection_keys: String,
    /// 是否開啟混打。
    #[serde(default)]
    pub mixed_input: bool,
    /// 是否開啟英文自動補完。
    #[serde(default)]
    pub english_autocomplete: bool,
    /// Emoji 捷徑。
    #[serde(default)]
    pub emoji_shortcut: String,
    /// `=` 是否先開 Emoji。
    #[serde(default)]
    pub emoji_equals_first: bool,
    /// Ctrl 加標點是否輸入全形標點。
    #[serde(default)]
    pub ctrl_punctuation: bool,
    /// 是否接受無聲調輸入。
    #[serde(default)]
    pub toneless: bool,
}

/// 一組具名的輸入風格。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preset {
    /// 風格代號。
    #[serde(default)]
    pub id: String,
    /// 顯示名稱。
    #[serde(default)]
    pub name: String,
    /// 行為規則。
    #[serde(default)]
    pub rules: Rules,
    /// 舊設定快照。
    #[serde(default)]
    pub behavior: Behavior,
}

/// 取得 Linux 上的預設。
pub fn get(id: &str) -> Option<Preset> {
    get_for_platform(id, "linux")
}

/// 依平台取得預設。
///
/// 平台只影響切換鍵、模式記憶與候選游標的預設值；不認得的平台沿用
/// Linux 的預設。
pub fn get_for_platform(id: &str, goos: &str) -> Option<Preset> {
    let mut behavior = Behavior {
        toggle_key: "right".to_string(),
        mode_memory: "native".to_string(),
        start_mode: "zhuyin".to_string(),
        esc_clear: true,
        space_selects: false,
        candidate_cursor: "after".to_string(),
        candidate_layout: "vertical".to_string(),
        selection_keys: "1234567890".to_string(),
        emoji_shortcut: "equals".to_string(),
        emoji_equals_first: true,
        ctrl_punctuation: true,
        ..Behavior::default()
    };
    if goos == "windows" {
        behavior.toggle_key = "both".to_string();
    }
    if goos == "darwin" {
        behavior.toggle_key = "none".to_string();
        behavior.mode_memory = "shared".to_string();
        behavior.candidate_cursor = "before".to_string();
    }
    let rules = Rules {
        version: VERSION,
        candidate_navigation: "0c".to_string(),
        shift_delete: "0c".to_string(),
        up: "0c".to_string(),
        candidate_tab: "length".to_string(),
        candidate_page_size: 10,
        punctuation: "0c".to_string(),
        shift_space: "disabled".to_string(),
        tab: "0c".to_string(),
        candidate_space: "page".to_string(),
        candidate_enter: "select".to_string(),
        shift_letter: "inline".to_string(),
    };
    let mut preset = Preset {
        id: id.to_string(),
        name: String::new(),
        rules,
        behavior,
    };
    match id {
        "0c" => preset.name = "靈犀注音".to_string(),
        "microsoft" => {
            preset.name = "微軟新注音".to_string();
            preset.behavior.toggle_key = "both".to_string();
            preset.behavior.emoji_shortcut = "off".to_string();
            preset.rules.punctuation = "microsoft".to_string();
            preset.rules.shift_space = "fullwidth".to_string();
            preset.rules.up = "cycle".to_string();
            preset.rules.candidate_tab = "grid".to_string();
        }
        "chewing" => {
            preset.name = "新酷音".to_string();
            preset.behavior.candidate_cursor = "before".to_string();
            preset.behavior.candidate_layout = "horizontal".to_string();
            preset.behavior.emoji_shortcut = "off".to_string();
            preset.rules.tab = "segment".to_string();
            preset.rules.punctuation = "chewing".to_string();
            preset.rules.candidate_navigation = "horizontal".to_string();
            preset.rules.shift_space = "fullwidth".to_string();
        }
        "mcbopomofo" => {
            preset.name = "小麥注音".to_string();
            preset.behavior.toggle_key = "none".to_string();
            preset.behavior.candidate_cursor = "before".to_string();
            preset.behavior.esc_clear = false;
            preset.behavior.emoji_shortcut = "off".to_string();
            preset.rules.punctuation = "standard".to_string();
            preset.rules.tab = "cycle".to_string();
            preset.rules.shift_space = "space".to_string();
            preset.rules.shift_letter = "commit".to_string();
            preset.rules.candidate_page_size = 9;
        }
        "macos" => {
            preset.name = "macOS 內建注音".to_string();
            preset.behavior.toggle_key = "none".to_string();
            preset.behavior.candidate_cursor = "before".to_string();
            preset.behavior.emoji_shortcut = "off".to_string();
            preset.rules.shift_delete = "tone".to_string();
        }
        _ => return None,
    }
    Some(preset)
}

/// 列出 Linux 上的全部預設。
pub fn presets() -> Vec<Preset> {
    presets_for_platform("linux")
}

/// 列出指定平台的全部預設。
pub fn presets_for_platform(goos: &str) -> Vec<Preset> {
    ["0c", "microsoft", "chewing", "mcbopomofo", "macos"]
        .iter()
        .filter_map(|id| get_for_platform(id, goos))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_catalog_and_copy_are_independent() {
        for goos in ["linux", "windows", "darwin"] {
            for preset in presets_for_platform(goos) {
                preset.rules.validate().expect("預設規則應合法");
                let data = serde_json::to_vec(&preset).expect("序列化");
                let mut copied: Preset = serde_json::from_slice(&data).expect("反序列化");
                assert_eq!(copied, preset, "{} 的複本不完整", preset.id);
                copied.rules.tab = "cycle".to_string();
                let again = get_for_platform(&preset.id, goos).expect("預設存在");
                assert_eq!(again, preset, "改動複本不能污染目錄");
            }
        }
        assert!(get("asus").is_none(), "不支援的廠牌");
    }

    #[test]
    fn test_invalid_rules_are_rejected() {
        let preset = get("0c").expect("0c 存在");
        let mut bad_tab = preset.rules.clone();
        bad_tab.tab = "unknown".to_string();
        for rules in [
            Rules {
                version: 1,
                ..Rules::default()
            },
            Rules {
                version: 2,
                ..Rules::default()
            },
            bad_tab,
        ] {
            assert!(rules.validate().is_err(), "不合法規則被接受：{rules:?}");
        }
        assert!((Rules::default()).validate().is_ok(), "全零規則沿用舊行為");
    }

    #[test]
    fn test_shift_letter_lower_is_valid() {
        let mut preset = get("0c").expect("0c 存在");
        assert_eq!(preset.rules.shift_letter, "inline", "預設值被改動");
        preset.rules.shift_letter = "lower".to_string();
        assert!(preset.rules.validate().is_ok());
        preset.rules.shift_letter = "upper".to_string();
        assert!(
            preset.rules.validate().is_err(),
            "未知的 shift_letter 被接受"
        );
    }
}
