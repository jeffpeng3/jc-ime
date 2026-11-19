//! 報告輸出：`ja/baseline.json` 形狀的 JSON 與人類可讀表格。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{EvalError, Result};
use crate::metrics::{CaseResult, CorpusMetrics};

/// 報告與 baseline 的格式版本。
pub const SCHEMA: u32 = 1;

/// 產出這份報告的引擎與資料識別。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EngineInfo {
    /// 引擎名稱，例如 `stats`。
    #[serde(default)]
    pub name: String,
    /// 資料目錄。
    #[serde(default)]
    pub data: String,
    /// 詞典雜湊。
    #[serde(default)]
    pub lexicon_sha256: String,
    /// 連線矩陣雜湊。
    #[serde(default)]
    pub connection_sha256: String,
}

/// 一份可與 `ja/baseline.json` 逐欄對照的報告。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// 格式版本。
    pub schema: u32,
    /// 產生這份報告的 `rules.json` 雜湊。
    #[serde(default)]
    pub rules_sha256: String,
    /// 引擎與資料識別。
    pub engine: EngineInfo,
    /// 量測日期字串；格式由呼叫端決定。
    #[serde(default)]
    pub measured_on: String,
    /// 題庫名稱對應的聚合指標。
    pub corpora: BTreeMap<String, CorpusMetrics>,
}

impl Report {
    /// 建立空報告。
    pub fn new(engine: EngineInfo, rules_sha256: impl Into<String>) -> Self {
        Self {
            schema: SCHEMA,
            rules_sha256: rules_sha256.into(),
            engine,
            measured_on: String::new(),
            corpora: BTreeMap::new(),
        }
    }

    /// 加入一個題庫的指標。
    pub fn insert(&mut self, name: impl Into<String>, metrics: CorpusMetrics) {
        self.corpora.insert(name.into(), metrics);
    }

    /// 輸出成與 baseline 相同形狀的 JSON。
    pub fn to_json_pretty(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|e| EvalError::Encode(e.to_string()))
    }

    /// 輸出成人類可讀的固定寬度表格。
    pub fn to_table(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "{:<24} {:>5} {:>7} {:>7} {:>8} {:>7} {:>8} {:>8} {:>8}\n",
            "corpus", "n", "top1", "top5", "b_exact", "b_f1", "p50_ms", "p95_ms", "timeout"
        ));
        for (name, m) in &self.corpora {
            out.push_str(&format!(
                "{:<24} {:>5} {:>6.1}% {:>6.1}% {:>7.1}% {:>7.4} {:>8.3} {:>8.3} {:>7.1}%\n",
                name,
                m.n,
                m.top1_rate * 100.0,
                m.top5_rate * 100.0,
                m.boundary_exact_rate * 100.0,
                m.boundary_f1,
                m.latency_p50_ms,
                m.latency_p95_ms,
                m.timeout_rate * 100.0
            ));
        }
        out
    }
}

/// 一個題庫的逐題結果，供人工對照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorpusRun {
    /// 題庫名稱。
    pub name: String,
    /// 授權。
    #[serde(default)]
    pub license: String,
    /// 來源。
    #[serde(default)]
    pub source: String,
    /// 題庫檔雜湊。
    #[serde(default)]
    pub sha256: String,
    /// 逐題結果。
    pub items: Vec<CaseResult>,
}

/// 一次完整執行的結果：聚合報告加逐題證據。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// 與 baseline 相同形狀的聚合報告。
    pub report: Report,
    /// 每個題庫的逐題結果。
    pub corpora: Vec<CorpusRun>,
}

impl Run {
    /// 輸出聚合報告 JSON。
    pub fn to_json_pretty(&self) -> Result<String> {
        self.report.to_json_pretty()
    }

    /// 輸出含逐題結果的 JSON。
    pub fn to_items_json_pretty(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|e| EvalError::Encode(e.to_string()))
    }

    /// 輸出人類可讀表格。
    pub fn to_table(&self) -> String {
        self.report.to_table()
    }
}

/// 退步判定結果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GateResult {
    /// 全部指標是否都在容許範圍內。
    pub passed: bool,
    /// 未通過的項目；每行一個原因。
    pub problems: Vec<String>,
}

impl GateResult {
    /// 由問題清單建立結果。
    pub fn from_problems(problems: Vec<String>) -> Self {
        Self {
            passed: problems.is_empty(),
            problems,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(n: usize, top1: f64, f1: f64) -> CorpusMetrics {
        CorpusMetrics {
            n,
            top1_rate: top1,
            top5_rate: top1,
            boundary_n: n,
            boundary_exact_rate: top1,
            boundary_f1: f1,
            latency_p50_ms: 0.125,
            latency_p95_ms: 0.5,
            timeout_rate: 0.0,
        }
    }

    #[test]
    fn report_json_uses_baseline_keys() {
        let mut report = Report::new(EngineInfo::default(), "abc");
        report.insert("modern-500", metrics(500, 0.232, 0.445));
        let json = report.to_json_pretty().unwrap();
        for key in [
            "\"schema\"",
            "\"rules_sha256\"",
            "\"engine\"",
            "\"measured_on\"",
            "\"corpora\"",
            "\"latency_p50_ms_informational\"",
            "\"timeout_rate_informational\"",
        ] {
            assert!(json.contains(key), "缺少 {key}：\n{json}");
        }
        assert!(json.contains("\"moder"));
        let loaded: Report = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded, report);
    }

    #[test]
    fn table_has_one_row_per_corpus() {
        let mut report = Report::new(EngineInfo::default(), "abc");
        report.insert("hard-cases", metrics(35, 0.1428, 0.2375));
        let table = report.to_table();
        assert!(table.starts_with("corpus"));
        assert!(table.contains("hard-cases"));
        assert!(table.contains("14.3%"));
    }

    #[test]
    fn gate_flags_drop_beyond_tolerance() {
        let mut base = Report::new(EngineInfo::default(), "rules");
        base.insert("modern-500", metrics(500, 0.24, 0.45));
        let mut now = Report::new(EngineInfo::default(), "rules");
        now.insert("modern-500", metrics(500, 0.20, 0.45));

        let strict = GateResult::from_problems(vec!["drop".to_string()]);
        assert!(!strict.passed);

        let gate = crate::ja::Gate {
            max_top1_drop: 0.05,
            max_top5_drop: 0.05,
            max_boundary_exact_drop: 0.05,
            max_boundary_f1_drop: 0.05,
        };
        let pass = gate.check(&now, &base);
        assert!(pass.passed, "問題：{:?}", pass.problems);
    }
}
