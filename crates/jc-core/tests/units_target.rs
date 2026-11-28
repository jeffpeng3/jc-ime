//! 顯示字元 target 與刪除的 lane 無關回歸。
//!
//! 同一組 keys 在任何 lane 狀態下都要得到同一組候選；候選範圍只由
//! `compute_units()` 的游標字元決定。日文資料不存在時整支跳過。

use std::sync::Arc;

use jc_core::key::Key;
use jc_core::mixed::{Dictionary, English, Word};
use jc_core::session::{Session, SessionConfig};
use jc_core::{KeyCode, Mode};

fn key(code: KeyCode) -> Key {
    Key::new(code, None, false)
}

fn type_keys(session: &mut Session, keys: &str) {
    for ch in keys.chars() {
        session.press(Key::char(ch, false));
    }
}

fn ja_session(ja_only: bool) -> Option<Session> {
    let path = jc_testdata::path("packs/ja-lexicon-mozc-v2.pack")?;
    let decoder = jc_core::ja::decode::JaDecoder::open_with_context(&path)
        .ok()?
        .with_particle_rewrite(true);
    let mut config = SessionConfig::new(Mode::Mixed);
    config.ja = Some(Arc::new(decoder));
    config.ja_only = ja_only;
    Some(Session::new(config))
}

fn surfaces(session: &Session) -> Vec<String> {
    session
        .units()
        .iter()
        .map(|unit| unit.surface.clone())
        .collect()
}

fn keys(session: &Session) -> Vec<String> {
    session
        .units()
        .iter()
        .map(|unit| unit.keys.clone())
        .collect()
}

fn candidates(session: &Session) -> Vec<String> {
    session
        .candidates()
        .iter()
        .map(|candidate| candidate.surface.clone())
        .collect()
}

/// 促音：兩個 lane 的 units 與同一組 keys 的候選都要相同。
#[test]
fn sokuon_units_and_candidates_match_across_lanes() {
    for (input, want_surfaces, want_keys) in [
        ("kka", ["っ", "か"], ["k", "ka"]),
        ("tta", ["っ", "た"], ["t", "ta"]),
        ("sshi", ["っ", "し"], ["s", "shi"]),
    ] {
        let Some(mut ja) = ja_session(true) else {
            eprintln!("缺少 ja pack，跳過");
            return;
        };
        let Some(mut mixed) = ja_session(false) else {
            return;
        };
        type_keys(&mut ja, input);
        type_keys(&mut mixed, input);
        assert_eq!(surfaces(&ja), want_surfaces, "{input} ja");
        assert_eq!(surfaces(&mixed), want_surfaces, "{input} mixed");
        assert_eq!(keys(&ja), want_keys, "{input} ja keys");
        assert_eq!(keys(&mixed), want_keys, "{input} mixed keys");

        ja.press(key(KeyCode::End));
        mixed.press(key(KeyCode::End));
        ja.press(key(KeyCode::Down));
        mixed.press(key(KeyCode::Down));
        assert_eq!(
            candidates(&ja),
            candidates(&mixed),
            "{input}：同一畫面同一按鍵的候選不能因 lane 不同"
        );
        assert_eq!(ja.candidate_target(), mixed.candidate_target(), "{input}");
    }
}

/// `kyou` 在兩個 lane 都是 `今日`，且逐字 keys 相同。
#[test]
fn kyou_units_match_across_lanes() {
    let Some(mut ja) = ja_session(true) else {
        return;
    };
    let Some(mut mixed) = ja_session(false) else {
        return;
    };
    for session in [&mut ja, &mut mixed] {
        type_keys(session, "kyou");
        session.ja_live_idle();
        assert_eq!(session.preedit(), "今日");
        assert_eq!(surfaces(session), ["今", "日"]);
        assert_eq!(keys(session), ["kyo", "u"]);
    }
}

/// `今日` 的 Home/End Down 都只對一個顯示字元，不是整個詞或文節。
#[test]
fn multi_han_down_targets_one_display_unit() {
    let Some(mut s) = ja_session(true) else {
        return;
    };
    type_keys(&mut s, "kyouha");
    s.ja_live_idle();
    assert_eq!(s.preedit(), "今日は");

    s.press(key(KeyCode::Home));
    let home = s.press(key(KeyCode::Down));
    assert_eq!(home.candidates[0].surface, "今", "Home 只對 今");
    assert_eq!(s.candidate_target(), Some((0, 1, "kyo".to_string())));
    s.press(key(KeyCode::Escape));

    let end = s.press(key(KeyCode::End));
    assert_eq!(end.cursor_rune, 3);
    s.press(key(KeyCode::Down));
    assert_eq!(s.candidates()[0].surface, "は", "End 只對 は");
    assert_eq!(s.candidate_target(), Some((2, 1, "ha".to_string())));
}

/// `ko-` 在任何 lane 都保留日文來源與 `ko`／`-`。
#[test]
fn long_vowel_keys_are_stable() {
    let mut mixed = Session::new(SessionConfig::new(Mode::Mixed));
    type_keys(&mut mixed, "ko-");
    assert_eq!(surfaces(&mixed), ["こ", "ー"]);
    assert_eq!(keys(&mixed), ["ko", "-"]);

    let Some(mut ja) = ja_session(true) else {
        return;
    };
    type_keys(&mut ja, "ko-");
    assert_eq!(surfaces(&ja), ["こ", "ー"]);
    assert_eq!(keys(&ja), ["ko", "-"]);
}

