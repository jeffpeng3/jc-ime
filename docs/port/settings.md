# 設定：檔案、引擎與設定程式

`settings.json` 是使用者設定的唯一來源。`jc-settings` 定義資料模型、驗證與原子
寫檔；`jc-engine` 啟動時讀取並套用到 `SessionConfig`；`jc-settings-ui` 是編輯
介面。三者不各自維護一份 JSON 契約。

## 檔案位置與優先序

- `--settings <path>` 有給就讀它；否則用 `jc_settings::paths::settings_path(資料根目錄)`。
- 沒有資料根目錄（`--allow-empty-data` 且沒給 `--settings`）時不讀檔，使用平台
  預設值。
- 優先序：CLI（`--mode`、`--neural`）→ `settings.json` 的 `ja_neural_mode` →
  舊欄位 `selection` 的遷移結果 → 平台預設。
- `--print-settings` 印出合併後的生效值與 `settings_state`
  （`ok`／`missing`／`corrupt`／`unreadable`），不載入資料。

缺檔不警告；壞檔、讀不到或單一欄位不合法只記警告，引擎照常啟動。未知欄位保存在
`Settings::extra`，存檔時原樣寫回；schema 比程式新時只讀。

## 熱重載

`crates/jc-engine/src/config_watch.rs` 每秒 `stat` 設定檔（mtime + size）。變更時
重讀並走與啟動相同的驗證，成功才更新：

1. `Registry` 的 keymap、bindings、effective 設定與 `settings_revision`。
2. 各 worker 在下一次請求開頭 `sync_settings()`，用 `Session::apply_config` 套到
   自己的 session；只換設定與衍生狀態，不清組字。
3. `settings_revision` +1；讀檔失敗、JSON 壞掉或 chord 不合法時保留舊設定，
   錯誤放進 `HealthInfo.settings_error`。
4. `Request::ApplySettings` 立即走同一條路徑並回傳新的 `HealthInfo`。

TSF 在 `Ready` 取得 bindings，取得焦點時刷新，按鍵路徑最多每 5 秒用 `Health`
輪詢。只有 `settings_revision` 變了才換表。

## 欄位與行為

| 欄位 | 影響 | 狀態 |
|---|---|---|
| `ja_neural_mode` | `auto`（低呼叫門檻）、`quality`（只看成本差）、`off` | 生效 |
| `ja_conversion` | `live`（預設，停頓後整句轉換）／`sentence`（手動整句） | 生效 |
| `ja_live_conversion` | 舊欄位，遷移成 `ja_conversion` | 遷移用 |
| `ja_rerank` | 日文 reranker；預設 `false` | 生效 |
| `ja_learning` | 日文個人學習 | 生效 |
| `fuzzy` | 模糊音配對；`off` 的配對不加入 | 生效 |
| `selection_keys`、`emoji_shortcut`、`shift_symbol_halfwidth` | 候選與符號行為 | 生效 |
| `ctrl_punctuation`、`toneless`、`cjk_latin_space`、`zhuyin_error_hint` | 注音輸入規則 | 生效 |
| `document_context` | 個人學習是否使用文件上下文 | 生效 |
| `english_autocomplete` | 英文補全 | 生效 |
| `keybinds` | 符號／表情選單 chord | 生效（熱重載） |
| `caps_lock_english` | 由外殼套用；引擎在 health 回報能力 | 外殼 |
| `punctuation`／`punct_mark` | 送出前補全形逗號 | 生效（簡化） |
| `output_variant` | 簡繁輸出；需 `opencc/` | 生效 |
| `primary_language` | 沒給 `--mode` 時選日文或混打 | 生效 |
| `start_mode`、`toggle_key`、`left_shift`、`mixed_input` | 舊欄位；中英切換已固定為單按 Shift，中文一律混打 | 舊檔相容／忽略 |
| `esc_clear`、`advance_after_select`、`mode_memory`、`candidate_*`、`simplified_hotkey` | 屬外殼行為，核心沒有對應欄位 | 外殼 |
| `keyboard_layout` | 固定標準大千鍵盤 | 忽略 |
| `max_candidates` | 不是設定欄位；固定 `DEFAULT_MAX_CANDIDATES = 200` | 固定值 |

## 設定程式

`jc-settings-ui` 是 GPUI 原生視窗，讀寫全部經過 `jc-settings`。分頁與項目：

- 注音：模糊音四組、選字模型、學習、模型狀態（唯讀）。
- 日文：神經轉換模式、逐字変換、個人詞庫開關。
- 輸入：空白鍵行為、符號／表情選單快捷鍵。
- 資料：資料根目錄、設定檔、manifest 與模型狀態（唯讀，可重新檢查）。

快捷鍵欄位即時驗證 chord，存檔後由引擎熱重載接手。已移除的設定項不再顯示，JSON
欄位保留給舊檔相容。模型頁依 `jc-data` 的 catalog 解析路徑，只列仍使用或隨安裝包
散布的角色；缺檔顯示「需下載」。

第一次啟動就建立 `settings.json`，之後每次變更立即存檔。寫入走
`jc_settings::atomic_file`：同目錄暫存檔、`sync`、改名，不會留下半個檔案。壞檔或
較新 schema 不覆寫。`jc-settings` 沒有跨行程鎖；同時寫入時是 last-writer-wins，
呼叫端若需要互斥應持有 `<資料根目錄>/update/settings-write.lock`。
