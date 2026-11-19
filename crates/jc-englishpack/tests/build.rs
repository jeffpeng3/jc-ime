//! 建置器行為測試；對應前身 `internal/englishpack/pack_test.go`。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use jc_core::mixed::{sha256_hex, Dictionary, EnglishManifest, EnglishSource, Word};
use jc_englishpack::{compile, import_lists, load, write};

const LICENSE: &[u8] = b"Example copyright and permission text\n";
const FIXTURE_TSV: &[u8] = b"example\t35\t0\nexcluded\t50\t1\n";

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("jc-englishpack-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建立暫存目錄");
    dir
}

fn fixture_source(tag: &str) -> PathBuf {
    let src = temp_dir(tag);
    let metadata = serde_json::json!({
        "source": {
            "name": "Test source",
            "version": "1",
            "url": "https://example.test",
            "sha256": sha256_hex(b"upstream"),
            "license": "test",
            "attribution": "test author"
        },
        "normalized_sha256": sha256_hex(FIXTURE_TSV),
        "license_sha256": sha256_hex(LICENSE),
        "words": 2
    });
    write_file(&src.join("esdb.tsv"), FIXTURE_TSV);
    write_file(&src.join("LICENSE.txt"), LICENSE);
    write_file(&src.join("technical.tsv"), b"example\t300\n");
    write_file(
        &src.join("source.json"),
        serde_json::to_string(&metadata).expect("JSON").as_bytes(),
    );
    src
}

fn write_file(path: &Path, data: &[u8]) {
    std::fs::write(path, data).expect("寫入測試檔");
}

fn read_file(path: &Path) -> Vec<u8> {
    std::fs::read(path).expect("讀取測試檔")
}

#[test]
fn test_compile_deterministic_and_no_suggest_round_trip() {
    let src = fixture_source("compile");
    let out1 = temp_dir("compile-out1").join("pack");
    let out2 = temp_dir("compile-out2").join("pack");
    for out in [&out1, &out2] {
        compile(&src, out).expect("建置");
    }
    for name in ["words.bin", "manifest.json", "NOTICE.txt"] {
        assert_eq!(
            read_file(&out1.join(name)),
            read_file(&out2.join(name)),
            "{name} 不一致"
        );
    }
    let (dict, manifest) = load(&out1).expect("載入");
    assert_eq!(dict.len(), 2);
    assert_eq!(manifest.words, 2);
    assert!(dict.has("excluded"));
    assert!(dict.prefix("excl", 5).is_empty());
    let prefix = dict.prefix("ex", 1);
    assert_eq!(prefix[0].text, "example");
    assert_eq!(prefix[0].weight, 300.0);
    assert!(compile(&src, &out1).is_err(), "覆寫了既有資料包");
}

#[test]
fn test_rejected_source_does_not_publish_or_overwrite() {
    for name in ["esdb.tsv", "LICENSE.txt"] {
        let src = fixture_source(&format!("reject-{name}"));
        let out = temp_dir(&format!("reject-out-{name}")).join("pack");
        write_file(&src.join(name), b"tampered");
        assert!(compile(&src, &out).is_err(), "接受了未驗證的 {name}");
        assert!(!out.exists(), "拒絕後仍寫出產物");
    }
    let src = fixture_source("reject-weight");
    write_file(&src.join("technical.tsv"), b"example\tNaN\n");
    let out = temp_dir("reject-weight-out").join("pack");
    assert!(compile(&src, &out).is_err(), "接受了不合法的權重");
}

#[test]
fn test_load_rejects_corruption_and_missing_attribution() {
    for name in ["words.bin", "NOTICE.txt", "manifest.json"] {
        let out = temp_dir(&format!("corrupt-out-{name}")).join("pack");
        compile(&fixture_source(&format!("corrupt-{name}")), &out).expect("建置");
        write_file(&out.join(name), b"tampered");
        assert!(load(&out).is_err(), "接受了損壞的 {name}");
    }
    let out = temp_dir("no-source-out").join("pack");
    let dict = Dictionary::new([Word::new("example", 1.0)]);
    let manifest = EnglishManifest {
        format: 1,
        words: 1,
        ranking: "test".to_string(),
        sources: Vec::new(),
        inputs: BTreeMap::new(),
        files: BTreeMap::new(),
    };
    assert!(
        write(&out, &dict, manifest, b"test").is_err(),
        "缺來源仍寫出"
    );
    assert!(!out.exists());
}

#[test]
fn test_import_lists_matches_reviewed_normalization() {
    let src = fixture_source("import");
    let exports = temp_dir("import-exports");
    let mut metadata: serde_json::Value =
        serde_json::from_slice(&read_file(&src.join("source.json"))).expect("讀取 source.json");
    let data = b"Example\nexample\nexcluded!\nfoo_bar\nword's\n";
    let mut rows = Vec::new();
    for size in [10, 20, 35, 40, 50, 55, 60] {
        let name = format!("{size}.txt");
        write_file(&exports.join(&name), data);
        rows.push(serde_json::json!({
            "size": size,
            "file": name,
            "sha256": sha256_hex(data)
        }));
    }
    let want = b"example\t10\t0\nexcluded\t10\t1\n";
    metadata["exports"] = serde_json::Value::Array(rows);
    metadata["normalized_sha256"] = sha256_hex(want).into();
    write_file(
        &src.join("source.json"),
        serde_json::to_string(&metadata).expect("JSON").as_bytes(),
    );
    import_lists(&src, &exports).expect("匯入");
    assert_eq!(read_file(&src.join("esdb.tsv")), want, "正規化結果不同");
    write_file(&exports.join("60.txt"), b"tampered");
    assert!(import_lists(&src, &exports).is_err(), "接受了未驗證的匯出");
    assert_eq!(
        read_file(&src.join("esdb.tsv")),
        want,
        "失敗的匯入改動了來源"
    );
}

#[test]
fn test_product_pack_loads() {
    let words = jc_testdata::require_data!("english/words.bin");
    let dir = words.parent().expect("資料包目錄");
    let (dict, manifest) = load(dir).expect("載入英文資料包");
    assert_eq!(manifest.format, 1);
    assert_eq!(dict.len(), manifest.words);
    assert!(dict.len() > 80_000, "詞數 {}", dict.len());
    assert!(dict.has("hello"));
    assert_eq!(dict.corrections("recieve", 1)[0].text, "receive");
    let source: &EnglishSource = &manifest.sources[0];
    assert!(source.commit.is_some(), "manifest 沒有來源 commit");
}
