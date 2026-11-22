# T10a：jc-tsf 的 COM DLL 外殼（已正名）

Worktree：`/mnt/work/jc/wt/t10a-dll-shell`
Branch：`t10a-dll-shell`（base：`tsf-overhaul`，含 T0～T9）

> 下一次 rc 已移除舊版並把所有 `next` 後綴拿掉。本文的 `jc-tsf` 即原本的
> `jc-tsf-next`；舊 `jc-tsf` crate 已刪除。本文供移植歷史對照，現行程序見
> [`rollout.md`](rollout.md)。

把 0c 的 COM DLL 進入點與 class factory 搬到 TSF，讓 `jc-tsf` 能被
`regsvr32` 註冊成輸入法；註冊常式共用 `jc-tsf-register` 的 `Registration`。

## Port 對照

Go 檔案｜Rust 檔案｜狀態
---|---|---
`platforms/windows/tsf/src/lib.rs`（`DllGetClassObject`／`DllCanUnloadNow`／`DllRegisterServer`／`DllUnregisterServer`）｜`crates/jc-tsf/src/dll.rs`｜ported（比對 `CLSID_JC_TSF`，註冊走 `register_with(&current_registration())`）
`platforms/windows/tsf/src/factory.rs`（`ClassFactory`、`CreateInstance`、`LockServer`）｜`crates/jc-tsf/src/dll.rs` 的 `ClassFactory`｜ported（`Ref<'_,T>` → `Ref<T>`；`CreateInstance` 改建立 T6 的 `service::TextService`）
`platforms/windows/tsf/src/register.rs`（`Registration` 參數化）｜`crates/jc-tsf-register/src/register.rs`｜ported：`Registration::default()` 是現行身分，DLL 呼叫 `register()`／`unregister()`
`platforms/windows/tsf/src/service.rs`（T6 的 TextService）｜`crates/jc-tsf/src/service.rs`｜n/a：T6 已完成，本任務未更動其 Activate guard 與 LangBar 成對邏輯

## T0 API 差異的落點

- `windows_link::link!` → `windows_core::link!`：外殼本身沒有 `link!`，既有移植在 `msctf.rs`。
- `Error::from_win32()` → `Error::from_thread()`：外殼不直接取 Win32 錯誤，註冊過程的取法在 `jc-tsf-register/src/register.rs::module_path()`。
- `Ref<'_, T>` → `Ref<T>`：`CreateInstance` 的 `punkouter` 用 windows 0.62 的 `Ref<IUnknown>` 簽章。

## 可見性與 crate type

- `jc-tsf` 是 `crate-type = ["cdylib", "rlib"]`，COM 匯出只定義在這份 DLL。
- `jc-tsf-register` 只產 rlib，不放 `DllGetClassObject` 等符號，避免
  `cargo test --workspace` 鏈成同一執行檔時出現 LNK2005。
- `DllRegisterServer` 用 `current_registration()`（值取自 `guids.rs`），
  `Registration::default()` 就是同一組現行身分。

## i686 轉型修正

`candidate.rs` 的 `SetWindowLongPtrW`／`GetWindowLongPtrW` 在 windows-rs 0.62 的
32-bit target 被 alias 成 `SetWindowLongW`／`GetWindowLongW`（收／回 `i32`），
64-bit 才是 `isize`。用 `#[cfg(target_pointer_width)]` 的兩個小 helper 封裝
`set_window_userdata` 與 `window_userdata`，四處呼叫點統一走 helper，避免直接在
呼叫點 `as isize` 讓 i686 編譯失敗。

## 驗證

- `cargo check -p jc-tsf --target x86_64-pc-windows-msvc --all-targets --offline`。
- `cargo check -p jc-tsf --target i686-pc-windows-msvc --all-targets --offline`。

結果見 `/tmp/opencode/remove-old-impl-result.md`。
未跑 MSI build（Windows only）。
