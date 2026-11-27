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

登入自動啟動由安裝器寫入 HKLM `Run`，指令形狀是：

```text
jc-engine --supervise --data <資料根目錄> --settings <LOCALAPPDATA>\jc-ime\settings.json --socket jc-ime-<SID 尾段>
```

`--socket` 讓引擎與 TIP 用同一條 pipe；`JC_IME_PIPE` 可覆寫，但 TIP 跑在宿主
行程裡，逐 TIP 設環境變數不可靠，所以安裝器用 `--socket`。

## 按需啟動

`Run` 只在登入時觸發一次；若使用者把 supervisor 與子行程都 kill 掉，TIP 在連不上
pipe 時會 detached spawn 引擎：

| 事件 | 動作 |
|---|---|
| `OnSetFocus` | `FocusPipe` 啟動 worker，worker 呼叫 `engine::connect()` |
| connect 得到 `NotFound`／`PipeBusy` | detached spawn，等 pipe 出現後再連一次 |
| 其他錯誤 | 只記日誌，不 spawn |

spawn 只在 `crates/jc-tsf/src/engine.rs` 的 `connect()` 失敗分支，於 worker 執行
緒，不阻塞 UI。

- 用 `Command::args([...])` 逐項傳 argv，不把整串命令列交給
  `CreateProcessW::lpCommandLine`；`\"` 與末尾 `\` 的解析會讓 `--data` 吃到後面的
  `--settings`。
- 用 `CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP`，不用 `DETACHED_PROCESS`：
  後者仍會替 console 子系統配置新視窗。引擎維持 console subsystem，管理員可在
  PowerShell 執行並看 stdout。
- 路徑用 `GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS)` 取得這份
  DLL 的目錄，組出 `jc-engine.exe`；資料目錄取 `%ProgramData%\jc-ime\data`，
  設定檔取 `%LOCALAPPDATA%\jc-ime\settings.json`。不讀 Run 機碼的字串。
- `SpawnCooldown` 3 秒內不重複 spawn；spawn 後以 10 次 × 100 ms 有界等待 pipe。
  跨行程重複由 supervisor lockfile 擋掉。spawn 失敗只記警告，按鍵照常排隊。

## pipe 與連線

- Windows 用 named pipe，名稱預設 `jc-ime-<SID 尾段>`；也可用
  `HKCU\Software\jc-ime\EnginePipe` 覆寫。
- Unix 用 Unix domain socket；`--socket` 指定路徑。
- 訊框是 4-byte 小端長度前綴加 JSON，預設上限 1 MiB。
- 每條連線一個 worker thread，可並行。
- `FocusPipe` 用 focus 事件管理連線，斷線後有 120 ms 寬限與重連退避。

## 檔案位置

| 項目 | 位置 |
|---|---|
| 監督與重啟 | `crates/jc-engine/src/main.rs`（`run_supervisor`、`child_args`） |
| 單一實例與 log | `crates/jc-engine/src/supervise.rs` |
| TIP 連線與按需 spawn | `crates/jc-tsf/src/engine.rs`（`connect`、`launch_detached`、`SpawnCooldown`） |
| pipe 名稱 | `crates/jc-windows`、`crates/jc-tsf/src/ipc.rs` |
