# ja-personal：日文個人學習移植對照

日文個人學習整包移植到 `crates/jc-core/src/ja/personal.rs`。行為以 Go
`core/ja/personal` 為準：學習檔格式、左文節雜湊、反悔／忘記／重置／暫停、
N-best 重排與候選重排都照搬，沒有依用途刪減函式。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/personal/doc.go` | `crates/jc-core/src/ja/personal.rs` module 文件 | ported |
| `core/ja/personal/store.go` | 同上：`Store`、`Entry`、`Selection`、`key`、`normalize_left`、`context_hash`、`load`、`save`、`load_file`、`save_file` | ported |
| `core/ja/personal/learning.go` | 同上：`Learner`、`Converter`、`JaLearning` 實作、`DEFAULT_BOOST`／`DEFAULT_MAX_BOOST`／`DEFAULT_NBEST` | ported |
| `core/ja/personal/store_test.go` | 同上 `mod tests`（19 個測試） | ported |
| `core/ja/personal/learning_test.go` | 同上 `mod tests`（11 個測試） | ported |
| `core/personal/ja.go` | 同上：`FILE_NAME`、`ja_path` | ported（整合點，見下） |
| `core/personal/ja_test.go` | 同上：`test_ja_path_uses_same_directory_and_separate_file`、`test_ja_file_name_matches_shared_store_stub`、`test_japanese_learning_never_touches_zhuyin_store` | rewritten（共用 store 尚未存在，測試用本地 stub） |
| `internal/atomicfile/atomicfile.go` | 同上：`Store::save_file` 的暫存檔＋`sync_all`＋原子替換 | rewritten（只涵蓋個人學習檔需要的寫入協定） |

## 測試對照

Go 測試名沿用為 `test_*`：

| Go 測試 | Rust 測試 |
|---|---|
| `TestKeyUsesReadingAndNormalizedLeftContext` | `test_key_uses_reading_and_normalized_left_context` |
| `TestRecordLookupAndJSONContract` | `test_record_lookup_and_json_contract` |
| `TestJSONContractShape` | `test_json_contract_shape` |
| `TestSaveLoadRoundTripIsStable` | `test_save_load_round_trip_is_stable` |
| `TestSaveFileReplacesWholeFileAndCleansTemp` | `test_save_file_replaces_whole_file_and_cleans_temp` |
| `TestSaveFileFailureKeepsOriginalAndCleansTemp` | `test_save_file_failure_keeps_original_and_cleans_temp` |
| `TestLoadMissingFileIsEmpty` | `test_load_missing_file_is_empty` |
| `TestLoadCorruptFileReportsAndCanRecover` | `test_load_corrupt_file_reports_and_can_recover` |
| `TestLoadNewerFormatAndLanguageRejected` | `test_load_newer_format_and_language_rejected` |
| `TestLoadSkipsInvalidEntries` | `test_load_skips_invalid_entries` |
| `TestUndoRestoresWholeTransaction` | `test_undo_restores_whole_transaction` |
| `TestForgetMatchesSurfaceAndSegments` | `test_forget_matches_surface_and_segments` |
| `TestResetClearsLearningButKeepsPaused` | `test_reset_clears_learning_but_keeps_paused` |
| `TestPausedStillAppliesExistingRanking` | `test_paused_still_applies_existing_ranking` |
| `TestConcurrentRecordOnOneStore` | `test_concurrent_record_on_one_store` |
| `TestConcurrentWritersLeaveCompleteFile` | `test_concurrent_writers_leave_complete_file` |
| `TestConcurrentRecordAndSave` | `test_concurrent_record_and_save` |
| `TestZeroStoreIsUsable` | `test_zero_store_is_usable` |
| `TestLookupKeyAndLen` | `test_lookup_key_and_len` |
| `TestLearnedSelectionRanksFirst` | `test_learned_selection_ranks_first` |
| `TestDifferentLeftContextDoesNotReuseLearning` | `test_different_left_context_does_not_reuse_learning` |
| `TestLearningDoesNotOverrideClearStatisticalWinner` | `test_learning_does_not_override_clear_statistical_winner` |
| `TestCandidatesPutLearnedFirst` | `test_candidates_put_learned_first` |
| `TestLearningSourcesAreRecorded` | `test_learning_sources_are_recorded` |
| `TestLearnClausesChainsLeftContextAndSegments` | `test_learn_clauses_chains_left_context_and_segments` |
| `TestCostOnlyAtSentenceStartWithMatchingContext` | `test_cost_only_at_sentence_start_with_matching_context` |
| `TestLearnClausesAtRecordsTime` | `test_learn_clauses_at_records_time` |
| `TestLearnCandidateWithoutClauses` | `test_learn_candidate_without_clauses` |
| `TestNilStoreLearnerIsInert` | `test_nil_store_learner_is_inert` |
| `TestConverterPrefixesDelegate` | `test_converter_prefixes_delegate` |
| `TestJaPathUsesSameDirectoryAndSeparateFile` | `test_ja_path_uses_same_directory_and_separate_file` |
| `TestJaFileNameMatchesJapaneseStore` | `test_ja_file_name_matches_shared_store_stub` |
| `TestJapaneseLearningNeverTouchesZhuyinStore` | `test_japanese_learning_never_touches_zhuyin_store` |
| — | `test_last_used_zero_and_timestamps_are_provided_by_host`（補足 `Selection.At = 0` 契約） |

## 刻意差異

| 項目 | 說明 |
|---|---|
| 錯誤型別 | Go 的 `NewerFormatError`、`LanguageError`、`CorruptError` 改成 `thiserror` 的 `LoadError`、`SaveError`、`LoadFileError`。Go 的 `LoadFile` 在損壞時回傳 `(空白 Store, *CorruptError)`；Rust 用 `LoadFileError::Corrupt { store, .. }`，`store()` 取得同一份空白資料，其餘錯誤不得寫回。 |
| 檔案存取 | Go 注入 `fs.FS`；Rust 沒有對應的共用 trait，`load`／`save` 走 `Read`／`Write`，`load_file`／`save_file` 走 `std::fs`。`save_file` 合併 Go 測試用的 `internal/atomicfile` 協定：同目錄暫存檔、`sync_all`、關閉後 `rename`，失敗刪暫存檔。同一路徑的並行寫入仍由宿主序列化。 |
| Store 形狀 | Go 公開 `Format`、`Language`、`Entries` 欄位並以 `sync.RWMutex` 保護；Rust 欄位私有，內層 `RwLock<Inner>`，零值用 `Store::default()`。JSON 形狀由 `Entry`、`WireStore`、`RawStore` 固定，不變。 |
| 鎖毒化 | Go 的 mutex 沒有毒化概念；Rust 讀寫鎖中毒時取回內層資料繼續，不把 panic 傳給呼叫端。 |
| 學習加成尺度 | Go 的 `Learner.Cost` 回傳 `float64` 分數；Rust 的 `JaLearning::cost` 是 `i32` 詞成本，因此負加成乘上 `COST_SCALE = 1000` 後四捨五入。`Converter` 的整句重排仍在 `f64` 分數尺度。 |
| 候選結構 | Rust 的 `JaCandidate` 沒有 `Clauses`，`LearnCandidate` 一律把整筆候選當一個文節；`Candidates` 重排也以單一文節計分。Go 在候選帶有文節時會逐節計分。 |
| 候選分數 | Go 的 `lang.Candidate.Score` 改成 `JaCandidate.cost`（越小越優先），重排分數取 `-cost`，其餘排序鍵與 Go 相同。 |
| 空解碼器 | Go 的 `Converter` 允許 `Decoder == nil` 並回錯誤；Rust 用 `&JaDecoder`，所有權保證不會是空指標，因此沒有這個錯誤分支。 |
| 左文正規化 | 空白判定用 `char::is_whitespace`，大小寫轉換用 `char::to_lowercase`（完整映射）；Go 用 `unicode.IsSpace`、`unicode.ToLower`（單字元映射）。日常前文結果相同。 |
| 索引 | Go 允許直接建構 `Store{Entries: ...}`，`Lookup` 在索引為 nil 時線性掃描；Rust 欄位私有，所有寫入路徑都維護索引，不需要 fallback。 |
| `count` 型別 | Go `int` 改成 `i64`，JSON 仍是整數；載入時 `<= 0` 一律跳過。 |

## 整合點

`personal-zh` 仍在另一個 worktree，共用 personal store 尚未合併。這支 task 的
owned path 不包含共用 store，因此：

- [`FILE_NAME`] 與 [`ja_path`] 先放在 `ja/personal.rs`，行為對齊 Go
  `core/personal.JaFileName`、`JaPath`。
- 測試用 `ZH_STUB_FILE_NAME = "user.json"` 的本地 stub 驗證契約：日文與注音
  同目錄、不同檔名，日文學習、重置都不改動注音檔的任何位元組。
- `personal-zh` 合併後，`ja_path` 應改成呼叫共用版本，避免檔名常數漂移；
  商店介面（載入、存檔、路徑）已在這裡凍結，可直接對接。

## 驗證

- `cargo check -p jc-core --all-targets`：通過。
- `cargo clippy -p jc-core --all-targets -- -D warnings`：通過。
- `cargo fmt -p jc-core`：已執行。
- 依共通規則沒有執行 `cargo test`；測試已編譯，由整合階段統一跑。
