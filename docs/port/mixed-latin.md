# mixed-latin：拉丁字串雙語言切分移植對照

Go 參考實作沒有對應的切分器。切分器重用既有已移植的羅馬字對照表與注音
鍵盤，沒有從 Go 搬新行為。`unify-segment` 之後它是混合路由唯一的拉丁 DP，
不是可選的第二個答案。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/input`（羅馬字對照表與促音規則） | `crates/jc-core/src/ja/input.rs`：`Table::entries`、`can_sokuon` | rewritten（只加接入點，行為不變） |
| `core/zh/keyboard`（大千排列與音節類別） | `crates/jc-core/src/zh/keyboard.rs`、`crates/jc-core/src/zh/syllable.rs` | ported（既有，本 task 只呼叫） |
| `core/zh`（音節表） | `crates/jc-core/src/zh/pinyin.rs`：`is_legal_base` | ported（既有，本 task 只呼叫） |
| 無 | `crates/jc-core/src/mixed_latin.rs`：`LatinSplitter`、`Options`、`SplitLexicon`、`LexiconAdapter`、`toned_syllables` | n/a（Go 沒有對應） |
| `core/ja/mixed`（英數片段推斷） | `crates/jc-core/src/mixed.rs`：`plan_input`、`build_atoms`、`visible_plans` | rewritten（單一管線，見 `docs/port/unify-segment.md`） |
| `core/session/mixed.go`（混打待定按鍵） | `crates/jc-core/src/session/mixed.rs`：`AsciiInput::typed`、`incremental_latin_split`、`apply_latin_split` | rewritten（逐鍵走同一管線） |
| `cmd/0c-eval-mixed` | `crates/jc-cli/src/split_cmd.rs`：`mixed-split` | rewritten（只印一份結果） |
| 無 | `docs/reports/mixed-latin-2026-11-20.md` | n/a |

## 測試對照

Go 沒有可沿用的測試名；新測試都在 `mixed_latin.rs`、`mixed.rs` 與
`session/mixed.rs` 的 `mod tests`：

| Rust 測試 | 涵蓋 |
|---|---|
| `test_fixture_rishireej_savul3` | `rishireej/ savul3` 的四段切分 |
| `test_english_word_stays_literal` | `rishireej/ sasmall` 的英文尾巴 |
| `test_common_english_beats_romaji` | `code` 留英文、`kanji` 走日文 |
| `test_pure_romaji` / `test_pure_zhuyin` | 純羅馬字與純注音 |
| `test_mixed_with_spaces` | 空白與混用 |
| `test_space_is_first_tone_for_zhuyin` | 空白是一聲 |
| `test_space_after_kana_is_pause` | 日文段尾空白是停頓 |
| `test_space_tone_then_japanese` | `ej/ sa` → 供さ |
| `test_tone_keys_mark_zhuyin` | 聲調固定注音段 |
| `test_unparsable_returns_none` | 無法完整解析時回 `None` |
| `test_incremental_matches_full` | 增量結果等於整串重算 |
| `test_set_text_appends_only` | 追加只算新尾端 |
| `test_lexicon_prefers_known_word` | 詞典加分改變歧義 |
| `test_r2_space_is_first_tone_only_when_run_parses` | R2 的三個條件 |
| `test_initial_lang_carries_previous_run_language` | `Options::initial_lang` |
| `test_punct_at_run_start_is_not_a_final` | R3 的 run 開頭標點 |
| `test_mixed_latin_split_fixture` | Router 端到端 |
| `test_mixed_latin_split_is_default` | 預設就走 DP |
| `test_mixed_latin_split_trailing_zhu_yin_final` | `5k42. g4n0 vul3` → `這都是三小` |
| `test_mixed_latin_split_trailing_period_stays_literal` | 英文 `hello.` 的句點仍是標點 |
| `test_incremental_matches_router` | 532 筆按鍵流的增量與路由一致性 |

## 尾端標點與注音韻母

`.` `,` `;` 在大千鍵盤是韻母 ㄡ ㄝ ㄤ。R3 要求標點接在缺韻母的合法音節
後面才當韻母，因此 `5k42.` 的句點是 ㄉㄡ 的 ㄡ，`hello.` 的句點仍是符號。
`.` `,` `;` `/` `-` 出現在 run 開頭或前面不是合法音節時，`split_at_symbol_punct`
會把它切成獨立的符號 atom。

測試用 `InMemoryLexicon` 的 `LexEntry` 成本要低於未知詞成本（8），否則解碼器
會選未知詞弧，表面上看起來像「只還原讀音、沒有轉成漢字」；`cost = 0` 與正式
詞庫一致。
