//! 引擎與 wrapper 之間的協定。長度前綴 + JSON，跨行程且可除錯。
//!
//! 訊框是 4-byte 小端長度前綴加上 JSON 內容；請求與回應都是列舉，以 `op`
//! 欄位區分。回應一律是完整快照，wrapper 不需要自己保存組字狀態。

pub mod frame;
pub mod message;

use serde::{Deserialize, Serialize};

pub use frame::{DEFAULT_MAX_FRAME, HEADER_LEN};
pub use jc_core::candidate::CandidateSource;
pub use jc_core::{Key, KeyCode, Mode, Snapshot};
pub use message::{read_message, read_payload, write_message};

/// 協定版本。握手時兩邊的版本必須相同。
///
/// 版本 2 在 [`Response::State`] 加上 `cursor_rune`、`suggestion`、`label` 與
/// `navigating`。版本 3 再加上 `details`、`segments` 與 `highlighted`。這些
/// 欄位都有預設值，舊 wrapper 讀到的 JSON 形狀只多不減，但握手版本一定要是
/// [`PROTOCOL_VERSION`]，不能靜默接受舊版本。
pub const PROTOCOL_VERSION: u32 = 3;

/// 一個候選的結構化資訊。
///
/// 與 [`Response::State`] 的 `candidates` **等長、同序**：`details[i]` 描述的
/// 就是 `candidates[i]`。wrapper 唯一的假設是這個對齊關係，不要自行重排。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateDetail {
    /// 讀音。注音為ㄅㄆㄇ，日文為平假名。
    pub reading: String,
    /// 候選來源。
    pub source: CandidateSource,
    /// 路徑成本，**越小越優先**。沿用 MeCab 的整數成本，不是機率。
    ///
    /// 只供除錯與排序說明；候選順序已由核心決定。
    pub cost: i32,
    /// 是否來自學習結果；`source == personal` 時為 `true`，其餘省略。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub learned: bool,
    /// 詞性名稱，來自日文 pack 的 POS 表；沒有詞性時省略。
    ///
    /// 注音候選沒有詞性，這個欄位一定是 `None`。名稱由共用表查出，候選本身
    /// 只保存 POS id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<String>,
}

/// 組字片段，位移一律以 **rune**（Unicode scalar value）計算。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentInfo {
    /// 片段起點（含），rune 位移。
    pub start: usize,
    /// 片段終點（不含），rune 位移。
    pub end: usize,
    /// 這個片段的輸入模式：`zhuyin`、`japanese`、`english` 或 `mixed`。
    pub mode: String,
    /// 書寫系統：`bopomofo`、`kana`、`han`、`latin`、`digit`、`symbol`、
    /// `mixed` 或 `other`。
    pub script: String,
}

/// 核心的 [`Mode`] 轉成線格式名稱。
fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Zh => "zhuyin",
        Mode::Ja => "japanese",
        Mode::En => "english",
        Mode::Mixed => "mixed",
    }
}

/// 引擎端回報的錯誤種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// 握手的協定版本與 [`PROTOCOL_VERSION`] 不同。
    VersionMismatch,
    /// JSON 內容無法解碼，或缺少必要欄位。
    BadFrame,
    /// 訊框超過長度上限。
    FrameTooLarge,
    /// `op` 不是引擎認得的操作。
    UnknownOp,
    /// 引擎內部狀態不允許這個請求。
    Internal,
}

/// wrapper 送給引擎的請求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// 握手，確認版本與客戶端身分。
    Hello { protocol: u32, client: String },
    /// 送出一個按鍵。
    Key { key: Key },
    /// 單按左右 Shift：在目前語言與純英文 En 之間切換。
    ///
    /// 引擎記住切換前的模式，再收到一次就回到原本的 Mixed 或 Ja。
    Toggle,
    /// 取得目前組字狀態。
    Snapshot,
    /// 送出目前的組字結果。
    Commit,
    /// 要求引擎結束行程。
    Shutdown,
    /// 引擎不認得的操作。由 serde 的 `other` 收下，讓引擎可以回
    /// [`ErrorCode::UnknownOp`] 而不是直接斷線。
    #[serde(other)]
    Unknown,
}

