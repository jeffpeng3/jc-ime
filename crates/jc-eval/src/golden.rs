//! 注音 golden 與混合輸入 golden 題庫的讀取。
//!
//! 這兩個題庫描述「協定層級的行為」；評測只負責載入與驗證結構，不重跑
//! 引擎，因此 loader 可以在沒有 `jc-core` 的情況下使用。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{parse_json, read_bytes, EvalError, Result};
use crate::metrics::segment_boundaries;

/// `golden/zh/cases.json` 支援的格式版本。
pub const GOLDEN_FORMAT: u32 = 1;

/// 注音 golden 題庫檔。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenZhFile {
    /// 格式版本。
    pub format: u32,
    /// 案例。
    pub cases: Vec<GoldenCase>,
}

/// 一個 golden 案例。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenCase {
    /// 案例名稱，同時是 expected 檔名。
    pub name: String,
    /// 說明。
    #[serde(default)]
    pub note: String,
    /// 引擎選項。
    #[serde(default)]
    pub options: GoldenOptions,
    /// 個人化資料種子。
    #[serde(default)]
    pub seed: Option<GoldenSeed>,
    /// 驅動步驟。
    pub actions: Vec<GoldenAction>,
}

/// 每個案例的引擎選項。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GoldenOptions {
    /// 不強制聲調。
    #[serde(default)]
    pub toneless: bool,
    /// 混合輸入。
    #[serde(default)]
    pub mixed_input: bool,
    /// 英文自動補全。
    #[serde(default)]
    pub english_autocomplete: bool,
    /// 簡體輸出。
    #[serde(default)]
    pub simplified: bool,
    /// 中文與拉丁字母間補空格。
    #[serde(default)]
    pub cjk_latin_space: bool,
    /// Ctrl 標點。
    #[serde(default)]
    pub ctrl_punct: bool,
    /// 注音錯誤提示。
    #[serde(default)]
    pub zhuyin_error_hint: bool,
    /// Esc 保留組字。
    #[serde(default)]
    pub esc_keep_composition: bool,
    /// 空白鍵插入空白。
    #[serde(default)]
    pub space_inserts_space: bool,
    /// 0.2.0 預覽行為；未設定時由引擎決定。
    #[serde(default)]
    pub preview02: Option<bool>,
    /// 選字鍵設定。
    #[serde(default)]
    pub selection_keys: String,
    /// 模糊音配對 id 對應模式。
    #[serde(default)]
    pub fuzzy: BTreeMap<String, String>,
    /// Tab 補全門檻覆寫。
    #[serde(default)]
    pub suggest: Option<GoldenSuggest>,
    /// 神經重排的固定替身。
    #[serde(default)]
    pub rerank: Option<GoldenRerank>,
    /// 補標點的固定替身。
    #[serde(default)]
    pub punct: Option<GoldenPunct>,
}

/// Tab 補全選項覆寫。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GoldenSuggest {
    /// 最低信心。
    #[serde(default)]
    pub min_conf: Option<f64>,
    /// 最少音節數。
    #[serde(default)]
    pub min_syllables: Option<usize>,
    /// 注意力成本。
    #[serde(default)]
    pub attention_cost: Option<f64>,
    /// 是否改寫前綴。
    #[serde(default)]
    pub rewrite_prefix: Option<bool>,
    /// 改寫的最低信心。
    #[serde(default)]
    pub min_rewrite_conf: Option<f64>,
}

/// 固定替身重排設定。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GoldenRerank {
    /// 重排權重。
    #[serde(default)]
    pub weight: f64,
    /// 邊界。
    #[serde(default)]
    pub margin: f64,
    /// 提示間隔。
    #[serde(default)]
    pub hint_gap: f64,
    /// 偏好字串。
    #[serde(default)]
    pub prefer: Vec<String>,
    /// 偏好分數。
    #[serde(default)]
    pub prefer_score: f64,
}

/// 固定替身補標點設定。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GoldenPunct {
    /// 插入的標點。
    #[serde(default)]
    pub mark: String,
}

