//! 快捷鍵綁定：chord 語法、驗證與正規化。
//!
//! 綁定是「action → chord」的字串表，存在 `settings.json` 的 `[keybinds]`。
//! 引擎解析成正規化 chord 後放進 keymap；外殼只比對字串，不自己決定行為。

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// chord 解析或綁定驗證的錯誤。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChordError {
    /// 空字串。
    Empty,
    /// 只有按鍵，沒有任何修飾鍵。
    NoModifier,
    /// 只有 Shift；會吃掉一般大寫輸入。
    ShiftOnly,
    /// 修飾鍵或按鍵名稱不認得。
    UnknownKey {
        /// 原始片段。
        key: String,
    },
    /// 兩個 action 綁了同一個 chord。
    DuplicateChord {
        /// 正規化後的 chord。
        chord: String,
    },
}

impl fmt::Display for ChordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "快捷鍵不可為空"),
            Self::NoModifier => write!(f, "快捷鍵需要 Ctrl 或 Alt（不能是純按鍵）"),
            Self::ShiftOnly => {
                write!(f, "快捷鍵不能只用 Shift；請加上 Ctrl 或 Alt")
            }
            Self::UnknownKey { key } => write!(f, "不認得的按鍵 {key:?}；可用單一字元或 f1..f12"),
            Self::DuplicateChord { chord } => {
                write!(f, "同一個快捷鍵 {chord} 綁定了多個動作")
            }
        }
    }
}

impl std::error::Error for ChordError {}

/// 一個正規化後的按鍵組合。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Chord {
    /// Ctrl。
    pub ctrl: bool,
    /// Alt。
    pub alt: bool,
    /// Shift。
    pub shift: bool,
    /// 單一字元（小寫字母）或具名鍵（`f1`..`f12`、`enter` 等）。
    pub key: String,
}

impl Chord {
    /// 解析 `ctrl+alt+,` 這種字串；大小寫不敏感，且必須有 Ctrl 或 Alt。
    pub fn parse(text: &str) -> Result<Self, ChordError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ChordError::Empty);
        }
        let mut ctrl = false;
        let mut alt = false;
        let mut shift = false;
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        if parts.len() < 2 {
            return Err(ChordError::NoModifier);
        }
        for part in &parts[..parts.len() - 1] {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "alt" => alt = true,
                "shift" => shift = true,
                "" => return Err(ChordError::Empty),
                other => {
                    return Err(ChordError::UnknownKey {
                        key: other.to_string(),
                    })
                }
            }
        }
        if !(ctrl || alt || shift) {
            return Err(ChordError::NoModifier);
        }
        if !ctrl && !alt {
            return Err(ChordError::ShiftOnly);
        }
        let key = normalize_key(parts[parts.len() - 1])?;
        Ok(Self {
            ctrl,
            alt,
            shift,
            key,
        })
    }

    /// 正規化字串；修飾鍵固定依 ctrl、alt、shift 排序。
    pub fn as_str(&self) -> String {
        let mut out = String::new();
        if self.ctrl {
            out.push_str("ctrl+");
        }
        if self.alt {
            out.push_str("alt+");
        }
        if self.shift {
            out.push_str("shift+");
        }
        out.push_str(&self.key);
        out
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_str())
    }
}

/// 單一字元或具名鍵轉成正規化名稱。
fn normalize_key(raw: &str) -> Result<String, ChordError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(ChordError::Empty);
    }
    if raw.chars().count() == 1 {
        let ch = raw.chars().next().expect("已確認一個字元");
        return Ok(if ch.is_ascii_alphabetic() {
            ch.to_ascii_lowercase().to_string()
        } else {
            ch.to_string()
        });
    }
    let name = raw.to_ascii_lowercase();
    let known = matches!(
        name.as_str(),
        "f1" | "f2"
            | "f3"
            | "f4"
            | "f5"
            | "f6"
            | "f7"
            | "f8"
            | "f9"
            | "f10"
            | "f11"
            | "f12"
            | "enter"
            | "escape"
            | "backspace"
            | "tab"
            | "space"
            | "delete"
            | "home"
            | "end"
            | "pageup"
            | "pagedown"
            | "up"
            | "down"
            | "left"
            | "right"
    );
    if known {
        Ok(name)
    } else {
        Err(ChordError::UnknownKey {
            key: raw.to_string(),
        })
    }
}

fn default_symbol_menu() -> String {
    "ctrl+alt+,".to_string()
}

fn default_emoji_palette() -> String {
    "ctrl+alt+.".to_string()
}

/// 使用者的快捷鍵綁定。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keybinds {
    /// 開符號選單。
    #[serde(default = "default_symbol_menu")]
    pub symbol_menu: String,
    /// 開表情選單。
    #[serde(default = "default_emoji_palette")]
    pub emoji_palette: String,
}

impl Default for Keybinds {
    fn default() -> Self {
        Self {
            symbol_menu: default_symbol_menu(),
            emoji_palette: default_emoji_palette(),
        }
    }
}

impl Keybinds {
    /// 從 JSON 讀取；缺欄位或不是物件時用預設值。
    pub fn from_json(value: &serde_json::Value) -> Self {
        serde_json::from_value(value.clone()).unwrap_or_default()
    }

    /// 解析指定 action 的 chord。
    pub fn parse_chord(&self, action: &str) -> Result<Chord, ChordError> {
        let text = match action {
            "symbol_menu" => &self.symbol_menu,
            "emoji_palette" => &self.emoji_palette,
            other => {
                return Err(ChordError::UnknownKey {
                    key: other.to_string(),
                })
            }
        };
        Chord::parse(text)
    }

