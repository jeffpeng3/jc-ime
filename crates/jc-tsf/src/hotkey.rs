//! 設定檔的快捷鍵改用 Win32 `RegisterHotKey` 攔截。
//!
//! TSF 的 key sink 收不到部分 Alt 組合（AltGr 會被當成 Ctrl＋Alt，部分宿主
//! 直接把 Alt 組合交給選單）。這裡在 TSF 行程內開一個 message-only window，
//! 依 `settings.json` 的 `[keybinds]` 註冊全域快捷鍵；收到 `WM_HOTKEY` 後把
//! 對應的 [`Hotkey`] 交給 engine，由 keymap 執行 command。
//!
//! 只有已設定的組合會被系統攔下，其餘按鍵照舊走 key sink。註冊失敗（被其他
//! 程式占用、F12 等保留鍵）一定寫日誌，並保留原本的 key sink 路徑當 fallback。
//!
//! 解析、註冊清單與去重是純資料，Linux 也編、也測；Win32 的視窗與
//! `RegisterHotKey` 只在 Windows 編譯。

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
    //! Win32 message-only window 與 `RegisterHotKey`。

    use std::cell::{Cell, RefCell};

    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, KillTimer,
        RegisterClassW, SetTimer, SetWindowLongPtrW, UnregisterClassW, GWLP_USERDATA, HWND_MESSAGE,
        WINDOW_EX_STYLE, WINDOW_STYLE, WM_DESTROY, WM_HOTKEY, WM_NCDESTROY, WM_TIMER, WNDCLASSW,
    };
    use windows_core::{w, PCWSTR};

    use super::{
        plan, read_keybinds, settings_path, settings_stamp, Hotkey, Keybinds, Skipped, Stamp,
    };
    use crate::log;

    const CLASS: PCWSTR = w!("jcHotkeyWindow");
    /// 設定檔重載與註冊重試的 timer。
    const TIMER_ID: usize = 0x0c1a;
    const TIMER_MS: u32 = 2_000;
    /// `RegisterHotKey` 的 id 起點；同一條執行緒的其他程式各自用自己的 hwnd。
    const ID_BASE: i32 = 0x4a00;

    #[derive(Clone)]
    struct Registered {
        id: i32,
        hotkey: Hotkey,
    }

    /// 一個服務的全域快捷鍵視窗。
    ///
    /// 視窗由 [`crate::service::TextService`] 的 `State` 持有；`service` 指標
    /// 只在同一條 TSF 執行緒上使用，且視窗一定比服務先銷毀。
    pub struct HotkeyWindow {
        hwnd: HWND,
        instance: HINSTANCE,
        service: *const crate::service::TextService_Impl,
        enabled: Cell<bool>,
        plan: RefCell<Vec<Hotkey>>,
        registered: RefCell<Vec<Registered>>,
        stamp: Cell<Option<Stamp>>,
        last_failure: RefCell<Option<String>>,
    }

    impl HotkeyWindow {
        /// 建立 message-only window 並依設定註冊；不在前景時先不註冊。
        pub fn new(service: &crate::service::TextService_Impl) -> windows_core::Result<Box<Self>> {
            unsafe {
                let instance: HINSTANCE = GetModuleHandleW(None)?.into();
                // 類別是行程內的；重複註冊由系統忽略錯誤。
                let _ = RegisterClassW(&WNDCLASSW {
                    lpfnWndProc: Some(window_proc),
                    hInstance: instance,
                    lpszClassName: CLASS,
                    ..Default::default()
                });
                let hwnd = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    CLASS,
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
                )?;
                let window = Box::new(Self {
                    hwnd,
                    instance,
                    service,
                    enabled: Cell::new(false),
                    plan: RefCell::new(Vec::new()),
                    registered: RefCell::new(Vec::new()),
                    stamp: Cell::new(None),
                    last_failure: RefCell::new(None),
                });
                set_userdata(hwnd, &*window as *const Self as isize);
                let _ = SetTimer(Some(hwnd), TIMER_ID, TIMER_MS, None);
                // 只有前景行程能持有全域組合；背景行程先註冊會占住組合，
                // 前景的 TSF 反而收不到 WM_HOTKEY。
                let foreground = crate::candidate::foreground_belongs_to(std::process::id());
                log::log(&format!(
                    "hotkey：建立訊息視窗 hwnd={:#x} foreground={foreground}",
                    hwnd.0 as usize
                ));
                window.enabled.set(foreground);
                window.reload(true);
                Ok(window)
            }
        }

        /// 這個 TSF 取得前景：重新載入設定並註冊。
        pub fn enable(&self) {
            self.enabled.set(true);
            if !self.reload_if_changed() {
                self.register_missing();
            }
        }

        /// 失去前景：反註冊，避免霸占其他行程的快捷鍵。
        pub fn disable(&self) {
            self.enabled.set(false);
            self.unregister_all();
        }

        /// 設定檔變了才重讀，並重新註冊。
        fn reload_if_changed(&self) -> bool {
            let stamp = settings_path().as_deref().and_then(settings_stamp);
            if stamp == self.stamp.get() {
                return false;
            }
            self.reload(false);
            true
        }

        fn reload(&self, startup: bool) {
            let Some(path) = settings_path() else {
                if startup {
                    log::log("hotkey：找不到 LOCALAPPDATA，只能使用預設綁定");
                    self.apply_defaults();
                }
                self.stamp.set(None);
                return;
            };
            let stamp = settings_stamp(&path);
            self.stamp.set(stamp);
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

        fn apply_defaults(&self) {
            let (plan, skipped) = plan(&Keybinds::default()).expect("預設 keybinds 一定合法");
            self.apply(plan, skipped);
        }

        fn apply(&self, plan: Vec<Hotkey>, skipped: Vec<Skipped>) {
            for skip in skipped {
                log::log(&format!(
                    "hotkey 略過 {}（{}）：{}",
                    skip.action, skip.chord, skip.reason
                ));
            }
            self.unregister_all();
            *self.plan.borrow_mut() = plan;
            self.register_missing();
        }

        /// 註冊還沒成功的項目；失敗會留下可讀的原因，同一組失敗不重複洗版。
        fn register_missing(&self) {
            if !self.enabled.get() {
                return;
            }
            let plan = self.plan.borrow().clone();
            let mut registered = self.registered.borrow_mut();
            let mut failures = Vec::new();
            for (index, hotkey) in plan.iter().enumerate() {
                let id = ID_BASE + index as i32;
                if registered.iter().any(|entry| entry.id == id) {
                    continue;
                }
                let result = unsafe {
                    RegisterHotKey(Some(self.hwnd), id, hotkey.win_modifiers(), hotkey.vk)
                };
                match result {
                    Ok(()) => registered.push(Registered {
                        id,
                        hotkey: hotkey.clone(),
                    }),
                    Err(error) => failures.push(format!(
                        "{}（{} vk={:#04x}）：{error}",
                        hotkey.chord, hotkey.action, hotkey.vk
                    )),
                }
            }
            drop(registered);
            self.report(&failures);
        }

        fn report(&self, failures: &[String]) {
            if failures.is_empty() {
                if self.last_failure.borrow_mut().take().is_some() {
                    log::log("hotkey：所有設定的快捷鍵都註冊成功");
                }
                return;
            }
            let summary = failures.join("；");
            let mut last = self.last_failure.borrow_mut();
            if last.as_deref() == Some(summary.as_str()) {
                return;
            }
            log::log(&format!(
                "hotkey 註冊失敗（可能被其他程式占用或系統保留）：{summary}"
            ));
            *last = Some(summary);
        }

        fn unregister_all(&self) {
            let registered = std::mem::take(&mut *self.registered.borrow_mut());
            for entry in registered {
                if let Err(error) = unsafe { UnregisterHotKey(Some(self.hwnd), entry.id) } {
                    log::log(&format!(
                        "hotkey：反註冊 {} 失敗：{error}",
                        entry.hotkey.chord
                    ));
                }
            }
        }

        /// `WM_HOTKEY`：只處理目前註冊表裡的 id，過期的佇列訊息直接忽略。
        fn press(&self, id: i32) {
            let hotkey = self
                .registered
                .borrow()
                .iter()
                .find(|entry| entry.id == id)
                .map(|entry| entry.hotkey.clone());
            let Some(hotkey) = hotkey else {
                log::log(&format!("hotkey：收到未註冊的 id={id}，忽略"));
                return;
            };
            // SAFETY: 視窗由 service 的 State 持有，不可能活得比 service 久；
            // 訊息由同一條 TSF 執行緒送出。
            if let Some(service) = unsafe { self.service.as_ref() } {
                service.hotkey_pressed(&hotkey);
            }
        }

        fn tick(&self) {
            if !self.enabled.get() {
                return;
            }
            if !self.reload_if_changed() {
                // 上一次被其他程式占用時，每秒重試一次；失敗訊息已去重。
                self.register_missing();
            }
        }
    }

    impl Drop for HotkeyWindow {
        fn drop(&mut self) {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_ID);
            }
            self.disable();
            unsafe {
                set_userdata(self.hwnd, 0);
                let _ = DestroyWindow(self.hwnd);
                // 同一行程還有多個 service 時會失敗，留給最後一個離開才移除。
                let _ = UnregisterClassW(CLASS, Some(self.instance));
            }
        }
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_HOTKEY => {
                let _ = crate::guarded("Hotkey", || {
                    if let Some(window) = window_at(hwnd) {
                        window.press(wparam.0 as i32);
                    }
                    Ok(())
                });
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_ID => {
                if let Some(window) = window_at(hwnd) {
                    window.tick();
                }
                LRESULT(0)
            }
            // 這是宿主的訊息迴圈：不要讓 DefWindowProc 的 WM_DESTROY 預設處理
            // 影響宿主的 WM_QUIT 或視窗鏈。
            WM_DESTROY | WM_NCDESTROY => LRESULT(0),
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
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

    unsafe fn window_at(hwnd: HWND) -> Option<&'static HotkeyWindow> {
        let pointer = userdata(hwnd) as *const HotkeyWindow;
        pointer.as_ref()
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
}
