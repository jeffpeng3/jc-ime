# candle-punct：日文標點模型的 candle 實作

> **已移除（2026-11-20）**：標點模型不再使用，`candle/punct.rs`、
> `candle-punct-check` 與 `candle-quant punct` 都已刪除。本文保留為歷史紀錄，
> 移除清單見 [`drop-punct.md`](drop-punct.md)。

同一個 checkpoint、同一組 token id，把日文標點模型的 forward 從 ONNX
Runtime 換成 candle。encoder 直接複用 `candle-spike-ja` 的實作：pre-LN
transformer、fused QKV、`gelu_erf`、以輸出維度切給 rayon 的 linear。這輪
不移除 `ort`，兩個後端並存，用 `--backend` 切換，預設仍是 `ort`。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/neural/native/punct_ort.go`（`PunctBackend`） | `crates/jc-model/src/punctuation_model.rs` | rewritten：改成 `Engine::Ort`／`Engine::Candle`，介面與 window 邏輯不動 |
| `internal/neural/native/punct_plan.go`（window、merge、decision） | `crates/jc-model/src/punct_plan.rs` | n/a：先前 task 已完成，本 task 沿用 |
| `internal/neural/native/manifest.go`（模型 manifest） | `crates/jc-model/src/manifest.rs`、`crates/jc-model/src/candle/mod.rs` | rewritten：candle 權重路徑沿用 manifest 的 `model` 欄位 |
| —（`oc_training/model/punctuation.py` 上游 PyTorch 模型） | `crates/jc-model/src/candle/mod.rs`、`crates/jc-model/src/candle/punct.rs` | rewritten：pre-LN encoder、fused QKV、per-token 分類頭照權重名稱重寫 |
| —（`oc_training/export/punctuation.py` ONNX 匯出） | `crates/jc-model/src/candle/punct.rs` | rewritten：`arange(seq)` 位置與 `[batch, seq, 4]` logits 直接對齊 |
| — | `crates/jc-cli/src/bin/candle-punct-check.rs` | 新增：逐組比對 logits、argmax、標點輸出與延遲 |
| — | `scripts/convert-punctuation-safetensors.py` | 新增：`best.pt` → safetensors；沒有 checkpoint 時可從 ONNX initializer 還原 |

## 實作

- 架構與 reranker 相同：D512／8 層／8 heads／FFN 1024／詞表 32768，
  `max_positions = 512`。載入時用張量 shape 驗證，對不上就回錯。
- 共用 `candle::Encoder`、`candle::Layer` 與 `linear_chunked`；標點模型只
  多一個 `classifier`（4 類）與 `arange(seq)` 位置。reranker 的
  right-anchored 位置表不適用這裡。
- 前處理與 ORT 共用同一份 `Tokenizer` 與 sliding window：256 token、stride
  128。ORT 逐窗呼叫 `Run`；candle 把所有視窗合成一次 batch forward，短序列
  以 pad + attention mask 對齊。
- `attention_mask` 全為真時直接跳過遮罩加法；有 padding 時才建
  `(rows, 1, 1, width)` 的 `-1e9` 加成遮罩。
- LayerNorm eps 1e-5、GELU 用 `gelu_erf`（PyTorch `F.gelu` 預設精確版），
  全程 fp32。
- candle 後端不初始化 ONNX Runtime，`ORT_DYLIB_PATH` 沒設也能跑。
- candle 的 forward 無法中途停止；`spec.timeout()` 在整批跑完後檢查，預算
  取「視窗數 × 單窗預算」，超過就整批視為失敗、輸出原文。產品預設每窗
  100 ms。

## 權重轉換

checkpoint 的 `state_dict` 名稱與 `oc_training.model.punctuation.PunctuationTagger`
相同，candle 照同名讀取，不需要改名：

```sh
/mnt/work/0c/.venv/bin/python scripts/convert-punctuation-safetensors.py \
  --checkpoint /mnt/work/0c/runs/p2-24a-punctuation-public-core/best.pt \
  --output /mnt/work/jc/jc-ime-data/models/punctuation-public-core/model.safetensors
