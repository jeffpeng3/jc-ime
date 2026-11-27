//! 語言引擎介面（P1）。
//!
//! 這一輪 module 仍是 `pub(crate)`：`CandidateAction` 是 `pub(crate)`，trait
//! 的 `apply()` 還不對外公開；P2 把引擎狀態與候選動作接上後再公開。
//!
//! 這一輪只建立骨架與快速路徑：
//! - [`LanguageEngine`] 與 [`LanguageId`]／[`Interest`]／[`InputContext`]／
//!   [`Hypothesis`] 等設計型別。
//! - 注音 [`ZhEngine`]、日文 [`JaEngine`]、英數 [`AsciiEngine`] 三個 owned
//!   adapter。
//! - [`Router`] 的 P1 快速路徑：唯一 `Claim` 或唯一參與者直接跑該 lane；
//!   多個參與者退回 `Session` 既有路徑。
//!
//! 這一輪不改畫面、候選、提交或轉換：`Session::press_char()` 只多問一次
//! router，真正的 lane 實作仍是原本的 `press_zh_char()`／`press_char()`。

mod ascii;
mod context;
mod hypothesis;
mod id;
mod ja;
mod prefix;
mod registry;
mod router;
mod span;
mod r#trait;
mod zh;

pub use ascii::AsciiEngine;
pub use context::{InputContext, LaneView};
#[allow(unused_imports)] // P1 先保留完整介面；release() 在 P2 才接上。
pub use hypothesis::{CommitUnit, Hypothesis, ReleaseReason, Released, StyleHint};
pub use id::{Interest, LanguageId};
pub use ja::JaEngine;
pub(crate) use prefix::PrefixEffect;
pub use prefix::PrefixEngine;
pub use r#trait::{EngineKind, LanguageEngine};
pub use registry::EngineRegistry;
pub use router::{Route, Router};
pub use span::{Span, StateToken};
pub(crate) use zh::starts_key;
pub use zh::ZhEngine;

/// P1 的轉換探針。
///
/// 只用來證明 `interest()` 沒有偷跑 `Session::rebuild()` 或
/// `mixed::plan_input()`。thread-local 讓平行的單元測試互不干擾。
#[cfg(test)]
pub(crate) mod probe {
    use std::cell::Cell;

    thread_local! {
        static REBUILDS: Cell<usize> = const { Cell::new(0) };
        static PLANS: Cell<usize> = const { Cell::new(0) };
    }

    pub(crate) fn note_rebuild() {
        REBUILDS.with(|count| count.set(count.get() + 1));
    }

    pub(crate) fn note_plan_input() {
        PLANS.with(|count| count.set(count.get() + 1));
    }

    pub(crate) fn reset() {
        REBUILDS.with(|count| count.set(0));
        PLANS.with(|count| count.set(0));
    }

    pub(crate) fn rebuild_count() -> usize {
        REBUILDS.with(Cell::get)
    }