    /// 所有 action 的正規化 chord；重複綁定回錯誤。
    pub fn chords(&self) -> Result<BTreeMap<Chord, &'static str>, ChordError> {
        let mut out = BTreeMap::new();
        for (action, chord) in [
            ("symbol_menu", self.parse_chord("symbol_menu")?),
            ("emoji_palette", self.parse_chord("emoji_palette")?),
        ] {
            if out.insert(chord.clone(), action).is_some() {
                return Err(ChordError::DuplicateChord {
                    chord: chord.as_str(),
                });
            }
        }
        Ok(out)
    }

    /// action → 正規化 chord 字串；給協定的 `bindings` 使用。
    pub fn bindings(&self) -> Result<BTreeMap<String, String>, ChordError> {
        let mut out = BTreeMap::new();
        for (chord, action) in self.chords()? {
            out.insert(action.to_string(), chord.as_str());
        }
        Ok(out)
    }

    /// 修改一個 action 的綁定並驗證；失敗時不改內容。
    pub fn set(&mut self, action: &str, value: &str) -> Result<(), ChordError> {
        let chord = Chord::parse(value)?;
        let mut next = self.clone();
        match action {
            "symbol_menu" => next.symbol_menu = chord.as_str(),
            "emoji_palette" => next.emoji_palette = chord.as_str(),
            other => {
                return Err(ChordError::UnknownKey {
                    key: other.to_string(),
                })
            }
        }
        next.chords()?;
        *self = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bindings_parse() {
        let keybinds = Keybinds::default();
        assert_eq!(
            keybinds.parse_chord("symbol_menu").unwrap().as_str(),
            "ctrl+alt+,"
        );
        assert_eq!(
            keybinds.parse_chord("emoji_palette").unwrap().as_str(),
            "ctrl+alt+."
        );
        assert_eq!(keybinds.chords().unwrap().len(), 2);
    }

    #[test]
    fn parse_chord_is_case_insensitive_and_normalizes_order() {
        let chord = Chord::parse("ALT+Ctrl+Shift+F12").unwrap();
        assert_eq!(chord.as_str(), "ctrl+alt+shift+f12");
        let chord = Chord::parse("Shift+CTRL+A").unwrap();
        assert_eq!(chord.as_str(), "ctrl+shift+a");
    }

    #[test]
    fn chord_without_modifier_is_rejected() {
        assert_eq!(Chord::parse("a"), Err(ChordError::NoModifier));
        assert_eq!(Chord::parse("f1"), Err(ChordError::NoModifier));
        assert_eq!(Chord::parse(""), Err(ChordError::Empty));
    }

    #[test]
    fn shift_only_chord_is_rejected() {
        assert_eq!(Chord::parse("shift+a"), Err(ChordError::ShiftOnly));
        let mut keybinds = Keybinds::default();
        assert_eq!(
            keybinds.set("symbol_menu", "shift+a"),
            Err(ChordError::ShiftOnly)
        );
        assert_eq!(keybinds, Keybinds::default(), "失敗時不改內容");
    }

    #[test]
    fn unknown_key_is_rejected() {
        assert!(matches!(
            Chord::parse("ctrl+not-a-key"),
            Err(ChordError::UnknownKey { .. })
        ));
    }

    #[test]
    fn duplicate_bindings_are_rejected() {
        let mut keybinds = Keybinds::default();
        assert!(matches!(
            keybinds.set("emoji_palette", "ctrl+alt+,"),
            Err(ChordError::DuplicateChord { .. })
        ));
        assert_eq!(keybinds, Keybinds::default(), "失敗時不改內容");
    }

    #[test]
    fn missing_fields_use_defaults() {
        let keybinds = Keybinds::from_json(&serde_json::json!({}));
        assert_eq!(keybinds, Keybinds::default());
        let keybinds = Keybinds::from_json(&serde_json::json!({"symbol_menu": "ctrl+shift+s"}));
        assert_eq!(keybinds.symbol_menu, "ctrl+shift+s");
        assert_eq!(keybinds.emoji_palette, "ctrl+alt+.");
    }

    #[test]
    fn bindings_map_action_to_normalized_chord() {
        let bindings = Keybinds::default().bindings().unwrap();
        assert_eq!(bindings["symbol_menu"], "ctrl+alt+,");
        assert_eq!(bindings["emoji_palette"], "ctrl+alt+.");
    }

    #[test]
    fn settings_set_edits_and_round_trips_keybinds() {
        use crate::settings::Settings;

        let mut settings = Settings::default();
        settings
            .set("keybind-symbol-menu", "ctrl+shift+s", "linux")
            .unwrap();
        assert_eq!(settings.keybinds.symbol_menu, "ctrl+shift+s");
        let json = serde_json::to_value(&settings).unwrap();
        assert_eq!(json["keybinds"]["symbol_menu"], "ctrl+shift+s");
        assert_eq!(json["keybinds"]["emoji_palette"], "ctrl+alt+.");
        let parsed = Settings::from_json_for(json.to_string().as_bytes(), "linux").unwrap();
        assert_eq!(parsed.keybinds, settings.keybinds);

        assert!(settings
            .set("keybind-symbol-menu", "not-a-chord", "linux")
            .is_err());
        assert!(
            settings
                .set("keybind-symbol-menu", "shift+a", "linux")
                .is_err(),
            "Shift-only chord 會吃掉大寫字母"
        );
        assert!(
            settings
                .set("keybind-emoji-palette", "ctrl+shift+s", "linux")
                .is_err(),
            "重複綁定要拒絕"
        );
    }
}
