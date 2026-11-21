# spawn-argv-fix：detached spawn 的引數解析錯誤

Worktree：`/mnt/work/jc/wt/spawn-argv-fix`（branch `spawn-argv-fix`）

## 根因

rc.12 真機 log 顯示 kill 引擎後 TIP detached spawn，引擎卻把整串
`--data "..." --settings "..."` 當成**一個** `--data` 的值：

```
[WARN jc_engine] 設定：檔案名稱、目錄名稱或磁碟區標籤語法錯誤 (os error 123)
jc-engine: 載入資料失敗：C:\ProgramData\jc-ime\data" --settings C:\Users\jeff\AppData\Local\jc-ime\settings.json:
資料根目錄不存在
```

Run 機碼的值本身是對的，末尾的 `\` 也是對的。問題在
`launch_detached()` 把命令列字串直接丟給 `CreateProcessW::lpCommandLine`。
`CreateProcessW` 的引號規則與 ShellExecute／Run 機碼不同：`\"` 被當成跳脫引號，
於是 `--data` 的值一路吃到後面的 `--settings`。

## 做法

採用任務書做法 A，不重造 `CommandLineToArgvW` parser：

- `std::process::Command::new(exe).args([...])` 逐項傳遞 argv，由標準函式庫在
  Windows 上產生正確跳脫的命令列再呼叫 `CreateProcessW`。
- `creation_flags((DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP).0)` 保留 detached
  行為；`spawn()` 後立刻 drop child handle，引擎仍是獨立行程。
- 不從 Run 機碼讀字串，改用 `module_dir()` 推 INSTALLFOLDER、用系統環境變數推
  data／settings 路徑，與 MSI 的 `EngineAutostart` 指向相同檔案。Run 機碼的值
  完全沒動， login 時的自動啟動仍由 Windows 照原樣處理。
- 3 秒冷卻、10×100 ms 等待 pipe、supervisor lockfile 擋重複、失敗只
  `logging::warn` 全部保留；`pipe_name`、transport、TSF Activate/focus 沒改。

## argv 怎麼拆

不再需要「拆字串」這一步。原本的單一命令列：

```text
"...\jc-engine.exe" --supervise --data "C:\ProgramData\jc-ime\data\" --settings "C:\...\settings.json"
```

改成 `EngineCommand { exe, args }`，`args` 是五個獨立元素：

| 索引 | 值 |
|---|---|
| 0 | `--supervise` |
| 1 | `--data` |
| 2 | `C:\ProgramData\jc-ime\data\` |
| 3 | `--settings` |
| 4 | `C:\Users\jeff\AppData\Local\jc-ime\settings.json` |

末尾 `\` 留在索引 2 的字串裡，`--settings` 是索引 3。`Command` 負責把它跳脫成
`CreateProcessW` 看得懂的形式，引擎端的 `CommandLineToArgvW` 會拆回這五個值。

## 測試

依共通規則只寫、只編譯，不執行 `cargo test`：

- `engine_command_keeps_data_and_settings_as_separate_args`：驗證 argv 順序與
  元素邊界。
- `engine_command_keeps_trailing_backslash_inside_data_arg`：驗證 Run 機碼那種
  末尾 `\` 的路徑仍只佔 `--data` 的一個 argv 元素，`--settings` 是下一個元素。

其餘既有測試（`spawn_cooldown_allows_one_spawn_per_window`、
`only_missing_or_busy_pipe_triggers_spawn`、`pipe_name_is_stable_within_process`、
`queue_marks_disconnected_by_default`）保留。

驗證指令：

```sh
cargo fmt -p jc-tsf
cargo check -p jc-tsf --target x86_64-pc-windows-msvc --all-targets
cargo clippy -p jc-tsf --target x86_64-pc-windows-msvc --all-targets -- -D warnings
```

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| n/a（Go 版沒有 TSF／detached spawn） | `crates/jc-tsf/src/engine.rs` | rewritten：`launch_detached()` 改用 `Command::args()` + `creation_flags`，移除 Run 機碼字串解析 |
| n/a | `docs/port/engine-on-demand.md` | 文件更新：補 lpCommandLine 引號規則差異與 argv 做法 |

## 真機驗證

1. 安裝 MSI，登入後確認打字正常（`tsf.log` 有「已連上引擎 pipe」）。
2. 工作管理員結束所有 `jc-engine.exe`（supervisor 與子行程都 kill）。
3. `Get-Process jc-engine` 沒有行程，`Get-ChildItem \\.\pipe\ | ? Name -like
   'jc-ime*'` 沒有 pipe。
4. 點一下記事本的文字框：`tsf.log` 先出現「已 detached spawn 引擎」，接著
   「按需啟動後已連上引擎 pipe」，打字立刻有注音候選。
5. 看引擎 log 的 `--data` 值是單純的路徑（例如
   `C:\ProgramData\jc-ime\data\`），不含 `--settings`，且不再出現「資料根目錄
   不存在」或 os error 123。
