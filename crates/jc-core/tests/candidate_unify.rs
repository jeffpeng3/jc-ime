//! 候選清單單一來源、排序、上限與套用統一。
//!
//! 這幾條只驗使用者看得到的契約：
//!
//! - 同一串 keys 由 `↓`、`↑`、`Tab` 開窗都得到同一份清單。
//! - `[0]` 目前值、`[1]` 原始輸入，推薦接在後面。
//! - 上限 200 是硬上限，`max_candidates` 設更高也不得超過。
//! - `↑` 開的窗和 `↓` 一樣是兩段式 Enter。

use std::sync::Arc;

use jc_core::lattice::{InMemoryLexicon, LexEntry};
use jc_core::session::{Session, SessionConfig, DEFAULT_MAX_CANDIDATES};
use jc_core::{Key, KeyCode, Mode, Snapshot};

fn press(session: &mut Session, key: Key) -> Snapshot {
    session.press(key)
}

fn code(session: &mut Session, code: KeyCode) -> Snapshot {
    press(session, Key::new(code, None, false))
}

fn type_keys(session: &mut Session, text: &str) {
    for ch in text.chars() {
        press(session, Key::char(ch, false));
    }
}

fn surfaces(snap: &Snapshot) -> Vec<String> {
    snap.candidates
        .iter()
        .map(|candidate| candidate.surface.clone())
        .collect()
}

fn zh_session(entries: &[(&str, &str, i32)]) -> Session {
    let mut lexicon = InMemoryLexicon::new();
    for (surface, reading, cost) in entries {
        lexicon.push(LexEntry::new(*surface, *reading, *cost, 0));
    }
    let mut config = SessionConfig::new(Mode::Mixed);
    config.zh_lexicon = Some(Arc::new(lexicon));
    Session::new(config)
}

fn wo_session() -> Session {
    zh_session(&[("我", "ㄨㄛˇ", 0), ("窩", "ㄨㄛˇ", 10)])
}

/// `↓`、`↑`、`Tab` 三種開窗得到同一份清單，前兩項固定是目前值與原始輸入。
#[test]
fn test_down_up_tab_share_the_same_candidate_list() {
    let mut down = wo_session();
    type_keys(&mut down, "ji3");
    let down = code(&mut down, KeyCode::Down);

    let mut up = wo_session();
    type_keys(&mut up, "ji3");
    let up = code(&mut up, KeyCode::Up);

    let mut tab = wo_session();
    type_keys(&mut tab, "ji3");
    let tab = code(&mut tab, KeyCode::Tab);

    let want = surfaces(&down);
    assert_eq!(want[0], "我", "[0] 目前值：{want:?}");
    assert_eq!(want[1], "ji3", "[1] 原始輸入：{want:?}");
    assert_eq!(surfaces(&up), want, "↑ 與 ↓ 共用同一份清單");
    assert_eq!(surfaces(&tab), want, "Tab 與 ↓ 共用同一份清單");
    assert_eq!(up.preedit, "我");
    assert_eq!(tab.preedit, "我");
}

/// `↑` 開的窗仍是兩段式 Enter：第一次只套用、第二次才送出。
#[test]
fn test_up_candidate_enter_stays_two_stage() {
    let mut s = wo_session();
    type_keys(&mut s, "ji3");
    let up = code(&mut s, KeyCode::Up);
    assert!(up.navigating, "↑ 要進入選取模式");

    let first = code(&mut s, KeyCode::Enter);
    assert_eq!(first.preedit, "我", "第一次 Enter 只套用目前值");
    assert!(s.snapshot().candidates.is_empty() || !s.snapshot().navigating);

    let second = code(&mut s, KeyCode::Enter);
    assert_eq!(second.preedit, "");
    assert_eq!(s.commit(), "我", "第二次 Enter 才送出");
}

/// 超過 200 個同音詞時，任何開窗路徑都截在 200；`max_candidates` 設更高
/// 也不得突破硬上限。
#[test]
fn test_candidate_cap_is_200_even_with_larger_config() {
    let mut entries = Vec::new();
    for index in 0..250u32 {
        let surface = char::from_u32(0x4e00 + index).expect("CJK 候選");
        entries.push((surface.to_string(), "ㄨㄛˇ".to_string(), index as i32));
    }
    let mut lexicon = InMemoryLexicon::new();
    for (surface, reading, cost) in &entries {
        lexicon.push(LexEntry::new(surface.clone(), reading.clone(), *cost, 0));
    }
    let mut config = SessionConfig::new(Mode::Mixed);
    config.zh_lexicon = Some(Arc::new(lexicon));
    config.max_candidates = 500;
    let mut s = Session::new(config);
    type_keys(&mut s, "ji3");
    assert!(
        s.candidates().len() <= DEFAULT_MAX_CANDIDATES,
        "未開窗清單超過 200：{}",
        s.candidates().len()
    );
    let down = code(&mut s, KeyCode::Down);
    assert!(
        down.candidates.len() <= DEFAULT_MAX_CANDIDATES,
        "↓ 開窗超過 200：{}",
        down.candidates.len()
    );
}

/// 混打已完成注音音節也有 `[0]` 目前值、`[1]` 原始輸入。
#[test]
fn test_mixed_completed_zh_has_current_and_raw() {
    let mut s = zh_session(&[("小", "ㄒㄧㄠˇ", 0), ("曉", "ㄒㄧㄠˇ", 10)]);
    type_keys(&mut s, "savul3");
    assert_eq!(
        s.preedit(),
        "さ小",
        "前段 sa 走日文，尾端 vul3 是完成的注音"
    );
    let list: Vec<String> = s
        .candidates()
        .iter()
        .map(|candidate| candidate.surface.clone())
        .collect();
    assert_eq!(list[0], "小", "[0] 目前值：{list:?}");
    assert_eq!(list[1], "vul3", "[1] 原始輸入：{list:?}");
}
