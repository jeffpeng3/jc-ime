//! 英數與標點混雜的長串延遲測試。
//!
//! 這是實機回報的形狀：隨機英數加 `; ' , . /`，單鍵 25–30ms，停手後
//! 一兩秒才追上。純小寫 run 的預算測試在 `long_run_latency.rs`；這支測試
//! 另外量混合組，兩組數字分開列，確認混合路徑沒有拖慢原本的快速路徑。
//!
//! 資料包不在時整組跳過；`JC_TESTDATA` 可覆寫資料根目錄。
//! 時間斷言只在 release 生效，實際驗收數字以 release 為準。

use std::sync::Arc;
use std::time::{Duration, Instant};

use jc_core::lattice::{InMemoryLexicon, LexEntry};
use jc_core::score::{probability_cost, BigramLm, ScoreContext, ZhLm};
use jc_core::session::{Session, SessionConfig};
use jc_core::{Key, Mode};

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

/// 量測用的兩種長串形狀。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// 原本就有快速路徑的純小寫 run。
    Lower,
    /// 英數加 `; ' , . /` 的混合 run。
    Mixed,
}

impl Shape {
    fn name(self) -> &'static str {
        match self {
            Shape::Lower => "lower",
            Shape::Mixed => "mixed",
        }
    }
}

const MIXED_KEYS: &str = "abc;def'ghi,jkl.mno/pqr0123456789";

fn sample_key(shape: Shape, index: usize) -> char {
    match shape {
        Shape::Lower => 'z',
        Shape::Mixed => MIXED_KEYS.as_bytes()[index % MIXED_KEYS.len()] as char,
    }
}

fn sample_text(shape: Shape, n: usize) -> String {
    (0..n).map(|index| sample_key(shape, index)).collect()
}

fn has_bopomofo(text: &str) -> bool {
    text.chars()
        .any(|ch| ('\u{3100}'..='\u{312f}').contains(&ch))
}

/// 一次長串量測的結果。
struct Typing {
    last: Duration,
    p50: Duration,
    preedit: String,
}

/// 逐鍵量 `press`；回傳最後一鍵、後半段 p50 與最終 preedit。
fn measure_shape(shape: Shape, n: usize) -> Option<Typing> {
    let mut session = real_session(Mode::Mixed)?;
    let mut last = Duration::ZERO;
    let mut tail = Vec::with_capacity(n / 2 + 1);
    let mut preedit = String::new();
    for index in 0..n {
        let started = Instant::now();
        let snapshot = session.press(Key::char(sample_key(shape, index), false));
        let elapsed = started.elapsed();
        preedit = snapshot.preedit;
        if index + 1 == n {
            last = elapsed;
        }
        if index >= n / 2 {
            tail.push(elapsed);
        }
    }
    tail.sort_unstable();
    Some(Typing {
        last,
        p50: tail[tail.len() / 2],
        preedit,
    })
}

/// 32／64／128／256 的每鍵耗時表；`--nocapture` 才看得到。
#[test]
fn long_run_latency_table() {
    for shape in [Shape::Lower, Shape::Mixed] {
        for n in [32, 64, 128, 256] {
            let Some(typing) = measure_shape(shape, n) else {
                eprintln!("缺少 build-zh-bigram，跳過長串延遲表");
                return;
            };
            eprintln!(
                "{:<5} n={n:<4} last={:>10.3?} p50={:>10.3?}",
                shape.name(),
                typing.last,
                typing.p50
            );
        }
    }
}

/// 混合標點長串必須整段字面，不能冒出 ㄎㄨㄑㄩ 這類注音。
#[test]
fn mixed_punctuation_long_run_stays_literal() {
    for n in [32, 64, 128, 256] {
        let Some(typing) = measure_shape(Shape::Mixed, n) else {
            eprintln!("缺少 build-zh-bigram，跳過混合長串字面測試");
            return;
        };
        assert!(
            !has_bopomofo(&typing.preedit),
            "{n} 字混合長串出現注音：{:?}",
            typing.preedit
        );
        assert_eq!(
            typing.preedit,
            sample_text(Shape::Mixed, n),
            "{n} 字混合長串"
        );
    }
}

/// 混合標點長串的單鍵成本不能隨總長度線性上升。
///
/// 時間斷言只在 release 生效；debug 建置的常數與最佳化都慢一個量級。比較用
/// 最後一鍵，並給 200µs 的下限，避免 microbenchmark 的單次排程抖動誤判。
#[test]
fn mixed_punctuation_long_run_cost_stays_flat() {
    let Some(small) = measure_shape(Shape::Mixed, 64) else {
        eprintln!("缺少 build-zh-bigram，跳過混合長串延遲測試");
        return;
    };
    let Some(large) = measure_shape(Shape::Mixed, 256) else {
        eprintln!("缺少 build-zh-bigram，跳過混合長串延遲測試");
        return;
    };
    #[cfg(not(debug_assertions))]
    {
        let floor = Duration::from_micros(200);
        let budget = std::cmp::max(small.last, floor) * 3;
        assert!(
            large.last <= budget,
            "混合長串 256 字最後一鍵 {:?}，超過 64 字 {:?} 的三倍（預算 {budget:?}）",
            large.last,
            small.last
        );
    }
    let _ = (small, large);
}

/// 純小寫長串的既有快速路徑不能被混合路徑拖慢，也不能被改寫。
#[test]
fn pure_lowercase_long_run_stays_fast_and_literal() {
    let Some(small) = measure_shape(Shape::Lower, 64) else {
        eprintln!("缺少 build-zh-bigram，跳過純小寫長串測試");
        return;
    };
    let Some(large) = measure_shape(Shape::Lower, 256) else {
        eprintln!("缺少 build-zh-bigram，跳過純小寫長串測試");
        return;
    };
    assert_eq!(large.preedit, "z".repeat(256));
    #[cfg(not(debug_assertions))]
    {
        let budget = std::cmp::max(small.last, Duration::from_millis(8)) * 3;
        assert!(
            large.last <= budget,
            "純小寫 256 字最後一鍵 {:?}，超過 64 字 {:?} 的三倍（預算 {budget:?}）",
            large.last,
            small.last
        );
    }
    let _ = (small, large);
}
