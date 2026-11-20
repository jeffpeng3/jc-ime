# english-personal：英文拼字建議接個人化

Worktree：`/mnt/work/jc/wt/english-personal`
Branch：`english-personal`（base：`main`）

接受過的英文拼字修正會影響之後的候選排序；拒絕時沿用
`personal-improve` 的弱負向訊號。內建詞表仍然先查，個人學習只負責把學過的
修正排到前面。

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/personal/english.go`：`EnglishCorrections`、`EnglishPrefix` | `crates/jc-core/src/personal/english.rs`：`english_corrections`、`english_prefix` | ported；本 task 未改演算法 |
| `core/session/english_autocomplete.go`：`refreshEnglishCandidates` | `crates/jc-core/src/session/mixed.rs`：`english_candidates` | rewritten：改成先問內建詞表，再問 `Store`；學過的修正排前面 |
| `core/session/session.go`：送出時 `LearnEnglish` | `crates/jc-core/src/session/core.rs`：`learn_en_candidate`、`learn_english_choice` | rewritten：Go 沒有「打錯 → 修正」配對；本版在 `Store` 另存一張表 |
| `core/personal/english.go`：`LearnEnglish` | `crates/jc-core/src/personal/english.rs`：`note_english_correction` | n/a：Go 只學修正後的詞，沒有記錄原本打錯的拼法 |
| `core/session/mixed.go`：`openMixedCandidates` 的英文段 | `crates/jc-core/src/session/mixed.rs`：`rebuild_mixed`、`Session::select_by_digit` | rewritten：混合模式接受候選時也記錄修正 |
| 弱負向訊號（Mozkey） | `crates/jc-core/src/personal/mod.rs`：`skips`、`SKIP_LIMIT`、`SKIP_PENALTY` | ported：與注音、日文共用同一張跳過表 |

## 資料結構

`Store` 新增 `english_pairs: BTreeMap<String, EnglishCorrection>`，鍵是打錯的
拼法（小寫），值是學到的修正：

| 欄位 | 意義 |
|---|---|
| `text` | 修正後的拼法，保留接受時的大小寫 |
| `accepted` | 接受次數 |
| `usage` | 使用次數，和 `accepted` 一起算排序權重 |
| `updated` | 更新時間 |

上限 `MAX_ENGLISH_PAIRS`（1024），超過時先淘汰權重最低的紀錄。
`clean_english` 會移除不合法、指向 tombstone 或大小寫鍵不一致的條目。
`forget_english` 會同時清掉以該詞為修正結果或打錯拼法的紀錄。

## 查詢入口

| API | 行為 |
|---|---|
| `Store::english_correction(wrong)` | 有學習紀錄且可自動套用時回傳修正；這是 session 決定「排第一」的入口 |
| `Store::english_candidates(prefix)` | 打錯或修正後拼法以 `prefix` 開頭的所有紀錄，依權重由高到低 |
| `Store::english_correction_target(wrong)` | 忽略跳過與長度門檻，給 session 判斷這次是否又跳過 |
| `Store::note_english_correction(wrong, right)` | 記一次接受；短字也寫入，查詢時才擋 |

## 學習何時生效、何時不生效

| 情況 | 是否改變排序 | 說明 |
|---|---|---|
| 打錯的拼法至少 7 個字母，接受修正 | 是 | 下一次同一個拼法直接以修正為第一候選 |
| 打錯的拼法短於 7 個字母 | 否 | 只寫 `english_pairs`，查詢回 `None` |
| 沒有學習紀錄 | 否 | 原字第一，內建前綴補完與拼字建議照舊 |
| 連續跳過學過的修正 1 到 2 次 | 仍排第一 | 只累計跳過次數 |
| 連續跳過 3 次 | 否 | 不再排第一，但修正仍在候選清單裡 |
| 跳過後手選一次修正 | 是 | 清掉跳過計數，下一次恢復第一 |
| 選的是前綴補完（例如 `kube → kubectl`） | 否 | 不寫入修正表，只算一般的英文輸入 |
| 修正後的詞被 `forget`／tombstone | 否 | 紀錄一併移除 |
| `SessionConfig::learning = false` | 否 | 不記錄、不查詢個人修正 |

最短觸發長度與注音一致：`MIN_TRIGGER_LEN = 7`。英文以字母數計，注音與日文
沿用 `personal::reading_len` 的非空白字元數。

## 接入點

1. `Session::rebuild_en` 與 `rebuild_mixed` 先問 `english_correction`；有值時
   把修正放在第一項，原字保留在第二項。
2. `english_candidates` 先算內建詞表的前綴補完與拼字建議，再把
   `Store::english_candidates` 的結果排在前面；同一字面只留一次。
3. `Session::learn_candidate` 的 `Mode::En` 分支呼叫 `learn_en_candidate`；
   混合模式在 `select_by_digit` 接受英文候選時呼叫 `learn_english_choice`。
4. 跳過訊號記在既有的 `Store::skips`，鍵是打錯的拼法（小寫），與注音、
   日文共用 `SKIP_LIMIT = 3` 與 `SKIP_PENALTY = 40`。

## 測試

只寫不跑，依共通規則留給整合階段。

| 需求 | 測試 |
|---|---|
| 接受 `recieve → receive` 後第一候選是 `receive` | `session::tests::test_english_learned_correction_ranks_first` |
| 跳過 3 次後不再排第一，但仍在候選中 | `session::tests::test_english_skip_demotes_learned_correction`、`personal::english::tests::english_correction_ranks_first_and_skip_demotes` |
| 手選一次恢復第一 | `session::tests::test_english_manual_choice_restores_learned_correction` |
| 5 個字母只記錄、排序不變 | `session::tests::test_english_short_correction_records_only`、`personal::english::tests::english_short_correction_records_but_does_not_rank` |
| 沒有學習紀錄時行為不變 | `session::tests::test_english_without_learning_keeps_existing_order` |

## 驗證

```sh
cargo check --workspace --all-targets
cargo run -p jc-cli --release -- eval zh-golden     # 45/45
cargo run -p jc-cli --release -- eval ja --corpus modern-500
```

實跑結果：

| 指令 | 結果 |
|---|---|
| `cargo check --workspace --all-targets` | 通過 |
| `eval zh-golden` | 45/45 |
| `eval ja --corpus modern-500` | 91.6%（neural 逾時 33 次） |
| `eval ja --corpus modern-500 --neural quality` | 93.6%（逾時 0 次） |

英文個人化只改 `Mode::En`、`Mode::Mixed` 與 `personal::english`，注音與日文
路徑沒有改動；日文差異來自本次執行的 neural 逾時，`quality` 模式回到題庫
預期的 93.6%。
