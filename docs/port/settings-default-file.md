# settings-default-file：首次開啟即可修改的預設設定檔

Worktree：`/mnt/work/jc/wt/settings-default-file`（branch `settings-default-file`）

## 根因

`settings.json` 不存在時 `jc_settings::read` 回傳 `fresh_settings`，`read_only` 是
`false`，所以「必須先複製」不是唯讀模式造成。`fresh_settings` 會選用內建 `0c`
風格，`Settings::profile_locked` 因此鎖住 14 個 `input_profile` 管理的鍵，UI 把
項目反灰並要求先按「複製成自訂輸入習慣」。

`settings.rs` 的 `parsed.read_only()` 只處理較新 schema 與不合法的風格快照，是
防止覆寫新格式的保護，與缺檔無關，這次保留。

## 做法

B + C：

1. `SettingsView::load` 在缺檔時把預設值存成 `settings.json`，並把內建風格轉成
   自訂快照，第一次開啟就能改。
2. 輸入頁移除鎖定標示與「複製」按鈕；既有檔案若還用內建風格，第一次修改時
   由 `apply` 自動轉成自訂快照後再寫入。

中英混打開關同時從 UI 移除。DP 已是唯一分詞路徑，引擎對中文使用者一律進
`Mode::Mixed`。`mixed_input` 已從 JSON 契約移除：舊檔含這個欄位時直接忽略，
不進 `extra`、不寫回，`input_profile.values` 裡的同名鍵也一併丟棄。

## Go 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/settings/settings.go`（讀寫、缺檔預設） | `crates/jc-settings/src/settings.rs` | 已在 settings-core 移植；本 task 只調整 Windows 混打預設 |
| `internal/settings/profiles.go`（風格快照與鎖定） | `crates/jc-settings/src/profiles.rs` | rewritten；風格快照不再含 `mixed-input` |
| `platforms/settings/ui/input-styles.js`（鎖定與複製） | `crates/jc-settings-ui/src/ui.rs`、`state.rs` | rewritten；移除複製，改為首次啟動建立可改檔 |
| `platforms/settings/ui/pages.js`（輸入頁項目） | `crates/jc-settings-ui/src/options.rs` | rewritten；移除 `mixed-input` 開關 |

## 文件

- `docs/install.md`：使用者設定第一次開啟設定程式就建立。
- `docs/port/settings-ui.md`：移除「複製」與混打開關的說明。
- `docs/port/settings-wire.md`：`mixed_input` 標為已移除、舊檔忽略；`start_mode`
  不再決定中文啟動模式。
