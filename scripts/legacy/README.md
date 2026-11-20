# legacy：只為 ONNX Runtime 存在的腳本

2026-11-20 移除 ONNX Runtime 後，這裡的腳本不再參與建置或驗證。保留是為了
回溯 `docs/reports/quant-study-2026-11-20.md` 的數字與當時的量化流程。

| 檔案 | 用途 | 為什麼不再用 |
|---|---|---|
| `quantize_models.py` | 用 `onnxruntime.quantization` 產生 int8／int4 ONNX | 推論已改 candle；量化改走 `candle-quant export` 的 GGUF |
| `measure_onnx.py` | 量 ONNX 檔案大小、ORT 延遲、RSS 與 fp32 一致性 | 沒有 ORT 可量；candle 的對應量測在 `candle-quant bench`、`candle-zh-check` |
| `e2e_punct.py` | 日文標點端到端比較 | 標點模型已於 2026-11-20 移除，`jc-cli convert --punct` 也不存在 |
| `dump-inputs/` | 替 `measure_onnx.py` dump 固定輸入的小 crate | 只服務 `measure_onnx.py` |

現行替代路徑：

- 轉檔與量化：`crates/jc-model/src/bin/candle-quant.rs`（`export`／`bench`／
  `compare`）與 `scripts/candle-quant-run.sh`。
- 注音選字權重：`cargo run -p jc-cli --release --bin candle-zh-check -- --convert --quant q8_0`。
- GGUF 模型（Jinen）的評測與延遲：`scripts/quant/eval_jinen_quant.py`、
  `scripts/quant/latency_jinen.py`，仍在使用。

這些腳本依賴 `onnxruntime` Python 套件與舊版 `jc-cli` 參數，直接執行會失敗；
要重跑請先 checkout 2026-11-20 之前的 commit。
