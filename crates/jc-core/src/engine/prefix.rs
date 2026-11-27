//! 前導解析引擎：反引號標點與 `u`／`b` 碼位查詢。
//!
//! 這是第一個 [`EngineKind::Control`] 引擎：`interest()` 只在反引號被按下、
//! 或前導序列已經開始時回 `Claim`。前導期間其他語言引擎不參與，按鍵由
//! [`PrefixEngine::press()`] 的狀態機獨佔。
//!
//! 引擎只維護狀態與顯示資料；組字、游標與提交仍由
//! [`crate::session::Session`] 執行。`u` 的碼位是 Unicode scalar，`b` 的碼位
//! 走 [`crate::big5::big5_to_unicode`] 的跨平台查表。

use crate::candidate::Candidate;
use crate::key::{Key, KeyCode};
use crate::session::CandidateAction;

use super::{
    CommitUnit, Hypothesis, InputContext, Interest, LanguageEngine, LanguageId, Span, StateToken,
    StyleHint,
};

/// 碼位種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CodeKind {
    /// Unicode scalar；1–6 位十六進位。
    Unicode,
    /// Big5 雙位元組；正好 4 位十六進位。
    Big5,
}

impl CodeKind {
    /// 最多接受幾位十六進位。
    fn max_digits(self) -> usize {
        match self {
            Self::Unicode => 6,
            Self::Big5 => 4,
        }
    }

    /// 解析完成後的字元；失敗回 `None`。
    fn decode(self, digits: &str) -> Option<char> {
        match self {
            Self::Unicode => {
                if digits.is_empty() || digits.len() > self.max_digits() {
                    return None;
                }
                let value = u32::from_str_radix(digits, 16).ok()?;
                char::from_u32(value)
            }
            Self::Big5 => {
                if digits.len() != 4 {
                    return None;
                }
                let code = u16::from_str_radix(digits, 16).ok()?;
                crate::big5::big5_to_unicode(code)
            }
        }
    }
}

/// 前導狀態。
#[derive(Clone, Debug, PartialEq, Eq)]
enum PrefixState {
    /// 不在前導。
    Idle,
    /// 已按反引號，等下一個鍵決定是標點還是碼位。
    AwaitKind { raw: String },
    /// 已看到 `u`／`b`，正在收集十六進位。
    Collect { kind: CodeKind, digits: String },
    /// 遇到不合法的字元；保留原文，等空白收尾。
    Invalid { raw: String },
}

/// 一鍵對畫面造成的效果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PrefixEffect {
    /// 開始反引號前導；由 `Session` 走既有 lane 交給混打管線。
    Start,
    /// 把字面接在目前組字後面。
    Display(String),
    /// 用 `text` 取代最後 `raw_len` 個顯示字元。
    Replace { raw_len: usize, text: String },
    /// 移除最後 `raw_len` 個顯示字元。
    Remove { raw_len: usize },
    /// 前導結束，畫面維持原樣（錯誤時 raw 照字面留下）。
    Keep,
    /// 前導放開這一鍵，交給既有路徑（標點前導或一般按鍵）。
    Release,
}

/// 前導解析引擎。
#[derive(Debug)]
pub struct PrefixEngine {
    id: LanguageId,
    state: PrefixState,
    epoch: u64,
    revision: u64,
}

impl Default for PrefixEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl PrefixEngine {
    /// 建立前導引擎。
    pub fn new() -> Self {
        Self {
            id: LanguageId::PREFIX,
            state: PrefixState::Idle,
            epoch: 0,
            revision: 0,
        }
    }

    /// 是否正在前導序列中。
    pub(crate) fn is_active(&self) -> bool {
        !matches!(self.state, PrefixState::Idle)
    }

    /// 目前顯示的原文；不在前導時是空字串。
    pub(crate) fn raw(&self) -> String {
        match &self.state {
            PrefixState::Idle => String::new(),
            PrefixState::AwaitKind { raw } => raw.clone(),
            PrefixState::Collect { kind, digits } => {
                let kind = match kind {
                    CodeKind::Unicode => 'u',
                    CodeKind::Big5 => 'b',
                };
                format!("`{kind}{digits}")
            }
            PrefixState::Invalid { raw } => raw.clone(),
        }
    }

