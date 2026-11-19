//! 候選詞與來源。

/// 候選詞的來源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CandidateSource {
    /// 系統詞典。
    Lexicon,
    /// 使用者詞庫或學習結果。
    Personal,
    /// 由前文預測。
    Predicted,
    /// 原樣輸出的字面（英文、符號、數字）。
    Literal,
}

/// 一個轉換候選。
///
/// `cost` 沿用 MeCab 語意：**越小越優先**。長度等於讀音的候選通常成本較低。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// 輸出字面。
    pub surface: String,
    /// 對應讀音。
    pub reading: String,
    /// 路徑成本，越小越優先。
    pub cost: i32,
    /// 候選來源。
    pub source: CandidateSource,
}

impl Candidate {
    /// 建立候選。
    pub fn new(
        surface: impl Into<String>,
        reading: impl Into<String>,
        cost: i32,
        source: CandidateSource,
    ) -> Self {
        Self {
            surface: surface.into(),
            reading: reading.into(),
            cost,
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_keeps_fields_and_accepts_str_and_string() {
        let a = Candidate::new("我", "ㄨㄛˇ", 12, CandidateSource::Lexicon);
        assert_eq!(a.surface, "我");
        assert_eq!(a.reading, "ㄨㄛˇ");
        assert_eq!(a.cost, 12);
        assert_eq!(a.source, CandidateSource::Lexicon);

        let b = Candidate::new(
            String::from("watashi"),
            String::from("わたし"),
            0,
            CandidateSource::Literal,
        );
        assert_eq!(b.surface, "watashi");
        assert_eq!(b.reading, "わたし");
        assert_eq!(b.cost, 0);
        assert_eq!(b.source, CandidateSource::Literal);
        assert_ne!(a, b);
    }
}
