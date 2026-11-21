# tsf-pipe：`ERROR_PIPE_BUSY` 的雙重成因與取捨

Worktree：`/mnt/work/jc/wt/pipe-busy`（branch `pipe-busy`）

## 真機證據

`supervisor.log` 有兩條不同時間的「監督者啟動」，子行程 pid 分別是 14640
與 11372。`Get-Process` 同時看到兩個 `jc-engine`（4812、11372）與一個
Notepad（5116）。TIP（pid 15132）反覆 Activate／shutdown，每次連
`jc-ime-1001` 都拿到 os error 231。

## 成因一：監督者互斥是 session 範圍，pipe 是帳號範圍

`InstanceLock::acquire` 只認 `CreateMutexW` 的 `ERROR_ALREADY_EXISTS`。原本的
名稱是 `Local\jc-ime-engine-supervisor`，`Local\` 前綴把 kernel object 放在
**目前登入 session**；pipe 名稱卻由 SID 尾段構成，是**帳號範圍**。同一使用者
開兩個 RDP session 時有兩把 mutex，兩個監督者都拿到鎖，各自 spawn 一個子行程
搶同一條 pipe。第二個子行程的 `CreateNamedPipeW` 失敗並退避，TIP 連上的那個
引擎如果正在服務別的連線，就回 `ERROR_PIPE_BUSY`。

此外，鎖只跟監督者的生命週期綁在一起。升級或工作管理員殺掉監督者時，鎖被
作業系統釋放，但子行程若沒跟著死，仍持著 pipe handle。新監督者合法取得鎖，
新子行程卻 bind 失敗。MSI 的 `MajorUpgrade` 並不會先停止執行中的
`jc-engine.exe`。

### 修法

- Windows 鎖改為 `%LOCALAPPDATA%\jc-ime\Local-jc-ime-engine-supervisor.lock`，
  以 `CreateFileW` 分享模式 0 開啟。第二個行程拿到
  `ERROR_SHARING_VIOLATION` → `Error::AlreadyRunning`。`LOCALAPPDATA` 跨
  session 相同，範圍與 pipe 名稱一致。
- 子行程指派到 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 的 job object。監督者被
  殺時子行程一起結束，pipe 釋放；job 建立失敗只記警告，不擋啟動。
- 新增 `#[cfg(windows)] second_supervisor_is_rejected_by_lock`，與 unix 版對稱。
- 升級順序寫在 `docs/port/engine-supervisor.md`：先停行程，再跑 MSI，再確認
  只剩下一個 `jc-engine` 與一條 `jc-ime-*` pipe。

## 成因二：引擎一次只服務一條連線

即使只有一個監督者，TIP 的反覆 Activate／shutdown 仍可能撞上 231。
`Control::shutdown` 不會等待 engine worker 關掉 pipe；`jc-tsf-smoke` 的檢查 5
甚至要 `sleep(200ms)` 才讓檢查 6 連得進去（`crates/jc-tsf-smoke/src/checks.rs:267`）。

**沒有改成多 instance。** `SessionConfig` 與 `Session` 都不是 `Send`：
`jc_core::ja::live::Config` 有 `Rc<dyn NeuralGate>` 與 `Rc<dyn Fn() -> Instant>`
（`crates/jc-core/src/ja/live.rs:415`）。要把 `Session` 移到服務執行緒，或改成
每 client 一份 session，都必須先改 `jc-core` 把 `Rc` 換成 `Arc`
（並讓 `NeuralGate` 為 `Send + Sync`）。那超出本 worktree 的邊界，因此停在
這裡回報，不在本分支偷改別人的 crate。

`MaxInstances` 維持 **1**。引擎沒有更動，所以不會因為多執行緒共用
`Session` 而讓兩個程式的組字互串。

### 實際修法：client 端 `WaitNamedPipeW`

`jc-windows/src/pipe.rs` 新增 `NamedPipe::connect_with_wait`：`CreateFileW`
拿到 `ERROR_PIPE_BUSY` 時呼叫 `WaitNamedPipeW`，有 instance 釋出就再試，
`ERROR_SEM_TIMEOUT` 時回報原本的 busy。`Client::connect` 把原本給 handshake
的 `timeout`（TIP 為 500 ms）也用作等待上限。微軟的
[Named Pipe Client](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-client)
與 [WaitNamedPipeW](https://learn.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-waitnamedpipew)
文件就是這個流程；`WaitNamedPipeW` 成功只保證「當下有 instance」，所以仍要
回到 `CreateFileW` 重試。

這吸收的是短暫佔用。若某個 client 永久佔著 instance，第二個只能等滿 500 ms
再依 `RECONNECT_BACKOFF`（2 秒）重試，仍無法打字。要根治多 client 同時活著
的情況，需要前面提到的 `jc-core` 調整，建議另開任務書處理。

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| n/a（Go 版沒有監督者） | `crates/jc-engine/src/supervise.rs` | rewritten：Windows 單一實例改帳號範圍鎖檔＋job object |
| n/a | `crates/jc-windows/src/pipe.rs` | rewritten：client 端加 `WaitNamedPipeW` |
| n/a | `crates/jc-windows/src/lib.rs` | 小改：`Client::connect` 帶等待 |
| n/a | `docs/port/named-pipe.md`、`docs/port/engine-supervisor.md`、`docs/port/pipe-name.md` | 文件更新 |

## 驗證

- `cargo check --workspace --all-targets`：通過
- `cargo clippy --workspace --all-targets -- -D warnings`：通過
- `cargo fmt --all -- --check`：通過
- `cargo check -p jc-engine --all-targets --target x86_64-pc-windows-msvc`：通過
- `cargo clippy -p jc-engine -p jc-windows --all-targets --target x86_64-pc-windows-msvc -- -D warnings`：通過
- `cargo test`：依共通規則未執行，由整合階段與 CI 跑

Windows 測試（只寫、未執行）：`second_supervisor_is_rejected_by_lock`、
`busy_pipe_waits_for_a_free_instance`。

## 真機驗證

1. 開兩個記事本，都切到 jc 注音，輪流打字；兩邊都要出字。
2. `Get-Process jc-engine` 只看到一個監督者與一個子行程。
3. `Get-ChildItem \\.\pipe\ | ? Name -like 'jc-ime*'` 只看到一條 pipe。
4. `taskkill` 殺掉監督者後，`Get-Process jc-engine` 不應留下子行程。
5. 升級 MSI 前後重複 2～4，確認沒有殘留行程。
