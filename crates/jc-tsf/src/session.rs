//! 0c `platforms/windows/tsf/src/session.rs`（main 8f443af）的移植。
//!
//! 狀態機、generation、log 重播、`restore_requests`、3 次重試加 20ms 等待
//! 與快照結構照抄。Adapter 只改兩處：`open_request` 改送 jc `hello`，
//! `parse()` 前先將 jc `Response::State` 轉成 0c snapshot 的 JSON 形狀。
//!
//! T0 結果（`docs/port/0c-api-diff.md`）已套用。三類全域差異在本檔的落點：
//!
//! 1. `windows_link::link!` → `windows_core::link!`：session.rs 沒有 link! 宣告；
//!    0c 唯一一處在 msctf.rs（T6），本 crate 不直接依賴 windows-link。
//! 2. `Error::from_win32()` → `Error::from_thread()`：session.rs 沒有 Error 呼叫；
//!    0c 的兩處在 register.rs 與 langbar.rs（T6）。`HRESULT::from_win32(u32)` 沒改名。
//! 3. `Ref<'_, T>` / `OutRef<'_, T>` → `Ref<T>` / `OutRef<T>`：session.rs 沒有 _Impl trait；
//!    0c 的 68 處都不在 session.rs。
//!
//! 0c session.rs 測試原本用 CreateNamedPipeW 等 Win32 API 建立假引擎，移植時改成
//! 記憶體 `Transport`，所以本檔不直接呼叫 Win32。真正的 overlapped I/O 留給 T2。

use std::time::{Duration, Instant};

use jc_proto::{Response, PROTOCOL_VERSION as JC_PROTOCOL_VERSION};
use serde_json::{json, Value};

/// debug-keypath：0c 的 `zlog` 改接 crate 層，讓按鍵路徑的 log 寫進 tsf.log。
use crate::log::zlog;

/// jc 協定版本（0c 的 open 原本帶 0，這裡改帶 hello 的 3）。
pub const PROTOCOL_VERSION: u32 = JC_PROTOCOL_VERSION;

/// hello 的客戶端識別。
const CLIENT_NAME: &str = "jc-tsf";

/// 引擎重啟時等待新引擎的期限（0c `ipc::RESTART_WAIT`）。
pub const RESTART_WAIT: Duration = Duration::from_secs(5);

/// 從 0c `keys.rs` 搬過來的最小集合；session 移植只需要這幾個判斷。
pub mod keys {
    use serde_json::Value;

    /// 引擎載入的鍵盤排列（health 的 `keyboard_layout`）。
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Layout {
        Standard,
        ETen,
    }

    /// 空組字時可開始組字的字元。
    pub fn starts_composition(layout: Layout, key: &str) -> bool {
        let keys = match layout {
            Layout::Standard => "`abcdefghijklmnopqrstuvwxyz125890,.;/-<>?:\"!~[]{}'\\()&^_",
            Layout::ETen => "`abcdefghijklmnopqrstuvwxyz7890,.;/-='\\<>?:\"!~[]{}()&^_",
        };
        key.len() == 1 && keys.contains(key)
    }

    /// 聲調鍵；標準排列 3 4 6 7，倚天 1 2 3 4。
    pub fn tone_key(layout: Layout, key: &str) -> bool {
        let keys = match layout {
            Layout::Standard => "3467",
            Layout::ETen => "1234",
        };
        key.len() == 1 && keys.contains(key)
    }

    /// #123 的新鍵名換回原本的鍵。
    pub fn vocab_base(key: &str) -> Option<&'static str> {
        Some(match key {
            "ShiftLeft" => "Left",
            "ShiftRight" => "Right",
            "ShiftDelete" => "Delete",
            "ShiftBackspace" => "Backspace",
            _ => return None,
        })
    }

    /// 引擎是否認得 Ctrl＋標點鍵名。
    pub fn ctrl_punct_known(key: &str, engine_keys: &str) -> bool {
        match key.strip_prefix("Ctrl+") {
            Some(c) if c.chars().count() == 1 => ",.';?!".contains(c) || engine_keys.contains(c),
            _ => false,
        }
    }

    /// 哪一個 Shift。
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Side {
        Left,
        Right,
    }

    /// 引擎載入的中英切換鍵設定（health 的 `left_shift`、`right_shift`、`shift_tap_ms`）。
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct ShiftConfig {
        pub left: bool,
        pub right: bool,
        pub tap_ms: u64,
    }

    impl Default for ShiftConfig {
        fn default() -> Self {
            Self {
                left: false,
                right: true,
                tap_ms: 300,
            }
        }
    }

    impl ShiftConfig {
        /// 從 health 的 info 取出；缺欄位照舊引擎處理。
        pub fn from_info(info: Option<&Value>) -> ShiftConfig {
            let mut config = ShiftConfig::default();
            let Some(info) = info else { return config };
            let flag = |key: &str| info[key].as_str().map(|v| v == "true");
            config.left = flag("left_shift").unwrap_or(false);
            config.right = flag("right_shift").unwrap_or(true);
            if let Some(ms) = info["shift_tap_ms"]
                .as_str()
                .and_then(|v| v.parse::<u64>().ok())
            {
                config.tap_ms = ms;
            }
            config
        }

        /// 這個 Shift 是否切換中英。
        pub fn toggles(&self, side: Side) -> bool {
            match side {
                Side::Left => self.left,
                Side::Right => self.right,
            }
        }
    }
}

/// 一個組字片段。jc 的 `SegmentInfo` 只有位移與 mode，沒有樣式文字。
#[derive(Clone, Debug, Default)]
pub struct Segment {
    pub text: String,
    pub style: String,
}

/// 0c 快照。註解標出每一欄的 jc 來源；「jc 尚無對應」表示 jc wire 格式沒有
/// 這一項，只能給預設值或由別的欄位推導。
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// 由 jc `Response` 是否為 State 決定。
    pub ok: bool,
    /// jc 尚無對應；由 preedit／candidates／suggestion 推導。
    pub handled: bool,
    /// jc `Response::Committed` 的文字；State 沒有這一欄，空字串。
    pub commit: String,
    /// jc `State::preedit` 依 `segments` 切段；沒有 segments 時整段一段。
    pub preedit: Vec<Segment>,
    /// jc `State::cursor_rune`（權威值）。
    pub cursor_rune: usize,
    /// jc core 有這個欄位但 wire 格式沒送；由 `cursor_rune` 換算。
    pub cursor_utf16: usize,
    /// jc `State::suggestion`，Tab 會接受的補全尾綴。
    pub suggestion: String,
    /// jc `State::candidates`，ID 用順序代替。
    pub candidates: Vec<(i64, String)>,
    /// jc 尚無對應的直接欄位；由 `segments` 起點推導，沒有時 0。
    pub candidate_start: usize,
    /// jc 尚無對應的直接欄位；由 `segments` 範圍推導，沒有時 0。
    pub candidate_length: usize,
    /// jc `details[i].reading` 權充附註；jc 沒有 0c 的模糊音提示。
    pub candidate_notes: Vec<String>,
    /// jc 尚無對應；先給 `["1".."9"]`（第 10 項 0，英文模式空字串）。
    pub candidate_keys: Vec<String>,
    /// jc 尚無對應；0。
    pub columns: usize,
    /// jc 尚無對應；空字串。
    pub candidate_kind: String,
    /// jc 尚無對應；空字串。
    pub candidate_title: String,
    /// jc 尚無對應；0。
    pub page: i64,
    /// jc 尚無對應；0。
    pub pages: i64,
    /// jc `State::highlighted`。
    pub highlight: i64,
    /// jc wire 沒有整體 mode；由 `segments[].mode` 推導，沒有時 false。
    pub english: bool,
    /// jc 尚無對應；false。
    pub full_width: bool,
    /// jc 尚無對應的獨立旗標；由 `suggestion` 是否存在推導。
    pub english_autocomplete: bool,
    /// 直接影響 su 問題：必須由 jc `suggestion` 映射，不能給預設值。
    pub english_pending: bool,
    /// jc 尚無對應；false。
    pub english_can_start: bool,
    /// jc 尚無對應；false。
    pub caps_lock: bool,
    /// jc 尚無對應；false。
    pub caps_english: bool,
    /// 由 session 的 generation 變化計算，不來自 jc。
    pub reconnected: bool,
    /// jc 尚無對應；0。
    pub context_id: u64,
    /// jc 尚無對應；空陣列。
    pub notices: Vec<String>,
    /// jc 尚無對應；空字串。
    pub reading_hint: String,
    /// jc 尚無對應；false。
    pub output_simplified: bool,
    /// jc 尚無對應；false。
    pub bell: bool,
    /// jc wire 沒有整體 mode；由 `segments[].mode == "japanese"` 推導。
    pub japanese: bool,
    /// jc 尚無對應；空字串。
    pub language: String,
    /// jc 尚無對應；空字串。
    pub input_method: String,
    /// jc 尚無對應；false。
    pub converting: bool,
    /// jc 尚無對應；0。
    pub clause_index: i64,
    /// jc 尚無對應；0。
    pub clause_count: i64,
}

impl Snapshot {
    /// 要顯示的提示：多則時以「；」串成一行。
    pub fn notice(&self) -> String {
        self.notices.join("；")
    }

    /// 組字區的文字。
    pub fn text(&self) -> String {
        self.preedit.iter().map(|s| s.text.as_str()).collect()
    }

    /// 字寬是每個 session 的狀態；這些字元在半形可直接交給宿主。
    pub fn full_width_extra_key(&self, key: &str) -> bool {
        self.full_width
            && key.len() == 1
            && (key.as_bytes()[0].is_ascii_uppercase() || " \\'\"<>?".contains(key))
    }

    /// 全形時沒在組字的數字鍵盤一律打成全形字元。
    pub fn full_width_keypad(&self, key: &str) -> bool {
        self.full_width
            && key
                .strip_prefix("KP")
                .is_some_and(|c| c.len() == 1 && c.as_bytes()[0].is_ascii_graphic())
    }

    /// Predict only keys the English word buffer will actually consume.
    pub fn english_wants_key(&self, key: &str) -> bool {
        let literal = key.strip_prefix("KP").unwrap_or(key);
        if self.english
            && self.full_width
            && (literal.len() == 1 && literal.as_bytes()[0].is_ascii_graphic() || key == " ")
        {
            return true;
        }
        if !self.english_autocomplete || !self.english {
            return false;
        }
        let letter = key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic();
        if self.english_pending {
            letter && self.text().len() < 48
                || matches!(key, "Backspace" | "Escape" | "Enter")
                || (!self.candidates.is_empty() && matches!(key, "Tab" | "Up" | "Down"))
        } else {
            self.english_can_start && letter
        }
    }

    /// 是否正在組字。
    pub fn composing(&self) -> bool {
        !self.preedit.is_empty() || !self.candidates.is_empty()
    }

    /// 組字區是否仍在標示要加入詞庫的範圍。
    pub fn marking(&self) -> bool {
        self.preedit.iter().any(|s| s.style == "marked")
    }

    /// 日文變換中目前文節的說明。
    pub fn clause_title(&self) -> String {
        if self.clause_count <= 0 {
            return String::new();
        }
        let index = (self.clause_index.max(0) + 1).min(self.clause_count);
        format!("文節 {index}/{}", self.clause_count)
    }

    /// 候選窗的標題。
    pub fn candidate_title(&self) -> String {
        let clause = self.clause_title();
        match (self.candidate_title.is_empty(), clause.is_empty()) {
            (true, _) => clause,
            (false, true) => self.candidate_title.clone(),
            (false, false) => format!("{}　{clause}", self.candidate_title),
        }
    }
}

