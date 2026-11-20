# punct-wire：標點模型接進輸出

> **已移除（2026-11-20）**：模型標點整條路徑已從程式刪除，`--punct`、
> `SessionConfig::punct` 與 `SharedPunctuator` 都不存在。本文保留為歷史紀錄；
> 保留的規則是 `ctrl_punct`（`Ctrl+,` 全形逗號）與 `punct_mark`（送出前插入
> 單一標點）。移除清單見 [`drop-punct.md`](drop-punct.md)。

把日文 `ja-punct` 與注音 `zh-punct` 兩個標點模型接進送出階段。預設關閉；
`--punct on` 或 `SessionConfig::punct` 有值時才推論。模型缺席、建置沒有
`onnx`、逾時或推論失敗都回傳原文，不改動任何原字。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/punct/punct.go`（`Marks`、`Edit`、`Apply`、`ApplyJapanese`） | `crates/jc-core/src/punct.rs` | ported：先前 task 已完成，本 task 沿用 |
| `core/ja/punct/punct.go`（規則式 `Completer`：接續詞、括號、句尾） | — | n/a：部署改由模型判斷，不再使用規則式補標點；只沿用它的觸發時機與「只插入、不重寫」契約 |
| `core/output/punctuate.go`（`LanguagePunctuator` 半形轉全形） | `crates/jc-core/src/output/mod.rs` | ported：先前 task 已完成，本 task 不動 |
| `internal/neural/native/punct_plan.go`（manifest、視窗、merge、decision、`charEdits`、`tokenEdits`、`eligibleBoundaries`） | `crates/jc-model/src/punct_plan.rs`、`punct_zh.rs`、`punctuation_model.rs` | ported／rewritten：純函式照搬；manifest 只保留本引擎需要的欄位 |
| `internal/neural/native/punct_ort.go`（`PunctBackend`） | `crates/jc-model/src/punctuation_model.rs`（日文）、`punct_zh.rs`（注音） | rewritten：同步、單一 session、100 ms watchdog，不加 async runtime |
| `internal/neural/punct_manifest.go` | `crates/jc-model/src/punct_zh.rs`、`manifest.rs` | rewritten：日文沿用 `ModelSpec`，注音另讀 `zuin-punct-runtime-v1` |
| `core/session/session.go`（`punctuateWith`、`punctuateJapanese`） | `crates/jc-core/src/session.rs`（`Session::commit`、`model_punctuated`） | rewritten：改成 trait 介面，提交邊界呼叫 |
| `core/session/language.go`（`languageCommit` 呼叫 `punctuateJapanese`） | `crates/jc-core/src/session.rs` | rewritten：注音與日文共用同一條 commit 路徑 |
| `cmd/0c-server`（模型包標點設定） | `crates/jc-cli/src/cli.rs`、`converter.rs`、`data.rs` | rewritten：`convert --punct on\|off` |
| `core/session/punct_test.go` | `crates/jc-core/src/session.rs`、`crates/jc-model/src/punctuation_model.rs` | rewritten：行為測試加純函式測試，不搬 Go 的 session 內部時序 |

## 觸發時機

- 送出時補一次：`Session::commit()` 取到的文字（含 Enter 先放進
  `pending_commit` 的結果）在清空組字前經過模型。
- 使用者已輸入標點時不重複：注音模型只在「漢字後、下一個字也是漢字」的
  邊界插入，段尾永不插入；日文模型套用前會跳過緊鄰既有標點的插入點，
  所以 `今日は寒い。`、`元気ですか？` 不會變成 `。。`、`？？`。
- 英文、私人或暫停中的輸入不補；`SessionConfig::punct` 為 `None` 時完全
  不呼叫模型。
- 模型不可用、feature 關閉、推論超過 100 ms 或輸出不合契約時回傳原文。

## 模型與前處理

日文（`ja-punct`，`punctuation-public-core/`）：

- 輸入 `input_ids`／`attention_mask`，輸出 `logits [1, seq, 4]`。
- 標籤順序固定 `none, comma, period, question`，對應標點
  `""`、`、`、`。`、`？`。
- 前處理沿用 `model-wire-ja` 的 `Tokenizer`：dummy prefix、空白轉 `▁`、
  最長前綴匹配、詞表外 byte fallback。
- 視窗 256 token、stride 128；每個位置取離視窗邊緣最遠的預測。
- token 結尾經 `normalize_map` 映射回原文 rune；byte fallback 與空白
  token 不插入。

注音（`zh-punct/`）：

- 輸入 `ids [1, L]`，內容是 `[BOS=1, TASK=7, 原字 id...]`；輸出
  `probs [L, 7]`。
- 標籤 `""`、`，`、`。`、`？`、`！`、`、`、`；`；字元 id 是
  `characters[i] + 8`，詞表外送 `[UNK]=4`。
- 視窗 128、stride 64、門檻 0.63、單次上限 256 字，全部讀
  `manifest.json`，缺欄位時用同一個預設。
- 只在漢字後、下一個字是漢字或空白時考慮；下一個可見字元必須是漢字或
  ASCII 英數；網址內不插；段尾永不插入。句首單一列舉字（一、二、甲…）
  後不補頓號。

## 執行

```sh
export ORT_DYLIB_PATH=/path/to/libonnxruntime.so

