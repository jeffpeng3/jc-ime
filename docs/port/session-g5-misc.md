# session-g5-misc：其餘 session 行為

Worktree：`/mnt/work/jc/wt/session-g5-misc`
Branch：`session-g5-misc`（base：`main`）
上游：`/mnt/work/jc/ref/0c-ime` `8f443af`，`core/session/` 的 5 個檔案，共 959 行。

`crates/jc-core/src/session/misc.rs` 用 `impl Session` 掛在既有的 `Session`
上，不新增 trait。`Session` 只新增一個設定欄位 `document_context`；沒有新增
可變狀態，也沒有引入全域狀態。

## 逐檔對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/session/pending_literal.go` | `session/misc.rs`（`PendingLiteral`、`shift_pending_literals`、`restorable_pending_literal`） | partial／rewritten：待定字面的資料層、位置換算（`end`／`mark`）、收回條件與插入／刪除位移照搬。真正把音節插進字面、游標編輯後收回組音（`insertSyllableLiteral`、`restorePendingLiteral`、`retypeSyllableLiteral`）deferred：`ZhInput` 只有尾端 push／pop，沒有依索引插入或移除音節的 API。`clearPendingZhuyin` 同樣 deferred：本埠切到英文會整段清掉組字，沒有「保留已組好的字」的英文組字區。 |
| `core/session/vocab_mark.go` | `session/misc.rs`（`mark_hint`、`add_composition_word`、`WordMark`、`MIN_COMPOSE_WORD`、`text_in`、`typed_reading`、`system_has`） | partial／rewritten：加詞判斷、提示字串、讀音用使用者打的注音、加入後立即可打、系統詞優先、重複詞說明皆照搬；以音節索引範圍為介面，不依賴游標。`markState`／`vocabKey`（Shift＋←／→、Enter、Esc、刪除選取）與 `markSegments` 標示樣式 deferred：`Snapshot` 沒有 `Notices` 與 `StyleMarked`，`Session` 也沒有組字游標；`normalizeVocabKey` 的 `Preview02` 開關在本埠不存在。`forgetCandidate` 已由 `core.rs` 的既有簡化版覆蓋（沒有 notice 與重開候選）。 |
| `core/session/local_evidence.go` | `session/misc.rs`（`LocalEvidence`、`Session::local_evidence`） | rewritten：從目前仍有效的手選鎖定重建證據、相鄰音節與鄰詞弱觀察（0.25）、以位置排序並用 `BTreeMap` 固定加總順序，全部照搬。查詢函式改成 `LocalEvidence::at`。接進解碼器 deferred：`decode::bias` 的個人化掛鉤（`reading_lift`）固定回 0，`Decoder` 沒有依音節位置查證據的評分器。 |
| `core/session/document_context.go` | `session/misc.rs`（`document_left`、`committed_tail`）、`session/mod.rs`（`SessionConfig::document_context`） | partial／rewritten：已提交左文的視窗（沿用 `MAX_LEFT_CONTEXT_UNITS`）與「自己最近提交的完整末詞」已移植，尾端前綴與字面不算。`confirmDocumentContext` 的握手、`neuralContext`／`neuralCommitContext`、`documentScorer` 混分 deferred：協定沒有 `Context`／`ContextID`，`RerankOptions` 沒有前文長度，`Decoder` 也沒有文件評分器。 |
| `core/session/corrections.go` | `session/misc.rs`（`correction_selections`） | partial／rewritten：`configureCorrections` 取出手選片段的部分（排除跨音、字面邊界與被字面切開的鎖定，依位置排序）已移植。`CorrectionPenalty` 與 `enclosedSelections` deferred：`Session` 用的是 `personal::Store`，`correction_penalty` 在 `personal::Model` 上；`decode::bias::correction_penalty` 仍是回 0 的掛鉤。 |

## 測試對照

