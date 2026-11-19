# candle-spike-ja：日文 reranker 的 candle 實作

同一個 checkpoint、同一組 token id，把日文 reranker 的 forward 從 ONNX
Runtime 換成 candle。這輪不移除 `ort`，兩個後端並存，用 `--backend` 切換。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `oc_training/model/reranker.py`（上游 PyTorch 模型） | `crates/jc-model/src/candle/mod.rs` | rewritten：pre-LN encoder、fused QKV 與 scalar head 照權重名稱重寫 |
| `oc_training/export/reranker.py`（ONNX 匯出與 DeploymentWrapper） | `crates/jc-model/src/candle/mod.rs` | rewritten：`position_ids()` 與池化 slot 直接從 `ModelSpec` 推導 |
| `oc_training/eval/deploy.py`（right-anchored 佈局） | `crates/jc-model/src/candle/mod.rs` | ported：位置表 `[0, 61..120, 121, 122, 123, 124..183]` |
| `internal/neural/native/rerank_ort.go` | `crates/jc-model/src/rerank.rs` | rewritten：`Reranker` 內部改成 `Engine::Ort`／`Engine::Candle` |
| — | `crates/jc-cli/src/bin/candle-ja-check.rs` | 新增：逐組比對 logits、排序與延遲 |
| — | `scripts/convert-reranker-safetensors.py` | 新增：`best.pt` → safetensors 一次性轉檔 |

## 實作

- 架構固定為 D512／8 層／8 heads／FFN 1024／詞表 32768，對應
  `reranker-extra` 的 `RerankerConfig`。載入時用張量 shape 驗證，對不上就
  回錯，不會靜默降級。
- 前處理與模型無關的部分沿用 `layout::pack_batch`，candle 與 ORT 拿到
  完全相同的 `input_ids`／`attention_mask`。
- 絕對位置是部署佈局的 right-anchored 位置，不是 `arange(124)`：最近左文
  token 用 120、候選 122、右文 124..183。序列長度 124、池化 slot 62。
  這兩個數字由 `seq_len = 2 * context + 4`、`candidate_slot = context + 2`
  檢查，不是寫死。
- attention 的 padding mask 以 `-1e9` 加在 key 維度，softmax 後為 0；
  池化 slot 永遠是真 token，不會出現全遮罩列。
- **只算真 token**：被遮罩的格子對沒被遮罩的 query 貢獻為 0，所以先把
  `attention_mask == 0` 的位置全部丟掉，位置嵌入仍用原本的絕對位置，池化
  改成每一列自己的候選索引。124 格全滿時等同原本計算；IME 的短上下文
  （前後各 0..30 token）能把序列縮到 5..60 格。`candle-ja-check --shrink N`
  就是拿來驗這條路徑。
- LayerNorm eps 1e-5、GELU 用 `gelu_erf`（PyTorch `F.gelu` 預設的精確版），
  全程 fp32。
- candle 的 CPU gemm 是單執行緒，所以 `linear_chunked` 把權重沿輸出維度
  切給 rayon；每個 chunk 仍是同一個 gemm、同一條 k 軸，數值不變。
- candle 後端不初始化 ONNX Runtime，`ORT_DYLIB_PATH` 沒設也能跑。

## 權重轉換

checkpoint 的 `state_dict` 名稱與 `oc_training.model.reranker.Reranker`
相同，candle 照同名讀取，不需要改名。轉檔只做 dtype 正規化：

```sh
/mnt/work/0c/.venv/bin/python scripts/convert-reranker-safetensors.py \
  --checkpoint /mnt/work/0c/runs/p2-23f-extra-finetune/best.pt \
  --output /mnt/work/jc/jc-ime-data/models/reranker-extra/model.safetensors
```

輸出 104 個張量、33,864,193 個參數、fp32（135 MB）。safetensors 的
metadata 記錄來源 checkpoint 路徑、sha256 與 `model_config`。模型檔不進 git。

載入順序：`JC_RERANK_WEIGHTS` → 模型目錄的 `model.safetensors` →
manifest `model` 檔名換成 `.safetensors`。

## 後端切換

```sh
jc-cli eval ja --corpus modern-500 --rerank on --backend candle
jc-cli eval ja --corpus modern-500 --rerank on --backend ort
```

`jc_model::set_default_backend` 是行程層級設定，`jc-core` 的 reranker 介面
不動；`--backend` 在載入模型前設定，預設仍是 `ort`。也可以用
`JC_RERANK_BACKEND=candle` 覆寫。

兩個後端的差異：

