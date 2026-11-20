# candidate-paging：候選分頁與數字選字

分頁由核心持有，頁面大小固定 9。外殼只送 `PageUp`／`PageDown` 與數字鍵，
不再自己算頁邊界；`State` 直接回報核心認定的 `page` 與 `page_size`。

## 頁碼與頁面大小

| 項目 | 決定 |
|---|---|
| 頁面大小 | `candidate::PAGE_SIZE = 9`，全核心唯一來源 |
| 頁碼持有者 | 核心；由 `highlighted / page_size` 推導，`Session` 不另存 `page` |
| 外殼看到的頁碼 | `Snapshot.page`，線格式是 `State.page` |
| 外殼看到的頁面大小 | `Snapshot.page_size`，線格式是 `State.page_size`，永遠是 9 |
| 預設候選數 | `DEFAULT_MAX_CANDIDATES = PAGE_SIZE * 2 = 18` |

`Session::current_page()` 是唯一算頁碼的地方。翻頁、方向鍵、Tab、Home／End
與 `Session::select` 都只改 `highlighted`，頁碼自然跟著變，不會出現「高亮在
第 2 頁、頁碼還停在第 1 頁」的兩套狀態。

`InputStyle.rules.candidate_page_size` 只為舊設定檔保留，讀取與驗證照舊，
但不再影響分頁；`Session::page_size()` 一律回傳 9。

## 邊界行為

| 操作 | 行為 |
|---|---|
| `PageDown` 在最後一頁 | 回到第 0 頁（頭尾相接） |
| `PageUp` 在第 0 頁 | 到最後一頁（頭尾相接） |
| 候選窗還沒開時按 `PageUp`／`PageDown` | 先開窗並進入選取模式，高亮不動 |
| `Session::page(n)` 超出頁數 | 不改變狀態 |
| 數字鍵選到不存在的候選 | 吃掉按鍵，組字、候選、高亮、`navigating` 都不變 |
| 候選不足一頁 | 超出候選數的數字鍵忽略，不會退回注音鍵 |

數字鍵的絕對索引是 `current_page() * PAGE_SIZE + 頁內位置`。頁內位置由
`selection_index` 決定：預設 `1`–`9` 對應 0–8；舊的 10 鍵設定只取前 9 個，
`0` 在 9 項頁面上不是選字鍵。

## 協定

協定版本仍是 **3**。`Response::State` 新增兩個可忽略欄位，舊 wrapper 讀到
的 JSON 形狀只多不減：

| 欄位 | 序列化 | 舊 wrapper 反序列化 |
|---|---|---|
| `page` | 0 時省略 | 預設 0 |
| `page_size` | 一律輸出 9 | 缺欄位時預設 9 |

wrapper 應直接使用 `page` 與 `page_size`。`highlighted` 仍是絕對候選索引；
`page` 只是方便畫頁，不能拿 `highlighted / 9` 在 wrapper 重算。

## 外殼接線狀態

這個 branch 的 `crates/jc-tsf` 只有組字骨架，沒有候選窗。候選窗在
`tsf-candidates` branch 的 `crates/jc-tsf/src/candidates.rs`，那裡目前把
`PAGE_SIZE` 寫死成 9，並由 `highlighted` 反推頁碼。合併後要改成讀
`State.page`／`State.page_size`，刪掉本地的頁面常數；本 branch 沒有那個
檔案，不能先改別人的 worktree。

## golden 相容

題庫的 `candidates-paging` 動作是照 Go 參考實作錄的，Go 版一頁 10 項。
`jc-cli` 的 golden driver 用 `GOLDEN_PAGE_SIZE = 10` 把題庫頁碼換算成候選
索引，只為維持題庫原語意；產品路徑仍然固定 9。題庫是唯讀資料，沒有改
expected。換算後 `eval zh-golden` 仍是 45/45。

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/session/selection_keys.go`（`candidatePage`、`candidateID`、`pageSize`） | `crates/jc-core/src/session/input_style.rs`、`crates/jc-core/src/session/core.rs`、`crates/jc-core/src/candidate.rs` | rewritten：頁碼由 `highlighted` 推導，頁面大小固定 9 |
| `core/session/candidates.go`（翻頁） | `crates/jc-core/src/session/core.rs`（`page_step`） | ported：頭尾相接 |
| `core/session/session.go`（`Page` 請求） | `crates/jc-core/src/session/mod.rs`（`Session::page`） | rewritten：頁面大小改讀 `PAGE_SIZE`，越界不改狀態 |
| `core/session/selection_keys.go`（數字選字） | `crates/jc-core/src/session/core.rs`（`select_by_digit`） | rewritten：加上頁位移，超出候選數時吃掉按鍵 |
| n/a（Go 沒有協定頁碼欄位） | `crates/jc-proto/src/lib.rs`（`State.page`、`State.page_size`） | rewritten：v3 可忽略欄位 |
| n/a（Go 沒有 TSF 外殼） | `crates/jc-tsf/src/candidates.rs`（另一個 branch） | 待整合：改用 `State.page`／`State.page_size` |

## 測試（只寫不跑）

| 測試 | 斷言 |
|---|---|
| `test_digit_on_second_page_selects_page_offset` | 18 個候選、第 2 頁按 `1` 選第 10 個 |
| `test_digit_selection_uses_page_derived_from_highlight` | 同頁移到索引 10 後按 `2` 仍選索引 10 |
| `test_digit_beyond_candidate_count_keeps_state` | 3 個候選按 `5`，組字與高亮不變 |
| `test_page_keys_switch_pages_and_wrap` | 最後一頁再 `PageDown` 回到第 0 頁 |
| `test_candidate_page_size_can_be_nine` | 預設與小麥注音風格都是 9 |
| `state_carries_core_page_and_page_size` | `State.page`／`page_size` 與快照一致 |
| `state_accepts_version_two_shape_with_defaults` | 舊 JSON 缺 `page_size` 時得到 9 |

## 驗證

| 指令 | 結果 |
|---|---|
| `cargo check -p jc-core --all-targets` | 通過 |
| `cargo check -p jc-proto --all-targets` | 通過 |
| `cargo check -p jc-proto -p jc-engine -p jc-cli -p jc-windows -p jc-tsf -p jc-tsf-smoke -p jc-settings --all-targets` | 通過 |
| `cargo check --workspace --all-targets` | 未完成：`jc-settings-ui` 的 `yeslogic-fontconfig-sys` 找不到系統 `fontconfig.pc`，與本 task 無關 |
| `cargo run -p jc-cli --release -- eval zh-golden` | 45/45 |
