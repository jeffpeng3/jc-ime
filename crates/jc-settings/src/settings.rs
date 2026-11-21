//! 使用者可切換的引擎設定：資料模型、驗證、檔案格式與版本遷移。
//!
//! 對照前身 `internal/settings/settings.go`。引擎只在啟動時讀取；欄位缺漏時
//! 使用預設值。檔案前後相容：本版不認得的欄位與已知欄位的未知值都原文保留到
//! 下次存檔，只有讀不懂的那個欄位退回預設；schema 比本版新時只讀不寫。
//!
//! 平台（`goos`）只影響預設值與幾項衍生行為，呼叫端必須自己傳入，本 crate 不
//! 讀任何作業系統 API。

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use jc_core::zh::fuzzy;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::atomic_file::write_atomic;
use crate::paths;
use crate::profiles::fresh_settings;

/// 舊欄位 `selection` 的值：神經模型。
///
/// 新檔改由 [`JA_NEURAL_AUTO`]／[`JA_NEURAL_OFF`] 表示；這個值只在讀舊檔與
/// 相容投影時使用。
pub const NEURAL: &str = "neural";
/// 舊欄位 `selection` 的值：只用統計引擎。
pub const STATISTICAL: &str = "statistical";

/// 日文神經轉換：低呼叫門檻（預設）。
pub const JA_NEURAL_AUTO: &str = "auto";
/// 日文神經轉換：品質優先，只看 `gap < 460`。
pub const JA_NEURAL_QUALITY: &str = "quality";
/// 日文神經轉換：完全關閉。
pub const JA_NEURAL_OFF: &str = "off";

/// 設定檔名。
pub const FILE_NAME: &str = paths::FILE_NAME;

/// 本版寫入、也完全理解的檔案格式。
pub const SCHEMA_VERSION: i32 = 2;

/// 標準（大千）注音鍵盤排列。
pub const LAYOUT_STANDARD: &str = "standard";
/// 倚天 41 鍵。
pub const LAYOUT_ETEN: &str = "eten";
/// 許氏。
pub const LAYOUT_HSU: &str = "hsu";

/// 注音（預設）。
pub const LANGUAGE_ZH: &str = "zh";
/// 日文。
pub const LANGUAGE_JA: &str = "ja";

/// 繁體中文（預設）。
pub const VARIANT_TRADITIONAL: &str = "traditional";
/// 簡體中文。
pub const VARIANT_SIMPLIFIED: &str = "simplified";
/// 日文新字體。
pub const VARIANT_SHINJITAI: &str = "shinjitai";

/// 羅馬字（預設）。
pub const JA_INPUT_ROMAJI: &str = "romaji";
/// 直接かな。
pub const JA_INPUT_KANA: &str = "kana";

/// 日文逐字変換（預設）：長度／邊界／停頓觸發，統計優先，停頓後補正。
pub const JA_CONVERSION_LIVE: &str = "live";
/// 日文整句変換：打完才轉，行為與舊版相同。
pub const JA_CONVERSION_SENTENCE: &str = "sentence";

/// かな／漢字與英文並列（預設）。
pub const JA_AMBIGUOUS_BOTH: &str = "both";
/// 詞典英文詞優先。
pub const JA_AMBIGUOUS_ENGLISH: &str = "english";
/// 合法羅馬字優先。
pub const JA_AMBIGUOUS_KANA: &str = "kana";

/// 主要語言先載入，其他語言第一次用到時才載入（預設）。
pub const MODEL_MODE_AUTO: &str = "auto";
/// 所有模型都載入。
pub const MODEL_MODE_ALL: &str = "all";
/// 只載入主要語言。
pub const MODEL_MODE_PRIMARY: &str = "primary";
/// 不載入模型，一律使用統計轉換。
pub const MODEL_MODE_NONE: &str = "none";

/// `model_residency_mb=0`：依實際載入的語言數自動選擇。
pub const MODEL_RESIDENCY_AUTO_MB: i64 = 0;
/// 單語言預算。
pub const MODEL_RESIDENCY_SINGLE_MB: i64 = 250;
/// 雙語言預算。
pub const MODEL_RESIDENCY_DUAL_MB: i64 = 400;
/// 預算上限。
pub const MODEL_RESIDENCY_MAX_MB: i64 = 4096;
/// 命令列與設定介面的自動值。
pub const MODEL_RESIDENCY_AUTO_VALUE: &str = "auto";

/// 沒有任何設定時的模糊音模式。
pub const DEFAULT_FUZZY: &str = "select";

/// 選字鍵：數字排（預設）。
pub const SELECT_DIGITS: &str = "1234567890";
/// 選字鍵：`1234qweras`。
pub const SELECT_QWER: &str = "1234qweras";
/// 選字鍵：`asdfghjkl;`。
pub const SELECT_HOME: &str = "asdfghjkl;";

/// 單按右 Shift 切換。
pub const TOGGLE_RIGHT: &str = "right";
/// 左右 Shift 都切換。
pub const TOGGLE_BOTH: &str = "both";
/// 單按左 Shift 切換。
pub const TOGGLE_LEFT: &str = "left";
/// 不用 Shift 切換。
pub const TOGGLE_NONE: &str = "none";

/// 照平台原本的做法記住中英模式。
pub const MODE_NATIVE: &str = "native";
/// 全部共用同一個中英模式。
pub const MODE_SHARED: &str = "shared";

/// 開始時是注音模式。
pub const START_ZHUYIN: &str = "zhuyin";
/// 開始時是英文模式。
pub const START_ENGLISH: &str = "english";

/// 已拿掉的設定檔欄位：讀檔時忽略，存檔時不寫回。
const RETIRED_KEYS: [&str; 4] = [
    "preview_0_2",
    "language_memory",
    "language_hotkey",
    "english_profile",
];

/// 讀取結果的檔案狀態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    /// 讀取成功。
    Ok,
    /// 檔案不存在（含舊路徑也不存在）。
    Missing,
    /// 檔案不是可解析的 JSON 物件。
    Corrupt,
    /// 檔案存在但讀不到。
    Unreadable,
}

impl FileState {
    /// 機器可讀的代號。
    pub fn as_str(self) -> &'static str {
        match self {
            FileState::Ok => "ok",
            FileState::Missing => "missing",
            FileState::Corrupt => "corrupt",
            FileState::Unreadable => "unreadable",
        }
    }
}

/// 設定驗證錯誤；`Set` 回傳的代碼供機器判斷。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SetError {
    /// 未知的設定鍵。
    #[error("未知的設定 {key:?}；可用：{available}")]
    UnknownKey {
        /// 呼叫端傳入的鍵。
        key: String,
        /// 可用鍵的清單。
        available: String,
    },
    /// 值不在允許清單內。
    #[error("{key} 只接受 {values}")]
    InvalidValue {
        /// 設定鍵。
        key: String,
        /// 允許值。
        values: String,
    },
    /// 此項由選用的輸入習慣固定。
    #[error("此項由選用的輸入習慣固定；請先複製成自訂")]
    LockedByProfile {
        /// 設定鍵。
        key: String,
    },
    /// 其他給人讀的錯誤。
    #[error("{0}")]
    Message(String),
}

/// [`SetError`] 的機器可讀代碼。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetCode {
    /// 未知的設定鍵。
    UnknownKey,
    /// 值不合法。
    InvalidValue,
    /// 由輸入習慣固定。
    LockedByProfile,
    /// 輸入習慣本身的操作錯誤。
    Profile,
}

impl SetCode {
    /// 機器可讀的代號。
    pub fn as_str(self) -> &'static str {
        match self {
            SetCode::UnknownKey => "unknown_key",
            SetCode::InvalidValue => "invalid_value",
            SetCode::LockedByProfile => "locked_by_profile",
            SetCode::Profile => "profile",
        }
    }
}

impl SetError {
    /// 機器可讀代碼。
    pub fn code(&self) -> SetCode {
        match self {
            SetError::UnknownKey { .. } => SetCode::UnknownKey,
            SetError::InvalidValue { .. } => SetCode::InvalidValue,
            SetError::LockedByProfile { .. } => SetCode::LockedByProfile,
            SetError::Message(_) => SetCode::Profile,
        }
    }

    /// 相關的設定鍵；一般訊息沒有鍵時回傳空字串。
    pub fn key(&self) -> &str {
        match self {
            SetError::UnknownKey { key, .. }
            | SetError::InvalidValue { key, .. }
            | SetError::LockedByProfile { key } => key,
            SetError::Message(_) => "",
        }
    }
}