| Go 測試 | Rust 測試 | 狀態 |
|---|---|---|
| `retype_literal_test.go`（4 個）、`toggle_pending_test.go`（10 個） | `test_pending_literal_positions_and_syllable`、`test_shift_pending_literal_*`（5 個）、`test_restorable_pending_literal_needs_cursor_and_no_tone` | rewritten：只搬 `pendingLiteral` 的純資料行為與 `shiftPendingLiterals`；需要 `ZhInput` 逐格編輯與切換模式的案例 deferred（見上表）。 |
| `TestMarkAndAddWord` | `test_vocab_mark_hint_and_add_word` | rewritten：驗提示、加入、讀音、加入後立刻打得出；標示樣式與游標不驗。 |
| `TestMarkDuplicateAndSystemWord` | `test_vocab_mark_system_word_is_priority` | rewritten：系統詞「設為優先」與重複詞說明照搬。 |
| `TestMarkHintsAndEdits`（範圍檢查） | `test_vocab_mark_rejects_single_char_and_empty_range` | rewritten：單字與空範圍的提示、拒絕照搬；⌫ 刪除選取屬游標編輯，deferred。 |
| `TestMarkCancelAndOtherKeys`、`TestMarkRejectsPrivateAndLiterals`、`TestMarkKeysAreOrdinaryKeysWithoutPreview`、`TestUndoKey*`、`TestCandidateNoteForCustomWord`、`TestVocabAddBeatsEarlier*`、`TestVocabRemoveForgetsAndUndoRestores` | 無 | n/a／deferred：需要 `Snapshot.Notices`、標示樣式、組字游標、`Preview02`、私人／暫停狀態或候選附註欄位；`Session` 目前都沒有。 |
| `local_evidence_test.go`（6 個） | `test_local_evidence_from_neighbouring_lock`、`test_local_evidence_requires_lock_and_learning` | rewritten：只驗證據的建構（同讀音第二處、鄰詞 0.25、自身不重複累加、關閉學習）；重排結果與持久學習 deferred。 |
| `corrections_test.go`（3 個） | `test_correction_selections_reflect_locks` | rewritten：只驗手選片段取出；持久更正、候選排序與私人／暫停 deferred。 |
| `document_context_test.go`（7 個） | `test_document_context_uses_committed_tail` | rewritten：只驗已提交左文、完整末詞與設定關閉；握手、無效化、前文混分與評分結果 deferred。 |

## Deferred 依賴

| 行為 | 依賴 |
|---|---|
| 待定音節插入字面、游標編輯後收回、`↓` 列出原樣音節與其他聲調 | `ZhInput`／`Session` 的音節索引插入、移除與組字游標（Stage 6 主狀態機） |
| 切到英文只清未打完的注音、立即切回還原 | 英文模式的組字區保留注音音節；協定 `Restore` 旗標 |
| Shift＋←／→ 標示範圍、Enter 加入、Esc 取消、標示樣式 | `Snapshot` 的 `Notices`／`StyleMarked` 與組字游標 |
| 候選窗反悔鍵的 notice、重開候選、批次復原 | `Candidate` 沒有來源 ID；`Snapshot` 沒有 `VocabChanged`／`Forgot` |
| 本地證據參與解碼 | `Decoder` 需要依音節位置查個人證據的評分器（`decode/bias.rs` 的 `reading_lift`） |
| 文件前文混分、確認握手、神經前文 | 協定 `Context`／`ContextID`、`RerankOptions` 的前文長度、`Decoder` 的文件評分器 |
| 修正偏好成本、被長詞包住的手選 | `Session` 改用 `personal::Model`（`correction_penalty` 在 `Model` 上）與 `decode/bias.rs` 的 `correction_penalty` |

## 對 `mod.rs` 的最小改動

- 新增 `pub mod misc;`。
- `SessionConfig` 新增 `document_context: bool`（預設 `true`）；只影響
  `document_left` 與 `committed_tail` 的查詢，解碼結果不變。
- 沒有新增 `Session` 欄位；待定字面與加詞範圍都由呼叫端傳入。

## 驗證

- `cargo check --workspace --all-targets`：通過。
- `cargo clippy --workspace --all-targets -- -D warnings`：通過（只剩環境的
  `-Ctarget-feature` 警告）。
- `cargo fmt --all -- --check`：通過。
- `eval zh-golden`：45/45。
- `eval mixed`：6/6（`pending_literal` 沒有改動混合模式的路徑，仍照任務書
  重跑確認）。
- 依共通規則沒有執行 `cargo test`；測試只寫、編譯。
