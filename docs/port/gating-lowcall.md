# gating-lowcall：把日文神經模型門檻改成低呼叫版

預設門檻改成低呼叫版：

```text
top2 - top1 < 472.5  且  mean_candidates > 5.98   → 呼叫 Jinen
```

品質優先版保留為 `--neural quality`，只看 `gap < 460`。`--neural off`
不載入模型、不算門檻，輸出與純統計逐題相同。

## 兩版對照

量測環境：GB10，`cargo build --release`，Jinen v2 small Q5_K_M，
8 threads，`ajimee-200` 同一段時間。品質門檻用同一台機器重跑。

| 設定 | 題庫 | top1 | 下限 | 呼叫率 | p50 | p95 | neural 逾時 |
|---|---|---:|---:|---:|---:|---:|---:|
| `auto`（預設） | `modern-500` | 92.2% | 92.0 | 20.6% | 6.9 ms | 60.8 ms | 0 |
| `auto`（預設） | `hard-cases` | 77.1% | 77.0 | 37.1% | 8.7 ms | 39.4 ms | 0 |
| `auto`（預設） | `ajimee-200` | 69.5% | 69.0 | 56.5% | 43.3 ms | 178.8 ms | 1 |
| `quality` | `modern-500` | 93.6% | 92.0 | 30.2% | 7.8 ms | 57.6 ms | 0 |
| `quality` | `hard-cases` | 80.0% | 77.0 | 54.3% | 17.2 ms | 45.8 ms | 0 |
| `quality` | `ajimee-200` | 71.0% | 69.0 | 74.0% | 50.1 ms | 193.8 ms | 1 |
| `off` | `modern-500` | 86.0% | 統計 | 0% | 3.3 ms | 6.4 ms | 0 |
| `off` | `hard-cases` | 54.3% | 統計 | 0% | 0.9 ms | 6.0 ms | 0 |
| `off` | `ajimee-200` | 46.0% | 統計 | 0% | 6.5 ms | 55.6 ms | 0 |

三組題庫合計呼叫率 31.2%（229／735），與 `jinen-gating-v2` 的離線掃描
一致。合併 735 題的混合延遲是 p50 8.2 ms、p95 112.7 ms。

`quality` 的 `ajimee-200` 量到 71.0%，離線掃描是 71.5%；差距來自 1 次
180 ms 逾時，該題回到統計結果。逾時沒有放寬，題庫也沒有調整。

## 延遲說明

`ajimee-200` 單獨的 p95 是 178.8 ms，高於任務書的 130 ms。原因是
small 模型在 `ajimee-200` 自己的 p95 約 170 ms，而這組題庫的低呼叫版
仍有 56.5% 的題目觸發；混合延遲的 p95 由三組一起算，才會落在
112.7 ms。長句的統計轉換本身也可能到 160 ms，逾時題目會再加上等待時間。

沒有為了壓 p95 去改題庫、放寬 timeout 或加答案側訊號。若要求單獨
`ajimee-200` 的 p95 ≤ 130 ms，需要降低這組的呼叫率或換更快的模型，
兩者都會改變已驗證的品質門檻。

## `mean_candidates` 定義

統計 top-1 路徑上每個詞段為一個詞群。對每個詞段查同讀音的詞典詞條，
詞面去重後計數；查不到詞條的未知詞段記 1。平均值就是
`mean_candidates`，沒有詞段時為 1.0。

`ambiguous_ratio`、`max_candidates` 也用同一組詞群計數。實作在
`crates/jc-core/src/ja/decode.rs`：

- `segment_candidate_counts(result, source)`：回傳每個詞段的候選詞面數。
- `mean_candidates(result, source)`：回傳平均值。
- `JaDecoder::results_with_gap(reading, ctx, n)`：一次搜尋同時回傳前 `n`
  條結果與 gating 的 `gap`。

`jinen-gate` 與 `Session`／`JaConverter` 都呼叫這一份，沒有各自重寫。
`gap` 只看最前面兩條原始路徑，詞面相同視為沒有備案；這個語意與
`decoder.results(..., 2)` 相同，但候選清單與門檻共用同一次搜尋，不再
為了 top-2 多解一次。

