# candidate-paging：候選分頁與數字選字

分頁由核心持有，頁面大小固定 9。外殼只送 `PageUp`／`PageDown` 與數字鍵，
不再自己算頁邊界；頁面大小是核心的公開常數
`jc_core::output::CANDIDATE_PAGE_SIZE`，不佔協定欄位。

## 頁碼與頁面大小

| 項目 | 決定 |
|---|---|
| 頁面大小 | `output::CANDIDATE_PAGE_SIZE = 9`，核心與外殼共用的唯一來源 |
| 頁碼持有者 | 核心；由 `highlighted / page_size` 推導，`Session` 不另存 `page` |
| 外殼取得頁面大小 | 直接引用 `jc_core::output::CANDIDATE_PAGE_SIZE` |
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

協定版本仍是 **3**，`Response::State` 不新增分頁欄位。新增欄位必須可省略，
而 `page_size` 每次都固定 9，放上線只會讓舊 fixture 無法 round-trip；因此
分頁資訊留在程式碼常數：

- 核心用 `output::CANDIDATE_PAGE_SIZE` 分頁與算數字選字。
- wrapper 直接 import 同一個常數，不從 `State` 猜頁面大小。
- `highlighted` 仍是絕對候選索引；wrapper 若要頁碼，用
  `highlighted / CANDIDATE_PAGE_SIZE`。

## 外殼接線

`crates/jc-tsf/src/candidates.rs` 直接讀
`jc_core::output::CANDIDATE_PAGE_SIZE`：`CandidatePages::starts()` 用它算
`0, 9, 18, …`，不保留本地的 `PAGE_SIZE`，也不從 `State` 取頁面大小。
`GetCurrentPage` 由 `highlighted` 與頁首反推，預設分頁下與核心一致。

`SetPageIndex` 自訂頁首只影響 Windows 的顯示，下一次資料更新會重設回核心
的頁面大小。

## golden 相容

題庫的 `candidates-paging` 動作是照 Go 參考實作錄的，Go 版一頁 10 項。
`jc-cli` 的 golden driver 用 `GOLDEN_PAGE_SIZE = 10` 把題庫頁碼換算成候選
索引，只為維持題庫原語意；產品路徑仍然固定 9。題庫是唯讀資料，沒有改
expected。換算後 `eval zh-golden` 仍是 45/45。

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/session/selection_keys.go`（`candidatePage`、`candidateID`、`pageSize`） | `crates/jc-core/src/session/input_style.rs`、`crates/jc-core/src/session/core.rs`、`crates/jc-core/src/output/mod.rs` | rewritten：頁碼由 `highlighted` 推導，頁面大小是公開常數 9 |
| `core/session/candidates.go`（翻頁） | `crates/jc-core/src/session/core.rs`（`page_step`） | ported：頭尾相接 |
| `core/session/session.go`（`Page` 請求） | `crates/jc-core/src/session/mod.rs`（`Session::page`） | rewritten：頁面大小改讀 `PAGE_SIZE`，越界不改狀態 |
| `core/session/selection_keys.go`（數字選字） | `crates/jc-core/src/session/core.rs`（`select_by_digit`） | rewritten：加上頁位移，超出候選數時吃掉按鍵 |
| n/a（Go 沒有協定頁碼欄位） | `crates/jc-proto/src/lib.rs` | n/a：不加線上欄位，維持 fixture 可 round-trip |
| n/a（Go 沒有 TSF 外殼） | `crates/jc-tsf/src/candidates.rs` | rewritten：候選窗讀 `CANDIDATE_PAGE_SIZE` |
| n/a（Go 沒有 TSF 外殼） | `crates/jc-tsf/src/composition.rs` | rewritten：不再傳遞頁面大小欄位 |

## 測試（只寫不跑）

| 測試 | 斷言 |
|---|---|
| `test_digit_on_second_page_selects_page_offset` | 18 個候選、第 2 頁按 `1` 選第 10 個 |
| `test_digit_selection_uses_page_derived_from_highlight` | 同頁移到索引 10 後按 `2` 仍選索引 10 |
| `test_digit_beyond_candidate_count_keeps_state` | 3 個候選按 `5`，組字與高亮不變 |
| `test_page_keys_switch_pages_and_wrap` | 最後一頁再 `PageDown` 回到第 0 頁 |
| `test_candidate_page_size_can_be_nine` | 預設與小麥注音風格都是 9 |
| `test_selection_key_columns` | 原始 `1234qweras` 回 4 欄；數字設定回 5 欄 |

## 驗證

| 指令 | 結果 |
|---|---|
| `cargo check -p jc-core -p jc-proto --all-targets` | 通過 |
| `cargo check -p jc-tsf --all-targets` | 通過 |
| `cargo check -p jc-tsf --target x86_64-pc-windows-msvc --all-targets` | 通過 |
| `RUST_FONTCONFIG_DLOPEN=1 cargo test -p jc-core session::` | 相關測試全過；3 個 main 既有失敗留給 integration-fix |
| `cargo test -p jc-proto` | 通過 |
| `RUST_FONTCONFIG_DLOPEN=1 cargo check --workspace --all-targets` | 通過 |
| `cargo run -p jc-cli --release -- eval zh-golden` | 45/45 |
