//! 引擎輸出的假設資料。

use super::{Span, StateToken};

/// 引擎對某一段按鍵流的假設。
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)] // P1 只保留型別；P2 的 feed() 才會建構。
pub struct Hypothesis {
    /// 覆蓋的按鍵流範圍；可以是多鍵，不必是單鍵。
    pub span: Span,
    /// 顯示字面。
    pub surface: String,
    /// 讀音；注音用空白分隔音節，日文用かな，ASCII 可為空。
    pub reading: String,
    /// 越小越優先；沿用 `Candidate::cost` 的方向。
    pub score: i64,
    /// 產生這個假設的狀態版本。
    pub state_token: StateToken,
}

/// Enter／空白一次送出的單位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // `Char` 給 P1.5 的前導引擎。
pub enum CommitUnit {
    /// 前導碼位：只插入一個字。
    Char,
    /// 注音音節。
    Syllable,
    /// 日文文節。
    Clause,
    /// 整段輸入或英數 run。
    Run,
}

/// 顯示風格；只影響 `Session` 怎麼呈現，不影響排序。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // `Prefix` 給 P1.5 的前導引擎。
pub enum StyleHint {
    /// 照字面，不再轉換。
    Literal,
    /// 注音：全形標點、聲調與候選介面。
    ZhFullwidth,
    /// 日文：かな與 F6–F10 轉換。
    JaKana,
    /// 前導：反引號與碼位輸入。
    Prefix,
}

/// 引擎主動把先前吃掉的按鍵交回 router。
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)] // P1 只保留型別；P2 的 release() 才會建構。
pub struct Released {
    /// 要重播的按鍵。
    pub keys: String,
    /// 這些按鍵原本在按鍵流的位置。
    pub span: Span,
    /// 放手的原因。
    pub reason: ReleaseReason,
}

/// 引擎放手的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // P2 的 release() 才會使用。
pub enum ReleaseReason {
    /// 語言邊界：前面的按鍵要交給下一個引擎重算。
    Handoff,
    /// 使用者取消。
    Cancelled,
    /// 語法錯誤，照字面留下。
    Invalid,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::LanguageId;

    #[test]
    fn hypothesis_carries_a_full_state_token() {
        let token = StateToken {
            engine: LanguageId::ZH,
            epoch: 7,
            revision: 2,
        };
        let hypothesis = Hypothesis {
            span: Span::new(0, 3),
            surface: "我".to_string(),
            reading: "ㄨㄛˇ".to_string(),
            score: -1,
            state_token: token,
        };
        assert_eq!(hypothesis.span.len(), 3);
        assert_eq!(hypothesis.state_token, token);
    }
}
