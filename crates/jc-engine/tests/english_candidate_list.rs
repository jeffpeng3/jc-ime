//! 英文 Tab 候選清單以真實資料鎖住。
//!
//! `hellp` 按 Tab 只開窗，清單固定是字面、`hello`、`help`、`hell`；初始
//! 高亮在第一項，不含全形逐字外觀。關閉 `english_autocomplete` 時不得出現
//! 英文補全，混合模式 `ji3hellp` 的目標仍是整段英文 run。資料不存在時跳過。

use jc_core::key::Key;
use jc_core::session::Session;
use jc_core::{KeyCode, Mode};

fn key(code: KeyCode) -> Key {
    Key::new(code, None, false)
}

fn press_all(session: &mut Session, keys: &[Key]) {
    for &key in keys {
        session.press(key);
    }
}

fn chars(text: &str) -> Vec<Key> {
    text.chars().map(|ch| Key::char(ch, false)).collect()
}

fn surfaces(session: &Session) -> Vec<String> {
    session
        .candidates()
        .iter()
        .map(|candidate| candidate.surface.clone())
        .collect()
}

fn english_session(mut config: jc_core::session::SessionConfig, autocomplete: bool) -> Session {
    config.english_autocomplete = autocomplete;
    Session::new(config)
}

#[test]
fn english_tab_candidates_match_real_data() {
    let Some(root) = jc_testdata::root() else {
        eprintln!("缺少測試資料：JC_TESTDATA");
        return;
    };
    let data = jc_engine::loader::load_engine_data(&root).unwrap();

    // Tab 開窗：清單、初始高亮與兩段式 Enter。
    let mut s = english_session(data.session_config(Mode::Mixed), true);
    press_all(&mut s, &chars("hellp"));
    let tab = s.press(key(KeyCode::Tab));
    assert_eq!(tab.preedit, "hellp", "Tab 只能開窗，不能直接套用");
    assert!(tab.navigating, "Tab 要進入選取模式");
    assert_eq!(tab.highlighted, 0, "初始高亮必須是第一項字面");
    assert_eq!(
        surfaces(&s),
        vec!["hellp", "hello", "help", "hell"],
        "清單只放字面與真正的補全詞"
    );
    // 移到 hell：第一次 Enter 只套用，第二次才送出。
    press_all(&mut s, &[key(KeyCode::Down); 3]);
    let first = s.press(key(KeyCode::Enter));
    assert_eq!(first.preedit, "hell", "第一次 Enter 只套用候選");
    s.press(key(KeyCode::Enter));
    assert_eq!(s.commit(), "hell", "第二次 Enter 才 commit");

    // 關閉英文補全：沒有 hello，Tab 交還應用程式。
    let mut s = english_session(data.session_config(Mode::Mixed), false);
    press_all(&mut s, &chars("hellp"));
    let tab = s.press(key(KeyCode::Tab));
    assert_eq!(tab.preedit, "hellp", "Tab 不得改組字");
    assert!(!tab.navigating, "沒有英文補全不得開窗");
    assert!(!s.last_key_consumed(), "沒有補全時 Tab 要交還應用程式");
    assert_eq!(surfaces(&s), vec!["hellp"], "關閉補全只剩字面候選");

    // 混合模式：目標是整段英文 run，不是最後一個字元。
    let mut s = english_session(data.session_config(Mode::Mixed), true);
    press_all(&mut s, &chars("ji3hellp"));
    assert_eq!(s.snapshot().preedit, "我hellp");
    let tab = s.press(key(KeyCode::Tab));
    assert_eq!(tab.preedit, "我hellp", "Tab 只開窗");
    assert_eq!(tab.highlighted, 0);
    assert_eq!(surfaces(&s), vec!["hellp", "hello", "help", "hell"]);
    s.press(key(KeyCode::Down));
    let first = s.press(key(KeyCode::Enter));
    assert_eq!(first.preedit, "我hello", "整段 hellp 換成 hello");
    s.press(key(KeyCode::Enter));
    assert_eq!(s.commit(), "我hello");
}
