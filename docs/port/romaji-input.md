# romaji-input port table

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/session/ja_mixed.go`（`maybeJapanese`） | `crates/jc-core/src/session/mixed.rs`（`try_japanese_run`） | ported（既有；本 task 只補 `test_watashi_continues_in_mixed_mode`） |
| `testdata/jaeval/snapshot.json#ja-word-watashi` | `crates/jc-core/src/mixed_latin.rs`（`test_pure_romaji`） | rewritten：改用切分器合成題庫，斷言 `watashi` → `ja/わたし` |
| `core/lattice/scenario_test.go`（`watashi` 詞條） | `crates/jc-core/src/mixed.rs`（`test_watashi_routes_to_japanese`） | rewritten：只驗證路由器語言判定與表面，不移植 scenario harness |
| `internal/settings/*.go`（`mixed_input` 開關） | n/a | 由 `settings-default-file` task 移除；本 task 不改開關或預設值 |
