# unify-segment：把兩條分詞路徑合成一條

Go 參考實作沒有對應的切分器；這個 task 只重組既有 Rust 程式。下表列出改動
位置與狀態。

| 原本的路徑 | 現在的檔案 | 狀態 |
|---|---|---|
| A：`mixed_latin::split()` 當成第二個答案 | `crates/jc-core/src/mixed_latin.rs`：`split`、`space_is_first_tone`、`Options::initial_lang` | rewritten（降成 `mixed.rs` 呼叫的內部 DP） |
| B：`Router::convert()` → `build_atoms` → 逐 run DP | `crates/jc-core/src/mixed.rs`：`plan_input`、`push_token_run`、`push_run_text`、`visible_plans` | rewritten（唯一的 tokenize／DP／後處理管線） |
| 增量路徑 `incremental_latin_split`／`apply_latin_split` | `crates/jc-core/src/session/mixed.rs`：`AsciiInput::typed`、`plan_input` 重用 | rewritten（與路由同一份片段） |
| `mixed-split` 兩行並列 | `crates/jc-cli/src/split_cmd.rs` | rewritten（只印一份結果；`--steps` 印三階段） |

## 五條規則

| 規則 | 內容 | 實作位置 |
|---|---|---|
| R1 切段 | 注音、日文假名／漢字、英數 run、符號、空白；大寫與全角不進 DP | `build_atoms`、`is_latin_split_input` |
| R2 空白是一聲 | 前一個英數 run 加空白能完整解析成注音，且最後一個音節原本沒有聲調鍵時，空白併進 run | `space_is_first_tone`、`push_token_run`、`AsciiInput::space` |
| R3 標點當韻母 | `. , ; / -` 在 run 開頭，或前面不是缺韻母的合法音節時，是符號 atom | `compute_cell` 的開頭標點限制、`split_at_symbol_punct`、`punct_is_final` |
| R4 語言連貫 | `Options::initial_lang` 接住前一個 run 的語言，切換成本照算 | `Dp::new`、`build_atoms` 的 `last_lang` |
| R5 後處理 | `group_words` 是唯一的片語合併；英文詞判定只用 `is_english_word` | `plan_input`、`LexiconAdapter::english` |

## 前後對照

| 輸入 | 統一前（A／B） | 統一後 |
|---|---|---|
| `5k42. g4n0 vul3` | A：`ㄓㄜˋ ㄉㄡ ㄕˋ ㄙㄢ ㄒㄧㄠˇ`；B：`this` 片段切開 | `這都是三小`（單片） |
| `1ohk ` | A：空白是一聲；B：空白是分隔 | `杯測`（同一聲） |
| `lo y` | A：`ㄠ ㄟ ㄗ`；B：`lo y` | `ㄠ ㄟ ㄗ`（凹欸資） |
| `hello.` | A：句點當 ㄡ；B：英文加句點 | `hello.`（句點是符號） |
| `abc,def` | A：整串解析失敗；B：一個英數 run | `abc`＋`,`＋`def` |
| `dk4 xi` | A：兩段都注音；B：`xi` 兩邊合法時逐 run 跳到日文 | 兩段都注音（R4） |
| `m ` | A：ㄩ；B：分隔 | `ㄩ` 的一聲（淤） |

任務書的 `m ` → `媽` 與 `lo y` → `ㄠ ㄗ` 兩個範例和專案的按鍵表衝突：大千
鍵盤 `m` 是 ㄩ、`a8` 才是 ㄇㄚ；`l` 是 ㄠ、`o` 是 ㄟ，所以 `lo` 是兩個音節。
統一行為以按鍵表為準，測試固定記錄實際輸出（見
`test_unify_segment_nine_cases`）。

## 測試

| Rust 測試 | 涵蓋 |
|---|---|
| `test_unify_segment_nine_cases` | 任務書的九個案例 |
| `test_symbol_punctuation_stays_symbol` | R3 的開頭、中間與尾端標點 |
| `test_language_carries_between_latin_runs` | R4 的 `xi`／`wu` 連續性 |
| `test_r2_space_is_first_tone_only_when_run_parses` | R2 的三個條件 |
| `test_initial_lang_carries_previous_run_language` | `Options::initial_lang` |
| `test_punct_at_run_start_is_not_a_final` | run 開頭標點不產生注音 |
| `test_incremental_matches_router` | 532 筆按鍵流，session 增量與路由逐位元組一致 |

## 既有期望值變更

| 測試 | 舊 | 新 | 原因 |
|---|---|---|---|
| `test_mixed_latin_split_off_keeps_literal` | `rishireej/ savul3` 照字面 | 改名為 `test_mixed_latin_split_is_default`，輸出 `りしれ供さ小` | 開關移除，DP 成為唯一路徑 |
| `test_latin_run_keeps_trailing_final_only_when_split_completes` | 直接呼叫 `split_latin_run` | 改名 `test_symbol_punctuation_stays_symbol`，改測路由片段 | `split_latin_run` 已被 R3 的 tokenize 規則取代 |
| `test_mixed_complete_english_word_keeps_space` 的 `helloji ` | `hello窩` | `helloじ ` | R2 要求整段 run 可解析；`helloji ` 不符，空白是分隔 |
| `test_mixed_english_suggestion_and_completion` 的 `ji3hellp` | `我hellp` | `我hellㄣ` | 統一管線把 `p` 解成 ㄣ；英文補完仍用原始按鍵流 |
