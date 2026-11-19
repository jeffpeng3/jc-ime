# jc-settings port 對照表

前身：`internal/settings`（2,454 行）與 `internal/modelsets`（1,706 行），Go main
`8f443af`。這輪只做資料模型、驗證與檔案格式，不含 UI 與任何作業系統 API。

## 實作摘要

- 平台差異一律由呼叫端傳入 `goos` 字串（`Settings::default_for_platform`、
  `read`、`load`、`set`、`profile_action`…）。crate 內不讀 `runtime.GOOS`，
  也不組 `LOCALAPPDATA`；檔案位置由 `paths::settings_path(資料根目錄)` 決定。
- 設定檔的 JSON 保留策略照前身：不認得的欄位與已知欄位的未知值存進
  `Settings::extra`，存檔時原樣寫回；schema 比本版新時只讀。`Serialize` 以
  固定順序輸出，未知欄位排在已知欄位之後（鍵排序）。
- 欄位值驗證走 `Settings::set`，錯誤是 `SettingsError::Set(SetError)`；
  `SetError::code()` 提供 `unknown_key`、`invalid_value`、`locked_by_profile`、
  `profile`，對照前身的 `CodeUnknownKey`、`CodeInvalidValue`。
- 模糊音直接接 `jc-core::zh::fuzzy`，輸入習慣規則接
  `jc-core::inputstyle`，不重複實作規則表。
- 模型包只做 manifest 解析、結構／release 驗證、字典包雜湊比對、檔案大小與
  SHA-256 核對、`active.json` 安裝與再驗證；下載與 pin 不在本 crate。
- `write_atomic` 移植 `internal/atomicfile` 的用法（同目錄暫存檔、`sync`、
  改名），Unix 上檔案 0600、目錄 0700。

已知差異（皆為刻意）：

- 前身的 `Item`／`Items`／`LanguageItems`／`ValueLabels`／`Description`／
  `DisabledReason`／`DisabledBy` 是設定程式的介面契約，屬 UI，未移植。
- `Change`／`AdoptBothShift` 前身在前身 `internal/filelock` 的寫入鎖內執行；
  本 crate 沒有跨 process 鎖，公開文件要求呼叫端自行持有
  `update/settings-write.lock`。
- `AdoptBothShift` 改寫檔案時，前身逐字保留成員順序；Rust 版用
  `serde_json::Map` 重寫，鍵會排序，值不變。
