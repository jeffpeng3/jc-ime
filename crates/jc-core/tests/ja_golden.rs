//! 日文轉換的 golden 快照。
//!
//! 快照在重構前由現行程式碼產生（`JC_JA_GOLDEN_UPDATE=1`），之後只做比對：
//! 詞庫版面的改動不得改變任何一句的轉換字面。題庫用 `0c-testdata/ja` 的
//! `modern_500`、`hard_cases`、`ajimee_200`，加上 `decode.rs` real-pack 測試
//! 的固定讀音。資料包不存在時跳過，不讓沒有資料的環境失敗。

use std::path::{Path, PathBuf};

use jc_core::ja::decode::{JaContext, JaDecoder};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const PACK: &str = "packs/ja-lexicon-mozc-v2.pack";

/// 快照檔的版號；格式改變時遞增。
const GOLDEN_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Golden {
    version: u32,
    items: Vec<GoldenItem>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct GoldenItem {
    id: String,
    input: String,
    surfaces: Vec<String>,
}

/// 題庫檔案與 id 前綴。
const CORPORA: [(&str, &str); 3] = [
    ("0c-testdata/ja/modern_500.json", "modern"),
    ("0c-testdata/ja/hard_cases.json", "hard"),
    ("0c-testdata/ja/ajimee_200.json", "ajimee"),
];

/// `decode.rs` real-pack 測試用到的讀音，外加助詞改寫與同音詞邊界。
const EXTRA_INPUTS: [(&str, &str); 14] = [
    ("decode-watashiwa", "わたしわ"),
    ("decode-watashiwa-plain", "わたしわ"),
    ("decode-konnichiwa", "こんにちわ"),
    ("decode-aruiha", "あるいわ"),
    ("decode-honwoyomu", "ほんおよむ"),
    ("decode-mizuonomu", "みずおのむ"),
    ("decode-nihoneiku", "にほんえいく"),
    ("decode-gakkoueiku", "がっこうえいく"),
    ("decode-eki", "えき"),
    ("decode-ocha", "おちゃ"),
    ("decode-sunawachi", "すなわち"),
    ("decode-iwaba", "いわば"),
    ("decode-hashi", "はし"),
    ("decode-32kana", "あじをしめるみせにあじをしめるみせにあじをしめるみせにねこねこね"),
];

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("ja-golden.json")
}

/// 讀題庫的 `items[].reading`，id 用 `<前綴>-<原始 id>`。
fn corpus_items(path: &Path, prefix: &str) -> Vec<(String, String)> {
    let raw = std::fs::read_to_string(path).unwrap_or_else(|err| panic!("讀 {} 失敗：{err}", path.display()));
    let value: Value = serde_json::from_str(&raw).unwrap_or_else(|err| panic!("解析 {} 失敗：{err}", path.display()));
    let items = value["items"].as_array().unwrap_or_else(|| panic!("{} 沒有 items", path.display()));
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let reading = item["reading"]
                .as_str()
                .unwrap_or_else(|| panic!("{} 第 {index} 筆沒有 reading", path.display()));
            let id = item["id"]
                .as_str()
                .map(|id| format!("{prefix}-{id}"))
                .unwrap_or_else(|| format!("{prefix}-{index:04}"));
            (id, reading.to_owned())
        })
        .collect()
}

fn inputs() -> Option<Vec<(String, String)>> {
    let mut out = Vec::new();
    for (rel, prefix) in CORPORA {
        let path = jc_testdata::path(rel)?;
        out.extend(corpus_items(&path, prefix));
    }
    out.extend(
        EXTRA_INPUTS
            .iter()
            .map(|(id, reading)| ((*id).to_owned(), (*reading).to_owned())),
    );
    Some(out)
}

fn generate(decoder: &JaDecoder) -> Golden {
    let ctx = JaContext::default();
    let items = inputs()
        .expect("已確認資料存在")
        .into_iter()
        .map(|(id, input)| {
            let mut surfaces: Vec<String> = decoder
                .results(&input.chars().collect::<Vec<_>>(), &ctx, 3)
                .into_iter()
                .map(|result| result.surface())
                .collect();
            if surfaces.is_empty() {
                surfaces.push(String::new());
            }
            GoldenItem {
                id,
                input,
                surfaces,
            }
        })
        .collect();
    Golden {
        version: GOLDEN_VERSION,
        items,
    }
}

#[test]
fn test_ja_conversion_matches_golden() {
    let Some(pack) = jc_testdata::path(PACK) else {
        eprintln!("缺少測試資料：{PACK}");
        return;
    };
    let decoder = JaDecoder::open_with_context(&pack)
        .expect("開啟日文 pack")
        .with_particle_rewrite(true);
    let actual = generate(&decoder);

    if std::env::var_os("JC_JA_GOLDEN_UPDATE").is_some() {
        let path = fixture_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("建立 fixtures 目錄");
        }
        let mut json = serde_json::to_string_pretty(&actual).expect("序列化 golden");
        json.push('\n');
        std::fs::write(&path, json).expect("寫入 golden");
        eprintln!("已更新 golden：{}（{} 筆）", path.display(), actual.items.len());
        return;
    }

    let path = fixture_path();
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "讀 golden {} 失敗：{err}；用 JC_JA_GOLDEN_UPDATE=1 重新產生",
            path.display()
        )
    });
    let expected: Golden = serde_json::from_str(&raw).expect("解析 golden");

    assert_eq!(
        expected.version, GOLDEN_VERSION,
        "golden 版號不符，請重新產生"
    );
    assert_eq!(
        expected.items.len(),
        actual.items.len(),
        "golden 筆數不符（預期 {}，實際 {}）",
        expected.items.len(),
        actual.items.len()
    );
    for (want, got) in expected.items.iter().zip(&actual.items) {
        assert_eq!(want.id, got.id, "golden 順序不符");
        assert_eq!(
            want, got,
            "{}（{}）轉換結果改變：預期 {:?}，實際 {:?}",
            want.id, want.input, want.surfaces, got.surfaces
        );
    }
}
