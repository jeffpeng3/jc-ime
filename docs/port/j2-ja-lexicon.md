# j2-ja-lexicon：日文詞典與連接矩陣

查詢層與連接成本移植。詞典來源改成 `jc-data` 的 `0CJADIC\0` v1 pack，
Go 的 `Mem`（TSV trie）與 `Bin`（`0CJLEX01`）都不再需要；兩者的對外
查詢語意由 `JaLexicon` 直接以 pack 的排序實作。

## 對照表

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/lexicon/lexicon.go` | `crates/jc-core/src/ja/lexicon.rs` | rewritten：`Mem` 與 TSV `Load` 由 pack 取代；`Entry` 改成 `JaEntry`，POS 保留 pack 的原始字串，另以 `JaPosClass` 對應粗分類 |
| `core/ja/lexicon/lexicon_test.go` | `crates/jc-core/src/ja/lexicon.rs` | rewritten：TSV 載入與 `Add` 驗證改由 `jc-data` 的 pack 驗證負責；同義測試改在 pack 建的 `JaLexicon` 上斷言 |
| `core/ja/lexicon/bin.go` | `crates/jc-core/src/ja/lexicon.rs` | rewritten：`0CJLEX01` 載入器不移植，pack 已是驗證過的零拷貝格式；`Lookup`／`Prefix`／`LongestWord` 語意保留 |
| `core/ja/lexicon/bin_test.go` | `crates/jc-core/src/ja/lexicon.rs` | rewritten：`TestBinMatchesMem` 的 Mem／Bin 對照改成 pack 排序行為測試；`LoadBin` 損壞檔測試改成 `JaLexicon::open` 壞 magic |
| `core/ja/lexicon/connection.go` | `crates/jc-core/src/ja/lexicon.rs` | rewritten：`Matrix` 改為 `SingleColumnCost`，`Connection` 改為 `ConnectionCost`；`ContextMatrix` 整段不移植（pack v1 沒有 context ID） |
| `core/ja/lexicon/context_test.go` | — | n/a：context 矩陣與 context ID 欄位在 pack v1 不存在，介面只留 `ConnectionCost` trait |
| `core/ja/lm/lm.go` | `crates/jc-core/src/ja/lm.rs` | ported：`0CJLM001` 解碼、unigram／bigram 查詢、`HasLeft` 與空模型語意一致 |
| `core/ja/lm/lm_test.go` | `crates/jc-core/src/ja/lm.rs` | ported：五個測試全搬；`nil` 模型改成 `LanguageModel::new()` 的中性行為 |

## 設計決定

- **成本尺度**：pack 的 `i16` 原值直接放大成 `i32`，不重新縮放
  （`MIN_WORD_COST = -32767`、`MAX_WORD_COST = 32767`）。curated
  `ja-src/connection.tsv` 用小數記錄，讀入時乘 1000 對齊，例如 `0.3`
  變成 `300`；保底值 `DEFAULT_CONNECTION_COST = 1000`（前身 1.0）。
- **POS**：`JaEntry::pos` 保留 pack 的完整字串（UniDic／IPADic 皆為
  `大分類,細分類,...`）。`JaPosClass::classify` 取第一個逗號欄位對應
  curated 連接表使用的粗分類；數詞、接尾、形容動詞語幹另外處理。
- **連接矩陣**：先做詞性對詞性的 `SingleColumnCost`，讀
  `ja-src/connection.tsv`。Mozc 的 `connection_single_column.txt` 是
  `size` 加整整 `size×size` 的 context ID 成本，pack v1 沒有 left/right
  context ID 可以對應，所以不讀；完整矩陣之後由新的 `ConnectionCost`
  實作接手。
- **未知詞**：`UNKNOWN_WORD_COST = 8000`（前身每個かな 8.0）、
  `unknown_entry()` 產生詞性 `other`、split `C` 的 fallback。
  `unknown_word_cost(units)` 依かな數累加。
- **split**：pack 的 C／A／B 原樣保留在 `JaEntry::split`，查詢層不改變
  輸出粒度；未知詞固定 C。
- **LM**：先提供記憶體模型與 `0CJLM001` 解碼，讓 curated 資料與測試可
  先跑。`ja-src/lm/` 是次數與機率 TSV，不是 `lm.bin`；真實 `lm.bin`
  待 `jc-data` 編譯器產出後再接。`jc-ime-data/build-zh/lm.bin` 是注音
  模型，不使用。

## 測試

- `JaLexicon`：自建最小合法 pack 測 lookup／prefix／排序／最長讀音／
  壞 magic；真實 pack 測 `私`、`本`、`読む`、`日本語` 的讀音、POS、
  活用與成本，資料不存在時由 `require_data!` 跳過。
- `SingleColumnCost`：行內 TSV 測解析、BOS／EOS、保底值與七種壞行；
  另讀 `ja-src/connection.tsv` 斷言具體成本。
- `LanguageModel`：編碼後解碼、空模型、magic／截斷／尾端、負成本、
  NaN 拒絕、`from_pairs` 的 left 集合。
