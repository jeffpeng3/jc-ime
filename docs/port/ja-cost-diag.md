# ja-cost-diag：拆解 `わたしわ` → `私羽` 9068

## 結論

9068 全部由兩項組成：詞成本 4612、連接成本 4456。`私わ` 的詞成本便宜 4346，
但連接成本貴 4590，淨差 244，所以 `私羽` 勝出。

差距來自 Mozc `connection_single_column.txt` 的 context 連接矩陣，不是解碼器
邏輯、不是成本正規化、也不是候選被過濾。decoder 不動。

```
            詞成本   連接成本   合計
私 + 羽      4612     4456     9068   ← rank 1
私 + わ       266     9046     9312   ← rank 2
差距        -4346    +4590      244
```

244 = 4590 − 4346。兩條路徑的 BOS 連接（`0 → 私` = 3490）與 EOS 連接
（`X → 0` = 0）完全相同，差額只在中間那一段：

| 連接 | 成本 |
|---|---:|
| `1902 → 1949`（私 → 羽） | 966 |
| `1902 → 431`（私 → わ） | 5556 |
| 差 | 4590 |

## 1. `convert` 實際載入的 pack

`jc-cli convert` 走 `jc_cli::data::load_ja_decoder` → `jc_engine::loader::resolve_ja_pack`，
順序是 `--pack`、`JC_JA_PACK`、`packs/ja-lexicon-mozc-v2.pack`、`packs/ja-lexicon-public-core.pack`。
本機沒有設環境變數，因此載入：

| 項目 | 值 |
|---|---|
| 絕對路徑 | `/mnt/work/jc/jc-ime-data/packs/ja-lexicon-mozc-v2.pack` |
| 檔案 SHA-256 | `15926f48dca7a6917f067a41f8ec49e2474c11d0cdebce5486a81f51fd0c84c6` |
| payload SHA-256 | `9818042a52e71f85b402bfa46a939ce61026cd5138d9fa4e6e5ecf8e81e2ea0e` |
| 大小／詞條數 | 85,508,727 bytes／1,220,427 筆 |
| 格式 | v2，`context_ids = true`、`connection_matrix = true`、`context_count = 2672` |

檔案 SHA-256 與 `packs/ja-lexicon-mozc-v2.manifest.json` 的 `outputs.pack.sha256`
一致。v2 pack 帶 2672×2672 的連接矩陣，`loader::open_pack_decoder` 因此在
`JaDecoder` 上掛 `with_context_cost`，Viterbi 走 context 矩陣而不是詞性表。

`convert` 這條路徑沒有掛 reranker、沒有 LM（`lm_weight = 0`）、沒有開啟
`prefer_hiragana`，所以輸出純粹是「詞成本 + context 連接成本」的 Viterbi。

## 2. 計分公式

`crates/jc-core/src/ja/decode.rs`：

- 詞成本：`viterbi` 內 `go_cost(node.entry.cost)`，即 `raw / 1000`
  （`COST_SCALE = 1_000.0`）。輸出時 `raw_cost` 再 `round(cost * 1000)`。
  對整數成本是恆等轉換，9068 就是原始整數相加。
- 連接成本：`connection_cost` 只要左詞有 right ID、右詞有 left ID 就查
  `JaContextCost::cost(rid, lid)`；缺一邊才退回詞性表。未知詞沒有 ID，
  因此與未知詞相連時一律 `DEFAULT_CONNECTION_COST = 1000`。
- 句首 `initial_key` 的 right ID 固定 0（BOS），句尾 `final_connection_cost`
  查 `cost(rid, 0)`（EOS）。

`crates/jc-core/src/ja/decode.rs` 沒有任何「同讀音最低成本歸零」的正規化；
`crates/jc-jadic/src/import.rs` 匯入時也不做成本位移，只對
（surface, reading, POS, 活用, lid, rid）完全相同的重複列取最小值
（`import.rs` 的 `duplicates_dropped`）。`私` 的兩筆 lid 不同，兩筆都保留。

所以成本就是：

