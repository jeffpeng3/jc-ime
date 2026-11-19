# candle-quant：量化權重

同一個 checkpoint、同一個 forward，只把權重換成 GGUF 量化格式，量「記憶體
vs 精度 vs 延遲」。報告見 `docs/reports/candle-quant-2026-11-20.md`。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/neural/native/rerank_ort.go`（權重載入與推論） | `crates/jc-model/src/candle/quant.rs` | rewritten：同一個 encoder 改吃 GGUF 的量化權重 |
| `scripts/convert-reranker-safetensors.py`（checkpoint → safetensors） | `crates/jc-model/src/candle/quant.rs`（`export_gguf`） | rewritten：safetensors → GGUF，量化用 candle 的 `QTensor::quantize` |
| `scripts/convert-punctuation-safetensors.py` | `crates/jc-model/src/candle/quant.rs`（`export_punct_gguf`） | rewritten：同上，分類頭換成 `classifier` |
| — | `crates/jc-model/src/bin/candle-quant.rs` | 新增：`export`／`bench`／`punct`／`compare` |
| — | `scripts/candle-quant-run.sh` | 新增：一次跑完匯出、量測與比對 |
| `crates/jc-model/src/candle/mod.rs`（`candle-spike-ja`） | `crates/jc-model/src/candle/mod.rs` | rewritten：`Encoder` 改成 `Encoder<L>`，fp32 與量化共用 forward、位置表與池化 |
| `crates/jc-model/src/candle/punct.rs`（`candle-punct`） | `crates/jc-model/src/candle/punct.rs` | ported：改接泛型 `Encoder<Linear>`，其餘不動 |
| `crates/jc-model/src/rerank.rs` | `crates/jc-model/src/rerank.rs` | rewritten：`.gguf` 走 `QuantReranker`，其餘維持 fp32 路徑 |
| `crates/jc-model/src/punctuation_model.rs` | `crates/jc-model/src/punctuation_model.rs` | rewritten：`.gguf` 走 `QuantPunctuation`，權重環境變數改成 `JC_PUNCT_WEIGHTS` |
| llama.cpp 的 ggml 量化與 GGUF 格式 | `candle-core::quantized` | n/a：直接用 candle 的 `QTensor`／`gguf_file`，沒有自己的實作 |

## 介面

| 項目 | 值 |
|---|---|
| 量化檔 | `models/quant/candle/<model>/<dtype>/model.gguf`（原始 safetensors 不動） |
| 等級 | `q8_0`（8.5 bits/weight）、`q5k`（5.5）、`q4k`（4.5） |
| reranker 權重覆寫 | `JC_RERANK_WEIGHTS=<gguf>` |
| 標點權重覆寫 | `JC_PUNCT_WEIGHTS=<gguf>` |
| 後端 | `--backend candle`；`.gguf` 自動走量化路徑 |

## 測試

| 測試 | 內容 |
|---|---|
| `candle::quant::tests::quant_names_roundtrip` | 等級名稱、位元數 |
| `candle::quant::tests::only_matmul_weights_are_quantized` | 只有 linear 與詞嵌入被量化 |
| `candle::quant::tests::quantized_levels_stay_close_to_fp32` | 三個等級的 reranker logits 與 fp32 差距在容許範圍 |
| `candle::quant::tests::punct_quantized_levels_load` | 三個等級的標點模型可載入，argmax 與 fp32 一致 |
