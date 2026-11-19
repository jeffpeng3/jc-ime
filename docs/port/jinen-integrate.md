# jinen-integrate：把 Jinen v2 做成離線轉換後端（candle 版）

用 candle 載入 Jinen v2 xsmall 的 Q5_K_M GGUF，跑三組題庫，與 Python
（llama-cpp-python 0.3.37）版本逐題對照。這一輪只做離線推論與量測，沒有接進
`Session`、沒有改預設行為。

## 結論

| 檢查 | 標準 | 結果 |
|---|---|---|
| `ajimee-200` accept | 與 Python 68.5% 相差 ≤ 1pp | **68.0%（136/200）**，差 −0.5pp，通過 |
| `modern-500` top-1 | 與 Python 比較並記錄 | **81.4%（407/500）**，Python 81.6%（408/500），差 −0.2pp |
| `hard-cases` top-1 | 記錄 | **77.1%（27/35）**，Python 同為 77.1% |
| xsmall 延遲 p50 | ≤ 60 ms | **28.59 ms**（4 threads，loadavg 26.4） |
| 記憶體 | RSS 增量 | 載入前 3.2 MB → 載入後 120.3 MB，峰值 **127.3 MB** |

Rust 與 Python 的差距全部來自量化 gemm 的數值差：735 條 prompt 的 token id
完全相同（0 筆不符），5 題輸出不同，且 5 題的貪婪選擇都發生在最高分與次高分
差距 0.0018～0.0331 的位置。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| — | `crates/jc-model/src/candle/jinen.rs` | 新增：GGUF 載入、tokenizer、prompt、greedy 解碼 |
| — | `crates/jc-cli/src/bin/jinen-eval.rs` | 新增：三組題庫量測、延遲／RSS、逐題 JSON |
| — | `crates/jc-model/Cargo.toml` | 新增 `candle-transformers`、`tokenizers` 依賴 |

這個功能在 Go 版沒有對應實作，Jinen 先前只有 `scripts/eval_jinen_ajimee.py`
（llama-cpp-python）的評測腳本，所以沒有可搬的行為測試。

## 模型與執行條件

| 項目 | 值 |
|---|---|
| GGUF | `models/external/jinen-v2-xsmall/jinen-v2-xsmall-Q5_K_M.gguf`（sha256 `24ff3af5…`） |
| 架構 | Qwen3：6 層、hidden 512、FFN 1408、8 heads／8 KV heads、head_dim 64、rope θ 10000、RMS eps 1e-6、context 1024 |
| 參數 | 35.7M |
| 量化 | Q5_K_M：Q5_K×34、F32×25（norm）、Q5_1×4（`ffn_down`）、Q6_K×3（`token_embd` 與 blk.2／5 的 `attn_v`）、Q8_0×2（blk.2／5 的 `ffn_down`） |
| runtime | candle 0.11（CPU，`candle-transformers::quantized_qwen3`） |
| threads | 4（`CANDLE_NUM_THREADS=4`、`RAYON_NUM_THREADS=4`，與 Python 的 `n_threads=4` 相同） |
| 機器 | GB10（20 核，10×X925＋10×A725，128GB UMA），量測期間 loadavg 26～27 |

## Tokenizer

GGUF 內的詞表是 llama.cpp 格式（`tokenizer.ggml.model = llama`），沒有
`merges` 欄位。HF repo `togatogah/jinen-v2-xsmall.gguf` 另外提供
`tokenizer.json`（revision `3910fd01`，sha256
`477c91a788bf97e7ed993a4441cfb2c4d60033e505ade17d6c367bc09ed6e6ba`），
用 `tokenizers` crate 直接讀：

- BPE，詞表 32000，`byte_fallback = true`。
- normalizer 是 NFKC，pre-tokenizer 是 `Digits(individual_digits=true)`。
- decoder 是 `ByteFallback` → `Fuse`，特殊 token 解碼時跳過。
- 額外 token `U+EE00`、`U+EE01`、`U+EE02` 的 id 分別是 32000、32001、32002，
  與 GGUF 的 32003 個詞逐一對應。

`tokenizer.json` 的 post-processor 會自動附上 `</s>`，這裡用
`add_special_tokens=false` 關掉，改成手動補 `BOS`（id 2）。理由是
llama.cpp 的 `tokenize` 預設 `add_bos=true`，Python 版跑 `create_completion`
時實際送出的序列開頭就是 `<s>`；不補 BOS 會少一顆 token。

檔案位置：資料目錄是唯讀，所以 `tokenizer.json` 放在 worktree 的
`models/external/jinen-v2-xsmall/`（`models/` 已被 `.gitignore` 排除）。
`jinen-eval` 的搜尋順序是 `--tokenizer` → `JC_JINEN_TOKENIZER` →
GGUF 同目錄 → 目前目錄的 `models/external/<slug>/`。

## Prompt 與停止條件

照模型卡：

```text
U+EE02<left_context>U+EE00<katakana_reading>U+EE01
```

- 讀音先做平假名→片假名（U+3041..=U+3096 加 0x60，一對一）。
- 有左文脈時只取尾端 64 個字元；沒有文脈時整段 `U+EE02<left_context>` 省略。
- 整段再做 NFKC。題庫的讀音是平假名，prompt 長度 8～50 tokens（平均 21.8）。
- 解碼是 greedy：每一步取 logit 最大的 token（同 llama.cpp 的
  `temperature=0`、`top_k=1`）。
- 停止條件：遇到 `</s>`（id 3）或滿 `max_new_tokens=256`；每題開始前清空
  KV cache，避免前一題的快取前綴影響結果。三組題庫都沒有截斷、沒有空輸出。
- 輸出直接取 `</s>` 之前的字串，不再做任何後處理。

## 量測結果

