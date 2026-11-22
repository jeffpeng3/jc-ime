# fix-panic-categories：TIP 按鍵 panic 與補兩個 category

修 rc.16 實機回報的兩個問題：每一鍵都在 `service.rs` 的 `RefCell` 重入 panic；
HKLM 的 TIP Category 只註冊三個，少 SYSTRAYSUPPORT 與 COMLESS。

## 對照表

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `platforms/windows/tsf/src/service.rs`：`State::styled_key`、`japanese_key`、`ctrl_punct_key`、`vocab_keys`、`simplified_hotkey`、`shift_toggles` | `crates/jc-tsf/src/service.rs` 的 `impl State` | rewritten：helper 改成 State 方法，呼叫端用單一 `borrow_mut()` 問完，不再對同一個 RefCell 重入 |
| `platforms/windows/tsf/src/service.rs`：`wants_key` | `crates/jc-tsf/src/service.rs`：`TextService_Impl::wants_key` | rewritten：原本持 `borrow_mut()` 再呼叫 TextService 上的 helper，改成就地呼叫 `st.*` |
| `platforms/windows/tsf/src/service.rs`：`shift_released` | `crates/jc-tsf/src/service.rs`：`TextService_Impl::shift_released` | rewritten：同一個 `st` 呼叫 `shift_toggles` 並用 `st.tap(side)` 清狀態 |
| `platforms/windows/tsf/src/service.rs`：`english_key_policy`、`OnKeyDown` | `crates/jc-tsf/src/service.rs`：`english_key_policy`、`OnKeyDown` | checked：三個 `styled_key` caller 本來就沒有持有跨呼叫的 borrow，改成 `borrow_mut().styled_key(...)` 的短期借用 |
| `platforms/windows/tsf/src/register.rs`：`CATEGORIES`（六項） | `installer/jc-tip-register/src/platform.rs`：`CATEGORIES` | rewritten：註冊與反註冊共用同一份五項清單，補 SYSTRAYSUPPORT、COMLESS |
| `platforms/windows/tsf/src/register.rs`：註冊與反註冊分類 | `crates/jc-tsf-register/src/register.rs`：`register_with`、`unregister_with` | rewritten：regsvr32 路徑的兩端都補上這兩個 GUID |

## panic 根因與修法

`wants_key()` 先取 `let mut st = self.state.borrow_mut()`，接著在 1294／1308 呼叫
`self.styled_key()`；port 過去的 `styled_key()` 是 `TextService` 方法，內部又
`self.state.borrow_mut()`，同一個 `RefCell` 被 mutable 借兩次直接 panic。
`shift_released()` 也一樣：持 `borrow_mut()` 呼叫 `self.shift_toggles()`，
後者內部先 `borrow_mut()` 再 `borrow()`。

0c 把這些 helper 放在 `State` 上，呼叫端對同一個 `State` 操作，天然只有一層借用。
這裡照同一結構：把 `styled_key`／`vocab_keys`／`japanese_key`／`ctrl_punct_key`／
`simplified_hotkey`／`shift_toggles` 移到 `impl State`，加 `tap()` 取代手動比對
left/right shift；原本的語意、判斷順序與測試完全不變。

## 三個 styled_key caller

- `english_key_policy`（原 1172）：呼叫前沒有活著的 borrow，改為短期
  `self.state.borrow_mut().styled_key(...)`。
- `wants_key`（原 1294、1308）：改用已借到的 `st.styled_key(...)`，不再重入。
- `OnKeyDown`（原 1811）：`vocab`／`english` 先讀成區域變數，各 helper 各自短期借用。

另外 `shift_toggles`（原 1076）的 `borrow_mut` 後接 `borrow` 一併消除：
整段移動到 `State` 內，只有單一 `&mut self`。

## 分類

windows-rs 0.62 的 `windows::Win32::UI::TextServices` 直接提供
`GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT`（`{25504FB4-7BAB-4BC1-9C69-CF81890F0EF5}`）與
`GUID_TFCAT_TIPCAP_COMLESS`（`{364215D9-75BC-11D7-A6EF-00065B84435C}`）。

- MSI 路徑：`installer/jc-tip-register/src/platform.rs` 的 `CATEGORIES` 從三項變五項；
  `machine_register_impl` 與 `machine_unregister_impl` 共用它，兩端同步。
  單元測試改成 `categories_are_five_distinct_guids`，仍逐項檢查非空、非零、不重複與完整順序。
- regsvr32 路徑：`crates/jc-tsf-register/src/register.rs` 的 `register_with` 與
  `unregister_with` 都補上兩次 `RegisterCategory`／`UnregisterCategory` 與對應日誌。
- 同步更新 `main.rs` 用法訊息與 `docs/status.md`、`docs/port/{rollout,tsf-installer,x86-tip}.md`
  裡舊的「三個分類」說法。

## 驗證

```text
cargo fmt --all -- --check                                                     通過
cargo check -p jc-tsf -p jc-tsf-register --all-targets                         通過
cargo clippy --workspace --all-targets -- -D warnings                          通過
cargo check --workspace --all-targets --exclude jc-cli --exclude jc-settings-ui --target x86_64-pc-windows-msvc  通過
cargo check -p jc-tip-register --all-targets --target x86_64-pc-windows-msvc   通過（在 installer/jc-tip-register 目錄執行）
cargo check -p jc-tip-register --all-targets --target i686-pc-windows-msvc     通過（同上）
cargo check -p jc-tsf -p jc-tsf-register --all-targets --target i686-pc-windows-msvc  通過
cargo test --workspace --no-fail-fast --exclude jc-settings-ui                1415 passed、0 failed、1 ignored
```
