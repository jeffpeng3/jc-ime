# rerank-tokenizer：用真正的 SentencePiece 取代最長前綴

`rerank-fix` 量到 reranker 在整句候選上把 top-1 從 86.0% 拉到 40.8%，並列出兩個
原因：候選粒度（訓練是單 token、評測是整句）與 tokenizer（只有詞表、沒有 piece
分數，只能用最長前綴匹配近似）。這個 task 只解第二個，量測「把 tokenizer 換成
真正的 SentencePiece」能拉回多少。答案：幾乎沒有（40.8% → 41.0%）。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/spm/model.go`（手寫 protobuf walk、`trie`） | `crates/jc-model/src/spm.rs`（`prost` 最小 schema、`Trie`） | ported：改用 `prost` derive，不再手寫 wire 解析 |
| `internal/spm/encode.go`（`Normalize`、`EncodeTokens` 的 Viterbi、byte fallback） | `crates/jc-model/src/spm.rs`（`Normalizer::normalize`、`Model::encode_normalized`） | ported：`EncodeTokens` 的 span 回映（`NormalizeMap`、`Token.Start/End`）沒有搬，本 task 不需要 |
| `internal/spm/encode.go`（`NewEncoder` 的 trie 建表規則） | `crates/jc-model/src/spm.rs`（`Model::parse`） | ported：只把 NORMAL／USER_DEFINED 放進 trie，BYTE 只當輸出用；`min_score` 只看 NORMAL；USER_DEFINED 的 `0.1*(len-1)` 分數照搬 |
| `internal/spm/spm_test.go`（`TestNormalize`、`TestEncodeViterbi`、`TestEncodeUnknown`） | `crates/jc-model/src/tokenizer.rs`（`test_normalize`、`test_encode_viterbi`、`test_encode_unknown`） | ported：同一份測試模型與期望 id |
| `internal/spm/spm_test.go`（`TestGolden200`） | — | n/a：`sample-ja.txt` 不在資料集裡，改用語料 735 句直接對 Python 驗證（見下） |
| 前身只有 `vocab.json` 的推論路徑 | `crates/jc-model/src/tokenizer.rs`（`from_json`、`Strategy::MaxMatch`） | 保留：舊模型包與對照用 |

`internal/spm` 拒絕 `precompiled_charsmap`；Rust 版對 `identity` 與 `nfkc` 系列
規則放行（本模型是 `identity`、charsmap 長度 0），其它規則一律報錯，不猜。

## 實作

`spm.rs` 用 `prost` 解三塊：

- `ModelProto.pieces`：`piece`（1）、`score`（2）、`type`（3）。
- `TrainerSpec`：`model_type`（3）、`byte_fallback`（35）、`unk/bos/eos/pad_id`（40–43）。
- `NormalizerSpec`：`name`（1）、`precompiled_charsmap`（2）、
  `add_dummy_prefix`／`remove_extra_whitespaces`／`escape_whitespaces`（3–5）。

編碼：

1. 正規化：規則名含 `nfkc` 先做 NFKC（`unicode-normalization`），再合併 ASCII
   空白、加 dummy prefix、空白轉 U+2581。本模型的 `identity` 規則不做字元折疊，
   所以 `\t`、U+3000 原樣保留（與 C++ 一致）。
2. lattice：每個起始字元用 piece 前綴樹（攤平的 `HashMap<(節點, 字元), 節點>`）
   找所有前綴；該位置沒有任何單字 piece 時放一個 unknown 節點，分數
   `min_score - 10`。
3. Viterbi：取整句總分最高的切法，同分保留先插入的節點（與 C++／Go 相同）。
4. 輸出：unknown 節點在 `byte_fallback` 開啟時展開成 UTF-8 byte piece，
   詞表缺該 byte piece 就退回 `<unk>`。

`layout.rs` 不用改：左右 60 token 的切法本來就呼叫 `Tokenizer::encode`，換來源
後自動吃到 piece 分數。新增一個用真模型的測試，確認左文視窗真的被填滿 60 格、
靠右對齊、保留前段（與前身 `capIDs` 一致）。

## 驗證

- 對 `modern_500`、`anthy_corpus1`、`hard_cases` 共 735 句 surface，與 Python
  `sentencepiece` 0.2.3 的 `encode(out_type=int)` 逐句比對：**0 句不同**。
- 單元測試內嵌 8 組期望 id（含 `ｶﾀｶﾅ`、`😀`、`<0x41>` 這種會被誤判成 byte
  piece 的字面字串），值由同一份 Python 產生。
- 前身三個測試（`TestNormalize`、`TestEncodeViterbi`、`TestEncodeUnknown`）照搬
  id 與分數，確認 Viterbi 與 Go 版一致。

## 20 句對照

`modern-500` 每 25 句抽一句，同一份詞表跑兩種切法：

| # | 句子 | unigram | max-match | 相同？ |
|---:|---|---|---|---|
| 1 | 私は本を読む。 | `▁·私は·本·を·読む·。` | `▁·私は·本·を·読む·。` | ✅ |
| 2 | 彼はご飯を食べる。 | `▁·彼は·ご·飯を食·べる·。` | `▁·彼は·ご·飯を食·べる·。` | ✅ |
| 3 | 学生は本を読む。 | `▁·学生·は·本·を·読む·。` | `▁学·生·は·本·を·読む·。` | **❌** |
| 4 | 家族はご飯を食べる。 | `▁·家族·は·ご·飯を食·べる·。` | `▁·家族·は·ご·飯を食·べる·。` | ✅ |
| 5 | 私は学校に自転車で行く。 | `▁·私は·学校·に·自転車·で·行く·。` | `▁·私は·学校·に·自転車·で·行く·。` | ✅ |
| 6 | 彼は店に自転車で行く。 | `▁·彼は·店·に·自転車·で·行く·。` | `▁·彼は·店·に·自転車·で·行く·。` | ✅ |
| 7 | 学生は学校に自転車で行く。 | `▁·学生·は·学校·に·自転車·で·行く·。` | `▁学·生·は·学校·に·自転車·で·行く·。` | **❌** |
| 8 | 家族は店に自転車で行く。 | `▁·家族·は·店·に·自転車·で·行く·。` | `▁·家族·は·店·に·自転車·で·行く·。` | ✅ |
| 9 | 私は今日に学校へ行く。 | `▁·私は·今日·に·学校·へ行く·。` | `▁·私は·今日·に·学校·へ行く·。` | ✅ |
| 10 | 彼は週末に学校へ行く。 | `▁·彼は·週·末·に·学校·へ行く·。` | `▁·彼は·週·末·に·学校·へ行く·。` | ✅ |
| 11 | 学生は今日に学校へ行く。 | `▁·学生·は·今日·に·学校·へ行く·。` | `▁学·生·は·今日·に·学校·へ行く·。` | **❌** |
| 12 | 家族は週末に学校へ行く。 | `▁·家族·は·週·末·に·学校·へ行く·。` | `▁·家族·は·週·末·に·学校·へ行く·。` | ✅ |
| 13 | 本について学校で話す。 | `▁本·について·学校·で·話す·。` | `▁本·について·学校·で·話す·。` | ✅ |
| 14 | 音楽について店で話す。 | `▁·音楽·について·店·で·話す·。` | `▁·音楽·について·店·で·話す·。` | ✅ |
| 15 | ご飯について学校で話す。 | `▁·ご·飯·について·学校·で·話す·。` | `▁·ご·飯·について·学校·で·話す·。` | ✅ |
| 16 | 花について店で話す。 | `▁·花·について·店·で·話す·。` | `▁·花·について·店·で·話す·。` | ✅ |
| 17 | 今日は天気がいい。 | `▁·今日は·天気·がいい·。` | `▁·今日は·天気·がいい·。` | ✅ |
| 18 | 朝は雪が少ない。 | `▁·朝·は·雪·が少ない·。` | `▁·朝·は·雪·が少ない·。` | ✅ |
| 19 | 週末は天気がいい。 | `▁·週·末·は·天気·がいい·。` | `▁·週·末·は·天気·がいい·。` | ✅ |
| 20 | 金曜日は雪が少ない。 | `▁·金·曜日·は·雪·が少ない·。` | `▁·金·曜日·は·雪·が少ない·。` | ✅ |

4／20 不同，成因都一樣：詞表沒有 `▁学生`，最長前綴只能吃 `▁学`＋`生`，
unigram 則發現「`▁`＋`学生`」更便宜。

| piece | id | 分數 |
|---|---:|---:|
| `▁` | 263 | −3.612 |
| `▁学` | 7077 | −10.747 |
| `学生` | 901 | −8.790 |
| `生` | 528 | −8.167 |

- unigram：`▁` + `学生` = −12.402
- max-match：`▁学` + `生` = −18.914

差 6.51 分，正好是「学生は本を読む。」兩者總分的差距（−44.24 對 −50.75）。
`▁家族`、`▁今日` 這類詞也不在詞表，但後面的 token 剛好一樣，所以總數沒變。

整體影響：

| 語料 | 切法不同 | unigram token | max-match token |
|---|---:|---:|---:|
| `modern-500` | 40 / 500（8.0%） | 3580 | 3580 |
| 三份題庫合計 | 93 / 735（12.7%） | 5249 | 5256 |

token 數幾乎不變（都在句首那個 `▁`），所以張量形狀、60 token 視窗都不受影響，
只有候選字的 piece id 換了。這也解釋了為什麼下面的量測幾乎沒動。

## 量測

`modern-500`，同一份 `model.int8.onnx`；只有 tokenizer 換掉
（`JC_JA_MODELS` 指向一份把 `ja-rerank.vocab` 指到
`models/ja-tokenizer/public_core_ja.model` 的 manifest）：

| 設定 | tokenizer | top-1 | top-5 | boundary | p50 | p95 | changed |
|---|---|---:|---:|---:|---:|---:|---:|
| `--rerank off` | 任一 | 86.0% | 98.0% | 88.2% | 3.03 ms | 5.91 ms | — |
| `--rerank on` | 最長前綴（舊） | 40.8% | 98.0% | 68.2% | 30.48 ms | 41.02 ms | 275 |
| `--rerank on` | SentencePiece | **41.0%** | 98.0% | 66.0% | 25.97 ms | 34.31 ms | 274 |
| `--rerank on --left rolling` | 最長前綴（舊） | 57.8% | 98.0% | 76.2% | 28.83 ms | 40.33 ms | 169 |
| `--rerank on --left rolling` | SentencePiece | **54.8%** | 98.0% | 75.0% | 31.28 ms | 44.97 ms | 176 |

舊 tokenizer 那兩列是這個 worktree 重跑的對照組，數字與 `rerank-fix` 完全一致，
所以差異只可能來自 tokenizer。推論時間不變（p50 20–23 ms，8 執行緒）。

## 結論

tokenizer 修正只換回 **0.2 個百分點**（40.8% → 41.0%），滾動左文甚至退 3.0 個
百分點（57.8% → 54.8%）。差距 45 個百分點沒有回來，`rerank-fix` 的判斷成立：

**候選粒度才是主要限制**。訓練時候選是單一 SentencePiece token，評測餵的是整句
surface（6–10 個 token、含標點），模型看到的分佈完全不同；tokenizer 再準，
餵進去的東西也不是它學過的東西。滾動左文的差異則是模型對左文長度／內容很敏感，
1 個百分點的 token 差就能換掉 7 題的排序，屬於雜訊等級，沒有可用訊號。

要讓 reranker 真的有用，得改候選單位（文節／單詞候選）或重訓容忍整句候選的
模型；兩者都不在這個 task 的 owned paths。`--rerank on` 維持 opt-in、預設關閉。

## 重現

```sh
export ORT_DYLIB_PATH=/path/to/libonnxruntime.so.1.32.0
# manifest 把 ja-rerank.vocab 指到 models/ja-tokenizer/public_core_ja.model
export JC_JA_MODELS=/path/to/model-set
cargo run -p jc-cli --release --features onnx -- eval ja --corpus modern-500 --rerank off
cargo run -p jc-cli --release --features onnx -- eval ja --corpus modern-500 --rerank on
cargo run -p jc-cli --release --features onnx -- eval ja --corpus modern-500 --rerank on --left rolling
```

兩種切法的對照（同一份詞表）：

```sh
cargo run -p jc-cli -- tokenize ja --text "学生は本を読む。"
cargo run -p jc-cli -- tokenize ja --diff < 題庫句子.txt
```
