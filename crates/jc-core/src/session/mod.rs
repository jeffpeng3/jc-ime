//! 輸入工作階段：把注音／羅馬字輸入、分段、路由與轉換串成一次完整輸入。
//!
//! [`Session`] 是外殼唯一需要持有的狀態。每次 [`Session::press`] 都回傳完整
//! [`Snapshot`]，外殼不必自己組字；[`Session::commit`] 取走目前結果。
//!
//! 三條路徑共用同一個介面：
//! - 注音：`zh::input::ZhInput` 累積音節，`decode::Decoder` 在詞格上解碼。
//! - 日文：`ja::input::RomajiInput` 累積かな，`ja::decode::JaDecoder` 轉換。
//! - 英文與混合：混合模式由 `mixed::AsciiInput` 推斷英數按鍵，其餘交給
//!   `mixed::Router` 逐段路由。
//!
//! 沒有詞典時注音與日文仍可運作：詞格只有未知詞邊，輸出等於讀音本身，
//! 方便在沒有資料的環境跑協定與薄層行為測試。

use std::collections::HashMap;
use std::sync::Arc;

mod core;
mod segments;

pub use segments::tone_mark_keys;

use crate::candidate::{Candidate, CandidateSource};
use crate::decode::{Decoder, Hypothesis, Locks};
use crate::emoji::{self, Stat};
use crate::ja::decode::{JaClause, JaContext, JaDecoder, JaResult};
use crate::ja::input::RomajiInput;
use crate::ja::live::{self, Controller as LiveController, Plan as LivePlan};
use crate::ja::personal as ja_personal;
use crate::key::{Key, KeyCode};
use crate::lang::Mode;
use crate::lattice::{InMemoryLexicon, Lattice};
use crate::mixed::{space_text, AsciiInput, AsciiPiece, Spacing};
use crate::output::{utf16_len, HansConverter, SharedPunctuator, Snapshot};
use crate::personal::PersonalStore;
use crate::punct::{self, Edit};
use crate::reading::Unit;
use crate::score::ScoreContext;
use crate::segment::{Script, Segment};
use crate::symbols;
use crate::zh::han::is_han;
use crate::zh::input::ZhInput;
use crate::zh::syllable::Syllable;

/// 這個 crate 的語意版本，供協定握手與診斷使用。
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 預設候選數。
pub const DEFAULT_MAX_CANDIDATES: usize = 5;

/// 候選視窗一頁幾項。
pub const PAGE_SIZE: usize = 10;

/// 聲調選填最多展開幾種組合；超過就維持原讀音。
pub(super) const TONELESS_COMBINATIONS: usize = 256;

/// 模糊音最多展開幾種讀音組合。
pub(super) const FUZZY_COMBINATIONS: usize = 16;

/// 預設開啟的模糊音配對：ㄗ／ㄓ、ㄘ／ㄔ、ㄙ／ㄕ、ㄣ／ㄥ。
pub const DEFAULT_FUZZY_PAIRS: [(char, char); 4] =
    [('ㄗ', 'ㄓ'), ('ㄘ', 'ㄔ'), ('ㄙ', 'ㄕ'), ('ㄣ', 'ㄥ')];

/// 累積幾次學習事件後清理一次個人資料。
pub(super) const LEARN_PRUNE_INTERVAL: usize = 64;

/// 左文保留的字元上限，對齊前身 `MaxLeftContextUnits`。
pub(super) const MAX_LEFT_CONTEXT_UNITS: usize = 128;

/// 建立 [`Session`] 所需的設定。
///
/// 資料來源以值持有：注音詞典是小型記憶體詞典，日文解碼器用 [`Arc`] 共用。
/// 沒有給詞典時，該語言仍可組字，只是轉換結果等於讀音。
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// 主要輸入模式。
    pub mode: Mode,
    /// 一次轉換保留幾個候選。
    pub max_candidates: usize,
    /// 注音詞典；`None` 時注音只做原樣輸出。
    pub zh_lexicon: Option<InMemoryLexicon>,
    /// 注音成本設定；`None` 用 [`ScoreContext::default`]。
    pub zh_ctx: Option<ScoreContext>,
    /// 注音 bigram 語言模型（`lm.bin`）；`None` 或沒有 bigram 時只用 unigram。
    pub zh_lm: Option<Arc<crate::score::ZhLm>>,
    /// 日文解碼器；`None` 時日文只做原樣輸出。
    pub ja: Option<Arc<JaDecoder>>,
    /// 是否呼叫日文解碼器上的重排器；預設關閉，實測 modern-500 全量重排
    /// 由 86.0% 退到 82.0%，因此不再預設啟用。解碼器上的詞群門檻
    /// （[`crate::ja::decode::DEFAULT_WORD_RERANK_GATE`]）預設只送統計
    /// 不確定的詞群，但這裡仍保守關閉。
    pub ja_rerank: bool,
    /// 日文逐字変換設定；預設關閉，關閉時日文走一般変換流程。
    pub ja_live: live::Config,
    /// 聲調選填：沒有聲調的音節展開成五個聲調再解碼。
    pub toneless: bool,
    /// Ctrl 標點：`Ctrl+,` 輸出全形逗號。
    pub ctrl_punct: bool,
    /// 模糊音配對；每一項是兩個可互換的注音符號。預設四組全開。
    pub fuzzy_pairs: Vec<(char, char)>,
    /// 神經重排的固定替身設定。
    pub rerank: Option<RerankOptions>,
    /// 提交前補標點：在最後一個漢字前插入這個字元。
    pub punct_mark: Option<char>,
    /// 英文自動補字；混合模式下用 Tab 接受。
    pub english_autocomplete: bool,
    /// 送出時在漢字與 ASCII 英文字母之間補半形空格。
    pub cjk_latin_space: bool,
    /// 繁體轉簡體的轉換器；`None` 時原樣送出。
    pub simplified: Option<Arc<crate::zhconv::Converter>>,
    /// 送出時呼叫的標點模型；`None` 時完全不補標點。
    pub punct: Option<SharedPunctuator>,
    /// 個人化學習資料；預設為空。搭配 [`SessionConfig::learning`] 生效。
    pub personal: PersonalStore,
    /// 個人化學習是否啟用；預設啟用，關閉時只讀不寫。
    pub learning: bool,
}

