//! 實體按鍵到 `jc_proto::Key` 的轉換。
//!
//! 注音鍵位表不另外抄一份，直接用 `jc_core::zh::Keyboard`（標準／大千排列）。
//! 這裡只負責三件事：
//!
//! 1. 判斷哪個鍵要攔（[`Modifiers::pass_through`] 先行放行 Ctrl／Alt／Win）。
//! 2. 把虛擬鍵碼轉成實際字元（`ToUnicodeEx`，尊重鍵盤配置與 Shift）。
//! 3. 把字元換成引擎看得懂的注音或全形標點。
//!
//! 只有組字中才攔具名鍵（Enter、Esc、Backspace、方向鍵…）；沒有組字時這些鍵
//! 一律放行，否則應用程式會收不到換行與退格。

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

/// 單一虛擬鍵是否按著。
fn pressed(key: VIRTUAL_KEY) -> bool {
    // SAFETY: GetKeyState 只讀目前執行緒的鍵盤狀態。
    unsafe { GetKeyState(key.0 as i32) < 0 }
}

/// 把按鍵事件轉成要送給引擎的按鍵序列。
///
/// 回 `None` 代表這個鍵不歸我們處理，呼叫端必須放行。回 `Some` 且內容為多個
/// 按鍵的情形成立於多字元標點（例如 `^` 對應刪節號 `……`）。
///
/// `composing` 是「引擎目前有組字或候選」；它決定具名鍵要不要攔。
pub fn translate(vk: u16, scan: u32, composing: bool) -> Option<Vec<Key>> {
    let modifiers = Modifiers::current();
    if modifiers.pass_through() {
        return None;
    }

    if let Some(code) = named_key(vk, modifiers.shift, composing) {
        return Some(vec![Key::new(code, None, modifiers.shift)]);
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
/// 注音鍵位優先，其次是全形標點；都不是的時候，組字中當字面送，沒組字就放行
/// （例如 `=` 與 `` ` `` 在沒有候選窗的第一版沒有用途）。
fn map_char(ch: char, shift: bool, composing: bool) -> Option<Vec<Key>> {
    let keyboard = Keyboard::standard();
    if let Some(symbol) = keyboard.symbol(ch.to_ascii_lowercase()) {
        return Some(vec![Key::char(symbol, shift)]);
    }
    if let Some(punct) = keyboard.punct(ch) {
        return Some(punct.chars().map(|c| Key::char(c, shift)).collect());
    }
    if composing && !ch.is_control() {
        return Some(vec![Key::char(ch, shift)]);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_map_to_bopomofo() {
        let keys = map_char('j', false, false).expect("j 是注音鍵");
        assert_eq!(keys, vec![Key::char('ㄨ', false)]);

        let keys = map_char('1', false, false).expect("1 是注音鍵");
        assert_eq!(keys, vec![Key::char('ㄅ', false)]);
    }

    #[test]
    fn shift_digits_produce_fullwidth_punct() {
        let keys = map_char('!', true, false).expect("! 是全形驚嘆號");
        assert_eq!(keys, vec![Key::char('！', true)]);
    }

    #[test]
    fn multi_character_punct_becomes_several_keys() {
        let keys = map_char('^', true, false).expect("^ 是刪節號");
        assert_eq!(keys, vec![Key::char('…', true), Key::char('…', true)]);
    }

    #[test]
    fn plain_chars_pass_through_without_composition() {
        assert_eq!(map_char('=', false, false), None);
        assert_eq!(
            map_char('=', false, true),
            Some(vec![Key::char('=', false)])
        );
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
}
