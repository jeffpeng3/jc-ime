# 引擎行程：監督、按需啟動與 pipe

引擎是獨立行程，不是 TSF 的子行程；崩潰或結束後會自動回來，每個使用者最多一個
監督者。不做 Windows 服務：IME 是使用者層級元件，服務跑在 session 0，碰不到
使用者的 pipe 與輸入焦點。

## 監督者

`jc-engine --supervise` 啟動父行程與子行程：

- 父行程持有 `Local\jc-ime-engine-supervisor` 單一實例鎖；第二個監督者拿到
  `ERROR_SHARING_VIOLATION` 後以 exit 0 結束。
- 子行程被結束或崩潰後，父行程以退避重啟。
- log 由 `LogRing` 寫進 `%LOCALAPPDATA%\jc-ime\supervisor.log`，不依賴 console。
- `--instance` 預設 `main`；個人資料前綴 `--personal-prefix` 預設 `personal`，
  存成 `personal.json`。

## 登入自動啟動

`jc-engine.exe` 是 console subsystem，直接由 HKLM `Run` 啟動會出現終端機視窗。
安裝器寫入的 Run 值改用無視窗的 `jc-engine-launcher.exe`：

```text
"<安裝目錄>\jc-engine-launcher.exe" --supervise --data "<ProgramData>\jc-ime\data" --settings "%LOCALAPPDATA%\jc-ime\settings.json" --personal-dir "%LOCALAPPDATA%\jc-ime" --personal-prefix personal
```

- `jc-engine-launcher.exe` 自己是 `windows` subsystem，以 `CREATE_NO_WINDOW`
  啟動 `jc-engine.exe`，並把啟動器收到的 argv 原樣轉交。
- `--personal-dir` 必須指向使用者目錄；資料根目錄在 `ProgramData`，引擎寫入
  個人資料會失敗。
- **不要加 `--socket`**：引擎與 TIP 都呼叫 `jc_windows::resolve_pipe_name()`，
  用目前使用者的 SID 尾段推出同一條 `jc-ime-<rid>`。登入時另外指定 `--socket`
  會讓兩邊名稱不一致，TIP 就連不上。`--socket` 只留給手動除錯或 Unix socket。
- `--supervise` 不能省；直接伺服模式沒給 `--socket` 會退回 stdio。

安裝完成後的即時啟動也走同一個 launcher，參數相同。

## 按需啟動

`Run` 只在登入時觸發一次；若使用者把 supervisor 與子行程都 kill 掉，TIP 在連不上
pipe 時會自行啟動引擎：

| 事件 | 動作 |
|---|---|
| `OnSetFocus` | `FocusPipe` 啟動 worker，worker 呼叫 `Client::connect()` |
| connect 得到 `NotFound`／`PipeBusy` | spawn 引擎，等 pipe 出現後再連一次 |
| 其他錯誤 | 只記日誌，不 spawn |

實作在 `crates/jc-tsf/src/ipc.rs`（`spawn_engine`、`spawn_server`、`connect`、
`may_spawn`）與 `crates/jc-tsf/src/ipc_transport.rs`（`engine_command`）。

- TIP 直接啟動 `<安裝目錄>\jc-engine.exe`，只帶 `--supervise` 與 `--data`。
  pipe 名稱由 `resolve_pipe_name()` 決定，不自己算 `--socket`；個人資料目錄用
  引擎預設的 `%LOCALAPPDATA%\jc-ime`。
- 用 `Command::args([...])` 逐項傳 argv，不把整串命令列交給
  `CreateProcessW::lpCommandLine`；`\"` 與末尾 `\` 的解析會讓 `--data` 吃到後面的
  參數。
- 用 `CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP`，不用 `DETACHED_PROCESS`：
  後者仍會替 console 子系統配置新視窗。引擎維持 console subsystem，管理員手動
  執行時仍可看 stdout。
- `may_spawn` 有速率限制；spawn 後以有界重試等待 pipe。跨行程重複由 supervisor
  lockfile 擋掉。spawn 失敗只記警告，按鍵照常排隊。
- 沙箱應用程式裡的 DLL 不能啟動程式，只能連上已在執行的引擎。

## pipe 與連線

- Windows 用 named pipe，名稱由 `jc_windows::resolve_pipe_name()` 決定，預設
  `jc-ime-<SID 尾段>`；`JC_IME_PIPE` 或
  `HKCU\Software\jc-ime\EnginePipe` 可覆寫。
- Unix 用 Unix domain socket；手動啟動時可用 `--socket` 指定路徑。
- 訊框是 4-byte 小端長度前綴加 JSON，預設上限 1 MiB。
- 每條連線一個 worker thread，可並行。
- `FocusPipe` 用 focus 事件管理連線，斷線後有 120 ms 寬限與重連退避。

## 檔案位置

| 項目 | 位置 |
|---|---|
| 監督與重啟 | `crates/jc-engine/src/main.rs`（`run_supervisor`、`child_args`） |
| 單一實例與 log | `crates/jc-engine/src/supervise.rs` |
| 登入用無視窗啟動器 | `crates/jc-engine-launcher/src/main.rs`、`lib.rs` |
| TIP 連線與按需 spawn | `crates/jc-tsf/src/ipc.rs`（`spawn_engine`、`spawn_server`、`connect`） |
| TIP 的引擎命令 | `crates/jc-tsf/src/ipc_transport.rs`（`engine_command`） |
| pipe 名稱 | `crates/jc-windows/src/pipe_name.rs`、`crates/jc-tsf/src/ipc.rs` |
