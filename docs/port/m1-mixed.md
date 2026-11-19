# m1-mixed：混合路由與英文直通

混合輸入移植到 `crates/jc-core/src/mixed.rs`（`Mode::Mixed` 是唯一入口）。
行為以 Go 的 `core/ja/mixed`、`core/lang` 與 `core/english` 為準：語言判定、
片段切分、逐段路由、英文直通與詞表前綴補完都照規則搬過來；Go 的
`lang.Profile`／`Composer`／`Converter` 介面沒有搬，改成具體型別 `Router`。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/mixed/doc.go` | `crates/jc-core/src/mixed.rs` module 文件 | ported |
| `core/ja/mixed/config.go` | 同上：`Config`、`Config::normalized` | ported |
| `core/ja/mixed/token.go` | 同上：`Kind`、`classify`、`scan`、`is_url`、`is_email`、`is_identifier`、`Token` | ported |
| `core/ja/mixed/english.go` | 同上：`English`、`Dictionary`、`Suggestion`、`English::candidates` | rewritten：`lang.Candidate` 換成 `jc_core::Candidate`，`Source`／`Annotation` 換成 `CandidateSource`，拼字建議（`Corrections`）這輪不做 |
| `core/ja/mixed/spacing.go` | 同上：`Spacing`、`space_text`、`needs_space_between` | ported |
| `core/ja/mixed/width.go` | 同上：字寬偵測與轉換、`width_variant` | ported |
| `core/ja/mixed/infer.go` | 同上：`literal_for`、`kana_for`、`replay_romaji` | rewritten：Go 借用 `Composer` 的私有判定，Rust 直接重用同一組純函式 |
| `core/ja/mixed/converter.go` | 同上：`Router::{segments, convert, detect, prefixes}`、`MixedResult` | rewritten：Go 包住 `lang.Converter`；Rust 直接持有 `decode::Decoder`、`JaDecoder` 與 `English`，逐段路由 |
| `core/ja/mixed/composer.go` | 同上：`literal_for`、`kana_for`、`split_kana_prefix`、`group_words` | rewritten：`session.rs` 與 `lang.Composer` 不在本 task 的 owned paths；按鍵狀態機的判定改成純函式，分段與片語合併直接作用在輸入字串上 |
| `core/ja/mixed/profile.go` | — | n/a：`lang.Profile` 註冊層在 M1 還沒進 `jc-core`，`Mode::Mixed` 已是唯一入口 |
| `core/ja/mixed/token_test.go` | `mixed.rs`：`test_classify`、`test_literal_kinds`、`test_is_url_email_identifier`、`test_scan`、`test_scan_url_and_email_boundaries` | ported |
| `core/ja/mixed/spacing_test.go` | 同上：`test_space_text_default_no_space`、`test_space_text_always`、`test_needs_space_between`、`test_spacing_names` | ported |
| `core/ja/mixed/width_test.go` | 同上：`test_width_conversion`、`test_width_variant`、`test_width_detectors` | ported |
| `core/ja/mixed/english_test.go` | 同上：`test_english_suggest_and_complete`、`test_english_learn_and_forget`、`test_english_candidates` | rewritten：候選斷言改成 `CandidateSource`；`TestEnglishCandidateTypes` 改成編譯期型別檢查（`Vec<Candidate>`） |
| `core/ja/mixed/converter_test.go` | 同上：`test_literal_runs_are_not_converted`、`test_route_email_and_url_are_literal`、`test_route_zh_segment`、`test_spacing_between_segments`、`test_conversion_uses_output_offsets`、`test_english_prefix_completion`、`test_route_without_decoders_falls_back_to_reading`、`test_route_empty_input` | rewritten：Go 的假 `lang.Converter` 換成真的 `JaDecoder`＋記憶體 `JaWordSource` 與 `InMemoryLexicon` |
| `core/ja/mixed/composer_test.go` | 同上：`test_kana_for_and_literal_for`、`test_split_kana_prefix_before_english`、`test_primitives_stay_literal` | rewritten：沒有 `lang.Composer` 可按，改成等價的純函式與分段行為測試 |
| `core/ja/mixed/profile_test.go` | 同上：`test_long_words_are_joined_across_boundaries`、`test_literal_runs_are_not_converted` 的注音／日文段 | rewritten：session 整合屬 M4；這裡直接驗 `Router::convert` 的送出字面與片段 |
| `core/lang/lang.go` | `lang.rs`（既有）、`mixed.rs` 的 `Mode` 使用 | n/a：`Profile`／`Limits`／`InputMethod` 是 Go 的註冊契約，Rust 這輪沒有 profile 層；`Mode` 沿用 `c0-contracts` 定好的型別 |
| `core/lang/multilingual.go` | `mixed.rs`：片段 `mode` 即語言標籤 | rewritten：`LanguageTag`／`OutputVariant` 尚未進 Rust 契約，片段直接帶 `Mode` 與 `Script` |
| `core/lang/composer.go` | — | n/a：Rust 由 `zh::input::ZhInput` 與 `ja::input::RomajiInput` 各自提供，沒有共用 trait |
| `core/lang/converter.go` | — | n/a：同上，`decode::Decoder` 與 `JaDecoder` 直接當轉換器 |
| `core/lang/key.go` | — | n/a：`KeyPolicy` 屬 M4 的 session 層 |
| `core/lang/lang_test.go` | — | n/a：測的是 Go 介面可組合性（dummy profile/composer/converter），Rust 沒有對應抽象 |
| `core/lang/multilingual_test.go` | `mixed.rs`：`test_detect_mode` | rewritten：只保留語言標籤的行為（`OutputVariant` 這輪沒有實作） |
| `core/english/dictionary.go` | `mixed.rs`：`Dictionary` | rewritten：不建 trie，改用排序陣列＋二分搜尋；`Corrections`／`NearCommon` 這輪不做 |
| `core/english/codec.go` | `mixed.rs`：`Dictionary::{decode, open}` | ported（`0cEN\x00\x01` v1 讀取端） |
| `core/english/prototype.go` | `mixed.rs`：`prototype()` | ported |
| `core/english/alnum.go` | `mixed.rs`：`is_fullwidth_alnum` 等字寬函式（與 `mixed/width.go` 合併） | ported |
| `core/english/alnum.tsv` | — | n/a：那是全角對照的產出資料；`to_halfwidth_latin`／`to_fullwidth_latin` 用碼位差直接換 |
| `core/english/dictionary_test.go` | `mixed.rs`：`test_dictionary_prefix_and_case`、`test_dictionary_decode_roundtrip` | ported（`TestTypoChannelsAndValidWordProtection` 見下） |
| `core/english/index_test.go` | — | n/a：測 trie 索引；Rust 用二分搜尋，改由 `test_dictionary_prefix_and_case` 驗行為 |

## 刻意差異

| 項目 | 說明 |
|---|---|
| 轉換器注入 | `Router` 的三個轉換器都是選配。沒有注音詞典時注音段照原樣輸出，沒有日文解碼器時かな段照原樣輸出，這樣沒有資料也能測分段與語言判定。 |
| 語言標籤 | Go 的 `lang.LanguageTag`／`OutputVariant` 沒有搬；片段的 `Segment::mode`（`Zh`／`Ja`／`En`）就是語言，`Segment::script` 是字面種類。 |
| 片語切分 | 全小寫英文字以單一空白相連時視為一個片語一起判定；兩個字以上的片語裡，單一かな單位的短字不當羅馬字，因此 `desu is a good sentence` 是「日文段＋英文片語」。 |
| かな前綴切分 | `kyouhaAPI`、`kyouhahttps://…` 會在英數段前切開，但只有大寫、數字、全角或已知英文詞開頭的後綴才切，避免把 `sentence` 誤切成かな。 |
| 拼字建議 | `Corrections`／`NearCommon`（編輯距離通道）這輪不做，英文只做直通與前綴補完。 |
| 詞表來源 | `data/english/words.bin` 目前不存在（打包器另有混淆與雜湊驗證，屬資料 pipeline）。沒有檔案時用 `prototype()`，有檔案時 `Dictionary::open` 直接讀明文 `0cEN` v1。 |
| 游標 | `MixedResult::cursor_utf16` 是整句的 UTF-16 長度；`MixedSegment` 另有 `start_utf16`／`end_utf16`。`Conversion` 的 `Segment` 依既有慣例改用**輸出**位元組位移。 |

## 測試資料

| 測試 | 資料 | 缺資料時 |
|---|---|---|
| 其餘全部單元測試 | 內建原型詞彙與記憶體詞典 | 不需要外部資料 |
| `test_golden_mixed_cases` | `0c-testdata/golden/mixed/cases.json`（6 個案例） | `require_data!` 跳過 |

golden 測試只比片段邊界、語言與書寫系統；`span` 是 rune 位移，測試內換成
UTF-8 位元組位移再比對。轉換字面不在這輪的斷言範圍。