```
總成本 = Σ 詞成本 + conn(0, 第一個詞 lid) + Σ conn(前詞 rid, 後詞 lid) + conn(最後 rid, 0)
```

## 3. 9068 的每一項

路徑 `わたし` + `わ`，取 `私`（代名詞）與 `羽`（接尾）：

| 項目 | 值 | 來源 |
|---|---:|---|
| 私 | 263 | 名詞,代名詞,一般 lid=1902 rid=1902 |
| 羽 | 4349 | 名詞,接尾,一般 lid=1949 rid=1949 |
| BOS → 私 | 3490 | `matrix(0, 1902)` |
| 私 → 羽 | 966 | `matrix(1902, 1949)` |
| 羽 → EOS | 0 | `matrix(1949, 0)` |
| **合計** | **9068** | 4612 + 4456 |

對照 `私わ`：

| 項目 | 值 | 來源 |
|---|---:|---|
| 私 | 263 | 同上 |
| わ | 3 | 助詞,終助詞 lid=431 rid=431 |
| BOS → 私 | 3490 | `matrix(0, 1902)` |
| 私 → わ | 5556 | `matrix(1902, 431)` |
| わ → EOS | 0 | `matrix(431, 0)` |
| **合計** | **9312** | 266 + 9046 |

矩陣值直接從 Mozc 原始資料 `mozc-3.35.6354/connection_single_column.txt`
（row = 左詞 right ID、col = 右詞 left ID）覆核，四個值都相同：
`(0,1902)=3490`、`(1902,1949)=966`、`(1902,431)=5556`、`(1949,0)=0`。

## 4. 任務書表格漏掉的 `私`

任務書的原始資料表只列 `わたし / 私 / 1851 / 10227`。Mozc 實際上還有另一筆：

```text
わたし  1902  1902  263   私     # 名詞,代名詞,一般
わたし  1851  1851  10227 私     # 名詞,一般
```

解碼器選的是 263 那筆（代名詞）。若改用 1851 那筆，`私 + わ` 會是
10230 + conn(0,1851)=1066 + conn(1851,431)=5101 + conn(431,0)=0 = 16397，
更不可能勝出。因此「私 10227 + わ 3 應該贏」這個推論在兩個層面都不成立：
詞成本不是 10227，且成本不是只有詞成本。

## 5. `わ` 有沒有進候選

有。start 3 的 lattice 節點包含：

| surface | pos | lid | rid | cost |
|---|---|---|---:|---:|---:|
| わ | 助詞,終助詞 | 431 | 431 | 3 |
| 和 | 名詞,一般 | 1851 | 1851 | 4494 |
| 羽 | 名詞,接尾,助数詞 | 2011 | 2011 | 4241 |
| 羽 | 名詞,接尾,一般 | 1949 | 1949 | 4349 |
| 話 | 名詞,接尾,助数詞 | 2011 | 2011 | 2634 |

`convert` 這條路徑沒有 POS 過濾、沒有最小成本門檻、沒有把 `わ` 當未知詞。
`わ`（cost 3）就是 `私わ` 9312 的那個節點。`JaBuilder` 另外固定加一個
單かな未知詞節點（cost 8000），但它的成本遠高於真詞，不影響前兩名。

## 6. 為什麼不是 decoder 寫錯

同樣的資料丟給 Go 前身，答案一樣：

```text
$ /tmp/opencode/0c-jadic mozc -package-dir /mnt/work/jc/jc-ime-data/mozc-3.35.6354 -out /tmp/opencode/ja-mozc-go
$ /tmp/opencode/0c-jadic convert -data /tmp/opencode/ja-mozc-go -top 3 わたしわ
  [0] 私羽  分數 -9.068
  [1] 私わ  分數 -9.312
  [2] わたしわ  分數 -9.655
```

Go 的 `core/ja/decode/viterbi.go` 公式與 Rust 逐項相同：

- `h.cost + node.Entry.Cost + conn + lm`（`viterbi.go:194`）
- 句尾 `h.cost + conn`（`viterbi.go:209`）
- `connectionCost`：左有 right ID、右有 left ID 就 `cc.ContextCost(rid, lid)`，
  否則退回 POS 表（`viterbi.go:236`）
