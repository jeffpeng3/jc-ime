# mixed-latin：拉丁字串雙語言切分移植對照

這個 task 是研究原型，Go 參考實作沒有對應的切分器。切分器重用既有已移植的
羅馬字對照表與注音鍵盤，沒有從 Go 搬新行為；下表列出重用來源與新寫的檔案。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/input`（羅馬字對照表與促音規則） | `crates/jc-core/src/ja/input.rs`：`Table::entries`（新增唯讀列舉）、`can_sokuon` 改為 `pub(crate)` | rewritten（只加接入點，行為不變） |
| `core/zh/keyboard`（大千排列與音節類別） | `crates/jc-core/src/zh/keyboard.rs`、`crates/jc-core/src/zh/syllable.rs` | ported（既有，本 task 只呼叫） |
| `core/zh`（音節表） | `crates/jc-core/src/zh/pinyin.rs`：`is_legal_base` | ported（既有，本 task 只呼叫） |
| 無 | `crates/jc-core/src/mixed_latin.rs`：DP 切分器、`LatinSplitter`、`Options`、`SplitLexicon`、`ZhLexiconAdapter`、`LexiconAdapter`、`toned_syllables` | n/a（新研究原型，Go 沒有對應） |
| `core/ja/mixed`（英數片段推斷） | `crates/jc-core/src/mixed.rs`：`Config::mixed_latin_split`、`split_latin_atoms`、`Plan::zh`／`verbatim`、`visible_plans` | rewritten（只加關閉時完全相同的接入點） |
| `core/session/mixed.go`（混打待定按鍵） | `crates/jc-core/src/session/mixed.rs`：`latin_run_text`、`incremental_latin_split`、`apply_latin_split` | rewritten（開關關閉時走原路徑） |
| `cmd/0c-eval-mixed` | `crates/jc-cli/src/split_cmd.rs`：`mixed-split` 子命令 | rewritten（研究用量測工具） |
| 無 | `docs/reports/mixed-latin-2026-11-20.md` | n/a |

## 測試對照

Go 沒有可沿用的測試名；新測試都在 `mixed_latin.rs` 與 `mixed.rs`、`session/mixed.rs`
的 `mod tests`：

| Rust 測試 | 涵蓋 |
|---|---|
| `test_fixture_rishireej_savul3` | `rishireej/ savul3` 的四段切分 |
| `test_english_word_stays_literal` | `rishireej/ sasmall` 的英文尾巴 |
| `test_common_english_beats_romaji` | `code` 留英文、`kanji` 走日文 |
| `test_pure_romaji` | 純羅馬字 |
| `test_pure_zhuyin` | 純注音 |
| `test_mixed_with_spaces` | 空白與混用 |
| `test_space_is_first_tone_for_zhuyin` | 空白是一聲 |
| `test_space_after_kana_is_pause` | 日文段尾空白是停頓 |
| `test_space_tone_then_japanese` | `ej/ sa` → 供さ |
| `test_tone_keys_mark_zhuyin` | 聲調固定注音段 |
| `test_unparsable_returns_none` | 無法完整解析時回 `None` |
| `test_preview_keeps_tail_literal` | 逐鍵顯示的尾巴 |
| `test_incremental_matches_full` | 增量結果等於整串重算 |
| `test_set_text_appends_only` | 追加只算新尾端 |
| `test_lexicon_prefers_known_word` | 詞典加分改變歧義 |
| `test_mixed_latin_split_fixture` | Router 端到端 |
| `test_mixed_latin_split_off_keeps_literal` | 開關關閉時行為不變 |
| `test_mixed_latin_split_commit` | Session 逐鍵與送出 |
