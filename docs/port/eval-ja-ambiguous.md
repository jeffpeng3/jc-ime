# eval-ja-ambiguous：日文歧義題庫量測

## 題庫格式

`0c-testdata/eval/ja-lexicon-ambiguous.json`（schema 1，1200 題）
與 `testdata/ja/*.json` 不同：

| 欄位 | 內容 |
|---|---|
| `reading` | 一個假名讀音 |
| `reference` | 詞典偏好詞面（同讀音成本最低的漢字），不是人工標註 |
| `candidates` | 2–6 個純漢字詞面與 pack 的 signed i16 成本，已依 `(cost, surface)` 排序 |
| `holdout` | 報告用的 400 題；其餘 800 題是 dev |
| `pack` | 建置時 pin 的 pack 與 manifest SHA-256 |

題目沒有 `segments`，所以邊界指標沒有分母（`boundary_n = 0`），表格印 0.0%
是「不計分」，不是切分全錯。

## 對照表

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/eval/ja_lexicon.go`（`JALexCorpus`、`LoadJALexCorpus`、`validate`） | `crates/jc-eval/src/corpus.rs`（`load_ja_lexicon_ambiguous`、`validate_ambiguous`） | ported：既有 loader，本 task 只接線 |
| `internal/eval/ja_lexicon.go`（`RunJALex` 的 holdout／dev／all 切分） | `crates/jc-cli/src/bin/ja-eval.rs`（`LoadedCorpus::rows`、`Split`） | rewritten：沿用 `jc-eval::evaluate` 的 `Report`，不重寫指標 |
| `internal/eval/ja_lexicon.go`（`runJALexRanker`） | `crates/jc-cli/src/bin/ja-eval.rs`（`StatsConverter`） | rewritten：直接取 `JaDecoder::results` 的排序，Rust 目前沒有獨立的 `score.Ranker` 回退層 |
| `internal/eval/ja_lexicon.go`（`RunJALex` 的 dictionary 臂） | n/a | 本輪只量產品實際路徑（統計轉換）；dictionary 臂依定義恆為 1，沒有量測價值 |
| `cmd/0c-eval-single ja`（CLI 參數） | `crates/jc-cli/src/bin/ja-eval.rs`（`--file`、`--split`） | rewritten：`--file` 直接給題庫，`--corpus ja-lexicon-ambiguous` 會到 `0c-testdata/eval` 找 |
| `internal/eval/single_test.go`（`TestJALexCorpusPinned`） | `crates/jc-eval/src/corpus.rs`（`real_ambiguous_corpus_loads`） | ported（既有）：斷言 1200 題與 400 題 holdout |
| `internal/eval/single_test.go`（`TestJALexBuildDeterministic`、`TestJALexRunDeterministic`、`TestRunJALexArms`） | — | n/a：題庫重建屬 ja-pack-v2 的範圍；本輪只量測，且 Rust 沒有獨立的 ranker 臂可對照兩組排序 |

## 指令

```sh
cargo run -p jc-cli --release --bin ja-eval -- \
  --pack /mnt/work/jc/jc-ime-data/packs/ja-lexicon-mozc-v2.pack \
  --file /mnt/work/jc/jc-ime-data/0c-testdata/eval/ja-lexicon-ambiguous.json \
  --top 5
```

`--file` 會自動辨識 `testdata/ja` 與 `eval/ja-lexicon-ambiguous.json` 兩種格式；
歧義題庫預設輸出 holdout／dev／all 三個切分，`--split holdout` 可只跑一個。

## 結果（pack：`ja-lexicon-mozc-v2.pack`）

pack SHA-256 `15926f48dca7a6917f067a41f8ec49e2474c11d0cdebce5486a81f51fd0c84c6`。

| 切分 | n | top-1 | top-5 | 邊界 | p50 ms |
|---|---:|---:|---:|---|---:|
| holdout | 400 | 30.2%（121/400） | 58.5%（234/400） | n/a（無 segments） | 0.357 |
| dev | 800 | 33.1%（265/800） | 58.0%（463/800） | n/a | 0.390 |
| all | 1200 | 32.2%（386/1200） | 58.2%（698/1200） | n/a | 0.384 |

沒有 timeout，p95 在 1.2 ms 上下。

## 對照：題庫 pin 的 public-core pack

題庫候選是從 `ja-lexicon-public-core.pack`（SHA-256 `8e57a18d…`）抽的；
`ja-eval` 會偵測 pin 與實跑 pack 不同並印警告。換回同一份 pack（v1，無
context 矩陣，走 POS 成本）作為對照：

| 切分 | n | top-1 | top-5 | p50 ms |
|---|---:|---:|---:|---:|
| holdout | 400 | 52.2% | 93.8% | 0.451 |
| dev | 800 | 51.7% | 93.0% | 0.499 |
| all | 1200 | 51.9% | 93.2% | 0.479 |

這與 `baseline.json` 的 ranker top-1 0.5000／MRR 0.6916（holdout）同級。
v2 較低的原因是候選成本來自 Mozc，而 reference 是 public-core 的成本最低詞面：
holdout 漏掉的 165 題裡只有 32 題輸出純假名（詞典沒收錄），其餘是選了另一個
合法漢字（例如 `仇花` → `徒花`）。這是 pack 不同源，不是 decoder 壞掉，
因此沒有改 `jc-core`。

## 5 題樣本（v2 pack、holdout 切分）

| id | 期望（詞典偏好） | 實際 top-1 | 命中 |
|---|---|---|---|
| ja-0010 | 赤玉 | 赤玉 | top1 |
| ja-0036 | 雨具 | 雨具 | top1 |
| ja-0011 | 赤虫 | アカムシ | top5 |
| ja-0001 | 秋鹿 | あいか | 未進前五 |
| ja-0027 | 仇花 | 徒花 | 未進前五 |

後兩題是預期中的歧義失敗：`秋鹿` 在 Mozc 沒有詞條，退回假名；`徒花` 是合理
的另一個漢字選擇。top-5 內出現正確詞面的比例 58.5%，與 top-1 30.2% 的差距
反映 N-best 有抓到、排序偏好不同。

## 驗證

- `blaze check -p jc-cli --all-targets` 通過。
- `cargo check -p jc-cli --all-targets` 通過。
- `--file`、`--corpus ja-lexicon-ambiguous`、`--split holdout` 三種入口都跑完。
- 新增單元測試：參數解析（`--file`／`--split`）、切分選擇、格式辨識。
