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

use crate::candidate::{Candidate, CandidateSource};
use crate::decode::{Decoder, Hypothesis, Locks};
use crate::emoji::{self, Stat};
use crate::ja::decode::{JaContext, JaDecoder};
use crate::ja::input::RomajiInput;
use crate::ja::live::{self, Controller as LiveController, Plan as LivePlan};
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
const TONELESS_COMBINATIONS: usize = 256;

/// 模糊音最多展開幾種讀音組合。
const FUZZY_COMBINATIONS: usize = 16;

/// 預設開啟的模糊音配對：ㄗ／ㄓ、ㄘ／ㄔ、ㄙ／ㄕ、ㄣ／ㄥ。
pub const DEFAULT_FUZZY_PAIRS: [(char, char); 4] =
    [('ㄗ', 'ㄓ'), ('ㄘ', 'ㄔ'), ('ㄙ', 'ㄕ'), ('ㄣ', 'ㄥ')];

/// 累積幾次學習事件後清理一次個人資料。
const LEARN_PRUNE_INTERVAL: usize = 64;

/// 目前 Unix 秒數；時鐘早於 epoch 時回傳 0。
fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// 左文保留的字元上限，對齊前身 `MaxLeftContextUnits`。
const MAX_LEFT_CONTEXT_UNITS: usize = 128;

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
    /// 是否呼叫日文解碼器上的重排器；預設關閉，實測 modern-500 由 86.0%
    /// 退到 82.0%，因此不再預設啟用。
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
enum MenuKind {
    /// 反引號開的符號選單。
    Symbols,
    /// 等號開的表情選單。
    Emoji,
}

/// 開啟中的候選選單。
#[derive(Debug, Clone, Copy)]
struct Menu {
    /// 選單種類。
    kind: MenuKind,
    /// 目前分頁序號。
    tab: usize,
}

/// 一次輸入的工作階段。
///
/// 只保留組字所需的狀態；送出後立即清空，因此呼叫端每次都用快照重畫。
pub struct Session {
    config: SessionConfig,
    zh: ZhInput,
    romaji: RomajiInput,
    lexicon: InMemoryLexicon,
    /// 不含自訂詞的原始詞典；自訂詞變動時由它重建 [`Session::lexicon`]。
    base_lexicon: InMemoryLexicon,
    /// 個人化學習資料。
    personal: PersonalStore,
    ctx: ScoreContext,
    zh_lm: Option<Arc<crate::score::ZhLm>>,
    /// 注音以外的字面按鍵（標點、英文、直接輸入的注音符號）。
    literal: String,
    /// 字面按鍵是在幾個已完成音節之後打的；用來插回轉換結果中間。
    literal_anchor: usize,
    /// 字面按鍵在目前轉換結果中的位元組位置。
    literal_offset: usize,
    /// 英文與混合模式累積的原始輸入。
    raw: String,
    candidates: Vec<Candidate>,
    highlighted: usize,
    /// 候選視窗是否由 Tab／方向鍵開啟；數字鍵只在開啟時選字。
    candidate_window: bool,
    segments: Vec<Segment>,
    /// 已轉換的部分。
    base: String,
    /// 已送出的文字尾巴；下一次轉換當左文用（前身 `lang.Context.Left`）。
    committed: String,
    /// 尚未轉換的尾巴（未完成的注音或羅馬字）。
    pending: String,
    /// `Enter` 先放進這裡，下一次 [`Session::commit`] 取走。
    pending_commit: Option<String>,
    /// Tab 接受的補全結果；有值時取代組字結果。
    accepted_completion: Option<String>,
    /// 尾端補全建議。
    suggestion: Option<String>,
    /// 待選路徑的快照；提交時用來依實際片段學習。
    last_hypotheses: Vec<Hypothesis>,
    /// 混合模式的英數按鍵推斷。
    ascii: AsciiInput,
    /// 目前開啟的候選選單。
    menu: Option<Menu>,
    /// 表情分頁的最後選擇；下次開選單時沿用。
    palette_tab: usize,
    /// 符號選單「最近」分頁的順序。
    symbol_recent: Vec<String>,
    /// 符號「最近」清單的常用分數。
    symbol_stats: HashMap<String, Stat>,
    /// 表情選單「最近」分頁的順序。
    emoji_recent: Vec<String>,
    /// 表情「最近」清單的常用分數。
    emoji_stats: HashMap<String, Stat>,
    /// 日文逐字変換控制器；關閉時為 `None`，日文走一般変換。
    live: Option<LiveController>,
    /// 最近一次逐字変換的決定，供 debounce 到期後重畫。
    live_plan: Option<LivePlan>,
    /// 上次清理個人資料後累積的學習事件數。
    learn_events: usize,
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
            base: String::new(),
            committed: String::new(),
            pending: String::new(),
            pending_commit: None,
            accepted_completion: None,
            suggestion: None,
            last_hypotheses: Vec::new(),
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
        if key.ctrl {
            self.press_ctrl(key);
        } else if !self.menu_key(&key) {
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

    /// 把送出的文字接到左文尾巴，長度上限對齊前身 `MaxLeftContextUnits`。
    fn remember_committed(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.committed.push_str(text);
        let len = self.committed.chars().count();
        if len > MAX_LEFT_CONTEXT_UNITS {
            let skip = len - MAX_LEFT_CONTEXT_UNITS;
            self.committed = self.committed.chars().skip(skip).collect();
        }
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
            segments: self.segments.clone(),
            candidates: self.candidates.clone(),
            highlighted: self.highlighted,
        }
    }

    /// 游標位置，以 UTF-16 code unit 計算。
    pub fn cursor_utf16(&self) -> usize {
        utf16_len(&self.preedit_string())
    }