/// 固定替身重排設定。
///
/// 只啟用重排路徑，不是模型本身：候選字包含 [`RerankOptions::prefer`] 任一項
/// 時，成本扣掉 `weight * prefer_score` 換算後的毫成本。
#[derive(Debug, Clone, PartialEq)]
pub struct RerankOptions {
    /// 重排權重。
    pub weight: f64,
    /// 偏好的候選字串。
    pub prefer: Vec<String>,
    /// 偏好候選的原始分數；0 代表 10。
    pub prefer_score: f64,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            mode: Mode::Zh,
            max_candidates: DEFAULT_MAX_CANDIDATES,
            zh_lexicon: None,
            zh_ctx: None,
            zh_lm: None,
            ja: None,
            ja_rerank: false,
            ja_live: live::Config::default(),
            toneless: false,
            ctrl_punct: false,
            fuzzy_pairs: DEFAULT_FUZZY_PAIRS.to_vec(),
            rerank: None,
            punct_mark: None,
            english_autocomplete: false,
            cjk_latin_space: false,
            simplified: None,
            punct: None,
            personal: PersonalStore::new(),
            learning: true,
        }
    }
}

impl SessionConfig {
    /// 用指定模式建立預設設定。
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            ..Self::default()
        }
    }
}

/// 候選選單的種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MenuKind {
    /// 反引號開的符號選單。
    Symbols,
    /// 等號開的表情選單。
    Emoji,
}

/// 開啟中的候選選單。
#[derive(Debug, Clone, Copy)]
pub(super) struct Menu {
    /// 選單種類。
    pub(super) kind: MenuKind,
    /// 目前分頁序號。
    pub(super) tab: usize,
}

/// 一次輸入的工作階段。
///
/// 只保留組字所需的狀態；送出後立即清空，因此呼叫端每次都用快照重畫。
pub struct Session {
    pub(super) config: SessionConfig,
    pub(super) zh: ZhInput,
    pub(super) romaji: RomajiInput,
    pub(super) lexicon: InMemoryLexicon,
    /// 不含自訂詞的原始詞典；自訂詞變動時由它重建 [`Session::lexicon`]。
    pub(super) base_lexicon: InMemoryLexicon,
    /// 個人化學習資料。
    pub(super) personal: PersonalStore,
    pub(super) ctx: ScoreContext,
    pub(super) zh_lm: Option<Arc<crate::score::ZhLm>>,
    /// 注音以外的字面按鍵（標點、英文、直接輸入的注音符號）。
    pub(super) literal: String,
    /// 字面按鍵是在幾個已完成音節之後打的；用來插回轉換結果中間。
    pub(super) literal_anchor: usize,
    /// 字面按鍵在目前轉換結果中的位元組位置。
    pub(super) literal_offset: usize,
    /// 英文與混合模式累積的原始輸入。
    pub(super) raw: String,
    pub(super) candidates: Vec<Candidate>,
    pub(super) highlighted: usize,
    /// 候選視窗是否由 Tab／方向鍵開啟；數字鍵只在開啟時選字。
    pub(super) candidate_window: bool,
    pub(super) segments: Vec<Segment>,
    /// 使用者指定的片段鎖定：音節起點 → 鎖定的詞。
    ///
    /// 只有 [`Session::lock_segment`] 會寫入；一般選字不走這條路。
    pub(super) locks: Locks,
    /// 手動編輯過的片段副本；下一次按鍵後由組字狀態重新推導。
    pub(super) segment_manual: Option<Vec<Segment>>,
    /// 剛按下的孤立聲調鍵；緊接的空白會把它換成聲調符號。
    pub(super) pending_tone: Option<segments::PendingTone>,
    /// 已轉換的部分。
    pub(super) base: String,
    /// 已送出的文字尾巴；下一次轉換當左文用（前身 `lang.Context.Left`）。
    pub(super) committed: String,
    /// 尚未轉換的尾巴（未完成的注音或羅馬字）。
    pub(super) pending: String,
    /// `Enter` 先放進這裡，下一次 [`Session::commit`] 取走。
    pub(super) pending_commit: Option<String>,
    /// Tab 接受的補全結果；有值時取代組字結果。
    pub(super) accepted_completion: Option<String>,
    /// 尾端補全建議。
    pub(super) suggestion: Option<String>,
    /// 待選路徑的快照；提交時用來依實際片段學習。
    pub(super) last_hypotheses: Vec<Hypothesis>,
    /// 日文待選結果的快照；提交時用來依文節學習。
    pub(super) last_ja_results: Vec<JaResult>,
    /// 混合模式的英數按鍵推斷。
    pub(super) ascii: AsciiInput,
    /// 目前開啟的候選選單。
    pub(super) menu: Option<Menu>,
    /// 表情分頁的最後選擇；下次開選單時沿用。
    pub(super) palette_tab: usize,
    /// 符號選單「最近」分頁的順序。
    pub(super) symbol_recent: Vec<String>,
    /// 符號「最近」清單的常用分數。
    pub(super) symbol_stats: HashMap<String, Stat>,
    /// 表情選單「最近」分頁的順序。
    pub(super) emoji_recent: Vec<String>,
    /// 表情「最近」清單的常用分數。
    pub(super) emoji_stats: HashMap<String, Stat>,
    /// 日文逐字変換控制器；關閉時為 `None`，日文走一般変換。
    pub(super) live: Option<LiveController>,
    /// 最近一次逐字変換的決定，供 debounce 到期後重畫。
    pub(super) live_plan: Option<LivePlan>,
    /// 上次清理個人資料後累積的學習事件數。
    pub(super) learn_events: usize,
}

impl Default for Session {
    fn default() -> Self {
        Self::new(SessionConfig::default())
    }
}

impl Session {
    /// 依設定建立工作階段。
    pub fn new(mut config: SessionConfig) -> Self {
        // 關閉重排時複製一份解碼器再停用；`JaDecoder` 內部都是 `Arc`，
        // 複製成本與共享詞典相同。
        if !config.ja_rerank {
            if let Some(decoder) = &config.ja {
                config.ja = Some(Arc::new(
                    decoder.as_ref().clone().with_rerank_enabled(false),
                ));
            }
        }
        let ctx = config.zh_ctx.clone().unwrap_or_default();
        let base_lexicon = config.zh_lexicon.clone().unwrap_or_default();
        let personal = config.personal.clone();
        let lexicon = personal.augment_lexicon(&base_lexicon);
        let zh_lm = config.zh_lm.clone();
        // 逐字変換控制器與日文解碼器共用同一份詞典；關閉時完全不建立。
        let live = if config.ja_live.enabled {
            config.ja.as_ref().map(|decoder| {
                LiveController::new(decoder.as_ref().clone(), config.ja_live.clone())
            })
        } else {
            None
        };
        Self {
            config,
            zh: ZhInput::standard(),
            romaji: RomajiInput::new(),
            lexicon,
            base_lexicon,
            personal,
            ctx,
            zh_lm,
            literal: String::new(),
            literal_anchor: 0,
            literal_offset: 0,
            raw: String::new(),
            candidates: Vec::new(),
            highlighted: 0,
            candidate_window: false,
            segments: Vec::new(),
            locks: Locks::new(),
            segment_manual: None,
            pending_tone: None,
            base: String::new(),
            committed: String::new(),
            pending: String::new(),
            pending_commit: None,
            accepted_completion: None,
            suggestion: None,
            last_hypotheses: Vec::new(),
            last_ja_results: Vec::new(),
            ascii: AsciiInput::new(),
            menu: None,
            palette_tab: 0,
            symbol_recent: Vec::new(),
            symbol_stats: HashMap::new(),
            emoji_recent: Vec::new(),
            emoji_stats: HashMap::new(),
            live,
            live_plan: None,
            learn_events: 0,
        }
    }

