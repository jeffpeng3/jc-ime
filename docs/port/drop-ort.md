# drop-ort：移除 ONNX Runtime

2026-11-20 起 `jc-model`、`jc-core` 與 `jc-cli` 只有 candle 一個推論後端。
candle 已覆蓋全部模型，注音選字 Q8_0 比 ORT int8 更快、更省（9.36 ms／
74.7 MiB vs 25.3 ms／84 MB），因此移除 ORT，不改決策邏輯，也不刪資料檔。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/neural/native/backend_ort.go` | `crates/jc-model/src/candle/mod.rs`、`crates/jc-model/src/candle/zh.rs` | rewritten：forward 由 candle 承接，ORT session 路徑刪除 |
| `internal/neural/backend.go`（執行期探測） | `crates/jc-model/src/manifest.rs` | rewritten：可用性只看模型檔，不再探測動態庫 |
| `internal/neural/native/neural.go`（後端選擇） | `crates/jc-model/src/rerank.rs` | rewritten：`Engine` 只剩 fp32／GGUF 兩種權重 |
| `internal/neural/residency.go`（session 與 dynamic load） | — | n/a：ORT 動態載入整個移除，沒有對應概念 |
| `core/rerank/rerank.go` | `crates/jc-core/src/zh/select.rs` | ported：只刪 ORT 分支，`choose`、融合、margin、候選順序不動 |
| `core/session/rerank.go` | `crates/jc-core/src/zh/select.rs` | ported：`rerank_nbest` 不變 |
| `internal/neural/native/input/input.go` | `crates/jc-core/src/zh/select.rs`（`prepare`） | n/a：本輪沒有改輸入組裝 |
| — | `crates/jc-cli/src/cli.rs`（`--backend`） | rewritten：只接受 `candle`，其他值回錯誤 |
| — | `scripts/legacy/*` | n/a：ORT 專用腳本移到 legacy，見該目錄 README |

## 移除清單

程式：

- `crates/jc-model/src/runtime.rs`：ORT 初始化、動態庫搜尋（`ORT_DYLIB_PATH`、
  `libonnxruntime`）、session 建立、watchdog 中止與相關 log，整檔刪除。
- `crates/jc-model/src/rerank.rs`：`Engine::Ort`、`load_with_backend`、
  `backend()`、`score_packed_with(timeout)`。
- `crates/jc-model/src/candle/mod.rs`：`Backend`、`set_default_backend`、
  `default_backend`、`JC_RERANK_BACKEND`。
- `crates/jc-model/src/manifest.rs`：載入時的 ONNX Runtime 探測；預設模型檔從
  `model.int8.onnx` 改成 `model.safetensors`。
- `crates/jc-model/src/error.rs`：`ModelError::Runtime`。
- `crates/jc-core/src/zh/select.rs`：`Engine::Ort`、`run_ort`、watchdog、
  `SelectModel::load` 的 `ort_lib` 參數、`DEFAULT_TIMEOUT_MS`。
- `crates/jc-cli/src/cli.rs`、`eval_cmd.rs`、`converter.rs`：`Backend` 欄位與
  `set_default_backend` 呼叫；`--backend` 只做值驗證。
- `crates/jc-cli/src/bin/candle-ja-check.rs`：整支只為 ORT 對照存在，刪除。
  單一權重的量測改由 `candle-quant bench` 負責。
- `crates/jc-cli/src/bin/candle-zh-check.rs`：移除 ORT 模型與 `--ort-lib`，
  改成量化對 fp32 的比較。
- `jc-model`、`jc-core` 的 `ort` 依賴與 `Cargo.lock` 條目。

腳本：

- `scripts/quant/quantize_models.py`、`measure_onnx.py`、`e2e_punct.py`、
  `dump-inputs/` 移到 `scripts/legacy/`，原因與替代路徑寫在
  `scripts/legacy/README.md`。
- `scripts/quant/` 保留 `eval_jinen_quant.py`、`latency_jinen.py`，兩者用
  llama-cpp-python 讀 GGUF，與 ORT 無關。

## 保留但不再使用

- `models/zh-select/model.onnx`：不再推論，只當
  `candle-zh-check --convert` 的轉檔來源。檔案留在資料目錄。
- `models/reranker-extra/model.int8.onnx`：不再載入。資料檔保留。
- 舊 manifest 的 `"model": "model.int8.onnx"`：只影響可用性判定，實際權重由
  candle 的 `weights_path` 找 safetensors／GGUF。manifest 屬於資料，未改。
- `legal/`：原本就沒有 ONNX Runtime 條目，無需標記；`THIRD-PARTY-NOTICES`
  提到的 `*.onnx` 是模型檔紀錄，照原樣保留。

## 驗證

```sh
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

兩者都通過。`grep` 的第三條依任務書原文會誤中 `import::`、`Report::` 等
識別字；另外 `scripts/legacy/` 依指示保留原始 ORT 字串。排除這兩類後：

```sh
grep -rnE '(^|[^A-Za-z_])(onnxruntime|ort::)' crates/ scripts/ \
  --exclude-dir=legacy        # 無結果
grep -rn -- "--backend ort" crates/ scripts/   # 無結果
```

功能與數字（GB10，`JC_ZH_SELECT_WEIGHTS=model.q8_0.gguf`）：

| 檢查 | 結果 |
|---|---|
| `jc-cli eval zh-golden` | **45/45** |
| 注音選字推論 p50 | **9.56 ms**（candle-zh 記錄 9.36 ms，同一台共用機器） |
| `--backend candle` | 45/45，p50 10.44 ms |
| `--backend ort` | `--backend 只接受 candle，實際為 ort`，exit 1 |
| 日文 reranker fp32 | `candle-quant bench` 載入與打分正常 |

## 不在 owned paths 的必要改動

以下只改函式簽名或依賴，沒有改協定與行為：

- `crates/jc-core/src/ja/rerank.rs`：`load_with_ort_lib` 移除，
  `load_with_spec` 去掉 `ort_lib` 參數。
- `crates/jc-core/Cargo.toml`：移除 `ort` 依賴。
- `crates/jc-engine/src/loader.rs`：`SelectModel::load` 改傳一個參數。
- `README.md`：crate 表與 reranker 說明改成 candle。

## 沒做完

- 沒有重跑 `eval ja --corpus modern-500 --rerank on` 的完整 500 題；只確認
  reranker 載入與打分正常。ORT 與 candle 的日文差異先前已記錄在
  `docs/port/candle-spike-ja.md`。
- candle 的 forward 無法中途停止，manifest 的 `timeout_ms` 只剩載入時驗證，
  沒有執行期中止。這是 candle-zh／candle-spike-ja 已記錄的限制。
