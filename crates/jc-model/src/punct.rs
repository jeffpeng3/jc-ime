//! 標點模型介面。
//!
//! [`Punctuator`] 是不需要模型的字串介面；[`PunctuationModel`] 與
//! [`crate::ZhPunctuationModel`] 在 `onnx` feature 下載入真正的模型。
//! 這裡保留 identity 實作，讓沒有模型的路徑維持原樣。

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

/// 便利函式：沒有模型可用時原樣回傳。
///
/// 真正的模型請用 [`crate::PunctuationModel`] 或
/// [`crate::ZhPunctuationModel`]；這個函式保留給沒有載入模型的路徑。
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
