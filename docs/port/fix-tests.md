# fix-tests：全量測試修到綠

基準：`main` `af59d44`。驗證：`cargo test --workspace --no-fail-fast` exit 0，
`cargo check --workspace --all-targets` 通過。

第一次全量測試在 `jc-cli` 先失敗，因為 `--no-fail-fast` 後才看到其他 crate。
下表是完整失敗清單（34 個測試、9 個 target）。

## 失敗表

| crate | 測試 | 檔案:行 | 錯誤 | 分類 |
|---|---|---|---|---|
| jc-cli | `eval_cmd::tests::named_keys_follow_protocol_names` | `crates/jc-cli/src/eval_cmd.rs:762` | `named_key("Ctrl+,")` 被斷言為 Err，但 golden 具名鍵就是 `Ctrl+,` | 測試期望錯 |
| jc-cli | `test_ja_eval_and_cli_eval_ja_agree_on_same_pack` | `crates/jc-cli/tests/eval_ja_parity.rs:32` | 表格解析取到標題列，`n` 解析失敗 | 測試工具錯 |
| jc-cli | `test_default_pack_comes_from_data_root` | `crates/jc-cli/tests/eval_ja_parity.rs:106` | 用 `0c-testdata` 當資料根目錄，實際 pack 在 `jc-ime-data/packs` | 測試工具錯 |
| jc-core | `ja::input::tests::test_kana_composer_dakuten_and_small_kana` | `crates/jc-core/src/ja/input.rs:1163` | 小書きかな只回傳新單位，測試卻拿累積單位比事件 | 測試斷言錯 |
| jc-core | `ja::lm::tests::test_nan_rejected` | `crates/jc-core/src/ja/lm.rs:362` | NaN 寫到最後 8 bytes，實際蓋掉 bigram 數量而非成本 | 測試位移錯 |
| jc-core | `lattice::lexicon::tests::prefix_includes_exact_and_longer_entries` | `crates/jc-core/src/lattice/lexicon.rs:246` | `prefix` 少完全相等的詞條 | 實作 bug |
| jc-core | `lattice::lexicon::tests::prefix_stops_at_syllable_boundary` | `crates/jc-core/src/lattice/lexicon.rs:225` | 同上 | 實作 bug |
| jc-core | `lattice::viterbi::tests::test_connection_cost_can_change_choice` | `crates/jc-core/src/lattice/viterbi.rs:329` | 罰則只擋 `prev_pos`，`甲乙` 仍同成本且穩定鍵較小 | 測試設計錯 |
| jc-core | `lattice::viterbi::tests::test_nbest_uses_lattice_context_by_default` | `crates/jc-core/src/lattice/viterbi.rs:346` | 只有兩條路徑，第二條不可能 110；且 `乙丙` 同成本勝出 | 測試設計錯 |
| jc-core | `mixed::tests::test_segments_use_utf8_offsets_and_modes` | `crates/jc-core/src/mixed.rs:3592` | 聲調 `ˋ` 是 2 bytes，測試用 6×3 算成 18 | 測試算術錯 |
| jc-core | `mixed::tests::test_golden_mixed_cases` | `crates/jc-core/src/mixed.rs:3729` | 空 Router 沒有英文詞表，`is a good sentence` 被拆開 | 測試設定錯 |
| jc-core | `session::tests::test_session_segments_follow_zhuyin_path_and_cursor` | `crates/jc-core/src/session.rs:1370` | 只按 `t`，沒有完成 `ㄔ` 音節 | 測試工具錯 |
| jc-core | `session::tests::test_automatic_mixed_chinese_english_chinese` | `crates/jc-core/src/session.rs:1563` | 空白用 `Char(' ')` 送出，沒走 Space 的混打結算 | 測試工具錯 |
| jc-core | `session::tests::test_cjk_latin_space_in_mixed_input` | `crates/jc-core/src/session.rs:1612` | 同上；`su3` 是「你」，測試寫成「我」 | 測試工具錯＋期望錯 |
| jc-core | `session::tests::test_mixed_complete_english_word_keeps_space` | `crates/jc-core/src/session.rs:1579` | 同上 | 測試工具錯 |
| jc-core | `session::tests::test_emoji_palette_selection_and_recent` | `crates/jc-core/src/session.rs:1730` | 數字鍵選取記錄的是高亮項，不是被選項 | 實作 bug |
| jc-core | `session::tests::test_symbol_menu_recent_tab` | `crates/jc-core/src/session.rs:1689` | 「最近」存在時開到第二個分頁；Esc 清掉整個組字 | 實作 bug＋測試期望錯 |
| jc-core | `session::tests::test_symbol_menu_tabs` | `crates/jc-core/src/session.rs:1677` | 同數字鍵記錄 bug | 實作 bug |
| jc-core | `session::tests::test_page_moves_highlight_to_page_start` | `crates/jc-core/src/session.rs:1534` | 候選含字面項，第 11 項不是「子」 | 測試假設錯 |
| jc-core | `session::tests::test_toneless_readings_expand_to_cheapest_tones` | `crates/jc-core/src/session.rs:1500` | 「我」與「窩」同成本，平手取到一聲 | 測試 fixture 錯 |
| jc-core | `zhconv::tests::test_real_catalog_converts_taiwan_forms` | `crates/jc-core/src/zhconv.rs:526` | `TSPhrases.txt` 只有 484 條，門檻 1000 太高 | 測試假設錯 |
| jc-data | `zh::lexicon::tests::lookup_finds_all_readings_of_surface` | `crates/jc-data/src/zh/lexicon.rs:547` | 同詞面讀音依讀音排序，測試要求插入順序 | 測試期望錯 |
| jc-data | `zh::lm::tests::unsorted_words_are_rejected` | `crates/jc-data/src/zh/lm.rs:322` | 「你」「我」其實已排序 | 測試資料錯 |
| jc-data | `zh::tests::real_lexicon_answers_exact_and_prefix` | `crates/jc-data/src/zh/tests.rs:34` | 真實資料的「我」有 ㄨㄛˇ、ㄜˇ 兩個讀音 | 測試假設錯 |
| jc-engine | `tests::shutdown_marks_process_exit` | `crates/jc-engine/src/lib.rs:343` | `is_closing` 把行程 shutdown 當連線狀態，`reset_session` 清不掉 | 實作 bug |
| jc-eval | `ja::tests::run_is_deterministic_and_scores_accept_variants` | `crates/jc-eval/src/ja.rs:689` | 兩次跑的 latency 牆鐘值不同 | 測試不穩定 |
| jc-model | `layout::tests::packs_bos_left_eos_candidate_eos` | `crates/jc-model/src/layout.rs:149` | tokenizer 沒試到字串結尾，`▁東京都` 被切成兩段 | 實作 bug |
| jc-model | `layout::tests::candidate_longer_than_slot_is_truncated` | `crates/jc-model/src/layout.rs:179` | 同上，左文多一個 token | 實作 bug |
| jc-model | `layout::tests::empty_context_still_has_separator` | `crates/jc-model/src/layout.rs:188` | 同上，候選 id 錯 | 實作 bug |
| jc-model | `tokenizer::tests::encode_prefers_longest_piece` | `crates/jc-model/src/tokenizer.rs:357` | 同上 | 實作 bug |
| jc-model | `tokenizer::tests::unknown_runes_use_byte_fallback` | `crates/jc-model/src/tokenizer.rs:377` | 期望值把 byte 當 id，且漏掉 `▁` | 測試期望錯 |
| jc-model | `tokenizer::tests::decode_roundtrips_pieces_and_byte_fallback` | `crates/jc-model/src/tokenizer.rs:383` | decode 會把 `▁` 還原成空白，測試卻期望保留 `▁` | 測試期望錯 |
| jc-platform | `fake_scheduler_fires_due_timers_in_time_order` | `crates/jc-platform/tests/contract.rs:166` | 到期時間 100/120/150，測試期望 149ms 前都不觸發 | 測試期望錯 |
| jc-proto | `tests::key_uses_core_shape` | `crates/jc-proto/src/lib.rs:118` | `Key` 加了 `ctrl` 後 JSON 多一個欄位 | 實作 bug（序列化形狀） |