    fn max_candidates(&self) -> usize {
        self.config.max_candidates.max(1)
    }

    /// Ctrl 組合鍵：目前只處理啟用後的 `Ctrl+,`。
    fn press_ctrl(&mut self, key: Key) {
        if !self.config.ctrl_punct {
            return;
        }
        if key.code == KeyCode::Char(',') {
            self.push_literal("，");
        }
    }

    /// 把字面文字插進組字結果；記錄插入位置。
    fn push_literal(&mut self, text: &str) {
        if self.literal.is_empty() {
            self.literal_anchor = self.zh.syllables().len();
        }
        self.literal.push_str(text);
    }

    /// 接受目前的補全建議；沒有建議時回傳 `false`。
    fn accept_suggestion(&mut self) -> bool {
        let Some(text) = self.suggestion.take() else {
            return false;
        };
        self.accepted_completion = Some(text);
        self.candidate_window = false;
        true
    }

    fn press_char(&mut self, ch: char) {
        self.accepted_completion = None;
        self.suggestion = None;
        let window = self.candidate_window;
        if window && self.select_by_digit(ch) {
            self.candidate_window = false;
            return;
        }
        if self.menu.is_some() {
            let kind = self.menu.map(|menu| menu.kind);
            if ch == '=' && kind == Some(MenuKind::Emoji) {
                self.menu = None;
                self.candidate_window = false;
                self.highlighted = 0;
                self.push_literal("=");
                return;
            }
            if ch == '`' && kind == Some(MenuKind::Symbols) {
                self.menu = None;
                self.candidate_window = false;
                self.highlighted = 0;
                return;
            }
            self.menu = None;
            self.highlighted = 0;
        }
        self.candidate_window = false;
        if !window {
            self.highlighted = 0;
        }
        match self.config.mode {
            Mode::Zh => {
                if self.zh.pending().is_empty() {
                    if ch == '`' {
                        self.open_menu(MenuKind::Symbols, 0);
                        return;
                    }
                    if ch == '=' {
                        self.open_menu(MenuKind::Emoji, self.palette_tab);
                        return;
                    }
                }
                match self.zh.press(Key::char(ch, false)) {
                    crate::zh::input::ZhInputEvent::Consumed
                    | crate::zh::input::ZhInputEvent::SyllableComplete(_) => {}
                    crate::zh::input::ZhInputEvent::Commit(reading) => {
                        // 空白鍵在沒有未完成注音時提交；改用轉換後的結果。
                        if let Some(text) = self.convert_reading(&reading) {
                            self.pending_commit = Some(text);
                            self.zh.reset();
                        }
                    }
                    crate::zh::input::ZhInputEvent::Cancel => self.zh.reset(),
                    crate::zh::input::ZhInputEvent::Ignored => {
                        let punct = self.zh.keyboard().punct(ch);
                        let text = punct.map(str::to_string).unwrap_or_else(|| ch.to_string());
                        self.push_literal(&text);
                    }
                }
            }
            Mode::Ja => {
                self.romaji.press(Key::char(ch, false));
            }
            Mode::En => self.raw.push(ch),
            Mode::Mixed => {
                if !self.ascii.tone(ch, &self.lexicon) {
                    self.ascii.press(ch);
                }
            }
        }
    }

    /// 數字鍵在候選視窗開啟時選字；成功時把該候選放進待提交區。
    fn select_by_digit(&mut self, ch: char) -> bool {
        if self.candidates.is_empty()
            || !self.candidate_window
            || (self.config.mode == Mode::Zh
                && !self.zh.pending().is_empty()
                && self.menu.is_none())
        {
            return false;
        }
        let index = match ch {
            '1'..='9' => ch as usize - '1' as usize,
            '0' => 9,
            _ => return false,
        };
        let Some(candidate) = self.candidates.get(index).cloned() else {
            return false;
        };
        // 混合模式的候選是英文補完或拼字建議，選了就取代待定按鍵。
        if self.config.mode == Mode::Mixed && self.ascii.accept(&candidate.surface) {
            self.candidate_window = false;
            self.highlighted = 0;
            return true;
        }
        self.remember_text(&candidate.surface);
        self.learn_candidate(index);
        let text = if self.menu.is_some() {
            self.compose_literal(&candidate.surface)
        } else {
            self.insert_literal(&candidate.surface)
        };
        self.pending_commit = Some(self.finalize(text));
        self.clear_composition();
        true
    }

    fn space(&mut self) {
        match self.config.mode {
            Mode::Zh if !self.zh.pending().is_empty() => {
                self.zh.press(Key::new(KeyCode::Space, None, false));
            }
            _ if self.candidate_window && !self.candidates.is_empty() => {
                self.remember_selection();
                self.learn_composition();
                let text = self.finalize(self.current_text());
                self.pending_commit = Some(text);
                self.clear_composition();
            }
            Mode::En => self.raw.push(' '),
            Mode::Mixed => self.ascii.space(&self.lexicon),
            _ => {}
        }
    }

    fn backspace(&mut self) {
        self.candidate_window = false;
        match self.config.mode {
            Mode::Zh => {
                if self.zh.pending().is_empty()
                    && self.zh.syllables().is_empty()
                    && !self.literal.is_empty()
                {
                    self.literal.pop();
                } else {
                    self.zh.press(Key::new(KeyCode::Backspace, None, false));
                }
            }
            Mode::Ja => {
                self.romaji.backspace();
            }
            Mode::En => {
                self.raw.pop();
            }
            Mode::Mixed => {
                self.ascii.backspace();
            }
        }
    }

