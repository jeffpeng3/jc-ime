# session-backfill：補回 G2 的五個 n/a

Worktree：`/mnt/work/jc/wt/session-backfill`
Branch：`session-backfill`（base：`main`，已含 `proto-extend` 與 `session-g6-core`）
上游：`/mnt/work/jc/ref/0c-ime` `8f443af`，`core/session/` 的五個檔案。

這一輪把 `proto-extend` 留下的空欄位真的接上：新按鍵、`navigating`、
`suggestion`、`label`、`cursor_rune`、`tone_blocked`。四組題庫與 `main` 相同。

## 逐檔對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `candidate_mode.go` | `session/core.rs`、`session/segments.rs` | rewritten：候選顯示與明確選取分開。`Snapshot::navigating` 只在按過 ↓／↑／翻頁／Tab 後為 `true`；顯示中時 ←／→／Home／End 移動組字游標，↓ 進入選取模式，翻頁鍵沿用頁碼頭尾相接。上游的 `keepCandidates` 重開、候選格線與 `j`／`k` 換詞沒有對應狀態，維持 n/a。 |
| `displayed.go` | 無 | n/a：`decodeAroundUnreadable` 修的是「一個沒有字的音節讓後面全部退回注音」。本埠的 `Lattice::build` 對每個音節都補未知詞邊，解碼不會因為一個音節沒有詞條而讓後面的路徑失效，這個失效模式不存在。`fallbackDisplay` 的字面／近音保底已由 `literal` 插入與模糊音候選表達。 |
| `phonetic_hint.go` | `session/core.rs`（`snapshot_suggestion`、`accept_suggestion`） | partial：`Snapshot::suggestion` 已填注音尾端補全與混打英文補全，Tab 接受後以整詞取代組字。`findPhoneticHint` 的同音替換排名需要 `decode.Window`、一筆編輯成本與路徑鎖定，且協定沒有 `SuggestionEdit`，維持 n/a；被動接受的錯字不會被偷改。 |
| `candidate_labels.go` | `session/core.rs`（`highlighted_label`） | ported（子集）：`personal` 用自訂詞與手選紀錄，`related` 用已送出左文尾巴的連續漢字查 bigram，提升達 `2×` unigram 才標，`common` 用系統 unigram `≥1e-4`；優先序為個人、關聯、常用。個人證據門檻沒有公開介面，仍以自訂詞／手選判定。目前 `build-zh/lm.bin` 的 bigram 筆數是 0，因此 `related` 在現有資料上不會出現。 |
| `tone_check.go` | `session/segments.rs`（`install_tone_gate`）、`zh/input.rs` | ported：`ZhInput` 的 `ToneGate` 改成 `Arc` 閉包，工作階段把詞典查詢包進去。加聲調後完全沒有字、也不是任何詞的開頭、模糊音近音也沒有字時擋下，注音留在暫存；單一注音符號、聲調選填、混打、沒有開設定時不擋；同一處連按同一個聲調第二次放行一次。上游的快照 `Bell` 不在本埠協定，`refusedTone.cursor` 也沒有對應欄位。 |

## 接上的協定欄位

| 欄位／按鍵 | 行為 |
|---|---|
| `Down`／`Up` | 有候選時開窗並進入選取模式；↓ 第一下不移動反白，之後上下換項。 |
| `PageDown`／`PageUp` | 有候選時開窗並換頁，頁碼頭尾相接。 |
| `Home`／`End` | 選取模式中跳第一／最後一項；只是顯示候選時移動組字游標到開頭／結尾。 |
| `ArrowLeft`／`ArrowRight` | 只是顯示候選時移動組字游標；選取模式中換項；還沒開窗時維持原本的開窗行為（golden 的 `Down` 別名不受影響）。 |
| `navigating` | 明確選取模式才為 `true`；打字、Esc、退格、Delete、選字與送出都會離開。 |
| `suggestion` | 注音尾端補全或混打英文補全的尾綴；候選窗開著或已接受補全時為 `None`。 |
| `label` | `personal`／`related`／`common`，每列最多一個，不影響排序。 |
| `cursor_rune` | 由 `ZhInput` 的音節格沿目前解碼路徑換成 rune 位移；`cursor_utf16` 用同一個位元組位置換算。沒有路徑時留在結尾。 |
| `tone_blocked` | `ZhInput` 記錄最近一鍵是否擋下聲調，下一個按鍵清掉；`Session::tone_blocked()` 公開查詢。 |

