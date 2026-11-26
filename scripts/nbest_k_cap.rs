//! 量測 `max_candidates` 對注音單鍵延遲、候選數量與 State JSON 大小的影響。
//!
//! 由 `scripts/bench-nbest-k.sh` 複製到 `crates/jc-engine/tests/` 執行；直接
//! 放在 `scripts/` 是為了讓量測程式本身可以跟報告一起留下來，而不是多一個
//! 產品測試。每一列輸出都是 `RESULT <json>`，方便腳本或人工彙整。
//!
//! 需要 `JC_TESTDATA`（預設 `/mnt/work/jc/jc-ime-data`），缺少時直接失敗，
//! 不產生空表格。

use std::time::{Duration, Instant};

use jc_core::session::{Session, SessionConfig};
use jc_core::zh::keyboard::Keyboard;
use jc_core::zh::syllable::parse_reading;
use jc_core::{Key, KeyCode, Mode};
use jc_engine::loader::load_engine_data;

const K_VALUES: [usize; 5] = [20, 40, 80, 200, 500];
const SYLLABLES: [&str; 4] = ["ㄨㄛˇ", "ㄕˋ", "ㄧˋ", "ㄊㄞˊ ㄨㄢˊ"];
const REPS: usize = 200;
const WARMUP_REPS: usize = 30;
/// 每一組 (k, 音節) 跑幾輪；時序取各輪 p50／p95 的中位數，避開單輪漂移。
const PASSES: usize = 3;

/// 把讀音轉成標準（大千）排列的按鍵。
fn keys_for(reading: &str) -> String {
    let keyboard = Keyboard::standard();
    let mut keys = String::new();
    for syllable in parse_reading(reading) {
        keys.push_str(&keyboard.keys(&syllable).expect("音節有按鍵"));
    }
    keys
}