/// 設定操作錯誤。
#[derive(Debug, Error)]
pub enum SettingsError {
    /// 設定檔來自較新版本，本版不覆寫。
    #[error("設定檔來自較新版本，本版只讀不寫")]
    ReadOnly,
    /// 設定檔不是可解析的 JSON 物件。
    #[error("{}：設定檔無法解析：{detail}", path.display())]
    Corrupt {
        /// 讀取的路徑。
        path: PathBuf,
        /// 解析器的訊息。
        detail: String,
    },
    /// 單一設定值的驗證錯誤。
    #[error(transparent)]
    Set(#[from] SetError),
    /// 檔案系統錯誤。
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// 各組模糊音的啟用方式。
///
/// 欄位缺漏視為預設 `select`；值不認得時只有那一組當作 `off`。本版不認得的
/// 成員與型別不同的整個欄位原文保留到下次存檔。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fuzzy {
    /// ㄗ／ㄓ。
    pub z_zh: String,
    /// ㄘ／ㄔ。
    pub c_ch: String,
    /// ㄙ／ㄕ。
    pub s_sh: String,
    /// ㄣ／ㄥ。
    pub en_eng: String,
    /// 本版不認得、存檔要原樣寫回的成員。
    extra: BTreeMap<String, Value>,
    /// `fuzzy` 欄位不是物件時的原文。
    other: Option<Value>,
}

impl Fuzzy {
    fn field(&self, pair: fuzzy::Pair) -> &str {
        match pair {
            fuzzy::Pair::ZZh => &self.z_zh,
            fuzzy::Pair::CCh => &self.c_ch,
            fuzzy::Pair::SSh => &self.s_sh,
            fuzzy::Pair::EnEng => &self.en_eng,
        }
    }

    fn field_mut(&mut self, pair: fuzzy::Pair) -> &mut String {
        match pair {
            fuzzy::Pair::ZZh => &mut self.z_zh,
            fuzzy::Pair::CCh => &mut self.c_ch,
            fuzzy::Pair::SSh => &mut self.s_sh,
            fuzzy::Pair::EnEng => &mut self.en_eng,
        }
    }

    fn from_value(value: Value) -> Self {
        let mut fuzzy = Fuzzy::default();
        let Value::Object(members) = value else {
            fuzzy.other = Some(value);
            return fuzzy;
        };
        let mut members: BTreeMap<String, Value> = members.into_iter().collect();
        for pair in fuzzy::pairs() {
            if let Some(raw) = members.remove(pair.id()) {
                match raw.as_str() {
                    Some(text) => *fuzzy.field_mut(pair) = text.to_string(),
                    None => {
                        members.insert(pair.id().to_string(), raw);
                    }
                }
            }
        }
        fuzzy.extra = members;
        fuzzy
    }

    /// 寫入一組配對；明確設定會蓋掉先前保留的非字串原文。
    fn set(&mut self, pair: fuzzy::Pair, value: &str) {
        *self.field_mut(pair) = value.to_string();
        self.other = None;
        self.extra.remove(pair.id());
    }

    /// 這組配對在檔案裡是否有本版讀不懂的值。
    fn has_member(&self, pair: fuzzy::Pair) -> bool {
        self.extra.contains_key(pair.id())
    }

    /// 是否含本版不認得、存檔時原樣保留的內容。
    pub fn has_unknown(&self) -> bool {
        !self.extra.is_empty() || self.other.is_some()
    }

    /// 轉成核心使用的模糊音設定。
    pub fn config(&self) -> fuzzy::Config {
        let mut config = fuzzy::Config::new();
        if self.other.is_some() {
            return config;
        }
        for pair in fuzzy::pairs() {
            let mut value = self.field(pair);
            if value.is_empty() && !self.has_member(pair) {
                value = DEFAULT_FUZZY;
            }
            if let Some(mode) = fuzzy::Mode::parse(value) {
                config = config.with(pair, mode);
            }
        }
        config
    }
}

impl Serialize for Fuzzy {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut members: BTreeMap<String, Value> = self.extra.clone();
        for pair in fuzzy::pairs() {
            let value = self.field(pair);
            if !value.is_empty() {
                members.insert(pair.id().to_string(), Value::String(value.to_string()));
            }
        }
        if members.is_empty() {
            if let Some(other) = self.other.as_ref() {
                return other.serialize(serializer);
            }
        }
        members.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Fuzzy {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Fuzzy::from_value(Value::deserialize(deserializer)?))
    }
}

/// 使用者可切換的引擎設定。
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// 舊欄位：`neural` 或 `statistical`；由 [`Settings::ja_neural_mode`] 推出，
    /// 讀舊檔時反過來決定新欄位。
    pub selection: String,
    /// 提交時補標點。
    pub punctuation: bool,
    /// 依自己剛提交的文字選字。
    pub document_context: bool,
    /// 中英混打。
    pub mixed_input: bool,
    /// 英文模式自動補字。
    pub english_autocomplete: bool,
    /// 舊欄位：左 Shift 是否切換；由 [`Settings::toggle_key`] 推出。
    pub left_shift: bool,
    /// 中英切換鍵：`right`、`both`、`left`、`none`。
    pub toggle_key: String,
    /// 切換視窗後的中英模式：`native` 或 `shared`。
    pub mode_memory: String,
    /// 開始時的中英模式：`zhuyin`（預設）或 `english`。
    pub start_mode: String,
    /// 按 Esc 清除整段組字。
    pub esc_clear: bool,
    /// 空白鍵開候選（預設）。
    pub space_selects: bool,
    /// 從候選窗選字後游標移到所選詞後面。
    pub advance_after_select: bool,
    /// 修改字詞的位置：`before` 或 `after`。
    pub candidate_cursor: String,
    /// 選字排列：`vertical`、`grid`、`horizontal`。
    pub candidate_layout: String,
    /// Emoji 選單：`off` 或 `equals`。
    pub emoji_shortcut: String,
    /// 將 `=` 放在 Emoji 選單第一項。
    pub emoji_equals_first: bool,
    /// 注音鍵盤排列。
    pub keyboard_layout: String,
    /// 送出時的字形慣例。
    pub output_variant: String,
    /// 用快捷鍵切換輸出簡體字。
    pub simplified_hotkey: bool,
    /// 候選窗的選字鍵。
    pub selection_keys: String,
    /// Ctrl 加標點打全形標點。
    pub ctrl_punctuation: bool,
    /// Shift 加符號打半形。
    pub shift_symbol_halfwidth: bool,
    /// 所有符號鍵打半形；只在 [`Settings::shift_symbol_halfwidth`] 也是
    /// true 時成立。
    pub all_symbols_halfwidth: bool,
    /// 送出時在漢字與英文字母相連的地方補半形空格。
    pub cjk_latin_space: bool,
    /// 打錯注音時提示。
    pub zhuyin_error_hint: bool,
    /// Caps Lock 開著時直接打英文。
    pub caps_lock_english: bool,
    /// 不打聲調也能選字。
    pub toneless: bool,
    /// 各組模糊音。
    pub fuzzy: Fuzzy,
    /// 日文輸入方式。
    pub ja_input_method: String,
    /// 日英同形時的優先順序。
    pub ja_ambiguous: String,
    /// 日文変換方式：`live`（預設，逐字変換）或 `sentence`（整句変換）。
    pub ja_conversion: String,
    /// 舊欄位投影：`ja_conversion == live`。讀舊檔與既有的 on/off 介面用。
    pub ja_live_conversion: bool,
    /// 日文轉換候選列出記号與顔文字。
    pub ja_symbols: bool,
    /// 日文選字寫入日文個人詞庫。
    pub ja_learning: bool,
    /// 日文候選使用神經模型重排。
    pub ja_model: bool,
    /// 日文神經轉換模式：`auto`（預設）／`quality`／`off`。
    pub ja_neural_mode: String,
    /// 日文候選使用 reranker 重排；預設關閉，可手動開。
    pub ja_rerank: bool,
    /// 模型常駐模式。
    pub model_mode: String,
    /// 主要語言：`zh`（預設）或 `ja`。
    pub primary_language: String,
    /// 模型常駐預算（MB）；0 表示自動。
    pub model_residency_mb: i64,
    /// 允許載入 placeholder 模型。
    pub allow_placeholders: bool,

    /// 本版沒有解讀、但存檔要原樣寫回的成員。
    extra: BTreeMap<String, Value>,
    /// `input_profile` 的內容；存檔時原樣寫回。
    pub(crate) profiles: Option<Value>,
    /// 舊欄位 `selection` 與 `ja_neural_mode` 同時出現且語意不同。
    pub(crate) selection_conflict: bool,
    /// 檔案的 schema 比本版新，或不是整數，所以不覆寫。
    pub(crate) read_only: bool,
    pub(crate) profile_invalid: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self::default_for_platform("linux")
    }
}

