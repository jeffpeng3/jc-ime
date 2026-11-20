//! 候選詞與來源。

use std::sync::Arc;

/// 詞性名稱表。
///
/// 候選只保存 POS id，名稱由這張共用表查。表以 [`Arc`] 共用，複製候選時
/// 不會複製字串，也不會讓每個候選各自持有一份詞性名稱。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PosTable {
    names: Arc<[Box<str>]>,
}

impl PosTable {
    /// 以名稱清單建立表；索引就是 POS id。
    pub fn new<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Box<str>>,
    {
        Self {
            names: names.into_iter().map(Into::into).collect(),
        }
    }

    /// 依 POS id 查名稱；越界回傳 `None`。
    pub fn name(&self, id: u32) -> Option<&str> {
        self.names.get(id as usize).map(|name| name.as_ref())
    }

    /// 名稱數量。
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// 是否沒有任何名稱。
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// 候選詞的來源。
///
/// 線格式為 snake_case（`lexicon`、`personal`、`predicted`、`literal`），
/// 由 `jc-proto` 的 `CandidateDetail` 直接沿用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// pack 的 POS id；來源沒有詞性時是 `None`（注音一律如此）。
    pub pos: Option<u32>,
    /// POS id 對應的名稱表，與其他候選共用。
    pos_table: Option<PosTable>,
}

impl Candidate {
    /// 建立候選；沒有詞性。
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
            pos: None,
            pos_table: None,
        }
    }

    /// 附上 POS id 與查名用的表。
    ///
    /// 表由 [`std::sync::Arc`] 共用，候選之間不會各自複製詞性字串。
    pub fn with_pos(mut self, id: u32, table: PosTable) -> Self {
        self.pos = Some(id);
        self.pos_table = Some(table);
        self
    }

    /// 清掉詞性；候選字面被換掉時使用，避免顯示上一筆的詞性。
    pub fn clear_pos(&mut self) {
        self.pos = None;
        self.pos_table = None;
    }

    /// 查目前候選的詞性名稱。
    ///
    /// 沒有 POS id、沒有名稱表或 id 越界時回傳 `None`。
    pub fn pos_name(&self) -> Option<&str> {
        let id = self.pos?;
        self.pos_table.as_ref()?.name(id)
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

    #[test]
    fn test_new_candidate_has_no_pos() {
        let candidate = Candidate::new("我", "ㄨㄛˇ", 12, CandidateSource::Lexicon);
        assert_eq!(candidate.pos, None, "注音候選沒有詞性");
        assert_eq!(candidate.pos_name(), None);
    }

    #[test]
    fn test_with_pos_keeps_id_and_shares_names() {
        let table = PosTable::new(["名詞,普通名詞,一般,*", "動詞,自立,*,*"]);
        let noun = Candidate::new("今日", "きょう", 1, CandidateSource::Lexicon)
            .with_pos(0, table.clone());
        let verb =
            Candidate::new("食べる", "たべる", 2, CandidateSource::Lexicon).with_pos(1, table);

        assert_eq!(noun.pos, Some(0));
        assert_eq!(noun.pos_name(), Some("名詞,普通名詞,一般,*"));
        assert_eq!(verb.pos_name(), Some("動詞,自立,*,*"));

        let missing = Candidate::new("京", "きょう", 3, CandidateSource::Lexicon)
            .with_pos(9, PosTable::new(["名詞"]));
        assert_eq!(missing.pos, Some(9));
        assert_eq!(missing.pos_name(), None, "id 越界不 panic");
    }
}
