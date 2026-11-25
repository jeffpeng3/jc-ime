//! 英數與標點混雜的長串延遲測試。
//!
//! 這是實機回報的形狀：隨機英數加 `; ' , . /`，單鍵 25–30ms，停手後
//! 一兩秒才追上。純小寫 run 的預算測試在 `long_run_latency.rs`；這支測試
//! 另外量混合組，兩組數字分開列，確認混合路徑沒有拖慢原本的快速路徑。
//!
//! 資料包不在時整組跳過；`JC_TESTDATA` 可覆寫資料根目錄。
//! 時間斷言只在 release 生效，實際驗收數字以 release 為準。

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use jc_core::lattice::{InMemoryLexicon, LexEntry};
use jc_core::score::{probability_cost, BigramLm, ScoreContext, ZhLm};
use jc_core::session::{Session, SessionConfig};
use jc_core::{Key, Mode};

/// 量測測試一次只跑一個，避免同一支測試程式內的平行負載拉高單鍵時間。
static PERF_LOCK: Mutex<()> = Mutex::new(());

fn perf_guard() -> MutexGuard<'static, ()> {
    PERF_LOCK.lock().unwrap_or_else(|error| error.into_inner())
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

fn has_kana(text: &str) -> bool {
    text.chars()
        .any(|ch| ('\u{3040}'..='\u{30ff}').contains(&ch))
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

fn type_keys(keys: &str) -> Option<String> {
    let mut session = real_session(Mode::Mixed)?;
    let mut preedit = String::new();
    for ch in keys.chars() {
        preedit = session.press(Key::char(ch, false)).preedit;
    }
    Some(preedit)
}

/// 長英文詞不能因為尾巴視窗被切成かな或注音。
#[test]
fn long_english_word_stays_literal() {
    let _guard = perf_guard();
    let word = "pneumonoultramicroscopicsilicovolcanoconiosis";
    for keys in [
        word.to_string(),
        format!("{}{word}", "z".repeat(100)),
        format!("{} {word}", "z".repeat(100)),
        format!("{}{word}", "z".repeat(64)),
    ] {
        let Some(preedit) = type_keys(&keys) else {
            eprintln!("缺少 build-zh-bigram，跳過長英文詞測試");
            return;
        };
        assert_eq!(preedit, keys, "長英文詞要原樣字面");
        assert!(!has_kana(&preedit), "不得出現假名：{preedit:?}");
        assert!(!has_bopomofo(&preedit), "不得出現注音：{preedit:?}");
    }
}

/// 長 run 後面的連續注音鏈要整條轉字，且下一個音節還在打時不能退回按鍵。
#[test]
fn long_run_zh_chain_matches_expected() {
    let _guard = perf_guard();
    let z = "z".repeat(100);
    let a = "a".repeat(100);
    for (keys, want) in [
        (format!("{z}ji3su3"), format!("{z}我你")),
        (format!("{z}ji3su3g4"), format!("{z}我你是")),
        (format!("{a}xji3su3"), format!("{a}x我你")),
        (format!("{z}ji3"), format!("{z}我")),
        (format!("{z}su3"), format!("{z}你")),
        (format!("{z}ji3s"), format!("{z}我s")),
        (format!("{z}ji3su"), format!("{z}我ㄋㄧ")),
        (
            format!("{z}ji3pneumonoultramicroscopicsilicovolcanoconiosis"),
            format!("{z}我pneumonoultramicroscopicsilicovolcanoconiosis"),
        ),
        (
            format!("{z}su3{}", "z".repeat(100)),
            format!("{z}su3{}", "z".repeat(100)),
        ),
    ] {
        let Some(preedit) = type_keys(&keys) else {
            eprintln!("缺少 build-zh-bigram，跳過注音鏈測試");
            return;
        };
        assert_eq!(preedit, want, "{keys:?}");
        assert!(!preedit.contains("ji3"), "{keys:?} 退回原始按鍵");
    }
}

/// 逐鍵量最後一鍵；用來檢查 256 字附近仍 ≤1ms。
fn measure_keys(keys: &str) -> Option<(Duration, String)> {
    let mut session = real_session(Mode::Mixed)?;
    let mut last = Duration::ZERO;
    let mut preedit = String::new();
    for ch in keys.chars() {
        let started = Instant::now();
        let snapshot = session.press(Key::char(ch, false));
        last = started.elapsed();
        preedit = snapshot.preedit;
    }
    Some((last, preedit))
}

/// 使用者點名的情境在 256 字附近，單鍵仍要 ≤1ms。
#[test]
fn long_run_scenarios_stay_within_budget() {
    let _guard = perf_guard();
    let z240 = "z".repeat(240);
    let z200 = "z".repeat(200);
    let word = "pneumonoultramicroscopicsilicovolcanoconiosis";
    for (keys, suffix) in [
        (format!("{z240}ji3su3g4"), "我你是"),
        (format!("{z240}ji3s"), "我s"),
        (format!("{z240}su3"), "你"),
        (format!("{z200}ji3{}", "z".repeat(40)), ""),
        (format!("{word}su3"), "你"),
    ] {
        let Some((last, preedit)) = measure_keys(&keys) else {
            eprintln!("缺少 build-zh-bigram，跳過 256 字延遲測試");
            return;
        };
        assert!(
            preedit.ends_with(suffix),
            "{keys:?} 尾巴不符：{:?}",
            &preedit[preedit.len().saturating_sub(24)..]
        );
        #[cfg(not(debug_assertions))]
        assert!(
            last < Duration::from_millis(1),
            "{} 字最後一鍵 {:?} 超過 1ms",
            keys.chars().count(),
            last
        );
        let _ = last;
    }
}

/// 20／32／64／128／256／512／1024 的每鍵耗時表；`--nocapture` 才看得到。
#[test]
fn long_run_latency_table() {
    let _guard = perf_guard();
    for shape in [Shape::Lower, Shape::Mixed] {
        for n in [20, 32, 64, 128, 256, 512, 1024] {
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
    let _guard = perf_guard();
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
    let _guard = perf_guard();
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

/// 短字串與超長字串都要維持字面且成本有界。
///
/// 20 字沒有長 run，走原本的完整規劃；512／1024 字只規劃 16 字尾巴。
#[test]
fn short_and_very_long_compositions_stay_flat() {
    let _guard = perf_guard();
    for shape in [Shape::Lower, Shape::Mixed] {
        for n in [20, 512, 1024] {
            let Some(typing) = measure_shape(shape, n) else {
                eprintln!("缺少 build-zh-bigram，跳過短／超長字串測試");
                return;
            };
            assert_eq!(
                typing.preedit,
                sample_text(shape, n),
                "{} {n} 字要完整字面",
                shape.name()
            );
            #[cfg(not(debug_assertions))]
            {
                let budget = Duration::from_millis(1);
                assert!(
                    typing.last < budget,
                    "{} {n} 字最後一鍵 {:?}，超過 {budget:?}",
                    shape.name(),
                    typing.last
                );
            }
        }
    }
}

/// 純小寫長串的既有快速路徑不能被混合路徑拖慢，也不能被改寫。
#[test]
fn pure_lowercase_long_run_stays_fast_and_literal() {
    let _guard = perf_guard();
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