- `quarantine` 的時間戳改成由呼叫端傳 `YYYYMMDDTHHMMSSZ`（另有
  `quarantine_now`），避免拉入日期時間函式庫。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/settings/settings.go` | `crates/jc-settings/src/settings.rs` | rewritten：設定型別、預設、JSON 讀寫、遷移、驗證齊全；`Item` 系列與說明文字屬 UI，未移植 |
| `internal/settings/settings_test.go` | `crates/jc-settings/src/tests.rs` | rewritten：改成 table-driven 與 `Result` 斷言；`Items` 契約的測試移除 |
| `internal/settings/paths.go` | `crates/jc-settings/src/paths.rs` | rewritten：資料根目錄由呼叫端傳入，不再讀 `GOOS` / 家目錄 |
| `internal/settings/paths_linux_test.go` | `crates/jc-settings/src/tests.rs` | rewritten：新舊路徑、權限、符號連結與寫入失敗都保留 |
| `internal/settings/profiles.go` | `crates/jc-settings/src/profiles.rs` | rewritten：保留快照、驗證、鎖定與操作；`ProfileView`／`ProfileItem` 的說明文字屬 UI，未移植 |
| `internal/settings/profiles_test.go` | `crates/jc-settings/src/tests.rs` | rewritten：遷移、複本、排列限制、未知 metadata 與鎖定都保留；`Items` 相關測試移除 |
| `internal/settings/profile_rules.go` | `crates/jc-settings/src/profile_rules.rs` | ported |
| `internal/settings/change.go` | `crates/jc-settings/src/change.rs` | rewritten：讀改寫邏輯保留，寫入鎖由呼叫端負責 |
| `internal/settings/change_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：值沒變不重寫、未知欄位保留、損毀／較新版本不覆寫；鎖等待測試 n/a |
| `internal/settings/both_shift.go` | `crates/jc-settings/src/both_shift.rs` | rewritten：遷移邏輯保留；輸出鍵排序，無檔案鎖 |
| `internal/settings/both_shift_test.go` | `crates/jc-settings/src/tests.rs` | rewritten：只保留一次遷移與 marker 的行為測試 |
| `internal/settings/model_settings_test.go` | `crates/jc-settings/src/tests.rs` | rewritten：預設、舊檔遷移、持久化、讀不懂的值保留 |
| `internal/settings/advance_after_select_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：併入布林設定 table |
| `internal/settings/all_symbols_halfwidth_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：三選一持久化、殘值 AND、`effective_ctrl_punctuation`／`effective_toneless` |
| `internal/settings/candidate_cursor_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：平台預設與持久化 |
| `internal/settings/caps_lock_english_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：併入布林設定 table |
| `internal/settings/cjk_latin_space_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：併入布林設定 table |
| `internal/settings/ctrl_punctuation_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：持久化與 `effective_ctrl_punctuation` |
| `internal/settings/disabled_by_test.go` | — | n/a：`DisabledBy`／`DisabledReason` 是設定程式頁面契約（UI） |
| `internal/settings/english_autocomplete_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：併入布林設定 table |
| `internal/settings/esc_clear_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：併入布林設定 table |
| `internal/settings/ja_ambiguous_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：`effective_ja_ambiguous` 與持久化 |
| `internal/settings/keep_toggle_key_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：缺檔、舊 `left_shift`、未知值、較新 schema、壞檔 |
| `internal/settings/keyboard_layout_test.go` | `crates/jc-settings/src/tests.rs` | rewritten：排列限制與風格複本 |
| `internal/settings/language_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：`language`→`primary_language` 遷移、日文選項；`Languages`／`Capabilities` 屬 UI 未移植 |
| `internal/settings/left_shift_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：平台預設、舊欄位推出 `toggle_key`、未知值保留 |
| `internal/settings/mixed_profile_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：English profile 舊鍵移除、新欄位優先 |
| `internal/settings/mixed_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：併入布林設定 table |
| `internal/settings/output_simplified_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：舊欄位遷移到 `output_variant` |
| `internal/settings/palette_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：`candidate_layout`／`emoji_shortcut` 未知值保留 |
| `internal/settings/preview_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：retired key 存檔移除，`on` 成功、`off` 回 invalid_value |
| `internal/settings/selection_keys_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：併入選項型 table |
| `internal/settings/shift_symbol_halfwidth_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：預設關、讀不懂保留、風格不鎖 |
| `internal/settings/simplified_hotkey_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：併入布林設定 table |
| `internal/settings/space_selects_mixed_test.go` | — | n/a：停用原因的 UI 契約；混打本身的引擎行為在 `jc-core` |
| `internal/settings/space_selects_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：併入布林設定 table |
| `internal/settings/start_mode_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：預設、`start_mode_locked`、讀不懂原樣保留；Items 契約 n/a |
| `internal/settings/toggle_key_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：平台預設、`left_shift` 連動 |
| `internal/settings/toneless_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：併入布林設定 table 與 `effective_toneless` |
| `internal/settings/zhuyin_error_hint_test.go` | `crates/jc-settings/src/features_tests.rs` | rewritten：併入布林設定 table |
| `internal/modelsets/manifest.go` | `crates/jc-settings/src/modelset/manifest.rs` | ported：解析、結構／release 驗證、字典雜湊、檔案核對、張量契約 |
| `internal/modelsets/manifest_test.go` | `crates/jc-settings/src/modelset/tests.rs` | rewritten：合成 fixture；正式 repo lock 的測試改成合成 release fixture |
| `internal/modelsets/install.go` | `crates/jc-settings/src/modelset/install.rs` | ported：`active.json`、暫存樹搬移、重裝保護、安裝後再核對 |
| `internal/modelsets/fetch.go` | — | n/a：Hugging Face HTTP 下載與快取，屬網路流程，不在資料模型這一輪 |
| `internal/modelsets/fetch_test.go` | — | n/a：同上 |
| `internal/modelsets/pin.go` | — | n/a：pin 需要下載遠端檔案；驗證用的 `valid_revision`／`HfSource::pinned` 已移植 |
| `internal/modelsets/placeholder_test.go` | — | n/a：需要 `OC_PLACEHOLDER_MODELS` 指向模型檔，資料不在 `jc-ime-data`；placeholder 規則由 `modelset/tests.rs` 的合成模型包覆蓋 |
| `internal/filelock`（被 settings 使用） | — | n/a：跨 process 鎖不在本 crate；`change`／`both_shift` 的文件要求呼叫端自行持有鎖 |
| `internal/atomicfile`（被 modelsets 使用） | `crates/jc-settings/src/atomic_file.rs` | rewritten：只保留原子寫檔與 JSON 寫入 |
