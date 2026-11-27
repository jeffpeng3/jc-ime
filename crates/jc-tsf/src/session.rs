//! 0c `platforms/windows/tsf/src/session.rs`（main 8f443af）的移植。
//!
//! 狀態機、generation、log 重播、3 次重試加 20ms 等待
//! 與快照結構照抄。Adapter 只改兩處：`open_request` 改送 jc `hello`，
//! `parse()` 前先將 jc `Response::State` 轉成 0c snapshot 的 JSON 形狀。
//!
//! T0 的 windows-rs API 差異已套用。三類全域差異在本檔的落點：
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

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use jc_proto::{Response, PROTOCOL_VERSION as JC_PROTOCOL_VERSION};
use serde_json::{json, Value};

#[cfg(not(windows))]
use self::keys::from_protocol as protocol_key;
#[cfg(windows)]
use crate::keys::from_protocol as protocol_key;

/// debug-keypath：0c 的 `zlog` 改接 crate 層，讓按鍵路徑的 log 寫進 tsf.log。
use crate::log::zlog;

/// jc 協定版本（0c 的 open 原本帶 0，這裡改帶 hello 的 3）。
pub const PROTOCOL_VERSION: u32 = JC_PROTOCOL_VERSION;

/// hello 的客戶端識別。
const CLIENT_NAME: &str = "jc-tsf";

/// 內部預算標記：0c 的 `display` 轉成 `snapshot` 後仍要走收尾預算。
///
/// 這個欄位只在本行程的 `Session` → `PipeTransport` 之間使用，`ipc::Client`
/// 送訊框前會拿掉，不會出現在引擎收到的 JSON。
pub(crate) const TEARDOWN_HINT: &str = "_jc_teardown";

/// 引擎重啟時等待新引擎的期限（0c `ipc::RESTART_WAIT`）。
pub const RESTART_WAIT: Duration = Duration::from_secs(5);

/// 從 0c `keys.rs` 搬過來的最小集合；session 移植只需要這幾個判斷。
pub mod keys {
    #[cfg(not(windows))]
    use jc_proto::{Key, KeyCode};
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

    /// 0c 協定字串鍵 → jc 結構化 [`Key`]。
    ///
    /// 與 Windows 專用的 `crate::keys::from_protocol` 同一套規則；非 Windows
    /// 建置保留這一份，讓 `to_jc_request` 也能把 private 旗標帶進 Key。
    #[cfg(not(windows))]
    pub fn from_protocol(key: &str) -> Option<Key> {
        if key == "BackTab" || key == "ShiftTab" {
            return Some(Key::new(KeyCode::ShiftTab, None, false));
        }
        let (base, shift) = match key.strip_prefix("Shift") {
            Some(rest) if !rest.is_empty() => {
                let base = match rest {
                    "Space" => " ",
                    other => other,
                };
                (base, true)
            }
            _ => (key, false),
        };
        if let Some(c) = base.strip_prefix("Ctrl+") {
            if c.chars().count() != 1 {
                return None;
            }
            let ch = c.chars().next()?;
            return Some(Key::new(KeyCode::Char(ch), Some(ch), shift).with_ctrl());
        }
        let code = match base {
            "Enter" => KeyCode::Enter,
            "Escape" => KeyCode::Escape,
            "Backspace" => KeyCode::Backspace,
            "Delete" => KeyCode::Delete,
            "Tab" => KeyCode::Tab,
            "Left" => KeyCode::ArrowLeft,
            "Right" => KeyCode::ArrowRight,
            "Up" => KeyCode::Up,
            "Down" => KeyCode::Down,
            "PageUp" => KeyCode::PageUp,
            "PageDown" => KeyCode::PageDown,
            "Home" => KeyCode::Home,
            "End" => KeyCode::End,
            " " => KeyCode::Space,
            other => {
                if let Some(n) = other.strip_prefix('F') {
                    if let Ok(n) = n.parse::<u8>() {
                        if (1..=24).contains(&n) {
                            return Some(Key::new(KeyCode::F(n), None, shift));
                        }
                    }
                }
                let original = other.strip_prefix("KP").unwrap_or(other);
                if original.chars().count() != 1 {
                    return None;
                }
                let mut ch = original.chars().next()?;
                let upper = ch.is_ascii_uppercase();
                if upper {
                    ch = ch.to_ascii_lowercase();
                }
                if !('!'..='~').contains(&ch) {
                    return None;
                }
                return Some(Key::new(KeyCode::Char(ch), Some(ch), shift || upper));
            }
        };
        Some(Key::new(code, None, shift))
    }

    /// jc 結構化 [`Key`] → 0c 協定字串鍵；測試用的反向轉換。
    #[cfg(not(windows))]
    pub fn to_protocol(key: &Key) -> Option<String> {
        let name = match key.code {
            KeyCode::Enter => "Enter",
            KeyCode::Escape => "Escape",
            KeyCode::Backspace if key.shift => "ShiftBackspace",
            KeyCode::Backspace => "Backspace",
            KeyCode::Delete if key.shift => "ShiftDelete",
            KeyCode::Delete => "Delete",
            KeyCode::Convert => "Convert",
            KeyCode::NonConvert => "NonConvert",
            KeyCode::Tab | KeyCode::ShiftTab => "BackTab",
            KeyCode::ArrowLeft if key.shift => "ShiftLeft",
            KeyCode::ArrowRight if key.shift => "ShiftRight",
            KeyCode::ArrowLeft => "Left",
            KeyCode::ArrowRight => "Right",
            KeyCode::Up => "Up",
            KeyCode::Down => "Down",
            KeyCode::Home => "Home",
            KeyCode::End => "End",
            KeyCode::PageUp => "PageUp",
            KeyCode::PageDown => "PageDown",
            KeyCode::Space if key.shift => "ShiftSpace",
            KeyCode::Space => " ",
            KeyCode::F(n) if (1..=24).contains(&n) => return Some(format!("F{n}")),
            KeyCode::Char(ch) => {
                let mut out = String::new();
                if key.ctrl {
                    out.push_str("Ctrl+");
                }
                if key.shift && ch.is_ascii_alphabetic() {
                    out.push(ch.to_ascii_uppercase());
                } else {
                    out.push(ch);
                }
                return Some(out);
            }
            _ => return None,
        };
        Some(name.to_string())
    }
}

/// 一個組字片段。jc 的 `SegmentInfo` 只有位移與 mode，沒有樣式文字。
#[derive(Clone, Debug, Default)]
pub struct Segment {
    pub text: String,
    pub style: String,
}

