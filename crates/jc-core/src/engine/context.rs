//! 一鍵的唯讀輸入上下文。

use std::time::Instant;

use crate::lang::Mode;
use crate::zh::keyboard::Keyboard;

use super::LanguageId;

/// `interest()` 可用的 lane 快照。
///
/// P1 只把路由需要的 O(1) 旗標放進來；真正的轉換狀態仍由 `Session` 持有。
/// 這是 owned adapter 與 `Session` 之間唯一的路由接縫，不借用 `&mut Session`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaneView {
    /// 這次組字是否走純注音路徑。
    pub zh_lane: bool,
    /// 這次組字是否走日文羅馬字路徑（含手動 `ja_only`）。
    pub ja_lane: bool,
    /// 「Mixed 但只走羅馬字」的手動入口。
    pub ja_only: bool,
    /// `AsciiInput` 是否完全沒有按鍵。
    pub ascii_empty: bool,
    /// `AsciiInput::typed()` 的位元組長度；只用來判斷羅馬字前綴長度。
    pub ascii_len: usize,
    /// `AsciiInput` 是否還沒有任何已完成的片段。
    pub ascii_pieces_empty: bool,
    /// 是否還沒有已完成的注音音節。
    pub syllables_empty: bool,
    /// 注音是否還有未完成的按鍵。
    pub zh_pending: bool,
    /// 這個 session 是否載入日文解碼器；沒有時日文引擎不參與。
    pub ja_available: bool,
    /// 前導序列（反引號、`u`／`b`）是否正在進行。
    ///
    /// 前導由控制引擎獨佔；語言引擎看到這個旗標一律回 `No`，不進 Viterbi，
    /// 也不接收按鍵。
    pub prefix_active: bool,
    /// 注音鍵盤排列；`starts_key()` 用它避免與 `ZhInput` 漂移。
    pub keyboard: Keyboard,
}

impl Default for LaneView {
    fn default() -> Self {
        Self {
            zh_lane: false,
            ja_lane: false,
            ja_only: false,
            ascii_empty: true,
            ascii_len: 0,
            ascii_pieces_empty: true,
            syllables_empty: true,
            zh_pending: false,
            ja_available: false,
            prefix_active: false,
            keyboard: Keyboard::standard(),
        }
    }
}

/// 一鍵的唯讀上下文；不含 `&mut Session`，引擎不能從這裡改畫面。
#[derive(Debug)]
#[allow(dead_code)] // `mode`／`cursor_rune` 是 P2 context 用的欄位。
pub struct InputContext<'a> {
    /// 目前輸入模式。
    pub mode: Mode,
    /// 目前擁有這一串輸入的引擎；`None` 表示尚未決定。
    pub active: Option<LanguageId>,
    /// 是否正在組字。
    pub composing: bool,
    /// 候選窗是否開著。
    pub candidate_window: bool,
    /// 游標在顯示字串的 rune 位移；只用來判斷前後，不拿來切按鍵。
    pub cursor_rune: usize,
    /// 目前完整的原始按鍵流（UTF-8）。
    pub keys: &'a str,
    /// 目前組字顯示字串。
    pub preedit: &'a str,
    /// 已固定的左文，供 context scorer 使用；`interest()` 不得讀。
    pub left_context: &'a str,
    /// P2 的軟／硬預算；`None` 表示同步路徑不限時。
    pub deadline: Option<Instant>,
    /// P1 的 lane 快照；所有欄位都是 O(1) 複製。
    pub lanes: LaneView,
}

impl<'a> InputContext<'a> {
    /// 建立只帶路由資訊的上下文。
    ///
    /// 字串欄位留空是刻意的：P1 的 `interest()` 不得讀按鍵流或前文字，
    /// `Session` 也不必為每個按鍵複製整段 preedit。
    pub fn routing(mode: Mode, active: Option<LanguageId>, lanes: LaneView) -> Self {
        Self {
            mode,
            active,
            composing: lanes.zh_lane || lanes.ja_lane || !lanes.ascii_empty,
            candidate_window: false,
            cursor_rune: 0,
            keys: "",
            preedit: "",
            left_context: "",
            deadline: None,
            lanes,
        }
    }

    /// 引擎在長迴圈裡定期檢查；過期就回 `None`。
    #[allow(dead_code)] // P2 的 feed() 才會呼叫。
    pub fn expired(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lanes() -> LaneView {
        LaneView {
            ascii_empty: true,
            ascii_pieces_empty: true,
            syllables_empty: true,
            ..LaneView::default()
        }
    }

    #[test]
    fn routing_context_does_not_copy_input_text() {
        let ctx = InputContext::routing(Mode::Mixed, None, lanes());
        assert!(ctx.keys.is_empty());
        assert!(ctx.preedit.is_empty());
        assert!(ctx.left_context.is_empty());
        assert!(!ctx.composing);
        assert!(ctx.lanes.syllables_empty);
    }

    #[test]
    fn expired_only_when_a_deadline_has_passed() {
        let mut ctx = InputContext::routing(Mode::Mixed, None, lanes());
        assert!(!ctx.expired());
        ctx.deadline = Some(Instant::now());
        assert!(ctx.expired());
    }
}
