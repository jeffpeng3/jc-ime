//! 轉換結果與引擎錯誤。

use crate::lang::Mode;
use crate::segment::Segment;

/// 一次轉換的結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversion {
    /// 輸出字面。
    pub surface: String,
    /// 產生這個字面的片段。
    pub segments: Vec<Segment>,
    /// 整體路徑成本，越小越優先。
    pub cost: i32,
}

/// 核心對外回報的錯誤。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    /// 沒有可轉換的輸入。
    #[error("沒有可轉換的輸入")]
    EmptyInput,
    /// 讀音格式不合法。
    #[error("無效的讀音：{0}")]
    InvalidReading(String),
    /// 目前模式不支援這個操作。
    #[error("模式 {0:?} 不支援這個操作")]
    UnsupportedMode(Mode),
    /// 找不到必要的資料檔。
    #[error("找不到資料：{0}")]
    MissingData(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::Mode;
    use crate::segment::Script;

    #[test]
    fn conversion_keeps_surface_segments_and_cost() {
        let c = Conversion {
            surface: "わたし".to_string(),
            segments: vec![Segment::new(0, 9, Mode::Ja, Script::Kana)],
            cost: 42,
        };
        assert_eq!(c.surface, "わたし");
        assert_eq!(c.segments.len(), 1);
        assert_eq!(c.cost, 42);
    }

    #[test]
    fn engine_error_messages_include_details() {
        let e = EngineError::UnsupportedMode(Mode::En);
        assert_eq!(e.to_string(), "模式 En 不支援這個操作");

        let e = EngineError::InvalidReading("ㄅ".to_string());
        assert_eq!(e.to_string(), "無效的讀音：ㄅ");

        let e = EngineError::EmptyInput;
        assert_eq!(e.to_string(), "沒有可轉換的輸入");

        let e = EngineError::MissingData("lexicon.bin".to_string());
        assert_eq!(e.to_string(), "找不到資料：lexicon.bin");
    }
}
