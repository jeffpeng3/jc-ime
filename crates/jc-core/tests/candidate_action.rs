//! 候選套用收成 `CandidateAction` 後的行為鎖。
//!
//! 這些測試只驗使用者看得到的結果：Enter 兩段式、選單一次送出、前導變體
//! 例外、`zh_window_last` 的游標與清單，以及學習只記一次。

use std::sync::Arc;

use jc_core::lattice::{InMemoryLexicon, LexEntry};
use jc_core::personal::CONFIRM_UNIT;
use jc_core::session::{Session, SessionConfig};
use jc_core::{Key, KeyCode, Mode, Snapshot};

fn press(session: &mut Session, key: Key) -> Snapshot {
    session.press(key)
}

fn enter(session: &mut Session) -> Snapshot {
    press(session, Key::new(KeyCode::Enter, None, false))
}

fn down(session: &mut Session) -> Snapshot {
    press(session, Key::new(KeyCode::Down, None, false))
}

fn press_chars(session: &mut Session, text: &str) {
    for ch in text.chars() {
        press(session, Key::char(ch, false));
    }
}

/// 只有「我」「窩」兩個同音字的注音 session。
fn zh_session() -> Session {
    let mut lexicon = InMemoryLexicon::new();
    for (surface, reading, cost) in [("我", "ㄨㄛˇ", 0), ("窩", "ㄨㄛˇ", 10)] {
        lexicon.push(LexEntry::new(surface, reading, cost, 0));
    }
    let mut config = SessionConfig::new(Mode::Mixed);
    config.zh_lexicon = Some(Arc::new(lexicon));
    Session::new(config)
}

fn select_surface(session: &mut Session, surface: &str) -> usize {
    let at = session
        .candidates()
        .iter()
        .position(|candidate| candidate.surface == surface)
        .unwrap_or_else(|| {
            let list: Vec<&str> = session
                .candidates()
                .iter()
                .map(|candidate| candidate.surface.as_str())
                .collect();
            panic!("候選 {surface} 找不到：{list:?}")
        });
    session.select(at);
    at
}

/// 1. 一般候選窗：第一次 Enter 只套用高亮、組字保留；第二次才送出。
#[test]
fn test_action_enter_applies_then_commits() {
    let mut s = zh_session();
    press_chars(&mut s, "ji3");
    down(&mut s);
    assert_eq!(s.preedit(), "我");

    let first = enter(&mut s);
    assert_eq!(first.preedit, "我", "第一次 Enter 只套用高亮，組字要留著");

    let second = enter(&mut s);
    assert_eq!(second.preedit, "");
    assert_eq!(s.commit(), "我", "第二次 Enter 才送出整段");
}

/// 2. 選單：一次 Enter 插入一次，不重複。
#[test]
fn test_action_menu_enter_inserts_once() {
    let mut s = Session::new(SessionConfig::new(Mode::Mixed));
    let snapshot = s.open_symbol_menu();
    let expected = snapshot.candidates[0].surface.clone();

    let applied = enter(&mut s);
    assert_eq!(applied.preedit, "", "選單一次送出，不留組字");
    assert_eq!(s.commit(), expected, "只插入一次");
    assert_eq!(s.commit(), "", "同一次選取不能被送出兩次");
}

/// 3. 前導符號變體：高亮即目前字面時 Enter 一次送出，不套用兩次。
#[test]
fn test_action_prefix_variant_enter_commits_once() {
    let mut s = Session::new(SessionConfig::new(Mode::Mixed));
    press_chars(&mut s, "`,");
    assert_eq!(s.preedit(), "，");

    let applied = enter(&mut s);
    assert_eq!(applied.preedit, "", "高亮即目前字面時直接送出");
    assert_eq!(s.commit(), "，", "變體只套用一次");
    assert_eq!(s.commit(), "", "不能重複送出");
}

/// 4. `zh_window_last`：套用最後一格後游標在音節後，候選清單仍完整。
#[test]
fn test_action_zh_window_last_cursor_and_candidates() {
    let mut s = zh_session();
    press_chars(&mut s, "ji3");
    down(&mut s);
    select_surface(&mut s, "窩");

    let applied = enter(&mut s);
    assert_eq!(applied.preedit, "窩", "只換最後一個音節");
    assert_eq!(applied.cursor_rune, 1, "游標停在套用後的音節後");
    assert_eq!(s.preedit(), "窩", "套用不送出整段");
    assert!(
        applied
            .candidates
            .iter()
            .any(|candidate| candidate.surface == "窩"),
        "套用後清單仍看得到目前的字：{:?}",
        applied
            .candidates
            .iter()
            .map(|candidate| candidate.surface.as_str())
            .collect::<Vec<_>>()
    );

    enter(&mut s);
    assert_eq!(s.commit(), "窩");
}

/// 6. 學習：一次套用只記一次，後續 Enter 是送出，不再重複記同一筆。
#[test]
fn test_action_learning_recorded_once_per_apply() {
    let mut s = zh_session();
    press_chars(&mut s, "ji3");
    down(&mut s);
    select_surface(&mut s, "窩");
    enter(&mut s);

    let learned = s.personal().uni.get("窩").copied().unwrap_or_default();
    assert!(
        (learned - CONFIRM_UNIT).abs() < f64::EPSILON,
        "一次套用只記一份證據，實際 {learned}"
    );

    enter(&mut s);
    let after_commit = s.personal().uni.get("窩").copied().unwrap_or_default();
    assert!(
        (after_commit - CONFIRM_UNIT).abs() < f64::EPSILON,
        "第二次 Enter 是送出，不能重複學習，實際 {after_commit}"
    );
}
