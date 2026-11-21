# engine-pipe-name：autostart 與 TIP 的 pipe 名稱一致

Worktree：`/mnt/work/jc/wt/engine-pipe-name`（branch `engine-pipe-name`）

## 根因

真機上 TIP 啟用成功，但打字全是英文，`tsf.log` 重複出現：

```text
[jc-tsf] 連線引擎 pipe jc-ime-1001 失敗：系統找不到指定的檔案 (os error 2)
```

TIP 沒有 override，從 SID 尾段推出 `jc-ime-1001`；WXS 的 `EngineAutostart`
寫死 `--socket jc-ime`，讓引擎用固定名稱。兩邊不同名，pipe 連不上，按鍵
全部放行成英文。

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `cmd/0c-server/transport_windows.go`（`defaultPipe`／`userPipe`，以使用者 SID 命名） | `crates/jc-windows/src/pipe_name.rs`（`resolve_pipe_name`） | rewritten：前綴從 `0c` 改 `jc-ime`，解析順序一致；邏輯本輪不改 |
| 無（Go 沒有 Run key autostart） | `installer/jc-ime.wxs` 的 `EngineAutostart` | n/a：MSI 才有 Run 值，本輪移除 `--socket jc-ime` 覆寫 |
| 無 | `crates/jc-tsf/src/engine.rs` 的 `pipe_name()`、`crates/jc-engine/src/main.rs` 的 `default_supervisor_socket()` | rewritten：兩端都呼叫同一個 `resolve_pipe_name()` |
| 無 | `installer/validate-linux.sh`、`crates/jc-windows/src/pipe_name.rs` 的測試 | n/a：新增檢查，鎖住 autostart 不帶 `--socket` |

## 修正與一致性

`installer/jc-ime.wxs` 的 `EngineAutostart` 只保留 `--data` 與 `--settings`。引擎
走 `default_supervisor_socket()` → `resolve_pipe_name()`，TIP 走
`engine::pipe_name()` → 同一個 `resolve_pipe_name()`，因此兩邊都從 SID 尾段
得到 `jc-ime-<rid>`。`pipe_name.rs` 的解析邏輯沒有改動。

優先順序：`JC_IME_PIPE` > `HKCU\Software\jc-ime\EnginePipe` > SID 尾段
`jc-ime-<rid>` > 取不到 SID 時退回 `jc-ime` 並記一次警告。覆寫只適合除錯，
必須兩端同時設成同一個值；只改一方的 `--socket` 會再次造成不同名。

防線：

- `installer/validate-linux.sh` 檢查 `EngineAutostart` 沒有 `--socket`，且
  `--data`、`--settings` 都還在。
- `crates/jc-windows/src/pipe_name.rs` 的
  `autostart_override_would_split_tip_and_engine` 鎖住「override 會讓兩端
  不同名」，提醒 autostart 不能帶 `--socket`。
- 文件同步更新：`docs/install.md`、`docs/wrapper.md`、
  `docs/port/tsf-installer.md`、`crates/jc-engine/src/main.rs` 的用法註解。

## 驗證

| 指令 | 結果 |
|---|---|
| `cargo check --workspace --all-targets` | 通過 |
| `cargo test -p jc-windows` | 通過 |
| `cargo fmt --all -- --check` | 通過 |
| `grep -n -- '--socket jc-ime' installer/jc-ime.wxs` | 沒有輸出 |
| `installer/validate-linux.sh` | autostart 檢查通過；本機缺少 .NET runtime，`wix` 無法啟動，WiX 編譯與綁定階段未能驗證 |

## 真機驗證

重新安裝 MSI 後登入，在記事本切到「jc 注音」打字要能出中文，且
`%LOCALAPPDATA%\jc-ime\tsf.log`（`JC_TSF_LOG=1`）不再出現
「連線引擎 pipe … 失敗」。若 Run 項目仍帶舊參數，先從工作管理員結束
`jc-engine.exe` 再登入，讓它以新值啟動。
