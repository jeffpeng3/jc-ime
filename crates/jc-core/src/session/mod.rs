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

mod backends;
mod composition;
mod core;
mod input_style;
pub mod misc;
mod mixed;
pub mod neural;
mod segments;
pub mod symbols;

pub use input_style::{
    full_width_idle_keys, halfwidth_shift_keys, idle_keys, zhuyin_key_action, InputStyle, KeyAction,
};
pub use neural::{
    ContextChars, NeuralConfig, NeuralDecision, NeuralEngine, NeuralError, NeuralInput,
    NeuralModel, NeuralRequest, NeuralRuntime, NeuralStatsSnapshot, RejectReason, Validation,
    DEFAULT_GAP_THRESHOLD, DEFAULT_LEFT_CONTEXT_CHARS, DEFAULT_MIN_MEAN_CANDIDATES,
    DEFAULT_RIGHT_CONTEXT_CHARS, DEFAULT_TIMEOUT_MS, QUALITY_GAP_THRESHOLD,
};
pub use segments::tone_mark_keys;

use crate::candidate::{Candidate, CandidateSource};
use crate::composition::Composition;
use crate::decode::{Decoder, Hypothesis, Locks};
use crate::emoji::{self, Stat};
use crate::ja::decode::{JaClause, JaContext, JaDecoder, JaResult};
use crate::ja::input::RomajiInput;
use crate::ja::live::{self, Controller as LiveController, Plan as LivePlan};
use crate::ja::personal as ja_personal;
use crate::key::{Key, KeyCode};
use crate::lang::{Lang, Mode};
use crate::lattice::{InMemoryLexicon, Lattice};
use crate::mixed::{default_english, space_text, English, Spacing};
use crate::output::{rune_offset, utf16_offset, Snapshot};
use crate::personal::PersonalStore;
use crate::punct::{self, Edit};
use crate::reading::Unit;
use crate::score::ScoreContext;
use crate::segment::{Script, Segment};
use crate::zh::han::is_han;
use crate::zh::input::ZhInput;
use crate::zh::syllable::Syllable;
use mixed::AsciiInput;

/// 這個 crate 的語意版本，供協定握手與診斷使用。
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 預設候選數：兩頁，讓第二頁的數字選字有實際用途。
pub const DEFAULT_MAX_CANDIDATES: usize = PAGE_SIZE * 2;

/// 候選視窗一頁幾項；核心唯一的頁面大小，固定 9。
///
/// 由 [`crate::output::CANDIDATE_PAGE_SIZE`] 轉出，避免 `session` 內外各留
/// 一份常數。
pub use crate::output::CANDIDATE_PAGE_SIZE as PAGE_SIZE;

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
    /// 混合模式但只走羅馬字：原本的手動日文入口，現在不再切換輸入模式。
    pub ja_only: bool,
    /// 一次轉換保留幾個候選。
    pub max_candidates: usize,
    /// 注音詞典；`None` 時注音只做原樣輸出。
    pub zh_lexicon: Option<Arc<InMemoryLexicon>>,
    /// 注音成本設定；`None` 用 [`ScoreContext::default`]。
    pub zh_ctx: Option<ScoreContext>,
    /// 注音 bigram 語言模型（`lm.bin`）；`None` 或沒有 bigram 時只用 unigram。
    pub zh_lm: Option<Arc<crate::score::ZhLm>>,
    /// 英文詞表；`None` 時使用行程共用的 [`crate::mixed::default_english`]。
    ///
    /// 引擎從自己的資料根目錄載入 `english/` 後放進來，安裝版因此不必依賴
    /// `JC_IMEDATA` 或 `JC_ENGLISH_PACK`。`jc-cli` 仍可直接用全域詞表。
    pub english: Option<Arc<English>>,
    /// 日文解碼器；`None` 時日文只做原樣輸出。
    pub ja: Option<Arc<JaDecoder>>,
    /// 是否呼叫日文解碼器上的重排器；預設關閉，實測 modern-500 全量重排
    /// 由 86.0% 退到 82.0%，因此不再預設啟用。解碼器上的詞群門檻
    /// （[`crate::ja::decode::DEFAULT_WORD_RERANK_GATE`]）預設只送統計
    /// 不確定的詞群，但這裡仍保守關閉。
    pub ja_rerank: bool,
    /// 日文神經轉換（Jinen）設定；預設開啟，但沒有注入模型時完全不呼叫。
    ///
    /// 統計結果永遠先算好；只有成本差小於
    /// [`NeuralConfig::gap_threshold`] 且平均候選數大於
    /// [`NeuralConfig::min_mean_candidates`] 時才問模型，通過五條驗證才替換。
    /// `--neural off` 把 [`NeuralConfig::enabled`] 設成 `false`，回到舊路徑。
    pub ja_neural: NeuralConfig,
    /// 日文逐字変換設定；預設關閉，關閉時日文走一般変換流程。
    pub ja_live: live::Config,
    /// 聲調選填：沒有聲調的音節展開成五個聲調再解碼。
    pub toneless: bool,
    /// 打錯注音時擋聲調：沒有字的讀音按聲調時不收，留在暫存等使用者改。
    ///
    /// 對照上游 `ZhuyinErrorHint`。預設關閉，讓沒有詞典的情境仍能原樣留下
    /// 注音；由外殼依設定開啟。
    pub zhuyin_error_hint: bool,
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
    /// 個人化學習資料；預設為空。搭配 [`SessionConfig::learning`] 生效。
    pub personal: PersonalStore,
    /// 個人化學習是否啟用；預設啟用，關閉時只讀不寫。
    pub learning: bool,
    /// 最短觸發長度：讀音短於這個字元數時，只記錄證據，不改變候選排序。
    ///
    /// 對照 Mozkey 的 feedback 最短自動套用長度（7 字元），預設
    /// [`crate::personal::MIN_TRIGGER_LEN`]。被動接受的提交受這個門檻限制；
    /// 明確手選是使用者意圖，不受限制。注音的 `eval zh-golden` 學習案例都是
    /// 手選，因此門檻不影響驗收。
    pub min_trigger_len: usize,
    /// 文件層文脈是否可用；預設開啟。
    ///
    /// 目前只影響 [`Session::document_left`] 與 [`Session::committed_tail`]
    /// 的查詢；解碼評分尚未混入文件前文。
    pub document_context: bool,
    /// 輸入風格與鍵位；預設零規則，沿用既有鍵位。
    pub style: InputStyle,
    /// 日文片段假名切換鍵；預設 F7。
    pub ja_toggle_key: KeyCode,
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
            mode: Mode::Mixed,
            ja_only: false,
            max_candidates: DEFAULT_MAX_CANDIDATES,
            zh_lexicon: None,
            zh_ctx: None,
            zh_lm: None,
            english: None,
            ja: None,
            ja_rerank: false,
            ja_neural: NeuralConfig::default(),
            ja_live: live::Config::default(),
            toneless: false,
            zhuyin_error_hint: false,
            ctrl_punct: false,
            fuzzy_pairs: DEFAULT_FUZZY_PAIRS.to_vec(),
            rerank: None,
            punct_mark: None,
            english_autocomplete: false,
            cjk_latin_space: false,
            simplified: None,
            personal: PersonalStore::new(),
            learning: true,
            min_trigger_len: crate::personal::MIN_TRIGGER_LEN,
            document_context: true,
            style: InputStyle::default(),
            ja_toggle_key: KeyCode::F(7),
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

/// 正在編輯的片段視窗。
///
/// `start`、`len` 都是音節索引；候選清單只解碼這一段，選字也只鎖定這一段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct EditWindow {
    /// 視窗起點（音節索引）。
    pub(super) start: usize,
    /// 視窗長度（音節數）。
    pub(super) len: usize,
}

