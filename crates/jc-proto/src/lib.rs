//! 引擎與 wrapper 之間的協定。長度前綴 + JSON，跨行程且可除錯。
//!
//! 訊框是 4-byte 小端長度前綴加上 JSON 內容；請求與回應都是列舉，以 `op`
//! 欄位區分。回應一律是完整快照，wrapper 不需要自己保存組字狀態。

pub mod frame;
pub mod message;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use frame::{DEFAULT_MAX_FRAME, HEADER_LEN};
pub use jc_core::candidate::CandidateSource;
pub use jc_core::{Key, KeyCode, Lang, Mode, Snapshot};
pub use message::{read_message, read_payload, write_message};

/// 協定版本。握手時兩邊的版本必須相同。
///
/// 版本 2 在 [`Response::State`] 加上 `cursor_rune`、`suggestion`、`label` 與
/// `navigating`。版本 3 再加上 `details`、`segments` 與 `highlighted`。這些
/// 欄位都有預設值，舊 wrapper 讀到的 JSON 形狀只多不減，但握手版本一定要是
/// [`PROTOCOL_VERSION`]，不能靜默接受舊版本。
///
/// 0c adapter 的擴充 op（見 [`Request`]）沿用版本 3：新 op 都是獨立 variant，
/// 既有 hello／key／toggle／snapshot／commit 的 JSON 形狀完全不變，舊 DLL 不必
/// 升級；不認識新 op 的舊引擎由 [`Request::Unknown`] 收下並回 `UnknownOp`。
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

/// 引擎健康狀態，對應 0c `Reply.Info`。
///
/// `health`、`apply_settings` 與 `drain` 都回報這個結構。各欄位都有預設值，
/// 舊引擎只回 `ok` 時也能解成全預設。字形開關（`output_simplified`）放在這裡
/// 而不是 [`Response::State`]，因為它是引擎的全域設定，與單次組字無關；T1a
/// 的 Snapshot 轉換從 [`HealthInfo::as_info`] 讀取。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthInfo {
    /// 是否正在更新交接。0c 收到 `true` 會準備重連新版引擎。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub draining: bool,
    /// 目前連線數。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub connections: usize,
    /// 距離最近一次按鍵的秒數；尚未收到按鍵時省略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_seconds: Option<u64>,
    /// session 總數。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub sessions: usize,
    /// 正在組字的 session 數。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub composing: usize,
    /// 引擎行程 id；僅供診斷。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// 目前是否輸出簡體字。T1a 的 `learn_simplified()` 從這裡讀值。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub output_simplified: bool,
    /// 目前 session 的英文詞表詞數；原型詞表約 104 詞。
    ///
    /// 安裝後 smoke 用它確認引擎真的載入 `english/words.bin`，而不是退回原型。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub english_words: usize,
    /// 引擎是否支援清除學習紀錄（0c `personal_reset`）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub personal_reset: bool,
    /// 啟動時的中英模式：`english` 或 `zhuyin`。外殼據此決定開機是否直接英文。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_mode: Option<String>,
    /// 其他能力旗標；0c 以 `info[name] == "true"` 判定是否支援。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub capabilities: BTreeMap<String, bool>,
    /// 最近一次超過 200ms 的請求 op；沒有慢請求時省略。
    ///
    /// 實機回報變慢時，`doctor` 與 TSF 的 `tsf.log` 直接讀這個欄位，
    /// 不必回頭翻引擎 log。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_slow_op: Option<String>,
    /// 上面那個慢請求的耗時（毫秒）；沒有慢請求時省略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_slow_ms: Option<u64>,
    /// 上面那個慢請求的線路 JSON 長度（位元組）；沒有慢請求時省略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_slow_input_len: Option<usize>,
}