/// 取 rune 範圍的字串，越界時安靜地夾住。
fn slice_runes(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

/// 把 rune 位置換成 UTF-16 位置（jc wire 只送 cursor_rune）。
fn utf16_at_rune(text: &str, rune: usize) -> usize {
    text.chars().take(rune).map(|c| c.len_utf16()).sum()
}

/// jc `Response::State` → 0c snapshot 的 JSON 形狀。
///
/// 0c 的 `parse()` 完全不懂 jc 型別；這裡先把 jc 的欄位攤成 0c 的字串鍵，
/// 缺的欄位給預設值並在 [`Snapshot`] 的註解標「jc 尚無對應」。
pub fn state_to_snapshot_value(response: &Response) -> Value {
    let Response::State {
        preedit,
        candidates,
        details,
        segments,
        highlighted,
        cursor_rune,
        suggestion,
        ..
    } = response
    else {
        return Value::Null;
    };

    let mut preedit_segments: Vec<Value> = Vec::new();
    if segments.is_empty() {
        if !preedit.is_empty() {
            preedit_segments.push(json!({"text": preedit, "style": "normal"}));
        }
    } else {
        for segment in segments {
            preedit_segments.push(json!({
                "text": slice_runes(preedit, segment.start, segment.end),
                "style": if segment.mode == "japanese" { "clause" } else { "normal" },
            }));
        }
    }

    let (candidate_start, candidate_length) = if segments.is_empty() {
        (0, 0)
    } else {
        let start = segments[0].start;
        let end = segments.last().map(|s| s.end).unwrap_or(start);
        (start, end.saturating_sub(start))
    };

    let all_english = !segments.is_empty() && segments.iter().all(|s| s.mode == "english");
    let japanese = segments.iter().any(|s| s.mode == "japanese");
    let pending = suggestion.is_some();

    let items: Vec<Value> = candidates
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let note = details
                .get(i)
                .map(|d| d.reading.clone())
                .unwrap_or_default();
            let key = if all_english {
                String::new()
            } else {
                ((i + 1) % 10).to_string()
            };
            json!({"id": i as i64, "text": text, "note": note, "key": key})
        })
        .collect();

    json!({
        "handled": !preedit.is_empty() || !candidates.is_empty() || pending,
        "preedit": preedit_segments,
        "cursor": {"rune": cursor_rune, "utf16": utf16_at_rune(preedit, *cursor_rune)},
        "suggestion": suggestion.clone().unwrap_or_default(),
        "candidates": {
            "start": candidate_start,
            "length": candidate_length,
            "highlight": highlighted,
            "items": items,
        },
        "mode": if japanese { "japanese" } else if all_english { "english" } else { "zhuyin" },
        "english_autocomplete": pending,
        "english_pending": pending,
    })
}

/// jc `Response` → 0c reply Value。轉換在 `parse()` 之前完成。
pub fn response_to_reply(response: &Response) -> Value {
    match response {
        // 引擎配置的 id 原樣帶到 0c reply；`Session::ensure_open()` 會記住它，
        // 之後 reset／Drop 的 Close 就送這個 id，而不是寫死 1。舊引擎的 Ready
        // 沒有 session 欄位時，jc-proto 反序列化會補 1，行為與從前一致。
        Response::Ready {
            engine, session, ..
        } => json!({
            "ok": true, "session": session, "engine": engine, "info": {},
        }),
        Response::State { .. } => json!({
            "ok": true, "snapshot": state_to_snapshot_value(response),
        }),
        Response::Committed { text } => json!({
            "ok": true, "snapshot": {"handled": true, "commit": text},
        }),
        Response::Error { message, .. } => json!({"ok": false, "error": message}),
        // T3 擴充的 op：health 的 info 攤成 0c 的字串 map，learn_simplified()
        // 就能讀到 output_simplified；其餘回報照 0c Reply 的形狀組。
        Response::Health { info } => json!({
            "ok": true, "info": info.as_info(),
        }),
        Response::Acknowledged { session } => json!({
            "ok": true, "session": session,
        }),
        Response::Vocab { vocab } => json!({
            "ok": true, "vocab": vocab,
        }),
    }
}

/// 0c envelope → jc 請求。
///
/// `open` 是唯一在 [`open_request`] 已经改成 hello 的地方；其餘 op 依
/// planning 文件對應到 key／toggle／commit。`health`、`close` 與設定類
/// 請求在 T3 的 jc-proto op 扩充前由 Transport 直接處理。
pub fn to_jc_request(envelope: &Value) -> Value {
    match envelope["op"].as_str().unwrap_or("") {
        "handle" => match envelope["req"]["type"].as_str().unwrap_or("") {
            "key" => json!({"op": "key", "key": envelope["req"]["key"]}),
            "toggle" => json!({"op": "toggle"}),
            "commit" => json!({"op": "commit"}),
            other => {
                let mut request = envelope["req"].clone();
                request["op"] = Value::from(other);
                request
            }
        },
        _ => envelope.clone(),
    }
}

/// IPC transport。T2 在這裡接 windows-rs 0.62 的 overlapped I/O。
pub trait Transport {
    /// 送出 jc 請求，回傳 jc `Response` 的 JSON；擴充 op 可回 0c 形狀的 reply。
    fn request(&mut self, request: &Value) -> Option<Value>;
    /// 目前是否持有連線。
    fn connected(&self) -> bool;
    /// 建立連線；成功後 [`Client`] 會遞增 generation。
    fn connect(&mut self) -> bool;
    /// 關閉連線。
    fn close(&mut self);
    /// 不送請求，只看對端是否還活著。
    fn alive(&mut self) -> bool;
    /// 測試用：模擬對端關掉連線。
    fn disconnect_for_test(&mut self) {
        self.close();
    }
}

/// 沒有引擎時的 transport：所有請求都失敗。
#[derive(Default)]
pub struct NullTransport;

impl Transport for NullTransport {
    fn request(&mut self, _request: &Value) -> Option<Value> {
        None
    }
    fn connected(&self) -> bool {
        false
    }
    fn connect(&mut self) -> bool {
        false
    }
    fn close(&mut self) {}
    fn alive(&mut self) -> bool {
        false
    }
}

/// 0c `Client` 的連線、generation 與重啟期限語意，IPC 部分交給 [`Transport`]。
pub struct Client {
    transport: Box<dyn Transport>,
    generation: u32,
    restart_until: Option<Instant>,
    closing_writer: bool,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    /// 沒有引擎的 client；測試用 [`Client::with_transport`] 換掉 transport。
    pub fn new() -> Client {
        Self::with_transport(Box::new(NullTransport))
    }

    /// 換入 transport。
    pub fn with_transport(transport: Box<dyn Transport>) -> Client {
        Client {
            transport,
            generation: 0,
            restart_until: None,
            closing_writer: false,
        }
    }

    /// 測試：模擬對端關掉連線。
    #[cfg(test)]
    pub(crate) fn disconnect_for_test(&mut self) {
        self.transport.disconnect_for_test();
    }

    /// 測試：縮短重啟期限，避免測試真的等 5 秒。
    #[cfg(test)]
    pub(crate) fn set_restart_until_for_test(&mut self, until: Instant) {
        self.restart_until = Some(until);
    }

    /// 每次重新連線後遞增；session 屬於某一代連線。
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// 引擎正在重啟，還在等新引擎的期限內。
    pub fn restart_pending(&self) -> bool {
        self.restart_until
            .is_some_and(|until| std::time::Instant::now() < until)
    }

    /// 等新引擎的期限。
    pub fn restart_deadline(&self) -> Option<Instant> {
        self.restart_until
    }

    /// 呼叫端不再等。
    pub fn abandon_restart(&mut self) {
        self.restart_until = None;
        self.closing_writer = false;
    }

    /// 從現在起 RESTART_WAIT 內等它，已經在等就不延長。
    fn expect_restart(&mut self) {
        if self.restart_until.is_none() {
            self.restart_until = Some(std::time::Instant::now() + RESTART_WAIT);
        }
    }

    /// I/O 失敗後：對端關掉才當成重啟。
    fn close_after_failure(&mut self) {
        if !self.transport.alive() {
            self.expect_restart();
        }
        self.close();
    }

    /// 目前是否持有連線。
    pub fn connected(&self) -> bool {
        self.transport.connected()
    }

    /// 連線是否還在：不送請求，只看對端。
    pub fn alive(&mut self) -> bool {
        if !self.transport.alive() {
            self.expect_restart();
            self.close();
            return false;
        }
        true
    }

    /// 只連已經在執行的引擎（spike 沒有 spawn）。
    pub fn connect_existing(&mut self) -> bool {
        if self.closing_writer && self.restart_pending() {
            return false;
        }
        self.connect()
    }

    /// 確保已連上。
    pub fn connect(&mut self) -> bool {
        if self.transport.connected() {
            return true;
        }
        if self.transport.connect() {
            self.generation = self.generation.wrapping_add(1);
            true
        } else {
            false
        }
    }

    /// 放掉連線；下一次呼叫照常重連。
    pub fn release(&mut self) {
        self.close();
    }

    fn close(&mut self) {
        self.transport.close();
    }

    /// 送出請求並讀回 0c 形狀的 reply；翻譯與轉換都在 `parse()` 之前。
    pub fn call(&mut self, envelope: &Value) -> Option<Value> {
        if !self.connect() {
            // 對端已經關掉連線（引擎退出／重啟）時，連不上的這一次就要記下重啟期限；
            // 和 alive() 與 close_after_failure() 的判斷一致，讓 Preflight 立刻知道要等。
            if !self.transport.alive() {
                self.expect_restart();
            }
            return None;
        }
        let request = to_jc_request(envelope);
        let Some(raw) = self.transport.request(&request) else {
            self.close_after_failure();
            return None;
        };
        let reply = match serde_json::from_value::<Response>(raw.clone()) {
            Ok(response) => response_to_reply(&response),
            Err(_) => raw, // T3 之前的 health／close 等擴充 op
        };
        if reply["ok"].as_bool() == Some(false) && reply["error"].as_str() == Some("引擎正在切換")
        {
            self.expect_restart();
            self.closing_writer = true;
            self.close();
        } else if reply["ok"].as_bool() == Some(true) {
            self.abandon_restart();
        }
        Some(reply)
    }
}

/// 重開 session（引擎重啟、斷線）時要不要送「設定模式」還原外殼記得的模式。
fn restore_mode(known: bool, want_english: bool, start_english: bool) -> Option<bool> {
    (known && want_english != start_english).then_some(want_english)
}

/// 重開 session 時依序送出的還原請求。
fn restore_requests(
    known: bool,
    want_english: bool,
    caps_english: bool,
    caps_lock_english: bool,
    start_english: bool,
) -> Vec<Value> {
    let caps = want_english && caps_english;
    let mut out: Vec<Value> = restore_mode(known, want_english && !caps, start_english)
        .map(|english| json!({"type": "mode", "on": english}))
        .into_iter()
        .collect();
    if caps && caps_lock_english {
        out.push(json!({"type": "caps_lock", "on": true}));
    }
    out
}

/// health 回報的新 session 開始時的模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Chinese,
    English,
    Japanese,
}

fn initial_mode(info: &Value) -> Option<Mode> {
    match info["initial_mode"].as_str()? {
        "english" => Some(Mode::English),
        "japanese" => Some(Mode::Japanese),
        "zhuyin" => Some(Mode::Chinese),
        _ => None,
    }
}

#[allow(dead_code)]
fn initial_english(info: &Value) -> Option<bool> {
    initial_mode(info).map(|mode| mode == Mode::English)
}

/// 開 session 的請求。Adapter 改點一：0c `open` 改成 jc `hello`。
fn open_request() -> Value {
    json!({"op": "hello", "protocol": PROTOCOL_VERSION, "client": CLIENT_NAME})
}

/// 沒有在組字時的聲調鍵要不要交給引擎。
fn idle_tone_keys(info: &Value) -> bool {
    info["tone_mark_keys"] == "true" || info["mixed_input"] == "true"
}

/// 沒在組字時這個鍵要不要交給引擎。
fn idle_key_from(info: &Value, key: &str, full_width: bool) -> bool {
    if let Some(keys) = info["idle_keys_full_width"].as_str().filter(|_| full_width) {
        return key.chars().count() == 1 && keys.contains(key);
    }
    if let Some(keys) = info["idle_keys"].as_str() {
        return key.chars().count() == 1 && keys.contains(key);
    }
    let layout = if info["keyboard_layout"] == "eten" {
        keys::Layout::ETen
    } else {
        keys::Layout::Standard
    };
    if key == "=" && layout == keys::Layout::Standard {
        info["emoji_shortcut"] == "equals"
    } else if keys::tone_key(layout, key) {
        idle_tone_keys(info)
    } else {
        keys::starts_composition(layout, key)
    }
}

/// 把 #123 的新鍵名換回原本的鍵。
fn downgrade_vocab_key(request: &mut Value) {
    if request["type"] != "key" {
        return;
    }
    if let Some(base) = request["key"].as_str().and_then(keys::vocab_base) {
        request["key"] = Value::from(base);
    }
}

/// 引擎沒有回報 clause_display 時，文節樣式退回一般組字。
fn apply_clause_capability(s: &mut Snapshot, clause_display: bool) {
    if clause_display {
        return;
    }
    for segment in &mut s.preedit {
        if segment.style == "clause" || segment.style == "clause_active" {
            segment.style = "normal".to_string();
        }
    }
    s.clause_index = 0;
    s.clause_count = 0;
}

