# onnx-default：移除 onnx feature，改成執行期偵測

ONNX 不再是編譯期開關。`ort` 是一般依賴，使用 `load-dynamic`；有沒有可用的
ONNX Runtime 由執行期探測決定，沒有就全部走統計路徑。

## 決定

同一個 commit 只會有一種行為，CI 與本機一致。模型或 ONNX Runtime 是否可用
是部署環境的事，不該用 `--features onnx` 決定。

- `jc-model`、`jc-core`、`jc-cli` 的 `[features] onnx` 全部移除。
- 所有 `#[cfg(feature = "onnx")]` 改成一般程式碼。
- `--rerank`、`--select-model`、`--punct` 仍是執行期參數，預設不變（off）。

## 載入方式

`ort` 依賴：

```toml
ort = { version = "=2.0.0-rc.15", default-features = false, features = [
    "std",
    "load-dynamic",
    "api-31",
] }
```

`load-dynamic` 讓 `cargo build` 不下載、不連結 ONNX Runtime。執行期依序找：

1. `--ort-lib`（只有 `probe` 工具有這個參數）
2. `ORT_DYLIB_PATH`
3. 系統動態庫：`libonnxruntime.so`、`libonnxruntime.so.1`（Linux）、
   `libonnxruntime.dylib`（macOS）、`onnxruntime.dll`（Windows）

## 執行期偵測

`jc_model::ModelRuntime::probe()` 嘗試載入動態庫，結果快取：

- 成功 → `RuntimeStatus::Available`
- 失敗 → `RuntimeStatus::Unavailable { reason }`，原因只記錄一次（`log::info`）

`ModelSet::load()` 把模型檔解析與執行期能力分開：檔案齊全但 ONNX Runtime
不可用時，角色是 `Availability::Unavailable`，路徑仍會解析出來。既有的
「模型檔不存在 → 不可用」行為不變。

呼叫端一律先看可用性：

| 入口 | 沒有 ONNX Runtime 時 |
|---|---|
| `--rerank on` | 警告一次，保留統計排序 |
| `--select-model on` | 警告一次，保留統計排序 |
| `--punct on` | 警告一次，輸出原文 |

不 panic，也不把錯誤當成命令失敗。`probe` 工具會直接印出不可用的原因。

## ORT 取得與放置

| 平台 | 做法 |
|---|---|
| Linux | 從 [ONNX Runtime releases](https://github.com/microsoft/onnxruntime/releases) 下載 `onnxruntime-linux-x64-*.tgz`，解開後把 `lib/libonnxruntime.so*` 放到系統 library path，或設 `ORT_DYLIB_PATH=/path/libonnxruntime.so` |
| macOS | 下載 `onnxruntime-osx-*.tgz`，`lib/libonnxruntime.dylib` 放同一個搜尋路徑或設 `ORT_DYLIB_PATH` |
| Windows | 下載 `onnxruntime-win-x64-*.zip`，把 `onnxruntime.dll` 放在 `jc-cli.exe` 同目錄（或 PATH 上的目錄），也可以設 `ORT_DYLIB_PATH` 指到完整路徑 |

Python 套件內的 `onnxruntime/capi/libonnxruntime.so*` 也可以直接拿來設
`ORT_DYLIB_PATH`，不需要另外安裝系統套件。

CI 不安裝 ORT 也能過：`cargo check`、`cargo clippy`、`cargo test` 都不需要
動態庫；需要推論的 smoke test 才設 `ORT_DYLIB_PATH`。

## 驗證

```sh
env -u ORT_DYLIB_PATH cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check

# 有 ORT 時
ORT_DYLIB_PATH=/path/libonnxruntime.so cargo run -p jc-model --bin probe -- \
  --models /mnt/work/jc/jc-ime-data/models
```

## Port 對照表

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/neural/native/backend_ort.go` | `crates/jc-model/src/runtime.rs`、`crates/jc-model/src/rerank.rs` | rewritten：build tag 改成執行期 `probe()` |
| `internal/neural/native/backend_stub.go` | `crates/jc-model/src/manifest.rs`、`crates/jc-model/src/runtime.rs` | rewritten：stub 改成 `Availability::Unavailable` 與統計路徑 |
| `internal/neural/native/punct_ort.go` | `crates/jc-model/src/punctuation_model.rs`、`crates/jc-model/src/punct_zh.rs` | rewritten：沒有 ORT 時回傳原文 |
| `core/ja/rerank`（權重與載入） | `crates/jc-core/src/ja/rerank.rs` | rewritten：載入失敗保留統計順序 |
| `internal/neural/native/input`（注音選字輸入） | `crates/jc-core/src/zh/select.rs` | rewritten：`--select-model` 失敗時警告並回退 |
| `tools/onnxprobe/main.go` | `crates/jc-model/src/bin/probe.rs` | ported |
