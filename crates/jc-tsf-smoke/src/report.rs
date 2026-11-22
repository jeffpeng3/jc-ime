//! 檢查結果的輸出格式。
//!
//! 人類模式每一項檢查固定輸出一行：`[標籤] 編號. 標題：說明`；失敗項目再補一行
//! `       修復：...`。標籤只有四種：
//!
//! - `[PASS]`：通過
//! - `[WARN]`：有問題但不影響結束碼
//! - `[FAIL]`：失敗，會讓行程以 1 結束
//! - `[SKIP]`：無法判定（例如引擎沒開、輸入法沒安裝），不算失敗
//!
//! 加上 `--json` 時，人類可讀的行改走 stderr，stdout 只在最後輸出一個
//! `jc-ime-doctor/1` JSON 物件。這個模組不碰 Win32，Linux 的 `cargo check`
//! 也會編譯它，JSON 序列化與計數測試因此在 Linux 就能跑。

use serde::Serialize;

/// 通過。
pub const PASS: &str = "[PASS]";
/// 警告。
pub const WARN: &str = "[WARN]";
/// 失敗。
pub const FAIL: &str = "[FAIL]";
/// 無法判定。
pub const SKIP: &str = "[SKIP]";
/// 輔助資訊。
pub const INFO: &str = "[INFO]";

/// JSON schema 的識別字；格式改動時要一起改。
pub const SCHEMA: &str = "jc-ime-doctor/1";

/// 單項檢查的狀態。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// 通過。
    Pass,
    /// 有問題但不影響結束碼。
    Warn,
    /// 失敗，結束碼為 1。
    Fail,
    /// 無法判定。
    Skip,
}

impl Status {
    /// 人類輸出用的標籤。
    pub fn tag(self) -> &'static str {
        match self {
            Self::Pass => PASS,
            Self::Warn => WARN,
            Self::Fail => FAIL,
            Self::Skip => SKIP,
        }
    }
}

/// 一項檢查的結果，同時是人類輸出的資料來源與 JSON 的 `checks` 元素。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// 穩定的機器可讀 id，例如 `profile.machine`。
    pub id: String,
    /// 人類可讀標題（不含編號）。
    pub title: String,
    /// 判定結果。
    pub status: Status,
    /// 一行說明。
    pub detail: String,
    /// 失敗時可執行的下一步；其他狀態不輸出這個欄位。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

/// JSON 的 `summary` 物件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// 通過數。
    pub pass: usize,
    /// 警告數。
    pub warn: usize,
    /// 失敗數。
    pub fail: usize,
    /// 略過數。
    pub skip: usize,
}

/// 完整的 JSON 文件。
#[derive(Debug, Clone, Serialize)]
struct JsonReport<'a> {
    schema: &'static str,
    ok: bool,
    summary: Summary,
    checks: &'a [Check],
}

/// 累計各項結果並負責輸出。
#[derive(Debug, Default)]
pub struct Report {
    checks: Vec<Check>,
    json: bool,
}

impl Report {
    /// 建立報表；`json` 為真時 stdout 只留最後的 JSON。
    pub fn new(json: bool) -> Self {
        Self {
            checks: Vec::new(),
            json,
        }
    }

    /// 記一項通過。
    pub fn pass(&mut self, id: &str, title: &str, detail: impl Into<String>) {
        self.push(id, title, Status::Pass, detail.into(), None);
    }

    /// 記一項警告。
    pub fn warn(&mut self, id: &str, title: &str, detail: impl Into<String>) {
        self.push(id, title, Status::Warn, detail.into(), None);
    }

    /// 記一項失敗；`fix` 是給使用者的可執行下一步。
    pub fn fail(
        &mut self,
        id: &str,
        title: &str,
        detail: impl Into<String>,
        fix: impl Into<String>,
    ) {
        self.push(id, title, Status::Fail, detail.into(), Some(fix.into()));
    }

    /// 記一項無法判定。
    pub fn skip(&mut self, id: &str, title: &str, detail: impl Into<String>) {
        self.push(id, title, Status::Skip, detail.into(), None);
    }

