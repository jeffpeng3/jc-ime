# settings-wire：把設定接進引擎

Worktree：`/mnt/work/jc/wt/settings-wire`（branch `settings-wire`）

## 目標

`jc-engine` 啟動時讀 `settings.json`，轉成 `jc_core::session::SessionConfig`；
CLI 參數優先於檔案。壞檔、缺檔與單一欄位值不合法都只記警告，不讓引擎啟動
失敗。

## Go 對照

Go 參考實作把設定接到引擎的程式碼集中在 server 啟動流程，沒有獨立模組。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/settings/settings.go`（`Read`／`Load`、`Report.Notes`） | `crates/jc-settings/src/settings.rs`（`read`、`load`） | 已在 settings-core 移植；本 task 只使用 |
| `cmd/0c-server/main.go`（讀設定、逐欄寫進 `eng`） | `crates/jc-engine/src/config.rs`（`load`、`effective`、`session_config`） | rewritten；改成純函式，CLI 覆寫用 `Overrides` 表示 |
| `cmd/0c-server/main.go`（`eng.SpaceInsertsSpace`、`eng.EscKeepComposition`、`eng.AdvanceAfterSelect`、切換鍵） | `crates/jc-core/src/session/input_style.rs`（`InputStyle`） | 部分 ported：`space_selects` 有對應，Esc／選字後游標／切換鍵在 Rust 屬於外殼，核心沒有欄位 |
| `cmd/0c-server/main.go`（`-neural-*`、`-punct-*` 明確參數優先） | `crates/jc-engine/src/main.rs`（`--neural`、`--mode`） | rewritten；`--settings`、`--print-settings` 是新增的除錯入口 |

## 日文轉換預設

`ja_conversion` 預設 `live`：

- `live`：打字期間顯示假名，停止輸入 300 ms 後做一次完整轉換（統計＋現有
  門檻＋Jinen）。`modern-500` top-1 92.2%（與整句模式相同），每題推論 0.21 次，
  停頓到轉換完成 p95 50 ms。
- `sentence`：與舊版相同，空白／Enter 才轉；`modern-500` 92.2%、
  `--neural quality` 93.6%。

CLI 對照：`jc-cli eval ja --no-live` 走 `sentence`，其餘日文評測預設 `live`。
舊 `settings.json` 沒有 `ja_conversion` 時視為 `live`；只有舊
`ja_live_conversion: false` 時視為 `sentence`。

## 啟動流程

1. `--settings <path>` 有給就讀它；否則用
   `jc_settings::paths::settings_path(data_root)`（Linux 標準目錄會換到 `0c/`）。
   沒有資料根目錄（`--allow-empty-data` 且沒給 `--settings`）時不讀檔，用平台
   預設值。
2. `jc_settings::settings::read` 一次讀完；缺檔不警告，壞檔／讀不到記一則警告，
   `Report.Notes`（單欄位退回或保留）逐條記警告。
3. `config::effective` 把檔案值與 CLI 覆寫合併成 `EffectiveSettings`。
4. `config::session_config` 只覆寫 `SessionConfig` 中由設定決定的欄位；詞典、
   語言模型、神經 runtime 等資料欄位保留載入器的值。
5. `--print-settings` 在第 3 步後直接輸出 JSON 並結束，不載入資料。

## 設定欄位 → 行為

「引擎」欄指 `SessionConfig`；「外殼」表示需要 TSF／CLI 端處理，引擎只印在
`--print-settings` 裡。

| `Settings` 欄位 | 影響 | 狀態 |
|---|---|---|
| `ja_neural_mode` | `NeuralConfig`：`auto` ＝ 低呼叫門檻（`gap < 472.5` 且平均候選數 > 5.98）、`quality` ＝ 只看 `gap < 460`、`off` ＝ `enabled=false` | 生效 |
| `ja_rerank` | `SessionConfig::ja_rerank`；預設 `false`，開啟後日文候選走 reranker | 生效 |
| `selection` | 舊欄位：`statistical` 遷移成 `ja_neural_mode = off`、`neural` 遷移成 `auto`；兩者都在且語意不同時以 `ja_neural_mode` 為準並記一則警告，一致時不警告 | 遷移用 |
| `punctuation` | `punct_mark = Some('，')`；本版以全形逗號代表「送出前補一個標點」 | 生效（簡化） |
| `document_context` | `SessionConfig::document_context` | 生效 |
| `mixed_input` | 啟動模式改 `mixed`；`style.mixed_input` | 生效 |
| `english_autocomplete` | `SessionConfig::english_autocomplete` | 生效 |
| `esc_clear` | 引擎沒有對應欄位；核心的 Escape 一律清組字，是否轉送 Esc 由外殼決定 | 本版忽略（外殼） |
| `space_selects` | `style.space_inserts_space = !space_selects` | 生效 |
| `advance_after_select` | 選字後游標位置；核心提交後不控制游標 | 本版忽略（外殼） |
| `toggle_key`／`left_shift` | 中英切換鍵；引擎只收按鍵，切換由外殼送出 `toggle`／`--mode` | 本版忽略（外殼） |
| `start_mode` | 沒給 `--mode` 時的啟動模式：`english` → `en`，否則 `zh` | 生效 |
| `primary_language` | 沒給 `--mode` 且為 `ja` 時啟動模式改 `ja` | 生效 |
| `selection_keys` | `style.selection_keys` | 生效 |
| `emoji_shortcut` | `style.emoji_shortcut` | 生效 |
| `shift_symbol_halfwidth` | `style.shift_symbol_halfwidth`；也參與 `ctrl_punctuation`、`toneless` 的收斂 | 生效 |
| `ctrl_punctuation` | `SessionConfig::ctrl_punct`（`effective_ctrl_punctuation`，`all_symbols_halfwidth` 時強制開） | 生效 |
| `cjk_latin_space` | `SessionConfig::cjk_latin_space` | 生效 |
| `zhuyin_error_hint` | `SessionConfig::zhuyin_error_hint` | 生效 |
| `toneless` | `SessionConfig::toneless`（`effective_toneless`，所有符號半形時暫停） | 生效 |
| `fuzzy` | `SessionConfig::fuzzy_pairs`；`off` 的配對不加入 | 生效 |
| `ja_conversion` | `ja_live.enabled` 與轉換方式：`live`（預設，延遲整句轉換）／`sentence`（手動整句轉換） | 生效 |
| `ja_live_conversion` | 舊欄位：`true` → `ja_conversion = live`、`false` → `sentence`；兩者都在時以 `ja_conversion` 為準 | 遷移用 |
| `ja_learning` | `SessionConfig::learning`；本版只有一個學習開關 | 生效 |
| `input_profile` | `style.rules = profile_rules(goos)`；風格的鍵位規則 | 生效 |
| `mode_memory` | 切換視窗後的中英模式；引擎是單一行程 | 本版忽略（外殼） |
| `candidate_cursor` | 修改字詞的位置；候選窗由外殼畫 | 本版忽略（外殼） |
| `candidate_layout` | 候選排列；外殼 UI | 本版忽略（外殼） |
| `emoji_equals_first` | `=` 是否排在 Emoji 選單第一項；外殼 UI | 本版忽略（外殼） |
| `keyboard_layout` | 本版固定標準大千鍵盤（`ZhInput::standard`） | 本版忽略 |
| `output_variant`／`simplified_hotkey` | 引擎不載入 OpenCC；簡體輸出只在 `jc-cli` 評測路徑 | 本版忽略 |
| `all_symbols_halfwidth` | 不直接寫 `SessionConfig`；只經由 `effective_ctrl_punctuation`／`effective_toneless` 影響 | 間接生效 |
| `caps_lock_english` | Caps Lock 直接打英文；外殼決定按鍵怎麼送 | 本版忽略（外殼） |
| `ja_input_method` | 日文只有羅馬字（`RomajiInput`），沒有直接かな | 本版忽略 |
| `ja_ambiguous` | 解碼器在建構時固定 `both`，`SessionConfig` 沒有欄位 | 本版忽略 |
| `ja_symbols` | 候選是否列記号／顔文字；解碼器開關未接線 | 本版忽略 |
| `ja_model` | 引擎不讀；神經轉換由 `ja_neural_mode` 決定，這個值只保留給 UI | 本版忽略 |
| `model_mode`／`model_residency_mb`／`allow_placeholders` | 載入器啟動即載入固定資料，不做常駐調度或 placeholder | 本版忽略 |
| 未知欄位 | 由 `jc-settings` 原樣保留，引擎不讀 | 不適用 |

### `Settings` 裡沒有對應的欄位

- `max_candidates`：`Settings` 沒有這個欄位（未知鍵只會原文保留）。本版固定用
  `jc_core::session::DEFAULT_MAX_CANDIDATES`（5），並在 `--print-settings` 印出。
- `esc_clear`、`advance_after_select`、`toggle_key`／`left_shift`、`mode_memory`、
  `caps_lock_english`：Go 版把這些放在 `eng`，Rust 核心的 `SessionConfig`／
  `InputStyle` 沒有同名欄位，屬於外殼（TSF／CLI 前端）的按鍵分派與游標行為。
  任務書要求不改 `jc-core` 的 `SessionConfig`，因此本版只輸出、不套用。
- 其餘列為「本版忽略」的欄位，理由如表。

## 優先序

1. CLI：`--mode`、`--neural`（`on`／`off`；`on` 對應 `auto`，`off` 對應 `off`）。
2. `settings.json` 的 `ja_neural_mode`（`auto`／`quality`／`off`）。
3. 舊欄位 `selection` 的遷移結果。
4. 平台預設值（`Settings::default_for_platform`；缺檔時是 `fresh_settings`，
   已套用內建 `0c` 輸入習慣）。

`--max-frame` 是協定層參數，不屬於 `SessionConfig`，不受設定檔影響。

## `--print-settings`

輸出 `PrintedSettings`：`settings_path`、`settings_state`（`ok`／`missing`／
`corrupt`／`unreadable`）與展開後的生效值。欄位包含 `mode`、`max_candidates`、
`selection`、`neural`、`ja_neural_mode`、`ja_rerank`、`mixed_input`、
`english_autocomplete`、`esc_clear`、`space_selects`、`advance_after_select`、
`toggle_key`、`left_shift`、`start_mode`、`document_context`、`punctuation`、
`punct_mark`、`toneless`、`zhuyin_error_hint`、`ctrl_punctuation`、
`cjk_latin_space`、`fuzzy_pairs`、`ja_live`、`learning`、`selection_keys`、
`emoji_shortcut`、`shift_symbol_halfwidth`。`selection` 已反映 `--neural` 與
`ja_neural_mode`，所以與 `neural` 一致。

## 測試

- `crates/jc-engine/src/config.rs`：完整設定逐欄比對、缺檔、壞 JSON 單一警告、
  非法列舉值、CLI 覆寫、`--print-settings` JSON 與 `SessionConfig` 一致。
- `crates/jc-engine/tests/settings.rs`：真的起 `jc-engine
  --print-settings --allow-empty-data`，驗證檔案＋CLI 合併、缺檔與壞檔的狀態與
  警告。
- `crates/jc-engine/src/main.rs`：`--settings`、`--neural`、`--print-settings`
  的參數解析。
