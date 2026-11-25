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

/// 檢查 13 的 id。
pub const UI_PROBE_ID: &str = "ui.probe";
/// 檢查 13 的標題。
pub const UI_PROBE_TITLE: &str = "UI 探針";

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
    /// 最近一次慢請求的 op；只有慢請求檢查會填。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_slow_op: Option<String>,
    /// 最近一次慢請求的耗時（毫秒）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_slow_ms: Option<u64>,
    /// 最近一次慢請求的輸入長度（位元組）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_slow_input_len: Option<usize>,
    /// 檢查 13（UI 探針）的 cell；只有那一項會填。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui_probe: Option<UiProbeCells>,
}

/// 檢查 13 回報的 cell。
///
/// 人類輸出與 JSON 共用同一組值；`candidate_rect` 用 `l,t,r,b` 字串，
/// 沒有看到候選窗時是空字串。`latency_ms` 是送出最後一個鍵到第一次看到
/// 可見候選窗的毫秒數，沒看到時記 0。`committed_text` 是候選窗開著時按
/// Enter 之後 EDIT 的文字，正常要含「我」。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UiProbeCells {
    /// `interactive` 或 `headless`。
    pub desktop: String,
    /// 組字字串；取不到時是空字串。
    pub composition: String,
    /// 是否看到可見的候選窗。
    pub candidate_visible: bool,
    /// 候選窗的 `l,t,r,b`；沒有時是空字串。
    pub candidate_rect: String,
    /// 第一次看到候選窗的延遲（毫秒）。
    pub latency_ms: u64,
    /// 按 Esc 之後候選窗是否消失。
    pub escape_hides: bool,
    /// 候選窗開著時按 Enter 之後 EDIT 的文字。
    pub committed_text: String,
}

impl UiProbeCells {
    /// 無互動桌面時的 cell：什麼都沒驗到。
    pub fn headless() -> Self {
        Self {
            desktop: "headless".to_string(),
            composition: String::new(),
            candidate_visible: false,
            candidate_rect: String::new(),
            latency_ms: 0,
            escape_hides: false,
            committed_text: String::new(),
        }
    }

    /// 互動桌面但還沒開始量測時的 cell。
    pub fn interactive() -> Self {
        Self {
            desktop: "interactive".to_string(),
            ..Self::headless()
        }
    }

    /// 一行 `key=value` 摘要，供 `detail` 使用。
    pub fn detail(&self) -> String {
        format!(
            "desktop={}；composition={}；candidate_visible={}；candidate_rect={}；latency_ms={}；escape_hides={}；committed_text={}",
            self.desktop,
            self.composition,
            self.candidate_visible,
            self.candidate_rect,
            self.latency_ms,
            self.escape_hides,
            self.committed_text
        )
    }
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
    version: &'static str,
    ok: bool,
    /// 整份報告的三態：`fail`（任一 fail）、`skip`（沒有 fail 但有 skip）、
    /// `pass`（其他）。只看 `ok` 分不出「全部驗過」與「有項目沒驗到」。
    status: Overall,
    summary: Summary,
    checks: &'a [Check],
}

