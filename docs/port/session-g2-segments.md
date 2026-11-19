# session-g2-segments：分段與候選呈現

上游：`/mnt/work/jc/ref/0c-ime` `8f443af`，`core/session/` 的 9 個檔案。
Base：`session-core-split`（`Session` 已拆成 `mod.rs` 與 `core.rs`）。

## 逐檔對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `segments.go` | `crates/jc-core/src/session/segments.rs` | rewritten：`segments`／`cursor`／`active_segment`／`segment_at`、`replace`／`insert`／`remove_segment`、`segment_locked`／`lock_segment`／`unlock_segment`。上游 `Segment` 有讀音、語言與 rune 範圍，`jc-core::Segment` 只有位元組範圍、模式與書寫系統，因此片段標籤以現有型別能表達的部分為準。`SetCursor` 標 n/a：`ZhInput`／`RomajiInput` 沒有組字中游標，這一輪不假造可移動游標。 |
| `untoned.go` | `crates/jc-core/src/session/segments.rs` | rewritten：`'` 分隔音節（`untonedSeparator`）、接不上的注音先完成目前音節（`untonedBreak`）、Enter／Tab／方向鍵前完成未打完音節（`untonedKey`）。現有模型把一聲音節當成可展開聲調的音節，因此用一聲完成；解碼效果與上游的 `UntonedMark` 相同。`untonedRun`、`untonedEnglish`、`typedReading`、`resolvedSyllables`、`lockReadings`、`rejectedReadings` 標 n/a：現有 session 沒有 `Preview02`／自訂英文詞追蹤，也沒有上游的 `path`／`matched` 學習介面。 |
| `phonetic_hint.go` | 無 | n/a：上游把提示放在 `Snapshot.Suggestion`／`SuggestionEdit`，並用 Tab 接受後寫入鎖定；`jc-core::Snapshot` 沒有提示欄位，直接改組字字面會讓 Enter 誤提交提示。依任務書「候選顯示欄位衝突先回報」，不改協定。 |
| `tone_mark.go` | `crates/jc-core/src/session/segments.rs` | ported：孤立聲調鍵先留數字，緊接的空白換成 `ˇ`／`ˋ`／`ˊ`／`˙`；`ToneMarkKeys` 對應 `tone_mark_keys`。`literalKeyAlternatives` 標 n/a：候選清單沒有原按鍵中繼資料，`Snapshot` 也不提供第二項來源。 |
| `candidate_mode.go` | 無 | n/a：`KeyCode` 沒有 Down／Up／Home／End／PageUp／PageDown，`Snapshot` 沒有 `navigating`、候選範圍、頁碼與游標 rune；`ZhInput` 也沒有組字游標。現有 `candidate_window` 只能表達「有候選」，無法拆成上游的「顯示中」與「明確選取」。 |
| `displayed.go` | 無 | n/a：現有 `rebuild_zh` 在整段解不出時直接顯示原讀音，沒有上游的逐音節 `displayed` 保底表；`decodeAroundUnreadable` 需要在 `core.rs` 改寫解碼流程，超出本群組「不改 core 行為」的邊界。 |
| `phonetic_candidates.go` | `crates/jc-core/src/session/segments.rs` | rewritten：`oneSymbolApart` 與鄰音表已移植，尾端 4–8 音節差一個注音符號的完整詞會列為候選。`phoneticWindow` 標 n/a：`jc-core` 沒有上游的 `decode.Window`／`OnlyCosted` 介面。 |
| `candidate_labels.go` | 無 | n/a：`Candidate` 與 `Snapshot` 沒有標籤欄位，`personal` 也沒公開上游的有效使用證據；直接改 `Candidate` 會動到協定。依任務書先回報。 |
| `tone_check.go` | 無 | n/a：`ZhInput::press_char` 在聲調鍵進來時直接完成音節，session 沒有「擋下聲調、留在暫存」的 hook；`refusedTone`／`toneRetry` 需要 `core.rs` 與前端一起改。 |

## 對 `mod.rs`／`core.rs` 的最小呼叫點

- `mod.rs`：新增 `locks`、`segment_manual`、`pending_tone` 三個欄位、`mod segments` 與 `tone_mark_keys` 轉出；`press` 在一般按鍵路徑前呼叫 `segments_press`，並讓 `snapshot` 使用 `segments()` 的視圖。
- `core.rs`：只加 `locks` 傳給 `best`／`nbest`、重建前清掉超出音節數的鎖定、`clear_composition` 清空三個新欄位，以及把 `phonetic_candidates` 接在模糊音候選之後。鎖定為空時解碼結果與原本完全相同。

## 測試

`crates/jc-core/src/session/segments.rs` 的 `tests`：

| Go 測試 | Rust 測試 |
|---|---|
| `TestSessionSegmentsFollowZhuyinPathAndCursor` | `test_session_segments_follow_zhuyin_path_and_cursor` |
| `TestSessionSegmentsKeepLongWordMerged` | `test_session_segments_keep_long_word_merged` |
| `TestSessionSegmentsReturnCopiesAndEditLanguages` | `test_session_segments_return_copies_and_edit` |
| `TestSessionSegmentSelectionLocksOnlyThatSegment` | `test_session_segment_locks_only_that_segment` |
| `TestLoneToneKeyPlusSpaceTypesToneMark` | `test_lone_tone_key_plus_space_types_tone_mark` |
| `TestToneKeyStaysDigitUnlessSpaceFollows` | `test_tone_key_stays_digit_unless_space_follows` |
| `TestToneKeyRunStaysNumber` | `test_tone_key_stays_digit_unless_space_follows` |
| `untoned_test.go` 的 `untonedSeparator`／`untonedBreak` | `test_toneless_separator_splits_syllables`、`test_toneless_break_starts_new_syllable` |
| `phonetic_candidates_test.go` 的單符號差 | `test_phonetic_candidates_find_one_symbol_apart_word`、`test_one_symbol_apart` |

另外 `test_manual_segments_expire_on_next_key` 驗證手動片段只存活到下一次按鍵，`test_tone_mark_keys` 驗證公開設定判斷。

## 驗證

- `cargo check -p jc-core --all-targets`：通過，只剩環境既有的 `-Ctarget-feature` 警告。
- `cargo fmt -p jc-core`：已執行。
- `cargo run -p jc-cli --release -- eval zh-golden`：45/45。
- `cargo run -p jc-cli --release -- eval ja --corpus modern-500`：86.0%。

依共通規則沒有執行 `cargo test`；測試只寫、編譯，交由整合階段統一跑。
