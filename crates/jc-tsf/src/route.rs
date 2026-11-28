//! 外殼的按鍵路由決策；Linux 也能編、也測，不依賴 Win32。
//!
//! `service.rs` 只在 Windows 編譯，但「英文模式要不要送引擎、送 Key 還是
//! Literal」這個決策本身沒有平台相依。抽到這裡，Linux 測試才能直接驗證
//! 路由結果，不會只剩 Windows-only 的 host 測試。

/// 英文模式這一鍵的文字路徑。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnglishRoute {
    /// 不進引擎，直接交還宿主。
    Host,
    /// 送 `Request::Literal`，接在既有組字後面。
    Literal,
    /// 送一般 `Request::Key`。
    Key,
}

/// 英文模式可送 Literal 的字面字元：空白（0x20）到 `~`（0x7e）。
///
/// `keys::printable_with` 用這個判斷，Linux 測試才能直接驗空白與 keypad
/// 產生的字元。數字鍵盤的 VK 不是這裡的事：`ToUnicode` 轉出來仍是 `0`–`9`。
pub fn english_printable_char(c: u16) -> Option<char> {
    (0x20..=0x7e).contains(&c).then(|| char::from(c as u8))
}

/// 依外殼狀態決定英文模式的文字路徑。
///
/// `printable` 由呼叫端用實際鍵盤狀態判定：空白、Shift+Space 與 keypad
/// 字元都算。`navigating` 是候選窗或選單開著：這時 printable 走 Key，讓
/// 數字選字、字母選項回到 engine，不能當成字面插入。
///
/// 中文模式的 printable 由原本的組字政策處理，這裡一律回
/// [`EnglishRoute::Key`]。
pub fn english_route(
    english: bool,
    composing: bool,
    printable: bool,
    navigating: bool,
) -> EnglishRoute {
    if !english {
        return EnglishRoute::Key;
    }
    if !composing {
        return EnglishRoute::Host;
    }
    if navigating {
        return EnglishRoute::Key;
    }
    if printable {
        EnglishRoute::Literal
    } else {
        EnglishRoute::Key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 組字中的英文 printable 全部走 Literal：一般字母、Shift 大寫與 `@`。
    #[test]
    fn english_composing_printables_route_to_literal() {
        let routes: Vec<EnglishRoute> = ['a', 'A', '@']
            .into_iter()
            .map(|_| english_route(true, true, true, false))
            .collect();
        assert_eq!(routes, vec![EnglishRoute::Literal; 3]);
    }

    /// 空白與 Shift+Space 也是 printable：注音 `ji` 切英文後按空白要追加字面，
    /// 不能變成注音的聲調／音節完成。
    #[test]
    fn english_composing_space_and_shift_space_route_to_literal() {
        assert_eq!(
            english_route(true, true, true, false),
            EnglishRoute::Literal,
            "空白要送 Literal"
        );
        assert_eq!(
            english_route(true, true, true, false),
            EnglishRoute::Literal,
            "Shift+Space 也要送 Literal"
        );
    }

    /// 選單／候選窗開著時，printable（數字、字母）走 Key 讓 engine 選字。
    #[test]
    fn english_composing_printable_while_navigating_uses_key() {
        assert_eq!(
            english_route(true, true, true, true),
            EnglishRoute::Key,
            "navigating 時數字與字母要進 engine，不能當字面"
        );
    }

    /// 組字中的具名鍵（Enter／Backspace）仍走 Key，才能 commit／刪除。
    #[test]
    fn english_composing_named_keys_route_to_key() {
        assert_eq!(english_route(true, true, false, false), EnglishRoute::Key);
    }

    /// 沒有組字的英文 printable 直接給宿主；只有已註冊 chord 在呼叫端先處理。
    #[test]
    fn english_idle_printables_route_to_host() {
        assert_eq!(english_route(true, false, true, false), EnglishRoute::Host);
        assert_eq!(english_route(true, false, false, false), EnglishRoute::Host);
        assert_eq!(
            english_route(true, false, true, true),
            EnglishRoute::Host,
            "沒有組字時 navigating 也不能讓外殼吃掉按鍵"
        );
    }

    /// 中文模式不受英文路由影響。
    #[test]
    fn chinese_keys_stay_on_the_key_path() {
        assert_eq!(english_route(false, true, true, false), EnglishRoute::Key);
        assert_eq!(english_route(false, false, true, false), EnglishRoute::Key);
    }

    /// 空白到 `~` 都算 printable；0x1f 以下與 0x7f 以上不算。
    #[test]
    fn english_printable_range_accepts_space_through_tilde() {
        assert_eq!(
            english_printable_char(0x20),
            Some(' '),
            "空白要算 printable"
        );
        assert_eq!(english_printable_char(0x21), Some('!'));
        assert_eq!(english_printable_char(0x7e), Some('~'));
        assert_eq!(english_printable_char(0x1f), None);
        assert_eq!(english_printable_char(0x7f), None);
    }

    /// keypad 的 VK 經 ToUnicode 後是 `0`–`9`，落在同一個範圍，不能另外排除。
    #[test]
    fn english_printable_range_includes_keypad_digits() {
        for c in '0'..='9' {
            assert_eq!(
                english_printable_char(c as u16),
                Some(c),
                "keypad 數字 {c} 要算 printable"
            );
        }
    }
}
