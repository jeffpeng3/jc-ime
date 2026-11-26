//! 反引號前導符號的 Enter：組字只剩字面時要一鍵送出。

use std::sync::Arc;

use jc_core::lattice::{InMemoryLexicon, LexEntry};
use jc_core::session::{Session, SessionConfig};
use jc_core::{Key, KeyCode, Mode, Snapshot};

fn session(mode: Mode) -> Session {
    Session::new(SessionConfig::new(mode))
}

fn press(session: &mut Session, key: Key) -> Snapshot {
    session.press(key)
}

fn press_chars(session: &mut Session, chars: &str) {
    for ch in chars.chars() {
        press(session, Key::char(ch, false));
    }
}

fn enter(session: &mut Session) -> Snapshot {
    press(session, Key::new(KeyCode::Enter, None, false))
}

#[test]
fn test_enter_commits_single_literal_without_double_press() {
    let mut s = session(Mode::Mixed);
    press_chars(&mut s, "`,");
    assert_eq!(s.preedit(), "，");
    let snap = enter(&mut s);
    assert_eq!(snap.preedit, "", "Enter 要直接送出，不要留到第二次");
    assert!(snap.candidates.is_empty(), "送出後不該還開著候選窗");
    assert_eq!(s.commit(), "，");
}

#[test]
fn test_enter_commits_multiple_prefix_symbols() {
    let mut s = session(Mode::Mixed);
    press_chars(&mut s, "`;`[");
    assert_eq!(s.preedit(), "；［");
    let snap = enter(&mut s);
    assert_eq!(snap.preedit, "", "整段字面要一次送出");
    assert_eq!(s.commit(), "；［");
}

#[test]
fn test_enter_still_two_stage_when_selecting_variant() {
    let mut s = session(Mode::Mixed);
    press_chars(&mut s, "`[");
    press(&mut s, Key::new(KeyCode::Down, None, false));
    let first = enter(&mut s);
    assert_eq!(first.preedit, "「", "第一次 Enter 只套用變體，組字還在");
    let second = enter(&mut s);
    assert_eq!(second.preedit, "");
    assert_eq!(s.commit(), "「");
}

#[test]
fn test_enter_still_two_stage_for_zh_candidate_window() {
    let mut lexicon = InMemoryLexicon::new();
    for (surface, reading, cost) in [("我", "ㄨㄛˇ", 0), ("窩", "ㄨㄛˇ", 10)] {
        lexicon.push(LexEntry::new(surface, reading, cost, 0));
    }
    let mut config = SessionConfig::new(Mode::Mixed);
    config.zh_lexicon = Some(Arc::new(lexicon));
    let mut s = Session::new(config);

    press_chars(&mut s, "ji3");
    press(&mut s, Key::new(KeyCode::Down, None, false));
    let first = enter(&mut s);
    assert_eq!(first.preedit, "我", "候選窗的 Enter 只套用，組字要留著");
    let second = enter(&mut s);
    assert_eq!(second.preedit, "");
    assert_eq!(s.commit(), "我");
}
