# os-isolation：平台隔離稽核與 CI 守門

這一輪不改行為，只做三件事：補齊平台專屬程式碼的 `cfg` 邊界、把 `jc-linux`／
`jc-mac` 的 unix 實作位置獨立出來、在 Linux CI 加上 Windows 目標的型別檢查。

## 稽核表

判準：平台專屬程式碼必須有一層 `cfg`，而且在錯的平台建置時要嘛不編譯、要嘛回
明確錯誤。表格的「結果」欄是這次動過的地方。

| 位置 | 內容 | 現況 | 結果 |
|---|---|---|---|
| `jc-engine/src/main.rs` | `use std::os::unix::net::{UnixListener, UnixStream}` | `#[cfg(unix)]` | 已正確 |
| `jc-engine/src/main.rs` | `mod pipe_windows;` | `#[cfg(windows)]` | 已正確 |
| `jc-engine/src/main.rs` | `serve_socket`、`prepare_socket_path` | 三份實作：`cfg(unix)`／`cfg(windows)`／`cfg(not(any(unix, windows)))`；第三份回 `anyhow::Error`，不 panic | 已正確 |
| `jc-engine/src/main.rs` | `default_supervisor_socket`、`SOCKET_HELP` | 各三份，依平台給不同路徑與說明 | 已正確 |
| `jc-engine/src/pipe_windows.rs` | 整個檔案都是 Win32 | 原本只有 `mod` 上的 `#[cfg(windows)]` | **加上檔案內 `#![cfg(windows)]`**，兩層 |
| `jc-engine/src/supervise.rs` | `platform` 模組（單一實例鎖、停止訊號、健康檢查） | `cfg(unix)`／`cfg(windows)`／`cfg(not(any(unix, windows)))` 三份 | 已正確 |
| `jc-engine/src/supervise.rs` | `spawn_child` 的 `creation_flags` | 區塊內 `#[cfg(windows)]` | 已正確 |
| `jc-engine/src/supervise.rs` | `terminate`：SIGTERM 對 Ctrl+Break | 區塊內 `#[cfg(unix)]`／`#[cfg(windows)]` | 已正確 |
| `jc-engine/src/supervise.rs` | 需要 `/bin/sh` 與 SIGTERM 的測試 | 逐項 `#[cfg(unix)]`；其餘（退避曲線、log 環）平台無關 | 已正確 |
| `jc-engine/tests/socket.rs` | 整份用 `UnixStream` 起引擎 | 檔案開頭 `#![cfg(unix)]` | 已正確 |
| `jc-windows/src/pipe.rs` | `CreateFileW` 實作 | `#[cfg(windows)] mod imp`；非 Windows 用 `#[cfg(not(windows))] mod imp` 的替身回 `Error::Unsupported` | 已正確 |
| `jc-windows/src/worker.rs` | `CancelSynchronousIo`、執行緒 handle | 函式內 `#[cfg(windows)]`／`#[cfg(not(windows))]` | 已正確 |
| `jc-windows/Cargo.toml` | `windows-sys` | `[target.'cfg(windows)'.dependencies]` | 已正確 |
| `jc-tsf/src/lib.rs` | 整個 crate 都是 TSF／COM | crate 開頭 `#![cfg(windows)]`，非 Windows 只剩空 cdylib | 已正確 |
| `jc-tsf/Cargo.toml` | `windows`、`windows-core`、`jc-core`、`jc-proto`、`jc-windows` | 原本是無條件相依，Linux 也會編 | **全部移到 `[target.'cfg(windows)'.dependencies]`** |
| `jc-tsf-smoke/src/main.rs` | 兩份 `main` | `#[cfg(windows)]`／`#[cfg(not(windows))]`；六個 windows 專屬模組逐項 `#[cfg(windows)]` | 已正確 |
| `jc-tsf-smoke/src/report.rs` | 只做輸出格式化，不碰 Win32 | 平台無關，兩邊都編（`cfg_attr` 關掉未使用警告） | 已正確 |
| `jc-tsf-smoke/Cargo.toml` | `windows`、`jc-windows` 等 | `[target.'cfg(windows)'.dependencies]` | 已正確 |
| `jc-linux`／`jc-mac` | 佔位 client，但 `lib.rs` 沒有任何平台邊界 | 未來放入 `UnixStream` 會直接編進 Windows | **拆成 `lib.rs`（平台無關）＋ `unix.rs`（`#![cfg(unix)]`）＋ `unsupported.rs`（`#![cfg(not(unix))]`）** |

