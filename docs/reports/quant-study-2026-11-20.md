# 精度與效能取捨：fp32／int8／int4 與外部 GGUF（2026-11-20）

Worktree：`/mnt/work/jc/wt/quant-study`（branch `quant-study`）
量化檔：`/mnt/work/jc/jc-ime-data/models/quant/<model>/<precision>/`

## 摘要

1. **日文 reranker 用 int8**。int8 的排序幾乎與 fp32 相同（top-1 一致率 100%、平均 Spearman 0.991），p50 32 ms、p95 37 ms，是唯一穩定落在 100 ms 預算內的精度。fp32 在負載下 p50 107 ms 起跳，會頻繁觸發 watchdog；int4 的 p95 在 82–315 ms 之間，端到端 55% 逾時，不可用。
2. **日文標點用 int8**。int8 的 token argmax 一致率 99.44%，端到端 93% 句子完全相同、其餘只差逗號，字元編輯相似度 99.58%。int4 也幾乎無損，但延遲是 int8 的兩倍，只省 16 MB，不划算。
3. **注音選字維持 fp32**。這是對量化最敏感的模型：int8 的 logits 最大誤差 10.3、平均 Spearman 0.859、候選 top-1 一致率 92.3%，dev model-only 從 46/46 掉到 45/46；int4 的 logits 幾乎失去排序意義（全體 Spearman -0.096、候選 top-1 61.5%），不可用。若記憶體壓力大，int8 可把 131 MB 降到 33 MB，但必須重新校準 margin 並接受品質風險。
4. **reranker 目前讓端到端變差**：modern-500 top-1 在 `--rerank off` 是 86.0%，`--rerank on` 的 fp32 是 83.4%、int8 82.0%。這不是量化造成，而是重排本身在現有 gating 下選錯。精度建議只在「確定要開 rerank」時成立；產品預設應維持關閉，先修重排。
5. **外部 Jinen v2 xsmall 的 Q5_K_M 最划算**：llama.cpp 下 28.3 MB、AJIMEE 200 題無文脈 NFKC 59.0%／accept+NFKC 73.5%，與 f16 的 58.0%／73.5% 同級，輸出 92.5% 完全相同。Q8_0 有 38.7 MB 但沒有更好；Q4_K_M 26.3 MB，NFKC 57.0%，掉 2 個百分點。兩者延遲接近，Q5 是預設。
6. **candle 欄位待補**。`candle-spike-ja` 已合併到 main（`688f1f6`），但本 worktree 依共通規則不 merge，ONT 以外的 candle f16／Q8_0／Q5K／Q4K／Q4_0 尚未量測。

## 環境與方法

| 項目 | 設定 |
|---|---|
| 機器 | GB10，20 核（10×Cortex-X925 + 10×Cortex-A725），128 GB UMA |
| ONNX Runtime | 1.32.0（CPUExecutionProvider） |
| onnx | 1.23.2 |
| llama.cpp | llama-cpp-python 0.3.37 |
| 執行緒 | 固定 `intra_op_num_threads=8`；llama.cpp `n_threads=8`、`n_batch=512`、`n_ctx=1024` |
| 延遲 | 每個精度 3 輪、每輪 100 次，取干擾最少一輪的 p50／p95；暖機 10 次 |
| 負載 | 量測期間 loadavg 19–31（其他 worktree 同時跑編譯與 candle benchmark） |
| 數值比較 | 與 fp32 的原始輸出逐值比較；Spearman 用 SciPy |
| 數值輸入 | reranker：modern-500 前 40 題、每題 8 候選、seq 124；標點：10 組串接長文、19 個 236–256 token 視窗；注音：golden 13 題的統計 N-best、最長 L=165、16 候選 |

量化方式：

- 日文 int8 使用上游官方 `model.int8.onnx`（動態量化），沒有重新產生。
- 注音 int8 用 `onnxruntime.quantization.quantize_dynamic`（QInt8、per-tensor）。
- int4 用 ORT 的 `MatMulNBitsQuantizer`。ORT 1.32 已把舊名 `MatMul4BitsQuantizer` 改名，參數為 bits=4、block_size=128、非對稱、`MatMul`＋`Gather`。
- 原始檔完全沒有覆蓋；fp32 的日文權重從上游固定 revision 下載後校驗 SHA-256。

