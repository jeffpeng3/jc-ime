//! 真實注音資料的回歸測試：輕聲按鍵延遲與字元邊界 panic。
//!
//! 資料包不在時整組跳過；`JC_TESTDATA` 可覆寫資料根目錄。

use std::sync::Arc;
use std::time::{Duration, Instant};

use jc_core::lattice::{InMemoryLexicon, LexEntry};
use jc_core::score::{probability_cost, BigramLm, ScoreContext, ZhLm};
use jc_core::session::{Session, SessionConfig};
use jc_core::{Key, Mode};

/// 一次按鍵允許的最長延遲；超過就是使用者感受到的卡鍵。
const KEY_BUDGET: Duration = Duration::from_millis(50);

fn real_session(mode: Mode) -> Option<Session> {
    real_session_with_lm(mode, true)
}

fn real_session_with_lm(mode: Mode, with_lm: bool) -> Option<Session> {
    let dir = jc_testdata::path("build-zh-bigram")?;
    let bundle = jc_data::zh::Bundle::open(&dir).ok()?;
    let lm = ZhLm::from_lm(bundle.lm);
    let mut lexicon = InMemoryLexicon::new();
    for view in bundle.lexicon.entries() {
        let unigram = lm.uni(view.surface);
        let cost = probability_cost(view.prw as f64).saturating_add(probability_cost(unigram));
        lexicon.push(LexEntry::new(
            view.surface,
            bundle.lexicon.reading(view.index),
            cost,
            0,
        ));
    }
    let mut config = SessionConfig::new(mode);
    config.zh_lexicon = Some(Arc::new(lexicon));
    config.zh_ctx = Some(ScoreContext {
        unknown_cost: 30_000,
        ..ScoreContext::default()
    });
    if with_lm {
        config.zh_lm = Some(Arc::new(lm));
    }
    Some(Session::new(config))
}

fn press(session: &mut Session, keys: &str) -> String {
    let mut preedit = String::new();
    for ch in keys.chars() {
        preedit = session.press(Key::char(ch, false)).preedit;
    }
    preedit
}

#[test]
fn tone7_key_stays_within_budget() {
    let Some(mut session) = real_session(Mode::Zh) else {
        eprintln!("缺少 build-zh-bigram，跳過延遲測試");
        return;
    };
    let mut preedit = String::new();
    for ch in "gp6au42k7".chars() {
        let start = Instant::now();
        preedit = session.press(Key::char(ch, false)).preedit;
        let elapsed = start.elapsed();
        assert!(
            elapsed < KEY_BUDGET,
            "按鍵 {ch:?} 花了 {elapsed:?}，超過 {KEY_BUDGET:?}；目前 preedit={preedit:?}"
        );
    }
    assert_eq!(preedit, "神秘的");
}

#[test]
fn tone7_without_bigram_stays_within_budget() {
    let Some(mut session) = real_session_with_lm(Mode::Zh, false) else {
        eprintln!("缺少 build-zh-bigram，跳過無 bigram 延遲測試");
        return;
    };
    let mut preedit = String::new();
    for ch in "gp6au42k7".chars() {
        let start = Instant::now();
        preedit = session.press(Key::char(ch, false)).preedit;
        let elapsed = start.elapsed();
        assert!(
            elapsed < KEY_BUDGET,
            "無 bigram 時按鍵 {ch:?} 花了 {elapsed:?}，超過 {KEY_BUDGET:?}；目前 preedit={preedit:?}"
        );
    }
    assert_eq!(preedit, "神秘的");
}

#[test]
fn tone7_fourth_syllable_never_reverts_both_characters() {
    let Some(mut session) = real_session(Mode::Zh) else {
        eprintln!("缺少 build-zh-bigram，跳過重算測試");
        return;
    };
    press(&mut session, "gp6au42k");
    let snapshot = session.press(Key::char('4', false));
    assert!(
        snapshot.preedit.starts_with("神秘"),
        "第四鍵不得把前兩個字同時退回原始注音：{:?}",
        snapshot.preedit
    );
}

#[test]
fn tone7_then_au4_does_not_panic() {
    let Some(mut session) = real_session(Mode::Zh) else {
        eprintln!("缺少 build-zh-bigram，跳過 panic 回歸測試");
        return;
    };
    let preedit = press(&mut session, "gp6au42k7au4");
    assert!(
        preedit.starts_with("神秘"),
        "續打ㄇㄧ後仍應保留已轉換的字：{preedit:?}"
    );
}

/// 這個分支從 v0.1.21 開出，還沒有 `fix-early-zh-convert` 的多音節規則；
/// 從 v0.1.22 起，同一個 run 裡未完成的 `ㄨㄢ` 才不會提早轉字。
///
/// `j0w96` 是行為探針：有規則時停在 `ㄨㄢ台`，沒有時已經變成 `灣台`。
/// 探針只選期望值，實際輸出仍逐字比對。
fn keeps_unfinished_zhuyin() -> bool {
    let mut lex = InMemoryLexicon::new();
    lex.push(LexEntry::new("台", "ㄊㄞˊ", 0, 0));
    lex.push(LexEntry::new("灣", "ㄨㄢ", 0, 0));
    let mut config = SessionConfig::new(Mode::Mixed);
    config.zh_lexicon = Some(Arc::new(lex));
    let mut session = Session::new(config);
    press(&mut session, "j0w96") == "ㄨㄢ台"
}

#[test]
fn existing_zh_sequences_stay_stable() {
    fn assert_sequence(keys: &str, expected: &str) {
        let Some(mut session) = real_session(Mode::Mixed) else {
            eprintln!("缺少 build-zh-bigram，跳過既有序列測試");
            return;
        };
        assert_eq!(press(&mut session, keys), expected, "序列 {keys:?}");
    }

    assert_sequence("au42k7", "密的");
    assert_sequence("w96j0", "台ㄨㄢ");
    if keeps_unfinished_zhuyin() {
        assert_sequence("w96j0w96j0", "台ㄨㄢ台ㄨㄢ");
        assert_sequence("w96j0w96j0 ", "台ㄨㄢ台灣");
    } else {
        // v0.1.21 基底：同一個 run 的未完成音節仍會提早轉字。
        assert_sequence("w96j0w96j0", "台灣台灣");
        assert_sequence("w96j0w96j0 ", "台灣台灣");
    }
}