/// 案例開始前的個人化資料。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GoldenSeed {
    /// 單字頻率。
    #[serde(default)]
    pub unigrams: BTreeMap<String, f64>,
    /// 雙字頻率，鍵為「左詞\t詞」。
    #[serde(default)]
    pub bigrams: BTreeMap<String, f64>,
    /// 總量。
    #[serde(default)]
    pub total: f64,
    /// 自訂詞。
    #[serde(default)]
    pub vocab: Vec<GoldenVocab>,
}

/// 自訂詞。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenVocab {
    /// 詞面。
    pub text: String,
    /// 讀音。
    pub readings: Vec<String>,
}

/// 驅動 session 的一步；五個欄位只能填一個。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GoldenAction {
    /// 標籤。
    #[serde(default)]
    pub label: String,
    /// 以標準排列打出的完整音節。
    #[serde(default)]
    pub syllables: String,
    /// 逐符號打；空白代表空白鍵。
    #[serde(default)]
    pub bare: String,
    /// 逐字元送 ASCII 或具名鍵。
    #[serde(default)]
    pub keys: String,
    /// 送一個具名鍵。
    #[serde(default)]
    pub key: String,
    /// 直接送協定請求。
    #[serde(default)]
    pub request: Option<serde_json::Value>,
}

impl GoldenAction {
    /// 檢查五個欄位是否恰好填一個。
    pub fn validate(&self) -> std::result::Result<(), String> {
        let count = [
            !self.syllables.is_empty(),
            !self.bare.is_empty(),
            !self.keys.is_empty(),
            !self.key.is_empty(),
            self.request.is_some(),
        ]
        .iter()
        .filter(|x| **x)
        .count();
        if count != 1 {
            return Err("syllables、bare、keys、key、request 必須恰好填一個".to_string());
        }
        if let Some(request) = &self.request {
            let kind = request.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if kind.is_empty() {
                return Err("request.type 不能是空的".to_string());
            }
        }
        Ok(())
    }
}

/// 載入後的注音 golden 題庫。
#[derive(Debug, Clone, PartialEq)]
pub struct GoldenZhSuite {
    /// `cases.json` 路徑。
    pub path: PathBuf,
    /// 題庫內容。
    pub file: GoldenZhFile,
}

impl GoldenZhSuite {
    /// 所有案例。
    pub fn cases(&self) -> &[GoldenCase] {
        &self.file.cases
    }
}

/// 讀取 `<dir>/cases.json` 並驗證。
pub fn load_golden_zh(dir: &Path) -> Result<GoldenZhSuite> {
    let path = dir.join("cases.json");
    let raw = read_bytes(&path)?;
    let file: GoldenZhFile = parse_json(&path, &raw)?;
    validate_golden_zh(&path, &file)?;
    Ok(GoldenZhSuite { path, file })
}

fn validate_golden_zh(path: &Path, file: &GoldenZhFile) -> Result<()> {
    if file.format != GOLDEN_FORMAT {
        return Err(EvalError::invalid(
            path,
            format!("格式 {}，本版支援 {}", file.format, GOLDEN_FORMAT),
        ));
    }
    if file.cases.is_empty() {
        return Err(EvalError::invalid(path, "沒有案例"));
    }
    let mut seen = BTreeSet::new();
    for (index, case) in file.cases.iter().enumerate() {
        if !valid_name(&case.name) {
            return Err(EvalError::invalid(
                path,
                format!("第 {} 個案例名稱 {:?} 不能當檔名", index + 1, case.name),
            ));
        }
        if !seen.insert(case.name.clone()) {
            return Err(EvalError::invalid(
                path,
                format!("案例名稱重複：{}", case.name),
            ));
        }
        if case.actions.is_empty() {
            return Err(EvalError::invalid(path, format!("{}：沒有動作", case.name)));
        }
        for (step, action) in case.actions.iter().enumerate() {
            action.validate().map_err(|message| {
                EvalError::invalid(
                    path,
                    format!("{} 第 {} 步：{}", case.name, step + 1, message),
                )
            })?;
        }
    }
    Ok(())
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && !name.contains('\\') && name != "." && name != ".."
}

/// `golden/mixed/cases.json` 支援的格式版本。
pub const MIXED_GOLDEN_FORMAT: u32 = 1;

/// 混合輸入 golden 題庫檔。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenMixedFile {
    /// 格式版本。
    pub format: u32,
    /// 案例。
    pub cases: Vec<MixedCase>,
}

