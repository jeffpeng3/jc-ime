# eval-fuzzy：模糊音評測

## 執行

```sh
cargo run -p jc-cli --release -- eval fuzzy
cargo run -p jc-cli --release -- eval fuzzy --json
```

`eval fuzzy` 讀 `0c-testdata/fuzzy/fuzzy-input.json`，先比對
`fuzzy-input.manifest.json` 的雜湊、題數、seed 與語言／擾動計數；不符就
失敗。注音走產品預設的 `select` 模糊音設定（`z-zh`、`c-ch`、`s-sh`、
`en-eng` 全部 select），日文走同一份 pack 的統計轉換器 N-best。沒有日文
pack 時日文案例仍留在分母，候選全部算未命中並記 `no_decoder`。

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/fuzzy/fuzzy.go` | `crates/jc-core/src/zh/fuzzy.rs` | ported：配對、模式、`Alternatives`、`Rules`、`Widen`、`Frontend`、`AutoPolicy` |
| `core/fuzzy/fuzzy_test.go` | `crates/jc-core/src/zh/fuzzy.rs` 測試 | ported：含 419 個基底的 254／144／21／20 分支分布 |
| `internal/eval/fuzzy_corpus.go` | `crates/jc-eval/src/corpus.rs`、`crates/jc-eval/src/fuzzy.rs` | ported：題庫 loader 已在 `corpus.rs`；manifest 與雜湊比對搬到 `fuzzy.rs` |
| `internal/eval/fuzzy_run.go` | `crates/jc-eval/src/fuzzy.rs`、`crates/jc-cli/src/eval_cmd.rs` | rewritten：沒有 `engine.Session` 抽象，注音改由 `fuzzy::Config::variants` 展開後合併 N-best；日文候選直接走 `JaDecoder` |
| `internal/eval/fuzzy_test.go` | 同上兩處測試 | rewritten：保留可重現性與分母檢查，拿掉 Go 專屬的 stub decoder 與 build 測試 |
| `cmd/0c-eval-fuzzy/main.go`（`run`） | `crates/jc-cli/src/eval_cmd.rs::fuzzy` | rewritten：改成 `jc-cli eval fuzzy`，輸出 `FuzzyReport` |
| `cmd/0c-eval-fuzzy/main.go`（`build`） | n/a | 題庫已凍結；重建屬資料工作，不在這個 crate |
| `internal/fuzzyeval/fuzzyeval.go` | n/a | 那是 issue #45 的近音噪音評測（`Noise`／`clean`／`noisy` 角色），不是 P2-62 題庫；本任務只移植它的規則依賴 `core/fuzzy` |
| `internal/fuzzyeval/fuzzy_test.go` | n/a | 同上，測試對象是另一套評測流程 |

## 數字（2026-11-19，seed 62）

`cargo run -p jc-cli --release -- eval fuzzy`：

| 語言 | 擾動 | n | raw top1 | raw top5 | raw 文字 | fallback top1 | fallback top5 |
|---|---|---:|---:|---:|---:|---:|---:|
| zh | doubled-key | 92 | 100.0% | 100.0% | 100.0% | 100.0% | 100.0% |
| zh | missing-key | 92 | 5.4% | 5.4% | 5.4% | 5.4% | 5.4% |
| zh | near-sound | 43 | 100.0% | 100.0% | 100.0% | 100.0% | 100.0% |
| zh | wrong-key | 92 | 2.2% | 2.2% | 2.2% | 2.2% | 2.2% |
| zh | 合計 | 319 | 44.5% | 44.5% | 44.5% | 44.5% | 44.5% |
| ja | doubled-key | 200 | 0.0% | 0.0% | 2.5% | 0.0% | 0.0% |
| ja | missing-key | 200 | 4.0% | 8.5% | 3.0% | 4.0% | 8.5% |
| ja | wrong-key | 200 | 0.5% | 1.0% | 0.5% | 0.5% | 1.0% |
| ja | 合計 | 600 | 1.5% | 3.2% | 2.0% | 1.5% | 3.2% |
| 全部 | 合計 | 919 | 16.4% | 17.5% | 16.8% | 16.4% | 17.5% |

前身 baseline（`ref/0c-ime` `8f443af`，同一份題庫；日文用
`packs/ja-lexicon-public-core.pack`）：

| 語言 | 擾動 | n | raw top1 | raw top5 | raw 文字 | fallback top1 | fallback top5 |
|---|---|---:|---:|---:|---:|---:|---:|
| zh | doubled-key | 92 | 100.0% | 100.0% | 100.0% | 100.0% | 100.0% |
| zh | missing-key | 92 | 0.0% | 0.0% | 0.0% | 0.0% | 0.0% |
| zh | near-sound | 43 | 81.4% | 81.4% | 0.0% | 81.4% | 81.4% |
| zh | wrong-key | 92 | 2.2% | 2.2% | 1.1% | 2.2% | 2.2% |
| zh | 合計 | 319 | 40.4% | 40.4% | 29.2% | 40.4% | 40.4% |
| ja | doubled-key | 200 | 0.0% | 0.0% | 0.5% | 0.0% | 0.0% |
| ja | missing-key | 200 | 4.5% | 10.5% | 6.5% | 4.5% | 10.5% |
| ja | wrong-key | 200 | 1.0% | 2.5% | 0.5% | 1.0% | 2.5% |
| ja | 合計 | 600 | 1.8% | 4.3% | 2.5% | 1.8% | 4.3% |

注音整體 40.4% → 44.5%，差距來自 near-sound（81.4% → 100%）與
missing-key（0% → 5.4%）。wrong-key、doubled-key 與前身相同或在其誤差內。
日文整題只差 0.3 個百分點，差異來自 Rust 解碼器與 Go 解碼器的路徑成本不同，
不是題庫或比對放寬。

前身的 zh `intended` 低於 top1，是因為沒有候選時送出的是組字字串、且 35 題
`no_candidates`、3 題 `no_commit`。Rust 這邊未知詞邊保證一定有候選，
`intended` 等於 top1；這是引擎行為差異，不是指標定義改變。

## 失敗分類

`--json` 的每題有 `miss_class`，表格最後也會列出。分類規則：

| 分類 | 定義 | 本機結果 |
|---|---|---|
| `ranking` | 某個讀音變體在詞典裡找得到預期詞面，但被擠出候選 | zh 0、ja 0 |
| `dictionary` | 讀音正確，但詞典沒有這個詞面 | zh 0、ja 541 |
| `rule` | near-sound 案例中，模糊音規則展不出能容納預期詞面的讀音 | zh 0 |
| `noise` | wrong-key／missing-key 破壞了讀音，不屬於任何近音配對，規則無法還原 | zh 174 |

zh 的 174 題 `noise` 全是 wrong-key 與 missing-key。README 把這兩類定義成
「錯鍵／漏鍵」，只有 near-sound 才由 `core/fuzzy` 的配對修復；因此這些失誤
不是規則缺漏，也沒有為了提高通過率放寬比對。ja 的 541 題是統計轉換器沒有
把擾動後的讀音轉回預期詞面，屬詞典／解碼覆蓋問題。

## 已知差異

- 注音候選沒有經過 `Session`，而是直接展開 `fuzzy::Config::variants` 後建詞格
  合併 N-best。`Session` 目前沒有模糊音與無聲調整合路徑，硬接會漏掉
  missing-key 的 5 個聲調展開。
- 每個位置最多一次替代、整句最多兩次換音；前身是「每個詞兩次」。對本題庫的
  單詞案例等價，長句可能比前身嚴格。
- fallback 角色在沒有模型時就是 raw 的候選順序，因此兩個角色數字相同；
  前身 `score.Ranker` 在無模型時也是同一條統計路徑。
