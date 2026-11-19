# mixed-e2e：Mixed 端到端轉換

`Router::convert` 現在會把整句混合輸入實際轉成文字，`jc-cli eval mixed`
逐句比對片段邊界、語言、書寫系統、片段文字與整句輸出，6 個 golden 案例全部
一致。日文片段 `watashiwa` 經助詞讀音交替輸出 `私は`，原因見〈期望值改為
私は〉。

## 對照表

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/mixed/converter.go` | `crates/jc-core/src/mixed.rs`：`Router::convert`、`convert_plan` | rewritten：Go 包住 `lang.Converter`；Rust 直接持 `Decoder`、`JaDecoder`、`English`，逐段路由 |
| `core/ja/mixed/composer.go`（`groupWords`） | `crates/jc-core/src/mixed.rs`：`group_words` | rewritten：修正片語掃描索引，單一空白相連的小寫英文字會一起判定 |
| `core/ja/mixed/literal` 符號處理 | `crates/jc-core/src/mixed.rs`：`convert_plan` 的 `Mode::Ja` 分支 | rewritten：符號與非羅馬字字面不進日文解碼器，照原樣輸出 |
| `core/lattice/scenario_test.go`（情境 A／B） | `crates/jc-cli/src/eval_cmd.rs`：`mixed`、`golden/mixed/cases.json` | rewritten：用真正的 `Router` 跑整句，比邊界也比表面文字 |
| `cmd/0c-eval-mixed`（`run`） | `crates/jc-cli/src/eval_cmd.rs`：`mixed` | rewritten：只跑 `golden/mixed` 的 6 個案例，輸出表格或 `--json` |
| `core/session/automatic_mixed_test.go`、`core/session/ja_script_test.go` | — | n/a：本 task 驗的是 `Router` 整句轉換；session 的按鍵推斷屬 M4，尚未接上日文片段 |
| `internal/golden/mixed.go` | `crates/jc-eval/src/golden.rs`（既有） | n/a：loader 已在 w1-eval 完成，本 task 只使用 |

## 端到端一例

輸入：

```
watashiwaㄉㄚˋㄐㄧㄅㄚdesu is a good sentence
```

前身 fixture 的期望輸出（已更新為 `私は`）：

```
私わ大雞巴です is a good sentence
```

Rust `jc-cli eval mixed` 實際輸出：

```
私は大雞巴です is a good sentence
```

更新後的 fixture 與實際輸出完全一致。注音片段 `大雞巴`、日文 `です`、空白與
英文 `is a good sentence` 都相同，片段邊界也一致。`watashiwa` 的讀音是
`わたしわ`，助詞 `は` 唸 `わ`，所以 `私は` 才是正確表記；fixture 已改成
`私は`。

## 6 個案例

`cargo run -p jc-cli -- eval mixed` 的實際結果：

| 案例 | 期望輸出 | 實際輸出 | 結果 |
|---|---|---|---|
| `scenario-a-primary-ja` | `私は大雞巴です is a good sentence` | `私は大雞巴です is a good sentence` | 一致 |
| `scenario-b-primary-zh` | `私は大雞巴です is a good sentence` | `私は大雞巴です is a good sentence` | 一致 |
| `scenario-c-proper-nouns` | `kubectl pCloud GitHub` | `kubectl pCloud GitHub` | 一致 |
| `long-word-cross-segment-zhuyin` | `麻婆豆腐` | `麻婆豆腐` | 一致 |
| `long-word-cross-segment-kana` | `東京大学` | `東京大学` | 一致 |
| `long-word-cross-segment-zhuyin-three-pieces` | `中華民國` | `中華民國` | 一致 |

片段層級的邊界、語言與書寫系統：6 個案例全部一致。

## 修正

1. `group_words` 第一個迴圈從 `j = i` 開始卻拿 `atoms[j]` 當空白，條件永遠
   不成立，片語從來沒有合併。改成看 `atoms[j + 1]` 的空白與 `atoms[j + 2]`
   的單字後，`desu is a good sentence` 正確切成「日文 `desu`＋符號空白＋
   英文片語 `is a good sentence`」。
2. 日文片段的符號（空白）原本會送進 `JaDecoder`，`kana` 為空時輸出空字串。
   現在 `Mode::Ja` 只解碼真的由羅馬字還原的かな；符號與其他字面照原樣輸出。

## 期望值改為 私は

`scenario-a`／`scenario-b` 的 `watashiwa` 讀音是 `わたしわ`。日文助詞 `は` 唸
`わ`，所以正確表記是 `私は`；`私わ` 不是標準日文。助詞讀音交替在 lattice
階段把 `は` 的詞弧放進候選，`私 + は` 勝出，因此 `jc-cli` 與 `jc-engine`
都輸出 `私は`。

fixture 的 `surface` 與 `output` 已從 `私わ` 改成 `私は`，比較邏輯沒有放寬：
仍然逐片段比邊界、語言、書寫系統與表面文字。改寫前的統計首位 `私羽`（9068）
與亞軍 `私わ`（9312）都不該是這句的期望值；成本差異的歷史診斷見
`docs/port/ja-cost-diag.md`。

## 驗證

```sh
cargo check -p jc-core -p jc-cli --all-targets
cargo run -p jc-cli -- eval mixed
cargo run -p jc-cli -- eval mixed --json
```

`eval mixed` 的比對項目：

- 整句輸出與 `cases.json` 的 `output`；
- 片段數；
- 每個片段的輸入邊界（`span` 由 rune 換成 UTF-8 位元組）；
- 每個片段的語言與書寫系統；
- 每個片段的表面文字。

`--data` 未指定時依序使用 `<資料根>/build-zh` 的二進位資料包與
`golden/zh/data` 的固定詞庫。
