//! TSF DLL 與 jc-ui 共用的呈現訊息與 helper。移植自 0c 的 common crate。
//!
//! 除了 pipe 前綴與函式名稱，結構欄位、serde 屬性、選字常數與幾何 helper
//! 都照 0c 逐字保留，讓 T12b 的 DLL 側能直接用同一份型別組訊框。
//! 需要 Win32 的三個函式（[`user_sid`]、[`ui_pipe`]、[`pipe_sddl`]）只在
//! Windows 編；其餘部分在 Linux 也能編譯與測試。

use serde::{Deserialize, Serialize};

/// 以 NUL 結尾的 UTF-16。
pub fn wide(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

/// 目前使用者的 SID 字串（小寫），與 jc engine 的 pipe 命名一致。
#[cfg(windows)]
pub fn user_sid() -> Option<String> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows_core::PWSTR;

    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let mut len = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u8; len as usize];
        let r = GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr() as *mut c_void),
            len,
            &mut len,
        );
        let _ = CloseHandle(token);
        r.ok()?;
        let tu = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = PWSTR::null();
        ConvertSidToStringSidW(tu.User.Sid, &mut s).ok()?;
        let out = s.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(s.0 as *mut c_void)));
        out.map(|x| x.to_lowercase())
    }
}

fn user_pipe(prefix: &str) -> String {
    #[cfg(windows)]
    {
        match user_sid() {
            Some(sid) => format!(r"\\.\pipe\{}-{}", prefix, sid),
            None => format!(r"\\.\pipe\{}", prefix),
        }
    }
    #[cfg(not(windows))]
    {
        format!(r"\\.\pipe\{}", prefix)
    }
}

/// UI（候選窗）pipe 的前綴。用 `jc-ime-ui` 而非 0c 的 `0c-ui`，
/// 讓新版 DLL 與舊版 0c-ui 同時註冊時不會連到對方的視窗。
pub const UI_PIPE_PREFIX: &str = "jc-ime-ui";

/// jc-ui 的候選窗 pipe：`\\.\pipe\jc-ime-ui-<SID>`。
pub fn ui_pipe() -> String {
    user_pipe(UI_PIPE_PREFIX)
}

/// pipe 的存取控制（與 0c 相同）：本人完全存取；ALL APPLICATION PACKAGES 與
/// ALL RESTRICTED APPLICATION PACKAGES 可讀寫（沙箱宿主）；低完整性等級可開啟。
/// 其他使用者連不上。
#[cfg(windows)]
pub fn pipe_sddl() -> String {
    let sid = user_sid().unwrap_or_else(|| "OW".to_string());
    format!(
        "D:P(A;;GA;;;{})(A;;GRGW;;;S-1-15-2-1)(A;;GRGW;;;S-1-15-2-2)S:(ML;;NW;;;LW)",
        sid.to_uppercase()
    )
}

/// 訊框：4 位元組小端序長度＋JSON（與 bridge/ipc 相同）。
pub const MAX_FRAME: u32 = 1 << 20;

/// 把訊息序列化成 4-byte LE 長度＋JSON 訊框；超過 [`MAX_FRAME`] 回 `None`。
pub fn frame<T: Serialize>(v: &T) -> Option<Vec<u8>> {
    let body = serde_json::to_vec(v).ok()?;
    if body.len() as u32 > MAX_FRAME {
        return None;
    }
    let mut out = (body.len() as u32).to_le_bytes().to_vec();
    out.extend_from_slice(&body);
    Some(out)
}

