//! 檢查 13 的判定規則。
//!
//! 這裡不碰 Win32：Windows 側只負責量測，把結果整理成 [`Observation`]，由
//! [`assess`] 決定 PASS／FAIL／無法判定、由 [`cells`] 產生 JSON 的 `ui_probe`
//! 物件。Linux 的 `cargo test` 也會跑這裡的單元測試，所以「哪些 cell 算 fail」
//! 不需要 Windows 實機就能驗。
//!
//! 判定原則：
//!
//! - `composition` 是盡力取得的組字字串，只回報、不參與判定。CUAS 的 text
//!   store 常常讀不到，空字串不代表失敗。
//! - 候選窗出現之後，第一次 Enter 就送出整段、第二次 Enter 沒上屏或送錯字、
//!   Esc 關不掉、位置不對都是 **FAIL**：按鍵已經進到 TIP，這是真的產品行為。
//! - 讀不到組字字串且候選窗完全沒出現是 **inconclusive**：TIP 根本沒接到
//!   按鍵，先懷疑前景／TSF／桌面環境，不能算產品 bug。

use crate::report::UiProbeCells;

/// 96 DPI 下的水平容許差（DIP）；實際容許值會依 monitor DPI 放大。
pub const MAX_DX_DIP: i32 = 300;
/// 候選窗上緣與插入點的容許垂直差下限；負值代表視窗翻到插入點上方。
pub const MIN_DY: i32 = -200;
/// 候選窗上緣與插入點的容許垂直差上限。
pub const MAX_DY: i32 = 160;
/// 換算 DIP 的基準 DPI。
pub const BASE_DPI: u32 = 96;
/// `ji3` 的第一個候選；第二次 Enter 之後 EDIT 必須有這個字。
pub const EXPECTED_COMMIT: &str = "我";

/// 一個矩形（螢幕座標）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// 左緣。
    pub left: i32,
    /// 上緣。
    pub top: i32,
    /// 右緣。
    pub right: i32,
    /// 下緣。
    pub bottom: i32,
}

impl Rect {
    /// 寬或高不是正的。
    pub fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    /// `other` 完全落在自己裡面。
    pub fn contains(&self, other: &Rect) -> bool {
        other.left >= self.left
            && other.top >= self.top
            && other.right <= self.right
            && other.bottom <= self.bottom
    }

    /// 兩個矩形有面積大於零的重疊。
    pub fn intersects(&self, other: &Rect) -> bool {
        other.left < self.right
            && other.right > self.left
            && other.top < self.bottom
            && other.bottom > self.top
    }
}

impl std::fmt::Display for Rect {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{},{},{},{}",
            self.left, self.top, self.right, self.bottom
        )
    }
}

/// 一次探針量到的東西。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// EDIT 的組字字串；盡力取得的回報值，讀不到時為空，不參與判定。
    pub composition: String,
    /// 是否看到可見的候選窗。
    pub candidate_visible: bool,
    /// 候選窗 rect；沒看到時是 `None`。
    pub candidate_rect: Option<Rect>,
    /// 送出最後一個鍵到第一次看到候選窗的延遲（毫秒）。
    pub latency_ms: u64,
    /// 候選窗開著時第一次 Enter 之後 EDIT 的文字；正確應該還是空的。
    pub first_enter_text: String,
    /// 第二次 Enter 送出整段之後 EDIT 的文字。
    pub committed_text: String,
    /// 插入點的螢幕座標。
    pub caret: Option<(i32, i32)>,
    /// 虛擬螢幕 rect。
    pub screen: Option<Rect>,
    /// 插入點所在 monitor 的 work area。
    pub work_area: Option<Rect>,
    /// 第二輪候選窗是否真的出現；只有這時才驗得到 Esc。
    pub escape_verified: bool,
    /// 送 Esc 之後候選窗是否消失。
    pub escape_hides: bool,
}

impl Observation {
    /// 還沒量到任何東西的初始值。
    pub fn empty() -> Self {
        Self {
            composition: String::new(),
            candidate_visible: false,
            candidate_rect: None,
            latency_ms: 0,
            first_enter_text: String::new(),
            committed_text: String::new(),
            caret: None,
            screen: None,
            work_area: None,
            escape_verified: false,
            escape_hides: false,
        }
    }
}

