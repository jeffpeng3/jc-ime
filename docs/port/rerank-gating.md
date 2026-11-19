# rerank-gating：只重排統計不確定的詞群

全量詞層級重排在 `modern-500` 把 top-1 從 86.0% 壓到 82.0%，但在
P2-23E homophone 題庫上把 44.5% 拉到 61.0%。模型本身有效，錯的是對統計
已經很確定的詞群也送進去。這個 task 加一層統計門檻，只讓「前兩名拉不開」
的詞群進模型。

## 判準

lattice 建好、第一次 Viterbi 完成後，對每個起點、每個讀音終點收集
「同讀音多詞面」的候選群（規則沿用 `rerank-wordlevel`：1 到 4 字全漢字、
讀音至少 2 個單位、成本視窗 2500、最多 8 個候選）。

群組內每個候選的排序成本是

```text
total_i = word_cost_i + connection(prev_state, candidate_i)
gap     = total_2 - total_1
```

`prev_state` 是統計第一名路徑在該起點的左側狀態（詞性與 right context
ID），所以 `gap` 反映的是這句裡實際的競爭關係，不是詞典裡的孤立成本。
候選依 `total` 由低到高排序，第一筆就是統計第一名；`gap` 越小越不確定。

門檻 `--gate`：

| 值 | 意義 |
|---|---|
| `off` | 不設門檻，所有合格詞群都送（對照組） |
| `N` | `gap < N` 才送，單位是 Mozc 原始整數 |
| `cand:N` | 候選數至少 `N` 個才送 |

未指定時用 `DEFAULT_WORD_RERANK_GATE`。通過門檻的詞群依 `gap` 由小到大
排序，最多送 `WORD_RERANK_MAX_GROUPS`（3）組，其餘留給統計。門檻只看
統計成本與候選結構，執行期不讀題庫 label。

統計新增三個欄位：`groups_total`（門檻前合格詞群）、`groups`（實際送進
模型）、`skipped`（被門檻擋下）。`changed` 與 `timeout` 語意不變。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/rerank/search.go`（候選群組） | `crates/jc-core/src/ja/decode.rs`（`word_groups`、`WordGroup`） | rewritten：加上連接成本與 `gap` |
| `core/session/rerank.go`（觸發條件） | `crates/jc-core/src/ja/decode.rs`（`WordGate`、`word_deltas`） | rewritten：原本無條件觸發，改成統計門檻 |
| `core/ja/rerank/config.go`（權重） | `crates/jc-core/src/ja/rerank.rs`（`RerankStats`） | ported：統計加上 `groups_total`、`skipped` |
| 無對應實作 | `crates/jc-cli/src/cli.rs`（`--gate`） | n/a：量測參數 |

## 量測

`model.int8.onnx`，8 執行緒，`--rerank on`。top-1／top-5 是可重現值；
p50 受機器負載影響，取多次執行的較低值（同一批量測的統計路徑 p50 是
3.1 到 4.5 ms）。

### modern-500

| 門檻 | top-1 | top-5 | p50 | 送模型詞群 |
|---|---:|---:|---:|---:|
| `--rerank off` | 86.0% | 98.0% | 3.1 ms | 0 |
| `off`（全量） | 82.2% | 94.0% | 31.3 ms | 848 / 4289 |
| `50` | 86.0% | 98.0% | 6.3 ms | 190 / 4289 |
| `100` | **86.0%** | **98.0%** | **7.3 ms** | 258 / 4289 |
| `200` | 86.0% | 97.0% | 8.2 ms | 296 / 4289 |
| `300` | 86.0% | 97.0% | 35.3 ms | 491 / 4289 |
| `350` | 86.0% | 97.0% | 43.0 ms | 527 / 4289 |
| `500` | 84.0% | 95.0% | 46.0 ms | 614 / 4289 |
| `cand:3` | 81.4% | 93.6% | 62.0 ms | 899 / 4289 |

### hard-cases

| 門檻 | top-1 | top-5 | p50 |
|---|---:|---:|---:|
| `--rerank off` | 54.3% | 97.1% | 0.7 ms |
| `off` | 54.3% | 85.7% | 36.4 ms |
| `50` | 54.3% | 97.1% | 1.2 ms |
| `100` | **54.3%** | **97.1%** | **1.5 ms** |
| `200` | 54.3% | 97.1% | 1.5 ms |
| `350` | 57.1% | 94.3% | 21.0 ms |
| `500` | 57.1% | 94.3% | 32.9 ms |

### ajimee-200

| 門檻 | top-1 | top-5 | p50 |
|---|---:|---:|---:|
| `--rerank off` | 46.0% | 65.0% | 6.2 ms |
| `off` | 45.5% | 62.5% | 79.4 ms |
| `50` | 46.5% | 65.5% | 26.5 ms |
| `100` | **46.5%** | 64.5% | 73.3 ms |
| `200` | 46.0% | 64.5% | 66.1 ms |
| `500` | 45.5% | 62.5% | 81.7 ms |

`ajimee-200` 是長句，就算只送 6% 的詞群，p50 也壓不下來：統計路徑本身
就要 6.2 ms，題庫的 p95 已到 51.7 ms。

## 結論

預設選 `Gap(100)`：

- `modern-500` top-1 86.0%、top-5 98.0%，與統計完全相同，p50 7.3 ms，
  低於 20 ms 的門檻。
- 全量重排送到 4289 個詞群中的 848 個，門檻 100 只送 258 個（6%），
  延遲從 31 ms 降到 7 ms。
- `ajimee-200` top-1 從 46.0% 升到 46.5%。

**`hard-cases` 的 `> 54.3%` 沒達成**：門檻 100 時是 54.3%，與統計相同。
唯一會提升的是 350 以上的門檻（57.1%），代價是 `modern-500` 的 p50 跳到
21 到 43 ms，而 500 更讓 top-1 掉回 84.0%。`hard-cases` 只有 35 題，
57.1% 與 54.3% 的差距是 1 題；要同時守住 `modern-500` 與 p50，只能放棄
這 1 題。

其他候選結構也試過：

- 只重排統計最佳路徑實際選過的詞群：全量重排的 `modern-500` 掉到
  74.6%，比全量更糟，淘汰。
- 只重排「兩候選」且成本差在門檻內的詞群：`modern-500` 幾乎沒有詞群
  符合（350 時只有 11 組），`hard-cases` 一組都沒有，等於關閉重排，
  淘汰。

## 重現

```sh
export ORT_DYLIB_PATH=/path/to/libonnxruntime.so.1.32.0
cargo run -p jc-cli --release -- eval ja --corpus modern-500 --rerank on --gate 100
cargo run -p jc-cli --release -- eval ja --corpus hard-cases --rerank on --gate 100
cargo run -p jc-cli --release -- eval ja --corpus ajimee-200 --rerank on --gate 100
```

`--gate off` 重現全量重排；`--gate 350`、`--gate 500` 重現上表的取捨。
