//! 日文羅馬字引擎 adapter。
//!
//! P1 只 owned 路由狀態；`RomajiInput`、`JaDecoder` 與 `LiveController` 仍
//! 留在 `Session`。`LiveController` 的 `Controller` 不保證 `Send`，因此不在
//! 這一輪搬進引擎。

use crate::candidate::Candidate;
use crate::key::{Key, KeyCode};
use crate::session::CandidateAction;

use super::{
    CommitUnit, Hypothesis, InputContext, Interest, LaneView, LanguageEngine, LanguageId, Span,
    StateToken, StyleHint,
};

/// owned 日文 adapter。
#[derive(Debug)]
#[allow(dead_code)] // P1 shadow 欄位；feed() 在 P2 才會讀。
pub struct JaEngine {
    id: LanguageId,
    lanes: LaneView,
    epoch: u64,
    revision: u64,
    run_keys: String,
}

impl Default for JaEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl JaEngine {
    /// 建立日文引擎。
    pub fn new() -> Self {
        Self {
            id: LanguageId::JA,
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

impl LanguageEngine for JaEngine {
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
        // 已擁有這段輸入，或手動 ja_only：這一鍵一定是日文。
        if ctx.active == Some(self.id) || ctx.lanes.ja_only {
            return Interest::Claim;
        }
        // 注音 lane 優先；既有 `press_char()` 也是先看日文，再看注音。
        if ctx.active == Some(LanguageId::ZH) || ctx.lanes.zh_lane {
            return Interest::No;
        }
        // 沒有解碼器時日文不參與，按鍵照既有混打路徑處理。
        if !ctx.lanes.ja_available {
            return Interest::No;
        }
        // 羅馬字前綴至少要兩個小寫 ASCII：目前待定按鍵至少一鍵，這一鍵
        // 再補一鍵。真正的 `kana_for()`／英文詞前綴判斷留到 P2 的 `feed()`。
        if ch.is_ascii_lowercase() && ctx.lanes.ascii_len >= 1 && ctx.lanes.ascii_pieces_empty {
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
        // P1 候選仍由 `Session::rebuild_ja()` 產生。
        Vec::new()
    }

    fn apply(&mut self, _action: &CandidateAction) {}

    fn commit_unit(&self) -> CommitUnit {
        CommitUnit::Clause
    }

    fn style(&self) -> StyleHint {
        StyleHint::JaKana
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

    fn ja_lanes() -> LaneView {
        LaneView {
            ascii_empty: false,
            ascii_len: 1,
            ascii_pieces_empty: true,
            syllables_empty: true,
            ja_available: true,
            ..LaneView::default()
        }
    }

    #[test]
    fn romaji_prefix_is_maybe_not_a_claim() {
        let engine = JaEngine::new();
        assert_eq!(
            engine.interest(&Key::char('a', false), &ctx(None, ja_lanes())),
            Interest::Maybe
        );
    }

    #[test]
    fn active_ja_lane_is_a_claim() {
        let engine = JaEngine::new();
        assert_eq!(
            engine.interest(
                &Key::char('a', false),
                &ctx(Some(LanguageId::JA), LaneView::default())
            ),
            Interest::Claim
        );
    }

    #[test]
    fn no_decoder_means_no_participation() {
        let engine = JaEngine::new();
        let lanes = LaneView {
            ascii_empty: false,
            ascii_len: 1,
            ascii_pieces_empty: true,
            ja_available: false,
            ..LaneView::default()
        };
        assert_eq!(
            engine.interest(&Key::char('a', false), &ctx(None, lanes)),
            Interest::No
        );
    }

    #[test]
    fn ja_only_is_a_claim_even_without_a_decoder() {
        let engine = JaEngine::new();
        let lanes = LaneView {
            ja_only: true,
            ..LaneView::default()
        };
        assert_eq!(
            engine.interest(&Key::char('a', false), &ctx(None, lanes)),
            Interest::Claim
        );
    }

    #[test]
    fn zh_lane_excludes_japanese() {
        let engine = JaEngine::new();
        let lanes = LaneView {
            zh_lane: true,
            ..ja_lanes()
        };
        assert_eq!(
            engine.interest(&Key::char('a', false), &ctx(None, lanes)),
            Interest::No
        );
    }
}
