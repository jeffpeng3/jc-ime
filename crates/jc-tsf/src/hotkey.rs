//! 設定檔的快捷鍵改用 Win32 `RegisterHotKey` 攔截。
//!
//! TSF 的 key sink 收不到部分 Alt 組合（AltGr 會被當成 Ctrl＋Alt，部分宿主
//! 直接把 Alt 組合交給選單）。這裡依 `settings.json` 的 `[keybinds]` 註冊
//! 全域快捷鍵；收到 `WM_HOTKEY` 後把對應的 [`Hotkey`] 交給 engine，由 keymap
//! 執行 command。
//!
//! TSF 的 UI 執行緒不保證跑 GetMessage 迴圈：在那條執行緒註冊會成功，但
//! `WM_HOTKEY` 只會被塞進佇列而不派送。因此視窗與所有 `RegisterHotKey`／
//! `UnregisterHotKey` 都放在專用執行緒，該執行緒自己跑 GetMessage／
//! DispatchMessage。熱鍵執行緒不直接碰 TSF；它把 chord 放進共用佇列，再
//! PostMessage 到 UI 執行緒的轉送視窗。UI 執行緒沒有訊息迴圈時，下一顆
//! 按鍵的入口也會 drain 同一個佇列。
//!
//! 只有已設定的組合會被系統攔下，其餘按鍵照舊走 key sink。註冊成功與失敗、
//! 收到 `WM_HOTKEY` 都寫 `tsf.log`（chord、VK、錯誤碼）。註冊失敗（被其他
//! 程式占用、F12 等保留鍵）保留原本的 key sink 路徑當 fallback。
//!
//! 解析、註冊清單、去重與「何時註冊／反註冊」的決策是純資料，Linux 也編、
//! 也測；Win32 的執行緒、視窗與 `RegisterHotKey` 只在 Windows 編譯。

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use jc_proto::{Key, KeyCode};
use jc_settings::keybinds::{Chord, ChordError, Keybinds};
use serde_json::Value;

// Win32 虛擬鍵碼。放在純資料區，Linux 測試才能驗證 chord → VK。
const VK_BACK: u32 = 0x08;
const VK_TAB: u32 = 0x09;
const VK_RETURN: u32 = 0x0d;
const VK_ESCAPE: u32 = 0x1b;
const VK_SPACE: u32 = 0x20;
const VK_PRIOR: u32 = 0x21;
const VK_NEXT: u32 = 0x22;
const VK_END: u32 = 0x23;
const VK_HOME: u32 = 0x24;
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_RIGHT: u32 = 0x27;
const VK_DOWN: u32 = 0x28;
const VK_DELETE: u32 = 0x2e;
const VK_F1: u32 = 0x70;
const VK_OEM_1: u32 = 0xba;
const VK_OEM_PLUS: u32 = 0xbb;
const VK_OEM_COMMA: u32 = 0xbc;
const VK_OEM_MINUS: u32 = 0xbd;
const VK_OEM_PERIOD: u32 = 0xbe;
const VK_OEM_2: u32 = 0xbf;
const VK_OEM_3: u32 = 0xc0;
const VK_OEM_4: u32 = 0xdb;
const VK_OEM_5: u32 = 0xdc;
const VK_OEM_6: u32 = 0xdd;
const VK_OEM_7: u32 = 0xde;

/// 設定檔的變更戳記：修改時間（奈秒）與檔案大小。
pub type Stamp = (u64, u64);

/// chord 轉成 Win32 註冊資料時的錯誤。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyError {
    /// chord 字串本身不合法。
    Chord(ChordError),
    /// chord 合法，但沒有對應的 Win32 虛擬鍵，或需要 Shift 才能打出。
    Unsupported {
        /// 正規化後的按鍵名稱。
        key: String,
        /// 不能註冊的原因。
        reason: String,
    },
}

impl fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Chord(error) => write!(f, "{error}"),
            Self::Unsupported { key, reason } => {
                write!(f, "按鍵 {key:?} 無法註冊：{reason}")
            }
        }
    }
}

impl From<ChordError> for HotkeyError {
    fn from(error: ChordError) -> Self {
        Self::Chord(error)
    }
}

/// 一個可註冊的快捷鍵：action、正規化 chord 與 Win32 虛擬鍵。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotkey {
    /// engine keymap 的 action，例如 `symbol_menu`。
    pub action: String,
    /// 正規化 chord 字串，例如 `ctrl+alt+,`。
    pub chord: String,
    /// Ctrl。
    pub ctrl: bool,
    /// Alt。
    pub alt: bool,
    /// Shift。
    pub shift: bool,
    /// 正規化按鍵名稱。
    pub key: String,
    /// Win32 虛擬鍵碼。
    pub vk: u32,
}

/// 因為重複或無法表示而沒有註冊的項目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// engine keymap 的 action。
    pub action: String,
    /// 設定裡寫的 chord（正規化後）。
    pub chord: String,
    /// 跳過的原因。
    pub reason: String,
}

impl Hotkey {
    /// 解析一個 action 的 chord 字串。
    pub fn parse(action: &str, text: &str) -> Result<Self, HotkeyError> {
        let chord = Chord::parse(text)?;
        Self::from_chord(action, &chord)
    }

    /// 把已解析的 chord 轉成註冊資料。
    pub fn from_chord(action: &str, chord: &Chord) -> Result<Self, HotkeyError> {
        let (vk, needs_shift) = vk_for_key(&chord.key)?;
        if needs_shift && !chord.shift {
            return Err(HotkeyError::Unsupported {
                key: chord.key.clone(),
                reason: "這個符號需要 Shift；請把 shift 寫進 chord".to_string(),
            });
        }
        Ok(Self {
            action: action.to_string(),
            chord: chord.as_str(),
            ctrl: chord.ctrl,
            alt: chord.alt,
            shift: chord.shift,
            key: chord.key.clone(),
            vk,
        })
    }