端到端切換精度沒有改程式。`jc-cli` 沒有 `--backend` 參數，改用既有的 `JC_MODELS`／`JC_JA_MODELS` 指向 `models/quant/overlay/<precision>/`。這點與任務書假設不同，已在限制一節記錄。

## 大表：模型 × runtime × 精度

延遲是單次推論，reranker 固定 8 候選 batch。RSS 是載入後相對載入前的增量。

| 模型 | runtime | 精度 | 檔案 MB | RSS Δ MB | p50 ms | p95 ms | 最大 abs err | Spearman | 排序一致率 | 端到端 |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---|
| 日文 reranker | ONNX | fp32 | 135.6 | 205 | 107.4 | 140.8 | 0 | 1.000 | 100.0% | top-1 83.4%，逾時 11.2% |
| 日文 reranker | ONNX | int8 | 34.3 | 79 | 31.9 | 36.6 | 0.240 | 0.991 | 100.0% | top-1 82.0%，逾時 1.2% |
| 日文 reranker | ONNX | int4 | 18.5 | 58 | 70.8 | 82.1 | 0.965 | 0.961 | 97.5% | top-1 83.8%，逾時 55.4% |
| 日文標點 | ONNX | fp32 | 135.6 | 197 | 38.5 | 40.1 | 0 | 1.000 | 100.0% | 基準 |
| 日文標點 | ONNX | int8 | 34.2 | 78 | 12.1 | 12.5 | 0.319 | 0.9998 | 99.44% | 93% 句子相同，編輯相似度 99.58% |
| 日文標點 | ONNX | int4 | 18.4 | 58 | 23.1 | 23.6 | 1.15 | 0.9971 | 97.97% | 90% 句子相同，編輯相似度 99.43% |
| 注音選字 | ONNX | fp32 | 130.7 | 273 | 35.9 | 36.0 | 0 | 1.000 | 100.0% | golden 45/45，dev 46/46 |
| 注音選字 | ONNX | int8 | 33.0 | 84 | 25.3 | 25.7 | 10.3 | 0.859 | 92.3% | golden 45/45，dev 45/46 |
| 注音選字 | ONNX | int4 | 75.8 | 112 | 27.9 | 28.0 | 88.9 | 0.539 | 61.5% | golden 45/45，dev 44/46 |
| Jinen v2 xsmall | llama.cpp | f16 | 72.1 | 90 | 44.1 | 86.2 | 0 | 1.000 | 100.0% | NFKC 58.0%，accept+NFKC 73.5% |
| Jinen v2 xsmall | llama.cpp | Q8_0 | 38.7 | 58 | 17.5 | 39.0 | 不適用 | 不適用 | 98.5% 輸出相同 | NFKC 58.5%，accept+NFKC 74.0% |
| Jinen v2 xsmall | llama.cpp | Q5_K_M | 28.3 | 48 | 16.5 | 34.9 | 不適用 | 不適用 | 92.5% 輸出相同 | NFKC 59.0%，accept+NFKC 73.5% |
| Jinen v2 xsmall | llama.cpp | Q4_K_M | 26.3 | 46 | 16.5 | 54.3 | 不適用 | 不適用 | 94.5% 輸出相同 | NFKC 57.0%，accept+NFKC 73.0% |

注：

- reranker／選字的「排序一致率」是每個案例的候選 top-1 相同比例；標點是逐 token argmax 相同比例。
- Spearman 是每案例分數排序的 Spearman 平均（標點為全部 logits 的整體Spearman）。
- Jinen 的「輸出相同」是與 f16 逐字元完全相同的比例；llama.cpp 的量化是 weight-only，沒有可對齊的 fp32 logits。
- Jinen 的 p50 是三次量測中干擾最少的一次；另外兩次因其他工作佔滿 CPU，p50 曾到 82–224 ms。

## 端到端細節

### 日文 reranker（modern-500）

| 設定 | top-1 | top-5 | 套用重排的組數 | 逾時 | 推論 p50／p95 |
|---|---:|---:|---:|---:|---:|
| `--rerank off` | 86.0% | 98.0% | 0 | 0 | 不適用 |
| fp32 | 83.4% | 96.4% | 538/544 | 30 | 32.6／64.1 ms |
| int8 | 82.0% | 95.2% | 556/556 | 1 | 17.7／41.7 ms |
| int4 | 83.8% | 96.4% | 267/486 | 382 | 40.5／91.8 ms |

