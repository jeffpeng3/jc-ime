# ja-live-neural：逐字変換接上 Jinen

逐字変換（`ja_live`）原本完全走統計路徑，開啟後一次神經模型都不會呼叫。
這輪把逐字路徑接上同一套門檻與五條驗證：統計結果先算好，門檻命中才問
模型，輸出通過驗證才替換，逾時或模型缺席一律回到統計結果。`--neural off`
或模型沒載入時，逐字路徑與改動前逐鍵相同。

## 接線位置

| 位置 | 內容 |
|---|---|
| `crates/jc-core/src/session/neural.rs` | `maybe_convert` 是唯一一份門檻與驗證；新增 `LiveGate` 與 `live_gate()`，把逐字路徑的請求轉成 `NeuralInput` |
| `crates/jc-core/src/ja/live.rs` | `Config::neural`（`Option<Rc<dyn NeuralGate>>`）；`Controller::convert` 先取統計與訊號，再呼叫採用器 |
| `crates/jc-core/src/session/mod.rs` | `Session::new` 依 `ja_neural` 建立採用器，只有開啟且模型已載入才有值 |
| `crates/jc-cli/src/{cli,eval_cmd,converter}.rs` | `eval ja --live`，可與 `--neural auto／quality／off` 併用 |

逐字路徑的流程：

1. `Controller::convert` 取得本次要轉換的尾端讀音。
2. 神經關閉時走原本的 `Converter::convert`（單一路徑）；開啟時走
   `Converter::statistical`，用同一次 `results_with_gap(..., 2)` 取得
   top-1、成本差與平均候選數，不另外解一次。
3. 門檻（`gap < 472.5` 且 `mean_candidates > 5.98`）、半截讀音判定與五條
   驗證都在 `neural::maybe_convert`。`live.rs` 沒有第二份門檻或驗證。
4. 通過驗證才把整段尾端換成一個文節（神經輸出沒有詞邊界，與一般轉換
   相同）；任何失敗保留統計結果。
5. 通過後的下一次轉換不做文節邊界變動比較。神經結果只有一個文節，
   邊界集合是空的，照統計切法比較會被誤判成大幅變動而保留舊畫面，
   實測會讓 top-1 掉 6 到 7 點（見下方「邊界保留」）。

## 半截讀音

「統計結果等於讀音就不呼叫模型」原本寫在 `session/core.rs`，這輪搬進
`neural::maybe_convert`，一般轉換與逐字路徑共用同一個判斷。逐字路徑按鍵
轉換時「き」「きょ」仍會產生かな fallback，這條規則把這些呼叫全部擋掉，
只有「きょう」這種已成立詞的完整讀音才會進模型。

測試 `test_half_reading_skips_engine` 與 `test_live_gate_uses_shared_thresholds`
直接斷言兩種情境下模型呼叫次數是 0。

## 關閉時的行為

`--neural off` 讓 `NeuralConfig::enabled = false`，`live_gate()` 回傳
`None`，控制器連 `statistical()` 都不呼叫，逐鍵走原本的 `convert()`。
`--live --neural off` 的輸出與改動前的逐字変換逐題相同（48.8%）。

`eval ja`（不帶 `--live`）的一般路徑與 base commit `09a0df2` 逐項相同：

| 執行 | base top-1 | 本輪 top-1 |
|---|---:|---:|
| `--neural off` | 86.0% | 86.0% |
| `--neural auto`（預設） | 92.2% | 92.2% |
| `--neural quality` | 93.6% | 93.6% |
| `hard-cases --neural auto` | 77.1% | 77.1% |
| `ajimee-200 --neural auto` | 69.0% | 69.0% |

`modern-500` 的 93.6 是 `--neural quality` 的數字；預設（auto，472.5 加
平均候選數）是 92.2%，與 `docs/port/jinen-default.md` 記載一致。

## 量測

環境：GB10，`cargo build --release`，Jinen v2 small Q5_K_M，8 threads，
tokenizer 沿用 `jinen-v2-xsmall/tokenizer.json`。

