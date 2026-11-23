# session-g6-core：主狀態機整合

Worktree：`/mnt/work/jc/wt/session-g6-core`
Branch：`session-g6-core`（base：`main`，已含 G1–G5）
上游：`/mnt/work/jc/ref/0c-ime` `8f443af`，`core/session/session.go`（2,220 行）。

本輪只改 `crates/jc-core/src/session/core.rs` 與 `session/mod.rs`，把 G1–G5 的
hook 接進同一條按鍵流程，並補上快照欄位。沒有改任何群組檔案的常數或門檻。

## 上游行為 → Rust 位置

| 上游（`session.go`／相關檔） | Rust 位置 | 狀態 |
|---|---|---|
| `Session` 欄位、`New`、`SetLayout` | `session/mod.rs`（`Session`、`Session::new`） | ported（欄位已依 Stage 0 分組） |
| `Handle` 的請求分派 | `Session::press`、`set_mode`、`toggle_english`、`open_symbol_menu`、`open_symbol_tab`、`open_palette`、`open_emoji`、`page`、`select`、`commit`、`reset` | rewritten：Rust 用一個 `Key`／公開方法對應多種 `proto.Request`，沒有 `handled` 旗標 |
| `Handle` 的 `Commit`／`Cancel` | `Session::commit`、`reset` | ported |
| `Handle` 的 `Page`／`Select` | `Session::page`、`select`、`core::select_by_digit` | ported |
| `key()` 的按鍵順序（風格、選單、候選、一般鍵） | `Session::press` → `style_key`（G1）→ `menu_key`（G3）→ `segments_press`（G2）→ `core` 分派 | integrated |
| `key()` 的 `Enter` | `core::finish_composition` | ported：選字＋送出一次完成（Rust 協定以一次 `Enter` 回 `Commit`） |
| `key()` 的 `Escape` 優先權 | `core::escape`、`clear_pending_zhuyin` | ported：關候選窗 → 清未完成注音 → 清整段 |
| `key()` 的空白 | `core::space`、`segments::literal_space`、`segments::finish_tone_key` | ported：完成音節／開候選窗／選字；孤立聲調鍵＋空白在 G2 |
| `key()` 的 `Backspace`（候選窗） | `core::backspace` 前段 | ported：只關窗、不刪字 |
| `key()` 的 `Delete`（候選窗） | `core::delete_forward` | rewritten：沒有組字游標，只關窗 |
| `key()` 的 `Tab` | `Session::press` 的 `Tab` 分支、`core::accept_suggestion` | rewritten：補全優先，選取模式中換下一項；上游的換詞長未接 |
| `key()` 的 `Up`／`Down` | `core::up`、`core::down` | ported：↓ 第一次進選取模式不移動，之後上下換項 |
| `key()` 的 `PageUp`／`PageDown` | `core::page_step` | ported：頁碼頭尾相接 |
| `key()` 的 `Home`／`End` | `core::candidate_edge` | rewritten：只在選取模式換第一／最後一項；只是顯示時上游移動組字游標，本埠沒有游標 |
| `key()` 的左右鍵 | `core::navigate` | rewritten：本埠沒有組字游標，左右鍵沿用既有「開窗／換項」簡化 |
| `commit()` | `Session::commit`、`core::finish_composition`、`finalize`、`output_text`、`model_punctuated` | ported（學習、簡體與中英空格沿用既有路徑；簡體只在 commit 轉換） |
| `reset()` | `core::clear_composition` | ported：新增 `navigating` 一併清除 |
| `snapshot()` 的候選清單與反白 | `Session::snapshot`、`core::rebuild` | ported（欄位子集） |
| `snapshot()` 的 `Suggestion` | `core::snapshot_suggestion` | partial：注音補全尾綴與混打英文補全已填；`SuggestionEdit` 不在本埠協定 |
| `snapshot()` 的 `label` | `core::highlighted_label`（`COMMON_LABEL_PROBABILITY`） | partial：`personal`（自訂詞／手選）與 `common`（unigram ≥ 1e-4）已填，`related` 未接 |
| `snapshot()` 的 `navigating` | `Session::snapshot`、`core` 的選取模式 | ported |
| `candidatePage`／`pages` | `Session::page_size`、`core::page_step`、`Session::page` | ported |
| `openCandidates`／`openCandidatesNow` | `core::rebuild_zh` 的候選清單＋`core::space`／`down` 開窗 | rewritten：候選清單一直跟著組字存在，`candidate_window` 只表示「已開窗」 |
| `moveHighlight` | `core::move_highlight`、`navigate`、`up`／`down` | ported（沒有跨詞長） |
| `selectCandidate` | `core::select_by_digit`、`finish_composition`、`learn_candidate` | rewritten：選字即送進待提交區，不保留組字游標與鎖定 |
| `accept`／`acceptZh` | `core::accept_suggestion` | rewritten：接受後以整詞取代組字，不新增音節與鎖定 |
| `composing`／`Composing` | `input_style::composing`（注音）、`snapshot` 的組字字串 | partial：沒有 `cand != nil` 的公開查詢 |
| `setMode`／`Toggle` | `Session::set_mode`、`toggle_english` | ported：`toggle_english` 記住切換前模式，再按一次回到 Mixed/Ja，而非固定 Zh |
| `capsLockChanged`／`CapsLockOn` | 無 | n/a：`Key` 沒有 caps_lock 請求，外殼直接送 `set_mode` |
| `View`／`status` | `Session::snapshot` | rewritten：模式與語言欄位由外殼的 `Snapshot` 表達 |
| `ctrlPunct` | `core::press_ctrl` | ported（G1） |
| `pressKey`／`symbol`／`insert` | `zh::input::ZhInput`、`segments`、`core::press_char` | ported（G1／G2） |
| `backspace`（組字內） | `core::backspace`、`ZhInput::press` | ported（尾端退格） |
| `eraseBeforeCursor`／`shiftLocks`／`moveTo` | `segments::lock_segment`、`decode::Locks` | deferred：沒有組字游標與任意位置插入 |
| `lattice`／`signature`／`refresh`／`sugKey` | `core::rebuild*`、`Session::snapshot` | rewritten：每次按鍵重建，沒有路徑簽章快取 |
| `openCandidatesNow` 的詞長／游標選窗 | `core::rebuild_zh` | deferred：固定整段組字，沒有詞長切換 |
| `stepCandidate`／`unitStart`／`unitEnd` | 無 | n/a：`j`／`k` 換字需要組字游標與分詞範圍 |
| `nextCandidateLength`／`candidateAnchor` | 無 | deferred：候選清單沒有多詞長 |
| `readingPins`／`output` | `Session::output_text`、`zhconv::Converter` | rewritten：簡體轉換在 commit 走整串轉換，沒有讀音 pin |
| `punctuate`／`punctuateWith` | `core::puncted`、`model_punctuated`、`config.punct` | ported（G5 前的既有路徑） |
| `ForgetLearning` | `Session::undo_learning`、`forget_word` | partial：沒有「整份清除後這段組字不學」旗標 |
| `Learned` | `personal::PersonalStore` 查詢 | rewritten |

