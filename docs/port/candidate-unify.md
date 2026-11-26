# candidate-unify：語言無關的候選窗

候選窗只是一個通用渲染器。輸入是一份 `Candidate` 清單；語言差異留在「產生
清單」那一層，顯示、編號、換行與選取不再依語言分岔。

## 資料契約

核心送給外殼的每個候選都帶同一組欄位：

| 欄位 | 來源 | 說明 |
|---|---|---|
| 文字 | 核心 `Candidate::surface` | 候選字面，順序即顯示順序 |
| 編號 | 核心 `candidate::selection_key(index)` | 直排列號 `1`–`9`、`0`，每一個直排各一組 |
| 附註 | 核心 `Candidate::display_note()` | 目前是讀音；沒有讀音時是空字串 |
| 來源 | 核心 `CandidateSource` | `lexicon`、`personal`、`predicted`、`literal` |
| 讀音／詞性 | 核心候選資料 | 只供顯示與除錯，不改變選取規則 |

線格式由 `jc-proto` 的 `CandidateDetail.key`／`note` 攜帶。舊引擎沒有這兩個
欄位時，外殼才依位置補 `1`–`0`；不再有「英文候選不畫編號」的分支。

## 版面與數字鍵

多欄時每一欄是一條**直排**：先填滿第一欄的十列，再換下一欄。同一欄的候選
共用一組列號 `1`–`0`，不同欄可以各有一組；數字鍵只作用在**目前這一欄**。

「目前欄」的定義：以 `highlighted` 所在頁為準，頁內索引除以十就是目前欄。
`←`／`→` 換欄時會停在同一個列號，所以目前欄也跟著高亮移動。

| 狀態 | 列數 | 欄數 | `↑`／`↓` | `←`／`→` | 數字鍵 |
|---|---|---|---|---|---|
| 預設直向 | 10 | 1 | 頁內線性 ±1 | 第一次切換成四欄；單欄時不再移動 | 選該頁第 1–10 項 |
| 橫向 | 10 | 4 | 頁內線性 ±1，跨欄自然接續 | 跳一整欄（±10），停在同一列 | 選目前欄的第 1–10 列 |

- `↑`／`↓` 就是索引 `-1`／`+1`：
  - 第 1 欄第 10 列按 `↓` 到第 2 欄第 1 列。
  - 第 2 欄第 1 列按 `↑` 到第 1 欄第 10 列。
- 頁內最後一項按 `↓` 回到本頁第一項；頁內第一項按 `↑` 到本頁最後一項。
  不跨頁，換頁只由 `PageUp`／`PageDown` 負責。
- `←`／`→` 跳一整欄：索引 `-10`／`+10`。直向單欄時是 no-op；四欄時在
  本頁第一欄按 `←`、最後一欄按 `→` 也不移動。
- 一頁容量是「列數 × 欄數」：直向十項，橫向四十項。
- 頁內索引的版面位置是 `(列, 欄) = (index % 10, index / 10)`。
- 換頁只移動 `highlighted`，頁碼一律由 `highlighted / page_size` 推導。
- 第一次按 `←`／`→` 只展開成四欄並立刻重排；之後才在欄之間移動。
- 切換欄數不改變高亮索引，同一個候選仍然反白。
- 換頁維持同一個欄數；`Esc`、關窗與重新開窗會回到直向單欄。
- 四欄時每一欄都會畫自己的列號；這是列號，不是全頁唯一的流水號。
- 預設候選上限是 20（`DEFAULT_MAX_CANDIDATES`），所以四欄窗通常只排滿前
  兩欄各十列；要提高上限才會看到 40 項。容量上限不影響上述索引規則。

## 選取與 Enter

候選窗開著時，`Enter` 套用高亮候選、關窗、保留組字；再按一次才送出整段。
這適用於英文補全、字面候選、注音與日文片段。唯一例外是前導符號的變體窗：
高亮項就是目前組字已顯示的字面，套用不會換字，因此 `Enter` 直接送出整段。
這個判斷在核心用資料完成，外殼沒有語言分支。

符號／表情**選單**維持原本的一次送出：選單候選代表要立即插入的字元，按一次
`Enter` 就套用並送出，沒有第二次。這是選單與一般候選窗的明確差異。

英文補全候選和其他來源走同一條路徑；`jc-tsf` 的英文按鍵政策在候選窗開著時
不再攔 `Enter`。

## 程式位置

| 檔案 | 內容 |
|---|---|
| `crates/jc-core/src/candidate.rs` | `selection_key`、`display_note` |
| `crates/jc-core/src/candidate_layout.rs` | 列數、欄數、直排移動、頁碼與儲存格 |
| `crates/jc-core/src/output/mod.rs` | `Snapshot.rows`／`columns` |
| `crates/jc-proto/src/lib.rs` | `CandidateDetail.key`／`note`、`State.rows`／`columns` |
| `crates/jc-tsf/src/session.rs` | 線格式到外殼快照的映射，不再依語言補編號 |
| `crates/jc-tsf/src/candidate_layout.rs` | 可見頁範圍與逐列索引的純函式 |
| `crates/jc-tsf/src/service.rs` | 依核心版面切出目前頁，交給候選窗 |

## 測試

`jc-core`：

- `test_enter_in_english_suggestion_keeps_composition`
- `test_english_candidates_have_number_keys`
- `test_selection_key_unique_within_column`
- `test_candidate_rows_default_is_ten`
- `test_candidate_columns_four_after_arrow_right`
- `test_four_column_down_moves_one_row`
- `test_two_column_down_moves_one_row`
- `test_layout_switch_keeps_highlight_on_same_candidate`
- `test_digit_selects_row_in_current_column`
- `test_arrow_left_right_switches_column_same_row`
- `test_digit_in_vertical_mode_selects_item`
- `test_page_down_keeps_column_count`
- `test_down_moves_into_next_column`
- `test_up_moves_into_previous_column`
- `test_left_right_jump_ten`
- `test_down_at_page_end_wraps_to_first`
- `test_vertical_mode_left_right_is_noop`
- `test_enter_still_two_stage_when_selecting_variant`（前導變體窗選了不同字時）

`jc-tsf`：

- `page_window_follows_columns_and_highlight`：給定候選數、欄數、highlight，
  驗證可見頁範圍與每一列要畫的索引。