type BoolSetter = fn(&mut Settings, bool);
type StringSetter = fn(&mut Settings, &str);

const BOOL_FIELDS: [(&str, BoolSetter); 23] = [
    ("punctuation", |s, v| s.punctuation = v),
    ("document_context", |s, v| s.document_context = v),
    ("mixed_input", |s, v| s.mixed_input = v),
    ("english_autocomplete", |s, v| s.english_autocomplete = v),
    ("left_shift", |s, v| s.left_shift = v),
    ("esc_clear", |s, v| s.esc_clear = v),
    ("space_selects", |s, v| s.space_selects = v),
    ("advance_after_select", |s, v| s.advance_after_select = v),
    ("emoji_equals_first", |s, v| s.emoji_equals_first = v),
    ("simplified_hotkey", |s, v| s.simplified_hotkey = v),
    ("ctrl_punctuation", |s, v| s.ctrl_punctuation = v),
    ("shift_symbol_halfwidth", |s, v| {
        s.shift_symbol_halfwidth = v
    }),
    ("all_symbols_halfwidth", |s, v| s.all_symbols_halfwidth = v),
    ("cjk_latin_space", |s, v| s.cjk_latin_space = v),
    ("zhuyin_error_hint", |s, v| s.zhuyin_error_hint = v),
    ("caps_lock_english", |s, v| s.caps_lock_english = v),
    ("toneless", |s, v| s.toneless = v),
    ("ja_live_conversion", |s, v| {
        s.ja_live_conversion = v;
        s.ja_conversion = if v {
            JA_CONVERSION_LIVE
        } else {
            JA_CONVERSION_SENTENCE
        }
        .to_string();
    }),
    ("ja_symbols", |s, v| s.ja_symbols = v),
    ("ja_learning", |s, v| s.ja_learning = v),
    ("ja_model", |s, v| s.ja_model = v),
    ("ja_rerank", |s, v| s.ja_rerank = v),
    ("allow_placeholders", |s, v| s.allow_placeholders = v),
];

const STRING_FIELDS: [(&str, StringSetter, &[&str]); 15] = [
    (
        "candidate_cursor",
        |s, v| s.candidate_cursor = v.to_string(),
        &["before", "after"],
    ),
    (
        "candidate_layout",
        |s, v| s.candidate_layout = v.to_string(),
        &["vertical", "grid", "horizontal"],
    ),
    (
        "emoji_shortcut",
        |s, v| s.emoji_shortcut = v.to_string(),
        &["off", "equals"],
    ),
    (
        "keyboard_layout",
        |s, v| s.keyboard_layout = v.to_string(),
        &[LAYOUT_STANDARD, LAYOUT_ETEN, LAYOUT_HSU],
    ),
    (
        "selection_keys",
        |s, v| s.selection_keys = v.to_string(),
        &[SELECT_DIGITS, SELECT_QWER, SELECT_HOME],
    ),
    (
        "toggle_key",
        |s, v| s.toggle_key = v.to_string(),
        &[TOGGLE_RIGHT, TOGGLE_BOTH, TOGGLE_LEFT, TOGGLE_NONE],
    ),
    (
        "mode_memory",
        |s, v| s.mode_memory = v.to_string(),
        &[MODE_NATIVE, MODE_SHARED],
    ),
    (
        "start_mode",
        |s, v| s.start_mode = v.to_string(),
        &[START_ZHUYIN, START_ENGLISH],
    ),
    (
        "ja_input_method",
        |s, v| s.ja_input_method = v.to_string(),
        &[JA_INPUT_ROMAJI, JA_INPUT_KANA],
    ),
    (
        "ja_ambiguous",
        |s, v| s.ja_ambiguous = v.to_string(),
        &[JA_AMBIGUOUS_BOTH, JA_AMBIGUOUS_ENGLISH, JA_AMBIGUOUS_KANA],
    ),
    (
        "ja_conversion",
        |s, v| s.ja_conversion = v.to_string(),
        &[JA_CONVERSION_LIVE, JA_CONVERSION_SENTENCE],
    ),
    (
        "ja_neural_mode",
        |s, v| s.ja_neural_mode = v.to_string(),
        &[JA_NEURAL_AUTO, JA_NEURAL_QUALITY, JA_NEURAL_OFF],
    ),
    (
        "model_mode",
        |s, v| s.model_mode = v.to_string(),
        &[
            MODEL_MODE_AUTO,
            MODEL_MODE_ALL,
            MODEL_MODE_PRIMARY,
            MODEL_MODE_NONE,
        ],
    ),
    (
        "primary_language",
        |s, v| s.primary_language = v.to_string(),
        &[LANGUAGE_ZH, LANGUAGE_JA],
    ),
    (
        "output_variant",
        |s, v| s.output_variant = v.to_string(),
        &[VARIANT_TRADITIONAL, VARIANT_SIMPLIFIED, VARIANT_SHINJITAI],
    ),
];

/// 平台預設的中英切換鍵。
pub fn default_toggle_key(goos: &str) -> &'static str {
    match goos {
        "darwin" => TOGGLE_NONE,
        "windows" => TOGGLE_BOTH,
        _ => TOGGLE_RIGHT,
    }
}

/// 平台預設的中英模式記憶方式。
pub fn default_mode_memory(goos: &str) -> &'static str {
    if goos == "darwin" {
        MODE_SHARED
    } else {
        MODE_NATIVE
    }
}

/// 平台預設的修改字詞位置。
pub fn default_candidate_cursor(goos: &str) -> &'static str {
    if goos == "darwin" {
        "before"
    } else {
        "after"
    }
}

impl Settings {
    /// 指定平台的預設值。
    pub fn default_for_platform(goos: &str) -> Self {
        let mut settings = Settings {
            selection: NEURAL.to_string(),
            punctuation: false,
            document_context: true,
            // 混打已是正式功能；Windows 注音直接進 Mixed 模式，別讓羅馬字被當注音解。
            mixed_input: goos == "windows",
            english_autocomplete: false,
            left_shift: false,
            toggle_key: default_toggle_key(goos).to_string(),
            mode_memory: default_mode_memory(goos).to_string(),
            start_mode: START_ZHUYIN.to_string(),
            esc_clear: true,
            space_selects: true,
            advance_after_select: false,
            candidate_cursor: default_candidate_cursor(goos).to_string(),
            candidate_layout: "vertical".to_string(),
            emoji_shortcut: "equals".to_string(),
            emoji_equals_first: true,
            keyboard_layout: LAYOUT_STANDARD.to_string(),
            output_variant: VARIANT_TRADITIONAL.to_string(),
            simplified_hotkey: false,
            selection_keys: SELECT_DIGITS.to_string(),
            ctrl_punctuation: true,
            shift_symbol_halfwidth: false,
            all_symbols_halfwidth: false,
            cjk_latin_space: false,
            zhuyin_error_hint: true,
            caps_lock_english: false,
            toneless: false,
            fuzzy: Fuzzy::default(),
            ja_input_method: JA_INPUT_ROMAJI.to_string(),
            ja_ambiguous: JA_AMBIGUOUS_BOTH.to_string(),
            ja_conversion: JA_CONVERSION_LIVE.to_string(),
            ja_live_conversion: true,
            ja_symbols: true,
            ja_learning: true,
            ja_model: true,
            ja_neural_mode: JA_NEURAL_AUTO.to_string(),
            ja_rerank: false,
            model_mode: MODEL_MODE_AUTO.to_string(),
            primary_language: LANGUAGE_ZH.to_string(),
            model_residency_mb: MODEL_RESIDENCY_AUTO_MB,
            allow_placeholders: false,
            extra: BTreeMap::new(),
            profiles: None,
            selection_conflict: false,
            read_only: false,
            profile_invalid: false,
        };
        settings.left_shift = settings.toggles_left();
        settings
    }

    /// 解析 JSON；`goos` 決定缺漏欄位的預設值。
    pub fn from_json_for(bytes: &[u8], goos: &str) -> Result<Self, serde_json::Error> {
        let value: Value = serde_json::from_slice(bytes)?;
        match value {
            Value::Object(members) => Ok(Settings::from_members(members, goos)),
            _ => Err(<serde_json::Error as serde::de::Error>::custom(
                "設定檔必須是 JSON 物件",
            )),
        }
    }

