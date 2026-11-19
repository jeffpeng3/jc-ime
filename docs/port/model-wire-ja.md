# model-wire-ja：日文 reranker 接進轉換

把 `jc-model` 的 ONNX reranker 接到 `jc-core` 的日文 N-best 重排。依賴是
optional feature `onnx`，預設關閉；feature off 時 `jc-core`、`jc-cli` 的
建置圖與公開 API 都沒有 `ort`。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/decode/hooks.go` | `crates/jc-core/src/ja/decode.rs` | ported：沿用既有 `JaReranker`，加上 `with_rerank_enabled` 執行期開關 |
| `core/ja/rerank/rerank.go` | `crates/jc-core/src/ja/rerank.rs` | rewritten：只保留 N=5 單輪重排，不含 research 與多輪 verify |
| `core/ja/rerank/config.go` | `crates/jc-core/src/ja/rerank.rs` | rewritten：權重改成常數，不開放執行期 Config |
| `internal/neural/ja_wiring.go` | `crates/jc-core/src/ja/rerank.rs` | rewritten：`NeuralReranker` 直接實作 `JaReranker` |
| `internal/neural/native/rerank_ort.go` | `crates/jc-model/src/rerank.rs` | n/a：已在 jc-model 完成，本 task 只呼叫 |
| `core/ja/rerank/search.go` | — | n/a：限制式重搜不在本 task 範圍 |
| `core/ja/rerank/calibrate.go` | — | n/a：沒有 T33 樣本，不做網格校準 |
| `core/ja/rerank/stability.go` | — | n/a：穩定度監測不在本 task 範圍 |
| `cmd/0c-server/main.go`（日文 rerank 設定） | `crates/jc-cli/src/cli.rs`、`eval_cmd.rs` | rewritten：改成 `eval ja --rerank on\|off` |
| `core/ja/live/live.go`（`SetEnabled`） | `crates/jc-core/src/session.rs`、`ja/decode.rs` | rewritten：`SessionConfig::ja_rerank` 與 `JaDecoder::with_rerank_enabled` |

## 接線

- `jc-core` 的 `jc-model` 依賴是 optional，feature `onnx`：
  `onnx = ["dep:jc-model", "jc-model/onnx"]`。
- `jc-cli` 的 `onnx` feature 只轉發 `jc-core/onnx`。
- 預設 `--rerank off`。`--rerank on` 由模型集合的 `manifest.json` 找
  `ja-rerank`（`JC_JA_MODELS` 或資料根目錄的 `models/`），載入失敗就中止，
  不會靜默退回。
- 執行時需要 `ORT_DYLIB_PATH` 指向 ONNX Runtime 動態庫。
- `JaDecoder` 的 hook trait（`JaWordSource`、`ConnectionCost`、`JaContextCost`、
  `JaLearning`、`JaVerifier`、`JaReranker`）都要求 `Send + Sync`，因此
  `Arc<JaDecoder>` 可以跨執行緒共用，clippy 的 `arc_with_non_send_sync`
  不再觸發。

## 切法

模型佈局沿用前身與 jc-model：

```text
<bos> [left 右對齊，最多 60 token] <eos> candidate <eos> [pad]
seq 124，candidate slot 62，單側 context 60 token
```

左文從 `JaContext::left` 取靠近游標的尾端：逐字往前擴，直到 tokenizer
編碼超過 60 個 token。模型端會再做右對齊與裁切。

右文（`JaContext::right`）保留在介面上，不進張量：目前 ONNX 匯出只有
`input_ids`／`attention_mask`，jc-model 的 `pack_batch` 也沒有右文參數。
這與前身部署一致，IME 沒有 lookahead。

## 公式

```text
stats_i  = (stat_score_i - min_stat) / (max_stat - min_stat)
neural_i = (raw_score_i  - min_raw)  / (max_raw  - min_raw)
final_i  = stats_i + 0.1 * neural_i + (i == 0 ? 0.05 : 0)
```

- 統計分數是主要訊號；神經分數只在統計接近時拆解平手。
- 統計第一名有 0.05 的保護分。
- `span == 0` 時正規化給 0.5。
- 同分保留原本順序，輸出可重現。
- 推論預算 100 ms；逾時、模型缺席或分數數量不符都原順序返回。
- 累計 `calls`、`scored`、`changed`、`timeout`、`error` 與推論延遲
  p50／p95，`eval ja --rerank on` 會印在表格後面。

## 量測

環境：GB10，`ORT_DYLIB_PATH` 指向 onnxruntime 1.32.0。

```sh
cargo run -p jc-cli --release --features onnx -- eval ja --corpus modern-500 --rerank off
cargo run -p jc-cli --release --features onnx -- eval ja --corpus modern-500 --rerank on
```

| 設定 | n | top-1 | top-5 | boundary exact | p50 | p95 | timeout |
|---|---:|---:|---:|---:|---:|---:|---:|
| `--rerank off` | 500 | 86.0% | 98.0% | 88.2% | 3.26 ms | 6.75 ms | 0.0% |
| `--rerank on` | 500 | 85.6% | 98.0% | 87.8% | 77.87 ms | 88.10 ms | 0.0% |

`--rerank on` 的 rerank 統計：

```text
calls 500，scored 500，changed 2，timeout 0，error 0
inference p50 74.73 ms／p95 77.47 ms
```

reranker 確實被呼叫：500 題全部取得分數，2 題第一名被換掉。兩題都是換錯，
所以 top-1 少 0.4 個百分點。

## 為什麼沒有提升

1. 題庫一次轉換整句，`JaContext::left` 是空的；模型只看到候選本身，沒有
   前文可消歧。
2. 直接拿原始分數排 5 個候選時，top-1 只有 **18.8%**（top-5 98.0%），
   模型在無前文時常把漢字候選換成かな候選。因此不能讓神經分數主導。
3. 匯出的 ONNX 使用絕對位置 0..123，candidate 在 slot 62；訓練 checkpoint
   的 candidate 絕對位置是 122。出貨佈局本來就會讓候選離開訓練位置，
   這是既有 jc-model／匯出端限制，不在本 task 的 owned paths。

因此 `--rerank on` 保持 opt-in，預設關閉；等 jc-model 的匯出位置修正或
session 真的提供左文後，再重新量測。
