//! 表情、符號與分頁的選單記憶（對照 `core/personal/emoji.go`）。

use super::Model;

impl Model {
    /// 記住選過的表情，只改有上限的顯示順序。
    pub fn remember_emoji(&mut self, text: &str) {
        if crate::emoji::contains(text) {
            let now = self.store.time;
            let (order, stats) =
                crate::emoji::pick(&self.store.emoji_recent, &self.store.emoji_stats, text, now);
            self.store.emoji_recent = order;
            self.store.emoji_stats = stats;
        }
    }

    /// 記住上次真的選了東西的表情分頁。
    pub fn remember_palette_tab(&mut self, tab: &str) {
        self.store.palette_tab = tab.to_string();
    }

    /// 記住選過的符號，排進「最近」分頁。
    pub fn remember_symbol(&mut self, text: &str) {
        if crate::symbols::contains(text) {
            let now = self.store.time;
            let (order, stats) = crate::emoji::rank(
                &crate::symbols::clean_recent(&self.store.symbol_recent),
                &self.store.symbol_stats,
                text,
                now,
                crate::symbols::MAX_RECENT,
            );
            self.store.symbol_recent = order;
            self.store.symbol_stats = stats;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::personal::{NoLexicon, NoLm};

    #[test]
    fn emoji_only_changes_palette_order() {
        let mut m = Model::new(Box::new(NoLexicon), Box::new(NoLm));
        m.remember_emoji("😀");
        assert_eq!(m.store.emoji_recent.first().map(String::as_str), Some("😀"));
        assert!(m.store.uni.is_empty());
    }

    #[test]
    fn symbol_outside_catalog_is_ignored() {
        let mut m = Model::new(Box::new(NoLexicon), Box::new(NoLm));
        m.remember_symbol("？");
        assert_eq!(
            m.store.symbol_recent.first().map(String::as_str),
            Some("？")
        );
        m.remember_symbol("不是符號");
        assert!(!m.store.symbol_recent.iter().any(|item| item == "不是符號"));
    }
}
