//! 注音引擎 adapter。
//!
//! P1 只把路由需要的狀態 owned 在引擎裡；`ZhInput`、音節與候選仍留在
//! `Session`，由既有 `press_zh_char()`／`rebuild_zh()` 執行。`feed()` 目前是
//! shadow：不驅動畫面，P2 才接上真正的假設產生。

use crate::candidate::Candidate;
use crate::key::{Key, KeyCode};
use crate::session::CandidateAction;
use crate::zh::keyboard::Keyboard;
use crate::zh::syllable::{class_of, Class};

use super::{
    CommitUnit, Hypothesis, InputContext, Interest, LaneView, LanguageEngine, LanguageId, Span,
    StateToken, StyleHint,
};

/// Mixed 從空白開始打注音鍵時，這一鍵會不會開啟注音 lane。
///
/// 與 `Session::mixed_zh_lane_starts()` 的按鍵分類共用，避免兩份規則漂移。
/// `keyboard` 由 `Session` 傳入，未來非標準排列也會與 `ZhInput` 一致。
/// 只看按鍵本身，不讀詞典、不掃按鍵流。
pub(crate) fn starts_key(keyboard: Keyboard, ch: char) -> bool {
    let class = class_of(ch);
    if class != Class::None {
        return class != Class::Tone;
    }
    // 鍵盤上的標點（`, . / ; -`）雖然也是注音鍵，單獨按下時是字面
    // 標點，不自動開始一個注音音節。
    keyboard.symbol(ch).is_some_and(|symbol| {
        class_of(symbol) != Class::Tone && (ch.is_ascii_alphabetic() || ch.is_ascii_digit())
    })
}

/// 這一鍵是不是注音鍵盤上的鍵。
pub(crate) fn key_is_zh(keyboard: Keyboard, ch: char) -> bool {
    class_of(ch) != Class::None || keyboard.symbol(ch).is_some()
}

/// owned 注音 adapter。
#[derive(Debug)]
#[allow(dead_code)] // P1 shadow 欄位；feed() 在 P2 才會讀。
pub struct ZhEngine {
    id: LanguageId,
    lanes: LaneView,
    epoch: u64,
    revision: u64,
    /// P1 shadow：這次 run 看過的按鍵。`Session` 不讀它。
    run_keys: String,
}

impl Default for ZhEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ZhEngine {
    /// 建立注音引擎。
    pub fn new() -> Self {
        Self {
            id: LanguageId::ZH,
            lanes: LaneView::default(),
            epoch: 0,
            revision: 0,
            run_keys: String::new(),
        }
    }

    #[allow(dead_code)] // P1 shadow token；P2 的 feed() 才會回傳。
    fn token(&self) -> StateToken {
        StateToken {
            engine: self.id,
            epoch: self.epoch,
            revision: self.revision,
        }
    }
}

impl LanguageEngine for ZhEngine {
    fn id(&self) -> LanguageId {
        self.id
    }

    fn observe(&mut self, ctx: &InputContext<'_>) {
        self.lanes = ctx.lanes;
        self.epoch = self.epoch.wrapping_add(1);
    }

    fn interest(&self, key: &Key, ctx: &InputContext<'_>) -> Interest {
        if ctx.lanes.prefix_active {
            return Interest::No;
        }
        let KeyCode::Char(ch) = key.code else {
            return Interest::No;
        };
        // sticky：已經在注音 lane 就一定是我的。真正打不成注音時由既有
        // `mixed_zh_lane_continues()` 交回混打。
        if ctx.active == Some(self.id) || ctx.lanes.zh_lane {
            return Interest::Claim;
        }
        if ctx.active == Some(LanguageId::JA) || ctx.lanes.ja_lane {
            return Interest::No;
        }
        if ctx.lanes.ascii_empty && ctx.lanes.syllables_empty && starts_key(ctx.lanes.keyboard, ch)
        {
            return Interest::Claim;
        }
        if key_is_zh(ctx.lanes.keyboard, ch) {
            return Interest::Maybe;
        }
        Interest::No
    }

