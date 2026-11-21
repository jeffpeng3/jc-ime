# engine-on-demand：TIP 按需啟動引擎

Worktree：`/mnt/work/jc/wt/engine-on-demand`（branch `engine-on-demand`）

## 問題

`HKLM\...\Run` 只在登入時觸發一次；supervisor 的重試只照顧「子行程死了、
父行程還活著」。使用者在工作管理員 kill 掉 `jc-engine.exe`（或連 supervisor
一起 kill）後，沒有人會把引擎拉回來。

## 決定

TIP 在連不上 engine pipe 時 detached spawn 引擎。引擎仍是**獨立行程**，不是
TIP 的子行程：`std::process::Command` 用 `DETACHED_PROCESS |
CREATE_NEW_PROCESS_GROUP` 啟動後立刻 drop child handle，不用 job object 的
`KILL_ON_JOB_CLOSE`。TIP 結束或當機都不影響引擎。

指令用 `Command::args([...])` 逐項傳遞 argv，不再把整串命令列交給
`CreateProcessW::lpCommandLine`。原因見 `docs/port/spawn-argv-fix.md`：
`CreateProcessW` 對 `\"` 的引號解析與 ShellExecute／Run 機碼不同，末尾 `\` 會
被當成跳脫引號，讓 `--data` 吃到後面的 `--settings`。

supervisor 的 lockfile（ `%LOCALAPPDATA%\jc-ime\Local-jc-ime-engine-supervisor.lock`
）保證全系統只有一個監督者。第二個 spawn 拿到 `ERROR_SHARING_VIOLATION` 會直接
以 exit code 0 結束，不算錯誤。

## 觸發點與時序

| 事件 | 動作 |
|---|---|
| `OnSetFocus` | `FocusPipe` 照原樣啟動 worker；worker 呼叫 `engine::connect()` |
| connect 拿到 `NotFound` / `PipeBusy` | detached spawn 引擎，等 pipe 出現後再連一次 |
| connect 拿到其他錯誤 | 只記日誌，不 spawn（引擎還活著） |

`FocusPipe` 的狀態機與 120 ms 寬限期完全沒動。spawn 只加在
`crates/jc-tsf/src/engine.rs` 的 `connect()` 連線失敗分支。這條路徑在 worker
執行緒，不會阻塞 UI 執行緒。

## 啟動指令

與 MSI 的 `EngineAutostart` 指向相同檔案：

```text
<INSTALLFOLDER>\jc-engine.exe --supervise --data <DATADIR> --settings <LOCALAPPDATA>\jc-ime\settings.json
```

路徑推導：用
`GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS)` +
`GetModuleFileNameW` 取得**這份 DLL** 的所在目錄當 INSTALLFOLDER，組出
`jc-engine.exe`；資料目錄取 `%ProgramData%\jc-ime\data`，設定檔取
`%LOCALAPPDATA%\jc-ime\settings.json`。不用 `GetModuleHandleW(None)`，那會拿到
宿主行程（記事本等）的 exe。

不讀 Run 機碼的字串。機碼的值本身是對的，作為登入時的自動啟動路径繼續由
Windows 使用；它的引號是給 ShellExecute 解析，TIP 把同一串字串交給
`CreateProcessW` 會得到不同的 argv。詳見 `docs/port/spawn-argv-fix.md`。

## 避免 spawn 風暴

- 行程內 `SpawnCooldown` 用 3 秒冷卻；時間內的連線失敗不再 spawn。跨行程的
  重複由 supervisor lockfile 擋掉。
- spawn 後等 pipe 出現：10 次 × 100 ms 有界重試，成功就連上，失敗只記警告。
- worker 自己的 2 秒 `RECONNECT_BACKOFF` 沒變，所以最壞情況約每 3 秒一次 spawn。
- 建構指令或 `Command::spawn()` 失敗只記 `logging::warn`，不 panic、不中斷打字；
  `test_key`／`key_down` 仍照 `Queue::is_connected()` 放行按鍵。

## 實作位置

| 項目 | 檔案 |
|---|---|
| 連線失敗觸發與等待 pipe | `crates/jc-tsf/src/engine.rs`：`connect()`、`connection_warrants_spawn()`、`wait_for_pipe()` |
| detached spawn | `crates/jc-tsf/src/engine.rs`：`launch_detached()` |
| argv 建構與路徑推導 | `crates/jc-tsf/src/engine.rs`：`engine_command()`、`engine_command_from_paths()`、`EngineCommand`、`module_dir()` |
| 冷卻 | `crates/jc-tsf/src/engine.rs`：`SpawnCooldown` |

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| n/a（Go 版沒有 TSF／Run 機碼） | `crates/jc-tsf/src/engine.rs` | rewritten：新增按需 detached spawn |
| n/a | `installer/jc-ime.wxs` | n/a：只讀，沿用既有 `EngineAutostart` 指令 |
| n/a | `docs/port/focus-pipe.md`、`docs/port/engine-supervisor.md` | 文件更新：補按需啟動說明 |

## 驗證

```sh
cargo check --workspace --all-targets
cargo check -p jc-tsf --all-targets --target x86_64-pc-windows-msvc
cargo clippy -p jc-tsf --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo fmt --all -- --check
```

單元測試（只寫、未執行；Windows 上由 CI 跑）：

- `spawn_cooldown_allows_one_spawn_per_window`：3 秒內不重複放行。
- `engine_command_keeps_data_and_settings_as_separate_args`：argv 順序與元素邊界。
- `engine_command_keeps_trailing_backslash_inside_data_arg`：末尾 `\` 不讓 `--data` 吃到 `--settings`。
- `only_missing_or_busy_pipe_triggers_spawn`：只有 pipe 不存在或忙碌才 spawn。

依共通規則未執行 `cargo test`；TIP smoke test 只能在 Windows 執行，Linux 上以
`cargo check -p jc-tsf-smoke --target x86_64-pc-windows-msvc` 驗證可編譯。

## 真機驗證

1. 安裝 MSI，登入後確認 `C:\ProgramData\jc-ime\data` 與 `settings.json` 存在，
   打字正常（`tsf.log` 有「已連上引擎 pipe」）。
2. 工作管理員結束所有 `jc-engine.exe`（supervisor 與子行程都 kill）。
3. `Get-Process jc-engine` 沒有行程，`Get-ChildItem \\.\pipe\ | ? Name -like
   'jc-ime*'` 沒有 pipe。
4. 點一下記事本的文字框：`tsf.log` 先出現「已 detached spawn 引擎」，接著
   「按需啟動後已連上引擎 pipe」；打字立刻有注音候選。引擎 log 的 `--data`
   值是單純的路徑，不含 `--settings`。
5. 短時間內反覆 kill + 點文字框：`tsf.log` 的 spawn 間隔至少 3 秒，沒有 spawn
   風暴；`supervisor.log` 只有一條「監督者啟動」（第二個被 lockfile 擋下）。
6. 關掉記事本再開：`jc-engine.exe` 仍在，`Get-Process` 看得到，證明引擎是獨立
   行程。