`mean_candidates` 在統計轉換完成時就算好，放進 `NeuralInput`；
`maybe_convert` 只讀欄位，不會在等候模型的路徑上解碼或查詞典。

## 設定與命令列

`NeuralConfig` 新增 `min_mean_candidates: f32`，預設 `5.98`；
`gap_threshold` 改成 `f32`，預設 `472.5`。`gap` 本身是整數原始成本差，
和 `472.5` 比較等於 `gap <= 472` 才觸發。

| 模式 | `gap_threshold` | `min_mean_candidates` | `enabled` |
|---|---:|---:|---|
| `auto` | 472.5 | 5.98 | `true` |
| `quality` | 460.0 | 0.0（不檢查） | `true` |
| `off` | 不檢查 | 不檢查 | `false` |

```sh
cargo run -p jc-cli --release -- eval ja --corpus modern-500
cargo run -p jc-cli --release -- eval ja --corpus modern-500 --neural quality
cargo run -p jc-cli --release -- eval ja --corpus modern-500 --neural off
```

## 為何預設選低呼叫版

1. 三組題庫仍全過下限：92.2／77.1／69.5。
2. 呼叫率從 43.3% 降到 31.2%，少呼叫 89 題。
3. 合併 p95 從 123.9 ms 降到 112.7 ms，避開 `ajimee-200` 大量長句
   撞上 180 ms 預算、等了卻拿不到結果的情況。
4. 品質優先版仍在，需要 93.6／80.0／71.5 時用 `--neural quality`。

## 驗證

```sh
cargo check -p jc-core --all-targets   # 通過
cargo check -p jc-cli --all-targets    # 通過
cargo run -p jc-cli --release -- eval ja --corpus modern-500
cargo run -p jc-cli --release -- eval ja --corpus hard-cases
cargo run -p jc-cli --release -- eval ja --corpus ajimee-200
cargo run -p jc-cli --release -- eval ja --corpus ajimee-200 --neural off
```

`--neural off` 三組數字與統計基線相同：86.0／54.3／46.0。關閉路徑與
統計共用 `results_with_gap` 的前 `n` 條結果，單元測試
`test_results_with_gap_candidates_match_results` 直接比對兩者相等。

## 測試

| 測試 | 內容 |
|---|---|
| `test_default_thresholds_are_low_call` | 預設 472.5／5.98，quality 是 460／0 |
| `test_gap_threshold_skips_engine`／`test_gap_threshold_accepts_engine_output` | 新 gap 邊界 472／473 |
| `test_mean_candidates_threshold_skips_engine` | `mean_candidates` 5.98 與 5.99 的邊界 |
| `test_quality_mode_ignores_mean_candidates` | quality 不看平均候選數 |
| `test_ja_neural_skips_when_mean_candidates_below_min` | Session 在平均候選數不足時不呼叫 |
| `test_mean_candidates_counts_deduped_surfaces` | 同讀音詞面去重後計數 |
| `test_results_with_gap_matches_top_two_paths` | gap 與沒有備案的情境 |
| `test_results_with_gap_candidates_match_results` | 關閉神經時候選與原路徑相同 |

## Go 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `src/session/session.cc`（Zenz 採用流程、180 ms 預算） | `crates/jc-core/src/session/neural.rs`、`crates/jc-core/src/session/core.rs` | rewritten（既有） |
| `src/session/zenz_output_validator.cc` | `crates/jc-core/src/session/neural.rs` | rewritten（既有） |
| `scripts/spike/ja-zenz/run_spike.py`（只有推論 spike） | `crates/jc-cli/src/bin/jinen-gate.rs` | n/a：Go 版沒有 gating 實驗可搬 |

## 未完成與注意

- `ajimee-200` 單獨 p95 是 178.8 ms，沒有達到任務書的 130 ms；合併三組
  是 112.7 ms。原因與可選做法寫在「延遲說明」。
- `quality` 的 `ajimee-200` 是 71.0%，離線掃描 71.5%；差距是 1 次逾時。
- `gap` 是整數，`472.5` 與 `472` 的觸發集合相同；保留小數是為了沿用
  掃描結果的原始門檻。
- 量測是共用機器上的牆鐘時間，逾時次數會隨其他工作負載變動。
