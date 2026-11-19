# english-pack：英文詞表建置與接線

Go 來源：`/mnt/work/jc/ref/0c-ime`（main `8f443af`）的 `internal/englishpack` 與
`core/english`。建置器在新 crate `crates/jc-englishpack`；執行期查詢在
`crates/jc-core/src/mixed.rs`。資料不進 git，輸出到
`/mnt/work/jc/jc-ime-data/english/`。

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/englishpack/compile.go` | `crates/jc-englishpack/src/compile.rs`：`compile`、`SourceMetadata`、`Export`、`commit_from_version` | ported |
| `internal/englishpack/import.go` | `crates/jc-englishpack/src/import.rs`：`import_lists` | ported |
| `internal/englishpack/pack.go` | `crates/jc-englishpack/src/pack.rs`：`write`、`load`；manifest 型別與驗證在 `jc-core/src/mixed.rs` | ported（`LoadWithTransform` 的資料解封裝沒有搬，我們的產物不混淆） |
| `internal/englishpack/pack_test.go` | `crates/jc-englishpack/tests/build.rs`：`test_compile_deterministic_and_no_suggest_round_trip`、`test_rejected_source_does_not_publish_or_overwrite`、`test_load_rejects_corruption_and_missing_attribution`、`test_import_lists_matches_reviewed_normalization` | ported |
| `internal/englishpack/benchmark_test.go` | `crates/jc-englishpack/tests/build.rs`：`test_product_pack_loads` | n/a：Go 是量測用 benchmark，Rust 改驗真實資料包可載入；實測 `query` 含載入 12 ms |
| `core/english/dictionary.go` | `mixed.rs`：`Dictionary`、`Word`、`Suggestion`、`Correction`、`corrections`、`near_common` | rewritten：不建 trie，改用排序陣列＋二分搜尋；拼字建議掃描長度相近的詞並用有界 DP |
| `core/english/codec.go` | `mixed.rs`：`Dictionary::{encode, decode, open, load_pack}` | ported（`0cEN\x00\x01` v1 讀寫端） |
| `core/english/prototype.go` | `mixed.rs`：`prototype`、`default_english`、`English::load_default` | rewritten：原型只當資料包載入失敗的退路，不再預設使用 |
| `core/english/alnum.go` | `mixed.rs`：`is_fullwidth_alnum` 等字寬函式 | n/a：M1 已併入 `mixed.rs` 的字寬規則 |
| `core/english/alnum.tsv` | — | n/a：全角對照直接用碼位差換算，不需要資料檔 |
| `core/english/dictionary_test.go` | `mixed.rs`：`test_english_suggest_and_complete`、`test_english_learn_and_forget`、`test_english_candidates`、`test_english_candidates_append_corrections` | ported（`TestTypoChannelsAndValidWordProtection` 見下） |
| `core/english/index_test.go` | `mixed.rs`：`test_prefix_covers_entire_dictionary`、`test_common_neighbour_is_not_hidden_by_candidate_limit`、`test_correction_matches_exhaustive_distance`、`test_no_suggest_words_recognized_and_unicode_identifiers_untouched`、`test_codec_roundtrip_and_malformed_input`、`test_typo_channels_and_valid_word_protection` | rewritten：索引結構不同，斷言改成行為與完整結果向量 |
| `core/ja/mixed/english.go` 的 `Candidates` | `mixed.rs`：`English::candidates` | rewritten：拼字建議接上 `Correction`；`lang.Candidate` 換成 `jc_core::Candidate`，`Annotation` 沒有對應欄位，改用 `CandidateSource`＋`cost` |
| `core/ja/mixed/english_test.go` | `mixed.rs`：`test_english_suggest_and_complete`、`test_english_learn_and_forget`、`test_english_candidates` | ported（`TestEnglishCandidateTypes` 是 Go 介面檢查，Rust 由編譯期型別保證） |

## `words.bin` 格式（0cEN v1）

前身實作是 `magic "0cEN\x00\x01"`（6 位元組）＋小端序 `u32` 詞數＋逐筆詞條。
任務書寫的 `0cEN\x00\x00\x01` 與前身原始碼不符，這裡照前身原始碼實作。

| 欄位 | 大小 | 說明 |
|---|---|---|
| magic | 6 | `30 63 45 4e 00 01` |
| count | 4 | 小端序 `u32`，上限 200,000 |
| 每筆詞條 | 1 + n + 9 | 長度 `u8`、小寫 ASCII 詞面、`f64` 小端序權重、`u8` no-suggest 旗標 |

詞條依詞面嚴格遞增排序；權重是 `100 - ESDB size` 與產品詞表初值，代表排序
先驗，不是語料詞頻。檔尾沒有總和檢查，完整性靠 `manifest.json` 的 SHA-256。

## `manifest.json`

`format`（固定 1）、`words`、`ranking`、`sources`（name／version／url／sha256／
license／attribution／commit）、`inputs`（`esdb.tsv`、`technical.tsv`、
`source.json`、`LICENSE.txt` 的 SHA-256）、`files`（`words.bin`、`NOTICE.txt`
的 SHA-256 與大小）。map 依 key 排序輸出，重跑同一份輸入會得到逐位元組相同的
三個檔案。`commit` 是新增欄位，取自 `source.json` 的 `source.commit`，沒有時
從 `version` 內的 40 字元 commit 取。

`load_pack` 會拒絕：目錄是符號連結、manifest 欄位缺漏或格式不是 1、任一檔案
大小或 SHA-256 不符、`NOTICE.txt` 空白、詞表解不開、詞數與 manifest 不符。

## 載入與退路

`English::load_default` 依序找 `JC_ENGLISH_PACK`、`JC_IMEDATA`／`JC_TESTDATA`
資料根目錄下的 `english/`、`/mnt/work/jc/jc-ime-data/english/`；找到就驗證
manifest 並解碼，任何一步失敗都記一筆 `log::warn` 後退回 `prototype()`（104 詞）。
`Router` 沒有指定 `english` 時用同一個 `default_english()`（`OnceLock`，全行程
只載一次），所以 `prefixes`／`classify`／`literal_for` 看到的是同一份詞表。

## 拼字建議

`Dictionary::corrections` 是前身 `Corrections` 的行為移植：完整單字不建議、
少於 3 個字元不建議、6 個字元以上容許編輯距離 2、no-suggest 詞不推薦、全大寫
查詢回全大寫、首字大寫查詢回首字大寫，結果依距離→權重→詞面排序。
`near_common` 是不受候選上限影響的「附近有沒有常見詞」查詢。

實作差異：前身用 trie 逐層 DP，這裡掃排序陣列，先用長度差過濾，再用整列最小
值超限就放棄的 DP。82,015 詞的實測查詢（含載入與輸出）約 12 ms。

`English::candidates` 先給前綴補完，補不滿 `limit` 才補拼字建議；建議的成本
多 `CORRECTION_PENALTY × 距離`，因此一定排在完整補完之後。

## 建置與驗證

```sh
sh scripts/build_english_pack.sh            # 來源預設 ref/0c-ime/data/english
./target/release/jc-englishpack verify --dir /mnt/work/jc/jc-ime-data/english
./target/release/jc-englishpack query "zebr"
```

實際產出：82,015 詞，`words.bin` 1,522,028 位元組，
`SHA-256 aa03049852110645bce87b5a7bcb9a383a1b0748e59465827148adb534701782`；
`NOTICE.txt` 5,360 位元組。同一份來源用前身 `cmd/0c-english` 建出的
`words.bin` 與這裡逐位元組相同；manifest 只多 `commit` 欄位，`NOTICE.txt`
標題改成 `jc-ime 英文詞表`。`strace` 顯示 `manifest.json` 與 `words.bin` 都被
開啟，`query` 印出來源是 `/mnt/work/jc/jc-ime-data/english/words.bin`，不是原型。

## 來源與授權

| 項目 | 內容 |
|---|---|
| 詞表 | ESDB／SCOWL 2026.02.25，commit `7e99edab8e32f9f9ea2b15f249ca8d4d67237410`（A／B／Z，variant ≤ 1，小寫 ASCII，size ≤ 60） |
| 授權 | SCOWLv2 的 MIT 式授權，全文來自 `data/english/LICENSE.txt`（`090575a1…` 已固定），原樣附在產出的 `NOTICE.txt` |
| 姓名標示 | Copyright 2000-2026 Kevin Atkinson；Alan Beale（12dicts／ENABLE2K）特別致謝 |
| 產品詞表 | `technical.tsv` 104 筆由本專案自行編列，不代表語料詞頻 |
| 上游雜湊 | `source.json` 固定下載檔 `770dd3e4…`、正規化 `esdb.tsv` `0bc3e0c8…` |

`NOTICE.txt` 是產出物，不進 git；repo 沒有另一份 NOTICE 要改。

## 測試

| 測試 | 資料 | 缺資料時 |
|---|---|---|
| `jc-englishpack/tests/build.rs` 的 fixture 測試 | 暫存目錄自建 2 詞來源 | 不需要外部資料 |
| `test_product_pack_loads`（jc-englishpack） | `english/words.bin` 等三檔 | `require_data!` 跳過 |
| `test_real_english_pack_loads`（jc-core） | 同上 | `require_data!` 跳過 |
| 其餘 `mixed.rs` 測試 | 記憶體詞表與原型 | 不需要外部資料 |

## 未完成

- 任務書驗證指令 `jc-cli convert "hello wor" --mode en` 需要 `jc-cli` 支援
  `--mode`，但 `jc-cli` 不在本 task 的 owned paths，沒有改。等價的執行期路徑由
  `jc-englishpack query` 覆蓋（同一個 `Router::prefixes` 與 `default_english`）。
- 前身 `LoadWithTransform` 的資料解封裝、`cmd/0c-english` 的打包器整合沒有搬。
