# ja-particle-rewrite：助詞讀音交替

`わたしわ` → `私は`、`ほんおよむ` → `本を読む`、`にほんえいく` → `日本へ行く`。

## 決策：讀音交替，不改寫詞面

前身的 `core/ja/rewrite` 沒有助詞改寫，Go 也沒有可對照的實作；這條規則是
本任務新增的日文轉換行為。

實作放在 **lattice 建立階段**：對輸入讀音的每個「わ」「え」「お」，在同一個
詞的長度範圍內額外嘗試「は」「へ」「を」的讀音，讓詞典與 Viterbi 決定是否
採用。詞面不經任何硬編表改寫。

選讀音層而不是表層層的原因：

- `わたしわ` 的統計第一名是 `私羽`（9068），`私わ` 是第二名（9312）。
  Viterbi 之後才改寫，路徑上沒有「わ 助詞」節點，永遠得不到 `私は`。
- 讀音層把 `は` 助詞的詞弧放進 lattice 後，`私 + は` 是 4423，自然勝出，
  不需要調成本或連接矩陣。

選讀音交替而不是「わ→は 對照表」的原因：

- `すなわち`、`いわば` 本來就以「わ」表記。對照表會把它們改成錯的
  `すなはち`、`いはば`；讀音交替只是多一個候選，詞典原本的 `すなわち`、
  `いわば` 成本更低，結果不變。

## 規則

| 輸入讀音 | 交替讀音 | 接受的詞條 | 例子 |
|---|---|---|---|
| わ | は | 助詞，或讀音超過一單位、詞典真的以「は」表記的詞 | `わたしわ` → `私は`、`こんにちわ` → `こんにちは`、`あるいわ` → `あるいは` |
| え | へ | 只接受助詞 | `にほんえいく` → `日本へ行く`、`がっこうえいく` → `学校へ行く` |
| お | を | 只接受助詞 | `ほんおよむ` → `本を読む`、`みずおのむ` → `水を飲む` |

限制的原因：

- `え→へ` 若開放給一般詞，`えき` 會多出 `へき`（壁）的候選，實測讓
  modern-500 的 `〜についてえきではなす` 從 `駅` 變成 `壁`。
- `お→を` 同理，`おちゃ` 不能變成 `をちゃ`。
- `わ→は` 必須保留整詞匹配，否則 `こんにちは`、`あるいは` 這種把助詞
  讀音含在詞內的詞條查不到。

## 實作位置

`crates/jc-core/src/ja/decode.rs` 的 `JaBuilder::build`：

1. 先做原本的 `source.matches(reading, start, ...)`。
2. `particle_rewrite` 開啟時，逐一檢查 `start..start+longest` 的每個單位。
3. 遇到「わ」「え」「お」時複製讀音、替換該單位，再查一次詞典。
4. 只收「詞條讀音在該位置真的使用交替字元」的匹配，其餘與第一次查詢重複。
5. `particle_alternation_allowed` 依上表過濾詞條。
6. 未知詞 fallback、學習鉤子、`prefer_hiragana` 與 Viterbi 都不改。

`crates/jc-core/src/ja/rewrite.rs` 只提供純函式
`particle_reading_alternative`：`わ→は`、`え→へ`、`お→を`，其餘回 `None`。

開關：

- 核心 `JaDecoder::with_particle_rewrite(bool)` 預設關閉，既有統計行為不變。
- `jc-engine` 的 `loader::load_ja_decoder`、`load_ja_decoder_with_pack` 與
  `load_curated_decoder` 一律開啟；`jc-cli` 直接沿用 loader，因此 CLI、引擎與
  測試使用同一條路徑。

## Mixed golden 的期望值

`golden/mixed` 的 `watashiwa` 讀音是 `わたしわ`。日文助詞 `は` 唸 `わ`，所以
`わたしわ` 的正確表記是 `私は`；`私わ` 不是標準日文。助詞讀音交替把 `は` 的詞弧
放進 lattice 後，`私 + は` 勝出，`scenario-a`／`scenario-b` 都輸出 `私は`。

因此 fixture 的 `surface` 與 `output` 從 `私わ` 改成 `私は`。比較仍然逐片段比
邊界、語言、書寫系統與表面文字，沒有放寬。改寫前的 Rust 統計首位是 `私羽`
（9068），`私わ` 是 9312；兩者都不該是這句的期望值。

## 驗證

```sh
cargo run -p jc-cli --release -- convert "わたしわ"
cargo run -p jc-cli --release -- convert "こんにちわ"
cargo run -p jc-cli --release -- convert "あるいわ"
cargo run -p jc-cli --release -- convert "ほんおよむ"
cargo run -p jc-cli --release -- convert "にほんえいく"
cargo run -p jc-cli --release -- convert "すなわち"
cargo run -p jc-cli --release -- convert "いわば"
cargo run -p jc-cli --release -- eval ja --corpus modern-500
cargo run -p jc-cli --release -- eval ja --corpus hard-cases
cargo run -p jc-cli --release -- eval ja --corpus ajimee-200
```

| 題目 | 關閉 | 開啟 |
|---|---|---|
| `わたしわ` | 私羽 | 私は |
| `こんにちわ` | こんにちわ | こんにちは |
| `あるいわ` | あるいわ | あるいは |
| `ほんおよむ` | 本お読む | 本を読む |
| `にほんえいく` | 日本栄区 | 日本へ行く |
| `すなわち` | すなわち | すなわち |
| `いわば` | いわば | いわば |

| 題庫 | 改寫前 top1 | 改寫後 top1 |
|---|---:|---:|
| modern-500 | 86.0% | 86.0% |
| hard-cases | 54.3% | 54.3% |
| ajimee-200 | 46.0% | 46.0% |

逐題 `--items` 比對三組題庫的 top1 輸出，開啟與關閉完全相同；modern-500
的 top5、boundary_exact、boundary_f1 也相同。

## Go 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/decode/builder.go` | `crates/jc-core/src/ja/decode.rs`（`JaBuilder`） | rewritten：新增 `particle_rewrite` 與交替讀音查詢；Go 版沒有這個行為 |
| `core/ja/decode/viterbi.go` | `crates/jc-core/src/ja/decode.rs` | n/a：Viterbi 未改，交替讀音只是多產生詞弧 |
| `core/ja/rewrite/rewriter.go` | `crates/jc-core/src/ja/rewrite.rs` | n/a：前身只有かな、英數、數字與記号候選，沒有助詞改寫可移植 |
| `core/ja/rewrite/kana.go` | `crates/jc-core/src/ja/rewrite.rs` | n/a：本任務不改かな正規化 |
| `cmd/0c-jadic/convert.go` | `crates/jc-cli/src/cli.rs`、`crates/jc-cli/src/data.rs` | rewritten：CLI 載入解碼器時開啟助詞讀音交替 |

## 測試

`crates/jc-core/src/ja/decode.rs`：

- `test_particle_rewrite_reading_alternation`：開／關對照，涵蓋
  `わたしわ`、`こんにちわ`、`あるいわ`、`ほんおよむ`、`みずおのむ`、
  `にほんえいく`、`がっこうえいく`，並確認 `えき`、`おちゃ` 不受影響。
- `test_particle_rewrite_keeps_existing_wa_words`：`すなわち`、`いわば`
  開啟前後一致。

`crates/jc-core/src/ja/rewrite.rs`：

- `test_particle_reading_alternative_table`：交替表與非交替字元。
