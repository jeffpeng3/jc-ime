# T6：service 狀態機 + LangBar

移植自 0c `platforms/windows/tsf/src/`（main `8f443af`）到 `crates/jc-tsf`。
0c 的 TSF 本身是 Rust；表格沿用共通規則的三欄格式，第一欄為來源檔案（0c Rust）。

| Go 檔案（0c Rust 來源） | Rust 檔案 | 狀態 |
|---|---|---|
| `service.rs`（TIP 狀態機、Present、handle_key、english_key_policy、activate/deactivate、焦點三態、compartment、display attribute provider） | `crates/jc-tsf/src/service.rs` | rewritten：保留 53 欄等價的 State、焦點三態與所有 COM sink；UI／CUAS／候選窗相關的 0c 依附改成 adapter 內的精簡實作 |
| `langbar.rs`（ITfLangBarItemButton、選單、icon、tooltip） | `crates/jc-tsf/src/langbar.rs` | rewritten：選單改接 jc-settings-ui 執行檔，移除 0c-ui 選單管道 |
| `log.rs` | `crates/jc-tsf/src/log.rs` | ported：T7a 的過渡 log 改接真正的 crate 層（檔案 + OutputDebugString） |
| `caret.rs`（thread_has_caret、位置退化策略） | `crates/jc-tsf/src/caret.rs` | rewritten：T7a 的過渡插入點探測改接這裡；位置來源保留 GetTextExt／上一次位置 |
| `context.rs`（已確認前文） | `crates/jc-tsf/src/context.rs` | ported |
| `msctf.rs`（COM-less 建立 category mgr） | `crates/jc-tsf/src/msctf.rs` | rewritten：`windows_link::link!` → `windows_core::link!`（T0） |
| `guids.rs`、`displayattr.rs` | `crates/jc-tsf/src/guids.rs`、`displayattr.rs` | ported：Service 的 display attribute provider 需要七種 GUID |
| `cuas.rs`、`ui.rs`、`uielement.rs`、`candidate.rs`、`candidate_model.rs` | — | n/a：0c-ui／CUAS 相容層與候選窗不在本 crate；本階段保留 State 欄位與呼叫點，不移植其實作 |
| `imm32_order_tests.rs`、`focus_host_tests.rs`、`privacy_host_tests.rs` | crates/jc-tsf/src/privacy_host_tests.rs（T7a 已移植） | n/a：T8 另行移植 host 測試；本任務依任務書不跑 test |

## 兩條硬要求

1. 不補 Down：`handle_key()` 直接 `session.key(key)` 再 `present()`，全 crate 沒有
   `needs_selection_prefix()` 或數字前插入 `Down` 的程式碼。
2. english_pending 自動 commit：`commit_english_pending()` 同時要求
   `snapshot.english && snapshot.english_pending`；0c 的兩處 `finish("commit")` 都改走它。

## 接合

- T5：`present_in_edit()` 呼叫 `edit::apply()`，`OnLayoutChange → requery()` 呼叫
  `edit::request_locate()`（done 改 `FnOnce`）。
- T7a：`allowed()`／`keyboard_openclose_changed()` 呼叫 `privacy::field`、
  `hotkey_allowed`、`note_context`、`english_passthrough`、`imm32_context`；過渡的
  log／object_created/destroyed／guarded／thread_has_caret 改接 `crate::log` 與 `crate::caret`。
- T1a：State 持有 `Session`，Deactivate 不清它；Snapshot 直接交給 `edit::apply`。