/// 候選的一列。欄位照 0c：`id` 供選字回傳，`note` 是低調附註，`key` 是鍵標。
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct UiCandidate {
    pub id: i64,
    pub text: String,
    /// 低調附註（模糊音近音為「近音 ㄓ」）；沒有附註、或舊 DLL 沒送時為空字串。
    #[serde(default)]
    pub note: String,
    /// 鍵標：核心給的選字鍵（1、q、;）；Some("") 不畫（英文模式的補字候選）。
    /// 舊 DLL 沒送時為 None：依位置畫 1–0（見 [`candidate_key`]）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// 第 index 項候選的鍵標：DLL 送來的選字鍵；舊 DLL 沒送時依位置 1–9、0。
pub fn candidate_key(index: usize, c: &UiCandidate) -> String {
    match &c.key {
        Some(key) => key.clone(),
        None => ((index + 1) % 10).to_string(),
    }
}

/// DLL 送給 jc-ui 的呈現訊息：畫什麼、畫在哪。不含文件文字，不含組字以外的內容。
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct UiMessage {
    /// false：隱藏候選窗（其餘欄位忽略）。
    pub show: bool,
    /// 游標的螢幕座標矩形（實體像素）。
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub candidates: Vec<UiCandidate>,
    /// 反白的候選在 candidates 裡的索引。
    pub highlight: i64,
    /// 1 起算的頁碼與總頁數。
    pub page: i64,
    pub pages: i64,
    #[serde(default)]
    pub columns: usize,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    /// Tab 補全提示（一般為後半；同音替換時為「原文 → 完整詞」）。
    pub suggestion: String,
    /// 核心的一次性提示，畫在最上面；幾秒後或下一鍵後消失。
    #[serde(default)]
    pub notice: String,
    /// 組字區還在標示要加入詞庫的範圍：這時的提示是標示的說明，不自動收起。
    #[serde(default)]
    pub marking: bool,
    /// 讀音提示：中英混打時照字面顯示的數字按空白會組成的注音。
    #[serde(default)]
    pub reading: String,
    /// 送出者（宿主行程 ID）；不同宿主的訊息互相取代。
    pub owner: u32,
    /// 同一宿主 TSF 執行緒的訊息視窗；滑鼠選字要把 WM_SELECT_CANDIDATE 送回這裡。
    #[serde(default)]
    pub target: u64,
    #[serde(default)]
    pub revision: u32,
}

/// UI → TSF：wParam 是畫面版本（revision），lParam 是候選 ID；不傳文字或指標。
/// 值與 0c 逐字相同（0x8000 + 55）。
pub const WM_SELECT_CANDIDATE: u32 = 0x8000 + 55;

/// 候選之前的列：提示（notice）、讀音提示（reading）、補全提示（suggestion）、標題（title）。
/// 提示優先：有提示時取代標題；讀音提示只在沒有候選時出現。
pub fn header_lines(m: &UiMessage) -> Vec<&str> {
    let title = if m.notice.is_empty() {
        m.title.as_str()
    } else {
        ""
    };
    [
        m.notice.as_str(),
        m.reading.as_str(),
        m.suggestion.as_str(),
        title,
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect()
}

/// 提示列的高度：提示、讀音提示與補全提示用一般字級，列高跟著放大。
pub fn notice_row(small_row: i32) -> i32 {
    small_row * 7 / 5
}

/// 這一列是不是用一般字級畫（提示、讀音提示、補全提示），標題是小字。
pub fn large_heading(m: &UiMessage, heading: &str) -> bool {
    !m.notice.is_empty() && heading == m.notice
        || !m.reading.is_empty() && heading == m.reading
        || !m.suggestion.is_empty() && heading == m.suggestion
}

/// 候選之前所有列的總高度（提示、讀音提示與補全提示每列 notice_row，標題 small_row）。
pub fn header_height(m: &UiMessage, small_row: i32) -> i32 {
    header_lines(m)
        .iter()
        .map(|h| {
            if large_heading(m, h) {
                notice_row(small_row)
            } else {
                small_row
            }
        })
        .sum()
}

/// 提示顯示多久後由候選窗自己收起。
pub const NOTICE_MS: u32 = 4000;

/// 提示是否要計時自動收起：有提示、且不是標示中的說明。
pub fn notice_expires(m: &UiMessage) -> bool {
    !m.notice.is_empty() && !m.marking
}

/// 呈現訊息是否有東西可畫（候選、補全提示、提示或讀音提示）。
pub fn has_content(m: &UiMessage) -> bool {
    !m.candidates.is_empty()
        || !m.suggestion.is_empty()
        || !m.notice.is_empty()
        || !m.reading.is_empty()
}

/// 與繪製一致的候選列命中；提示、頁碼、邊距不會選字。
pub fn hit_candidate(m: &UiMessage, y: i32, pad: i32, row: i32, small_row: i32) -> Option<i64> {
    let first = pad + header_height(m, small_row);
    if y < first || row <= 0 {
        return None;
    }
    m.candidates.get(((y - first) / row) as usize).map(|c| c.id)
}

/// 多排格子的幾何：繪製與滑鼠命中共用，保留最後一列空格，空格不會選到別的候選。
pub fn candidate_cells(
    m: &UiMessage,
    width: i32,
    pad: i32,
    row: i32,
    small_row: i32,
) -> Vec<CandidateCell> {
    let columns = m.columns.clamp(1, 10);
    let gap = if columns > 1 { pad / 2 } else { 0 };
    let cell_width = ((width - 2 * pad - gap * (columns as i32 - 1)) / columns as i32).max(1);
    let top = pad + header_height(m, small_row);
    m.candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| CandidateCell {
            id: candidate.id,
            left: pad + (index % columns) as i32 * (cell_width + gap),
            top: top + (index / columns) as i32 * row,
            width: cell_width,
            height: row,
        })
        .collect()
}

/// 多排格子其中一格的位置與候選 ID。
#[derive(Clone, Debug, PartialEq)]
pub struct CandidateCell {
    pub id: i64,
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

impl CandidateCell {
    /// 座標是否落在格子內。
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.left + self.width && y >= self.top && y < self.top + self.height
    }
}
