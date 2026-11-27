//! 引擎註冊表。
//!
//! 排序一律用 [`LanguageId`] 的字串，不用註冊順序。P1 只註冊 owned adapter：
//! `Box<dyn LanguageEngine + 'static>`，不存 `&'session mut Session`。

use std::collections::BTreeMap;

use crate::key::Key;

use super::{InputContext, LanguageEngine, LanguageId, Route, Router};

/// 已註冊的語言引擎集合。
pub struct EngineRegistry {
    engines: Vec<Box<dyn LanguageEngine + 'static>>,
    index: BTreeMap<LanguageId, usize>,
    router: Router,
}

impl Default for EngineRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)] // P1 的查詢／走訪 API；P2 的 FanOut 會用。
impl EngineRegistry {
    /// 建立空註冊表。
    pub fn new() -> Self {
        Self {
            engines: Vec::new(),
            index: BTreeMap::new(),
            router: Router,
        }
    }

    /// 註冊一個引擎；重複 id 直接 panic。
    ///
    /// 重複註冊是程式設定錯誤，不是使用者輸入；P1 不在執行期降級。
    pub fn register(&mut self, engine: Box<dyn LanguageEngine + 'static>) {
        let id = engine.id();
        assert!(
            !self.index.contains_key(&id),
            "duplicate language engine id: {id}"
        );
        self.index.insert(id, self.engines.len());
        self.engines.push(engine);
    }

    /// 取得引擎。
    pub fn get(&self, id: LanguageId) -> Option<&dyn LanguageEngine> {
        self.index
            .get(&id)
            .and_then(|at| self.engines.get(*at))
            .map(|engine| engine.as_ref())
    }

    /// 取得可變引擎；P2 的 `feed()` 會用到。
    pub fn get_mut(&mut self, id: LanguageId) -> Option<&mut (dyn LanguageEngine + 'static)> {
        let at = *self.index.get(&id)?;
        self.engines.get_mut(at).map(|engine| engine.as_mut())
    }

    /// 已註冊的 id，依字串排序。
    pub fn ids(&self) -> Vec<LanguageId> {
        self.index.keys().copied().collect()
    }

    /// 註冊數量。
    pub fn len(&self) -> usize {
        self.engines.len()
    }

    /// 是否沒有任何引擎。
    pub fn is_empty(&self) -> bool {
        self.engines.is_empty()
    }

    /// 對一鍵做 P1 快速路由。
    pub fn route(&mut self, key: &Key, ctx: &InputContext<'_>) -> Route {
        self.router.route(&mut self.engines, key, ctx)
    }
}
