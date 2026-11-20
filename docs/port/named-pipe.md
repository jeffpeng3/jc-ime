# named-pipe：Windows named pipe 傳輸

`--socket` 在 Windows 走 named pipe，行為與 unix socket 相同：一次一條連線、
序列處理、斷線後可重連、`Shutdown` 結束行程。訊框格式與讀寫邏輯沿用
`jc_proto`，`jc-proto` 沒有任何改動。

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `cmd/0c-server/main.go`（unix socket 監聽） | `crates/jc-engine/src/main.rs`（`serve_socket`、`prepare_socket_path`） | 不動，行為不變 |
| `cmd/0c-server/transport_windows.go`（`winio.ListenPipe`、名稱解析） | `crates/jc-engine/src/pipe_windows.rs`（`serve_pipe`、`pipe_name`、`create_pipe`） | rewritten；改用 blocking Win32 API，不用 `go-winio` |
| `cmd/0c-server/transport_windows.go`（`serving`／`alreadyRunning` 探測） | `crates/jc-engine/src/pipe_windows.rs`（`create_pipe` 失敗即回報） | rewritten；不主動探測，靠 `CreateNamedPipeW` 失敗判斷 |
| `cmd/0c-server/transport_windows.go`（SDDL 限本人＋AppContainer） | n/a | 這輪不做；改用 `PIPE_REJECT_REMOTE_CLIENTS`，權限走 pipe 預設 DACL |
| `cmd/0c-server/transport_windows.go`（使用者 SID 命名） | n/a | 不在本 task 範圍；名稱由 `--socket` 決定 |
| `cmd/0c-server/logoff_windows_test.go`（pipe 連線測試） | `crates/jc-engine/src/pipe_windows.rs`（`#[cfg(test)]`） | rewritten；直接在本行程開 server，涵蓋斷線重連與 `Shutdown` |

## Pipe 名稱規則

`--socket <值>` 一律取最後一段，掛到 `\\.\pipe\` 之下：

| 輸入 | 實際 pipe |
|---|---|
| `jc-engine` | `\\.\pipe\jc-engine` |
| `\\.\pipe\jc-engine` | `\\.\pipe\jc-engine` |
| `\\.\PIPE\jc-engine` | `\\.\pipe\jc-engine` |
| `C:\tmp\engine.pipe` | `\\.\pipe\engine.pipe` |
| `/tmp/engine.sock` | `\\.\pipe\engine.sock` |
| `\\otherhost\pipe\jc-engine` | `\\.\pipe\jc-engine` |
| 空字串、只有分隔符 | 啟動失敗，`--socket 需要 pipe 名稱或路徑`／`--socket 的 pipe 名稱是空的` |

最後一列的行為同時擋掉遠端：名稱永遠落在本機 `\\.\pipe\` 命名空間，
`\\主機\pipe\...` 不會被當成遠端 pipe。pipe 本身再開 `PIPE_REJECT_REMOTE_CLIENTS`。

## Pipe 設定

| 項目 | 值 |
|---|---|
| 方向 | `PIPE_ACCESS_DUPLEX` |
| 類型 | `PIPE_TYPE_MESSAGE \| PIPE_READMODE_MESSAGE \| PIPE_WAIT` |
| 遠端 | `PIPE_REJECT_REMOTE_CLIENTS` |
| instance 數 | 1（`PIPE_MAX_INSTANCES`），序列處理 |
| 輸入／輸出緩衝 | 64 KiB（`PIPE_BUFFER_SIZE`，與 Go 版 `winio.PipeConfig` 一致） |
| 讀寫 | blocking `ReadFile`／`WriteFile`，單一執行緒，沒有 async runtime |

## 連線迴圈與錯誤處理

```text
CreateNamedPipeW
  └─ loop
       ConnectNamedPipe（ERROR_PIPE_CONNECTED 視為成功）
       讀寫同一套 read_message／write_message（serve）
       用戶端斷線（ERROR_BROKEN_PIPE／ERROR_PIPE_NOT_CONNECTED／ERROR_NO_DATA）
         → serve 當成 EOF 結束，log::warn，回到等待下一個連線
       收到 Shutdown → 寫完回應，結束迴圈，關閉 pipe，行程結束
       DisconnectNamedPipe → engine.reset_session() → 下一輪
