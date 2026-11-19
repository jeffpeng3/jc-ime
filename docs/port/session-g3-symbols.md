# session-g3-symbols：符號與選單群組

上游：`/mnt/work/jc/ref/0c-ime` `8f443af`，`core/session/` 的
`symbols.go`（374）、`palette.go`（339）、`symbol_palette.go`（122）、
`all_symbols.go`（51）。base 是 `session-core-split`（`session/mod.rs` ＋
`session/core.rs`）。

本輪只補符號／表情選單與最近使用的缺口。`zh-symbols` 留下的行為保持原樣，
沒有重寫；需要游標或字面區間編輯的部分標 deferred，等 `core`／`segments`
群組到位再接。

## 逐檔對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/session/symbols.go` | `crates/jc-core/src/session/symbols.rs`（`GROUPS`、`UNITS`、`home`、`alternatives`）、`crates/jc-core/src/symbols.rs`（分類目錄 `TABS`）、`session/core.rs`（選取與記錄） | partial：資料層與目錄 ported；`↓` 同組變體編輯 deferred（依賴 `core` 的字面區間編輯，`insertLiteral`／`literalBounds` 一帶） |
| `core/session/palette.go` | `crates/jc-core/src/session/symbols.rs`（提示列 `menu_title`）、`session/core.rs`（`menu_items`、`menu_key`、`remember_text`）、`crates/jc-core/src/emoji.rs`（`palette`、`rank`） | rewritten：選取、最近使用、提示列 ported；格子移動與片語分頁 deferred（見下） |
| `core/session/symbol_palette.go` | `crates/jc-core/src/session/symbols.rs`（`RECENT` 分頁提示列）、`session/mod.rs`（`open_symbol_menu`、`open_symbol_tab`）、`session/core.rs`（分頁切換、`remember_symbol` 等價路徑） | rewritten：分頁、頭尾相接、最近分頁 ported；`openSymbolMenuAt`（組字中 `↑` 定位）deferred |
| `core/session/all_symbols.go` | 無 | deferred：屬 `input_style` 群組的「標點寬度：所有符號鍵打半形」設定，依賴 `input_style.rs` 與 `IdleKeys` |

## 模組命名

`session/symbols.rs` 與 crate 的 `crates/jc-core/src/symbols.rs` 同名。`mod.rs`
不再 `use crate::symbols;`，改由 `session::symbols` 重新匯出 `TABS`、`RECENT`、
`clean_recent`、`tab_items`、`contains`、`width_variant`、`MAX_RECENT`；
`core.rs` 的 `symbols::...` 寫法與既有測試都不用動。

## 已有 vs 新增

| 行為（上游） | 這一輪之前 | 本輪 |
|---|---|---|
| 反引號開符號選單，有紀錄時從「最近」開 | 有（`open_symbol_menu`、`menu_tab_names`） | 不動 |
| `Tab`／`Shift＋Tab` 換分頁、頭尾相接、換分頁回到第一頁 | 有（`menu_key`） | 不動，補測試 |
| 最近分頁只放一頁、上限 `MaxRecent`＝10、丟掉不在目錄的項目 | 有（`symbols::clean_recent`、`MAX_RECENT`、`menu_items`） | 不動，補上限測試 |
| 選符號記最近（常用＋最近排序），選單選取不學詞、不記表情 | 有（`remember_text`、`emoji::rank`、`learn_candidate`） | 不動 |
| `Esc`／`Backspace`／`Delete` 只關選單、不刪組字 | 有（`menu_key`） | 不動 |
| 等號開表情、第一項是半形等號、選它不算最近使用 | 有（`press_char`、`menu_items`） | 不動 |
| 表情分頁的最後選擇；只在真的選了東西時更新 | 有（`palette_tab`） | 不動 |
| 提示列：`標點 · Tab → 括號引號`、`Tab → 顏文字` | **缺** | **新增** `Session::menu_title()`，補測試 |
| 同組符號表（`↓` 的變體來源）、多字元符號清單 | **缺** | **新增** `symbols::GROUPS`、`UNITS`、`home`、`alternatives`，補測試 |
| 每個分組符號都在選單目錄裡（上游 `TestSymbolMenuKeepsEveryGroupSymbol`） | 缺（沒有分組表可對照） | 新增測試，現在會擋目錄漏項 |
| 穩定候選 ID（`symbols.ID`、片語 `phraseIDBase`） | 無 | n/a：`Candidate` 沒有 ID 欄位，`Session::select` 以索引選取 |
| 私人欄位不列「最近」、暫停學習只不記 | 無此概念 | n/a：`Session` 沒有 private／paused 狀態，個人資料由宿主過濾 |

`Snapshot` 沒有提示列欄位，也由 core 群組凍結，所以提示列走公開方法
`menu_title()`，不動 `Snapshot`。

## Deferred

| 行為 | 依賴 |
|---|---|
| `↓` 在字面符號上列出同組變體並替換（`selectLiteral`、`literalBounds`、`literalStep`） | `core`：字面區間的游標與編輯；資料層（`GROUPS`／`UNITS`）已就位 |
| 組字中 `↑` 在符號上開選單並定位（`openSymbolMenuAt`） | `core`：游標位置與替換區間；`segments`：候選呈現 |
| `↑↓←→` 依候選欄位移動、跨頁（`gridMove`、`candidateColumns`） | `segments`：`CandidateColumns`／`displayColumns` 與翻頁；`Snapshot` 目前沒有欄位數 |
| 「我的片語」分頁、片語標籤截斷與換行（`phrases`、`phraseLabel`） | `misc`：`personal` 的片語（`Preview02` 開關） |
| 標點寬度（`all_symbols.go`、`halfwidthShift`） | `input_style`：標點寬度設定與 idle keys |

## 驗證

```sh
cargo check -p jc-core --all-targets
cargo fmt -p jc-core
```

測試只編譯不執行（依共通規則）；新增測試在
`crates/jc-core/src/session/symbols.rs` 的 `tests`：

- `test_symbol_menu_titles`
- `test_palette_titles`
- `test_symbol_groups_cover_menu_catalog`
- `test_symbol_groups_home_and_alternatives`
- `test_symbol_units_are_multichar`
- `test_symbol_recent_is_one_page`
