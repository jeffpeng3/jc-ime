//! 語義按鍵。

use serde::{Deserialize, Serialize};

/// 按鍵的語義種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum KeyCode {
    /// 可列印字元。
    Char(char),
    /// 退格。
    Backspace,
    /// 換行或確認。
    Enter,
    /// 取消或關閉候選。
    Escape,
    /// 空白鍵。
    Space,
    /// 向左。
    ArrowLeft,
    /// 向右。
    ArrowRight,
    /// 向上。
    Up,
    /// 向下。
    Down,
    /// 移到最前面。
    Home,
    /// 移到最後面。
    End,
    /// 上一頁。
    PageUp,
    /// 下一頁。
    PageDown,
    /// Tab（補全或換段）。
    Tab,
    /// Shift+Tab（往回換段）。
    ShiftTab,
    /// 刪除。
    Delete,
    /// 功能鍵 F1–F24。
    F(u8),
}

/// 一個按鍵事件。
///
/// `text` 是按下這個鍵實際產生的字元；具名鍵為 `None`。
/// `shift` 記錄 Shift 是否按著，用來區分標點與大小寫語意。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Key {
    /// 語義鍵碼。
    pub code: KeyCode,
    /// 這個鍵產生的字元（若有）。
    pub text: Option<char>,
    /// Shift 是否按著。
    pub shift: bool,
    /// Ctrl 是否按著；`false` 時不寫進 JSON，維持舊協定形狀。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ctrl: bool,
}

impl Key {
    /// 建立按鍵事件。
    pub fn new(code: KeyCode, text: Option<char>, shift: bool) -> Self {
        Self {
            code,
            text,
            shift,
            ctrl: false,
        }
    }

    /// 建立可列印字元的按鍵。
    pub fn char(ch: char, shift: bool) -> Self {
        Self::new(KeyCode::Char(ch), Some(ch), shift)
    }

    /// 加上 Ctrl 修飾鍵。
    pub fn with_ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_key_carries_text_and_shift() {
        let k = Key::char('A', true);
        assert_eq!(k.code, KeyCode::Char('A'));
        assert_eq!(k.text, Some('A'));
        assert!(k.shift);
    }

    #[test]
    fn named_key_has_no_text() {
        let k = Key::new(KeyCode::Backspace, None, false);
        assert_eq!(k.code, KeyCode::Backspace);
        assert_eq!(k.text, None);
        assert!(!k.shift);
    }

    #[test]
    fn navigation_keys_use_snake_case_names() {
        let cases = [
            (KeyCode::Up, "up"),
            (KeyCode::Down, "down"),
            (KeyCode::Home, "home"),
            (KeyCode::End, "end"),
            (KeyCode::PageUp, "page_up"),
            (KeyCode::PageDown, "page_down"),
            (KeyCode::ShiftTab, "shift_tab"),
        ];
        for (code, name) in cases {
            let json = serde_json::to_string(&code).unwrap();
            assert_eq!(json, format!(r#"{{"kind":"{name}"}}"#), "{code:?}");
            assert_eq!(serde_json::from_str::<KeyCode>(&json).unwrap(), code);
        }
    }

    #[test]
    fn shift_tab_is_distinct_from_tab() {
        let tab = Key::new(KeyCode::Tab, None, false);
        let shift_tab = Key::new(KeyCode::ShiftTab, None, true);
        assert_ne!(tab.code, shift_tab.code);
        assert_ne!(tab, shift_tab);
    }
}
