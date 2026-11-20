# jinen-default：把 Jinen small 接進 Session 並設為預設

這一輪把 `jinen-gating-v2` 的結論接進日文轉換：統計結果永遠先算好，
`top1` 與 `top2` 的成本差小於 460 時才呼叫 Jinen v2 small；模型輸出要通過
五條驗證才替換，逾時、模型缺席或載入失敗都回到統計結果。`--neural off`
完全不載入模型、不算門檻，輸出與舊路徑逐題相同。

## 設定

`SessionConfig` 新增 `ja_neural: NeuralConfig`，預設值如下：

| 欄位 | 預設 | 說明 |
|---|---:|---|
| `enabled` | `true` | `--neural off` 設成 `false` |
| `gap_threshold` | 460 | `top2 - top1` 的原始成本差；沒有 `top2` 視為一定觸發 |
| `model` | `Small` | `jinen-v2-small-Q5_K_M.gguf` |
| `timeout_ms` | 180 | 超過就沿用統計結果 |
| `max_context_chars.left` | 24 | 左文脈只取尾端 |
| `max_context_chars.right` | 6 | 右文脈只取開頭；Jinen prompt 目前沒有右文脈欄位 |
| `runtime` | `None` | 由 `jc-engine` 載入模型後注入；`None` 時不呼叫模型 |

`NeuralRuntime` 是一個 worker 執行緒加一組 channel。請求送出後最多等
`timeout_ms`；worker 會先丟掉佇列裡的舊請求，只算最新的一筆，避免快速輸入
時把過期讀音逐個算完。模型只活在 worker 裡，`Session` 與 `SessionConfig`
仍然可以自由複製。

## 轉換流程

1. 統計 Viterbi 先產生候選與文節，這是 baseline，也是 fallback。
2. 只有 `enabled` 且 `runtime` 存在時才算門檻。門檻沿用 gating 實驗的定義：
   取 `decoder.results(..., 2)`，同詞面去重後只剩一條就視為沒有備案、一定
   觸發。這與候選清單的 N-best 分開算，所以不會因為候選數變多而改變門檻。
3. `top2 - top1 < 460` 才把讀音、左文脈（尾端 24 字元）送進 Jinen。
4. 輸出通過五條驗證才替換統計第一名；候選清單、組字字串與提交結果一起換。
5. 神經輸出沒有詞邊界，替換時整段當一個文節，避免片段範圍對不上。
6. 任何失敗都保留統計結果，不報錯、不阻塞輸入。

## 五條驗證

實作在 `crates/jc-core/src/session/neural.rs` 的 `validate()`，順序如下：

| 驗證 | 規則 | 拒絕原因 |
|---|---|---|
| 空輸出 | 空字串直接拒絕 | `empty` |
| 等於統計結果 | 直接接受，不替換 | — |
| 長度 | 輸出字元數 ≤ `reading_len * 3 + 8` | `too_long` |
| context echo | 左文脈尾端與輸出開頭重疊 ≥ 4 字元，且重疊 ≥ `max(1, max(reading_len, baseline_len)) * 2 + 1`，或輸出長度 ≥ 重疊＋baseline 長度 | `reading_echo` |
| 絵文字 | 輸出與統計結果的 cluster 序列必須完全相同；ZWJ、膚色、variation selector、keycap 都算 cluster 的一部分 | `emoji_changed` |
| 首尾截斷 | 比統計結果短，且是統計結果的前綴或後綴 | `truncated` |

每次拒絕都寫 `log::debug!`，並記進 `NeuralRuntime` 的統計
（`NeuralStatsSnapshot`）。CLI 在評測結束後印出
`neural：calls … accepted … skipped … timeout … rejected […]`。

## 逾時與回退

| 情況 | 行為 |
|---|---|
| 180 ms 內沒有回應 | 用統計結果；記 `timeout` |
| 模型檔不存在 | 不建立 worker；`runtime = None`，轉換照舊 |
| 模型載入失敗 | `load_ja_neural` 回錯誤，呼叫端記一次警告後照舊 |
| 推論錯誤 | 用統計結果；記 `unavailable` |
| 驗證失敗 | 用統計結果；記對應原因 |
| 輸出等於統計結果 | 視為無變化，不替換 |

## 執行緒數

Jinen 的 candle 路徑在 180 ms 預算下對執行緒數很敏感。`jc-engine` 在載入
模型前把 `CANDLE_NUM_THREADS`／`RAYON_NUM_THREADS` 設成 8（宿主已設定時
尊重宿主）。量測（ajimee-200、small、4／8／20 threads）：

| threads | top1 | neural 逾時（148 次呼叫） | p50 |
|---:|---:|---:|---:|
| 4 | 65.0% | 34 | 86 ms |
| **8** | **71.0%** | **4** | **55 ms** |
| 20 | 46.0% | 148（全部逾時） | 194 ms |

20 threads 會讓 candle 的 barrier pool 過度競爭，不能只按核心數開。

## 量測結果

環境：GB10，`cargo build --release`，8 threads，模型
`models/external/jinen-v2-small/jinen-v2-small-Q5_K_M.gguf`，tokenizer 沿用
`models/quant/jinen-v2-xsmall/tokenizer.json`。