`modern-500`，500 題、7,240 次按鍵轉換：

| 執行 | top-1 | top5 | b_exact | b_f1 | 每次轉換呼叫率 | 模型呼叫 |
|---|---:|---:|---:|---:|---:|---:|
| 一般路徑 auto | 92.2% | 98.4% | 84.8% | 0.9088 | 20.6% | 103 |
| 一般路徑 off | 86.0% | 98.0% | 88.2% | 0.9017 | — | 0 |
| `--live` off | 48.8% | 97.8% | 36.6% | 0.7818 | — | 0 |
| `--live` auto | **54.4%** | 99.0% | 50.4% | 0.8180 | 38.5% | 2,780 |
| `--live` quality | **54.8%** | 99.2% | 56.4% | 0.8304 | 45.0% | 3,260 |

`--live` auto 三次重跑是 54.0%、54.2%、54.4%；`--live` off 兩次都是 48.8%。

延遲：

| 執行 | 每題累積 p50 | 每題累積 p95 | 每鍵 p50 | 每鍵 p95 | 模型逾時 |
|---|---:|---:|---:|---:|---:|
| 一般路徑 auto | 7.1 ms | 55.1 ms | — | — | 0／103 |
| `--live` off | 5.9 ms | 13.0 ms | 0.14 ms | 0.61 ms | — |
| `--live` auto | 127 ms | 313 ms | 1.2 ms | 31 ms | 59／2,780 |
| `--live` quality | 145 ms | 321 ms | 1.9 ms | 30 ms | 8／3,260 |

「每題累積」是評測表的 `p50_ms`／`p95_ms`：一題會跑完所有按鍵，數字是
整句的總和。「每鍵」是 `live：按鍵轉換 …` 那行，逐次 `input()` 的牆鐘
時間，才是使用者按一個鍵等多久。

### 呼叫次數與延遲代價

- 每題平均呼叫 5.6 次（2,780／500），一般路徑是 0.21 次（103／500），
  約 **27 倍**。逐字路徑每打一個かな或停頓就轉換一次，同一段讀音會
  重複送進模型。
- 每鍵 p50 是 1.2 ms、p95 是 31 ms；沒有撞到 180 ms 上限的鍵幾乎
  感覺不到模型。
- 2.1% 的呼叫（59／2,780）會吃滿 180 ms 預算後回統計結果。換算下來
  大約每 10 題有 1 題會被 180 ms 的等待卡住一次。
- 每題累積 p95 是 313 ms，超過 200 ms。這個數字是 14 次按鍵轉換的
  總和，不是單次互動延遲，但它決定「一題要花多少 CPU 才完成」。
- 評測表的 `timeout` 欄（58.8%）是整題超過 `rules.json` 的
  `timeout_ms`（100 ms）的比例，與神經逾時不是同一個指標。

### 邊界保留

神經輸出的文節沒有邊界，接受後下一次轉換若照統計切法比對邊界，會把
每個統計邊界都算成變動，觸發 `max_boundary_changes` 的保留機制，畫面
停在舊結果。修正前 `--live` auto 是 47.2%（比 off 的 48.8% 還差），
修正後是 54.4%。`test_neural_accept_does_not_trigger_boundary_hold`
固定這個行為。

## 是否建議預設開啟

**不建議在逐字路徑預設開啟神經轉換。**

- 逐鍵 p95 是 31 ms，單次互動沒有超過 200 ms 的門檻，品質也比
  `--live` off 多 5 到 6 點（48.8% → 54.0–54.4%）。
- 但每題累積 p95 是 313 ms，已經超過 200 ms；呼叫次數是一般路徑的
  27 倍，2.1% 的呼叫會吃滿 180 ms 預算。
- 逐字変換本身預設關閉，維持現狀即可：`ja_live.enabled` 預設 `false`，
  使用者要逐字変換時再自行決定要不要開神經轉換。開啟時品質會變好，
  代價是每題多約 130 ms 的轉換時間與偶發的 180 ms 等待。

