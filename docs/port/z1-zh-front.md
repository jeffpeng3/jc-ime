# z1-zh-front 對照表

注音音節、讀音、按鍵、分段的移植。Go 來源：`/mnt/work/jc/ref/0c-ime`（main `8f443af`）。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/zhuyin/zhuyin.go` | `crates/jc-core/src/zh/syllable.rs` | ported：`Syllable`、`Class`、`Composer`；`ParseReading`／`Key` 改為 `parse_reading`／`key` 函式 |
| `core/zhuyin/untoned.go` | `crates/jc-core/src/zh/syllable.rs` | ported：`Untone`／`Untoned`／`Tones`／`Text`／`HasUntoned`／`LegalPrefix`（`LegalPrefix` 在 `zh/pinyin.rs`） |
| `core/zhuyin/han.go` | `crates/jc-core/src/zh/han.rs` | ported：漢字範圍固定在 Unicode 15.0 的資料表 |
| `core/zhuyin/pinyin.go` | `crates/jc-core/src/zh/pinyin.rs` | ported：拼音轉注音、音節表、`FromPinyin`；另外接受全形字母與全形空白（任務書要求的正規化） |
| `core/zhuyin/reading.go` | `crates/jc-core/src/zh/syllable.rs`、`src/reading.rs` | rewritten：Go 的 `reading.Unit` 別名轉接改成 `Syllable::reading_unit`、`reading::from_syllables`／`to_syllables`。`nil` 與空切片在 Rust 是同一件事，不另設測試 |
| `core/zhuyin/zhuyin_test.go` | `crates/jc-core/src/zh/syllable.rs` | ported：`test_valid`、`test_composer`、`test_composer_edits_for_dynamic_layouts`、`test_composer_backspace_follows_display_order`；`TestParseReading` 併入 `test_parse_reading_and_key` |
| `core/zhuyin/untoned_test.go` | `crates/jc-core/src/zh/syllable.rs`、`zh/pinyin.rs` | ported：`TestUntoned` 併入 `test_untone`、`test_legal_prefix` |
| `core/zhuyin/han_test.go` | `crates/jc-core/src/zh/han.rs` | ported：`test_is_han_is_pinned` |
| `core/zhuyin/pinyin_test.go` | `crates/jc-core/src/zh/pinyin.rs` | ported：`test_from_pinyin`、`test_legal`、`test_tone_helpers` |
| `core/zhuyin/reading_test.go` | `crates/jc-core/src/reading.rs` | ported：`test_reading_key_matches_zhuyin_key`、`test_reading_units_round_trip`、`test_reading_unit_validity_matches_zhuyin` |
| `core/reading/reading.go` | `crates/jc-core/src/reading.rs` | ported：`Unit`／`Sequence`／`Key`／`Valid`，注音與かな的合法性規則照原樣 |
| `core/reading/reading_test.go` | `crates/jc-core/src/reading.rs` | ported：`test_key`、`test_unit_valid_kana`、`test_unit_valid_zhuyin`、`test_unit_invalid`。Go 的 `Unit("\xff")` 非法 UTF-8 案例無法在 Rust 的 `String` 表達，省略 |
| `core/input/input.go` | `crates/jc-core/src/zh/input.rs` | ported：`Hyp`／`Lattice`／`Frontend`／`Exact`；`cost` 用 `f64` |
| `core/input/input_test.go` | `crates/jc-core/src/zh/input.rs` | ported：`test_hyp_uses_reading_unit`、`test_exact_produces_reading_units`；追加沒打聲調展開五聲的測試 |
| `core/segment/segment.go` | `crates/jc-core/src/segment.rs` | rewritten：`Span` 併入 `Segment` 的 `start`／`end`，改用 UTF-8 位元組位移（c0 契約）；Language 標籤改成 `Mode`；`Script` 增加 `Han`、`Mixed`，`Other` 對應 Go 的 `ScriptUnknown` |
| `core/segment/segmenter.go` | `crates/jc-core/src/segment.rs` | ported：`Splitter`／`Split`／`Detect` 與合併規則照原樣 |
| `core/segment/segment_test.go` | `crates/jc-core/src/segment.rs` | 部分 ported：`test_script_names_and_strength`。`TestSpan` n/a：`Span` 併入 `Segment`，`len()`／越界行為已有 c0 測試；`TestScriptJSONRoundTrip` n/a：serde 由各 crate 自行決定，c0 說這輪不急；`TestSegmentValid` n/a：Rust `Segment` 沒有 reading／surface／language 欄位，驗證屬於 lattice；`TestSessionSegmentsKeepsPerSegmentLanguages` n/a：`SessionSegments` 屬 session task |
| `core/segment/segmenter_test.go` | `crates/jc-core/src/segment.rs` | ported：`test_split_table`、`test_split_names_are_stable`、`test_detect`、`test_split_constructors_agree`、`test_split_does_not_keep_state`；`FuzzSplit` 改成固定種子案例 `split_always_covers_input`（Rust 端不引入 fuzz 工具鏈） |
| `core/segment/session.go` | — | n/a：`SessionSegments` 是 session 的介面，不在本 task 的 owned paths |
| `core/layout/layout.go` | `crates/jc-core/src/zh/keyboard.rs` | 部分 ported：標準（大千）與倚天 41 鍵的靜態表、標點、`Keys`、`ByName`；許氏依狀態解讀照任務書只預留 `LayoutKind::Hsu` |
| `core/layout/layout_test.go` | `crates/jc-core/src/zh/keyboard.rs` | 部分 ported：`test_standard_round_trip`、`test_standard_punct`、`test_eten_table`、`test_eten_round_trip`、`test_eten_punct`、`test_by_name`。`ByName("hsu")` 改成斷言尚未實作的錯誤 |
| `core/layout/hsu.go` | — | n/a：許氏是依組音狀態解讀同一鍵的動態排列，任務書指定這輪只做預設排列、其他用 enum 預留 |
| `core/layout/hsu_test.go` | — | n/a：隨 `hsu.go` 保留，等動態排列實作時一起搬 |

## 相依套件

- `unicode-script`：分段時判斷 Latin。漢字與かな仍用自己的表，不跟工具鏈的
  Unicode 版本跑。
- `unicode_categories`：分段時判斷十進位數字（`Nd`），跟前身的
  `unicode.IsDigit` 一致；`std` 的 `is_numeric` 會把 `①` 也算成數字。

## 與契約的差異

- `Segment` 的位移沿用 c0 的 UTF-8 位元組，Go 測試的 rune 索引在測試內用
  `at()` 轉換，期望值維持與 Go 相同。
- `Script` 在 c0 的六種之外加上 `Han` 與 `Mixed`。Go 的分段規則以漢字與
  英數混合片段為邊界依據，少了這兩種就必須在測試裡把 `Han` 併進 `Other`、
  把 `abc123` 降級成 `Latin`，會丟掉呼叫端需要的資訊。`Other` 保留給
  Go 的 `ScriptUnknown`（不在本專案語言範圍的字母與數字），沒有刪除任何
  c0 的變體。
- `core/input` 的 Go 版本是音節格（`Hyp`／`Exact`），任務書另外要求
  按鍵累積狀態機。兩者都在 `zh/input.rs`：`Exact` 照 Go 搬，`ZhInput`
  是新的行為測試（`Consumed`／`SyllableComplete`／`Commit`／`Cancel`），
  沒有 Go 對應。
