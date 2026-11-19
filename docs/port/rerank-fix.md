# rerank-fix：位置對齊、左文與融合

`model-wire-ja` 把 ONNX reranker 接進日文 N-best 之後 top-1 退步（86.0% →
85.6%），並留下兩個疑點：位置布局（122 vs 62）與空的
`JaContext::left`。這個 task 逐項驗證，量測全部用 `modern-500` 與
`--rerank on`。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/rerank/config.go`（`Weights`、`MaxLeftContextUnits`） | `crates/jc-core/src/ja/rerank.rs` | ported：權重改成常數 `NEURAL_WEIGHT`／`STATS_WEIGHT`／`DRAFT_BONUS` |
| `core/ja/rerank/rerank.go`（`fuse`、`request` 的左文裁切） | `crates/jc-core/src/ja/rerank.rs` | ported：神經分數用原值，不做批內正規化 |
| `internal/neural/native/rerank_plan.go`（`rerankBatch`、`capIDs`） | `crates/jc-model/src/layout.rs` | rewritten：`left` 改成 `Option<&str>`，`None` 時左文區放 BOS |
| `internal/neural/bundle.go`（`DefaultThreads`） | `crates/jc-model/src/manifest.rs`（`ModelSpec::threads`） | ported：預設 `min(8, 邏輯核心數)` |
| `internal/neural/residency.go`（`Spec.Threads` 傳到 backend） | `crates/jc-core/src/ja/rerank.rs`（`load_from_models`） | rewritten：改讀 manifest 的 `ModelSpec`，不再用預設值 |
| `core/ja/decode/hooks.go`（`ctx.Left` → `Request.LeftContext`） | `crates/jc-core/src/session.rs`（`committed`） | ported：送出的文字累積成左文，上限 128 字 |
| `internal/neural/native/rerank_ort.go` | `crates/jc-model/src/rerank.rs`、`runtime.rs` | n/a：前一個 task 已完成 |
| `core/ja/rerank/search.go`、`calibrate.go`、`stability.go` | — | n/a：不在本 task 範圍 |

## 位置布局

直接讀 shipped 的 `model.int8.onnx`：

- pooling 的 `Gather` 索引是常數 `62`（`0x3E`）。
- `position_embedding` 的索引是 `Constant[0, 61, 62, …, 120, 121, 122, 123,
  124, …, 183]` 依 `seq` 切片。
- 因此 slot 62 的候選字拿到絕對位置 **122**，與訓練 checkpoint 一致；
  左文 60 個 token 落在 61..120，分隔符 121。

`model-wire-ja` 文件寫「匯出把絕對位置寫成 0..123」是誤讀：0..123 只是
那條索引陣列的開頭。實際布局已經 right-anchored，不需要改
`crates/jc-model` 的輸入組裝。

用兩份圖改寫成不同布局各跑一次（`JC_JA_MODELS` 指向暫存模型集）：

| 布局 | slot | 位置 id | top-1 | top-5 | boundary | changed |
|---|---:|---|---:|---:|---:|---:|
| shipped（訓練布局） | 62 | 61..120／121／**122**／123 | 40.8% | 98.0% | 68.2% | 275 |
| 假設版本（`arange(0..123)`） | 62 | 0..123，候選 62 | 34.0% | 98.0% | 71.6% | 307 |
| 字面 122（`arange`＋pool 122） | 122 | 0..123，候選 122 | 48.4% | 98.0% | 75.0% | 248 |

122 比 62 好 14.4 個百分點，但三種布局都遠低於統計基準，而且 shipped
布局就是訓練布局。結論：位置對齊不是這次退步的原因，保持原布局。

## 左文

- `crates/jc-model`：`left` 改成 `Option<&str>`；`None` 或空字串時在最
  靠近分隔符的左文 slot 放 BOS，不再留一整排 PAD。
- `crates/jc-core`：`Session` 累積已送出的文字（上限 128 字，
  前身 `MaxLeftContextUnits`），`rebuild_ja` 放進 `JaContext::left`。
- `crates/jc-cli`：`eval ja --left rolling` 把前一題的第一名接到左文，
  模擬連續輸入；預設 `off`，統計基準不變。

直接呼叫 ONNX 的診斷（`modern-500` 前 200 題、只讓神經分數排序前 5 名）：

| 左文 | 神經 top-1 |
|---|---:|
| 空 | 21.0% |
| 前一題 surface | 24.0% |
| 本題參考前文（oracle） | 20.5% |
| BOS 標記 | 27.5% |

左文有幫助但幅度小；連 oracle 前文都只有 20.5%，模型本身在這批候選上
排序能力不足。

題庫一次轉換整句，`eval ja` 沒有前文可用（前身 Go 的 `lang.Context{}` 也
是空的），所以 `--left off` 不是 bug，是這個題庫的輸入條件。

## 分數融合

前身 `DefaultConfig` 的權重是 `Weights{Neural: 1, Stats: 0.2, Draft: 0.1}`，
神經分數是驗證器回的原始分數（長度正規化 logprob），不做批內正規化。
原本的 Rust 版本把神經分數正規化到 `[0,1]` 再乘 0.1，等於模型幾乎不說話。

改成前身公式後（`NEURAL_WEIGHT = 1.0`、`STATS_WEIGHT = 0.2`、
`DRAFT_BONUS = 0.1`）：

| 設定 | top-1 | top-5 | boundary | p50 | p95 |
|---|---:|---:|---:|---:|---:|
| `--rerank off` | 86.0% | 98.0% | 88.2% | 3.07 ms | 6.24 ms |
| `--rerank on`（空左文） | 40.8% | 98.0% | 68.2% | 26.14 ms | 34.53 ms |
| `--rerank on --left rolling` | 57.8% | 98.0% | 76.2% | 25.78 ms | 33.84 ms |
| 修正前（0.1 權重） | 85.6% | 98.0% | 87.8% | 77.87 ms | 88.10 ms |

`--rerank on` 的 rerank 統計（空左文）：calls 500、scored 500、changed
275、timeout 0、error 0，推論 p50 19.86 ms／p95 23.45 ms。

0.1 權重看起來「沒有退步」，是因為它讓模型幾乎不影響排序（changed 只有
2 題）；換成前身權重後，模型真正的排序能力才暴露出來。

## 延遲

p50 從 77.87 ms 降到 26 ms，來源有兩個：

1. `NeuralReranker::load_from_models` 只取目錄，載入時又用
   `ModelSpec::default()`，manifest 的 `threads` 永遠沒生效，session 固定
   單執行緒。
2. `ModelSpec::threads()` 的預設值是 1，前身是 `min(8, NumCPU)`。

修正後推論 p50 19.86 ms（8 執行緒），其餘是 watchdog 執行緒與張量搬移。
GB10 上 20 執行緒，8 條 intra-op 已經足夠。

## 結論

這個 reranker 在 `modern-500` 的整句候選上無效：位置布局、左文、融合權重
都修到與前身一致後，top-1 仍從 86.0% 掉到 40.8%（有滾動左文 57.8%）。
訓練資料的候選是單一 SentencePiece token，評測餵的是整句 surface，候選
粒度不一致是主要限制；再來是 tokenizer 只有詞表、沒有 piece 分數，只能
用最長前綴匹配近似訓練時的切法。

因此 `--rerank on` 保持 opt-in，預設關閉。要在日文轉換真的拿到增益，需要
把重排單位改成文節（單詞候選）並提供真正的左文，或重新訓練容忍整句候選
的模型；兩者都不在這個 task 的 owned paths。

## 重現

```sh
export ORT_DYLIB_PATH=/path/to/libonnxruntime.so.1.32.0
cargo run -p jc-cli --release --features onnx -- eval ja --corpus modern-500 --rerank off
cargo run -p jc-cli --release --features onnx -- eval ja --corpus modern-500 --rerank on
cargo run -p jc-cli --release --features onnx -- eval ja --corpus modern-500 --rerank on --left rolling
```