## 本輪接上的 hook

1. `Session::navigating`：候選窗顯示與明確選取分開。`down` 第一次只進入選取
   模式，第二次才換項；打字、Esc、退格、Delete、選字、送出都離開這個模式。
2. 新按鍵：`Down`／`Up`／`PageDown`／`PageUp`／`Home`／`End`，沿用
   `page_size()` 與頁碼頭尾相接。
3. 空白鍵在音節完成後開候選窗（上游 `openCandidates(0)`）；「空白只插空白」
   設定（`InputStyle::space_inserts_space`）維持不開窗。
4. Esc 優先權：先關候選窗，再清未完成的注音，最後才清整段組字。`ZhInput`
   沒有清暫存的公開 API，`clear_pending_zhuyin` 用退格逐格退掉暫存，不碰已
   完成的音節。
5. 候選窗開著時退格與 Delete 只關窗，不刪字。
6. `Snapshot.suggestion` 填入 Tab 會補上的尾綴（注音補全、混打英文補全）。
7. `Snapshot.label` 填入 `personal`（自訂詞、手選過的單字）與 `common`
   （系統 unigram 機率 ≥ `COMMON_LABEL_PROBABILITY`）。
8. `Snapshot.navigating` 反映明確選取模式。
9. 選取模式中的 Tab 換下一項，不再先接受補全。

## 仍 deferred