    fn from_members(members: Map<String, Value>, goos: &str) -> Self {
        let mut settings = Settings::default_for_platform(goos);
        let mut members = members;
        let mut kept: BTreeMap<String, Value> = BTreeMap::new();
        let has_toggle = members.contains_key("toggle_key");
        let has_primary_language = members.contains_key("primary_language");
        let has_output_variant = members.contains_key("output_variant");
        let has_ja_neural_mode = members.contains_key("ja_neural_mode");
        // 舊欄位：等新欄位讀完再遷移，兩者都在時以新欄位為準。
        let legacy_selection = members.remove("selection");

        for (key, setter) in BOOL_FIELDS {
            if let Some(raw) = members.remove(key) {
                match raw.as_bool() {
                    Some(value) => setter(&mut settings, value),
                    None => {
                        kept.insert(key.to_string(), raw);
                    }
                }
            }
        }
        // #282：舊版只認得 shift_symbol_halfwidth，讀取時兩者取 AND。
        settings.all_symbols_halfwidth =
            settings.all_symbols_halfwidth && settings.shift_symbol_halfwidth;
        for key in RETIRED_KEYS {
            members.remove(key);
        }
        if let Some(raw) = members.remove("fuzzy") {
            settings.fuzzy = Fuzzy::from_value(raw);
        }
        for (key, setter, values) in STRING_FIELDS {
            if let Some(raw) = members.remove(key) {
                match raw.as_str() {
                    Some(value) if values.contains(&value) => setter(&mut settings, value),
                    _ => {
                        kept.insert(key.to_string(), raw);
                    }
                }
            }
        }
        // 舊欄位 selection 遷移：statistical → off、neural → auto。
        // 兩個都寫且語意不同時以 ja_neural_mode 為準，只記一次警告。
        if let Some(raw) = legacy_selection {
            match raw.as_str() {
                Some(value) if value == NEURAL || value == STATISTICAL => {
                    let implied = Settings::mode_for_selection(value);
                    if has_ja_neural_mode {
                        // 兩者都在：關／不關的結論不同才算衝突，quality 與 neural
                        // 都代表啟用，不警告。
                        let selection_off = value == STATISTICAL;
                        let mode_off = settings.effective_ja_neural_mode() == JA_NEURAL_OFF;
                        if selection_off != mode_off {
                            settings.selection_conflict = true;
                        }
                    } else {
                        settings.ja_neural_mode = implied.to_string();
                    }
                }
                _ => {
                    kept.insert("selection".to_string(), raw);
                }
            }
        }
        settings.selection = settings.selection_projection().to_string();
        // 新欄位為準；舊檔只有 bool 時上面的 setter 已經投影成字串。
        settings.ja_live_conversion = settings.ja_live_enabled();
        // 舊檔沒有 primary_language 時沿用既有的 language。
        if let Some(legacy) = members.remove("language") {
            if !has_primary_language
                && legacy
                    .as_str()
                    .is_some_and(|value| value == LANGUAGE_ZH || value == LANGUAGE_JA)
            {
                if let Some(value) = legacy.as_str() {
                    settings.primary_language = value.to_string();
                }
            }
        }
        // 舊的輸出簡體開關換成輸出變體；兩者都有時以新欄位為準。
        if let Some(legacy) = members.remove("output_simplified") {
            if !has_output_variant && legacy.as_bool() == Some(true) {
                settings.output_variant = VARIANT_SIMPLIFIED.to_string();
            }
        }
        if let Some(raw) = members.remove("model_residency_mb") {
            match raw.as_i64() {
                Some(mb) if (0..=MODEL_RESIDENCY_MAX_MB).contains(&mb) => {
                    settings.model_residency_mb = mb;
                }
                _ => {
                    kept.insert("model_residency_mb".to_string(), raw);
                }
            }
        }
        // 舊檔只有 left_shift：推出切換鍵。新檔以 toggle_key 為準。
        if !has_toggle && settings.left_shift {
            settings.toggle_key = TOGGLE_BOTH.to_string();
        }
        settings.left_shift = settings.toggles_left();
        if let Some(raw) = members.remove("schema") {
            match raw.as_i64() {
                Some(number) if number <= i64::from(SCHEMA_VERSION) => {}
                _ => settings.read_only = true,
            }
        }
        if let Some(raw) = members.remove("input_profile") {
            settings.load_profiles(raw);
        }
        kept.extend(members);
        settings.extra = kept;
        settings
    }

    /// 主要語言的相容投影。
    pub fn language(&self) -> &str {
        &self.primary_language
    }

    /// `output_variant` 的相容投影。
    pub fn output_simplified(&self) -> bool {
        self.effective_output_variant() == VARIANT_SIMPLIFIED
    }

    /// 主要語言是不是日文。
    pub fn japanese(&self) -> bool {
        self.primary_language == LANGUAGE_JA
    }

    /// 收斂過的日英同形優先順序。
    pub fn effective_ja_ambiguous(&self) -> &str {
        match self.ja_ambiguous.as_str() {
            JA_AMBIGUOUS_ENGLISH | JA_AMBIGUOUS_KANA => &self.ja_ambiguous,
            _ => JA_AMBIGUOUS_BOTH,
        }
    }

    /// 收斂過的日文変換方式；不認得的值退回 `live`。
    pub fn effective_ja_conversion(&self) -> &str {
        match self.ja_conversion.as_str() {
            JA_CONVERSION_SENTENCE => JA_CONVERSION_SENTENCE,
            _ => JA_CONVERSION_LIVE,
        }
    }

    /// 日文是否走逐字変換；舊欄位與新欄位的共用入口。
    pub fn ja_live_enabled(&self) -> bool {
        self.effective_ja_conversion() == JA_CONVERSION_LIVE
    }

    /// 收斂過的日文神經轉換模式；不認得的值退回 `auto`。
    pub fn effective_ja_neural_mode(&self) -> &str {
        match self.ja_neural_mode.as_str() {
            JA_NEURAL_QUALITY | JA_NEURAL_OFF => &self.ja_neural_mode,
            _ => JA_NEURAL_AUTO,
        }
    }