/// 一個混合輸入案例。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MixedCase {
    /// 案例名稱。
    pub name: String,
    /// 說明。
    #[serde(default)]
    pub note: String,
    /// 主要語言。
    pub primary: String,
    /// 輸入字串。
    pub input: String,
    /// 期望輸出。
    pub output: String,
    /// 個人化詞彙。
    #[serde(default)]
    pub personal: Vec<String>,
    /// 輸入片段。
    #[serde(default)]
    pub pieces: Vec<MixedPiece>,
    /// 輸出片段。
    pub segments: Vec<MixedSegment>,
}

impl MixedCase {
    /// 參考輸出的詞邊界。
    pub fn boundaries(&self) -> Vec<usize> {
        segment_boundaries(&self.segment_surfaces())
    }

    /// 片段輸出的字串清單。
    pub fn segment_surfaces(&self) -> Vec<String> {
        self.segments.iter().map(|s| s.surface.clone()).collect()
    }
}

/// 輸入片段。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MixedPiece {
    /// 在輸入中的位置。
    pub span: Span,
    /// 書寫系統。
    pub script: String,
    /// 語言。
    pub language: MixedLanguage,
    /// 說明。
    #[serde(default)]
    pub note: String,
}

/// 一個輸出片段。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MixedSegment {
    /// 在輸入中的位置。
    pub span: Span,
    /// 書寫系統。
    pub script: String,
    /// 逐字讀音；可能沒有。
    #[serde(default)]
    pub reading: Vec<String>,
    /// 輸出文字。
    pub surface: String,
    /// 語言。
    pub language: MixedLanguage,
    /// 變體，例如 `zh-Hant`、`ja`、`en`。
    pub variant: String,
    /// 來源。
    pub source: String,
    /// 其他可接受的片段解讀。
    #[serde(default)]
    pub also: Vec<MixedAlternative>,
}

/// 同一段輸入的其他解讀。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MixedAlternative {
    /// 在輸入中的位置。
    pub span: Span,
    /// 書寫系統。
    pub script: String,
    /// 輸出文字。
    pub surface: String,
    /// 語言。
    pub language: MixedLanguage,
    /// 變體。
    pub variant: String,
    /// 來源。
    pub source: String,
}

/// 語言標記。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MixedLanguage {
    /// 語言代號。
    pub language: String,
}

/// 輸入中的位置，以 `char` 計算。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    /// 起始 offset。
    pub start: usize,
    /// 結束 offset，不含。
    pub end: usize,
}

/// 載入後的混合輸入 golden 題庫。
#[derive(Debug, Clone, PartialEq)]
pub struct GoldenMixedSuite {
    /// `cases.json` 路徑。
    pub path: PathBuf,
    /// 題庫內容。
    pub file: GoldenMixedFile,
}

impl GoldenMixedSuite {
    /// 所有案例。
    pub fn cases(&self) -> &[MixedCase] {
        &self.file.cases
    }
}

/// 讀取 `<dir>/cases.json` 並驗證。
pub fn load_golden_mixed(dir: &Path) -> Result<GoldenMixedSuite> {
    let path = dir.join("cases.json");
    let raw = read_bytes(&path)?;
    let file: GoldenMixedFile = parse_json(&path, &raw)?;
    validate_golden_mixed(&path, &file)?;
    Ok(GoldenMixedSuite { path, file })
}