    /// 目前模式。
    pub fn mode(&self) -> Mode {
        self.config.mode
    }

    /// 目前的個人化學習資料；宿主用它存檔。
    ///
    /// [`Session::new`] 會複製 [`SessionConfig::personal`]，因此學習結果只在
    /// 這份工作階段內；要持久化時由宿主在適當時間取出並寫檔。
    pub fn personal(&self) -> &PersonalStore {
        &self.personal
    }

    /// 目前高亮的候選索引。
    pub fn highlighted(&self) -> usize {
        self.highlighted
    }

    /// 目前候選清單。
    pub fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }

    /// 目前組字字串。
    pub fn preedit(&self) -> String {
        self.base_with_highlight()
    }

    /// 處理一個按鍵並回傳完整快照。
    ///
    /// `Enter` 會把組字結果放進待提交區，下一次 [`Session::commit`] 取走；
    /// `Escape` 清空組字；左右方向鍵與 Tab 在候選間移動。
    pub fn press(&mut self, key: Key) -> Snapshot {
        self.expire_segment_view();
        if key.ctrl {
            self.pending_tone = None;
            self.press_ctrl(key);
        } else if !self.menu_key(&key) {
            if !self.segments_press(&key) {
                match key.code {
                    KeyCode::Enter => self.finish_composition(),
                    KeyCode::Escape => self.clear_composition(),
                    KeyCode::Delete if key.shift && self.candidate_window => {
                        self.forget_candidate();
                    }
                    KeyCode::Backspace if key.shift && self.candidate_window => {
                        self.forget_candidate();
                    }
                    KeyCode::Delete => self.clear_composition(),
                    KeyCode::Backspace => self.backspace(),
                    KeyCode::Space => self.space(),
                    KeyCode::Tab => {
                        let accepted = self.config.mode == Mode::Mixed
                            && self.config.english_autocomplete
                            && self.ascii.accept_suggestion();
                        if !accepted && !self.accept_suggestion() {
                            self.move_highlight(1);
                        }
                    }
                    KeyCode::ArrowRight => self.navigate(1),
                    KeyCode::ArrowLeft => self.navigate(-1),
                    KeyCode::Char(ch) => self.press_char(ch),
                }
            }
        } else {
            self.pending_tone = None;
        }
        self.rebuild();
        self.snapshot()
    }

    /// 切換到指定模式；清空目前組字。
    ///
    /// 英文模式切換（`request: {"type":"toggle"}`）由此進入。
    pub fn set_mode(&mut self, mode: Mode) -> Snapshot {
        self.clear_composition();
        self.config.mode = mode;
        self.rebuild();
        self.snapshot()
    }

    /// 在中文與英文模式之間切換。
    pub fn toggle_english(&mut self) -> Snapshot {
        let mode = if self.config.mode == Mode::En {
            Mode::Zh
        } else {
            Mode::En
        };
        self.set_mode(mode)
    }

    /// 開啟符號選單；有最近使用的符號時從「最近」開，否則從第一個分類開。
    pub fn open_symbol_menu(&mut self) -> Snapshot {
        // 「最近」永遠是第一個分頁；沒有紀錄時第一個分頁就是「標點」。
        self.open_menu(MenuKind::Symbols, 0)
    }

    /// 依名稱開啟符號分頁；名稱不存在時回到第一個分頁。
    pub fn open_symbol_tab(&mut self, name: &str) -> Snapshot {
        let names = self.menu_tab_names(MenuKind::Symbols);
        let tab = names.iter().position(|item| *item == name).unwrap_or(0);
        self.open_menu(MenuKind::Symbols, tab)
    }

    /// 開啟表情選單，沿用上次真的選了東西的分頁。
    pub fn open_palette(&mut self) -> Snapshot {
        self.open_menu(MenuKind::Emoji, self.palette_tab)
    }

    /// 依種類開啟表情分頁；`"kaomoji"` 是顏文字，其他值開表情。
    pub fn open_emoji(&mut self, kind: &str) -> Snapshot {
        let tab = usize::from(kind == "kaomoji");
        self.open_menu(MenuKind::Emoji, tab)
    }

    /// 符號選單「最近」分頁保存的順序。
    pub fn symbol_recent(&self) -> &[String] {
        &self.symbol_recent
    }

    /// 表情選單「最近」分頁保存的順序。
    pub fn emoji_recent(&self) -> &[String] {
        &self.emoji_recent
    }

    /// 目前選單分頁的名稱；沒有開選單時回傳 `None`。
    pub fn menu_tab(&self) -> Option<&'static str> {
        let menu = self.menu?;
        self.menu_tab_names(menu.kind).get(menu.tab).copied()
    }

    /// 翻到候選的第 `page` 頁；頁面大小固定 10。
    ///
    /// 目前候選清單保留全部候選，翻頁只是把高亮移到該頁第一項。
    pub fn page(&mut self, page: usize) -> Snapshot {
        let index = page.saturating_mul(PAGE_SIZE);
        if index < self.candidates.len() {
            self.highlighted = index;
            self.candidate_window = true;
        }
        self.rebuild();
        self.snapshot()
    }

    /// 目前高亮的候選字串；沒有候選時回傳 `None`。
    pub fn highlighted_text(&self) -> Option<&str> {
        self.candidates
            .get(self.highlighted)
            .map(|candidate| candidate.surface.as_str())
    }

    /// 直接指定高亮的候選；超出範圍時回到 0。
    pub fn select(&mut self, index: usize) -> Snapshot {
        if index < self.candidates.len() {
            self.highlighted = index;
            self.candidate_window = true;
        } else {
            self.highlighted = 0;
        }
        self.rebuild();
        self.snapshot()
    }

    /// 送出目前的組字結果並清空狀態。
    ///
    /// 有 `Enter` 放進來的結果時直接回傳它；否則送出目前高亮的候選或組字
    /// 字串。沒有組字內容時回傳空字串。
    pub fn commit(&mut self) -> String {
        if let Some(text) = self.pending_commit.take() {
            let text = self.model_punctuated(text);
            self.clear_composition();
            self.remember_committed(&text);
            return text;
        }
        self.remember_selection();
        if self.config.mode == Mode::Mixed {
            self.ascii.flush();
        }
        let text = self.model_punctuated(self.finalize(self.current_text()));
        self.clear_composition();
        self.remember_committed(&text);
        text
    }

    /// 目前已送出的左文。
    pub fn committed(&self) -> &str {
        &self.committed
    }

    /// 清空組字狀態，包含尚未取走的提交結果。
    ///
    /// 已送出的左文保留；那是文件內容，不屬於組字狀態。
    pub fn reset(&mut self) {
        self.pending_commit = None;
        self.clear_composition();
    }

    /// 日文逐字変換是否開啟。
    pub fn ja_live_enabled(&self) -> bool {
        self.live.as_ref().is_some_and(LiveController::enabled)
    }

    /// debounce 是否還在等待；外殼據此安排計時器。
    pub fn ja_live_pending(&self) -> bool {
        self.live.as_ref().is_some_and(LiveController::pending)
    }

    /// debounce 到期時間；沒有待處理轉換時為 `None`。
    pub fn ja_live_deadline(&self) -> Option<std::time::Instant> {
        self.live_plan.as_ref().and_then(|plan| plan.deadline)
    }

    /// 檢查日文逐字変換的 debounce 期限；到期就轉換並更新組字狀態。
    ///
    /// 期限未到或功能關閉時回傳目前快照，不改變狀態。
    pub fn tick_live(&mut self) -> Snapshot {
        let plan = match &mut self.live {
            Some(live) => {
                let (plan, converted) = live.tick();
                converted.then_some(plan)
            }
            None => None,
        };
        if let Some(plan) = plan {
            let units = self.romaji.units().to_vec();
            let reading: String = units.iter().map(Unit::as_str).collect();
            self.apply_live_plan(&plan, &units, &reading);
        }
        self.snapshot()
    }

    /// 目前的完整快照。
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            preedit: self.base_with_highlight(),
            cursor_utf16: self.cursor_utf16(),
            segments: self.segments(),
            candidates: self.candidates.clone(),
            highlighted: self.highlighted,
        }
    }

    /// 游標位置，以 UTF-16 code unit 計算。
    pub fn cursor_utf16(&self) -> usize {
        utf16_len(&self.preedit_string())
    }

    /// 反悔最近一次分語言學習；回傳是否有可反悔的紀錄。
    ///
    /// 只還原共用 store 記下的語言學習（日文與後續接上的語言），不影響注音的
    /// 階層式計數。重建中組字內容後，排序會回到學習前的狀態。
    pub fn undo_learning(&mut self) -> bool {
        let undone = self.personal.undo_language_note();
        if undone {
            self.rebuild();
        }
        undone
    }

    /// 撤銷與某個詞有關的學習；供 `forget` 協定請求使用。
    pub fn forget_word(&mut self, word: &str) -> bool {
        let changed = self.personal.forget(word);
        if changed {
            self.highlighted = 0;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::LexEntry;

    fn lexicon() -> InMemoryLexicon {
        let mut lex = InMemoryLexicon::new();
        for (surface, reading) in [
            ("我", "ㄨㄛˇ"),
            ("想", "ㄒㄧㄤˇ"),
            ("吃", "ㄔ"),
            ("風", "ㄈㄥ"),
            ("景", "ㄐㄧㄥˇ"),
            ("風景", "ㄈㄥ ㄐㄧㄥˇ"),
            ("麻婆豆腐", "ㄇㄚˊ ㄆㄛˊ ㄉㄡˋ ㄈㄨˇ"),
            ("城市", "ㄔㄥˊ ㄕˋ"),
            ("程式", "ㄔㄥˊ ㄕˋ"),
            ("你", "ㄋㄧˇ"),
            ("宗", "ㄗㄨㄥ"),
            ("中", "ㄓㄨㄥ"),
            ("麻", "ㄇㄚˊ"),
            ("煩", "ㄈㄢˊ"),
            ("窩", "ㄨㄛ"),
            ("龍", "ㄌㄨㄥˊ"),
            ("風", "ㄈㄥ"),
            ("學", "ㄒㄩㄝˊ"),
            ("國", "ㄍㄨㄛˊ"),
        ] {
            lex.push(LexEntry::new(surface, reading, 0, 0));
        }
        lex
    }

    fn session() -> Session {
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lexicon());
        Session::new(config)
    }

    fn type_syllable(session: &mut Session, reading: &str) {
        let keyboard = crate::zh::keyboard::Keyboard::standard();
        for syllable in crate::zh::syllable::parse_reading(reading) {
            let keys = keyboard.keys(&syllable).expect("音節應該有對應按鍵");
            for ch in keys.chars() {
                session.press(Key::char(ch, false));
            }
        }
    }

    #[test]
    fn test_commit_exactly_once() {
        let mut s = session();
        type_syllable(&mut s, "ㄈㄥ");
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        let snap = s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "風景");
        assert_eq!(snap.preedit, "");
        assert!(snap.candidates.is_empty());
        // 已無組字時再按 Enter 不得重複提交。
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "");
    }

    #[test]
    fn test_commit_accumulates_left_context() {
        let mut s = session();
        type_syllable(&mut s, "ㄈㄥ");
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        assert_eq!(s.commit(), "風景");
        assert_eq!(s.committed(), "風景");
        type_syllable(&mut s, "ㄨㄛˇ");
        assert_eq!(s.commit(), "我");
        assert_eq!(s.committed(), "風景我");
        // Enter 先放進 pending_commit 的路徑也要計入。
        type_syllable(&mut s, "ㄔ");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "吃");
        assert_eq!(s.committed(), "風景我吃");
    }

    #[test]
    fn test_committed_left_context_is_capped() {
        let mut s = session();
        for _ in 0..80 {
            type_syllable(&mut s, "ㄈㄥ");
            type_syllable(&mut s, "ㄐㄧㄥˇ");
            s.commit();
        }
        assert_eq!(s.committed().chars().count(), MAX_LEFT_CONTEXT_UNITS);
        assert!(s.committed().ends_with("風景"), "{}", s.committed());
    }

    #[test]
    fn test_session_segments_follow_zhuyin_path_and_cursor() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_syllable(&mut s, "ㄒㄧㄤˇ");
        type_syllable(&mut s, "ㄔ");
        let snap = s.snapshot();
        assert_eq!(snap.preedit, "我想吃");
        assert_eq!(snap.cursor_utf16, 3);
        assert_eq!(snap.segments.len(), 3);
        let words: Vec<&str> = ["我", "想", "吃"].to_vec();
        for (segment, word) in snap.segments.iter().zip(words) {
            assert_eq!(&snap.preedit[segment.start..segment.end], word);
            assert_eq!(segment.mode, Mode::Zh);
        }
    }

    #[test]
    fn test_long_word_stays_merged() {
        let mut s = session();
        type_syllable(&mut s, "ㄈㄥ");
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        let snap = s.snapshot();
        assert_eq!(snap.preedit, "風景");
        assert_eq!(snap.segments.len(), 1);
    }

    #[test]
    fn test_backspace_removes_last_syllable() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_syllable(&mut s, "ㄒㄧㄤˇ");
        s.press(Key::new(KeyCode::Backspace, None, false));
        assert_eq!(s.snapshot().preedit, "我");
    }

    #[test]
    fn test_escape_clears_composition() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        let snap = s.press(Key::new(KeyCode::Escape, None, false));
        assert_eq!(snap.preedit, "");
        assert_eq!(s.commit(), "");
    }

    #[test]
    fn test_candidates_are_ranked_and_tab_moves_highlight() {
        let mut s = session();
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        let snap = s.snapshot();
        assert!(!snap.candidates.is_empty());
        let first = snap.candidates[0].surface.clone();
        assert_eq!(snap.preedit, first);
        s.press(Key::new(KeyCode::Tab, None, false));
        assert_eq!(s.highlighted(), 1 % snap.candidates.len().max(1));
    }

    #[test]
    fn test_digit_selects_candidate() {
        let mut s = session();
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        let candidates = s.candidates().to_vec();
        s.press(Key::new(KeyCode::ArrowRight, None, false));
        s.press(Key::char('2', false));
        assert_eq!(s.commit(), candidates[1].surface);
    }

    #[test]
    fn test_without_lexicon_commits_reading() {
        let mut s = Session::new(SessionConfig::new(Mode::Zh));
        let keyboard = crate::zh::keyboard::Keyboard::standard();
        for ch in keyboard
            .keys(&crate::zh::syllable::Syllable::parse("ㄇㄚ").unwrap())
            .unwrap()
            .chars()
        {
            s.press(Key::char(ch, false));
        }
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "ㄇㄚ");
    }

    #[test]
    fn test_ctrl_comma_inserts_fullwidth_comma() {
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lexicon());
        config.ctrl_punct = true;
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄨㄛˇ");
        s.press(Key::new(KeyCode::Char(','), Some(','), false).with_ctrl());
        type_syllable(&mut s, "ㄒㄧㄤˇ");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "我，想");
    }

    #[test]
    fn test_punctuator_inserts_mark_before_last_han() {
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lexicon());
        config.punct_mark = Some('，');
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄈㄥ");
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "風，景");
    }

    /// 測試用標點模型：只在注音模式補句尾句點。
    #[derive(Debug)]
    struct SentencePeriod;

    impl crate::output::ModelPunctuator for SentencePeriod {
        fn punctuate(&self, text: &str, mode: Mode) -> String {
            if mode == Mode::Zh && !text.ends_with('。') {
                format!("{text}。")
            } else {
                text.to_string()
            }
        }
    }

    #[test]
    fn test_commit_calls_model_punctuator_on_enter() {
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lexicon());
        config.punct = Some(std::sync::Arc::new(SentencePeriod));
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄈㄥ");
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "風景。");
    }

    #[test]
    fn test_commit_punctuates_pending_result_once() {
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lexicon());
        config.punct = Some(std::sync::Arc::new(SentencePeriod));
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄈㄥ");
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "風景。");
        // 已清空，下一次提交不會再補一次。
        assert_eq!(s.commit(), "");
    }

    #[test]
    fn test_commit_without_model_keeps_text_verbatim() {
        let mut s = session();
        type_syllable(&mut s, "ㄈㄥ");
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "風景");
    }

    #[test]
    fn test_rerank_prefers_configured_candidate() {
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lexicon());
        config.rerank = Some(RerankOptions {
            weight: 1.0,
            prefer: vec!["城市".to_string()],
            prefer_score: 10.0,
        });
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄔㄥˊ");
        type_syllable(&mut s, "ㄕˋ");
        assert_eq!(s.candidates()[0].surface, "城市");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "城市");
    }

    #[test]
    fn test_toneless_readings_expand_to_cheapest_tones() {
        // 讓「我」與「窩」成本不同，才有一條唯一的最便宜組合。
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        lex.push(LexEntry::new("窩", "ㄨㄛ", 10, 0));
        lex.push(LexEntry::new("想", "ㄒㄧㄤˇ", 0, 0));
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lex);
        config.toneless = true;
        let mut s = Session::new(config);
        for keys in ["ji", "vu;"] {
            for ch in keys.chars() {
                s.press(Key::char(ch, false));
            }
            s.press(Key::new(KeyCode::Space, None, false));
        }
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "我想");
    }

    #[test]
    fn test_fuzzy_candidates_appear_before_literal() {
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lexicon());
        config.fuzzy_pairs = vec![('ㄗ', 'ㄓ')];
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄗㄨㄥ");
        let surfaces: Vec<&str> = s
            .candidates()
            .iter()
            .map(|candidate| candidate.surface.as_str())
            .collect();
        assert_eq!(surfaces, ["宗", "中", "ㄗㄨㄥ"]);
    }

    #[test]
    fn test_learning_enabled_by_default_changes_first_candidate() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        lex.push(LexEntry::new("窩", "ㄨㄛˇ", 5, 0));
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lex);
        let mut s = Session::new(config);
        assert!(s.config.learning, "學習預設必須開啟");
        type_syllable(&mut s, "ㄨㄛˇ");
        assert_eq!(s.candidates()[0].surface, "我");
        // 開候選窗，移到第二候選，用 Enter 送出手選。
        s.press(Key::new(KeyCode::ArrowRight, None, false));
        s.press(Key::new(KeyCode::ArrowRight, None, false));
        assert_eq!(s.highlighted_text(), Some("窩"));
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "窩");
        // 同一讀音再打一次：手選過的字排第一。
        type_syllable(&mut s, "ㄨㄛˇ");
        assert_eq!(s.candidates()[0].surface, "窩");
    }

    #[test]
    fn test_learning_disabled_keeps_statistical_order() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        lex.push(LexEntry::new("窩", "ㄨㄛˇ", 5, 0));
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lex);
        config.learning = false;
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄨㄛˇ");
        s.press(Key::new(KeyCode::ArrowRight, None, false));
        s.press(Key::new(KeyCode::ArrowRight, None, false));
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "窩");
        type_syllable(&mut s, "ㄨㄛˇ");
        assert_eq!(s.candidates()[0].surface, "我");
    }

    #[test]
    fn test_default_fuzzy_pairs_cover_four_groups() {
        assert_eq!(SessionConfig::default().fuzzy_pairs, DEFAULT_FUZZY_PAIRS);
        assert!(
            !SessionConfig::default().ja_rerank,
            "日文 reranker 預設關閉"
        );
    }

    #[test]
    fn test_page_moves_highlight_to_page_start() {
        let mut lexicon = InMemoryLexicon::new();
        for (index, surface) in ["甲", "乙", "丙", "丁", "戊", "己", "庚", "辛", "壬", "癸"]
            .iter()
            .chain(["子", "丑"].iter())
            .enumerate()
        {
            lexicon.push(LexEntry::new(*surface, "ㄐㄧ", index as i32, 0));
        }
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lexicon);
        config.max_candidates = 20;
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄐㄧ");
        s.page(1);
        assert_eq!(s.highlighted(), PAGE_SIZE, "翻頁把高亮移到頁首");
        assert_eq!(
            s.highlighted_text(),
            s.candidates().get(PAGE_SIZE).map(|c| c.surface.as_str())
        );
    }

    fn mixed_session() -> Session {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(lexicon());
        Session::new(config)
    }

    fn type_keys(session: &mut Session, keys: &str) {
        for ch in keys.chars() {
            let key = if ch == ' ' {
                Key::new(KeyCode::Space, None, false)
            } else {
                Key::char(ch, false)
            };
            session.press(key);
        }
    }

    /// 對照 `TestAutomaticMixedChineseEnglishChinese`：英數鍵在注音組字中
    /// 由聲調鍵或空白決定是中文還是英文。
    #[test]
    fn test_automatic_mixed_chinese_english_chinese() {
        for (keys, want) in [
            ("ji3hellot ", "我hello吃"),
            ("helloji3", "hello我"),
            ("t ", "吃"),
            ("a86z06", "麻煩"),
            ("ji3hello worldji3", "我hello world我"),
        ] {
            let mut s = mixed_session();
            type_keys(&mut s, keys);
            let snap = s.press(Key::new(KeyCode::Enter, None, false));
            assert_eq!(s.commit(), want, "{keys:?}");
            assert_eq!(snap.preedit, "");
        }
    }

    /// 對照 `TestMixedCompleteEnglishWordKeepsSpace`：完整英文詞接空白保留
    /// 空白，組得成注音的字尾讓給中文。
    #[test]
    fn test_mixed_complete_english_word_keeps_space() {
        for (keys, want) in [
            ("hello ", "hello "),
            ("helloji ", "hello窩"),
            ("ji3hello ji3", "我hello 我"),
        ] {
            let mut s = mixed_session();
            type_keys(&mut s, keys);
            assert_eq!(s.snapshot().preedit, want, "{keys:?}");
            s.press(Key::new(KeyCode::Enter, None, false));
            assert_eq!(s.commit(), want, "{keys:?}");
        }
    }

    #[test]
    fn test_mixed_english_suggestion_and_completion() {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(lexicon());
        config.english_autocomplete = true;
        let mut s = Session::new(config);
        type_keys(&mut s, "ji3hellp");
        assert_eq!(s.snapshot().preedit, "我hellp");
        s.press(Key::new(KeyCode::Tab, None, false));
        assert_eq!(s.snapshot().preedit, "我hello");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "我hello");
    }

    /// 對照 `TestCJKLatinSpaceInMixedInput`：組字區照打的顯示，送出時才補空格。
    #[test]
    fn test_cjk_latin_space_in_mixed_input() {
        for (keys, off, on) in [
            ("ji3hellot ", "我hello吃", "我 hello 吃"),
            ("ji3hello su3", "我hello 你", "我 hello 你"),
        ] {
            for space in [false, true] {
                let mut config = SessionConfig::new(Mode::Mixed);
                config.zh_lexicon = Some(lexicon());
                config.cjk_latin_space = space;
                let mut s = Session::new(config);
                type_keys(&mut s, keys);
                assert_eq!(s.snapshot().preedit, off, "{keys:?}");
                s.press(Key::new(KeyCode::Enter, None, false));
                assert_eq!(s.commit(), if space { on } else { off }, "{keys:?}");
            }
        }
    }

    #[test]
    fn test_simplified_output_converts_commit_only() {
        let path = jc_testdata::require_data!("opencc");
        let converter = crate::zhconv::tw2s_from_dir(&path).expect("讀取 OpenCC 字典");
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lexicon());
        config.simplified = Some(Arc::new(converter));
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄌㄨㄥˊ");
        assert_eq!(s.snapshot().preedit, "龍");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "龙");

        let mut s = session();
        type_syllable(&mut s, "ㄈㄥ");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "風");
    }

    #[test]
    fn test_reset_clears_pending_commit() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        s.press(Key::new(KeyCode::Enter, None, false));
        s.reset();
        assert_eq!(s.commit(), "");
        assert_eq!(s.snapshot().preedit, "");
    }

    /// 對照 `TestSymbolMenuTabs`：反引號從標點開，Tab／Shift＋Tab 換分頁，
    /// 選了就插入目前組字尾端。
    #[test]
    fn test_symbol_menu_tabs() {
        let mut s = session();
        let snap = s.press(Key::char('`', false));
        assert_eq!(s.menu_tab(), Some("標點"));
        assert_eq!(snap.candidates[0].surface, "，");
        assert!(snap.candidates.len() > PAGE_SIZE);

        let mut seen = Vec::new();
        for _ in 0..symbols::TABS.len() {
            seen.push(s.menu_tab().unwrap_or(""));
            s.press(Key::new(KeyCode::Tab, None, false));
        }
        assert_eq!(
            seen,
            ["標點", "括號引號", "數學", "箭頭", "圖形", "編號", "注音"]
        );
        assert_eq!(s.menu_tab(), Some("標點"), "Tab 頭尾相接");

        s.press(Key::new(KeyCode::Tab, None, true));
        assert_eq!(s.menu_tab(), Some("注音"), "Shift＋Tab 往前");

        type_syllable(&mut s, "ㄨㄛˇ");
        s.press(Key::char('`', false));
        assert_eq!(s.snapshot().preedit, "我", "開選單不動組字");
        s.press(Key::char('7', false)); // 標點分頁第 7 項是「；」
        assert_eq!(s.commit(), "我；");
        assert_eq!(s.symbol_recent(), ["；"]);
    }

    /// 對照 `TestSymbolMenuRecentTab`：有紀錄時從「最近」開，只放一頁。
    #[test]
    fn test_symbol_menu_recent_tab() {
        let mut s = session();
        s.press(Key::char('`', false));
        s.press(Key::char('1', false)); // 「，」
        assert_eq!(s.commit(), "，");
        assert_eq!(s.symbol_recent(), ["，"]);
        let snap = s.press(Key::char('`', false));
        assert_eq!(s.menu_tab(), Some(symbols::RECENT));
        assert_eq!(snap.candidates.len(), 1);
        assert_eq!(snap.candidates[0].surface, "，");

        // Esc 關閉選單，保留原本組字。
        type_syllable(&mut s, "ㄨㄛˇ");
        s.press(Key::char('`', false));
        let snap = s.press(Key::new(KeyCode::Escape, None, false));
        assert_eq!(snap.preedit, "我");
        assert!(s.menu_tab().is_none(), "Esc 只關選單");
    }

    /// 前身 `symbolGroups` 會用到的鍵盤標點與多字元符號都要在選單目錄裡。
    #[test]
    fn test_symbol_menu_keeps_every_keyboard_symbol() {
        let keyboard = crate::zh::keyboard::Keyboard::standard();
        for key in "<>?:\"!~[]{}\\()&'^_".chars() {
            if let Some(punct) = keyboard.punct(key) {
                assert!(
                    symbols::contains(punct),
                    "符號選單少了鍵盤 {key:?} 的 {punct:?}"
                );
            }
        }
        for item in ["……", "——", "←", "→", "↑", "↓"] {
            assert!(symbols::contains(item), "符號選單少了 {item:?}");
        }
    }

    /// 對照 `TestPaletteDirectSelectionAndEscapes` 的等號選單：`=` 開表情，
    /// Tab 換顏文字，選了就記下最近使用。
    #[test]
    fn test_emoji_palette_selection_and_recent() {
        let mut s = session();
        let snap = s.press(Key::char('=', false));
        assert_eq!(s.menu_tab(), Some("emoji"));
        assert_eq!(snap.candidates[0].surface, "=");
        assert_eq!(snap.candidates[1].surface, "😀");

        s.press(Key::char('2', false)); // 第二項「😀」
        assert_eq!(s.commit(), "😀");
        assert_eq!(s.emoji_recent(), ["😀"]);

        s.press(Key::char('=', false));
        assert_eq!(s.menu_tab(), Some("emoji"));
        assert_eq!(s.candidates()[1].surface, "😀", "最近使用的排前面");

        s.press(Key::new(KeyCode::Tab, None, false));
        assert_eq!(s.menu_tab(), Some("kaomoji"));
        s.press(Key::char('1', false));
        assert_eq!(s.commit(), "(^_^)");
        assert_eq!(s.emoji_recent(), ["😀", "(^_^)"]);

        // 下次直接開在上次選用的分頁。
        s.press(Key::char('=', false));
        assert_eq!(s.menu_tab(), Some("kaomoji"));
    }

    /// 對照 `TestPaletteDefaultEqualsChoiceAndOptOut`：等號本身是第一項，
    /// 選它不會被記成最近使用的表情。
    #[test]
    fn test_emoji_palette_equals_is_literal() {
        let mut s = session();
        s.press(Key::char('=', false));
        s.press(Key::char('1', false));
        assert_eq!(s.commit(), "=");
        assert!(s.emoji_recent().is_empty());

        // 選單開著再按一次等號，直接留下半形等號。
        s.press(Key::char('=', false));
        let snap = s.press(Key::char('=', false));
        assert_eq!(snap.preedit, "=");
        assert!(snap.candidates.is_empty());
        assert_eq!(s.commit(), "=");
    }

    /// 全形數字：英文模式打出 ASCII 數字時，候選提供 `０-９` 變體。
    #[test]
    fn test_full_width_digit_candidates() {
        let mut s = Session::new(SessionConfig::new(Mode::En));
        for ch in "123".chars() {
            s.press(Key::char(ch, false));
        }
        assert_eq!(s.snapshot().preedit, "123");
        s.press(Key::new(KeyCode::Tab, None, false));
        assert_eq!(s.candidates()[1].surface, "１２３");
        s.press(Key::char('2', false));
        assert_eq!(s.commit(), "１２３");
    }

    /// 英文拼錯時，候選在原字後面補上拼字建議；打對的字沒有拼字建議。
    #[test]
    fn test_english_candidates_include_spelling_suggestions() {
        let mut s = Session::new(SessionConfig::new(Mode::En));
        for ch in "recieve".chars() {
            s.press(Key::char(ch, false));
        }
        let surfaces: Vec<&str> = s
            .candidates()
            .iter()
            .map(|candidate| candidate.surface.as_str())
            .collect();
        assert_eq!(surfaces.first().copied(), Some("recieve"));
        let at = surfaces
            .iter()
            .position(|text| *text == "receive")
            .expect("要有 receive 的拼字建議");
        assert!(at > 0, "拼字建議要排在原字後面：{surfaces:?}");

        let mut correct = Session::new(SessionConfig::new(Mode::En));
        for ch in "hello".chars() {
            correct.press(Key::char(ch, false));
        }
        let alternatives: Vec<&str> = correct
            .candidates()
            .iter()
            .map(|candidate| candidate.surface.as_str())
            .filter(|text| text.len() == 5 && !text.eq_ignore_ascii_case("hello"))
            .collect();
        assert!(
            alternatives.is_empty(),
            "打對的字不該有建議：{alternatives:?}"
        );
    }

    /// 混合模式的待定英文詞也能看到拼字建議，選了就取代原按鍵。
    #[test]
    fn test_mixed_candidates_include_spelling_suggestions() {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(lexicon());
        let mut s = Session::new(config);
        type_keys(&mut s, "recieve");
        let surfaces: Vec<String> = s
            .candidates()
            .iter()
            .map(|candidate| candidate.surface.clone())
            .collect();
        assert_eq!(surfaces.first().map(String::as_str), Some("recieve"));
        let at = surfaces
            .iter()
            .position(|text| text == "receive")
            .expect("要有 receive 的拼字建議");
        assert!(at > 0, "拼字建議要排在原字後面：{surfaces:?}");

        s.press(Key::new(KeyCode::Tab, None, false));
        s.press(Key::char('2', false));
        assert_eq!(s.snapshot().preedit, "receive");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "receive");
    }

    /// 最小日文詞條來源，讓日文 session 不必載入 pack 也能轉換。
    struct LiveTestSource(Vec<crate::ja::lexicon::JaEntry>);

    impl crate::ja::decode::JaWordSource for LiveTestSource {
        fn longest_reading_units(&self) -> usize {
            self.0.iter().map(|entry| entry.units()).max().unwrap_or(0)
        }

        fn matches<'a>(
            &'a self,
            reading: &[char],
            start: usize,
            out: &mut dyn FnMut(crate::ja::decode::JaWordMatch<'a>),
        ) {
            for entry in &self.0 {
                let units: Vec<char> = entry.reading.chars().collect();
                let end = start + units.len();
                if end > reading.len() || reading[start..end] != units[..] {
                    continue;
                }
                out(crate::ja::decode::JaWordMatch {
                    end,
                    entry,
                    left_id: None,
                    right_id: None,
                });
            }
        }

        fn exact(&self, reading: &[char]) -> Vec<&crate::ja::lexicon::JaEntry> {
            self.0
                .iter()
                .filter(|entry| entry.reading.chars().eq(reading.iter().copied()))
                .collect()
        }

        fn prefixed(&self, reading: &[char]) -> Vec<&crate::ja::lexicon::JaEntry> {
            self.0
                .iter()
                .filter(|entry| {
                    let units: Vec<char> = entry.reading.chars().collect();
                    units.len() >= reading.len() && units[..reading.len()] == *reading
                })
                .collect()
        }
    }

    fn ja_live_session(config: live::Config) -> Session {
        let entry = crate::ja::lexicon::JaEntry {
            surface: "今日".into(),
            reading: "きょう".into(),
            pos: Arc::from("名詞,普通名詞,一般,*"),
            conj_type: Arc::from("*"),
            conj_form: Arc::from("*"),
            cost: 0,
            split: crate::ja::lexicon::Split::C,
        };
        let decoder = Arc::new(JaDecoder::new(Arc::new(LiveTestSource(vec![entry]))));
        let mut session_config = SessionConfig::new(Mode::Ja);
        session_config.ja = Some(decoder);
        session_config.ja_live = config;
        Session::new(session_config)
    }

    fn type_ja(session: &mut Session, keys: &str) {
        for ch in keys.chars() {
            session.press(Key::char(ch, false));
        }
    }

    fn ja_entry(surface: &str, reading: &str, cost: i32) -> crate::ja::lexicon::JaEntry {
        crate::ja::lexicon::JaEntry {
            surface: surface.into(),
            reading: reading.into(),
            pos: Arc::from("名詞,普通名詞,一般,*"),
            conj_type: Arc::from("*"),
            conj_form: Arc::from("*"),
            cost,
            split: crate::ja::lexicon::Split::C,
        }
    }

    /// 同讀音、成本差距可被學習翻盤的日文 session。
    fn ja_learning_session(learning: bool) -> Session {
        let source = LiveTestSource(vec![
            ja_entry("今日", "きょう", 1_000),
            ja_entry("京", "きょう", 1_400),
            ja_entry("きょう", "きょう", 3_000),
        ]);
        let mut config = SessionConfig::new(Mode::Ja);
        config.ja = Some(Arc::new(JaDecoder::new(Arc::new(source))));
        config.learning = learning;
        Session::new(config)
    }

    /// 手選第二候選後，同讀音的首選會改變；undo 回到原狀。
    #[test]
    fn test_ja_learning_ranks_manual_choice_first_and_undo_restores() {
        use crate::personal::LanguageId;

        let mut s = ja_learning_session(true);
        type_ja(&mut s, "kyou");
        assert_eq!(s.candidates()[0].surface, "今日");
        // 第一次方向鍵只開候選窗，第二次移到第二候選。
        s.press(Key::new(KeyCode::ArrowRight, None, false));
        s.press(Key::new(KeyCode::ArrowRight, None, false));
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "京");
        assert_eq!(
            s.personal()
                .language_reading_count(LanguageId::Ja, "き ょ う", "京"),
            2,
            "明確手選記兩倍證據"
        );

        // 同一讀音再打一次：學習後的首選是剛剛手選的詞。
        type_ja(&mut s, "kyou");
        assert_eq!(s.candidates()[0].surface, "京");

        // 反悔後回到統計順序。
        assert!(s.undo_learning());
        assert_eq!(s.candidates()[0].surface, "今日");
    }

    /// 學習關閉時，手選第二候選不會改變下一次的首選。
    #[test]
    fn test_ja_learning_disabled_keeps_statistical_order() {
        let mut s = ja_learning_session(false);
        type_ja(&mut s, "kyou");
        s.press(Key::new(KeyCode::ArrowRight, None, false));
        s.press(Key::new(KeyCode::ArrowRight, None, false));
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "京");
        type_ja(&mut s, "kyou");
        assert_eq!(s.candidates()[0].surface, "今日");
    }

    /// 單次被動接受不足以翻掉明顯的統計差距。
    #[test]
    fn test_ja_passive_learning_keeps_clear_statistical_winner() {
        let mut s = ja_learning_session(true);
        type_ja(&mut s, "kyou");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "今日");
        type_ja(&mut s, "kyou");
        assert_eq!(s.candidates()[0].surface, "今日");
    }

    /// 沒有開啟逐字変換時，日文走原本的 nbest 流程，結果與先前相同。
    #[test]
    fn test_ja_live_disabled_by_default_keeps_plain_conversion() {
        let mut s = ja_live_session(live::Config::default());
        assert!(!s.ja_live_enabled());
        type_ja(&mut s, "kyou");
        assert_eq!(s.snapshot().preedit, "今日");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "今日");
    }

    /// 開啟逐字変換後，輸入到詞邊界就顯示轉換結果，不必等 Enter。
    #[test]
    fn test_ja_live_enabled_converts_without_enter() {
        let config = live::Config {
            enabled: true,
            debounce: std::time::Duration::ZERO,
            ..live::Config::default()
        };
        let mut s = ja_live_session(config);
        assert!(s.ja_live_enabled());
        type_ja(&mut s, "kyou");
        assert_eq!(s.snapshot().preedit, "今日");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "今日");
    }
}
