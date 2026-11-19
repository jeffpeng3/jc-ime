# j3-ja-decode：移植對照

日文轉換核心移植到 `crates/jc-core/src/ja/decode.rs`。行為以 Go
`core/ja/decode` 為準：完整 Viterbi、N-best、詞邊界、書寫系統偏好、
學習／驗證／重排鉤子與 context ID 降級規則都照搬。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/decode/doc.go` | `crates/jc-core/src/ja/decode.rs` module 文件 | ported |
| `core/ja/decode/convert.go` | 同上：`JaDecoder::{convert, nbest, results, candidates, prefixes}` | ported |
| `core/ja/decode/viterbi.go` | 同上：`JaDecoder::search`、`StateKey`、N-best 保留策略 | ported |
| `core/ja/decode/lattice.go` | 同上：`JaBuilder`、`JaNode`、`Lattice` | ported |
| `core/ja/decode/segment.go` | 同上：`JaSegmenter`、`clauses`、`JaClause` | ported |
| `core/ja/decode/script.go` | 同上：`apply_script_policy` 與かな／拉丁判定 | ported |
| `core/ja/decode/hooks.go` | 同上：`JaLearning`、`JaVerifier`、`JaReranker` 與空實作 | ported |
| `core/ja/decode/decode_test.go` | 同上 `mod tests` | ported（13 個單元測試＋2 個真實 pack 測試） |
| `core/ja/decode/script_test.go` | 同上 `mod tests` | ported（2 個測試） |
| `core/ja/decode/lm_test.go` | 同上 `mod tests` | ported（3 個測試） |
| `core/ja/decode/context_test.go` | 同上 `mod tests` | ported（1 個測試） |
| `core/ja/decode/testdata/lexicon.tsv` | `mod tests` 的 `corpus_entries()` | ported（成本 ×1000 轉成 pack 的整數尺度） |
| `core/ja/decode/testdata/anthy-corpus-sample.txt` | `mod tests` 的 `ANTHY_SAMPLE` | ported |

## 刻意差異

| 項目 | 說明 |
|---|---|
| 詞成本尺度 | Go 用 `float64`（Mozc 成本除以 1000）；Rust 的 `JaEntry` 是 `i32` 原始整數。DP 內部除以 1000 還原前身尺度，LM 成本與鉤子分數不需要縮放。 |
| 記憶體詞典 | Go 的 `lexicon.Mem`／`NewMem` 沒有搬進 `ja/lexicon.rs`；改由公開的 `JaWordSource` trait 注入，測試用線性掃描實作。正式路徑是 `JaDictionary`，以 `JaLexicon` 建讀音索引。 |
| context ID | pack v1 沒有 left/right ID，`JaDictionary` 一律回報 `None`，實際走詞性表；`JaContextCost` 與 `JaNode::{left_id,right_id}` 保留完整介面，測試以記憶體詞條驗證 context 矩陣確實會改變路徑。 |
| 候選型別 | `jc-core` 的 `Candidate` 沒有 `Clauses`／`Annotation` 欄位，改用 `JaCandidate`（surface、reading、cost、source）。 |
| `lang.Context` | 由 `JaContext { left, right }` 取代。 |
| 錯誤型別 | Go 的 `Convert` 只有空輸入會回空結果，Rust 對應 `Option`／空 `Vec`，沒有新增錯誤變數。 |
| 輸出邊界 | `Conversion.segments` 是文節在 `surface` 上的 UTF-8 位元組範圍；`JaResult::clauses` 保留文節與詞的對應。 |

## 測試資料

| 測試 | 資料 | 缺資料時 |
|---|---|---|
| `test_anthy_corpus_samples` 等 13 個單元測試 | 內建測試詞典 | 不需要外部資料 |
| `test_real_pack_converts_common_sentences` | `packs/ja-lexicon-public-core.pack`、`ja-src/connection.tsv` | `require_data!` 跳過 |
| `test_real_pack_modern_500_first_20` | 同上＋`0c-testdata/ja/modern_500.json` | `require_data!` 跳過 |

`modern-500` 的完整 top-1／邊界指標由 `jc-eval` 負責；這裡只驗前 20 題的
top1 非空、長度合理、`segments` 連續覆蓋整個字面。
