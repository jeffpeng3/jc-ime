//! 模糊輸入題庫的報告：語言×擾動分組的 raw／fallback 指標。
//!
//! 形狀對照前身 `cmd/0c-eval-fuzzy` 的報告；本模組只做彙總與輸出，候選由
//! `jc-cli` 用真正的解碼器產生。兩個角色的分母都是完整 `n`，降級原因只
//! 記錄、不把案例移出分母。

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::corpus::FuzzySourcePin;
use crate::error::{parse_json, read_bytes, EvalError, Result};
use crate::sha256::hex_digest;

/// 報告格式版本。
pub const FUZZY_SCHEMA: u32 = 1;

/// `fuzzy-input.manifest.json`。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FuzzyManifest {
    /// 格式版本。
    pub schema: u32,
    /// 題庫名稱。
    pub corpus: String,
    /// 題庫檔名。
    pub path: String,
    /// 題庫檔雜湊。
    pub sha256: String,
    /// 題數。
    pub count: usize,
    /// 固定種子。
    #[serde(default)]
    pub seed: i64,
    /// 產生器。
    #[serde(default)]
    pub generator: String,
    /// 來源 pin。
    #[serde(default)]
    pub sources: Vec<FuzzySourcePin>,
    /// 擾動種類計數。
    #[serde(default)]
    pub kinds: BTreeMap<String, usize>,
    /// 語言計數。
    #[serde(default)]
    pub languages: BTreeMap<String, usize>,
    /// 模型使用宣告。
    #[serde(default)]
    pub models: FuzzyManifestModels,
    /// 品質說明。
    #[serde(default)]
    pub quality: String,
}

/// manifest 的模型宣告。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuzzyManifestModels {
    /// 是否使用模型。
    #[serde(default)]
    pub used: bool,
    /// 說明。
    #[serde(default)]
    pub note: String,
}

/// 讀取 manifest，回傳內容與檔案雜湊。
pub fn load_fuzzy_manifest(path: &Path) -> Result<(FuzzyManifest, String)> {
    let raw = read_bytes(path)?;
    let manifest: FuzzyManifest = parse_json(path, &raw)?;
    Ok((manifest, hex_digest(&raw)))
}

/// 比對題庫與 manifest；回傳問題清單，空清單代表一致。
pub fn fuzzy_corpus_problems(
    corpus: &crate::corpus::FuzzyCorpus,
    corpus_sha: &str,
    manifest: &FuzzyManifest,
) -> Vec<String> {
    let mut problems = Vec::new();
    if manifest.schema != corpus.schema {
        problems.push(format!(
            "schema {} 與題庫 {} 不符",
            manifest.schema, corpus.schema
        ));
    }
    if manifest.sha256 != corpus_sha {
        problems.push(format!(
            "sha256 {} 與題庫 {} 不符",
            manifest.sha256, corpus_sha
        ));
    }
    if manifest.count != corpus.count {
        problems.push(format!(
            "count {} 與題庫 {} 不符",
            manifest.count, corpus.count
        ));
    }
    if manifest.seed != corpus.seed {
        problems.push(format!(
            "seed {} 與題庫 {} 不符",
            manifest.seed, corpus.seed
        ));
    }
    if manifest.models.used {
        problems.push("manifest 宣告使用了模型；模糊題庫不得載入模型".to_string());
    }
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut languages: BTreeMap<String, usize> = BTreeMap::new();
    for item in &corpus.items {
        *kinds.entry(item.kind.clone()).or_insert(0) += 1;
        *languages.entry(item.language.clone()).or_insert(0) += 1;
    }
    if kinds != manifest.kinds {
        problems.push(format!(
            "擾動種類計數 {} 與 manifest {:?} 不符",
            kinds.len(),
            manifest.kinds
        ));
    }
    if languages != manifest.languages {
        problems.push(format!(
            "語言計數 {languages:?} 與 manifest {:?} 不符",
            manifest.languages
        ));
    }
    problems
}

