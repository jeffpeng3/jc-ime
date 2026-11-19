//! 平台無關的輸入法核心：鍵序列、片段、路徑評分、候選。
//!
//! 這個 crate 不得依賴任何作業系統 API，所有平台相關行為都由
//! `jc-platform` 的 trait 注入，方便在 Linux 上開發與測試。

/// 輸入模式。Mixed 是唯一會同時接受注音、日文羅馬字與英文的模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Zh,
    Ja,
    En,
}

/// 組字區中的一個片段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub start: usize,
    pub end: usize,
    pub mode: Mode,
}

impl Segment {
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// 這個 crate 的語意版本，供協定握手與診斷使用。
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_length_is_end_minus_start() {
        let s = Segment {
            start: 2,
            end: 5,
            mode: Mode::Ja,
        };
        assert_eq!(s.len(), 3);
        assert!(!s.is_empty());
    }
}