/// 一次輸入的工作階段。
///
/// 只保留組字所需的狀態；送出後立即清空，因此呼叫端每次都用快照重畫。
pub struct Session {
    pub(super) config: SessionConfig,
    pub(super) zh: ZhInput,
    /// 已完成的注音音節；組字游標與片段編輯都以這份清單為準。
    ///
    /// `ZhInput` 只負責尚未完成的注音；音節打完就搬進這裡，才能在中間插入
    /// 與刪除。對照上游 `s.syls`。
    pub(super) syllables: Vec<Syllable>,
    /// 組字游標位置（音節格，`0..=syllables.len()`）。
    ///
    /// 對外一律換算成 rune 位移；內部用音節格才能對應讀音與鎖定。
    pub(super) cursor: usize,
    /// 日文逐字変換的組字游標（顯示字串的位元組位置）。
    ///
    /// 注音路徑用音節格 [`Session::cursor`]；日文沒有音節格，另外記在
    /// 顯示字串上，左右鍵與片段候選共用同一個位置。
    pub(super) ja_cursor: usize,
    /// 混合模式的組字游標：片段邊界索引，`0..=segments.len()`。
    ///
    /// [`Self::MIXED_CURSOR_END`] 代表結尾；打字時一律回到結尾，方向鍵與刪除
    /// 才會把游標停在兩個片段之間。混合模式沒有音節格，游標以顯示片段為單位。
    pub(super) mixed_cursor: usize,
    /// 游標所在片段的編輯視窗；有值時候選清單只算這一段。
    pub(super) edit: Option<EditWindow>,
    /// 換字後是否保留候選窗，直到 Esc 或游標離開目前片段。
    pub(super) keep_candidates: bool,
    pub(super) romaji: RomajiInput,
    pub(super) lexicon: Arc<InMemoryLexicon>,
    /// 不含自訂詞的原始詞典；自訂詞變動時由它重建 [`Session::lexicon`]。
    pub(super) base_lexicon: Arc<InMemoryLexicon>,
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
    pub(super) candidates: Vec<Candidate>,
    pub(super) highlighted: usize,
    /// 候選視窗是否由 Tab／方向鍵開啟；數字鍵只在開啟時選字。
    pub(super) candidate_window: bool,
    /// 使用者是否已進入明確的候選選取模式。
    ///
    /// 對照上游 `navigating`：候選窗顯示中不代表要選字；方向鍵、翻頁鍵或
    /// 上下鍵進入後才為 `true`。打字、選字、關窗與送出都會離開這個模式。
    pub(super) navigating: bool,
    pub(super) segments: Vec<Segment>,
    /// 語言無關的組字模型；`Auto`／`User` 所有權的唯一來源。
    ///
    /// 注音與日文都只透過 [`Composition`] 判斷哪一段被使用者固定；語言後端
    /// 的原生表示由 [`Session::sync_zh_locks`] 與 [`Session::sync_ja_users`]
    /// 推導，不再各自維護鎖定。
    pub(super) composition: Composition,
    /// 注音解碼器的片段鎖定；由 [`Session::composition`] 的 `User` 標記推導。
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
    /// 這次組字是否走純注音路徑。
    ///
    /// 混打從第一個注音鍵開始先走注音狀態機；遇到打不成注音的英數按鍵時
    /// 整段交回混打管線（`ascii`）。
    pub(super) zh_lane: bool,
    /// 注音路徑目前尚未完成音節的原始按鍵。
    ///
    /// 由注音轉混打時要重播整段按鍵；已完成的音節用
    /// [`crate::zh::keyboard::Keyboard::keys`] 還原按鍵，這一份只記最後一個
    /// 還沒完成的音節。
    pub(super) zh_pending_keys: String,
    /// Mixed 的純注音候選窗是否只列最後一個音節。
    ///
    /// 按 ↓／空白開窗時成立；候選只顯示游標那一格，但 Enter 仍送出整段。
    pub(super) zh_window_last: bool,
    /// Mixed 純注音組字中按過空白的位置（音節索引）。
    ///
    /// 空白在混打管線裡切開拉丁 run；顯示片段合併時不能跨過它。
    pub(super) zh_run_breaks: Vec<usize>,
    /// 混合模式依按鍵推斷出來的日文片段。
    ///
    /// 有值時 [`Session::config`] 的 `mode` 暫時是 `Mixed` 但只走羅馬字（`ja_only`），送出或取消後
    /// 回到原本的模式（對照 `ja_mixed.go` 的 `japaneseRun`）。
    pub(super) ja_run: bool,
    /// 日文推斷從混合模式接手時吃掉的原始羅馬字按鍵。
    ///
    /// 推斷結束要交回混打時，用這串按鍵重新走統一管線，不能直接送出已顯示
    /// 的かな：`rishiree` 會顯示成 `りしれえ`，但批次規劃會把 `e` 留給後面的
    /// 注音（`rishire`＋`ej/`）。
    pub(super) ja_run_keys: String,
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
    /// 目前按鍵是否來自私密欄位；每個按鍵明確覆寫，commit／cancel 後歸零。
    pub(super) private: bool,
    /// 輸入風格與鍵位；由設定複製進來。
    pub(super) style: InputStyle,
    /// 是否為全形（Shift＋空白切換）；風格狀態，不存檔。
    pub(super) full_width: bool,
    /// 微軟風格的反引號前導是否等待下一個字元。
    pub(super) symbol_prefix: bool,
    /// 執行期是否把送出的文字轉成簡體。
    ///
    /// 與 [`SessionConfig::simplified`] 分開：轉換器是資料依賴，這個旗標才是
    /// 使用者的即時開關。沒有轉換器時一律保持 `false`。
    pub(super) output_simplified: bool,
}

impl Default for Session {
    fn default() -> Self {
        Self::new(SessionConfig::default())
    }
}