    /// `selection` 的舊值所對應的神經模式。
    fn mode_for_selection(value: &str) -> &'static str {
        if value == STATISTICAL {
            JA_NEURAL_OFF
        } else {
            JA_NEURAL_AUTO
        }
    }

    /// `selection` 的相容投影：關閉時是 `statistical`，其餘是 `neural`。
    fn selection_projection(&self) -> &'static str {
        if self.effective_ja_neural_mode() == JA_NEURAL_OFF {
            STATISTICAL
        } else {
            NEURAL
        }
    }

    /// 是否只有明確選かな時才優先かな。
    pub fn ja_ambiguous_kana_first(&self) -> bool {
        self.effective_ja_ambiguous() == JA_AMBIGUOUS_KANA
    }

    /// 是否使用日英並列。
    pub fn ja_ambiguous_lists_both(&self) -> bool {
        self.effective_ja_ambiguous() == JA_AMBIGUOUS_BOTH
    }

    /// 收斂過的輸出變體。
    pub fn effective_output_variant(&self) -> &str {
        match self.output_variant.as_str() {
            VARIANT_SIMPLIFIED | VARIANT_SHINJITAI => &self.output_variant,
            _ => VARIANT_TRADITIONAL,
        }
    }

    /// 收斂過的主要語言。
    pub fn effective_primary_language(&self) -> &str {
        if self.primary_language == LANGUAGE_JA {
            LANGUAGE_JA
        } else {
            LANGUAGE_ZH
        }
    }

    /// 開始時是否為英文模式；沒有 Shift 能切回注音時一律是注音。
    pub fn starts_english(&self, goos: &str) -> bool {
        self.start_mode == START_ENGLISH && !self.start_mode_locked(goos)
    }

    /// 實際使用的開始模式。
    pub fn effective_start_mode(&self, goos: &str) -> &'static str {
        if self.starts_english(goos) {
            START_ENGLISH
        } else {
            START_ZHUYIN
        }
    }

    /// 中英切換鍵不用 Shift、這個平台又沒有其他切回注音的方法時固定從注音開始。
    pub fn start_mode_locked(&self, goos: &str) -> bool {
        self.toggle_key == TOGGLE_NONE && goos != "windows"
    }

    /// 單按左 Shift 是否切換。
    pub fn toggles_left(&self) -> bool {
        self.toggle_key == TOGGLE_BOTH || self.toggle_key == TOGGLE_LEFT
    }

    /// 單按右 Shift 是否切換。
    pub fn toggles_right(&self) -> bool {
        self.toggle_key == TOGGLE_BOTH || self.toggle_key == TOGGLE_RIGHT
    }

    fn set_toggle_key(&mut self, value: &str) {
        self.toggle_key = value.to_string();
        self.left_shift = self.toggles_left();
    }

    /// 模型常駐預算（MB）；`dual` 表示同時載入兩個以上語言。
    pub fn model_budget_mb(&self, dual: bool) -> i64 {
        if self.model_residency_mb > 0 {
            return self.model_residency_mb;
        }
        if dual {
            MODEL_RESIDENCY_DUAL_MB
        } else {
            MODEL_RESIDENCY_SINGLE_MB
        }
    }

    /// 預算的位元組版本。
    pub fn model_budget_bytes(&self, dual: bool) -> i64 {
        self.model_budget_mb(dual) << 20
    }

    /// 預算來源：`configured`、`phase2-single` 或 `phase2-dual`。
    pub fn model_budget_source(&self, dual: bool) -> &'static str {
        if self.model_residency_mb > 0 {
            "configured"
        } else if dual {
            "phase2-dual"
        } else {
            "phase2-single"
        }
    }

    /// 各組模糊音的設定。
    pub fn fuzzy_config(&self) -> fuzzy::Config {
        self.fuzzy.config()
    }

    /// 標點寬度：`off`、`on` 或 `all`。
    pub fn symbol_width(&self) -> &'static str {
        if self.shift_symbol_halfwidth && self.all_symbols_halfwidth {
            "all"
        } else if self.shift_symbol_halfwidth {
            "on"
        } else {
            "off"
        }
    }

    /// 引擎實際使用的「Ctrl＋標點打全形」。
    pub fn effective_ctrl_punctuation(&self) -> bool {
        self.ctrl_punctuation || self.symbol_width() == "all"
    }

    /// 引擎實際使用的「不打聲調也能選字」。
    pub fn effective_toneless(&self) -> bool {
        self.toneless && self.symbol_width() != "all"
    }

    /// 檔案是否來自較新版本：是的話存檔會拒絕。
    pub fn read_only(&self) -> bool {
        self.read_only
    }

    /// 讀取時的退回與保留說明，給人讀。
    pub fn notes(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut kept = self.extra.clone();
        for key in SAVE_ORDER {
            if let Some(raw) = kept.remove(key) {
                out.push(format!(
                    "{}：本版不認得 {}，暫用預設值，存檔時保留原值",
                    key.replace('_', "-"),
                    compact(&raw)
                ));
            }
        }
        if !kept.is_empty() {
            let names: Vec<String> = kept.keys().cloned().collect();
            out.push(format!(
                "含本版不認得的欄位：{}；存檔時原樣保留",
                names.join("、")
            ));
        }
        if self.fuzzy.has_unknown() {
            out.push("fuzzy：含本版不認得的成員，存檔時原樣保留".to_string());
        }
        if self.selection_conflict {
            out.push(
                "ja-neural-mode 與舊欄位 selection 語意不同；以 ja-neural-mode 為準".to_string(),
            );
        }
        if self.read_only {
            out.push(SettingsError::ReadOnly.to_string());
        }
        out
    }

    pub(crate) fn forget(&mut self, key: &str) {
        self.extra.remove(key);
    }
}

/// 存檔時的固定欄位順序。
const SAVE_ORDER: [&str; 41] = [
    "selection",
    "punctuation",
    "document_context",
    "mixed_input",
    "english_autocomplete",
    "left_shift",
    "esc_clear",
    "space_selects",
    "candidate_cursor",
    "advance_after_select",
    "candidate_layout",
    "emoji_shortcut",
    "emoji_equals_first",
    "keyboard_layout",
    "selection_keys",
    "toggle_key",
    "mode_memory",
    "start_mode",
    "output_variant",
    "simplified_hotkey",
    "ctrl_punctuation",
    "shift_symbol_halfwidth",
    "all_symbols_halfwidth",
    "cjk_latin_space",
    "zhuyin_error_hint",
    "caps_lock_english",
    "toneless",
    "ja_input_method",
    "ja_ambiguous",
    "ja_conversion",
    "ja_live_conversion",
    "ja_symbols",
    "ja_learning",
    "ja_model",
    "ja_neural_mode",
    "ja_rerank",
    "model_mode",
    "primary_language",
    "model_residency_mb",
    "allow_placeholders",
    "fuzzy",
];

fn compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn put_known<M: SerializeMap>(
    map: &mut M,
    kept: &mut BTreeMap<String, Value>,
    key: &str,
    value: Value,
) -> Result<(), M::Error> {
    match kept.remove(key) {
        Some(raw) => map.serialize_entry(key, &raw),
        None => map.serialize_entry(key, &value),
    }
}

impl Serialize for Settings {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut kept = self.extra.clone();
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("schema", &SCHEMA_VERSION)?;
        let text = |value: &str| Value::String(value.to_string());
        put_known(
            &mut map,
            &mut kept,
            "selection",
            text(self.selection_projection()),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "punctuation",
            Value::Bool(self.punctuation),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "document_context",
            Value::Bool(self.document_context),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "mixed_input",
            Value::Bool(self.mixed_input),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "english_autocomplete",
            Value::Bool(self.english_autocomplete),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "left_shift",
            Value::Bool(self.left_shift),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "esc_clear",
            Value::Bool(self.esc_clear),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "space_selects",
            Value::Bool(self.space_selects),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "candidate_cursor",
            text(&self.candidate_cursor),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "advance_after_select",
            Value::Bool(self.advance_after_select),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "candidate_layout",
            text(&self.candidate_layout),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "emoji_shortcut",
            text(&self.emoji_shortcut),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "emoji_equals_first",
            Value::Bool(self.emoji_equals_first),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "keyboard_layout",
            text(&self.keyboard_layout),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "selection_keys",
            text(&self.selection_keys),
        )?;
        put_known(&mut map, &mut kept, "toggle_key", text(&self.toggle_key))?;
        put_known(&mut map, &mut kept, "mode_memory", text(&self.mode_memory))?;
        put_known(&mut map, &mut kept, "start_mode", text(&self.start_mode))?;
        put_known(
            &mut map,
            &mut kept,
            "output_variant",
            text(self.effective_output_variant()),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "simplified_hotkey",
            Value::Bool(self.simplified_hotkey),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "ctrl_punctuation",
            Value::Bool(self.ctrl_punctuation),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "shift_symbol_halfwidth",
            Value::Bool(self.shift_symbol_halfwidth),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "all_symbols_halfwidth",
            Value::Bool(self.all_symbols_halfwidth),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "cjk_latin_space",
            Value::Bool(self.cjk_latin_space),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "zhuyin_error_hint",
            Value::Bool(self.zhuyin_error_hint),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "caps_lock_english",
            Value::Bool(self.caps_lock_english),
        )?;
        put_known(&mut map, &mut kept, "toneless", Value::Bool(self.toneless))?;
        put_known(
            &mut map,
            &mut kept,
            "ja_input_method",
            text(&self.ja_input_method),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "ja_ambiguous",
            text(self.effective_ja_ambiguous()),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "ja_conversion",
            text(&self.ja_conversion),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "ja_live_conversion",
            Value::Bool(self.ja_conversion == JA_CONVERSION_LIVE),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "ja_symbols",
            Value::Bool(self.ja_symbols),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "ja_learning",
            Value::Bool(self.ja_learning),
        )?;
        put_known(&mut map, &mut kept, "ja_model", Value::Bool(self.ja_model))?;
        put_known(
            &mut map,
            &mut kept,
            "ja_neural_mode",
            text(self.effective_ja_neural_mode()),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "ja_rerank",
            Value::Bool(self.ja_rerank),
        )?;
        put_known(&mut map, &mut kept, "model_mode", text(&self.model_mode))?;
        put_known(
            &mut map,
            &mut kept,
            "primary_language",
            text(self.effective_primary_language()),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "model_residency_mb",
            Value::from(self.model_residency_mb),
        )?;
        put_known(
            &mut map,
            &mut kept,
            "allow_placeholders",
            Value::Bool(self.allow_placeholders),
        )?;
        if let Some(profiles) = self.profiles.as_ref() {
            map.serialize_entry("input_profile", profiles)?;
        }
        if self.fuzzy != Fuzzy::default() {
            map.serialize_entry("fuzzy", &self.fuzzy)?;
        }
        for (key, value) in kept {
            map.serialize_entry(&key, &value)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Settings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::Object(members) => Ok(Settings::from_members(members, "linux")),
            _ => Err(serde::de::Error::custom("設定檔必須是 JSON 物件")),
        }
    }
}

