//! 英數／混打引擎 adapter。
//!
//! P1 只 owned 路由狀態；`AsciiInput`、`crate::mixed::Router` 與候選仍留在
//! `Session`。`feed()` 是 shadow，P2 才由引擎產生真正的字面／混打假設。

use crate::candidate::Candidate;
use crate::key::{Key, KeyCode};
use crate::session::CandidateAction;
use crate::zh::han::is_han;

use super::{
    CommitUnit, Hypothesis, InputContext, Interest, LaneView, LanguageEngine, LanguageId, Span,
    StateToken, StyleHint,
};

/// owned ASCII adapter。
#[derive(Debug)]
#[allow(dead_code)] // P1 shadow 欄位；feed() 在 P2 才會讀。
pub struct AsciiEngine {
    id: LanguageId,
    lanes: LaneView,
    epoch: u64,
    revision: u64,
    run_keys: String,
}

impl Default for AsciiEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AsciiEngine {
    /// 建立 ASCII 引擎。
    pub fn new() -> Self {
        Self {
            id: LanguageId::ASCII,
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

/// 可顯示的英數字元。
fn printable(ch: char) -> bool {
    ch == ' ' || ch.is_ascii_graphic()
}

impl LanguageEngine for AsciiEngine {
    fn id(&self) -> LanguageId {
        self.id
    }

    fn observe(&mut self, ctx: &InputContext<'_>) {
        self.lanes = ctx.lanes;
        self.epoch = self.epoch.wrapping_add(1);
    }

    fn interest(&self, key: &Key, ctx: &InputContext<'_>) -> Interest {
        let KeyCode::Char(ch) = key.code else {
            return Interest::No;
        };
        // 候選窗開著時數字鍵屬於候選介面，不歸語言引擎。
        if ctx.candidate_window && ch.is_ascii_digit() {
            return Interest::No;
        }
        if printable(ch) || is_han(ch) {
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
        // P1 候選仍由 `Session::rebuild_mixed()` 與 `CandidateSet` 產生。
        Vec::new()
    }

    fn apply(&mut self, _action: &CandidateAction) {}

    fn commit_unit(&self) -> CommitUnit {
        CommitUnit::Run
    }

    fn style(&self) -> StyleHint {
        StyleHint::Literal
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

    fn ctx(candidate_window: bool) -> InputContext<'static> {
        let mut context = InputContext::routing(Mode::Mixed, None, LaneView::default());
        context.candidate_window = candidate_window;
        context
    }

    #[test]
    fn printable_ascii_is_maybe() {
        let engine = AsciiEngine::new();
        assert_eq!(
            engine.interest(&Key::char('!', false), &ctx(false)),
            Interest::Maybe
        );
    }

    #[test]
    fn candidate_digit_is_not_mine() {
        let engine = AsciiEngine::new();
        assert_eq!(
            engine.interest(&Key::char('1', false), &ctx(true)),
            Interest::No
        );
        assert_eq!(
            engine.interest(&Key::char('1', false), &ctx(false)),
            Interest::Maybe
        );
    }

    #[test]
    fn named_keys_and_han_are_classified() {
        let engine = AsciiEngine::new();
        assert_eq!(
            engine.interest(&Key::new(KeyCode::Enter, None, false), &ctx(false)),
            Interest::No
        );
        assert_eq!(
            engine.interest(&Key::char('我', false), &ctx(false)),
            Interest::Maybe
        );
    }
}