/// 一個角色（raw 或 fallback）的指標。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FuzzyArmMetrics {
    /// 完整分母。
    pub n: usize,
    /// top-1 命中數。
    pub top1: usize,
    /// top-1 命中率。
    pub top1_rate: f64,
    /// 前五名命中數。
    pub top5: usize,
    /// 前五名命中率。
    pub top5_rate: f64,
    /// 直接提交等於預期文字的案例數。
    pub intended: usize,
    /// 直接提交命中率。
    pub intended_rate: f64,
    /// 候選中找不到預期文字的案例數。
    pub missing: usize,
    /// 候選清單是空白的案例數。
    pub no_candidates: usize,
    /// 回退角色把 raw 的失誤修好的案例數。
    pub recovered: usize,
    /// 回退角色把 raw 的命中改壞的案例數。
    pub harmed: usize,
}

impl FuzzyArmMetrics {
    fn add(&mut self, rank: usize, intended: bool, has_candidates: bool) {
        self.n += 1;
        if rank == 1 {
            self.top1 += 1;
        }
        if (1..=5).contains(&rank) {
            self.top5 += 1;
        }
        if intended {
            self.intended += 1;
        }
        if rank == 0 {
            self.missing += 1;
        }
        if !has_candidates {
            self.no_candidates += 1;
        }
    }

    fn finalize(&mut self) {
        self.top1_rate = ratio(self.top1, self.n);
        self.top5_rate = ratio(self.top5, self.n);
        self.intended_rate = ratio(self.intended, self.n);
    }
}

/// 留在分母裡的降級原因統計。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuzzyDegraded {
    /// 至少有一個原因的案例數。
    pub cases: usize,
    /// 打字、轉換、送出或工作階段錯誤。
    pub typing_errors: usize,
    /// 沒有讀音。
    pub no_reading: usize,
    /// 沒有日文解碼器。
    pub no_decoder: usize,
    /// 候選清單是空的。
    pub no_candidates: usize,
    /// 提交是空的。
    pub no_commit: usize,
    /// 羅馬字按鍵沒有被工作階段接收。
    pub unhandled_keys: usize,
}

/// 單題結果。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FuzzyCaseResult {
    /// 題目 id。
    pub id: String,
    /// 語言。
    pub language: String,
    /// 擾動種類。
    pub kind: String,
    /// 預期文字。
    pub reference: String,
    /// 題庫輸入。
    pub input: String,
    /// 擾動後的讀音。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub noisy_reading: String,
    /// 尚未完成的按鍵。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pending: String,
    /// 直接提交的文字。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub output: String,
    /// raw 角色的名次；0 表示不在候選中。
    pub rank: usize,
    /// raw 角色的前五名。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub raw_top5: Vec<String>,
    /// 直接提交是否命中。
    #[serde(default)]
    pub intended: bool,
    /// 回退角色的名次。
    #[serde(default)]
    pub fallback_rank: usize,
    /// 回退角色的前五名。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback_top5: Vec<String>,
    /// 回退角色的第一名是否命中。
    #[serde(default)]
    pub fallback_intended: bool,
    /// 回退角色由誰服務；沒有候選時是 `none`。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fallback_outcome: String,
    /// 是否降級。
    #[serde(default)]
    pub degraded: bool,
    /// 降級原因，逗號分隔。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub degraded_reason: String,
    /// 這一題花費的毫秒數。
    #[serde(default)]
    pub duration_ms: f64,
    /// 失敗分類：`rule`、`dictionary`、`ranking`、`noise` 或空字串。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub miss_class: String,
}

/// 語言×擾動種類的彙總；語言或種類為空字串代表合計。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FuzzyGroupReport {
    /// 語言。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub language: String,
    /// 擾動種類。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// 題數。
    pub n: usize,
    /// raw 角色指標。
    pub raw: FuzzyArmMetrics,
    /// 回退角色指標。
    pub fallback: FuzzyArmMetrics,
    /// 回退角色的結果分布。
    #[serde(default)]
    pub fallback_outcomes: BTreeMap<String, usize>,
    /// 降級統計。
    pub degraded: FuzzyDegraded,
}

