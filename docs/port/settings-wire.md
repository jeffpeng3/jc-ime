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

## 熱重載

引擎啟動後由 `crates/jc-engine/src/config_watch.rs` 每 1 秒 `stat` 設定檔
（mtime + size）。檔案變了才重讀：

1. `config::load` → `config::effective`，沿用與啟動時相同的驗證。
2. 更新共用 `Registry`：keymap、`bindings`（action → chord）、effective 設定與
   `settings_revision`。
3. 各 worker 在下一次 `Engine::handle` 開頭呼叫 `sync_settings()`，用
   `Session::apply_config` 把新設定套到自己的 session；只換設定與衍生狀態，
   不清組字。
4. 成功才把 `settings_revision` +1，並在 log 印「重新載入設定（revision N）」。
   讀檔失敗、JSON 壞掉或 chord 不合法（例如沒有 Ctrl／Alt）時保留舊設定、
   記 warning，revision 不變，錯誤放在 `HealthInfo.settings_error`。
5. watcher 第一次 `stat` 的基準是 `config::load` 當下的戳記，不是資料載入完成
   後的 `stat`；啟動載入期間改的檔案會在 watcher 起跑後補上。

`Request::ApplySettings` 不再只設 `settings_pending`，而是立即走同一條重載
路徑並回傳新的 `HealthInfo`。沒有 TSF 連線時熱重載也照常運作。

`HealthInfo` 另外提供：

| 欄位 | 意義 |
|---|---|
| `bindings` | action → 正規化 chord（例如 `symbol_menu` → `ctrl+alt+,`） |
| `settings_revision` | 每次成功套用 +1；TSF 用它判斷是否要換本地 bindings |
| `settings_error` | 最近一次重載失敗的原因；成功後清空 |

TSF 在 `Ready` 拿到 bindings；`OnSetFocus`／`OnSetThreadFocus` 各刷新一次，
按鍵路徑最多每 5 秒用一次 `Health` 輪詢。只有 `settings_revision` 變了才換表；
連不上引擎時沿用上一次的表。因此改快捷鍵後，引擎 1 秒內套用，TSF 的本地表
最多 5 秒或下一次取得焦點時跟上。

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
| `mixed_input` | 已從 JSON 契約移除；舊檔含這個欄位時直接忽略，引擎一律混打並設 `style.mixed_input = true` | 已移除（舊檔忽略） |
| `english_autocomplete` | `SessionConfig::english_autocomplete` | 生效 |
| `esc_clear` | 引擎沒有對應欄位；核心的 Escape 一律清組字，是否轉送 Esc 由外殼決定 | 本版忽略（外殼） |
| `space_selects` | 已停用；`style.space_inserts_space` 固定 `true`（空白只插空格），設定檔明確寫 `true` 時記一次警告 | 已停用（舊檔相容） |
| `advance_after_select` | 選字後游標位置；核心提交後不控制游標 | 本版忽略（外殼） |
| `toggle_key`／`left_shift` | 中英切換已固定為單按左／右 Shift；TSF 送 `{"op":"toggle"}`，引擎在目前語言與 En 間切換。欄位保留給舊檔相容 | 已移除 UI（舊檔忽略） |
| `start_mode` | 仍保留設定值，但中文使用者的啟動模式不再受它影響；`--mode` 覆寫仍優先 | 本版忽略（中文） |
| `primary_language` | 沒給 `--mode` 且為 `ja` 時啟動 `ja`，其餘中文一律 `mixed`（不再有 `zh`） | 生效 |
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
| `output_variant`／`output_simplified` | 舊 `output_simplified` 遷移成 `output_variant`；`effective.output_simplified` 在引擎啟動時設 session 初始旗標。轉換器載入成功才生效，之後由 `output_simplified` op 即時切換 | 生效（需 `opencc/`） |
| `simplified_hotkey` | 簡體輸出的外殼快捷鍵 | 本版忽略（外殼） |
| `keybinds` | `symbol_menu`／`emoji_palette` 的 chord；引擎解析後呼叫 `Session::command`，TSF 只轉發 | 生效（熱重載） |
| `all_symbols_halfwidth` | 不直接寫 `SessionConfig`；只經由 `effective_ctrl_punctuation`／`effective_toneless` 影響 | 間接生效 |
| `caps_lock_english` | Caps Lock 直接打英文；外殼決定按鍵怎麼送 | 本版忽略（外殼） |
| `ja_input_method` | 日文只有羅馬字（`RomajiInput`），沒有直接かな | 本版忽略 |
| `ja_ambiguous` | 解碼器在建構時固定 `both`，`SessionConfig` 沒有欄位 | 本版忽略 |
| `ja_symbols` | 候選是否列記号／顔文字；解碼器開關未接線 | 本版忽略 |
| `ja_model` | 引擎不讀；神經轉換由 `ja_neural_mode` 決定，UI 已移除，欄位保留給舊檔相容 | 已移除 UI（舊檔忽略） |
| `model_mode`／`model_residency_mb`／`allow_placeholders` | 載入器啟動即載入固定資料，不做常駐調度或 placeholder | 本版忽略 |
| 未知欄位 | 由 `jc-settings` 原樣保留，引擎不讀 | 不適用 |

### `Settings` 裡沒有對應的欄位

- `max_candidates`：`Settings` 沒有這個欄位（未知鍵只會原文保留）。本版固定用
  `jc_core::session::DEFAULT_MAX_CANDIDATES`（5），並在 `--print-settings` 印出。
- `esc_clear`、`advance_after_select`、`mode_memory`、`caps_lock_english`：
  Go 版把這些放在 `eng`，Rust 核心的 `SessionConfig`／`InputStyle` 沒有同名欄位，
  屬於外殼（TSF／CLI 前端）的按鍵分派與游標行為，本版只輸出、不套用。
- `mixed_input` 已從 `Settings` struct、序列化與 `keys()` 移除；讀到舊檔時直接
  丟棄，不進 `extra`、不產生警示、不寫回。`input_profile.values` 裡的同名鍵也
  一併忽略。引擎一律對中文使用者用混打模式。
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
- `crates/jc-engine/src/lib.rs`：`ctrl+alt+,` 命中 keymap、未綁定回
  `handled=false`、設定檔變更後 revision +1 且不清組字、`ApplySettings` 立即
  生效、壞檔保留舊設定並回報 `settings_error`。
- `crates/jc-settings/src/keybinds.rs`：chord 解析、重複綁定與 round-trip。
- `scripts/verify-keybinds.sh`：起引擎、送快捷鍵、改設定檔後確認熱重載。
