# composition-cursor：組字游標與片段編輯

上游：`/mnt/work/jc/ref/0c-ime` `8f443af`。
Branch：`composition-cursor`。Owned paths 只有 `crates/jc-core/src/session/**` 與本文件；
`jc-model`、`jc-engine` 的協定沒有改。

本輪把注音組字從「游標固定在結尾」改成「工作階段持有游標與音節清單」：

- 已完成的音節搬到 `Session::syllables`，`ZhInput` 只留尚未完成的注音。
- `Session::cursor` 是音節格位置；對外一律換算成 rune 位移。
- 任意位置插入、Backspace／Delete、換詞長、`j`／`k` 換字與 `keepCandidates`
  都只動游標所在片段，其他片段由整段解碼重新推導。

## 上游對照表

| Go 檔案／函式 | Rust 位置 | 狀態 |
|---|---|---|
| `session.go` `Session.syls`、`cursor` | `session/mod.rs` 的 `syllables`、`cursor` | rewritten：音節清單與游標改由 `Session` 持有，`ZhInput` 只負責暫存注音。 |
| `session.go` `insert`、`pressKey` 的音節插入 | `core::insert_syllable`、`press_char`、`complete_pending` | rewritten：完成的音節插在游標位置，不再固定接在尾端。 |
| `session.go` `eraseBeforeCursor`、`backspace` | `core::remove_syllable`、`backspace` | ported：Backspace 刪游標前一音節，暫存注音先逐格退。 |
| `session.go` `shiftLocks` | `core::shift_locks` | rewritten：鎖定、字面插入點一起平移；被編輯點切到的鎖定解除。 |
| `session.go` `moveTo`、`segments.go` `SetCursor` | `core::move_cursor_to` | rewritten：游標以音節格儲存，對外換成 rune；暫存注音未完成時不移動。 |
| `segments.go` `Cursor`、`ActiveSegment`、`SegmentAt` | `segments::cursor`、`active_segment`、`segment_at` | ported：片段結尾屬於下一個片段，整段結尾仍算最後一段。 |
| `session.go` `stepCandidate`、`unitStart`／`unitEnd` | `core::cell_segment`、`press_char` 的 `j`／`k` | rewritten：`j`／`k` 只在明確選取模式中換候選；換的是游標片段，不是前後詞。 |
| `session.go` `moveHighlight` | `core::move_highlight` | ported：候選清單頭尾相接；跨詞長移動仍 n/a。 |
| `session.go` `nextCandidateLength`、`candidateAnchor` | `core::resize_segment` | rewritten：Shift＋←／→ 直接調整游標片段長度；Tab 換詞長未接。 |
| `session.go` `selectCandidate`、`decode.Lock` | `core::select_window_candidate` | rewritten：游標片段選字只寫入該段的鎖定，不送出整段組字。 |
| `session.go` `keepCandidates` 欄位 | `session/mod.rs` 的 `keep_candidates` | ported：換字後候選窗留著，Esc 或游標離開片段才關。 |
| `session.go` `openCandidatesNow` 的詞長／游標選窗 | `core::rebuild_zh`、`window_candidates` | rewritten：候選只解碼編輯視窗的讀音；整段顯示仍由完整解碼產生。 |
| `session.go` `candidateAnchor` 的模糊音多詞長 | 無 | deferred：候選清單仍沒有每個詞長的完整集合。 |
| `session.go` `insert` 的字面順序鍵 | `core::shift_locks` 的 `literal_anchor` | partial：只保留單一字面字串的插入點，沒有逐格字面鍵。 |

## 游標語意

- 內部 `Session::cursor` 是音節格，`0..=syllables.len()`；`Snapshot.cursor_rune`
  與 `Snapshot.cursor_utf16` 都描述同一個 rune 邊界，沒有使用 byte 位移。
- 游標在片段結尾屬於下一個片段（半開區間）；在整段結尾算最後一個片段。
- 暫存注音非空時不移動游標；新的注音打完成一個音節後插在游標位置，游標移到
  該音節後面。
- 游標移到中間後，候選清單只算游標所在片段；選字只鎖定這一段，其他片段由
  整段解碼重算。游標回到整段結尾時恢復整段候選。
