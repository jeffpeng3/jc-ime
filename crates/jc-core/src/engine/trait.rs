//! 語言引擎介面。
//!
//! 引擎只回資料：`interest()` 判斷這一鍵屬不屬於自己，`feed()` 產生
//! [`Hypothesis`]，`candidates()` 提供候選。畫面、組字與提交仍由
//! [`crate::session::Session`] 執行。

use crate::candidate::Candidate;
use crate::key::Key;
use crate::session::CandidateAction;

use super::{
    CommitUnit, Hypothesis, InputContext, Interest, LanguageId, Released, Span, StyleHint,
};

/// 引擎種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineKind {
    /// 參與語言假設與合併。
    Language,
    /// 前導之類的控制引擎；獨佔按鍵，不進語言合併。
    Control,
}

/// 一個語言引擎。
///
/// P1 註冊的三個 adapter 都是 owned：不借用 `&mut Session`，因此可以放進
/// `Box<dyn LanguageEngine + 'static>`。
pub trait LanguageEngine: Send {
    /// 穩定 id；同一 registry 不可重複。
    fn id(&self) -> LanguageId;

    /// 引擎種類；預設是語言引擎。
    fn kind(&self) -> EngineKind {
        EngineKind::Language
    }

    /// 複製這次按鍵的 O(1) lane 快照。
    ///
    /// P1 的 adapter 用它更新自己的路由狀態；`interest()` 只讀這裡。
    fn observe(&mut self, _ctx: &InputContext<'_>) {}

    /// O(1) 的興趣判斷；不得轉換、不得查字典。
    fn interest(&self, key: &Key, ctx: &InputContext<'_>) -> Interest;

    /// 吃一鍵，回傳目前這段的假設。
    ///
    /// P1 的 router 只呼叫 `interest()`；真正的 lane 執行仍留在 `Session`。
    /// 這個方法先保留給 P2，adapter 只維護自己的 shadow 狀態。
    #[allow(dead_code)] // P1 的 router 只選 lane；P2 才呼叫 feed()。
    fn feed(&mut self, key: &Key, ctx: &InputContext<'_>) -> Option<Hypothesis>;

    /// `span` 這段的其他候選；順序即建議順序。
    #[allow(dead_code)] // P1 候選仍由 CandidateSet 產生；P2 才接。
    fn candidates(&self, span: Span) -> Vec<Candidate>;

    /// 套用被選中的動作；只更新引擎狀態，不改畫面、不提交。
    #[allow(dead_code)] // P1 的 apply() 還是 no-op；P2 才接。
    fn apply(&mut self, action: &CandidateAction);

    /// Enter／空白一次送出的單位。
    #[allow(dead_code)] // P1 只保留介面；P2 的 commit 流程會用。
    fn commit_unit(&self) -> CommitUnit;

    /// 顯示風格。
    #[allow(dead_code)] // P1 只保留介面；P2 的顯示流程會用。
    fn style(&self) -> StyleHint;

    /// 把先前吃掉的按鍵交回 router；沒有就回 `None`。
    #[allow(dead_code)] // P1 的 handoff 仍由 Session 舊函式執行。
    fn release(&mut self) -> Option<Released> {
        None
    }

    /// 清空這次組字；取消、提交、換 session 時呼叫。
    #[allow(dead_code)] // P1 的 Session 清除仍走舊路徑。
    fn reset(&mut self) {}
}
