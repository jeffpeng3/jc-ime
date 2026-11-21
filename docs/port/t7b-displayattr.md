# T7b 移植對照：0c Windows TSF → jc-tsf-next

來源：0c-ime main `8f443af` 的 `platforms/windows/tsf/src/`。
0c 本身是 Rust，沒有 Go 檔案；沿用 common.md 的三欄格式，第一欄改成 0c 檔案。

| 0c 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `displayattr.rs`（498 行，含 tests） | `crates/jc-tsf-next/src/displayattr.rs` | ported（生產碼 1-293 行；GUID 從 `guids.rs` 併入本檔；tests 留 T8） |
| `guids.rs` 的七個 `GUID_DISPLAY_ATTR_*` | `displayattr.rs::guids` | ported |
| `caret.rs`（276 行，含 tests） | `crates/jc-tsf-next/src/caret.rs` | ported（生產碼 1-189 行；tests 留 T8） |
| `context.rs`（87 行） | `crates/jc-tsf-next/src/context.rs` | ported（全檔） |
| `uielement.rs`（311 行，含 tests） | `crates/jc-tsf-next/src/uielement.rs` | rewritten（`/zuin_win_common::UiMessage` 改為本檔 `Candidates`，附 `From<&Snapshot>`；tests 留 T8） |
| `cuas.rs`（170 行，含 tests） | `crates/jc-tsf-next/src/cuas.rs` | ported（生產碼 1-102 行；tests 留 T8；`#![allow(dead_code)]` 等 T6 接線） |
| `edit.rs` 內嵌的最小 displayattr | `crates/jc-tsf-next/src/edit.rs` | rewritten（移除內嵌 `mod displayattr`，改 `use crate::displayattr::{...}`；`apply()` 簽章與寫入順序不變） |

共用差異（照 `docs/port/0c-api-diff.md`）：本批模組沒有 `windows_link::link!`、`Error::from_win32()` 或 `Ref<'_, T>`，0.62 的 consumer／`_Impl` 簽章與 0.61.3 相同，且 `GetSysColor` 在 0.62 收 `SYS_COLOR_INDEX`，0c 傳入的 `COLOR_*` 常數型別正好相符，無需改動。