/// 執行設定與定位說明。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FuzzyRunConfig {
    /// 日文 N-best 候選數。
    pub top_n: usize,
    /// 注音 fuzzy 設定。
    pub fuzzy: String,
    /// 回退角色說明。
    pub ranker: String,
    /// 模型說明。
    pub models: String,
    /// 品質說明。
    pub quality: String,
}

/// 回退角色的請求計數；與前身 `score.Counters` 相同的分母關係。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuzzyRankerCounters {
    /// 有候選、進入排序的請求數。
    pub requests: usize,
    /// 由模型服務的請求數。
    pub model: usize,
    /// 由統計回退服務的請求數。
    pub fallback: usize,
    /// 逾時後由統計回退服務的請求數。
    pub timeout: usize,
    /// 因隱私規則跳過模型的請求數。
    pub privacy: usize,
}

/// 引擎與資料識別。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FuzzyEngineInfo {
    /// 資料目錄。
    #[serde(default)]
    pub data_dir: String,
    /// 注音詞典雜湊。
    #[serde(default)]
    pub lexicon_sha256: String,
    /// 注音語言模型雜湊。
    #[serde(default)]
    pub lm_sha256: String,
    /// 日文 pack。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub japanese_pack: Option<FuzzyPackInfo>,
    /// 模型說明。
    #[serde(default)]
    pub models: String,
    /// 是否使用模型。
    #[serde(default)]
    pub models_used: bool,
    /// 品質說明。
    #[serde(default)]
    pub quality: String,
}

/// 日文參考 pack 的 identity。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuzzyPackInfo {
    /// 路徑。
    pub path: String,
    /// 檔案雜湊。
    #[serde(default)]
    pub sha256: String,
    /// 詞條數。
    #[serde(default)]
    pub entries: usize,
}

/// 完整報告。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FuzzyReport {
    /// 格式版本。
    pub schema: u32,
    /// 題庫名稱。
    pub corpus: String,
    /// 題庫檔雜湊。
    pub corpus_sha256: String,
    /// 固定種子。
    pub seed: i64,
    /// 引擎與資料識別。
    pub engine: FuzzyEngineInfo,
    /// 執行設定。
    pub config: FuzzyRunConfig,
    /// 回退請求計數。
    pub ranker_counters: FuzzyRankerCounters,
    /// 語言×擾動分組。
    pub groups: Vec<FuzzyGroupReport>,
    /// 逐題結果。
    #[serde(default)]
    pub cases: Vec<FuzzyCaseResult>,
}

impl FuzzyReport {
    /// 用通用引擎資訊建立空報告。
    pub fn new(corpus: impl Into<String>, sha256: impl Into<String>, seed: i64) -> Self {
        Self {
            schema: FUZZY_SCHEMA,
            corpus: corpus.into(),
            corpus_sha256: sha256.into(),
            seed,
            ..Self::default()
        }
    }

    /// 彙總逐題結果並填入分組與回退計數。
    pub fn summarize(&mut self) {
        self.groups = summarize_fuzzy(&self.cases);
        let requests = self
            .cases
            .iter()
            .filter(|case| !case.raw_top5.is_empty() || case.rank > 0)
            .count();
        self.ranker_counters = FuzzyRankerCounters {
            requests,
            model: 0,
            fallback: requests,
            timeout: 0,
            privacy: 0,
        };
    }