    /// 送給 engine 的結構化按鍵；修飾鍵沿用設定，不讀當下鍵盤狀態。
    ///
    /// 熱鍵被系統攔下後，實體修飾鍵可能已經放開；用設定值才能與 keymap
    /// 的正規化 chord 對上。
    pub fn to_key(&self) -> Key {
        let (code, text) = match key_code(&self.key) {
            Some((code, text)) => (code, text),
            None => {
                let ch = self
                    .key
                    .chars()
                    .next()
                    .expect("Hotkey 的 key 至少有一個字元");
                (KeyCode::Char(ch), Some(ch))
            }
        };
        let mut key = Key::new(code, text, self.shift);
        key.ctrl = self.ctrl;
        key.alt = self.alt;
        key
    }

    /// 修飾鍵的位元組合，供去重使用。
    fn modifier_bits(&self) -> u32 {
        (u32::from(self.ctrl)) | (u32::from(self.alt) << 1) | (u32::from(self.shift) << 2)
    }

    /// Win32 的 `RegisterHotKey` 修飾鍵；`MOD_NOREPEAT` 讓長按只觸發一次。
    #[cfg(windows)]
    fn win_modifiers(&self) -> windows::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT,
        };
        let mut modifiers = HOT_KEY_MODIFIERS(MOD_NOREPEAT.0);
        if self.ctrl {
            modifiers |= MOD_CONTROL;
        }
        if self.alt {
            modifiers |= MOD_ALT;
        }
        if self.shift {
            modifiers |= MOD_SHIFT;
        }
        modifiers
    }
}

/// 具名鍵的語義鍵碼；不是具名鍵回 `None`。
fn key_code(key: &str) -> Option<(KeyCode, Option<char>)> {
    Some(match key {
        "enter" => (KeyCode::Enter, None),
        "escape" => (KeyCode::Escape, None),
        "backspace" => (KeyCode::Backspace, None),
        "tab" => (KeyCode::Tab, None),
        "space" => (KeyCode::Space, None),
        "delete" => (KeyCode::Delete, None),
        "home" => (KeyCode::Home, None),
        "end" => (KeyCode::End, None),
        "pageup" => (KeyCode::PageUp, None),
        "pagedown" => (KeyCode::PageDown, None),
        "up" => (KeyCode::Up, None),
        "down" => (KeyCode::Down, None),
        "left" => (KeyCode::ArrowLeft, None),
        "right" => (KeyCode::ArrowRight, None),
        _ => {
            if let Some(number) = key.strip_prefix('f') {
                let number = number.parse::<u8>().ok()?;
                if (1..=12).contains(&number) {
                    return Some((KeyCode::F(number), None));
                }
            }
            return None;
        }
    })
}

/// 按鍵名稱 → Win32 虛擬鍵；第二個值表示這個字元本身需要 Shift。
fn vk_for_key(key: &str) -> Result<(u32, bool), HotkeyError> {
    if let Some(vk) = named_vk(key) {
        return Ok((vk, false));
    }
    let mut chars = key.chars();
    let Some(ch) = chars.next() else {
        return Err(HotkeyError::Unsupported {
            key: key.to_string(),
            reason: "按鍵名稱是空字串".to_string(),
        });
    };
    if chars.next().is_some() {
        return Err(HotkeyError::Unsupported {
            key: key.to_string(),
            reason: "不是單一字元或支援的具名鍵".to_string(),
        });
    }
    if ch.is_ascii_alphabetic() {
        return Ok((u32::from(ch.to_ascii_uppercase()), false));
    }
    if ch.is_ascii_digit() {
        return Ok((ch as u32, false));
    }
    let (vk, needs_shift) = match ch {
        ',' => (VK_OEM_COMMA, false),
        '.' => (VK_OEM_PERIOD, false),
        ';' => (VK_OEM_1, false),
        ':' => (VK_OEM_1, true),
        '/' => (VK_OEM_2, false),
        '?' => (VK_OEM_2, true),
        '`' => (VK_OEM_3, false),
        '~' => (VK_OEM_3, true),
        '[' => (VK_OEM_4, false),
        '{' => (VK_OEM_4, true),
        '\\' => (VK_OEM_5, false),
        '|' => (VK_OEM_5, true),
        ']' => (VK_OEM_6, false),
        '}' => (VK_OEM_6, true),
        '\'' => (VK_OEM_7, false),
        '"' => (VK_OEM_7, true),
        '-' => (VK_OEM_MINUS, false),
        '_' => (VK_OEM_MINUS, true),
        '=' => (VK_OEM_PLUS, false),
        '+' => (VK_OEM_PLUS, true),
        '!' => (0x31, true),
        '@' => (0x32, true),
        '#' => (0x33, true),
        '$' => (0x34, true),
        '%' => (0x35, true),
        '^' => (0x36, true),
        '&' => (0x37, true),
        '*' => (0x38, true),
        '(' => (0x39, true),
        ')' => (0x30, true),
        other => {
            return Err(HotkeyError::Unsupported {
                key: key.to_string(),
                reason: format!("沒有對應的 Win32 虛擬鍵（{other:?}）"),
            })
        }
    };
    Ok((vk, needs_shift))
}

fn named_vk(key: &str) -> Option<u32> {
    Some(match key {
        "enter" => VK_RETURN,
        "escape" => VK_ESCAPE,
        "backspace" => VK_BACK,
        "tab" => VK_TAB,
        "space" => VK_SPACE,
        "delete" => VK_DELETE,
        "home" => VK_HOME,
        "end" => VK_END,
        "pageup" => VK_PRIOR,
        "pagedown" => VK_NEXT,
        "up" => VK_UP,
        "down" => VK_DOWN,
        "left" => VK_LEFT,
        "right" => VK_RIGHT,
        _ => {
            let number = key.strip_prefix('f')?.parse::<u32>().ok()?;
            if (1..=12).contains(&number) {
                VK_F1 + number - 1
            } else {
                return None;
            }
        }
    })
}

/// 從已解析的 `Keybinds` 建立註冊清單；不支援的組合與重複組合跳過。
pub fn plan(keybinds: &Keybinds) -> Result<(Vec<Hotkey>, Vec<Skipped>), ChordError> {
    let chords = keybinds.chords()?;
    let pairs = chords
        .into_iter()
        .map(|(chord, action)| (action.to_string(), chord.as_str()));
    Ok(plan_pairs(pairs))
}