/// 0c 的 `parse()`：吃 0c reply 的 `snapshot` 物件，原樣移植。
fn parse(reply: &Value) -> Snapshot {
    let mut s = Snapshot::default();
    let Some(j) = reply.get("snapshot") else {
        return s;
    };
    s.ok = true;
    s.handled = j["handled"].as_bool().unwrap_or(false);
    s.commit = j["commit"].as_str().unwrap_or("").to_string();
    s.context_id = j["context_id"].as_u64().unwrap_or(0);
    s.suggestion = j["suggestion"].as_str().unwrap_or("").to_string();
    s.english = j["mode"].as_str() == Some("english");
    s.japanese = j["mode"].as_str() == Some("japanese");
    s.language = j["language"].as_str().unwrap_or("").to_string();
    s.input_method = j["input_method"].as_str().unwrap_or("").to_string();
    s.converting = j["converting"].as_bool().unwrap_or(false);
    s.clause_index = j["clause_index"].as_i64().unwrap_or(0);
    s.clause_count = j["clause_count"].as_i64().unwrap_or(0);
    s.full_width = j["full_width"].as_bool().unwrap_or(false);
    s.english_autocomplete = j["english_autocomplete"].as_bool().unwrap_or(false);
    s.english_pending = j["english_pending"].as_bool().unwrap_or(false);
    s.english_can_start = j["english_can_start"].as_bool().unwrap_or(false);
    s.caps_lock = j["caps_lock"].as_bool().unwrap_or(false);
    s.caps_english = j["caps_english"].as_bool().unwrap_or(false);
    s.reading_hint = j["reading_hint"]["text"].as_str().unwrap_or("").to_string();
    s.output_simplified = j["output_simplified"].as_bool().unwrap_or(false);
    s.bell = j["bell"].as_bool().unwrap_or(false);
    if let Some(notices) = j["notices"].as_array() {
        s.notices = notices
            .iter()
            .filter_map(|n| n.as_str())
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .collect();
    }
    if let Some(p) = j["preedit"].as_array() {
        for g in p {
            s.preedit.push(Segment {
                text: g["text"].as_str().unwrap_or("").to_string(),
                style: g["style"].as_str().unwrap_or("normal").to_string(),
            });
        }
    }
    s.cursor_rune = j["cursor"]["rune"].as_u64().unwrap_or(0) as usize;
    s.cursor_utf16 = j["cursor"]["utf16"].as_u64().unwrap_or(0) as usize;
    if let Some(c) = j.get("candidates") {
        s.columns = c["columns"].as_u64().unwrap_or(0).min(10) as usize;
        s.candidate_kind = c["kind"].as_str().unwrap_or("").to_string();
        s.candidate_title = c["title"].as_str().unwrap_or("").to_string();
        s.candidate_start = c["start"].as_u64().unwrap_or(0) as usize;
        s.candidate_length = c["length"].as_u64().unwrap_or(0) as usize;
        s.page = c["page"].as_i64().unwrap_or(0);
        s.pages = c["pages"].as_i64().unwrap_or(0);
        s.highlight = c["highlight"].as_i64().unwrap_or(0);
        if let Some(items) = c["items"].as_array() {
            for it in items {
                s.candidates.push((
                    it["id"].as_i64().unwrap_or(0),
                    it["text"].as_str().unwrap_or("").to_string(),
                ));
                s.candidate_notes
                    .push(it["note"].as_str().unwrap_or("").to_string());
                let index = s.candidate_keys.len();
                s.candidate_keys.push(match it["key"].as_str() {
                    Some(key) => key.to_string(),
                    None if s.english => String::new(),
                    None => ((index + 1) % 10).to_string(),
                });
            }
        }
    }
    s
}

/// 一個輸入欄位在 server 上的 session：請求、重連後的重播、快照的解讀。
pub struct Session {
    client: Client,
    /// 測試用：`handle` 送出的每個請求的 type，依序記下。
    #[cfg(test)]
    pub(crate) sent: Vec<String>,
    id: u64,
    generation: u32,
    english: bool,
    /// 不含 Caps Lock 暫時英文的程式模式；關掉 Caps Lock 要回到這裡。
    /// jc 的 snapshot 沒有 caps_english，外殼必須自己記住這層。
    base_english: bool,
    replay_english: bool,
    pub english_state: Snapshot,
    down: bool,
    log: Vec<Value>,
    cleared: Option<(Vec<Value>, bool)>,
    toggled: bool,
    context_enabled: bool,
    pending_context: Option<(u64, String)>,
    private: bool,
    vocab_keys: bool,
    mode_known: bool,
    shared: bool,
    caps_lock_english: bool,
    caps_generation: u32,
    caps_english: bool,
    japanese: bool,
    japanese_cap: bool,
    convert_keys_cap: bool,
    clause_display_cap: bool,
    live_conversion_cap: bool,
    simplified_live: bool,
    simplified: bool,
    simplified_at: Option<Instant>,
    idle_info: Option<Value>,
    idle_generation: u32,
}

impl Session {
    /// 建立沒有引擎的 session。
    pub fn new() -> Session {
        Session::with_client(Client::new())
    }

    /// 測試：換掉用戶端（模擬引擎斷線）。
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn set_client_for_test(&mut self, client: Client) {
        self.client = client;
    }

    /// 用指定的 client 建立 session。
    pub fn with_client(client: Client) -> Session {
        Session {
            client,
            #[cfg(test)]
            sent: Vec::new(),
            id: 0,
            generation: 0,
            english: false,
            base_english: false,
            replay_english: false,
            english_state: Snapshot::default(),
            down: false,
            log: Vec::new(),
            cleared: None,
            toggled: false,
            context_enabled: std::env::var("ZUIN_DOCUMENT_CONTEXT").as_deref() != Ok("0"),
            pending_context: None,
            private: false,
            vocab_keys: false,
            mode_known: false,
            shared: false,
            caps_lock_english: false,
            caps_generation: 0,
            caps_english: false,
            japanese: false,
            japanese_cap: false,
            convert_keys_cap: false,
            clause_display_cap: false,
            live_conversion_cap: false,
            simplified_live: false,
            simplified: false,
            simplified_at: None,
            idle_info: None,
            idle_generation: 0,
        }
    }

    /// 從 health／open 的 info 記下「輸出簡體字」的能力與目前的值。
    fn learn_simplified(&mut self, info: &Value) -> bool {
        let before = (self.simplified_live, self.simplified);
        self.simplified_live = info["output_simplified_live"] == "true";
        self.simplified = self.simplified_live && info["output_simplified"] == "true";
        self.simplified_at = Some(Instant::now());
        before != (self.simplified_live, self.simplified)
    }

    /// 從 health／open 的 info 記下日文能力旗標。
    fn learn_capabilities(&mut self, info: &Value, generation: u32) {
        self.caps_lock_english = info["caps_lock_english"] == "true";
        self.japanese_cap = info["japanese"] == "true";
        self.convert_keys_cap = info["convert_keys"] == "true";
        self.clause_display_cap = info["clause_display"] == "true";
        self.live_conversion_cap = info["live_conversion"] == "true";
        self.caps_generation = generation;
        if !self.japanese_cap {
            self.japanese = false;
        }
    }

    /// 「輸出簡體字」目前的值。
    pub fn simplified(&self) -> Option<bool> {
        self.simplified_live.then_some(self.simplified)
    }

    /// 換欄位時重新確認「輸出簡體字」。
    pub fn refresh_simplified(&mut self) -> bool {
        let probe = !self.client.connected();
        if probe && !self.client.connect_existing() {
            return false;
        }
        let reply = self.client.call(&json!({"op": "health"}));
        if probe && self.id == 0 {
            self.client.release();
        }
        match reply.filter(|r| r["ok"] == true) {
            Some(reply) => self.learn_simplified(&reply["info"]),
            None => false,
        }
    }

    /// 換欄位用的 refresh_simplified：時效內不再問。
    pub fn refresh_simplified_after(&mut self, fresh: Duration) -> bool {
        if self.simplified_at.is_some_and(|at| at.elapsed() < fresh) {
            return false;
        }
        self.refresh_simplified()
    }

    /// 切換「輸出簡體字」。
    pub fn output_simplified(&mut self, on: bool) -> Snapshot {
        if !self.simplified_live {
            return Snapshot::default();
        }
        self.handle(json!({"type": "output_simplified", "on": on}))
    }

    /// 不知道中英模式時，向引擎查新 session 開始時的模式。
    pub fn learn_mode(&mut self, spawn: bool) -> bool {
        zlog!(
            "learn_mode 進入 spawn={} mode_known={}",
            spawn,
            self.mode_known
        );
        if self.mode_known {
            return false;
        }
        let probe = !spawn && !self.client.connected();
        if probe && !self.client.connect_existing() {
            zlog!("learn_mode 放棄：connect_existing 失敗");
            return false;
        }
        let reply = self.client.call(&json!({"op": "health"}));
        zlog!(
            "health op 回應 ok={} connected={} probe={}",
            reply.as_ref().is_some_and(|r| r["ok"] == true),
            self.client.connected(),
            probe
        );
        let generation = self.client.generation();
        if probe && self.id == 0 {
            self.client.release();
        }
        let Some(reply) = reply.filter(|r| r["ok"] == true) else {
            zlog!("learn_mode 放棄：health 沒有 ok");
            return false;
        };
        self.mode_known = true;
        self.shared = reply["info"]["mode_memory"] == "shared";
        self.learn_capabilities(&reply["info"], generation);
        let simplified = self.learn_simplified(&reply["info"]);
        let before = (self.english, self.japanese);
        if let Some(mode) = initial_mode(&reply["info"]) {
            self.english = mode == Mode::English;
            self.base_english = self.english;
            self.japanese = mode == Mode::Japanese;
            self.caps_english = false;
        }
        let changed = before != (self.english, self.japanese) || simplified;
        zlog!(
            "learn_mode 結束 changed={} mode_known={} english={} japanese={}",
            changed,
            self.mode_known,
            self.english,
            self.japanese
        );
        changed
    }

    /// 測試：設定日文模式與能力（不經引擎）。
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn set_japanese_for_test(&mut self, japanese: bool, convert_keys: bool) {
        self.japanese_cap = true;
        self.convert_keys_cap = convert_keys;
        self.clause_display_cap = convert_keys;
        self.japanese = japanese;
    }

    /// 目前的日文模式。
    pub fn japanese(&self) -> bool {
        self.japanese && self.japanese_cap
    }

    /// 目前的輸入方式。
    pub fn input_method(&self) -> &str {
        match self.english_state.input_method.as_str() {
            "" if self.japanese() => "romaji",
            method => method,
        }
    }

    /// 引擎是否支援日文。
    pub fn japanese_supported(&self) -> bool {
        self.japanese_cap
    }

    /// 引擎是否認得變換／無變換與 F6 到 F10。
    pub fn convert_keys(&self) -> bool {
        self.japanese_supported() && self.convert_keys_cap
    }

    /// 引擎是否回報文節樣式與目前文節。
    pub fn clause_display(&self) -> bool {
        self.japanese_supported() && self.clause_display_cap
    }

    /// 引擎是否回報ライブ変換。
    #[allow(dead_code)]
    pub fn live_conversion(&self) -> bool {
        self.japanese_supported() && self.live_conversion_cap
    }

    /// 目前的 session 是否可以送 #123 的新鍵名。
    pub fn vocab_keys(&self) -> bool {
        self.vocab_keys && self.id != 0 && self.generation == self.client.generation()
    }

    /// 每次按鍵前由外殼依欄位政策設定。
    pub fn set_private(&mut self, private: bool) {
        self.private = private;
        if private {
            self.pending_context = None;
        }
    }

    /// 開始新組字前由外殼呼叫：回傳剛提交、等待確認的文字。
    pub fn take_pending_context(&mut self) -> Option<(u64, String)> {
        if self.private {
            self.pending_context = None;
        }
        self.pending_context.take()
    }

    /// 外殼核對游標前的文字後回報。
    pub fn confirm_context(&mut self, id: u64, confirmed: bool) {
        if !confirmed || id == 0 || self.id == 0 || self.generation != self.client.generation() {
            return;
        }
        let _ = self.client.call(
            &json!({"op": "handle", "session": self.id, "req": {"type": "context", "on": true, "context_id": id}}),
        );
    }

    /// 目前是否英文模式。
    pub fn english(&self) -> bool {
        self.english
    }

    /// 按鍵當下的 Caps Lock 鎖定狀態要不要告訴引擎。
    pub fn caps_needs_sync(&self, on: bool) -> bool {
        self.caps_lock_english && (self.english_state.caps_lock || self.caps_english) != on
    }

    /// Caps Lock 鍵本身：確認 caps_lock_english 的快取還是新的。
    pub fn refresh_caps_lock_english(&mut self) {
        if self.client.connected()
            && self.caps_generation == self.client.generation()
            && self.client.alive()
        {
            return;
        }
        let probe = !self.client.connected();
        if probe && !self.client.connect_existing() {
            return;
        }
        let reply = self.client.call(&json!({"op": "health"}));
        let generation = self.client.generation();
        if probe && self.id == 0 {
            self.client.release();
        }
        if let Some(reply) = reply.filter(|r| r["ok"] == true) {
            self.learn_capabilities(&reply["info"], generation);
        }
    }

    /// 回報 Caps Lock 狀態。
    pub fn caps_lock(&mut self, on: bool) -> Snapshot {
        self.handle(json!({"type": "caps_lock", "on": on}))
    }

    /// 用快捷鍵切換簡體字是否可用。
    pub fn simplified_hotkey(&mut self) -> bool {
        let Some(reply) = self
            .client
            .call(&json!({"op": "health"}))
            .filter(|r| r["ok"] == true)
        else {
            return false;
        };
        self.learn_simplified(&reply["info"]);
        self.simplified_live && reply["info"]["simplified_hotkey"] == "true"
    }

