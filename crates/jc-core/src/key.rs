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
    /// Tab（補全或換段）。
    Tab,
    /// 刪除。
    Delete,
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
    /// Ctrl 是否按著。
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
}
