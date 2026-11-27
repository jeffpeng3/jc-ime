//! 引擎識別碼與這一鍵的興趣分類。

use std::fmt;

/// 語言引擎的穩定識別碼。
///
/// tie-break、註冊表與診斷一律用這個字串，不用列舉順序。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LanguageId(&'static str);

#[allow(dead_code)] // P1 的公開查詢；P2 的合併與診斷會用。
impl LanguageId {
    /// 注音引擎。
    pub const ZH: Self = Self("zh");
    /// 日文羅馬字引擎。
    pub const JA: Self = Self("ja");
    /// 英數／混打引擎。
    pub const ASCII: Self = Self("ascii");
    /// 前導解析引擎；P1.5 才會註冊。
    pub const PREFIX: Self = Self("prefix");

    /// 穩定的字串形式。
    pub const fn as_str(self) -> &'static str {
        self.0
    }

    /// 用穩定字串建立識別碼；只用於測試替身。
    pub const fn from_static(value: &'static str) -> Self {
        Self(value)
    }
}

impl fmt::Display for LanguageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// 這一鍵對某個引擎的意義。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interest {
    /// 不參與；不得讀取按鍵以外的輸入。
    No,
    /// 可能是我的，但完整判斷要留到 `feed()`。
    Maybe,
    /// 一定是我的；只有一個 `Claim` 時走快速路徑。
    Claim,
}

impl Interest {
    /// 是否參與這次按鍵。
    #[allow(dead_code)] // P2 的 router 診斷會用。
    pub fn participates(self) -> bool {
        !matches!(self, Self::No)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_ids_are_stable_and_ordered_by_text() {
        assert_eq!(LanguageId::ZH.as_str(), "zh");
        assert_eq!(LanguageId::JA.to_string(), "ja");
        assert!(LanguageId::ASCII < LanguageId::JA);
        assert!(LanguageId::JA < LanguageId::ZH);
    }

    #[test]
    fn only_no_is_outside_the_route() {
        assert!(!Interest::No.participates());
        assert!(Interest::Maybe.participates());
        assert!(Interest::Claim.participates());
    }
}