    /// 引擎是否認得 Ctrl＋標點的鍵名而且設定開著。
    pub fn ctrl_punctuation(&mut self, key: &str) -> bool {
        self.client
            .call(&json!({"op": "health"}))
            .is_some_and(|reply| {
                reply["ok"] == true
                    && reply["info"]["ctrl_punctuation"] == "true"
                    && keys::ctrl_punct_known(
                        key,
                        reply["info"]["ctrl_punctuation_keys"]
                            .as_str()
                            .unwrap_or(""),
                    )
            })
    }

    /// 引擎目前載入的中英切換鍵設定。
    pub fn shift_config(&mut self) -> keys::ShiftConfig {
        if self.client.connected() {
            self.client.alive();
        }
        let reply = self.client.call(&json!({"op": "health"}));
        let info = reply
            .as_ref()
            .filter(|r| r["ok"] == true)
            .map(|r| &r["info"]);
        keys::ShiftConfig::from_info(info)
    }

    /// 沒在組字時這個鍵要不要交給引擎。
    pub fn style_shift_space(&mut self) -> bool {
        let _ = self.idle_key("");
        matches!(
            self.idle_info
                .as_ref()
                .and_then(|v| v["input_style_shift_space"].as_str()),
            Some("fullwidth" | "space" | "single")
        )
    }

    /// Caps Lock  lower 風格。
    pub fn style_caps_lower(&mut self) -> bool {
        let _ = self.idle_key("");
        self.idle_info
            .as_ref()
            .is_some_and(|v| v["input_style_caps_letter"] == "lower")
    }

    /// 沒在組字時這個鍵要不要交給引擎。
    pub fn idle_key(&mut self, key: &str) -> bool {
        let fresh = self.idle_info.is_some()
            && self.client.connected()
            && self.idle_generation == self.client.generation()
            && self.client.alive();
        let full_width = self.english_state.full_width;
        if !fresh {
            match self
                .client
                .call(&json!({"op": "health"}))
                .filter(|r| r["ok"] == true)
            {
                Some(reply) => {
                    self.idle_info = Some(reply["info"].clone());
                    self.idle_generation = self.client.generation();
                }
                None => {
                    return idle_key_from(
                        self.idle_info.as_ref().unwrap_or(&Value::Null),
                        key,
                        full_width,
                    )
                }
            }
        }
        self.idle_info
            .as_ref()
            .is_some_and(|info| idle_key_from(info, key, full_width))
    }

    /// 是否已從引擎學到中英模式（debug log 用，只有 Windows 的 service 會讀）。
    #[cfg(windows)]
    pub(crate) fn mode_known(&self) -> bool {
        self.mode_known
    }

    /// server 是否可用。
    pub fn available(&mut self) -> bool {
        if self.down && self.client.connect() {
            self.down = false;
        }
        !self.down
    }

    /// 開 session 並在必要時重播。
    fn ensure_open(&mut self) -> bool {
        if self.id != 0 && self.generation == self.client.generation() {
            return true;
        }
        let reply = self.client.call(&open_request());
        let Some(reply) = reply else {
            self.id = 0;
            return false;
        };
        if !reply["ok"].as_bool().unwrap_or(false) {
            zlog!("open: {}", reply["error"].as_str().unwrap_or("?"));
            self.id = 0;
            return false;
        }
        // 記住引擎在 Ready 配置的 id；reset／Drop 的 Close 必須送同一個值。
        // 舊引擎或手工偽造的 reply 沒有 session 時退回 1，與原本的固定值相同。
        self.id = reply["session"].as_u64().filter(|id| *id != 0).unwrap_or(1);
        self.generation = self.client.generation();
        self.vocab_keys = reply["info"]["vocab_keys"] == "true";
        self.shared = reply["info"]["mode_memory"] == "shared";
        self.learn_capabilities(&reply["info"], self.generation);
        let _ = self.learn_simplified(&reply["info"]);
        let start_english = reply["info"]["start_mode"] == "english";
        if reply["info"]["start_mode"] == "japanese" && self.japanese_cap && !self.english {
            self.japanese = true;
        }
        self.pending_context = None;
        if self.context_enabled {
            let _ = self.client.call(
                &json!({"op": "handle", "session": self.id, "req": {"type": "context", "context_enabled": true}}),
            );
        }
        if let Some(info) = reply["info"].as_object() {
            zlog!(
                "open: session={} server={} data={}",
                self.id,
                info.get("executable_sha256")
                    .and_then(|v| v.as_str())
                    .map(|s| &s[..s.len().min(12)])
                    .unwrap_or("?"),
                info.get("data_sha256")
                    .and_then(|v| v.as_str())
                    .map(|s| &s[..s.len().min(12)])
                    .unwrap_or("?")
            );
        }
        let want = if self.log.is_empty() {
            self.english
        } else {
            self.replay_english
        };
        for req in restore_requests(
            self.mode_known,
            want,
            self.caps_english,
            self.caps_lock_english,
            start_english,
        ) {
            let restored = self
                .client
                .call(&json!({"op": "handle", "session": self.id, "req": req}));
            if !restored
                .as_ref()
                .and_then(|r| r["ok"].as_bool())
                .unwrap_or(false)
            {
                self.id = 0;
                return false;
            }
        }
        let log = self.log.clone();
        for req in &log {
            let r = self
                .client
                .call(&json!({"op": "handle", "session": self.id, "req": req}));
            if r.is_none() {
                self.id = 0;
                return false;
            }
        }
        true
    }

