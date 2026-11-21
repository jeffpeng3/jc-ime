//! 實體按鍵到 `jc_proto::Key` 的轉換。
//!
//! wrapper 只送「實體按鍵產生的字元」，不自己把字母翻成注音符號：`jc-core`
//! 的 `Keyboard::standard()` 才是注音鍵位的唯一來源。這樣同一套按鍵在注音、
//! 日文羅馬字與英文模式都能由引擎自己判定；數字鍵也才能同時是注音鍵與候選
//! 選字鍵，由引擎決定。
//!
//! 這裡只負責四件事：
//!
//! 1. 判斷哪個鍵要攔（[`Modifiers::pass_through`] 先行放行 Ctrl／Alt／Win）。
//! 2. 把虛擬鍵碼轉成實際字元（`ToUnicodeEx`，尊重鍵盤配置與 Shift）。
//! 3. 組字中把具名鍵（Enter、Esc、方向鍵…）換成 `KeyCode`。
//! 4. 沒有組字時把 Enter、Space、Tab、Backspace 等放行給應用程式。
//!
//! 字母一律轉小寫再送，Shift 只放在 `shift` 欄位；注音鍵位是大千排列的
//! 小寫字母，Shift 不改變注音。Shift 過的符號（`<`、`?`、`^`…）照原字元
//! 送，引擎會依標點風格決定全形輸出。

use jc_core::zh::Keyboard;
use jc_proto::{Key, KeyCode};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, GetKeyboardLayout, GetKeyboardState, ToUnicodeEx, VIRTUAL_KEY, VK_BACK,
    VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_LWIN, VK_MENU, VK_NEXT,
    VK_PRIOR, VK_RETURN, VK_RIGHT, VK_RWIN, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};

/// 目前的修飾鍵狀態。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// Shift 是否按著。
    pub shift: bool,
    /// Ctrl 是否按著。
    pub ctrl: bool,
    /// Alt 是否按著。
    pub alt: bool,
    /// Win 鍵是否按著。
    pub win: bool,
}

impl Modifiers {
    /// 從目前的執行緒鍵盤狀態取得修飾鍵。
    pub fn current() -> Self {
        Self {
            shift: pressed(VK_SHIFT),
            ctrl: pressed(VK_CONTROL),
            alt: pressed(VK_MENU),
            win: pressed(VK_LWIN) || pressed(VK_RWIN),
        }
    }

    /// Ctrl／Alt／Win 一律放行：這些是應用程式或系統的快捷鍵，不是輸入法的。
    pub fn pass_through(self) -> bool {
        self.ctrl || self.alt || self.win
    }
}

/// 左 Shift 的虛擬鍵碼（`VK_LSHIFT`）。
pub const VK_LSHIFT: u16 = 0xA0;
/// 右 Shift 的虛擬鍵碼（`VK_RSHIFT`）。
pub const VK_RSHIFT: u16 = 0xA1;

/// 是否為用來切換中英的左右 Shift。
pub fn is_shift(vk: u16) -> bool {
    vk == VK_LSHIFT || vk == VK_RSHIFT
}

/// 單一虛擬鍵是否按著。
fn pressed(key: VIRTUAL_KEY) -> bool {
    // SAFETY: GetKeyState 只讀目前執行緒的鍵盤狀態。
    unsafe { GetKeyState(key.0 as i32) < 0 }
}

/// 單按 Shift 的狀態機。
///
/// 按左或右 Shift 時開始記錄；其間若有其他按鍵先按下，這次就不算單按。
/// key-up 由 [`ShiftTap::key_up`] 判定是否要送 `Request::Toggle`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShiftTap {
    /// 正在追蹤的 Shift 虛擬鍵碼；`None` 代表沒有。
    down: Option<u16>,
    /// Shift 按下之後是否出現過其他按鍵。
    touched: bool,
}

impl ShiftTap {
    /// 重設狀態；焦點改變或引擎斷線時呼叫。
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// 記錄 key-down；回傳是否為左右 Shift。
    pub fn key_down(&mut self, vk: u16) -> bool {
        if is_shift(vk) {
            self.down = Some(vk);
            self.touched = false;
            true
        } else {
            if self.down.is_some() {
                self.touched = true;
            }
            false
        }
    }