    /// 取消前導；不留下任何字。
    #[cfg(test)]
    #[allow(dead_code)] // 狀態機測試用；Session 走 Esc 的按鍵路徑。
    pub(crate) fn cancel(&mut self) -> PrefixEffect {
        if !self.is_active() {
            return PrefixEffect::Release;
        }
        let raw_len = self.raw().chars().count();
        self.state = PrefixState::Idle;
        PrefixEffect::Remove { raw_len }
    }

    /// 吃一鍵並回傳畫面效果。
    ///
    /// 只有 `Claim` 的按鍵才該送到這裡；`Session` 先用 [`Self::interest`]
    /// 判斷，其他引擎不會在這段期間收到按鍵。
    pub(crate) fn press(&mut self, key: &Key, ctx: &InputContext<'_>) -> PrefixEffect {
        if ctx.expired() {
            return PrefixEffect::Release;
        }
        self.revision = self.revision.wrapping_add(1);
        let named = !matches!(key.code, KeyCode::Char(_));
        let ch = match key.code {
            KeyCode::Char(ch) => Some(key.text.unwrap_or(ch)),
            _ => None,
        };
        let is_space = key.code == KeyCode::Space || ch == Some(' ');

        match std::mem::replace(&mut self.state, PrefixState::Idle) {
            PrefixState::Idle => {
                if ch == Some('`') {
                    self.state = PrefixState::AwaitKind {
                        raw: "`".to_string(),
                    };
                    PrefixEffect::Start
                } else {
                    PrefixEffect::Release
                }
            }
            PrefixState::AwaitKind { mut raw } => {
                if matches!(key.code, KeyCode::Escape | KeyCode::Backspace) {
                    let raw_len = raw.chars().count();
                    PrefixEffect::Remove { raw_len }
                } else if let Some(ch) = ch {
                    let kind = match ch {
                        'u' | 'U' => Some(CodeKind::Unicode),
                        'b' | 'B' => Some(CodeKind::Big5),
                        _ => None,
                    };
                    if let Some(kind) = kind {
                        raw.push(ch);
                        self.state = PrefixState::Collect {
                            kind,
                            digits: String::new(),
                        };
                        PrefixEffect::Display(ch.to_string())
                    } else {
                        // 既有標點前導接手；狀態已回到 Idle。
                        PrefixEffect::Release
                    }
                } else {
                    // Enter／方向鍵：raw 照字面留下，按鍵照一般路徑處理。
                    PrefixEffect::Release
                }
            }
            PrefixState::Collect { kind, mut digits } => {
                if matches!(key.code, KeyCode::Escape) {
                    let raw_len = kind_raw_len(kind, &digits);
                    PrefixEffect::Remove { raw_len }
                } else if key.code == KeyCode::Backspace {
                    if digits.pop().is_none() {
                        // 位數已空：再退一次取消整個前導。
                        let raw_len = kind_raw_len(kind, &digits);
                        PrefixEffect::Remove { raw_len }
                    } else {
                        self.state = PrefixState::Collect { kind, digits };
                        PrefixEffect::Remove { raw_len: 1 }
                    }
                } else if is_space {
                    match kind.decode(&digits) {
                        Some(ch) => {
                            let raw_len = kind_raw_len(kind, &digits);
                            PrefixEffect::Replace {
                                raw_len,
                                text: ch.to_string(),
                            }
                        }
                        None => {
                            // 錯誤：raw 照字面留下，游標可以繼續前進。
                            PrefixEffect::Keep
                        }
                    }
                } else if let Some(ch) = ch {
                    if ch.is_ascii_hexdigit() && digits.len() < kind.max_digits() {
                        digits.push(ch);
                        self.state = PrefixState::Collect { kind, digits };
                        PrefixEffect::Display(ch.to_string())
                    } else {
                        let raw = format!("`{}{digits}{ch}", kind_char(kind));
                        self.state = PrefixState::Invalid { raw };
                        PrefixEffect::Display(ch.to_string())
                    }
                } else if named {
                    // Enter／方向鍵等：清除前導，raw 留下。
                    PrefixEffect::Release
                } else {
                    PrefixEffect::Release
                }
            }
            PrefixState::Invalid { mut raw } => {
                if matches!(key.code, KeyCode::Escape) {
                    let raw_len = raw.chars().count();
                    PrefixEffect::Remove { raw_len }
                } else if key.code == KeyCode::Backspace {
                    raw.pop();
                    if raw.chars().count() <= 2 {
                        let raw_len = raw.chars().count();
                        PrefixEffect::Remove { raw_len }
                    } else {
                        self.state = PrefixState::Invalid { raw };
                        PrefixEffect::Remove { raw_len: 1 }
                    }
                } else if is_space {
                    // 錯誤收尾：原文已經是字面，維持不動。
                    PrefixEffect::Keep
                } else if let Some(ch) = ch {
                    raw.push(ch);
                    self.state = PrefixState::Invalid { raw };
                    PrefixEffect::Display(ch.to_string())
                } else if named {
                    PrefixEffect::Release
                } else {
                    self.state = PrefixState::Invalid { raw };
                    PrefixEffect::Release
                }
            }
        }
    }

