//! 前文多樣性索引（對照 `core/personal/diversity.go`）。

use std::collections::BTreeMap;

use super::{cut_pair, Model, Store};

/// 每個前文「後接詞多樣性」的整數累加索引。
#[derive(Debug, Clone, Default)]
pub struct DiversityIndex {
    /// 對應的 store 位址；換 store 就失效。
    store: Option<*const Store>,
    /// 詞對筆數。
    bi: usize,
    /// 總量。
    total: f64,
    /// 當時使用的被動權重。
    unit: f64,
    /// 前文 → 1e9 單位的多樣性。
    by_left: Option<BTreeMap<String, i64>>,
}

const SCALE: f64 = 1e9;

impl Model {
    fn diversity_units(&self, value: f64) -> i64 {
        if value <= 0.0 || value.is_nan() {
            return 0;
        }
        (self.params.w_passive.min(value) * SCALE).round() as i64
    }

    /// 先驗等效樣本數。
    pub fn kappa(&self, left: &str) -> f64 {
        let mut k = self.params.kappa_bi;
        if self.params.type_prior > 0.0 {
            k = k.max(self.params.type_prior * self.diversity(left));
        }
        k
    }

    pub(super) fn diversity_valid(&self) -> bool {
        self.diversity.by_left.is_some()
            && self.diversity.store == Some(&self.store as *const Store)
            && self.diversity.bi == self.store.bi.len()
            && self.diversity.total == self.store.total
            && self.diversity.unit == self.params.w_passive
    }

    /// 前文多樣性。
    pub fn diversity(&self, left: &str) -> f64 {
        if !self.diversity_valid() {
            let by_left = self.rebuild_diversity();
            by_left.get(left).copied().unwrap_or(0) as f64 / SCALE
        } else {
            self.diversity
                .by_left
                .as_ref()
                .and_then(|map| map.get(left).copied())
                .unwrap_or(0) as f64
                / SCALE
        }
    }

    fn rebuild_diversity(&self) -> BTreeMap<String, i64> {
        // 索引的更新需要 &mut，查詢路徑只重建區域結果；下一次觀察會再整份建索引。
        let mut by_left = BTreeMap::new();
        for (key, value) in &self.store.bi {
            let (left, _) = cut_pair(key);
            *by_left.entry(left.to_string()).or_insert(0) += self.diversity_units(*value);
        }
        by_left
    }

    pub(super) fn change_bi(&mut self, left: &str, before: f64, after: f64) {
        if !self.diversity_valid() {
            return;
        }
        let delta = self.diversity_units(after) - self.diversity_units(before);
        if let Some(by_left) = self.diversity.by_left.as_mut() {
            *by_left.entry(left.to_string()).or_insert(0) += delta;
        }
        self.diversity.bi = self.store.bi.len();
        self.diversity.total = self.store.total;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::personal::{NoLexicon, NoLm};

    fn model() -> Model {
        Model::new(Box::new(NoLexicon), Box::new(NoLm))
    }

    #[test]
    fn frequent_left_word_keeps_unseen_continuations() {
        let mut m = model();
        for _ in 0..100 {
            m.observe("的", "詞", 0.25);
        }
        assert!(m.kappa("的") >= 1.0);
        assert_eq!(m.kappa("少見"), 1.0);
    }

    #[test]
    fn diversity_increments_with_observations() {
        let mut m = model();
        m.observe("前", "甲", 0.25);
        m.change_bi("前", 0.0, 0.25);
        assert!((m.diversity("前") - 0.25).abs() < 1e-9);
        m.observe("前", "乙", 0.25);
        m.change_bi("前", 0.25, 0.5);
        assert!((m.diversity("前") - 0.5).abs() < 1e-9);
    }
}
