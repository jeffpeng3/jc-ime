# personal-improve：把 Mozkey 的三個回饋機制併入個人化

Worktree：`/mnt/work/jc/wt/personal-improve`
Branch：`personal-improve`（base：`main`）
前置：`docs/port/personal-zh.md`、`docs/reports/mozkey-study-2026-11-20.md`

## 範圍

在既有的 `personal` 上補三個機制，不新增儲存層、不改 UI、不動 `prune` 的
策略與預設門檻，也不引入 Zenz 或 feedback TSV。

| 機制 | 落點 | 狀態 |
|---|---|---|
| rollback window | `personal/mod.rs`（`Store`）、`session/core.rs`（`press` 前置） | 完成 |
| 弱負向訊號 | `personal/mod.rs`（`Store::skips`、`Store::rank_key`）、`session/core.rs` | 完成 |
| 最短觸發長度 | `personal/mod.rs`（`reading_len`、`note_passive_evidence`、`note_language_surface`）、`session/**` | 完成 |

## 檔案對照

| Mozkey 檔案 | 本專案檔案 | 狀態 |
|---|---|---|
| `src/session/zenz_feedback_store.cc:642` 到 `:698`（自動封鎖統計、`feedback_auto_blocked`） | `crates/jc-core/src/personal/mod.rs`（`skips`、`skip_count`、`rank_key`） | rewritten：門檻降到「連續 3 次跳過就不排第一」，不封鎖、不刪除 |
| `src/session/session.cc` 的 `accepted_rollback` 補償 | `crates/jc-core/src/personal/mod.rs`（`begin_rollback`、`rollback_learning`、`accept_learning`） | rewritten：用整份 `Store` 快照補償，語意與補償式撤銷相同 |
| `src/protocol/config.proto:421` 到 `:422`（feedback 最短自動套用 7 字元） | `crates/jc-core/src/session/mod.rs`（`SessionConfig::min_trigger_len`）、`personal/mod.rs`（`MIN_TRIGGER_LEN`） | rewritten：被動證據受門檻限制，明確手選不受限 |
| — | `crates/jc-core/src/personal/mod.rs`（`RollbackWindow`） | n/a：Mozkey 沒有對應結構，快照式實作是本版的選擇 |

## 1. Rollback window

提交學到的東西先留一份快照，下一次輸入決定要留下還是補償。

1. 選字（`Session::learn_candidate`）先呼叫 `Store::begin_rollback(now_ms)`，
   存下學習前的整份 `Store`。
2. `Session::press` 一進來就呼叫 `resolve_pending_learning`：
   - 下一個鍵是 `Backspace`、`Esc` 或 `Ctrl+Z`，且距提交不超過
     `ROLLBACK_WINDOW_MS`：`Store::rollback_learning` 把整份資料換回去，
     這次學習等於沒發生。
   - 其他按鍵或超出窗口：`Store::accept_learning` 丟掉快照，學習留下。
3. 快照 `#[serde(skip)]`，不寫進 `personal.json`；重啟後不會有待確認的學習，
   記憶體最多只有一份。

補償範圍包含那一次提交寫下的全部欄位：`uni`／`bi`／`ctx`／`evidence_*`、
`picked`／`readings`、`languages`／`cross_language`、`skips`，以及
`language_notes` journal。因此「打錯字馬上刪掉」不會被學進去，後續的
`undo_language_note` 也不會指到已被補償的紀錄。

`Store::undo_learning` 與 `Session::forget_word` 會先 `accept_learning`，
避免之後才發生的操作被一個舊快照蓋掉。

## 2. 弱負向訊號

`Store::skips` 是「詞面 → 連續被跳過次數」，寫進 `personal.json`。

- 選字時（`Session::note_candidate_skips`）如果選到的不是學過的詞，就把候選
  清單中排在它前面、且學過的詞各記一次跳過。
- 選到學過的詞就是把它選回來：`clear_skip` 清掉計數（自我修復）。
- `Store::rank_key` 的規則：
  - 自訂詞與手選釘選仍是第 0 組，優先於負向訊號。
  - 跳過次數未達 `SKIP_LIMIT`：每次折抵 `SKIP_PENALTY` 毫成本，只是降權。
  - 達到 `SKIP_LIMIT`：排到第 2 組，也就是一般候選之後；候選不會被移除。
- `Store::forget` 會一併清掉跳過計數；`Store::note_manual` 也會清。
- 日文：`Session::note_ja_skips` 對「前面被跳過、且曾選過的整句」記跳過；
  `Session::rank_ja_learning` 在統計＋學習排序之後，若第一名已達門檻，就跟
  第二名交換。只換一格，不動其他候選。
