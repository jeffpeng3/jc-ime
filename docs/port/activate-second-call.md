# activate-second-call：第二次 `AdviseKeyEventSink` 的 `E_INVALIDARG`

## 結論

`jc-tsf` 的 `Activate`／`shutdown` 沒有問題，問題在冒煙測試把**應用程式的
TfClientId** 當成**文字服務的 TfClientId** 傳給 `ITfTextInputProcessor::Activate`。

真機 log 裡同行程兩次 `Activate：開始` 來自兩個不同的 TIP 物件：

1. `client_id=25`：msctf 在 `ITfThreadMgr::Activate()` 內部激活已安裝的
   `jc-tsf`（真實 TSF 路徑），拿到的是 TIP 自己的 client id，`AdviseKeyEventSink`
   成功。
2. `client_id=32`：冒煙測試建立另一個 `TextService`，把
   `ITfThreadMgr::Activate()` 回傳的**應用程式** client id 直接傳給它的
   `Activate`。msctf 看到 32 不屬於任何 TIP，回 `E_INVALIDARG`。

記事本兩次 `Activate` 的 `client_id` 都是 57 且都成功，因為那兩次都是 msctf
用同一個 TIP client id 激活同一個 TIP；`shutdown()` 的 `UnadviseKeyEventSink(57)`
正確清掉 sink，所以第二次能再 `AdviseKeyEventSink(57)`。這也反證 IME 的 cleanup
順序正確。

`checks.rs` 全文只有一處 `manager.Activate()`（第 192 行），`main.rs` 也只呼叫
`checks::run` 一次；log 的第二次 `Activate` 不是冒煙程式自己呼叫的，是 msctf 在
第一次 `ITfThreadMgr::Activate()` 裡觸發的。

## 文件依據

| 來源 | 關鍵內容 |
|---|---|
| [ITfThreadMgr::Activate](https://learn.microsoft.com/en-us/windows/win32/api/msctf/nf-msctf-itfthreadmgr-activate) | 「This method can be called more than once from a thread, but each call must be matched with a corresponding call to ITfThreadMgr::Deactivate」；同一 thread 重複 Activate 本身合法，所以不是「不能Activate 兩次」。 |
| [TfClientId](https://learn.microsoft.com/en-us/windows/win32/tsf/tfclientid) | 「An application obtains a TfClientId value when it calls ITfThreadMgr::Activate. The TfClientId value for a text service is passed to the ITfTextInputProcessor::Activate method.」兩種 id 來源不同。 |
| [ITfClientId::GetClientId](https://learn.microsoft.com/en-us/windows/win32/api/msctf/nf-msctf-itfclientid-getclientid) | 「enables TSF objects that do not fit either of these categories to obtain their own client identifier」；手動建立的 TIP 物件要用這個 API 取得 id。 |
| [AdviseKeyEventSink](https://learn.microsoft.com/en-us/windows/win32/api/msctf/nf-msctf-itfkeystrokemgr-advisekeyeventsink) | `E_INVALIDARG` =「One or more parameters are invalid」；`CONNECT_E_ADVISELIMIT` =「The client identified by *tid* has a key event sink installed.」 |
| [Wine msctf `threadmgr.c`](https://github.com/wine-mirror/wine/blob/master/dlls/msctf/threadmgr.c) | `AdviseKeyEventSink` 先用 `get_textservice_clsid(tid)` 查tid 所屬 TIP，查不到就回 `E_INVALIDARG`；`ActivateEx` 在回傳 application id 前會先呼叫 `activate_textservices()`。 |

## 修改

| 檔案 | 內容 |
|---|---|
| `crates/jc-tsf-smoke/src/checks.rs` | 檢查 5 改用 `ITfClientId::GetClientId(&CLSID_JC_TSF)` 取得 TIP client id，再把兩個 id 都印出來。`CONNECT_E_ADVISELIMIT` 代表 msctf 已激活同一個 TIP，判為通過，其餘錯誤照樣失敗。加 `classify_tip_activate` 與單元測試，覆蓋 S_OK／ADVISELIMIT／E_INVALIDARG。 |
| `crates/jc-tsf-smoke/Cargo.toml` | 加 `Win32_System_Ole`（`CONNECT_E_ADVISELIMIT`）。 |
| `crates/jc-tsf/src/lib.rs`、`composition.rs`、`Cargo.toml` | 只加診斷：日誌附 `GetCurrentThreadId()`、記錄 `UnadviseKeyEventSink`／`UnadviseSink` 的 HRESULT、重複 `Deactivate` 時註明沒有 control。沒有改 `Activate` 或 cleanup 的邏輯。 |

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| n/a（Go 沒有 TSF 外殼） | `crates/jc-tsf-smoke/src/checks.rs`（檢查 5、`classify_tip_activate`） | rewritten；修正 TIP client id 來源並加判定測試 |
| n/a | `crates/jc-tsf/src/lib.rs`、`composition.rs` | rewritten；只加診斷日誌 |

## 真機要跑的指令

```bat
set JC_TSF_LOG=1
jc-tsf-smoke.exe
type %LOCALAPPDATA%\jc-ime\tsf.log
```

新的檢查 5 輸出會同時列出 `應用程式 client_id` 與 `TIP client_id`。已安裝時預期看到
`ITfTextInputProcessor::Activate` 回 `CONNECT_E_ADVISELIMIT` 且檢查 5 為 `[PASS]`，
日誌的第二次 `advise` 也是 `0x80040201`；不該再出現 `0x80070057`。若仍出現
`E_INVALIDARG`，新的 `advise` 行會附 thread id，且 `shutdown` 行會顯示反註冊的
HRESULT，可以立刻分辨是 id 來源還是 cleanup 問題。

## 驗證

| 指令 | 結果 |
|---|---|
| `cargo check --workspace --all-targets` | 通過 |
| `cargo check -p jc-tsf -p jc-tsf-smoke --target x86_64-pc-windows-msvc --all-targets` | 通過 |
| `cargo clippy -p jc-tsf -p jc-tsf-smoke --target x86_64-pc-windows-msvc --all-targets -- -D warnings` | 零警告 |
| `cargo fmt --all -- --check` | 通過 |
| `cargo test`／`blaze test` | 依共通規則未執行；`tip_activate_accepts_repeated_activation` 只編譯 |
