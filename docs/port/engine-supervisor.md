# engine-supervisor：引擎常駐與自動重啟

Worktree：`/mnt/work/jc/wt/engine-supervisor`（branch `engine-supervisor`）

## 目標

引擎行程被結束或崩潰後會自動回來，而且每個使用者最多只有一個監督者。
不做 Windows 服務：IME 是使用者層級的元件，服務跑在 session 0，碰不到
使用者的 pipe 與輸入焦點。

## Go 對照

Go 參考實作沒有監督者，`0c-server` 由使用者自行啟動或由外殼拉起。這個
task 是新增行為，沒有可直接搬的測試。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `cmd/0c-server/main.go`（啟動流程） | `crates/jc-engine/src/main.rs`（`run_supervisor`、`child_args`） | rewritten：多了 `--supervise`，子行程參數由父行程轉送 |
| n/a | `crates/jc-engine/src/supervise.rs`（`run`、`run_with`、`InstanceLock`、`LogRing`） | rewritten：全新模組，行為依任務書 |
| n/a | `crates/jc-engine/src/pipe_windows.rs`（既有 pipe 伺服器） | n/a：子行程直接沿用，沒有修改 |
| n/a | `crates/jc-tsf/src/engine.rs`（`DEFAULT_PIPE`） | n/a：只讀，確認預設 pipe 名稱一致 |
| n/a | `installer/**` | n/a：這個 worktree 沒有 installer 目錄，待辦寫在最後一節 |

## 啟動方式

```text
jc-engine --supervise [--socket <name>] [--supervisor-log <path>]
          [--data <dir>] [--settings <path>] [--mode <zh|ja|mixed|en>]
          [--neural <on|off>] [--max-frame <bytes>] [--allow-empty-data]
```

- 監督者用 `std::env::current_exe()` 啟動同一支執行檔，子行程參數是
  `jc-engine [原參數] --socket <name>`。
- `--socket` 沒給時，Windows 用 `jc-ime`（與 `jc-tsf::engine::DEFAULT_PIPE`
  一致），unix 用 `$XDG_RUNTIME_DIR/jc-ime-engine.sock`，沒有
  `XDG_RUNTIME_DIR` 就用暫存目錄。
- 子行程的 stdout／stderr 由監督者接管，逐行寫進同一份 log（前綴
  `子行程 stdout:`／`子行程 stderr:`），不會因為 pipe 滿而卡住引擎。
- `--print-settings` 與 `--supervise` 併用會直接報錯。

## 單一實例

| 平台 | 機制 |
|---|---|
| Windows | 具名 mutex `Local\jc-ime-engine-supervisor`（`CreateMutexW` + `ERROR_ALREADY_EXISTS`）；`Local\` 命名空間是登入使用者範圍 |
| Linux | abstract unix socket `jc-ime-engine-supervisor`；名稱被佔用時 bind 回 `EADDRINUSE`，行程結束自動消失 |
| 其他 unix | `$TMPDIR/jc-ime-engine-supervisor.sock`；偵測到殘留檔案且沒人監聽就清掉重試一次 |

第二個監督者取得鎖失敗時回 `Error::AlreadyRunning`，`main` 記一行 info 後
以 exit code 0 結束。登入時 `Run` key 觸發兩次不會留下錯誤視窗。

## 退避曲線

| 連續失敗次數 | 等待 | 備註 |
|---|---|---|
| 1 | 1s | |
| 2 | 2s | |
| 3 | 5s | |
| 4 以後 | 10s | 上限 |
| 5 | 300s | 寫 log `連續失敗 5 次，停止重試 300s`，暫停結束後失敗次數歸零 |

- 「失敗」的定義是子行程以非 0 狀態或被訊號結束。若子行程存活超過
  `stable_after`（60s）才結束，之前的連續失敗次數先歸零。
- 子行程正常結束（exit code 0，例如 IME 送出 `Shutdown`）不算失敗，等
  曲線第一個值 1s 後重啟。
- 啟動子行程本身就失敗（執行檔不見、權限不足）也走同一條退避與暫停邏輯。

## 健康檢查

每 30 秒檢查一次子行程的 socket／pipe：

- Windows：`WaitNamedPipeW(name, 1)`，只看 pipe 存不存在，不建立連線。唯一的
  instance 被 IME 佔用時會回 `ERROR_SEM_TIMEOUT`，視為健康；只有找不到 pipe
  才記警告。
- unix：`UnixStream::connect` 一次後立刻關閉。引擎一次只服務一條連線；IME
  正在連線時檢查連線只會排在 backlog，等 IME 斷線後才被接受，讀到 EOF、
  重置 session 再回到等待。不會影響正在進行的請求。
- 連不上但子行程還活著：寫 `健康檢查失敗：連不上 <path>，子行程仍在執行 pid=...`。
  恢復時再寫一行 `健康檢查恢復正常`。健康檢查不會殺掉子行程，也不會觸發重啟。

## Log

- 位置：`%LOCALAPPDATA%\jc-ime\supervisor.log`；沒有 `LOCALAPPDATA` 時依序用
  `$XDG_STATE_HOME/jc-ime/supervisor.log`、`~/.local/state/jc-ime/supervisor.log`、
  暫存目錄。`--supervisor-log <path>` 可覆寫。
- 環狀上限 1 MiB。寫入前若會超過上限，保留檔尾約一半的內容（從換行邊界
  開始），插入 `[log] 已達大小上限，丟棄最舊內容`，再 append 新行。
- 每行格式：`[   12.345s] 訊息`，秒數相對於監督者啟動時間。
- 目錄不存在會自動建立；log 開不起來時監督者直接失敗，不會無聲常駐。

## 停止與子行程收尾

- Windows：`SetConsoleCtrlHandler` 攔 `Ctrl+C`、`Ctrl+Break`、關閉視窗、
  登出、關機；另外開一個隱藏訊息視窗接 `WM_ENDSESSION`。
- unix：攔 `SIGINT`、`SIGTERM`。
- 收到停止訊號後先送溫和訊號（unix `SIGTERM`、Windows `CTRL_BREAK_EVENT`
  到子行程自己的 process group），等 5 秒；沒結束才 `kill`。
- 子行程被訊號直接結束時，unix socket 檔可能留著；下次啟動時
  `prepare_socket_path` 會偵測沒有監聽者並清掉。

## 與 IME 重連的互動

- pipe／socket 名稱固定，TSF 讀 `HKCU\Software\jc-ime\EnginePipe`（預設
  `jc-ime`），因此引擎重啟後位址不變，不需要重設 IME。
- 重啟期間（1s 到 10s，最長暫停 5 分鐘）連線會失敗。`jc-tsf` 每次請求都
  會嘗試重連，回到可用狀態後下一個按鍵就恢復；使用者看到的是短暫的
  「連不上」，不是需要重開機的狀態。
- 監督者只保證「引擎回來」，不重送失敗的請求。組字狀態在引擎記憶體裡，
  崩潰會遺失當下未提交的組字，這是既有架構的限制。

## 驗證

```sh
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