    /// 記錄 key-up；回傳 true 代表構成單按 Shift，應切換 En。
    pub fn key_up(&mut self, vk: u16) -> bool {
        if !is_shift(vk) {
            return false;
        }
        let tap = self.down == Some(vk) && !self.touched;
        self.down = None;
        self.touched = false;
        tap
    }
}

/// 把按鍵事件轉成要送給引擎的按鍵序列。
///
/// 回 `None` 代表這個鍵不歸我們處理，呼叫端必須放行。回 `Some` 目前一定是
/// 單一按鍵；多字元標點（例如 `^` 對應刪節號 `……`）由引擎展開。
///
/// `composing` 是「引擎目前有組字或候選」；它決定具名鍵與空白鍵要不要攔。
pub fn translate(vk: u16, scan: u32, composing: bool) -> Option<Vec<Key>> {
    let modifiers = Modifiers::current();
    if modifiers.pass_through() {
        return None;
    }

    if let Some(code) = named_key(vk, modifiers.shift, composing) {
        return Some(vec![Key::new(code, None, modifiers.shift)]);
    }

    // 空白只在組字中由 `named_key` 攔；沒組字時要留給應用程式，否則使用者
    // 打不出空白。
    if vk == VK_SPACE.0 {
        return None;
    }

    let ch = char_of(vk, scan)?;
    map_char(ch, modifiers.shift, composing)
}

/// 具名鍵對應；沒有組字時一律不攔。
fn named_key(vk: u16, shift: bool, composing: bool) -> Option<KeyCode> {
    if vk == VK_TAB.0 && shift {
        return composing.then_some(KeyCode::ShiftTab);
    }
    if !composing {
        return None;
    }
    let code = match vk {
        v if v == VK_RETURN.0 => KeyCode::Enter,
        v if v == VK_ESCAPE.0 => KeyCode::Escape,
        v if v == VK_BACK.0 => KeyCode::Backspace,
        v if v == VK_DELETE.0 => KeyCode::Delete,
        v if v == VK_SPACE.0 => KeyCode::Space,
        v if v == VK_LEFT.0 => KeyCode::ArrowLeft,
        v if v == VK_RIGHT.0 => KeyCode::ArrowRight,
        v if v == VK_UP.0 => KeyCode::Up,
        v if v == VK_DOWN.0 => KeyCode::Down,
        v if v == VK_PRIOR.0 => KeyCode::PageUp,
        v if v == VK_NEXT.0 => KeyCode::PageDown,
        v if v == VK_TAB.0 => KeyCode::Tab,
        // Home／End 不在第一版的攔截範圍：組字游標固定在前端，沒有東西可移。
        v if v == VK_HOME.0 || v == VK_END.0 => return None,
        _ => return None,
    };
    Some(code)
}

/// 用目前鍵盤配置解出這個虛擬鍵產生的字元。
///
/// 死鍵（`ToUnicodeEx` 回負值）視為不處理，直接放行。
fn char_of(vk: u16, scan: u32) -> Option<char> {
    let mut state = [0u8; 256];
    // SAFETY: state 是 256 bytes 的完整緩衝。
    unsafe { GetKeyboardState(&mut state) }.ok()?;
    let layout = unsafe { GetKeyboardLayout(0) };
    let mut buffer = [0u16; 4];
    // SAFETY: buffer 長度正確；layout 來自 GetKeyboardLayout。
    let count = unsafe { ToUnicodeEx(vk as u32, scan, &state, &mut buffer, 0, Some(layout)) };
    if count <= 0 {
        return None;
    }
    char::decode_utf16(buffer[..count as usize].iter().copied())
        .next()?
        .ok()
}

