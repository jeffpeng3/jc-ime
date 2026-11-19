# zh-golden-rest：zhconv 與 mixed ASCII 推斷

現況：`jc-cli eval zh-golden` 通過 **41/45**。剩下的 4 例全部是 M4 個人化
（`learning-*`、`undo-*`），本次沒有動期望值，也沒有碰學習層。

| 階段 | 通過 |
|---|---|
| 進 branch 前（`main` 的 zh-golden 文件） | 35/45 |
| 補完 zhconv 與 mixed ASCII 推斷 | 41/45 |

## zhconv

對照 `core/zhconv/zhconv.go` 逐段移植：`ParseDict`、`ParseReversed`、
`short_circuit` 字典組、`ConvertPinned` 與 `Pin` 讀音固定表。

字典是 OpenCC 1.4.2 官方文字檔（Apache-2.0），前身放在
`internal/simplified/opencc/`。本次複製到資料根目錄的 `opencc/`，不進 git：

```
/mnt/work/jc/jc-ime-data/opencc/
  CJK_Compatibility_Ideographs.txt
  TWVariants.txt
  TWVariantsRevPhrases.txt
  TSPhrases.txt
  TSCharacters.txt
  LICENSE
  README.md
```

`jc-core` 只做比對，讀檔由 `jc-cli` 的 `data::load_zhconv` 負責；`opencc/`
不存在時回傳帶路徑的錯誤，不硬編字典。

`output` 新增 `HansConverter`，`VariantRenderer` 的 `ZhHans` 不再回
`MissingConverter`；`session` 在送出時套用簡體轉換，組字區仍顯示繁體，
與 golden 的 preedit／commit 分工一致。讀音 pin 表已移植並有測試，session
目前沒有逐字讀音的來源，因此 commit 只走整串轉換。

## mixed ASCII 推斷

`core/session/mixed.go` 的完整狀態機與候選、學習、日文接手交錯，這次只移植
commit 需要的核心：

- 同一類別重複的按鍵不算注音（`hello` 的 h／e 都是聲母）；
- 聲調鍵或空白能完成可解釋的音節時當中文；
- 整串組不成時，從最長且詞表認得的英文前段切開，後段接聲調當中文
  （`helloji3` → `hello我`、`ji3hellot ` → `我hello吃`）；
- 完整英文詞接空白保留空白（`hello `）；組不成注音的英文照字面；
- 英文自動補字：前綴補完優先，其次差一鍵的修正，Tab 接受
  （`ji3hellp` → `ji3hello`）。

候選清單、↓ 選原按鍵／注音解釋、數字型號與帳號切法（`mixedAfterAlnum`、
`mixedAfterToken` 等）沒有移植；golden 沒有涵蓋，留給後續 task。

`cjk_latin_space` 只在送出時套用 `CjkLatinSpacer` 的同一條規則，組字區照
打的顯示。

## 對照表

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/zhconv/zhconv.go` | `crates/jc-core/src/zhconv.rs` | ported：字典解析、反轉、轉換鏈、pinned 位置與 Pin 表 |
| `core/zhconv/zhconv_test.go` | `crates/jc-core/src/zhconv.rs` 的 `mod tests` | ported：`test_convert_follows_opencc_matching`、`test_pinned_positions_keep_their_output_and_split_segments`、`test_pin_table`、`test_parse_errors` |
| `internal/simplified/simplified.go` | `crates/jc-cli/src/data.rs` 的 `load_zhconv` | rewritten：改從資料根目錄的 `opencc/` 讀檔，不 embed |
| `core/output/render.go`（簡體） | `crates/jc-core/src/output/mod.rs` | ported：新增 `HansConverter`，`VariantRenderer` 接上 `ZhHans` |
| `core/session/mixed.go`（待定按鍵） | `crates/jc-core/src/mixed.rs` 的 `AsciiInput` | 部分：`mixedText`、`finishMixedTone`、`mixedBoundary`、`mixedSuggestion` 核心；候選與帳號切法 n/a |
| `core/session/mixed.go`（送出） | `crates/jc-core/src/session.rs` 的 `mixed_surface`、`finish_composition` | rewritten：逐片段解碼注音、英文照字面 |
| `core/session/english_autocomplete.go` | `crates/jc-core/src/mixed.rs` 的 `AsciiInput::suggestion` | 部分：Tab 接受前綴補完與一鍵修正；候選導覽未移植 |
| `core/session/cjk_latin_space.go` | `crates/jc-core/src/mixed.rs` 的 `space_text`、`crates/jc-core/src/session.rs` 的 `finalize` | ported：送出時補空格 |
| `core/session/automatic_mixed_test.go` | `crates/jc-core/src/session.rs` 的 `mod tests` | ported：`TestAutomaticMixedChineseEnglishChinese`、`TestMixedCompleteEnglishWordKeepsSpace` 的子集 |
| `core/session/cjk_latin_space_test.go` | `crates/jc-core/src/mixed.rs`、`crates/jc-core/src/session.rs` 的 `mod tests` | 部分：`TestSpaceCJKLatinRules`、`TestCJKLatinSpaceInMixedInput` |
| `core/session/candidates.go`（混合候選） | — | n/a：本次不移植候選視窗 |
| `core/personal`、`core/vocab` | — | n/a：M4 |

## 未完成

- 4 例學習依賴（D 類）：`learning-select-preference`、
  `learning-forget-request`、`undo-learned-word`、`undo-custom-word`。
- `mixed.go` 的候選、↓ 選注音解釋、英數型號／帳號切法與日文接手。
- 簡體輸出的逐字讀音 pin：轉換器與 pin 表已就緒，session 尚無逐字讀音來源。