若之後模型變快或支援快取（同一段讀音不重送），這個結論可以重評。

## 命令列

```sh
# 一般路徑，數字不變
cargo run -p jc-cli --release -- eval ja --corpus modern-500

# 逐字変換，不接神經（對照組）
cargo run -p jc-cli --release -- eval ja --corpus modern-500 --live --neural off

# 逐字変換加神經
cargo run -p jc-cli --release -- eval ja --corpus modern-500 --live
```

`--live` 只給 `eval ja`，其他題庫會回 `eval fuzzy 不接受 --live`。

## 測試

| 測試 | 內容 |
|---|---|
| `test_neural_gate_replaces_whole_tail` | 採用時整段換成一個文節，讀音與門檻訊號正確 |
| `test_neural_gate_receives_locked_prefix_as_left` | 已鎖定詞面接進左文脈 |
| `test_neural_gate_rejection_keeps_statistical_result` | 回 `None` 時保留統計結果與切法 |
| `test_neural_gate_absent_keeps_statistical_result` | 沒有採用器時不多解 top-2 |
| `test_neural_accept_does_not_trigger_boundary_hold` | 神經結果不得觸發邊界保留 |
| `test_half_reading_skips_engine` | 半截讀音不呼叫模型（共用規則） |
| `test_live_gate_uses_shared_thresholds` | 逐字採用器走同一份門檻與半截讀音判斷 |
| `test_ja_live_neural_adopts_complete_reading_only` | Session 逐字路徑：き、きょ 不呼叫，きょう 只呼叫一次 |
| `test_ja_live_neural_off_keeps_statistical_result` | `enabled = false` 時逐字路徑完全不呼叫 |
| `test_ja_live_neural_respects_mean_candidates` | 逐字路徑套用平均候選數門檻 |
| `parses_ja_live_switch` | `--live` 只有 `eval ja` 接受，可與 `--neural off` 併用 |
| `live_path_converts_per_keystroke_and_is_deterministic` | 逐鍵轉換、逐鍵記延遲、結果可重現 |

測試需要真的模型時用 `NeuralEngine` 替身；逐字路徑只需要 `live::NeuralGate`
替身，不用 GGUF。

## Go 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/live/live.go`（逐字変換狀態機） | `crates/jc-core/src/ja/live.rs` | ported（既有）；神經採用鉤子是 Rust 新增，Go 沒有對應 |
| `core/ja/live/live_test.go`（debounce、鎖定、邊界保留） | `crates/jc-core/src/ja/live.rs`（`mod tests`） | rewritten：沿用既有行為測試，新增採用器測試 |
| `core/session`（Zenz 採用流程、fallback 順序） | `crates/jc-core/src/session/neural.rs`、`session/core.rs` | rewritten（既有）；半截讀音判定從 `core.rs` 搬進共用層 |
| `cmd/0c-eval-ja`（日文評測） | `crates/jc-cli/src/eval_cmd.rs`、`converter.rs` | rewritten：新增 `--live` 逐字模擬與逐鍵延遲統計 |
| — | `crates/jc-core/src/ja/live.rs` 的 `NeuralGate` | n/a：Go 逐字変換沒有神經模型接線 |

## 未完成與注意

- 右文脈仍沒有送進模型；逐字路徑的 `GateRequest.right` 固定空字串，
  與一般路徑一致。
- 逐字路徑的統計訊號是「尾端讀音的 top-2」，不是「已鎖定片段的
  top-2」；神經輸出會整體替換尾端，包含還會變動的最後一個文節。
- 採用器在 `Session::new` 建立時綁定 `ja_neural`；建立後才改
  `SessionConfig::ja_neural` 不會換掉鉤子，需要重建 Session。
- 神經輸出整段一個文節，會讓 `b_exact` 偏向「整句無邊界」；表格的
  `b_exact` 不能拿來跟一般路徑直接比。
- 量測是同一台機器、同一段時間的牆鐘時間；模型逾時次數對負載敏感，
  重跑會有 0 到 200 次的差距，top-1 差異在 0.4 點以內。
