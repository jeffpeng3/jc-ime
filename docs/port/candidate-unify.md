# candidate-unify：語言無關的候選窗

候選窗只是一個通用渲染器。輸入是一份 `Candidate` 清單；語言差異留在「產生
清單」那一層，顯示、編號、換行與選取不再依語言分岔。

## 資料契約

核心送給外殼的每個候選都帶同一組欄位：

| 欄位 | 來源 | 說明 |
|---|---|---|
| 文字 | 核心 `Candidate::surface` | 候選字面，順序即顯示順序 |
| 編號 | 核心 `candidate::selection_key(index)` | 頁內位置 `1`–`9`、`0`，所有來源共用 |
| 附註 | 核心 `Candidate::display_note()` | 目前是讀音；沒有讀音時是空字串 |
| 來源 | 核心 `CandidateSource` | `lexicon`、`personal`、`predicted`、`literal` |
| 讀音／詞性 | 核心候選資料 | 只供顯示與除錯，不改變選取規則 |

線格式由 `jc-proto` 的 `CandidateDetail.key`／`note` 攜帶。舊引擎沒有這兩個
欄位時，外殼才依位置補 `1`–`0`；不再有「英文候選不畫編號」的分支。

## 版面規則

| 狀態 | 列數 | 欄數 | `↑`／`↓` | `←`／`→` |
|---|---|---|---|---|
| 預設直向 | 10 | 1 | 一次一項 | 第一次切換成四欄 |
| 橫向 | 10 | 4 | 一次一列 | 一次一格 |

- 一頁容量是「列數 × 欄數」：直向十項，橫向四十項。
- 換頁只移動 `highlighted`，頁碼一律由 `highlighted / page_size` 推導。
- 第一次按 `←`／`→` 只展開成四欄並立刻重排；之後才在候選間移動。
- 切換欄數不改變高亮索引，同一個候選仍然反白。
- `Esc`、關窗與重新開窗會回到直向單欄。

## 選取與 Enter

候選窗開著時，`Enter` 一律套用高亮候選、關窗、保留組字；沒有候選窗時才送出
整段。唯一例外是前導符號的變體窗：高亮項就是目前組字已顯示的字面，套用不會
換字，因此 `Enter` 直接送出整段。這個判斷在核心用資料完成，外殼沒有語言分支。

英文補全候選和其他來源走同一條路徑；`jc-tsf` 的英文按鍵政策在候選窗開著時
不再攔 `Enter`。

## 程式位置

| 檔案 | 內容 |
|---|---|
| `crates/jc-core/src/candidate.rs` | `selection_key`、`display_note` |
| `crates/jc-core/src/candidate_layout.rs` | 列數、欄數、一列步進、頁碼與儲存格 |
| `crates/jc-core/src/output/mod.rs` | `Snapshot.rows`／`columns` |
| `crates/jc-proto/src/lib.rs` | `CandidateDetail.key`／`note`、`State.rows`／`columns` |
| `crates/jc-tsf/src/session.rs` | 線格式到外殼快照的映射，不再依語言補編號 |
| `crates/jc-tsf/src/candidate_layout.rs` | 可見頁範圍與逐列索引的純函式 |
| `crates/jc-tsf/src/service.rs` | 依核心版面切出目前頁，交給候選窗 |

## 測試

`jc-core`：

- `test_enter_in_english_suggestion_keeps_composition`
- `test_english_candidates_have_number_keys`
- `test_candidate_rows_default_is_ten`
- `test_candidate_columns_four_after_arrow_right`
- `test_four_column_down_moves_one_row`
- `test_two_column_down_moves_one_row`
- `test_layout_switch_keeps_highlight_on_same_candidate`

`jc-tsf`：

- `page_window_follows_columns_and_highlight`：給定候選數、欄數、highlight，
  驗證可見頁範圍與每一列要畫的索引。