    fn token(&self) -> StateToken {
        StateToken {
            engine: self.id,
            epoch: self.epoch,
            revision: self.revision,
        }
    }
}

/// 前導原文的顯示長度（含反引號與 `u`／`b`）。
fn kind_raw_len(_kind: CodeKind, digits: &str) -> usize {
    1 + 1 + digits.chars().count()
}

fn kind_char(kind: CodeKind) -> char {
    match kind {
        CodeKind::Unicode => 'u',
        CodeKind::Big5 => 'b',
    }
}

impl LanguageEngine for PrefixEngine {
    fn id(&self) -> LanguageId {
        self.id
    }

    fn kind(&self) -> super::EngineKind {
        super::EngineKind::Control
    }

    fn observe(&mut self, _ctx: &InputContext<'_>) {
        self.epoch = self.epoch.wrapping_add(1);
    }

    fn interest(&self, key: &Key, _ctx: &InputContext<'_>) -> Interest {
        // 前導序列進行中，任何鍵都先歸前導引擎（含 Esc、Backspace、空白），
        // 才不會被其他引擎插隊。
        if self.is_active() {
            return Interest::Claim;
        }
        if !key.ctrl && key.code == KeyCode::Char('`') {
            return Interest::Claim;
        }
        Interest::No
    }

    fn feed(&mut self, key: &Key, ctx: &InputContext<'_>) -> Option<Hypothesis> {
        let effect = self.press(key, ctx);
        let raw = self.raw();
        if raw.is_empty() && effect == PrefixEffect::Release {
            return None;
        }
        Some(Hypothesis {
            span: Span::new(0, raw.len()),
            surface: raw,
            reading: String::new(),
            score: 0,
            state_token: self.token(),
        })
    }

    fn candidates(&self, _span: Span) -> Vec<Candidate> {
        // 標點變體仍由 `Session::open_prefix_variants()` 產生。
        Vec::new()
    }

    fn apply(&mut self, _action: &CandidateAction) {}

    fn commit_unit(&self) -> CommitUnit {
        CommitUnit::Char
    }

    fn style(&self) -> StyleHint {
        StyleHint::Prefix
    }

    fn reset(&mut self) {
        self.state = PrefixState::Idle;
        self.revision = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{AsciiEngine, JaEngine, ZhEngine};
    use crate::lang::Mode;

    fn ctx() -> InputContext<'static> {
        InputContext::routing(Mode::Mixed, None, super::super::LaneView::default())
    }

    fn press(engine: &mut PrefixEngine, text: &str) -> Vec<PrefixEffect> {
        text.chars()
            .map(|ch| engine.press(&Key::char(ch, false), &ctx()))
            .collect()
    }

    #[test]
    fn interest_only_claims_backtick_or_an_active_run() {
        let mut engine = PrefixEngine::new();
        let context = ctx();
        assert_eq!(
            engine.interest(&Key::char('u', false), &context),
            Interest::No,
            "單獨的 u 仍是注音／英數鍵"
        );
        assert_eq!(
            engine.interest(&Key::char('`', false), &context),
            Interest::Claim
        );
        engine.press(&Key::char('`', false), &context);
        assert_eq!(
            engine.interest(&Key::char('u', false), &context),
            Interest::Claim
        );
        assert_eq!(
            engine.interest(&Key::new(KeyCode::Escape, None, false), &context),
            Interest::Claim
        );
    }

