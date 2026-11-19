//! 日文題庫、`rules.json`、`baseline.json` 的讀取與評測。
//!
//! 題庫格式與 `0c-ime` 的 `testdata/ja` 相同；轉換器由 [`crate::Converter`]
//! 注入，這裡不需要 `jc-core`。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{parse_json, read_bytes, EvalError, Result};
use crate::metrics::{evaluate, EvalCase, RunOptions};
use crate::report::{CorpusRun, EngineInfo, GateResult, Report, Run, SCHEMA};
use crate::sha256::hex_digest;
use crate::Converter;

/// 日文題庫、規則與 baseline 的格式版本。
pub const JA_SCHEMA: u32 = 1;

/// 一題日文評測。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JaCase {
    /// 題目識別碼。
    #[serde(default)]
    pub id: String,
    /// 讀音。
    pub reading: String,
    /// 主要參考詞面。
    pub surface: String,
    /// 其他可接受的詞面。
    #[serde(default)]
    pub accept: Vec<String>,
    /// 讀音的詞切分，供來源核對。
    #[serde(default)]
    pub reading_segments: Vec<String>,
    /// 參考詞切分。
    #[serde(default)]
    pub segments: Vec<String>,
    /// 標籤。
    #[serde(default)]
    pub tags: Vec<String>,
    /// 來源索引。
    #[serde(default)]
    pub source_index: Option<String>,
}

/// 一份凍結的日文題庫。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JaCorpus {
    /// 題庫名稱。
    pub name: String,
    /// 授權。
    pub license: String,
    /// 來源。
    pub source: String,
    /// 說明。
    #[serde(default)]
    pub notes: String,
    /// 取樣種子。
    #[serde(default)]
    pub seed: Option<i64>,
    /// 來源檔雜湊。
    #[serde(default)]
    pub corpus_files: BTreeMap<String, String>,
    /// 來源題庫雜湊。
    #[serde(default)]
    pub source_sha256: Option<String>,
    /// 宣告題數。
    #[serde(default)]
    pub count: usize,
    /// 題目。
    pub items: Vec<JaCase>,
}

/// `rules.json` 對單一題庫的 pin。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CorpusRule {
    /// 檔名，相對於 `rules.json`。
    #[serde(default)]
    pub file: String,
    /// 檔案 SHA-256。
    #[serde(default)]
    pub sha256: String,
    /// 題數。
    #[serde(default)]
    pub count: usize,
    /// 計入邊界指標的題數；0 表示不檢查。
    #[serde(default)]
    pub boundary_cases: usize,
    /// 授權。
    #[serde(default)]
    pub license: String,
    /// 來源。
    #[serde(default)]
    pub source: String,
}

/// 退步容許量；只涵蓋決定性品質指標。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Gate {
    /// top-1 命中率最多允許下降多少。
    #[serde(default)]
    pub max_top1_drop: f64,
    /// top-5 命中率最多允許下降多少。
    #[serde(default)]
    pub max_top5_drop: f64,
    /// 邊界完全相同的比例最多允許下降多少。
    #[serde(default)]
    pub max_boundary_exact_drop: f64,
    /// 邊界 F1 最多允許下降多少。
    #[serde(default)]
    pub max_boundary_f1_drop: f64,
}