/// 單一案例、單一輪的全部樣本。
struct Samples {
    /// 每個 repetition 最後一鍵（完成音節、產生候選）的延遲。
    last_key: Vec<Duration>,
    /// 每個 repetition 所有按鍵的延遲。
    all_keys: Vec<Duration>,
    /// 開候選窗（↓）的延遲。
    down: Vec<Duration>,
    /// 打完後把快照轉成 `Response::State` 並序列化成 JSON 的延遲。
    serialize: Vec<Duration>,
    /// 打完後候選陣列長度。
    candidates: usize,
    /// 打完後 `Response::State` 的 JSON bytes。
    state_bytes: usize,
    /// 打完後候選本身佔用的記憶體估計（結構 + 字串容量）。
    candidate_bytes: usize,
    /// 開窗後 State JSON bytes。
    down_state_bytes: usize,
    /// 打完後的前幾個候選。
    top: Vec<String>,
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn state_bytes(session: &Session) -> usize {
    serde_json::to_vec(&jc_proto::Response::state(session.snapshot()))
        .expect("State 可序列化")
        .len()
}

fn candidate_bytes(session: &Session) -> usize {
    let struct_size = std::mem::size_of::<jc_core::candidate::Candidate>();
    session
        .candidates()
        .iter()
        .map(|candidate| struct_size + candidate.surface.capacity() + candidate.reading.capacity())
        .sum()
}

fn run_typing(session: &mut Session, keys: &str) -> Duration {
    let mut last = Duration::ZERO;
    for ch in keys.chars() {
        let started = Instant::now();
        session.press(Key::char(ch, false));
        last = started.elapsed();
    }
    last
}

fn measure_once(config: &SessionConfig, keys: &str) -> Samples {
    let mut samples = Samples {
        last_key: Vec::with_capacity(REPS),
        all_keys: Vec::with_capacity(REPS * keys.chars().count()),
        down: Vec::with_capacity(REPS),
        serialize: Vec::with_capacity(REPS),
        candidates: 0,
        state_bytes: 0,
        candidate_bytes: 0,
        down_state_bytes: 0,
        top: Vec::new(),
    };

    for _ in 0..WARMUP_REPS {
        let mut session = Session::new(config.clone());
        run_typing(&mut session, keys);
        session.press(Key::new(KeyCode::Down, None, false));
    }

    for _ in 0..REPS {
        let mut session = Session::new(config.clone());
        for ch in keys.chars() {
            let started = Instant::now();
            session.press(Key::char(ch, false));
            samples.all_keys.push(started.elapsed());
        }
        samples.last_key.push(*samples.all_keys.last().unwrap());
        samples.candidates = session.candidates().len();
        let started = Instant::now();
        samples.state_bytes = state_bytes(&session);
        samples.serialize.push(started.elapsed());
        samples.candidate_bytes = candidate_bytes(&session);
        samples.top = session
            .candidates()
            .iter()
            .take(8)
            .map(|candidate| candidate.surface.clone())
            .collect();
        let started = Instant::now();
        let snapshot = session.press(Key::new(KeyCode::Down, None, false));
        samples.down.push(started.elapsed());
        samples.down_state_bytes = serde_json::to_vec(&jc_proto::Response::state(snapshot))
            .expect("State 可序列化")
            .len();
    }

    samples.last_key.sort_unstable();
    samples.all_keys.sort_unstable();
    samples.down.sort_unstable();
    samples.serialize.sort_unstable();
    samples
}

fn dump(k: usize, reading: &str, keys: &str, passes: &[Samples]) {
    let last = passes.last().expect("至少一輪");
    let key = reading.replace(' ', "");
    let value = serde_json::json!({
        "k": k,
        "syllable": key,
        "keys": keys,
        "reps": REPS,
        "passes": passes.len(),
        "keys_per_rep": keys.chars().count(),
        "last_ms": {
            "p50": median(passes.iter().map(|p| p.last_key[p.last_key.len() / 2].as_secs_f64() * 1000.0).collect()),
            "p95": median(passes.iter().map(|p| percentile(&p.last_key, 95.0).as_secs_f64() * 1000.0).collect()),
            "max": passes.iter().map(|p| p.last_key.last().copied().unwrap_or_default().as_secs_f64() * 1000.0).fold(0.0, f64::max),
        },
        "all_ms": {
            "p50": median(passes.iter().map(|p| p.all_keys[p.all_keys.len() / 2].as_secs_f64() * 1000.0).collect()),
            "p95": median(passes.iter().map(|p| percentile(&p.all_keys, 95.0).as_secs_f64() * 1000.0).collect()),
            "max": passes.iter().map(|p| p.all_keys.last().copied().unwrap_or_default().as_secs_f64() * 1000.0).fold(0.0, f64::max),
        },
        "down_ms": {
            "p50": median(passes.iter().map(|p| p.down[p.down.len() / 2].as_secs_f64() * 1000.0).collect()),
            "p95": median(passes.iter().map(|p| percentile(&p.down, 95.0).as_secs_f64() * 1000.0).collect()),
            "max": passes.iter().map(|p| p.down.last().copied().unwrap_or_default().as_secs_f64() * 1000.0).fold(0.0, f64::max),
        },
        "serialize_ms": {
            "p50": median(passes.iter().map(|p| p.serialize[p.serialize.len() / 2].as_secs_f64() * 1000.0).collect()),
            "p95": median(passes.iter().map(|p| percentile(&p.serialize, 95.0).as_secs_f64() * 1000.0).collect()),
        },
        "candidates": last.candidates,
        "state_bytes": last.state_bytes,
        "down_state_bytes": last.down_state_bytes,
        "candidate_bytes": last.candidate_bytes,
        "top": last.top,
    });
    println!("RESULT {value}");
}

fn config_for(data: &jc_engine::loader::EngineData, k: usize) -> SessionConfig {
    let mut config = data.session_config(Mode::Mixed);
    config.max_candidates = k;
    // 學習會改寫候選順序，量測時固定住。
    config.learning = false;
    config
}

#[test]
fn nbest_cap_measurements() {
    let root = jc_testdata::root().expect("缺少測試資料：JC_TESTDATA");
    let data = load_engine_data(&root).expect("載入引擎資料");
    println!(
        "DATA root={} zh_entries={} has_bigram={} select_model={}",
        root.display(),
        data.zh.lexicon.len(),
        data.zh.lm.as_ref().is_some_and(|lm| lm.bigrams() > 0),
        jc_core::zh::select::installed().is_some()
    );

    let configs: Vec<(usize, SessionConfig)> = K_VALUES
        .iter()
        .map(|&k| (k, config_for(&data, k)))
        .collect();

    // 全域預熱：第一次 nbest／select 推論、詞典分頁不列入樣本。
    for (_, config) in &configs {
        for reading in SYLLABLES {
            let keys = keys_for(reading);
            for _ in 0..WARMUP_REPS {
                let mut session = Session::new(config.clone());
                run_typing(&mut session, &keys);
                session.press(Key::new(KeyCode::Down, None, false));
            }
        }
    }

    // cases[k_index][syllable_index] 收集每一輪的樣本。
    let mut cases: Vec<Vec<Vec<Samples>>> = (0..configs.len())
        .map(|_| (0..SYLLABLES.len()).map(|_| Vec::new()).collect())
        .collect();

    for pass in 0..PASSES {
        // 每一輪把 k 與音節的順序輪轉，讓 CPU 頻率與暖機效應平均落在各組。
        let mut k_order: Vec<usize> = (0..configs.len()).collect();
        let k_count = k_order.len();
        k_order.rotate_left(pass % k_count);
        for k_index in k_order {
            let (_, config) = &configs[k_index];
            let mut syllable_order: Vec<usize> = (0..SYLLABLES.len()).collect();
            let syllable_count = syllable_order.len();
            syllable_order.rotate_left(pass % syllable_count);
            for syllable_index in syllable_order {
                let keys = keys_for(SYLLABLES[syllable_index]);
                let samples = measure_once(config, &keys);
                cases[k_index][syllable_index].push(samples);
            }
        }
    }

    for (k_index, (k, _)) in configs.iter().enumerate() {
        for (syllable_index, reading) in SYLLABLES.iter().enumerate() {
            dump(
                *k,
                reading,
                &keys_for(reading),
                &cases[k_index][syllable_index],
            );
        }
    }
}
