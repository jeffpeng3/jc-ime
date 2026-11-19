//! 標點模型介面。
//!
//! 目前只有 identity stub：`punctuate` 原樣回傳輸入。真正的標點模型在
//! `onnx` feature 下可以載入並跑推論（見 [`crate::PunctuationModel`]），
//! 但把 logits 轉成插入位置尚未接上。

/// 標點介面。實作只能插入標點，不得改動原字。
pub trait Punctuator {
    /// 在文字中插入標點並回傳結果。
    fn punctuate(&self, text: &str) -> String;
}

/// identity 實作：原樣回傳。
#[derive(Debug, Clone, Copy, Default)]
pub struct IdentityPunctuator;

impl Punctuator for IdentityPunctuator {
    fn punctuate(&self, text: &str) -> String {
        text.to_string()
    }
}

/// 便利函式：目前是 identity stub，直接回傳輸入。
///
/// 這是刻意的：標點模型接線完成前，呼叫端不應看到文字被改動。
pub fn punctuate(text: &str) -> String {
    IdentityPunctuator.punctuate(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_returns_input_unchanged() {
        assert_eq!(punctuate("今天天氣很好"), "今天天氣很好");
        assert_eq!(punctuate(""), "");
    }
}