/// 從 action → chord 字串建立註冊清單。
///
/// 同一組實體按鍵只留第一個（例如 `ctrl+alt+shift+1` 與 `ctrl+alt+shift+!`
/// 在 US 鍵盤上是同一個 VK），後面的記進 [`Skipped`]。
pub fn plan_pairs<I>(pairs: I) -> (Vec<Hotkey>, Vec<Skipped>)
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut seen: BTreeMap<(u32, u32), String> = BTreeMap::new();
    let mut plan = Vec::new();
    let mut skipped = Vec::new();
    for (action, text) in pairs {
        let hotkey = match Hotkey::parse(&action, &text) {
            Ok(hotkey) => hotkey,
            Err(error) => {
                skipped.push(Skipped {
                    action,
                    chord: text,
                    reason: error.to_string(),
                });
                continue;
            }
        };
        let identity = (hotkey.modifier_bits(), hotkey.vk);
        if let Some(first) = seen.get(&identity) {
            skipped.push(Skipped {
                action,
                chord: hotkey.chord,
                reason: format!("與 {first} 使用同一個組合，只註冊第一個"),
            });
            continue;
        }
        seen.insert(identity, action.clone());
        plan.push(hotkey);
    }
    (plan, skipped)
}

/// `RegisterHotKey` 的 id 起點；同一條執行緒的其他程式各自用自己的 hwnd。
const ID_BASE: i32 = 0x4a00;

/// 熱鍵執行緒的指令。Windows 由視窗訊息轉成這些指令；Linux 測試直接餵。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// 取得前景：依目前設定註冊。
    Enable,
    /// 失去前景：全部反註冊。
    Disable,
    /// 設定檔可能變了；呼叫端先比對戳記再決定要不要 reload。
    Tick,
    /// 收到 `WM_HOTKEY`。
    Hotkey { id: i32 },
}

/// 熱鍵執行緒的副作用出口；Windows 接到 Win32，測試用記錄用的假 sink。
pub trait Sink {
    /// 註冊一組組合；失敗回可讀原因，呼叫端下一次 tick 會重試。
    fn register(&mut self, id: i32, hotkey: &Hotkey) -> Result<(), String>;
    /// 反註冊一個 id。
    fn unregister(&mut self, id: i32);
    /// 把已註冊的組合送回 TSF。
    fn press(&mut self, hotkey: &Hotkey);
}

/// 註冊決策：哪些 id 已註冊、何時補註冊或全部反註冊。
///
/// 不碰 Win32 也不讀檔，只維護「計畫」與「已註冊」。Windows 的熱鍵執行緒
/// 與 Linux 測試走同一段決策，差別只在 [`Sink`] 的實作。
#[derive(Debug, Default)]
pub struct Machine {
    enabled: bool,
    plan: Vec<Hotkey>,
    /// (id, plan 索引)。
    registered: Vec<(i32, usize)>,
}

impl Machine {
    /// 空的決策器：尚未啟用、沒有計畫。
    pub fn new() -> Self {
        Self::default()
    }

    /// 是否處於前景（只有前景才註冊）。
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// 目前的註冊計畫。
    pub fn plan(&self) -> &[Hotkey] {
        &self.plan
    }

    /// 目前已註冊的 id。
    pub fn registered(&self) -> Vec<i32> {
        self.registered.iter().map(|(id, _)| *id).collect()
    }

    /// 設定前景狀態但先不註冊；`enable` 會補上。
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// 取得前景：標記啟用並補齊尚未註冊的組合。
    pub fn enable(&mut self, sink: &mut dyn Sink) {
        self.enabled = true;
        self.register_missing(sink);
    }

    /// 失去前景：標記停用並全部反註冊。
    pub fn disable(&mut self, sink: &mut dyn Sink) {
        self.enabled = false;
        self.unregister_all(sink);
    }

    /// 換一份計畫：先反註冊舊的，再依前景狀態註冊新的。
    pub fn apply(&mut self, sink: &mut dyn Sink, plan: Vec<Hotkey>) {
        self.unregister_all(sink);
        self.plan = plan;
        self.register_missing(sink);
    }

    /// 補註冊還沒成功的 id；失敗的不進 `registered`，下一次 tick 再試。
    pub fn register_missing(&mut self, sink: &mut dyn Sink) {
        if !self.enabled {
            return;
        }
        let Self {
            plan, registered, ..
        } = self;
        for (index, hotkey) in plan.iter().enumerate() {
            let id = ID_BASE + index as i32;
            if registered.iter().any(|(entry, _)| *entry == id) {
                continue;
            }
            if sink.register(id, hotkey).is_ok() {
                registered.push((id, index));
            }
        }
    }

    /// 全部反註冊，清空已註冊清單。
    pub fn unregister_all(&mut self, sink: &mut dyn Sink) {
        let Self { registered, .. } = self;
        for (id, _) in registered.drain(..) {
            sink.unregister(id);
        }
    }

    /// 分派一個 `WM_HOTKEY`；只有目前註冊中的 id 會送到 sink。
    pub fn hotkey(&mut self, sink: &mut dyn Sink, id: i32) -> bool {
        let Self {
            plan, registered, ..
        } = self;
        let Some((_, index)) = registered.iter().find(|(entry, _)| *entry == id) else {
            return false;
        };
        let Some(hotkey) = plan.get(*index) else {
            return false;
        };
        sink.press(hotkey);
        true
    }
}

/// 安裝後的設定檔路徑：`%LOCALAPPDATA%\jc-ime\settings.json`（與 installer 相同）。
pub fn settings_path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(base).join("jc-ime").join("settings.json"))
}

/// 設定檔戳記；檔案不存在時回 `None`。
pub fn settings_stamp(path: &Path) -> Option<Stamp> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0);
    Some((modified, metadata.len()))
}

/// 讀取設定檔的 `keybinds`；檔案不存在時用預設值。
///
/// 壞檔只回錯誤，不自行改名或覆寫；熱重載失敗時呼叫端保留目前註冊。
pub fn read_keybinds(path: &Path) -> Result<Keybinds, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Keybinds::default())
        }
        Err(error) => return Err(format!("讀取 {} 失敗：{error}", path.display())),
    };
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("解析 {} 失敗：{error}", path.display()))?;
    Ok(value
        .get("keybinds")
        .map(Keybinds::from_json)
        .unwrap_or_default())
}

