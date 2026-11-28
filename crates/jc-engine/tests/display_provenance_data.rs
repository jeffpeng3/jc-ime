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

fn down(session: &mut Session) -> jc_core::output::Snapshot {
    session.press(Key::new(KeyCode::Down, None, false))
}

fn right(session: &mut Session) {
    session.press(Key::new(KeyCode::ArrowRight, None, false));
}

fn escape(session: &mut Session) {
    session.press(Key::new(KeyCode::Escape, None, false));
}

fn surfaces(snapshot: &jc_core::output::Snapshot) -> Vec<String> {
    snapshot
        .candidates
        .iter()
        .map(|candidate| candidate.surface.clone())
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

/// decoder／live 日文 lane 的 ↓ 也以游標右側的顯示字元為 target。
#[test]
fn real_ja_decoder_down_targets_one_display_unit() {
    let Some(root) = jc_testdata::root() else {
        eprintln!("缺少測試資料：JC_TESTDATA");
        return;
    };
    let data = jc_engine::loader::load_engine_data(&root).unwrap();

    let mut s = session(&data);
    type_keys(&mut s, "watashi");
    s.ja_live_idle();
    assert_eq!(s.preedit(), "私");

    // 停頓轉換後選回かな，游標停在 `わ|たし`。
    let opened = down(&mut s);
    let kana = opened
        .candidates
        .iter()
        .position(|candidate| candidate.surface == "わたし")
        .expect("候選要有 わたし");
    s.choose_candidate(kana).expect("選 わたし");
    assert_eq!(s.preedit(), "わたし");
    assert_eq!(s.snapshot().cursor_rune, 1, "選完かな游標在第一個字之後");

    // わ|たし：只列 た 與它的按鍵，不列整段 わたし。
    let snap = down(&mut s);
    assert_eq!(snap.preedit, "わたし");
    assert_eq!(snap.cursor_rune, 1, "↓ 不移動游標");
    assert_eq!(snap.candidates[0].surface, "た", "target 是游標右側的 た");
    assert_eq!(snap.candidates[1].surface, "ta", "第二項是該字的按鍵");
    assert!(
        !surfaces(&snap).contains(&"わたし".to_string()),
        "不能列整段：{:?}",
        surfaces(&snap)
    );

    escape(&mut s);
    right(&mut s);
    assert_eq!(s.snapshot().cursor_rune, 2);

    // わた|し：只列 し 與 shi，同樣不回到整段。
    let snap = down(&mut s);
    assert_eq!(snap.preedit, "わたし");
    assert_eq!(snap.cursor_rune, 2, "↓ 不移動游標");
    assert_eq!(snap.candidates[0].surface, "し", "target 是游標右側的 し");
    assert_eq!(snap.candidates[1].surface, "shi", "第二項是該字的按鍵");
    assert!(
        !surfaces(&snap).contains(&"わたし".to_string()),
        "不能列整段：{:?}",
        surfaces(&snap)
    );

    // 選一個只屬於 し 的候選，前面的 わた 不能被重轉成漢字。
    let hit = snap
        .candidates
        .iter()
        .position(|candidate| candidate.surface == "氏")
        .expect("真實 decoder 要有 氏");
    let snap = s.choose_candidate(hit).expect("套用 氏");
    assert_eq!(snap.preedit, "わた氏", "只換 し，前面的 わた 保持かな");
}