fp32 因逾時跳過 30 組，int8 只跳過 1 組。重排目前整體是負貢獻，套用得越完整，top-1 掉得越多；int8 比 fp32 低 1.4 個百分點，主要來自它幾乎沒有逾時，並不是 int8 的排序比較差。int4 的 83.8% 是在 79% 的重排都被 watchdog 跳過後得到的，不能解讀成準確率較高。

### 日文標點（100 句 modern-500，`convert --punct on`）

| 精度 | 句子完全相同 | 字元位置相同率 | 編輯相似度 | 基礎文字相同 |
|---|---:|---:|---:|---:|
| int8 vs fp32 | 93% | 94.9% | 99.58% | 100/100 |
| int4 vs fp32 | 90% | 92.8% | 99.43% | 100/100 |

所有差異都是「、」的插入位置；拿掉標點後 100 句的基礎文字完全相同。字元位置相同率會被插入造成的位移懲罰，編輯相似度較能反映實際差異。

### 注音選字

| 精度 | golden | dev exact（model-only） | dev noisy（model-only） | noisy 端到端 top-1 | 端到端改壞 |
|---|---:|---:|---:|---:|---:|
| fp32 | 45/45 | 46/46 | 48/161 | 45.3% | 2 |
| int8 | 45/45 | 45/46 | 47/161 | 45.3% | 2 |
| int4 | 45/45 | 44/46 | 48/161 | 46.0% | 1 |

dev 的「統計首選錯」在 exact 集是 0、noisy 集是 52；模型只靠分數救回的數量在所有精度都是 0。因此「統計錯時救回」這個指標目前無法區分精度，真正的差異在 model-only top-1 與分數穩定度。int4 的 noisy 端到端略高，是因為 margin 3.0 把大部分覆寫擋掉，行為接近統計基準，不能當成模型品質較好。

### Jinen v2 xsmall（AJIMEE 200）

無左文脈，200 題：

| 量化 | literal exact | NFKC exact | accept | accept+NFKC | 與 f16 輸出相同 | 字元相似度 |
|---|---:|---:|---:|---:|---:|---:|
| f16 | 51.5% | 58.0% | 69.0% | 73.5% | 100% | 100% |
| Q8_0 | 52.0% | 58.5% | 69.5% | 74.0% | 98.5% | 99.85% |
| Q5_K_M | 52.0% | 59.0% | 68.5% | 73.5% | 92.5% | 98.50% |
| Q4_K_M | 50.5% | 57.0% | 68.5% | 73.0% | 94.5% | 98.28% |

給原始左文脈尾端 64 字的 100 題，NFKC 分別是 f16 60.0%、Q8_0 60.0%、Q5_K_M 61.0%、Q4_K_M 61.0%；accept+NFKC 是 75.0%、75.0%、76.0%、76.0%。四者差距在 1 個百分點內，量化沒有明顯改變品質排序。

模型卡宣稱 NFKC 78.5–79.0%，本機只重現 58–59%。這與 `jinen-eval` 分支先前記錄的 59.0% 完全一致，差異來自模型卡與本專案的 prompt／計分不同，不是本次量化造成。

## 建議

| 模型 | 建議 | 理由 |
|---|---|---|
| 日文 reranker | **int8**，但產品預設先關閉重排 | int8 幾乎無損、p95 37 ms；fp32 在負載下超過 100 ms，int4 大量逾時。重排本身目前讓 top-1 從 86.0% 掉到 82–83%，應先修 gating |
| 日文標點 | **int8** | argmax 99.44%、端到端只有逗號差異、延遲 12 ms。int4 省 16 MB 但慢一倍，不值得 |
| 注音選字 | **fp32** | int8 已經掉一個 dev 案例且 logits 誤差 10.3；int4 分數排序崩壞。只有記憶體不足時才考慮 int8，且要重新校準 |
| Jinen v2 xsmall | **Q5_K_M** | 28.3 MB、品質與 f16 同級、延遲與 Q8/Q4 相同。Q8 沒有品質收益，Q4 掉 2 個百分點 |
| candle 執行期 | **暫不引入** | 現有 ONNX int8 已在延遲與品質上足夠；candle 的 Q4 只有在需要更小體積或跨平台一致時才有理由。等 candle 欄位量完再決定 |

以目前延遲預算回答四個問題：