impl HealthInfo {
    /// 轉成 0c 的 `info` map（值一律是字串，`true`/`false` 或數字）。
    ///
    /// 0c 的外殼與 T1a 的 `learn_simplified()` 都讀這個形狀，所以和能力判斷
    /// 有關的欄位在這裡統一輸出；新增欄位只會多 key，不會改既有 key。
    pub fn as_info(&self) -> BTreeMap<String, String> {
        let mut info = BTreeMap::new();
        info.insert("draining".to_string(), self.draining.to_string());
        info.insert("connections".to_string(), self.connections.to_string());
        if let Some(seconds) = self.idle_seconds {
            info.insert("idle_seconds".to_string(), seconds.to_string());
        }
        info.insert("sessions".to_string(), self.sessions.to_string());
        info.insert("composing".to_string(), self.composing.to_string());
        info.insert("english_words".to_string(), self.english_words.to_string());
        if let Some(pid) = self.pid {
            info.insert("pid".to_string(), pid.to_string());
        }
        info.insert("settings_deferred_apply".to_string(), "true".to_string());
        if let Some(mode) = &self.start_mode {
            info.insert("start_mode".to_string(), mode.clone());
            info.insert(
                "initial_mode".to_string(),
                if mode == "english" {
                    "english"
                } else {
                    "zhuyin"
                }
                .to_string(),
            );
        }
        if self.personal_reset {
            info.insert("personal_reset".to_string(), "true".to_string());
        }
        if self.output_simplified {
            info.insert("output_simplified".to_string(), "true".to_string());
        }
        // 慢請求診斷：TSF 的 health 處理與 doctor 都從 info map 讀這些 key。
        if let Some(op) = &self.last_slow_op {
            info.insert("last_slow_op".to_string(), op.clone());
        }
        if let Some(ms) = self.last_slow_ms {
            info.insert("last_slow_ms".to_string(), ms.to_string());
        }
        if let Some(len) = self.last_slow_input_len {
            info.insert("last_slow_input_len".to_string(), len.to_string());
        }
        for (name, enabled) in &self.capabilities {
            info.insert(name.clone(), enabled.to_string());
        }
        info
    }
}

/// `vocab` 指令的回報。
///
/// 0c 的 `VocabResponse` 欄位很多且仍會變動。T3 只固定成功旗標與存檔提示，
/// 其餘交給 `raw` 原樣攜帶，等 T6 接手自訂詞管理時再展開。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VocabInfo {
    /// 指令是否成功（0c `VocabResponse.ok`）。
    #[serde(default)]
    pub ok: bool,
    /// 是否改了詞彙資料，宿主要存檔。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub changed: bool,
    /// 指令層級的錯誤訊息。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 引擎回傳的完整 JSON。T6 之前不解釋內容。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
}