## 修法

實作：

- `InMemoryLexicon::prefix` 把完全相等的讀音視為合法前綴。
- `Key.ctrl` 加上 `serde(default, skip_serializing_if = "Not::not")`，`false` 不寫進 JSON。
- `Engine`：`Shutdown` 同時設 `close_connection`；`is_closing` 只看連線狀態，
  `is_shutdown` 保存行程狀態，`reset_session` 可以清掉前者。
- `Tokenizer::encode` 在字元邊界迴圈後再試完整剩餘字串，最長前綴匹配才會包含
  字串結尾；修正 `▁東京都`、byte fallback 與 rerank 佈局。
- `Session` 選單：數字鍵用實際選取的候選記錄最近使用；「最近」永遠是第 0 分頁；
  選單開著時 Esc 只關選單。
- `Session` 混打測試輔助函式把 `' '` 轉成 `KeyCode::Space`，讓空白走
  `AsciiInput::space` 的英→中結算。

測試（不改產品期望，只修測試機制或錯誤假設）：

- 日文かな事件只斷言 `Completed`，累積單位另外比對。
- NaN 測試改寫在成本欄位，不再蓋到 bigram 數量。
- Viterbi 罰則改擋前後詞性；N-best 加上反向連接成本，讓期望值可達。
- mixed 位移用實際字串長度；golden 測試補英文詞表與 `min_english_len = 1`。
- session 測試用完整音節與 `Space` 按鍵；翻頁比對頁首索引與實際候選；
  toneless 用成本可分的 fixture。
