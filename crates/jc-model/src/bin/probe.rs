//! 手動探測：載入 reranker 與標點模型各跑一次推論並印延遲。
//!
//! ```text
//! cargo run -p jc-model --features onnx --bin probe -- \
//!   [--models /mnt/work/jc/jc-ime-data/models] [--ort-lib /path/libonnxruntime.so]
//! ```
//!
//! 只在使用 `--features onnx` 時編譯（`required-features`）。

use std::path::PathBuf;
use std::time::Instant;

use jc_model::{ModelSpec, PunctuationModel, Reranker, ZhPunctuationModel};

fn main() {
    let mut models = PathBuf::from("/mnt/work/jc/jc-ime-data/models");
    let mut rerank_dir: Option<PathBuf> = None;
    let mut punct_dir: Option<PathBuf> = None;
    let mut ort_lib: Option<PathBuf> = None;
    let mut left = String::from("今日は");
    let mut text = String::from("今日は晴れです");
    let mut zh_text = String::from("好的我知道了明天見");

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .unwrap_or_else(|| usage(&format!("{arg} 缺少參數")))
        };
        match arg.as_str() {
            "--models" => models = PathBuf::from(value()),
            "--rerank-dir" => rerank_dir = Some(PathBuf::from(value())),
            "--punct-dir" => punct_dir = Some(PathBuf::from(value())),
            "--ort-lib" => ort_lib = Some(PathBuf::from(value())),
            "--left" => left = value(),
            "--text" => text = value(),
            "--zh-text" => zh_text = value(),
            "-h" | "--help" => {
                println!(
                    "用法：probe [--models DIR] [--rerank-dir DIR] [--punct-dir DIR] \
                     [--ort-lib LIB] [--left TEXT] [--text TEXT] [--zh-text TEXT]"
                );
                return;
            }
            other => usage(&format!("未知參數：{other}")),
        }
    }

    let rerank_dir = rerank_dir.unwrap_or_else(|| models.join("reranker-extra"));
    let punct_dir = punct_dir.unwrap_or_else(|| models.join("punctuation-public-core"));
    let spec = ModelSpec::default();

    let reranker = match Reranker::load_with_spec(&rerank_dir, &spec, ort_lib.as_deref()) {
        Ok(r) => r,
        Err(e) => fail(&format!("reranker 載入失敗：{e}")),
    };
    let candidates = ["東京", "東京都"];
    let started = Instant::now();
    let scores = reranker.score_batch(Some(&left), "", &candidates);
    let rerank_ms = started.elapsed().as_secs_f64() * 1000.0;
    match &scores {
        Some(scores) => println!(
            "reranker：{} 個候選，延遲 {rerank_ms:.2} ms，分數 {scores:.4?}",
            scores.len()
        ),
        None => println!("reranker：延遲 {rerank_ms:.2} ms，逾時或失敗（回 None）"),
    }

    let punct = match PunctuationModel::load_with_spec(&punct_dir, &spec, ort_lib.as_deref()) {
        Ok(p) => p,
        Err(e) => fail(&format!("標點模型載入失敗：{e}")),
    };
    let started = Instant::now();
    let logits = punct.logits(&text);
    let punct_ms = started.elapsed().as_secs_f64() * 1000.0;
    match &logits {
        Some(rows) => println!(
            "punctuation：{} token，延遲 {punct_ms:.2} ms，classes {}",
            rows.len(),
            rows.first().map(|r| r.len()).unwrap_or(0)
        ),
        None => println!("punctuation：延遲 {punct_ms:.2} ms，逾時或失敗（回 None）"),
    }
    println!("punctuate：{:?} -> {:?}", text, punct.punctuate(&text));

    let zh_dir = models.join("zh-punct");
    let zh = match ZhPunctuationModel::load_with_spec(&zh_dir, &spec, ort_lib.as_deref()) {
        Ok(model) => model,
        Err(e) => fail(&format!("注音標點模型載入失敗：{e}")),
    };
    let started = Instant::now();
    let zh_out = zh.punctuate(&zh_text);
    let zh_ms = started.elapsed().as_secs_f64() * 1000.0;
    println!(
        "zh punctuate：{:?} -> {:?}（{zh_ms:.2} ms）",
        zh_text, zh_out
    );

    if scores.is_none() || logits.is_none() {
        std::process::exit(2);
    }
}

fn usage(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}