- 詞成本與連接成本都乘 `scale = 0.001`（`lexicon/bin.go`、`connection.go`）

Go 的 `0c-jadic mozc` 匯入會把 `connection_single_column.txt` 寫成
`connection.bin`（`0CJCON01`），`convert` 會自動載入，所以兩邊看到同一張
2672×2672 矩陣，得到同一組 9068／9312。

Go 直接讀 Rust 的 `.pack` 不可行：`internal/jadic` 只支援 v1（`FormatVersion = 1`、
`EntrySize = 20`、無 context ID／矩陣），這份 pack 是 v2（`entry_size = 24`、
`context_count = 2672`）。因此改用 Go 自己的 Mozc 匯入路徑做等價比對。

反過來說，如果把連接矩陣拿掉、每段改用固定詞性成本 1000，排序會變成
`わたしわ` 3003 > `私わ` 3266 > `私羽` 7612。這正是 curated 種子資料的處境，
也說明這題的名次完全由矩陣決定。

原本觀察到的「Go 是 `私わ`」則來自 curated 種子詞庫：

```text
$ /tmp/opencode/0c-jadic convert -data /mnt/work/jc/jc-ime-data/ja-src -top 3 わたしわ
  [0] 私わ  分數 -11.000
  [1] わたしわ  分數 -29.300
```

`ja-src/lexicon/words.tsv` 只有 140 筆，`わたし` 只有「私 0.8 noun」一筆，
沒有「羽」，`わ` 也不在詞庫，只能走未知詞（8000）加 POS 連接。那個環境下
根本沒有 `私羽` 這條路徑，「Go 比較喜歡 `私わ`」是同義反覆。

## 7. 決策

**不改 decoder。** 9068 是這份 Mozc 詞典加連接矩陣的正確最小值：

- 詞成本正確（263 + 4349）
- 連接成本正確（3490 + 966 + 0）
- 沒有正規化、沒有縮放誤差、沒有候選遺漏

若要讓 `わたしわ` 輸出 `私わ`，只能改資料或改產品偏好，不是修 decoder：

1. 換一份對「代名詞 → 終助詞」沒那麼貴的連接矩陣（重新匯入或不同來源）。
2. 在 decoder 加一條「句尾助詞優先」的驗證器或重排規則，但那會改變所有句子，
   需要評測數據支撐。
3. 接受 `私羽`：Mozc 原始資料下它就是最低成本路徑。

依任務書要求，沒有為了改變結果而動任何成本。

## 診斷工具

新增 `crates/jc-cli/src/bin/ja-cost.rs`：

```sh
cargo run -p jc-cli --release --bin ja-cost -- わたしわ
cargo run -p jc-cli --release --bin ja-cost -- --pack <pack> --limit 20 わたしわ
```

輸出 pack 絕對路徑與 SHA-256、完整 lattice（surface／reading／pos／lid／rid／cost／未知詞）、
所有路徑的詞成本／連接成本／總成本、前 3 條的逐項拆解、固定詞性連接的對照，
最後印 `JaDecoder::open_with_context` 的實際 N-best。

## Port 狀態

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/decode/viterbi.go` | `crates/jc-core/src/ja/decode.rs` | 已對照，計分公式逐項相同；新增 `test_real_mozc_pack_watashiwa_cost_breakdown` |
| `core/ja/decode/builder.go` | `crates/jc-core/src/ja/decode.rs`（`JaBuilder`） | 已對照，未知詞與 `prefer_hiragana` 行為相同 |
| `core/ja/lexicon/connection.go` | `crates/jc-core/src/ja/lexicon.rs`、`crates/jc-data/src/ja/pack.rs` | 已對照，矩陣值與 Mozc 原始檔一致 |
| `internal/jadic/pack.go` | `crates/jc-data/src/ja/pack.rs` | n/a：Go 只支援 v1，Rust 讀 v2，格式不同 |
| `cmd/0c-jadic/convert.go` | `crates/jc-cli/src/bin/ja-cost.rs` | rewritten：改成成本拆解診斷工具 |
