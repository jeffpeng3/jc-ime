//! 長串英文輸入的延遲與行為回歸測試。
//!
//! 資料包不在時整組跳過；`JC_TESTDATA` 可覆寫資料根目錄。
//!
//! 預算在 release 是每鍵 5ms；debug 建置的常數與最佳化都慢一個量級，
//! 放寬到 50ms，實際驗收數字以 release 為準。

use std::sync::Arc;
use std::time::{Duration, Instant};

use jc_core::lattice::{InMemoryLexicon, LexEntry};
use jc_core::score::{probability_cost, BigramLm, ScoreContext, ZhLm};
use jc_core::session::{Session, SessionConfig};
use jc_core::{Key, Mode};

fn key_budget() -> Duration {
    if cfg!(debug_assertions) {
        Duration::from_millis(50)
    } else {
        Duration::from_millis(5)
    }
}

fn real_session(mode: Mode) -> Option<Session> {
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
    config.zh_lm = Some(Arc::new(lm));
    Some(Session::new(config))
}

fn press(session: &mut Session, keys: &str) -> String {
    let mut preedit = String::new();
    for ch in keys.chars() {
        preedit = session.press(Key::char(ch, false)).preedit;
    }
    preedit
}

/// 逐鍵量 `press`；回傳（最後一鍵, 後半段 p50）。
fn measure_typing(n: usize) -> Option<(Duration, Duration)> {
    let mut session = real_session(Mode::Mixed)?;
    let mut last = Duration::ZERO;
    let mut tail = Vec::with_capacity(n / 2 + 1);
    for index in 0..n {
        let started = Instant::now();
        let snapshot = session.press(Key::char('z', false));
        let elapsed = started.elapsed();
        if index + 1 == n {
            last = elapsed;
            assert_eq!(
                snapshot.preedit.chars().count(),
                n,
                "{n} 個 z 的 preedit 長度不對：{:?}",
                snapshot.preedit.chars().count()
            );
        }
        if index >= n / 2 {
            tail.push(elapsed);
        }
    }
    tail.sort_unstable();
    Some((last, tail[tail.len() / 2]))
}

fn assert_typing_budget(n: usize) {
    let Some((last, p50)) = measure_typing(n) else {
        eprintln!("缺少 build-zh-bigram，跳過長串延遲測試");
        return;
    };
    let budget = key_budget();
    assert!(
        last < budget,
        "{n} 個 z 的最後一鍵花了 {last:?}，超過 {budget:?}"
    );
    assert!(
        p50 < budget,
        "{n} 個 z 的後半段 p50 是 {p50:?}，超過 {budget:?}"
    );
}

#[test]
fn two_hundred_z_keys_stay_within_budget() {
    assert_typing_budget(200);
}

#[test]
fn sixteen_hundred_z_keys_stay_within_budget() {
    assert_typing_budget(1600);
}

#[test]
fn sixteen_hundred_z_preedit_is_complete() {
    let Some(mut session) = real_session(Mode::Mixed) else {
        eprintln!("缺少 build-zh-bigram，跳過長串顯示測試");
        return;
    };
    let preedit = press(&mut session, &"z".repeat(1600));
    assert_eq!(preedit.chars().count(), 1600);
    assert_eq!(preedit, "z".repeat(1600));
}

#[test]
fn long_literal_run_does_not_offer_english_completion() {
    let Some(mut session) = real_session(Mode::Mixed) else {
        eprintln!("缺少 build-zh-bigram，跳過英文補全測試");
        return;
    };
    press(&mut session, &"z".repeat(200));
    let snapshot = session.snapshot();
    assert!(
        snapshot.suggestion.is_none(),
        "長字面 run 不該有英文補全：{:?}",
        snapshot.suggestion
    );
    assert!(
        snapshot.candidates.iter().all(|candidate| {
            !candidate.surface.chars().all(|ch| ch.is_ascii_alphabetic())
                || candidate.surface == "z".repeat(200)
        }),
        "長字面 run 不該列英文補全候選：{:?}",
        snapshot
            .candidates
            .iter()
            .map(|candidate| candidate.surface.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn long_run_followed_by_zhuyin_still_splits() {
    let Some(mut session) = real_session(Mode::Mixed) else {
        eprintln!("缺少 build-zh-bigram，跳過長 run 注音切分測試");
        return;
    };
    let preedit = press(&mut session, &format!("{}su3", "z".repeat(200)));
    assert_eq!(
        preedit,
        format!("{}你", "z".repeat(200)),
        "200 個 z 後面的 su3 要切成注音"
    );
    assert_eq!(preedit.chars().count(), 201);
}

#[test]
fn existing_mixed_sequences_stay_unchanged() {
    fn assert_sequence(keys: &str, expected: &str) {
        let Some(mut session) = real_session(Mode::Mixed) else {
            eprintln!("缺少 build-zh-bigram，跳過既有序列測試");
            return;
        };
        assert_eq!(press(&mut session, keys), expected, "序列 {keys:?}");
    }

    assert_sequence(
        &format!("sa{}", "a".repeat(70)),
        &format!("sa{}", "a".repeat(70)),
    );
    assert_sequence("rishireej/ savul3", "りしれ供さ小");
    assert_sequence("w96j0", "台ㄨㄢ");
    assert_sequence("ji394su3", "我愛你");
}