/// 一次量測的判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// 所有斷言都成立。
    Pass,
    /// 互動桌面上真的有產品行為不成立。
    Fail,
    /// 沒有 TIP 收到按鍵的證據：無法判定產品行為，不算失敗。
    Inconclusive,
}

/// [`assess`] 的結果：判定與不成立的地方。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assessment {
    /// 這次量測的判定。
    pub decision: Decision,
    /// 不成立的地方；`Inconclusive` 時是環境問題的說明。
    pub problems: Vec<String>,
}

/// 判定這次量測是 PASS、FAIL 還是 inconclusive。
///
/// `composition` 只是回報值，不影響判定：CUAS 的 text store 不保證讀得到組字
/// 字串。只有連候選窗都沒出現、又讀不到組字字串時，才代表按鍵沒有進到 TIP
/// （前景或 TSF 沒接上、桌面環境不對），這種量測不能用來判定產品 bug。只要
/// 看到候選窗，就代表按鍵已經進到 TIP；之後 Enter 沒上屏、Esc 關不掉或位置
/// 不對都是真的 FAIL。
pub fn assess(observation: &Observation, dpi: u32) -> Assessment {
    // 候選窗只要有 rect，就代表它真的被建立過；即使不可見也是產品行為，
    // 不能當成「TIP 沒接到按鍵」。
    if observation.composition.trim().is_empty()
        && !observation.candidate_visible
        && observation.candidate_rect.is_none()
    {
        return Assessment {
            decision: Decision::Inconclusive,
            problems: vec![
                "讀不到組字字串且候選窗沒有出現：TIP 沒有接到按鍵，這是環境問題，無法判定產品行為"
                    .to_string(),
            ],
        };
    }

    let problems = problems(observation, dpi);
    let decision = if problems.is_empty() {
        Decision::Pass
    } else {
        Decision::Fail
    };
    Assessment { decision, problems }
}

/// 依 monitor DPI 換算的水平容許差；300 DIP 在 96 DPI 是 300px、在 144 DPI 是 450px。
pub fn max_dx(dpi: u32) -> i32 {
    let dpi = if dpi == 0 { BASE_DPI } else { dpi };
    ((MAX_DX_DIP as i64) * (dpi as i64) / (BASE_DPI as i64)) as i32
}

/// 回傳所有不成立的地方；空 `Vec` 代表 PASS。
pub fn problems(observation: &Observation, dpi: u32) -> Vec<String> {
    let mut problems = Vec::new();

    match observation.candidate_rect {
        None => problems.push(format!(
            "{}ms 內沒有看到本行程可見的 jcCandidateUiWindow",
            observation.latency_ms
        )),
        Some(rect) => {
            if !observation.candidate_visible {
                problems.push(format!("候選窗存在但不可見：rect={rect}"));
            }
            if rect.is_empty() {
                problems.push(format!("候選窗 rect 是空的：{rect}"));
            }
            match observation.screen {
                Some(screen) if !screen.contains(&rect) => {
                    problems.push(format!("候選窗 rect 不在虛擬螢幕 {screen} 內：{rect}"))
                }
                None => problems.push("取不到虛擬螢幕範圍".to_string()),
                _ => {}
            }
            match observation.work_area {
                Some(work) if !rect.intersects(&work) => problems.push(format!(
                    "候選窗 rect 與插入點所在 monitor 的 work area {work} 不相交：{rect}"
                )),
                None => problems.push("取不到插入點所在 monitor 的 work area".to_string()),
                _ => {}
            }
            match observation.caret {
                Some((x, y)) => {
                    let dx = rect.left - x;
                    let dy = rect.top - y;
                    let allowed = max_dx(dpi);
                    if dx.abs() > allowed || !(MIN_DY..=MAX_DY).contains(&dy) {
                        problems.push(format!(
                            "候選窗位置不合理：rect={rect}、插入點=({x},{y})、dx={dx}（容許 ±{allowed}）、dy={dy}（容許 {MIN_DY}..={MAX_DY}）"
                        ));
                    }
                }
                None => problems
                    .push("取不到 EDIT 插入點（GetCaretPos 與 client rect 都失敗）".to_string()),
            }
        }
    }

    // 只有候選窗真的出現時，Enter 與 Esc 才有東西可驗；沒出現時上面已經
    // 有「逾時」這條，避免再補兩條誤導的訊息。
    if observation.candidate_visible {
        // 第一次 Enter 只該套用高亮候選；整段必須等第二次 Enter。
        if !observation.first_enter_text.trim().is_empty() {
            problems.push(format!(
                "候選窗開著時第一次 Enter 就送出整段「{}」；應該只套用高亮候選、關窗、保留組字",
                observation.first_enter_text
            ));
        }
        if !observation.committed_text.contains(EXPECTED_COMMIT) {
            if observation.committed_text.trim().is_empty() {
                problems.push(
                    "第二次 Enter 後 EDIT 仍沒有文字：第一次 Enter 沒上屏、第二次也沒送出，可能是 CUAS 或 TIP 插入路徑"
                        .to_string(),
                );
            } else {
                problems.push(format!(
                    "第二次 Enter 送出的字不是第一候選，沒有包含「{EXPECTED_COMMIT}」：EDIT 的文字是「{}」",
                    observation.committed_text
                ));
            }
        }

        if !observation.escape_verified {
            problems.push("第二輪候選窗沒有再出現，無法驗證 Esc 會關閉候選窗".to_string());
        } else if !observation.escape_hides {
            problems.push("按 Esc 後 1000ms 內候選窗仍可見".to_string());
        }
    }

    problems
}

