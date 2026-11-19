//! 凍結題庫的評測工具：讀取題庫、跑轉換、輸出可比較的指標。
//!
//! 題庫格式沿用 `0c-ime` 的 `testdata/ja`、`testdata/eval`、注音 golden，
//! 讓新舊引擎可以在同一批題目上逐句比對。

/// 單題結果。
#[derive(Debug, Clone, PartialEq)]
pub struct CaseResult {
    pub id: String,
    pub expected: String,
    pub actual: String,
}

impl CaseResult {
    pub fn is_exact(&self) -> bool {
        self.expected == self.actual
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_is_reported() {
        let r = CaseResult {
            id: "1".into(),
            expected: "本".into(),
            actual: "本".into(),
        };
        assert!(r.is_exact());
    }
}