`cargo run -p jc-cli --release --bin jinen-eval --` 的完整輸出（4 threads）：

| 題庫 | n | literal exact | NFKC exact | accept | accept＋NFKC | p50 | p95 | CPU p50 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `ajimee-200` | 200 | 51.0% | 58.0% | **68.0%** | 73.0% | 28.59 ms | 79.71 ms | 110 ms |
| `modern-500` | 500 | 81.4% | 81.4% | **81.4%** | 81.4% | 19.69 ms | 34.23 ms | 80 ms |
| `hard-cases` | 35 | 77.1% | 77.1% | **77.1%** | 77.1% | 19.16 ms | 28.28 ms | 80 ms |

`modern-500` 與 `hard-cases` 沒有 `accept` 變體，accept 命中等於 literal
exact。`ajimee-200` 的 accept 判定與 Python 版相同：輸出等於 `surface` 或
任何一個 `accept` 變體。

與 Python 對照（同一台機器、同一個 window、同一批題目）：

| 題庫 | Rust accept | Python accept | 差異 |
|---|---:|---:|---:|
| `ajimee-200` | 136（68.0%） | 137（68.5%） | −1 題 |
| `modern-500` | 407（81.4%） | 408（81.6%） | −1 題 |
| `hard-cases` | 27（77.1%） | 27（77.1%） | 0 |

## 差距解釋：量化數值差，不是 tokenizer 或 prompt

- 用 `tokenizers`（Python binding）對三組共 735 條 prompt 編碼，再與
  llama.cpp `tokenize(add_bos=True)` 逐 id 比對：**735/735 完全相同**。
  Rust 版用同一個 `tokenizer.json` 與同一條 `tokenizers` 路徑，prompt
  因此逐位元組相同。
- 逐題輸出比對：735 題中 730 題完全相同，5 題不同。這 5 題在解碼第一步就
  分歧，當下最高分與第二高分差距：

| 題庫 | 題號 | candle | llama.cpp | 第一步差距 |
|---|---|---|---:|
| `ajimee-200` | `ajimee-087` | もとにしながら… | 元にしながら… | 0.0018 |
| `ajimee-200` | `ajimee-174` | …それを各三番にて… | …それを拡散板にて… | 0.0331 |
| `modern-500` | `modern-082` | 子どもは写真を見る。 | 子供は写真を見る。 | 0.0188 |
| `modern-500` | `modern-188` | 子どもは公園に徒歩で行く。 | 子供は公園に徒歩で行く。 | 0.0096 |
| `modern-500` | `modern-416` | 明日は雪が少ない。 | あしたは雪が少ない。 | 0.0281 |

candle 與 llama.cpp 對同一份 Q5_K 權重的解量化與 gemm 實作不同，logit 在
0.03 以內的差距就會翻轉 greedy 的 argmax；後續 token 的差距通常遠大於此
（例：`ajimee-174` 後續步驟的差距 0.9～12.7）。這也解釋了為什麼 `ajimee-200`
與 `modern-500` 各只差 1 題，而 `hard-cases` 完全一致。

## 延遲與記憶體

- 載入（mmap ＋ 解量化 embedding 與 norm）：98～106 ms。
- 每題延遲包含 prefill 與解碼，沒有 KV 前綴快取重用；每題都清空快取。
- 延遲在共租機器上量測，Rust 最終三組的 `loadavg` 是 26～27，重跑會變動。
  Python 對照跑在同一段共租期間（`loadavg` 19～22），`ajimee-200` p50 是
  156 ms、CPU p50 498 ms；原始報告在較乾淨的 window 量到 28.7 ms。這台機器
  目前無法重現 28.7 ms，所以跨 runtime 的延遲只能說「candle 沒有比
  llama.cpp 慢」，不能宣稱倍數。
- RSS：Rust 行程基礎 3.1～3.2 MB，載入後 120.2～120.3 MB，峰值
  126.0～127.3 MB；KV cache 的 `RawInterleavedKvCache` 每層固定配置 4096
  格，是主要增量之一。

## 重現

```sh
cargo run -p jc-cli --release --bin jinen-eval -- --model xsmall --corpus ajimee-200
cargo run -p jc-cli --release --bin jinen-eval -- --model xsmall --corpus modern-500
cargo run -p jc-cli --release --bin jinen-eval -- --model xsmall --corpus hard-cases
```

逐題結果寫到 `target/jinen-eval/jinen-xsmall-<corpus>.json`。`--trace <id>`
會額外印出每一步的 top logit、次高分與差距，用來檢查貪婪翻轉。其他參數：
`--limit`、`--threads`、`--context`、`--max-new-tokens`、`--gguf`、
`--tokenizer`、`--out`。

Python 對照用 `/tmp/opencode/jinen_ref.py`（工作用腳本，沒有進 repo），
prompt、NFKC、greedy 與計分都和 `scripts/eval_jinen_ajimee.py` 相同，
另外加上逐題 `cpu_ms`。

## 未完成與注意

- 沒有接進 `Session`，也沒有改任何預設行為；`jc-core` 未動。
- 本輪只跑 xsmall；`--model small` 的路徑已實作，但沒有下載 small 的
  `tokenizer.json` 與量測。
- 左文脈 prompt 已實作（尾端 64 字元），三組題庫都沒有餵文脈，所以
  「給文脈 +10pp」沒有在 Rust 版重測。
- `tokenizer.json` 不在唯讀的資料目錄，必須放在 worktree 的
  `models/external/jinen-v2-xsmall/` 或用環境變數指定；檔案不進 git。
- 模型與題庫授權：Jinen 是 CC-BY-SA-4.0（衍生與散布需同授權），
  AJIMEE-Bench 是 CC-BY-SA-3.0，只做本機評測。