#[cfg(windows)]
mod win {
    //! 專用執行緒上的 message-only window 與 `RegisterHotKey`。
    //!
    //! TSF 的 UI 執行緒不保證跑 GetMessage 迴圈：在那裡 `RegisterHotKey` 會
    //! 成功，但 `WM_HOTKEY` 只會被塞進佇列而不派送。這裡把視窗、註冊與
    //! 設定熱重載都搬到自建執行緒，該執行緒自己跑 GetMessage／DispatchMessage。
    //!
    //! `WM_HOTKEY` 不能直接在熱鍵執行緒呼叫 service：TSF 狀態綁在 UI 執行緒。
    //! 熱鍵執行緒把 chord 放進共用佇列，再 PostMessage 到位於 UI 執行緒的轉送
    //! 視窗；轉送視窗的 wndproc 呼叫 service。UI 執行緒沒有訊息迴圈時，
    //! 下一顆按鍵的入口也會 drain 同一個佇列。

    use std::collections::{BTreeSet, VecDeque};
    use std::ffi::c_void;
    use std::sync::mpsc::{self, Sender};
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;

    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
        GetWindowLongPtrW, KillTimer, PostMessageW, PostThreadMessageW, RegisterClassW, SetTimer,
        SetWindowLongPtrW, TranslateMessage, UnregisterClassW, GWLP_USERDATA, HWND_MESSAGE, MSG,
        WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_DESTROY, WM_HOTKEY, WM_NCDESTROY, WM_QUIT,
        WM_TIMER, WNDCLASSW,
    };
    use windows_core::{w, PCWSTR};

    use super::{
        plan, read_keybinds, settings_path, settings_stamp, Hotkey, Keybinds, Machine, Sink,
        Skipped, Stamp,
    };
    use crate::log;

    const THREAD_CLASS: PCWSTR = w!("jcHotkeyThread");
    const TARGET_CLASS: PCWSTR = w!("jcHotkeyTarget");
    /// 熱鍵執行緒 → UI 執行緒：處理佇列裡的 `WM_HOTKEY`。
    const WM_FORWARD_HOTKEY: u32 = WM_APP + 0x51a;
    /// UI 執行緒 → 熱鍵執行緒：取得／失去前景。
    const WM_ENABLE: u32 = WM_APP + 0x51b;
    const WM_DISABLE: u32 = WM_APP + 0x51c;
    /// 設定檔重載與註冊重試的 timer；在熱鍵執行緒上。
    const TIMER_ID: usize = 0x0c1a;
    const TIMER_MS: u32 = 2_000;

    /// 熱鍵執行緒的 Win32 出口：每一筆都寫 log。
    struct WinSink {
        hwnd: HWND,
        target: HWND,
        queue: Arc<Mutex<VecDeque<Hotkey>>>,
        /// 已記過失敗的 chord；避免每 2 秒重試時洗版。
        failures: BTreeSet<String>,
    }

    impl Sink for WinSink {
        fn register(&mut self, id: i32, hotkey: &Hotkey) -> Result<(), String> {
            let result =
                unsafe { RegisterHotKey(Some(self.hwnd), id, hotkey.win_modifiers(), hotkey.vk) };
            match result {
                Ok(()) => {
                    if self.failures.remove(&hotkey.chord) && self.failures.is_empty() {
                        log::log("hotkey：之前失敗的組合都註冊成功了");
                    }
                    log::log(&format!(
                        "hotkey：註冊成功 {}（{} vk={:#04x} id={id}）",
                        hotkey.chord, hotkey.action, hotkey.vk
                    ));
                    Ok(())
                }
                Err(error) => {
                    let reason = format!(
                        "{}（{} vk={:#04x}）：{error}（HRESULT {:#010x}）",
                        hotkey.chord,
                        hotkey.action,
                        hotkey.vk,
                        error.code().0 as u32
                    );
                    if self.failures.insert(hotkey.chord.clone()) {
                        log::log(&format!(
                            "hotkey 註冊失敗（可能被其他程式占用或系統保留）：{reason}"
                        ));
                    }
                    Err(reason)
                }
            }
        }

        fn unregister(&mut self, id: i32) {
            if let Err(error) = unsafe { UnregisterHotKey(Some(self.hwnd), id) } {
                log::log(&format!(
                    "hotkey：反註冊 id={id} 失敗：{error}（HRESULT {:#010x}）",
                    error.code().0 as u32
                ));
            }
        }

        fn press(&mut self, hotkey: &Hotkey) {
            log::log(&format!(
                "hotkey：收到 WM_HOTKEY {}（{} vk={:#04x}）",
                hotkey.chord, hotkey.action, hotkey.vk
            ));
            self.queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push_back(hotkey.clone());
            if let Err(error) =
                unsafe { PostMessageW(Some(self.target), WM_FORWARD_HOTKEY, WPARAM(0), LPARAM(0)) }
            {
                log::log(&format!("hotkey：轉送到 TSF 執行緒失敗：{error}"));
            }
        }
    }

    /// 熱鍵執行緒自己的狀態；只被該執行緒存取。
    struct ThreadState {
        machine: Machine,
        sink: WinSink,
        stamp: Option<Stamp>,
    }

    impl ThreadState {
        /// 熱鍵執行緒的指令入口；Windows 訊息與 Linux 測試都走這裡的決策。
        fn handle(&mut self, command: super::Command) {
            match command {
                super::Command::Enable => self.enable(),
                super::Command::Disable => self.disable(),
                super::Command::Tick => self.tick(),
                super::Command::Hotkey { id } => {
                    let Self { machine, sink, .. } = self;
                    machine.hotkey(sink, id);
                }
            }
        }

        fn enable(&mut self) {
            self.machine.set_enabled(true);
            if !self.reload_if_changed() {
                let Self { machine, sink, .. } = self;
                machine.register_missing(sink);
            }
        }

        fn disable(&mut self) {
            let Self { machine, sink, .. } = self;
            machine.disable(sink);
        }

        /// 設定檔變了才重讀，並重新註冊。
        fn reload_if_changed(&mut self) -> bool {
            let stamp = settings_path().as_deref().and_then(settings_stamp);
            if stamp == self.stamp {
                return false;
            }
            self.reload(false);
            true
        }

        fn reload(&mut self, startup: bool) {
            let Some(path) = settings_path() else {
                if startup {
                    log::log("hotkey：找不到 LOCALAPPDATA，只能使用預設綁定");
                    self.apply_defaults();
                }
                self.stamp = None;
                return;
            };
            let stamp = settings_stamp(&path);
            self.stamp = stamp;
            if stamp.is_none() && startup {
                log::log(&format!(
                    "hotkey：沒有設定檔 {}，使用預設綁定",
                    path.display()
                ));
                self.apply_defaults();
                return;
            }
            let result = read_keybinds(&path)
                .and_then(|keybinds| plan(&keybinds).map_err(|error| format!("keybinds：{error}")));
            match result {
                Ok((plan, skipped)) => self.apply(plan, skipped),
                Err(error) if startup => {
                    log::log(&format!("hotkey：讀取設定失敗（{error}），改用預設綁定"));
                    self.apply_defaults();
                }
                Err(error) => {
                    log::log(&format!(
                        "hotkey：重新載入設定失敗（{error}），保留目前的註冊"
                    ));
                }
            }
        }

        fn apply_defaults(&mut self) {
            let (plan, skipped) = plan(&Keybinds::default()).expect("預設 keybinds 一定合法");
            self.apply(plan, skipped);
        }

        fn apply(&mut self, plan: Vec<Hotkey>, skipped: Vec<Skipped>) {
            for skip in &skipped {
                log::log(&format!(
                    "hotkey 略過 {}（{}）：{}",
                    skip.action, skip.chord, skip.reason
                ));
            }
            log::log(&format!(
                "hotkey：載入 {} 組快捷鍵（略過 {} 組）",
                plan.len(),
                skipped.len()
            ));
            let Self { machine, sink, .. } = self;
            machine.apply(sink, plan);
        }

        fn tick(&mut self) {
            if !self.machine.is_enabled() {
                return;
            }
            if !self.reload_if_changed() {
                // 上一次被其他程式占用時，每 2 秒重試一次；失敗訊息已去重。
                let Self { machine, sink, .. } = self;
                machine.register_missing(sink);
            }
        }
    }

    /// 熱鍵執行緒主體：建視窗、跑訊息迴圈，直到收到 `WM_QUIT`。
    fn run(
        target: usize,
        queue: Arc<Mutex<VecDeque<Hotkey>>>,
        ready: Sender<Result<(u32, usize), String>>,
    ) {
        unsafe {
            let instance: HINSTANCE = match GetModuleHandleW(None) {
                Ok(module) => module.into(),
                Err(error) => {
                    let _ = ready.send(Err(format!("取得 HINSTANCE 失敗：{error}")));
                    return;
                }
            };
            // 類別是行程內的；重複註冊由系統忽略錯誤。
            let _ = RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(thread_proc),
                hInstance: instance,
                lpszClassName: THREAD_CLASS,
                ..Default::default()
            });
            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE(0),
                THREAD_CLASS,
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance),
                None,
            ) {
                Ok(hwnd) => hwnd,
                Err(error) => {
                    let _ = ready.send(Err(format!("建立熱鍵視窗失敗：{error}")));
                    return;
                }
            };
            let state = Box::new(ThreadState {
                machine: Machine::new(),
                sink: WinSink {
                    hwnd,
                    target: HWND(target as *mut c_void),
                    queue,
                    failures: BTreeSet::new(),
                },
                stamp: None,
            });
            let pointer = Box::into_raw(state);
            set_userdata(hwnd, pointer as isize);
            let _ = SetTimer(Some(hwnd), TIMER_ID, TIMER_MS, None);
            let thread_id = GetCurrentThreadId();
            if ready.send(Ok((thread_id, hwnd.0 as usize))).is_err() {
                // 呼叫端已經放棄：不要留下視窗與執行緒。
                cleanup(hwnd, instance);
                return;
            }
            log::log(&format!(
                "hotkey：專用執行緒啟動 tid={thread_id} hwnd={:#x}",
                hwnd.0 as usize
            ));
            // 只有前景行程能持有全域組合；背景行程先註冊會占住組合，
            // 前景的 TSF 反而收不到 WM_HOTKEY。Activate 當下先問一次，
            // 之後由 OnSetFocus 的 enable／disable 接手。
            let foreground = crate::candidate::foreground_belongs_to(std::process::id());
            log::log(&format!(
                "hotkey：啟動時前景={foreground}（背景時等 OnSetFocus 再註冊）"
            ));
            if let Some(state) = state_at(hwnd) {
                state.machine.set_enabled(foreground);
                state.reload(true);
            }
            let mut message = MSG::default();
            loop {
                let result = GetMessageW(&mut message, None, 0, 0);
                if result.0 == -1 {
                    log::log("hotkey：GetMessage 失敗，熱鍵執行緒結束");
                    break;
                }
                if result.0 == 0 {
                    break;
                }
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            cleanup(hwnd, instance);
            log::log("hotkey：專用執行緒結束");
        }
    }

    /// 收尾：先在同一條執行緒反註冊，再銷毀視窗與類別。
    unsafe fn cleanup(hwnd: HWND, instance: HINSTANCE) {
        let _ = KillTimer(Some(hwnd), TIMER_ID);
        let pointer = userdata(hwnd) as *mut ThreadState;
        if !pointer.is_null() {
            set_userdata(hwnd, 0);
            let mut state = Box::from_raw(pointer);
            state.machine.disable(&mut state.sink);
            drop(state);
        }
        let _ = DestroyWindow(hwnd);
        // 同一行程還有多個 service 時會失敗，留給最後一個離開才移除。
        let _ = UnregisterClassW(THREAD_CLASS, Some(instance));
    }

    unsafe extern "system" fn thread_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_HOTKEY => {
                let _ = crate::guarded("Hotkey", || {
                    if let Some(state) = state_at(hwnd) {
                        state.handle(super::Command::Hotkey {
                            id: wparam.0 as i32,
                        });
                    }
                    Ok(())
                });
                LRESULT(0)
            }
            WM_ENABLE => {
                let _ = crate::guarded("HotkeyEnable", || {
                    if let Some(state) = state_at(hwnd) {
                        state.handle(super::Command::Enable);
                    }
                    Ok(())
                });
                LRESULT(0)
            }
            WM_DISABLE => {
                let _ = crate::guarded("HotkeyDisable", || {
                    if let Some(state) = state_at(hwnd) {
                        state.handle(super::Command::Disable);
                    }
                    Ok(())
                });
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_ID => {
                let _ = crate::guarded("HotkeyTimer", || {
                    if let Some(state) = state_at(hwnd) {
                        state.handle(super::Command::Tick);
                    }
                    Ok(())
                });
                LRESULT(0)
            }
            // 不要讓 DefWindowProc 的 WM_DESTROY 預設處理影響執行緒的 WM_QUIT。
            WM_DESTROY => LRESULT(0),
            WM_NCDESTROY => {
                let pointer = userdata(hwnd) as *mut ThreadState;
                if !pointer.is_null() {
                    set_userdata(hwnd, 0);
                    drop(Box::from_raw(pointer));
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }

    /// 轉送視窗的內容：service 指標與共用佇列，兩者都比視窗長壽。
    struct TargetCtx {
        service: *const crate::service::TextService_Impl,
        queue: Arc<Mutex<VecDeque<Hotkey>>>,
    }

    /// 一個服務的全域快捷鍵：UI 執行緒的轉送視窗＋專用執行緒。
    ///
    /// 由 [`crate::service::TextService`] 的 `State` 持有；`new`／`Drop` 都在
    /// TSF 的 UI 執行緒，`Drop` 會送 `WM_QUIT` 並 join，卸載不留執行緒。
    pub struct HotkeyWindow {
        target: HWND,
        instance: HINSTANCE,
        /// 熱鍵執行緒的 message-only window；只拿來 PostMessage 指令。
        hwnd: HWND,
        thread_id: u32,
        thread: Option<JoinHandle<()>>,
        ctx: *mut TargetCtx,
        queue: Arc<Mutex<VecDeque<Hotkey>>>,
    }

    impl HotkeyWindow {
        /// 建立轉送視窗並啟動專用執行緒；等到執行緒回報 hwnd/tid 才返回。
        pub fn new(service: &crate::service::TextService_Impl) -> Result<Box<Self>, String> {
            unsafe {
                let instance: HINSTANCE = GetModuleHandleW(None)
                    .map(Into::into)
                    .map_err(|error| format!("取得 HINSTANCE 失敗：{error}"))?;
                let _ = RegisterClassW(&WNDCLASSW {
                    lpfnWndProc: Some(target_proc),
                    hInstance: instance,
                    lpszClassName: TARGET_CLASS,
                    ..Default::default()
                });
                let target = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    TARGET_CLASS,
                    w!(""),
                    WINDOW_STYLE(0),
                    0,
                    0,
                    0,
                    0,
                    Some(HWND_MESSAGE),
                    None,
                    Some(instance),
                    None,
                )
                .map_err(|error| format!("建立轉送視窗失敗：{error}"))?;
                let queue = Arc::new(Mutex::new(VecDeque::new()));
                let ctx = Box::into_raw(Box::new(TargetCtx {
                    service,
                    queue: Arc::clone(&queue),
                }));
                set_userdata(target, ctx as isize);
                let target_raw = target.0 as usize;
                let (ready_tx, ready_rx) = mpsc::channel::<Result<(u32, usize), String>>();
                let thread_queue = Arc::clone(&queue);
                let spawned = std::thread::Builder::new()
                    .name("jc-hotkey".to_string())
                    .spawn(move || run(target_raw, thread_queue, ready_tx));
                let handle = match spawned {
                    Ok(handle) => handle,
                    Err(error) => {
                        discard_target(target, instance, ctx);
                        return Err(format!("建立熱鍵執行緒失敗：{error}"));
                    }
                };
                match ready_rx.recv() {
                    Ok(Ok((thread_id, hwnd))) => Ok(Box::new(Self {
                        target,
                        instance,
                        hwnd: HWND(hwnd as *mut c_void),
                        thread_id,
                        thread: Some(handle),
                        ctx,
                        queue,
                    })),
                    Ok(Err(error)) => {
                        let _ = handle.join();
                        discard_target(target, instance, ctx);
                        Err(error)
                    }
                    Err(_) => {
                        let _ = handle.join();
                        discard_target(target, instance, ctx);
                        Err("熱鍵執行緒沒有回報（可能 panic）".to_string())
                    }
                }
            }
        }

        /// 取得前景：請熱鍵執行緒重載設定並註冊。
        pub fn enable(&self) {
            self.post(WM_ENABLE, "enable");
        }

        /// 失去前景：請熱鍵執行緒全部反註冊。
        pub fn disable(&self) {
            self.post(WM_DISABLE, "disable");
        }

        /// 取出待處理的 `WM_HOTKEY`；轉送視窗與按鍵入口共用。
        pub fn take_pending(&self) -> Vec<Hotkey> {
            let mut queue = self
                .queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            queue.drain(..).collect()
        }

        fn post(&self, message: u32, what: &str) {
            if let Err(error) =
                unsafe { PostMessageW(Some(self.hwnd), message, WPARAM(0), LPARAM(0)) }
            {
                log::log(&format!("hotkey：送出 {what} 失敗：{error}"));
            }
        }
    }

    impl Drop for HotkeyWindow {
        fn drop(&mut self) {
            if let Some(handle) = self.thread.take() {
                match unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) } {
                    Ok(()) => log::log("hotkey：送出 WM_QUIT"),
                    Err(error) => log::log(&format!("hotkey：送出 WM_QUIT 失敗：{error}")),
                }
                match handle.join() {
                    Ok(()) => log::log("hotkey：專用執行緒已 join"),
                    Err(_) => log::log("hotkey：專用執行緒 panic，已結束"),
                }
            }
            if !self.target.0.is_null() {
                unsafe {
                    set_userdata(self.target, 0);
                    let pointer = std::mem::replace(&mut self.ctx, std::ptr::null_mut());
                    if !pointer.is_null() {
                        drop(Box::from_raw(pointer));
                    }
                    let _ = DestroyWindow(self.target);
                    let _ = UnregisterClassW(TARGET_CLASS, Some(self.instance));
                }
                self.target = HWND::default();
            }
        }
    }

    unsafe extern "system" fn target_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_FORWARD_HOTKEY => {
                let _ = crate::guarded("HotkeyForward", || {
                    let pointer = userdata(hwnd) as *const TargetCtx;
                    if let Some(ctx) = pointer.as_ref() {
                        loop {
                            let next = ctx
                                .queue
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .pop_front();
                            let Some(hotkey) = next else { break };
                            if let Some(service) = ctx.service.as_ref() {
                                service.hotkey_pressed(&hotkey);
                            }
                        }
                    }
                    Ok(())
                });
                LRESULT(0)
            }
            // 這是宿主的訊息迴圈：不要讓 DefWindowProc 影響宿主的 WM_QUIT 或視窗鏈。
            WM_DESTROY | WM_NCDESTROY => LRESULT(0),
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }

    /// 建立失敗時的收尾：轉送視窗、ctx、類別。
    unsafe fn discard_target(target: HWND, instance: HINSTANCE, ctx: *mut TargetCtx) {
        set_userdata(target, 0);
        if !ctx.is_null() {
            drop(Box::from_raw(ctx));
        }
        let _ = DestroyWindow(target);
        let _ = UnregisterClassW(TARGET_CLASS, Some(instance));
    }

    /// 熱鍵執行緒目前的狀態指標；只在該執行緒（含 wndproc）使用。
    unsafe fn state_at(hwnd: HWND) -> Option<&'static mut ThreadState> {
        let pointer = userdata(hwnd) as *mut ThreadState;
        pointer.as_mut()
    }

    /// `SetWindowLongPtrW`／`GetWindowLongPtrW` 在 32-bit target 收 `i32`。
    #[cfg(target_pointer_width = "64")]
    unsafe fn set_userdata(hwnd: HWND, value: isize) {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, value);
    }

    #[cfg(target_pointer_width = "32")]
    unsafe fn set_userdata(hwnd: HWND, value: isize) {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, value as i32);
    }

    #[cfg(target_pointer_width = "64")]
    unsafe fn userdata(hwnd: HWND) -> isize {
        GetWindowLongPtrW(hwnd, GWLP_USERDATA)
    }

    #[cfg(target_pointer_width = "32")]
    unsafe fn userdata(hwnd: HWND) -> isize {
        GetWindowLongPtrW(hwnd, GWLP_USERDATA) as isize
    }
}