/// 讀取設定檔的完整結果。
#[derive(Debug)]
pub struct Report {
    /// 實際讀取的路徑；新舊檔都不存在時為指定的新路徑。
    pub path: PathBuf,
    /// 設定值。
    pub settings: Settings,
    /// 檔案狀態。
    pub state: FileState,
    /// 檔案中的 schema；沒有時為 0。
    pub schema: i32,
    /// 檔案來自較新版本，存檔會拒絕。
    pub read_only: bool,
    /// 讀不懂而退回預設、或原樣保留的欄位。
    pub notes: Vec<String>,
    /// 讀取失敗的原因；成功時為 `None`。
    pub error: Option<SettingsError>,
}

fn read_file(path: &Path) -> (Vec<u8>, bool, Option<io::Error>) {
    match fs::read(path) {
        Ok(bytes) => (bytes, false, None),
        Err(err) if err.kind() == io::ErrorKind::NotFound => match fs::symlink_metadata(path) {
            Err(stat_err) if stat_err.kind() == io::ErrorKind::NotFound => (Vec::new(), true, None),
            // 存在但目標遺失的符號連結：不可當成缺檔而退回舊檔。
            _ => (Vec::new(), false, Some(err)),
        },
        Err(err) => (Vec::new(), false, Some(err)),
    }
}

/// 讀取設定。
///
/// 新路徑不存在時才讀 Linux 的舊路徑；兩者都不存在回傳預設值。個別欄位讀不懂
/// 只有那個欄位退回預設，不算錯誤。
pub fn read(path: &Path, goos: &str) -> Report {
    let (mut bytes, mut missing, mut failure) = read_file(path);
    let mut actual = path.to_path_buf();
    if missing {
        if let Some(legacy) = paths::legacy_path(path) {
            let (old, old_missing, old_error) = read_file(&legacy);
            if !old_missing {
                bytes = old;
                actual = legacy;
                missing = false;
                failure = old_error;
            }
        }
    }
    if missing {
        return Report {
            path: actual,
            settings: fresh_settings(goos),
            state: FileState::Missing,
            schema: 0,
            read_only: false,
            notes: Vec::new(),
            error: None,
        };
    }
    if let Some(err) = failure {
        return Report {
            path: actual,
            settings: Settings::default_for_platform(goos),
            state: FileState::Unreadable,
            schema: 0,
            read_only: false,
            notes: Vec::new(),
            error: Some(SettingsError::Io(err)),
        };
    }
    match Settings::from_json_for(&bytes, goos) {
        Ok(settings) => {
            let read_only = settings.read_only();
            let notes = settings.notes();
            Report {
                path: actual,
                settings,
                state: FileState::Ok,
                schema: schema_of(&bytes),
                read_only,
                notes,
                error: None,
            }
        }
        Err(err) => Report {
            path: actual.clone(),
            settings: Settings::default_for_platform(goos),
            state: FileState::Corrupt,
            schema: 0,
            read_only: false,
            notes: Vec::new(),
            error: Some(SettingsError::Corrupt {
                path: actual,
                detail: err.to_string(),
            }),
        },
    }
}

fn schema_of(bytes: &[u8]) -> i32 {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| value.get("schema").and_then(Value::as_i64))
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or(0)
}

/// 讀取設定；檔案不存在時回傳預設值。
///
/// 讀不懂的個別欄位已退回預設，不算錯誤；檔案不是可解析的 JSON 物件時回傳
/// 預設值與錯誤。
pub fn load(path: &Path, goos: &str) -> (Settings, Option<SettingsError>) {
    let report = read(path, goos);
    (report.settings, report.error)
}

/// 先寫暫存檔再改名；來自較新版本的檔案回傳 [`SettingsError::ReadOnly`]。
pub fn save(path: &Path, settings: &Settings) -> Result<(), SettingsError> {
    if settings.read_only {
        return Err(SettingsError::ReadOnly);
    }
    let mut bytes = serde_json::to_vec_pretty(settings).map_err(io::Error::other)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)?;
    Ok(())
}

/// 把無法解析的設定檔改名保留（`settings.json.corrupt-<stamp>`）。
///
/// `stamp` 的格式是 `YYYYMMDDTHHMMSSZ`（UTC）。
pub fn quarantine(path: &Path, stamp: &str) -> io::Result<PathBuf> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let target = path.with_file_name(format!("{name}.corrupt-{stamp}"));
    fs::rename(path, &target)?;
    Ok(target)
}

/// 以目前時間呼叫 [`quarantine`]。
pub fn quarantine_now(path: &Path) -> io::Result<PathBuf> {
    quarantine(path, &utc_stamp(SystemTime::now()))
}

fn utc_stamp(now: SystemTime) -> String {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// 由 1970-01-01 起算的天數換算西元日期。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (if month <= 2 { y + 1 } else { y }, month as u32, day as u32)
}

/// 設定檔沒有記下中英切換鍵時寫入 `value`，回報是否寫入。
///
/// 已有 `toggle_key`、舊版的 `left_shift=true`、來自較新版本或無法解析的檔案
/// 都不動。只加這一個欄位，其他欄位與未知欄位逐字保留。
pub fn keep_toggle_key(path: &Path, value: &str) -> Result<bool, SettingsError> {
    if ![TOGGLE_RIGHT, TOGGLE_BOTH, TOGGLE_LEFT, TOGGLE_NONE].contains(&value) {
        return Err(SetError::InvalidValue {
            key: "toggle-key".to_string(),
            values: "right、both、left、none".to_string(),
        }
        .into());
    }
    let (bytes, missing, failure) = read_file(path);
    if let Some(err) = failure {
        return Err(SettingsError::Io(err));
    }
    if missing {
        if let Some(legacy) = paths::legacy_path(path) {
            if !read_file(&legacy).1 {
                // 讀的是舊路徑的設定：不在新路徑另寫一份把它蓋掉。
                return Ok(false);
            }
        }
    }
    let entry = serde_json::to_string(value).unwrap_or_default();
    let out = if missing {
        format!("{{\n  \"schema\": {SCHEMA_VERSION},\n  \"toggle_key\": {entry}\n}}\n").into_bytes()
    } else {
        let members: Map<String, Value> =
            serde_json::from_slice(&bytes).map_err(|_| corrupt(path))?;
        let parsed = Settings::from_json_for(&bytes, "linux").map_err(|_| corrupt(path))?;
        let left_shift_true = members.get("left_shift") == Some(&Value::Bool(true));
        if members.contains_key("toggle_key") || parsed.read_only() || left_shift_true {
            return Ok(false);
        }
        let mut body = bytes.clone();
        while body
            .last()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        {
            body.pop();
        }
        if body.last() != Some(&b'}') {
            return Err(corrupt(path));
        }
        body.pop();
        let separator = if members.is_empty() { "" } else { "," };
        body.extend_from_slice(format!("{separator}\n  \"toggle_key\": {entry}\n}}\n").as_bytes());
        let check: Map<String, Value> = serde_json::from_slice(&body).map_err(|_| corrupt(path))?;
        if check.len() != members.len() + 1 {
            return Err(corrupt(path));
        }
        body
    };
    write_atomic(path, &out)?;
    Ok(true)
}

fn corrupt(path: &Path) -> SettingsError {
    SettingsError::Corrupt {
        path: path.to_path_buf(),
        detail: "不是 JSON 物件".to_string(),
    }
}

/// 設定值的讀取方式。
#[derive(Clone, Copy)]
pub(crate) enum Get {
    /// 一般欄位。
    Field(fn(&Settings) -> String),
    /// 一組模糊音。
    Fuzzy(fuzzy::Pair),
    /// 輸入習慣的規則；規則與平台無關，固定用 Linux 的預設目錄。
    Rule(fn(&jc_core::inputstyle::Rules) -> String),
}

/// 一個可修改的設定鍵與它的目前值。
#[derive(Clone)]
pub(crate) struct Spec {
    pub(crate) key: String,
    pub(crate) values: &'static [&'static str],
    pub(crate) get: Get,
}

