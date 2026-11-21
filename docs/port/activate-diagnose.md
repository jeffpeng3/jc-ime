# activate-diagnose：`Activate` 回 `0x80070057`

> 後續更新：這份日誌配置已定位出真正原因，見
> `docs/port/activate-second-call.md`。`0x80070057` 來自冒煙測試把應用程式
> client id 傳給 `ITfTextInputProcessor::Activate`，不是 `jc-tsf` 的 cleanup
> 或訊息視窗問題。以下保留原本的診斷背景。

真機跑 `jc-tsf-smoke.exe` 時，檢查 5 的 `ITfTextInputProcessor::Activate` 回
`0x80070057`（`E_INVALIDARG`）。其他六項都通過：DLL 載入、`DllGetClassObject`、
四個介面 QI 且是同一個物件、`CoCreateInstance(CLSID_TF_ThreadMgr)`、
`ITfThreadMgr::Activate` 拿到 `client_id`。

這個 task 只做兩件事：把 `Activate` 的每一步寫進日誌，讓下一次真機執行能直接指出
失敗的那一行；順手修掉一個確定錯的 `HINSTANCE` 來源。

## 移植對照

Go 參考實作沒有 TSF 外殼，這一輪沒有行為要搬。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| n/a（Go 沒有 TSF 實作） | `crates/jc-tsf/src/composition.rs`（`create_message_window`、`module_instance`、`advise`） | rewritten；只加日誌與 `HINSTANCE` 修正 |
| n/a | `crates/jc-tsf/src/lib.rs`（`Activate` 的開始／完成／失敗日誌） | rewritten |
| n/a | `crates/jc-tsf/src/logging.rs`（`hresult`） | rewritten |
| n/a | `crates/jc-tsf-smoke/src/checks.rs`（檢查 5 失敗時提示日誌位置） | rewritten |

## 日誌

成功路徑一律 `debug`（只在 `JC_TSF_LOG=1` 才落檔，平常只送
`OutputDebugStringW`），失敗路徑一律 `warn`。

| 行 | 內容 |
|---|---|
| `Activate：開始（client_id = <id>）` | 進入 `ITfTextInputProcessor::Activate` |
| `create_message_window：RegisterClassW = <值>，GetLastError = <n>` | class 註冊結果；`GetLastError` 為 1410（`ERROR_CLASS_ALREADY_EXISTS`）時視為正常 |
| `create_message_window：CreateWindowExW = <hwnd>` | 視窗建立成功 |
| `create_message_window：CreateWindowExW 失敗，GetLastError = <n>（HRESULT <hr>）` | 視窗建立失敗；`HRESULT` 就是 `HRESULT_FROM_WIN32(GetLastError)` |
| `advise：cast ITfKeystrokeMgr = <HRESULT>` | `ITfThreadMgr::cast::<ITfKeystrokeMgr>` |
| `advise：AdviseKeyEventSink = <HRESULT>（client_id = <id>）` | 按鍵 sink |
| `advise：cast ITfSource = <HRESULT>` | `ITfThreadMgr::cast::<ITfSource>` |
| `advise：cast IUnknown = <HRESULT>` | `ITfThreadMgrEventSink` → `IUnknown` |
| `advise：AdviseSink = <HRESULT>（IID_ITfThreadMgrEventSink，cookie = <n>）` | 換焦點 sink |
| `Activate：完成` / `Activate：失敗，HRESULT = <hr>：<error>` | 收尾 |

## 逐項結論

| 項目 | 結論 |
|---|---|
| `hInstance` 用 `GetModuleHandleW(None)` | **是 bug，但不會讓 class 註冊或視窗建立失敗。** 它拿到的是宿主 exe 的 handle；`RegisterClassW` 不驗證 `WNDPROC` 屬於哪個模組，而 `CreateWindowExW` 也不比對 class 的 `hInstance`，所以兩者照樣成功。真正的問題是 class 會掛在 exe 模組上（`GCLP_HMODULE`、之後的 `UnregisterClassW` 都對不上），已改成 `GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, …)`，和 `register.rs::module_path()` 用同一個錨點技巧。 |
| `WINDOW_STYLE(0)` 配 `HWND_MESSAGE` | **不是原因。** `WINDOW_STYLE(0)` 等於 `WS_OVERLAPPED`，是合法樣式；message-only 視窗只需要 `hwndParent = HWND_MESSAGE`。會讓 `CreateWindowExW` 直接失敗的是 `WS_CHILD`（要求真正的父視窗），程式沒有用。維持 0，另外在註解寫明。 |
| `AdviseKeyEventSink` 的 `client_id` | **型別與來源都正確。** `TfClientId` 就是 `u32`，值是 `ITfThreadMgr::Activate` 的回傳值（真機輸出 `client_id=1`），與 Microsoft 範例 `AdviseKeyEventSink(_tfClientId, this, TRUE)` 一致。文件列的錯誤碼是 `E_INVALIDARG`／`CONNECT_E_ADVISELIMIT`，後者代表同一個 `tid` 已有 sink，與首跑不符。 |
| `AdviseSink` 的 `riid` | **已經正確。** 傳的是 `<ITfThreadMgrEventSink as Interface>::IID`，就是 `IID_ITfThreadMgrEventSink`；`ITfSource` 從 `ITfThreadMgr` 取得時，這是文件明列的合法值。 |

## 根因判斷

`0x80070057` 只可能來自三處，其餘呼叫的失敗碼是 `E_NOINTERFACE`：

1. `GetModuleHandleW(None)`（實務上不會失敗），
2. `CreateWindowExW`（`HRESULT_FROM_WIN32(GetLastError)`），
3. `AdviseKeyEventSink` 或 `AdviseSink` 直接把 `E_INVALIDARG` 回給我們。

沒有實機沒辦法再縮小；上面三處現在各有一行日誌，下一次真機跑就能定案。已知
`client_id=1`、兩個 QI 都成功（否則錯誤會是 `0x80004002`），所以不需要再從這兩
邊猜。

## 真機要跑的指令

```bat
set JC_TSF_LOG=1
jc-tsf-smoke.exe
type %LOCALAPPDATA%\jc-ime\tsf.log
```

看最後一個 `[jc-tsf … warn]` 行：`create_message_window：…` 代表是 Win32 視窗
建立，`advise：…` 代表是 msctf 的 sink 註冊。

## 驗證

| 指令 | 結果 |
|---|---|
| `cargo check --workspace --all-targets` | 通過 |
| `cargo check -p jc-tsf -p jc-tsf-smoke --all-targets --target x86_64-pc-windows-msvc` | 通過 |
| `cargo check -p jc-tsf --features test-export --target x86_64-pc-windows-msvc` | 通過 |
| `cargo clippy -p jc-tsf -p jc-tsf-smoke --all-targets --target x86_64-pc-windows-msvc` | 零警告 |
| `cargo fmt -p jc-tsf -p jc-tsf-smoke` | 已執行 |