    /// 印一行輔助資訊，不計入任何計數。
    pub fn info(&self, detail: impl AsRef<str>) {
        self.write_line(&format!("{INFO} {}", detail.as_ref()));
    }

    /// 印一行標題或說明（不計入任何計數）。
    pub fn heading(&self, text: impl AsRef<str>) {
        self.write_line(text.as_ref());
    }

    /// 失敗項數；行程結束碼由它決定。
    pub fn failed(&self) -> usize {
        self.count(Status::Fail)
    }

    /// 警告項數。
    pub fn warned(&self) -> usize {
        self.count(Status::Warn)
    }

    /// 計數摘要。
    pub fn summary(&self) -> Summary {
        Summary {
            pass: self.count(Status::Pass),
            warn: self.count(Status::Warn),
            fail: self.count(Status::Fail),
            skip: self.count(Status::Skip),
        }
    }

    /// 一行中文摘要，例如 `8 通過、1 警告、2 失敗`。
    pub fn summary_text(&self) -> String {
        let summary = self.summary();
        let mut text = format!(
            "{} 通過、{} 警告、{} 失敗",
            summary.pass, summary.warn, summary.fail
        );
        if summary.skip > 0 {
            text.push_str(&format!("、{} 略過", summary.skip));
        }
        text
    }

    /// 一行總結。
    pub fn verdict(&self) -> String {
        let summary = self.summary();
        if summary.fail > 0 {
            format!(
                "安裝診斷失敗；請依上面各項的「修復」處理後重跑（{} 項失敗）",
                summary.fail
            )
        } else if self.warned() > 0 {
            format!("安裝可用，但有 {} 項警告", summary.warn)
        } else {
            "全部通過".to_string()
        }
    }

    /// 序列化成 `jc-ime-doctor/1` JSON 物件。
    pub fn to_json(&self) -> String {
        let document = JsonReport {
            schema: SCHEMA,
            ok: self.failed() == 0,
            summary: self.summary(),
            checks: &self.checks,
        };
        // 這裡的欄位全部是可序列化的基本型別，不會失敗。
        serde_json::to_string_pretty(&document).unwrap_or_else(|error| {
            format!(
                r#"{{"schema":"{SCHEMA}","ok":false,"summary":{{"pass":0,"warn":0,"fail":1,"skip":0}},"checks":[],"serialize_error":"{error}"}}"#
            )
        })
    }

    /// 印出最後的摘要；JSON 模式另外把 JSON 寫到 stdout。
    pub fn finish(&self) {
        self.heading("");
        self.heading(format!("結果：{}", self.summary_text()));
        self.heading(format!("總結：{}", self.verdict()));
        if self.json {
            println!("{}", self.to_json());
        }
    }

    /// 加入一項並即時輸出。
    fn push(&mut self, id: &str, title: &str, status: Status, detail: String, fix: Option<String>) {
        let ordinal = self.checks.len() + 1;
        self.write_line(&line(status.tag(), &format!("{ordinal}. {title}"), &detail));
        if let Some(fix) = &fix {
            self.write_line(&format!("       修復：{fix}"));
        }
        self.checks.push(Check {
            id: id.to_string(),
            title: title.to_string(),
            status,
            detail,
            fix,
        });
    }

    /// 人類模式下寫 stdout；JSON 模式下改寫 stderr，讓 stdout 只有 JSON。
    fn write_line(&self, text: &str) {
        if self.json {
            eprintln!("{text}");
        } else {
            println!("{text}");
        }
    }

    fn count(&self, status: Status) -> usize {
        self.checks
            .iter()
            .filter(|check| check.status == status)
            .count()
    }
}

/// 把 32 位元值格式化成 `0x` 開頭的大寫十六進位。
pub fn hex(value: u32) -> String {
    format!("0x{value:08X}")
}