    /// 送出一個 proto.Request。連線中斷時自動重連、重開 session 並重播。
    pub fn handle(&mut self, mut request: Value) -> Snapshot {
        if self.private {
            request["private"] = Value::Bool(true);
        }
        let generation_before = self.client.generation();
        let english_before = self.english;
        let kind = request["type"].as_str().unwrap_or("").to_string();
        // jc 的 State 只有組字片段帶模式，Committed 完全不帶；0c 快照的
        // mode／caps_english 得由外殼自己記住，成功後補回快照。
        let mut base = self.base_english;
        let mut want_english = self.english;
        let mut want_caps = self.caps_english;
        match kind.as_str() {
            "mode" => {
                base = request["on"].as_bool().unwrap_or(false);
                want_english = base;
                want_caps = false;
            }
            "toggle" => {
                base = !base;
                want_english = base;
                want_caps = false;
            }
            "caps_lock" => {
                let on = request["on"].as_bool().unwrap_or(false);
                want_caps = on && !base;
                want_english = on || base;
            }
            _ => {}
        }
        #[cfg(test)]
        self.sent.push(kind.clone());
        let mut cleared = None;
        if kind != "context" && kind != "display" {
            self.toggled = false;
            cleared = self.cleared.take();
        }
        let mut attempts = 0;
        let mut wait_until: Option<Instant> = None;
        loop {
            attempts += 1;
            if attempts > 3 {
                wait_until = wait_until.or(self.client.restart_deadline());
                if !wait_until.is_some_and(|until| Instant::now() < until) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            if !self.client.connect() || !self.ensure_open() {
                continue;
            }
            if !self.vocab_keys {
                downgrade_vocab_key(&mut request);
            }
            zlog!(
                "handle 送引擎 op={} session={} req={}",
                kind,
                self.id,
                request
            );
            let reply = self
                .client
                .call(&json!({"op": "handle", "session": self.id, "req": request}));
            let Some(reply) = reply else {
                zlog!("handle op={} 引擎沒有回應 attempt={}", kind, attempts);
                continue;
            };
            if !reply["ok"].as_bool().unwrap_or(false) {
                zlog!(
                    "handle op={} 引擎回 ok=false error={}",
                    kind,
                    reply["error"].as_str().unwrap_or("?")
                );
                self.id = 0;
                continue;
            }
            let mut s = parse(&reply);
            s.reconnected = generation_before != self.client.generation();
            // 有組字片段時引擎的模式是權威值；空組字與提交則補外殼記得的模式。
            if s.commit.is_empty() && !s.preedit.is_empty() {
                // 引擎的模式照用。
            } else {
                s.english = want_english;
                s.japanese = self.japanese && self.japanese_cap;
            }
            s.caps_english = want_caps;
            s.caps_lock = want_caps;
            s.output_simplified = self.simplified;
            if matches!(kind.as_str(), "mode" | "caps_lock" | "output_simplified") {
                s.handled = true;
            }
            if !s.commit.is_empty() {
                self.pending_context =
                    (self.context_enabled && !self.private && s.context_id != 0 && !s.reconnected)
                        .then(|| (s.context_id, s.commit.clone()));
            } else if s.reconnected {
                self.pending_context = None;
            }
            self.english = s.english;
            self.base_english = base;
            self.caps_english = s.caps_english;
            self.japanese = s.japanese && self.japanese_cap;
            apply_clause_capability(&mut s, self.clause_display());
            if self.simplified_live {
                self.simplified = s.output_simplified;
                self.simplified_at = Some(Instant::now());
            }
            self.mode_known = true;
            self.english_state = s.clone();
            self.down = false;
            if !s.commit.is_empty() || kind == "cancel" || !s.composing() {
                if kind == "toggle" && s.commit.is_empty() && !self.log.is_empty() {
                    let mut log = std::mem::take(&mut self.log);
                    log.push(request);
                    self.cleared = Some((log, self.replay_english));
                }
                self.log.clear();
            } else if kind != "caps_lock" && kind != "output_simplified" {
                if self.log.is_empty() {
                    match cleared.take().filter(|_| kind == "toggle") {
                        Some((log, english)) => {
                            self.log = log;
                            self.replay_english = english;
                        }
                        None => self.replay_english = english_before,
                    }
                }
                self.log.push(request);
            }
            zlog!(
                "handle 引擎回 op={} ok={} handled={} composing={} preedit={} commit={}",
                kind,
                s.ok,
                s.handled,
                s.composing(),
                s.text(),
                s.commit
            );
            return s;
        }
        zlog!("handle op={} 結束：server 不可用，放行按鍵", kind);
        self.client.abandon_restart();
        self.down = true;
        Snapshot::default()
    }

    /// 清除重播紀錄。
    pub fn reset(&mut self) {
        self.log.clear();
        self.cleared = None;
        self.toggled = false;
        self.english_state = Snapshot::default();
        self.pending_context = None;
        if self.shared {
            self.mode_known = false;
        }
        if self.id != 0 && self.generation == self.client.generation() {
            let _ = self
                .client
                .call(&json!({"op": "close", "session": self.id}));
        }
        self.id = 0;
    }

    /// 送一個按鍵。
    pub fn key(&mut self, key: &str) -> Snapshot {
        self.handle(json!({"type": "key", "key": key}))
    }

    /// 為游標所在片段重開候選。
    pub fn candidates(&mut self) -> Snapshot {
        self.handle(json!({"type": "candidates"}))
    }

    /// 單按 Shift 或輸入指示器按鈕切換中英。
    pub fn toggle(&mut self) -> Snapshot {
        let restore = self.toggled;
        let snapshot = self.handle(json!({"type": "toggle", "restore": restore}));
        self.toggled = snapshot.ok;
        snapshot
    }

    /// 沒有送給核心就交還宿主的按鍵同樣表示切換之後有操作。
    pub fn bypassed(&mut self) {
        self.toggled = false;
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.id != 0 && self.generation == self.client.generation() {
            let _ = self
                .client
                .call(&json!({"op": "close", "session": self.id}));
        }
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn chinese_full_width_preflight_covers_literal_keys_without_claiming_tones() {
        let snap = parse(&json!({"snapshot":{"mode":"zhuyin","full_width":true}}));
        for key in ["A", "Z", " ", "'", "\"", "\\", "<", ">", "?"] {
            assert!(snap.full_width_extra_key(key), "{key}");
        }
        for key in ["3", "a", "Tab", "ShiftSpace"] {
            assert!(!snap.full_width_extra_key(key), "{key}");
        }
        assert!(!Snapshot::default().full_width_extra_key("\\"));
        for key in ["KP1", "KP0", "KP*", "KP+", "KP-", "KP.", "KP/"] {
            assert!(snap.full_width_keypad(key), "{key}");
            assert!(!Snapshot::default().full_width_keypad(key), "半形 {key}");
        }
        for key in ["1", "KP", "Left", "@"] {
            assert!(!snap.full_width_keypad(key), "{key}");
        }
    }

    #[test]
    fn full_width_english_preflight_consumes_printable_keys_without_completion() {
        let snap = parse(&json!({"snapshot":{"mode":"english","full_width":true}}));
        for key in ["a", "A", "0", "9", " ", "'", "\\"] {
            assert!(snap.english_wants_key(key), "{key}");
        }
        for key in ["KP1", "KP0", "KP*", "KP+", "KP."] {
            assert!(snap.english_wants_key(key), "{key}");
        }
        for key in ["Tab", "Escape", "Left", "KP"] {
            assert!(!snap.english_wants_key(key), "{key}");
        }
        assert!(!parse(&json!({"snapshot":{"mode":"english"}})).english_wants_key("a"));
        assert!(
            !parse(&json!({"snapshot":{"mode":"english"}})).english_wants_key("KP1"),
            "半形數字鍵盤照舊交給宿主"
        );
    }

    #[test]
    fn english_key_preflight_matches_buffer_boundaries() {
        let mut s = parse(
            &json!({"snapshot": {"mode": "english", "english_autocomplete": true,
            "english_pending": true, "preedit": [{"text":"hel"}],
            "candidates": {"items":[{"id":0,"text":"hello"}]}}}),
        );
        assert!(s.english_pending);
        for k in [
            "h",
            "H",
            "Backspace",
            "Escape",
            "Enter",
            "Tab",
            "Up",
            "Down",
        ] {
            assert!(s.english_wants_key(k), "{}", k);
        }
        for k in [
            " ", ".", "1", "Left", "Right", "Delete", "Home", "End", "PageUp", "PageDown",
        ] {
            assert!(!s.english_wants_key(k), "must pass host action: {}", k);
        }
        s.candidates.clear();
        for k in ["Tab", "Up", "Down"] {
            assert!(!s.english_wants_key(k));
        }
        s.preedit[0].text = "a".repeat(48);
        assert!(
            !s.english_wants_key("a"),
            "word limit must pass the next letter"
        );
        let legacy = parse(&json!({"snapshot":{"mode":"english"}}));
        assert!(!legacy.english_pending && !legacy.english_wants_key("a"));
    }

    #[test]
    fn open_request_is_profile_free() {
        let open = open_request();
        assert_eq!(open["op"], "hello", "adapter 改點一：0c open 改成 jc hello");
        assert_eq!(open["protocol"], PROTOCOL_VERSION);
        assert_eq!(open["protocol"], 3);
        assert_eq!(open["client"], CLIENT_NAME);
        assert!(open.get("version").is_none());
        assert!(open.get("language").is_none(), "open 不能帶語言：{open}");
        assert_eq!(
            open.as_object().unwrap().len(),
            3,
            "hello 只有 op／protocol／client：{open}"
        );
    }

    #[test]
    fn restore_mode_only_when_it_differs_from_the_start() {
        assert_eq!(restore_mode(true, true, false), Some(true));
        assert_eq!(restore_mode(true, false, false), None);
        assert_eq!(restore_mode(true, false, true), Some(false));
        assert_eq!(restore_mode(true, true, true), None);
        assert_eq!(restore_mode(false, false, true), None);
        assert_eq!(
            initial_english(&json!({"initial_mode": "english"})),
            Some(true)
        );
        assert_eq!(
            initial_english(&json!({"initial_mode": "zhuyin"})),
            Some(false)
        );
        assert_eq!(
            initial_english(&json!({"initial_mode": "japanese"})),
            Some(false)
        );
        assert_eq!(
            initial_mode(&json!({"initial_mode": "japanese"})),
            Some(Mode::Japanese)
        );
        assert_eq!(
            initial_mode(&json!({"initial_mode": "zhuyin"})),
            Some(Mode::Chinese)
        );
        assert_eq!(initial_mode(&json!({"initial_mode": "future"})), None);
        assert_eq!(initial_english(&json!({"version": "old"})), None);
    }

    #[test]
    fn caps_english_is_restored_as_temporary() {
        let caps_on = json!({"type": "caps_lock", "on": true});
        let zhuyin = json!({"type": "mode", "on": false});
        let english = json!({"type": "mode", "on": true});
        assert_eq!(
            restore_requests(true, true, true, true, false),
            vec![caps_on.clone()]
        );
        assert_eq!(
            restore_requests(true, true, true, true, true),
            vec![zhuyin.clone(), caps_on.clone()]
        );
        assert_eq!(
            restore_requests(true, true, true, false, false),
            Vec::<Value>::new()
        );
        assert_eq!(
            restore_requests(true, true, true, false, true),
            vec![zhuyin.clone()]
        );
        assert_eq!(
            restore_requests(true, true, false, true, false),
            vec![english.clone()]
        );
        assert_eq!(
            restore_requests(true, false, false, true, true),
            vec![zhuyin.clone()]
        );
        assert_eq!(
            restore_requests(false, true, true, true, false),
            vec![caps_on]
        );
    }

    #[test]
    fn candidate_notes_line_up_with_candidates() {
        let reply: Value = serde_json::from_str(
            r#"{"snapshot":{"handled":true,"candidates":{"page":0,"pages":1,"highlight":0,
                "items":[{"id":0,"text":"吃"},{"id":1,"text":"痴","note":"近音 ㄔ"}]}}}"#,
        )
        .unwrap();
        let s = parse(&reply);
        assert_eq!(s.candidates.len(), 2);
        assert_eq!(
            s.candidate_notes,
            vec!["".to_string(), "近音 ㄔ".to_string()]
        );
        assert_eq!(s.candidate_keys, vec!["1", "2"]);
    }

    #[test]
    fn candidate_range_is_parsed_for_segment_following() {
        let s = parse(&json!({"snapshot": {"handled": true, "candidates": {
            "start": 1, "length": 2, "items": [{"id": 4, "text": "風景"}]}}}));
        assert_eq!((s.candidate_start, s.candidate_length), (1, 2));
        assert_eq!(s.candidates, vec![(4, "風景".to_string())]);
        let old = parse(&json!({"snapshot": {"candidates": {"items": [{"id": 0, "text": "我"}]}}}));
        assert_eq!((old.candidate_start, old.candidate_length), (0, 0));
    }

    #[test]
    fn candidate_keys_follow_the_engine() {
        let s = parse(
            &json!({"snapshot": {"mode": "zhuyin", "candidates": {"items": [
            {"id": 0, "text": "是", "key": "1"}, {"id": 1, "text": "式", "key": "q"}, {"id": 2, "text": "示", "key": ";"}]}}}),
        );
        assert_eq!(s.candidate_keys, vec!["1", "q", ";"]);
        let items: Vec<Value> = (0..10).map(|i| json!({"id": i, "text": "字"})).collect();
        let old = parse(&json!({"snapshot": {"mode": "zhuyin", "candidates": {"items": items}}}));
        assert_eq!(
            old.candidate_keys,
            vec!["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"]
        );
        let english = parse(
            &json!({"snapshot": {"mode": "english", "candidates": {"items": [{"id": 0, "text": "hello"}]}}}),
        );
        assert_eq!(english.candidate_keys, vec![""]);
    }

    #[test]
    fn bell_is_parsed() {
        let s = parse(&json!({"snapshot": {"handled": true, "bell": true,
            "preedit": [{"text": "ㄅㄩ", "style": "pending"}]}}));
        assert!(s.bell && s.text() == "ㄅㄩ");
        assert!(!parse(&json!({"snapshot": {"handled": true}})).bell);
    }

    #[test]
    fn notices_and_marked_style_are_parsed() {
        let s = parse(&json!({"snapshot": {"handled": true,
            "preedit": [{"text": "我和", "style": "normal"}, {"text": "世傑", "style": "marked"}],
            "notices": ["已加入詞庫：世傑", "", 3]}}));
        assert_eq!(s.notices, vec!["已加入詞庫：世傑".to_string()]);
        assert_eq!(s.notice(), "已加入詞庫：世傑");
        assert_eq!(s.preedit[1].style, "marked");
        assert!(s.marking());
        let two = parse(&json!({"snapshot": {"notices": ["甲", "乙"]}}));
        assert_eq!(two.notice(), "甲；乙");
        let old = parse(&json!({"snapshot": {"handled": true}}));
        assert!(old.notices.is_empty() && old.notice().is_empty());
        assert!(!two.marking() && !old.marking());
    }

    #[test]
    fn reading_hint_is_parsed() {
        let s = parse(&json!({"snapshot": {"handled": true,
            "preedit": [{"text": "20", "style": "reading"}],
            "reading_hint": {"start": 0, "length": 2, "text": "ㄉㄢ"}}}));
        assert_eq!(s.reading_hint, "ㄉㄢ");
        assert_eq!(s.preedit[0].style, "reading");
        assert!(parse(&json!({"snapshot": {"handled": true}}))
            .reading_hint
            .is_empty());
    }

    #[test]
    fn japanese_snapshot_fields_are_parsed() {
        let s = parse(&json!({"snapshot": {
            "handled": true, "mode": "japanese", "language": "ja", "input_method": "romaji",
            "converting": true, "clause_index": 1, "clause_count": 2,
            "preedit": [{"text": "今日", "style": "clause"}, {"text": "は", "style": "clause_active"}]
        }}));
        assert!(s.japanese && !s.english);
        assert_eq!(s.language, "ja");
        assert_eq!(s.input_method, "romaji");
        assert!(s.converting);
        assert_eq!((s.clause_index, s.clause_count), (1, 2));
        assert_eq!(s.clause_title(), "文節 2/2");
        assert_eq!(s.candidate_title(), "文節 2/2");
        assert_eq!(s.preedit[0].style, "clause");
        assert_eq!(s.preedit[1].style, "clause_active");
        let old = parse(&json!({"snapshot": {"handled": true, "mode": "zhuyin"}}));
        assert!(!old.japanese && !old.converting && old.clause_title().is_empty());
        assert_eq!(old.candidate_title(), "");
        let english = parse(&json!({"snapshot": {"mode": "english", "language": "ja"}}));
        assert!(!english.japanese && english.english);
    }

    #[test]
    fn clause_title_combines_with_candidate_title_and_guards_bad_indexes() {
        let mut s = parse(&json!({"snapshot": {"mode": "japanese",
            "clause_index": 0, "clause_count": 3, "candidates": {"title": "変換", "items": []}}}));
        assert_eq!(s.clause_title(), "文節 1/3");
        assert_eq!(s.candidate_title(), "変換　文節 1/3");
        s.clause_index = 9;
        assert_eq!(s.clause_title(), "文節 3/3");
        s.clause_index = -1;
        assert_eq!(s.clause_title(), "文節 1/3");
        s.clause_count = 0;
        assert_eq!(s.candidate_title(), "変換");
    }

    #[test]
    fn clause_styles_fall_back_when_the_engine_lacks_the_capability() {
        let mut s = parse(
            &json!({"snapshot": {"mode": "japanese", "clause_index": 1, "clause_count": 2,
            "preedit": [{"text": "今日", "style": "clause"}, {"text": "は", "style": "clause_active"}]}}),
        );
        apply_clause_capability(&mut s, false);
        assert!(s.preedit.iter().all(|g| g.style == "normal"));
        assert_eq!((s.clause_index, s.clause_count), (0, 0));
        assert!(s.clause_title().is_empty());
        let mut supported = parse(&json!({"snapshot": {"mode": "japanese", "clause_index": 1,
            "clause_count": 2, "preedit": [{"text": "は", "style": "clause_active"}]}}));
        apply_clause_capability(&mut supported, true);
        assert_eq!(supported.preedit[0].style, "clause_active");
        assert_eq!(supported.clause_title(), "文節 2/2");
    }

    #[test]
    fn japanese_capabilities_gate_the_new_keys() {
        let mut s = Session::new();
        s.learn_capabilities(&json!({}), 0);
        assert!(!s.japanese_supported() && !s.convert_keys() && !s.clause_display());
        s.learn_capabilities(
            &json!({"japanese": "true", "convert_keys": "true", "clause_display": "true", "live_conversion": "true"}),
            0,
        );
        assert!(
            s.japanese_supported() && s.convert_keys() && s.clause_display() && s.live_conversion()
        );
        s.set_japanese_for_test(true, true);
        assert!(s.japanese());
        s.learn_capabilities(&json!({}), 0);
        assert!(
            !s.japanese_supported() && !s.japanese() && !s.convert_keys() && !s.clause_display()
        );
        s.learn_capabilities(&json!({"japanese": "true"}), 0);
        s.set_japanese_for_test(true, false);
        assert!(s.japanese() && !s.convert_keys() && !s.clause_display());
    }

    #[test]
    fn vocab_key_names_fall_back_for_unsupported_engines() {
        for (name, base) in [
            ("ShiftLeft", "Left"),
            ("ShiftRight", "Right"),
            ("ShiftDelete", "Delete"),
            ("ShiftBackspace", "Backspace"),
        ] {
            let mut r = json!({"type": "key", "key": name, "private": true});
            downgrade_vocab_key(&mut r);
            assert_eq!(r, json!({"type": "key", "key": base, "private": true}));
        }
        for mut r in [
            json!({"type": "key", "key": "Left"}),
            json!({"type": "key", "key": "a"}),
            json!({"type": "select", "key": "ShiftLeft", "index": 1}),
        ] {
            let before = r.clone();
            downgrade_vocab_key(&mut r);
            assert_eq!(r, before);
        }
    }

    // Adapter 改點二的關鍵測試：english_pending 必須來自 jc suggestion。
    #[test]
    fn english_pending_comes_from_jc_suggestion() {
        let state = Response::State {
            preedit: "hel".into(),
            cursor: 3,
            candidates: vec!["hello".into()],
            details: vec![jc_proto::CandidateDetail {
                reading: "hello".into(),
                source: jc_core_source(),
                cost: 0,
                learned: false,
                pos: None,
            }],
            segments: vec![jc_proto::SegmentInfo {
                start: 0,
                end: 3,
                mode: "english".into(),
                script: "latin".into(),
            }],
            highlighted: 0,
            cursor_rune: 3,
            suggestion: Some("lo".into()),
            label: None,
            navigating: false,
        };
        let reply = response_to_reply(&state);
        let s = parse(&reply);
        assert!(s.ok && s.english && s.english_pending && s.english_autocomplete);
        assert_eq!(s.suggestion, "lo");
        assert_eq!(s.text(), "hel");
        assert_eq!(s.cursor_utf16, 3);
        assert_eq!(s.candidate_keys, vec![""]);
        assert!(s.english_wants_key("Enter"), "#235：Enter 只送出單字");
        assert!(
            !s.english_wants_key("1"),
            "英文模式的數字是文字，不是選字鍵"
        );

        let no_suggestion = Response::State {
            preedit: "hel".into(),
            cursor: 3,
            candidates: Vec::new(),
            details: Vec::new(),
            segments: Vec::new(),
            highlighted: 0,
            cursor_rune: 3,
            suggestion: None,
            label: None,
            navigating: false,
        };
        let s = parse(&response_to_reply(&no_suggestion));
        assert!(!s.english_pending, "沒有 suggestion 就沒有英文補全");
        assert!(!s.english_autocomplete);
    }

    #[test]
    fn ready_reply_carries_the_engine_session_id() {
        let ready = Response::Ready {
            protocol: PROTOCOL_VERSION,
            engine: "jc-engine test".into(),
            session: 23,
        };
        let reply = response_to_reply(&ready);
        assert_eq!(reply["ok"], true);
        assert_eq!(reply["session"], 23, "adapter 不可再寫死 session 1");
        assert_eq!(reply["engine"], "jc-engine test");

        let old: Response =
            serde_json::from_str(r#"{"op":"ready","protocol":3,"engine":"old"}"#).unwrap();
        assert_eq!(
            response_to_reply(&old)["session"],
            1,
            "舊引擎沒有 session 欄位時沿用 1"
        );
    }

    /// 測試用候選來源；只需一個合法的 `CandidateSource` 值。
    fn jc_core_source() -> jc_proto::CandidateSource {
        jc_proto::CandidateSource::Lexicon
    }
}