impl Gate {
    /// 依容許量比較兩份報告。
    ///
    /// 除指標下降外，也檢查規則與引擎雜湊、題數、缺少或新增的題庫；任何一項
    /// 不符都算失敗，避免 baseline 被悄悄移動。
    pub fn check(&self, new: &Report, baseline: &Report) -> GateResult {
        let mut problems = Vec::new();

        if !baseline.rules_sha256.is_empty() && baseline.rules_sha256 != new.rules_sha256 {
            problems.push(format!(
                "rules hash {} 與 baseline {} 不同；請刻意更新 baseline",
                new.rules_sha256, baseline.rules_sha256
            ));
        }
        for (field, got, want) in [
            (
                "lexicon",
                &new.engine.lexicon_sha256,
                &baseline.engine.lexicon_sha256,
            ),
            (
                "connection",
                &new.engine.connection_sha256,
                &baseline.engine.connection_sha256,
            ),
        ] {
            if !want.is_empty() && got != want {
                problems.push(format!(
                    "{field} hash {got} 與 baseline {want} 不同；請刻意更新 baseline"
                ));
            }
        }

        for (name, want) in &baseline.corpora {
            let Some(got) = new.corpora.get(name) else {
                problems.push(format!("{name}：報告缺少這個題庫"));
                continue;
            };
            if got.n != want.n {
                problems.push(format!("{name}：n={}，baseline 是 {}", got.n, want.n));
            }
            let mut check = |metric: &str, got: f64, want: f64, tolerance: f64| {
                if got + tolerance < want {
                    problems.push(format!("{name}：{metric} {got:.6} < baseline {want:.6}"));
                }
            };
            check(
                "top1_rate",
                got.top1_rate,
                want.top1_rate,
                self.max_top1_drop,
            );
            check(
                "top5_rate",
                got.top5_rate,
                want.top5_rate,
                self.max_top5_drop,
            );
            check(
                "boundary_exact_rate",
                got.boundary_exact_rate,
                want.boundary_exact_rate,
                self.max_boundary_exact_drop,
            );
            check(
                "boundary_f1",
                got.boundary_f1,
                want.boundary_f1,
                self.max_boundary_f1_drop,
            );
        }

        for name in new.corpora.keys() {
            if !baseline.corpora.contains_key(name) {
                problems.push(format!("{name}：baseline 沒有這個題庫"));
            }
        }

        GateResult::from_problems(problems)
    }
}

/// `ja/rules.json`。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RuleSet {
    /// 格式版本。
    pub schema: u32,
    /// 規則版本。
    pub version: String,
    /// 凍結日期。
    #[serde(default)]
    pub frozen_at: String,
    /// 引擎名稱。
    #[serde(default)]
    pub engine: String,
    /// 每次轉換取幾個候選。
    pub top_n: usize,
    /// 超過幾毫秒算 timeout。
    pub timeout_ms: u64,
    /// 題庫 pin。
    pub corpora: BTreeMap<String, CorpusRule>,
    /// 指標定義。
    #[serde(default)]
    pub metric_definitions: BTreeMap<String, String>,
    /// 退步容許量。
    #[serde(default)]
    pub gate: Gate,
    /// 檔案 SHA-256；由 loader 填入。
    #[serde(skip)]
    pub sha256: String,
}

impl RuleSet {
    /// 依 `rules.json` 的 `max_*_drop` 判定是否退步。
    pub fn gate(&self, new: &Report, baseline: &Report) -> GateResult {
        self.gate.check(new, baseline)
    }
}

/// 載入後的題庫與它的 pin。
#[derive(Debug, Clone, PartialEq)]
pub struct JaCorpusEntry {
    /// `rules.json` 的 pin。
    pub rule: CorpusRule,
    /// 題庫內容。
    pub corpus: JaCorpus,
    /// 檔案 SHA-256。
    pub sha256: String,
    /// 檔案路徑。
    pub path: PathBuf,
}

/// 讀取單一題庫並驗證。
pub fn load_ja_corpus(path: &Path) -> Result<JaCorpusEntry> {
    let raw = read_bytes(path)?;
    let corpus: JaCorpus = parse_json(path, &raw)?;
    validate_ja_corpus(path, &corpus)?;
    Ok(JaCorpusEntry {
        rule: CorpusRule::default(),
        corpus,
        sha256: hex_digest(&raw),
        path: path.to_path_buf(),
    })
}

fn validate_ja_corpus(path: &Path, corpus: &JaCorpus) -> Result<()> {
    if corpus.name.is_empty() || corpus.license.is_empty() || corpus.source.is_empty() {
        return Err(EvalError::invalid(path, "name、license、source 都是必填"));
    }
    if corpus.items.is_empty() {
        return Err(EvalError::invalid(path, "沒有 items"));
    }
    if corpus.count != 0 && corpus.count != corpus.items.len() {
        return Err(EvalError::invalid(
            path,
            format!(
                "count {} 與 {} 筆 items 不符",
                corpus.count,
                corpus.items.len()
            ),
        ));
    }
    for (index, item) in corpus.items.iter().enumerate() {
        let where_ = format!("第 {} 題", index + 1);
        if item.reading.is_empty() || item.surface.is_empty() {
            return Err(EvalError::invalid(
                path,
                format!("{where_}：reading 或 surface 是空的"),
            ));
        }
        if !item.segments.is_empty() && item.segments.concat() != item.surface {
            return Err(EvalError::invalid(
                path,
                format!("{where_}：segments 串接後不是 surface"),
            ));
        }
        if !item.reading_segments.is_empty() && item.reading_segments.concat() != item.reading {
            return Err(EvalError::invalid(
                path,
                format!("{where_}：reading_segments 串接後不是 reading"),
            ));
        }
        if item
            .reading
            .chars()
            .any(|r| matches!(r, ' ' | '\t' | '\n' | '\r'))
        {
            return Err(EvalError::invalid(
                path,
                format!("{where_}：reading 含空白"),
            ));
        }
    }
    Ok(())
}

