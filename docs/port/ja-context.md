# ja-context：接上 context ID 並量測

## 對照表

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/lexicon/connection.go`（`ContextMatrix`、`Connections.ContextCost`） | `crates/jc-core/src/ja/lexicon.rs`（`JaConnectionCost`） | rewritten：不再讀 `0CJCON01`，直接查 pack v2 的 connection section；越界回 `None` |
| `core/ja/decode/viterbi.go`（`initialKey`、`connectionCost`、`finalConnectionCost`） | `crates/jc-core/src/ja/decode.rs`（`initial_key`、`connection_cost`、`final_connection_cost`） | ported：BOS／EOS 都是 context ID 0；左或右缺少 ID 時退回 POS |
| `core/ja/decode/lattice.go`（未知詞 fallback） | `crates/jc-core/src/ja/decode.rs`（`JaBuilder::build`） | ported：未知詞沒有 context ID，走 POS 成本 |
| `core/ja/decode/context_test.go`（`TestContextMatrixChangesPath`） | `crates/jc-core/src/ja/decode.rs`（`test_context_matrix_changes_path`） | ported：同一讀音、同 POS，只有 left／right ID 不同時由矩陣決定路徑 |
| `cmd/0c-eval-ja/main.go`、`internal/eval/ja*.go` | `crates/jc-cli/src/bin/ja-eval.rs` + `jc-eval` | rewritten：沿用 `jc-eval` 的 `Report`、`evaluate` 與題庫 loader；只換轉換器 |
| `internal/jadic/mozc.go`（矩陣寫入） | `crates/jc-jadic`（ja-import） | n/a：已在 ja-import 完成，本 task 只讀 |

## 實作決定

- `JaLexicon` 的 pack 改成 `Arc<Pack>`，詞典與 `JaConnectionCost` 共用同一份
  映射與解碼結果；`JaDecoder::open_with_context` 只開一次檔。
- 詞條的 left／right ID 不進 `JaEntry`，由 `JaDictionary` 以詞條編號平行的
  陣列保存，`JaWordMatch`／`JaNode` 原樣帶到 Viterbi。
- 未知詞沿用前身行為：`JaNode` 的 `left_id`／`right_id` 都是 `None`，因此
  與前後詞相接時退回 POS 成本；不是用 0 假裝成 BOS／EOS。前身的
  `lexicon.Entry` 未知詞同樣沒有 ID，這是刻意保留的 fallback。
- v1 pack 沒有矩陣：`JaConnectionCost::cost` 回 `None`，解碼器走 POS 成本，
  不 panic；`ja-eval` 會印警告。
- `JaSegmenter` 的 merge 門檻維持 0，與 `0c-eval-ja` 的 decoder 設定一致；
  `PreferHiragana` 不開啟，reference 數字就是不開啟的結果。

## 量測

```sh
cargo run -p jc-cli --release --bin ja-eval -- \
  --pack /mnt/work/jc/jc-ime-data/packs/ja-lexicon-mozc-v2.pack \
  --corpus modern-500 --top 5
cargo run -p jc-cli --release --bin ja-eval -- \
  --pack /mnt/work/jc/jc-ime-data/packs/ja-lexicon-mozc-v2.pack \
  --corpus hard-cases --top 5
```

| Corpus | n | top-1 | top-5 | boundary exact | boundary F1 | p50 ms |
|---|---:|---:|---:|---:|---:|---:|
| modern-500 | 500 | 86.0% | 98.0% | 88.2% | 0.9017 | 3.216 |
| hard-cases | 35 | 54.3% | 97.1% | 71.4% | 0.8600 | 0.747 |

對照前身（`data/ja/README.md`，Mozc＋context 矩陣）：modern-500 86.0%／
98.0%／88.2%／0.902、hard-cases 54.3%／97.1%／68.6%／0.863。top-1、top-5
與 modern-500 邊界完全相同；hard-cases 邊界差 2.8 個百分點，來自文節切分
細節，不影響 top-1／top-5。

負向對照：同一支 bin 換成 v1 的 `ja-lexicon-public-core.pack`，modern-500
top-1 是 0.0%，證明有矩陣與沒矩陣走的是不同路徑，不是靜默退回 POS。
