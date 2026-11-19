# ja-live-rewrite 對照表

日文逐字変換（`core/ja/live`）與轉換後改寫（`core/ja/rewrite`）的完整移植。
Go 來源：`/mnt/work/jc/ref/0c-ime`（main `8f443af`）。

`j1-ja-input` 已搬 `rewrite` 的輸入正規化部分，本任務只補上其餘檔案，並把
`live` 整包收進來。界線如下：

- **j1 已搬**：`kana.go`／`kana_test.go` 的かな與全半角轉換。
- **本任務新搬**：`live` 全部；`rewrite` 的 `rewriter.go`、`number.go`、
  `symbols.go` 與對應測試；`session.rs` 的逐字変換觸發。

## Go 檔案對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/live/live.go` | `crates/jc-core/src/ja/live.rs` | ported：`Config`／`DefaultConfig`／`Plan`／`Stats`／`Controller` 全部搬入；`lang.Converter` 改成 `live::Converter`，`JaDecoder` 直接實作；`reading.Sequence` 用 `crate::reading::Sequence`；零值 `lang.Result` 改成 `Option<JaResult>`；時鐘注入改成 `Rc<dyn Fn() -> Instant>` |
| `core/ja/live/live_test.go` | `crates/jc-core/src/ja/live.rs` | ported：`test_disabled_by_default`、`test_debounce_and_tick`、`test_debounce_restarts_and_cancel`、`test_previous_result_is_draft_while_pending`、`test_locked_prefix_stable_across_twenty_kana`、`test_conversion_uses_locked_prefix_as_context`、`test_backspace_drops_locked_prefix`、`test_budget_falls_back_to_draft`、`test_boundary_changes_are_capped`、`test_conversion_error_keeps_draft`、`test_candidates_and_invalid_span`、`test_set_enabled_resets_state`、`test_zero_config_is_disabled`。假轉換器改用 `Rc<RefCell<_>>` 記錄呼叫，時鐘改用 `Cell<Instant>` |
| `core/ja/rewrite/kana.go` | `crates/jc-core/src/ja/rewrite.rs` | ported（j1 已搬，未改行為） |
| `core/ja/rewrite/kana_test.go` | `crates/jc-core/src/ja/rewrite.rs` | ported（j1 已搬）：`test_to_hiragana_table`、`test_to_katakana_table`、`test_to_halfwidth_kana_table`、`test_latin_width_table` |
| `core/ja/rewrite/rewriter.go` | `crates/jc-core/src/ja/rewrite.rs` | ported：`SourceRewrite`／`DefaultMaxCandidates`／註解常數、`Rewriter`、`Candidates`、`Append`。核心 `Candidate` 沒有 `annotation` 欄位，改用 `RewriteCandidate`；`Append` 的浮點 `Score`（越大越好）改成 `cost`（越小越好），補助候選成本接在主要候選最高成本之後 |
| `core/ja/rewrite/rewriter_test.go` | `crates/jc-core/src/ja/rewrite.rs` | ported：`test_append_keeps_primary_first`、`test_append_without_primary`、`test_candidates_order_and_source`、`test_candidates_reads_reading_and_copies`、`test_max_candidates`、`test_nil_symbols`。分數比較改成成本比較；讀音是自有 `String`，複製測試只斷言內容 |
| `core/ja/rewrite/number.go` | `crates/jc-core/src/ja/rewrite.rs` | ported：`numberCandidates`、`parseArabic`、`kanjiNumber`、`formatGroup` 與全部註解常數 |
| `core/ja/rewrite/number_test.go` | `crates/jc-core/src/ja/rewrite.rs` | ported：`test_number_candidates_table`、`test_number_candidates_order`、`test_mozc_comparison_cases` |
| `core/ja/rewrite/symbols.go` | `crates/jc-core/src/ja/rewrite.rs` | ported：`Symbol`、`Symbols`、`NewSymbols`／`Add`／`Len`／`Lookup`、`normalizeReading`。`io.Reader` 對應 `BufRead`；`fs.FS` 的 `LoadSymbolsFile`／`LoadSymbolsFS` 改成 `std::fs` 的 `load_symbols_file`／`load_symbols_dir`，檔名排序與錯誤訊息保留 |
| `core/ja/rewrite/symbols_test.go` | `crates/jc-core/src/ja/rewrite.rs` | ported：`test_load_symbols_fs`、`test_load_symbols_table`、`test_symbol_candidates`、`test_candidates_skipped_when_unchanged`。資料改用 `jc_testdata::require_data!("ja-src/symbols")`，缺資料就 skip |
| `core/ja/rewrite/doc.go` | `crates/jc-core/src/ja/rewrite.rs` | rewritten：套件說明併入 module doc，並補上輸出端改寫與 `append` 的用法 |
| `core/ja/live/MOZC.md` | — | n/a：Mozc 行為紀錄，案例已寫進 `test_mozc_comparison_cases` |
| — | `crates/jc-core/src/session.rs` | 新增：`SessionConfig::ja_live`、`Session::tick_live`／`ja_live_pending`／`ja_live_deadline`，以及 `rebuild_ja` 的逐字変換路徑。預設 `Config::default()` 關閉，關閉時程式碼路徑與原本完全相同 |
| — | `crates/jc-core/src/session.rs` | 新增測試：`test_ja_live_disabled_by_default_keeps_plain_conversion`、`test_ja_live_enabled_converts_without_enter` |

## 契約差異

- `live::Converter` 的錯誤統一成 `ConvertError`，候選範圍錯誤是
  `LiveError::Span`；前身的任意 `error` 不再需要。
- `Plan::result` 是 `Option<JaResult>`；沒有結果時為 `None`，不再回傳零值
  轉換。`Plan` 另外保留 `boundary_changes` 與 `timed_out` 供外殼診斷。
- `Controller::draft` 回傳 `Option<&JaResult>`，`Stats` 欄位改為
  `conversions`／`holds`／`timeouts`／`last_duration`／
  `last_boundary_changes`。
- `rewrite::append` 回傳 `Vec<RewriteCandidate>`，同時涵蓋主要候選與補助
  候選；主要候選的成本與順序不變，補助候選接在其後。
- `Symbols::lookup` 回傳借用切片；`LoadSymbolsFS` 沒有 `fs.FS` 抽象，直接
  讀取目錄下的 `.tsv`，排序規則與 Go `fs.Glob` 相同。
- session 的逐字変換在 debounce 等待期間顯示前次結果再加上尚未轉換的
  かな，畫面不會先退回かな；`tick_live` 由外殼的計時器呼叫。