/// 讀取 `rules.json` 並計算檔案雜湊。
pub fn load_rules(path: &Path) -> Result<RuleSet> {
    let raw = read_bytes(path)?;
    let mut rules: RuleSet = parse_json(path, &raw)?;
    if rules.schema != JA_SCHEMA {
        return Err(EvalError::invalid(
            path,
            format!("schema {}，本版支援 {}", rules.schema, JA_SCHEMA),
        ));
    }
    if rules.version.is_empty() || rules.top_n == 0 || rules.timeout_ms == 0 {
        return Err(EvalError::invalid(
            path,
            "version、top_n、timeout_ms 都是必填",
        ));
    }
    if rules.corpora.is_empty() {
        return Err(EvalError::invalid(path, "沒有 corpora"));
    }
    for (name, rule) in &rules.corpora {
        if rule.file.is_empty() || rule.sha256.is_empty() || rule.count == 0 {
            return Err(EvalError::invalid(
                path,
                format!("題庫 {name} 需要 file、sha256、count"),
            ));
        }
    }
    rules.sha256 = hex_digest(&raw);
    Ok(rules)
}

/// 依 `rules.json` 載入所有題庫，順序固定為名稱排序。
pub fn load_ja_corpus_set(rules: &RuleSet, rules_path: &Path) -> Result<Vec<JaCorpusEntry>> {
    let base = rules_path.parent().unwrap_or(Path::new("."));
    let mut out = Vec::with_capacity(rules.corpora.len());
    for (name, rule) in &rules.corpora {
        let mut path = PathBuf::from(&rule.file);
        if path.is_relative() {
            path = base.join(path);
        }
        let mut entry = load_ja_corpus(&path)?;
        if entry.sha256 != rule.sha256 {
            return Err(EvalError::invalid(
                &path,
                format!("sha256 {} 與 rules pin {} 不符", entry.sha256, rule.sha256),
            ));
        }
        if entry.corpus.name != *name {
            return Err(EvalError::invalid(
                &path,
                format!(
                    "題庫名稱 {:?} 與 rules key {:?} 不符",
                    entry.corpus.name, name
                ),
            ));
        }
        if entry.corpus.items.len() != rule.count {
            return Err(EvalError::invalid(
                &path,
                format!("{} 題，rules pin {}", entry.corpus.items.len(), rule.count),
            ));
        }
        let scored = entry
            .corpus
            .items
            .iter()
            .filter(|item| !item.segments.is_empty())
            .count();
        if rule.boundary_cases != 0 && scored != rule.boundary_cases {
            return Err(EvalError::invalid(
                &path,
                format!("{scored} 題有邊界，rules pin {}", rule.boundary_cases),
            ));
        }
        if !rule.license.is_empty() && entry.corpus.license != rule.license {
            return Err(EvalError::invalid(
                &path,
                format!("license {:?} 與 rules pin 不符", entry.corpus.license),
            ));
        }
        if !rule.source.is_empty() && entry.corpus.source != rule.source {
            return Err(EvalError::invalid(
                &path,
                format!("source {:?} 與 rules pin 不符", entry.corpus.source),
            ));
        }
        entry.rule = rule.clone();
        out.push(entry);
    }
    Ok(out)
}