impl Spec {
    pub(crate) fn value(&self, settings: &Settings) -> String {
        match self.get {
            Get::Field(get) => get(settings),
            Get::Fuzzy(pair) => settings.fuzzy_config().mode(pair).as_str().to_string(),
            Get::Rule(get) => get(&settings.profile_rules("linux")),
        }
    }
}

fn spec(key: &str, values: &'static [&'static str], get: fn(&Settings) -> String) -> Spec {
    Spec {
        key: key.to_string(),
        values,
        get: Get::Field(get),
    }
}

pub(crate) fn specs() -> Vec<Spec> {
    let mut out = vec![
        spec("selection", &[NEURAL, STATISTICAL], |s| s.selection.clone()),
        spec("punctuation", &["on", "off"], |s| on_off(s.punctuation)),
        spec("document-context", &["on", "off"], |s| {
            on_off(s.document_context)
        }),
        spec("mixed-input", &["on", "off"], |s| on_off(s.mixed_input)),
        spec("english-autocomplete", &["on", "off"], |s| {
            on_off(s.english_autocomplete)
        }),
        spec(
            "toggle-key",
            &[TOGGLE_RIGHT, TOGGLE_BOTH, TOGGLE_LEFT, TOGGLE_NONE],
            |s| s.toggle_key.clone(),
        ),
        spec("start-mode", &[START_ZHUYIN, START_ENGLISH], |s| {
            s.start_mode.clone()
        }),
        spec("mode-memory", &[MODE_NATIVE, MODE_SHARED], |s| {
            s.mode_memory.clone()
        }),
        spec("esc-clear", &["on", "off"], |s| on_off(s.esc_clear)),
        spec("space-selects", &["on", "off"], |s| on_off(s.space_selects)),
        spec("candidate-cursor", &["before", "after"], |s| {
            s.candidate_cursor.clone()
        }),
        spec("advance-after-select", &["on", "off"], |s| {
            on_off(s.advance_after_select)
        }),
        spec(
            "candidate-layout",
            &["vertical", "grid", "horizontal"],
            |s| s.candidate_layout.clone(),
        ),
        spec(
            "output-variant",
            &[VARIANT_TRADITIONAL, VARIANT_SIMPLIFIED, VARIANT_SHINJITAI],
            |s| s.effective_output_variant().to_string(),
        ),
        spec("simplified-hotkey", &["on", "off"], |s| {
            on_off(s.simplified_hotkey)
        }),
        spec("emoji-shortcut", &["off", "equals"], |s| {
            s.emoji_shortcut.clone()
        }),
        spec("emoji-equals-first", &["on", "off"], |s| {
            on_off(s.emoji_equals_first)
        }),
        spec(
            "keyboard-layout",
            &[LAYOUT_STANDARD, LAYOUT_ETEN, LAYOUT_HSU],
            |s| s.keyboard_layout.clone(),
        ),
        spec(
            "selection-keys",
            &[SELECT_DIGITS, SELECT_QWER, SELECT_HOME],
            |s| s.selection_keys.clone(),
        ),
        spec("ctrl-punctuation", &["on", "off"], |s| {
            on_off(s.ctrl_punctuation)
        }),
        spec("caps-lock-english", &["on", "off"], |s| {
            on_off(s.caps_lock_english)
        }),
        spec("toneless", &["on", "off"], |s| on_off(s.toneless)),
        spec("shift-symbol-halfwidth", &["off", "on", "all"], |s| {
            s.symbol_width().to_string()
        }),
        spec("cjk-latin-space", &["on", "off"], |s| {
            on_off(s.cjk_latin_space)
        }),
        spec("zhuyin-error-hint", &["on", "off"], |s| {
            on_off(s.zhuyin_error_hint)
        }),
    ];
    for pair in fuzzy::pairs() {
        out.push(Spec {
            key: format!("fuzzy-{}", pair.id()),
            values: &["off", "select", "auto"],
            get: Get::Fuzzy(pair),
        });
    }
    for rule in crate::profile_rules::rule_specs() {
        out.push(Spec {
            key: rule.key.to_string(),
            values: rule.values,
            get: Get::Rule(rule.get),
        });
    }
    out
}

pub(crate) fn language_specs() -> Vec<Spec> {
    vec![
        spec("ja-input-method", &[JA_INPUT_ROMAJI, JA_INPUT_KANA], |s| {
            s.ja_input_method.clone()
        }),
        spec(
            "ja-ambiguous",
            &[JA_AMBIGUOUS_BOTH, JA_AMBIGUOUS_ENGLISH, JA_AMBIGUOUS_KANA],
            |s| s.effective_ja_ambiguous().to_string(),
        ),
        spec("ja-conversion", &["live", "sentence"], |s| {
            s.effective_ja_conversion().to_string()
        }),
        spec("ja-live-conversion", &["on", "off"], |s| {
            on_off(s.effective_ja_conversion() == JA_CONVERSION_LIVE)
        }),
        spec("ja-symbols", &["on", "off"], |s| on_off(s.ja_symbols)),
        spec("ja-learning", &["on", "off"], |s| on_off(s.ja_learning)),
        spec("ja-model", &["on", "off"], |s| on_off(s.ja_model)),
        spec(
            "ja-neural-mode",
            &[JA_NEURAL_AUTO, JA_NEURAL_QUALITY, JA_NEURAL_OFF],
            |s| s.effective_ja_neural_mode().to_string(),
        ),
        spec("ja-rerank", &["on", "off"], |s| on_off(s.ja_rerank)),
    ]
}

pub(crate) fn model_specs() -> Vec<Spec> {
    vec![
        spec(
            "model-mode",
            &[
                MODEL_MODE_AUTO,
                MODEL_MODE_ALL,
                MODEL_MODE_PRIMARY,
                MODEL_MODE_NONE,
            ],
            |s| s.model_mode.clone(),
        ),
        spec("primary-language", &[LANGUAGE_ZH, LANGUAGE_JA], |s| {
            s.effective_primary_language().to_string()
        }),
        spec(
            "model-residency-mb",
            &[MODEL_RESIDENCY_AUTO_VALUE, "250", "400", "512", "1024"],
            |s| {
                if s.model_residency_mb == MODEL_RESIDENCY_AUTO_MB {
                    MODEL_RESIDENCY_AUTO_VALUE.to_string()
                } else {
                    s.model_residency_mb.to_string()
                }
            },
        ),
        spec("allow-placeholders", &["on", "off"], |s| {
            on_off(s.allow_placeholders)
        }),
    ]
}

pub(crate) fn hidden_specs() -> Vec<Spec> {
    vec![
        spec("left-shift", &["on", "off"], |s| on_off(s.toggles_left())),
        spec("language", &[LANGUAGE_ZH, LANGUAGE_JA], |s| {
            s.effective_primary_language().to_string()
        }),
        spec("output-simplified", &["on", "off"], |s| {
            on_off(s.output_simplified())
        }),
        spec("preview-0-2", &["on"], |_| "on".to_string()),
    ]
}

fn lookup(key: &str) -> Option<Spec> {
    specs()
        .into_iter()
        .chain(language_specs())
        .chain(model_specs())
        .chain(hidden_specs())
        .find(|spec| spec.key == key)
}

pub(crate) fn on_off(value: bool) -> String {
    if value {
        "on".to_string()
    } else {
        "off".to_string()
    }
}

/// 列出可用 `set` 修改的項目與允許值，供命令列說明使用。
pub fn keys() -> BTreeMap<String, String> {
    specs()
        .into_iter()
        .chain(language_specs())
        .chain(model_specs())
        .map(|spec| (spec.key.to_string(), spec.values.join("|")))
        .collect()
}

impl Settings {
    /// 回傳某個設定目前的值。
    pub fn value(&self, key: &str) -> Option<String> {
        lookup(key).map(|spec| spec.value(self))
    }

    /// 依命令列的鍵與值修改設定。
    ///
    /// 明確設定某個欄位會丟掉先前為它保留的原文；拒絕的設定不改變內容。
    pub fn set(&mut self, key: &str, value: &str, goos: &str) -> Result<(), SettingsError> {
        let mut next = self.clone();
        next.apply_set(key, value, goos)?;
        next.validate_profile_combination(goos)?;
        *self = next;
        Ok(())
    }