掃描方式：`grep -rn "std::os::\|libc::\|windows_sys\|windows::Win32\|target_os"` 掃
六個 owned crate，再逐項對照上表。除表列項目外沒有其他平台專屬 import。

## 每個 crate 的平台邊界

| crate | Unix | Windows | 其他平台 |
|---|---|---|---|
| `jc-engine` | unix socket、SIGTERM、abstract socket 鎖 | named pipe、具名 mutex、Ctrl+Break | stdio 可用；`--socket` 回明確錯誤；鎖是 no-op |
| `jc-windows` | `NamedPipe` 替身，`connect` 回 `Error::Unsupported` | `CreateFileW` + 背景往返執行緒 | 同 Unix |
| `jc-tsf` | 空 cdylib，零相依 | 完整 TSF 文字服務 | 同 Unix |
| `jc-tsf-smoke` | 印訊息後以 2 結束 | 七項檢查 | 同 Unix |
| `jc-linux` | `unix.rs`：唯一允許出現 `UnixStream` 的地方 | `unsupported.rs`：公開介面照舊，全部回 `Unsupported` | 同 Windows |
| `jc-mac` | 同 `jc-linux`（之後接 launchd） | 同 `jc-linux` | 同 Windows |

`jc-linux`／`jc-mac` 的規則是硬性的：`lib.rs` 不得出現 `std::os::unix` 或
`UnixStream`，`unix.rs` 開頭有 `#![cfg(unix)]`，`unsupported.rs` 開頭有
`#![cfg(not(unix))]`。兩個模組都只提供 `UnixSocket`，`Client` 與 `Error` 留在
`lib.rs` 共用，公開介面在 Windows 與 Unix 完全一樣。

## CI 守門

Linux job 新增一步：

```yaml
- name: Windows 目標型別檢查
  run: >
    cargo check --workspace --all-targets
    --exclude jc-cli --exclude jc-settings-ui
    --target x86_64-pc-windows-msvc
```

理由：Linux 建置看不到任何 `cfg(windows)` 分支。`jc-tsf` 在 Linux 是空殼，
`jc-engine` 的 named pipe、`jc-windows` 的 `CreateFileW` 全部不編譯，所以
「Linux CI 綠燈」對 Windows 程式碼沒有保證，只能靠 `windows (msvc)` job 在
Windows 上跑。加上這一步之後，Linux job 也會用 Windows 目標做型別檢查，`cfg`
接錯、`windows-sys` feature 少開、Windows 專屬 API 用錯，在 PR 就會失敗。

`cargo check` 不連結，因此 Linux runner 不需要 MSVC 工具鏈，只要
`rustup target add x86_64-pc-windows-msvc`（已加在 toolchain step 的 `targets`）。

兩個 crate 排除在外，原因是它們的 C 相依需要 MSVC 的 C 編譯器與 Windows SDK：

- `jc-cli`：`ureq` → `rustls` → `ring`，build script 會叫 `cc` 編 C 與組合語言。
- `jc-settings-ui`：GPUI，資源檔要 resource compiler，另有 C 相依。

這兩個 crate 的 Windows 建置由 `windows (msvc)` job 的 `cargo build --workspace`
負責；它們沒有平台專屬模組，不在這次稽核範圍。

## 驗證

```text
cargo check --workspace --all-targets --exclude jc-settings-ui            通過
cargo check --workspace --all-targets --target x86_64-pc-windows-msvc \
    --exclude jc-cli --exclude jc-settings-ui                             通過
cargo clippy --workspace --all-targets --exclude jc-settings-ui -- -D warnings
                                                                          通過
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc \
    --exclude jc-cli --exclude jc-settings-ui -- -D warnings              通過
cargo fmt --all -- --check                                                通過
```

`cfg` 確實生效的兩個實測（做完就還原）：

1. 把 `jc-linux/src/unsupported.rs` 暫時移走，跑
   `cargo check -p jc-linux --target x86_64-pc-windows-msvc`，得到
   `error[E0583]: file not found for module 'unsupported'`，證明 Windows 目標
   走的是非 unix 那份。