    /// 輸出 JSON。
    pub fn to_json_pretty(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|err| EvalError::Encode(err.to_string()))
    }

    /// 輸出人類可讀的表格。
    pub fn to_table(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "題庫 {}（seed {}，{} 題）；注音資料 {}；模型 none\n",
            self.corpus,
            self.seed,
            self.cases.len(),
            self.engine.data_dir
        ));
        out.push_str(&format!(
            "注音 fuzzy={}；回退角色：{}\n",
            self.config.fuzzy, self.config.ranker
        ));
        out.push_str(&format!(
            "回退請求：{}（model {}、fallback {}、timeout {}、privacy {}）\n",
            self.ranker_counters.requests,
            self.ranker_counters.model,
            self.ranker_counters.fallback,
            self.ranker_counters.timeout,
            self.ranker_counters.privacy
        ));
        out.push_str(&format!(
            "\n{:<6} {:<12} {:>5} | {:<31} | {:<31} | {}\n",
            "語言", "擾動", "n", "raw top1/top5/文字", "fallback top1/top5/文字", "降級"
        ));
        for group in &self.groups {
            let language = if group.language.is_empty() {
                "(all)"
            } else {
                group.language.as_str()
            };
            let kind = if group.kind.is_empty() {
                "(合計)"
            } else {
                group.kind.as_str()
            };
            out.push_str(&format!(
                "{:<6} {:<12} {:>5} | {:>6.1}% {:>6.1}% {:>6.1}% | {:>6.1}% {:>6.1}% {:>6.1}% | {}\n",
                language,
                kind,
                group.n,
                group.raw.top1_rate * 100.0,
                group.raw.top5_rate * 100.0,
                group.raw.intended_rate * 100.0,
                group.fallback.top1_rate * 100.0,
                group.fallback.top5_rate * 100.0,
                group.fallback.intended_rate * 100.0,
                group.degraded.cases
            ));
        }
        for group in &self.groups {
            if !group.kind.is_empty() || group.degraded.cases == 0 {
                continue;
            }
            out.push_str(&format!(
                "{} 降級原因（仍在分母）：typing {}、no_reading {}、no_decoder {}、no_candidates {}、no_commit {}、unhandled {}\n",
                if group.language.is_empty() {
                    "(all)"
                } else {
                    group.language.as_str()
                },
                group.degraded.typing_errors,
                group.degraded.no_reading,
                group.degraded.no_decoder,
                group.degraded.no_candidates,
                group.degraded.no_commit,
                group.degraded.unhandled_keys
            ));
        }
        for (language, counts) in self.miss_class_counts() {
            let label = if language.is_empty() {
                "(all)"
            } else {
                language.as_str()
            };
            out.push_str(&format!(
                "{} 失誤分類：rule {}、dictionary {}、ranking {}、noise {}\n",
                label,
                counts.get("rule").copied().unwrap_or(0),
                counts.get("dictionary").copied().unwrap_or(0),
                counts.get("ranking").copied().unwrap_or(0),
                counts.get("noise").copied().unwrap_or(0)
            ));
        }
        out
    }

    /// 依語言統計失誤分類；只列出有失誤的語言與全體。
    pub fn miss_class_counts(&self) -> BTreeMap<String, BTreeMap<String, usize>> {
        let mut out: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
        for case in &self.cases {
            if case.miss_class.is_empty() {
                continue;
            }
            for language in [case.language.clone(), String::new()] {
                *out.entry(language)
                    .or_default()
                    .entry(case.miss_class.clone())
                    .or_insert(0) += 1;
            }
        }
        out
    }
}

/// 彙總逐題結果：每個語言×種類、每個語言與全體各一組。
pub fn summarize_fuzzy(cases: &[FuzzyCaseResult]) -> Vec<FuzzyGroupReport> {
    let mut groups: BTreeMap<(String, String), FuzzyGroupReport> = BTreeMap::new();
    for case in cases {
        for key in [
            (case.language.clone(), case.kind.clone()),
            (case.language.clone(), String::new()),
            (String::new(), String::new()),
        ] {
            let group = groups
                .entry(key.clone())
                .or_insert_with(|| FuzzyGroupReport {
                    language: key.0.clone(),
                    kind: key.1.clone(),
                    ..FuzzyGroupReport::default()
                });
            add_case(group, case);
        }
    }
    let mut out: Vec<FuzzyGroupReport> = groups.into_values().collect();
    for group in &mut out {
        group.raw.finalize();
        group.fallback.finalize();
    }
    out
}

