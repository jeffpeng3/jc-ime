//! 冷啟動預熱：量一次預熱本身與預熱後的第一鍵。
//!
//! 需要 `JC_TESTDATA`（預設 `/mnt/work/jc/jc-ime-data`）；資料不存在時跳過。
//! 門檻用寬鬆值，讓 debug 測試在慢機器上也能跑；正式驗收請用
//! `cargo test --release -p jc-engine --test prewarm -- --nocapture`。

use std::time::Instant;

use jc_core::session::Session;
use jc_core::{Key, Mode};
use jc_engine::loader::load_engine_data;

/// 實機上第一鍵會變慢的四音節輸入，用來量第一鍵與預熱後的最後一鍵。
const KEYS: &str = "gp6au42k7au4";

fn type_keys(session: &mut Session, keys: &str) -> (String, f64) {
    let mut last = 0.0;
    let mut preedit = String::new();
    for ch in keys.chars() {
        let started = Instant::now();
        preedit = session.press(Key::char(ch, false)).preedit;
        last = started.elapsed().as_secs_f64() * 1000.0;
    }
    (preedit, last)
}

#[test]
fn prewarm_warms_first_key_and_keeps_select_model() {
    let Some(root) = jc_testdata::root() else {
        eprintln!("缺少測試資料：JC_TESTDATA");
        return;
    };
    let data = load_engine_data(&root).expect("載入資料");
    let mut config = data.session_config(Mode::Zh);
    // 對齊引擎 `EffectiveSettings` 的預設候選數；候選越多，select 第一次
    // 推論越接近實機。
    config.max_candidates = jc_core::session::DEFAULT_MAX_CANDIDATES;
    let had_model = jc_core::zh::select::installed().is_some();

    let started = Instant::now();
    let stats = data.prewarm(&config);
    let total = started.elapsed();

    println!("{}", stats.log_line());
    println!(
        "pipeline={:.1}ms 量測總時間={:.1}ms",
        stats.pipeline.as_secs_f64() * 1000.0,
        total.as_secs_f64() * 1000.0
    );
    assert!(stats.decode.as_nanos() > 0, "第一次解碼要有耗時");
    assert!(stats.lexicon.as_nanos() > 0, "詞典掃描要有耗時");
    assert!(total.as_millis() < 5_000, "預熱不該拖垮啟動：{total:?}");
    if had_model {
        assert!(
            jc_core::zh::select::installed().is_some(),
            "預熱後要把原本的 select 模型裝回去"
        );
    }

    let mut samples = Vec::new();
    for _ in 0..5 {
        let mut session = Session::new(config.clone());
        let (preedit, last) = type_keys(&mut session, KEYS);
        assert!(!preedit.is_empty(), "第一鍵要有組字結果");
        samples.push(last);
    }
    let cold = samples[0];
    samples.sort_by(f64::total_cmp);
    let median = samples[samples.len() / 2];
    println!("第一鍵 first={cold:.1}ms median={median:.1}ms all={samples:?}",);
    assert!(median < 500.0, "預熱後第一鍵中位數過高：{median:.1}ms");
}