| 行為 | 依賴 |
|---|---|
| 組字游標（←／→／Home／End 移動、任意位置插入與刪除、`eraseBeforeCursor`、`shiftLocks`） | `ZhInput`／`RomajiInput` 的游標 API 與 `Segment` 的 rune 範圍 |
| Tab／上下鍵換詞長（`nextCandidateLength`、`candidateAnchor`、跨詞長 `moveHighlight`） | 候選清單要保留每個詞長的項目；目前只有整段組字一份 |
| `j`／`k` 換前後一個詞（`stepCandidate`、`unitStart`／`unitEnd`） | 組字游標與分詞範圍 |
| 選字後鎖定片段並留在組字區（`selectCandidate` 的 `decode.Lock`） | Rust 選字目前直接送進待提交區；改成鎖定會動到四組題庫的既有輸出 |
| `keepCandidates`：候選窗跟著之後的組字留在畫面 | 需要在打字後重開清單，且會讓數字鍵在開窗後變成選字鍵；與現有題庫流程衝突 |
| `related` 候選標示 | 需要已提交左文的最後一個完整詞（`contextLeft`）與文件前文權重 |
| 組字中加詞標示、反悔 notice、`VocabChanged`／`Forgot` | G5 已列：`Snapshot` 沒有 `Notices` 等人員欄位 |
| Caps Lock 模式切換 | `Key` 沒有 caps_lock 請求 |
| 日文 runtime 的候選／`Converting` 欄位 | `Snapshot` 沒有語言 runtime 欄位，本輪不動 |
| 同音替換預覽（`previewRewrite`）與神經次佳提示 | 沒有 `SuggestionEdit` 與神經 hint 來源 |

## 與上游的差異

- 空白鍵在候選窗開著時是「選字」，不是上游預設的「翻頁」。這是
  `defaults-policy` 留下的產品決定，本輪不改。
- 左右鍵沿用既有的「第一次開窗、第二次換項」，不對應上游的翻頁與組字游標。
- 選字（數字鍵、空白、Enter）一次完成送出；上游先鎖定、下一次 Enter 才送出。
- 候選清單一直隨組字重建，`candidate_window` 只是顯示開關；上游用
  `cand` 指標與 `keepCandidates` 控制。

## 測試

新增 13 個行為測試在 `crates/jc-core/src/session/core.rs` 的 `tests`：

| 上游測試 | Rust 測試 |
|---|---|
| `TestDownEntersSelectionModeThenArrowsPage`（前半） | `test_down_enters_selection_mode_without_moving` |
| `TestDownEntersSelectionModeThenArrowsPage`（翻頁） | `test_page_keys_switch_pages_and_wrap` |
| `TestHomeEndDependOnSelectionMode` | `test_home_end_move_only_in_selection_mode` |
| `TestTypingLeavesSelectionModeAndCandidateWindowFollows` | `test_typing_leaves_selection_mode` |
| `TestEscLeavesSelectionMode` | `test_escape_closes_candidate_window_and_keeps_composition` |
| `TestCandidateWindowSwallowsKeys`（退格） | `test_backspace_closes_candidate_window_without_deleting` |
| `TestCandidateWindowSwallowsKeys`（Delete） | `test_delete_only_closes_candidate_window` |
| `TestShownCandidatesLetArrowsMoveCompositionCursor`（空白開窗） | `test_space_opens_candidates_after_complete_syllable` |
| `TestStyleCandidatePageSizeAndSpace` 的關閉設定 | `test_space_inserts_space_keeps_literal_behavior` |
| `TestPotentialSuggestions` 類的補全尾綴 | `test_snapshot_suggestion_is_completion_tail` |
| `TestCandidateLabels` 的個人標示 | `test_snapshot_label_marks_personal_vocab` |
| （無直接對應） | `test_escape_clears_pending_syllable_before_composition` |
| （無直接對應） | `test_snapshot_label_is_none_without_evidence` |

## 驗證

- `cargo test --workspace --no-fail-fast`：1,057 passed、1 ignored（45 suites）。
- `cargo clippy --workspace --all-targets -- -D warnings`：通過，只有環境的
  `-Ctarget-feature` 警告。
- `eval mixed`：6/6。
- `eval zh-golden`：45/45。
- `eval ja --corpus modern-500`：86.0%。
- `eval fuzzy`：注音 top-1 44.5%。
