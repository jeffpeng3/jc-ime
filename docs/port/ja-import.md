# ja-import：Mozc 匯入與 pack v2

新增 `jc-jadic` crate，把 Mozc OSS 3.35.6354 `dictionary_oss` 轉成
`0CJADIC\0` v2 pack。寫入端包含來源解析、去重排序、pack 編碼、
manifest 與 CLI。

## 對照表

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/jadic/mozc.go` | `crates/jc-jadic/src/mozc.rs` | rewritten：只取 `dictionaryNN.txt`／`id.def`／`connection_single_column.txt`；word 成本不縮放，left/right id 不重映射 |
| `internal/jadic/mozc.go`（`LoadMozcIDDef`、`scanMozcDictionary`、`writeMozcConnections`） | `crates/jc-jadic/src/mozc.rs` | rewritten：POS 取 CSV 前四欄，活用型／活用形取第五、六欄；矩陣直接寫 context id，不再聚合成 POS 對 |
| `internal/jaimport/entry.go`（`NormalizeReading`） | `crates/jc-jadic/src/reading.rs` | ported：片假名轉平假名、半形空白忽略、非かな視為不可用讀音 |
| `internal/jaimport/pack.go` | `crates/jc-jadic/src/pack.rs` | rewritten：Go 版寫 TSV／`0CJLEX01`，本版照 pack v2 spec 寫 96-byte header、24-byte entry、connection section 與 SHA-256 trailer |
| `internal/jadic/manifest.go`、`import.go` | `crates/jc-jadic/src/import.rs` | rewritten：輸出 `0c-lexicon-manifest/1` 與 pack v2 的 `pack_format` 區塊 |
| `internal/jadic/inspect.go`、`convert.go`、`symbols`／`aux`／filter 流程 | — | n/a：任務書只列三個來源；記号辭典、`aux_dictionary.tsv`、`dictionary_filter.tsv` 不在本 crate 範圍 |

## 格式決定

- POS table 是 `id.def` 前四欄 CSV 的排序去重字串，例如
  `名詞,固有名詞,地域,一般`；entry 的 `pos_id` 由 `left_id` 查表。
- 活用型／活用形 table 是 `id.def` 第五、六欄的排序去重字串，entry
  存 u8 索引。`*` 也留在 table 內。
- entry 的 `flags` 固定 0（split C），因為 Mozc 沒有 split 資訊。
- 成本保留 MeCab 原值，範圍檢查 `-32767..32767`，不縮放。
- 連接矩陣依 spec 以 row-major 寫入：第 k 筆對應
  `left = k / context_count`、`right = k % context_count`。
- 讀音無法轉成平假名（數字、記号、`・` 等）的列跳過，計入
  `counts.filtered_reading`；其餘解析錯誤一律帶檔名與行號。
- 去重 identity 是 `(surface, reading, POS, conj_type, conj_form,
  left_id, right_id)`，重複時保留最低成本。
- 排序依 surface 的 code point，同 surface 連續；`surface index`
  指向該 surface 的第一筆 entry。

## 真實資料結果

`cargo run -p jc-jadic --release -- import --mozc
/mnt/work/jc/jc-ime-data/mozc-3.35.6354 --out
/mnt/work/jc/jc-ime-data/packs`：

| 項目 | 值 |
|---|---|
| `ja-lexicon-mozc-v2.pack` 大小 | 85,508,727 bytes |
| entry_count | 1,220,427 |
| surface_count | 983,412 |
| context_count | 2,672 |
| POS／活用型／活用形 | 81 / 46 / 30 |
| 來源列數 | 1,289,076 |
| 可用讀音列數 | 1,285,596 |
| 去重丟棄 | 65,169 |
| 讀音不可用而跳過 | 3,480 |
| pack SHA-256 | `15926f48dca7a6917f067a41f8ec49e2474c11d0cdebce5486a81f51fd0c84c6` |
| payload SHA-256 | `9818042a52e71f85b402bfa46a939ce61026cd5138d9fa4e6e5ecf8e81e2ea0e` |
| manifest SHA-256 | `ccc488040e43b9a061c0c6adf82e068654d4aa37e4fb69499eb1b9eb05afe6e1` |

## 驗證

- `cargo check -p jc-jadic --all-targets` 通過。
- 產出的 pack 以獨立 Python 解碼器核對：section 連續且落在檔內、
  trailer SHA-256 正確、table 排序、surface index 與 entry 一致、
  所有讀音都是平假名、矩陣長度 `2672²`、值域 `0..15281`。
- 抽查 lookup：`食べる→たべる`（動詞,自立,*,*／一段／基本形）、
  `日本語→にっぽんご`、`東京→とうきょう`。

## Spec 未定義或衝突

- 任務書要求「兩個檔的 SHA-256 都寫進 manifest」，但 manifest 無法
  包含自身的 SHA-256。manifest 內寫 pack 的整檔與 payload SHA-256，
  manifest 自身 SHA-256 由 CLI 輸出與本文件記錄。
- `dictionary_filter.tsv` 與 `aux_dictionary.tsv` 未列在任務書來源，
  未匯入；若需要與 Go 版逐條一致，要再補這兩個來源。