```

輸出 104 個張量、33,865,732 個參數、fp32（135,473,800 bytes）。safetensors
metadata 記錄來源 checkpoint 路徑、sha256 與 `model_config`。模型檔不進
git。

沒有 checkpoint 時改用 `--onnx`：ONNX 匯出把 linear 權重轉置並匿名成
`onnx::MatMul_*`，腳本依 initializer 順序（每層 `in_proj`、`out_proj`、
`linear1`、`linear2`，最後 `classifier`）還原成 checkpoint 名稱。

載入順序：`JC_PUNCT_WEIGHTS` → 模型目錄的 `model.safetensors` →
manifest `model` 檔名換成 `.safetensors`。

## 後端切換

```sh
cargo run -p jc-cli --release -- convert "わたしはほんをよむ" --punct on --backend candle
cargo run -p jc-cli --release -- convert "わたしはほんをよむ" --punct on --backend ort
```

`jc_model::set_default_backend` 是行程層級設定，`jc-core` 的介面不動；
`--backend` 在載入模型前設定。也可以用 `JC_PUNCT_BACKEND=candle` 覆寫。
預設仍是 `ort`。

| 項目 | `--backend ort` | `--backend candle` |
|---|---|---|
| 權重 | 模型目錄的 `model.int8.onnx`（manifest 預設） | `model.safetensors`（fp32） |
| 需要 ONNX Runtime | 是 | 否 |
| 逾時 | `RunOptions::terminate` 中止單次推論 | forward 無法中止，整批跑完後檢查 |
| window | 逐窗 `Run` | 所有視窗合成一次 batch |

## 對齊驗證

`candle-punct-check` 讀 P2-24A validation 的前 N 個 256-token 視窗，用相同
id 跑 candle、ORT fp32、ORT int8。ORT fp32 對照模型是
`/mnt/work/0c/models/trained/punctuation-public-core/punctuation-public-core.fp32.onnx`
（訓練產物，不在產品目錄）；ORT 需要 `ORT_DYLIB_PATH`。

```sh
ORT_DYLIB_PATH=<...>/libonnxruntime.so.1.32.0 \
cargo run -p jc-cli --release --bin candle-punct-check -- --groups 200
```

結果（GB10，20 核，機器同時有其他工作在跑，延遲見下節）：

```text
groups 200，seq 256，tokens 51200
candle vs ort-fp32：max |Δ| 2.289e-5，mean |Δ| 2.097e-6，argmax 一致 51200/51200
candle vs ort-int8：max |Δ| 1.019e0，mean |Δ| 4.973e-2，argmax 一致 51054/51200
```

標點輸出一致率用 `0c-testdata/ja/modern_500.json` 的 surface，移除既有
標點後交給兩個後端：

```text
candle vs ort-fp32：字元一致 5099/5099（100.00%），整句相同 500/500，candle 插入 340/500 篇
candle vs ort-int8：字元一致 5052/5101（99.04%），整句相同 484/500，candle 插入 340/500 篇
```

candle 與 ORT fp32 的差異只剩浮點運算順序。與 int8 的差異是量化誤差：
ORT fp32 自己對 int8 的 logits 也差 1.019，argmax 有 146/51200 個位置不同。
實際會看到少數句子在 `、` 與 `。` 之間擺動，例如
`やまださんはがくせいです` 的第 4 個候選，candle（fp32）輸出
`山田サンは、学生です。`，ORT int8 輸出 `山田サンは学生です。`。要求逐字
相同時，對照基準要用 ORT fp32；這也是本輪驗收的定義。

## 延遲

`candle-punct-check --seq N` 每次 forward 一個視窗；GB10 20 核，機器同時
有其他工作在跑，數字取多次執行的中位數區間。

| 視窗 | candle p50 | ort-fp32 p50 | ort-int8 p50 | candle／fp32 | candle／int8 |
|---|---:|---:|---:|---:|---:|
| 13 token（IME 短句） | 11.8–13.2 ms | 2.0–2.4 ms | 1.29–1.43 ms | 5.2–6.5x | 7.3–10.2x |
| 256 token（視窗上限） | 61.25 ms | 17.51 ms | 18.03 ms | 3.6x | 3.5x |

`convert` 對 5 個候選各推論一次（含 tokenizer）：

| 後端 | punct p50 | p95 |
|---|---:|---:|
| `--backend ort` | 1.09–1.17 ms | 1.89–2.12 ms |
| `--backend candle` | 6.49–7.64 ms | 13.79–15.17 ms |

**延遲標準未通過**：candle 比 ORT 慢 3.5 倍以上，短句時差 5 到 10 倍。
原因是 candle 的 CPU gemm 沒有 BLAS 或 SVE 特化，而且每次 forward 都重新
包裝整份權重：32 個 linear 各讀 0.5–3 MB，短序列時固定成本主導。fp32 權重
是 int8 的 4 倍，光是讀完一輪就比 ORT 的整次推論久。

試過但沒有採用的方向：

- `linear_chunked`：batch 大時（rows ≥ 64）沿輸出維度切給 rayon 有小幅
  幫助；IME 的短序列反而變慢，所以 rows < 64 直接走 candle 內建 gemm。
- candle 的 Q8_0 `QMatMul`：單次 matmul 比 fp32 快約 2.5 倍，但量化後
  logits 誤差到 1e-1，無法同時滿足 logits < 1e-3；kernel 也是單執行緒，
  切給 rayon 後仍追不上 ORT。
- fp16 權重：256 token 時快 1.8 倍，短序列反而更慢（轉換成本），且誤差
  超過 1e-3。

## 結論

數值對齊通過：logits、argmax、最終標點輸出都與 ORT fp32 完全一致。candle
可以當作不需要 ONNX Runtime 的對照與驗證後端。

延遲不通過：以目前的 candle CPU 後端，標點模型不適合放進 IME 的送出
路徑。要達標需要預先打包的權重與 int8 kernel，這超出本輪「換 forward」
的範圍。產品預設仍走 ORT，`--backend candle` 只作為對照。

## 下一步

- 若要 candle 出貨，先解決 gemm 的權重打包：把 linear 權重預先轉成
  gemm 的 packed 格式，或接上 armpl／BLAS，才有機會把短序列壓到毫秒級。
- 或者讓 candle 直接吃 ORT 匯出的 per-channel int8 權重，實作
  `MatMulInteger` 等價路徑；此時對照基準要改成 ORT int8，不能再宣稱
  logits < 1e-3。
- 標點模型的 batch forward 已可用，之後可以讓 `convert` 一次送 5 個候選，
  減少重複讀權重。
