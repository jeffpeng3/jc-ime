# T10a：jc-tsf-next 的 COM DLL 外殼

Worktree：`/mnt/work/jc/wt/t10a-dll-shell`
Branch：`t10a-dll-shell`（base：`tsf-overhaul`，含 T0～T9）

把 0c 的 COM DLL 進入點與 class factory 搬到新版 TSF，讓 `jc-tsf-next` 能被
`regsvr32` 註冊成輸入法；沿用 T9 已參數化的註冊常式，不碰已發行的舊 CLSID。

## Port 對照

Go 檔案｜Rust 檔案｜狀態
---|---|---
`platforms/windows/tsf/src/lib.rs`（`DllGetClassObject`／`DllCanUnloadNow`／`DllRegisterServer`／`DllUnregisterServer`）｜`crates/jc-tsf-next/src/dll.rs`｜ported（改比對 `CLSID_JC_TSF_NEXT`，註冊走 `register_with()`）
`platforms/windows/tsf/src/factory.rs`（`ClassFactory`、`CreateInstance`、`LockServer`）｜`crates/jc-tsf-next/src/dll.rs` 的 `ClassFactory`｜ported（`Ref<'_,T>` → `Ref<T>`；`CreateInstance` 改建立 T6 的 `service::TextService`）
`platforms/windows/tsf/src/register.rs`（`Registration` 參數化）｜`crates/jc-tsf/src/register.rs`｜n/a：T9 已完成，本任務只把模組改為 `pub` 並引用
`platforms/windows/tsf/src/service.rs`（T6 的 TextService）｜`crates/jc-tsf-next/src/service.rs`｜n/a：T6 已完成，本任務未更動其 Activate guard 與 LangBar 成對邏輯

## T0 API 差異的落點

- `windows_link::link!` → `windows_core::link!`：外殼本身沒有 `link!`，既有移植在 `msctf.rs`。
- `Error::from_win32()` → `Error::from_thread()`：外殼不直接取 Win32 錯誤，註冊過程的取法在 `jc-tsf/src/register.rs::module_path()`。
- `Ref<'_, T>` → `Ref<T>`：`CreateInstance` 的 `punkouter` 用 windows 0.62 的 `Ref<IUnknown>` 簽章。

## 可見性與 crate type

- `jc-tsf` 改為 `crate-type = ["cdylib", "rlib"]`，`register` 模組改為 `pub mod register;`，讓新版 DLL 能直接呼叫 `register_with()`／`unregister_with()` 並傳入 `Registration`。
- `jc-tsf` 的四個 COM 匯出用 `dll-exports`（預設開）包住；`jc-tsf-next` 以 `default-features = false` 相依，避免兩邊的 `DllGetClassObject` 等符號進到同一份 DLL。
- `jc-tsf-next` 改為 `crate-type = ["cdylib", "rlib"]`。

## 驗證

`cargo check -p jc-tsf-next --target x86_64-pc-windows-msvc --all-targets --offline`：通過，0 error、0 warning。
未跑 test，未 commit。