- `segments` 仍是 `preedit` 的 UTF-8 位元組半開區間；未完成的注音會補成最後
  一個片段，選字後片段一定重新覆蓋整個 `preedit`。

## 按鍵

| 按鍵 | 行為 |
|---|---|
| ←／→ | 候選窗顯示中且還沒進入選取模式時移動游標；已在頭尾時改成換候選。 |
| Home／End | 顯示候選時移到組字頭／尾；選取模式中移到候選第一／最後一項。 |
| Backspace | 有暫存注音先退一格；否則刪游標前一音節。 |
| Delete | 刪游標後一音節；沒有可刪音節時只關候選窗。 |
| Shift＋←／→ | 縮短／加長游標片段的讀音長度，重新解碼候選。 |
| ↓／↑ | 進入選取模式與換候選；第一下只進入選取模式。 |
| `j`／`k` | 選取模式中換前後一個候選，只套用到游標片段。其他時候仍是注音鍵。 |
| Enter | 游標片段編輯中先把候選套到該段；其餘情況送出整段組字。 |
| Esc | 先關候選窗並離開片段編輯，保留組字；再按才清暫存或整段。 |

`keepCandidates` 在片段換字成功後保持候選窗與選取模式；Esc 或游標離開該片段
後恢復一般顯示。

## 解除的 deferred

`docs/port/session-g6-core.md` 列出的下列項目由本輪解除：

| 原 deferred | 現況 |
|---|---|
| 組字游標（←／→／Home／End） | 已接；移動時可跨音節，對外是 rune 位移。 |
| `eraseBeforeCursor`、`shiftLocks`、`moveTo` | 已接；插入與刪除會平移鎖定與字面插入點。 |
| 任意位置插入與刪除 | 已接；完成的音節可插在游標位置。 |
| `j`／`k` 換詞 | partial：只換游標片段的候選，沒有上游的前後詞巡覽。 |
| 上下鍵換詞長（`nextCandidateLength`） | partial：Shift＋左右可直接調詞長；Tab 循環與跨詞長 `moveHighlight` 仍未接。 |
| 選字後鎖定片段並留在組字區 | 已接；游標片段選字只鎖定該段。 |
| `keepCandidates` | 已接；換字後候選窗保留到 Esc 或游標離開。 |

## 神經模型

組字中的半截讀音仍不送 Jinen：`rebuild_ja` 在統計結果等於讀音時回
`NeuralDecision::Skipped`。本輪沒有改這條規則，只補上以 stub 計數的行為測試。

## 測試

| 任務書測試 | Rust 測試 |
|---|---|
| 中間插入與直接打完整字串相同 | `test_insert_syllable_at_cursor_matches_direct_typing` |
| 中間 Backspace／Delete 刪對位置、片段連續 | `test_backspace_and_delete_in_middle_keep_segments_whole` |
| 換詞長改變候選、片段覆蓋 preedit | `test_resize_segment_changes_candidates_and_keeps_segments` |
| `j`／`k` 只動游標片段 | `test_jk_changes_only_cursor_segment` |
| `keepCandidates` 換字後保留、Esc 關閉 | `test_keep_candidates_after_segment_change_until_escape` |
| 半截讀音不呼叫神經模型 | `test_ja_neural_not_called_for_incomplete_reading` |

另外更新兩個舊測試，反映新的游標編輯語意：

| 舊行為 | 新測試 |
|---|---|
| 候選窗開著時 Backspace 只關窗 | `test_backspace_deletes_before_cursor_and_closes_candidates` |
| Delete 只關候選窗 | `test_delete_removes_after_cursor_or_closes_candidates` |

依共通規則沒有執行 `cargo test`；測試只寫、編譯，交由整合階段統一跑。

## 驗證

- `cargo check -p jc-core --all-targets`：通過。
- `cargo fmt -p jc-core`：已執行。
- `cargo run -p jc-cli --release -- eval zh-golden`：45/45。
- `cargo run -p jc-cli --release -- eval ja --corpus modern-500`：92.2%（預設 `auto` 門檻）。
- `cargo run -p jc-cli --release -- eval ja --corpus hard-cases`：77.1%。
- `cargo run -p jc-cli --release -- eval mixed`：6/6。