/// 把量測結果整理成 JSON 的 `ui_probe` cell。
#[cfg_attr(not(windows), allow(dead_code))]
pub fn cells(observation: &Observation, desktop: &str) -> UiProbeCells {
    UiProbeCells {
        desktop: desktop.to_string(),
        composition: observation.composition.clone(),
        candidate_visible: observation.candidate_visible,
        candidate_rect: observation
            .candidate_rect
            .map(|rect| rect.to_string())
            .unwrap_or_default(),
        latency_ms: observation.latency_ms,
        escape_hides: observation.escape_hides,
        committed_text: observation.committed_text.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    /// 1920x1080、插入點在 (400,300)、候選窗在插入點右下方、Enter 送出「我」。
    fn healthy() -> Observation {
        Observation {
            composition: "ㄨㄛˇ".to_string(),
            candidate_visible: true,
            candidate_rect: Some(rect(400, 320, 600, 360)),
            latency_ms: 87,
            first_enter_text: String::new(),
            committed_text: "我".to_string(),
            caret: Some((400, 300)),
            screen: Some(rect(0, 0, 1920, 1080)),
            work_area: Some(rect(0, 0, 1920, 1040)),
            escape_verified: true,
            escape_hides: true,
        }
    }

    #[test]
    fn healthy_observation_has_no_problems() {
        assert_eq!(problems(&healthy(), 96), Vec::<String>::new());
    }

    #[test]
    fn healthy_assessment_passes() {
        let assessment = assess(&healthy(), 96);
        assert_eq!(assessment.decision, Decision::Pass);
        assert_eq!(assessment.problems, Vec::<String>::new());
    }

    #[test]
    fn no_keys_received_is_inconclusive_not_fail() {
        let mut observation = healthy();
        observation.composition = String::new();
        observation.candidate_visible = false;
        observation.candidate_rect = None;
        observation.latency_ms = 2000;
        observation.committed_text = String::new();
        observation.escape_verified = false;
        observation.escape_hides = false;

        let assessment = assess(&observation, 96);
        assert_eq!(assessment.decision, Decision::Inconclusive);
        assert!(
            assessment.problems[0].contains("TIP 沒有接到按鍵"),
            "{:?}",
            assessment.problems
        );
    }

    #[test]
    fn composition_started_without_candidate_is_fail() {
        let mut observation = healthy();
        observation.candidate_visible = false;
        observation.candidate_rect = None;
        observation.latency_ms = 2000;
        observation.committed_text = String::new();
        observation.escape_verified = false;
        observation.escape_hides = false;

        let assessment = assess(&observation, 96);
        assert_eq!(assessment.decision, Decision::Fail);
        assert!(
            assessment
                .problems
                .iter()
                .any(|problem| problem.contains("2000ms 內沒有看到")),
            "{:?}",
            assessment.problems
        );
    }

    #[test]
    fn composition_empty_but_ui_works_is_pass() {
        let mut observation = healthy();
        observation.composition = String::new();

        // CUAS 的 text store 不一定給組字字串；候選窗、Enter、Esc 都對就是
        // 端到端成功，不能因為 composition 空就判失敗。
        let assessment = assess(&observation, 96);
        assert_eq!(assessment.decision, Decision::Pass);
        assert_eq!(assessment.problems, Vec::<String>::new());
        assert_eq!(cells(&observation, "interactive").composition, "");
    }

    #[test]
    fn composition_empty_and_candidate_missing_is_inconclusive() {
        let mut observation = healthy();
        observation.composition = String::new();
        observation.candidate_visible = false;
        observation.candidate_rect = None;
        observation.latency_ms = 2000;
        observation.committed_text = String::new();
        observation.escape_verified = false;
        observation.escape_hides = false;

        let assessment = assess(&observation, 96);
        assert_eq!(assessment.decision, Decision::Inconclusive);
        assert!(
            assessment
                .problems
                .iter()
                .any(|problem| problem.contains("TIP 沒有接到按鍵")),
            "{:?}",
            assessment.problems
        );
    }

    #[test]
    fn invisible_candidate_without_composition_is_fail() {
        let mut observation = healthy();
        observation.composition = String::new();
        observation.candidate_visible = false;

        // 有候選窗 rect 就代表視窗真的建立過；不可見才是要抓的 bug，
        // 不能因為 composition 空就退成 inconclusive。
        let assessment = assess(&observation, 96);
        assert_eq!(assessment.decision, Decision::Fail);
        assert!(
            assessment
                .problems
                .iter()
                .any(|problem| problem.contains("存在但不可見")),
            "{:?}",
            assessment.problems
        );
    }

    #[test]
    fn candidate_appears_and_escape_hides_but_enter_does_not_commit_is_fail() {
        let mut observation = healthy();
        observation.committed_text = String::new();
        observation.escape_verified = true;
        observation.escape_hides = true;

        let assessment = assess(&observation, 96);
        assert_eq!(assessment.decision, Decision::Fail);
        assert!(
            assessment
                .problems
                .iter()
                .any(|problem| problem.contains("CUAS") && problem.contains("Enter")),
            "{:?}",
            assessment.problems
        );
    }

    #[test]
    fn blank_composition_adds_no_problem() {
        let mut observation = healthy();
        observation.composition = "  ".to_string();
        assert_eq!(problems(&observation, 96), Vec::<String>::new());
    }

    #[test]
    fn missing_candidate_reports_latency_only() {
        let mut observation = healthy();
        observation.candidate_visible = false;
        observation.candidate_rect = None;
        observation.latency_ms = 2001;
        observation.committed_text = String::new();
        let problems = problems(&observation, 96);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("2001ms 內沒有看到"), "{problems:?}");
        assert!(
            !problems.iter().any(|problem| problem.contains("CUAS")),
            "沒有候選窗時不該補 Enter 的訊息：{problems:?}"
        );
    }

    #[test]
    fn empty_rect_fails() {
        let mut observation = healthy();
        observation.candidate_rect = Some(rect(100, 100, 100, 100));
        let problems = problems(&observation, 96);
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("rect 是空的")),
            "{problems:?}"
        );
    }

    #[test]
    fn rect_outside_virtual_screen_fails() {
        let mut observation = healthy();
        observation.candidate_rect = Some(rect(1800, 320, 2000, 360));
        let problems = problems(&observation, 96);
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("不在虛擬螢幕")),
            "{problems:?}"
        );
    }

    #[test]
    fn rect_outside_work_area_fails_even_inside_screen() {
        let mut observation = healthy();
        // 工作列在下方：work area 到 1040，候選窗掉到 1050。
        observation.candidate_rect = Some(rect(400, 1050, 600, 1080));
        let problems = problems(&observation, 96);
        assert!(
            problems.iter().any(|problem| problem.contains("work area")),
            "{problems:?}"
        );
    }

    #[test]
    fn horizontal_tolerance_scales_with_dpi() {
        let mut observation = healthy();
        observation.candidate_rect = Some(rect(720, 320, 920, 360));
        // dx = 320：96 DPI 容許 300 → fail；120 DPI 容許 375 → pass。
        assert!(problems(&observation, 96)
            .iter()
            .any(|problem| problem.contains("位置不合理")));
        assert_eq!(problems(&observation, 120), Vec::<String>::new());
        assert_eq!(max_dx(96), 300);
        assert_eq!(max_dx(120), 375);
        assert_eq!(max_dx(0), 300, "取不到 DPI 時用 96");
    }

    #[test]
    fn candidate_above_caret_is_allowed() {
        let mut observation = healthy();
        observation.candidate_rect = Some(rect(400, 120, 600, 160));
        // dy = -180：翻到上方仍算正常。
        assert_eq!(problems(&observation, 96), Vec::<String>::new());

        observation.candidate_rect = Some(rect(400, 40, 600, 80));
        // dy = -260：太遠。
        assert!(problems(&observation, 96)
            .iter()
            .any(|problem| problem.contains("位置不合理")));
    }

    #[test]
    fn missing_caret_fails() {
        let mut observation = healthy();
        observation.caret = None;
        let problems = problems(&observation, 96);
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("取不到 EDIT 插入點")),
            "{problems:?}"
        );
    }

    #[test]
    fn empty_commit_reports_cuas_hint() {
        let mut observation = healthy();
        observation.committed_text = String::new();
        let problems = problems(&observation, 96);
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("CUAS") && problem.contains("Enter")),
            "{problems:?}"
        );
    }

    #[test]
    fn wrong_commit_fails() {
        let mut observation = healthy();
        observation.committed_text = "喔".to_string();
        let problems = problems(&observation, 96);
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("沒有包含「我」")),
            "{problems:?}"
        );
    }

    #[test]
    fn first_enter_committing_the_whole_composition_fails() {
        let mut observation = healthy();
        observation.first_enter_text = "我".to_string();
        let problems = problems(&observation, 96);
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("第一次 Enter 就送出整段")),
            "{problems:?}"
        );
    }

    #[test]
    fn empty_second_enter_reports_first_enter_hint() {
        let mut observation = healthy();
        observation.committed_text = String::new();
        let problems = problems(&observation, 96);
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("第一次 Enter 沒上屏")),
            "{problems:?}"
        );
    }

    #[test]
    fn wrong_second_enter_reports_first_candidate() {
        let mut observation = healthy();
        observation.committed_text = "喔".to_string();
        let problems = problems(&observation, 96);
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("第二次 Enter 送出的字不是第一候選")),
            "{problems:?}"
        );
    }

    #[test]
    fn commit_may_contain_more_than_expected() {
        let mut observation = healthy();
        observation.committed_text = "我的".to_string();
        assert_eq!(problems(&observation, 96), Vec::<String>::new());
    }

    #[test]
    fn escape_must_be_verified_on_a_second_window() {
        let mut observation = healthy();
        observation.escape_verified = false;
        observation.escape_hides = false;
        let first = problems(&observation, 96);
        assert!(
            first
                .iter()
                .any(|problem| problem.contains("第二輪候選窗沒有再出現")),
            "{first:?}"
        );

        observation.escape_verified = true;
        let problems = problems(&observation, 96);
        assert!(
            problems.iter().any(|problem| problem.contains("按 Esc 後")),
            "{problems:?}"
        );
    }

    #[test]
    fn cells_carry_every_measurement() {
        let observation = healthy();
        let probe_cells = cells(&observation, "interactive");
        assert_eq!(probe_cells.desktop, "interactive");
        assert_eq!(probe_cells.composition, "ㄨㄛˇ");
        assert!(probe_cells.candidate_visible);
        assert_eq!(probe_cells.candidate_rect, "400,320,600,360");
        assert_eq!(probe_cells.latency_ms, 87);
        assert!(probe_cells.escape_hides);
        assert_eq!(probe_cells.committed_text, "我");

        let empty = cells(&Observation::empty(), "headless");
        assert_eq!(empty.candidate_rect, "");
        assert_eq!(empty.committed_text, "");
    }
}
