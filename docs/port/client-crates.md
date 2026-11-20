# client-crates：wrapper 端 client

`jc-windows` 是實作，`jc-linux` 與 `jc-mac` 是佔位。三個 crate 都只透過
`jc-proto` 收送訊息：長度前綴、JSON 與 `PROTOCOL_VERSION` 全部沿用
`jc-proto`，沒有第二份訊框實作。

## 介面

| 項目 | 簽章 | 說明 |
|---|---|---|
| `Client<S>` | `pub struct Client<S>` | 握手完成的 client；串流的所有權在背景執行緒 |
| `Client::connect` | `(name, client_name, timeout) -> Result<Self, Error>` | 只在 `Client<NamedPipe>`；連 `\\.\pipe\<name>` 後握手 |
| `Client::connect_with_retry` | `(name, client_name, timeout, attempts, backoff) -> Result<Self, Error>` | 啟動階段重連 |
| `Client::handshake` | `(stream, client_name, timeout) -> Result<Self, Error>` | 任意 `Read + Write` 串流，測試與 stdio 用 |
| `Client::request` | `(&mut self, &Request, timeout) -> Result<Response, Error>` | 送一個請求等一個回應 |
| `Client::is_closed` | `(&self) -> bool` | 送出 `Shutdown` 或讀到 EOF 後為真 |
| `NamedPipe` | `connect(name) -> Result<Self, Error>` | Windows named pipe 傳輸 |
| `Error` | `Io`、`Protocol`、`VersionMismatch`、`Timeout`、`Unsupported`、`BadFrame` | 與任務書一致 |

與任務書的兩個差異，都是實作上的必要條件：

1. `handshake` 多了 `S: Send + 'static`。逾時靠一條持有串流的背景執行緒，
   串流必須能移到另一條執行緒。
2. 引擎只回 `Error { code: VersionMismatch }` 時不會附上自己的版本，因此
   `VersionMismatch.engine` 記為 `0`（未知）。若引擎回 `Ready` 但版本不同，
   `engine` 就是 `Ready.protocol` 的值。

握手一律送 `jc_proto::PROTOCOL_VERSION`，沒有寫死數字，也不會退回舊版本。

## 逾時設計

每次往返都在一條背景執行緒上完成「寫出請求、讀回回應」，呼叫端用
`mpsc::recv_timeout` 等結果。選這個做法、不用 overlapped I/O 的理由：

- 同一套程式碼能套用在任何 `Read + Write` 串流，包含測試用的假串流與 stdio；
  overlapped I/O 只能用在 named pipe，測試就得另外做一套。
- 不需要在 wrapper 裡引入 async runtime；TSF DLL 的執行緒模型保持單純。
- 訊框仍由 `jc_proto::read_message`／`write_message` 處理，包含「先驗長度再
  配置緩衝」的規則，超長前綴不會配置大緩衝。

**逾時後連線不可信**：管線裡可能還有半個回應，下一個請求會讀到上一個請求的
尾巴。所以 `request` 逾時後把 client 標成 `Unusable`，之後的呼叫直接回
`Error::Timeout`，不再碰串流；呼叫端必須重新連線。這也讓「重送 `Key` 造成
重複輸入」不可能發生。

逾時後會呼叫 Win32 `CancelSynchronousIo` 打斷背景執行緒上阻塞中的讀取，client
被丟掉時也一樣，讓 named pipe handle 真的關閉、引擎可以服務下一個連線。非
Windows 平台沒有這個機制，被阻塞的執行緒會留到對端關閉串流為止；那裡只影響
清理，不影響逾時判定。

## 重連策略

- `connect_with_retry` 只在啟動階段使用：引擎還沒起來時，最多試 `attempts` 次，
  每次失敗後固定等 `backoff`。`attempts` 為 0 視同 1 次。
- 已連線後斷線不自動重送請求：重送 `Key` 會造成重複輸入，`Commit` 重送則可能
  重複上字。要不要重連由 wrapper 決定。
- 版本不符不重試其他版本：兩邊版本必須相同，這是凍結的協定規則。

## crate 狀態

| crate | 狀態 | 傳輸 | 測試 |
|---|---|---|---|
| `jc-windows` | 實作完成 | Windows named pipe（`CreateFileW`，`GENERIC_READ \| GENERIC_WRITE`、`OPEN_EXISTING`） | 7 項行為測試 + pipe 路徑測試 |
| `jc-linux` | 佔位 | 之後接 Unix domain socket | `connect`／`connect_with_retry`／`handshake` 回 `Unsupported` |
| `jc-mac` | 佔位 | 之後接 Unix domain socket／launchd | 同上 |

非 Windows 平台上 `NamedPipe` 型別仍存在，`connect` 回 `Unsupported`，所以
Linux CI 能編譯、能跑測試。佔位 crate 沒有 `todo!()`，所有入口都回明確的
`Unsupported`。

## Go → Rust 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `cmd/0c-server/transport_windows.go`（`userPipe`、`defaultPipe` 的 `\\.\pipe\` 組法） | `crates/jc-windows/src/pipe.rs`（`pipe_path`） | ported；只搬名稱組法，SID 由呼叫端決定 |
| `cmd/0c-server/transport_windows.go`（`winio.DialPipe`、`serving`） | `crates/jc-windows/src/pipe.rs`、`src/worker.rs` | rewritten；改用 `CreateFileW` 加背景往返執行緒 |
| `cmd/0c-server/transport_windows.go`（`listenPipe`、SDDL、pipe 權限） | n/a | 引擎端責任，不在 client crate |
| `cmd/0c-server/main.go`（伺服器端握手與版本檢查） | `crates/jc-windows/src/lib.rs`（client 端 `complete_handshake`） | rewritten；client 只驗 `Ready.protocol` |
| `cmd/0c-update/auto_windows.go`（`dialEngine`、`pipeName`） | `crates/jc-windows/src/pipe.rs`、`src/lib.rs`（`connect`、`connect_with_retry`） | rewritten；timeout 與重試次數改由呼叫端給 |
| n/a（Go 沒有對應的 wrapper client） | `crates/jc-windows/tests/client.rs` | rewritten；假串流行為測試 |
| n/a（Go 只有 unix socket 伺服器端） | `crates/jc-linux`、`crates/jc-mac` | n/a；佔位 crate，等 Unix socket client |

## 驗證

```text
cargo check --workspace --all-targets                                     通過
cargo check --workspace --all-targets --target x86_64-pc-windows-gnu      通過（1 個既有 warning，來自 jc-model）
cargo clippy -p jc-windows -p jc-linux -p jc-mac --all-targets -- -D warnings
                                                                         通過（只剩 .cargo/config 的 fullfp16 警告）
cargo fmt -p jc-windows -p jc-linux -p jc-mac                            執行
```

`cargo clippy --workspace --all-targets -- -D warnings` 目前會失敗，原因都在
既有程式碼、不在這個 task：`jc-cli/src/bin/jinen-gate.rs` 的
`unnecessary_sort_by`，以及 Windows target 上 `jc-model/src/runtime.rs` 的
`unused_mut`。兩者都沒有修改。

本機只裝了 `x86_64-pc-windows-gnu` 的 std，`x86_64-pc-windows-msvc` 沒有安裝。
Windows 分支的實際編譯由 CI 的 `windows (msvc)` job 驗證；本機已用
`x86_64-pc-windows-gnu` 檢查 `CreateFileW`、`CancelSynchronousIo` 與
`windows-sys` feature 的程式碼路徑。`msvc` 與 `gnu` 的 `cfg(windows)` 相同，
差別只在 linker 與 std 供應。