/// 讀取 `baseline.json`；形狀與 [`Report`] 相同。
pub fn load_baseline(path: &Path) -> Result<Report> {
    let raw = read_bytes(path)?;
    let baseline: Report = parse_json(path, &raw)?;
    if baseline.schema != SCHEMA {
        return Err(EvalError::invalid(
            path,
            format!("schema {}，本版支援 {}", baseline.schema, SCHEMA),
        ));
    }
    if baseline.rules_sha256.is_empty() || baseline.corpora.is_empty() {
        return Err(EvalError::invalid(path, "rules_sha256 與 corpora 都是必填"));
    }
    Ok(baseline)
}

impl From<&JaCase> for EvalCase {
    fn from(item: &JaCase) -> Self {
        EvalCase {
            id: item.id.clone(),
            reading: item.reading.clone(),
            reference: item.surface.clone(),
            accept: item.accept.clone(),
            segments: item.segments.clone(),
        }
    }
}

impl RunOptions {
    /// 依 `rules.json` 的 `top_n` 與 `timeout_ms` 建立參數。
    pub fn from_rules(rules: &RuleSet) -> Self {
        Self::new(rules.top_n, rules.timeout_ms)
    }
}

/// 跑完一組日文題庫，回傳聚合報告與逐題結果。
pub fn run_ja(
    converter: &dyn Converter,
    rules: &RuleSet,
    corpora: &[JaCorpusEntry],
    engine: EngineInfo,
    options: &RunOptions,
) -> Run {
    let mut options = *options;
    if options.top_n == 0 {
        options.top_n = rules.top_n;
    }
    if options.timeout_ms == 0 {
        options.timeout_ms = rules.timeout_ms;
    }

    let mut report = Report::new(engine, rules.sha256.clone());
    let mut runs = Vec::with_capacity(corpora.len());
    for entry in corpora {
        let cases: Vec<EvalCase> = entry.corpus.items.iter().map(EvalCase::from).collect();
        let evaluation = evaluate(converter, &cases, &options);
        report.insert(entry.corpus.name.clone(), evaluation.metrics);
        runs.push(CorpusRun {
            name: entry.corpus.name.clone(),
            license: entry.corpus.license.clone(),
            source: entry.corpus.source.clone(),
            sha256: entry.sha256.clone(),
            items: evaluation.items,
        });
    }
    Run {
        report,
        corpora: runs,
    }
}