    fn feed(&mut self, key: &Key, ctx: &InputContext<'_>) -> Option<Hypothesis> {
        if ctx.expired() {
            return None;
        }
        let KeyCode::Char(ch) = key.code else {
            return None;
        };
        // transaction：先確認所有條件，再提交 shadow 狀態。
        let mut shadow = self.run_keys.clone();
        shadow.push(ch);
        self.run_keys = shadow;
        self.revision = self.revision.wrapping_add(1);
        Some(Hypothesis {
            span: Span::new(0, self.run_keys.len()),
            surface: ctx.preedit.to_string(),
            reading: String::new(),
            score: 0,
            state_token: self.token(),
        })
    }

    fn candidates(&self, _span: Span) -> Vec<Candidate> {
        // P1 候選仍由 `Session::rebuild_zh()` 與 `CandidateSet` 產生。
        Vec::new()
    }

    fn apply(&mut self, _action: &CandidateAction) {}

    fn commit_unit(&self) -> CommitUnit {
        CommitUnit::Syllable
    }

    fn style(&self) -> StyleHint {
        StyleHint::ZhFullwidth
    }

    fn reset(&mut self) {
        self.run_keys.clear();
        self.revision = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::Mode;

    fn ctx(active: Option<LanguageId>, lanes: LaneView) -> InputContext<'static> {
        InputContext::routing(Mode::Mixed, active, lanes)
    }

    #[test]
    fn starts_key_matches_the_session_classification() {
        assert!(starts_key(Keyboard::standard(), 'j'));
        // 聲調鍵不會自己開 lane。
        assert!(!starts_key(Keyboard::standard(), '3'));
        assert!(!starts_key(Keyboard::standard(), ' '));
        assert!(!starts_key(Keyboard::standard(), '!'));
    }

    #[test]
    fn tone_key_inside_the_lane_is_a_claim() {
        let engine = ZhEngine::new();
        let lanes = LaneView {
            zh_lane: true,
            ..LaneView::default()
        };
        assert_eq!(
            engine.interest(&Key::char('3', false), &ctx(Some(LanguageId::ZH), lanes)),
            Interest::Claim
        );
    }

    #[test]
    fn starting_key_is_a_claim_only_on_an_empty_run() {
        let engine = ZhEngine::new();
        let empty = LaneView {
            ascii_empty: true,
            ascii_pieces_empty: true,
            syllables_empty: true,
            ..LaneView::default()
        };
        assert_eq!(
            engine.interest(&Key::char('j', false), &ctx(None, empty)),
            Interest::Claim
        );
        let dirty = LaneView {
            ascii_empty: false,
            ascii_pieces_empty: true,
            syllables_empty: true,
            ..LaneView::default()
        };
        assert_eq!(
            engine.interest(&Key::char('j', false), &ctx(None, dirty)),
            Interest::Maybe
        );
    }

    #[test]
    fn non_zh_ascii_is_not_a_candidate() {
        let engine = ZhEngine::new();
        assert_eq!(
            engine.interest(&Key::char('!', false), &ctx(None, LaneView::default())),
            Interest::No
        );
        assert_eq!(
            engine.interest(
                &Key::new(KeyCode::Enter, None, false),
                &ctx(None, LaneView::default())
            ),
            Interest::No
        );
    }

    #[test]
    fn an_expired_feed_does_not_advance_shadow_state() {
        let mut engine = ZhEngine::new();
        let mut context = ctx(None, LaneView::default());
        context.deadline = Some(std::time::Instant::now());
        assert_eq!(engine.feed(&Key::char('j', false), &context), None);
        let live = ctx(None, LaneView::default());
        let hypothesis = engine
            .feed(&Key::char('j', false), &live)
            .expect("未過期時要產生 shadow 假設");
        assert_eq!(hypothesis.span, Span::new(0, 1));
        assert_eq!(hypothesis.state_token.engine, LanguageId::ZH);
    }
}