## 超出 owned paths 的必要修改

| 檔案 | 修改 |
|---|---|
| `crates/jc-core/src/session/mod.rs` | 新增 `SessionConfig::zhuyin_error_hint`（預設 `false`，由外殼依設定開啟）、`Session::new` 安裝聲調閘門、`snapshot`／`cursor_utf16` 改共用 `cursor_byte()`。 |
| `crates/jc-core/src/zh/input.rs` | `ToneGate` 從函式指標改成 `Arc<dyn Fn>`，`ZhInput` 的 `Debug` 改手寫（策略不參與輸出）。 |

`PROTOCOL_VERSION` 沒有改。

## 測試

| Go 測試 | Rust 測試 |
|---|---|
| `TestShownCandidatesLetArrowsMoveCompositionCursor` | `test_arrows_move_composition_cursor_when_shown`、`test_home_end_move_composition_cursor_when_shown`、`test_home_end_cursor_inside_multi_syllable_word` |
| `TestDownEntersSelectionModeThenArrowsPage`（第一段） | `test_down_enters_selection_mode_without_moving` |
| `TestDownEntersSelectionModeThenArrowsPage`（翻頁） | `test_page_keys_switch_pages_and_wrap`、`test_page_keys_open_candidates_before_shown` |
| `TestHomeEndDependOnSelectionMode` | `test_home_end_move_only_in_selection_mode` |
| `TestTypingLeavesSelectionModeAndCandidateWindowFollows` | `test_typing_leaves_selection_mode` |
| `TestToneWithoutCharactersIsRefused` | `test_tone_blocked_keeps_pending_when_reading_has_no_word`、`test_tone_blocked_flag_clears_on_next_key` |
| `TestToneWithReadingIsNotRefused` | `test_tone_allowed_when_reading_has_word` |
| `TestSingleSymbolIsNotRefused` | `test_single_symbol_reading_is_not_blocked` |
| `TestSameToneAgainKeepsTheLiteral` | `test_tone_retry_allows_deliberate_bopomofo` |
| `TestToneCheckOffInMixedInputAndToneless`（設定關閉與聲調選填） | `test_tone_gate_is_off_by_default`、`test_tone_gate_is_off_when_toneless` |
| `nearReachable` | `test_fuzzy_near_reading_is_not_blocked` |
| `TestCandidateLabelsDoNotFollowRankOrHighlight`（關聯／常用） | `test_label_context_uses_trailing_han_only`、`test_snapshot_label_is_none_without_evidence` |
| `TestPersonalLabelRequiresRepeatedEvidenceAndPreservesStore`（子集） | `test_snapshot_label_marks_personal_vocab` |
| （無直接對應） | `test_snapshot_suggestion_is_completion_tail` |

## 驗證

- `cargo test --workspace --no-fail-fast`：1,070 passed、1 ignored（45 suites）。
- `cargo clippy --workspace --all-targets -- -D warnings`：通過。
- `eval mixed`：6/6。
- `eval zh-golden`：45/45。
- `eval ja --corpus modern-500`：86.0%。
- `eval fuzzy`：注音 top-1 44.5%。

## 未完成事項

- `zhuyin_error_hint` 的設定接線：`jc-settings` 有 `zhuyin_error_hint`，但 `jc-engine`／`jc-cli` 不在 owned paths，這一輪只提供 `SessionConfig` 欄位，沒有把設定檔的值傳進來。
- `related` 標示已接上 bigram，但目前 `lm.bin` 沒有 bigram 資料；模型換成有 bigram 的版本後才會出現。
- 組字游標只支援移動與顯示；任意位置插入、刪除與 `eraseBeforeCursor`、`shiftLocks` 仍未接。
- 候選清單固定是整段組字，換游標不會像上游一樣只列目前片段的候選。
