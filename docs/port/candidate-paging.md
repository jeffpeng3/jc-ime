# candidate-paging：候選分頁與數字選字

分頁由核心持有，預設直向頁面大小固定 10。外殼只送 `PageUp`／`PageDown` 與數字鍵，
不再自己算頁邊界；頁面大小由 `candidate_layout::CandidateLayout` 決定，協定只送
選填的列數與欄數。

## 頁碼與頁面大小

| 項目 | 決定 |
|---|---|
| 頁面大小 | 直向 `10`、橫向四欄 `10 × 4 = 40`，由 `candidate_layout::CandidateLayout` 統一計算 |
| 頁碼持有者 | 核心；由 `highlighted / page_size` 推導，`Session` 不另存 `page` |
| 外殼取得頁面大小 | 核心送 `State.rows`／`columns`；舊引擎由外殼用 10 與 1 補預設 |
| 預設候選數 | `DEFAULT_MAX_CANDIDATES = 列數 × 四欄欄數 × 2 = 80`（兩頁四欄版面） |

`Session::current_page()` 是唯一算頁碼的地方。翻頁、方向鍵、Tab、Home／End
與 `Session::select` 都只改 `highlighted`，頁碼自然跟著變，不會出現「高亮在
第 2 頁、頁碼還停在第 1 頁」的兩套狀態。

`InputStyle.rules.candidate_page_size` 只為舊設定檔保留，讀取與驗證照舊，
但不再影響分頁；`Session::page_size()` 依目前欄數回傳 10 或 40；`selection_keys()` 仍以預設十列為準。

## 邊界行為

| 操作 | 行為 |
|---|---|
| `↓` 在頁尾 | 到下一頁第 1 項（線性 +1，跨頁） |
| `↑` 在頁首 | 到上一頁最後一項（線性 -1，跨頁） |
| `↓` 在整份清單最後一項 | 停在原地，不回捲 |
| `↑` 在整份清單第一項 | 停在原地，不回捲 |
| `PageDown` 在最後一頁 | 回到第 0 頁（頭尾相接） |
| `PageUp` 在第 0 頁 | 到最後一頁（頭尾相接） |
| 候選窗還沒開時按 `PageUp`／`PageDown` | 先開窗並進入選取模式，高亮不動 |
| `Session::page(n)` 超出頁數 | 不改變狀態 |
| 數字鍵選到不存在的候選 | 吃掉按鍵，組字、候選、高亮、`navigating` 都不變 |
| 候選不足一頁 | 超出候選數的數字鍵忽略，不會退回注音鍵 |

數字鍵是「目前直排的列號」。頁首由 `current_page() * page_size` 取得，目前欄
由 `highlighted` 換算，絕對索引是
`頁首 + 目前欄 × 10 + 列`；`selection_index` 只負責把按鍵換成列 0–9。
直向只有一欄，所以 `1`–`0` 就是該頁第 1–10 項；四欄時只選目前這一欄。

## 協定

協定版本仍是 **3**，`Response::State` 只新增選填的 `rows`／`columns`，不送頁碼。
新增欄位可省略，舊 fixture 仍能 round-trip；頁碼由外殼用列數與欄數推導：

- 核心用目前 `CandidateLayout::page_size()` 分頁與算數字選字。
- wrapper 用 `State.rows`／`columns` 算頁面大小；舊引擎補 10 與 1。
- `highlighted` 仍是絕對候選索引；wrapper 若要頁碼，用
  `highlighted / (rows × columns)`。

## 外殼接線

`crates/jc-tsf/src/candidate_layout.rs` 不保留本地頁面大小：`CandidatePages::starts()`
用 `State.rows × State.columns` 算頁首。
`GetCurrentPage` 由 `highlighted` 與頁首反推，預設十列下與核心一致。

`SetPageIndex` 自訂頁首只影響 Windows 的顯示，下一次資料更新會重設回核心
的頁面大小。

## golden 相容

題庫的 `candidates-paging` 動作是照 Go 參考實作錄的，Go 版一頁 10 項。
`jc-cli` 的 golden driver 用 `GOLDEN_PAGE_SIZE = 10` 把題庫頁碼換算成候選
索引，只為維持題庫原語意；產品直向也是 10，兩邊一致。題庫是唯讀資料，沒有改
expected。換算後 `eval zh-golden` 仍是 45/45。

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/session/selection_keys.go`（`candidatePage`、`candidateID`、`pageSize`） | `crates/jc-core/src/session/input_style.rs`、`crates/jc-core/src/session/core.rs`、`crates/jc-core/src/output/mod.rs` | rewritten：頁碼由 `highlighted` 推導，頁面大小由版面列數 × 欄數決定 |
| `core/session/candidates.go`（翻頁） | `crates/jc-core/src/session/core.rs`（`page_step`） | ported：頭尾相接 |
| `core/session/session.go`（`Page` 請求） | `crates/jc-core/src/session/mod.rs`（`Session::page`） | rewritten：頁面大小改讀目前版面，越界不改狀態 |
| `core/session/selection_keys.go`（數字選字） | `crates/jc-core/src/session/core.rs`（`select_by_digit`） | rewritten：加上頁位移，超出候選數時吃掉按鍵 |
| n/a（Go 沒有協定頁碼欄位） | `crates/jc-proto/src/lib.rs` | n/a：不加線上欄位，維持 fixture 可 round-trip |
| n/a（Go 沒有 TSF 外殼） | `crates/jc-tsf/src/candidate_layout.rs` | rewritten：候選窗依核心列數／欄數切頁 |
| n/a（Go 沒有 TSF 外殼） | `crates/jc-tsf/src/composition.rs` | rewritten：不再傳遞頁面大小欄位 |

## 測試（只寫不跑）

| 測試 | 斷言 |
|---|---|
| `test_digit_on_second_page_selects_page_offset` | 19 個候選、第 2 頁按 `1` 選第 11 個 |
| `test_digit_selection_uses_page_derived_from_highlight` | 同頁移到索引 11 後按 `2` 仍選索引 11 |
| `test_digit_beyond_candidate_count_keeps_state` | 3 個候選按 `5`，組字與高亮不變 |
| `test_page_keys_switch_pages_and_wrap` | 最後一頁再 `PageDown` 回到第 0 頁 |
| `test_candidate_page_size_can_be_ten` | 預設與小麥注音風格都是 10 |
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