/// 開啟中的符號／表情選單：種類、標題與分頁列。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuState {
    /// `symbols` 或 `emoji`。
    pub kind: String,
    /// 分頁列上方的標題；可空。
    pub title: String,
    /// 分頁名稱，順序即顯示順序。
    pub tabs: Vec<String>,
    /// 目前分頁索引，從 0 起算。
    pub tab: usize,
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
    /// 核心給的顯示附註；舊引擎由 `details[i].reading` 補。
    pub candidate_notes: Vec<String>,
    /// 核心給的選字鍵標籤；舊引擎沒有時依位置補 `1`–`0`。
    pub candidate_keys: Vec<String>,
    /// 候選窗一頁幾列；舊引擎沒有時預設十列。
    pub rows: usize,
    /// 候選窗一列幾項；舊引擎沒有時預設單欄。
    pub columns: usize,
    /// jc 尚無對應；空字串。
    pub candidate_title: String,
    /// jc 尚無對應；0。
    pub page: i64,
    /// jc 尚無對應；0。
    pub pages: i64,
    /// jc `State::highlighted`。
    pub highlight: i64,
    /// jc `State::navigating`：使用者已用方向鍵／翻頁鍵進入選取。
    ///
    /// 候選窗只在這個狀態顯示（新注音的習慣：打字時看組字，按方向鍵才開清單）。
    pub navigating: bool,
    /// 外殼的英文模式。jc wire 沒有整體模式；由外殼保存並在每個回覆補回。
    pub english: bool,
    /// jc 尚無對應；false。
    pub full_width: bool,
    /// jc 尚無對應；false。
    pub caps_lock: bool,
    /// jc 尚無對應；false。
    pub caps_english: bool,
    /// 由 session 的 generation 變化計算，不來自 jc。
    pub reconnected: bool,
    /// jc 尚無對應；空陣列。
    pub notices: Vec<String>,
    /// jc 尚無對應；空字串。
    pub reading_hint: String,
    /// jc 尚無對應；false。
    pub output_simplified: bool,
    /// jc 尚無對應；false。
    pub bell: bool,
    /// jc 尚無對應；false。
    pub converting: bool,
    /// jc 尚無對應；0。
    pub clause_index: i64,
    /// jc 尚無對應；0。
    pub clause_count: i64,
    /// jc `State::menu`：開啟中的符號／表情選單；沒有開選單時為 `None`。
    pub menu: Option<MenuState>,
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

    /// 是否正在組字。
    pub fn composing(&self) -> bool {
        !self.preedit.is_empty() || !self.candidates.is_empty() || self.menu.is_some()
    }

    /// 是否要顯示候選窗。
    ///
    /// 選單開著時一律顯示，不受 `navigating` 影響；其他情況沿用「組字中且
    /// 使用者已進入選取」的規則。內容是否足夠交給呼叫端再判斷。
    pub fn wants_candidate_window(&self) -> bool {
        self.menu.is_some() || self.composing() && self.navigating
    }

    /// 符號／表情選單是否開著。
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
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
        navigating,
        rows,
        columns,
        handled,
        menu,
        ..
    } = response
    else {
        return Value::Null;
    };

    let total_runes = preedit.chars().count();
    let mut preedit_segments: Vec<Value> = Vec::new();
    let mut covered_runes = 0usize;
    // 沒有片段時整段 preedit 都是待處理輸入；`covered_runes` 保持 0，游標才
    // 會跟著移到整段之後。
    let whole_preedit = segments.is_empty();
    if whole_preedit {
        if !preedit.is_empty() {
            preedit_segments.push(json!({"text": preedit, "style": "normal"}));
        }
    } else {
        for segment in segments {
            let start = segment.start.min(total_runes);
            let end = segment.end.min(total_runes);
            // 片段之間若有缺口也照樣顯示；`segments` 的契約是涵蓋整個 preedit，
            // 但舊引擎／舊路徑可能留下沒標到的字。
            if start > covered_runes {
                preedit_segments.push(json!({
                    "text": slice_runes(preedit, covered_runes, start),
                    "style": "normal",
                }));
            }
            let from = start.max(covered_runes);
            if end > from {
                preedit_segments.push(json!({
                    "text": slice_runes(preedit, from, end),
                    "style": if segment.mode == "japanese" { "clause" } else { "normal" },
                }));
            }
            covered_runes = covered_runes.max(end);
        }
    }
    // 日文逐鍵預覽的尾巴是尚未完成的羅馬字，引擎只放在 `preedit`，不在
    // `segments`。這裡若不補回來，外殼每鍵只看到上一次的かな，實機就像
    // 「打完才顯示」。尾巴一律接在最後，游標也跟著移到尾巴之後。
    let tail_runes = total_runes.saturating_sub(covered_runes);
    if tail_runes > 0 && !whole_preedit {
        preedit_segments.push(json!({
            "text": slice_runes(preedit, covered_runes, total_runes),
            "style": "normal",
        }));
    }
    let cursor_rune = if *cursor_rune >= covered_runes {
        (*cursor_rune + tail_runes).min(total_runes)
    } else {
        (*cursor_rune).min(total_runes)
    };

    let (candidate_start, candidate_length) = if segments.is_empty() {
        (0, 0)
    } else {
        let start = segments[0].start;
        let end = segments.last().map(|s| s.end).unwrap_or(start);
        (start, end.saturating_sub(start))
    };

    let pending = suggestion.is_some();
    // 引擎可以明確說「這次沒處理」（例如候選 index 越界）；有值時尊重它。
    // 組字中或選單開著時的 Backspace／Delete，即使把 preedit 或候選刪空，
    // 引擎也會明確回 `handled = true`；這裡不能改用 preedit 猜，否則那一鍵
    // 會被放行給應用程式。只有舊引擎沒帶 `handled` 時才退回內容推斷。
    let handled = handled.unwrap_or(!preedit.is_empty() || !candidates.is_empty() || pending);

    let items: Vec<Value> = candidates
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let detail = details.get(i);
            let note = detail
                .and_then(|d| d.note.clone())
                .or_else(|| detail.map(|d| d.reading.clone()))
                .unwrap_or_default();
            let key = detail
                .and_then(|d| d.key.clone())
                .unwrap_or_else(|| jc_proto::candidate::selection_key(i));
            json!({"id": i as i64, "text": text, "note": note, "key": key})
        })
        .collect();

    json!({
        "handled": handled,
        "preedit": preedit_segments,
        "cursor": {"rune": cursor_rune, "utf16": utf16_at_rune(preedit, cursor_rune)},
        "suggestion": suggestion.clone().unwrap_or_default(),
        "candidates": {
            "start": candidate_start,
            "length": candidate_length,
            "highlight": highlighted,
            "rows": rows,
            "columns": columns,
            "items": items,
        },
        "navigating": navigating,
        "menu": menu.as_ref().map(|menu| json!({
            "kind": menu.kind,
            "title": menu.title,
            "tabs": menu.tabs,
            "tab": menu.tab,
        })),
    })
}

