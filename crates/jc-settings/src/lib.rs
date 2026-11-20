//! 設定與模型集：資料模型、驗證與檔案格式。
//!
//! 這個 crate 是 TSF 外殼與設定程式共用的純資料層，不含 UI，也不呼叫任何
//! 作業系統 API。平台差異（預設切換鍵、開始模式等）一律由呼叫端傳入
//! `goos` 字串；檔案位置由呼叫端傳入資料根目錄。
//!
//! 對照前身：`internal/settings` 與 `internal/modelsets`。逐檔對照見
//! `docs/port/settings-core.md`。

pub mod atomic_file;
pub mod both_shift;
pub mod change;
pub mod modelset;
pub mod paths;
pub mod profile_rules;
pub mod profiles;
pub mod settings;

pub use change::{change, WRITE_LOCK};
pub use paths::{legacy_path, settings_path, settings_path_for_store};
pub use profiles::{Profile, ProfileAction};
pub use settings::{
    keep_toggle_key, keys, load, quarantine, quarantine_now, read, save, FileState, Fuzzy, Report,
    SetCode, SetError, Settings, SettingsError, DEFAULT_FUZZY, FILE_NAME, JA_AMBIGUOUS_BOTH,
    JA_AMBIGUOUS_ENGLISH, JA_AMBIGUOUS_KANA, JA_CONVERSION_LIVE, JA_CONVERSION_SENTENCE,
    JA_INPUT_KANA, JA_INPUT_ROMAJI, JA_NEURAL_AUTO, JA_NEURAL_OFF, JA_NEURAL_QUALITY, LANGUAGE_JA,
    LANGUAGE_ZH, LAYOUT_ETEN, LAYOUT_HSU, LAYOUT_STANDARD, MODEL_MODE_ALL, MODEL_MODE_AUTO,
    MODEL_MODE_NONE, MODEL_MODE_PRIMARY, MODEL_RESIDENCY_AUTO_MB, MODEL_RESIDENCY_AUTO_VALUE,
    MODEL_RESIDENCY_DUAL_MB, MODEL_RESIDENCY_MAX_MB, MODEL_RESIDENCY_SINGLE_MB, MODE_NATIVE,
    MODE_SHARED, NEURAL, SCHEMA_VERSION, SELECT_DIGITS, SELECT_HOME, SELECT_QWER, START_ENGLISH,
    START_ZHUYIN, STATISTICAL, TOGGLE_BOTH, TOGGLE_LEFT, TOGGLE_NONE, TOGGLE_RIGHT,
    VARIANT_SHINJITAI, VARIANT_SIMPLIFIED, VARIANT_TRADITIONAL,
};

#[cfg(test)]
mod features_tests;
#[cfg(test)]
mod tests;
