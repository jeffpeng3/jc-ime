# zh-symbols：符號、emoji、全形數字、標點

參考實作：`/mnt/work/jc/ref/0c-ime` main `8f443af`。

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/symbols/catalog.go` | `crates/jc-core/src/symbols.rs` | ported：分頁目錄、`Locate`、`Contains`、`ID`、`CleanRecent` |
| `core/symbols/catalog_test.go` | `crates/jc-core/src/symbols.rs` 測試 | ported：`test_tabs_unique_and_ids_stable`、`test_clean_recent` |
| `core/emoji/catalog.go` | `crates/jc-core/src/emoji.rs` | ported：表情／顏文字目錄、`Contains`、`ID`、`CleanRecent`、`Palette` |
| `core/emoji/frecency.go` | `crates/jc-core/src/emoji.rs` | ported：`Stat`、`Decayed`、`SeedStats`、`Pick`、`Rank`、`CleanStats`、`KeepStats` |
| `core/emoji/frecency_test.go` | `crates/jc-core/src/emoji.rs` 測試 | ported：四個 frecency 測試 |
| `core/punct/punct.go` | `crates/jc-core/src/punct.rs` | ported：`Marks`、`Edit`、`Punctuator`、`Apply`、`ApplyJapanese` |
| `core/punct/punct_test.go` | `crates/jc-core/src/punct.rs` 測試 | ported：中文漢字邊界與日文假名／段尾規則 |
| `core/ja/mixed/width.go` | `crates/jc-core/src/symbols.rs` | ported：全形英數偵測與互轉 |
| `core/ja/mixed/width_test.go` | `crates/jc-core/src/symbols.rs` 測試 | ported：`test_width_conversion`、`test_width_variant`、`test_width_detectors` |
| `core/session/symbol_palette.go` | `crates/jc-core/src/session.rs` | rewritten：反引號開選單、Tab／Shift＋Tab 換分頁、「最近」分頁、數字鍵選取 |
| `core/session/palette.go` | `crates/jc-core/src/session.rs` | rewritten：等號開表情／顏文字分頁、最近排序；不搬片語、grapheme 編輯與 grid 導覽 |
| `core/session/symbols.go` | `crates/jc-core/src/session.rs`、`symbols.rs` | rewritten：只保留符號目錄成員檢查與插入；↓ 同組變體未接 |
| `core/session/symbol_palette_test.go` | `crates/jc-core/src/session.rs` 測試 | rewritten：`test_symbol_menu_tabs`、`test_symbol_menu_recent_tab`、`test_symbol_menu_keeps_every_keyboard_symbol` |
| `core/session/palette_test.go` | `crates/jc-core/src/session.rs` 測試 | rewritten：`test_emoji_palette_selection_and_recent`、`test_emoji_palette_equals_is_literal`；其餘依賴私人欄位、片語與 grid 的案例未搬 |
| `core/session/all_symbols.go`、`core/session/shift_symbol.go` | 無 | n/a：設定「所有符號鍵打半形／Shift＋符號打半形」不在本次 owned paths，`SessionConfig` 沒有對應選項 |
| `core/session/dash_ellipsis_test.go` | `crates/jc-core/src/symbols.rs`、`session.rs` 測試 | 部分：`……`／`——` 已在目錄，鍵盤 `<`、`^`、`_` 的映射沿用既有 `zh/keyboard.rs`；↓ 變體未接 |

## Session 接線

- 中文模式、沒有未完成注音時按 `` ` `` 開符號選單；有「最近」紀錄時從「最近」開。
- 中文模式按 `=` 開表情選單，第一項是字面等號，其後是最近使用的表情；Tab 換到顏文字。
- 選單開著時 Tab／Shift＋Tab 換分頁，數字鍵選取；選取後立即放進待提交區。
- 符號與表情的「最近」都使用 `emoji::Rank` 的常用＋最近排序，符號上限 10 項、表情 64 項。
- `SessionConfig::punct_mark` 改走 `punct::apply` 驗證，仍在最後一個漢字前插入。
- 英文模式的 ASCII 英數字候選附全形變體，選了就送出 `０-９`／`Ａ-Ｚ`／`ａ-ｚ`。
- 主機時鐘未接：`Session` 內呼叫 `emoji::Rank` 時 `now = 0`，排序與遲滯有效，時間衰減要由宿主另外呼叫 `emoji::Rank` 傳入秒數。

## 資料來源與授權

| 項目 | 來源 | 授權 |
|---|---|---|
| 符號分頁目錄 | 前身 `core/symbols/catalog.go` 手動整理 | AGPL-3.0-only |
| 表情／顏文字目錄 | 前身 `core/emoji/catalog.go` 手動整理 | AGPL-3.0-only |
| 日文記号 `symbols.tsv` | `/mnt/work/jc/jc-ime-data/ja-src/symbols/symbols.tsv` | AGPL-3（`legal/copyright`：本專案整理，未整檔移植 Mozc） |
| 日文顏文字 `kaomoji.tsv` | `/mnt/work/jc/jc-ime-data/ja-src/symbols/kaomoji.tsv` | AGPL-3（同上） |

- 資料檔測試用 `jc_testdata::require_data!("ja-src/symbols/...")`，缺檔時 skip。
- `data/lexicon/symbols/*`（任務書所稱「注音那份」）在 `/mnt/work/jc/ref/0c-ime/` 與 `/mnt/work/jc/jc-ime-data/` 都不存在，沒有自行編表；符號目錄以前身 Go 原始碼為準。
- 前身 emoji 套件沒有關鍵字詞典；本次照實移植選單與最近排序，沒有新增關鍵字→emoji 對照。
- TSV 沒有貼進程式碼；目錄與測試資料分離。
