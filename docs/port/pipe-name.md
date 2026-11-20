# pipe-name：pipe 名稱加入使用者身分

Worktree：`/mnt/work/jc/wt/pipe-name`（branch `pipe-name`，base `main`）

## 目標

引擎 pipe 名稱從固定的 `jc-ime` 改成帶使用者識別的名稱，避免同一台機器上兩個
使用者同時登入時連到同一個引擎、把輸入送給別人。名稱解析規則只有一份實作，由
`jc-tsf`、`jc-engine` 與 `jc-windows` 共用。

## 放在哪裡

解析函式放在 `crates/jc-windows/src/pipe_name.rs`，由 `jc-windows` 公開。

選這個位置的理由：

- `jc-tsf` 的 `Cargo.toml` 不在這個 task 的 owned paths，但它已經依賴
  `jc-windows`。放在 `jc-windows` 讓 TSF 端不必新增依賴。
- `jc-windows` 本來就是唯一直接碰 Windows named pipe 的 crate，名稱推導要讀
  SID 與 `HKCU`，跟 pipe client 放在一起，Windows 整合只有一處。
- 不選 `jc-platform`：它目前沒有任何 crate 依賴，定位是「不引入 OS 型別的
  平台契約」。把 SID／註冊機碼查詢放進去會讓三個 crate 各新增一條依賴，還要
  在契約 crate 引入 `windows-sys`。

代價是 server（`jc-engine`）依賴 client crate `jc-windows`。這個依賴只在
Windows 目標啟用，內容也只有 pipe 名稱解析與既有的 client。

## 命名規則

依序判斷，第一個有值的就使用：

| 順序 | 來源 | 值 |
|---|---|---|
| 1 | 環境變數 `JC_IME_PIPE` | 原樣使用（trim 後非空才生效） |
| 2 | `HKCU\Software\jc-ime\EnginePipe`（REG_SZ） | 原樣使用（trim 後非空才生效） |
| 3 | 目前使用者的 SID | `jc-ime-<SID 最後一段>`，例如 `S-1-5-21-…-1001` → `jc-ime-1001` |
| 4 | 取不到 SID | `jc-ime`（共用名稱）＋ 一次警告 |

