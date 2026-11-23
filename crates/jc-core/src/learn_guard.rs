//! 內容側的學習守門。
//!
//! 欄位層的 private 旗標只能擋住外殼判定得到的私密欄位；使用者也可能在一般
//! 欄位輸入電話、身分證、卡號或網址。這裡是第二道防線：學習入口在寫入個人
//! 資料前先問 [`is_sensitive`]，寧可誤擋，不要漏擋。
//!
//! 守門只看內容，不修改任何欄位判定；被擋下的內容只記 debug log，不影響輸入。

use crate::zh::keyboard::Keyboard;
use crate::zh::syllable::{class_of, Class};

/// 判斷一段文字是否不該寫進個人學習資料。
///
/// 規則（任一成立就回傳 `true`）：
///
/// - 空字串或全空白；
/// - 5 位以上連續數字；
/// - email：含 `@`，且前後各有一個英數字；
/// - URL：含 `http://`、`https://` 或 `www.`（不分大小寫）；
/// - 信用卡格式：只由數字、空白與 dash 組成，數字 13 到 19 位；
/// - 台灣身分證格式：1 個英文字母加 9 位數字；
/// - 純英數混合且長度至少 16，且不像注音按鍵序列。
///
/// 注音按鍵序列的豁免是為了避免把「整串按鍵」誤判成帳號；真正的注音輸入
/// 送出的是漢字，不會走到這裡。
pub fn is_sensitive(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }
    if has_digit_run(trimmed, 5) {
        return true;
    }
    if has_email(trimmed) {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("http://") || lower.contains("https://") || lower.contains("www.") {
        return true;
    }
    if is_card_number(trimmed) {
        return true;
    }
    if is_taiwan_id(trimmed) {
        return true;
    }
    is_long_alnum_mixed(trimmed) && !looks_like_bopomofo_keys(trimmed)
}

/// 是否有至少 `run` 個連續 ASCII 數字。
fn has_digit_run(text: &str, run: usize) -> bool {
    let mut count = 0;
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            count += 1;
            if count >= run {
                return true;
            }
        } else {
            count = 0;
        }
    }
    false
}

/// `@` 的兩側是否都是英數字。
fn has_email(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    chars.iter().enumerate().any(|(at, ch)| {
        *ch == '@'
            && at > 0
            && chars.get(at + 1).is_some()
            && chars[at - 1].is_ascii_alphanumeric()
            && chars[at + 1].is_ascii_alphanumeric()
    })
}

/// 只由數字、空白與 dash 組成，且數字 13 到 19 位。
fn is_card_number(text: &str) -> bool {
    let mut digits = 0;
    let mut allowed = true;
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            digits += 1;
        } else if ch.is_ascii_whitespace() || ch == '-' {
            continue;
        } else {
            allowed = false;
            break;
        }
    }
    allowed && (13..=19).contains(&digits)
}

/// 1 個英文字母加 9 位數字，整串剛好 10 個字元。
fn is_taiwan_id(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    let rest: Vec<char> = chars.collect();
    rest.len() == 9 && rest.iter().all(char::is_ascii_digit)
}

/// 純英數、同時有字母與數字、長度至少 16。
fn is_long_alnum_mixed(text: &str) -> bool {
    if text.chars().count() < 16 {
        return false;
    }
    let mut letters = false;
    let mut digits = false;
    for ch in text.chars() {
        if ch.is_ascii_alphabetic() {
            letters = true;
        } else if ch.is_ascii_digit() {
            digits = true;
        } else {
            return false;
        }
    }
    letters && digits
}

/// 整串是否像標準（大千）排列的注音按鍵序列。
///
/// 必須全部是注音鍵，以聲調鍵或空白分節，每節依聲母、介音、韻母的順序
/// 排列且同類別不重複，而且至少有一個聲調鍵。這樣 `su3cl3` 會被豁免，
/// 一般長英數帳號不會。
fn looks_like_bopomofo_keys(text: &str) -> bool {
    let keyboard = Keyboard::standard();
    let mut has_tone = false;
    let mut has_symbol = false;
    let mut last = Class::None;
    for ch in text.chars() {
        match ch {
            '3' | '4' | '6' | '7' | ' ' => {
                has_tone = true;
                last = Class::None;
                continue;
            }
            _ => {}
        }
        let Some(symbol) = keyboard.symbol(ch) else {
            return false;
        };
        let class = class_of(symbol);
        if class == Class::None || class <= last {
            return false;
        }
        last = class;
        has_symbol = true;
    }
    has_symbol && has_tone
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_or_blank_is_sensitive() {
        assert!(is_sensitive(""));
        assert!(is_sensitive("   \t\n"));
    }

    #[test]
    fn plain_words_are_learnable() {
        assert!(!is_sensitive("風景"));
        assert!(!is_sensitive("hello"));
    }

    #[test]
    fn long_digit_runs_are_sensitive() {
        assert!(is_sensitive("0912345678"));
        assert!(is_sensitive("電話 12345 號"));
        assert!(!is_sensitive("1234"));
        assert!(!is_sensitive("a1b2c3d4"));
    }

    #[test]
    fn emails_are_sensitive() {
        assert!(is_sensitive("user@example.com"));
        assert!(is_sensitive("a@b"));
        assert!(!is_sensitive("no-at-sign"));
        assert!(!is_sensitive("at@ end"));
    }

    #[test]
    fn urls_are_sensitive() {
        assert!(is_sensitive("https://example.com/path"));
        assert!(is_sensitive("www.example.org"));
        assert!(!is_sensitive("example.com"));
        assert!(!is_sensitive("http:/broken"));
    }

    #[test]
    fn card_numbers_are_sensitive() {
        assert!(is_sensitive("4111 1111 1111 1111"));
        assert!(is_sensitive("4111-1111-1111-1111"));
        assert!(!is_sensitive("4111 1111"));
        assert!(!is_sensitive("1234-5678-9012"));
    }

    #[test]
    fn taiwan_ids_are_sensitive() {
        assert!(is_sensitive("A123456789"));
        assert!(is_sensitive("b234567890"));
        assert!(!is_sensitive("AB1234-5678"));
        assert!(!is_sensitive("A1234B6789"));
    }

    #[test]
    fn long_alnum_accounts_are_sensitive() {
        assert!(is_sensitive("abcdefgh12345678"));
        assert!(is_sensitive("abc1def2ghi3jkl4m"));
        assert!(!is_sensitive("abcdefghijklmnop"));
        assert!(!is_sensitive("abc123"));
    }

    #[test]
    fn bopomofo_key_sequences_are_exempt() {
        assert!(looks_like_bopomofo_keys("su3cl3"));
        assert!(looks_like_bopomofo_keys("su3cl3su3cl3su3cl3"));
        assert!(!is_sensitive("su3cl3su3cl3su3cl3"));
        assert!(!looks_like_bopomofo_keys("abcdefgh12345678"));
    }
}
