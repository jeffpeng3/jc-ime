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

## 教訓：拿掉固定 socket 還不夠，autostart 必須用監督模式

第一輪修正只把 autostart 的 `--socket jc-ime` 移除，以為引擎會自動走
`resolve_pipe_name()` 推出 `jc-ime-1001`。rc.9 真機 log 證明這樣不夠：

```text
[RUN 機碼] jc-engine.exe --data "C:\ProgramData\jc-ime\data\" --settings "%LOCALAPPDATA%\jc-ime\settings.json"
[tsf.log]  連線引擎 pipe jc-ime-1001 失敗：系統找不到指定的檔案 (os error 2)
[tsf.log]  沒有任何引擎 pipe 成功建立
[supervisor.log] 不存在（引擎監督者根本沒啟動）
```

原因在 `crates/jc-engine/src/main.rs`：沒有 `--socket` 的引擎走直接伺服模式，
`match options.mode` 命中 `Transport::Stdio`（約 166 行），完全不開 named pipe。
`resolve_pipe_name()` 只在監督者路徑 `run_supervisor()` →
`default_supervisor_socket()`（約 182、242 行）才會被呼叫；直接伺服模式根本
沒走到那條路徑。

正確修法是 autostart 加 `--supervise`（不加 socket）：

- `run_supervisor()` 在沒有 socket 時呼叫 `default_supervisor_socket()`
  → `resolve_pipe_name()` → `jc-ime-1001`，與 TIP 一致。
- `child_args()`（約 209 行）自動把 `--socket jc-ime-1001` 加給子行程，
  子行程用直接伺服模式監聽 pipe。
- 子行程結束由 `supervise::run` 依退避曲線重啟，常駐不需要外部看顧。
- 第二個實例回 `AlreadyRunning`，視為正常結束。

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `cmd/0c-server/transport_windows.go`（`defaultPipe`／`userPipe`，以使用者 SID 命名） | `crates/jc-windows/src/pipe_name.rs`（`resolve_pipe_name`） | rewritten：前綴從 `0c` 改 `jc-ime`，解析順序一致；邏輯本輪不改 |
| 無（Go 沒有 Run key autostart） | `installer/jc-ime.wxs` 的 `EngineAutostart` | n/a：MSI 才有 Run 值，本輪改用 `--supervise` 並移除固定 socket 覆寫 |
| 無 | `crates/jc-tsf/src/engine.rs` 的 `pipe_name()`、`crates/jc-engine/src/main.rs` 的 `default_supervisor_socket()` | rewritten：兩端都呼叫同一個 `resolve_pipe_name()` |
| 無 | `installer/validate-linux.sh`、`crates/jc-windows/src/pipe_name.rs` 的測試 | n/a：新增檢查，鎖住 autostart 帶 `--supervise`、不帶固定 socket |

## 修正與一致性

`installer/jc-ime.wxs` 的 `EngineAutostart` 帶 `--supervise`、`--data`、
`--settings`，不帶固定 socket。引擎走
`run_supervisor()` → `default_supervisor_socket()` → `resolve_pipe_name()`，TIP 走
`engine::pipe_name()` → 同一個 `resolve_pipe_name()`，因此兩邊都從 SID 尾段
得到 `jc-ime-<rid>`。`pipe_name.rs` 的解析邏輯沒有改動。

優先順序：`JC_IME_PIPE` > `HKCU\Software\jc-ime\EnginePipe` > SID 尾段
`jc-ime-<rid>` > 取不到 SID 時退回 `jc-ime` 並記一次警告。覆寫只適合除錯，
必須兩端同時設成同一個值；只改一方的固定 socket 會再次造成不同名。

防線：

- `installer/validate-linux.sh` 檢查 `EngineAutostart` 有 `--supervise`、
  沒有 `--socket`，且 `--data`、`--settings` 都還在。
- `crates/jc-windows/src/pipe_name.rs` 的
  `autostart_override_would_split_tip_and_engine` 鎖住「override 會讓兩端
  不同名」，提醒 autostart 不能用固定 socket。
- 文件同步更新：`docs/install.md`、`docs/wrapper.md`、
  `docs/port/tsf-installer.md`、`crates/jc-engine/src/main.rs` 的用法註解。

## 舊機升級：Run 機碼可能沒更新

`EngineAutostart` 的 component GUID 是
`{28B7FF4A-8D07-4C5F-AEE6-CCF43F7B717F}`，升級時沒有變動。WXS 的
`<MajorUpgrade AllowSameVersionUpgrades="yes" />` 用預設排程，會在
`InstallValidate` 之後移除舊產品，再由 `WriteRegistryValues` 重寫 Run 值，
所以正常升級會拿到新指令。少數情況（例如同版就地修復或手動中斷）如果 Run
機碼仍是舊值，引擎會退回直接伺服的 stdio，打字又變全英文。

升級後先確認 Run 值：

```powershell
Get-ItemProperty -Path "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run" -Name "jc-ime-engine"
```

若值不含 `--supervise`，手動寫入：

```powershell
$run = '"C:\Program Files\jc-ime\jc-engine.exe" --supervise --data "C:\ProgramData\jc-ime\data\" --settings "%LOCALAPPDATA%\jc-ime\settings.json"'
Set-ItemProperty -Path "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run" -Name "jc-ime-engine" -Value $run
```

寫入後登出再登入（或重新開機）讓 Run 項目重新啟動引擎。
`%LOCALAPPDATA%` 由 `REG_EXPAND_SZ` 在每位使用者登入時展開，不要換成固定路徑。
不改 component GUID：同一資源換 GUID 會讓 Windows Installer 視為不同元件，
反而可能在解除安裝時留下舊的 Run 值。

## 驗證

| 指令 | 結果 |
|---|---|
| `cargo check --workspace --all-targets` | 通過 |
| `cargo fmt --all -- --check` | 通過 |
| `grep -n -- '--supervise' installer/jc-ime.wxs` | autostart 的 Value 有此旗標 |
| `grep -n -- '--socket' installer/jc-ime.wxs` | 沒有輸出（連 XML 註解都不能寫，避免 `--` 讓 XML 失效） |
| `installer/validate-linux.sh` | autostart 檢查通過；本機缺少 .NET runtime，`wix` 無法啟動，WiX 編譯與綁定階段未能驗證 |

## 真機驗證

重新安裝 MSI 後登入，在記事本切到「jc 注音」打字要能出中文；同時確認
`%LOCALAPPDATA%\jc-ime\supervisor.log` 有產生、`tsf.log` 不再出現
「連線引擎 pipe … 失敗」。若 Run 值不含 `--supervise`，依上一節手動修正後
登出再登入。