| 題庫 | `--neural off` | 預設（neural on） | 下限 | Jinen 呼叫 | neural 逾時 |
|---|---:|---:|---:|---:|---:|
| `modern-500` | 86.0% | **92.0%** | 90% | 30% | 9／151 |
| `hard-cases` | 54.3% | **80.0%** | 75% | 54% | 0／19 |
| `ajimee-200` | 46.0% | **71.0%** | 68% | 74% | 4／148 |

混合延遲（統計一定算，觸發才等模型）：

| 題庫 | p50 | p95 |
|---|---:|---:|
| `modern-500` | 11.67 ms | 70.68 ms |
| `hard-cases` | 24.96 ms | 56.39 ms |
| `ajimee-200` | 55.31 ms | 239.90 ms |

`ajimee-200` 的 p95 是少數長句撞到 180 ms 上限；`RunOptions` 表的
`timeout` 欄是整次轉換超過 100 ms 的比例，兩者不是同一個指標。
`--neural off` 的三組數字與本輪基線完全相同，證明關閉時沒有走任何新路徑。

## 命令列

```sh
# 預設開啟；模型缺席只警告，維持統計轉換
cargo run -p jc-cli --release -- eval ja --corpus modern-500

# 完全回到舊路徑
cargo run -p jc-cli --release -- eval ja --corpus modern-500 --neural off
```

引擎行程（`jc-engine`）預設也在 Session 接上 Jinen；`JC_JINEN=off` 關閉。
模型路徑可用 `JC_JINEN_GGUF`、`JC_JINEN_TOKENIZER` 覆寫。

## 測試

| 測試 | 內容 |
|---|---|
| `test_validate_rejects_too_long` | 長度上限 |
| `test_validate_accepts_unchanged_output` | 等於統計結果 |
| `test_validate_rejects_context_echo`／`test_validate_allows_short_overlap` | context echo 與 4 字元下限 |
| `test_validate_rejects_emoji_changes` | 絵文字序列與膚色 |
| `test_validate_rejects_head_or_tail_truncation` | 首尾截斷 |
| `test_gap_threshold_skips_engine`／`test_gap_threshold_accepts_engine_output` | 門檻與替身引擎 |
| `test_timeout_falls_back`／`test_engine_error_falls_back` | 逾時與錯誤回退 |
| `test_ja_neural_unavailable_keeps_statistical_result` | 模型缺席時輸出與純統計完全相同 |
| `test_ja_neural_replaces_when_gap_below_threshold` | Session 實際替換 |
| `test_ja_neural_skips_when_gap_above_threshold` | Session 不呼叫模型 |
| `test_ja_neural_timeout_keeps_statistical_result` | Session 逾時回退 |

測試用 `NeuralEngine` 替身，不需要模型檔；只有整合評測需要真的 GGUF。

## Go 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `src/session/session.cc`（Zenz 採用流程、fallback 順序） | `crates/jc-core/src/session/neural.rs`、`crates/jc-core/src/session/core.rs` | rewritten：只搬先統計、後神經、失敗回統計的順序 |
| `src/session/zenz_output_validator.cc`（長度、echo、絵文字、截斷） | `crates/jc-core/src/session/neural.rs` | rewritten：四條採用條件與 `empty` |
| `src/session/zenz_context_selector.cc`（左右文脈取字） | `crates/jc-core/src/session/neural.rs` | rewritten：只做長度截斷，沒有段落／隱私規則 |
| `src/session/zenz_prompt_builder.cc`（prompt token 順序） | `crates/jc-model/src/candle/jinen.rs` | ported（jinen-integrate 既有） |
| `src/session/zenz_feedback_store.cc`（自動封鎖、學習） | — | n/a：第一版不做 feedback |
| `src/zenz/zenz_wire_protocol.h`（pipe／unix socket） | — | n/a：直接 in-process worker，不搬傳輸層 |
| `src/session/session.cc`（180 ms 預算） | `crates/jc-core/src/session/neural.rs` | rewritten：`recv_timeout` 加舊請求丟棄 |
| `scripts/eval_jinen_ajimee.py`（離線評測） | `crates/jc-cli/src/bin/jinen-eval.rs` | ported（jinen-integrate 既有） |

## 未完成與注意

- 只接一般日文轉換；`ja_live`（逐字変換）仍走自己的路徑，預設也是關閉。
  開啟逐字変換時不會套用 Jinen。
- 右文脈目前沒有送進模型：`jinen.rs` 的 prompt 只有左文脈；欄位先留著，
  之後模型卡支援時再接。
- context echo 只比左文脈，沒有 Mozkey 的「輸出真的以左文脈開頭時剝掉」
  與保護片段邏輯。
- `gap` 沿用 gating 實驗的 `results(..., 2)` 語意：同詞面去重後只剩一條
  就觸發，因此呼叫率高於「前兩個不同詞面的成本差」版本。這是 460 這個
  門檻被選出來的前提，兩者不能混用。
- 延遲是同一台機器、同一段時間的牆鐘時間；`ajimee-200` 的 p95 受少數
  長句與 180 ms 上限影響。
- Jinen 權重是 CC-BY-SA-4.0，只做本機評測；模型檔不進 git。