2. 在 `pipe_windows.rs` 暫時塞一行型別錯誤，跑
   `cargo check -p jc-engine --target x86_64-pc-windows-msvc`，直接回
   `error[E0308]: mismatched types`，證明 named pipe 真的被 Windows 目標編譯。

## 已知限制

- 本機沒有 `libfontconfig1-dev`、X11／Wayland 開發套件，`cargo check --workspace`
  在 `jc-settings-ui` 的 `yeslogic-fontconfig-sys` 就會失敗。這是環境缺套件，
  不是程式碼問題；CI 的 linux job 有安裝，本機驗證改用 `--exclude jc-settings-ui`。
- `--target x86_64-pc-windows-msvc` 只做型別檢查。實際連結、資源檔嵌入與執行
  仍由 `windows (msvc)` job 覆蓋，跨平台檢查不能取代它。
- 不是 unix 也不是 Windows 的平台（例如未來的手持裝置）刻意保持可編譯：
  stdio 模式可用，`--socket`、單一實例鎖、健康檢查退回明確的 no-op 或錯誤。
  這樣 `cargo check` 不會被未知平台擋住，但也不會假裝功能存在。
- `jc-tsf` 在非 Windows 產出的 cdylib 沒有任何符號。這是刻意的：不提供替代實作，
  呼叫端在非 Windows 本來就不該載入它。

## 新增平台程式碼時放哪裡

| 要加的程式碼 | 位置 | 必要的 cfg |
|---|---|---|
| 引擎的 unix socket 行為 | `jc-engine/src/main.rs` 既有的 `#[cfg(unix)]` 函式，或新檔 `pipe_unix.rs` | 新檔在 `main.rs` 用 `#[cfg(unix)] mod pipe_unix;`，檔案開頭再加 `#![cfg(unix)]` |
| 引擎的 Windows 傳輸 | `jc-engine/src/pipe_windows.rs` | 已在 `mod` 與檔案內各有一層 `#![cfg(windows)]` |
| 監督者的平台差異（鎖、訊號、健康檢查） | `jc-engine/src/supervise.rs` 的 `platform` 模組 | 新增一個 `#[cfg(...)] mod platform`，三個分支都要有，否則非目標平台編不過 |
| Windows wrapper client | `jc-windows` | Cargo 相依放 `[target.'cfg(windows)'.dependencies]`；非 Windows 的分支要回 `Error::Unsupported` |
| unix wrapper client | `jc-linux/src/unix.rs`、`jc-mac/src/unix.rs` | 只能寫在這兩個檔案；`lib.rs` 保持平台無關 |
| TSF／COM 程式碼 | `jc-tsf` | crate 已有 `#![cfg(windows)]`；新增相依一律放 `[target.'cfg(windows)'.dependencies]` |
| 冒煙程式的 Windows 檢查 | `jc-tsf-smoke/src/` 的 windows 模組 | 在 `main.rs` 用 `#[cfg(windows)] mod ...;` |

通則：

1. 平台專屬檔案在 `mod` 宣告與檔案開頭各加一層 `cfg`，拿掉其中一層就會編不過。
2. 不支援的平台保留同名公開介面，回 `Error::Unsupported`，不要 `todo!()`。
3. 平台專屬相依一律放 `[target.'cfg(...)'.dependencies]`，不要放 `[dependencies]`。
4. 新增平台分支後跑 `cargo check --workspace --all-targets --target <target>`，
   不要只看 host 的綠燈。

## Go → Rust 對照

這一輪是稽核與 CI 守門，沒有搬新行為；Go 參考實作只拿來確認平台分支的存在。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `cmd/0c-server/transport_windows.go` | `crates/jc-engine/src/pipe_windows.rs` | n/a：已在 engine 與 client 階段移植，這輪只補檔案層 `cfg` |
| `cmd/0c-server/main.go`（unix socket 分支） | `crates/jc-engine/src/main.rs`（`serve_socket`） | n/a：已移植，這輪確認 `cfg(unix)` |
| `cmd/0c-server/supervisor_*.go` | `crates/jc-engine/src/supervise.rs` | n/a：已移植，這輪確認 `platform` 三分支與測試 `cfg` |
| n/a（Go 沒有跨平台佔位 crate） | `crates/jc-linux`、`crates/jc-mac` | rewritten：拆出 `unix.rs`／`unsupported.rs` |
| n/a | `.github/workflows/ci.yml`、`docs/port/os-isolation.md` | rewritten |