#[cfg(test)]
mod idle_tone_key_tests {
    use super::*;

    #[test]
    fn idle_tone_keys_go_to_the_engine_in_mixed_input_too() {
        assert!(idle_tone_keys(
            &json!({"tone_mark_keys": "true", "mixed_input": "false"})
        ));
        assert!(idle_tone_keys(
            &json!({"tone_mark_keys": "false", "mixed_input": "true"})
        ));
        assert!(!idle_tone_keys(
            &json!({"tone_mark_keys": "false", "mixed_input": "false"})
        ));
        assert!(!idle_tone_keys(&json!({})));
    }

    #[test]
    fn idle_keys_follow_the_engine_report() {
        let hsu = json!({"idle_keys": "!\"&'(),.:;<=>?[\\]^_`abcdefghijklmnoprstuvwxyz{}~", "keyboard_layout": "hsu", "mixed_input": "true", "tone_mark_keys": "true"});
        for key in ["1", "3", "4", "6", "7", "0", "-", "/", "q"] {
            assert!(!idle_key_from(&hsu, key, false), "{key}");
        }
        for key in ["=", "a", ","] {
            assert!(idle_key_from(&hsu, key, false), "{key}");
        }
        let standard = json!({"keyboard_layout": "standard", "emoji_shortcut": "equals", "tone_mark_keys": "true"});
        for key in ["1", "2", "5", "3", "7", "=", "-", "/"] {
            assert!(idle_key_from(&standard, key, false), "{key}");
        }
        let eten = json!({"keyboard_layout": "eten", "emoji_shortcut": "equals", "tone_mark_keys": "true"});
        for key in ["1", "2", "7", "="] {
            assert!(idle_key_from(&eten, key, false), "{key}");
        }
        for key in ["5", "6"] {
            assert!(!idle_key_from(&eten, key, false), "{key}");
        }
        assert!(!idle_key_from(
            &json!({"emoji_shortcut": "off"}),
            "=",
            false
        ));
        assert!(!idle_key_from(&json!({}), "3", false));
    }

    #[test]
    fn full_width_idle_keys_follow_the_engine_report() {
        let info = json!({
            "idle_keys": "!\"&'(),-./0123456789:;<=>?[\\]^_`abcdefghijklmnopqrstuvwxyz{}~",
            "idle_keys_full_width": "!\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~",
        });
        for key in ["@", "#", "$", "%", "*", "+", "|", "A"] {
            assert!(!idle_key_from(&info, key, false), "半形 {key}");
            assert!(idle_key_from(&info, key, true), "全形 {key}");
        }
        for key in ["j", "3", ",", "["] {
            assert!(
                idle_key_from(&info, key, false) && idle_key_from(&info, key, true),
                "{key}"
            );
        }
        assert!(!idle_key_from(&info, "KP1", true));
        let partial = json!({"idle_keys": "abc", "idle_keys_full_width": "abc@"});
        assert!(idle_key_from(&partial, "@", true));
        assert!(!idle_key_from(&partial, "#", true));
        let old = json!({"idle_keys": "abc"});
        assert!(!idle_key_from(&old, "@", true));
        assert!(idle_key_from(&old, "a", true));
    }
}

#[cfg(test)]
mod tests {
    //! 0c 的 named-pipe fixture 改成記憶體 transport：行為與測試名照搬，
    //! 只想驗證狀態機，不碰真的 pipe 與 Windows API。
    use super::*;
    use std::sync::{Arc, Mutex};

    struct MockState {
        connected: bool,
        down: bool,
        hang: bool,
        reject_left: usize,
        keys: Vec<String>,
        requests: Vec<Value>,
        info: Value,
        english: bool,
        candidates: Vec<String>,
        vocab_keys: Option<String>,
        start_english: bool,
        session: u64,
        /// `true` 時 hello 回真正的 jc Ready 訊框，用來驗證 session id 流轉。
        ready_wire: bool,
        /// 每次按鍵都提交：模擬 Go 的 RestartServer／HandoffServer 假引擎。
        commit_keys: bool,
        /// 交接後第一個按鍵直接提交這段文字。
        handoff_commit: Option<String>,
        /// toggle 清掉的注音，restore 時放回。
        cleared_keys: Vec<String>,
    }

    impl MockState {
        fn new() -> Self {
            Self {
                connected: false,
                down: false,
                hang: false,
                reject_left: 0,
                keys: Vec::new(),
                requests: Vec::new(),
                info: json!({}),
                english: false,
                candidates: Vec::new(),
                vocab_keys: None,
                start_english: false,
                session: 1,
                ready_wire: false,
                commit_keys: false,
                handoff_commit: None,
                cleared_keys: Vec::new(),
            }
        }
    }

    /// 測試用回應回呼：把 request 轉成回傳值，必要時改寫 MockState。
    type Respond = Box<dyn Fn(&Value, &mut MockState) -> Option<Value>>;

    struct MockTransport {
        state: Arc<Mutex<MockState>>,
        respond: Respond,
    }

    impl Transport for MockTransport {
        fn request(&mut self, request: &Value) -> Option<Value> {
            let mut st = self.state.lock().unwrap();
            st.requests.push(request.clone());
            (self.respond)(request, &mut st)
        }
        fn connected(&self) -> bool {
            self.state.lock().unwrap().connected
        }
        fn connect(&mut self) -> bool {
            let mut st = self.state.lock().unwrap();
            if st.connected {
                return true;
            }
            if st.down {
                return false;
            }
            st.connected = true;
            true
        }
        fn close(&mut self) {
            self.state.lock().unwrap().connected = false;
        }
        fn alive(&mut self) -> bool {
            let st = self.state.lock().unwrap();
            st.connected && !st.down
        }
    }

    /// hello 與 health 共用的 info：開始時英文的引擎要同時回報 start_mode／initial_mode。
    fn mock_info(st: &MockState) -> Value {
        let mut info = st.info.clone();
        if let Some(v) = &st.vocab_keys {
            info["vocab_keys"] = Value::from(v.clone());
        }
        if st.start_english {
            info["start_mode"] = Value::from("english");
            info["initial_mode"] = Value::from("english");
        }
        info
    }

    /// 小型假引擎：注音按鍵序列對到具體組字字串，其餘照 0c 的 reply 形狀回。
    fn toy_response(request: &Value, st: &mut MockState) -> Option<Value> {
        if st.hang {
            return None;
        }
        if let Some(error) = st.info["error"].as_str() {
            return Some(json!({"ok": false, "error": error}));
        }
        if st.reject_left > 0 {
            st.reject_left -= 1;
            return Some(json!({"ok": false, "error": "引擎正在切換"}));
        }
        match request["op"].as_str().unwrap_or("") {
            "hello" => {
                // 新 session 從開始模式重新起，舊的注音緩衝留在舊 session。
                st.english = st.info["initial_mode"] == "english" || st.start_english;
                st.keys.clear();
                st.cleared_keys.clear();
                if st.ready_wire {
                    // T12d：驗證 session id 流轉時回真正的 jc Ready 訊框。
                    let ready = Response::Ready {
                        protocol: PROTOCOL_VERSION,
                        engine: "toy".into(),
                        session: st.session,
                    };
                    return serde_json::to_value(ready).ok();
                }
                Some(json!({"ok": true, "session": st.session, "info": mock_info(st)}))
            }
            "health" => Some(json!({"ok": true, "info": mock_info(st)})),
            "mode" => {
                st.english = request["on"].as_bool().unwrap_or(false);
                st.keys.clear();
                Some(json!(toy_state(st)))
            }
            "key" => {
                let key = request["key"].as_str().unwrap_or("").to_string();
                if st.commit_keys {
                    st.keys.push(key.clone()); // 假引擎照 Go 的 RestartServer 記下收到的鍵
                    return Some(json!(Response::Committed { text: key }));
                }
                if let Some(text) = st.handoff_commit.clone() {
                    return Some(json!(Response::Committed { text }));
                }
                st.keys.push(key);
                Some(json!(toy_state(st)))
            }
            "toggle" => {
                st.english = !st.english;
                if request["restore"].as_bool().unwrap_or(false) {
                    st.keys = st.cleared_keys.clone();
                } else {
                    st.cleared_keys = st.keys.clone();
                    st.keys.clear();
                }
                Some(json!(toy_state(st)))
            }
            "caps_lock" => {
                st.english = request["on"].as_bool().unwrap_or(false);
                Some(json!(toy_state(st)))
            }
            "output_simplified" => {
                st.info["output_simplified"] = request["on"].clone();
                Some(json!(toy_state(st)))
            }
            "snapshot" => Some(json!(toy_state(st))),
            _ => Some(json!({"ok": true})),
        }
    }

    fn toy_state(st: &mut MockState) -> Response {
        // Enter 提交組字、KP 數字鍵盤在半形交還宿主、英文的字詞邊界鍵也交還宿主，
        // 都不進組字區（boundary 判斷仍看最後一個實際收到的鍵）。
        let seq: Vec<&str> = st
            .keys
            .iter()
            .map(String::as_str)
            .filter(|k| {
                *k != "Enter"
                    && !k.starts_with("KP")
                    && !matches!(*k, " " | "Left" | "Delete" | ".")
            })
            .collect();
        let joined = seq.concat();
        let last = st.keys.last().map(String::as_str).unwrap_or("");
        // Caps Lock 帶來的暫時英文：大寫字母直接交給應用程式（不進自動補全）。
        let passthrough = st.english && last.len() == 1 && last.as_bytes()[0].is_ascii_uppercase();
        let mut preedit = if passthrough {
            String::new()
        } else if st.english {
            joined.clone()
        } else if joined == "su3cl3" {
            "你好".to_string()
        } else if joined == "su3" {
            "你".to_string()
        } else if joined == "su3c" {
            "你ㄏ".to_string()
        } else if joined == "j" {
            "ㄨ".to_string()
        } else if joined == "ji" {
            "ㄨㄛ".to_string()
        } else if joined == "c" {
            "ㄏ".to_string()
        } else if joined == "s" {
            "ㄙ".to_string()
        } else if joined == "u" {
            "ㄡ".to_string()
        } else if joined == "=" {
            String::new()
        } else {
            joined.clone()
        };

        let boundary = st.english && matches!(last, " " | "Left" | "Delete" | ".");
        let selected = st.english && last == "Tab" && !st.candidates.is_empty();
        let mut commit = String::new();
        if last == "Enter" || boundary {
            commit = std::mem::take(&mut preedit);
        } else if selected {
            commit = st.candidates[0].clone();
            preedit.clear();
        } else if joined == "==" {
            // Emoji 面板開著時第二個 = 提交字面。
            commit = "=".to_string();
            preedit.clear();
        }
        if !commit.is_empty() {
            st.keys.clear(); // 提交後清空組字緩衝
            return Response::Committed { text: commit };
        }

        let candidates = if joined == "=" || !st.candidates.is_empty() {
            if st.candidates.is_empty() {
                vec!["😀".to_string()]
            } else {
                st.candidates.clone()
            }
        } else {
            Vec::new()
        };
        let suggestion = if st.english && !preedit.is_empty() && preedit.chars().count() < 5 {
            Some("lo".to_string())
        } else {
            None
        };
        let cursor = preedit.chars().count();
        let segments = if preedit.is_empty() {
            Vec::new()
        } else {
            vec![jc_proto::SegmentInfo {
                start: 0,
                end: cursor,
                mode: if st.english { "english" } else { "zhuyin" }.to_string(),
                script: "han".to_string(),
            }]
        };
        Response::State {
            preedit,
            cursor,
            cursor_rune: cursor,
            candidates,
            details: Vec::new(),
            segments,
            highlighted: 0,
            suggestion,
            label: None,
            navigating: false,
        }
    }

