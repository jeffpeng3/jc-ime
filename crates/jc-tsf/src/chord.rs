//! 快捷鍵 chord 的正規化（Linux 也編、也測）。
//!
//! 引擎與 TSF 必須對同一個組合鍵產生相同的字串：修飾鍵固定依 ctrl、alt、
//! shift 排序，ASCII 字母轉小寫，其他字元原樣保留。AltGr 在 Windows 上是
//! Ctrl＋Alt，所以 `Ctrl+Alt+,` 正規化後就是 `ctrl+alt+,`。

/// 單一按鍵名稱的 chord 正規化：ASCII 字母轉小寫，其餘原樣。
pub fn normalize_key(key: &str) -> String {
    if key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic() {
        key.to_ascii_lowercase()
    } else {
        key.to_string()
    }
}

/// 修飾鍵與按鍵名稱組成 chord；沒有修飾鍵或鍵名是空字串時回 `None`。
pub fn chord_string(ctrl: bool, alt: bool, shift: bool, key: &str) -> Option<String> {
    if !(ctrl || alt || shift) || key.is_empty() {
        return None;
    }
    let mut out = String::new();
    if ctrl {
        out.push_str("ctrl+");
    }
    if alt {
        out.push_str("alt+");
    }
    if shift {
        out.push_str("shift+");
    }
    out.push_str(&normalize_key(key));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn altgr_comma_normalizes_like_the_engine() {
        // AltGr 是 Ctrl＋Alt；錄製的 chord 與引擎回的 bindings 都是這個字串。
        assert_eq!(
            chord_string(true, true, false, ",").as_deref(),
            Some("ctrl+alt+,")
        );
        assert_eq!(
            chord_string(true, true, false, ".").as_deref(),
            Some("ctrl+alt+.")
        );
    }

    #[test]
    fn letters_are_lowercase_and_modifier_order_is_fixed() {
        assert_eq!(
            chord_string(false, true, true, "S").as_deref(),
            Some("alt+shift+s")
        );
        assert_eq!(
            chord_string(true, false, true, "?").as_deref(),
            Some("ctrl+shift+?")
        );
        assert_eq!(
            chord_string(true, true, true, "a").as_deref(),
            Some("ctrl+alt+shift+a")
        );
    }

    #[test]
    fn no_modifier_or_empty_key_is_not_a_chord() {
        assert_eq!(chord_string(false, false, false, ","), None);
        assert_eq!(chord_string(true, true, false, ""), None);
    }
}