- 跳過表上限 `MAX_SKIPS`，超過先淘汰次數最少的條目。

## 3. 最短觸發長度

`personal::reading_len` 算讀音的非空白字元數。讀音短於
`SessionConfig::min_trigger_len`（預設 `MIN_TRIGGER_LEN` = 7）時：

| 路徑 | 短讀音的行為 |
|---|---|
| 注音被動接受 | `Store::note_passive_evidence`：只寫 `evidence_by_word`，不動 `uni`／`bi`／`ctx`，因此 `rank_key` 的折抵不變 |
| 日文被動接受 | `Store::note_language_surface`：只寫詞面次數與主要語言計數，不寫讀音與左文表；`UnifiedLearning` 讀的是後兩者，所以排序不變 |
| 混打的注音片段 | 同上，改走 `note_language_surface` |
| 明確手選（候選窗開著） | 不受門檻限制；`eval zh-golden` 的四個學習案例都是手選 |

`reading_len` 對注音是注音符號數（一個音節 1 到 3 個符號），對日文是假名數，
兩邊都不計分隔空白。門檻可調：`SessionConfig::min_trigger_len` 設 0 就等於
關閉。

## 常數

| 常數 | 值 | 位置 | 說明 |
|---|---|---|---|
| `ROLLBACK_WINDOW_MS` | 5000 | `personal/mod.rs` | 提交後可補償的時間 |
| `SKIP_LIMIT` | 3 | `personal/mod.rs` | 連續跳過幾次後不再排第一 |
| `SKIP_PENALTY` | 40 | `personal/mod.rs` | 每次跳過折抵的毫成本 |
| `MIN_TRIGGER_LEN` | 7 | `personal/mod.rs` | 最短自動套用的讀音長度 |
| `MAX_SKIPS` | 1024 | `personal/mod.rs` | 跳過計數表上限 |

## 已知差異與整合點

| 項目 | 現況 | 整合點 |
|---|---|---|
| 補償成本 | 每次提交複製整份 `Store`，與 `unlearn::Undo` 同一個策略 | 若 `Store` 變大再改成逐鍵 diff |
| 窗口的時間來源 | Session 用系統時鐘（毫秒）；`Store` API 由呼叫端傳 `now`，測試可以指定 | 需要重播時由宿主注入時間 |
| 日文負向訊號 | 只把第一名和第二名交換，沒有像注音那樣分組 | `UnifiedLearning` 若接上 `skips` 可以改成連續分數 |
| 短讀音的語言證據 | 只進 `surfaces` 與 `cross_language.primary`，不進反悔 journal | 需要逐筆反悔時改走 `note_language_weighted` 並另外標記 |
| `prune` | 未改策略與預設門檻 | — |

## 測試

| 需求 | 測試 |
|---|---|
| commit → 立刻 Backspace，學習回原狀 | `session::tests::test_commit_then_backspace_reverts_learning`、`personal::tests::test_rollback_restores_learning_inside_window` |
| 窗口到期保留學習 | `personal::tests::test_rollback_window_expires_and_keeps_learning` |
| 下一個輸入非撤銷鍵保留學習 | `session::tests::test_next_input_keeps_learning` |
| 連續跳過 3 次不再排第一、仍在候選中 | `personal::tests::test_skip_signal_penalises_without_deleting` |
| 選回來恢復優先度 | `personal::tests::test_selecting_skipped_word_restores_priority`、`session::tests::test_selecting_skipped_candidate_clears_skip` |
| 選字路徑真的寫入跳過訊號 | `session::tests::test_skipping_learned_candidate_records_signal` |
| 讀音 < 7 字元只記證據、排序不變 | `personal::tests::test_short_reading_records_evidence_only`、`test_short_reading_language_evidence_only`、`session::tests::test_ja_short_reading_records_evidence_only` |

## 驗證

```sh
cargo check -p jc-core --all-targets
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p jc-cli --release -- eval zh-golden
cargo run -p jc-cli --release -- eval ja --corpus modern-500
```

結果：

- `cargo test --workspace --no-fail-fast`：全綠；`jc-core` 588 個 lib 測試、
  `personal_port` 97 個。
- `cargo clippy --workspace --all-targets -- -D warnings`：通過，只剩環境原本
  的 `-Ctarget-feature fullfp16` 警告。
- `eval zh-golden`：45/45，與改動前相同。
- `eval ja --corpus modern-500`：top-1 86.0%、top-5 98.0%，與改動前相同。