- SID 只取最後一段（RID）。完整 SID 會讓 pipe 名稱接近 200 字元，而且
  `Local\` 命名空間本來就是登入使用者範圍。
- SID 格式必須是 `S-…-<數字>`，最後一段不是數字就當成取不到。
- 名稱只含 `[A-Za-z0-9-]`，不含使用者可控字串。
- 非 Windows 平台沒有 SID，固定用 `JC_IME_PIPE` 或 `jc-ime`，不產生警告。

## 三個呼叫點

| 使用端 | 位置 | 行為 |
|---|---|---|
| `jc-tsf`（client） | `crates/jc-tsf/src/engine.rs`（`pipe_name`） | 呼叫 `jc_windows::resolve_pipe_name`，用 `OnceLock` 快取整個行程只解析一次，`warning` 用 `crate::logging::warn` 記一次 |
| `jc-engine`（server） | `crates/jc-engine/src/main.rs`（`default_supervisor_socket`） | 監督者沒給 `--socket` 時用共用解析結果；同一份結果傳給子行程的 `--socket`，health check 也用同一個名稱 |
| `jc-windows`（client） | `crates/jc-windows/src/pipe_name.rs` | 規則本體；`Client::connect` 仍由呼叫端傳入名稱 |

`--socket` 明確指定時一律優先，不讀覆寫也不推導。要讓手動設定的名稱同時套用到
IME，必須把同樣的值寫進 `HKCU\Software\jc-ime\EnginePipe` 或 `JC_IME_PIPE`。

## Go 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `cmd/0c-server/transport_windows.go`（`defaultPipe`、`userPipe`） | `crates/jc-windows/src/pipe_name.rs`（`resolve_pipe_name`、`name_from_sid`、`choose_pipe_name`） | rewritten：Go 用完整 SID 轉小寫，這裡只取 RID；新增 `JC_IME_PIPE` 與註冊機碼覆寫 |
| `cmd/0c-server/transport_windows.go`（無 SID 時退回 `0c`） | 同上（`choose_pipe_name`） | rewritten：退回 `jc-ime` 並回報一次警告 |
| `cmd/0c-server/main.go`（`--socket` 預設） | `crates/jc-engine/src/main.rs`（`default_supervisor_socket`） | rewritten：監督者與子行程共用推導名稱 |
| `0c-tsf` 端（Go 沒有對應 Rust 前的 client 名稱解析） | `crates/jc-tsf/src/engine.rs`（`pipe_name`） | rewritten：改為共用函式，移除重複的註冊機碼讀取 |
| n/a | `crates/jc-windows/src/lib.rs` | rewritten：公開解析函式給另兩個 crate |

## 測試

`crates/jc-windows/src/pipe_name.rs`（不需要 Windows，Linux 也會跑）：

- `name_from_sid_uses_last_segment`：`S-1-5-21-1111-2222-3333-1001` → `jc-ime-1001`
- `name_from_sid_rejects_incomplete_values`：空字串、`S-1-5-21-`、非數字尾段 → `None`
- `same_user_gets_same_name`：同一個 SID 解析兩次得到同一個名稱
- `different_users_get_different_names`：`…-1001` 與 `…-1002` 得到不同名稱
- `override_wins_over_sid`、`blank_override_is_ignored`
- `missing_sid_falls_back_with_one_warning`、`unparsable_sid_falls_back`
- `non_windows_resolution_has_no_warning`（`cfg(not(windows))`）

`crates/jc-tsf/src/engine.rs`：

- `pipe_name_is_stable_within_process`：快取後兩次結果相同且非空

## 驗證結果

```sh
cargo check -p jc-windows -p jc-engine -p jc-tsf --all-targets          # 通過
cargo check --target x86_64-pc-windows-msvc -p jc-tsf -p jc-windows -p jc-engine --all-targets  # 通過
cargo check --target x86_64-pc-windows-msvc -p jc-tsf-smoke --all-targets  # 通過
RUST_FONTCONFIG_DLOPEN=1 cargo check --workspace --all-targets          # 通過
cargo test -p jc-windows                                                # 21 passed
```

`cargo check --workspace --all-targets` 直接跑會失敗，原因是
`yeslogic-fontconfig-sys` 的 build script 找不到 `fontconfig`（這台機器沒有
`fontconfig.pc`，`pkg-config --exists fontconfig` 失敗）。`jc-settings-ui` 的
`gpui` 依賴它。加上 `RUST_FONTCONFIG_DLOPEN=1` 讓 build script 改用 dlopen
後整個 workspace 通過；這個環境變數只影響驗證，不是程式碼變更。

## 需要 Windows 實機驗的項目

1. `OpenProcessToken` + `GetTokenInformation(TokenUser)` +
   `ConvertSidToStringSidW` 在一般使用者與系統管理員權杖下都拿得到 SID，且尾段
   與 `whoami /user` 一致。
2. 兩個一般使用者同時登入：兩邊各自得到 `jc-ime-<rid>`，`Get-ChildItem
   \\.\pipe\ | ? Name -like 'jc-ime*'` 看到兩條 pipe，互不影響。
3. IME 連線的 pipe 與 `--supervise` 子行程 bind 的 pipe 是同一條（TSF 日誌的
   `已連上引擎 pipe：…` 與引擎日誌的 `監聽 named pipe …` 同名）。
4. 註冊機碼 `HKCU\Software\jc-ime\EnginePipe` 設成自訂值後，重啟監督者與 IME
   都改用該名稱；刪掉後回到 SID 推導名稱。
5. `JC_IME_PIPE` 覆寫時，監督者、子行程與 TSF 都用同一個值。
6. 把使用者的 SID 查詢弄失敗（例如用受限權杖啟動）時，引擎與 IME 都退回
   `jc-ime`，且日誌各只有一行警告，IME 仍可輸入。
7. 同一使用者開兩個 RDP session：兩個監督者都會啟動子行程，其中一個 bind
   失敗並進入退避。名稱只帶使用者，不含 session 身分，這是已知限制。

## 待辦

- SID 尾段（RID）在同一台機器上唯一，但本機帳號與網域帳號的 RID 可能相同，
  理論上會撞名。若要完全避免，可改用完整 SID，代價是名稱長度。