1. **哪個模型對量化最敏感？** 注音選字。int8 的候選 top-1 一致率只有 92.3%，int4 只有 61.5%。日文 reranker 與標點在 int8 都接近無損。
2. **int8 是否幾乎無損？** 日文 reranker 與標點是；注音選字不是。int4 可用於日文標點，不建議用於 reranker（延遲）與注音選字（分數崩壞）。
3. **哪個精度最划算？** reranker 與標點都是 int8；注音選字是 fp32；Jinen 是 Q5_K_M。
4. **是否值得為 int4 引入 candle？** 目前不值得。ONNX int4 在 reranker 與選字都沒有競爭力；candle Q4 的價值要等 candle 欄位補完，特別是它能否在 100 ms 內跑完 reranker 的 8 候選 batch。

## candle 欄位（待補）

`candle-spike-ja` 已在 main 的 `688f1f6` 合併，提供日文 reranker 的 candle 後端；本 branch 依 `common.md` 不 merge。下表先留欄位，等整合後用同一批輸入補齊：

| 模型 | candle f16 | candle Q8_0 | candle Q5K | candle Q4K | candle Q4_0 |
|---|---|---|---|---|---|
| 日文 reranker | 待補 | 待補 | 待補 | 待補 | 待補 |
| 日文標點 | 待補 | 待補 | 待補 | 待補 | 待補 |
| 注音選字 | 待補 | 待補 | 待補 | 待補 | 待補 |
| Jinen v2 xsmall | 待補 | 待補 | 待補 | 待補 | 待補 |

## 限制

- 量測期間機器有其他工作，loadavg 19–31。延遲取三次中干擾最少的一輪；同一精度在不同負載下 p50 可差 2 倍。相對排序穩定，絕對值只代表本機當下狀況。
- 日文 reranker 的 int4 模型只有 `seq=124` 能跑。其他長度會在 position embedding 的 Add 出現 broadcast 錯誤；目前部署固定 124，因此可用，但這是 int4 轉換的相容性限制。
- 注音 int4 無法量化 `GatherND` 使用的 57 MB decoder 權重，檔案仍有 75.8 MB，比 int8 的 33 MB 大。量化器同時把輸入 embedding 轉成 4-bit 後，logits 明顯失真。
- `eval zh-golden --data build-zh` 在目前 CLI 會超過 60 秒仍未完成（select 關閉時也一樣），因此 golden 使用預設固定詞庫；dev 救回數用 `zh-calibrate` 量測。
- 沒有 `--backend` 參數，精度切換用 `JC_MODELS`／`JC_JA_MODELS` 環境變數，未修改 `jc-cli`。
- Jinen 的量化檔只用于評測；模型是 CC-BY-SA-4.0，散布衍生權重需同授權。

## 產出與重現

| 路徑 | 內容 |
|---|---|
| `docs/reports/quant-study-2026-11-20.json` | ONNX 大小、延遲、RSS、數值一致性 |
| `docs/reports/quant-e2e.json` | 三個模型的端到端結果 |
| `docs/reports/quant-jinen-gguf.json` | Jinen 四種量化的品質與輸出相似度 |
| `docs/reports/quant-jinen-latency.json` | Jinen 三輪延遲 |
| `scripts/quant/quantize_models.py` | 產生 int8／int4，輸出到 quant 目錄 |
| `scripts/quant/measure_onnx.py` | ONNX 量測與 fp32 數值比較 |
| `scripts/quant/e2e_punct.py` | 標點端到端字元一致率 |
| `scripts/quant/eval_jinen_quant.py` | Jinen AJIMEE 評測（改寫自 jinen-eval） |
| `scripts/quant/latency_jinen.py` | Jinen 三輪延遲量測 |

```sh
# ONNX 量化與量測
uv run --with onnxruntime --with onnx --with onnx_ir --with numpy \
  python scripts/quant/quantize_models.py
/tmp/opencode/quant-venv/bin/python scripts/quant/measure_onnx.py all

# 端到端（精度用 overlay 切換）
export ORT_DYLIB_PATH=/path/to/libonnxruntime.so
JC_JA_MODELS=/mnt/work/jc/jc-ime-data/models/quant/overlay/int8 \
  ./target/release/jc-cli eval ja --corpus modern-500 --rerank on
JC_MODELS=/mnt/work/jc/jc-ime-data/models/quant/overlay/int8 \
  ./target/release/jc-cli eval zh-golden --select-model on
python scripts/quant/e2e_punct.py

# Jinen GGUF
uv run --with llama-cpp-python python scripts/quant/eval_jinen_quant.py --model q5_k_m
uv run --with llama-cpp-python python scripts/quant/latency_jinen.py
```
