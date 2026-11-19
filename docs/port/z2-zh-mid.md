# z2-zh-mid 對照表

注音的候選圖與最短路徑：`Lexicon` 查詢介面、lattice 建構、Viterbi、
基礎成本。詞典二進位讀取屬 `w1-data-zh`，這裡只定介面與演算法。

## 契約差異

Rust 契約比 Go 精簡，對照時以下差異是刻意的：

- 讀音是「音節以單一空格分隔」的字串（`"ㄊㄞˊ ㄅㄟˇ"`），不是
  `reading.Sequence`；型別由 `z1-zh-front` 定義後再轉進來。
- 詞格位置是音節索引，不是 UTF-8 位元組位移。
- 沒有 `Language`／`Variant`／JSON；候選來源沿用 c0 的
  `CandidateSource`。
- 連接成本改成詞性對（`u16`）查表，取代 Go 的語言切換成本。
- 成本是 `i32`，越小越優先（c0 的 MeCab 語意），Go 是 `float64`。

成本數值來源：`DEFAULT_CONNECTION_COST = 1`（前身
`core/ja/lexicon.DefaultConnectionCost`）、`DEFAULT_UNKNOWN_COST = 8`
（`core/ja/decode.DefaultUnknownCost`）、`MAX_WORD_UNITS = 8`
（`core/decode.MaxWordLen`）；長度折扣 `DEFAULT_LENGTH_BONUS = 1`
對應前身 Viterbi 同成本時優先較少邊數的 tie-break。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/lattice/build.go` | `crates/jc-core/src/lattice/build.rs` | rewritten：`Input`／`CandidateSource` 改成讀音序列＋`Lexicon`；`BuildFrom` 的長詞跨片段在建邊時自然完成 |
| `core/lattice/viterbi.go` | `crates/jc-core/src/lattice/viterbi.rs`、`crates/jc-core/src/score.rs` | rewritten：語言切換成本改成詞性連接成本，DP 與同成本 tie-break 語意保留 |
| `core/lattice/candidate.go` | `crates/jc-core/src/lattice/candidate.rs` | rewritten：只留 `Edge`（音節 span、詞面、讀音、成本、詞性、來源），不含語言與 JSON |
| `core/lattice/candidate_test.go` | `crates/jc-core/src/lattice/candidate.rs` | rewritten：`TestCandidatePromotedFields`／`TestCandidateKey` 改成 `Edge` 欄位與鍵測試；JSON 測試 n/a：Rust 契約沒有序列化 |
| `core/lattice/viterbi_test.go` | `crates/jc-core/src/lattice/viterbi.rs` | rewritten：`TestNBestOrdersPathsAndIsDeterministic`、`TestLongWordWinsTieAgainstSplit`、`TestCandidateCostAndTransitionCost` 搬成詞性成本版；`TestSwitchCostProducesDistinctLanguages` 改成連接成本測試；`TestBuildFromMergesLongWordAcrossSegments` 由 build 測試承接 |
| `core/lattice/scenario_test.go` | `crates/jc-core/src/lattice/viterbi.rs` | rewritten：混合語言情境改成注音長詞／同音字與未知詞情境；日英混打屬 `j1-ja-input` |
| `core/lexicon/lexicon.go` | `crates/jc-core/src/lattice/lexicon.rs` | rewritten：trie 改成 `HashMap`＋插入順序清單；`Exact`／`Prefixed`／`Longest` 對應 `lookup`／`prefix`／`longest_word_units`；TSV 讀取屬 `w1-data-zh` |
| `core/lexicon/lexicon_test.go` | `crates/jc-core/src/lattice/lexicon.rs` | rewritten：只搬查詢語意（`TestLoadValidates` 的 Exact／Prefixed／HasPrefix、`TestLongestWord`）；讀檔與格式驗證 n/a：屬 `w1-data-zh` |
| `core/lexicon/candidates.go` | `crates/jc-core/src/lattice/lexicon.rs` | rewritten：查詢語意併入 `Lexicon`；語言推斷與 `Query.Candidate` 的 `Language`／`Variant` 欄位 n/a：Rust 契約沒有語言標籤 |
| `core/lexicon/candidates_test.go` | `crates/jc-core/src/lattice/lexicon.rs` | rewritten：前綴與多詞面語意搬過來；`TestQueryValid`／`TestQueryKey`／JSON／語言推斷 n/a：對應型別不在 Rust 契約 |
| `core/lexicon/compact.go` | — | n/a：編譯後二進位格式由 `w1-data-zh` 讀取 |
| `core/lexicon/compact_test.go` | — | n/a：同上，屬 `w1-data-zh` |
| `core/lexicon/neutral.go` | — | n/a：輕聲語氣詞補償是解碼前端的候選展開，屬 `z3-zh-back` 的 untoned |
| `core/lexicon/neutral_test.go` | — | n/a：同上 |
| `core/score/score.go` | `crates/jc-core/src/score.rs` | rewritten：Go 的多語言 `Scorer`／`Calibrator`／`Arbitrator` 契約留給模型 task；這裡實作詞格需要的單詞、連接、未知詞與長度成本 |
| `core/score/score_test.go` | `crates/jc-core/src/score.rs` | rewritten：契約型別測試改成長度折扣、未知詞、連接矩陣的具體數值測試 |
| `core/score/ranker.go` | — | n/a：模型重排與逾時控制，屬模型接線 task（任務書指定跳過） |
| `core/score/ranker_test.go` | — | n/a：同上 |
| `core/score/zh_adapter.go` | — | n/a：ONNX reranker adapter，屬模型接線 task（任務書指定跳過） |
| `core/score/zh_adapter_test.go` | — | n/a：同上 |
| `core/score/failsafe.go` | — | n/a：ranker 的失敗回退與隱私旁路，隨 ranker 一起跳過 |
| `core/score/failsafe_test.go` | — | n/a：同上 |

## 測試名稱

`TestNBestOrdersPathsAndIsDeterministic` → `test_nbest_orders_paths_and_is_deterministic`、
`TestLongWordWinsTieAgainstSplit` → `test_long_word_wins_tie_against_split`、
`TestBuildFromMergesLongWordAcrossSegments` → `test_build_covers_long_words_across_syllables`、
`TestLongestWord` → `test_longest_word_units_tracks_the_longest_entry`、
`TestLoadValidates`（查詢部分）→ `test_lookup_returns_exact_reading_only`、
`test_prefix_stops_at_syllable_boundary`。

`TestBuildRejectsInvalidInput` 的空輸入與非法 span 改成
`test_build_rejects_empty_input`、`test_build_rejects_empty_reading`。
`TestDuplicatesKeepLowestCostAndBadSpansAreIgnored` 改成
`test_build_dedupes_same_edge_keeping_lowest_cost`；範圍不符的候選在
Rust 契約不可能出現，改由 `test_build_ignores_entries_with_mismatched_reading`
覆蓋詞典實作錯誤的情況。
