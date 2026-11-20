# candle-zh：注音選字模型搬到 candle（Q8_0）

把 `zh-select` 的 forward 從 ONNX Runtime 換成 candle，先對齊 fp32，再量化成
Q8_0。ORT 後端保留，用 `--backend` 切換。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/neural/native/backend_ort.go`（sequence 模式） | `crates/jc-model/src/candle/zh.rs`（`ZhNet::scores`） | rewritten：九個輸入、404 個節點的 forward 逐段重寫 |
| `internal/neural/native/input/input.go`（輸入佈局） | `crates/jc-core/src/zh/select.rs`（`prepare`）、`crates/jc-model/src/candle/zh.rs`（`ZhInput`） | ported 既有實作，candle 直接吃同一份 `Arrays` |
| `internal/neural/native/neural.go`（後端選擇） | `crates/jc-model/src/candle/zh.rs`、`crates/jc-core/src/zh/select.rs`（`Engine`） | rewritten：`default_backend()` 決定 ORT session 或 candle 權重 |
| — | `crates/jc-cli/src/bin/candle-zh-check.rs` | 新增：轉檔、量化、逐組比對與延遲／記憶體量測 |
| — | `crates/jc-cli/src/cli.rs`、`eval_cmd.rs` | `eval zh-golden --backend ort|candle` |
| `core/rerank/rerank.go`、`core/session/rerank.go` | `crates/jc-core/src/zh/select.rs`（`choose`／`choose` 呼叫端） | 不動：融合、margin、候選順序完全沒改 |

## 架構

上游 `Funmula/0c-sequence-d512` 只有 ONNX，沒有 safetensors，所以 candle 的
forward 是照 `model.onnx` 的節點重寫，不是照 PyTorch 原始碼。

```text
[BOS] 前文(<=64) [SEP] | 共用音節(n-1) | 分支節點 + 候選前綴樹
```

- 詞嵌入按 `types` 分派：`type == 1` 用 `base_embedding[bases] +
  tone_embedding[tones]`，其餘用 `char_embedding[ids]`，最後加
  `type_embedding[types]`。
- 8 層 pre-LN：`x = x + attn(norm1(x))`、`x = x + mlp(norm2(x))`。
- **第 0 層的 attention norm 是 `embeddings.norm`**：ONNX 把它的輸出直接餵進
  第 0 層的 qkv，殘差流從歸一化後開始。candle 在進第一層前套用
  `embedding_norm`，第 0 層的 `attn_norm` 因此是 `None`，不能再歸一化一次。
  這是重寫時最容易多套一次的地方。
- RoPE 是 rotate-half：`cat(-x2, x1)`，角度是 `positions × inv_freq`，前後半
  各 32 維。`inv_freq` 直接從 ONNX 的 `expand` 常數轉出，不重算。
- attention 遮罩用 `-1e9` 加成；ORT 用 `-3.4e38`，softmax 後同為 0。
- MLP 的 gate／up 在 ONNX 已是同一條 `[512, 1536]` MatMul，candle 照這個
  佈局存成 `[1536, 512]`，切半後 `gelu(gate) * up`。
- 輸出頭只對 `proj_pos` 的列做 `dense -> gelu -> norm`，再乘整張字元表，
  取 `out_ids` 的 27,859 個位置做 log-softmax。`out_ids` 不是連續區間：
  decoder 有 27,865 列，扣掉 `0,1,2,3,5,7` 六個 token。
- `target_ids` 是 `out_ids` 的**下標**（`char_id - 6`），不是 token id；
  GatherND 的列索引是 `proj_pos` 的下標。

## 權重轉換與量化

`convert_onnx` 用手寫的 protobuf 讀取器解析 `ModelProto -> GraphProto ->
TensorProto`，只讀 initializer，不改數值：

```sh
cargo run -p jc-cli --release --bin candle-zh-check -- --convert --quant q8_0
```

輸出 `models/zh-select/model.safetensors`（58 個張量、32,638,412 個參數、
124.5 MiB）與 `models/zh-select/model.q8_0.gguf`（33.9 MiB、8.500
bits/weight）。模型檔不進 git。

- 命名換成 forward 用的語意名稱；MatMul 的 `[in, out]` 權重轉成 `[out, in]`。
- q／k／v 沿輸出維度合併成一條 `in_proj`，gate／up 保持合併，少兩次 matmul。
- 每層的 `val_N` 名稱沒有語意，轉檔時逐個驗形狀；形狀不符直接回錯。
- 量化只碰矩陣乘法的權重與字元表；LayerNorm、bias、`out_ids`、`inv_freq`
  留 fp32。字元表同時當輸入查表與輸出投影，量化版是同一個 `QMatMul`，
  查表走 `QMatMul::embedding`，不另外解量化。

## 後端切換

```sh
jc-cli eval zh-golden --backend ort
jc-cli eval zh-golden --backend candle
```

- `--backend` 在安裝模型前呼叫 `jc_model::set_default_backend`。預設仍是
  `ort`；`convert --backend candle` 也吃同一個設定。
- candle 的權重順序：`JC_ZH_SELECT_WEIGHTS` → `model.safetensors` →
  `model.gguf`。`.gguf` 自動走量化路徑。
- candle 後端不初始化 ONNX Runtime，`ORT_DYLIB_PATH` 沒設也能跑。
- 量化版沒有可中止的 watchdog：逾時只能在呼叫端退回統計排序，ORT 版的
  100 ms 中止行為沒有對應。

## 驗證數字

環境：GB10，20 核，ORT 1.32.0。比對樣本是 fuzzy 注音題解出的 46 組、732 個
有效候選（每組最多 16 個），ORT 與 candle 吃完全相同的 `prepare()` 結果。

| 檢查 | 標準 | fp32 candle | Q8_0 |
|---|---|---:|---:|
| logits 誤差 vs ORT fp32 | < 1e-3 | max 1.163e-4、mean 1.283e-5 | max 3.133e-1、mean 6.566e-2 |
| 逐對排序一致 | ≥ 98% | 5466/5466（100.00%） | 5439/5466（99.51%） |
| top-1 一致 | — | 46/46 | 46/46 |
| golden | 45/45 | **45/45** | **45/45** |
| 單次推論 p50 | ≤ 30 ms | 14.05 ms | 9.36 ms |
| 常駐記憶體 | ≤ 60 MiB | 124.5 MiB（mmap） | **45.3 MiB** |

誤差指的是模型輸出 `scores`（每個候選的 log-probability 總和），這是這個
模型唯一的輸出，也是 ORT 對照的同一條資料。

延遲是 `eval zh-golden` 的 100 次推論統計（同一台機器、共用負載）：

| 後端 | 推論 p50 | p95 | 端到端峰值 RSS |
|---|---:|---:|---:|
| ORT fp32 | 11.8–12.6 ms | 19.4–20.4 ms | 309.7 MiB |
| candle fp32 | 14.0–15.1 ms | 21.9–24.5 ms | 274.4 MiB |
| candle Q8_0 | 9.4–9.8 ms | 15.0–17.5 ms | 74.7 MiB |
| 不載入模型 | — | — | 13.2 MiB |

這台機器是共用的，同一組設定重跑 p50 會差 1–2 ms；上表列最近三次的範圍，
峰值 RSS 是一次 `eval zh-golden` 的 `Maximum resident set size`。

記憶體另外用單一權重檔的行程量（`candle-zh-check --load-only`，不載入詞庫）：

| 權重 | 檔案 | 載入後 VmRSS | 100 次推論後 |
|---|---:|---:|---:|
| Q8_0 | 33.9 MiB | 39.4 MiB | 45.3 MiB |

fp32 是 mmap，檔案頁會被核心回收，VmRSS 不穩定（同一支行程量到 128 MiB 到
17 MiB 都有），所以只拿檔案大小與端到端峰值比較。Q8_0 的端到端峰值比無模型
基準多 61.5 MiB，差額是長句候選的暫存張量（輸出投影 `[proj 列, 27,859]`），
推論結束就釋放，不是常駐。

## jc-core 的最小接線

`SelectModel` 的 `session: Mutex<Session>` 換成 `engine: Engine`，多兩個
分支 `Candle`／`CandleQuant`；`run()` 依 backend 分派，candle 只量延遲。
**`choose`、融合權重、margin、候選順序、`rerank_nbest` 一行都沒改**，golden
結果與 ORT 相同。這是整合所需的最小改動，不在任務書列的 owned paths 內，
先在此記錄。

## 沒做完

- 量化版沒有推論逾時中止，只有呼叫端在失敗時退回統計排序。
- 沒有做 KV 快取或只算真 token 的捷徑；目前每次 forward 都重算整段序列。
  最短的 golden 題 p50 已低於 30 ms，長句仍會線性變慢。
- 依驗證政策沒跑 `cargo test`，只 `cargo check --all-targets`；轉檔對齊與
  Q8_0 的測試已寫在 `candle/zh.rs`，等整合階段執行。
- 量化只驗 Q8_0；Q5K／Q4K 的匯出支援已共用，但這個模型對量化最敏感，
  沒有實測就不建議。