impl Session {
    /// 混合模式游標代表「在整段結尾」的哨兵值。
    pub(super) const MIXED_CURSOR_END: usize = usize::MAX;

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
        // 共用同一份詞典：`config` 的 `Arc` 由引擎的設定工廠提供，多個
        // session（多個程式）與 `lexicon`／`base_lexicon` 都指向同一塊記憶體。
        let base_lexicon = config
            .zh_lexicon
            .clone()
            .unwrap_or_else(|| Arc::new(InMemoryLexicon::new()));
        let personal = config.personal.clone();
        let lexicon = personal.augment_lexicon(&base_lexicon);
        let zh_lm = config.zh_lm.clone();
        let style = config.style.clone();
        let english = config.english.clone();
        // 逐字変換控制器與日文解碼器共用同一份詞典；關閉時完全不建立。
        //
        // 神經轉換開啟且模型已載入時，逐字路徑才拿到採用器；關閉或模型
        // 缺席時 `ja_live` 完全走原本的統計路徑。
        let live = if config.ja_live.enabled {
            config.ja.as_ref().map(|decoder| {
                let mut live_config = config.ja_live.clone();
                live_config.neural = neural::live_gate(&config.ja_neural);
                LiveController::new(decoder.as_ref().clone(), live_config)
            })
        } else {
            None
        };
        let mut session = Self {
            config,
            zh: ZhInput::standard(),
            syllables: Vec::new(),
            cursor: 0,
            ja_cursor: 0,
            mixed_cursor: Self::MIXED_CURSOR_END,
            edit: None,
            keep_candidates: false,
            romaji: RomajiInput::new(),
            lexicon,
            base_lexicon,
            personal,
            ctx,
            zh_lm,
            literal: String::new(),
            literal_anchor: 0,
            literal_offset: 0,
            candidates: Vec::new(),
            highlighted: 0,
            candidate_window: false,
            navigating: false,
            segments: Vec::new(),
            composition: Composition::new(),
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
            ascii: AsciiInput::new().with_english(english),
            zh_lane: false,
            zh_pending_keys: String::new(),
            zh_window_last: false,
            zh_run_breaks: Vec::new(),
            ja_run: false,
            ja_run_keys: String::new(),
            menu: None,
            palette_tab: 0,
            symbol_recent: Vec::new(),
            symbol_stats: HashMap::new(),
            emoji_recent: Vec::new(),
            emoji_stats: HashMap::new(),
            live,
            live_plan: None,
            learn_events: 0,
            private: false,
            style,
            full_width: false,
            symbol_prefix: false,
            output_simplified: false,
        };
        session.install_tone_gate();
        session
    }

    /// 目前模式。
    pub fn mode(&self) -> Mode {
        self.config.mode
    }

    /// 這個工作階段使用的英文詞表。
    ///
    /// 設定沒有指定時退回行程共用的 [`crate::mixed::default_english`]，
    /// 因此既有呼叫端不必改。
    pub fn english(&self) -> &English {
        match &self.config.english {
            Some(english) => english,
            None => default_english(),
        }
    }

    /// 目前英文詞表的詞數；health 用來確認載入的是資料包而不是原型。
    pub fn english_words(&self) -> usize {
        self.english().dictionary().len()
    }

    /// 建立與這個工作階段相同英文詞表的 [`AsciiInput`]。
    fn ascii_input(&self) -> AsciiInput {
        AsciiInput::new().with_english(self.config.english.clone())
    }

    /// 目前組字是否走注音路徑。
    ///
    /// 從純注音按鍵開始的這次組字走注音路徑，才會有一致的候選、游標、
    /// 鎖定與聲調閘門。混打片段仍在 [`AsciiInput`] 管線裡。
    pub(super) fn in_zh_lane(&self) -> bool {
        self.zh_lane
    }

    /// 目前組字是否走日文羅馬字路徑。
    ///
    /// `ja_run` 是混打自動推斷的日文片段；`ja_only` 是「Mixed 但只走羅馬字」
    /// 的手動入口。兩者共用同一條日文轉換路徑，不再切換輸入模式。
    pub(super) fn in_ja_lane(&self) -> bool {
        self.ja_run || self.config.ja_only
    }

    /// 目前是否把送出的文字轉成簡體。
    pub fn output_simplified(&self) -> bool {
        self.output_simplified
    }

    /// 設定是否把送出的文字轉成簡體。
    ///
    /// 沒有載入轉換器時保持關閉；這個狀態只影響 [`Session::commit`]，組字中的
    /// 預覽仍是繁體。
    pub fn set_output_simplified(&mut self, on: bool) {
        self.output_simplified = on && self.config.simplified.is_some();
    }

    /// 目前的個人化學習資料；宿主用它存檔。
    ///
    /// [`Session::new`] 會複製 [`SessionConfig::personal`]，因此學習結果只在
    /// 這份工作階段內；要持久化時由宿主在適當時間取出並寫檔。
    pub fn personal(&self) -> &PersonalStore {
        &self.personal
    }

    /// 換掉這個工作階段的個人化資料，並重建個人詞典。
    ///
    /// engine 在開 session 時用它把全行程共用的一份學習資料灌進新工作階段。
    /// 重建詞典是必要的，因為 [`Session::base_lexicon`] 之上還疊了個人片語與
    /// 自訂詞，只換 `personal` 會讓這些詞在畫面上消失。組字中的其他狀態不動。
    pub fn set_personal_store(&mut self, store: PersonalStore) {
        self.lexicon = store.augment_lexicon(&self.base_lexicon);
        self.config.personal = store.clone();
        self.personal = store;
    }

    /// 目前高亮的候選索引。
    pub fn highlighted(&self) -> usize {
        self.highlighted
    }

    /// 目前候選頁碼（0 起算）。
    ///
    /// 頁碼不另存，一律由 `highlighted / page_size` 推導。這樣翻頁、方向鍵與
    /// 直接指定高亮都只有一份頁邊界。
    pub fn current_page(&self) -> usize {
        self.highlighted / self.page_size().max(1)
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
    /// `Escape` 先關候選窗、再清未完成的注音，最後才清整段組字。上下鍵、
    /// 翻頁鍵、Home／End 在候選清單中移動，左右方向鍵與 Tab 也在候選間移動。
    pub fn press(&mut self, key: Key) -> Snapshot {
        self.expire_segment_view();
        // 私密狀態跟著每個按鍵走：true 設起來，false 明確清掉，不跨欄位殘留。
        self.private = key.private;
        // 私密欄位裡的反悔鍵不該動到前一個一般欄位的學習。
        if !self.private {
            self.resolve_pending_learning(&key);
        }
        if key.ctrl {
            self.pending_tone = None;
            self.press_ctrl(key);
        } else if !self.style_key(&key) && !self.menu_key(&key) {
            if !self.segments_press(&key) {
                match key.code {
                    KeyCode::Enter => self.finish_composition(),
                    KeyCode::Escape => self.escape(),
                    KeyCode::Delete if key.shift && self.candidate_window => {
                        self.forget_candidate();
                    }
                    KeyCode::Backspace if key.shift && self.candidate_window => {
                        self.forget_candidate();
                    }
                    KeyCode::Delete => self.delete_forward(),
                    KeyCode::Backspace => self.backspace(),
                    KeyCode::F(n) if KeyCode::F(n) == self.config.ja_toggle_key => {
                        self.ja_toggle_script();
                    }
                    KeyCode::F(6) => {
                        self.ja_transform(live::KanaTransform::Hiragana);
                    }
                    KeyCode::F(8) => {
                        self.ja_transform(live::KanaTransform::HalfwidthKana);
                    }
                    KeyCode::F(9) => {
                        self.ja_transform(live::KanaTransform::FullwidthLatin);
                    }
                    KeyCode::F(10) => {
                        self.ja_transform(live::KanaTransform::HalfwidthLatin);
                    }
                    KeyCode::Space => self.space(),
                    KeyCode::Down => self.down(),
                    KeyCode::Up => self.up(),
                    KeyCode::PageDown => self.page_step(1),
                    KeyCode::PageUp => self.page_step(-1),
                    KeyCode::Home => self.candidate_edge(false),
                    KeyCode::End => self.candidate_edge(true),
                    KeyCode::Tab => {
                        if self.navigating && !self.candidates.is_empty() {
                            // 選取模式中的 Tab 換下一個候選；候選選取優先於補全。
                            self.move_highlight(1);
                        } else {
                            let accepted = self.config.mode == Mode::Mixed
                                && !self.in_zh_lane()
                                && !self.in_ja_lane()
                                && self.config.english_autocomplete
                                && self.ascii.accept_suggestion();
                            if !accepted && !self.accept_suggestion() {
                                self.move_highlight(1);
                                if self.config.mode == Mode::Mixed {
                                    self.navigating = true;
                                }
                            }
                        }
                    }
                    KeyCode::ArrowRight if key.shift => self.resize_segment(1),
                    KeyCode::ArrowLeft if key.shift => self.resize_segment(-1),
                    KeyCode::ArrowRight => self.navigate(1),
                    KeyCode::ArrowLeft => self.navigate(-1),
                    KeyCode::Char(ch) => self.press_char(ch),
                    // 具名鍵與 Shift+Tab 沒有對應的組字動作時由組字區吃掉，
                    // 不讓應用程式在組字中移動焦點。
                    _ => {}
                }
            }
        } else {
            self.pending_tone = None;
        }
        // Esc 是取消：處理完就離開私密狀態，下一次按鍵再明確帶值。
        if key.code == KeyCode::Escape {
            self.private = false;
        }
        self.rebuild();
        self.snapshot()
    }

    /// 切換到指定模式；清空目前組字，並丟掉先前記錄的 Shift 切換來源。
    ///
    /// 單按 Shift 的英文切換由 [`Session::toggle_english`] 進入。
    pub fn set_mode(&mut self, mode: Mode) -> Snapshot {
        self.clear_composition();
        self.config.mode = mode;
        self.rebuild();
        self.snapshot()
    }

    /// 中英切換。
    ///
    /// 英文由外殼直接放行按鍵，不進引擎；引擎只回目前快照，不改變組字或
    /// 記住前一個模式。外殼自己保存中英模式並在重連後還原。
    pub fn toggle_english(&mut self) -> Snapshot {
        self.snapshot()
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

    /// 翻到候選的第 `page` 頁；頁面大小固定 [`PAGE_SIZE`]。
    ///
    /// 目前候選清單保留全部候選，翻頁只是把高亮移到該頁第一項；頁碼超出
    /// 範圍時不改變狀態。外殼不需要呼叫這個方法，`PageUp`／`PageDown` 會走
    /// 同一套頁邊界。
    pub fn page(&mut self, page: usize) -> Snapshot {
        let index = page.saturating_mul(self.page_size());
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
        // 取走這個欄位最後一個按鍵留下的私密狀態；提交後歸零，不跨欄位殘留。
        let private = self.private;
        self.private = false;
        if let Some(text) = self.pending_commit.take() {
            let text = self.output_text(text);
            self.clear_composition();
            self.remember_committed(&text);
            return text;
        }
        if self.config.mode == Mode::Mixed && self.in_zh_lane() && self.zh_window_last {
            self.zh_window_last = false;
            self.rebuild();
        }
        if !private {
            self.remember_selection();
        }
        if self.config.mode == Mode::Mixed && !self.in_zh_lane() && !self.in_ja_lane() {
            self.ascii.flush();
        }
        let text = self.output_text(self.finalize(self.current_text()));
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
        self.private = false;
        self.clear_composition();
    }

    /// 日文神經轉換是否開啟（模型是否載入不影響這個值）。
    pub fn ja_neural_enabled(&self) -> bool {
        self.config.ja_neural.enabled
    }

    /// 日文神經轉換的累計統計；模型未載入時為 `None`。
    pub fn ja_neural_stats(&self) -> Option<NeuralStatsSnapshot> {
        self.config
            .ja_neural
            .runtime
            .as_ref()
            .map(|runtime| runtime.stats())
    }

    /// 日文逐字変換是否開啟。
    /// 對游標所在的日文片段套用假名或英數轉換。
    ///
    /// 純字串轉換，不呼叫統計解碼或模型；其他片段與讀音都不變。外殼把
    /// `F6`／`F7`／`F8`／`F9`／`F10` 對到 [`live::KanaTransform`] 即可。
    pub fn ja_transform(&mut self, transform: live::KanaTransform) -> Snapshot {
        if let Some((index, _)) = self.ja_active_span() {
            let changed = self
                .live
                .as_mut()
                .is_some_and(|live| live.transform_clause(index, transform));
            if changed {
                if let Some(live) = self.live.as_mut() {
                    if !live.sentence_mode() {
                        live.lock_through(index);
                    }
                }
                self.capture_ja_users();
                self.finish_ja_script_edit(index);
            }
        }
        self.snapshot()
    }

    /// 切換游標所在片段的平假名／全角片假名。
    ///
    /// 漢字片段先還原成讀音；切換後鎖定該片段，後續輸入不會改回去。
    pub fn ja_toggle_script(&mut self) -> Snapshot {
        if let Some((index, span)) = self.ja_active_span() {
            let changed = self
                .live
                .as_mut()
                .is_some_and(|live| live.toggle_script(span).is_some());
            if changed {
                if let Some(live) = self.live.as_mut() {
                    if !live.sentence_mode() {
                        live.lock_through(index);
                    }
                }
                self.capture_ja_users();
                self.finish_ja_script_edit(index);
            }
        }
        self.snapshot()
    }

    /// 把游標所在片段還原成讀音，並搬回未鎖定的尾巴。
    ///
    /// 後續按鍵會從這個讀音重新切分與轉換；已鎖定的前綴保持不變。
    pub fn ja_unconvert(&mut self) -> Snapshot {
        if let Some((index, span)) = self.ja_active_span() {
            let changed = self
                .live
                .as_mut()
                .is_some_and(|live| live.unconvert(span).is_some());
            if changed {
                self.capture_ja_users();
                self.refresh_ja_live();
                let end = self
                    .segments()
                    .get(index)
                    .map(|segment| segment.end)
                    .unwrap_or_else(|| self.base.len());
                self.ja_cursor = end.min(self.base.len());
            }
        }
        self.snapshot()
    }

    /// 游標所在日文片段的索引與讀音範圍。
    fn ja_active_span(&self) -> Option<(usize, crate::ja::decode::JaSpan)> {
        if !self.in_ja_lane() {
            return None;
        }
        let index = self.active_segment()?;
        let span = self.live.as_ref()?.clause_spans().get(index).cloned()?;
        // `JaSpan` 的第二個參數是長度，不是結束位置。
        Some((
            index,
            crate::ja::decode::JaSpan::new(span.start, span.end - span.start),
        ))
    }

    /// 片段選字或變換後重畫，游標停在該片段結尾。
    fn finish_ja_script_edit(&mut self, index: usize) {
        self.refresh_ja_live();
        let end = self
            .segments()
            .get(index)
            .map(|segment| segment.end)
            .unwrap_or_else(|| self.base.len());
        self.ja_cursor = end.min(self.base.len());
    }

    pub fn ja_live_enabled(&self) -> bool {
        self.live.as_ref().is_some_and(LiveController::enabled)
    }

    /// debounce 是否還在等待；外殼據此安排計時器。
    pub fn ja_live_pending(&self) -> bool {
        self.live
            .as_ref()
            .is_some_and(|live| live.pending() || live.idle_pending())
    }

    /// debounce 到期時間；沒有待處理轉換時為 `None`。
    pub fn ja_live_deadline(&self) -> Option<std::time::Instant> {
        let idle = self.live.as_ref().and_then(LiveController::idle_deadline);
        let debounce = self.live_plan.as_ref().and_then(|plan| plan.deadline);
        match (debounce, idle) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }

    /// 立即觸發日文停頓轉換；外殼計時器與測試共用。
    pub fn ja_live_idle(&mut self) -> Snapshot {
        let plan = match &mut self.live {
            Some(live) => {
                let (plan, changed) = live.idle();
                changed.then_some(plan)
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

    /// 輸入結束時把日文尚未轉換的讀音補齊。
    ///
    /// 只對非延遲整句的逐字変換有效；延遲整句模式請用
    /// [`Session::ja_live_idle`]。回傳轉換後的快照。
    pub fn ja_live_flush(&mut self) -> Snapshot {
        let plan = match self.live.as_mut() {
            Some(live) => {
                let (plan, converted) = live.flush();
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
        let preedit = self.base_with_highlight();
        let byte = self.cursor_byte();
        let segments = self.segments();
        let owners = self.segment_owners(&preedit, &segments);
        Snapshot {
            cursor_utf16: utf16_offset(&preedit, byte),
            cursor_rune: rune_offset(&preedit, byte),
            preedit,
            segments,
            owners,
            candidates: self.candidates.clone(),
            highlighted: self.highlighted,
            suggestion: self.snapshot_suggestion(),
            label: self.highlighted_label().map(str::to_string),
            navigating: self.navigating,
        }
    }

    /// 游標位置，以 UTF-16 code unit 計算。
    pub fn cursor_utf16(&self) -> usize {
        utf16_offset(&self.base_with_highlight(), self.cursor_byte())
    }

    /// 反悔最近一次分語言學習；回傳是否有可反悔的紀錄。
    ///
    /// 只還原共用 store 記下的語言學習（日文與後續接上的語言），不影響注音的
    /// 階層式計數。重建中組字內容後，排序會回到學習前的狀態。
    pub fn undo_learning(&mut self) -> bool {
        self.personal.accept_learning();
        let undone = self.personal.undo_language_note();
        if undone {
            self.rebuild();
        }
        undone
    }

    /// 撤銷與某個詞有關的學習；供 `forget` 協定請求使用。
    pub fn forget_word(&mut self, word: &str) -> bool {
        self.personal.accept_learning();
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
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
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

    fn type_syllable_private(session: &mut Session, reading: &str, private: bool) {
        let keyboard = crate::zh::keyboard::Keyboard::standard();
        for syllable in crate::zh::syllable::parse_reading(reading) {
            let keys = keyboard.keys(&syllable).expect("音節應該有對應按鍵");
            for ch in keys.chars() {
                let mut key = Key::char(ch, false);
                key.private = private;
                session.press(key);
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
    fn test_private_keys_skip_learning_and_clear_after_commit() {
        let mut s = session();
        let before = s.personal().clone();

        type_syllable_private(&mut s, "ㄈㄥ", true);
        type_syllable_private(&mut s, "ㄐㄧㄥˇ", true);
        let mut down = Key::new(KeyCode::Down, None, false);
        down.private = true;
        s.press(down);
        let mut enter = Key::new(KeyCode::Enter, None, false);
        enter.private = true;
        s.press(enter);
        assert!(s.private, "提交前仍要記得這是私密欄位");
        assert_eq!(s.commit(), "風景");
        assert!(!s.private, "commit 後 private 必須歸零");
        assert_eq!(s.personal(), &before, "私密欄位不得寫入個人學習");

        // 下一個一般欄位的按鍵明確帶 false，要能正常學習。
        type_syllable(&mut s, "ㄈㄥ");
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        s.press(Key::new(KeyCode::Down, None, false));
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "風景");
        assert!(s.personal().is_learned("風景"));
        assert_ne!(s.personal(), &before);
    }

    #[test]
    fn test_private_escape_cancels_state_without_learning() {
        let mut s = session();
        let before = s.personal().clone();
        type_syllable_private(&mut s, "ㄐㄧㄥˇ", true);
        let mut escape = Key::new(KeyCode::Escape, None, false);
        escape.private = true;
        s.press(escape);
        assert!(!s.private, "cancel 後 private 必須歸零");
        assert_eq!(s.personal(), &before);
    }

    /// rollback window：提交後立刻退格，這次學習要補償回提交前。
    #[test]
    fn test_commit_then_backspace_reverts_learning() {
        let mut s = session();
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        // 開候選窗再送出手選，確保有明確學習紀錄。
        s.press(Key::new(KeyCode::Down, None, false));
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "景");
        assert!(s.personal().is_learned("景"));
        assert!(s.personal().has_pending_learning());

        s.press(Key::new(KeyCode::Backspace, None, false));
        assert!(!s.personal().is_learned("景"), "窗口內退格要撤銷學習");
        assert!(s.personal().uni.is_empty());
        assert!(s.personal().picked.is_empty());
        assert!(!s.personal().has_pending_learning());
    }

    /// rollback window：下一個輸入不是撤銷鍵，就把學習留下。
    #[test]
    fn test_next_input_keeps_learning() {
        let mut s = session();
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        s.press(Key::new(KeyCode::Down, None, false));
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "景");

        type_syllable(&mut s, "ㄈㄥ");
        assert!(s.personal().is_learned("景"));
        assert!(!s.personal().has_pending_learning());
    }

    /// 弱負向訊號的接線：跳過已學過的候選時記一次跳過。
    ///
    /// 連續跳過到門檻後如何降權由 `personal` 的排序測試涵蓋；這裡只確認
    /// 選字路徑真的把訊號寫進個人資料。
    #[test]
    fn test_skipping_learned_candidate_records_signal() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("井", "ㄐㄧㄥˇ", 0, 0));
        lex.push(LexEntry::new("景", "ㄐㄧㄥˇ", 40, 0));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        config.personal.note_seed("井", 1.0);
        let mut s = Session::new(config);

        type_syllable(&mut s, "ㄐㄧㄥˇ");
        assert_eq!(s.candidates()[0].surface, "井", "學過的詞先排第一");
        let index = s
            .candidates()
            .iter()
            .position(|candidate| candidate.surface == "景")
            .expect("景 是候選");
        s.select(index);
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "景");
        assert_eq!(s.personal().skip_count("井"), 1, "被跳過的學習詞記一次");
    }

    /// 弱負向訊號會自我修復：選回來一次就清掉計數。
    #[test]
    fn test_selecting_skipped_candidate_clears_skip() {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("井", "ㄐㄧㄥˇ", 0, 0));
        lex.push(LexEntry::new("景", "ㄐㄧㄥˇ", 40, 0));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        config.personal.note_seed("井", 1.0);
        for _ in 0..crate::personal::SKIP_LIMIT {
            config.personal.note_skip("井");
        }
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        assert_ne!(s.candidates()[0].surface, "井", "跳過 3 次的詞不再排第一");

        // 選回被跳過的詞：清掉計數，下一次回到原本優先度。
        let index = s
            .candidates()
            .iter()
            .position(|candidate| candidate.surface == "井")
            .expect("井 仍在候選中");
        s.select(index);
        assert_eq!(s.highlighted_text(), Some("井"));
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "井");
        assert_eq!(s.personal().skip_count("井"), 0);

        type_syllable(&mut s, "ㄐㄧㄥˇ");
        assert_eq!(s.candidates()[0].surface, "井");
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
            assert_eq!(segment.mode, Lang::Zh);
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
        let mut s = Session::new(SessionConfig::new(Mode::Mixed));
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
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
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
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
        config.punct_mark = Some('，');
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄈㄥ");
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "風，景");
    }

    #[test]
    fn test_commit_keeps_text_verbatim() {
        let mut s = session();
        type_syllable(&mut s, "ㄈㄥ");
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "風景");
    }

    #[test]
    fn test_rerank_prefers_configured_candidate() {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
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
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
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
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
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
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
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
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
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
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon));
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

    /// 18 個同音候選、頁面 9 項：第二頁按 `1` 選第 10 個，不是第 1 個。
    #[test]
    fn test_digit_on_second_page_selects_page_offset() {
        let mut s = eighteen_candidates_session();
        type_syllable(&mut s, "ㄕˋ");
        assert_eq!(s.candidates().len(), 18, "要剛好兩頁");
        s.press(Key::new(KeyCode::ArrowRight, None, false));
        s.press(Key::new(KeyCode::PageDown, None, false));
        assert_eq!(s.current_page(), 1, "PageDown 到第二頁");
        assert_eq!(s.page_size(), 9);

        let want = s.candidates()[9].surface.clone();
        s.press(Key::char('1', false));
        assert_eq!(s.commit(), want, "第二頁的 1 是第 10 個候選");
    }

    /// 頁碼由 `highlighted / page_size` 推導：同頁內移動高亮後，數字鍵仍以
    /// 頁首為基準。
    #[test]
    fn test_digit_selection_uses_page_derived_from_highlight() {
        let mut s = eighteen_candidates_session();
        type_syllable(&mut s, "ㄕˋ");
        s.press(Key::new(KeyCode::ArrowRight, None, false));
        s.press(Key::new(KeyCode::PageDown, None, false));
        assert_eq!(s.highlighted(), 9);
        s.press(Key::new(KeyCode::Down, None, false));
        assert_eq!(s.highlighted(), 10);

        let want = s.candidates()[10].surface.clone();
        s.press(Key::char('2', false));
        assert_eq!(s.commit(), want, "頁內第 2 項是索引 10");
    }

    /// 候選不足一頁時，超出範圍的數字鍵被吃掉，狀態不變。
    #[test]
    fn test_digit_beyond_candidate_count_keeps_state() {
        let mut lex = InMemoryLexicon::new();
        for (index, surface) in ["甲", "乙", "丙"].iter().enumerate() {
            lex.push(LexEntry::new(*surface, "ㄕˋ", index as i32, 0));
        }
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        config.max_candidates = DEFAULT_MAX_CANDIDATES;
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄕˋ");
        s.press(Key::new(KeyCode::ArrowRight, None, false));
        let before = s.snapshot();

        let after = s.press(Key::char('5', false));
        assert_eq!(after.preedit, before.preedit, "組字不變");
        assert_eq!(after.candidates, before.candidates, "候選不變");
        assert_eq!(after.highlighted, 0, "高亮不變");
        assert!(after.navigating, "仍在選取模式，沒有退回注音鍵");
    }

    /// 同音候選剛好 18 個的注音 session。
    fn eighteen_candidates_session() -> Session {
        let mut lex = InMemoryLexicon::new();
        for (index, surface) in "是事市試式世室士視示適勢誓釋逝飾侍嗜".chars().enumerate()
        {
            lex.push(LexEntry::new(
                surface.to_string(),
                "ㄕˋ",
                index as i32 * 10,
                0,
            ));
        }
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        config.max_candidates = DEFAULT_MAX_CANDIDATES;
        Session::new(config)
    }

    fn mixed_session() -> Session {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
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
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
        config.english_autocomplete = true;
        let mut s = Session::new(config);
        type_keys(&mut s, "ji3hellp");
        assert_eq!(s.snapshot().preedit, "我hellp");
        s.press(Key::new(KeyCode::Tab, None, false));
        assert_eq!(s.snapshot().preedit, "我hello");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "我hello");
    }

    /// 含本任務路徑所需讀音的混合 session；`su` 的注音前綴需要 ㄋㄧ 可查。
    fn mixed_zhuyin_session() -> Session {
        let mut lex = InMemoryLexicon::new();
        for (surface, reading) in [
            ("你", "ㄋㄧˇ"),
            ("好", "ㄏㄠˇ"),
            ("這", "ㄓㄜˋ"),
            ("尼", "ㄋㄧ"),
        ] {
            lex.push(LexEntry::new(surface, reading, 0, 0));
        }
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        config.english_autocomplete = true;
        Session::new(config)
    }

    /// `su` 是未完成的注音前綴：英文補全只留在提示，數字鍵補聲調而非選補全。
    #[test]
    fn test_su_prefix_tone_key_wins_over_digit_selection() {
        let mut s = mixed_zhuyin_session();
        type_keys(&mut s, "su");
        let surfaces: Vec<String> = s
            .candidates()
            .iter()
            .map(|candidate| candidate.surface.clone())
            .collect();
        assert!(
            !surfaces
                .iter()
                .any(|text| text.starts_with("su") && text.len() > 2),
            "未完成注音不該有英文補全候選：{surfaces:?}"
        );
        // 外殼在單一數字鍵前補 Down。
        s.press(Key::new(KeyCode::Down, None, false));
        type_keys(&mut s, "3cl3");
        assert_eq!(s.snapshot().preedit, "你好");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "你好");
    }

    /// `5k4` 不回歸：數字是聲調鍵，即使外殼補 Down 也完成 ㄓㄜˋ。
    #[test]
    fn test_5k4_tone_key_not_regressed() {
        let mut s = mixed_zhuyin_session();
        type_keys(&mut s, "5k");
        s.press(Key::new(KeyCode::Down, None, false));
        type_keys(&mut s, "4");
        assert_eq!(s.snapshot().preedit, "這");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "這");
    }

    /// 英文前綴不是注音，補全候選與 Tab 接受維持原行為。
    #[test]
    fn test_hello_tab_completion_unchanged() {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
        config.english_autocomplete = true;
        let mut s = Session::new(config);
        type_keys(&mut s, "hellp");
        assert_eq!(s.snapshot().suggestion.as_deref(), Some("hello"));
        let surfaces: Vec<String> = s
            .candidates()
            .iter()
            .map(|candidate| candidate.surface.clone())
            .collect();
        assert!(surfaces.iter().any(|text| text == "hello"), "{surfaces:?}");
        s.press(Key::new(KeyCode::Tab, None, false));
        assert_eq!(s.snapshot().preedit, "hello");
        // 完整英文詞沒有補全，Tab 不改內容。
        s.press(Key::new(KeyCode::Tab, None, false));
        assert_eq!(s.snapshot().preedit, "hello");
    }

    /// `sug` 已不是合法注音前綴，Down + 數字照舊選英文候選。
    #[test]
    fn test_sug_digit_selects_english_candidate() {
        let mut s = mixed_zhuyin_session();
        type_keys(&mut s, "sug");
        s.press(Key::new(KeyCode::Down, None, false));
        // 補全候選來自英文詞表；詞表是不是完整取決於資料包，CI 上可能只有
        // 內建原型詞。這裡不假設固定索引，挑第一個比 `sug` 長的英文補全。
        let candidates = s.candidates();
        let (index, expected) = candidates
            .iter()
            .enumerate()
            .find(|(_, candidate)| {
                candidate.surface.starts_with("sug") && candidate.surface.chars().count() > 3
            })
            .map(|(index, candidate)| (index, candidate.surface.clone()))
            .expect("sug 應該有英文補全候選");
        assert!(index < 9, "測試只按 1 到 9 選候選，實際索引 {index}");
        type_keys(&mut s, &(index + 1).to_string());
        assert_eq!(s.snapshot().preedit, expected);
        assert_ne!(s.snapshot().preedit, "sug3");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), expected);
    }

    /// `ji3hellp` 不回歸：注音完成後英文補全提示與 Tab 仍可用。
    #[test]
    fn test_ji3hellp_completion_not_regressed() {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
        config.english_autocomplete = true;
        let mut s = Session::new(config);
        type_keys(&mut s, "ji3hellp");
        assert_eq!(s.snapshot().preedit, "我hellp");
        assert_eq!(s.snapshot().suggestion.as_deref(), Some("hello"));
        s.press(Key::new(KeyCode::Tab, None, false));
        assert_eq!(s.snapshot().preedit, "我hello");
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
                config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
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
        let mut config = SessionConfig::new(Mode::Mixed);
        let mut lex = lexicon();
        lex.push(LexEntry::new("台", "ㄊㄞˊ", 0, 0));
        lex.push(LexEntry::new("灣", "ㄨㄢ", 0, 0));
        lex.push(LexEntry::new("設", "ㄕㄜˋ", 0, 0));
        lex.push(LexEntry::new("定", "ㄉㄧㄥˋ", 0, 0));
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        config.simplified = Some(Arc::new(converter));
        let mut s = Session::new(config);
        assert!(!s.output_simplified(), "預設不轉簡體");

        // 關閉時原樣送出，組字預覽也保持繁體。
        type_syllable(&mut s, "ㄊㄞˊ");
        type_syllable(&mut s, "ㄨㄢ");
        assert_eq!(s.snapshot().preedit, "台灣");
        assert_eq!(s.commit(), "台灣");

        // 開啟後，一般 commit 路徑轉成簡體；預覽仍是繁體。
        s.set_output_simplified(true);
        assert!(s.output_simplified());
        type_syllable(&mut s, "ㄕㄜˋ");
        type_syllable(&mut s, "ㄉㄧㄥˋ");
        assert_eq!(s.snapshot().preedit, "設定");
        assert_eq!(s.commit(), "设定");

        // Enter 先放進 pending_commit 的路徑也要轉換。
        type_syllable(&mut s, "ㄊㄞˊ");
        type_syllable(&mut s, "ㄨㄢ");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "台湾");

        // 再關閉就回到繁體。
        s.set_output_simplified(false);
        type_syllable(&mut s, "ㄊㄞˊ");
        type_syllable(&mut s, "ㄨㄢ");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "台灣");
    }

    #[test]
    fn test_output_simplified_without_converter_stays_off() {
        let mut s = session();
        s.set_output_simplified(true);
        assert!(!s.output_simplified(), "沒有轉換器時不得宣告開啟");
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

    /// 全形數字：混合模式的英數字面候選提供 `０-９` 變體。
    ///
    /// 英文直出模式已移除；等價路徑是混合模式待定英數按鍵的寬度候選。
    #[test]
    fn test_full_width_digit_candidates() {
        let mut s = Session::new(SessionConfig::new(Mode::Mixed));
        for ch in "123".chars() {
            s.press(Key::char(ch, false));
        }
        assert_eq!(s.snapshot().preedit, "123");
        s.press(Key::new(KeyCode::Tab, None, false));
        assert_eq!(s.candidates()[1].surface, "１２３");
        s.press(Key::char('2', false));
        assert_eq!(s.commit(), "１２３");
    }

    /// 單按 Shift 由外殼處理；引擎只回快照，不切換核心模式也不清組字。
    #[test]
    fn test_toggle_english_keeps_mixed_composition() {
        let mut s = Session::new(SessionConfig::new(Mode::Mixed));
        type_syllable(&mut s, "ㄨㄛˇ");
        let before = s.snapshot().preedit;
        assert_eq!(s.mode(), Mode::Mixed);
        s.toggle_english();
        assert_eq!(s.mode(), Mode::Mixed);
        assert_eq!(s.snapshot().preedit, before, "切英文不清緩衝");
        s.toggle_english();
        assert_eq!(s.mode(), Mode::Mixed);
        assert_eq!(s.snapshot().preedit, before);
    }

    /// 混合輸入中真正的日文片段也一樣：toggle 不改變核心狀態。
    #[test]
    fn test_toggle_english_keeps_japanese_run() {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.ja = Some(Arc::new(JaDecoder::new(Arc::new(LiveTestSource(
            Vec::new(),
        )))));
        let mut s = Session::new(config);
        type_ja(&mut s, "watashi");
        let before = s.snapshot().preedit;
        assert_eq!(before, "わたし", "要先建立 romaji → kana 的日文片段");
        assert!(s.ja_run, "這段組字必須是日文推斷出來的 ja_run");

        s.toggle_english();
        assert_eq!(s.mode(), Mode::Mixed);
        assert_eq!(s.snapshot().preedit, before, "切英文後日文片段仍在");
        s.toggle_english();
        assert_eq!(s.mode(), Mode::Mixed);
        assert_eq!(s.snapshot().preedit, before, "切回中文後日文片段仍在");
    }

    /// 引擎一律停在 Mixed，沒有「切換來源」可以回到其他模式。
    #[test]
    fn test_toggle_english_without_previous_mode_uses_mixed() {
        let mut s = english_session();
        s.toggle_english();
        assert_eq!(s.mode(), Mode::Mixed);
        s.toggle_english();
        assert_eq!(s.mode(), Mode::Mixed);
    }

    /// 外部 `set_mode` 之後 toggle 仍停在 Mixed，不會回到過期模式。
    #[test]
    fn test_set_mode_keeps_toggle_in_mixed() {
        let mut s = Session::new(SessionConfig::new(Mode::Mixed));
        s.toggle_english();
        s.set_mode(Mode::Mixed);
        s.toggle_english();
        assert_eq!(s.mode(), Mode::Mixed);
        s.toggle_english();
        assert_eq!(s.mode(), Mode::Mixed);
    }

    /// 英文拼錯時，候選在原字後面補上拼字建議；打對的字沒有拼字建議。
    #[test]
    fn test_english_candidates_include_spelling_suggestions() {
        let mut s = Session::new(SessionConfig::new(Mode::Mixed));
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

        let mut correct = Session::new(SessionConfig::new(Mode::Mixed));
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

    fn english_session() -> Session {
        // 混合模式需要注音詞典才會把打不成注音的短字判成英文；英文直出
        // 模式已移除，這裡用等價的 Mixed 設定重現原本的英文候選路徑。
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
        Session::new(config)
    }

    fn candidate_index(session: &Session, surface: &str) -> Option<usize> {
        session
            .candidates()
            .iter()
            .position(|candidate| candidate.surface == surface)
    }

    fn accept_english(session: &mut Session, surface: &str) -> String {
        let index = candidate_index(session, surface).expect("候選要有指定的字面");
        session.select(index);
        // Mixed 的英文候選要先進入選取模式，數字鍵才會套用游標那一項。
        session.press(Key::new(KeyCode::Down, None, false));
        let digit = char::from_digit((index % crate::session::PAGE_SIZE) as u32 + 1, 10)
            .expect("候選在頁內");
        session.press(Key::char(digit, false));
        session.commit()
    }

    /// 接受 `recieve → receive` 後，再打同一個拼法時學到的修正排第一。
    #[test]
    fn test_english_learned_correction_ranks_first() {
        let mut s = english_session();
        type_keys(&mut s, "recieve");
        assert_eq!(accept_english(&mut s, "receive"), "receive");

        type_keys(&mut s, "recieve");
        assert_eq!(s.candidates()[0].surface, "receive");
        assert!(candidate_index(&s, "recieve").is_some(), "原字仍留在候選裡");
    }

    /// 連續跳過學過的修正 3 次後不再排第一，但仍在候選中。
    #[test]
    fn test_english_skip_demotes_learned_correction() {
        let mut s = english_session();
        type_keys(&mut s, "recieve");
        assert_eq!(accept_english(&mut s, "receive"), "receive");

        for _ in 0..crate::personal::SKIP_LIMIT {
            type_keys(&mut s, "recieve");
            assert_eq!(accept_english(&mut s, "recieve"), "recieve");
        }

        type_keys(&mut s, "recieve");
        assert_ne!(s.candidates()[0].surface, "receive");
        assert!(
            candidate_index(&s, "receive").is_some(),
            "降權不是刪除，修正仍要留在候選裡"
        );
    }

    /// 跳過降權之後手選一次修正，下一次就恢復第一。
    #[test]
    fn test_english_manual_choice_restores_learned_correction() {
        let mut s = english_session();
        type_keys(&mut s, "recieve");
        assert_eq!(accept_english(&mut s, "receive"), "receive");
        for _ in 0..crate::personal::SKIP_LIMIT {
            type_keys(&mut s, "recieve");
            assert_eq!(accept_english(&mut s, "recieve"), "recieve");
        }
        type_keys(&mut s, "recieve");
        assert_ne!(s.candidates()[0].surface, "receive");

        assert_eq!(accept_english(&mut s, "receive"), "receive");
        type_keys(&mut s, "recieve");
        assert_eq!(s.candidates()[0].surface, "receive");
    }

    /// 5 個字母的拼法只記錄證據，排序維持原樣。
    #[test]
    fn test_english_short_correction_records_only() {
        let mut s = english_session();
        type_keys(&mut s, "hellp");
        assert_eq!(accept_english(&mut s, "hello"), "hello");
        assert_eq!(
            s.personal().english_correction("hellp"),
            None,
            "短於最短觸發長度只記錄"
        );

        type_keys(&mut s, "hellp");
        assert_eq!(s.candidates()[0].surface, "hellp");
        assert!(
            candidate_index(&s, "hello").is_some(),
            "內建拼字建議不受學習影響"
        );
    }

    /// 沒有學習紀錄時，英文候選與改動前相同：原字第一，拼字建議在後面。
    #[test]
    fn test_english_without_learning_keeps_existing_order() {
        let mut s = english_session();
        type_keys(&mut s, "recieve");
        assert_eq!(s.candidates()[0].surface, "recieve");
        assert!(candidate_index(&s, "receive").is_some());

        let mut correct = english_session();
        type_keys(&mut correct, "hello");
        let candidates = correct.candidates();
        assert_eq!(candidates[0].surface, "hello", "原字仍是第一");
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.source != CandidateSource::Personal),
            "沒有學習紀錄時不得出現個人化候選"
        );
        // 打對的完整單字不該有拼字建議；全形變體與更長的前綴補完照舊。
        let same_length: Vec<&str> = candidates
            .iter()
            .map(|candidate| candidate.surface.as_str())
            .filter(|text| {
                text.chars().count() == 5
                    && text.chars().all(|ch| ch.is_ascii_alphabetic())
                    && !text.eq_ignore_ascii_case("hello")
            })
            .collect();
        assert!(same_length.is_empty(), "拼字建議：{same_length:?}");
        for candidate in &candidates[2..] {
            assert!(
                candidate.surface.to_lowercase().starts_with("hello"),
                "補完要以原字開頭：{candidate:?}"
            );
        }
    }

    /// 混合模式的待定英文詞也能看到拼字建議，選了就取代原按鍵。
    #[test]
    fn test_mixed_candidates_include_spelling_suggestions() {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
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
        let at = candidate_index(&s, "receive").expect("要有 receive 的拼字建議");
        s.select(at);
        let digit =
            char::from_digit((at % crate::session::PAGE_SIZE) as u32 + 1, 10).expect("候選在頁內");
        s.press(Key::char(digit, false));
        assert_eq!(s.snapshot().preedit, "receive");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "receive");
    }

    /// 最小日文詞條來源，讓日文 session 不必載入 pack 也能轉換。
    struct LiveTestSource(Vec<crate::ja::lexicon::JaEntry<'static>>);

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
                    entry: entry.clone(),
                    left_id: None,
                    right_id: None,
                });
            }
        }

        fn exact<'a>(&'a self, reading: &[char]) -> Vec<crate::ja::lexicon::JaEntry<'a>> {
            self.0
                .iter()
                .filter(|entry| entry.reading.chars().eq(reading.iter().copied()))
                .cloned()
                .collect()
        }

        fn prefixed<'a>(&'a self, reading: &[char]) -> Vec<crate::ja::lexicon::JaEntry<'a>> {
            self.0
                .iter()
                .filter(|entry| {
                    let units: Vec<char> = entry.reading.chars().collect();
                    units.len() >= reading.len() && units[..reading.len()] == *reading
                })
                .cloned()
                .collect()
        }
    }

    fn ja_live_session(config: live::Config) -> Session {
        let entry = crate::ja::lexicon::JaEntry {
            surface: "今日".into(),
            reading: "きょう".into(),
            pos: "名詞,普通名詞,一般,*".into(),
            conj_type: "*".into(),
            conj_form: "*".into(),
            cost: 0,
            split: crate::ja::lexicon::Split::C,
        };
        let decoder = Arc::new(JaDecoder::new(Arc::new(LiveTestSource(vec![entry]))));
        let mut session_config = SessionConfig::new(Mode::Mixed);
        session_config.ja = Some(decoder);
        session_config.ja_only = true;
        session_config.ja_live = config;
        Session::new(session_config)
    }

    fn type_ja(session: &mut Session, keys: &str) {
        for ch in keys.chars() {
            session.press(Key::char(ch, false));
        }
    }

    fn ja_entry(
        surface: &'static str,
        reading: &'static str,
        cost: i32,
    ) -> crate::ja::lexicon::JaEntry<'static> {
        crate::ja::lexicon::JaEntry {
            surface: surface.into(),
            reading: reading.into(),
            pos: "名詞,普通名詞,一般,*".into(),
            conj_type: "*".into(),
            conj_form: "*".into(),
            cost,
            split: crate::ja::lexicon::Split::C,
        }
    }

    /// 使用者手選的片段在後續停頓重轉時保持不變。
    #[test]
    fn test_ja_deferred_sentence_keeps_user_segment_on_reconvert() {
        let source = LiveTestSource(vec![
            ja_entry("今日", "きょう", 1_000),
            ja_entry("は", "は", 500),
            ja_entry("いい", "いい", 500),
            ja_entry("天気", "てんき", 900),
            ja_entry("です", "です", 500),
            ja_entry("ね", "ね", 500),
        ]);
        let mut config = SessionConfig::new(Mode::Mixed);
        config.ja = Some(Arc::new(JaDecoder::new(Arc::new(source))));
        config.ja_only = true;
        config.ja_live = live::Config {
            enabled: true,
            debounce: std::time::Duration::ZERO,
            idle_delay: std::time::Duration::ZERO,
            deferred_sentence: true,
            ..live::Config::default()
        };
        let mut s = Session::new(config);
        type_ja(&mut s, "kyouhaiitenki");
        let first = s.ja_live_idle();
        assert_eq!(first.preedit, "今日はいい天気");

        // 游標移到「天気」片段，手動切換成平假名。
        s.press(Key::new(KeyCode::ArrowLeft, None, false));
        let picked = s.ja_toggle_script();
        assert_eq!(picked.preedit, "今日はいい てんき".replace(' ', ""));
        assert!(picked.preedit.ends_with("てんき"));

        // 繼續打「ですね」，停頓重轉；手選的てんき必須保留。
        type_ja(&mut s, "desune");
        let again = s.ja_live_idle();
        assert!(
            again.preedit.ends_with("てんきですね"),
            "重轉洗掉手選片段：{}",
            again.preedit
        );
        assert!(!again.preedit.contains("天気"), "{}", again.preedit);
        assert_eq!(
            again.preedit,
            "今日はいい てんきですね".replace(' ', ""),
            "手選片段保留率應為 100%"
        );
    }

    /// F7 切換假名後，停頓重轉不得改回漢字。
    #[test]
    fn test_ja_deferred_sentence_keeps_toggled_segment_on_reconvert() {
        let source = LiveTestSource(vec![
            ja_entry("東京", "とうきょう", 1_000),
            ja_entry("に", "に", 500),
        ]);
        let mut config = SessionConfig::new(Mode::Mixed);
        config.ja = Some(Arc::new(JaDecoder::new(Arc::new(source))));
        config.ja_only = true;
        config.ja_live = live::Config {
            enabled: true,
            debounce: std::time::Duration::ZERO,
            idle_delay: std::time::Duration::ZERO,
            deferred_sentence: true,
            ..live::Config::default()
        };
        let mut s = Session::new(config);
        type_ja(&mut s, "toukyou");
        assert_eq!(s.ja_live_idle().preedit, "東京");

        s.ja_toggle_script();
        assert!(
            s.user_marks()
                .iter()
                .any(|mark| mark.lang == crate::composition::Lang::Ja),
            "切換假名要留下語言無關的 User 標記"
        );

        type_ja(&mut s, "ni");
        let again = s.ja_live_idle();
        assert_eq!(again.preedit, "とうきょうに", "F7 固定片段被重轉改寫");
    }

    /// 拆回讀音後，該片段仍是 User，重轉不會再變成漢字。
    #[test]
    fn test_ja_deferred_sentence_keeps_unconverted_segment() {
        let source = LiveTestSource(vec![
            ja_entry("東京", "とうきょう", 1_000),
            ja_entry("に", "に", 500),
        ]);
        let mut config = SessionConfig::new(Mode::Mixed);
        config.ja = Some(Arc::new(JaDecoder::new(Arc::new(source))));
        config.ja_only = true;
        config.ja_live = live::Config {
            enabled: true,
            debounce: std::time::Duration::ZERO,
            idle_delay: std::time::Duration::ZERO,
            deferred_sentence: true,
            ..live::Config::default()
        };
        let mut s = Session::new(config);
        type_ja(&mut s, "toukyou");
        s.ja_live_idle();
        s.ja_unconvert();

        type_ja(&mut s, "ni");
        let again = s.ja_live_idle();
        assert_eq!(again.preedit, "とうきょうに", "拆回讀音後又被重轉成漢字");
    }

    /// 讀音 `とうきょう` 的候選含漢字、平假名、片假名與半角かな。
    #[test]
    fn test_ja_live_candidates_include_all_kana_scripts() {
        let source = LiveTestSource(vec![
            ja_entry("東京", "とうきょう", 1_000),
            ja_entry("とうきょう", "とうきょう", 2_000),
        ]);
        let mut config = SessionConfig::new(Mode::Mixed);
        config.ja = Some(Arc::new(JaDecoder::new(Arc::new(source))));
        config.ja_only = true;
        config.ja_live = live::Config {
            enabled: true,
            debounce: std::time::Duration::ZERO,
            trigger: live::Trigger::EveryKey,
            deferred_sentence: false,
            ..live::Config::default()
        };
        let mut s = Session::new(config);
        type_ja(&mut s, "toukyou");
        // 非延遲模式在輸入結束時補齊尚未轉換的讀音。
        s.ja_live_flush();
        assert_eq!(s.snapshot().preedit, "東京");
        let shown = s.press(Key::new(KeyCode::Space, None, false));
        let surfaces: Vec<&str> = shown
            .candidates
            .iter()
            .map(|candidate| candidate.surface.as_str())
            .collect();
        for want in ["東京", "とうきょう", "トウキョウ", "ﾄｳｷｮｳ"] {
            assert!(surfaces.contains(&want), "缺少 {want}：{surfaces:?}");
        }
        let kanji = surfaces.iter().position(|text| *text == "東京").unwrap();
        let hira = surfaces
            .iter()
            .position(|text| *text == "とうきょう")
            .unwrap();
        assert!(kanji < hira, "假名候選要排在漢字後面：{surfaces:?}");
    }

    /// 漢字片段按一次 F7 變平假名，再按變全角片假名。
    #[test]
    fn test_ja_toggle_script_from_kanji() {
        let mut s = ja_live_session(live::Config {
            enabled: true,
            debounce: std::time::Duration::ZERO,
            deferred_sentence: false,
            ..live::Config::default()
        });
        type_ja(&mut s, "kyou");
        assert_eq!(s.snapshot().preedit, "今日");
        assert_eq!(s.ja_toggle_script().preedit, "きょう");
        assert_eq!(s.ja_toggle_script().preedit, "キョウ");
    }

    /// 平假名片段切到片假名，再切回平假名。
    #[test]
    fn test_ja_toggle_script_from_hiragana() {
        let mut s = ja_live_session(live::Config {
            enabled: true,
            debounce: std::time::Duration::ZERO,
            deferred_sentence: false,
            ..live::Config::default()
        });
        type_ja(&mut s, "kyou");
        s.ja_toggle_script();
        assert_eq!(s.snapshot().preedit, "きょう");
        assert_eq!(s.ja_toggle_script().preedit, "キョウ");
        assert_eq!(s.ja_toggle_script().preedit, "きょう");
    }

    /// 拆回讀音後，讀音回到尾巴，補上後續輸入可重新切分。
    #[test]
    fn test_ja_unconvert_returns_reading_to_tail() {
        let source = LiveTestSource(vec![
            ja_entry("東京", "とうきょう", 1_000),
            ja_entry("東京都", "とうきょうと", 100),
        ]);
        let mut config = SessionConfig::new(Mode::Mixed);
        config.ja = Some(Arc::new(JaDecoder::new(Arc::new(source))));
        config.ja_only = true;
        config.ja_live = live::Config {
            enabled: true,
            debounce: std::time::Duration::ZERO,
            deferred_sentence: false,
            ..live::Config::default()
        };
        let mut s = Session::new(config);
        type_ja(&mut s, "toukyou");
        assert_eq!(s.snapshot().preedit, "東京");
        assert_eq!(s.ja_unconvert().preedit, "とうきょう", "片段拆回讀音");
        type_ja(&mut s, "to");
        assert_eq!(s.snapshot().preedit, "東京都", "讀音回到尾巴後可重新切分");
    }

    /// 讀音本身就是平假名時，候選不重複同一字串。
    #[test]
    fn test_ja_live_candidates_dedup_hiragana() {
        let source = LiveTestSource(vec![ja_entry("とうきょう", "とうきょう", 1_000)]);
        let mut config = SessionConfig::new(Mode::Mixed);
        config.ja = Some(Arc::new(JaDecoder::new(Arc::new(source))));
        config.ja_only = true;
        config.ja_live = live::Config {
            enabled: true,
            debounce: std::time::Duration::ZERO,
            trigger: live::Trigger::EveryKey,
            deferred_sentence: false,
            ..live::Config::default()
        };
        let mut s = Session::new(config);
        type_ja(&mut s, "toukyou");
        let shown = s.press(Key::new(KeyCode::Space, None, false));
        let count = shown
            .candidates
            .iter()
            .filter(|candidate| candidate.surface == "とうきょう")
            .count();
        assert_eq!(count, 1, "平假名候選不得重複");
    }

    /// 逐字変換的片段游標、Space 開片段候選與 Enter 套用。
    #[test]
    fn test_ja_live_cursor_and_segment_candidates() {
        use crate::ja::live::{KanaTransform, Trigger};

        let mut config = live::Config {
            enabled: true,
            debounce: std::time::Duration::ZERO,
            trigger: Trigger::EveryKey,
            deferred_sentence: false,
            ..live::Config::default()
        };
        config.neural = None;
        let mut s = ja_live_session(config);
        type_ja(&mut s, "kyou");
        s.ja_live_flush();
        assert_eq!(s.snapshot().preedit, "今日");

        let left = s.press(Key::new(KeyCode::ArrowLeft, None, false));
        assert_eq!(left.cursor_rune, 1, "← 移動日文組字游標");
        assert_eq!(left.preedit, "今日", "移動游標不改組字");

        let shown = s.press(Key::new(KeyCode::Space, None, false));
        assert!(!shown.navigating, "第一次 Space 只開片段候選");
        assert!(shown
            .candidates
            .iter()
            .any(|candidate| candidate.surface == "きょう"));
        assert!(shown
            .candidates
            .iter()
            .any(|candidate| candidate.surface == "キョウ"));

        // 第一下進入選取模式，第二下移到「きょう」。
        s.press(Key::new(KeyCode::Down, None, false));
        s.press(Key::new(KeyCode::Down, None, false));
        let picked = s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(picked.preedit, "きょう", "選字只換游標所在片段");
        let right = s.press(Key::new(KeyCode::ArrowRight, None, false));
        assert_eq!(right.preedit, "きょう", "游標移出後維持新結果");

        let kata = s.ja_transform(KanaTransform::Katakana);
        assert_eq!(kata.preedit, "キョウ", "F7 由片段讀音轉全角片假名");
        let hira = s.ja_transform(KanaTransform::Hiragana);
        assert_eq!(hira.preedit, "きょう", "F6 轉回平假名");
    }

    /// 同讀音、成本差距可被學習翻盤的日文 session。
    fn ja_learning_session(learning: bool) -> Session {
        let source = LiveTestSource(vec![
            ja_entry("今日", "きょう", 1_000),
            ja_entry("京", "きょう", 1_400),
            ja_entry("きょう", "きょう", 3_000),
        ]);
        let mut config = SessionConfig::new(Mode::Mixed);
        config.ja = Some(Arc::new(JaDecoder::new(Arc::new(source))));
        config.ja_only = true;
        config.learning = learning;
        Session::new(config)
    }

    /// 手選第二候選後，同讀音的首選會改變；undo 回到原狀。
    #[test]
    fn test_ja_learning_ranks_manual_choice_first_and_undo_restores() {
        use crate::personal::LanguageId;

        let mut s = ja_learning_session(true);
        type_ja(&mut s, "kyou");
        // 預設延遲整句：停頓後才轉換並產生候選。
        s.ja_live_idle();
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
        s.ja_live_idle();
        assert_eq!(s.candidates()[0].surface, "京");

        // 反悔後回到統計順序。
        assert!(s.undo_learning());
        s.ja_live_idle();
        assert_eq!(s.candidates()[0].surface, "今日");
    }

    /// 學習關閉時，手選第二候選不會改變下一次的首選。
    #[test]
    fn test_ja_learning_disabled_keeps_statistical_order() {
        let mut s = ja_learning_session(false);
        type_ja(&mut s, "kyou");
        s.ja_live_idle();
        s.press(Key::new(KeyCode::ArrowRight, None, false));
        s.press(Key::new(KeyCode::ArrowRight, None, false));
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "京");
        type_ja(&mut s, "kyou");
        s.ja_live_idle();
        assert_eq!(s.candidates()[0].surface, "今日");
    }

    /// 單次被動接受不足以翻掉明顯的統計差距。
    #[test]
    fn test_ja_passive_learning_keeps_clear_statistical_winner() {
        let mut s = ja_learning_session(true);
        type_ja(&mut s, "kyou");
        s.ja_live_idle();
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "今日");
        type_ja(&mut s, "kyou");
        s.ja_live_idle();
        assert_eq!(s.candidates()[0].surface, "今日");
    }

    /// 最短觸發長度：3 個假名不到 7 字元，被動接受只留證據、不改排序。
    #[test]
    fn test_ja_short_reading_records_evidence_only() {
        use crate::personal::LanguageId;

        let mut s = ja_learning_session(true);
        type_ja(&mut s, "kyou");
        s.ja_live_idle();
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "今日");
        assert_eq!(
            s.personal().language_surface_count(LanguageId::Ja, "今日"),
            1,
            "證據要記錄"
        );
        assert_eq!(
            s.personal()
                .language_reading_count(LanguageId::Ja, "き ょ う", "今日"),
            0,
            "短讀音不進讀音表"
        );

        type_ja(&mut s, "kyou");
        s.ja_live_idle();
        assert_eq!(s.candidates()[0].surface, "今日", "排序不變");
    }

    /// 沒有開啟逐字変換時，日文走原本的 nbest 流程，結果與先前相同。
    #[test]
    fn test_ja_live_disabled_by_default_keeps_plain_conversion() {
        let mut s = ja_live_session(live::Config {
            enabled: false,
            ..live::Config::default()
        });
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
            deferred_sentence: false,
            ..live::Config::default()
        };
        let mut s = ja_live_session(config);
        assert!(s.ja_live_enabled());
        type_ja(&mut s, "kyou");
        assert_eq!(s.snapshot().preedit, "今日");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "今日");
    }

    /// 測試用神經引擎：固定回覆，可選延遲與呼叫計數。
    struct StubEngine {
        reply: &'static str,
        delay: std::time::Duration,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl neural::NeuralEngine for StubEngine {
        fn convert(
            &mut self,
            _request: &neural::NeuralRequest,
        ) -> Result<String, neural::NeuralError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if !self.delay.is_zero() {
                std::thread::sleep(self.delay);
            }
            Ok(self.reply.to_string())
        }
    }

    /// 同讀音兩個詞、成本差 400 的日文 session 設定；可選擇接上神經引擎。
    fn ja_neural_config(
        second_cost: i32,
        runtime: Option<Arc<neural::NeuralRuntime>>,
        timeout_ms: u64,
    ) -> SessionConfig {
        let source = LiveTestSource(vec![
            ja_entry("今日", "きょう", 1_000),
            ja_entry("京", "きょう", second_cost),
            ja_entry("きょう", "きょう", 3_000),
            // 同讀音候選湊到 8 個詞面，讓預設 `mean_candidates > 5.98` 成立。
            ja_entry("凶", "きょう", 3_100),
            ja_entry("況", "きょう", 3_200),
            ja_entry("教", "きょう", 3_300),
            ja_entry("郷", "きょう", 3_400),
            ja_entry("協", "きょう", 3_500),
        ]);
        let mut config = SessionConfig::new(Mode::Mixed);
        config.ja = Some(Arc::new(JaDecoder::new(Arc::new(source))));
        config.ja_only = true;
        config.ja_neural = neural::NeuralConfig {
            timeout_ms,
            runtime,
            ..neural::NeuralConfig::default()
        };
        config
    }

    /// 同讀音兩個詞、成本差 400 的日文 session；可選擇接上神經引擎。
    fn ja_neural_session(
        second_cost: i32,
        runtime: Option<Arc<neural::NeuralRuntime>>,
        timeout_ms: u64,
    ) -> (Session, Arc<std::sync::atomic::AtomicUsize>) {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        (
            Session::new(ja_neural_config(second_cost, runtime, timeout_ms)),
            calls,
        )
    }

    fn stub_runtime(
        reply: &'static str,
        delay: std::time::Duration,
        calls: &Arc<std::sync::atomic::AtomicUsize>,
    ) -> Arc<neural::NeuralRuntime> {
        Arc::new(neural::NeuralRuntime::spawn(Box::new(StubEngine {
            reply,
            delay,
            calls: Arc::clone(calls),
        })))
    }

    /// 門檻命中且輸出通過驗證時，採用 Jinen 的結果。
    #[test]
    fn test_ja_neural_replaces_when_gap_below_threshold() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let runtime = stub_runtime("京", std::time::Duration::ZERO, &calls);
        let (mut s, _) = ja_neural_session(1_400, Some(runtime), 180);
        type_ja(&mut s, "kyou");
        // 延遲整句：停頓後才轉換，神經門檻也在這時候跑。
        s.ja_live_idle();
        assert_eq!(s.snapshot().preedit, "京");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "京");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// 成本差超過門檻時完全不呼叫模型。
    #[test]
    fn test_ja_neural_skips_when_gap_above_threshold() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let runtime = stub_runtime("京", std::time::Duration::ZERO, &calls);
        let (mut s, _) = ja_neural_session(2_000, Some(runtime), 180);
        type_ja(&mut s, "kyou");
        s.ja_live_idle();
        assert_eq!(s.snapshot().preedit, "今日");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "今日");
    }

    /// 組字中的半截讀音不送 Jinen；完整讀音才送一次。
    #[test]
    fn test_ja_neural_not_called_for_incomplete_reading() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let runtime = stub_runtime("京", std::time::Duration::ZERO, &calls);
        let (mut s, _) = ja_neural_session(1_400, Some(runtime), 180);

        type_ja(&mut s, "ky");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "半截讀音不呼叫神經模型"
        );

        type_ja(&mut s, "ou");
        s.ja_live_idle();
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "完整讀音才送模型"
        );
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "京");
    }

    /// 平均候選數沒有超過下限時不呼叫模型；就算 gap 很小也一樣。
    #[test]
    fn test_ja_neural_skips_when_mean_candidates_below_min() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let runtime = stub_runtime("京", std::time::Duration::ZERO, &calls);
        // 逐字変換的門檻在建立 Session 時就複製給控制器，必須先改設定。
        let mut config = ja_neural_config(1_400, Some(runtime), 180);
        config.ja_neural.min_mean_candidates = 9.0;
        let mut s = Session::new(config);
        type_ja(&mut s, "kyou");
        s.ja_live_idle();
        assert_eq!(s.snapshot().preedit, "今日");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "今日");
    }

    /// 推論超過預算就沿用統計結果。
    #[test]
    fn test_ja_neural_timeout_keeps_statistical_result() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let runtime = stub_runtime("京", std::time::Duration::from_millis(80), &calls);
        let (mut s, _) = ja_neural_session(1_400, Some(runtime), 5);
        type_ja(&mut s, "kyou");
        s.ja_live_idle();
        assert_eq!(s.snapshot().preedit, "今日");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "今日");
        let stats = s.ja_neural_stats().expect("要能取得統計");
        assert_eq!(stats.timeouts, 1);
    }

    /// 模型缺席或關閉時，輸出與純統計完全相同。
    #[test]
    fn test_ja_neural_unavailable_keeps_statistical_result() {
        let (mut missing, _) = ja_neural_session(1_400, None, 180);
        let (mut disabled, _) = ja_neural_session(1_400, None, 180);
        disabled.config.ja_neural.enabled = false;
        for session in [&mut missing, &mut disabled] {
            type_ja(session, "kyou");
            session.ja_live_idle();
            assert_eq!(session.snapshot().preedit, "今日");
            session.press(Key::new(KeyCode::Enter, None, false));
            assert_eq!(session.commit(), "今日");
        }
        assert!(missing.ja_neural_stats().is_none());
    }

    /// 逐字変換加上神經轉換的 session 設定。
    fn ja_live_neural_session(
        second_cost: i32,
        runtime: Option<Arc<neural::NeuralRuntime>>,
    ) -> Session {
        let mut config = ja_neural_config(second_cost, runtime, 180);
        config.ja_live = live::Config {
            enabled: true,
            debounce: std::time::Duration::ZERO,
            deferred_sentence: false,
            // 這個測試驗的是即時逐字路徑；停頓補正走更嚴的門檻。
            neural_timing: live::NeuralTiming::Inline,
            ..live::Config::default()
        };
        Session::new(config)
    }

    /// 逐字路徑只在完整讀音時呼叫模型，結果與一般路徑一致。
    #[test]
    fn test_ja_live_neural_adopts_complete_reading_only() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let runtime = stub_runtime("京", std::time::Duration::ZERO, &calls);
        let mut s = ja_live_neural_session(1_400, Some(runtime));
        assert!(s.ja_live_enabled());
        type_ja(&mut s, "kyou");
        // 輸入結束時補齊讀音；停頓再把最後一個文節送神經門檻。
        s.ja_live_flush();
        assert_eq!(s.snapshot().preedit, "京");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "半截讀音（き、きょ）不得呼叫模型"
        );
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "京");
    }

    /// `--neural off` 時逐字路徑完全不呼叫模型，結果回到統計值。
    #[test]
    fn test_ja_live_neural_off_keeps_statistical_result() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let runtime = stub_runtime("京", std::time::Duration::ZERO, &calls);
        let mut config = ja_neural_config(1_400, Some(runtime), 180);
        config.ja_neural.enabled = false;
        config.ja_live = live::Config {
            enabled: true,
            debounce: std::time::Duration::ZERO,
            deferred_sentence: false,
            ..live::Config::default()
        };
        let mut s = Session::new(config);
        type_ja(&mut s, "kyou");
        assert_eq!(s.snapshot().preedit, "今日");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    /// 逐字路徑的門檻與一般路徑相同：平均候選數不足就不呼叫。
    #[test]
    fn test_ja_live_neural_respects_mean_candidates() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let runtime = stub_runtime("京", std::time::Duration::ZERO, &calls);
        let mut config = ja_neural_config(1_400, Some(runtime), 180);
        config.ja_neural.min_mean_candidates = 9.0;
        config.ja_live = live::Config {
            enabled: true,
            debounce: std::time::Duration::ZERO,
            deferred_sentence: false,
            ..live::Config::default()
        };
        let mut s = Session::new(config);
        type_ja(&mut s, "kyou");
        assert_eq!(s.snapshot().preedit, "今日");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}
