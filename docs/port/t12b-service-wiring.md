# T12b 移植對照：0c Windows TSF → jc-tsf（候選窗 UI + CUAS 接線）

來源：0c-ime main `8f443af` 的 `platforms/windows/tsf/src/` 與 `platforms/windows/common/src/lib.rs`。
0c 本身是 Rust，沒有 Go 檔案；沿用 common.md 的三欄格式，第一欄改成 0c 檔案。

| 0c 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `common/src/lib.rs` 的 `UiMessage`（:124-165）、`UiCandidate`（:101-112）、`WM_SELECT_CANDIDATE`（:168）、`ui_pipe`（:64-66）、`NOTICE_MS`（:204）、`notice_expires`（:207） | `crates/jc-tsf/src/ui.rs` | rewritten（型別逐字照 0c；pipe 前綴 `jc-ime-ui`；T12a 合併後改 re-export `jc_ui::message`） |
| `ui.rs` 的 `UiClient`（:30-327） | `crates/jc-tsf/src/ui.rs::UiClient` | rewritten（只留 pipe 送出與 `Elements` 分流；沒有 in-process 候選窗與 spawn，等 T12a） |
| `candidate.rs`（130 行，生產碼） | `crates/jc-tsf/src/candidate.rs` | ported（回呼視窗、`WM_SELECT_CANDIDATE`、`WM_TIMER` 延後焦點確認、SelectEdit；`dll_instance()` 改 `GetModuleHandleW(None)`，前景判斷自己用 `GetGUIThreadInfo`） |
| `uielement.rs` 的 `Elements::present` | `crates/jc-tsf/src/uielement.rs` | ported（新增 `From<&UiMessage> for Candidates`；T7b 的 no-op 計數／zlog／guarded 改接 `crate::log`） |
| `service.rs` 的 `State.callback`（CandidateWindow）、`Shown.message`、present 的 UiMessage（:646-676）、`relocated` 重送（:838-879） | `crates/jc-tsf/src/service.rs` | ported |
| `service.rs` 的 `Completing`／`PendingRepost`／`Completed`（:74-116）、`State.cuas/completing/pending_repost`（:125-131） | `crates/jc-tsf/src/service.rs` | ported |
| `service.rs` 的 `complete_via_cuas`（:1053-1092）、`complete_and_repost`（:1338-1362）、`commit_before_host_key` 的 IMM32 分支（:1301-1331） | `crates/jc-tsf/src/service.rs` | ported（硬要求一、二未動） |
| `service.rs` 的 OnTestKeyDown／OnTestKeyUp／OnKeyDown 的 `pending_repost` 處理（:1670-1682、:1720-1722、:1747-1754） | `crates/jc-tsf/src/service.rs` | ported |
| `service.rs` 的 `OnCompositionTerminated` 的 `by_cuas` 判準（:1892-1925） | `crates/jc-tsf/src/service.rs` | ported |
| `service.rs` 的 Activate／Deactivate 候選窗與 Elements（:1467、:1513-1523、:1583-1584） | `crates/jc-tsf/src/service.rs` | ported |
| `imm32_order_tests.rs` 改寫過的 `State.cuas`／`pending_repost` 測試 | `crates/jc-tsf/src/imm32_order_tests.rs` | rewritten（改回原本語意：假相容層接進 `State.cuas`，`Completed`／`pending_repost` 直接斷言；`new_host` 注入，`focus_host_tests` 的 callback 改為真正的 `CandidateWindow`） |

T12a 接合點：本工作樹還沒有 `crates/jc-ui`。`ui.rs` 的型別、`ui_pipe()`、`WM_SELECT_CANDIDATE` 與 wire 格式（4-byte LE + JSON）都照 0c 形狀寫好；T12a 合併後把 `UiMessage`／`UiCandidate` 換成 `jc_ui::message` 的 re-export，並在 `UiClient::present` 補宿主行程內繪製與 `warm_up` 啟動 `jc-ui.exe`。欄位與訊息格式不用改。