fn add_case(group: &mut FuzzyGroupReport, case: &FuzzyCaseResult) {
    group.n += 1;
    let raw_has = !case.raw_top5.is_empty() || case.rank > 0;
    let fallback_has = !case.fallback_top5.is_empty() || case.fallback_rank > 0;
    group.raw.add(case.rank, case.intended, raw_has);
    group
        .fallback
        .add(case.fallback_rank, case.fallback_intended, fallback_has);
    if !case.fallback_outcome.is_empty() {
        *group
            .fallback_outcomes
            .entry(case.fallback_outcome.clone())
            .or_insert(0) += 1;
    }
    if case.rank == 1 && case.fallback_rank != 1 {
        group.fallback.harmed += 1;
    }
    if case.rank != 1 && case.fallback_rank == 1 {
        group.fallback.recovered += 1;
    }
    if case.degraded {
        group.degraded.cases += 1;
        for reason in case.degraded_reason.split(',') {
            match reason {
                "typing_error" | "candidate_error" | "decode_error" | "convert_error"
                | "commit_error" | "ja_session" | "no_session" => group.degraded.typing_errors += 1,
                "no_reading" => group.degraded.no_reading += 1,
                "no_decoder" => group.degraded.no_decoder += 1,
                "no_candidates" => group.degraded.no_candidates += 1,
                "no_commit" => group.degraded.no_commit += 1,
                "unhandled_keys" => group.degraded.unhandled_keys += 1,
                _ => {}
            }
        }
    }
}

/// 回傳預期文字在候選中的名次（從 1 起算；0 表示不在候選中）。
pub fn rank_of(candidates: &[String], reference: &str) -> usize {
    candidates
        .iter()
        .position(|candidate| candidate == reference)
        .map(|index| index + 1)
        .unwrap_or(0)
}

fn ratio(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(language: &str, kind: &str, rank: usize, intended: bool) -> FuzzyCaseResult {
        FuzzyCaseResult {
            id: format!("{language}/{kind}/{rank}"),
            language: language.to_string(),
            kind: kind.to_string(),
            reference: "甲".to_string(),
            rank,
            raw_top5: if rank > 0 {
                vec!["甲".to_string()]
            } else {
                Vec::new()
            },
            intended,
            fallback_rank: rank,
            fallback_intended: intended,
            fallback_outcome: if rank > 0 { "fallback" } else { "none" }.to_string(),
            ..FuzzyCaseResult::default()
        }
    }

    #[test]
    fn groups_cover_kind_language_and_all() {
        let cases = vec![
            case("zh", "wrong-key", 1, true),
            case("zh", "wrong-key", 0, false),
            case("zh", "near-sound", 2, false),
            case("ja", "missing-key", 1, true),
        ];
        let groups = summarize_fuzzy(&cases);
        let find = |language: &str, kind: &str| {
            groups
                .iter()
                .find(|group| group.language == language && group.kind == kind)
                .unwrap_or_else(|| panic!("缺少 {language}/{kind}"))
        };
        assert_eq!(find("zh", "wrong-key").n, 2);
        assert_eq!(find("zh", "wrong-key").raw.top1_rate, 0.5);
        assert_eq!(find("zh", "near-sound").raw.top5_rate, 1.0);
        assert_eq!(find("zh", "").n, 3);
        assert_eq!(find("", "").n, 4);
        assert_eq!(find("zh", "wrong-key").fallback_outcomes["fallback"], 1);
        assert_eq!(find("zh", "wrong-key").fallback_outcomes["none"], 1);
    }

    #[test]
    fn report_counts_fallback_requests() {
        let mut report = FuzzyReport::new("fuzzy-input", "abc", 62);
        report.cases = vec![
            case("zh", "wrong-key", 1, true),
            case("ja", "missing-key", 0, false),
        ];
        report.summarize();
        assert_eq!(report.ranker_counters.requests, 1);
        assert_eq!(report.ranker_counters.fallback, 1);
        assert_eq!(report.ranker_counters.model, 0);
        let table = report.to_table();
        assert!(table.contains("fuzzy-input"));
        assert!(table.contains("zh"));
        assert!(table.contains("(all)"));
    }

    #[test]
    fn rank_of_returns_one_based_index() {
        let candidates = vec!["乙".to_string(), "甲".to_string()];
        assert_eq!(rank_of(&candidates, "甲"), 2);
        assert_eq!(rank_of(&candidates, "丙"), 0);
        assert_eq!(rank_of(&[], "甲"), 0);
    }
}