    fn mock_client<F>(respond: F) -> (Client, Arc<Mutex<MockState>>)
    where
        F: Fn(&Value, &mut MockState) -> Option<Value> + 'static,
    {
        let state = Arc::new(Mutex::new(MockState::new()));
        let shared = state.clone();
        let transport = MockTransport {
            state,
            respond: Box::new(respond),
        };
        (Client::with_transport(Box::new(transport)), shared)
    }

    fn toy_session() -> (Session, Arc<Mutex<MockState>>) {
        let (client, state) = mock_client(toy_response);
        (Session::with_client(client), state)
    }

    /// 多個 session 共用同一個假引擎（模擬同一個程式開在不同欄位）。
    fn toy_session_with_state(state: &Arc<Mutex<MockState>>) -> Session {
        let transport = MockTransport {
            state: state.clone(),
            respond: Box::new(toy_response),
        };
        Session::with_client(Client::with_transport(Box::new(transport)))
    }

    #[test]
    fn engine_session_id_is_remembered_and_sent_on_close() {
        let (mut s, st) = toy_session();
        {
            let mut state = st.lock().unwrap();
            state.session = 7;
            state.ready_wire = true;
        }

        // 第一次按鍵會先 hello 開 session；Hello 回的是真正的 jc Ready 訊框。
        assert!(s.key("j").ok);
        assert_eq!(s.id, 7, "adapter 必須記住引擎配置的 id");

        s.reset();
        let requests = st.lock().unwrap().requests.clone();
        let close = requests.last().expect("reset 要送 Close");
        assert_eq!(close["op"], "close");
        assert_eq!(close["session"], 7, "Close 要送引擎給的 id，不是 1");
    }

    #[test]
    fn english_autocomplete_preserves_host_boundaries() {
        let (mut s, st) = toy_session();
        st.lock().unwrap().candidates = vec!["hello".into()];
        let start = s.handle(json!({"type": "mode", "on": true}));
        assert!(start.ok && start.english);
        for boundary in [" ", "Left", "Delete", "."] {
            for k in ["h", "e", "l"] {
                assert!(s.key(k).handled, "{boundary} {k}");
            }
            assert!(s.english_state.english_pending);
            assert!(
                !s.english_state.english_wants_key(boundary),
                "TestKeyDown must decline {boundary}"
            );
            let next = s.key(boundary);
            assert!(next.ok && !next.english_pending);
            assert_eq!(next.commit, "hel");
        }
        for k in ["h", "e", "l"] {
            s.key(k);
        }
        let accepted = s.key("Tab");
        assert!(accepted.handled);
        assert_eq!(accepted.commit, "hello");
        assert!(!s.english_state.english_wants_key("Enter"));
    }

    #[test]
    fn types_and_commits() {
        let (mut s, _) = toy_session();
        let mut last = Snapshot::default();
        for k in ["s", "u", "3", "c", "l", "3"] {
            last = s.key(k);
            assert!(last.ok, "server 不可用");
            assert!(last.handled);
        }
        assert_eq!(last.text(), "你好");
        assert_eq!(last.cursor_utf16, 2);
        assert!(last.composing());
        let committed = s.key("Enter");
        assert_eq!(committed.commit, "你好");
        assert!(!committed.composing());
        assert!(s.log.is_empty(), "提交後重播紀錄要清空");
    }

    #[test]
    fn emoji_opens_without_preedit_and_double_equals_commits_once() {
        let (mut s, st) = toy_session();
        st.lock().unwrap().info = json!({"emoji_shortcut": "equals"});
        assert!(s.idle_key("="));
        assert_eq!(s.id, 0, "查設定不能建立或改動組字");
        let palette = s.key("=");
        assert!(palette.ok && palette.handled && palette.composing());
        assert_eq!(palette.text(), "");
        assert_eq!(palette.candidate_kind, "", "jc 尚無對應 candidate_kind");
        assert!(!palette.candidates.is_empty());
        let literal = s.key("=");
        assert_eq!(literal.commit, "=", "第二次 = 提交字面（mock 的 = 沒組字）");
        assert!(!literal.composing());
    }

    #[test]
    fn commit_offers_document_context_once() {
        let (mut s, _) = toy_session();
        for k in ["s", "u", "3", "c", "l", "3"] {
            s.key(k);
        }
        let committed = s.key("Enter");
        assert_eq!(committed.commit, "你好");
        // jc 尚無對應 context_id：0 代號不提供前文。
        assert!(s.take_pending_context().is_none());
        s.reset();
        assert!(s.take_pending_context().is_none());
    }

    #[test]
    fn private_field_composes_without_offering_context() {
        let (mut s, _) = toy_session();
        s.set_private(true);
        for k in ["s", "u", "3", "c", "l", "3"] {
            assert!(s.key(k).handled);
        }
        assert!(
            s.log.iter().all(|r| r["private"] == true),
            "重播紀錄也要是私人組字"
        );
        assert_eq!(s.key("Enter").commit, "你好");
        assert!(s.take_pending_context().is_none(), "私人欄位不讀前文");
        s.set_private(false);
        for k in ["s", "u", "3"] {
            s.key(k);
        }
        assert!(s.log.iter().all(|r| r.get("private").is_none()));
        s.reset();
    }

    #[test]
    fn start_english_is_known_before_the_first_key() {
        let (mut s, st) = toy_session();
        st.lock().unwrap().start_english = true;
        assert!(!s.english());
        assert!(
            s.learn_mode(false) && s.english(),
            "Activate 時就知道開始時是英文"
        );
        assert!(!s.client.connected(), "Activate 的查詢不留連線");
        assert!(!s.learn_mode(true), "問過就不再問");
        let snap = s.key("t");
        assert!(
            snap.ok && snap.english && snap.english_pending,
            "第一個按鍵是英文補全"
        );
        let snap = s.toggle();
        assert!(snap.ok && !snap.english);
        s.client.disconnect_for_test();
        let snap = s.key("j");
        assert!(
            !snap.english && snap.handled && snap.text() == "ㄨ",
            "重開 session 後仍是這個程式切過的注音"
        );
        s.key("Escape");
        s.reset();
        assert!(
            !s.learn_mode(false) && !s.english(),
            "各程式各自記住：換欄位不重新問"
        );
    }

    #[test]
    fn output_simplified_switches_live_and_is_not_replayed() {
        let (mut menu, st) = toy_session();
        st.lock().unwrap().info =
            json!({"output_simplified_live": "true", "output_simplified": "false"});
        assert_eq!(menu.simplified(), None);
        assert!(menu.refresh_simplified());
        assert_eq!(menu.simplified(), Some(false));
        assert!(!menu.client.connected());
        let snap = menu.output_simplified(true);
        assert!(snap.ok && snap.handled, "{:?}", snap);
        assert_eq!(
            menu.simplified(),
            Some(false),
            "jc 尚無對應 output_simplified 快照欄位"
        );
        assert!(
            menu.log.iter().all(|r| r["type"] != "output_simplified"),
            "不記進重播紀錄"
        );
        menu.client.disconnect_for_test();
        assert!(menu.key("Enter").ok);
        assert_eq!(menu.simplified(), Some(false));
    }

    #[test]
    fn full_width_preflight_matches_the_engine() {
        let (mut s, _) = toy_session();
        for key in ["KP1", "KP0", "KP*", "KP+", "KP-", "KP.", "KP/"] {
            let want = s.english_state.full_width_keypad(key);
            let snap = s.key(key);
            assert_eq!(want, snap.handled, "{key}：{snap:?}");
            s.reset();
        }
        for key in ["1", "KP", "Left", "@"] {
            assert!(!Snapshot::default().full_width_keypad(key));
        }
    }

    #[test]
    fn simplified_hotkey_follows_the_setting() {
        let (mut s, st) = toy_session();
        assert!(!s.simplified_hotkey(), "設定關閉時不攔");
        st.lock().unwrap().info = json!({"simplified_hotkey": "true", "output_simplified_live": "true",
            "output_simplified": "false"});
        assert!(s.simplified_hotkey(), "設定開著");
        assert_eq!(s.simplified(), Some(false));
        let snap = s.output_simplified(true);
        assert!(snap.ok && snap.handled);
    }

    #[test]
    fn caps_lock_english_survives_focus_and_reconnect() {
        let (mut s, st) = toy_session();
        s.learn_mode(true);
        assert!(
            s.mode_known && !s.caps_needs_sync(true),
            "設定關閉時不送 caps_lock"
        );
        st.lock().unwrap().info = json!({"caps_lock_english": "true", "initial_mode": "zhuyin"});
        let mut other = toy_session_with_state(&st);
        other.learn_mode(true);
        assert!(other.mode_known && !other.english());
        assert!(other.caps_needs_sync(true) && !other.caps_needs_sync(false));
        let snap = other.caps_lock(true);
        assert!(snap.ok && snap.english, "暫時英文：{snap:?}");
        assert!(!other.caps_needs_sync(true));
        other.reset();
        assert!(other.english() && !other.caps_needs_sync(true) && other.caps_needs_sync(false));
        let snap = other.caps_lock(false);
        assert!(
            snap.ok && !snap.english && !snap.caps_english,
            "關 Caps Lock 回到注音：{snap:?}"
        );
        let _ = s;
    }

    #[test]
    fn caps_english_stays_temporary_after_engine_restart() {
        let (mut s, st) = toy_session();
        st.lock().unwrap().info = json!({"caps_lock_english": "true", "initial_mode": "english"});
        s.learn_mode(true);
        assert!(s.english(), "開始時英文");
        assert!(!s.toggle().english, "這個程式切到注音");
        let snap = s.caps_lock(true);
        assert!(snap.english);
        s.client.disconnect_for_test();
        let snap = s.key("A");
        assert!(
            snap.ok && snap.english && !snap.handled,
            "重開後仍是暫時的英文：{snap:?}"
        );
        let snap = s.caps_lock(false);
        assert!(
            snap.ok && !snap.english,
            "關掉 Caps Lock 回到這個程式的注音：{snap:?}"
        );
    }

    #[test]
    fn mode_and_mixed_composition_survive_reconnect() {
        let (mut s, _) = toy_session();
        for k in ["s", "u", "3"] {
            s.key(k);
        }
        s.handle(json!({"type": "toggle"}));
        s.key("a");
        s.client.disconnect_for_test();
        let snap = s.key("b");
        assert!(snap.english);
        assert!(snap.reconnected);
        s.key("Enter");
        s.client.disconnect_for_test();
        let snap = s.key("x");
        assert!(snap.english);
        s.reset();
        assert!(s.log.is_empty());
        let snap = s.handle(json!({"type": "mode", "on": false}));
        assert!(snap.text().is_empty());
        assert!(!snap.english);
    }

    #[test]
    fn immediate_toggle_back_restores_pending_zhuyin() {
        let (mut s, _) = toy_session();
        s.key("j");
        let english = s.toggle();
        assert!(
            english.english && !english.composing(),
            "切到英文清掉未打完的注音（#90）"
        );
        let back = s.toggle();
        assert!(!back.english);
        s.client.disconnect_for_test();
        let replayed = s.key("i");
        assert!(!replayed.english, "斷線重播後仍是注音");
        s.toggle();
        s.bypassed();
        let snap = s.toggle();
        assert!(
            !snap.english && !snap.composing(),
            "切換之間有交還宿主的按鍵就不放回"
        );
        s.reset();
    }