cargo run -p jc-cli --release --features onnx -- convert "わたしはほんをよむ" --punct on
cargo run -p jc-cli --release --features onnx -- convert "ㄏㄠˇ ㄉㄜ˙ ㄨㄛˇ ㄓ ㄉㄠˋ ㄌㄜ˙ ㄇㄧㄥˊ ㄊㄧㄢ ㄐㄧㄢˋ" \
  --data /mnt/work/jc/jc-ime-data/build-zh --punct on
```

模型集合由 `JC_MODELS` 或 `--testdata` 上層的 `models/` 決定。`--punct`
預設 `off`；off 時不載入模型，輸出與改動前逐字相同。

## 驗證

環境：GB10，onnxruntime 1.32.0，release 建置，`intra_threads = 1`。
`convert` 一次轉 5 個候選，因此每次量到 5 次推論。

### 日文：無標點輸入

| 讀音 | top-1 輸出 |
|---|---|
| `わたしはねこがすきです` | 私は猫が好きです。 |
| `あしたはあめがふるでしょう` | 明日は雨がフルでしょう。 |
| `これはほんです` | これは本です。 |
| `やまださんはがくせいです` | 山田さんは、学生です。 |
| `げんきですか` | 元気ですか。 |

推論延遲 p50 1.40–1.83 ms、p95 1.85–2.40 ms。

### 日文：已有標點

| 輸入 | 輸出 |
|---|---|
| `今日は寒い。` | 今日は寒い。 |
| `元気ですか？` | 元気ですか？ |
| `東京に行く！` | 東京に行く！ |
| `わたしは、本を読む` | わたしは、本を読む、 |
| `私は猫が好きです。` | 私は、猫が好きです。 |

既有標點沒有被重複插入；模型仍可能在另一個合法邊界加標點。

### 注音：無標點輸入

`--data /mnt/work/jc/jc-ime-data/build-zh`：

| 讀音 | top-1 輸出 |
|---|---|
| `ㄏㄠˇ ㄉㄜ˙ ㄨㄛˇ ㄓ ㄉㄠˋ ㄌㄜ˙ ㄇㄧㄥˊ ㄊㄧㄢ ㄐㄧㄢˋ` | 好的我知道了，明天見 |
| `ㄐㄧㄣ ㄊㄧㄢ ㄊㄧㄢ ㄑㄧˋ ㄏㄣˇ ㄏㄠˇ` | 今天天氣很好 |
| `ㄨㄛˇ ㄞˋ ㄋㄧˇ` | 我愛你 |
| `ㄒㄧㄤˇ ㄔ ㄇㄚˊ ㄆㄛˊ ㄉㄡˋ ㄈㄨˇ` | 想吃麻婆豆腐 |
| `ㄔㄥˊ ㄕˋ ㄏㄣˇ ㄇㄟˇ` | 城市很美 |

推論延遲 p50 3.99–5.99 ms、p95 4.18–6.67 ms。注音模型依模型卡不在段尾
補標點，所以短句可能維持原文。

### 注音：已有標點

| 輸入 | 輸出 |
|---|---|
| `好的我知道了，明天見` | 好的，我知道了，明天見 |
| `今天天氣很好。` | 今天天氣很好。 |
| `我愛你！` | 我愛你！ |
| `城市很美？` | 城市很美？ |
| `請記得帶資料` | 請記得帶資料 |

## 已知限制

- 日文模型在短句上常把 `、` 與 `。` 判得很接近；量化模型會依上下文在
  兩個標籤間擺動，輸出品質尚未達產品驗收。預設關閉。
- 注音模型只還原句中標點，不補段尾；`我愛你` 這類短句不會變成
  `我愛你。`。
- `convert` 目前把模型套用到全部候選；排序不變，只有詞面改變。
- 只做同步推論，沒有取消與背景候選窗；100 ms 預算由 watchdog 中止。
