# fix-windows-cfg：Windows 編譯的 cfg 邊界

這個 task 修 CI 的 `windows (msvc)` job。Go 參考實作有 Windows named pipe 分支
（`cmd/0c-server/main.go`），這輪不搬，只讓非 unix 平台編得過、stdio 可用。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `cmd/0c-server/main.go`（unix socket 監聽） | `crates/jc-engine/src/main.rs`（`serve_socket`、`prepare_socket_path`） | 加上 `#[cfg(unix)]`，行為不變 |
| `cmd/0c-server/main.go`（Windows named pipe 分支） | `crates/jc-engine/src/main.rs`（`#[cfg(not(unix))] serve_socket`） | n/a；這輪只回報 `named pipe 尚未實作`，不引入 Windows API |
| `cmd/0c-server/main.go`（stdio 模式） | `crates/jc-engine/src/main.rs`（`Mode::Stdio`） | 沒有 Go 對應；所有平台都能編、能跑 |
| `cmd/0c-server/main_test.go`（socket 測試） | `crates/jc-engine/tests/socket.rs` | 整份加 `#![cfg(unix)]`，非 unix 平台不編譯 |
| n/a | `crates/jc-engine/src/main.rs`（`socket_mode_reports_named_pipe_unimplemented`） | rewritten；只在非 unix 平台編譯，斷言錯誤訊息含 `named pipe 尚未實作` |

## 改了什麼

- `use std::os::unix::net::{UnixListener, UnixStream}` 加 `#[cfg(unix)]`。
- `serve_socket`、`prepare_socket_path` 限 unix；非 unix 提供同名 `serve_socket`
  回傳 `anyhow::Error`，訊息為 `named pipe 尚未實作，這個平台目前只能使用 stdio 模式`，
  不 panic、不 `exit`。
- `--socket` 的說明文字改成平台常數 `SOCKET_HELP`：unix 寫 unix domain socket，
  非 unix 寫 named pipe 尚未實作。
- 命令列解析不分平台，`--socket` 一律接受，實際啟動時才由 `serve_socket` 拒絕。
- `crates/jc-engine/tests/socket.rs` 加 `#![cfg(unix)]`，Windows 不編譯這份整合測試。

## 覆蓋範圍檢查

- 全 workspace 掃過 `std::os::unix`、`UnixListener`、`UnixStream`、`UnixDatagram`、
  `socket`：只有上面兩個檔案。`jc-proto` 的 `socket` 只出現在註解。
- 沒有其他 `cfg(target_os = ...)`／`cfg(windows)` 需要修正。
- `crates/jc-platform` 沒有平台限定的 `std::os` 用法。

## 驗證

```text
cargo check --workspace --all-targets                                    通過
cargo clippy --workspace --all-targets -- -D warnings                    通過
cargo check --workspace --all-targets --target x86_64-pc-windows-gnu     通過
cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings  通過
cargo check --workspace --all-targets --target x86_64-pc-windows-msvc    本機沒有安裝該 target 的 std（E0463），未安裝工具鏈
cargo build -p jc-engine --target x86_64-pc-windows-gnu                  缺 x86_64-w64-mingw32-dlltool，未連結
```

`x86_64-pc-windows-gnu` 與 `x86_64-pc-windows-msvc` 的 `cfg(unix)` 都是 false，
所以 cfg 覆蓋範圍一致；msvc 只差 linker 與 `std` 供應。