/// 引擎回給 wrapper 的結果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Response {
    /// 握手成功，附上引擎識別字串。
    Ready { protocol: u32, engine: String },
    /// 目前的完整組字快照。`cursor` 單位是字元（Unicode scalar value）。
    ///
    /// 版本 2 起多了 `cursor_rune`、`suggestion`、`label` 與 `navigating`；
    /// 版本 3 起多了 `details`、`segments` 與 `highlighted`。空值時不寫進
    /// JSON，舊 wrapper 看到的形狀與前一版相同。
    ///
    /// `cursor` 只為相容而留，權威值是 `cursor_rune`。
    State {
        preedit: String,
        cursor: usize,
        /// 候選字面，順序即顯示順序。
        candidates: Vec<String>,
        /// 與 `candidates` 等長、同序的結構化資訊。
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        details: Vec<CandidateDetail>,
        /// 組字片段，依序涵蓋整段 `preedit`。
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        segments: Vec<SegmentInfo>,
        /// 目前高亮的候選索引；0 時省略。
        #[serde(default, skip_serializing_if = "is_zero")]
        highlighted: usize,
        /// 游標的 rune 位置；與 `cursor` 同值，供需要 rune 位移的外殼使用。
        #[serde(default, skip_serializing_if = "is_zero")]
        cursor_rune: usize,
        /// Tab 會接受的補全尾綴。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        suggestion: Option<String>,
        /// 目前高亮候選的顯示原因。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        /// 是否已進入明確的候選選取模式。
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        navigating: bool,
    },
    /// 一次提交的結果。
    Committed { text: String },
    /// 請求被拒絕或處理失敗。
    Error { code: ErrorCode, message: String },
}

/// serde 的 `skip_serializing_if`：值為 0 時省略。
fn is_zero(value: &usize) -> bool {
    *value == 0
}

impl Response {
    /// 建立一個錯誤回應。
    pub fn error(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::Error {
            code,
            message: message.into(),
        }
    }