    fn finish_composition(&mut self) {
        if self.config.mode == Mode::Mixed {
            self.ascii.flush();
        }
        self.remember_selection();
        self.learn_composition();
        let text = match self.config.mode {
            Mode::Mixed => self.mixed_surface(),
            _ => self.current_text(),
        };
        let text = self.finalize(text);
        if !text.is_empty() {
            self.pending_commit = Some(text);
        }
        self.clear_composition();
    }

    /// 送出前套用簡體轉換與中英自動空格。
    fn finalize(&self, text: String) -> String {
        let text = self.puncted(text);
        let text = match &self.config.simplified {
            Some(converter) => converter.to_simplified(&text),
            None => text,
        };
        if self.config.cjk_latin_space {
            space_text(&text, Spacing::Always)
        } else {
            text
        }
    }

    /// 送出前呼叫標點模型；沒有設定時原樣回傳。
    fn model_punctuated(&self, text: String) -> String {
        let Some(punctuator) = &self.config.punct else {
            return text;
        };
        if text.is_empty() {
            return text;
        }
        punctuator.punctuate(&text, self.config.mode)
    }

    /// 依設定在最後一個漢字前插入標點；規則同 [`punct::apply`]。
    fn puncted(&self, text: String) -> String {
        let Some(mark) = self.config.punct_mark else {
            return text;
        };
        let chars: Vec<char> = text.chars().collect();
        let Some(offset) = chars.iter().rposition(|ch| is_han(*ch)) else {
            return text;
        };
        let edits = [Edit::new(offset, mark.to_string())];
        match punct::apply(&text, &edits) {
            (out, true) => out,
            (_, false) => text,
        }
    }

    /// 開啟選單並重建候選；選取時插入到目前組字尾端。
    fn open_menu(&mut self, kind: MenuKind, tab: usize) -> Snapshot {
        if self.literal.is_empty() {
            self.literal_anchor = self.zh.syllables().len();
        }
        self.literal_offset = self
            .segments
            .last()
            .map(|segment| segment.end)
            .unwrap_or(self.base.len());
        let names = self.menu_tab_names(kind);
        let tab = tab.min(names.len().saturating_sub(1));
        self.menu = Some(Menu { kind, tab });
        self.highlighted = 0;
        self.candidate_window = true;
        self.rebuild();
        self.snapshot()
    }

