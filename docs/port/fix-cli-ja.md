# fix-cli-ja：CLI 與 ja-eval 的日文分數對齊

## 根因

`jc-cli eval ja` 的 23.2% 來自兩個疊加問題：

1. **預設 pack 路徑錯**：`data::load_ja_decoder` 先看到 `ja-src/lexicon/words.tsv`
   就直接用 curated 詞庫，完全沒讀 `packs/`；退回路徑又把 v1
   `packs/ja-lexicon-public-core.pack` 排在 v2 前面。
2. **沒有傳 connection**：走 pack 時用 `JaDecoder::open`，只接
   `ja-src/connection.tsv` 的單欄詞性成本，沒有
   `JaLexicon::connection_cost()` 的 Mozc context 矩陣。`ja-eval` 則是
   `with_context_cost`，所以兩條路徑分數不同。

23.2% 是 curated 詞庫加詞性成本的水準；接上 v2 pack 的 2,672×2,672 矩陣後
才是 86.0%。

## 修正

| 項目 | 修正 |
|---|---|
| `eval ja` 參數 | 新增 `--pack <path>`，優先於 `JC_JA_PACK` 與資料根目錄的預設值 |
| 預設 pack | `resolve_ja_pack` 依 `--pack` → `JC_JA_PACK` → `packs/ja-lexicon-mozc-v2.pack` → `packs/ja-lexicon-public-core.pack` 解析，不寫死絕對路徑 |
| 連線成本 | 有矩陣的 v2 pack 用 `with_context_cost`；v1 才退回詞性成本 |
| 題庫來源 | `eval ja --corpus` 也接受 `.json` 路徑，與 `ja-eval` 一致 |
| 測試 | `tests/eval_ja_parity.rs` 用 `hard-cases` 前 5 題同時跑兩個 bin，比對 n／top1／top5 |

## 對照表

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `cmd/0c-eval-ja`（pack 與連線成本載入） | `crates/jc-cli/src/data.rs`：`resolve_ja_pack`、`load_ja_decoder_with_pack` | rewritten：v2 context 矩陣優先，預設路徑由 `JC_TESTDATA`／`--pack` 決定 |
| `cmd/0c-eval-ja`（題庫選擇） | `crates/jc-cli/src/eval_cmd.rs`：`ja` | rewritten：題庫名稱或 `.json` 路徑皆可 |
| `cmd/0c-cli` 參數解析 | `crates/jc-cli/src/cli.rs`：`EvalJa` | rewritten：新增 `--pack` |
| `cmd/0c-eval-ja` 的評測輸出 | `crates/jc-cli/tests/eval_ja_parity.rs` | rewritten：兩入口同 pack／同題庫的指標對照 |

## 實跑結果

```text
$ ja-eval --pack .../ja-lexicon-mozc-v2.pack --corpus modern-500 --top 5
modern-500  500  86.0%  98.0%  88.2%  0.9017

$ jc-cli eval ja --corpus modern-500 --pack .../ja-lexicon-mozc-v2.pack
modern-500  500  86.0%  98.0%  88.2%  0.9017
```

`hard-cases` 前 5 題：兩邊都是 `n=5`、`top1=0.0%`、`top5=100.0%`、
`b_exact=100.0%`。

## 驗證

- `cargo check -p jc-cli --all-targets` 通過。
- 兩條 `cargo run --release` 路徑 top-1 都是 86.0%。
