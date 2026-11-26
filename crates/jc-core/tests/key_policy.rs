//! `KeyPolicy` 的優先序與各 lane 的按鍵契約。
//!
//! 只驗使用者看得到的行為：候選介面先於 lane、Esc 層級、Backspace 單位、
//! 數字鍵在選取模式與注音 pending 的差異。

use std::sync::Arc;

use jc_core::lattice::{InMemoryLexicon, LexEntry};
use jc_core::session::{Session, SessionConfig};
use jc_core::{Key, KeyCode, Mode, Snapshot};

fn press(session: &mut Session, key: Key) -> Snapshot {
    session.press(key)
}

fn char_key(session: &mut Session, ch: char) -> Snapshot {
    press(session, Key::char(ch, false))
}

fn code_key(session: &mut Session, code: KeyCode) -> Snapshot {
    press(session, Key::new(code, None, false))
}

fn press_chars(session: &mut Session, text: &str) {
    for ch in text.chars() {
        char_key(session, ch);
    }
}

fn zh_session() -> Session {
    let mut lexicon = InMemoryLexicon::new();
    for (surface, reading, cost) in [("我", "ㄨㄛˇ", 0), ("窩", "ㄨㄛˇ", 10)] {
        lexicon.push(LexEntry::new(surface, reading, cost, 0));
    }
    let mut config = SessionConfig::new(Mode::Mixed);
    config.zh_lexicon = Some(Arc::new(lexicon));
    Session::new(config)
}

/// 候選窗開著時 Esc 先關窗，組字留著；第二次才清整段。
#[test]
fn test_esc_closes_candidate_before_composition() {
    let mut s = zh_session();
    press_chars(&mut s, "ji3");
    code_key(&mut s, KeyCode::Down);
    assert_eq!(s.preedit(), "我");

    let first = code_key(&mut s, KeyCode::Escape);
    assert_eq!(first.preedit, "我", "第一層 Esc 只關候選窗");
    let second = code_key(&mut s, KeyCode::Escape);
    assert_eq!(second.preedit, "", "第二層 Esc 才清整段組字");
}

/// 數字鍵在 ↓ 選取模式選字；注音 pending 時同一鍵仍是聲調。
#[test]
fn test_digit_selects_candidate_only_in_selection_mode() {
    let mut s = zh_session();
    press_chars(&mut s, "ji3");
    code_key(&mut s, KeyCode::Down);
    let selected = char_key(&mut s, '2');
    assert_eq!(selected.preedit, "ji3", "選取模式的 2 選回原始按鍵");
    assert_eq!(s.commit(), "ji3");

    let mut pending = zh_session();
    press_chars(&mut pending, "ji");
    let toned = char_key(&mut pending, '3');
    assert_eq!(toned.preedit, "我", "沒有選取模式時 3 是聲調，不是選字");
}

/// Tab 在選取模式換下一個候選，不吃掉按鍵。
#[test]
fn test_tab_moves_highlight_in_selection_mode() {
    let mut s = zh_session();
    press_chars(&mut s, "ji3");
    code_key(&mut s, KeyCode::Down);
    let before = s.candidates()[0].surface.clone();
    let snap = code_key(&mut s, KeyCode::Tab);
    assert_eq!(snap.highlighted, 1, "Tab 移到第二項");
    assert_eq!(s.candidates()[1].surface, "ji3");
    assert_ne!(s.candidates()[snap.highlighted].surface, before);
}

/// 注音 pending 的 Space 完成音節；Backspace 退一個注音單位；Esc 清 pending。
#[test]
fn test_zh_pending_space_backspace_escape() {
    let mut s = zh_session();
    press_chars(&mut s, "ji");
    assert_eq!(s.preedit(), "ㄨㄛ");
    let spaced = code_key(&mut s, KeyCode::Space);
    assert_eq!(
        spaced.preedit, "ㄨㄛ",
        "沒有聲調的 pending 空白不選字、不改變注音"
    );

    let mut backspace = zh_session();
    press_chars(&mut backspace, "ji");
    let deleted = code_key(&mut backspace, KeyCode::Backspace);
    assert_eq!(deleted.preedit, "ㄨ", "Backspace 只退一個注音單位");

    let mut escape = zh_session();
    press_chars(&mut escape, "ji");
    let cleared = code_key(&mut escape, KeyCode::Escape);
    assert_eq!(cleared.preedit, "", "Esc 先清未完成的注音");
}

/// 字面尾巴的 Backspace 只刪一個 rune；Esc 丟掉整段。
#[test]
fn test_literal_tail_backspace_and_escape() {
    let mut s = Session::new(SessionConfig::new(Mode::Mixed));
    s.literal("ab");
    assert_eq!(s.preedit(), "ab");

    let deleted = code_key(&mut s, KeyCode::Backspace);
    assert_eq!(deleted.preedit, "a", "只刪一個 rune");
    let cleared = code_key(&mut s, KeyCode::Escape);
    assert_eq!(cleared.preedit, "", "字面尾巴的 Esc 清整段");
}

/// 候選窗沒開時 Enter 直接送出整段，不代表候選介面搶走 Enter。
#[test]
fn test_enter_without_window_commits() {
    let mut s = zh_session();
    press_chars(&mut s, "ji3");
    let snap = code_key(&mut s, KeyCode::Enter);
    assert_eq!(snap.preedit, "");
    assert_eq!(s.commit(), "我");
}
