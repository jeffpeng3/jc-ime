# rerank-wordlevel：把 reranker 用在詞層級

`rerank-fix` 與 `rerank-tokenizer` 把 reranker 用在整句 N-best，`modern-500`
top-1 從 86.0% 掉到 41.0%。這個 task 改成訓練時的用法：對「同一讀音的多個
詞面」整批打分，分數轉成成本增量進 Viterbi。

## 結論

**詞層級的接線是對的，但這個模型在 `modern-500` 上比統計轉換差，門檻沒有
達成。**

- 接線驗證：直接對 P2-23E homophone 題庫（validation 前 200 組）打分，
  Rust 版 top-1 **83.0%**，與同一份模型在 Python 的 83.0% 一致；把轉換器
  接上重排後，同題庫 top-1 從 44.5% 升到 61.0%。喉管與單 piece 佈局都正確。
- `modern-500` 上，模型對可以判定正解的 440 個詞群只有 **48.0%**，統計詞典
  最便宜候選是 **63.6%**。模型偏好稀有漢字（`英`、`郭`、`綿`），在這批
  日常短句上不可用。
- `modern-500 --rerank on`：top-1 **82.0%**（off 86.0%）、top-5 95.2%、
  p50 78.8 ms。top-1 沒過；top-5 與 p50 過了。只重排最有歧義的 1 個詞群
  也只有 85.2%，模型在這批詞群上的方向就是錯的。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/rerank/rerank.go`（`fuse` 的權重與 draft 加分） | `crates/jc-core/src/ja/decode.rs`（`fuse_word_scores`、`word_delta`） | rewritten：改成詞群融合，公式見下 |
| `core/ja/rerank/config.go`（`Weights`、`MaxLeftContextUnits`） | `crates/jc-core/src/ja/decode.rs`、`ja/rerank.rs` | ported：`WORD_NEURAL_WEIGHT`／`WORD_STATS_WEIGHT`／`WORD_DRAFT_BONUS` |
| `core/ja/decode/hooks.go`（`Verifier.Score`、`Reranker.Rerank`） | `crates/jc-core/src/ja/decode.rs`（`JaReranker::score_words`） | rewritten：介面從整句改成詞群 |
| `core/ja/decode/viterbi.go`＋`core/ja/rerank/search.go`（限制式重搜） | `crates/jc-core/src/ja/decode.rs`（`viterbi`、`word_deltas`） | n/a：重搜不在本 task 範圍，本版用兩段式 Viterbi |
| `core/session/rerank.go`（觸發、逾時與回退） | `crates/jc-core/src/ja/decode.rs`（`RERANK_TIMEOUT` 預算、整批放棄） | rewritten：預算改成每次轉換 100 ms |
| `internal/neural/native/rerank_plan.go`（`capIDs`、candidate slot） | `crates/jc-model/src/layout.rs`（`candidate_piece`、`context_ids`、`pack_batch`） | rewritten：候選放單一 piece、補上右文 |

`crates/jc-model` 的兩處改動是接線必需：candidate slot 要放訓練時的單一
piece，右文 60 格一直沒被填。`spm.rs`／`tokenizer.rs` 沒動。

## 分組方式

lattice 建好後，對每個起點 `start` 把詞弧依「終點」分群。一組的條件：

1. 讀音至少 2 個かな單位（單かな助詞不在訓練分佈）。
2. 詞面 1 到 4 個字、全漢字（`々`、`〆` 例外）。
3. 詞面是單一 SentencePiece piece：優先裸 piece（`橋`），沒有才用
   `▁橋`。模型只有一個 candidate slot，多 piece 詞面會被丟掉。
4. 詞成本與該組最便宜候選相差不超過 2500（訓練的 `homophone_cost_window`），
   最多留 8 個候選。
5. 過濾後至少 2 個候選才送模型。
6. 一句最多送 3 個詞群（`WORD_RERANK_MAX_GROUPS`），依成本跨度由小到大
   排序，先送最模糊的。這是延遲上限，也避免模型去動統計已經很確定的詞。

未知詞 fallback 與假名候選不進群組，也不加成本，所以假名路徑的統計成本
不受影響。

## 分數公式

一組候選 `i` 的成本是 `cost_i`（詞典原始整數），模型分數是 `raw_i`
（越大越好）：

```text
stats_i  = (max_cost - cost_i) / (max_cost - min_cost)        # 全同時 0.5
fused_i  = 1.0 * raw_i + 0.2 * stats_i + if i == 0 { 0.1 } else { 0 }
delta_i  = (max_fused - fused_i) * 1000 / 1000                # DP 尺度
cost'_i  = cost_i + delta_i
```

`delta_i` 只加不減，模型第一名是 0；統計成本原樣留在詞弧上，增量只改變
Viterbi 裡的相對順序。`WORD_SCORE_SCALE = 1.0` 表示融合分數差 1 分等於
詞成本 1.0（詞典 1000 單位）。

兩段式 Viterbi：第一段算純統計最佳路徑，取它的已解碼前文與後文當左右
文；第二段把 `delta` 加進詞成本再跑一次。這樣每組的左右文都是完整句子
（近似 IME 已確定的前後文），不需要在 DP 中途猜。

## 上下文

模型位置表是 `[0, 61..183]`：BOS 在 0、左文 60 格在 61..120、分隔符 121、
候選 122、候選後 EOS 123、右文 124..183。`pack_batch` 照這個佈局填：

- 左文取靠近游標的最後 60 個 token，靠右對齊。
- 右文取緊接候選的前 60 個 token。
- `encode` 自動加的詞首 `▁` 會被丟掉；訓練 context 是連續語料，沒有這個
  假邊界，留著在 P2-23E 題庫上掉 5 個百分點。

右文是這次補上的。訓練資料兩側各 120 token，部署只有 60 格；`rerank-fix`
當時判斷「沒有 lookahead」而留空，但位置表明確留了 60 格右文。

## 逾時與回退

每次轉換 100 ms 預算（`RERANK_TIMEOUT`，不超過 manifest 的
`timeout_ms`）。每組送模型前檢查；超過就整批放棄，該次轉換用第一段的
統計結果，`timeout` 加一。單次推論失敗只影響那一組。

`eval timeout` 量的是整次 `convert_segmented` 超過題庫 100 ms 的比例，計時
範圍包含 lattice 建置、第一次 Viterbi、最多 3 組詞群準備與候選過濾、實際
送出的模型推論，以及有成本增量時的第二次 Viterbi。重排統計的 `timeout` 只記「下一組
送模型前已超過預算而整批放棄」。兩者用不同計時器。

在 `modern-500` 拆開量，關閉除錯輸出後整題 p50 約 73 到 76 ms、p95 約 138
到 144 ms：

| 階段 | p50 | p95 |
|---|---:|---:|
| lattice 建置 | 0.6 ms | 1.4 ms |
| 第一次 Viterbi | 16 ms | 35 ms |
| 詞群準備、過濾與模型推論 | 28 ms | 69 ms |
| 第二次 Viterbi（有增量時） | 18 ms | 37 ms |

500 題共挑出 3973 個詞群，截到每題最多 3 組後剩 1452 組；其中 556 組真的
送進模型，平均每題 1.1 次推論。單次推論 p50 約 20 ms、p95 約 46 到 50 ms。
一題只要碰到 2 到 3 次推論，或第二次 Viterbi 落到尾端，就會越過 100 ms。

因此 `eval timeout` 會落在 23% 到 29%（重跑受機器負載影響），重排統計的
`timeout` 只有 1 到 2 次。預算只在還有下一組時檢查，最後一組可以在 100 ms
前開始，跑完才超過，所以不記重排 timeout；`eval` 的計時器另外包含兩次
Viterbi。這個比例反映整條轉換路徑的延遲尾端；真正放棄重排的轉換是
1/500 到 2/500。

## 量測

`modern-500`，`model.int8.onnx`，8 執行緒：

| 設定 | top-1 | top-5 | p50 | p95 | eval timeout |
|---|---:|---:|---:|---:|---:|
| `--rerank off` | 86.0% | 98.0% | 7.57 ms | 22.13 ms | 0.0% |
| `--rerank on` | **82.0%** | 95.2% | 78.75 ms | 146.49 ms | 29.0% |

重排統計（on）：calls 500、groups 556、scored 556、changed 324、
timeout 2、error 0，單組推論 p50 20.32 ms／p95 49.98 ms。
`eval timeout` 是整題超過題庫 100 ms 的比例；重排自己只放棄 2 次。兩者的
差異與延遲拆解見上一節。

只開 1／2／3 個詞群的上限時 top-1 是 85.2%／84.0%／82.0%：介入越少越接近
統計，但任何介入都是淨損。

`hard-cases`：

| 設定 | top-1 | top-5 | p50 |
|---|---:|---:|---:|
| `--rerank off` | 54.3% | 97.1% | 1.11 ms |
| `--rerank on` | 54.3% | 94.3% | 45.71 ms |

## 為什麼沒過

1. **模型不適合這批句子。** `modern-500` 是 5 個模板的日常短句，句內左右
   文只有 1 到 3 個字；模型是在 120 token 專利／網路語料上訓練的。對
   440 個可判定正解的詞群，模型 48.0%、詞典最便宜候選 63.6%。
2. **可修的錯誤不在詞群裡。** 70 題錯的一半是 `子供`→`子ども`（片假名候選，
   非純漢字）、`車`→`来るまで`（不同切分）這類，同一讀音的漢字詞群救不到。
3. **延遲。** 一句平均 3.5 個合格詞群，每組 13 到 25 ms。上限 3 群讓 p50
   壓在 79 ms，但 p95 146 ms，仍有 29% 的題目超過 100 ms。

## 重現

```sh
export ORT_DYLIB_PATH=/path/to/libonnxruntime.so.1.32.0
cargo run -p jc-cli --release --features onnx -- eval ja --corpus modern-500 --rerank off
cargo run -p jc-cli --release --features onnx -- eval ja --corpus modern-500 --rerank on
cargo run -p jc-cli --release --features onnx -- eval ja --corpus hard-cases --rerank on
```

模型本身的正確率（P2-23E validation 前 200 組，60/60 上下文）用同一份
`model.int8.onnx` 量到 83.0%，與 Python 的 83.0% 相同；Rust 走的是
`NeuralReranker::score_words`。