#[cfg(windows)]
pub use win::HotkeyWindow;

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_hotkey(chord: &str) -> Result<Hotkey, HotkeyError> {
        Hotkey::parse("symbol_menu", chord)
    }

    #[test]
    fn default_chords_map_to_oem_keys() {
        let comma = parse_hotkey("ctrl+alt+,").unwrap();
        assert!(comma.ctrl && comma.alt && !comma.shift);
        assert_eq!(comma.key, ",");
        assert_eq!(comma.vk, VK_OEM_COMMA);
        assert_eq!(comma.chord, "ctrl+alt+,");

        let period = parse_hotkey("ctrl+alt+.").unwrap();
        assert_eq!(period.key, ".");
        assert_eq!(period.vk, VK_OEM_PERIOD);
    }

    #[test]
    fn letters_are_uppercase_virtual_keys() {
        let hotkey = parse_hotkey("ctrl+alt+s").unwrap();
        assert_eq!(hotkey.vk, u32::from(b'S'));
        assert_eq!(hotkey.key, "s");
        let hotkey = parse_hotkey("shift+ctrl+Z").unwrap();
        assert!(hotkey.shift && hotkey.ctrl);
        assert_eq!(hotkey.vk, u32::from(b'Z'));
        assert_eq!(hotkey.chord, "ctrl+shift+z");
    }

    #[test]
    fn shift_combinations_and_named_keys_are_supported() {
        let hotkey = parse_hotkey("ctrl+shift+space").unwrap();
        assert_eq!(hotkey.vk, VK_SPACE);
        assert!(hotkey.shift);
        let hotkey = parse_hotkey("ctrl+alt+f12").unwrap();
        assert_eq!(hotkey.vk, VK_F1 + 11);
        let hotkey = parse_hotkey("ctrl+alt+pageup").unwrap();
        assert_eq!(hotkey.vk, VK_PRIOR);
    }

    #[test]
    fn invalid_chords_are_rejected() {
        assert_eq!(parse_hotkey(""), Err(HotkeyError::Chord(ChordError::Empty)));
        assert_eq!(
            parse_hotkey("a"),
            Err(HotkeyError::Chord(ChordError::NoModifier))
        );
        assert_eq!(
            parse_hotkey("shift+a"),
            Err(HotkeyError::Chord(ChordError::ShiftOnly))
        );
        assert!(matches!(
            parse_hotkey("ctrl+alt+not-a-key"),
            Err(HotkeyError::Chord(ChordError::UnknownKey { .. }))
        ));
    }

    #[test]
    fn unsupported_keys_are_skipped_not_registered() {
        // 合法的單一字元，但沒有 Win32 虛擬鍵。
        assert!(matches!(
            parse_hotkey("ctrl+alt+ㄅ"),
            Err(HotkeyError::Unsupported { .. })
        ));
        // `?` 需要 Shift 才打得出來；chord 沒寫 shift 時不能註冊。
        assert!(matches!(
            parse_hotkey("ctrl+alt+?"),
            Err(HotkeyError::Unsupported { .. })
        ));
        assert_eq!(parse_hotkey("ctrl+alt+shift+?").unwrap().vk, VK_OEM_2);
    }

    #[test]
    fn default_keybinds_produce_two_registrations() {
        let (plan, skipped) = plan(&Keybinds::default()).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].action, "symbol_menu");
        assert_eq!(plan[0].chord, "ctrl+alt+,");
        assert_eq!(plan[1].action, "emoji_palette");
        assert_eq!(plan[1].chord, "ctrl+alt+.");
    }

    #[test]
    fn duplicate_physical_combinations_are_deduped() {
        let (plan, skipped) = plan_pairs(vec![
            ("symbol_menu".to_string(), "ctrl+alt+shift+1".to_string()),
            ("emoji_palette".to_string(), "ctrl+alt+shift+!".to_string()),
            ("symbol_menu".to_string(), "ctrl+alt+,".to_string()),
        ]);
        assert_eq!(plan.len(), 2, "同一個 VK＋修飾鍵只註冊一次");
        assert_eq!(plan[0].chord, "ctrl+alt+shift+1");
        assert_eq!(plan[1].chord, "ctrl+alt+,");
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].reason.contains("同一個組合"));
    }

    #[test]
    fn unsupported_entries_are_reported_with_their_chord() {
        let (plan, skipped) = plan_pairs(vec![
            ("symbol_menu".to_string(), "ctrl+alt+ㄅ".to_string()),
            ("emoji_palette".to_string(), "ctrl+alt+.".to_string()),
        ]);
        assert_eq!(plan.len(), 1);
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].action, "symbol_menu");
        assert_eq!(skipped[0].chord, "ctrl+alt+ㄅ");
        assert!(skipped[0].reason.contains("無法註冊"));
    }

    #[test]
    fn key_event_carries_configured_modifiers() {
        let key = parse_hotkey("ctrl+alt+,").unwrap().to_key();
        assert!(key.ctrl && key.alt && !key.shift);
        assert_eq!(key.code, KeyCode::Char(','));
        assert_eq!(key.text, Some(','));

        let key = parse_hotkey("ctrl+shift+enter").unwrap().to_key();
        assert!(key.ctrl && key.shift && !key.alt);
        assert_eq!(key.code, KeyCode::Enter);
        assert_eq!(key.text, None);
    }

    #[test]
    fn settings_without_keybinds_use_defaults() {
        let dir = std::env::temp_dir().join(format!(
            "jc-tsf-hotkey-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, br#"{"selection":"1234"}"#).unwrap();
        assert_eq!(read_keybinds(&path).unwrap(), Keybinds::default());
        std::fs::write(
            &path,
            br#"{"keybinds":{"symbol_menu":"ctrl+shift+s","emoji_palette":"ctrl+alt+."}}"#,
        )
        .unwrap();
        let keybinds = read_keybinds(&path).unwrap();
        assert_eq!(keybinds.symbol_menu, "ctrl+shift+s");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 熱鍵執行緒指令介面的假 sink：記錄順序，可指定失敗的 id。
    #[derive(Default)]
    struct FakeSink {
        attempts: usize,
        registered: Vec<(i32, String)>,
        unregistered: Vec<i32>,
        pressed: Vec<String>,
        fail: Vec<i32>,
    }

    impl Sink for FakeSink {
        fn register(&mut self, id: i32, hotkey: &Hotkey) -> Result<(), String> {
            self.attempts += 1;
            if self.fail.contains(&id) {
                return Err("fake 失敗".to_string());
            }
            self.registered.push((id, hotkey.chord.clone()));
            Ok(())
        }

        fn unregister(&mut self, id: i32) {
            self.unregistered.push(id);
        }

        fn press(&mut self, hotkey: &Hotkey) {
            self.pressed.push(hotkey.chord.clone());
        }
    }

    fn default_plan() -> Vec<Hotkey> {
        plan(&Keybinds::default()).unwrap().0
    }

    #[test]
    fn machine_only_registers_while_foreground() {
        let mut machine = Machine::new();
        let mut sink = FakeSink::default();
        machine.apply(&mut sink, default_plan());
        assert_eq!(sink.attempts, 0, "背景時只記計畫，不註冊");
        assert!(machine.registered().is_empty());

        machine.enable(&mut sink);
        assert_eq!(
            sink.registered,
            vec![
                (ID_BASE, "ctrl+alt+,".to_string()),
                (ID_BASE + 1, "ctrl+alt+.".to_string()),
            ]
        );

        machine.enable(&mut sink);
        assert_eq!(sink.attempts, 2, "已註冊的不重複註冊");
        assert_eq!(machine.registered(), vec![ID_BASE, ID_BASE + 1]);
    }

    #[test]
    fn disable_unregisters_all_and_enable_registers_again() {
        let mut machine = Machine::new();
        let mut sink = FakeSink::default();
        machine.apply(&mut sink, default_plan());
        machine.enable(&mut sink);
        machine.disable(&mut sink);
        assert_eq!(sink.unregistered, vec![ID_BASE, ID_BASE + 1]);
        assert!(machine.registered().is_empty());
        assert!(!machine.is_enabled());

        machine.enable(&mut sink);
        assert_eq!(machine.registered(), vec![ID_BASE, ID_BASE + 1]);
    }

    #[test]
    fn applying_a_new_plan_replaces_registrations() {
        let mut machine = Machine::new();
        let mut sink = FakeSink::default();
        machine.apply(&mut sink, default_plan());
        machine.enable(&mut sink);
        sink.registered.clear();
        let next = plan_pairs(vec![(
            "emoji_palette".to_string(),
            "ctrl+alt+.".to_string(),
        )])
        .0;
        machine.apply(&mut sink, next);
        assert_eq!(sink.unregistered, vec![ID_BASE, ID_BASE + 1]);
        assert_eq!(
            sink.registered,
            vec![(ID_BASE, "ctrl+alt+.".to_string())],
            "新計畫從 id 起點重新編號"
        );
    }

    #[test]
    fn only_registered_ids_are_dispatched() {
        let mut machine = Machine::new();
        let mut sink = FakeSink::default();
        machine.apply(&mut sink, default_plan());
        assert!(!machine.hotkey(&mut sink, ID_BASE), "尚未註冊時忽略");

        machine.enable(&mut sink);
        assert!(machine.hotkey(&mut sink, ID_BASE + 1));
        assert_eq!(sink.pressed, vec!["ctrl+alt+.".to_string()]);
        assert!(!machine.hotkey(&mut sink, ID_BASE + 9));
        assert_eq!(sink.pressed.len(), 1);
    }

    #[test]
    fn failed_registration_is_retried_on_the_next_tick() {
        let mut machine = Machine::new();
        let mut sink = FakeSink {
            fail: vec![ID_BASE],
            ..Default::default()
        };
        machine.apply(&mut sink, default_plan());
        machine.enable(&mut sink);
        assert_eq!(sink.attempts, 2);
        assert_eq!(machine.registered(), vec![ID_BASE + 1]);

        sink.fail.clear();
        machine.register_missing(&mut sink);
        assert_eq!(sink.attempts, 3);
        let mut ids = machine.registered();
        ids.sort();
        assert_eq!(ids, vec![ID_BASE, ID_BASE + 1]);
    }
}
