# zh-data：注音 lexicon.bin 與 lm.bin 讀取

## 範圍

把前身 `0c-ime` 的執行期注音資料包格式搬到 Rust：`lexicon.bin`（緊湊詞庫）、
`lm.bin`（unigram／bigram 語言模型）與 `manifest.json`（大小與 SHA-256）。
讀取端在 `jc-data::zh`，`jc-cli` 的 `convert`／`bench` 可用 `--data` 指向真實
資料包；`eval zh-golden` 仍使用凍結的 TSV，行為不變。

## Go 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/binfmt/binfmt.go` | `crates/jc-data/src/zh/format.rs`（`Reader`、`le_u16/le_u32/le_f32`） | ported |
| `core/lexicon/compact.go`（`Decode`、查詢、`LongestWord`、`Has`、`Lookup`） | `crates/jc-data/src/zh/lexicon.rs` | ported |
| `core/lexicon/compact.go`（`Build`、`Encode`） | `crates/jc-data/src/zh/testutil.rs`（只供測試產生最小資料包） | rewritten（測試用寫入器） |
| `core/lm/compact.go`（`DecodeCompact`、`Uni`、`Bi`、`HasLeft`、量化） | `crates/jc-data/src/zh/lm.rs` | ported |
| `core/lm/compact.go`（`BuildCompact`） | `crates/jc-data/src/zh/testutil.rs` | rewritten（測試用寫入器） |
| `internal/compile/pack.go`（manifest 的 `sha256`／`size` 欄位） | `crates/jc-data/src/zh/format.rs`（`Manifest`） | ported（讀取端） |
| `core/lexicon/lexicon.go`（`Source` 介面） | `crates/jc-data/src/zh/lexicon.rs`（`exact`／`prefixed`／`has_prefix`／`has`／`lookup`） | rewritten（借用檢視，不複製 `Entry`） |
| `core/lm/lm.go`（`System` 介面、`Unseen`、`BOS`） | `crates/jc-data/src/zh/lm.rs`（`uni`／`bi`／`UNSEEN`／`BOS`） | ported |

## 二進位格式

所有整數都是小端序。檔頭：

```text
magic[8]  u32 version  依序排列的區段…  u32 crc32
```

`crc32` 覆蓋 magic 到最後一個區段（不含檔尾 4 位元組），使用 IEEE 多項式
（`crc32fast::hash`，等同 Go `hash/crc32.ChecksumIEEE`）。每個陣列區段先寫
`u32` 元素數；字串先寫 `u32` 位元組數。讀完後必須沒有剩餘位元組。

### `lexicon.bin`（`ZUINLEX\0`，version 1）

| 順序 | 區段 | 內容 |
|---|---|---|
| 1 | `syls` | 音節字串，每個音節後接一個空格；位置即音節 ID |
| 2 | `rd` | `u16`，所有詞條的讀音 ID 串接 |
| 3 | `rdOff` | `u32[n]`，詞條讀音在 `rd` 的起點 |
| 4 | `sfOff` | `u32[n]`，詞面在 `strs` 的位元組位移 |
| 5 | `sfLen` | `u16[n]`，詞面位元組長度 |
| 6 | `rdLen` | `u8[n]`，讀音音節數 |
| 7 | `flags` | `u8[n]`，bit 0 自動修正、bit 1 罕用 |
| 8 | `prw` | `f32[n]`，P(r|w)，必須在 (0, 1] |
| 9 | `strs` | 所有詞面串接（UTF-8） |
| 10 | `byText` | `u32[n]`，詞條索引，依詞面排序 |

詞條依讀音 ID 序列（`cmpIDs` 的字典順序）排序，`byText` 在同一詞面時依
儲存順序決勝。查詢用二分搜尋，`prefixed` 在前綴邊界停止，`lookup` 走
`byText`。音節表在載入時轉成 `Vec<String>` 與 `HashMap<String, u16>`（只有
約 1.6 千筆），詞條、詞面與讀音都直接借用 mmap。

### `lm.bin`（`ZUINLM\0\0`，version 2）

