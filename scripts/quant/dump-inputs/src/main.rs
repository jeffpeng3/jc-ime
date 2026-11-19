//! 從正式程式路徑 dump 量化評測用的固定輸入。
//!
//! 只用公開 API：`jc_model` 的 tokenizer／layout、`jc_core` 的
//! `select::prepare`。輸出到 `/tmp/opencode/quant-inputs/`，供
//! `scripts/quant/measure_onnx.py` 使用。
//!
//! 注音候選由 `jc-cli convert` 產生，確保與產品統計解碼一致。
//! 執行前先 `cargo build --release -p jc-cli`。

use std::fs;
use std::path::Path;
use std::process::Command;

use jc_core::zh::select::{prepare, Request, SelectManifest, Vocabulary};
use jc_model::layout::pack_batch;
use jc_model::manifest::ModelSpec;
use jc_model::Tokenizer;
use serde_json::{json, Value};

const REPO: &str = "/mnt/work/jc/wt/quant-study";
const MODELS: &str = "/mnt/work/jc/jc-ime-data/models/quant";
const TESTDATA: &str = "/mnt/work/jc/jc-ime-data/0c-testdata";
const DATA_ZH: &str = "/mnt/work/jc/jc-ime-data/build-zh";
const OUT: &str = "/tmp/opencode/quant-inputs";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(OUT)?;
    let corpus: Value =
        serde_json::from_slice(&fs::read(format!("{TESTDATA}/ja/modern_500.json"))?)?;
    let items = corpus["items"].as_array().expect("items");
    dump_rerank(items)?;
    dump_punct(items)?;
    dump_zh_select()?;
    Ok(())
}

fn dump_rerank(items: &[Value]) -> Result<(), Box<dyn std::error::Error>> {
    let dir = Path::new(MODELS).join("ja-rerank");
    let tokenizer = Tokenizer::load(dir.join("vocab.json"))?;
    let spec = ModelSpec::default();
    let mut samples = Vec::new();
    for (idx, item) in items.iter().take(40).enumerate() {
        let segments: Vec<String> = item["segments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let mut candidates: Vec<String> = segments.clone();
        let mut cursor = idx + 1;
        while candidates.len() < 8 && cursor < items.len() {
            for seg in items[cursor]["segments"].as_array().unwrap() {
                candidates.push(seg.as_str().unwrap().to_string());
                if candidates.len() == 8 {
                    break;
                }
            }
            cursor += 1;
        }
        let left = if idx == 0 {
            None
        } else {
            Some(items[idx - 1]["surface"].as_str().unwrap())
        };
        let refs: Vec<&str> = candidates.iter().map(String::as_str).collect();
        let batch = pack_batch(&tokenizer, &spec, left, None, &refs)?;
        samples.push(json!({
            "id": item["id"],
            "ids": batch.ids,
            "mask": batch.mask,
            "rows": batch.rows,
            "seq": batch.seq,
        }));
    }
    fs::write(format!("{OUT}/rerank.json"), serde_json::to_vec(&samples)?)?;
    Ok(())
}

fn window_starts(n: usize, window: usize, stride: usize) -> Vec<usize> {
    if n == 0 || window == 0 {
        return Vec::new();
    }
    if n <= window {
        return vec![0];
    }
    let mut out = Vec::new();
    let mut start = 0;
    while start < n - window {
        out.push(start);
        start += stride;
    }
    out.push(n - window);
    out
}

fn dump_punct(items: &[Value]) -> Result<(), Box<dyn std::error::Error>> {
    let dir = Path::new(MODELS).join("ja-punct");
    let tokenizer = Tokenizer::load(dir.join("vocab.json"))?;
    let mut windows = Vec::new();
    for (group, chunk) in items.chunks(50).take(10).enumerate() {
        let text: String = chunk
            .iter()
            .map(|item| item["surface"].as_str().unwrap())
            .collect();
        let (tokens, _norm, _map) = tokenizer.encode_with_map(&text);
        for start in window_starts(tokens.len(), 256, 128) {
            let end = (start + 256).min(tokens.len());
            let ids: Vec<i64> = tokens[start..end].iter().map(|s| i64::from(s.id)).collect();
            windows.push(json!({ "source": format!("group-{group}"), "ids": ids }));
        }
    }
    fs::write(format!("{OUT}/punct.json"), serde_json::to_vec(&windows)?)?;
    Ok(())
}

/// 用 `jc-cli convert` 取得 golden 讀音的統計 N-best，再過濾成同字數候選。
fn zh_cases() -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let golden: Value =
        serde_json::from_slice(&fs::read(format!("{TESTDATA}/eval/zhuyin-golden.json"))?)?;
    let items = golden["items"].as_array().expect("items");
    let mut cases = Vec::new();
    let mut prev = String::new();
    for item in items {
        let reading = item["reading"].as_str().unwrap();
        let syllables: Vec<String> = reading.split_whitespace().map(str::to_string).collect();
        if syllables.len() < 2 {
            continue;
        }
        let output = Command::new(format!("{REPO}/target/release/jc-cli"))
            .args([
                "convert",
                reading,
                "--data",
                DATA_ZH,
                "--n",
                "16",
                "--json",
                "--select-model",
                "off",
            ])
            .output()?;
        let parsed: Vec<Value> = serde_json::from_slice(&output.stdout)?;
        let mut candidates: Vec<String> = Vec::new();
        for row in parsed {
            let surface = row["surface"].as_str().unwrap_or_default().to_string();
            if surface.chars().count() == syllables.len() && !candidates.contains(&surface) {
                candidates.push(surface);
            }
        }
        if candidates.len() < 2 {
            continue;
        }
        candidates.truncate(16);
        cases.push(json!({
            "id": item["id"],
            "context": prev,
            "reading": syllables,
            "candidates": candidates,
        }));
        prev = item["surface"].as_str().unwrap_or_default().to_string();
    }
    Ok(cases)
}

fn dump_zh_select() -> Result<(), Box<dyn std::error::Error>> {
    let dir = Path::new(MODELS).join("zh-select");
    let vocab = Vocabulary::load(dir.join("vocab.json"))?;
    let manifest = SelectManifest::load(dir.join("manifest.json"))?;
    let cases = zh_cases()?;
    let mut out = Vec::new();
    for case in &cases {
        let context = case["context"].as_str().unwrap_or("");
        let reading: Vec<String> = case["reading"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let candidates: Vec<String> = case["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let refs: Vec<&str> = candidates.iter().map(String::as_str).collect();
        let arrays = prepare(
            &vocab,
            &manifest,
            &Request {
                context,
                reading: &reading,
                candidates: &refs,
            },
        )?;
        out.push(json!({
            "id": case["id"],
            "candidates": candidates,
            "ids": arrays.ids,
            "bases": arrays.bases,
            "tones": arrays.tones,
            "types": arrays.types,
            "positions": arrays.positions,
            "mask": arrays.mask,
            "proj_pos": arrays.proj_pos,
            "target_node": arrays.target_node,
            "target_ids": arrays.target_ids,
        }));
    }
    fs::write(format!("{OUT}/zh-select.json"), serde_json::to_vec(&out)?)?;
    Ok(())
}
