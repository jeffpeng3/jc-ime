//! 顯示字元 provenance 的真實日文資料回歸。
//!
//! 這些案例一定要走 `in_ja_lane()`（真實 ja decoder），因為混打與純日文
//! lane 的按鍵來源不同：前者看 `AsciiInput::typed()`，後者看 `RomajiInput`
//! 的讀音單位。資料不存在時跳過。

use jc_core::key::Key;
use jc_core::session::Session;
use jc_core::KeyCode;
use jc_core::Mode;

fn session(data: &jc_engine::loader::EngineData) -> Session {
    Session::new(data.session_config(Mode::Mixed))
}

fn type_keys(session: &mut Session, keys: &str) {
    for ch in keys.chars() {
        session.press(Key::char(ch, false));
    }
}

fn backspace(session: &mut Session) -> String {
    session
        .press(Key::new(KeyCode::Backspace, None, false))
        .preedit
}

fn keys(session: &Session) -> Vec<(String, String)> {
    session
        .units()
        .iter()
        .map(|unit| (unit.surface.clone(), unit.keys.clone()))
        .collect()
}

#[test]
fn real_ja_decoder_keeps_unit_keys() {
    let Some(root) = jc_testdata::root() else {
        eprintln!("缺少測試資料：JC_TESTDATA");
        return;
    };
    let data = jc_engine::loader::load_engine_data(&root).unwrap();

    // 私 由 わたし 三個讀音單位轉成一個顯示字元；keys 必須是整段 watashi。
    let mut s = session(&data);
    type_keys(&mut s, "watashi");
    s.ja_live_idle();
    assert_eq!(s.preedit(), "私");
    assert_eq!(keys(&s), [("私".to_string(), "watashi".to_string())]);

    // kyo 走真實 decoder：刪 ょ 後留下 き，keys 仍是 k。
    let mut s = session(&data);
    type_keys(&mut s, "kyo");
    assert_eq!(s.preedit(), "きょ");
    assert_eq!(
        keys(&s),
        [
            ("き".to_string(), "k".to_string()),
            ("ょ".to_string(), "yo".to_string())
        ]
    );
    assert_eq!(backspace(&mut s), "き");
    assert_eq!(keys(&s), [("き".to_string(), "k".to_string())]);
}

#[test]
fn real_ja_decoder_deletes_private_and_shi_units() {
    let Some(root) = jc_testdata::root() else {
        eprintln!("缺少測試資料：JC_TESTDATA");
        return;
    };
    let data = jc_engine::loader::load_engine_data(&root).unwrap();

    // 私し：兩個顯示字元，各自有自己的 unit span。
    let mut s = session(&data);
    type_keys(&mut s, "watashi");
    s.ja_live_idle();
    type_keys(&mut s, "shi");
    assert_eq!(s.preedit(), "私し");
    assert_eq!(
        keys(&s),
        [
            ("私".to_string(), "watashi".to_string()),
            ("し".to_string(), "shi".to_string())
        ]
    );

    // 刪 し 只掉第二個 unit，私 與 watashi 留著。
    assert_eq!(backspace(&mut s), "私");
    assert_eq!(keys(&s), [("私".to_string(), "watashi".to_string())]);

    // 再刪 私：一個顯示字元帶走整段 watashi。
    assert_eq!(backspace(&mut s), "");
    assert!(s.units().is_empty());
}
