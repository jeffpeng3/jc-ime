//! 詞典與語言模型的二進位格式。零拷貝讀取（memmap2），格式版本明確。
//!
//! 目前是骨架：先固定「一個資料集 = manifest + 多個 section」的介面，
//! 實際格式在 M1／M2 依注音詞典與 Mozc 詞典的需求定義。

pub mod ja;
pub mod zh;

/// 資料集種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dataset {
    /// 注音詞典與成本。
    Zh,
    /// 日文詞典（Mozc）、連接矩陣與語言模型。
    Ja,
    /// 英文詞表與補完資料。
    En,
}

/// 已載入的資料集摘要，供診斷與版本核對。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetInfo {
    pub dataset: Dataset,
    pub version: String,
    pub entries: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dataset_info_keeps_identity() {
        let info = DatasetInfo {
            dataset: Dataset::Ja,
            version: "mozc-3.35.6354".into(),
            entries: 1,
        };
        assert_eq!(info.dataset, Dataset::Ja);
        assert_eq!(info.entries, 1);
    }
}