    pub(crate) fn apply_set(
        &mut self,
        key: &str,
        value: &str,
        goos: &str,
    ) -> Result<(), SettingsError> {
        if self.read_only {
            return Err(SettingsError::ReadOnly);
        }
        if self.profile_locked(key, goos) {
            return Err(SetError::LockedByProfile {
                key: key.to_string(),
            }
            .into());
        }
        let Some(spec) = lookup(key) else {
            let mut names: Vec<String> = specs().into_iter().map(|spec| spec.key).collect();
            names.sort_unstable();
            return Err(SetError::UnknownKey {
                key: key.to_string(),
                available: names.join("、"),
            }
            .into());
        };
        let valid = if key == "model-residency-mb" {
            value == MODEL_RESIDENCY_AUTO_VALUE
                || value
                    .parse::<i64>()
                    .is_ok_and(|mb| (0..=MODEL_RESIDENCY_MAX_MB).contains(&mb))
        } else {
            spec.values.contains(&value)
        };
        if !valid {
            if key == "preview-0-2" {
                return Err(SetError::InvalidValue {
                    key: key.to_string(),
                    values: "on（0.2.0 的我的詞彙、加詞按鍵、我的片語與本機 API 已正式開放，不能再關閉）"
                        .to_string(),
                }
                .into());
            }
            return Err(SetError::InvalidValue {
                key: key.to_string(),
                values: spec.values.join(" 或 "),
            }
            .into());
        }
        if self.set_rule(key, value)? {
            self.sync_profile();
            return Ok(());
        }
        match key {
            "selection" => {
                self.ja_neural_mode = Settings::mode_for_selection(value).to_string();
                self.selection = self.selection_projection().to_string();
                self.selection_conflict = false;
                self.forget("selection");
                self.forget("ja_neural_mode");
            }
            "punctuation" => {
                self.punctuation = value == "on";
                self.forget("punctuation");
            }
            "document-context" => {
                self.document_context = value == "on";
                self.forget("document_context");
            }
            "english-autocomplete" => {
                self.english_autocomplete = value == "on";
                self.forget("english_autocomplete");
            }
            "left-shift" => {
                let right = self.toggles_right();
                match (value == "on", right) {
                    (true, true) => self.set_toggle_key(TOGGLE_BOTH),
                    (true, false) => self.set_toggle_key(TOGGLE_LEFT),
                    (false, true) => self.set_toggle_key(TOGGLE_RIGHT),
                    (false, false) => self.set_toggle_key(TOGGLE_NONE),
                }
                self.forget("left_shift");
                self.forget("toggle_key");
            }
            "toggle-key" => {
                self.set_toggle_key(value);
                self.forget("toggle_key");
                self.forget("left_shift");
            }
            "mode-memory" => {
                self.mode_memory = value.to_string();
                self.forget("mode_memory");
            }
            "start-mode" => {
                self.start_mode = value.to_string();
                self.forget("start_mode");
            }
            "esc-clear" => {
                self.esc_clear = value == "on";
                self.forget("esc_clear");
            }
            "space-selects" => {
                self.space_selects = value == "on";
                self.forget("space_selects");
            }
            "advance-after-select" => {
                self.advance_after_select = value == "on";
                self.forget("advance_after_select");
            }
            "output-variant" => {
                self.output_variant = value.to_string();
                self.forget("output_variant");
            }
            "output-simplified" => {
                self.output_variant = if value == "on" {
                    VARIANT_SIMPLIFIED.to_string()
                } else {
                    VARIANT_TRADITIONAL.to_string()
                };
                self.forget("output_variant");
                self.forget("output_simplified");
            }
            "simplified-hotkey" => {
                self.simplified_hotkey = value == "on";
                self.forget("simplified_hotkey");
            }
            "ctrl-punctuation" => {
                self.ctrl_punctuation = value == "on";
                self.forget("ctrl_punctuation");
            }
            "shift-symbol-halfwidth" => {
                self.shift_symbol_halfwidth = value != "off";
                self.all_symbols_halfwidth = value == "all";
                self.forget("shift_symbol_halfwidth");
                self.forget("all_symbols_halfwidth");
            }
            "cjk-latin-space" => {
                self.cjk_latin_space = value == "on";
                self.forget("cjk_latin_space");
            }
            "zhuyin-error-hint" => {
                self.zhuyin_error_hint = value == "on";
                self.forget("zhuyin_error_hint");
            }
            "caps-lock-english" => {
                self.caps_lock_english = value == "on";
                self.forget("caps_lock_english");
            }
            "toneless" => {
                self.toneless = value == "on";
                self.forget("toneless");
            }
            "language" | "primary-language" => {
                self.primary_language = value.to_string();
                self.forget("primary_language");
                self.forget("language");
            }
            "ja-input-method" => {
                self.ja_input_method = value.to_string();
                self.forget("ja_input_method");
            }
            "ja-ambiguous" => {
                self.ja_ambiguous = value.to_string();
                self.forget("ja_ambiguous");
            }
            "ja-conversion" => {
                self.ja_conversion = if value == JA_CONVERSION_SENTENCE {
                    JA_CONVERSION_SENTENCE
                } else {
                    JA_CONVERSION_LIVE
                }
                .to_string();
                self.ja_live_conversion = self.ja_conversion == JA_CONVERSION_LIVE;
                self.forget("ja_conversion");
            }
            "ja-live-conversion" => {
                self.ja_conversion = if value == "on" {
                    JA_CONVERSION_LIVE
                } else {
                    JA_CONVERSION_SENTENCE
                }
                .to_string();
                self.ja_live_conversion = value == "on";
                self.forget("ja_live_conversion");
                self.forget("ja_conversion");
            }
            "ja-symbols" => {
                self.ja_symbols = value == "on";
                self.forget("ja_symbols");
            }
            "ja-learning" => {
                self.ja_learning = value == "on";
                self.forget("ja_learning");
            }
            "ja-model" => {
                self.ja_model = value == "on";
                self.forget("ja_model");
            }
            "ja-neural-mode" => {
                self.ja_neural_mode = value.to_string();
                self.selection = self.selection_projection().to_string();
                self.forget("ja_neural_mode");
                self.forget("selection");
                self.selection_conflict = false;
            }
            "ja-rerank" => {
                self.ja_rerank = value == "on";
                self.forget("ja_rerank");
            }
            "model-mode" => {
                self.model_mode = value.to_string();
                self.forget("model_mode");
            }
            "model-residency-mb" => {
                let mb = if value == MODEL_RESIDENCY_AUTO_VALUE {
                    MODEL_RESIDENCY_AUTO_MB
                } else {
                    match value.parse::<i64>() {
                        Ok(number) if (0..=MODEL_RESIDENCY_MAX_MB).contains(&number) => number,
                        _ => {
                            return Err(SetError::InvalidValue {
                                key: key.to_string(),
                                values: "auto 或 0 到 4096 的整數（MB）".to_string(),
                            }
                            .into());
                        }
                    }
                };
                self.model_residency_mb = mb;
                self.forget("model_residency_mb");
            }
            "allow-placeholders" => {
                self.allow_placeholders = value == "on";
                self.forget("allow_placeholders");
            }
            "mixed-input" => {
                self.mixed_input = value == "on";
                self.forget("mixed_input");
            }
            "candidate-cursor" => {
                self.candidate_cursor = value.to_string();
                self.forget("candidate_cursor");
            }
            "candidate-layout" => {
                self.candidate_layout = value.to_string();
                self.forget("candidate_layout");
            }
            "emoji-shortcut" => {
                self.emoji_shortcut = value.to_string();
                self.forget("emoji_shortcut");
            }
            "emoji-equals-first" => {
                self.emoji_equals_first = value == "on";
                self.forget("emoji_equals_first");
            }
            "keyboard-layout" => {
                let mut probe = self.clone();
                probe.keyboard_layout = value.to_string();
                if let Err(reason) = probe.profile_availability(&probe.active_profile(goos)) {
                    return Err(SetError::Message(reason).into());
                }
                self.keyboard_layout = value.to_string();
                self.forget("keyboard_layout");
            }
            "selection-keys" => {
                self.selection_keys = value.to_string();
                self.forget("selection_keys");
            }
            "preview-0-2" => return Ok(()),
            _ => {
                let Some(pair) = fuzzy::parse_pair(key.strip_prefix("fuzzy-").unwrap_or_default())
                else {
                    return Err(SetError::UnknownKey {
                        key: key.to_string(),
                        available: keys().keys().cloned().collect::<Vec<_>>().join("、"),
                    }
                    .into());
                };
                let value = if value == DEFAULT_FUZZY { "" } else { value };
                self.fuzzy.set(pair, value);
            }
        }
        self.sync_profile();
        Ok(())
    }
}