/// 檢查 Anthy 題庫只使用 `corpus.1.txt`，並確認題目都來自凍結的 fixture。
///
/// `fixture` 是 T03 的 `anthy_sample200.json`；沒給時只檢查題庫自身的
/// `corpus_files` 與授權。
pub fn check_ja_corpus_provenance(entries: &[JaCorpusEntry], fixture: Option<&Path>) -> Result<()> {
    let Some(anthy) = entries.iter().find(|e| e.corpus.name == "anthy-corpus1") else {
        return Err(EvalError::invalid(
            Path::new("<corpora>"),
            "缺少 anthy-corpus1 題庫",
        ));
    };
    if !anthy
        .corpus
        .license
        .to_lowercase()
        .contains("public domain")
    {
        return Err(EvalError::invalid(
            &anthy.path,
            format!("license {:?} 不是 public domain", anthy.corpus.license),
        ));
    }
    if anthy.corpus.corpus_files.len() != 1 {
        return Err(EvalError::invalid(
            &anthy.path,
            format!(
                "corpus_files 有 {} 筆，只允許 corpus.1.txt",
                anthy.corpus.corpus_files.len()
            ),
        ));
    }
    let Some(source_hash) = anthy.corpus.corpus_files.get("corpus.1.txt") else {
        return Err(EvalError::invalid(
            &anthy.path,
            "corpus_files 沒有記錄 corpus.1.txt 的雜湊",
        ));
    };
    if source_hash.is_empty() {
        return Err(EvalError::invalid(&anthy.path, "corpus.1.txt 的雜湊是空的"));
    }

    let Some(fixture) = fixture else {
        return Ok(());
    };
    let raw = read_bytes(fixture)?;
    #[derive(Deserialize)]
    struct Fixture {
        #[serde(default)]
        corpus_files: BTreeMap<String, String>,
        #[serde(default)]
        items: Vec<FixtureItem>,
    }
    #[derive(Deserialize)]
    struct FixtureItem {
        reading: String,
        surface: String,
    }
    let pinned: Fixture = parse_json(fixture, &raw)?;
    if pinned.corpus_files.len() != 1 {
        return Err(EvalError::invalid(
            fixture,
            format!(
                "fixture 列出 {} 個來源檔，只允許 corpus.1.txt",
                pinned.corpus_files.len()
            ),
        ));
    }
    if pinned.corpus_files.get("corpus.1.txt") != Some(source_hash) {
        return Err(EvalError::invalid(
            fixture,
            "fixture 的 corpus.1.txt 雜湊與題庫不符",
        ));
    }
    let want: BTreeSet<(&str, &str)> = pinned
        .items
        .iter()
        .map(|x| (x.reading.as_str(), x.surface.as_str()))
        .collect();
    for item in &anthy.corpus.items {
        if !want.contains(&(item.reading.as_str(), item.surface.as_str())) {
            return Err(EvalError::invalid(
                &anthy.path,
                format!("題目 {:?} 不在 T03 corpus.1 fixture 內", item.surface),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::CorpusMetrics;

    struct ByReading;

    impl Converter for ByReading {
        fn convert(&self, reading: &str, top_n: usize) -> Vec<String> {
            let candidates: &[&str] = match reading {
                "きょう" => &["今日"],
                "がっこう" => &["學校", "学校"],
                _ => &[],
            };
            candidates
                .iter()
                .take(top_n)
                .map(|s| (*s).to_string())
                .collect()
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jc-eval-ja-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_corpus(dir: &Path, json: &str) -> PathBuf {
        let path = dir.join("corpus.json");
        std::fs::write(&path, json).unwrap();
        path
    }

    #[test]
    fn count_mismatch_is_rejected() {
        let dir = temp_dir("count");
        let path = write_corpus(
            &dir,
            r#"{"name":"x","license":"CC0","source":"test","count":2,
                "items":[{"id":"a","reading":"あ","surface":"亜"}]}"#,
        );
        let err = load_ja_corpus(&path).unwrap_err().to_string();
        assert!(err.contains("count 2"), "{err}");
    }

    #[test]
    fn segments_must_join_surface() {
        let dir = temp_dir("segments");
        let path = write_corpus(
            &dir,
            r#"{"name":"x","license":"CC0","source":"test","count":1,
                "items":[{"id":"a","reading":"あい","surface":"愛",
                          "segments":["愛","い"]}]}"#,
        );
        let err = load_ja_corpus(&path).unwrap_err().to_string();
        assert!(err.contains("segments 串接"), "{err}");
    }

    #[test]
    fn reading_whitespace_is_rejected() {
        let dir = temp_dir("space");
        let path = write_corpus(
            &dir,
            r#"{"name":"x","license":"CC0","source":"test","count":1,
                "items":[{"id":"a","reading":"あ い","surface":"愛"}]}"#,
        );
        let err = load_ja_corpus(&path).unwrap_err().to_string();
        assert!(err.contains("空白"), "{err}");
    }

    #[test]
    fn run_is_deterministic_and_scores_accept_variants() {
        let rules = RuleSet {
            schema: JA_SCHEMA,
            version: "test".into(),
            top_n: 5,
            timeout_ms: 100,
            sha256: "rules".into(),
            ..Default::default()
        };
        let corpus = JaCorpus {
            name: "fixture".into(),
            license: "CC0".into(),
            source: "test".into(),
            count: 2,
            items: vec![
                JaCase {
                    id: "a".into(),
                    reading: "きょう".into(),
                    surface: "今日".into(),
                    ..Default::default()
                },
                JaCase {
                    id: "b".into(),
                    reading: "がっこう".into(),
                    surface: "学校".into(),
                    accept: vec!["學校".into()],
                    segments: vec!["学".into(), "校".into()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let entries = vec![JaCorpusEntry {
            rule: CorpusRule::default(),
            corpus,
            sha256: "corpus".into(),
            path: PathBuf::from("fixture.json"),
        }];
        let converter = ByReading;
        let first = run_ja(
            &converter,
            &rules,
            &entries,
            EngineInfo::default(),
            &RunOptions::from_rules(&rules),
        );
        let second = run_ja(
            &converter,
            &rules,
            &entries,
            EngineInfo::default(),
            &RunOptions::from_rules(&rules),
        );
        assert_eq!(first.report.corpora, second.report.corpora);
        assert_eq!(first.report.corpora["fixture"].top1_rate, 1.0);
        assert_eq!(first.report.corpora["fixture"].top5_rate, 1.0);
        assert_eq!(first.report.corpora["fixture"].boundary_n, 1);
        assert_eq!(first.report.corpora["fixture"].boundary_exact_rate, 0.0);
    }

    #[test]
    fn gate_flags_missing_and_extra_corpora() {
        let mut base = Report::new(EngineInfo::default(), "rules");
        base.insert(
            "a",
            CorpusMetrics {
                n: 2,
                top1_rate: 0.5,
                ..Default::default()
            },
        );
        base.insert(
            "b",
            CorpusMetrics {
                n: 2,
                top1_rate: 0.5,
                ..Default::default()
            },
        );

        let mut now = Report::new(EngineInfo::default(), "rules");
        now.insert(
            "a",
            CorpusMetrics {
                n: 2,
                top1_rate: 0.5,
                ..Default::default()
            },
        );
        now.insert("c", CorpusMetrics::default());

        let result = Gate::default().check(&now, &base);
        assert!(!result.passed);
        assert!(
            result.problems.iter().any(|p| p.contains("b：")),
            "{:?}",
            result.problems
        );
        assert!(
            result.problems.iter().any(|p| p.contains("c：")),
            "{:?}",
            result.problems
        );
    }

    #[test]
    fn gate_allows_drop_within_tolerance() {
        let mut base = Report::new(EngineInfo::default(), "rules");
        base.insert(
            "a",
            CorpusMetrics {
                n: 10,
                top1_rate: 0.8,
                ..Default::default()
            },
        );
        let mut now = Report::new(EngineInfo::default(), "rules");
        now.insert(
            "a",
            CorpusMetrics {
                n: 10,
                top1_rate: 0.75,
                ..Default::default()
            },
        );

        let strict = Gate::default().check(&now, &base);
        assert!(!strict.passed);
        let relaxed = Gate {
            max_top1_drop: 0.1,
            ..Default::default()
        };
        assert!(relaxed.check(&now, &base).passed);
    }

    #[test]
    fn rule_set_gate_uses_gate_field() {
        let rules = RuleSet {
            gate: Gate {
                max_top1_drop: 0.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut base = Report::new(EngineInfo::default(), "r");
        base.insert(
            "a",
            CorpusMetrics {
                n: 1,
                top1_rate: 1.0,
                ..Default::default()
            },
        );
        let mut now = Report::new(EngineInfo::default(), "r");
        now.insert(
            "a",
            CorpusMetrics {
                n: 1,
                top1_rate: 0.0,
                ..Default::default()
            },
        );
        assert!(!rules.gate(&now, &base).passed);
    }

    #[test]
    fn real_corpora_match_rules_pins() {
        let rules_path = jc_testdata::require_data!("0c-testdata/ja/rules.json");
        let rules = load_rules(&rules_path).unwrap();
        assert_eq!(rules.top_n, 5);
        assert_eq!(rules.timeout_ms, 100);
        assert_eq!(rules.corpora.len(), 4);

        let corpora = load_ja_corpus_set(&rules, &rules_path).unwrap();
        assert_eq!(corpora.len(), 4);
        let modern = corpora
            .iter()
            .find(|c| c.corpus.name == "modern-500")
            .unwrap();
        assert_eq!(modern.corpus.items.len(), 500);
        let anthy = corpora
            .iter()
            .find(|c| c.corpus.name == "anthy-corpus1")
            .unwrap();
        assert_eq!(anthy.corpus.items.len(), 200);
        assert_eq!(
            anthy
                .corpus
                .items
                .iter()
                .filter(|i| !i.segments.is_empty())
                .count(),
            199
        );
    }

    #[test]
    fn real_baseline_loads() {
        let path = jc_testdata::require_data!("0c-testdata/ja/baseline.json");
        let baseline = load_baseline(&path).unwrap();
        assert_eq!(baseline.schema, SCHEMA);
        assert_eq!(baseline.corpora.len(), 4);
        let modern = &baseline.corpora["modern-500"];
        assert_eq!(modern.n, 500);
        assert!(modern.boundary_n > 0);
    }
}
