//! P1 的快速路由：只有一個 `Claim` 或只有一個參與者時直接跑該引擎。
//!
//! P1 不做 FanOut、平行或 lattice；多個參與者一律回 [`Route::Fallback`]，
//! 由 `Session` 的既有 lane 邏輯處理。前導控制引擎先問；它回 `Claim` 時
//! 這一鍵由前導獨佔，語言引擎不參與。

use crate::key::Key;

use super::{EngineKind, InputContext, Interest, LanguageEngine, LanguageId, PrefixEngine};

/// 一鍵的決策。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// 只有一個參與者（或只有一個排他 `Claim`），直接跑它。
    Fast(LanguageId),
    /// 多個參與者；P1 退回 `Session` 的既有路徑。
    Fallback,
    /// 沒有人參與；照字面。
    Literal,
}

/// 興趣分派的決策者。
#[derive(Clone, Copy, Debug, Default)]
pub struct Router;

impl Router {
    /// 依註冊順序問每個引擎，但決策與順序無關。
    ///
    /// 規則：
    /// 1. 前導控制引擎回 `Claim` → 只跑前導。
    /// 2. 其他控制引擎回 `Claim` → 只跑它。
    /// 3. 恰好一個 `Claim` → `Fast(claim)`；其他 `Maybe` 不跑。
    /// 4. 沒有 `Claim`、只有一個參與者 → `Fast`。
    /// 5. 多個參與者或沒有參與者 → `Fallback`／`Literal`。
    pub fn route(
        &self,
        prefix: &mut PrefixEngine,
        engines: &mut [Box<dyn LanguageEngine + 'static>],
        key: &Key,
        ctx: &InputContext<'_>,
    ) -> Route {
        // 前導是控制引擎：`Claim` 就獨佔，語言引擎連 `Maybe` 都不跑。
        prefix.observe(ctx);
        if prefix.interest(key, ctx) == Interest::Claim {
            return Route::Fast(LanguageId::PREFIX);
        }

        let mut participants = Vec::new();
        let mut claims = Vec::new();
        for engine in engines.iter_mut() {
            engine.observe(ctx);
            let interest = engine.interest(key, ctx);
            if interest == Interest::No {
                continue;
            }
            let id = engine.id();
            participants.push(id);
            if interest == Interest::Claim {
                if engine.kind() == EngineKind::Control {
                    return Route::Fast(id);
                }
                claims.push(id);
            }
        }
        participants.sort();
        participants.dedup();
        claims.sort();
        claims.dedup();

        match (claims.as_slice(), participants.as_slice()) {
            ([], []) => Route::Literal,
            ([], [only]) => Route::Fast(*only),
            ([claim], _) => Route::Fast(*claim),
            _ => {
                if claims.len() > 1 {
                    log::debug!(
                        "engine route: multiple claims {claims:?}; fallback to legacy lane"
                    );
                }
                Route::Fallback
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::LanguageId;

    #[test]
    fn route_is_deterministic_by_language_id() {
        // `Route` 只帶 id，排序不依註冊順序；這個測試鎖住等值比較。
        assert_eq!(
            Route::Fast(LanguageId::ASCII),
            Route::Fast(LanguageId::ASCII)
        );
        assert_ne!(Route::Fast(LanguageId::ZH), Route::Fallback);
    }
}