    /// 由核心快照建立 [`Response::State`]。
    ///
    /// 協定的 `cursor` 沿用版本 1 的定義：組字字串結尾的字元數。需要游標
    /// 真實位置的外殼用 `cursor_rune`，那才是權威值；目前核心的游標都在
    /// 結尾，兩者相同。
    ///
    /// 這是核心快照到線格式的唯一映射點。`details` 與 `candidates` 由同一個
    /// 向量產生，順序與長度必然一致；`segments` 的 byte 位移在這裡換成 rune。
    pub fn state(snapshot: jc_core::Snapshot) -> Self {
        let jc_core::Snapshot {
            preedit,
            cursor_rune,
            segments,
            candidates,
            highlighted,
            suggestion,
            label,
            navigating,
            ..
        } = snapshot;
        let cursor = preedit.chars().count();
        let details = candidates
            .iter()
            .map(|candidate| CandidateDetail {
                reading: candidate.reading.clone(),
                source: candidate.source,
                cost: candidate.cost,
                learned: candidate.source == CandidateSource::Personal,
                pos: candidate.pos_name().map(str::to_string),
            })
            .collect();
        let segments = segments
            .iter()
            .map(|segment| SegmentInfo {
                start: jc_core::output::rune_offset(&preedit, segment.start),
                end: jc_core::output::rune_offset(&preedit, segment.end),
                mode: mode_name(segment.mode).to_string(),
                script: segment.script.as_str().to_string(),
            })
            .collect();
        Self::State {
            cursor,
            candidates: candidates
                .into_iter()
                .map(|candidate| candidate.surface)
                .collect(),
            details,
            segments,
            highlighted,
            cursor_rune,
            suggestion,
            label,
            navigating,
            preedit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_uses_op_tag() {
        let text = serde_json::to_string(&Request::Snapshot).unwrap();
        assert_eq!(text, r#"{"op":"snapshot"}"#);

        let hello = Request::Hello {
            protocol: PROTOCOL_VERSION,
            client: "test".into(),
        };
        let text = serde_json::to_string(&hello).unwrap();
        assert_eq!(text, r#"{"op":"hello","protocol":3,"client":"test"}"#);
        assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), hello);
    }

    #[test]
    fn toggle_uses_op_tag() {
        let text = serde_json::to_string(&Request::Toggle).unwrap();
        assert_eq!(text, r#"{"op":"toggle"}"#);
        assert_eq!(
            serde_json::from_str::<Request>(&text).unwrap(),
            Request::Toggle
        );
    }

    #[test]
    fn protocol_version_is_three() {
        assert_eq!(PROTOCOL_VERSION, 3, "版本 3 才認得結構化候選欄位");
    }

    #[test]
    fn key_round_trips() {
        for key in [
            Key::char('ㄇ', false),
            Key::new(KeyCode::Backspace, None, false),
        ] {
            let req = Request::Key { key };
            let text = serde_json::to_string(&req).unwrap();
            assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), req);
        }
    }

    #[test]
    fn key_uses_core_shape() {
        let text = serde_json::to_string(&Request::Key {
            key: Key::char('ㄇ', false),
        })
        .unwrap();
        assert_eq!(
            text,
            r#"{"op":"key","key":{"code":{"kind":"char","value":"ㄇ"},"text":"ㄇ","shift":false}}"#
        );
    }

    #[test]
    fn unknown_op_is_captured_instead_of_failing() {
        let req: Request = serde_json::from_str(r#"{"op":"teleport"}"#).unwrap();
        assert_eq!(req, Request::Unknown);
    }

    #[test]
    fn error_codes_are_snake_case() {
        let resp = Response::error(ErrorCode::VersionMismatch, "版本不符");
        let text = serde_json::to_string(&resp).unwrap();
        assert_eq!(
            text,
            r#"{"op":"error","code":"version_mismatch","message":"版本不符"}"#
        );
        assert_eq!(serde_json::from_str::<Response>(&text).unwrap(), resp);
    }

    #[test]
    fn state_omits_empty_optional_fields() {
        let resp = Response::state(jc_core::Snapshot::from_preedit("ㄇㄚ"));
        let text = serde_json::to_string(&resp).unwrap();
        assert_eq!(
            text,
            r#"{"op":"state","preedit":"ㄇㄚ","cursor":2,"candidates":[],"cursor_rune":2}"#
        );
    }

    #[test]
    fn state_serializes_suggestion_label_and_navigating() {
        let snapshot = jc_core::Snapshot {
            suggestion: Some("ㄚ".into()),
            label: Some("common".into()),
            navigating: true,
            ..jc_core::Snapshot::from_preedit("ㄇ")
        };
        let text = serde_json::to_string(&Response::state(snapshot)).unwrap();
        assert_eq!(
            text,
            r#"{"op":"state","preedit":"ㄇ","cursor":1,"candidates":[],"cursor_rune":1,"suggestion":"ㄚ","label":"common","navigating":true}"#
        );
    }

    #[test]
    fn state_accepts_version_one_shape() {
        let old = r#"{"op":"state","preedit":"我","cursor":1,"candidates":["我"]}"#;
        let resp: Response = serde_json::from_str(old).unwrap();
        assert_eq!(
            resp,
            Response::State {
                preedit: "我".into(),
                cursor: 1,
                candidates: vec!["我".into()],
                details: vec![],
                segments: vec![],
                highlighted: 0,
                cursor_rune: 0,
                suggestion: None,
                label: None,
                navigating: false,
            }
        );
    }

    #[test]
    fn state_omits_structure_when_snapshot_is_empty() {
        let text = serde_json::to_string(&Response::state(jc_core::Snapshot::empty())).unwrap();
        assert_eq!(
            text,
            r#"{"op":"state","preedit":"","cursor":0,"candidates":[]}"#
        );
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        for key in ["details", "segments", "highlighted"] {
            assert!(value.get(key).is_none(), "{key} 不應出現在空快照");
        }
    }

    #[test]
    fn state_details_align_with_candidate_list() {
        let candidates = vec![
            jc_core::candidate::Candidate::new(
                "我",
                "ㄨㄛˇ",
                12,
                jc_core::candidate::CandidateSource::Lexicon,
            ),
            jc_core::candidate::Candidate::new(
                "窩",
                "ㄨㄛ",
                -3,
                jc_core::candidate::CandidateSource::Personal,
            ),
        ];
        let snapshot = jc_core::Snapshot {
            candidates: candidates.clone(),
            highlighted: 1,
            ..jc_core::Snapshot::from_preedit("ㄨㄛˇ")
        };
        match Response::state(snapshot) {
            Response::State {
                candidates: surfaces,
                details,
                highlighted,
                ..
            } => {
                assert_eq!(surfaces, ["我", "窩"]);
                assert_eq!(
                    details.len(),
                    surfaces.len(),
                    "details 必須與 candidates 同長"
                );
                for (i, detail) in details.iter().enumerate() {
                    assert_eq!(
                        detail.reading, candidates[i].reading,
                        "details[{i}].reading"
                    );
                    assert_eq!(detail.source, candidates[i].source, "details[{i}].source");
                    assert_eq!(detail.cost, candidates[i].cost, "details[{i}].cost");
                }
                assert!(!details[0].learned, "lexicon 不是學習結果");
                assert!(details[1].learned, "personal 就是學習結果");
                assert_eq!(highlighted, 1);
            }
            other => panic!("應是 State，得到 {other:?}"),
        }
    }

    #[test]
    fn state_maps_pos_id_to_name_and_omits_missing_pos() {
        let table = jc_core::candidate::PosTable::new(["名詞,普通名詞,一般,*", "動詞,自立,*,*"]);
        let candidates = vec![
            jc_core::candidate::Candidate::new(
                "今日",
                "きょう",
                15,
                jc_core::candidate::CandidateSource::Lexicon,
            )
            .with_pos(0, table),
            jc_core::candidate::Candidate::new(
                "我",
                "ㄨㄛˇ",
                12,
                jc_core::candidate::CandidateSource::Lexicon,
            ),
        ];
        let snapshot = jc_core::Snapshot {
            candidates,
            ..jc_core::Snapshot::from_preedit("きょう")
        };
        let text = serde_json::to_string(&Response::state(snapshot)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let details = value["details"].as_array().expect("details");
        assert_eq!(details[0]["pos"], "名詞,普通名詞,一般,*");
        assert!(
            details[1].get("pos").is_none(),
            "注音候選沒有詞性，不得輸出 pos"
        );
    }

    #[test]
    fn state_segments_cover_preedit_in_runes() {
        let preedit = "ㄨㄛˇabc";
        let snapshot = jc_core::Snapshot {
            segments: jc_core::segment::split(preedit),
            ..jc_core::Snapshot::from_preedit(preedit)
        };
        let segments = match Response::state(snapshot) {
            Response::State { segments, .. } => segments,
            other => panic!("應是 State，得到 {other:?}"),
        };
        // 核心給的是 UTF-8 byte 位移，線上一律換成 rune。
        assert_eq!(segments.len(), 2);
        assert_eq!((segments[0].start, segments[0].end), (0, 3));
        assert_eq!(segments[0].mode, "zhuyin");
        assert_eq!(segments[0].script, "bopomofo");
        assert_eq!((segments[1].start, segments[1].end), (3, 6));
        assert_eq!(segments[1].mode, "mixed");
        assert_eq!(segments[1].script, "latin");

        let mut at = 0;
        for segment in &segments {
            assert_eq!(segment.start, at, "片段必須相接");
            assert!(segment.end > segment.start, "片段不可為空");
            at = segment.end;
        }
        assert_eq!(at, preedit.chars().count(), "片段必須覆蓋整段 preedit");
    }

    #[test]
    fn state_accepts_version_two_shape_with_defaults() {
        let old = r#"{"op":"state","preedit":"ㄨㄛˇ","cursor":3,"candidates":["我","窩"],"cursor_rune":3,"suggestion":"ˇ","label":"常用字","navigating":true}"#;
        let resp: Response = serde_json::from_str(old).unwrap();
        match resp {
            Response::State {
                preedit,
                candidates,
                details,
                segments,
                highlighted,
                ..
            } => {
                assert_eq!(preedit, "ㄨㄛˇ");
                assert_eq!(candidates, ["我", "窩"]);
                assert!(details.is_empty(), "v2 形狀沒有 details");
                assert!(segments.is_empty(), "v2 形狀沒有 segments");
                assert_eq!(highlighted, 0);
            }
            other => panic!("應是 State，得到 {other:?}"),
        }
    }

    #[test]
    fn state_carries_candidate_surfaces() {
        let snapshot = jc_core::Snapshot {
            candidates: vec![
                jc_core::candidate::Candidate::new(
                    "我",
                    "ㄨㄛˇ",
                    1,
                    jc_core::candidate::CandidateSource::Lexicon,
                ),
                jc_core::candidate::Candidate::new(
                    "窩",
                    "ㄨㄛ",
                    2,
                    jc_core::candidate::CandidateSource::Lexicon,
                ),
            ],
            ..jc_core::Snapshot::from_preedit("ㄨㄛˇ")
        };
        match Response::state(snapshot) {
            Response::State {
                candidates,
                cursor,
                cursor_rune,
                ..
            } => {
                assert_eq!(candidates, ["我", "窩"]);
                assert_eq!(cursor, 3);
                assert_eq!(cursor_rune, 3);
            }
            other => panic!("應是 State，得到 {other:?}"),
        }
    }
}