fn validate_golden_mixed(path: &Path, file: &GoldenMixedFile) -> Result<()> {
    if file.format != MIXED_GOLDEN_FORMAT {
        return Err(EvalError::invalid(
            path,
            format!("格式 {}，本版支援 {}", file.format, MIXED_GOLDEN_FORMAT),
        ));
    }
    if file.cases.is_empty() {
        return Err(EvalError::invalid(path, "沒有案例"));
    }
    let mut seen = BTreeSet::new();
    for case in &file.cases {
        if !valid_name(&case.name) {
            return Err(EvalError::invalid(
                path,
                format!("案例名稱 {:?} 無效", case.name),
            ));
        }
        if !seen.insert(case.name.clone()) {
            return Err(EvalError::invalid(
                path,
                format!("案例名稱重複：{}", case.name),
            ));
        }
        if case.primary.is_empty() || case.input.is_empty() || case.output.is_empty() {
            return Err(EvalError::invalid(
                path,
                format!("{}：primary、input、output 不能是空的", case.name),
            ));
        }
        if case.segments.is_empty() {
            return Err(EvalError::invalid(
                path,
                format!("{}：沒有 segments", case.name),
            ));
        }
        let input_len = case.input.chars().count();
        let mut expected_start = 0usize;
        for (i, segment) in case.segments.iter().enumerate() {
            if segment.span.start != expected_start || segment.span.end <= segment.span.start {
                return Err(EvalError::invalid(
                    path,
                    format!(
                        "{} 第 {} 個 segment 的 span {}..{} 不連續",
                        case.name,
                        i + 1,
                        segment.span.start,
                        segment.span.end
                    ),
                ));
            }
            if segment.surface.is_empty() {
                return Err(EvalError::invalid(
                    path,
                    format!("{} 第 {} 個 segment 沒有 surface", case.name, i + 1),
                ));
            }
            expected_start = segment.span.end;
        }
        if expected_start != input_len {
            return Err(EvalError::invalid(
                path,
                format!(
                    "{}：segments 結束在 {}，input 長度是 {}",
                    case.name, expected_start, input_len
                ),
            ));
        }
        let joined = case.segment_surfaces().concat();
        if joined != case.output {
            return Err(EvalError::invalid(
                path,
                format!("{}：segments 串接後不是 output", case.name),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jc-eval-golden-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_action_field_is_reported_with_case_name() {
        let dir = temp_dir("bad-action");
        std::fs::write(
            dir.join("cases.json"),
            r#"{"format":1,"cases":[{"name":"x","actions":[{"syllables":"ㄨㄛˇ","key":"Enter"}]}]}"#,
        )
        .unwrap();
        let err = load_golden_zh(&dir).unwrap_err().to_string();
        assert!(err.contains("cases.json"), "{err}");
        assert!(err.contains("x 第 1 步"), "{err}");
    }

    #[test]
    fn duplicate_case_name_is_rejected() {
        let dir = temp_dir("dup-name");
        std::fs::write(
            dir.join("cases.json"),
            r#"{"format":1,"cases":[
                {"name":"x","actions":[{"key":"Enter"}]},
                {"name":"x","actions":[{"key":"Enter"}]}
            ]}"#,
        )
        .unwrap();
        let err = load_golden_zh(&dir).unwrap_err().to_string();
        assert!(err.contains("案例名稱重複"), "{err}");
    }

    #[test]
    fn mixed_segment_gap_is_rejected() {
        let dir = temp_dir("mixed-gap");
        std::fs::write(
            dir.join("cases.json"),
            r#"{"format":1,"cases":[{"name":"x","primary":"zh","input":"abc","output":"abc",
                "segments":[
                    {"span":{"start":0,"end":1},"script":"latin","surface":"a","language":{"language":"en"},"variant":"en","source":"literal"},
                    {"span":{"start":2,"end":3},"script":"latin","surface":"c","language":{"language":"en"},"variant":"en","source":"literal"}
                ]}]}"#,
        )
        .unwrap();
        let err = load_golden_mixed(&dir).unwrap_err().to_string();
        assert!(err.contains("不連續"), "{err}");
    }

    #[test]
    fn real_golden_zh_has_45_cases() {
        let dir = jc_testdata::require_data!("0c-testdata/golden/zh");
        let suite = load_golden_zh(&dir).unwrap();
        assert_eq!(suite.cases().len(), 45);
        let long = suite
            .cases()
            .iter()
            .filter(|c| c.name.starts_with("long-sentence-"))
            .count();
        assert!(long >= 10, "長句只有 {long} 個");
        assert!(suite.cases().iter().all(|c| !c.actions.is_empty()));
    }

    #[test]
    fn real_golden_mixed_boundaries() {
        let dir = jc_testdata::require_data!("0c-testdata/golden/mixed");
        let suite = load_golden_mixed(&dir).unwrap();
        assert_eq!(suite.cases().len(), 6);
        let case = suite
            .cases()
            .iter()
            .find(|c| c.name == "scenario-a-primary-ja")
            .unwrap();
        assert_eq!(case.boundaries(), vec![2, 5, 7, 8]);
    }
}