| 順序 | 區段 | 內容 |
|---|---|---|
| 1 | `strs` | 所有詞串接（UTF-8，依字串排序） |
| 2 | `off` | `u32[n+1]`，詞在 `strs` 的位元組位移 |
| 3 | `uni` | `f32[n]`，unigram 機率 |
| 4 | `leftStart` | `u32[n+1]`，以第 i 個詞為前詞的 bigram 範圍 |
| 5 | `right` | 每筆 bigram 3 位元組的後詞 ID |
| 6 | `q` | 每筆 bigram 1 位元組的量化 `log10 P` |

量化：`q = clamp(round(-log10(P) / 8 * 255), 0, 255)`，還原
`P = 10^(-q / 255 * 8)`。`bigrams == 0` 時 `bi()` 直接回傳 0、`has_left()`
回傳 false，unigram 照常可用。未登錄詞的 `uni()` 回傳 `1e-8`（前身
`lm.Unseen`）。

## 驗證

`Bundle::open(dir)`：

1. 讀 `manifest.json`，取出 `lexicon.bin`、`lm.bin` 的 `size` 與 `sha256`。
2. 以 mmap 開啟兩個檔案，先比對大小與整檔 SHA-256。
3. 驗證魔術字、版本、檔尾 CRC32、區段結尾無殘留。
4. 驗證各欄位長度一致、位移在範圍內、`rdLen > 0`、音節 ID 與 `byText`
   索引在範圍內、`prw ∈ (0,1]`、詞表嚴格排序、bigram 後詞 ID 遞增。

單獨開啟（`Lexicon::open`、`Lm::open`）跳過 manifest 比對，其餘檢查相同。

## CLI 接入

`convert` 與 `bench` 新增 `--data <dir>`：

```sh
cargo run -p jc-cli --release -- convert "ㄨㄛˇ ㄞˋ" --data /mnt/work/jc/jc-ime-data/build-zh
cargo run -p jc-cli --release -- convert "ㄨㄛˇ ㄞˋ" --data /mnt/work/jc/jc-ime-data/build-zh-bigram
```

成本換算沿用既有注音路徑：`-ln(P(r|w)) * 1000 + -ln(P(w)) * 1000`，未知
unigram 用 `1e-8`，未知詞成本 30000。詞條灌入 `InMemoryLexicon`，因為
`jc-core` 的解碼器只接受該型別；`jc-data` 本身的查詢仍是零拷貝。

沒有 `--data` 時行為完全不變：注音走 `0c-testdata/golden/zh/data` 的凍結
TSV，`eval zh-golden` 不受影響。

## build-zh 與 build-zh-bigram 的差異

兩包的 `lexicon.bin` 相同（同一個 SHA-256），`lm.bin` 只差 bigram：

| 資料包 | `lm.bin` 大小 | 詞數 | bigram 數 |
|---|---|---|---|
| `build-zh` | 4.5 MB | 238021 | 0 |
| `build-zh-bigram` | 31.4 MB | 238021 | 7028326 |

`convert "ㄨㄛˇ ㄞˋ"` 在兩包的 5-best 完全相同：

```text
1  我愛   13366
2  我艾   14803
3  我礙   17997
4  我隘   19310
5  我曖   19338
```

`build-zh-bigram` 的 `lm.bi("我", "愛")` 約 0.0044，`build-zh` 回傳 0；但
`jc-core` 的注音解碼器以詞性連接成本取代前身 bigram，所以 CLI 的排序不變。
需要 bigram 的呼叫端可直接用 `jc_data::zh::Lm::bi`。

## 測試

- `zh/format.rs`：魔術字、版本、CRC、殘留位元組、manifest 大小與 SHA-256。
- `zh/lexicon.rs`：精確／前綴／詞面查詢、罕用旗標、壞版面。
- `zh/lm.rs`：unigram、bigram、量化順序、無 bigram 回退、未排序詞表。
- `zh/tests.rs`：真實 `build-zh`／`build-zh-bigram` 的 manifest、查詢與 bigram。
- `jc-cli/src/converter.rs`：真實詞庫轉換 `ㄨㄛˇ ㄞˋ`，首選為「我愛」。