/// 一音節多字：每個字有 keys，Delete 一次只刪一個顯示字元。
#[test]
fn multi_char_zh_syllable_is_one_unit_at_a_time() {
    use jc_core::lattice::{InMemoryLexicon, LexEntry};
    let mut lex = InMemoryLexicon::new();
    lex.push(LexEntry::new("台灣", "ㄊㄞˊ", 0, 0));
    let mut config = SessionConfig::new(Mode::Mixed);
    config.zh_lexicon = Some(Arc::new(lex));
    let mut s = Session::new(config);
    type_keys(&mut s, "w96");
    assert_eq!(s.preedit(), "台灣");
    assert_eq!(surfaces(&s), ["台", "灣"]);
    assert!(s.units().iter().all(|unit| !unit.keys.is_empty()));
    s.press(key(KeyCode::Home));
    let snap = s.press(key(KeyCode::Delete));
    assert_eq!(snap.preedit, "灣", "只刪一個顯示字元");
    assert_eq!(surfaces(&s), ["灣"]);
}

/// `iji3` 跨音節：沒有詞典時畫面是 `ㄛㄨㄛˇ`。游標停在 `ㄨ` 時，target
/// 從 `ㄨ` 往右只延伸相鄰的注音符號，所以是 `ㄨㄛˇ`（rune 1、長度 3），
/// 不會回頭併入左邊的 `ㄛ`。
#[test]
fn cross_syllable_cursor_targets_adjacent_zhuyin() {
    let mut s = Session::new(SessionConfig::new(Mode::Mixed));
    type_keys(&mut s, "iji3");
    assert_eq!(s.preedit(), "ㄛㄨㄛˇ");
    s.press(key(KeyCode::Home));
    s.press(key(KeyCode::ArrowRight));
    s.press(key(KeyCode::Down));
    assert_eq!(s.candidate_target().map(|t| (t.0, t.1)), Some((1, 3)));
    assert_eq!(s.candidates()[0].surface, "ㄨㄛˇ");
}

/// 固定英文詞表的 session；權重決定補完與拼字建議的順序。
///
/// 不可以走 `English::load_default()`：那會讀 `DEFAULT_DATA_ROOT`／
/// `JC_TESTDATA`，有沒有 `english/` 詞表會讓同一條測試在 CI 與本機得到
/// 不同清單。
fn english_session() -> Session {
    let words = [("hello", 3.0), ("help", 2.0), ("hell", 1.0)]
        .into_iter()
        .map(|(word, weight)| Word::new(word, weight))
        .collect::<Vec<_>>();
    let mut config = SessionConfig::new(Mode::Mixed);
    config.english = Some(Arc::new(English::new(Some(Dictionary::new(words)))));
    config.english_autocomplete = true;
    Session::new(config)
}

/// 英文整詞補全是獨立的 run 補全動作；一般 ↓ 只對游標字母。
#[test]
fn english_tab_stays_whole_run_but_down_targets_letter() {
    let mut s = english_session();
    type_keys(&mut s, "hellp");
    s.press(key(KeyCode::Tab));
    assert_eq!(
        candidates(&s),
        ["hellp", "hello", "help", "hell"],
        "Tab 的整詞補全清單不變"
    );
    assert_eq!(s.candidate_target().map(|t| t.1), Some(5));

    s.press(key(KeyCode::Escape));
    s.press(key(KeyCode::End));
    s.press(key(KeyCode::Down));
    assert_eq!(s.candidate_target().map(|t| (t.0, t.1)), Some((4, 1)));
    assert_eq!(s.candidates()[0].surface, "p", "↓ 只對最後一個字母");
}

/// `JC_TESTDATA` 指到不存在目錄時（等同 `DEFAULT_DATA_ROOT` 也不可用），
/// 英文補全測試仍要通過。
///
/// 在子行程把 `JC_TESTDATA` 指到不存在的目錄並拿掉 `JC_IMEDATA`；
/// `data_root()` 因此在看 `DEFAULT_DATA_ROOT` 之前就回 `None`，等同那個
/// 常數指向不存在的目錄。只跑上面那條測試，不會遞迴。
#[test]
fn english_completion_is_hermetic_without_testdata() {
    let missing =
        std::env::temp_dir().join(format!("jc-hermetic-no-testdata-{}", std::process::id()));
    assert!(!missing.exists(), "測試用的不存在路徑竟然存在：{missing:?}");

    let exe = std::env::current_exe().expect("取得測試執行檔");
    let output = std::process::Command::new(exe)
        .args([
            "--exact",
            "english_tab_stays_whole_run_but_down_targets_letter",
            "--nocapture",
        ])
        .env("JC_TESTDATA", &missing)
        .env_remove("JC_IMEDATA")
        .output()
        .expect("啟動子行程");
    assert!(
        output.status.success(),
        "沒有測試資料時英文補全測試失敗（status {}）\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 未完成的注音也由 DisplayUnit target 決定：游標在 `ㄧ` 時 ↓ 只對 `ㄧ`。
#[test]
fn pending_zhuyin_down_targets_cursor_symbol() {
    let mut s = Session::new(SessionConfig::new(Mode::Mixed));
    type_keys(&mut s, "su");
    assert_eq!(s.preedit(), "ㄋㄧ");
    let snap = s.press(key(KeyCode::Down));
    assert_eq!(snap.candidates[0].surface, "ㄧ");
    assert_eq!(s.candidate_target().map(|t| (t.0, t.1)), Some((1, 1)));
    assert!(snap.navigating, "pending target 仍由 ↓ 開窗並進入選取模式");
}