    #[test]
    fn language_engines_do_not_participate_during_a_prefix_run() {
        let mut lanes = super::super::LaneView {
            prefix_active: true,
            ascii_empty: false,
            ascii_len: 1,
            ja_available: true,
            ..super::super::LaneView::default()
        };
        lanes.zh_lane = true;
        let context = InputContext::routing(Mode::Mixed, Some(LanguageId::ZH), lanes);
        assert_eq!(
            ZhEngine::new().interest(&Key::char('j', false), &context),
            Interest::No
        );
        assert_eq!(
            JaEngine::new().interest(&Key::char('a', false), &context),
            Interest::No
        );
        assert_eq!(
            AsciiEngine::new().interest(&Key::char('a', false), &context),
            Interest::No
        );
    }

    #[test]
    fn unicode_scalars_decode_and_invalid_values_fail() {
        assert_eq!(CodeKind::Unicode.decode("4e00"), Some('一'));
        assert_eq!(CodeKind::Unicode.decode("4E2D"), Some('中'));
        assert_eq!(CodeKind::Unicode.decode("d800"), None, "代理對不可插入");
        assert_eq!(CodeKind::Unicode.decode("110000"), None, "超出 Unicode");
        assert_eq!(CodeKind::Unicode.decode(""), None);
    }

    #[test]
    fn big5_codes_decode_through_the_shared_table() {
        assert_eq!(CodeKind::Big5.decode("a440"), Some('一'));
        assert_eq!(CodeKind::Big5.decode("a4a4"), Some('中'));
        assert_eq!(CodeKind::Big5.decode("123"), None, "位數不足");
        assert_eq!(CodeKind::Big5.decode("zzzz"), None);
        assert_eq!(CodeKind::Big5.decode("8140"), None, "不是 Big5 合法碼位");
    }

    #[test]
    fn collect_space_completes_with_one_char() {
        let mut engine = PrefixEngine::new();
        let effects = press(&mut engine, "`u4e00 ");
        assert_eq!(effects[0], PrefixEffect::Start);
        assert_eq!(
            effects.last(),
            Some(&PrefixEffect::Replace {
                raw_len: 6,
                text: "一".to_string(),
            })
        );
        assert!(!engine.is_active());
    }

    #[test]
    fn escape_cancels_and_backspace_removes_digits() {
        let mut engine = PrefixEngine::new();
        press(&mut engine, "`b");
        assert_eq!(
            engine.press(&Key::char('a', false), &ctx()),
            PrefixEffect::Display("a".to_string())
        );
        assert_eq!(
            engine.press(&Key::new(KeyCode::Backspace, None, false), &ctx()),
            PrefixEffect::Remove { raw_len: 1 }
        );
        // 位數歸零：再退一次取消整個前導。
        assert_eq!(
            engine.press(&Key::new(KeyCode::Backspace, None, false), &ctx()),
            PrefixEffect::Remove { raw_len: 2 }
        );
        assert!(!engine.is_active());

        press(&mut engine, "`u4e00");
        assert_eq!(
            engine.press(&Key::new(KeyCode::Escape, None, false), &ctx()),
            PrefixEffect::Remove { raw_len: 6 }
        );
        assert!(!engine.is_active());
    }

    #[test]
    fn invalid_input_keeps_raw_until_space() {
        let mut engine = PrefixEngine::new();
        press(&mut engine, "`u");
        assert_eq!(
            engine.press(&Key::char('z', false), &ctx()),
            PrefixEffect::Display("z".to_string())
        );
        assert_eq!(
            engine.press(&Key::char('z', false), &ctx()),
            PrefixEffect::Display("z".to_string())
        );
        assert_eq!(
            engine.press(&Key::new(KeyCode::Space, None, false), &ctx()),
            PrefixEffect::Keep
        );
        assert!(!engine.is_active());
    }

    #[test]
    fn non_kind_key_releases_to_the_punctuation_tables() {
        let mut engine = PrefixEngine::new();
        press(&mut engine, "`");
        assert_eq!(
            engine.press(&Key::char(',', false), &ctx()),
            PrefixEffect::Release
        );
        assert!(!engine.is_active());
    }

    #[test]
    fn commit_unit_is_one_char_and_style_is_prefix() {
        let engine = PrefixEngine::new();
        assert_eq!(engine.commit_unit(), CommitUnit::Char);
        assert_eq!(engine.style(), StyleHint::Prefix);
        assert_eq!(engine.kind(), super::super::EngineKind::Control);
    }
}
