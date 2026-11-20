# candidate-click：滑鼠點選直接送數字鍵

候選窗的滑鼠點選原本是「模擬上／下鍵移動高亮，再送 `Space` 確認」：點第 7 項
要送 6 次 `Down` 加一次 `Space`，每一鍵都是一趟 pipe 往返。現在改成和鍵盤
`1`–`0` 選字完全同一條路徑：**點第 `n` 項只送一個數字鍵**。

## 換算規則

`SetSelection` 收到的是候選清單的絕對索引；先取頁內位置，再換成數字鍵。

| 步驟 | 公式 |
|---|---|
| 頁面大小 | `jc_core::output::CANDIDATE_PAGE_SIZE`（現值 9，外殼沒有寫死） |
| 頁內位置 | `nIndex % CANDIDATE_PAGE_SIZE` |
| 數字鍵 | 頁內位置 + 1，即第 1–9 項 → `1`–`9`；頁面 10 項時第 10 項 → `0` |

| 頁內索引 | 0 | 1 | … | 8 | 9（僅 10 項頁面） |
|---|---|---|---|---|---|
| 送出的鍵 | `1` | `2` | … | `9` | `0` |

換算邏輯是 `crates/jc-tsf/src/candidates.rs` 的 `digit_for`／`selection_key`，
頁面大小改動時不必改外殼程式碼，只要核心常數變 10，第 10 項自動變成 `0`。

## 點選流程

```text
Windows 候選窗（滑鼠單擊）
  └─ QI(ITfCandidateListUIElementBehavior)
       ├─ SetSelection(nIndex)
       │    ├─ nIndex >= GetCount() ────────────→ 記 log，忽略
       │    ├─ 引擎未連線 ─────────────────────→ 記 log，忽略
       │    ├─ nIndex 不在核心目前頁 ──────────→ 記 log，忽略
       │    ├─ 數字鍵 = digit_for(nIndex % PAGE) ┘
       │    ├─ settled = true
       │    └─ send_keys([數字鍵])   ← 核心選取並提交
       ├─ Finalize()
       │    ├─ settled → 略過（數字鍵已提交，不能再送 Space）
       │    └─ 未 settled → 送 Space（只呼叫 Finalize 的應用程式）
       └─ Abort()
            ├─ settled → 略過
            └─ 送 Escape
```

「不在核心目前頁」的檢查是必要的：數字鍵由核心換算成
`current_page * PAGE_SIZE + 頁內位置`，而 `current_page` 來自 `highlighted`。
點到別頁的項目卻只送數字鍵，核心會選到**目前頁的同位置候選**，等於提交錯字。
這種點選直接忽略並記 log，等下一輪 `State` 更新後再點。

## 與核心數字選字的對應關係

| 項目 | 鍵盤 | 滑鼠點選 |
|---|---|---|
| 觸發 | `1`–`9`（`candidate_page_size = 10` 時含 `0`） | `SetSelection(nIndex)` |
| 頁碼來源 | `highlighted / CANDIDATE_PAGE_SIZE` | 同左（只接受同頁索引） |
| 頁內位置 | 按鍵字元查 `selection_index` | `nIndex % CANDIDATE_PAGE_SIZE` |
| 選取索引 | `current_page * PAGE_SIZE + 頁內位置` | 同左 |
| 結果 | `select_by_digit` 選取並提交 | 同左 |

兩邊都走 `jc_core::session` 的 `select_by_digit`，wrapper 沒有自己的選字規則；
需要提交什麼字仍然只由引擎決定。因為數字鍵本身就代表「選取＋提交」，滑鼠點選
不再有「只移動高亮」的語意，也不再需要模擬方向鍵。

滑鼠與鍵盤共用同一個 `mpsc` 通道與 `Control::generation`：點選後立刻打字時，
過期的 `State` 照既有規則被丟棄，`Committed` 一律套用。

## 邊界行為

