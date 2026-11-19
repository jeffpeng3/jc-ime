//! 選過的注音文（對照 `core/personal/zhuyin_keys.go`）。

use super::{pair, Model, BOS, CONFIRM_UNIT};

/// 選過的注音文衰減到這裡就不算。
pub const ZHUYIN_KEY_FLOOR: f64 = 0.05;

/// 一次提交對注音文的影響。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZhuyinKeyAction {
    /// 使用者選了注音。
    Pick,
    /// 紀錄照常留著，暫停衰減。
    Keep,
    /// 選回原按鍵，整筆消失。
    Drop,
}

/// 提交時畫面上的一個注音鍵＋空白。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZhuyinKeyCommit {
    /// 前一個字；句首是空字串。
    pub left: String,
    /// 注音符號。
    pub symbol: String,
    /// 這次提交的影響。
    pub action: ZhuyinKeyAction,
}

impl ZhuyinKeyCommit {
    /// 建立事件。
    pub fn new(
        left: impl Into<String>,
        symbol: impl Into<String>,
        action: ZhuyinKeyAction,
    ) -> Self {
        Self {
            left: left.into(),
            symbol: symbol.into(),
            action,
        }
    }
}

impl Model {
    /// 是否沒有任何注音文紀錄。
    pub fn has_zhuyin_keys(&self) -> bool {
        !self.store.zhuyin_keys.is_empty()
    }

    /// left 後面這個注音符號是否直接出注音。
    pub fn zhuyin_key_picked(&self, left: &str, symbol: &str) -> bool {
        self.store
            .zhuyin_keys
            .get(&pair(left, symbol))
            .map(|entry| entry.at(self.store.time).strength(self.store.time))
            .is_some_and(|strength| strength >= ZHUYIN_KEY_FLOOR)
    }

    /// 更新注音文紀錄。
    pub fn learn_zhuyin_keys(&mut self, events: &[ZhuyinKeyCommit]) {
        for event in events {
            let key = pair(&event.left, &event.symbol);
            match event.action {
                ZhuyinKeyAction::Pick => {
                    if self
                        .store
                        .tombstones
                        .contains(&format!("{}{}", event.left, event.symbol))
                    {
                        continue;
                    }
                    let entry = self
                        .store
                        .zhuyin_keys
                        .get(&key)
                        .copied()
                        .unwrap_or_default();
                    self.store
                        .zhuyin_keys
                        .insert(key, entry.add(self.store.time, CONFIRM_UNIT));
                }
                ZhuyinKeyAction::Keep => {
                    if let Some(entry) = self.store.zhuyin_keys.get_mut(&key) {
                        if self.store.time > entry.updated {
                            entry.updated = self.store.time;
                        }
                    }
                }
                ZhuyinKeyAction::Drop => {
                    self.store.zhuyin_keys.remove(&key);
                }
            }
            self.gen += 1;
        }
    }
}

/// 句首符號；保留給呼叫端判斷。
pub const SENTENCE_START: &str = BOS;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::personal::{NoLexicon, NoLm};

    fn model() -> Model {
        Model::new(Box::new(NoLexicon), Box::new(NoLm))
    }

    #[test]
    fn pick_keep_drop() {
        let mut m = model();
        m.store.time = 100;
        m.learn_zhuyin_keys(&[ZhuyinKeyCommit::new("我", "ㄉ", ZhuyinKeyAction::Pick)]);
        assert!(m.zhuyin_key_picked("我", "ㄉ"));
        m.learn_zhuyin_keys(&[ZhuyinKeyCommit::new("我", "ㄉ", ZhuyinKeyAction::Keep)]);
        assert!(m.zhuyin_key_picked("我", "ㄉ"));
        m.learn_zhuyin_keys(&[ZhuyinKeyCommit::new("我", "ㄉ", ZhuyinKeyAction::Drop)]);
        assert!(!m.zhuyin_key_picked("我", "ㄉ"));
    }

    #[test]
    fn tombstone_blocks_pick() {
        let mut m = model();
        m.store.tombstones.insert("我ㄉ".to_string());
        m.learn_zhuyin_keys(&[ZhuyinKeyCommit::new("我", "ㄉ", ZhuyinKeyAction::Pick)]);
        assert!(!m.zhuyin_key_picked("我", "ㄉ"));
    }
}
