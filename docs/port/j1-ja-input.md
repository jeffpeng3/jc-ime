# j1-ja-input 對照表

日文輸入前端：羅馬字轉かな、直接かな輸入、輸入正規化。Go 來源：
`/mnt/work/jc/ref/0c-ime`（main `8f443af`）。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/input/composer.go` | `crates/jc-core/src/ja/input.rs` | rewritten：`Composer.Insert(string)` 改成 `RomajiInput::press(Key) -> RomajiEvent`；`KanaInput` 獨立成直接かな狀態機，支援濁點、半濁點與小書きかな |
| `core/ja/input/composer_test.go` | `crates/jc-core/src/ja/input.rs` | ported：`test_romaji_composer_table`、`test_backspace`、`test_flush`、`test_cancel`、`test_invalid_combinations`、`test_kana_composer`、`test_custom_romaji_table`、`test_symbols`；另外新增濁點、半濁點、舊假名與 preedit 測試。`TestUnitsAreCopied` n/a：Rust `units()` 回傳借用，編譯器已禁止呼叫端改動內部狀態；`Insert("")` 與一次插入多字元的案例改寫成逐字元按鍵 |
| `core/ja/input/table.go` | `crates/jc-core/src/ja/input.rs` | ported：`Table`、`DefaultRomajiTable`、`Lookup`、`IsPrefix`、促音規則與內建對照表 |
| `core/ja/input/symbols.go` | `crates/jc-core/src/ja/input.rs` | ported：`Symbols`、`DefaultSymbols`、`Lookup`、`Insert` |
| `core/ja/rewrite/kana.go` | `crates/jc-core/src/ja/rewrite.rs` | ported：`ToHiragana`／`ToKatakana`／`ToHalfwidthKana`／`ToFullwidthLatin`／`ToHalfwidthLatin` 與半角カナ展開、濁點組合 |
| `core/ja/rewrite/kana_test.go` | `crates/jc-core/src/ja/rewrite.rs` | ported：`test_to_hiragana_table`、`test_to_katakana_table`、`test_to_halfwidth_kana_table`、`test_latin_width_table`。`TestKanaCandidatesTable` n/a：かな候選是輸出端改寫，不屬於輸入正規化 |
| `core/ja/rewrite/rewriter.go` | — | n/a：產生かな、數字與記号候選屬於輸出端改寫，不是輸入正規化 |
| `core/ja/rewrite/rewriter_test.go` | — | n/a：同上 |
| `core/ja/rewrite/number.go` | — | n/a：數字表記候選屬於輸出端改寫 |
| `core/ja/rewrite/number_test.go` | — | n/a：同上 |
| `core/ja/rewrite/symbols.go` | — | n/a：記号與顔文字辭典屬於輸出端候選 |
| `core/ja/rewrite/symbols_test.go` | — | n/a：同上 |
| `core/ja/rewrite/doc.go` | — | n/a：套件說明，內容已寫進 `ja/rewrite.rs` 的 module doc |

## 契約差異

- 按鍵事件沿用 c0 的 `Key`／`KeyCode`。`press` 只接受 `KeyCode::Char`；
  退格、取消、結束輸入是獨立方法，與 Go 的 `Composer` 介面一致。
- 羅馬字未完成前綴保留在 `pending()`；`RomajiEvent` 只回報這次完成的かな，
  顯示用 `preedit_text()`、`segments()`、`cursor()`。
- 直接かな輸入的每一個かな都是一個 `Unit`，小書きかな不與前一個かな合併，
  與前身 `reading.Sequence` 的單位規則相同。濁點與半濁點會替換上一個單位。
- 舊假名 `ゐ`、`ゑ`、`を` 保持原樣，與前身一致，不轉成現代假名。
- `ja/mod.rs` 只新增 `pub mod input;` 與 `pub mod rewrite;` 兩行；其餘
  module 宣告沒有改動。