/// 核心的片段語言 [`Lang`] 轉成線格式名稱。
fn mode_name(mode: Lang) -> &'static str {
    match mode {
        Lang::Zh => "zhuyin",
        Lang::Ja => "japanese",
        Lang::En => "english",
        Lang::Mixed => "mixed",
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
    /// 引擎認得這個 op，但目前的版本還沒實作。
    ///
    /// 0c adapter 的 `context`／`mode`／`caps_lock`／`output_simplified` 在
    /// T6 接手前回這個，讓外殼能區分「協定不認識」與「尚未支援」。
    #[serde(rename = "unsupported")]
    Unsupported,
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

    // ---- 以下是 0c adapter 需要的擴充 op，全部沿用協定版本 3。 ----
    // 每個欄位都有 #[serde(default)]，舊 DLL 省略欄位或引擎還沒支援時都能
    // 序列化／反序列化；T1a 送出的精簡 JSON（只帶 on／context_id 等）也收得下。
    /// 0c `ping`：最簡易的存活檢查。引擎回 [`Response::Acknowledged`]。
    Ping,
    /// 0c `health`：查版本與狀態，不含文字。引擎回 [`Response::Health`]。
    Health,
    /// 0c `apply_settings`：等所有輸入框組字完成後再套用設定。
    ///
    /// T4／T6 才會真的延遲套用；T3 階段只保證協定能收發，引擎回
    /// [`Response::Health`] 讓外殼知道已收到。
    ApplySettings {
        /// 要套用的設定，鍵值對原樣攜帶，引擎目前不解讀。
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        settings: BTreeMap<String, String>,
    },
    /// 0c `drain`：更新交接。沒有組字時存檔並退出，外殼重連新版引擎。
    ///
    /// 真正動作在 T4；此處只定義訊息。
    Drain,
    /// 0c `vocab`：自訂詞彙管理指令，不需要 session。
    ///
    /// 真正的詞彙操作在 T6；此處保留完整指令讓上層原樣送到引擎。
    Vocab {
        /// 指令名稱，例如 `add`、`status`、`learned-undo`。
        #[serde(default, skip_serializing_if = "String::is_empty")]
        directive: String,
        /// 0c 信封的 `vocab` 物件，T6 之前不解釋。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        vocab: Option<serde_json::Value>,
    },
    /// 0c `reset_personal`：清空個人學習紀錄並立刻存檔。
    ///
    /// 真正動作在 T4；此處只定義訊息。
    ResetPersonal,
    /// 0c `close`：關閉一個 session。
    ///
    /// session 歸屬檢查在 T4；此處只定義訊息。
    Close {
        /// 要關閉的 session id；省略代表 0。
        #[serde(default, skip_serializing_if = "is_zero_u64")]
        session: u64,
    },
    /// 0c `handle` 的 `context` 請求：確認最近提交的文件位置或清除前文。
    ///
    /// T6 才會真的接上下文；jc 沒有對應狀態，引擎回「不支援」而不是
    /// [`ErrorCode::UnknownOp`]。
    Context {
        /// `true` 表示確認，`false` 表示清除。
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        on: bool,
        /// 提交時拿到的文件位置的代號，不是文件文字。
        #[serde(default, skip_serializing_if = "is_zero_u64")]
        context_id: u64,
        /// 外殼是否具備文件位置確認能力。
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        context_enabled: bool,
    },
    /// 0c `handle` 的 `mode` 請求：設定中英模式。
    ///
    /// T6 才會真的切換；此處讓引擎回明確的「不支援」。
    Mode {
        /// `true` 為英文，`false` 為注音。
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        on: bool,
    },
    /// 0c `handle` 的 `caps_lock` 請求：按鍵事件帶的 Caps Lock 狀態。
    ///
    /// T6 才會真的判斷；此處讓引擎回明確的「不支援」。
    CapsLock {
        /// Caps Lock 是否亮著。
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        on: bool,
    },
    /// 0c `handle` 的 `output_simplified` 請求：開關輸出簡體字。
    ///
    /// 引擎把它設成目前 session 的執行期旗標；轉換發生在 commit，組字中的
    /// 預覽仍是繁體。轉換器缺席時這個請求不會開啟功能。
    OutputSimplified {
        /// `true` 為輸出簡體。
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        on: bool,
    },
    /// 引擎不認得的操作。由 serde 的 `other` 收下，讓引擎可以回
    /// [`ErrorCode::UnknownOp`] 而不是直接斷線。
    #[serde(other)]
    Unknown,
}

/// 引擎回給 wrapper 的結果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Response {
    /// 握手成功，附上引擎識別字串與這條連線拿到的 session id。
    ///
    /// `session` 由引擎在 `Hello` 配置，adapter 記住後用在同一連線的 `Close`，
    /// 避免第二條以後的連線仍送 `session = 1` 而被歸屬檢查拒絕。舊引擎的 Ready
    /// 沒有這個欄位，反序列化時用 [`default_ready_session`] 補 1，與 T1a 原本
    /// 寫死 1 的行為一致。
    Ready {
        /// 協定版本。
        protocol: u32,
        /// 引擎識別字串。
        engine: String,
        /// 這條連線的 session id；舊訊框缺省時為 1。
        #[serde(default = "default_ready_session")]
        session: u64,
    },
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

    /// `ping`、`close`、`reset_personal` 的成功回報。
    ///
    /// 0c 的 Reply 一律有 `ok: true`；外殼那一刻不在意內容，所以這裡只帶
    /// 可選的 session id。
    Acknowledged {
        /// 適用時回傳 session id；省略代表 0。
        #[serde(default, skip_serializing_if = "is_zero_u64")]
        session: u64,
    },
    /// `health`、`apply_settings`、`drain` 的回報。
    ///
    /// T1a 的 `response_to_reply()` 會把 `info` 攤成 0c 的 `info` map，
    /// `learn_simplified()` 再從 `output_simplified` 讀字形開關。
    Health {
        /// 各項健康資訊，欄位都有預設值。
        #[serde(default)]
        info: HealthInfo,
    },
    /// `vocab` 的回報。
    Vocab {
        /// 詞彙指令結果；T6 之前只固定 `ok`／`changed`。
        #[serde(default)]
        vocab: VocabInfo,
    },
}

