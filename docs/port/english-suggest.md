# english-suggest：英文拼字建議接線

把已收進來的英文拼字建議接到 `Mode::En` 與 `Mode::Mixed` 的候選清單，並讓
`jc-cli convert --mode en` 能直接驗證。沒有新增拼字演算法。

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/english/dictionary.go`：`Corrections`、`NearCommon` | `crates/jc-core/src/mixed.rs`：`Dictionary::corrections`、`near_common`、`osa_within` | ported（前一個 task 已收；本 task 只呼叫） |
| `core/english/dictionary_test.go`：`TestTypoChannelsAndValidWordProtection` | `mixed.rs`：`test_typo_channels_and_valid_word_protection` | ported（前一個 task 已收） |
| `core/personal/english.go`：`EnglishCorrections`、`EnglishPrefix` | `crates/jc-core/src/personal/english.rs`：`english_corrections`、`english_prefix` | ported（未接 session，原因見「刻意差異」） |
| `core/personal/corrections.go`：`CorrectionFor` | `crates/jc-core/src/personal/corrections.rs`：`correction_for` | ported（注音手選更正學習，不是英文拼字；本 task 未改） |
| `core/session/english_autocomplete.go`：`refreshEnglishCandidates` | `crates/jc-core/src/session.rs`：`english_candidates`、`rebuild_en` | rewritten：Go 的 `candState`／`EnglishAutocomplete` 狀態機不在本 crate；Rust 直接組 `Vec<Candidate>`，原字固定第一項 |
| `core/session/mixed.go`：`openMixedCandidates` 的英文段、`mixedSuggestion` | `crates/jc-core/src/session.rs`：`rebuild_mixed`；`crates/jc-core/src/mixed.rs`：`AsciiInput::accept` | rewritten：Go 的 `mixed.accepted` 改寫成「選取後取代待定按鍵」，其餘注音候選不在本 task |
| `core/ja/mixed/english.go`：`Candidates` | `mixed.rs`：`English::candidates`、`English::completion_candidates`、`English::spelling_candidates` | rewritten：拆成前綴補完與拼字建議兩個查詢，讓 session 可以保留名額 |
| `core/session/english_autocomplete_test.go`：`TestEnglishAutocompleteSpellingCorrectionRequiresAcceptance`、`TestEnglishAutocompletePreservesCase`、`TestEnglishAutocompleteKnownSpellingIsProtected` | `session.rs`：`test_english_candidates_include_spelling_suggestions` | rewritten：Rust 候選第一項是原字，Go 第一項是建議；大小寫與「打對的字不修正」照樣斷言 |
| `core/session/english_autocomplete_test.go`：`TestEnglishAutocompleteReservesCorrectionCandidates` | `session.rs`：`english_candidates` 的名額保留 | ported：最多 3 個拼字建議，其餘名額才給前綴補完 |
| `core/session/automatic_mixed_test.go`：`TestMixedCompletionCorrectionAndUndo` | `session.rs`：`test_mixed_candidates_include_spelling_suggestions` | rewritten：只搬候選順序與選取後送出；Esc／退格 undo 屬 session 其他狀態，未搬 |
| `cmd/0c-cli`（沒有 `convert --mode en`） | `crates/jc-cli/src/cli.rs`、`converter.rs` | n/a：Go 沒有對應子命令；Rust 的開發 CLI 另加 `--mode en` 方便驗證拼字建議 |

## 閾值與數量（照前身預設）

| 項目 | 值 | 來源 |
|---|---|---|
| 編輯距離 | 1；輸入長度 ≥ 6 時容許 2 | `core/english/dictionary.go` `corrections` |
| 拼字建議最短輸入 | 3 個字母 | 同上（`len(p) < 3` 直接回空） |
| 候選清單最短輸入 | 2 個字母 | `core/session/english_autocomplete.go` `len(s.mixed.raw) < 2` |
| 單詞長度上限 | 48 bytes | `mixed.rs` `MAX_WORD_BYTES` |
| 拼字建議保留名額 | 3 | `english_autocomplete.go`／`openMixedCandidates` |
| 候選上限 | `SessionConfig::max_candidates`（預設 `DEFAULT_MAX_CANDIDATES`＝80） | `session.rs`；Go 英文補字用 6 |
| 排序 | 距離 → 權重（高→低）→ 詞面 | `Dictionary::corrections` |
| 成本換算 | `2_000_000 - 權重 + 1000 × 距離` | `mixed.rs` `cost_of_weight`、`CORRECTION_PENALTY` |
| 打對的字 | 詞表已有完整單字時不產生拼字建議 | `Dictionary::corrections` 的 `has` 檢查 |
| 大小寫 | 全大寫查詢回全大寫；首字母大寫回首字母大寫；其餘照詞表小寫 | `Dictionary::corrections` |

## 刻意差異

| 項目 | 說明 |
|---|---|
| 候選第一項 | Go 英文補字清單第一項就是建議；Rust 保留原字在第一項，選字後才取代，讓 `Mode::En` 的全形變體候選與既有行為不變。 |
| 個人英文證據 | `EnglishCorrections`／`EnglishPrefix` 掛在 `personal::Model`，session 只持有 `PersonalStore`（`Store`），沒有 Store 層的英文查詢入口。本 task 不改 `personal/*`（非 owned path），先用詞表層；個人學過的拼法要等 personal 提供 Store 層 API 再接。 |
| 任務書用語 | 任務書寫「`personal::corrections` 已有拼字建議」，實際 `personal::corrections` 是注音手選更正學習（`CorrectionFor`）；英文拼字建議在 `core/english/dictionary.go`，已收在 `mixed::Dictionary::corrections`。這裡照實際模組接線。 |
| `convert --mode en` | 只列原字與拼字建議，不含前綴補完與全形變體，讓驗證輸出可判讀；互動路徑的前綴補完仍由 session 候選提供。 |
| 個人資料 | `--user-data` 在 `--mode en` 不影響拼字建議；拼字候選只看詞表。 |

## 測試

| 測試 | 資料 | 缺資料時 |
|---|---|---|
| `jc-core::session::tests::test_english_candidates_include_spelling_suggestions` | 系統英文資料包；找不到時退回內建原型 | 原型也含 `receive`／`hello`，不需要外部資料 |
| `jc-core::session::tests::test_mixed_candidates_include_spelling_suggestions` | 同上 | 同上 |
| `jc-core::mixed::tests::test_typo_channels_and_valid_word_protection` | 內建原型詞彙 | 不需要外部資料 |

`cargo test` 依共通規則留給整合階段；本 task 只確認 `cargo check --all-targets`
與 `cargo clippy -D warnings`。

## 實跑結果（開發機）

| 指令 | 結果 |
|---|---|
| `jc-cli convert "recieve" --mode en` | `recieve`、`receive`、`relieve`、`believe`、`deceive` |
| `jc-cli convert "helo" --mode en` | `helo`、`hello`、`help`、`halo`、`held` |
| `jc-cli convert "hello" --mode en` | 只有 `hello` |
| `jc-cli convert "RECIEVE" --mode en` | `RECIEVE`、`RECEIVE`、…（大小寫保留） |