/// 組出一行輸出，格式固定為 `[標籤] 標題：說明`。
pub fn line(tag: &str, title: &str, detail: &str) -> String {
    format!("{tag} {title}：{detail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Report {
        let mut report = Report::new(false);
        report.pass("dll.load", "DLL 載入", "S_OK");
        report.warn("install.version", "版本資源", "沒有版本資源");
        report.fail(
            "profile.user",
            "使用者啟用",
            "IsEnabledLanguageProfile=false",
            "以目前使用者執行 jc-tip-register user-install",
        );
        report.skip("engine.pipe", "引擎 pipe", "pipe 不存在");
        report
    }

    #[test]
    fn line_format_is_stable() {
        assert_eq!(
            line(PASS, "1. LoadLibraryW", "S_OK"),
            "[PASS] 1. LoadLibraryW：S_OK"
        );
        assert_eq!(
            line(FAIL, "2. DllGetClassObject", "HRESULT 0x80040111"),
            "[FAIL] 2. DllGetClassObject：HRESULT 0x80040111"
        );
        assert_eq!(
            line(SKIP, "6. 引擎 pipe", "沒開"),
            "[SKIP] 6. 引擎 pipe：沒開"
        );
        assert_eq!(line(INFO, "tid", "1"), "[INFO] tid：1");
    }

    #[test]
    fn hex_pads_to_eight_digits() {
        assert_eq!(hex(0), "0x00000000");
        assert_eq!(hex(0x8000_4005), "0x80004005");
        assert_eq!(hex(0xffff_ffff), "0xFFFFFFFF");
    }

    #[test]
    fn summary_counts_each_status() {
        let report = sample();
        assert_eq!(
            report.summary(),
            Summary {
                pass: 1,
                warn: 1,
                fail: 1,
                skip: 1,
            }
        );
        assert_eq!(report.failed(), 1);
        assert_eq!(report.warned(), 1);
        assert_eq!(report.summary_text(), "1 通過、1 警告、1 失敗、1 略過");
    }

    #[test]
    fn skip_is_hidden_from_summary_when_zero() {
        let mut report = Report::new(false);
        report.pass("a", "A", "ok");
        report.pass("b", "B", "ok");
        assert_eq!(report.summary_text(), "2 通過、0 警告、0 失敗");
        assert_eq!(report.verdict(), "全部通過");
    }

    #[test]
    fn warn_only_is_not_a_failure() {
        let mut report = Report::new(false);
        report.pass("a", "A", "ok");
        report.warn("b", "B", "有問題");
        assert_eq!(report.failed(), 0);
        assert_eq!(report.verdict(), "安裝可用，但有 1 項警告");
    }

    #[test]
    fn json_document_has_fixed_schema_and_fields() {
        let report = sample();
        let text = report.to_json();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();

        assert_eq!(value["schema"], SCHEMA);
        assert_eq!(value["ok"], false);
        assert_eq!(value["summary"]["pass"], 1);
        assert_eq!(value["summary"]["warn"], 1);
        assert_eq!(value["summary"]["fail"], 1);
        assert_eq!(value["summary"]["skip"], 1);

        let checks = value["checks"].as_array().unwrap();
        assert_eq!(checks.len(), 4);
        assert_eq!(checks[0]["id"], "dll.load");
        assert_eq!(checks[0]["title"], "DLL 載入");
        assert_eq!(checks[0]["status"], "pass");
        assert!(checks[0].get("fix").is_none());
        assert_eq!(checks[2]["status"], "fail");
        assert_eq!(
            checks[2]["fix"],
            "以目前使用者執行 jc-tip-register user-install"
        );
        assert_eq!(checks[3]["status"], "skip");
    }

    #[test]
    fn json_status_only_uses_four_values() {
        for (status, text) in [
            (Status::Pass, "\"pass\""),
            (Status::Warn, "\"warn\""),
            (Status::Fail, "\"fail\""),
            (Status::Skip, "\"skip\""),
        ] {
            assert_eq!(serde_json::to_string(&status).unwrap(), text);
        }
    }

    #[test]
    fn ok_is_true_when_only_warn_and_skip() {
        let mut report = Report::new(false);
        report.warn("a", "A", "warn");
        report.skip("b", "B", "skip");
        let value: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
        assert_eq!(value["ok"], true);
    }
}