```

- 讀取遇到 `ERROR_MORE_DATA`（message 大於 8 KiB 的 `BufReader` 緩衝）時回傳已讀
  位元組，剩下的留給下一次 `ReadFile`，所以 1 MiB 上限的訊框不會被截斷。
- 寫入遇到 `ERROR_NO_DATA`／`ERROR_BROKEN_PIPE` 轉成 `BrokenPipe`，`serve` 回報
  `ServeError::Write`，只記錄 warning，不影響下一條連線。
- `CreateNamedPipeW` 失敗（例如另一個引擎已用同名 pipe）直接回報錯誤並結束，
  訊息附上 pipe 名稱。
- 每條連線用完就 drop `DuplicateHandle` 出來的 handle，再切斷 instance，
  避免 handle 洩漏。

## 與 Unix 版本的差異

| 面向 | Unix `serve_socket` | Windows `serve_pipe` |
|---|---|---|
| 端點 | socket 檔案路徑 | `\\.\pipe\<name>` |
| 建立 | `UnixListener::bind` | `CreateNamedPipeW`（message mode） |
| 重複啟動 | 先探測 socket 可否連上，能連就拒絕 | `CreateNamedPipeW` 失敗即拒絕 |
| 殘留物 | 啟動前移除殘留 socket 檔，結束後移除 | 沒有檔案，handle 關閉即消失 |
| 等待連線 | `listener.incoming()` | `ConnectNamedPipe`；`ERROR_PIPE_CONNECTED` 視為已連線 |
| 切斷連線 | drop stream | `DisconnectNamedPipe` |
| 讀寫型別 | `BufReader<UnixStream>`／`BufWriter<UnixStream>` | `BufReader<PipeStream>`／`BufWriter<PipeStream>` |
| 遠端存取 | 檔案系統權限 | `PIPE_REJECT_REMOTE_CLIENTS`＋名稱一律本機化 |
| 序列化 | 一次一條連線 | 一次一條連線（instance 數 1） |
| 斷線處理 | 錯誤只記錄，繼續等下一個連線 | 相同 |
| `Shutdown` | 跳出迴圈、移除 socket 檔 | 跳出迴圈、關閉 pipe |
| 每次連線狀態 | `engine.reset_session()` | 相同 |
| `--max-frame` | 相同 | 相同 |
| stdio 模式 | 不分平台 | 不分平台，未改動 |

## 未做

- 沒有比照 Go 版的 SDDL（限本人＋AppContainer 可讀寫）。pipe 走預設 DACL，
  只加上拒絕遠端 client。AppContainer 宿主要連線需要之後補 security descriptor。
- pipe 名稱沒有預設值，也沒有依使用者 SID 命名；由 wrapper 傳 `--socket`。

## 驗證

```text
cargo check --workspace --all-targets                                通過
cargo check --workspace --all-targets --target x86_64-pc-windows-gnu 通過（本機已裝此 target）
cargo check -p jc-engine --all-targets --target x86_64-pc-windows-gnu  通過
cargo clippy -p jc-engine --all-targets -- -D warnings               通過
cargo test                                                          依共通規則未執行，由整合階段與 CI 跑
x86_64-pc-windows-msvc                                               本機未安裝，未驗證
```

Windows 行為由 CI 的 `windows (msvc)` job（build ＋ test）驗證；`pipe_windows.rs`
的測試在 `cfg(windows)` 下編譯，涵蓋名稱轉換、斷線重連與 `Shutdown` 收線。
