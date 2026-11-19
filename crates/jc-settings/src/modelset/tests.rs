//! 模型包資料模型的測試。
//!
//! 從前身 `internal/modelsets` 的 `manifest_test.go`、`placeholder_test.go`
//! 搬過來。需要 repo 內正式 lock 檔的測試不在這裡（見
//! `docs/port/settings-core.md`）。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::install::{install, read_selection, selection_result, InstallOptions};
use super::manifest::{
    dictionary_pack_id, dictionary_sha256, hex_digest, parse_lock, Artifact, ArtifactAsset,
    ArtifactFile, Dictionary, HfFile, HfSource, Lock, ModelSetError, Options, Tensor, FORMAT_FP32,
    FORMAT_INT8, KIND_PUNCTUATION, KIND_RERANKER, QUALITY_UNTRAINED, SCHEMA, VARIANT_CC_BY_SA,
    VARIANT_PUBLIC_CORE,
};

fn tmpdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "jc-settings-ms-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("建立測試目錄");
    dir
}

fn synthetic_pack(tag: &str) -> (PathBuf, String) {
    let dir = tmpdir(tag).join("ja");
    fs::create_dir_all(&dir).expect("建立字典包目錄");
    let body = b"{\"format\":2,\"language\":\"ja\",\"lexicon\":\"ja-test\",\"files\":{}}\n";
    fs::write(dir.join("manifest.json"), body).expect("寫入字典 manifest");
    let sha = dictionary_sha256(&dir).expect("計算字典雜湊");
    (dir, sha)
}

fn digest_bytes(bytes: &[u8]) -> String {
    hex_digest(bytes)
}

fn synthetic_license(variant: &str) -> Value {
    if variant == VARIANT_PUBLIC_CORE {
        json!({"id": "MIT", "scope": "public-core", "distribution": "MIT only", "notice": "legal/MIT.txt"})
    } else {
        json!({"id": "CC-BY-SA-4.0", "scope": "cc-by-sa", "distribution": "carry notice", "notice": "legal/CC-BY-SA-4.0.txt"})
    }
}

fn synthetic_artifact(
    id: &str,
    kind: &str,
    variant: &str,
    rel: &str,
    size: i64,
    dict_sha: &str,
) -> Value {
    let output = if kind == KIND_PUNCTUATION {
        json!({"name": "logits", "dtype": "float32", "shape": ["batch", "seq", 4]})
    } else {
        json!({"name": "scores", "dtype": "float32", "shape": ["batch"]})
    };
    json!({
        "manifest": format!("{id}/manifest.json"),
        "artifact_id": id,
        "kind": kind,
        "variant": variant,
        "version": "test-1",
        "parameters": 42,
        "placeholder": true,
        "quality": QUALITY_UNTRAINED,
        "production_eligible": false,
        "dictionary_manifest_sha256": dict_sha,
        "license": synthetic_license(variant),
        "attribution": ["legal/copyright"],
        "source": {
            "training_data": "none (untrained placeholder)",
            "datasets": [],
            "checkpoint": "",
            "generator": "test",
            "fingerprint": "f",
        },
        "files": {
            FORMAT_INT8: {
                "path": rel,
                "bytes": size,
                "sha256": digest_bytes(format!("synthetic-onnx:{id}\n").as_bytes()),
                "quantization": "dynamic-quint8",
            }
        },
        "io": {
            "inputs": [
                {"name": "input_ids", "dtype": "int64", "shape": ["batch", "seq"]},
                {"name": "attention_mask", "dtype": "int64", "shape": ["batch", "seq"]},
            ],
            "outputs": [output],
        },
    })
}

fn synthetic_package(dict_sha: &str, artifacts: Vec<Value>) -> Value {
    json!({
        "schema": SCHEMA,
        "updated": "2026-11-19",
        "version": "test-1",
        "dictionary": {
            "pack_id": "ja-test",
            "language": "ja",
            "format": 2,
            "version": "test",
            "manifest_sha256": dict_sha,
            "source": "test",
        },
        "deployment": {
            "sequence_length": 124,
            "context_tokens_per_side": 60,
            "gating_top_k": 3,
            "format": "int8",
        },
        "license": synthetic_license(VARIANT_CC_BY_SA),
        "attribution": ["legal/copyright"],
        "placeholder": true,
        "quality": QUALITY_UNTRAINED,
        "production_eligible": false,
        "artifacts": artifacts,
    })
}