| 項目 | `--backend ort` | `--backend candle` |
|---|---|---|
| 權重 | 模型目錄的 `model.int8.onnx`（manifest 預設） | `model.safetensors`（fp32） |
| 需要 ONNX Runtime | 是 | 否 |
| 逾時 | `RunOptions::terminate` 中止單次推論 | forward 無法中止，只在群組之間檢查預算 |

## 對齊驗證

`candle-ja-check` 讀 `homophone-extra-v1/test.jsonl` 的前 N 組，用相同
id／mask 跑 candle、ORT fp32、ORT int8。ORT fp32 對照模型是
`/mnt/work/0c/models/trained/reranker-extra/model.onnx`（訓練產物，不在產品
目錄）；ORT 需要 `ORT_DYLIB_PATH`。

```sh
ORT_DYLIB_PATH=<...>/libonnxruntime.so.1.32.0 \
cargo run -p jc-cli --release --bin candle-ja-check -- --groups 200
```

結果（GB10，20 核，全程 fp32）：

```text
groups 200，candidates 927，seq 124，slot 62
candle vs ort-fp32：max |Δ| 1.752e-5，mean |Δ| 1.848e-6，排序一致 200/200
candle vs ort-int8：max |Δ| 2.535e-1，mean |Δ| 2.074e-2，排序一致 191/200
延遲（50 組，每組一次 batch forward）
  candle    mean 132.67 ms，p50 126.86 ms，p95 179.63 ms，min 96.84 ms
  ort-fp32  mean  37.54 ms，p50  39.57 ms，p95  54.30 ms，min 22.99 ms
  ort-int8  mean  20.63 ms，p50  20.66 ms，p95  28.32 ms，min 13.02 ms
PASS：logits 誤差 < 1e-3、排序完全一致
```

`--shrink 6`（每側只留 6 個 token）也跑過，驗證只算真 token 的路徑：

```text
candle vs ort-fp32：max |Δ| 1.383e-5，排序一致 20/20
candle             p50 23.24 ms
ort-fp32           p50 67.49 ms
```

124 格全滿時 candle 比 ORT fp32 慢 3.2 倍：candle 的 CPU gemm 沒有 SVE
特化，16 路切分只跑到約 190 GFLOPS。短上下文時 candle 靠跳過遮罩格子反超
ORT fp32，實測 IME 的延遲見下表。

## 驗證：modern-500

環境：GB10，ORT 1.32.0，`model.int8.onnx` 是產品 manifest 的預設模型。

```sh
cargo run -p jc-cli --release -- eval ja --corpus modern-500 --rerank on --backend candle
cargo run -p jc-cli --release -- eval ja --corpus modern-500 --rerank on --backend ort
```

| 後端 | n | top-1 | top-5 | boundary exact | boundary f1 | p50 | p95 | timeout |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `--backend candle` | 500 | 82.2% | 95.2% | 84.6% | 0.8842 | 36.24 ms | 64.63 ms | 0.0% |
| `--backend ort`（int8） | 500 | 82.0% | 95.2% | 84.4% | 0.8825 | 33.62 ms | 66.84 ms | 0.2% |

rerank 統計：

```text
candle：calls 500，groups 556，scored 556，changed 325，timeout 0
        inference p50 17.95 ms／p95 26.74 ms
ort   ：calls 500，groups 556，scored 556，changed 324，timeout 0
        inference p50 17.67 ms／p95 36.14 ms
```

單次推論（每組 5 個候選）candle 17.95 ms 對 ORT int8 17.67 ms，慢 1.6%。
這台機器共用，重跑時 candle inference p50 落在 18..21 ms、整題 p50 落在
36..41 ms，差異來自負載，不是後端。
兩個後端都送滿 556 組、沒有因預算被跳過。

**top-1 差 1 題（82.2% vs 82.0%）不是後端差異，是權重精度差異**：ORT
走產品 manifest 的 `model.int8.onnx`，candle 走 checkpoint 的 fp32。逐題
比對 500 題，只有 3 題的 top-5 排序不同，全部是漢字／片假名近乎同分的
候選；相同權重下 logits 差 1.8e-5、排序 200/200 一致。把 ORT 換成 fp32
`model.onnx` 反而無法在 100 ms 預算內跑完（modern-500 只 scored 197/479
組），所以產品基準只能跟 int8 比。

## 下一步

- candle 的 gemm 在 aarch64 沒有 SVE 特化，124 格全滿時仍比 ORT fp32 慢
  3.2 倍。真的要走 candle 出貨，先換掉 CPU gemm 或改吃 int8 權重。
- 目前只實作 reranker；標點與注音選字模型仍只在 `ort`，等 reranker 的
  權重格式定案再說。