/// serde 的 `skip_serializing_if`：值為 0 時省略。
fn is_zero(value: &usize) -> bool {
    *value == 0
}

/// `u64` 版的零值省略；session id 與 context id 用。
fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

/// 舊引擎 Ready 沒有 `session` 欄位時補 1，與 T1a 原本的固定值一致。
fn default_ready_session() -> u64 {
    1
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

    // ---- 0c adapter 擴充 op 的 wire 測試 ----

    #[test]
    fn extension_ops_use_snake_case_wire_names() {
        let cases: Vec<(Request, &str)> = vec![
            (Request::Ping, r#"{"op":"ping"}"#),
            (Request::Health, r#"{"op":"health"}"#),
            (Request::Drain, r#"{"op":"drain"}"#),
            (Request::ResetPersonal, r#"{"op":"reset_personal"}"#),
            (
                Request::ApplySettings {
                    settings: BTreeMap::new(),
                },
                r#"{"op":"apply_settings"}"#,
            ),
            (
                Request::ApplySettings {
                    settings: BTreeMap::from([("zh_simplified".into(), "true".into())]),
                },
                r#"{"op":"apply_settings","settings":{"zh_simplified":"true"}}"#,
            ),
            (
                Request::Vocab {
                    directive: "add".into(),
                    vocab: None,
                },
                r#"{"op":"vocab","directive":"add"}"#,
            ),
            (
                Request::Close { session: 7 },
                r#"{"op":"close","session":7}"#,
            ),
            (
                Request::Context {
                    on: true,
                    context_id: 42,
                    context_enabled: true,
                },
                r#"{"op":"context","on":true,"context_id":42,"context_enabled":true}"#,
            ),
            (Request::Mode { on: true }, r#"{"op":"mode","on":true}"#),
            (
                Request::CapsLock { on: true },
                r#"{"op":"caps_lock","on":true}"#,
            ),
            (
                Request::OutputSimplified { on: true },
                r#"{"op":"output_simplified","on":true}"#,
            ),
        ];
        for (request, expected) in cases {
            let text = serde_json::to_string(&request).unwrap();
            assert_eq!(text, expected, "{request:?} 的 wire 名稱");
            assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), request);
        }
    }

    #[test]
    fn extension_ops_default_missing_fields() {
        let request: Request = serde_json::from_str(r#"{"op":"close"}"#).unwrap();
        assert_eq!(request, Request::Close { session: 0 });

        let request: Request = serde_json::from_str(r#"{"op":"mode"}"#).unwrap();
        assert_eq!(request, Request::Mode { on: false });

        let request: Request =
            serde_json::from_str(r#"{"op":"context","on":true,"context_id":9}"#).unwrap();
        assert_eq!(
            request,
            Request::Context {
                on: true,
                context_id: 9,
                context_enabled: false,
            }
        );
    }

    #[test]
    fn t1a_handle_request_shape_round_trips() {
        // T1a 的 to_jc_request() 會保留 req 的 type 欄位，只換 op；serde 預設
        // 忽略未知欄位，所以這些請求仍能解到新 variant。
        let raw = r#"{"type":"caps_lock","on":true,"op":"caps_lock"}"#;
        let request: Request = serde_json::from_str(raw).unwrap();
        assert_eq!(request, Request::CapsLock { on: true });

        let raw = r#"{"type":"context","context_enabled":true,"op":"context"}"#;
        let request: Request = serde_json::from_str(raw).unwrap();
        assert_eq!(
            request,
            Request::Context {
                on: false,
                context_id: 0,
                context_enabled: true,
            }
        );
    }

    #[test]
    fn vocab_keeps_raw_directive_from_t1a() {
        let raw = r#"{"op":"vocab","vocab":{"op":"status"}}"#;
        let request: Request = serde_json::from_str(raw).unwrap();
        let Request::Vocab { directive, vocab } = request else {
            panic!("應是 Vocab");
        };
        assert_eq!(directive, "");
        assert_eq!(vocab, Some(serde_json::json!({"op": "status"})));
    }

    #[test]
    fn unknown_op_still_captured_after_extension() {
        let request: Request = serde_json::from_str(r#"{"op":"teleport","x":1}"#).unwrap();
        assert_eq!(request, Request::Unknown);
        assert_eq!(
            serde_json::to_string(&Request::Unknown).unwrap(),
            r#"{"op":"unknown"}"#
        );
    }

    #[test]
    fn old_request_shapes_are_byte_identical() {
        // 平行建置時舊 DLL 送的訊框必須逐字不變。
        assert_eq!(
            serde_json::to_string(&Request::Snapshot).unwrap(),
            r#"{"op":"snapshot"}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::Toggle).unwrap(),
            r#"{"op":"toggle"}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::Commit).unwrap(),
            r#"{"op":"commit"}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::Shutdown).unwrap(),
            r#"{"op":"shutdown"}"#
        );
        let hello = Request::Hello {
            protocol: PROTOCOL_VERSION,
            client: "old".into(),
        };
        assert_eq!(
            serde_json::to_string(&hello).unwrap(),
            r#"{"op":"hello","protocol":3,"client":"old"}"#
        );
    }

    #[test]
    fn health_info_feeds_t1a_output_simplified() {
        let info = HealthInfo {
            draining: true,
            connections: 2,
            idle_seconds: Some(9),
            sessions: 3,
            composing: 1,
            english_words: 80_000,
            pid: Some(42),
            output_simplified: true,
            personal_reset: true,
            start_mode: Some("english".to_string()),
            capabilities: BTreeMap::from([("caps_lock_english".into(), true)]),
            ..HealthInfo::default()
        };
        let info_map = info.as_info();
        // T1a 的 learn_simplified() 讀的就是這個 key 與值。
        assert_eq!(
            info_map.get("output_simplified").map(String::as_str),
            Some("true")
        );
        assert_eq!(info_map.get("draining").map(String::as_str), Some("true"));
        assert_eq!(info_map.get("connections").map(String::as_str), Some("2"));
        assert_eq!(info_map.get("idle_seconds").map(String::as_str), Some("9"));
        assert_eq!(info_map.get("sessions").map(String::as_str), Some("3"));
        assert_eq!(info_map.get("composing").map(String::as_str), Some("1"));
        assert_eq!(
            info_map.get("english_words").map(String::as_str),
            Some("80000")
        );
        assert_eq!(info_map.get("pid").map(String::as_str), Some("42"));
        assert_eq!(
            info_map.get("caps_lock_english").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            info_map.get("settings_deferred_apply").map(String::as_str),
            Some("true")
        );

        let empty = HealthInfo::default().as_info();
        assert!(!empty.contains_key("output_simplified"));
        assert_eq!(
            empty.get("draining").map(String::as_str),
            Some("false"),
            "0c 的 health 一律回 draining key"
        );
    }

    #[test]
    fn health_info_reports_the_last_slow_request() {
        let info = HealthInfo {
            last_slow_op: Some("key".into()),
            last_slow_ms: Some(412),
            last_slow_input_len: Some(37),
            ..HealthInfo::default()
        };
        let text = serde_json::to_string(&info).unwrap();
        assert!(text.contains(r#""last_slow_op":"key""#), "{text}");
        assert!(text.contains(r#""last_slow_ms":412"#), "{text}");
        assert!(text.contains(r#""last_slow_input_len":37"#), "{text}");
        let parsed: HealthInfo = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, info);

        let map = info.as_info();
        assert_eq!(map.get("last_slow_op").map(String::as_str), Some("key"));
        assert_eq!(map.get("last_slow_ms").map(String::as_str), Some("412"));
        assert_eq!(
            map.get("last_slow_input_len").map(String::as_str),
            Some("37")
        );

        // 沒有慢請求時三欄都不出現，舊外殼看到的形狀不變。
        let empty = HealthInfo::default();
        let text = serde_json::to_string(&empty).unwrap();
        assert!(!text.contains("last_slow"), "{text}");
        let map = empty.as_info();
        assert!(!map.contains_key("last_slow_op"));
        assert!(!map.contains_key("last_slow_ms"));
        assert!(!map.contains_key("last_slow_input_len"));
    }

    #[test]
    fn health_response_round_trips_with_concrete_wire_shape() {
        let info = HealthInfo {
            draining: true,
            connections: 2,
            sessions: 3,
            output_simplified: true,
            ..HealthInfo::default()
        };
        let response = Response::Health { info: info.clone() };
        let text = serde_json::to_string(&response).unwrap();
        assert_eq!(
            text,
            r#"{"op":"health","info":{"draining":true,"connections":2,"sessions":3,"output_simplified":true}}"#
        );
        assert_eq!(serde_json::from_str::<Response>(&text).unwrap(), response);

        let minimal: Response = serde_json::from_str(r#"{"op":"health"}"#).unwrap();
        assert_eq!(
            minimal,
            Response::Health {
                info: HealthInfo::default()
            }
        );
    }

    #[test]
    fn ready_carries_the_engine_session_id() {
        let ready = Response::Ready {
            protocol: PROTOCOL_VERSION,
            engine: "jc-engine 0.1.0".into(),
            session: 7,
        };
        let text = serde_json::to_string(&ready).unwrap();
        assert_eq!(
            text,
            r#"{"op":"ready","protocol":3,"engine":"jc-engine 0.1.0","session":7}"#
        );
        assert_eq!(serde_json::from_str::<Response>(&text).unwrap(), ready);
    }

    #[test]
    fn ready_defaults_to_session_one_for_old_engine_shape() {
        let old = r#"{"op":"ready","protocol":3,"engine":"jc-engine/0.1.0"}"#;
        assert_eq!(
            serde_json::from_str::<Response>(old).unwrap(),
            Response::Ready {
                protocol: PROTOCOL_VERSION,
                engine: "jc-engine/0.1.0".into(),
                session: 1,
            }
        );
    }

    #[test]
    fn acknowledged_and_vocab_responses_round_trip() {
        let ack = Response::Acknowledged { session: 9 };
        let text = serde_json::to_string(&ack).unwrap();
        assert_eq!(text, r#"{"op":"acknowledged","session":9}"#);
        assert_eq!(serde_json::from_str::<Response>(&text).unwrap(), ack);

        let ack_zero: Response = serde_json::from_str(r#"{"op":"acknowledged"}"#).unwrap();
        assert_eq!(ack_zero, Response::Acknowledged { session: 0 });

        let vocab = Response::Vocab {
            vocab: VocabInfo {
                ok: true,
                changed: true,
                error: None,
                raw: None,
            },
        };
        let text = serde_json::to_string(&vocab).unwrap();
        assert_eq!(text, r#"{"op":"vocab","vocab":{"ok":true,"changed":true}}"#);
        assert_eq!(serde_json::from_str::<Response>(&text).unwrap(), vocab);
    }

    #[test]
    fn unsupported_is_distinct_from_unknown_op() {
        let unsupported = Response::error(ErrorCode::Unsupported, "jc 尚未支援 context");
        let text = serde_json::to_string(&unsupported).unwrap();
        assert_eq!(
            text,
            r#"{"op":"error","code":"unsupported","message":"jc 尚未支援 context"}"#
        );
        let unknown = Response::error(ErrorCode::UnknownOp, "未知的操作");
        assert_ne!(unsupported, unknown);
        assert!(text.contains("unsupported"));
    }
}