struct Fixture {
    root: PathBuf,
    lock: Lock,
    lock_raw: Vec<u8>,
    dict_sha: String,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn new_fixture(tag: &str) -> Fixture {
    let (_, dict_sha) = synthetic_pack(tag);
    let base = tmpdir(&format!("{tag}-models"));
    let root = base.join("models");
    fs::create_dir_all(&root).expect("建立模型根目錄");
    let make_artifact = |id: &str, kind: &str, variant: &str| -> Value {
        let dir = root.join(id);
        fs::create_dir_all(&dir).expect("建立 artifact 目錄");
        let name = format!("{id}.placeholder.int8.onnx");
        let body = format!("synthetic-onnx:{id}\n");
        fs::write(dir.join(&name), body.as_bytes()).expect("寫入假 ONNX");
        let rel = format!("{id}/{name}");
        synthetic_artifact(id, kind, variant, &rel, body.len() as i64, &dict_sha)
    };
    let doc = json!({
        "schema": 1,
        "model_package": synthetic_package(&dict_sha, vec![
            make_artifact("reranker", KIND_RERANKER, VARIANT_CC_BY_SA),
            make_artifact("punctuation-public-core", KIND_PUNCTUATION, VARIANT_PUBLIC_CORE),
        ]),
    });
    let raw = serde_json::to_vec_pretty(&doc).expect("序列化 lock");
    let lock = parse_lock(&raw).expect("解析 lock");
    Fixture {
        root: base,
        lock,
        lock_raw: raw,
        dict_sha,
    }
}

fn validate_all(f: &Fixture) -> Result<super::manifest::ValidationResult, ModelSetError> {
    f.lock
        .model_package
        .as_ref()
        .expect("模型包")
        .validate(&Options {
            allow_placeholders: true,
            release: false,
            installed_dictionary_sha256: f.dict_sha.clone(),
            installed_dictionary_pack_id: String::new(),
            artifact_root: f.root.join("models"),
            format: FORMAT_INT8.to_string(),
            verify_files: true,
        })
}

#[test]
fn test_parse_lock_accepts_synthetic_package() {
    let f = new_fixture("accepts");
    let pkg = f.lock.model_package.as_ref().expect("模型包");
    assert_eq!(pkg.version, "test-1");
    assert_eq!(pkg.artifacts.len(), 2);
    assert_eq!(pkg.artifacts[0].kind, KIND_RERANKER);
    assert_eq!(pkg.artifacts[1].kind, KIND_PUNCTUATION);
}

#[test]
fn test_parse_lock_rejects_wrong_artifact_variant() {
    let f = new_fixture("wrong-variant");
    let mut doc: Value = serde_json::from_slice(&f.lock_raw).expect("解析");
    doc["model_package"]["artifacts"][0]["variant"] = json!(VARIANT_PUBLIC_CORE);
    let raw = serde_json::to_vec(&doc).expect("序列化");
    let err = parse_lock(&raw).expect_err("reranker 不是 cc-by-sa 應被拒絕");
    assert!(matches!(err, ModelSetError::VariantMixed(_)), "{err:?}");
}

#[test]
fn test_parse_lock_rejects_alternative_fields() {
    let f = new_fixture("alternative");
    let mut doc: Value = serde_json::from_slice(&f.lock_raw).expect("解析");
    doc["model_sets"] = json!({"schema": "0c.model-sets/1", "sets": {}});
    doc.as_object_mut().expect("物件").remove("model_package");
    let raw = serde_json::to_vec(&doc).expect("序列化");
    let err = parse_lock(&raw).expect_err("舊 model_sets 應被拒絕");
    assert!(matches!(err, ModelSetError::Schema(_)), "{err:?}");
}

#[test]
fn test_validate_rejects_dictionary_mismatch() {
    let f = new_fixture("dict-mismatch");
    let pkg = f.lock.model_package.as_ref().expect("模型包");
    let wrong = "b".repeat(64);
    let err = pkg
        .validate(&Options {
            allow_placeholders: true,
            installed_dictionary_sha256: wrong,
            artifact_root: f.root.join("models"),
            verify_files: true,
            ..Options::default()
        })
        .expect_err("字典不符應被拒絕");
    assert!(
        matches!(err, ModelSetError::DictionaryMismatch(_)),
        "{err:?}"
    );
    let err = pkg
        .validate(&Options {
            allow_placeholders: true,
            artifact_root: f.root.join("models"),
            verify_files: true,
            ..Options::default()
        })
        .expect_err("沒有已安裝字典應被拒絕");
    assert!(
        matches!(err, ModelSetError::DictionaryUnknown(_)),
        "{err:?}"
    );
    let err = pkg
        .validate(&Options {
            allow_placeholders: true,
            installed_dictionary_sha256: f.dict_sha.clone(),
            installed_dictionary_pack_id: "other".to_string(),
            artifact_root: f.root.join("models"),
            verify_files: true,
            ..Options::default()
        })
        .expect_err("字典包 ID 不符應被拒絕");
    assert!(
        matches!(err, ModelSetError::DictionaryMismatch(_)),
        "{err:?}"
    );
}

#[test]
fn test_validate_placeholder_gate() {
    let f = new_fixture("placeholder-gate");
    let pkg = f.lock.model_package.as_ref().expect("模型包");
    let err = pkg
        .validate(&Options {
            installed_dictionary_sha256: f.dict_sha.clone(),
            artifact_root: f.root.join("models"),
            verify_files: true,
            ..Options::default()
        })
        .expect_err("未允許 placeholder 應被拒絕");
    assert!(matches!(err, ModelSetError::Placeholder(_)), "{err:?}");

    let result = validate_all(&f).expect("允許 placeholder 時應通過");
    assert!(result.placeholder);
    assert_eq!(result.quality, QUALITY_UNTRAINED);
    assert_eq!(result.artifacts.len(), 2);
    assert!(result.dictionary_matches);
    assert_eq!(result.artifacts[0].format, FORMAT_INT8);
}

#[test]
fn test_validate_rejects_file_hash_mismatch() {
    let f = new_fixture("hash-mismatch");
    let path = f
        .root
        .join("models/reranker/reranker.placeholder.int8.onnx");
    fs::write(&path, "tampered").expect("改寫檔案");
    let err = validate_all(&f).expect_err("檔案雜湊不符應被拒絕");
    assert!(matches!(err, ModelSetError::FileMismatch(_)), "{err:?}");
}

#[test]
fn test_validate_rejects_tensor_contract() {
    let f = new_fixture("tensor");
    let mut lock = f.lock.clone();
    let pkg = lock.model_package.as_mut().expect("模型包");
    pkg.artifacts[0].io.outputs[0].shape = vec![json!("batch"), json!("seq")];
    let err = pkg
        .validate(&Options {
            allow_placeholders: true,
            installed_dictionary_sha256: f.dict_sha.clone(),
            ..Options::default()
        })
        .expect_err("reranker 輸出形狀應被拒絕");
    assert!(matches!(err, ModelSetError::TensorContract(_)), "{err:?}");
}

#[test]
fn test_validate_rejects_unmarked_placeholder() {
    let f = new_fixture("unmarked");
    let mut lock = f.lock.clone();
    let pkg = lock.model_package.as_mut().expect("模型包");
    pkg.placeholder = false;
    pkg.quality = "trained".to_string();
    for artifact in &mut pkg.artifacts {
        artifact.placeholder = false;
        artifact.quality = "trained".to_string();
    }
    let err = pkg
        .validate_structure()
        .expect_err("placeholder 標記不可移除");
    assert!(
        matches!(err, ModelSetError::PlaceholderUnmarked(_)),
        "{err:?}"
    );
}

#[test]
fn test_install_writes_one_package() {
    let f = new_fixture("install");
    let dest = f.root.join("dest");
    let selection = install(
        &f.lock,
        &InstallOptions {
            source_root: f.root.join("models"),
            dest_root: dest.clone(),
            allow_placeholders: true,
            installed_dictionary_sha256: f.dict_sha.clone(),
            ..InstallOptions::default()
        },
    )
    .expect("安裝應成功");
    assert_eq!(selection.version, "test-1");
    assert!(selection.placeholder);
    assert_eq!(selection.artifacts.len(), 2);
    for rel in [
        "reranker/reranker.placeholder.int8.onnx",
        "punctuation-public-core/punctuation-public-core.placeholder.int8.onnx",
    ] {
        assert!(dest.join(rel).is_file(), "缺少 {rel}");
    }
    let read = read_selection(&dest).expect("讀回 active.json");
    assert_eq!(read.version, selection.version);
    assert_eq!(read.artifacts.len(), 2);
    selection_result(&f.lock, &read, &dest, &f.dict_sha, true).expect("使用中的模型包應可驗證");

    let err = install(
        &f.lock,
        &InstallOptions {
            source_root: f.root.join("models"),
            dest_root: dest,
            allow_placeholders: true,
            installed_dictionary_sha256: f.dict_sha.clone(),
            ..InstallOptions::default()
        },
    )
    .expect_err("重裝應要求 force");
    assert!(matches!(err, ModelSetError::Selection(_)), "{err:?}");
}

#[test]
fn test_install_refuses_placeholder_without_setting() {
    let f = new_fixture("install-gate");
    let err = install(
        &f.lock,
        &InstallOptions {
            source_root: f.root.join("models"),
            dest_root: f.root.join("dest"),
            installed_dictionary_sha256: f.dict_sha.clone(),
            ..InstallOptions::default()
        },
    )
    .expect_err("安裝 placeholder 應要求允許");
    assert!(matches!(err, ModelSetError::Placeholder(_)), "{err:?}");
}

#[test]
fn test_dictionary_pack_id() {
    let (pack, _) = synthetic_pack("pack-id");
    assert_eq!(dictionary_pack_id(&pack), "ja-test");
    assert_eq!(dictionary_pack_id(Path::new("")), "");
}

#[test]
fn test_install_copies_assets_and_records_them() {
    let f = new_fixture("install-assets");
    let mut lock = f.lock.clone();
    let pkg = lock.model_package.as_mut().expect("模型包");
    for artifact in &mut pkg.artifacts {
        let dir = artifact.package_dir();
        let mut assets = Vec::new();
        for name in ["manifest.json", "vocab.json", "tokenizer.model"] {
            let body = format!("{name}:{}", artifact.artifact_id);
            let rel = format!("{dir}/{name}");
            let path = f.root.join("models").join(&rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("建立 asset 目錄");
            }
            fs::write(&path, body.as_bytes()).expect("寫入 asset");
            assets.push(ArtifactAsset {
                name: name.to_string(),
                path: rel,
                bytes: body.len() as i64,
                sha256: digest_bytes(body.as_bytes()),
            });
        }
        artifact.assets = assets;
    }
    let dest = f.root.join("dest");
    let selection = install(
        &lock,
        &InstallOptions {
            source_root: f.root.join("models"),
            dest_root: dest.clone(),
            allow_placeholders: true,
            installed_dictionary_sha256: f.dict_sha.clone(),
            ..InstallOptions::default()
        },
    )
    .expect("安裝應成功");
    assert_eq!(selection.assets.len(), 6);
    for asset in &selection.assets {
        assert!(
            dest.join(&asset.path).is_file(),
            "缺少安裝後的 asset {}",
            asset.path
        );
    }
}

#[test]
fn test_release_rejects_removed_revision_or_hash() {
    // 以合成模型包驗證 release 規則，不需要 repo 內的正式 lock 檔。
    let f = new_fixture("release");
    let mut pkg = f.lock.model_package.clone().expect("模型包");
    pkg.placeholder = false;
    pkg.quality = "trained".to_string();
    pkg.production_eligible = true;
    for artifact in &mut pkg.artifacts {
        artifact.placeholder = false;
        artifact.quality = "trained".to_string();
        artifact.production_eligible = true;
        let body = format!("synthetic-onnx:{}\n", artifact.artifact_id);
        let mut files = BTreeMap::new();
        files.insert(
            FORMAT_INT8.to_string(),
            ArtifactFile {
                path: format!("{}/model.int8.onnx", artifact.package_dir()),
                bytes: body.len() as i64,
                sha256: digest_bytes(body.as_bytes()),
                quantization: "dynamic-quint8".to_string(),
            },
        );
        files.insert(
            FORMAT_FP32.to_string(),
            ArtifactFile {
                path: format!("{}/model.onnx", artifact.package_dir()),
                bytes: 1,
                sha256: digest_bytes(b"fp32"),
                quantization: "fp32".to_string(),
            },
        );
        artifact.files = files;
        let dir = artifact.package_dir();
        let mut assets = Vec::new();
        let mut hf_assets = BTreeMap::new();
        for name in ["manifest.json", "vocab.json", "tokenizer.model"] {
            let body = format!("{name}:{}", artifact.artifact_id);
            let digest = digest_bytes(body.as_bytes());
            assets.push(ArtifactAsset {
                name: name.to_string(),
                path: format!("{dir}/{name}"),
                bytes: body.len() as i64,
                sha256: digest.clone(),
            });
            hf_assets.insert(
                name.to_string(),
                HfFile {
                    name: name.to_string(),
                    bytes: body.len() as i64,
                    sha256: digest,
                },
            );
        }
        artifact.assets = assets;
        artifact.hf = Some(HfSource {
            repository: "example/model".to_string(),
            revision: "7450e10fe8b673560044c5db08ad3006a06d7a6e".to_string(),
            license: "mit".to_string(),
            files: BTreeMap::from([
                (
                    FORMAT_INT8.to_string(),
                    HfFile {
                        name: "model.int8.onnx".to_string(),
                        bytes: body.len() as i64,
                        sha256: digest_bytes(body.as_bytes()),
                    },
                ),
                (
                    FORMAT_FP32.to_string(),
                    HfFile {
                        name: "model.onnx".to_string(),
                        bytes: 1,
                        sha256: digest_bytes(b"fp32"),
                    },
                ),
            ]),
            assets: hf_assets,
        });
        artifact.source.training_data = "corpus".to_string();
        artifact.source.generator = "test".to_string();
    }
    pkg.validate_release().expect("基準應通過");

    pkg.artifacts[0].hf.as_mut().expect("hf").revision.clear();
    let err = pkg.validate_release().expect_err("移除 revision 後應失敗");
    assert!(matches!(err, ModelSetError::Unpinned(_)), "{err:?}");

    pkg.artifacts[0].hf.as_mut().expect("hf").revision =
        "7450e10fe8b673560044c5db08ad3006a06d7a6e".to_string();
    pkg.artifacts[1].hf.as_mut().expect("hf").files.insert(
        FORMAT_INT8.to_string(),
        HfFile {
            name: "model.int8.onnx".to_string(),
            ..HfFile::default()
        },
    );
    let err = pkg.validate_release().expect_err("移除檔案雜湊後應失敗");
    assert!(matches!(err, ModelSetError::Unpinned(_)), "{err:?}");
}

#[test]
fn test_package_paths_and_unknown_shapes() {
    let artifact = Artifact {
        manifest: "reranker/manifest.json".to_string(),
        artifact_id: "reranker".to_string(),
        assets: vec![ArtifactAsset {
            name: "vocab.json".to_string(),
            path: "reranker/vocab.json".to_string(),
            ..ArtifactAsset::default()
        }],
        ..Artifact::default()
    };
    assert_eq!(artifact.package_dir(), "reranker");
    assert_eq!(
        artifact.package_root(Path::new("/models")),
        Path::new("/models/reranker")
    );
    assert_eq!(
        artifact.asset_path(Path::new("/models"), "vocab.json"),
        Some(PathBuf::from("/models/reranker/vocab.json"))
    );
    assert!(artifact.asset("missing").is_none());

    let io = super::manifest::TensorIo {
        inputs: vec![Tensor {
            name: "input_ids".to_string(),
            dtype: "int64".to_string(),
            shape: vec![json!("batch"), json!(1.0)],
            ..Tensor::default()
        }],
        outputs: Vec::new(),
    };
    assert_eq!(io.inputs.len(), 1);

    let dictionary = Dictionary {
        pack_id: "ja".to_string(),
        language: "ja".to_string(),
        format: 2,
        version: "1".to_string(),
        manifest_sha256: digest_bytes(b"x"),
        source: String::new(),
    };
    assert!(dictionary.validate().is_ok());
}