/// jc `Response` → 0c reply Value。轉換在 `parse()` 之前完成。
pub fn response_to_reply(response: &Response) -> Value {
    match response {
        // 引擎配置的 id 原樣帶到 0c reply；`Session::ensure_open()` 會記住它，
        // 之後 reset／Drop 的 Close 就送這個 id，而不是寫死 1。舊引擎的 Ready
        // 沒有 session 欄位時，jc-proto 反序列化會補 1，行為與從前一致。
        Response::Ready {
            engine,
            session,
            bindings,
            ..
        } => json!({
            "ok": true, "session": session, "engine": engine, "info": {}, "bindings": bindings,
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
            "ok": true, "info": info.as_info(), "bindings": info.bindings,
            "settings_revision": info.settings_revision,
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
            "key" => {
                // 0c 的 key 是字串鍵名，jc 的 `Key` 是 struct（code／text／shift）。
                // **一定**要轉：送 {"op":"key","key":"l"} 會讓引擎反序列化失敗，
                // pipe listener 直接把連線關掉，外殼之後每次寫入都失敗——實機
                // tsf.log 的「call: 寫入失敗，斷線」就是這樣來的。
                let raw = &envelope["req"]["key"];
                let private = envelope["req"]["private"].as_bool() == Some(true);
                let caps = envelope["req"]["caps"].as_bool() == Some(true);
                let shift = envelope["req"]["shift"].as_bool() == Some(true);
                if let Some(name) = raw.as_str() {
                    if let Some(mut key) = protocol_key(name) {
                        key.private = private;
                        key.caps = caps;
                        key.shift |= shift;
                        return json!({"op": "key", "key": key});
                    }
                }
                let mut key = raw.clone();
                if private && key.is_object() {
                    key["private"] = Value::Bool(true);
                }
                if caps && key.is_object() {
                    key["caps"] = Value::Bool(true);
                }
                if shift && key.is_object() {
                    key["shift"] = Value::Bool(true);
                }
                json!({"op": "key", "key": key})
            }
            "literal" => json!({
                "op": "literal",
                "text": envelope["req"]["text"].as_str().unwrap_or(""),
            }),
            "commit" => json!({"op": "commit"}),
            // 0c 的 display 是「給我目前的顯示狀態」。jc 沒有這個 op，送過去會回
            // UnknownOp；那個請求又進了重播紀錄，之後每次 ensure_open 重播都會
            // 失敗，整個 session 就再也開不起來（實機：每個鍵都「server 不可用」）。
            "display" => json!({"op": "snapshot"}),
            // 停頓觸發的ライブ変換：引擎沒有日文組字時回目前快照，是便宜的 no-op。
            "idle" => json!({
                "op": "idle",
                "session": envelope["session"].as_u64().unwrap_or(0),
            }),
            // 候選窗滑鼠點選：jc 有真正的 select op，index 是候選的絕對索引。
            // 舊路徑靠 fallback 會送出協定不認得的 op，引擎回 UnknownOp。
            "select" => json!({
                "op": "select",
                "index": envelope["req"]["index"].as_u64().unwrap_or(0),
                "session": envelope["session"].as_u64().unwrap_or(0),
            }),
            // 0c 的 cancel 是「宿主終止了組字，丟掉它」。jc 沒有 cancel op，
            // 對應的動作是按 Escape：引擎清掉組字。送原樣會回 UnknownOp，
            // 而且每次宿主終止組字都會重試三次（實機 log 會連三行「未知的操作」）。
            "cancel" => match protocol_key("Escape") {
                Some(key) => json!({"op": "key", "key": key}),
                None => json!({"op": "snapshot"}),
            },
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
    /// 只連已在執行的對端，**不啟動、不等待**（Activate 的探測用）。
    ///
    /// 預設等於 [`Transport::connect`]；正式管道會覆寫成不啟動引擎的版本，
    /// 否则每個載入 TIP 的程式（連 explorer 也是）都會在 Activate 時叫起引擎。
    fn connect_existing(&mut self) -> bool {
        self.connect()
    }
    /// 關閉連線。
    fn close(&mut self);
    /// 不送請求，只看對端是否還活著。
    fn alive(&mut self) -> bool;
    /// 上一次 `request` 失敗是不是對端關閉（引擎重啟），而不是單次逾時／I/O 失敗。
    ///
    /// 逾時只丟棄重播紀錄，重連後用 snapshot 對齊；對端關閉才保留重播。
    /// 沒有連線資訊的 transport 沿用舊行為，當成重啟。
    fn failed_peer_closed(&self) -> bool {
        true
    }
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
    /// 上一次請求是逾時／I/O 失敗（對端還活著），不是引擎重啟。
    transport_failure: bool,
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
            transport_failure: false,
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

    /// 上一次請求失敗是逾時／I/O 失敗（對端還活著），不是重啟。
    pub fn take_transport_failure(&mut self) -> bool {
        std::mem::take(&mut self.transport_failure)
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

    /// 只連已經在執行的引擎，不啟動、不等待。
    pub fn connect_existing(&mut self) -> bool {
        if self.closing_writer && self.restart_pending() {
            return false;
        }
        self.transport.connect_existing()
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
        let mut request = to_jc_request(envelope);
        // 0c 的 display 在協定上就是 snapshot；加上內部標記，讓 IPC 用 1 秒收尾
        // 預算，而不是互動 snapshot 的 10 秒。
        if envelope["op"].as_str() == Some("handle")
            && envelope["req"]["type"].as_str() == Some("display")
        {
            request[TEARDOWN_HINT] = Value::Bool(true);
        }
        let Some(raw) = self.transport.request(&request) else {
            if self.transport.failed_peer_closed() {
                self.expect_restart();
            } else {
                // 引擎還活著、只是這次沒在預算內回：不要把重播紀錄帶到下一個 session。
                self.transport_failure = true;
            }
            self.close();
            return None;
        };
        self.transport_failure = false;
        let reply = match serde_json::from_value::<Response>(raw.clone()) {
            Ok(response) => response_to_reply(&response),
            Err(_) => raw, // T3 之前的 health／close 等擴充 op
        };
        // health 帶回慢請求時留一行診斷；沒有慢請求的 health 不寫，避免洗版。
        if envelope["op"].as_str() == Some("health") {
            let info = &reply["info"];
            if let Some(op) = info["last_slow_op"].as_str() {
                let number = |value: &Value| {
                    value
                        .as_str()
                        .map(str::to_string)
                        .or_else(|| value.as_u64().map(|n| n.to_string()))
                        .unwrap_or_else(|| "?".to_string())
                };
                zlog!(
                    "health: last_slow_op={} last_slow_ms={} last_slow_input_len={}",
                    op,
                    number(&info["last_slow_ms"]),
                    number(&info["last_slow_input_len"])
                );
            }
        }
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

/// health 回報的新 session 開始時的模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Chinese,
    English,
}

fn initial_mode(info: &Value) -> Option<Mode> {
    match info["initial_mode"].as_str()? {
        "english" => Some(Mode::English),
        "zhuyin" => Some(Mode::Chinese),
        // `japanese` 不再是外殼的輸入模式：日文由 core 依片段推斷，
        // 能力只看 health 的 capability。
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

/// 0c 的 `parse()`：吃 0c reply 的 `snapshot` 物件，原樣移植。
fn parse(reply: &Value) -> Snapshot {
    let mut s = Snapshot::default();
    let Some(j) = reply.get("snapshot") else {
        return s;
    };
    s.ok = true;
    s.handled = j["handled"].as_bool().unwrap_or(false);
    s.commit = j["commit"].as_str().unwrap_or("").to_string();
    s.suggestion = j["suggestion"].as_str().unwrap_or("").to_string();
    s.converting = j["converting"].as_bool().unwrap_or(false);
    s.clause_index = j["clause_index"].as_i64().unwrap_or(0);
    s.clause_count = j["clause_count"].as_i64().unwrap_or(0);
    s.full_width = j["full_width"].as_bool().unwrap_or(false);
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
    // `navigating` 是 snapshot 的兄弟欄位，不在 candidates 物件裡。
    s.navigating = j["navigating"].as_bool().unwrap_or(false);
    if let Some(menu) = j.get("menu").filter(|menu| !menu.is_null()) {
        s.menu = Some(MenuState {
            kind: menu["kind"].as_str().unwrap_or("").to_string(),
            title: menu["title"].as_str().unwrap_or("").to_string(),
            tabs: menu["tabs"]
                .as_array()
                .map(|tabs| {
                    tabs.iter()
                        .filter_map(|tab| tab.as_str())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            tab: menu["tab"].as_u64().unwrap_or(0) as usize,
        });
    }
    if let Some(c) = j.get("candidates") {
        s.rows = match c["rows"].as_u64().unwrap_or(0) {
            0 => jc_proto::candidate_layout::DEFAULT_ROWS,
            rows => (rows as usize).min(jc_proto::candidate_layout::DEFAULT_ROWS),
        };
        s.columns = match c["columns"].as_u64().unwrap_or(0) {
            0 => jc_proto::candidate_layout::VERTICAL_COLUMNS,
            columns => (columns as usize).clamp(
                jc_proto::candidate_layout::VERTICAL_COLUMNS,
                jc_proto::candidate_layout::MAX_COLUMNS,
            ),
        };
        s.candidate_title = c["title"].as_str().unwrap_or("").to_string();
        s.candidate_start = c["start"].as_u64().unwrap_or(0) as usize;
        s.candidate_length = c["length"].as_u64().unwrap_or(0) as usize;
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
                    None => jc_proto::candidate::selection_key(index),
                });
            }
        }
        let per_page = (s.rows * s.columns).max(1);
        s.page = (s.highlight.max(0) as usize / per_page) as i64;
        s.pages = s.candidates.len().div_ceil(per_page) as i64;
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
    pub english_state: Snapshot,
    down: bool,
    log: Vec<Value>,
    /// 傳輸失敗後，下一次連上先送 display 對齊引擎目前的組字，不重播按鍵。
    align_pending: bool,
    private: bool,
    vocab_keys: bool,
    mode_known: bool,
    shared: bool,
    caps_lock_english: bool,
    caps_generation: u32,
    caps_english: bool,
    japanese_cap: bool,
    convert_keys_cap: bool,
    clause_display_cap: bool,
    simplified_live: bool,
    simplified: bool,
    simplified_at: Option<Instant>,
    idle_info: Option<Value>,
    idle_generation: u32,
    /// 引擎回報的 action → 正規化 chord；只在本地比對，命中才送按鍵。
    bindings: BTreeMap<String, String>,
    /// 上次學到的 `settings_revision`。
    bindings_revision: u64,
    /// 上次向引擎更新 bindings 的時間；最多 5 秒一次。
    bindings_at: Option<Instant>,
    /// health／open 回報的中英切換鍵設定。Shift 放開時只讀快取，不產生 IPC。
    shift_config: keys::ShiftConfig,
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

    /// 測試：直接設定 chord 綁定，不經 health。
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn set_bindings_for_test(
        &mut self,
        bindings: impl IntoIterator<Item = (String, String)>,
    ) {
        self.bindings = bindings.into_iter().collect();
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
            english_state: Snapshot::default(),
            down: false,
            log: Vec::new(),
            align_pending: false,
            private: false,
            vocab_keys: false,
            mode_known: false,
            shared: false,
            caps_lock_english: false,
            caps_generation: 0,
            caps_english: false,
            japanese_cap: false,
            convert_keys_cap: false,
            clause_display_cap: false,
            simplified_live: false,
            simplified: false,
            simplified_at: None,
            idle_info: None,
            idle_generation: 0,
            bindings: BTreeMap::new(),
            bindings_revision: 0,
            bindings_at: None,
            shift_config: keys::ShiftConfig::default(),
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
        self.caps_generation = generation;
        // 引擎說沒有 Caps Lock 英文能力時，暫時英文要退回程式模式，
        // 不讓重連後停在英文。
        if !self.caps_lock_english {
            self.caps_english = false;
            self.english = self.base_english;
        }
        self.learn_shift_config(info);
    }

    /// 從 health／open 的 info 記下中英切換鍵設定。
    ///
    /// `Ready` 的 info 是空物件；不要因此把已學到的設定洗回預設值。
    fn learn_shift_config(&mut self, info: &Value) {
        if info.as_object().is_none_or(|info| info.is_empty()) {
            return;
        }
        self.shift_config = keys::ShiftConfig::from_info(Some(info));
        // 同一份 health／open 也帶輸入風格；先快取起來，後續 idle_key
        // 不必為了一個設定再送 IPC。
        self.idle_info = Some(info.clone());
        self.idle_generation = self.client.generation();
    }

    /// 從 Ready／Health 的回應學到 bindings 與設定版次。
    fn learn_bindings(&mut self, reply: &Value) {
        if let Some(map) = reply["bindings"].as_object() {
            self.bindings = map
                .iter()
                .filter_map(|(action, chord)| {
                    chord
                        .as_str()
                        .map(|chord| (action.clone(), chord.to_string()))
                })
                .collect();
        }
        self.bindings_revision = reply["settings_revision"]
            .as_u64()
            .or_else(|| {
                reply["info"]["settings_revision"]
                    .as_str()
                    .and_then(|value| value.parse().ok())
            })
            .unwrap_or(self.bindings_revision);
    }

    /// 這一鍵的 chord 是否綁了 action；只查本地表，不產生 IPC。
    pub fn binding_for(&self, chord: &str) -> Option<&str> {
        self.bindings
            .iter()
            .find(|(_, bound)| bound.as_str() == chord)
            .map(|(action, _)| action.as_str())
    }

    /// 主動向引擎更新 bindings；連不上時保留上一次的表。
    ///
    /// `settings_revision` 沒變就不換表，避免每次焦點或輪詢都重建。
    pub fn refresh_bindings(&mut self) -> bool {
        if !self.client.connected() {
            return false;
        }
        let Some(reply) = self.client.call(&json!({"op": "health"})) else {
            return false;
        };
        if reply["ok"] != true {
            return false;
        }
        self.learn_shift_config(&reply["info"]);
        self.bindings_at = Some(Instant::now());
        let revision = reply["settings_revision"]
            .as_u64()
            .or_else(|| {
                reply["info"]["settings_revision"]
                    .as_str()
                    .and_then(|value| value.parse().ok())
            })
            .unwrap_or(0);
        if revision == self.bindings_revision && !self.bindings.is_empty() {
            return false;
        }
        let before = self.bindings.clone();
        self.learn_bindings(&reply);
        before != self.bindings
    }

    /// 每 5 秒最多更新一次 bindings；按鍵路徑用。
    pub fn maybe_refresh_bindings(&mut self) {
        // 英文模式是純 raw：按鍵路徑不得為了更新 bindings 產生 IPC。
        // Activate／OnSetFocus 仍會主動更新一次。
        if self.english {
            return;
        }
        if self
            .bindings_at
            .is_some_and(|at| at.elapsed() < Duration::from_secs(5))
        {
            return;
        }
        self.bindings_at = Some(Instant::now());
        let _ = self.refresh_bindings();
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
            Some(reply) => {
                self.learn_shift_config(&reply["info"]);
                self.learn_simplified(&reply["info"])
            }
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
        let before = self.english;
        if let Some(mode) = initial_mode(&reply["info"]) {
            self.english = mode == Mode::English;
            self.base_english = self.english;
            self.caps_english = false;
        }
        let changed = before != self.english || simplified;
        zlog!(
            "learn_mode 結束 changed={} mode_known={} english={}",
            changed,
            self.mode_known,
            self.english
        );
        changed
    }

    /// 測試：設定日文能力（不經引擎）。
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn set_japanese_for_test(&mut self, convert_keys: bool) {
        self.japanese_cap = true;
        self.convert_keys_cap = convert_keys;
        self.clause_display_cap = convert_keys;
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

    /// 目前的 session 是否可以送 #123 的新鍵名。
    pub fn vocab_keys(&self) -> bool {
        self.vocab_keys && self.id != 0 && self.generation == self.client.generation()
    }

    /// 每次按鍵前由外殼依欄位政策設定。
    pub fn set_private(&mut self, private: bool) {
        self.private = private;
    }

    /// 目前是否英文模式。
    pub fn english(&self) -> bool {
        self.english
    }

    /// 按鍵當下的 Caps Lock 鎖定狀態要不要更新外殼的暫時英文模式。
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

    /// 回報 Caps Lock 狀態：能力開啟時只切換外殼的暫時英文模式，不送引擎。
    ///
    /// 引擎不再處理 Caps Lock 請求；`caps_lock_english` 由 health／open 學到，
    /// 之後的模式切換純粹發生在外殼，重連時沿用 `english`／`caps_english`。
    pub fn caps_lock(&mut self, on: bool) -> Snapshot {
        if !self.caps_lock_english {
            return self.english_state.clone();
        }
        self.mode_known = true;
        self.caps_english = on && !self.base_english;
        self.english = on || self.base_english;
        let mut snapshot = self.english_state.clone();
        snapshot.ok = true;
        snapshot.handled = true;
        snapshot.english = self.english;
        snapshot.caps_english = self.caps_english;
        snapshot.caps_lock = self.caps_english;
        // 上一次快照可能帶著已送出的 commit；模式切換不能再送一次。
        snapshot.commit.clear();
        snapshot.notices.clear();
        snapshot.bell = false;
        self.english_state = snapshot.clone();
        snapshot
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
    ///
    /// 只讀 `health`／`open` 學到的快取。Shift 放開時會走這裡，不能在按鍵回呼
    /// 裡送 health：引擎慢或暫時連不上時，一次探測就會讓整個 session 進入
    /// 重啟等待，使用者只按了 Shift 卻看到「server 不可用」。
    pub fn shift_config(&self) -> keys::ShiftConfig {
        self.shift_config
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
                    self.learn_shift_config(&reply["info"]);
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
        self.learn_bindings(&reply);
        let _ = self.learn_simplified(&reply["info"]);
        let start_english = reply["info"]["start_mode"] == "english";
        // 開機直接英文只套用在還沒學過模式的第一次開 session；重連後保留
        // 使用者自己切換的狀態。
        if start_english && !self.mode_known {
            self.english = true;
            self.base_english = true;
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
        if self.align_pending {
            // 上次是逾時而不是重啟：引擎可能已經處理過那個請求，重播會重複輸入。
            // 只問目前組字，讓外殼狀態與引擎一致；對不上時下一鍵的 snapshot 會收掉組字。
            let reply = self
                .client
                .call(&json!({"op": "handle", "session": self.id, "req": {"type": "display"}}));
            let Some(reply) = reply else {
                self.id = 0;
                return false;
            };
            if reply["ok"].as_bool().unwrap_or(false) {
                let snapshot = parse(&reply);
                if snapshot.ok {
                    zlog!(
                        "對齊：重連後 snapshot preedit={} candidates={}",
                        snapshot.text(),
                        snapshot.candidates.len()
                    );
                    self.english_state = snapshot;
                }
            }
            self.align_pending = false;
        }
        true
    }

    /// 傳輸失敗（引擎還活著）：丟棄重播紀錄，下一次連上改用 snapshot 對齊。
    fn note_transport_failure(&mut self) -> bool {
        if !self.client.take_transport_failure() {
            return false;
        }
        if !self.log.is_empty() {
            zlog!(
                "傳輸失敗：丟棄 {} 筆重播紀錄，改用 snapshot 對齊",
                self.log.len()
            );
        }
        self.log.clear();
        self.align_pending = true;
        true
    }

    /// 送出一個 proto.Request。連線中斷時自動重連、重開 session 並重播。
    pub fn handle(&mut self, request: Value) -> Snapshot {
        self.handle_retrying(request, true)
    }

    /// 送出一個請求；`retry` 為 `false` 時失敗只放棄這一次。
    ///
    /// 停頓計時器走不重試的路徑：計時器在 UI 執行緒上跑，不能因為引擎沒回應
    /// 就連續重試或等重啟期限。下一次按鍵仍會走一般路徑重連。
    fn handle_retrying(&mut self, mut request: Value, retry: bool) -> Snapshot {
        if self.private {
            request["private"] = Value::Bool(true);
        }
        let generation_before = self.client.generation();
        let kind = request["type"].as_str().unwrap_or("").to_string();
        // jc 的 State 只有組字片段帶模式，Committed 完全不帶；0c 快照的
        // mode／caps_english 得由外殼自己記住，成功後補回快照。
        #[cfg(test)]
        self.sent.push(kind.clone());
        let mut attempts = 0;
        let mut wait_until: Option<Instant> = None;
        loop {
            attempts += 1;
            if attempts > 3 {
                if !retry {
                    break;
                }
                wait_until = wait_until.or(self.client.restart_deadline());
                if !wait_until.is_some_and(|until| Instant::now() < until) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let connected = if retry {
                self.client.connect()
            } else {
                // 計時器不主動啟動引擎，也不等重啟。
                self.client.connect_existing()
            };
            if !connected || !self.ensure_open() {
                // 引擎還活著只是這次沒回：不要在同一個按鍵上重試到整體期限，
                // 直接放行，下一次按鍵重連後用 snapshot 對齊。
                if self.note_transport_failure() || !retry {
                    break;
                }
                continue;
            }
            // 重開 session 可能剛學到新的能力（例如 Caps Lock 英文被關掉），
            // 按鍵請求要用重連後的模式，而不是進迴圈前的快照。
            let base = self.base_english;
            let want_english = self.english;
            let want_caps = self.caps_english;
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
                if self.note_transport_failure() || !retry {
                    break;
                }
                continue;
            };
            if !reply["ok"].as_bool().unwrap_or(false) {
                zlog!(
                    "handle op={} 引擎回 ok=false error={}",
                    kind,
                    reply["error"].as_str().unwrap_or("?")
                );
                self.id = 0;
                if !retry {
                    break;
                }
                continue;
            }
            let mut s = parse(&reply);
            s.reconnected = generation_before != self.client.generation();
            // 引擎的片段模式只描述組字語言，不是外殼的中英模式。外殼模式
            // 一律由自己保存；片段語言留在 `preedit` 樣式與 suggestion 來源。
            s.english = want_english;
            s.caps_english = want_caps;
            s.caps_lock = want_caps;
            s.output_simplified = self.simplified;
            if kind == "output_simplified" {
                s.handled = true;
            }
            if !s.commit.is_empty() || kind == "cancel" || !s.composing() {
                self.log.clear();
            } else if kind != "output_simplified" {
                self.log.push(request);
            }
            self.english = s.english;
            self.base_english = base;
            self.caps_english = s.caps_english;
            if self.simplified_live {
                self.simplified = s.output_simplified;
                self.simplified_at = Some(Instant::now());
            }
            self.mode_known = true;
            self.english_state = s.clone();
            self.down = false;
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
        self.align_pending = false;
        self.english_state = Snapshot::default();
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
        self.key_with_flags(key, false, false)
    }

    /// 送一個按鍵與 Caps／Shift 旗標；字串鍵本身保持基礎字元。
    pub fn key_with_flags(&mut self, key: &str, caps: bool, shift: bool) -> Snapshot {
        self.handle(json!({"type": "key", "key": key, "caps": caps, "shift": shift}))
    }

    /// 送出帶修飾鍵的結構化按鍵；快捷鍵命中時使用。
    pub fn key_chord(&mut self, key: &jc_proto::Key) -> Snapshot {
        self.handle(json!({"type": "key", "key": key}))
    }

    /// 把字面文字原樣交給引擎接在目前組字後面。
    pub fn literal(&mut self, text: &str) -> Snapshot {
        self.handle(json!({"type": "literal", "text": text}))
    }

    /// 停頓後請引擎立刻做日文ライブ変換。
    ///
    /// 引擎對非日文組字是 no-op，回目前快照；外殼用同一條 `present` 套用。
    /// 計時器路徑不重試、不主動啟動引擎，連不上就放棄這一次。
    pub fn idle(&mut self) -> Snapshot {
        if !self.client.connected() {
            zlog!("idle 略過：沒有連線");
            return Snapshot::default();
        }
        self.handle_retrying(json!({"type": "idle"}), false)
    }

    /// 為游標所在片段重開候選。
    pub fn candidates(&mut self) -> Snapshot {
        self.handle(json!({"type": "candidates"}))
    }

    /// 單按 Shift 或輸入指示器按鈕切換中英。
    ///
    /// 中英模式由外殼保存，引擎的 `toggle` 只是回目前快照（no-op）。這裡只改
    /// 本機狀態，不送 IPC、不提交組字：Shift 是修飾鍵，不該讓引擎慢或暫時
    /// 連不上時把整個 session 判成掛掉，也不該讓進行中的組字消失。
    pub fn toggle(&mut self) -> Snapshot {
        self.base_english = !self.base_english;
        self.english = self.base_english;
        self.caps_english = false;
        self.mode_known = true;
        let mut snapshot = self.english_state.clone();
        snapshot.ok = true;
        snapshot.handled = true;
        snapshot.english = self.english;
        snapshot.caps_english = false;
        snapshot.caps_lock = false;
        // 上一次快照可能帶著已送出的 commit；切換模式不能再送一次。
        snapshot.commit.clear();
        snapshot.notices.clear();
        snapshot.bell = false;
        snapshot
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
    fn suggestion_source_separates_zhuyin_from_english() {
        let state = |mode: &str, suggestion: Option<&str>| Response::State {
            preedit: "su".into(),
            cursor: 2,
            candidates: Vec::new(),
            details: Vec::new(),
            segments: vec![jc_proto::SegmentInfo {
                start: 0,
                end: 2,
                mode: mode.into(),
                script: "latin".into(),
            }],
            highlighted: 0,
            cursor_rune: 2,
            suggestion: suggestion.map(str::to_string),
            label: None,
            navigating: false,
            rows: 0,
            columns: 0,
            handled: None,
            menu: None,
        };
        // 補全尾綴仍照引擎給的來源顯示；外殼不再自行分類成英文待選字。
        let zhuyin = parse(&response_to_reply(&state("zhuyin", Some("你"))));
        assert_eq!(zhuyin.suggestion, "你");

        let english = parse(&response_to_reply(&state("english", Some("lo"))));
        assert_eq!(english.suggestion, "lo");

        let none = parse(&response_to_reply(&state("english", None)));
        assert!(none.suggestion.is_empty());

        // 片段語言不再變成外殼模式。
        let wire = state_to_snapshot_value(&state("english", Some("lo")));
        assert!(wire.get("mode").is_none(), "{wire}");
        assert!(
            wire.get("suggestion_kind").is_none(),
            "suggestion_kind 已刪除：{wire}"
        );
        assert!(!english.english, "片段語言不是外殼模式");
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
    fn initial_mode_has_no_japanese_input_mode() {
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
            None,
            "japanese 不再是外殼模式"
        );
        assert_eq!(
            initial_mode(&json!({"initial_mode": "japanese"})),
            None,
            "日文只看 capability"
        );
        assert_eq!(
            initial_mode(&json!({"initial_mode": "zhuyin"})),
            Some(Mode::Chinese)
        );
        assert_eq!(initial_mode(&json!({"initial_mode": "future"})), None);
        assert_eq!(initial_english(&json!({"version": "old"})), None);
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
    fn english_candidates_keep_core_keys_and_notes() {
        let response = Response::State {
            preedit: "hel".into(),
            cursor: 3,
            candidates: vec!["hello".into(), "help".into()],
            details: vec![
                jc_proto::CandidateDetail {
                    reading: "hello".into(),
                    source: jc_core_source(),
                    cost: 0,
                    learned: false,
                    pos: None,
                    key: Some("1".into()),
                    note: Some("問候".into()),
                },
                jc_proto::CandidateDetail {
                    reading: "help".into(),
                    source: jc_core_source(),
                    cost: 1,
                    learned: false,
                    pos: None,
                    key: Some("2".into()),
                    note: Some("help".into()),
                },
            ],
            segments: vec![jc_proto::SegmentInfo {
                start: 0,
                end: 3,
                mode: "english".into(),
                script: "latin".into(),
            }],
            highlighted: 0,
            cursor_rune: 3,
            suggestion: None,
            label: None,
            navigating: true,
            rows: 10,
            columns: 1,
            handled: None,
            menu: None,
        };
        let s = parse(&response_to_reply(&response));
        assert_eq!(s.candidate_keys, vec!["1", "2"]);
        assert_eq!(s.candidate_notes, vec!["問候", "help"]);
        assert!(!s.english, "片段語言不再推導成外殼模式");
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
        assert_eq!(english.candidate_keys, vec!["1"], "英文候選也有數字鍵");
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
            "handled": true, "converting": true, "clause_index": 1, "clause_count": 2,
            "preedit": [{"text": "今日", "style": "clause"}, {"text": "は", "style": "clause_active"}]
        }}));
        assert!(!s.english, "片段語言不是外殼模式");
        assert!(s.converting);
        assert_eq!((s.clause_index, s.clause_count), (1, 2));
        assert_eq!(s.clause_title(), "文節 2/2");
        assert_eq!(s.candidate_title(), "文節 2/2");
        // 引擎沒有 capability 時仍保留核心給的文節樣式。
        assert_eq!(s.preedit[0].style, "clause");
        assert_eq!(s.preedit[1].style, "clause_active");
        let old = parse(&json!({"snapshot": {"handled": true}}));
        assert!(!old.converting && old.clause_title().is_empty());
        assert_eq!(old.candidate_title(), "");
    }

    #[test]
    fn clause_title_combines_with_candidate_title_and_guards_bad_indexes() {
        let mut s = parse(&json!({"snapshot": {
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
    fn japanese_capabilities_gate_the_new_keys() {
        let mut s = Session::new();
        s.learn_capabilities(&json!({}), 0);
        assert!(!s.japanese_supported() && !s.convert_keys() && !s.clause_display());
        s.learn_capabilities(
            &json!({"japanese": "true", "convert_keys": "true", "clause_display": "true"}),
            0,
        );
        assert!(s.japanese_supported() && s.convert_keys() && s.clause_display());
        s.set_japanese_for_test(true);
        assert!(s.japanese_supported());
        s.learn_capabilities(&json!({}), 0);
        assert!(!s.japanese_supported() && !s.convert_keys() && !s.clause_display());
        s.learn_capabilities(&json!({"japanese": "true"}), 0);
        s.set_japanese_for_test(false);
        assert!(!s.convert_keys() && !s.clause_display());
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

    // 外殼現在只保存 suggestion 尾綴；英文補全不再有 pending／kind 狀態。
    #[test]
    fn english_suggestion_is_plain_text_without_pending() {
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
                key: None,
                note: None,
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
            rows: 0,
            columns: 0,
            handled: None,
            menu: None,
        };
        let reply = response_to_reply(&state);
        let s = parse(&reply);
        assert!(s.ok);
        assert_eq!(s.suggestion, "lo");
        assert_eq!(s.text(), "hel");
        assert_eq!(s.cursor_utf16, 3);
        assert_eq!(s.candidate_keys, vec!["1"]);

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
            rows: 0,
            columns: 0,
            handled: None,
            menu: None,
        };
        let s = parse(&response_to_reply(&no_suggestion));
        assert!(s.suggestion.is_empty(), "沒有 suggestion 就是空字串");
    }

    #[test]
    fn ready_reply_carries_the_engine_session_id() {
        let ready = Response::Ready {
            protocol: PROTOCOL_VERSION,
            engine: "jc-engine test".into(),
            session: 23,
            bindings: Default::default(),
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

    #[cfg(not(windows))]
    use super::keys::to_protocol as protocol_name;
    #[cfg(windows)]
    use crate::keys::to_protocol as protocol_name;

    /// 沒有 private 的字串鍵名也要轉成結構化 Key。
    ///
    /// 少了這一步，引擎收到 `{"op":"key","key":"l"}` 會反序列化失敗並關掉連線，
    /// 外殼之後每次寫入都失敗；實機症狀就是「每鍵都放行、只打出英文」。
    #[test]
    fn to_jc_request_converts_plain_key_names() {
        let envelope = json!({
            "op": "handle",
            "session": 1,
            "req": {"type": "key", "key": "l"},
        });
        let request = to_jc_request(&envelope);
        assert_eq!(request["op"], "key");
        let parsed: jc_proto::Request = serde_json::from_value(request).unwrap();
        match parsed {
            jc_proto::Request::Key { key } => {
                assert_eq!(key.code, jc_proto::KeyCode::Char('l'));
                assert!(!key.private);
            }
            other => panic!("應是 Key 請求：{other:?}"),
        }
    }

    /// Caps Lock 只帶旗標，字串鍵本身仍是基礎字元；Shift 另外帶。
    #[test]
    fn to_jc_request_carries_caps_and_shift_flags() {
        let envelope = json!({
            "op": "handle",
            "session": 1,
            "req": {"type": "key", "key": "a", "caps": true, "shift": true},
        });
        let request = to_jc_request(&envelope);
        let parsed: jc_proto::Request = serde_json::from_value(request).unwrap();
        match parsed {
            jc_proto::Request::Key { key } => {
                assert_eq!(key.code, jc_proto::KeyCode::Char('a'));
                assert_eq!(key.text, Some('a'));
                assert!(key.caps);
                assert!(key.shift);
            }
            other => panic!("應是 Key 請求：{other:?}"),
        }
    }

    /// 日文逐鍵預覽：引擎把未完成的羅馬字放在 `preedit`，但 `segments` 只涵蓋
    /// 已轉出的かな。外殼若只照 `segments` 切字，畫面每鍵都少一截，實機就變成
    /// 「打完才顯示」。這裡要求尾巴原樣接回，游標也落在尾巴之後。
    #[test]
    fn japanese_pending_romaji_tail_is_kept_in_preedit() {
        let wire = json!({
            "op": "state",
            "preedit": "わt",
            "cursor": 1,
            "candidates": [],
            "segments": [{"start": 0, "end": 1, "mode": "japanese", "script": "kana"}],
            "cursor_rune": 1
        });
        let response: jc_proto::Response = serde_json::from_value(wire).expect("state 要能解析");
        let reply = response_to_reply(&response);
        let snapshot = parse(&reply);
        assert_eq!(snapshot.text(), "わt", "未完成的羅馬字不可被切掉");
        assert_eq!(snapshot.cursor_rune, 2, "游標要落在未完成羅馬字之後");
        assert_eq!(snapshot.cursor_utf16, 2);

        // 第一鍵還沒有任何かな片段：整段都是未完成的羅馬字。
        let wire = json!({
            "op": "state", "preedit": "w", "cursor": 0, "candidates": [],
            "segments": [], "cursor_rune": 0
        });
        let response: jc_proto::Response = serde_json::from_value(wire).expect("state 要能解析");
        let snapshot = parse(&response_to_reply(&response));
        assert_eq!(snapshot.text(), "w");
        assert_eq!(snapshot.cursor_rune, 1);
    }

    /// 0c 的 `display` 要翻成 jc 的 `snapshot`。
    ///
    /// 直接送 `display` 引擎會回 UnknownOp，而這個請求會被記進重播紀錄；之後
    /// `ensure_open` 每次重播都失敗，session 再也開不起來——實機症狀是進英文
    /// 模式後每個鍵都「server 不可用，放行按鍵」。
    /// 0c 的 cancel 要翻成「按 Escape」，引擎才清得掉組字。
    ///
    /// 送原樣會回 UnknownOp，宿主每次終止組字都重試三次。
    /// 引擎的 `state` 要能一路帶到 TIP 的 `Snapshot`：`navigating` 與候選清單。
    ///
    /// 這是候選窗顯示的唯一依據（`present_in_edit` 的 `show` 條件）；
    /// 少了 `navigating` 就永遠不開窗，少了候選就是空窗。
    #[test]
    fn engine_state_reaches_snapshot_navigating_and_candidates() {
        let wire = json!({
            "op": "state",
            "preedit": "台",
            "cursor": 1,
            "candidates": ["台", "抬", "臺"],
            "details": [
                {"reading": "ㄊㄞˊ", "source": "lexicon", "cost": 9469},
                {"reading": "ㄊㄞˊ", "source": "lexicon", "cost": 12112},
                {"reading": "ㄊㄞˊ", "source": "lexicon", "cost": 12133}
            ],
            "segments": [{"start": 0, "end": 1, "mode": "zhuyin", "script": "latin"}],
            "cursor_rune": 1,
            "navigating": true
        });
        let response: jc_proto::Response = serde_json::from_value(wire).expect("state 要能解析");
        let reply = response_to_reply(&response);
        let snapshot = parse(&reply);
        assert!(snapshot.ok, "reply 要是 ok：{reply:?}");
        assert!(snapshot.navigating, "navigating 要帶到 Snapshot");
        assert!(snapshot.composing(), "有 preedit 就算組字中");
        assert_eq!(snapshot.candidates.len(), 3, "候選要帶到 Snapshot");
        assert_eq!(snapshot.candidates[0].1, "台");
    }

    /// 選單狀態要一路帶到 TSF 的 `Snapshot`，而且不靠 `navigating` 就能顯示。
    #[test]
    fn service_display_maps_menu_state_without_navigating() {
        let wire = json!({
            "op": "state",
            "preedit": "",
            "cursor": 0,
            "candidates": ["，", "、"],
            "details": [
                {"reading": "", "source": "literal", "cost": 0},
                {"reading": "", "source": "literal", "cost": 0}
            ],
            "cursor_rune": 0,
            "navigating": false,
            "menu": {
                "kind": "symbols",
                "title": "標點 · Tab → 括號引號",
                "tabs": ["標點", "括號引號", "數學"],
                "tab": 1
            }
        });
        let response: jc_proto::Response = serde_json::from_value(wire).expect("state 要能解析");
        let snapshot = parse(&response_to_reply(&response));
        let menu = snapshot.menu.as_ref().expect("menu 要帶到 Snapshot");
        assert_eq!(menu.kind, "symbols");
        assert_eq!(menu.title, "標點 · Tab → 括號引號");
        assert_eq!(menu.tabs, ["標點", "括號引號", "數學"]);
        assert_eq!(menu.tab, 1);
        assert!(snapshot.composing(), "選單開著就算組字");
        assert!(
            snapshot.wants_candidate_window(),
            "選單不需要 navigating 也顯示"
        );
        assert!(!snapshot.navigating);

        // 沒有 menu 時維持原本的顯示規則。
        let plain = Snapshot {
            candidates: vec![(0, "台".into())],
            ..Snapshot::default()
        };
        assert!(!plain.wants_candidate_window(), "沒 navigating 不開候選窗");
        let empty = Snapshot::default();
        assert!(!empty.wants_candidate_window());
    }

    /// Tab／Shift+Tab 送給引擎的鍵：Shift+Tab 走 `BackTab`，引擎端是
    /// `KeyCode::ShiftTab`，核心的選單分頁切換才看得到方向。
    #[test]
    fn tab_and_backtab_reach_the_engine_as_distinct_keys() {
        for (protocol, expected) in [
            ("Tab", jc_proto::KeyCode::Tab),
            ("BackTab", jc_proto::KeyCode::ShiftTab),
        ] {
            let request =
                to_jc_request(&json!({"op": "handle", "req": {"type": "key", "key": protocol}}));
            let jc_proto::Request::Key { key } = serde_json::from_value(request).expect("Key 請求")
            else {
                panic!("{protocol} 應轉成 Key 請求");
            };
            assert_eq!(key.code, expected, "{protocol}");
        }
    }

    #[test]
    fn to_jc_request_maps_cancel_to_escape() {
        let envelope = json!({"op": "handle", "session": 1, "req": {"type": "cancel"}});
        let request = to_jc_request(&envelope);
        assert_eq!(request["op"], "key");
        let parsed: jc_proto::Request = serde_json::from_value(request).unwrap();
        match parsed {
            jc_proto::Request::Key { key } => assert_eq!(key.code, jc_proto::KeyCode::Escape),
            other => panic!("應是 Escape：{other:?}"),
        }
    }

    #[test]
    fn to_jc_request_maps_idle_to_idle_op() {
        let envelope = json!({
            "op": "handle",
            "session": 4,
            "req": {"type": "idle"},
        });
        let request = to_jc_request(&envelope);
        assert_eq!(request["op"], "idle");
        assert_eq!(request["session"], 4);
        let parsed: jc_proto::Request = serde_json::from_value(request).unwrap();
        assert_eq!(parsed, jc_proto::Request::Idle { session: 4 });
    }

    #[test]
    fn to_jc_request_maps_display_to_snapshot() {
        let envelope = json!({
            "op": "handle",
            "session": 1,
            "req": {"type": "display", "columns": 0},
        });
        let request = to_jc_request(&envelope);
        assert_eq!(request["op"], "snapshot");
        let parsed: jc_proto::Request = serde_json::from_value(request).unwrap();
        assert!(matches!(parsed, jc_proto::Request::Snapshot));
    }

    #[test]
    fn display_requests_are_marked_as_teardown() {
        let (mut s, st) = toy_session();
        assert!(s.handle(json!({"type": "display", "columns": 0})).ok);
        s.key("a");
        let requests = st.lock().unwrap().requests.clone();
        let snapshot = requests
            .iter()
            .rev()
            .find(|request| request["op"] == "snapshot")
            .expect("display 要送 snapshot");
        assert_eq!(snapshot[TEARDOWN_HINT], true, "display 用收尾預算");
        let key = requests
            .iter()
            .rev()
            .find(|request| request["op"] == "key")
            .expect("key 要送 key");
        assert!(key.get(TEARDOWN_HINT).is_none(), "互動按鍵維持 10 秒預算");
    }

    #[test]
    fn to_jc_request_maps_select_to_protocol_select() {
        let envelope = json!({
            "op": "handle",
            "session": 7,
            "req": {"type": "select", "index": 2},
        });
        let request = to_jc_request(&envelope);
        assert_eq!(request["op"], "select");
        assert_eq!(request["index"], 2);
        assert_eq!(request["session"], 7);
        let parsed: jc_proto::Request = serde_json::from_value(request).unwrap();
        assert_eq!(parsed, jc_proto::Request::Select { index: 2 });
    }

    #[test]
    fn select_snapshot_applies_preedit_and_candidates() {
        let (client, st) = mock_client(|request, _st| match request["op"].as_str() {
            Some("hello") => Some(json!({"ok": true, "session": 1, "info": {}})),
            Some("select") => Some(json!(Response::State {
                preedit: "窩".into(),
                cursor: 1,
                candidates: vec!["窩".into(), "我".into()],
                details: Vec::new(),
                segments: vec![jc_proto::SegmentInfo {
                    start: 0,
                    end: 1,
                    mode: "zhuyin".into(),
                    script: "bopomofo".into(),
                }],
                highlighted: 0,
                cursor_rune: 1,
                suggestion: None,
                label: None,
                navigating: true,
                rows: 0,
                columns: 0,
                handled: None,
                menu: None,
            })),
            _ => Some(json!({"ok": true})),
        });
        let mut s = Session::with_client(client);
        let snapshot = s.handle(json!({"type": "select", "index": 1}));
        assert!(snapshot.ok && snapshot.handled);
        assert_eq!(snapshot.text(), "窩");
        assert_eq!(snapshot.candidates.len(), 2);
        assert_eq!(snapshot.candidate_keys, vec!["1", "2"]);

        let requests = st.lock().unwrap().requests.clone();
        let select = requests
            .iter()
            .rev()
            .find(|request| request["op"] == "select")
            .expect("要送出 select");
        assert_eq!(select["index"], 1);
        assert_eq!(select["session"], 1);
    }

    #[test]
    fn select_out_of_range_is_ok_but_not_handled() {
        let (client, _st) = mock_client(|request, _st| match request["op"].as_str() {
            Some("hello") => Some(json!({"ok": true, "session": 1, "info": {}})),
            Some("select") => Some(json!(Response::State {
                preedit: "ㄨㄛ".into(),
                cursor: 2,
                candidates: vec!["我".into()],
                details: Vec::new(),
                segments: Vec::new(),
                highlighted: 0,
                cursor_rune: 2,
                suggestion: None,
                label: None,
                navigating: false,
                rows: 0,
                columns: 0,
                handled: Some(false),
                menu: None,
            })),
            _ => Some(json!({"ok": true})),
        });
        let mut s = Session::with_client(client);
        let snapshot = s.handle(json!({"type": "select", "index": 99}));
        assert!(snapshot.ok, "越界仍是成功的請求");
        assert!(!snapshot.handled, "引擎說沒處理，外殼要照實轉出去");
        assert_eq!(snapshot.text(), "ㄨㄛ", "組字不清掉");
    }

    #[test]
    fn to_jc_request_moves_private_flag_into_key() {
        // 0c 的 key 是字串鍵名；private 為 true 時要轉成 jc Key 才帶得過去。
        let envelope = json!({
            "op": "handle",
            "session": 1,
            "req": {"type": "key", "key": "a", "private": true},
        });
        let request = to_jc_request(&envelope);
        assert_eq!(request["op"], "key");
        assert_eq!(request["key"]["private"], true);
        assert_eq!(request["key"]["text"], "a");
        let parsed: jc_proto::Request = serde_json::from_value(request).unwrap();
        match parsed {
            jc_proto::Request::Key { key } => {
                assert!(key.private);
                assert_eq!(key.code, jc_proto::KeyCode::Char('a'));
            }
            other => panic!("應是 Key 請求：{other:?}"),
        }

        // 已經是結構化 Key 的 envelope 直接補 private。
        let structured = json!({
            "op": "handle",
            "session": 1,
            "req": {
                "type": "key",
                "key": {"code": {"kind": "char", "value": "a"}, "text": "a", "shift": false},
                "private": true,
            },
        });
        let request = to_jc_request(&structured);
        assert_eq!(request["key"]["private"], true);
        assert_eq!(request["key"]["text"], "a");

        // private 為 false 時一樣轉成結構化 Key：引擎的 Key 是 struct，
        // 字串形狀會讓它反序列化失敗並關掉連線。
        let plain = json!({
            "op": "handle",
            "session": 1,
            "req": {"type": "key", "key": "a"},
        });
        let request = to_jc_request(&plain);
        assert_eq!(request["key"]["text"], "a");
        assert!(
            request["key"].get("private").is_none(),
            "沒指定就不寫 private"
        );
    }

    #[test]
    fn ready_bindings_drive_chord_matching_and_key_chord_carries_alt() {
        let (client, st) = mock_client(|request, _st| match request["op"].as_str() {
            Some("hello") => Some(
                serde_json::to_value(Response::Ready {
                    protocol: PROTOCOL_VERSION,
                    engine: "toy".into(),
                    session: 1,
                    bindings: [
                        ("emoji_palette".to_string(), "ctrl+alt+.".to_string()),
                        ("symbol_menu".to_string(), "ctrl+alt+,".to_string()),
                    ]
                    .into_iter()
                    .collect(),
                })
                .unwrap(),
            ),
            Some("key") => Some(json!(Response::State {
                preedit: String::new(),
                cursor: 0,
                candidates: Vec::new(),
                details: Vec::new(),
                segments: Vec::new(),
                highlighted: 0,
                cursor_rune: 0,
                suggestion: None,
                label: None,
                navigating: false,
                rows: 0,
                columns: 0,
                handled: Some(true),
                menu: None,
            })),
            _ => Some(json!({"ok": true})),
        });
        let mut s = Session::with_client(client);
        // 英文模式由外殼保存；快捷鍵比對不依賴中英模式。
        s.english = true;
        let key = jc_proto::Key::char(',', false).with_ctrl().with_alt();
        let snapshot = s.key_chord(&key);
        assert!(snapshot.ok && snapshot.handled);

        assert_eq!(s.binding_for("ctrl+alt+,"), Some("symbol_menu"));
        assert_eq!(s.binding_for("ctrl+alt+."), Some("emoji_palette"));
        assert_eq!(
            s.binding_for("ctrl+alt+x"),
            None,
            "未命中的 chord 不該有 action"
        );

        let requests = st.lock().unwrap().requests.clone();
        let sent = requests
            .iter()
            .rev()
            .find(|request| request["op"] == "key")
            .expect("要送 key");
        assert_eq!(sent["key"]["alt"], true, "chord 要帶 alt");
        assert_eq!(sent["key"]["ctrl"], true);
        assert_eq!(sent["key"]["text"], ",");
    }

    /// 英文模式的 bindings 更新不得產生 IPC；快取留給 Activate／OnSetFocus。
    #[test]
    fn english_mode_never_refreshes_bindings_over_ipc() {
        let (mut s, st) = toy_session();
        s.english = true;
        s.bindings_at = None;
        let before = st.lock().unwrap().requests.len();
        s.maybe_refresh_bindings();
        assert_eq!(
            st.lock().unwrap().requests.len(),
            before,
            "英文模式按鍵路徑不得為了 bindings 送 health"
        );
    }

    #[test]
    fn binding_lookup_is_local_and_does_not_send_requests() {
        let (client, st) = mock_client(|_request, _st| Some(json!({"ok": true})));
        let mut s = Session::with_client(client);
        s.bindings.insert("symbol_menu".into(), "ctrl+alt+,".into());
        s.english = true;
        let before = st.lock().unwrap().requests.len();
        assert_eq!(s.binding_for("ctrl+alt+,"), Some("symbol_menu"));
        assert_eq!(s.binding_for("ctrl+alt+x"), None);
        assert_eq!(
            st.lock().unwrap().requests.len(),
            before,
            "本地比對不得產生按鍵 IPC"
        );
    }

    struct MockState {
        connected: bool,
        down: bool,
        hang: bool,
        /// 下一次請求失敗時模擬「對端還活著、只是慢」的逾時，不是關線。
        timeout: bool,
        /// 接下來幾個請求直接不回應。
        fail_left: usize,
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
                timeout: false,
                fail_left: 0,
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
        fn failed_peer_closed(&self) -> bool {
            let st = self.state.lock().unwrap();
            // hang／timeout 模擬對方還活著只是慢；其餘失敗照舊當成關線。
            !(st.hang || st.timeout)
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
        if st.fail_left > 0 {
            st.fail_left -= 1;
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
                        bindings: Default::default(),
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
                // private 按鍵會被 to_jc_request 轉成結構化 Key；其他維持 0c 字串。
                let key = match request["key"].as_str() {
                    Some(name) => name.to_string(),
                    None => {
                        let name = serde_json::from_value::<jc_proto::Key>(request["key"].clone())
                            .ok()
                            .and_then(|key| protocol_name(&key))
                            .unwrap_or_default();
                        // `to_protocol` 把 Tab 與 ShiftTab 都寫成 "BackTab"（0c 約定）；
                        // TIP 實際送出的是 "Tab"，假引擎要照同一個名字判斷。
                        if name == "BackTab" {
                            "Tab".to_string()
                        } else {
                            name
                        }
                    }
                };
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
            rows: 0,
            columns: 0,
            handled: None,
            menu: None,
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

    /// 重開 session 不再送引擎不支援的 `mode`／`caps_lock` 還原請求。
    #[test]
    fn reopen_sends_no_mode_or_caps_lock_restore() {
        let (mut s, st) = toy_session();
        st.lock().unwrap().start_english = true;
        st.lock().unwrap().info = json!({"caps_lock_english": "true"});
        assert!(s.learn_mode(false) && s.english());
        s.caps_lock(true);
        s.key("t");
        s.client.disconnect_for_test();
        s.key("e");
        let requests = st.lock().unwrap().requests.clone();
        let hello = requests
            .iter()
            .rposition(|request| request["op"] == "hello")
            .expect("重連要送 hello");
        let after = &requests[hello + 1..];
        assert!(
            after
                .iter()
                .all(|request| !matches!(request["type"].as_str(), Some("mode" | "caps_lock"))),
            "重連只重播按鍵，不送不支援的還原請求：{after:?}"
        );
    }

    #[test]
    fn literal_request_becomes_a_jc_literal_op() {
        let request =
            to_jc_request(&json!({"op": "handle", "req": {"type": "literal", "text": "A"}}));
        assert_eq!(request, json!({"op": "literal", "text": "A"}));
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
        assert!(!palette.candidates.is_empty());
        let literal = s.key("=");
        assert_eq!(literal.commit, "=", "第二次 = 提交字面（mock 的 = 沒組字）");
        assert!(!literal.composing());
    }

    #[test]
    fn commit_does_not_send_document_context() {
        let (mut s, st) = toy_session();
        for k in ["s", "u", "3", "c", "l", "3"] {
            s.key(k);
        }
        let committed = s.key("Enter");
        assert_eq!(committed.commit, "你好");
        assert!(
            st.lock()
                .unwrap()
                .requests
                .iter()
                .all(|r| r["type"].as_str() != Some("context")),
            "TSF 不再送 context 請求"
        );
        s.reset();
    }

    #[test]
    fn private_field_composes_without_learning_or_context() {
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
        assert!(snap.ok && snap.english, "第一個按鍵仍標成英文模式");
        let snap = s.toggle();
        assert!(snap.ok && !snap.english);
        s.client.disconnect_for_test();
        let snap = s.key("j");
        assert!(
            !snap.english && snap.handled,
            "重開 session 後仍是這個程式切過的注音模式"
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
            // jc 的 Key 沒有鍵台旗標：`from_protocol` 把 KP1 還原成字元 '1'，
            // 引擎看到的就是「1」。所以全寬鍵台完全由外殼的 `full_width_keypad`
            // 決定——預測不消費時根本不會送出，這裡只驗送出去時引擎接得住。
            assert!(snap.ok, "{key} 送出去後引擎要接得住：{snap:?}");
            assert!(
                !want || snap.handled,
                "{key}：外殼預測要消費時引擎也要處理：{snap:?}"
            );
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
    fn caps_lock_english_switches_locally_without_ipc() {
        let (mut s, st) = toy_session();
        st.lock().unwrap().info = json!({"caps_lock_english": "true", "initial_mode": "zhuyin"});
        assert!(!s.learn_mode(true));
        assert!(s.mode_known);
        let before = st.lock().unwrap().requests.len();
        assert!(s.caps_needs_sync(true));
        let snap = s.caps_lock(true);
        assert!(snap.ok && snap.english && snap.caps_english, "{snap:?}");
        assert_eq!(
            st.lock().unwrap().requests.len(),
            before,
            "Caps Lock 只在 TSF 內切換模式，不送引擎"
        );
        let snap = s.caps_lock(false);
        assert!(snap.ok && !snap.english && !snap.caps_english, "{snap:?}");
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
        s.toggle();
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
        let snap = s.toggle();
        assert!(!snap.english, "切回中文只改外殼模式");
    }

    /// 打英文單字不該把整個輸入法切成英文模式。
    ///
    /// mixed 模式的 `Snapshot.mode` 是「這一段組字是什麼語言」；打完一句英文
    /// 它會是 english，但中英模式只由 toggle／mode／caps_lock 改變。實機症狀：
    /// 打到一半變全英文，要按兩下 Shift 才回來。
    #[test]
    fn english_word_does_not_flip_the_input_mode() {
        let (mut s, st) = toy_session();
        {
            let mut state = st.lock().unwrap();
            state.english = false;
        }
        let first = s.key("h");
        assert!(first.ok, "{first:?}");
        // 假引擎照 mixed 模式回報 english 片段。
        let mut state = st.lock().unwrap();
        state.english = true;
        drop(state);
        let typed = s.key("e");
        assert!(!typed.english, "英文片段不該把輸入模式切成英文：{typed:?}");
        assert!(!s.english(), "外殼的中英模式要維持中文");
    }

    #[test]
    fn shift_config_is_cached_and_toggle_sends_nothing() {
        let (mut s, st) = toy_session();
        st.lock().unwrap().info = json!({
            "left_shift": "true",
            "right_shift": "false",
            "shift_tap_ms": "450",
        });
        let _ = s.learn_mode(true);
        let before = st.lock().unwrap().requests.len();
        let config = s.shift_config();
        assert!(config.toggles(keys::Side::Left), "先從 health 學到設定");
        assert!(!config.toggles(keys::Side::Right));
        assert_eq!(config.tap_ms, 450);

        let snap = s.toggle();
        assert!(snap.ok && snap.english && snap.commit.is_empty());
        assert_eq!(
            st.lock().unwrap().requests.len(),
            before,
            "Shift 切換不得送任何 IPC"
        );
    }

    #[test]
    fn toggle_keeps_the_composition_without_asking_the_engine() {
        let (mut s, st) = toy_session();
        assert!(s.key("j").handled);
        let before = st.lock().unwrap().requests.len();
        let snap = s.toggle();
        assert!(snap.ok && snap.english, "切到英文");
        assert_eq!(snap.text(), "ㄨ", "組字留著");
        assert!(snap.commit.is_empty(), "不得提交組字");
        assert_eq!(
            st.lock().unwrap().requests.len(),
            before,
            "toggle 只改外殼狀態"
        );
    }

    #[test]
    fn toggle_works_without_an_engine() {
        let mut s = Session::new();
        let snap = s.toggle();
        assert!(snap.ok && s.english(), "連不上引擎時 Shift 仍要切英文");
        assert_eq!(snap.commit, "");
    }

    #[test]
    fn toggle_keeps_pending_zhuyin_without_committing() {
        let (mut s, _) = toy_session();
        s.key("j");
        let english = s.toggle();
        assert!(
            english.english && english.composing() && english.commit.is_empty(),
            "切到英文不提交、也不丟掉未打完的注音（#90）"
        );
        let back = s.toggle();
        assert!(!back.english && back.text() == "ㄨ" && back.commit.is_empty());
        s.client.disconnect_for_test();
        let replayed = s.key("i");
        assert!(!replayed.english, "斷線重播後仍是注音");
        s.toggle();
        let snap = s.toggle();
        assert!(
            !snap.english && snap.composing() && snap.commit.is_empty(),
            "切換之間有交還宿主的按鍵也不提交組字"
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
    fn transport_timeout_aligns_with_snapshot_instead_of_replaying() {
        let (mut s, st) = toy_session();
        for k in ["s", "u", "3"] {
            assert!(s.key(k).handled, "{k}");
        }
        assert_eq!(s.log.len(), 3);
        {
            let mut state = st.lock().unwrap();
            state.fail_left = 1;
            state.timeout = true;
        }
        let failed = s.key("c");
        assert!(!failed.ok, "這次請求要失敗");
        assert!(
            failed.commit.is_empty() && failed.text().is_empty(),
            "傳輸失敗不得提交或保留組字：{failed:?}"
        );
        assert!(s.log.is_empty(), "逾時後不得保留重播按鍵");
        {
            let mut state = st.lock().unwrap();
            state.timeout = false;
            state.requests.clear();
        }
        let snap = s.key("c");
        assert!(snap.ok && snap.reconnected, "{snap:?}");
        assert_eq!(snap.text(), "ㄏ", "重連後只送這次的鍵");
        let requests = st.lock().unwrap().requests.clone();
        let ops: Vec<&str> = requests
            .iter()
            .filter_map(|request| request["op"].as_str())
            .collect();
        assert_eq!(ops.first(), Some(&"hello"));
        assert!(
            ops.contains(&"snapshot"),
            "重連後先送 display 對齊：{ops:?}"
        );
        let keys = st.lock().unwrap().keys.clone();
        assert_eq!(
            keys,
            vec!["c".to_string()],
            "不得重播逾時前的按鍵：{keys:?}"
        );
    }

    #[test]
    fn peer_closed_keeps_the_replay_log() {
        let (mut s, st) = toy_session();
        for k in ["s", "u", "3"] {
            assert!(s.key(k).handled, "{k}");
        }
        {
            let mut state = st.lock().unwrap();
            state.fail_left = 1;
            state.timeout = false;
        }
        let snap = s.key("c");
        assert!(snap.ok && snap.reconnected, "{snap:?}");
        assert_eq!(snap.text(), "你ㄏ", "引擎重啟才重播組字");
        assert!(!s.client.restart_pending(), "接上新引擎就不再等");
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
        st.lock().unwrap().info = json!({"input_style_shift_space": "disabled"});
        assert!(!s.style_shift_space());
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
                    "left_shift": enabled.to_string(),
                });
            }
            s.client
                .set_restart_until_for_test(Instant::now() + Duration::from_millis(20));
            assert_eq!(s.style_shift_space(), enabled, "first space after handoff");
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
