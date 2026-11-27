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

/// 依外殼狀態決定英文模式的文字路徑。
///
/// `printable` 由呼叫端用實際鍵盤狀態判定（空白、具名鍵、數字鍵盤不算）。
/// 中文模式的 printable 由原本的組字政策處理，這裡一律回 [`EnglishRoute::Key`]。
pub fn english_route(english: bool, composing: bool, printable: bool) -> EnglishRoute {
    if !english {
        return EnglishRoute::Key;
    }
    if !composing {
        return EnglishRoute::Host;
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
            .map(|_| english_route(true, true, true))
            .collect();
        assert_eq!(routes, vec![EnglishRoute::Literal; 3]);
    }

    /// 組字中的具名鍵（Enter／Backspace）仍走 Key，才能 commit／刪除。
    #[test]
    fn english_composing_named_keys_route_to_key() {
        assert_eq!(english_route(true, true, false), EnglishRoute::Key);
    }

    /// 沒有組字的英文 printable 直接給宿主；只有已註冊 chord 在呼叫端先處理。
    #[test]
    fn english_idle_printables_route_to_host() {
        assert_eq!(english_route(true, false, true), EnglishRoute::Host);
        assert_eq!(english_route(true, false, false), EnglishRoute::Host);
    }

    /// 中文模式不受英文路由影響。
    #[test]
    fn chinese_keys_stay_on_the_key_path() {
        assert_eq!(english_route(false, true, true), EnglishRoute::Key);
        assert_eq!(english_route(false, false, true), EnglishRoute::Key);
    }
}