    /// 選單的分頁名稱。
    fn menu_tab_names(&self, kind: MenuKind) -> Vec<&'static str> {
        match kind {
            MenuKind::Symbols => {
                let mut names = Vec::new();
                if !self.symbol_recent.is_empty() {
                    names.push(symbols::RECENT);
                }
                names.extend(symbols::TABS.iter().map(|tab| tab.name));
                names
            }
            MenuKind::Emoji => vec!["emoji", "kaomoji"],
        }
    }

    /// 目前分頁的候選字面。
    fn menu_items(&self, menu: Menu) -> Vec<String> {
        match menu.kind {
            MenuKind::Symbols => {
                let names = self.menu_tab_names(menu.kind);
                let Some(name) = names.get(menu.tab) else {
                    return Vec::new();
                };
                if *name == symbols::RECENT {
                    symbols::clean_recent(&self.symbol_recent)
                        .into_iter()
                        .take(PAGE_SIZE)
                        .collect()
                } else {
                    symbols::tab_items(name)
                        .map(|items| items.iter().map(|item| (*item).to_string()).collect())
                        .unwrap_or_default()
                }
            }
            MenuKind::Emoji => {
                let kind = if menu.tab == 0 { "emoji" } else { "kaomoji" };
                let mut items = Vec::new();
                if kind == "emoji" {
                    items.push("=".to_string());
                }
                items.extend(emoji::palette(kind, &self.emoji_recent));
                items
            }
        }
    }

    fn rebuild_menu(&mut self) {
        let Some(menu) = self.menu else {
            return;
        };
        let items = self.menu_items(menu);
        self.candidates = items
            .into_iter()
            .map(|text| Candidate::new(text, "", 0, CandidateSource::Literal))
            .collect();
        self.candidate_window = true;
        if self.highlighted >= self.candidates.len() {
            self.highlighted = 0;
        }
    }

    /// 選單開啟時的 Tab／Shift＋Tab 與退格；其他按鍵交回一般處理。
    fn menu_key(&mut self, key: &Key) -> bool {
        let Some(menu) = self.menu else {
            return false;
        };
        match key.code {
            KeyCode::Tab => {
                let names = self.menu_tab_names(menu.kind);
                if names.is_empty() {
                    return false;
                }
                let step = if key.shift { -1 } else { 1 };
                let len = names.len() as isize;
                let tab = (menu.tab as isize + step).rem_euclid(len) as usize;
                self.menu = Some(Menu {
                    kind: menu.kind,
                    tab,
                });
                self.highlighted = 0;
                true
            }
            KeyCode::Backspace | KeyCode::Escape | KeyCode::Delete => {
                // 只關選單，不刪除已組好的字。
                self.menu = None;
                self.candidate_window = false;
                true
            }
            _ => false,
        }
    }

    /// 記下目前高亮候選的字面；只有明確開啟選單時才算。
    fn remember_selection(&mut self) {
        let Some(text) = self
            .candidates
            .get(self.highlighted)
            .map(|candidate| candidate.surface.clone())
        else {
            return;
        };
        self.remember_text(&text);
    }

    /// 記下指定字面的選取；數字鍵直接選取時要高亮以外的項目。
    fn remember_text(&mut self, text: &str) {
        let Some(menu) = self.menu else {
            return;
        };
        let text = text.to_string();
        match menu.kind {
            MenuKind::Symbols => {
                if symbols::contains(&text) {
                    let order = symbols::clean_recent(&self.symbol_recent);
                    let (order, stats) =
                        emoji::rank(&order, &self.symbol_stats, &text, 0, symbols::MAX_RECENT);
                    self.symbol_recent = order;
                    self.symbol_stats = stats;
                }
            }
            MenuKind::Emoji => {
                if text != "=" {
                    let (order, stats) = emoji::rank(
                        &self.emoji_recent,
                        &self.emoji_stats,
                        &text,
                        0,
                        emoji::MAX_RECENT,
                    );
                    self.emoji_recent = order;
                    self.emoji_stats = stats;
                    self.palette_tab = menu.tab;
                }
            }
        }
    }

    /// 方向鍵：候選窗還沒開時先開啟並停在目前候選，不立刻移動。
    ///
    /// 前身把「只是顯示」與「明確選取」分開；Down 第一次只進入選取模式，
    /// 第二次才換下一項。這裡用同一個規則簡化兩顆方向鍵。
    fn navigate(&mut self, delta: isize) {
        if self.candidates.is_empty() {
            return;
        }
        if !self.candidate_window {
            self.candidate_window = true;
            return;
        }
        self.move_highlight(delta);
    }

    /// 把這次送出的候選記進個人學習。
    fn learn_composition(&mut self) {
        let index = self.highlighted;
        self.learn_candidate(index);
    }

    /// 把指定候選記進個人學習；數字鍵直接選取時用得到。
    fn learn_candidate(&mut self, index: usize) {
        if !self.config.learning || self.config.mode != Mode::Zh || self.menu.is_some() {
            return;
        }
        self.learn_events += 1;
        self.personal.advance_time(now_seconds());
        if self.learn_events.is_multiple_of(LEARN_PRUNE_INTERVAL) {
            self.personal
                .prune_capacity(crate::personal::PrunePolicy::default());
        }
        let manual = self.candidate_window;
        let Some(candidate) = self.candidates.get(index).cloned() else {
            return;
        };
        let units: Vec<&str> = candidate
            .reading
            .split_whitespace()
            .filter(|unit| !unit.is_empty())
            .collect();
        if units.is_empty() {
            return;
        }
        let hypothesis = self
            .last_hypotheses
            .iter()
            .find(|hypothesis| hypothesis.text == candidate.surface);
        if let Some(hypothesis) = hypothesis {
            for segment in &hypothesis.segments {
                if segment.partial
                    || segment.start >= segment.end
                    || segment.end > units.len()
                    || !segment.word.chars().next().is_some_and(is_han)
                {
                    continue;
                }
                let reading = units[segment.start..segment.end].join(" ");
                let syllables = segment.end - segment.start;
                if manual {
                    self.personal
                        .note_manual(&segment.word, &reading, syllables);
                } else {
                    self.personal.note_passive(&segment.word, &reading);
                }
            }
        } else if candidate.surface.chars().next().is_some_and(is_han) {
            // 模糊音或自訂詞候選沒有對應路徑，整串當一個單位學習。
            let syllables = units.len();
            if manual {
                self.personal
                    .note_manual(&candidate.surface, &candidate.reading, syllables);
            } else {
                self.personal
                    .note_passive(&candidate.surface, &candidate.reading);
            }
        }
    }

    /// 候選窗的反悔鍵：自訂詞刪除、學到的詞忘記。
    fn forget_candidate(&mut self) {
        let Some(candidate) = self.candidates.get(self.highlighted).cloned() else {
            return;
        };
        if self.personal.is_vocab(&candidate.surface) {
            self.personal.remove_vocab(&candidate.surface);
            self.refresh_lexicon();
        } else if self.personal.is_learned(&candidate.surface) {
            self.personal.forget(&candidate.surface);
        }
        self.highlighted = 0;
    }

    /// 撤銷與某個詞有關的學習；供 `forget` 協定請求使用。
    pub fn forget_word(&mut self, word: &str) -> bool {
        let changed = self.personal.forget(word);
        if changed {
            self.highlighted = 0;
        }
        changed
    }

    /// 自訂詞變動後重建詞典。
    fn refresh_lexicon(&mut self) {
        self.lexicon = self.personal.augment_lexicon(&self.base_lexicon);
    }

    fn move_highlight(&mut self, delta: isize) {
        let len = self.candidates.len();
        if len == 0 {
            return;
        }
        self.candidate_window = true;
        if len == 1 {
            return;
        }
        let len = len as isize;
        let mut index = (self.highlighted as isize + delta) % len;
        if index < 0 {
            index += len;
        }
        self.highlighted = index as usize;
    }

    fn clear_composition(&mut self) {
        self.zh.reset();
        self.romaji.reset();
        self.ascii.reset();
        self.literal.clear();
        self.literal_anchor = 0;
        self.literal_offset = 0;
        self.raw.clear();
        self.candidates.clear();
        self.highlighted = 0;
        self.candidate_window = false;
        self.segments.clear();
        self.base.clear();
        self.pending.clear();
        self.last_hypotheses.clear();
        self.accepted_completion = None;
        self.suggestion = None;
        self.menu = None;
        if let Some(live) = &mut self.live {
            live.reset();
        }
        self.live_plan = None;
    }

    /// 目前要提交的文字：接受的補全優先，其次高亮候選，最後轉換結果。
    fn current_text(&self) -> String {
        if self.config.mode == Mode::Mixed {
            return self.mixed_surface();
        }
        if self.menu.is_some() {
            if let Some(candidate) = self.candidates.get(self.highlighted) {
                return self.compose_literal(&candidate.surface);
            }
        }
        if let Some(text) = &self.accepted_completion {
            return format!("{}{}", self.insert_literal(text), self.pending);
        }
        if let Some(candidate) = self.candidates.get(self.highlighted) {
            return format!(
                "{}{}",
                self.insert_literal(&candidate.surface),
                self.pending
            );
        }
        self.preedit_string()
    }

    /// 高亮候選或轉換結果加上尚未轉換的尾巴。
    fn preedit_string(&self) -> String {
        format!("{}{}", self.insert_literal(&self.base), self.pending)
    }

    /// 顯示用字串：高亮候選反映在組字區。
    fn base_with_highlight(&self) -> String {
        if self.menu.is_some() {
            // 選單開啟時組字區維持原樣，候選只是待選清單。
            return self.preedit_string();
        }
        let base = match (
            &self.accepted_completion,
            self.candidates.get(self.highlighted),
        ) {
            (Some(text), _) => text.as_str(),
            // 混合模式的候選只取代待定英數按鍵，`base` 已含前面的中文片段；
            // 直接拿候選當整個組字字串會把中文吃掉。
            (None, _) if self.config.mode == Mode::Mixed => self.base.as_str(),
            (None, Some(candidate)) => candidate.surface.as_str(),
            (None, None) => self.base.as_str(),
        };
        format!("{}{}", self.insert_literal(base), self.pending)
    }

    /// 把字面按鍵插回轉換結果的對應位置。
    fn insert_literal(&self, base: &str) -> String {
        if self.literal.is_empty() {
            return base.to_string();
        }
        let mut offset = self.literal_offset.min(base.len());
        while offset > 0 && !base.is_char_boundary(offset) {
            offset -= 1;
        }
        let mut out = String::with_capacity(base.len() + self.literal.len());
        out.push_str(&base[..offset]);
        out.push_str(&self.literal);
        out.push_str(&base[offset..]);
        out
    }

    /// 把選單選取的字面插回目前組字結果的對應位置。
    fn compose_literal(&self, text: &str) -> String {
        let mut offset = self.literal_offset.min(self.base.len());
        while offset > 0 && !self.base.is_char_boundary(offset) {
            offset -= 1;
        }
        let mut out = String::with_capacity(self.base.len() + self.literal.len() + text.len());
        out.push_str(&self.base[..offset]);
        out.push_str(&self.literal);
        out.push_str(text);
        out.push_str(&self.base[offset..]);
        out
    }

    fn rebuild(&mut self) {
        if self.menu.is_some() {
            self.rebuild_menu();
            return;
        }
        self.base.clear();
        self.pending.clear();
        self.candidates.clear();
        self.segments.clear();
        self.literal_offset = 0;
        match self.config.mode {
            Mode::Zh => self.rebuild_zh(),
            Mode::Ja => self.rebuild_ja(),
            Mode::En => self.rebuild_en(),
            Mode::Mixed => self.rebuild_mixed(),
        }
        if self.highlighted >= self.candidates.len() {
            self.highlighted = 0;
        }
    }

    fn rebuild_zh(&mut self) {
        self.pending = self.zh.pending();
        let syllables = self.zh.syllables().to_vec();
        if syllables.is_empty() {
            return;
        }
        let readings = self.zh_readings(&syllables);
        let units: Vec<&str> = readings.iter().map(String::as_str).collect();
        let Ok(lattice) = Lattice::build(&units, &self.lexicon, &self.ctx) else {
            self.base = units.join("");
            return;
        };
        let scorer = zh_scorer(&self.ctx, self.zh_lm.as_deref());
        let decoder = zh_decoder(&self.lexicon, &self.ctx, scorer.as_ref());
        let (segs, _) = decoder.best(&lattice, &Locks::new(), false);
        let conversion = decoder.conversion(&lattice, &segs, Mode::Zh);
        self.base = conversion.surface;
        self.segments = Self::segments_of(&segs, Mode::Zh, Script::Bopomofo);
        self.literal_offset = if self.literal_anchor == 0 {
            0
        } else {
            self.segments
                .get(self.literal_anchor - 1)
                .map(|segment| segment.end)
                .unwrap_or(self.base.len())
        };
        let reading = readings.join(" ");
        let literal = readings.join("");
        let (mut hypotheses, _) =
            decoder.nbest(&lattice, &Locks::new(), false, self.max_candidates());
        if self.config.learning {
            self.personal.rank_hypotheses(&mut hypotheses, &reading);
        }
        if let Some(options) = &self.config.rerank {
            Self::apply_rerank(&mut hypotheses, options);
        }
        if let Some(best) = hypotheses.first() {
            self.base = best.text.clone();
            self.segments = Self::segments_of(&best.segments, Mode::Zh, Script::Bopomofo);
        }
        self.last_hypotheses = hypotheses.clone();
        let mut candidates: Vec<Candidate> = hypotheses
            .into_iter()
            .map(|hypothesis| {
                let source = if hypothesis.text == literal {
                    CandidateSource::Literal
                } else {
                    CandidateSource::Lexicon
                };
                Candidate::new(hypothesis.text, reading.clone(), hypothesis.cost, source)
            })
            .collect();
        let near = self.fuzzy_candidates(&readings);
        if !near.is_empty() {
            let at = candidates
                .iter()
                .position(|candidate| candidate.surface == literal)
                .unwrap_or(candidates.len());
            for (offset, candidate) in near.into_iter().enumerate() {
                let index = (at + offset).min(candidates.len());
                candidates.insert(index, candidate);
            }
            let mut seen = Vec::new();
            candidates.retain(|candidate| {
                if seen.contains(&candidate.surface) {
                    false
                } else {
                    seen.push(candidate.surface.clone());
                    true
                }
            });
        }
        candidates.truncate(self.max_candidates());
        self.candidates = candidates;
        let (completion, _) = decoder.complete(&lattice, &segs);
        self.suggestion = if completion.found() {
            Some(completion.entry.surface)
        } else {
            None
        };
    }

    /// 目前音節要交給詞格的讀音。
    ///
    /// 聲調選填開啟時，沒有聲調的音節展開成五個聲調，從組合中挑成本最低的
    /// 一條；組合數超過上限（長句）時維持原樣，避免指數爆炸。
    fn zh_readings(&self, syllables: &[Syllable]) -> Vec<String> {
        let exact: Vec<String> = syllables
            .iter()
            .map(|syllable| syllable.as_str().to_string())
            .collect();
        if !self.config.toneless {
            return exact;
        }
        let rows: Vec<Vec<String>> = syllables
            .iter()
            .map(|syllable| {
                if syllable.tone() != 1 {
                    return vec![syllable.as_str().to_string()];
                }
                (1..=5)
                    .map(|tone| syllable.with_tone(tone).as_str().to_string())
                    .collect()
            })
            .collect();
        let combinations: usize = rows.iter().map(Vec::len).product();
        if combinations == 0 || combinations > TONELESS_COMBINATIONS {
            return exact;
        }
        let mut best: Option<(i32, Vec<String>)> = None;
        let mut index = vec![0usize; rows.len()];
        loop {
            let readings: Vec<String> = rows
                .iter()
                .zip(index.iter())
                .map(|(row, &i)| row[i].clone())
                .collect();
            let units: Vec<&str> = readings.iter().map(String::as_str).collect();
            if let Ok(lattice) = Lattice::build(&units, &self.lexicon, &self.ctx) {
                let scorer = zh_scorer(&self.ctx, self.zh_lm.as_deref());
                let decoder = zh_decoder(&self.lexicon, &self.ctx, scorer.as_ref());
                let (_, cost) = decoder.best(&lattice, &Locks::new(), false);
                if best.as_ref().is_none_or(|(best_cost, _)| cost < *best_cost) {
                    best = Some((cost, readings));
                }
            }
            let mut at = index.len();
            loop {
                if at == 0 {
                    return best.map(|(_, readings)| readings).unwrap_or(exact);
                }
                at -= 1;
                index[at] += 1;
                if index[at] < rows[at].len() {
                    break;
                }
                index[at] = 0;
            }
        }
    }

    /// 依固定替身設定重排：偏好字串命中時扣掉換算後的成本。
    fn apply_rerank(hypotheses: &mut [crate::decode::Hypothesis], options: &RerankOptions) {
        let score = if options.prefer_score > 0.0 {
            options.prefer_score
        } else {
            10.0
        };
        let bonus = (options.weight * score * 1000.0).round() as i32;
        hypotheses.sort_by_key(|hypothesis| {
            let preferred = options
                .prefer
                .iter()
                .any(|text| hypothesis.text.contains(text));
            if preferred {
                hypothesis.cost.saturating_sub(bonus)
            } else {
                hypothesis.cost
            }
        });
    }

    /// 模糊音候選：把已完成的音節換成近音讀音，取候選補進清單。
    fn fuzzy_candidates(&self, readings: &[String]) -> Vec<Candidate> {
        if self.config.fuzzy_pairs.is_empty() {
            return Vec::new();
        }
        let mut variants: Vec<Vec<String>> = vec![Vec::new()];
        for reading in readings {
            let mut alternatives = vec![reading.clone()];
            for &(left, right) in &self.config.fuzzy_pairs {
                for (from, to) in [(left, right), (right, left)] {
                    if reading.contains(from) {
                        let swapped: String = reading
                            .chars()
                            .map(|ch| if ch == from { to } else { ch })
                            .collect();
                        if !alternatives.contains(&swapped) {
                            alternatives.push(swapped);
                        }
                    }
                }
            }
            let mut next = Vec::new();
            for prefix in &variants {
                for alternative in &alternatives {
                    let mut variant = prefix.clone();
                    variant.push(alternative.clone());
                    next.push(variant);
                }
            }
            variants = next;
            if variants.len() > FUZZY_COMBINATIONS {
                variants.truncate(FUZZY_COMBINATIONS);
            }
        }
        let original = readings.join(" ");
        let mut out: Vec<Candidate> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for variant in variants {
            if variant.as_slice() == readings {
                continue;
            }
            let units: Vec<&str> = variant.iter().map(String::as_str).collect();
            let Ok(lattice) = Lattice::build(&units, &self.lexicon, &self.ctx) else {
                continue;
            };
            let scorer = zh_scorer(&self.ctx, self.zh_lm.as_deref());
            let decoder = zh_decoder(&self.lexicon, &self.ctx, scorer.as_ref());
            let (hypotheses, _) =
                decoder.nbest(&lattice, &Locks::new(), false, self.max_candidates());
            for hypothesis in hypotheses {
                if hypothesis.text == variant.join("") || seen.contains(&hypothesis.text) {
                    continue;
                }
                seen.push(hypothesis.text.clone());
                out.push(Candidate::new(
                    hypothesis.text,
                    original.clone(),
                    hypothesis.cost,
                    CandidateSource::Lexicon,
                ));
            }
        }
        out.sort_by_key(|candidate| candidate.cost);
        out
    }

    fn rebuild_ja(&mut self) {
        self.pending = self.romaji.pending().to_string();
        let units = self.romaji.units().to_vec();
        let reading: String = units.iter().map(Unit::as_str).collect();
        if reading.is_empty() {
            return;
        }
        if self.live.is_some() {
            self.rebuild_ja_live(&units);
            return;
        }
        let Some(decoder) = &self.config.ja else {
            self.base = reading;
            return;
        };
        let ctx = JaContext {
            left: self.committed.clone(),
            right: String::new(),
        };
        let results = decoder.nbest(&reading, &ctx, self.max_candidates());
        let Some(best) = results.first() else {
            self.base = reading;
            return;
        };
        self.base = best.surface.clone();
        self.segments = best.segments.clone();
        self.candidates = results
            .into_iter()
            .map(|result| {
                Candidate::new(
                    result.surface,
                    reading.clone(),
                    result.cost,
                    CandidateSource::Lexicon,
                )
            })
            .collect();
    }

    /// 逐字変換開啟時的日文重建：把讀音送進控制器，依決定畫出結果。
    fn rebuild_ja_live(&mut self, units: &[Unit]) {
        let plan = match &mut self.live {
            Some(live) => live.input(units, &JaContext::default()),
            None => return,
        };
        let reading: String = units.iter().map(Unit::as_str).collect();
        self.apply_live_plan(&plan, units, &reading);
    }

    /// 套用逐字変換的決定。
    ///
    /// 有新結果時整段換成新轉換；只有 draft 時顯示前次結果再加上尚未轉換的
    /// かな，讓畫面不會先退回かな。debounce 等待中的新かな因此仍可看見。
    fn apply_live_plan(&mut self, plan: &LivePlan, units: &[Unit], reading: &str) {
        self.live_plan = Some(plan.clone());
        let mut base = String::new();
        let mut segments = Vec::new();
        let mut display = reading.to_string();
        if let Some(result) = &plan.result {
            base.push_str(&result.surface());
            segments = result.conversion().segments;
            let covered: usize = result
                .clauses
                .iter()
                .map(|clause| clause.reading.chars().count())
                .sum();
            if covered < units.len() {
                let tail: String = units[covered..].iter().map(Unit::as_str).collect();
                let start = base.len();
                base.push_str(&tail);
                segments.push(Segment::new(start, base.len(), Mode::Ja, Script::Kana));
            }
            display = base.clone();
        } else {
            base = reading.to_string();
        }
        self.base = base;
        self.segments = segments;

        let mut candidates = Vec::new();
        if plan.convert {
            if let Some(decoder) = &self.config.ja {
                candidates = decoder
                    .nbest(reading, &JaContext::default(), self.max_candidates())
                    .into_iter()
                    .map(|result| {
                        Candidate::new(
                            result.surface,
                            reading.to_string(),
                            result.cost,
                            CandidateSource::Lexicon,
                        )
                    })
                    .collect();
            }
        }
        // 畫面上顯示的是高亮候選；第一個候選必須與逐字変換的結果一致。
        if candidates
            .first()
            .map(|candidate| candidate.surface.as_str())
            != Some(display.as_str())
        {
            candidates.insert(
                0,
                Candidate::new(display, reading.to_string(), 0, CandidateSource::Lexicon),
            );
        }
        self.candidates = candidates;
    }

    fn rebuild_en(&mut self) {
        self.base = self.raw.clone();
        if self.raw.is_empty() {
            return;
        }
        let limit = self.max_candidates();
        self.candidates.push(Candidate::new(
            self.raw.clone(),
            self.raw.clone(),
            0,
            CandidateSource::Literal,
        ));
        if let Some(full) = symbols::width_variant(&self.raw) {
            if full != self.raw {
                self.candidates.push(Candidate::new(
                    full,
                    self.raw.clone(),
                    1,
                    CandidateSource::Literal,
                ));
            }
        }
        // 前綴補完先排，拼字建議接在後面；打對的詞不會有建議。
        let extra = english_candidates(&self.raw, limit);
        let mut seen: Vec<String> = self
            .candidates
            .iter()
            .map(|candidate| candidate.surface.to_lowercase())
            .collect();
        for candidate in extra {
            let key = candidate.surface.to_lowercase();
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            self.candidates.push(candidate);
        }
        self.candidates.truncate(limit);
    }

    fn rebuild_mixed(&mut self) {
        if self.ascii.is_empty() {
            return;
        }
        self.base = self.mixed_surface();
        let mut at = 0usize;
        self.segments = self
            .ascii
            .pieces()
            .iter()
            .map(|piece| {
                let (text, mode, script) = match piece {
                    AsciiPiece::Zh(syllable) => {
                        (self.decode_syllable(syllable), Mode::Zh, Script::Bopomofo)
                    }
                    AsciiPiece::Literal(text) => (text.clone(), Mode::En, Script::Latin),
                };
                let start = at;
                at += text.len();
                Segment::new(start, at, mode, script)
            })
            .collect();
        if !self.ascii.pending().is_empty() {
            let pending = self.ascii.pending();
            let start = at;
            at += pending.len();
            self.segments
                .push(Segment::new(start, at, Mode::Zh, Script::Bopomofo));
        }
        // 待定按鍵是英文詞時，候選先放原按鍵，再放前綴補完與拼字建議。
        let raw = self.ascii.pending().to_string();
        if raw.is_empty() {
            return;
        }
        let limit = self.max_candidates();
        self.candidates.push(Candidate::new(
            raw.clone(),
            raw.clone(),
            0,
            CandidateSource::Literal,
        ));
        let mut seen = vec![raw.to_lowercase()];
        for candidate in english_candidates(&raw, limit) {
            let key = candidate.surface.to_lowercase();
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            self.candidates.push(candidate);
        }
        self.candidates.truncate(limit);
    }

    /// 已完成的注音音節轉成字面。
    fn decode_syllable(&self, syllable: &Syllable) -> String {
        let reading = syllable.as_str();
        let units = [reading];
        match Lattice::build(&units, &self.lexicon, &self.ctx) {
            Ok(lattice) => {
                let decoder = Decoder::new(&self.lexicon, &self.ctx);
                let (segs, _) = decoder.best(&lattice, &Locks::new(), false);
                decoder.conversion(&lattice, &segs, Mode::Zh).surface
            }
            Err(_) => reading.to_string(),
        }
    }

    /// 混合模式的完整字面：注音片段解碼，英數照字面，待定按鍵照顯示。
    fn mixed_surface(&self) -> String {
        let mut out = String::new();
        let mut run: Vec<String> = Vec::new();
        let flush = |run: &mut Vec<String>, out: &mut String| {
            if run.is_empty() {
                return;
            }
            let units: Vec<&str> = run.iter().map(String::as_str).collect();
            match Lattice::build(&units, &self.lexicon, &self.ctx) {
                Ok(lattice) => {
                    let decoder = Decoder::new(&self.lexicon, &self.ctx);
                    let (segs, _) = decoder.best(&lattice, &Locks::new(), false);
                    out.push_str(&decoder.conversion(&lattice, &segs, Mode::Zh).surface);
                }
                Err(_) => {
                    for unit in run.iter() {
                        out.push_str(unit);
                    }
                }
            }
            run.clear();
        };
        for piece in self.ascii.pieces() {
            match piece {
                AsciiPiece::Zh(syllable) => run.push(syllable.as_str().to_string()),
                AsciiPiece::Literal(text) => {
                    flush(&mut run, &mut out);
                    out.push_str(text);
                }
            }
        }
        flush(&mut run, &mut out);
        out.push_str(&self.ascii.display_pending());
        out
    }

    /// 把解碼片段轉成輸出字串上的位元組範圍。
    fn segments_of(segs: &[crate::decode::Seg], mode: Mode, script: Script) -> Vec<Segment> {
        let mut at = 0usize;
        segs.iter()
            .map(|seg| {
                let start = at;
                at += seg.word.len();
                Segment::new(start, at, mode, script)
            })
            .collect()
    }

    /// 把一整串讀音直接轉成最佳字面；給空白鍵提交與外部驅動使用。
    fn convert_reading(&self, reading: &str) -> Option<String> {
        let syllables = crate::zh::syllable::parse_reading(reading);
        if syllables.is_empty() {
            return None;
        }
        let units: Vec<&str> = syllables.iter().map(|s| s.as_str()).collect();
        let lattice = Lattice::build(&units, &self.lexicon, &self.ctx).ok()?;
        let scorer = zh_scorer(&self.ctx, self.zh_lm.as_deref());
        let decoder = zh_decoder(&self.lexicon, &self.ctx, scorer.as_ref());
        let (segs, _) = decoder.best(&lattice, &Locks::new(), false);
        Some(decoder.conversion(&lattice, &segs, Mode::Zh).surface)
    }
}