/// 整份報告的狀態。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Overall {
    /// 全部 pass 或 warn。
    Pass,
    /// 至少一項 fail。
    Fail,
    /// 沒有 fail，但至少一項 skip（無法驗證）。
    Skip,
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

    /// 記一項慢請求警告：不影響通過與否，但 JSON 會帶診斷欄位。
    pub fn warn_slow(
        &mut self,
        id: &str,
        title: &str,
        detail: impl Into<String>,
        op: &str,
        ms: u64,
        input_len: usize,
    ) {
        self.push(id, title, Status::Warn, detail.into(), None);
        let check = self.checks.last_mut().expect("剛加入一項檢查");
        check.last_slow_op = Some(op.to_string());
        check.last_slow_ms = Some(ms);
        check.last_slow_input_len = Some(input_len);
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

    /// 記檢查 13（UI 探針）並附上 cell。
    pub fn ui_probe(
        &mut self,
        status: Status,
        detail: impl Into<String>,
        fix: Option<String>,
        cells: UiProbeCells,
    ) {
        self.push(UI_PROBE_ID, UI_PROBE_TITLE, status, detail.into(), fix);
        let check = self.checks.last_mut().expect("剛加入一項檢查");
        check.ui_probe = Some(cells);
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

    /// 整份報告的三態。
    pub fn overall(&self) -> Overall {
        let summary = self.summary();
        if summary.fail > 0 {
            Overall::Fail
        } else if summary.skip > 0 {
            Overall::Skip
        } else {
            Overall::Pass
        }
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
            version: env!("CARGO_PKG_VERSION"),
            ok: self.failed() == 0,
            status: self.overall(),
            summary: self.summary(),
            checks: &self.checks,
        };
        // 這裡的欄位全部是可序列化的基本型別，不會失敗。
        serde_json::to_string_pretty(&document).unwrap_or_else(|error| {
            format!(
                r#"{{"schema":"{SCHEMA}","version":"{}","ok":false,"status":"fail","summary":{{"pass":0,"warn":0,"fail":1,"skip":0}},"checks":[],"serialize_error":"{error}"}}"#,
                env!("CARGO_PKG_VERSION")
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
            last_slow_op: None,
            last_slow_ms: None,
            last_slow_input_len: None,
            ui_probe: None,
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
        assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["ok"], false);
        assert_eq!(value["status"], "fail");
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
        assert_eq!(value["status"], "skip", "有 skip 時只看 ok 會誤以為全驗過");
    }

    #[test]
    fn status_is_pass_when_only_pass_and_warn() {
        let mut report = Report::new(false);
        report.pass("a", "A", "pass");
        report.warn("b", "B", "warn");
        assert_eq!(report.overall(), Overall::Pass);
        let value: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
        assert_eq!(value["status"], "pass");
        assert_eq!(value["ok"], true);
    }

    #[test]
    fn slow_request_is_a_warning_with_json_fields() {
        let mut report = Report::new(false);
        report.pass("engine.detail", "引擎細節", "正常");
        report.warn_slow(
            "engine.slow",
            "慢請求",
            "最近一次慢請求 op=key、耗時 412ms、輸入長度 37 位元組",
            "key",
            412,
            37,
        );
        assert_eq!(report.failed(), 0, "慢請求不影響通過與否");
        assert_eq!(report.warned(), 1);

        let value: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
        let check = &value["checks"][1];
        assert_eq!(check["status"], "warn");
        assert_eq!(check["last_slow_op"], "key");
        assert_eq!(check["last_slow_ms"], 412);
        assert_eq!(check["last_slow_input_len"], 37);
    }

    #[test]
    fn headless_ui_probe_cells_are_empty() {
        let cells = UiProbeCells::headless();
        assert_eq!(cells.desktop, "headless");
        assert_eq!(cells.composition, "");
        assert!(!cells.candidate_visible);
        assert_eq!(cells.candidate_rect, "");
        assert_eq!(cells.latency_ms, 0);
        assert!(!cells.escape_hides);
        assert_eq!(cells.committed_text, "");
        assert_eq!(UiProbeCells::interactive().desktop, "interactive");
    }

    #[test]
    fn ui_probe_check_serializes_cells_and_keeps_ok() {
        let mut report = Report::new(false);
        report.ui_probe(
            Status::Skip,
            "desktop=headless（沒有互動桌面）",
            None,
            UiProbeCells::headless(),
        );
        let value: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
        let check = &value["checks"][0];
        assert_eq!(check["id"], UI_PROBE_ID);
        assert_eq!(check["title"], UI_PROBE_TITLE);
        assert_eq!(check["status"], "skip");
        assert_eq!(check["ui_probe"]["desktop"], "headless");
        assert_eq!(check["ui_probe"]["candidate_visible"], false);
        assert_eq!(check["ui_probe"]["latency_ms"], 0);
        assert_eq!(check["ui_probe"]["committed_text"], "");
        assert_eq!(value["ok"], true);
        assert_eq!(value["status"], "skip");

        let mut failed = Report::new(false);
        failed.ui_probe(
            Status::Fail,
            "候選窗沒有出現",
            Some("看 tsf.log".to_string()),
            UiProbeCells {
                desktop: "interactive".to_string(),
                composition: "ㄨ".to_string(),
                candidate_visible: false,
                candidate_rect: String::new(),
                latency_ms: 2001,
                escape_hides: false,
                committed_text: String::new(),
            },
        );
        let value: serde_json::Value = serde_json::from_str(&failed.to_json()).unwrap();
        let check = &value["checks"][0];
        assert_eq!(check["status"], "fail");
        assert_eq!(check["fix"], "看 tsf.log");
        assert_eq!(check["ui_probe"]["composition"], "ㄨ");
        assert_eq!(check["ui_probe"]["latency_ms"], 2001);
        assert_eq!(value["ok"], false);
        assert_eq!(value["status"], "fail");
    }

    #[test]
    fn ui_probe_cells_detail_lists_every_cell() {
        let cells = UiProbeCells {
            desktop: "interactive".to_string(),
            composition: "ㄨㄛ".to_string(),
            candidate_visible: true,
            candidate_rect: "10,20,110,60".to_string(),
            latency_ms: 87,
            escape_hides: true,
            committed_text: "我".to_string(),
        };
        assert_eq!(
            cells.detail(),
            "desktop=interactive；composition=ㄨㄛ；candidate_visible=true；candidate_rect=10,20,110,60；latency_ms=87；escape_hides=true；committed_text=我"
        );
    }
}