- 資料測試改用真正未排序的詞序、依讀音排序的期望、真實「我」的兩個讀音。
- eval 比較前把 latency 歸零。
- tokenizer byte fallback 用實際 piece id，decode 期望改成含空白的輸出面。
- scheduler 依到期時間重寫時序。
- ja parity 從資料根目錄解析預設 pack，表格解析跳過標題列。

## 測試期望錯誤（依任務書先列出，未改產品行為）

以下測試的期望值本身與 Go 參考實作、資料或協定不一致，已改成可驗證的正確
期望，沒有改產品行為：

- `named_keys_follow_protocol_names`：`Ctrl+,` 是 golden 具名鍵。
- `test_cjk_latin_space_in_mixed_input`：`su3` 是「你」，不是「我」。
- `test_symbol_menu_recent_tab`：Esc 後保留的組字仍有候選，不該要求空清單。
- `test_page_moves_highlight_to_page_start`：頁首是第 11 個候選，不是固定「子」。
- `test_toneless_readings_expand_to_cheapest_tones`：原 fixture 兩個讀音同成本。
- `zhconv` 門檻：`TSPhrases.txt` 只有 484 條。
- `lookup_finds_all_readings_of_surface`：同詞面讀音依讀音排序。
- `unsorted_words_are_rejected`：原資料其實已排序。
- `real_lexicon_answers_exact_and_prefix`：真實「我」有兩個讀音。
- `fake_scheduler_fires_due_timers_in_time_order`：原時序不可能成立。
- `unknown_runes_use_byte_fallback`、`decode_roundtrips_pieces_and_byte_fallback`：
  期望值與 vocab id、`▁` 還原規則不符。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/lattice/lexicon/compact.go`（`Lookup`、前綴） | `crates/jc-core/src/lattice/lexicon.rs` | ported：補上完全相等前綴 |
| `core/session/symbol_palette.go`、`palette.go` | `crates/jc-core/src/session.rs` | rewritten：修正數字鍵記錄、最近分頁、Esc |
| `core/session/mixed.go` | `crates/jc-core/src/mixed.rs`、`session.rs` | 部分：測試輔助函式修正，未新增按鍵推斷 |
| `core/proto`（Key 形狀） | `crates/jc-core/src/key.rs` | rewritten：`ctrl` 省略 false |
| `core/engine`（Shutdown） | `crates/jc-engine/src/lib.rs` | rewritten：連線與行程狀態分離 |
| `internal/spm/encode.go` | `crates/jc-model/src/tokenizer.rs` | rewritten：最長前綴匹配補上字串結尾 |
| `internal/neural/native/rerank_plan.go` | `crates/jc-model/src/layout.rs` | ported：tokenizer 修正後佈局一致 |
| `core/lm/compact.go`（排序驗證） | `crates/jc-data/src/zh/lm.rs` | ported：測試資料改為真正未排序 |
| `core/lexicon/compact.go`（Lookup 排序） | `crates/jc-data/src/zh/lexicon.rs` | ported：測試改依讀音排序 |
| `internal/golden`（具名鍵、ja parity） | `crates/jc-cli/src/eval_cmd.rs`、`tests/eval_ja_parity.rs` | rewritten：修正測試假設 |
| `internal/eval`（指標比較） | `crates/jc-eval/src/ja.rs` | rewritten：排除 latency |
| 平台 fake scheduler | `crates/jc-platform/tests/contract.rs` | rewritten：Go 無對應實作，依到期時間重寫 |
