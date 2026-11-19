# core-misc：vocab、suggest、inputstyle、lm 收齊

上游：`/mnt/work/jc/ref/0c-ime` `8f443af`。範圍：`core/vocab`、`core/suggest`、
`core/inputstyle`、`core/lm` 全量移植，含測試。

## 逐檔對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/vocab/vocab.go` | `crates/jc-core/src/vocab.rs` | ported：`Set`、`Word`、`Phrase`、`Batch`、`Change`、`Error`、`NewerFormatError`、`Normalize`、`IsEnglish`、新增／刪除／改讀音／片語／復原、JSON 讀寫。 |
| `core/vocab/index.go` | `crates/jc-core/src/vocab.rs` | ported：讀音前綴樹、`Exact`、`HasPrefix`、`Prefixed`、`HasEnglish`、`HasEnglishPrefix`、`EnglishPrefix`、`EnglishLen`。 |
| `core/vocab/vocab_test.go` | `crates/jc-core/src/vocab.rs` 的 `tests` | ported；`TestQualityFixtureLoads` 改用 `jc_testdata::require_data!`，資料不存在時 skip。存檔縮排由 Go 的一個空白改成 `serde_json` 的兩個空白，測試改為斷言 JSON 內容（`"readings": []`）。 |
| `core/vocab/english_prefix_test.go` | `crates/jc-core/src/vocab.rs` 的 `test_english_prefix` | ported。 |
| `core/suggest/suggest.go` | `crates/jc-core/src/suggest.rs` | ported：`Suggestion`、`Options`、`DefaultOptions`、`For`、`ForWithLocks`、`explicitAppend`、`crossAppend`、`appendOnly`、`completion`。前身的 `decode.Explicit` 改成同一 crate 的本地 trait `ExplicitWords`，不直接依賴 `vocab`。 |
| `core/suggest/suggest_test.go` | `crates/jc-core/src/suggest.rs` 的 `test_benefit_depends_on_remaining_typing` | rewritten：Rust 解碼器以成本而非機率計分，原測試的 `Score` 假資料沒有對應；改用同前綴競爭詞做出相同信心，驗證效益門檻。另加 `test_script_is_used_in_suggestion_shape`、`test_explicit_words_extend_across_segments` 覆蓋建議形狀與跨分段自訂詞。 |
| `core/inputstyle/style.go` | `crates/jc-core/src/inputstyle.rs` | ported：`Rules`、`Behavior`、`Preset`、`Validate`、`Get`、`GetForPlatform`、`Presets`、`PresetsForPlatform`。 |
| `core/inputstyle/style_test.go` | `crates/jc-core/src/inputstyle.rs` 的 `tests` | ported（測試名改成 snake_case）。 |
| `core/lm/lm.go` | `crates/jc-core/src/lm.rs` | ported：`BOS`、`UNSEEN`、`System`、`Mem`、`NewMem`、`SetUni`、`SetBi`、`HasLeft`、`Load`。 |
| `core/lm/compact.go` | `crates/jc-core/src/lm.rs` | ported：`Compact`、`BuildCompact`、量化、`Uni`、`Bi`、`HasLeft`、`EachBigram`、`Words`、`Bigrams`、`Encode`、`DecodeCompact`、`Entries`。編解碼沿用 `jc-data` 的 `zh::format::Reader` 與 `crc32fast`，格式與 Go 的 `ZUINLM` 版本 2 相同。 |
| `core/lm/median.go` | `crates/jc-core/src/lm.rs` | ported：`VocabQuantile`、`Compact::UniQuantileByLen`、`Mem::UniQuantileByLen`。 |
| `core/lm/lm_test.go` | `crates/jc-core/src/lm.rs` 的 `tests` | ported；測試資料由 Go repo 的 `testdata/toy/lm.tsv` 改成 `jc_testdata` 的 `0c-testdata/golden/zh/data/lm.tsv`。另加 `test_load_rejects_malformed_lines`、`test_quantile_by_len_uses_ordinal_position`。 |
| `core/doc.go`、`core/purity_test.go` | 無 | n/a：屬於 `core` 根套件；Rust 的純邏輯邊界由 crate 相依與 clippy 檢查，不是這四個套件的內容。 |

## `core/lm` 與既有實作的關係

- `crates/jc-data/src/zh/lm.rs` 是 `lm.bin` 的唯讀零拷貝讀取器，只做驗證與查詢。
- `jc-core/src/lm.rs` 補齊 Go `core/lm` 還缺的部分：可修改的 `Mem`、TSV 載入、
  離線建置 `Compact`、`Encode`／`DecodeCompact` 與各字數分位數。
- `jc_data::zh::Lm` 另外實作 `lm::System`，因此注音資料包與記憶體模型可以透過
  同一個介面使用。

## 邊界調整

- 沒有改 `score.rs`、`session.rs`、`personal*`、`ja/*`。
- `crates/jc-core/src/lib.rs` 只新增 `pub mod inputstyle; pub mod lm; pub mod suggest;
  pub mod vocab;`。
- `crates/jc-core/Cargo.toml` 新增 `crc32fast = "1"`，只用於 `Compact` 的 CRC32。
- `suggest` 需要舊解碼器的補全語意；Rust 解碼器已提供 `complete`、
  `complete_homophones`、`complete_across`，因此直接接在同一 crate 的
  `decode::Decoder` 上，沒有跨 crate 耦合。