| 情況 | 行為 |
|---|---|
| `nIndex` 超出候選數 | 記 debug log，回 `S_OK`，不送鍵 |
| 索引落在核心目前頁之外 | 記 debug log，回 `S_OK`，不送鍵 |
| 引擎未連線 | 記 debug log，不排隊 |
| 候選窗停用（`actions` 為 `None`） | 直接回 `S_OK` |
| 單擊後 Windows 再呼叫 `Finalize` | `settled` 已設，不再補 `Space` |
| `Abort` 在點選之後 | 不送 `Escape`（候選已由數字鍵收掉） |
| 頁面大小改成 10 | 第 10 項送 `0`，其餘不變 |

## 移植對照

Go 參考實作沒有 TSF 外殼，點選流程是平台層新程式碼；核心的數字選字是既有功能，
直接沿用。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/session/selection_keys.go`（數字選字、頁碼） | `crates/jc-core/src/session/core.rs`（`select_by_digit`）、`crates/jc-core/src/output/mod.rs`（`CANDIDATE_PAGE_SIZE`） | 直接沿用，未修改 |
| n/a（Go 沒有 TSF 外殼） | `crates/jc-tsf/src/candidates.rs`（`SetSelection` 改送數字鍵、移除 `navigation_keys`／`requested`） | rewritten |
| n/a | `crates/jc-tsf/src/composition.rs`（`CandidateSnapshot` 不再帶 `navigating`） | rewritten |
| n/a | `crates/jc-tsf/src/keys.rs` | n/a：數字鍵用 `Key::char`，不需要新的按鍵轉換 |
| n/a | `docs/port/tsf-polish.md`（點選章節與未驗證清單） | rewritten |

### owned paths 的偏差

任務書列的 owned paths 只有 `crates/jc-tsf/src/composition.rs` 與
`crates/jc-tsf/src/keys.rs`；但 `ITfCandidateListUIElementBehavior` 的
`SetSelection`／`Finalize`／`Abort` 實作實際位在 `crates/jc-tsf/src/candidates.rs`
（任務書第 4 點要求「保留實作、內部改成送數字鍵」，指的就是這個檔案）。因此
這個 task 一併修改 `candidates.rs`；這是同一個 crate、沒有其他 task 認領，
`jc-core`／`jc-proto` 都沒有動。

## 測試（只寫不跑）

| 測試 | 斷言 |
|---|---|
| `click_sends_one_digit_key` | 頁內 0–8 依序送 `1`–`9`，且 `CANDIDATE_PAGE_SIZE == 9` |
| `digit_keys_look_like_keyboard_digits` | 第 3 項是單一 `KeyCode::Char('3')`，沒有 Shift／Ctrl |
| `click_index_is_page_relative` | 索引 9／10 送 `1`／`2` |
| `tenth_item_maps_to_zero` | `digit_for(9) == '0'`、`digit_for(8) == '9'`、`digit_for(10) == None` |
| `confirm_and_abort_use_engine_keys` | `Finalize` 仍是 `Space`、`Abort` 仍是 `Escape` |

## 驗證

| 指令 | 結果 |
|---|---|
| `RUST_FONTCONFIG_DLOPEN=1 cargo check --workspace --all-targets` | 通過 |
| `cargo check --target x86_64-pc-windows-msvc -p jc-tsf --all-targets` | 通過 |
| `cargo clippy -p jc-tsf --target x86_64-pc-windows-msvc --all-targets --no-deps -- -D warnings` | 零警告 |
| `cargo fmt -p jc-tsf` | 已執行 |

依 `common.md`，測試只編譯不執行。

## 需要實機驗證

1. Windows 內建候選窗單擊時是否為 `SetSelection` → `Finalize`；若是，數字鍵
   已提交，`Finalize` 被 `settled` 擋掉，不會多一個空白。
2. 是否有應用程式用 `SetSelection` 做「只移動高亮」的鍵盤導覽；那種用法現在
   會變成提前提交。
3. 候選窗顯示中但核心 `candidate_window` 為 false 的狀態（例如注音還沒打完）
   是否會出現；若會，數字鍵會退回注音鍵，點選等於插入一個數字。
4. 跨頁索引是否真的會傳進來；目前一律忽略並記 log。
5. `Abort` 的 `Escape` 是否會連未完成的注音一起清掉。
6. `GetSelection` 在 `State` 回來前仍回舊值時，選取框的畫面行為。
