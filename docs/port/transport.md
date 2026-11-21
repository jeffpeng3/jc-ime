# transport：用 `interprocess::local_socket` 統一 IPC

引擎與 client 的 transport 統一走 [`interprocess`](https://crates.io/crates/interprocess)
的 `local_socket`。呼叫端只有一套型別（`jc_windows::LocalSocket`）與一組 API；
平台差異由 interprocess 負責：

| 平台 | 底層 | 識別子 |
|---|---|---|
| Windows | 本機 named pipe | `\\.\pipe\<name>`，由 `GenericNamespaced` 補前綴 |
| Linux | abstract unix domain socket | 完整 `--socket` 字串，最多 107 bytes |
| 其他 Unix | unix domain socket 檔 | `/tmp/<最後一段>` |

名稱解析仍由 `jc_windows::resolve_pipe_name()` 決定：`JC_IME_PIPE` >
`HKCU\Software\jc-ime\EnginePipe` > `jc-ime-<SID 尾段>`，取不到 SID 才退回
`jc-ime` 並記警告。`pipe_name.rs` 沒有改動。`--socket` 會跳過 SID 推導；
`jc_windows::socket_name()` 把值轉成不含前綴的識別子，server、client 與健康
檢查共用同一個函式，避免兩端名稱漂移。

## 為什麼不用 TCP／WebSocket／gRPC

研究 windows-chewing-tsf、PIME、khiin-rs、rime/weasel、winxime、rakukan、
taigikeyboard 七個 TSF 輸入法後，沒有一個把 TCP loopback、WebSocket 或 gRPC
用在按鍵熱路徑。按鍵要求的延遲預算遠小於建立連線與 HTTP／序列化握手的成本，
而且 loopback TCP 還要把連接權限交給防火牆規則。khiin-rs 已在 production TSF
輸入法驗證 `local_socket` 這條路，因此選它。

## 保留的舊機制

- **序列化與訊框不變**：`jc_proto` 的 4-byte length + JSON 原樣保留。
  interprocess 明確不插入自己的 framing，byte stream 語意一致。
- **一對一**：server 一次只接受一條連線，`Shutdown` 後才結束；連線時機由
  focus-pipe（branch `focus-pipe`，改 `jc-tsf/src/lib.rs`）統一搬到
  `OnSetFocus`。本分支只動 client 的 transport 函式，不碰 focus 邏輯。
- **pipe-busy 修正**：Windows 上 `LocalSocket::connect()` 用
  `ConnectWaitMode::Timeout(wait)`，interprocess 內部的流程就是
  `CreateFileW` → `ERROR_PIPE_BUSY` → `WaitNamedPipeW` → 重試，等待上限是
  handshake 的 timeout。覆寫預設的 `Unbounded` 很重要，否則忙碌時會無限期
  阻塞 TSF worker。`Client::connect_with_retry()` 的有界重試與退避不變。
  supervisor 的帳號範圍 lockfile 與 job object 沒有改動。
- **SID 後綴**：名稱仍帶使用者 RID，同機多使用者不會連到同一個引擎。

## Windows ACL

`jc-engine` 不指定自訂 SDDL，直接用 `CreateNamedPipeW` 的預設 DACL：
LocalSystem、Administrators 與建立者（目前使用者）完整控制，
Everyone 與匿名帳號可讀。TIP 與引擎是同一使用者行程，照樣能連上；
interprocess 預設加 `PIPE_REJECT_REMOTE_CLIENTS`，遠端用戶連不進來。
pipe 名稱帶使用者 RID，同機多使用者也不會互連。

原本 transport-unified 曾組這段 SDDL，並透過
`ListenerOptionsExt::security_descriptor` 設到 pipe 上：

```text
O:<目前使用者 SID>G:BAD:(A;;GA;;;<SID>)(A;;GA;;;SY)(A;;GA;;;BA)
```

它讓 rc.13 真機引擎無法監聽。`interprocess` 2.4.9 的
`SecurityDescriptor::deserialize` 會把 self-relative SD 複製成 absolute SD，
但 `try_clone.rs` 複製 group 時誤呼叫 `set_owner`（應為 `set_group`），
因此 `G:BA` 把 owner 從目前使用者 SID 覆蓋成 BA (`S-1-5-32-544`)。
BA 是群組 SID，不能當物件的 owner，`CreateNamedPipeW` 回
ERROR_INVALID_OWNER (1307)，pipe 建立失敗，監督者於是不斷重生子行程。
任務書把它標成 ERROR_INVALID_SID，實際上 1307 是 ERROR_INVALID_OWNER
（ERROR_INVALID_SID 是 1337）。

SID 本身由 `jc_windows::current_user_sid()` 從目前行程 token 的 `TokenUser`
經 `ConvertSidToStringSidW` 產生，格式正確；`SY`、`BA` 縮寫也被
`ConvertStringSecurityDescriptorToSecurityDescriptorW` 接受。兩者都不是成因。
移除自訂 SDDL 同時避開這個上游 bug；預設 DACL 已涵蓋原本想授權的三個身分，
單機單使用者情境沒有安全損失。

## 取代的舊檔案

| 舊檔案 | 新檔案 | 狀態 |
|---|---|---|
| `crates/jc-engine/src/pipe_windows.rs`（`CreateNamedPipeW`／`ConnectNamedPipe`） | `crates/jc-engine/src/main.rs` 的 `serve_socket` | 移除；改走 interprocess `Listener` |
| `crates/jc-engine/src/main.rs` 的 Unix `UnixListener` 分支 | 同一個 `serve_socket` | rewritten；`prepare_socket_path` 移除，abstract namespace 由 OS 回收 |
| `crates/jc-windows/src/pipe.rs`（`CreateFileW`／`WaitNamedPipeW`） | `crates/jc-windows/src/transport.rs` | renamed＋rewritten；`NamedPipe` 改為跨平台的 `LocalSocket`，busy 等待保留 |
| `crates/jc-tsf/src/engine.rs` 的 `Client<NamedPipe>` | `Client<LocalSocket>` | rewritten；只換型別，連線時機留給 focus-pipe |
| `crates/jc-engine/tests/socket.rs` | 同檔 | rewritten；client 改走 interprocess，檢查行程結束後再也連不上 |
| `docs/port/named-pipe.md` | 本檔 | renamed＋rewritten |

`docs/port/tsf-pipe.md` 記錄的 pipe-busy 成因與取捨仍然有效，只有檔案路徑
提到 `pipe.rs` 的地方改成 `transport.rs`。

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `cmd/0c-server/main.go`（unix socket 監聽） | `crates/jc-engine/src/main.rs`（`serve_socket`） | rewritten；改用 interprocess `Listener`，Unix 走 abstract namespace |
| `cmd/0c-server/transport_windows.go`（`winio.ListenPipe`、名稱解析） | `crates/jc-engine/src/main.rs`（`serve_socket`） | rewritten；用 `GenericNamespaced`，沿用 Windows 預設 DACL |
| `cmd/0c-server/transport_windows.go`（`serving`／`alreadyRunning` 探測） | `crates/jc-engine/src/supervise.rs`（`health_ok`） | rewritten；Linux 用 interprocess 連 abstract socket，Windows 仍用 `WaitNamedPipeW` 探測 |
| `cmd/0c-server/transport_windows.go`（SDDL 限本人＋AppContainer） | `crates/jc-engine/src/main.rs`（`serve_socket`） | n/a；改用 Windows 預設 DACL，未做 AppContainer |
| `cmd/0c-server/transport_windows.go`（使用者 SID 命名） | `crates/jc-windows/src/pipe_name.rs` | 不在本 task 範圍；名稱解析原樣保留 |
| `cmd/0c-server/logoff_windows_test.go`（pipe 連線測試） | `crates/jc-windows/src/transport.rs`（`local_socket_round_trip`） | rewritten；server/client 都走 interprocess |

## 驗證

```text
cargo check --workspace --all-targets                                   通過
cargo check -p jc-engine -p jc-windows --all-targets --target x86_64-pc-windows-msvc 通過
cargo check -p jc-tsf --all-targets --target x86_64-pc-windows-msvc     通過
cargo clippy -p jc-engine -p jc-windows --all-targets -- -D warnings   通過
cargo clippy -p jc-engine -p jc-windows --all-targets --target x86_64-pc-windows-msvc -- -D warnings 通過
cargo fmt --all -- --check                                              通過
cargo test -p jc-windows                                                22 passed
cargo test -p jc-engine --lib --bins                                    48 passed
```

`cargo check --workspace --all-targets --target x86_64-pc-windows-msvc`
在本機卡在 `ring` 的 build script（環境沒有 MSVC C compiler），與本task無關；
三個受影響的 crate 已個別通過 Windows target check 與 clippy。

`cargo test -p jc-engine -p jc-windows` 的兩個失敗
（`engine_data.rs::engine_converts_zhuyin_with_data`、
`tests/socket.rs::socket_serves_connections_sequentially_and_shutdown_cleans_up`）
在 base commit `d3caea3` 也存在，已用乾淨 worktree 驗證，非本分支造成。