    #[test]
    fn reset_discards_pending_composition_even_after_disconnect() {
        let (mut s, _) = toy_session();
        for k in ["s", "u", "3"] {
            s.key(k);
        }
        s.client.disconnect_for_test();
        s.reset();
        let snap = s.key("c");
        assert_eq!(snap.text(), "ㄏ");
        assert!(snap.commit.is_empty());
    }

    #[test]
    fn passes_through_when_not_handled() {
        let (mut s, _) = toy_session();
        let snap = s.key("Enter");
        assert!(snap.ok);
        assert!(!snap.handled);
        assert!(!snap.composing());
    }

    #[test]
    fn replays_after_reconnect() {
        let (mut s, _) = toy_session();
        for k in ["s", "u", "3"] {
            s.key(k);
        }
        assert_eq!(s.log.len(), 3);
        s.client.disconnect_for_test();
        let snap = s.key("c");
        assert!(snap.ok);
        assert!(snap.reconnected);
        assert_eq!(snap.text(), "你ㄏ");
        let _ = s.handle(json!({"type": "cancel"}));
    }

    #[test]
    fn closing_writer_waits_for_replacement_and_commits_once() {
        let (mut s, st) = toy_session();
        st.lock().unwrap().reject_left = 1;
        st.lock().unwrap().handoff_commit = Some("你好".into());
        let snap = s.key("Enter");
        assert!(snap.ok && snap.handled, "handoff must retain the key");
        assert_eq!(snap.commit, "你好", "mock 的 Enter 提交目前組字");
        assert_eq!(
            st.lock().unwrap().keys,
            Vec::<String>::new(),
            "拒絕的請求沒執行；下一輪重播才收到"
        );
    }

    #[test]
    fn idle_letters_during_handoff_go_to_the_engine() {
        let (mut s, st) = toy_session();
        // Go 的 HandoffServer 從第一個請求就拒絕：先設好拒絕與引擎設定，
        // 第一次 preflight 才會同時驗證「回落舊推算」與「記下交接期限」。
        let hsu = json!({"idle_keys": "!\"&'(),.:;<=>?[\\]^_`abcdefghijklmnoprstuvwxyz{}~"});
        st.lock().unwrap().info = hsu.clone();
        st.lock().unwrap().reject_left = 1;
        assert!(s.idle_key("a"), "從沒問到過：字母交給引擎（同 0.2.4）");
        assert!(s.client.restart_pending(), "引擎回了「引擎正在切換」");
        s.idle_info = Some(hsu);
        assert!(s.idle_key("a") && s.idle_key("j"));
        assert!(!s.idle_key("1") && !s.idle_key("q"));
        st.lock().unwrap().keys.clear();
        let snap = s.key("a");
        assert!(snap.ok && snap.handled, "OnKeyDown 等交接結束才送");
        assert_eq!(
            st.lock().unwrap().keys,
            vec!["a".to_string()],
            "新引擎收到這個鍵一次"
        );
    }

    #[test]
    fn closing_writer_has_a_fixed_deadline() {
        let (mut s, st) = toy_session();
        st.lock().unwrap().hang = true;
        let start = std::time::Instant::now();
        assert!(!s.key("Enter").ok);
        assert!(
            start.elapsed() < RESTART_WAIT,
            "3 次重試就放行，不等重啟期限"
        );
        assert_eq!(st.lock().unwrap().keys.len(), 0, "卡住的引擎沒收到按鍵");
    }

    #[test]
    fn keys_typed_while_the_engine_restarts_reach_the_new_engine_in_order() {
        let (mut s, st) = toy_session();
        assert!(s.key("a").ok);
        {
            let mut state = st.lock().unwrap();
            state.connected = false;
            state.down = true;
            state.keys.clear();
        }
        let start = Instant::now();
        assert!(s.idle_key("g"), "交接中的字母交給引擎");
        assert!(
            start.elapsed() < Duration::from_millis(300),
            "TestKeyDown 不等：{:?}",
            start.elapsed()
        );
        assert!(s.client.restart_pending(), "舊引擎關掉連線：等新引擎");
        {
            let mut state = st.lock().unwrap();
            state.down = false;
            state.reject_left = 0;
        }
        s.client
            .set_restart_until_for_test(Instant::now() + Duration::from_millis(20));
        let first = s.key("g");
        assert!(first.ok && first.handled, "第一個字母要等到新引擎");
        assert!(first.reconnected);
        for k in ["4", "s", "u", "3"] {
            assert!(s.key(k).handled, "{k}");
        }
        assert!(!s.client.restart_pending(), "接上新引擎就不再等");
        let keys = st.lock().unwrap().keys.clone();
        assert_eq!(keys.iter().filter(|k| *k == "g").count(), 1, "沒有重複");
        assert!(
            keys.contains(&"4".to_string()) && keys.contains(&"3".to_string()),
            "沒有遺失"
        );
    }

    #[test]
    fn typing_through_a_handover_between_words_loses_nothing() {
        let (mut s, st) = toy_session();
        st.lock().unwrap().commit_keys = true;
        for k in ["s", "u", "3"] {
            assert!(s.key(k).ok, "{k}");
        }
        {
            let mut state = st.lock().unwrap();
            state.connected = false;
            state.down = true;
        }
        {
            let mut state = st.lock().unwrap();
            state.down = false;
            state.keys.clear();
        }
        for k in ["c", "l", "3"] {
            assert!(s.key(k).ok, "{k}");
        }
        let keys = st.lock().unwrap().keys.clone();
        assert_eq!(
            keys,
            vec!["c".to_string(), "l".to_string(), "3".to_string()]
        );
    }

    #[test]
    fn composition_survives_the_restart_and_is_replayed_once() {
        let (mut s, st) = toy_session();
        st.lock().unwrap().commit_keys = true;
        assert!(s.key("a").handled);
        s.log.push(json!({"type": "key", "key": "a"}));
        {
            let mut state = st.lock().unwrap();
            state.connected = false;
            state.down = true;
            state.keys.clear();
        }
        {
            let mut state = st.lock().unwrap();
            state.down = false;
        }
        let snap = s.key("b");
        assert!(snap.handled);
        let a_count = st.lock().unwrap().keys.iter().filter(|k| *k == "a").count();
        assert_eq!(a_count, 1, "新引擎先重播一次組字");
        assert!(st.lock().unwrap().keys.contains(&"b".to_string()));
    }

    #[test]
    fn engine_that_never_returns_releases_within_the_deadline() {
        let (mut s, st) = toy_session();
        assert!(s.key("a").handled);
        {
            let mut state = st.lock().unwrap();
            state.connected = false;
            state.down = true;
        }
        s.client
            .set_restart_until_for_test(Instant::now() + Duration::from_millis(30));
        let start = Instant::now();
        assert!(!s.key("g").ok);
        let waited = start.elapsed();
        assert!(
            waited >= Duration::from_millis(10) && waited < Duration::from_millis(500),
            "等到期限為止：{waited:?}"
        );
        let start = Instant::now();
        assert!(!s.key("4").ok);
        assert!(!s.available());
        assert!(
            start.elapsed() < Duration::from_millis(300),
            "之後不再等：{:?}",
            start.elapsed()
        );
        assert_eq!(st.lock().unwrap().keys, vec!["a".to_string()]);
    }

    #[test]
    fn a_hung_engine_is_not_a_restart() {
        let (mut s, st) = toy_session();
        assert!(s.key("a").handled);
        st.lock().unwrap().hang = true;
        let start = Instant::now();
        assert!(!s.key("g").ok);
        assert!(
            start.elapsed() < RESTART_WAIT,
            "逾時不記成重啟：{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn vocab_keys_follow_the_open_reply() {
        let (mut s, st) = toy_session();
        st.lock().unwrap().vocab_keys = Some("true".into());
        assert!(!s.vocab_keys(), "還沒開 session 不能送新鍵名");
        let first = s.key("j");
        assert!(first.ok && s.vocab_keys());
        s.key("ShiftLeft");
        assert_eq!(
            st.lock().unwrap().keys,
            vec!["j".to_string(), "ShiftLeft".to_string()]
        );
        for v in ["false", ""] {
            let (mut s, st) = toy_session();
            if !v.is_empty() {
                st.lock().unwrap().vocab_keys = Some(v.into());
            }
            s.key("j");
            assert!(!s.vocab_keys(), "{v:?}");
            s.key("ShiftLeft");
            s.key("ShiftBackspace");
            assert_eq!(
                st.lock().unwrap().keys,
                vec!["j".to_string(), "Left".to_string(), "Backspace".to_string()]
            );
            assert!(
                s.log.iter().all(|r| r["key"] != "ShiftLeft"),
                "重播紀錄也是原本的鍵"
            );
        }
    }

    #[test]
    fn unrelated_rejections_do_not_wait_for_handoff() {
        let (mut s, st) = toy_session();
        st.lock().unwrap().info = json!({"error": "協定版本不符"});
        let start = std::time::Instant::now();
        assert!(!s.key("Enter").ok);
        assert!(start.elapsed() < Duration::from_millis(500));
        assert_eq!(st.lock().unwrap().keys.len(), 0);
    }

    #[test]
    fn first_style_modifier_after_handoff_uses_replacement_capabilities() {
        let (mut s, st) = toy_session();
        st.lock().unwrap().info =
            json!({"input_style_shift_space": "disabled", "input_style_caps_letter": "platform"});
        assert!(!s.style_shift_space() && !s.style_caps_lower());
        assert!(!s.shift_config().toggles(keys::Side::Left));
        for enabled in [true, false] {
            {
                let mut state = st.lock().unwrap();
                state.connected = false;
                state.down = true;
            }
            {
                let mut state = st.lock().unwrap();
                state.down = false;
                state.info = json!({
                    "input_style_shift_space": if enabled { "fullwidth" } else { "disabled" },
                    "input_style_caps_letter": if enabled { "lower" } else { "platform" },
                    "left_shift": enabled.to_string(),
                });
            }
            s.client
                .set_restart_until_for_test(Instant::now() + Duration::from_millis(20));
            assert_eq!(s.style_shift_space(), enabled, "first space after handoff");
            assert_eq!(s.style_caps_lower(), enabled, "first caps after handoff");
            assert_eq!(
                s.shift_config().toggles(keys::Side::Left),
                enabled,
                "first shift after handoff"
            );
        }
    }

    #[test]
    fn caps_lock_key_checks_the_setting_without_holding_or_repeating() {
        let (mut s, st) = toy_session();
        s.refresh_caps_lock_english();
        assert_eq!(
            st.lock()
                .unwrap()
                .requests
                .iter()
                .filter(|r| r["op"] == "health")
                .count(),
            1
        );
        assert!(
            !s.client.connected(),
            "只按過 Caps Lock、沒打過字的程式不留連線"
        );
        assert!(!s.caps_needs_sync(true), "設定關閉");
        s.refresh_caps_lock_english();
        assert_eq!(
            st.lock()
                .unwrap()
                .requests
                .iter()
                .filter(|r| r["op"] == "health")
                .count(),
            2
        );
        s.key("j");
        let before = st
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| r["op"] == "health")
            .count();
        for _ in 0..3 {
            s.refresh_caps_lock_english();
        }
        assert_eq!(
            st.lock()
                .unwrap()
                .requests
                .iter()
                .filter(|r| r["op"] == "health")
                .count(),
            before,
            "連線還在時沿用 open 的值"
        );
        assert!(s.client.connected());
        {
            let mut state = st.lock().unwrap();
            state.connected = false;
            state.down = true;
        }
        {
            let mut state = st.lock().unwrap();
            state.down = false;
            state.info = json!({"caps_lock_english": "true"});
        }
        s.refresh_caps_lock_english();
        assert!(
            s.caps_needs_sync(true),
            "設定剛打開：這次 Caps Lock 就送 caps_lock"
        );
        let after = st
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| r["op"] == "health")
            .count();
        s.refresh_caps_lock_english();
        assert_eq!(
            st.lock()
                .unwrap()
                .requests
                .iter()
                .filter(|r| r["op"] == "health")
                .count(),
            after,
            "新連線上問到的值沿用"
        );
    }

    #[test]
    fn reopened_session_keeps_caps_english_temporary() {
        let (mut s, st) = toy_session();
        st.lock().unwrap().info = json!({"caps_lock_english": "true", "initial_mode": "zhuyin"});
        s.key("j");
        let caps = s.caps_lock(true);
        assert!(caps.english);
        {
            let mut state = st.lock().unwrap();
            state.connected = false;
            state.down = true;
            state.info = json!({"caps_lock_english": "false", "initial_mode": "zhuyin"});
        }
        {
            let mut state = st.lock().unwrap();
            state.down = false;
        }
        let from = st.lock().unwrap().requests.len();
        let snap = s.key("a");
        assert!(snap.ok && !snap.english, "設定關掉後照注音：{snap:?}");
        let guard = st.lock().unwrap();
        let modes: Vec<&Value> = guard.requests[from..]
            .iter()
            .filter(|r| r["op"] == "mode" || r["op"] == "caps_lock")
            .collect();
        assert!(modes.is_empty(), "設定關掉後照注音開始");
    }
}