/// 英文片段的候選：前綴補完先排，拼字建議接在後面。
///
/// 對照前身 `core/session/english_autocomplete.go` 的
/// `refreshEnglishCandidates` 與 `core/session/mixed.go` 的
/// `openMixedCandidates`：最多保留 3 個拼字建議，剩下的名額才給前綴補完，
/// 這樣補完不會把拼字建議擠掉。拼字建議直接用已收進來的
/// [`crate::mixed::Dictionary::corrections`]，沒有另寫演算法；打對的完整單字
/// 不會有建議，建議保留查詢時的大小寫。
fn english_candidates(raw: &str, limit: usize) -> Vec<Candidate> {
    if limit == 0 || raw.chars().count() < 2 || !raw.chars().all(|c| c.is_ascii_alphabetic()) {
        return Vec::new();
    }
    let english = crate::mixed::default_english();
    let mut seen = vec![raw.to_lowercase()];
    let mut corrections: Vec<Candidate> = Vec::new();
    for candidate in english.spelling_candidates(raw, limit.saturating_add(3)) {
        let key = candidate.surface.to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        corrections.push(candidate);
        if corrections.len() == 3 {
            break;
        }
    }
    let prefix_limit = limit.saturating_sub(corrections.len());
    let mut out: Vec<Candidate> = Vec::new();
    for candidate in english.completion_candidates(raw, limit.saturating_add(corrections.len())) {
        if out.len() == prefix_limit {
            break;
        }
        let key = candidate.surface.to_lowercase();
        if seen.contains(&key) || candidate.surface.chars().count() <= raw.chars().count() {
            continue;
        }
        seen.push(key);
        out.push(candidate);
    }
    out.extend(corrections);
    out.truncate(limit);
    out
}

/// 有 bigram 時建立內插評分器；`lm.bin` 沒有 bigram 時回傳 `None`，
/// 解碼完全走原本的 unigram 成本。
fn zh_scorer<'a>(
    ctx: &'a ScoreContext,
    lm: Option<&'a crate::score::ZhLm>,
) -> Option<crate::score::LmScorer<'a>> {
    lm.filter(|lm| lm.bigrams() > 0)
        .map(|lm| crate::score::LmScorer::new(ctx, lm))
}

/// 建立注音解碼器；有 bigram 時接上評分器並套用 beam 上限。
fn zh_decoder<'a>(
    lex: &'a InMemoryLexicon,
    ctx: &'a ScoreContext,
    scorer: Option<&'a crate::score::LmScorer<'a>>,
) -> Decoder<'a> {
    let mut decoder = match scorer {
        Some(scorer) => Decoder::new(lex, ctx).with_scorer(scorer),
        None => Decoder::new(lex, ctx),
    };
    if scorer.is_some() {
        decoder.beam = crate::decode::DEFAULT_BEAM;
    }
    decoder
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
