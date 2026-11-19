# mixed-e2e：Mixed 端到端轉換

`Router::convert` 現在會把整句混合輸入實際轉成文字，`jc-cli eval mixed`
逐句比對片段邊界、語言、書寫系統、片段文字與整句輸出。6 個 golden 案例
有 4 個完全一致，2 個只差日文詞面 `私わ` / `私羽`，原因見〈已知差距〉。

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

前身（`golden/mixed/cases.json` 的 `output`，Go `0c-eval-mixed` 的
`a-romaji-spec` 也是同一個詞面）：

```
私わ大雞巴です is a good sentence
```

Rust `jc-cli eval mixed` 實際輸出：

```
私羽大雞巴です is a good sentence
```

差異只有第一個日文片段：前身是 `私わ`，Rust 是 `私羽`。注音片段
`大雞巴`、日文 `です`、空白與英文 `is a good sentence` 都一致，片段邊界也
一致。

## 6 個案例

`cargo run -p jc-cli -- eval mixed` 的實際結果：

| 案例 | 期望輸出 | 實際輸出 | 結果 |
|---|---|---|---|
| `scenario-a-primary-ja` | `私わ大雞巴です is a good sentence` | `私羽大雞巴です is a good sentence` | 只差日文詞面 |
| `scenario-b-primary-zh` | `私わ大雞巴です is a good sentence` | `私羽大雞巴です is a good sentence` | 只差日文詞面 |
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

## 已知差距

`scenario-a`／`scenario-b` 的 `私わ` 在前身是 `私`＋`わ` 兩個詞面，Rust
目前把 `わたしわ` 整段交給日文解碼器，Mozc v2 pack 的成本是：

| 詞面 | 成本 |
|---|---:|
| `私羽` | 9068 |
| `私わ` | 9312 |

差距 244，`PreferHiragana` 的規則只排除片假名與拉丁詞面，不會排除同讀音的
漢字詞面，因此 `私羽` 勝出。前身 `0c-eval-mixed` 搭配 public-core pack
實際輸出 `私わ`，但 Rust 用同一個 public-core pack 的 n-best 首位是
`ワタシわ`，表示差異在 `crates/jc-core/src/ja/decode.rs` 與詞典載入，不在
mixed 路由。這需要日文 reranker／模型或 `ja/decode` 的成本調整，屬 M4
範圍，且不在本 task 的 owned paths，因此保留差距並記錄。

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