    pub(crate) fn plan_input_count() -> usize {
        PLANS.with(Cell::get)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{Key, KeyCode};
    use crate::lang::Mode;

    fn registry() -> EngineRegistry {
        let mut registry = EngineRegistry::new();
        registry.register(Box::new(ZhEngine::new()));
        registry.register(Box::new(JaEngine::new()));
        registry.register(Box::new(AsciiEngine::new()));
        registry
    }

    fn empty_lanes() -> LaneView {
        LaneView {
            ascii_empty: true,
            ascii_pieces_empty: true,
            syllables_empty: true,
            ..LaneView::default()
        }
    }

    fn route(
        registry: &mut EngineRegistry,
        ch: char,
        active: Option<LanguageId>,
        lanes: LaneView,
    ) -> Route {
        let context = InputContext::routing(Mode::Mixed, active, lanes);
        registry.route(&Key::char(ch, false), &context)
    }

    #[test]
    fn registry_registers_three_owned_adapters() {
        let registry = registry();
        assert_eq!(registry.len(), 4, "三個語言引擎加前導控制引擎");
        assert_eq!(
            registry.ids(),
            vec![
                LanguageId::ASCII,
                LanguageId::JA,
                LanguageId::PREFIX,
                LanguageId::ZH
            ]
        );
        assert_eq!(
            registry.get(LanguageId::ZH).unwrap().style(),
            StyleHint::ZhFullwidth
        );
        assert_eq!(
            registry.get(LanguageId::JA).unwrap().commit_unit(),
            CommitUnit::Clause
        );
        assert_eq!(
            registry.get(LanguageId::ASCII).unwrap().commit_unit(),
            CommitUnit::Run
        );
    }

    #[test]
    fn prefix_control_claims_backtick_and_the_whole_run() {
        let mut registry = registry();
        assert_eq!(
            route(&mut registry, '`', None, empty_lanes()),
            Route::Fast(LanguageId::PREFIX)
        );
        let context = InputContext::routing(Mode::Mixed, None, empty_lanes());
        let prefix = registry.get_mut(LanguageId::PREFIX).unwrap();
        prefix.feed(&Key::char('`', false), &context);
        assert!(registry.prefix().is_active());
        for key in ['u', '4', 'e', '0', '0', ' '] {
            assert_eq!(
                route(&mut registry, key, None, empty_lanes()),
                Route::Fast(LanguageId::PREFIX),
                "{key:?} 前導期間要由控制引擎獨佔"
            );
        }
    }

    #[test]
    fn single_claim_takes_the_fast_path() {
        let mut registry = registry();
        assert_eq!(
            route(&mut registry, 'j', None, empty_lanes()),
            Route::Fast(LanguageId::ZH)
        );
        let tone = LaneView {
            zh_lane: true,
            ..empty_lanes()
        };
        assert_eq!(
            route(&mut registry, '3', Some(LanguageId::ZH), tone),
            Route::Fast(LanguageId::ZH)
        );
    }

    #[test]
    fn single_participant_takes_the_fast_path() {
        let mut registry = registry();
        assert_eq!(
            route(&mut registry, '!', None, empty_lanes()),
            Route::Fast(LanguageId::ASCII)
        );
    }

    #[test]
    fn named_key_is_literal_when_nobody_participates() {
        let mut registry = registry();
        let context = InputContext::routing(Mode::Mixed, None, empty_lanes());
        assert_eq!(
            registry.route(&Key::new(KeyCode::Enter, None, false), &context),
            Route::Literal
        );
    }

    #[test]
    fn multiple_participants_fall_back_without_running_conversion() {
        let mut registry = registry();
        let lanes = LaneView {
            ascii_empty: false,
            ascii_len: 1,
            ascii_pieces_empty: true,
            syllables_empty: true,
            ja_available: true,
            ..LaneView::default()
        };
        probe::reset();
        assert_eq!(route(&mut registry, 'a', None, lanes), Route::Fallback);
        assert_eq!(probe::rebuild_count(), 0, "interest 不得呼叫 rebuild");
        assert_eq!(probe::plan_input_count(), 0, "interest 不得呼叫 plan_input");
    }

    #[test]
    fn no_and_claim_paths_do_not_convert() {
        let mut registry = registry();
        probe::reset();
        // `No` 路徑：具名鍵。
        let context = InputContext::routing(Mode::Mixed, None, empty_lanes());
        let _ = registry.route(&Key::new(KeyCode::Escape, None, false), &context);
        // `Claim` 路徑：注音開頭。
        let _ = route(&mut registry, 'j', None, empty_lanes());
        // 單一參與者：ASCII。
        let _ = route(&mut registry, '!', None, empty_lanes());
        assert_eq!(probe::rebuild_count(), 0);
        assert_eq!(probe::plan_input_count(), 0);
    }

    /// 快速路徑與既有決策的 golden 對照：preedit、候選、游標、commit 都要相同。
    #[test]
    fn fast_path_matches_legacy_for_the_p1_corpus() {
        use crate::lattice::{InMemoryLexicon, LexEntry};
        use crate::session::{Session, SessionConfig};
        use std::sync::Arc;

        fn lexicon() -> InMemoryLexicon {
            let mut lex = InMemoryLexicon::new();
            for (surface, reading, cost) in [
                ("我", "ㄨㄛˇ", 0),
                ("窩", "ㄨㄛˇ", 10),
                ("蘇", "ㄙㄨ", 0),
                ("你", "ㄋㄧˇ", 0),
                ("好", "ㄏㄠˇ", 0),
            ] {
                lex.push(LexEntry::new(surface, reading, cost, 0));
            }
            lex
        }

        fn new_session() -> Session {
            let mut config = SessionConfig::new(Mode::Mixed);
            config.zh_lexicon = Some(Arc::new(lexicon()));
            config.max_candidates = 20;
            Session::new(config)
        }

        for input in ["ji3", "su3", "watashi", "hellp", "wow"] {
            let mut fast = new_session();
            let mut legacy = new_session();
            legacy.set_legacy_routing(true);
            for (at, ch) in input.chars().enumerate() {
                let fast_snapshot = fast.press(Key::char(ch, false));
                let legacy_snapshot = legacy.press(Key::char(ch, false));
                assert_eq!(
                    fast_snapshot, legacy_snapshot,
                    "{input:?} 第 {at} 鍵的 snapshot 不一致"
                );
            }
            let fast_enter = fast.press(Key::new(KeyCode::Enter, None, false));
            let legacy_enter = legacy.press(Key::new(KeyCode::Enter, None, false));
            assert_eq!(
                fast_enter, legacy_enter,
                "{input:?} 的 Enter snapshot 不一致"
            );
            assert_eq!(fast.commit(), legacy.commit(), "{input:?} 的 commit 不一致");
        }
    }

    /// `Fast(JA)` 的 golden：`ja_only` + 解碼器讓第一鍵就是日文 `Claim`。
    #[test]
    fn fast_ja_matches_legacy_with_ja_only() {
        use crate::ja::decode::{JaDecoder, JaWordMatch, JaWordSource};
        use crate::ja::lexicon::JaEntry;
        use crate::session::{Session, SessionConfig};
        use std::sync::Arc;

        struct EmptyJaSource;

        impl JaWordSource for EmptyJaSource {
            fn longest_reading_units(&self) -> usize {
                0
            }

            fn matches<'a>(
                &'a self,
                _reading: &[char],
                _start: usize,
                _out: &mut dyn FnMut(JaWordMatch<'a>),
            ) {
            }

            fn exact<'a>(&'a self, _reading: &[char]) -> Vec<JaEntry<'a>> {
                Vec::new()
            }

            fn prefixed<'a>(&'a self, _reading: &[char]) -> Vec<JaEntry<'a>> {
                Vec::new()
            }
        }

        fn ja_session() -> Session {
            let mut config = SessionConfig::new(Mode::Mixed);
            config.ja_only = true;
            config.ja_live.enabled = false;
            config.ja = Some(Arc::new(JaDecoder::new(Arc::new(EmptyJaSource))));
            Session::new(config)
        }

        let mut fast = ja_session();
        let mut legacy = ja_session();
        legacy.set_legacy_routing(true);
        assert_eq!(
            fast.route_for_test('w'),
            Route::Fast(LanguageId::JA),
            "ja_only 的第一鍵要走 Fast(JA)"
        );

        for input in ["watashi", "ka"] {
            // 上面已經先問過 'w'；這裡用新 session 逐鍵對照。
            let mut fast = ja_session();
            let mut legacy = ja_session();
            legacy.set_legacy_routing(true);
            for (at, ch) in input.chars().enumerate() {
                let fast_snapshot = fast.press(Key::char(ch, false));
                let legacy_snapshot = legacy.press(Key::char(ch, false));
                assert_eq!(
                    fast_snapshot, legacy_snapshot,
                    "Fast(JA) {input:?} 第 {at} 鍵的 snapshot 不一致"
                );
            }
            let fast_enter = fast.press(Key::new(KeyCode::Enter, None, false));
            let legacy_enter = legacy.press(Key::new(KeyCode::Enter, None, false));
            assert_eq!(
                fast_enter, legacy_enter,
                "Fast(JA) {input:?} 的 Enter snapshot 不一致"
            );
            assert_eq!(
                fast.commit(),
                legacy.commit(),
                "Fast(JA) {input:?} 的 commit 不一致"
            );
        }
    }

    struct ClaimEngine {
        id: LanguageId,
    }

    impl ClaimEngine {
        fn new(id: &'static str) -> Self {
            Self {
                id: LanguageId::from_static(id),
            }
        }
    }

    impl LanguageEngine for ClaimEngine {
        fn id(&self) -> LanguageId {
            self.id
        }

        fn interest(&self, _key: &Key, _ctx: &InputContext<'_>) -> Interest {
            Interest::Claim
        }

        fn feed(&mut self, _key: &Key, _ctx: &InputContext<'_>) -> Option<Hypothesis> {
            None
        }

        fn candidates(&self, _span: Span) -> Vec<crate::candidate::Candidate> {
            Vec::new()
        }

        fn apply(&mut self, _action: &crate::session::CandidateAction) {}

        fn commit_unit(&self) -> CommitUnit {
            CommitUnit::Run
        }

        fn style(&self) -> StyleHint {
            StyleHint::Literal
        }
    }

    /// 多個 `Claim` 時 router 必須退回既有路徑，結果與 base 相同。
    #[test]
    fn multiple_claims_fall_back_to_the_legacy_path() {
        use crate::session::{Session, SessionConfig};

        let mut fast = Session::new(SessionConfig::new(Mode::Mixed));
        let mut legacy = Session::new(SessionConfig::new(Mode::Mixed));
        legacy.set_legacy_routing(true);
        for session in [&mut fast, &mut legacy] {
            session.register_engine_for_test(Box::new(ClaimEngine::new("test-a")));
            session.register_engine_for_test(Box::new(ClaimEngine::new("test-b")));
        }

        assert_eq!(
            fast.route_for_test('j'),
            Route::Fallback,
            "多個 Claim 不可挑其中一個跑"
        );

        // 全新 session 對照；上面的 route 查詢不改變組字狀態。
        let mut fast = Session::new(SessionConfig::new(Mode::Mixed));
        let mut legacy = Session::new(SessionConfig::new(Mode::Mixed));
        legacy.set_legacy_routing(true);
        for session in [&mut fast, &mut legacy] {
            session.register_engine_for_test(Box::new(ClaimEngine::new("test-a")));
            session.register_engine_for_test(Box::new(ClaimEngine::new("test-b")));
        }
        for (at, ch) in "ji3su3".chars().enumerate() {
            let fast_snapshot = fast.press(Key::char(ch, false));
            let legacy_snapshot = legacy.press(Key::char(ch, false));
            assert_eq!(
                fast_snapshot, legacy_snapshot,
                "多 Claim fallback 第 {at} 鍵的 snapshot 不一致"
            );
        }
        let fast_enter = fast.press(Key::new(KeyCode::Enter, None, false));
        let legacy_enter = legacy.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(fast_enter, legacy_enter);
        assert_eq!(fast.commit(), legacy.commit());
    }

    #[test]
    fn candidate_action_engine_defaults_to_none() {
        assert!(crate::session::CandidateAction::default().engine.is_none());
    }
}
