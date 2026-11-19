# w1-eval 移植對照

Worktree：`/mnt/work/jc/wt/w1-eval`
參考實作：`/mnt/work/jc/ref/0c-ime`（main `8f443af`）

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/eval/ja.go` | `crates/jc-eval/src/ja.rs`、`src/metrics.rs`、`src/report.rs` | ported：題庫／rules／baseline loader、top1／top5／boundary、latency、`JARegression` → `RuleSet::gate`。`clauseBoundaries` 改由 `Converter::convert_segmented` 提供邊界。 |
| `internal/eval/ja_test.go` | `src/ja.rs` 測試 | ported：count pin、segments 串接、reading 空白、determinism、gate、real corpus。fixture 雜湊比對保留在 `check_ja_corpus_provenance`，但 T03 fixture 不在 `jc-ime-data`，以 `Option<&Path>` 選用。 |
| `internal/eval/zh_gold.go` | `src/corpus.rs` | rewritten：`LoadZhGoldenCorpus` 與 validate 搬過來。`BuildZhGoldenCorpus` 讀 `expected/*.jsonl`，不在本任務範圍。 |
| `internal/eval/ja_lexicon.go` | `src/corpus.rs` | rewritten：`LoadJALexCorpus` 與 validate 搬過來。pack 取樣與排序評測不在本任務範圍。 |
| `internal/eval/fuzzy_corpus.go` | `src/corpus.rs` | rewritten：`LoadFuzzyCorpus` 與 validate 搬過來。產生器不在本任務範圍。 |
| `internal/eval/eval.go` | `src/metrics.rs` | rewritten：`Chunks`、`EditDistance`、`Percentile` 搬成 `chunks`、`edit_distance`、`percentile`。`Reader`／`Typist`／`TabStats` 依賴注音引擎，n/a。 |
| `internal/eval/rankmetrics.go` | `src/metrics.rs` | rewritten：top1／top5／boundary 的聚合與比率。 |
| `internal/golden/golden.go` | `src/golden.rs` | rewritten：`golden/zh` 與 `golden/mixed` 的 loader、名稱／動作／span 驗證。引擎 runner、expected 比對依賴 `core/engine`，n/a。 |
| `internal/golden/golden_test.go` | `src/golden.rs` 測試 | rewritten：`TestCoverage` → `real_golden_zh_has_45_cases`；loader 錯誤訊息測試。`TestBaseline`、`TestDeterministic` 需真引擎，n/a。 |
| `internal/jaeval/report.go` | `src/metrics.rs`、`src/report.rs` | rewritten：top1 聚合與表格輸出。`avg_candidates`、`longest` 綁舊 mixed corpus 形狀，未搬。 |
| `internal/jaeval/jaeval_test.go` | `src/metrics.rs` 測試 | rewritten：`TestReportAggregation` 的 top1 部分。snapshot 比較、mixed 切段 n/a。 |
| `internal/jaeval/corpus.go` | n/a | 這是 mixed zh/ja/en 輸入 corpus，不是本任務列的 `testdata/ja` 題庫；格式不同。 |
| `internal/jaeval/engine.go` | n/a | 轉換流程由 `Converter` 外部注入。 |
| `internal/jaeval/snapshot.go` | n/a | 逐題 snapshot 比對不在 w1-eval 範圍。 |
| `internal/eval/fuzzy_run.go` | n/a | 模糊輸入轉換流程不在本任務範圍。 |
| `internal/eval/single_baseline.go`、`single_test.go`、`zh_gold.go` builder | n/a | 注音 single baseline 與題庫重建需要真引擎。 |
| `internal/eval/mixed*.go` | n/a | mixed 評測流程不在本任務範圍；`golden/mixed` 只搬 loader。 |
| `cmd/0c-eval-*` | n/a | CLI 由後續 `jc-cli` 接線。 |

## 已知差異

- 題庫 SHA-256 由 crate 內最小的 `src/sha256.rs` 計算，避免額外依賴。
- `Report.measured_on` 由呼叫端填寫；crate 不引入時間函式庫。
- `ja/baseline.json` 只在測試中確認可載入與題數，不把分數寫成期望值。
