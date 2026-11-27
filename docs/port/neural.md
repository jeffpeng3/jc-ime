# 日文神經轉換（Jinen）

日文轉換永遠先算統計 Viterbi，這是 baseline 也是 fallback。只有成本差與平均候選
數過門檻時才把讀音送進 Jinen v2 small；輸出通過驗證才替換。模型缺席、逾時或輸出
不合格都保留統計結果，不報錯、不阻塞輸入。

一般整句轉換與逐字変換（`ja_conversion = live`）共用同一份門檻與驗證，實作在
`crates/jc-core/src/session/neural.rs` 的 `maybe_convert`。

## 設定

`SessionConfig.ja_neural` 的預設值：

| 欄位 | 預設 | 說明 |
|---|---:|---|
| `enabled` | `true` | `--neural off` 或 `JC_JINEN=off` 設成 `false` |
| `gap_threshold` | 472.5 | `top2 - top1` 的成本差；`quality` 模式改成 460 |
| `min_mean_candidates` | 5.98 | top-1 路徑平均候選數下限；`quality` 模式為 0 |
| `model` | `Small` | `jinen-v2-small-Q4_K_M.gguf` |
| `timeout_ms` | 180 | 超過就沿用統計結果 |
| `max_context_chars.left` | 24 | 左文脈只取尾端 |
| `max_context_chars.right` | 6 | 欄位保留；prompt 目前沒有右文脈 |
| `correction_gap_ratio` | 0.5 | 停頓後補正使用更嚴格的門檻 |

`jc-engine` 預設載入模型並注入 `NeuralRuntime`。模型路徑可用 `JC_JINEN_GGUF`、
`JC_JINEN_TOKENIZER` 覆寫；資料 manifest 的 `ja_neural`／`ja_tokenizer` 優先。
`NeuralRuntime` 是一個 worker 執行緒加 channel；送出後最多等 `timeout_ms`，worker
會丟掉佇列裡的舊請求，只算最新一筆。模型只活在 worker，`Session` 可自由複製。

## 流程

1. 統計 Viterbi 先產生候選與文節。
2. 門檻：`gap < gap_threshold` 且 `mean_candidates > min_mean_candidates`。同詞面
   去重後只剩一條視為沒有備案，一定觸發。
3. 讀音與左文脈（尾端 24 字元）送進 Jinen；半截讀音不送。
4. 輸出通過五條驗證才替換統計第一名；候選、組字字串與提交結果一起換。
5. 神經輸出沒有詞邊界，替換時整段當一個文節。
6. 任何失敗都保留統計結果。

## 五條驗證

`neural::validate()` 的順序：

| 驗證 | 規則 | 拒絕原因 |
|---|---|---|
| 空輸出 | 空字串直接拒絕 | `empty` |
| 等於統計結果 | 直接接受，不替換 | — |
| 長度 | 字元數 ≤ `reading_len * 3 + 8` | `too_long` |
| context echo | 左文脈尾端與輸出開頭重疊 ≥ 4 字元且超過下限，或輸出夠長 | `reading_echo` |
| 絵文字 | cluster 序列必須與統計結果完全相同 | `emoji_changed` |
| 首尾截斷 | 比統計結果短且是其前綴或後綴 | `truncated` |

每次拒絕寫 `log::debug!` 並記進 `NeuralStatsSnapshot`。CLI 評測結束印出
`neural：calls … accepted … skipped … timeout … rejected […]`。

## 逐字変換

`ja_conversion = live` 時，打字期間顯示假名，停頓後做一次完整轉換。逐字路徑的
統計訊號取尾端讀音的 top-2，門檻與一般路徑相同；神經輸出替換整個尾端。為避免
神經結果被誤判成邊界大幅變動，接受後的下一次轉換不做文節邊界比較。

`--live --neural off` 回到純統計逐字変換。`ja_conversion = sentence` 時空白或
Enter 才轉換。

## 執行緒與量測

Jinen 的 candle 路徑對執行緒數敏感。`jc-engine` 載入模型前把
`CANDLE_NUM_THREADS`／`RAYON_NUM_THREADS` 設成 8（宿主已設定時尊重宿主）。20
threads 會讓 barrier pool 過度競爭，實測 top-1 反而下降。

GB10、release build、8 threads、small Q5_K_M 的參考數字：

| 題庫 | `--neural off` | 預設 auto | quality |
|---|---:|---:|---:|
| `modern-500` | 86.0% | 92.2% | 93.6% |
| `hard-cases` | 54.3% | 77.1–80.0% | 80.0% |
| `ajimee-200` | 46.0% | 69.0–71.0% | 71.0% |

逐字変換 `modern-500`：`--live` off 48.8%，`--live` auto 54.4%，quality 54.8%。
每題累積 p95 約 313 ms；每鍵 p50 1.2 ms、p95 31 ms。逐字路徑呼叫次數是一般路徑
的約 27 倍，少數呼叫會吃滿 180 ms。

## 注意

- 右文脈尚未送進模型；`max_context_chars.right` 目前沒有效果。
- context echo 只比左文脈，沒有「輸出真的以左文脈開頭時剝掉」的保護。
- `ja_conversion` 與 `ja_neural` 在 `Session::new` 時綁定；執行期改設定要重建
  Session，或走引擎的熱重載路徑。
- Jinen 權重是 CC-BY-SA-4.0，只隨安裝包散布；模型檔不進 git。
