# T8a：imm32 與 focus host 測試移植

移植自 0c `platforms/windows/tsf/src/`（main `8f443af`）到 `crates/jc-tsf`。
0c 的 host 測試是 Rust；表格沿用共通規則的三欄格式，第一欄為來源檔案。

| Go 檔案（0c Rust 來源） | Rust 檔案 | 狀態 |
|---|---|---|
| `imm32_order_tests.rs`（1,268 行、23 個測試） | `crates/jc-tsf/src/imm32_order_tests.rs` | rewritten：FIFO 假相容層、App 順序模型與 23 個測試名稱照搬；`edit::tests::Doc` 改成本 crate 的 `t8a_support::Doc`，命名管線改成記憶體 Transport。`pending_repost`、`Cuas::complete` 由 service 的部分由假宿主 observable 狀態取代 |
| `focus_host_tests.rs`（377 行、8 個測試） | `crates/jc-tsf/src/focus_host_tests.rs` | ported：假 DocumentMgr／Context／Range／Composition 與 8 個測試照搬；`CandidateWindow` 改成 `Option<HWND>`，命名管線改成記憶體 Transport |
| `edit.rs` 的 `#[cfg(test)] mod tests`（假文件／假 context，供上面兩支使用） | `crates/jc-tsf/src/t8a_support.rs` | rewritten：只取假文件與假 context，套用 T0 的 `Ref<'_,T>` → `Ref<T>`；額外提供不需要 named pipe 的 `Scripted` Transport |

## 假宿主改寫方式

- 0c host 測試是 `service.rs` 的子模組，直接注入私有的 `State`。新 crate 的 `State`
  與 `TextService::state` 同樣是私有的，所以 `support`／兩支測試也宣告成
  `service.rs` 末端的 `#[cfg(test)] #[path] mod`（`lib.rs` 留註解指向那裡）。
  這是必要的測試接線，沒有改動 service 的生產邏輯。
- `ZUIN_TEST_*` 環境變數仍照 0c 當成跳過條件；假引擎 `Scripted` 提供記憶體 reply，
   integrator 若設定 pipe，`open()` 仍會建立同一個假引擎 Session。
- T0 差異：`Ref<'_,T>` → `Ref<T>`；這兩支測試沒有 `Error::from_win32()` 呼叫。

## 未移植項（新 crate 尚無對應）

- `State.cuas`／`Cuas::complete` 接線：`imm32_complete_reports_by_cuas_only_when_cuas_ended_it`
  改為直接驗證假相容層是否發出 `OnCompositionTerminated`。
- `State.pending_repost`：`lost_repost`／`pending_cleared` 及相關斷言改看假相容層訊息佇列。
- `TextService::complete_via_cuas()` 與 `Completed` enum：service.rs（T6）沒有移植，已改寫。
- `crate::hotkey_engine_lock()`：假引擎是每測試獨立，不需要全域鎖。

## 驗證

- `cargo check -p jc-tsf --all-targets --offline`：通過。
- `cargo check -p jc-tsf --all-targets --offline --target x86_64-pc-windows-msvc`：通過。
- 依任務書只編譯、不執行測試，不 commit。