/// 把解出的字元換成引擎要的按鍵。
///
/// 注音鍵位、日文羅馬字與英文都吃實體字元，所以這裡不查表轉成注音符號。
/// 沒組字時只攔注音鍵、標點與 Shift 符號；`=`、`` ` `` 與其他字元留給
/// 應用程式或之後的選單功能。
fn map_char(ch: char, shift: bool, composing: bool) -> Option<Vec<Key>> {
    if ch.is_control() || ch == ' ' {
        return None;
    }
    let key = if ch.is_ascii_alphabetic() {
        ch.to_ascii_lowercase()
    } else {
        ch
    };
    let keyboard = Keyboard::standard();
    let phonetic = keyboard.symbol(key).is_some();
    let punctuation = keyboard.punct(ch).is_some();
    if composing || phonetic || punctuation {
        return Some(vec![Key::new(KeyCode::Char(key), Some(key), shift)]);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_keep_the_physical_key() {
        let keys = map_char('j', false, false).expect("j 是注音鍵");
        assert_eq!(keys, vec![Key::new(KeyCode::Char('j'), Some('j'), false)]);

        let keys = map_char('1', false, false).expect("1 是注音鍵");
        assert_eq!(keys, vec![Key::new(KeyCode::Char('1'), Some('1'), false)]);
    }

    #[test]
    fn uppercase_letters_are_lowercased_with_shift() {
        let keys = map_char('J', true, false).expect("Shift+j 仍是注音鍵");
        assert_eq!(keys, vec![Key::new(KeyCode::Char('j'), Some('j'), true)]);
    }

    #[test]
    fn shift_digits_stay_shifted_punctuation() {
        let keys = map_char('!', true, false).expect("! 是全形驚嘆號的鍵");
        assert_eq!(keys, vec![Key::new(KeyCode::Char('!'), Some('!'), true)]);
    }

    #[test]
    fn punctuation_is_left_to_the_engine() {
        // 引擎的標點表才會決定 `^` 是兩個刪節號，wrapper 不先展開。
        let keys = map_char('^', true, false).expect("^ 是刪節號的鍵");
        assert_eq!(keys, vec![Key::new(KeyCode::Char('^'), Some('^'), true)]);
    }

    #[test]
    fn plain_chars_pass_through_without_composition() {
        assert_eq!(map_char('=', false, false), None);
        assert_eq!(
            map_char('=', false, true),
            Some(vec![Key::new(KeyCode::Char('='), Some('='), false)])
        );
    }

    #[test]
    fn space_is_not_a_printable_key() {
        assert_eq!(map_char(' ', false, false), None);
    }

    #[test]
    fn modifiers_pass_through() {
        let ctrl = Modifiers {
            ctrl: true,
            ..Default::default()
        };
        assert!(ctrl.pass_through());
        assert!(!Modifiers::default().pass_through());
    }

    #[test]
    fn named_keys_need_composition() {
        assert_eq!(named_key(VK_RETURN.0, false, false), None);
        assert_eq!(named_key(VK_RETURN.0, false, true), Some(KeyCode::Enter));
        assert_eq!(named_key(VK_TAB.0, true, true), Some(KeyCode::ShiftTab));
        assert_eq!(named_key(VK_TAB.0, true, false), None);
        assert_eq!(named_key(VK_HOME.0, false, true), None);
    }

    #[test]
    fn is_shift_recognizes_both_sides() {
        assert!(is_shift(VK_LSHIFT));
        assert!(is_shift(VK_RSHIFT));
        assert!(!is_shift(VK_SHIFT.0));
    }

    #[test]
    fn shift_tap_fires_on_same_side_key_up() {
        let mut tap = ShiftTap::default();
        assert!(tap.key_down(VK_LSHIFT));
        assert!(tap.key_up(VK_LSHIFT));

        let mut tap = ShiftTap::default();
        assert!(tap.key_down(VK_RSHIFT));
        assert!(tap.key_up(VK_RSHIFT));
    }

    #[test]
    fn shift_tap_ignores_other_keys_and_opposite_side() {
        let mut tap = ShiftTap::default();
        tap.key_down(VK_LSHIFT);
        tap.key_down(b'J' as u16);
        assert!(!tap.key_up(VK_LSHIFT), "搭配其他鍵就不算單按");

        let mut tap = ShiftTap::default();
        tap.key_down(VK_LSHIFT);
        assert!(!tap.key_up(VK_RSHIFT), "不同邊的 key-up 不算");
    }
}