Linux 上已實際跑過（`--allow-empty-data` 讓子行程不需要詞典）：

1. 啟動監督者後 `kill -9` 子行程，1 秒後出現新 pid，log 記
   `第 1 次失敗，1000ms 後重啟`。
2. 同時啟動第二個 `--supervise`，exit code 0，沒有建立第二個子行程。
3. 連續殺 5 次，log 依序出現 1000ms、2000ms、5000ms、10000ms，第 5 次
   `連續失敗 5 次，停止重試 300s`。
4. `SIGTERM` 監督者：子行程先結束，log 記 `收到停止訊號，結束子行程`、
   `監督者結束`，行程表沒有殘留。

`crates/jc-engine/src/supervise.rs` 的 `#[cfg(unix)]` 測試（只寫、未執行）：

- `second_supervisor_is_rejected_by_lock`
- `child_is_restarted_until_stopped`
- `five_failures_pause_before_retry`
- `health_check_warns_when_socket_missing`

跨平台測試：`backoff_curve_matches_spec`、`supervisor_mutex_is_per_user`、
`log_ring_keeps_newest_lines_under_limit`。

## 需要 Windows 才能驗的項目

- `Local\jc-ime-engine-supervisor` 具名 mutex 的跨行程互斥。`Local\` 是 session
  範圍，同一使用者的兩個 RDP session 會各有一個監督者；但 TSF 的 pipe 名稱
  目前是機器共用的 `jc-ime`，兩個子行程只有一個能 bind，另一個會退避並在
  5 次失敗後暫停。要支援多 session 得讓 pipe 名稱帶使用者或 session 識別，
  屬於 `jc-tsf` 的後續工作。
- `SetConsoleCtrlHandler` 對 Ctrl+C、關機、登出的實際事件順序。
- 隱藏訊息視窗收到 `WM_ENDSESSION`，以及 Windows 關機時 5 秒收尾是否足夠。
- `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT)` 對引擎子行程的效果。
- `WaitNamedPipeW(name, 1)` 在 IME 持有唯一 pipe instance 時確實回
  `ERROR_SEM_TIMEOUT`，確認不會誤報健康檢查失敗。

## 安裝器待辦

這個 worktree 沒有 `installer/**`，也沒有 `docs/install.md`，因此只記待辦：

1. 啟動指令改成 `jc-engine.exe --supervise`，並把資料目錄與 pipe 名稱一併
   寫進參數。
2. 開機自動啟動改寫 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
   （per-user），不要用 `HKLM\...\Run`。監督者的 `Local\` mutex 與 TSF 的
   `HKCU` pipe 設定都是使用者範圍，混用 `HKLM` 會在多位使用者機器上錯開。
3. 移除或避開任何直接啟動 `jc-engine.exe` 的舊註冊，否則監督者與裸引擎會
   同時搶同一個 pipe；裸引擎先 bind 成功時，監督者的子行程只會啟動失敗並
   進入退避。
