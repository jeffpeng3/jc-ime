# 日文 AI 輸入法與選字模型調查（2026-11-20）

## 摘要

結論先講：Zenzai 真正值得學的是「統計轉換當 draft、神經模型做 token 級驗證、失敗時加前綴約束重跑 Viterbi」這套系統。只接 zenz 權重做自由生成，不會自動得到官方 AJIMEE 的 80%。本專案 T03 spike 已經量過：`zenz-v3.1-small` free greedy 在 Anthy corpus.1 200 題只有 47.5%，低於 Anthy 自己的 80.0%，也低於 Zenzai 系統在 Anthy 全量 1,745 題的 67.6%。差別在約束與重搜，不在模型檔本身。

可直接試的模型是 **Jinen v2 xsmall / small**：35.7M / 109.5M、Qwen3 架構、GGUF 28MB / 81MB、CPU 4 threads p50 13ms / 48ms、AJIMEE NFKC 79.0% / 86.0%。**Zenz v3.2 small** 是第二順位：74MB Q5、GPT-2 91M 級、官方只給 GPU 建議，沒有公開 CPU 延遲與 v3.2 benchmark。**Zenz v2.5 medium** 310M / 619MB 只適合當品質上限參考，不適合放進 100ms CPU 路徑。

另外補查 reranker：目前查到真正針對 IME 候選的公開方案有兩個，`nakasyou/saikyo-ime-reranker` 與 Shizuku 的 GPT-2 judge。前者任務定義最接近我們，但 base 是 270M T5Gemma、Gemma 授權、程式 repo 沒有 LICENSE；後者是 41MB Q8 的 GPT-2 xsmall、模型 MIT、程式 GPL，最值得借它的 fusion 與 fallback。通用日文 reranker（hotchpotch 系列）不是 drop-in，必須在 IME 候選組上微調。

授權先講清楚：Zenzai 程式 MIT，權重 CC-BY-SA 4.0。CC 官方的 BY-SA 4.0 compatible license 清單只有 GPLv3，沒有 AGPLv3，所以不能說「CC-BY-SA 4.0 與 AGPL-3.0 相容」。未修改的權重可以當獨立散布單元，程式維持 AGPL，權重附 CC-BY-SA 4.0 全文與署名；本專案 `legal/README.md` 的 L-09 已經用這個模式散布 `ja-rerank`。量化、轉檔、微調會產生 Adapted Material，衍生權重必須按 CC-BY-SA 4.0 或 GPLv3 釋出，不能改標 AGPL。法律結論仍建議法務確認。

## 調查範圍與驗證方式

- 只查公開資料、HF metadata、GitHub 原始碼與論文。沒有下載完整權重，只對兩個 GGUF 抓前 2MB header 確認架構。
- 沒有改任何程式碼。本檔案是唯一新增產物。
- 模型大小取自 Hugging Face API 的 `siblings[].size`；授權取自 model card 的 `license` 欄位與 README。
- Mozc 的「沒有 neural 分支」用兩個方法確認：`git ls-remote --heads https://github.com/google/mozc.git` 只回 `master`；本地 `/mnt/work/jc/jc-ime-data/mozc-3.35.6354/src` 對 `neural|onnx|transformer|tensorflow` grep 無命中。
- Zenzai 推論流程以 `AzooKeyKanaKanjiConverter` 原始碼為準，不只看部落格。

## 候選總表

| 名稱 | 類型 | 授權 | 可用性 | 預期幫助 | 建議 |
|---|---|---|---|---|---|
| **Zenzai / AzooKeyKanaKanjiConverter** | 系統＋程式＋模型 | 程式 MIT；權重 CC-BY-SA 4.0 | 程式、權重、訓練資料、評測全部公開；CPU trait 有，官方建議 GPU | 同音詞與長文脈硬題提升大；整合成本高 | 借機制優先，模型第二 |
| **zenz-v3.2-small-gguf** | 模型 | CC-BY-SA 4.0 | 74MB Q5；GPT-2 small 級；GGUF | 比 v3.1 多右文脈與條件標籤；v3.2 無公開分數 | 先量 CPU 延遲再決定 |
| **zenz-v3.2-xsmall-gguf** | 模型 | CC-BY-SA 4.0 | 21MB Q5；參數量未標示 | 邊緣裝置用；品質未確認 | 只做 fallback 候選 |
| **zenz-v2.5-small / xsmall** | 模型 | CC-BY-SA 4.0 | 91M / 26M；181MB / 45MB safetensors | 官方 AJIMEE 80.0 / 66.5 | 可轉 GGUF 測試 |
| **zenz-v2.5-medium** | 模型 | CC-BY-SA 4.0 | 310M；619MB safetensors | 官方 AJIMEE 86.5，公開模型最高之一 | 只當上限，不進 CPU 路徑 |
| **Jinen v2 small / xsmall** | 模型 | CC-BY-SA 4.0 | 109.5M / 35.7M；Qwen3；GGUF 81MB / 28MB；CPU 48ms / 13ms p50 | AJIMEE 86.0 / 79.0 NFKC；可直接用 llama.cpp | **第一順位 spike** |
| **Genmai 310M / 90M** | 模型＋macOS IME | CC-BY-SA 4.0 | 1.24GB / 362MB fp32；Core ML 283MB / 82MB；Apple ANE | AJIMEE 93.0%（310M）；90M 幾乎同分 | macOS 參考，Linux/Windows 不直接可用 |
| **kanakanji-edge-33m** | 模型 | 權重宣告 Apache-2.0；tokenizer CC-BY-SA；訓練資料 CC-BY-SA/ODC-BY | 33.5M；Q8 36MB；AJIMEE 80.5；Anthy 70.5 | 很輕，CPU 可行 | 授權衝突未釐清前不要散布 |
| **duet-ja-kana-kanji-9m** | 模型 | CC-BY-SA 4.0 | 9M；ExecuTorch int8 12MB；AJIMEE 62.5 | 極低資源備援 | 只適合 edge 實驗 |
| **Saikyo IME reranker** | IME 專用 reranker | 模型 Gemma；程式 repo 無 LICENSE | T5Gemma 270M LoRA；adapter 5.9MB＋base 1.57GB；候選 pool 32；CPU fp32 | 任務定義最接近；授權與體積不宜 | 只借 listwise 訓練與 pool 設計 |
| **Shizuku judge** | IME 專用 reranker | 程式 GPL-3.0-or-later；模型 MIT | GPT-2 xsmall 6L/512H；Q8 約 41MB；context 32 tokens；3 threads | 最接近我們的 fusion/fallback | 借機制，不直接搬 macOS code |
| **hotchpotch Japanese reranker v2** | 通用日文 reranker | MIT | ModernBERT；ONNX int8 29.6MB / 37.4MB | 不是 IME 專用；zero-shot 不適用 | 可當 fine-tune base |
| **Karukan** | 系統＋Rust 程式 | MIT OR Apache-2.0 | Rust、llama.cpp、Jinen/zenz；CPU | 可直接借 inference 與 timeout/fallback 架構 | 借程式碼與架構 |
| **Akaza** | 系統＋Rust 程式 | MIT | 統計式 2-gram；Anthy 評測；無神經 | skip-bigram、length weight 等便宜特徵 | 借 rerank 特徵與評測 |
| **Mozc** | 系統＋程式＋詞典 | BSD-3-Clause＋NAIST/IPADIC/ICOT | 官方只有統計轉換；無 neural 分支 | 我們現有詞典與 baseline | 保留 |
| **ATOK** | 商用產品 | 專有 | 無模型、無程式、無技術細節 | 廠商宣稱深度學習降低誤轉換約 30% | 只借概念 |
| **Google 日本語入力** | 商用產品 | 專有 | 無神經技術揭露；Mozc OSS 是統計式 | 評測 baseline | 只借 baseline |
| **Apple 日本語入力** | 商用產品 | 專有 | 無官方技術文件 | 未確認 | 不建議投入 |
| **AJIMEE-Bench** | 評測集 | 資料 CC-BY-SA 3.0；程式 CC0-1.0 | 200 題；片假名輸入；可下載 | 同音詞硬題評估 | 立即用 |
| **Anthy corpus.1** | 評測集 | Public Domain（僅 corpus.1 有檔頭宣告） | 1,743 題；本專案已有 200 題 fixture | 一般文回歸 | 立即用 |
| **JWTD v2** | 評測／錯誤資料 | CC-BY-SA 3.0 | 約 70 萬句對；95MB | 錯誤分析、訓練來源 | 可選 |
| **zenz-v2.5-dataset** | 訓練資料 | CC-BY-SA 4.0＋ODC-BY 1.0 | 約 1.9 億對；36.5GB | 訓練小型轉換模型 | 目前不下載 |
| **KanaKanji-HardBench** | 評測集 | **未確認（repo 無 LICENSE）** | 400 題；27 系統分數公開 | 獨立壓力測試參考 | 只看分數，不散布 |
| **Sumibi benchmark** | 評測結果 | CC-BY-SA 3.0／CC0 | 628 題衍生集 | 與 LLM 比較 | 參考 |

## 1. Zenzai 深度調查

### 1.1 它是什麼

Zenzai 是 azooKey 的 neural かな漢字変換系統，核心模型叫 `zenz`。它不是單一模型檔，包含：

1. `AzooKeyKanaKanjiConverter`：Swift 寫的統計かな漢字変換引擎，負責 dictionary、lattice、Viterbi，並提供 Zenzai 的 draft。
2. `zenz`：GPT-2 架構的條件式語言模型，吃片假名讀音與文脈，輸出漢字かな交じり文。
3. `llama.cpp`：GGUF 推論 runtime。azooKey 對 llama.cpp 有 fork，但 upstream llama.cpp 也支援 GPT-2。
4. 投機式解碼與前綴約束：統計 draft 先出候選，zenz 做 token 驗證，失敗時把模型偏好的前綴變成約束，重跑 lattice。

主要來源：

- 官方文件：[Docs/zenzai.md](https://github.com/azooKey/AzooKeyKanaKanjiConverter/blob/main/Docs/zenzai.md)
- 論文：[ニューラルかな漢字変換システム Zenzai](https://www.anlp.jp/proceedings/annual_meeting/2025/pdf_dir/P1-19.pdf)，ANLP 2025 P1-19，CC BY 4.0
- 技術部落格：[ニューラルかな漢字変換エンジン「Zenzai」をazooKey on macOSに搭載します](https://zenn.dev/azookey/articles/ea15bacf81521e)
- 原始碼：[azooKey/AzooKeyKanaKanjiConverter](https://github.com/azooKey/AzooKeyKanaKanjiConverter)，MIT

### 1.2 權重在哪、大小、量化版本

| 模型 | 參數 | 檔案 | 大小 | 格式 | 授權 |
|---|---:|---|---:|---|---|
| `zenz-v1` | 90M | `ggml-model-Q8_0.gguf` | 102,293,152 bytes | GGUF Q8_0 | CC-BY-SA 4.0 |
| `zenz-v2.5-xsmall` | 26M | `model.safetensors` | 45,030,848 bytes | safetensors | CC-BY-SA 4.0 |
| `zenz-v2.5-small` | 91M | `model.safetensors` | 180,915,904 bytes | safetensors | CC-BY-SA 4.0 |
| `zenz-v2.5-medium` | 310M | `model.safetensors` | 619,038,328 bytes | safetensors | CC-BY-SA 4.0 |
| `zenz-v3.1-small` | 91M | `model.safetensors` | 180,915,904 bytes | safetensors | CC-BY-SA 4.0 |
| `zenz-v3.1-xsmall` | 26M | `model.safetensors` | 45,030,848 bytes | safetensors | CC-BY-SA 4.0 |
| `zenz-v3.2-small` | 未標示 | `ggml-model-Q5_K_M.gguf` | 73,871,936 bytes | GGUF Q5_K_M | CC-BY-SA 4.0 |
| `zenz-v3.2-xsmall` | 未標示 | `ggml-model-Q5_K_M.gguf` | 20,970,304 bytes | GGUF Q5_K_M | CC-BY-SA 4.0 |

來源：

- [zenz-v1](https://huggingface.co/Miwa-Keita/zenz-v1)
- [zenz-v2.5-small](https://huggingface.co/Miwa-Keita/zenz-v2.5-small)
- [zenz-v2.5-medium](https://huggingface.co/Miwa-Keita/zenz-v2.5-medium)
- [zenz-v2.5-xsmall](https://huggingface.co/Miwa-Keita/zenz-v2.5-xsmall)
- [zenz-v3.1-small](https://huggingface.co/Miwa-Keita/zenz-v3.1-small)
- [zenz-v3.2-small-gguf](https://huggingface.co/Miwa-Keita/zenz-v3.2-small-gguf)
- [zenz-v3.2-xsmall-gguf](https://huggingface.co/Miwa-Keita/zenz-v3.2-xsmall-gguf)

`zenz-v3.2-small` 的 model card 只有 `license: cc-by-sa-4.0`，沒有參數量。抓 GGUF 前 2MB header 得到 `general.architecture = gpt2`、`general.name = Gpt2 Small Japanese Char`。`zenz-v3.1-small` 的 `config.json` 是 12 層、768 hidden、12 heads、vocab 6000，約 91M。v3.2 是否完全同尺寸，**未確認**。

`zenz-v3.2-xsmall` 同樣沒有 model card 內容，大小 21MB，推估 26M 級，**未確認**。

### 1.3 推論流程：它吃什麼、輸出什麼

輸入：

- 讀音，片假名。使用者的平假名或羅馬字會先轉成片假名。
- 左文脈，v3 取尾端最多 40 字；v3.2 另外可加右文脈，取開頭最多 40 字。
- 條件標籤：`U+EE03` profile、`U+EE04` topic、`U+EE05` style、`U+EE06` preference。v2 的 user dictionary 條件只支援 v2 格式。

格式：

```text
v2: U+EE00<input_katakana>U+EE02<context>U+EE01<output></s>
v3: U+EE02<context>U+EE00<input_katakana>U+EE01<output></s>
v3.2: U+EE02<left_context>U+EE07<right_context>U+EE00<input_katakana>U+EE01<output></s>
```

輸出是漢字かな交じり文，遇到 EOS 停止。這些標籤與順序在 [Docs/zenzai.md](https://github.com/azooKey/AzooKeyKanaKanjiConverter/blob/main/Docs/zenzai.md) 與 `ZenzPromptBuilder.swift` 都有定義。

實際解碼不是單純 greedy：

1. 統計轉換器 `kana2lattice_all` 先產生 draft，一般模式 `N_best = 2`。
2. 取 draft 最佳候選，送進 `ZenzCandidateEvaluator`，對輸出 token 做 teacher forcing，逐 token 比對模型 argmax。
3. 全部 token 都符合，候選通過。
4. 遇到第一個不符合的 token，回傳 `fixRequired` 與新的前綴約束，可能是模型偏好的替代 token。
5. 用新約束重跑 `kana2lattice_all_with_prefix_constraint`，再驗證。這個 loop 由 `inferenceLimit` 限制次數。
6. `inferenceLimit == 0` 時直接回傳當下 draft，保證有結果。
7. rich candidate 模式會回傳多個 alternative constraint 與 probability ratio，讓上層產生更多候選。

原始碼位置：

- `Sources/KanaKanjiConverterModule/ConversionAlgorithms/Zenzai/zenzai.swift`
- `Sources/KanaKanjiConverterModule/ConversionAlgorithms/Zenzai/Zenz/ZenzCandidateEvaluator.swift`
- `Sources/KanaKanjiConverterModule/ConversionAlgorithms/Zenzai/Zenz/ZenzPromptBuilder.swift`
- `Sources/KanaKanjiConverterModule/ConversionAlgorithms/Zenzai/Zenz/ZenzContext.swift`

這個設計的關鍵是「輸出必須能被讀音解釋」。Zenzai 會把已固定的表面前綴從讀音中扣掉，只讓模型判斷剩下的部分；模型不能隨便把「おだのぶなり」改成「織田信長」後還通過，因為統計 lattice 與前綴約束會擋掉讀音不吻合的路徑。

### 1.4 模型大小與推論成本

官方說明：

- 「M1 以上的 macOS 環境が望ましいです。GPUを利用します。」
- 「モデルサイズに依存しますが、現状150MB程度のメモリを必要とします。」
- 「Linux環境・Windows環境でもCUDAを用いて動作します。」
- README 另外提供 `ZenzaiCPU` trait，設定 `n_gpu_layers = 0`，只留 CPU backend。

來源：[Docs/zenzai.md](https://github.com/azooKey/AzooKeyKanaKanjiConverter/blob/main/Docs/zenzai.md)、[README](https://github.com/azooKey/AzooKeyKanaKanjiConverter#readme)、`ZenzContext.swift`。

CPU 可行性：

- 程式面可行。`ZenzaiCPU` trait 明確把 GPU offload 關掉，thread 數由 `inferenceThreadCount` 依 CPU topology 決定。
- 官方沒有給 CPU 延遲數字。官方 50ms 說法出現在作者演講，對應 M 系列 GPU 的 on-device 推論，不是 CPU。
- 同等級小模型的 CPU 數字可以參考 Jinen：35.7M 的 xsmall 在 llama.cpp CPU 4 threads 下 p50 13ms、p90 29ms；109.5M 的 small p50 48ms、p90 111ms。Zenz v3.2 small 是 91M GPT-2，量級接近 Jinen small，但實際延遲**未確認**。

結論：Zenzai 在 CPU 上可跑，能不能進我們的 100ms 預算要看實測。以 91M 級模型推估，p50 有機會落在 50ms 附近，p90 可能超過 100ms。這是推估，**未確認**。

### 1.5 與 `jc-model` 的架構差異

| 面向 | `jc-model` 的 `ja-rerank` | Zenzai |
|---|---|---|
| 任務 | pointwise cross-encoder，對同一讀音的多個詞面打分 | 條件式語言模型，生成完整轉換字串，再做 token 驗證 |
| Runtime | ONNX Runtime，`ort` crate，`load-dynamic` | llama.cpp，GGUF |
| 模型大小 | int8 ONNX 34,268,510 bytes；fp32 135,613,420 bytes | v3.2 small Q5 73,871,936 bytes；xsmall 20,970,304 bytes |
| 輸入 | `left`、`candidate`、`right` 字串；最多 8 個候選 | 片假名讀音、左文脈、右文脈、profile/topic/style 標籤 |
| 輸出 | 每個候選一個 `f32` 分數 | token 序列；`pass` / `fixRequired` / `wholeResult`；可產生多候選 |
| 文脈 | 雙側各 60 tokens；目前右文只留在介面，沒進張量 | 左文 40 字尾端、右文 40 字開頭；v3.2 支援右文 |
| 讀音忠實度 | 候選本身來自詞典，天然有讀音 | 生成式，需靠 draft、前綴約束與讀音檢查 |
| 延遲策略 | 100ms 預算，逾時保留統計排序 | `inferenceLimit` 控制模型呼叫次數；GPU 建議；CPU trait |
| 與詞典關係 | 需要我們的 lattice 與候選 | 模型本身不需要詞典；Zenzai 系統用統計轉換器當 draft |
| 授權 | 程式 AGPL-3.0-only；模型 CC-BY-SA 4.0 | 程式 MIT；權重 CC-BY-SA 4.0 |

本專案相關程式：

- `crates/jc-model/src/rerank.rs`：`MAX_CANDIDATES = 8`、`MAX_TIMEOUT = 100ms`、`score_batch`。
- `crates/jc-core/src/ja/rerank.rs`：`CONTEXT_TOKENS_PER_SIDE = 60`、`RERANK_TIMEOUT = 100ms`、`WORD_RERANK_MAX_CANDIDATES = 8`；融合公式 `NEURAL * raw + STATS * stats + DRAFT`。
- `crates/jc-core/src/ja/decode.rs`：`Decoder::nbest`、`JaReranker::score_words`、`JaEntry { surface, reading }`。

主要差異是：`jc-model` 是判別式重排，輸出單一分數；Zenzai 是生成式驗證，輸出 token 級決策與約束。把 Zenzai 當成「比較強的 reranker」會漏掉它最重要的約束重搜。

### 1.6 授權：能否商用、散布、AGPL 相容

| 項目 | 授權 | 商用 | 散布 | 與 AGPL-3.0 的關係 |
|---|---|---|---|---|
| `AzooKeyKanaKanjiConverter` 程式 | MIT | 可 | 可，附 MIT | MIT 可併入 AGPL 專案 |
| `zenz` 權重 | CC-BY-SA 4.0 | 可 | 可，附署名與 CC-BY-SA 4.0 全文；衍生作品同樣 CC-BY-SA 4.0 | **不是 AGPL-compatible**；CC 官方清單只列 GPLv3 |
| `zenz-v2.5-dataset` | CC-BY-SA 4.0；llm-jp 部分 ODC-BY 1.0 | 可 | 可，附對應授權 | 與程式分開處理 |
| `zenz` 基底模型 `ku-nlp/gpt2-small-japanese-char` | CC-BY-SA 4.0 | 可 | 可 | 同上 |

CC 官方 compatible licenses 頁面明確寫：「GPLv3: The GNU General Public License version 3 was declared a BY-SA-Compatible License for version 4.0」。清單沒有 AGPLv3。來源：[Creative Commons Compatible Licenses](https://creativecommons.org/share-your-work/licensing-considerations/compatible-licenses/)。

實務判斷：

1. 未修改的 GGUF 當獨立下載項，程式碼維持 AGPL，權重單獨附 CC-BY-SA 4.0。這與本專案 `legal/README.md` 的 L-09 `ja-rerank` 做法一致。
2. 若我們量化、轉檔、微調，成果是 Adapted Material，必須用 CC-BY-SA 4.0 或 GPLv3 釋出。不能把權重改標成 AGPL-3.0-only。
3. 模型輸出是否受訓練資料 ShareAlike 影響，法律上沒有定論，**未確認**。不要把「模型可商用」寫成「模型輸出可任意授權」。
4. 以上不是法律意見。

## 2. 其他公開神經かな漢字変換模型

### 2.1 Jinen v2：目前最適合直接 spike 的 CPU 模型

| 項目 | small | xsmall |
|---|---:|---:|
| 參數 | 109.5M | 35.7M |
| 架構 | Qwen3（GGUF metadata `general.architecture = qwen3`） | Qwen3 |
| 量化檔 | Q5_K_M 81,117,824 bytes | Q5_K_M 28,261,056 bytes |
| Q8_0 | 117,197,472 bytes | 38,664,224 bytes |
| AJIMEE Acc@1（Q5） | 80.0% exact / 86.0% NFKC | 74.0% exact / 79.0% NFKC |
| CPU p50 / p90 / p99 | 48 / 111 / 171 ms | 13 / 29 / 45 ms |
| 條件 | llama.cpp b10200，CPU 4 threads，n_ctx 1024，左文脈 64 字 | 同上 |

來源：[jinen-v2-small.gguf](https://huggingface.co/togatogah/jinen-v2-small.gguf)、[jinen-v2-xsmall.gguf](https://huggingface.co/togatogah/jinen-v2-xsmall.gguf)。

格式：

```text
U+EE02<left_context>U+EE00<katakana_reading>U+EE01<output></s>
```

必要條件：

- 輸入必須先做 NFKC。模型卡明寫「NFKC 前提」，沒做 NFKC 會掉約 6 個百分點。
- 必須 greedy，`temperature 0`、`top_k 1`。
- 不支援 zenz-v3.2 的 `U+EE07` 右文脈。
- 沒有 draft 約束，屬自由生成。讀音忠實度要靠外部檢查。

優點：

- 已經有 Rust 系統 Karukan 直接使用，Apache-2.0 程式可以借。
- CPU 4 threads 的 small p50 48ms，xsmall p50 13ms，符合我們的 100ms 預算。
- AJIMEE 86.0% NFKC，與 zenz-v2.5-small 的 80.0% 同級或更好。

風險：

- model card 只列 NDL 書誌資料，完整訓練資料組成**未確認**。
- 109.5M 的 small p90 111ms，已超過我們的 100ms；若要穩定，xsmall 或先做 timeout/fallback。
- 自由生成可能出現讀音不忠實的輸出，需加讀音檢查。

### 2.2 Genmai 310M / 90M：Apple ANE 路線的高標

Genmai 是 macOS IME，模型以 Core ML 跑在 Apple Neural Engine。

| 項目 | genmai-310M | genmai-90M |
|---|---:|---:|
| 參數 | 310M | 90M |
| 架構 | GPT-2，24 層、1024 hidden | GPT-2，12 層、768 hidden |
| fp32 權重 | 1,238,046,208 bytes | 361,816,704 bytes |
| Core ML int8 | 282,957,440 bytes | 82,239,444 bytes |
| AJIMEE | 93.0% | 與 310M 幾乎相同（199/200 題 top-1 相同） |
| 一般句 / 難句 | 94.1% / 78.6% | 與 310M 相同（101/101、27/28） |
| 速度 | 整句 median 113ms；轉換鍵 median 30 / 39ms（一般 / AJIMEE） | 約 4ms/decode step；process footprint 50–90MB |
| 平台 | M4 Pro ANE | M4 Pro ANE |

來源：[genmai-310M](https://huggingface.co/subdiox/genmai-310M)、[genmai-90M](https://huggingface.co/subdiox/genmai-90M)、[genmai-ime](https://github.com/subdiox/genmai-ime)。

對我們的意義：

- 90M 的品質與 310M 幾乎相同，證明 distillation 很有效。
- 但 Core ML / ANE 是 Apple 專屬，不能直接放進 Linux/Windows 的 `jc-model`。
- 這是「小模型也能到 93% AJIMEE」的存在證明，值得注意。
- 授權標示有衝突：`genmai-90M` 卡片說 base `zenz-v3.2-small` 是 Apache-2.0，但官方 zenz-v3.2 卡片是 CC-BY-SA 4.0。以官方為準，應視為 CC-BY-SA 4.0。這個衝突**未確認**。

### 2.3 kanakanji-edge-33m：很輕，但授權要先釐清

| 項目 | 值 |
|---|---|
| 參數 | 33.5M |
| 架構 | LlamaForCausalLM，8 層、512 hidden、GQA |
| 檔案 | Q8 35,806,592 bytes；f16 67,196,192 bytes |
| AJIMEE | 80.5% / CER 2.55 |
| Anthy corpus.1 | 70.5% / CER 5.19 |
| 訓練 | 從零訓練，zenz-v2.5-dataset 6 epochs |
| 授權 | 權重宣告 Apache-2.0；tokenizer CC-BY-SA 4.0；訓練資料 CC-BY-SA 4.0＋ODC-BY |

來源：[kanakanji-edge-33m](https://huggingface.co/yuuki14202028/kanakanji-edge-33m)。

這是目前看到最輕、品質又可用的公開模型之一。33.5M、36MB Q8、AJIMEE 80.5%，比 zenz-v2.5-small 的 91M 更小。但作者把權重標成 Apache-2.0，訓練資料含 CC-BY-SA 4.0。ShareAlike 是否傳染到權重，**未確認**。在釐清前不要放進散布包，只能做內部 spike。

### 2.4 duet-ja-kana-kanji-9m：極低資源備援

| 項目 | 值 |
|---|---|
| 參數 | 9M，GPT-2 風格，4 層、384 hidden |
| 檔案 | ExecuTorch int8 12,281,664 bytes；fp32 37,922,567 bytes |
| AJIMEE | 62.5% / CER 6.44 |
| Bluesky held-out 5,000 | 64.9% / CER 6.24 |
| 授權 | CC-BY-SA 4.0 |

來源：[duet-ja-kana-kanji-9m](https://huggingface.co/jcbutter/duet-ja-kana-kanji-9m)。

只適合當最低資源 fallback。品質低於 33M 的 kanakanji-edge，但檔案更小、授權較清楚。

### 2.5 Zenz 衍生模型

- `genmai-90M` 從 `zenz-v3.2-small` dequantize 後 distillation。
- `kanakanji-edge-33m` 從零訓練在 `zenz-v2.5-dataset`。
- `duet-ja-kana-kanji-9m` 從 `zenz-v3.2-small` distillation。
- 其他 HF 上的 `zenz-*` fine-tune 很多，品質與授權參差不齊，沒有逐一驗證。

這些衍生模型說明一件事：zenz 的 1.9 億句對資料集比模型架構更有價值。與其追最新權重，不如先確認我們能不能合法使用這份資料訓練自己的小模型。

## 3. 商用產品與 Mozc

### 3.1 ATOK

官方頁面明確寫：

- 「人工知能（AI）のディープラーニング技術を採用した、変換エンジン「ATOKディープコアエンジン」と入力ミスをなかったことにしてくれる「ATOKディープコレクト」。」
- 「「ATOK ディープコアエンジン」の搭載により、誤変換が約30％削減。」
- 深度學習用在建立引擎，不是使用者端即時訓練。這是廠商說法，沒有公開模型、程式或訓練資料。

來源：[高精度な変換エンジン | ATOK Passport](https://www.atok.com/info/features/engine.html)。

可驗證的說法只有「ATOK 有使用 deep learning 技術」與「廠商自稱誤轉換約 30% 降幅」。模型大小、推論方式、是否 on-device 神經推論，**未確認**。

### 3.2 Google 日本語入力

官方產品頁只說「Web で使われている膨大な用語から辞書を作成」「辞書は定期的に自動更新」，沒有揭露 neural 轉換。來源：[Google 日本語入力](https://www.google.co.jp/ime/)。

Mozc OSS 是 Google 日本語輸入的開源子集。Zenzai 論文背景段落說 Mozc 使用 class 2-gram 與 word/reading unigram。Google 日本語輸入本體是否另外使用 neural 模型，沒有官方技術文件，**未確認**。

### 3.3 Apple 日本語入力

沒有找到 Apple 官方技術文件說明 iOS/macOS 日本語輸入使用 neural かな漢字変換。azooKey 的 Zenn 文章只寫「Apple はニューラル言語モデルを日本語入力に活用しているという噂があります」。這是傳聞，不是可驗證事實。**未確認**。

### 3.4 Mozc 官方有沒有 neural 實驗分支

沒有。

驗證：

```sh
git ls-remote --heads https://github.com/google/mozc.git
# 只回 master
```

本地 `/mnt/work/jc/jc-ime-data/mozc-3.35.6354/src` 對 `neural|onnx|transformer|tensorflow` grep 無命中。這不代表 Google 內部沒有，只代表公開 Mozc 沒有。Google 日本語輸入本體是否 neural，**未確認**。

Mozc 授權是 BSD-3-Clause，詞典另外含 NAIST-IPAdic、ICOT、沖繩辭書條款；本專案 `legal/README.md` L-01 已經記錄。商用與散布可行，需保留對應 notice。

## 4. 公開評測資料集

### 4.1 AJIMEE-Bench

| 項目 | 值 |
|---|---|
| 題數 | 200 |
| 輸入 | 片假名讀音 |
| 輸出 | `expected_output` 允許多個答案 |
| 文脈 | 100 題無文脈，100 題有左文脈 |
| 來源 | JWTD v2 測試資料中 `kanji-conversion_a` 類型抽 200 題 |
| 資料授權 | CC-BY-SA 3.0 |
| 程式授權 | CC0-1.0 |
| 位置 | [azooKey/AJIMEE-Bench](https://github.com/azooKey/AJIMEE-Bench) |

本專案已經有 fixture：`/mnt/work/jc/jc-ime-data/0c-testdata/ja/ajimee_200.json`，commit `401666cd`。

優點：針對同音詞與誤轉換，與我們要改善的痛點直接相關。

限制：只有 200 題，且是從錯誤資料抽樣，難度偏高。不能拿這個分數宣稱一般文品質。

### 4.2 Anthy corpus.1

| 項目 | 值 |
|---|---|
| 題數 | 1,743（本專案 fixture 抽 200） |
| 格式 | `｜読み｜...｜ ｜正解｜...｜` 的文節對齊 |
| 授權 | Public Domain，僅 `corpus.1.txt` 檔頭有宣告 |
| 來源 | [Anthy corpus](https://github.com/netsphere-labs/anthy/tree/master/corpus) |

本專案 `0c-testdata/ja/README.md` 明確寫：只讀 `corpus.1.txt`；`corpus.2.txt`、`corpus.3.txt`、`corpus.5.txt` 沒有授權聲明，`corpus.4.txt` 是錯誤示例，四個檔案都沒有進 fixture。

Akaza 文件說整個 anthy コーパス是 public domain，但本專案只確認 `corpus.1.txt`。若要擴充題庫，先確認其他檔的授權，**未確認**。

### 4.3 JWTD v2（日本語Wikipedia入力誤りデータセット）

| 項目 | 值 |
|---|---|
| 規模 | 約 70 萬句對 |
| 大小 | 95MB |
| 類別 | 誤字、脫字、衍字、轉字、漢字誤轉換 |
| 授權 | CC-BY-SA 3.0 |
| 來源 | [京都大学 NLP](https://nlp.ist.i.kyoto-u.ac.jp/?%E6%97%A5%E6%9C%AC%E8%AA%9EWikipedia%E5%85%A5%E5%8A%9B%E8%AA%A4%E3%82%8A%E3%83%87%E3%83%BC%E3%82%BF%E3%82%BB%E3%83%83%E3%83%88) |

用途：錯誤分析、產生訓練對、抽 hard cases。它是 AJIMEE 的來源，但本身不是轉換題庫，需再做讀音推定與人工檢查。

### 4.4 zenz-v2.5-dataset

| 項目 | 值 |
|---|---|
| 規模 | 約 1.9 億對「左文脈、輸入、輸出」 |
| 大小 | 36,538,797,184 bytes（約 36.5GB） |
| 檔案 | `train_llm-jp-corpus-v3.jsonl` 32.4GB；`train_wikipedia.jsonl` 4.1GB |
| 授權 | 大部分 CC-BY-SA 4.0；llm-jp 部分 ODC-BY 1.0 |
| 來源 | [Miwa-Keita/zenz-v2.5-dataset](https://huggingface.co/datasets/Miwa-Keita/zenz-v2.5-dataset) |

這是目前最完整的公開日文かな漢字轉換訓練資料。授權乾淨程度比模型權重好理解，但 36.5GB 對目前任務太大。若要訓練自己的 30M 到 90M 模型，這是首選資料來源。注意 Wikipedia 與 Common Crawl 的 ShareAlike/ODC-BY 條款。

### 4.5 KanaKanji-HardBench

| 項目 | 值 |
|---|---|
| 題數 | 400 |
| 內容 | 文脈同音詞、固定搭配、專業術語、複合詞邊界、慣用句、動詞表記、文節分割 |
| 系統數 | 27 |
| 授權 | **未確認，repo 沒有 LICENSE 檔** |
| 來源 | [fangquinlan/KanaKanji-HardBench](https://github.com/fangquinlan/KanaKanji-HardBench) |

這份 benchmark 很有參考價值，因為它同時測 Zenz、Jinen、Google 日本語入力，且公開逐項預測。但它沒有 LICENSE，資料不能假定可散布，也不能放進產品 gate。只看分數與方法。

### 4.6 Sumibi benchmark

| 項目 | 值 |
|---|---|
| 題數 | 628（舊版 200） |
| 來源 | AJIMEE/JWTD 衍生，加上自製例文 |
| 授權 | Wikipedia/JWTD 部分 CC-BY-SA 3.0；創作例文與程式 CC0-1.0 |
| 來源 | [Sumibi benchmark](https://github.com/kiyoka/Sumibi/blob/main/benchmark/README.md) |

適合拿來比較 LLM prompt 方法與本地模型，不是乾淨的產品題庫。

### 4.7 其他

- BCCWJ：ACL 2024 的 encoder-decoder 論文用它訓練與評估。BCCWJ 是授權語料，不能直接散布，**條款未確認**。
- Joyo Kanji Yomi Benchmark：只測漢字讀音，不是かな漢字轉換，不適合當主要指標。
- Mozc 內建測試：偏向功能與回歸，不是公開轉換 benchmark。

## 5. 公開數字怎麼看

### 5.1 AJIMEE-Bench

| 系統 | Acc@1 | CER | 備註 |
|---|---:|---:|---|
| Google 日本語入力 | 54.0 | 6.7 | 封閉產品 |
| Zenzai draft（統計轉換） | 44.5 | 7.6 | 同一篇論文 |
| Zenzai xsmall | 66.5 | 4.6 | 26M |
| Zenzai small | 80.0 | 2.6 | 91M |
| Zenzai medium | 86.5 | 1.7 | 310M |
| Jinen v2 xsmall Q5 | 79.0 | 未標 | NFKC |
| Jinen v2 small Q5 | 86.0 | 未標 | NFKC |
| Genmai 310M | 93.0 | 未標 | Apple ANE，5 beams |
| kanakanji-edge-33m | 80.5 | 2.55 | 33.5M |
| duet-ja-kana-kanji-9m | 62.5 | 6.44 | 9M |

Zenzai 數字來源：[ANLP 2025 P1-19](https://www.anlp.jp/proceedings/annual_meeting/2025/pdf_dir/P1-19.pdf)。Jinen 來源：model card。Genmai 來源：model card。kanakanji 與 duet 來源：各自的 model card。

這些數字不能直接橫向比較，因為：

- 有些用 exact match，有些先做 NFKC。
- 有些用 greedy，有些用 beam search。
- 有些有左文脈，有些沒有。
- AJIMEE 只有 200 題，抽樣誤差大。

### 5.2 Anthy corpus.1

| 系統 | top1 | 備註 |
|---|---:|---|
| Anthy 9100h | 80.0% | 在自己的訓練語料上，明顯偏高 |
| zenz-v3.1-small Q5，free greedy | 47.5% | 本專案 T03 spike，200 題，無 Zenzai 約束重搜 |
| Zenzai v1（5M 訓練） | 67.6% | 全量 1,745 題，1,179/1,745 |
| Google 日本語入力 CGI | 63.3% | 全量 1,745 題，1,104/1,745 |
| AzooKeyKanaKanjiConverter baseline | 61.0% | 全量 1,745 題，1,065/1,745 |
| kanakanji-edge-33m | 70.5% | 全量 1,745 題 |

來源：

- T03 數字：`/mnt/work/jc/jc-ime-data/0c-testdata/ja/README.md`。
- Zenzai v1、Google、baseline：Zenn 文章「zenz-v1の性能評価」。
- kanakanji：model card。

這張表最重要的訊息：**free greedy 的 zenz 在一般文上不一定贏統計轉換**。Zenzai 系統的 67.6% 來自有 draft 約束的完整系統；T03 的 47.5% 是拔掉約束後的結果。兩者不能直接比較，但足以說明「只接模型」的風險。

### 5.3 KanaKanji-HardBench

| 系統 | AFH-400 |
|---|---:|
| Gemini 3.1 Pro (High) | 96.0% |
| Zenz v2.5 medium | 54.0% |
| Jinen v2 small | 45.0% |
| Zenz v3.1 small | 43.0% |
| Zenz v3.2 small | 38.0% |
| Zenz v3.2 xsmall | 30.8% |
| Google 日本語入力 3.34.6260.0 | 29.5% |

來源：[KanaKanji-HardBench leaderboard](https://github.com/fangquinlan/KanaKanji-HardBench)。

這份 benchmark 顯示：

- Zenz v3.2 small 在硬題上低於 v3.1 small 與 v2.5 medium。新版不一定全面更好。
- Jinen v2 small 45.0%，高於 Zenz v3.2 small 的 38.0%，與 AJIMEE 的排序不完全一致。
- Google 日本語入力只有 29.5%，與 AJIMEE 的 54.0% 差距很大。Benchmark 分佈主導結果。

它沒有評 Zenz v3.2 的 `U+EE07` 右文脈，所以不能拿這份分數否定 v3.2 的右文脈功能。

## 6. 可借鏡的機制，對應到 `ja/decode` 與 `ja/rerank`

### 6.1 統計 draft ＋ 神經 token 驗證 ＋ 前綴約束重搜

這是最值得照抄的部分。

我們現有的路徑：

1. `Decoder::nbest` 產生 N-best。
2. `NeuralReranker::score_words` 對同一讀音的多個詞面打分。
3. `fuse_word_scores` 把神經分數、統計分數、draft 加成融合。
4. 超過 100ms 就整批放棄，保留統計排序。

Zenzai 的路徑：

1. 統計轉換器產生 draft。
2. 神經模型對 draft 的輸出 token 做 teacher forcing。
3. 第一個不符合的 token 變成前綴約束。
4. 用約束重跑 Viterbi，再驗證。
5. 限制模型呼叫次數。

對我們的落點：

- `crates/jc-core/src/ja/decode.rs` 已經有 `Decoder::nbest` 與 `Locks`，可加上「prefix constraint」參數，讓 neural 驗證結果反過來限制 lattice。
- `crates/jc-core/src/ja/rerank.rs` 的 `JaReranker` 目前只回 `Option<Vec<f32>>`。要借這個機制，需要新的回傳型別，例如 `Pass { score }`、`FixRequired { prefix }`、`WholeResult { text }`。
- `crates/jc-model/src/rerank.rs` 的 `MAX_CANDIDATES = 8` 與 100ms timeout 可以保留，但要多一個「單一候選 token 驗證」API，不只是一批 scalar 分數。

工作量：中等。主要難點是 tokenizer 對齊、讀音 byte/ruby 計數、以及 timeout 下的 fallback。

### 6.2 讀音忠實度檢查

Zenzai 用統計 lattice 與前綴約束保證輸出可被讀音解釋。我們可以用更便宜的方法：

1. 神經模型產生候選字串。
2. 用 `Decoder::audit_target` 檢查 `surface_segmentable` 與 `reading_reachable`。
3. 不通過就丟棄，保留統計候選。

`crates/jc-core/src/decode/nbest.rs` 的 `AuditTarget` 已經有這兩個欄位，`jc-eval` 的錯誤分層也在用。這是最低風險、最高報酬的借鏡點，不需要新 runtime。

### 6.3 Token 級分數，取代單句 scalar

目前 `jc-model` 對一個候選只回一個 `f32`。Zenzai 的 `ZenzCandidateEvaluator` 會逐 token 算 logprob，遇到 mismatch 還能回傳替代 token 與 probability ratio。這種訊號比單一分數有用，因為：

- 可以知道錯誤發生在第幾個 token。
- 可以產生「只改這一段」的約束。
- 可以做 candidate 的多樣性控制。

對我們的落點：`JaReranker` 增加一個 optional 的 token-level API，現有 scalar 路徑不動。這樣不會破壞現有 reranker。

### 6.4 右文脈與條件標籤

Zenz v3.2 支援 `U+EE07` 右文脈，以及 profile/topic/style/preference。我們目前 `ja/rerank` 的 `right` 只留在介面，沒有進張量。`jc-model` 的 `score` 雖然有 `right` 參數，但实际張量只用到 `left` 與 `candidate`。若要借右文脈：

- 先確認我們的輸入流程有沒有可靠的右文。`ja/decode` 的 `JaContext` 有 `right` 欄位。
- 在 `NeuralReranker` 把右文 token 餵進模型，序列長度仍是 124，需重新訓練或調整 layout。
- 右文對「はし」這種同音詞有幫助，但對即時輸入，右文通常不完整，收益有限。

### 6.5 逐次輸入的增量快取

Zenzai 用 `ZenzaiCache` 保存上一次的 lattice、constraint、satisfying candidate，逐字輸入時只做差分更新。還有一個 `defersEvaluationForPendingInput`，羅馬字尾端還沒確定時先不評估。

對我們的落點：

- `crates/jc-core/src/ja/live.rs` 與 `session.rs` 已經有 live conversion 的狀態機。
- 可以借「輸入與 constraint 當 cache key，重用上一次 lattice」的做法，降低長句每次重算的成本。
- 對我們的 100ms 預算，這比換更大的模型更有效。

### 6.6 訓練資料與 loss

Zenzai 的訓練資料 construction：

- 從 Wikipedia 與 llm-jp-corpus-v3 取文字。
- 用 MeCab + ipadic-NEologd 推定讀音。
- 產生「左文脈、片假名輸入、漢字かな輸出」三元組。
- 只對輸出 token 算 cross entropy，輸入 token 不算 loss。
- 加入讀音搖晃（ニホン/ニッポン）增加輸入多樣性。

我們可以：

- 用 `zenz-v2.5-dataset` 或自己的 public-core 詞典 + Wikipedia 產生資料。
- 訓練 30M 到 90M 的模型，目標是 CPU 推論。
- loss 只算輸出，讓模型專注在轉換。

這是高工作量選項，但比直接接 310M 模型更可控。

### 6.7 類似 reranker 的公開方案（補充）

先分三類，避免把不同任務混在一起：

1. **IME 專用候選 reranker**：輸入是「讀音、左文脈、候選表記」，輸出是同組候選的分數或排序。這一類和我們的 `ja/rerank` 最接近。
2. **生成式 verifier**：用語言模型對完整候選算 log-likelihood，當 reranker 用。Zenzai 的 candidate evaluator 與 Shizuku 的 judge 都屬於這一類。
3. **通用日文 reranker**：query/passage cross-encoder，訓練目標是檢索相關度。這一類要在 IME 候選組上微調，不能直接拿來用。

| 名稱 | 類型 | 輸入／輸出 | 模型與大小 | 授權 | 可用性 | 對我們的意義 |
|---|---|---|---|---|---|---|
| **Saikyo IME reranker** | IME 專用，listwise | `<ctx> <input> <output> <score>`；同組候選比較 | T5Gemma 270M，LoRA r16；adapter 5.9MB；base 1.57GB | 模型 Gemma；程式 repo 無 LICENSE；資料逐列授權 | 任務最接近；CPU fp32，CUDA 4-bit；100ms debounce；有 fallback | 借 listwise loss 與候選 pool 32 |
| **Shizuku judge** | IME 專用，generative verifier | 左文脈＋完整候選；輸出 log-likelihood | GPT-2 xsmall 6 層、512 hidden；Q8 約 41MB | 程式 GPL-3.0-or-later；模型 MIT | macOS 實作；3 threads、context 32 tokens、max 200 tokens | 最接近我們的 fusion/fallback |
| **Zenzai candidate evaluator** | IME 專用，token verifier | 讀音＋左文脈＋候選；輸出 pass/fixRequired/wholeResult | zenz 26M 到 310M；GGUF | 程式 MIT；權重 CC-BY-SA 4.0 | 需要 llama.cpp；不是獨立 reranker | 借 token 級驗證與前綴約束 |
| **Karukan NllScorer** | 模型 NLL scorer | `reading_katakana`＋`surface`；輸出 per-character NLL | Jinen/zenz；GGUF | 程式 MIT OR Apache-2.0 | 目前用於離線詞典建置，不是 runtime | 可改成 runtime 候選 scorer |
| **Roman BERT PLL** | 通用 BERT，pseudo-likelihood | 左文脈＋候選；輸出分數 | `bert-base-japanese-v3` | 未確認；只有部落格 | 25 個候選 79ms；無公開程式 | 借「有左文才打分」的規則 |
| **Akaza feature reranker** | 統計特徵 | k-best 路徑；輸出重排 | 無神經；MARISA 模型 | MIT | 已實作、可調權重 | 便宜的 baseline 與特徵 |
| **hotchpotch tiny / xsmall v2** | 通用日文 cross-encoder | query＋passage；輸出相關度 | ModernBERT；ONNX int8 29.6MB / 37.4MB | MIT | 有 ONNX，可在 CPU 跑 | 可當 IME 微調 base，不能 zero-shot |
| **Ruri v3 reranker 310M** | 通用日文 cross-encoder | query＋passage；輸出相關度 | ModernBERT 310M；1.26GB | Apache-2.0 | 品質高，體積大 | 品質上限參考 |

#### Saikyo IME reranker

這是目前找到最接近我們任務定義的公開 reranker。它的輸入格式是：

```text
<ctx> 左文脈 <input> 讀音 <output> 候選表記 <score>
```

同一組候選一起打分，loss 是 `listwise_cross_entropy + 0.2 * binary_cross_entropy`，候選 pool 32，CLI 有 `--candidate-pool 32`。程式是 Python，CUDA 預設 4-bit，CPU 用 fp32；`--realtime` 預設 100ms debounce，神經推論失敗時回退 classical 排序，並記錄 `degraded_reason`。

來源：

- 模型卡：[nakasyou/saikyo-ime-reranker](https://huggingface.co/nakasyou/saikyo-ime-reranker)
- 程式：[nakasyou/reranker-ime](https://github.com/nakasyou/reranker-ime)
- 訓練資料：[nakasyou/japanese-conversion](https://huggingface.co/datasets/nakasyou/japanese-conversion)

數字：

- 一般 eval top-1 0.8516、MRR 0.9104、pairwise 0.9703。
- ambiguity eval top-1 0.6719、MRR 0.7968、pairwise 0.9264。
- 兩組各 256 個 candidate group，來自訓練 run 的保存指標，不是獨立 benchmark。

限制：

- base 是 `google/t5gemma-2-270m-270m`，1.57GB；adapter 5.9MB。CPU fp32 的延遲沒有公開，270M 對我們的 100ms 預算偏重。
- 模型授權是 Gemma，不是 OSI 授權；與 AGPL 的相容性未確認。商用需遵守 Gemma Terms。
- 程式 repo 沒有 LICENSE 檔，GitHub API 的 license 是 `null`。不能直接搬程式碼。
- 資料集 17.9GB、346 個 parquet、約 25.6M 列；沒有全域 license 欄位，逐列有 `from_license`，例如 `cc-by-4.0`。散布或再訓練前要逐來源確認。
- 訓練 manifest 沒有釘住 dataset revision，可重現性有限。

對我們的價值：**listwise loss、候選 pool 32、context+reading+candidate 的輸入格式、fallback 設計**。這些可以照抄，不需要用它的權重。

#### Shizuku judge

這是目前找到最適合我們預算的 IME reranker 實作。它是 macOS IME，轉換引擎是 AzooKeyKanaKanjiConverter 的 Rust 移植，後面接一個 41MB 的 GPT-2 xsmall judge。

作法：

- 模型：`rinna/japanese-gpt2-xsmall`，6 層、512 hidden、8 heads，權重轉成 Q8_0 GGUF，約 41MB。
- 輸入：左文脈最多 32 tokens，候選最多 200 tokens。文脈放在候選前面，讓分數等於 `P(candidate | context)`。
- 分數：`engine_score + 1.0 * judge_log_likelihood`。engine 的統計分數原樣保留，神經分數只是加分。
- 未知 token 處理：候選含未知 token 時，把它的分數壓到不含未知 token 候選的最小值，避免罕見漢字被未知 token 的高機率推上去。
- fallback：模型載入中、載入失敗、推論失敗、panic，全部回傳 `None`，保留引擎排序。載入在背景執行，不阻塞輸入。
- 執行緒：3 threads，避免 IME 佔滿 CPU。
- 快取：用前綴樹重用在相同文脈下的 KV，逐字輸入時只算新增 token。

來源：

- [n-seiji/shizuku-ime](https://github.com/n-seiji/shizuku-ime)
- [rinna/japanese-gpt2-xsmall](https://huggingface.co/rinna/japanese-gpt2-xsmall)
- `src/judge.rs` 的 `score`、`rerank`、`floor_unknown`、`order_by_total`

限制：

- 程式是 GPL-3.0-or-later，且用 macOS Accelerate 做 sgemm；直接搬到 Linux/Windows 要換 BLAS 後端。GPL 與 AGPL 的相容性比 Gemma 清楚，但仍建議法務確認。
- 模型 MIT，權重乾淨。
- repo 沒有公開 judge 的 top-1 或延遲數字，只有 `--judge-bench` 工具。實際增益**未確認**。
- 它用 generative log-likelihood，和我們的 cross-encoder 不同。我們的 `jc-model` 是同讀音多詞面的 pointwise scorer；Shizuku 是完整句子的 LM 分數。兩者可以互補，不能直接替換。

對我們的價值：**fusion 公式、未知 token floor、背景載入、失敗回退、3 threads、KV 前綴快取**。這些都是低成本、可直接對應到 `ja/rerank` 的設計。

#### Zenzai candidate evaluator

Zenzai 的 candidate evaluator 不是獨立 reranker，但它是最強的 verifier。它對候選做 teacher forcing，逐 token 比對模型 argmax，回傳 `pass`、`fixRequired`、`wholeResult`。這比單一分數多了「錯在第幾個 token」與「下一個約束是什麼」。

如果我們只想要 reranker，可以先用它的 `pass` 分數與 token logprob，不必實作完整的前綴重搜。這樣工作量比整套 Zenzai 小，但仍需要 llama.cpp。

#### Karukan NllScorer

Karukan 的 `NllScorer::compute_nll(reading_katakana, surface)` 會算單一 `(讀音, 表記)` 的 per-character NLL。它目前用在 `sudachi-dict --model-scores`，拿來給詞典候選排序，不是 runtime reranker。程式是 MIT OR Apache-2.0，模型是 Jinen/zenz，授權比 Saikyo 清楚。

對我們的價值：如果我們想用 Jinen 當 scorer，不必自己寫 tokenizer 與 NLL，可以直接參考 `karukan-engine/src/kanji/llamacpp.rs` 的實作。這是 Rust，和我們的 stack 接近。

#### Roman BERT PLL

Roman IME 用 `bert-base-japanese-v3` 對 25 個候選算 pseudo-log-likelihood，一次 79ms。它有一個值得抄的規則：**左文是空的時候不要用 BERT**。因為 BERT 是雙向模型，沒有左文時只看右文，實測 top-1 從字典的 77% 到 80% 掉到 61%。所以它在文頭、換行、句點後直接跳過 BERT，保留字典順序。

來源：[Roman IME 開發記](https://zenn.dev/mobazou/articles/roman-local-llm-ime)

限制：只有部落格，沒有公開程式或模型。數字是作者自行量測，不能當 benchmark。

對我們的價值：**context 不足時不要硬跑 neural reranker**。我們的 `ja/rerank` 目前只要 `left` 有值就會跑；可以加一條「左文長度低於門檻就跳過」的規則，省 latency 也避免變差。

#### Akaza feature reranker

Akaza 是統計式轉換器，沒有神經 reranker，但它有一個清楚的特徵重排：

```text
rerank_cost = 1.0 * unigram
            + bigram_weight * bigram
            + unknown_bigram_weight * unknown_bigram
            + length_weight * token_count
            + skip_bigram_weight * skip_bigram
```

預設 `bigram_weight=1.0`、`unknown_bigram_weight=1.0`、`length_weight=2.0`、`skip_bigram_weight=0.2`。`length_weight` 用來偏好較長的詞單位，`skip_bigram` 抓 1 詞跳過的文脈。這些權重有 grid search 與評估紀錄。

來源：[Akaza 変換エンジンの仕組み](https://akaza-im.github.io/akaza/internals/conversion-engine.html)

對我們的價值：如果 neural reranker 的增益不夠，這些特徵是便宜的 baseline。我們的 `fuse_word_scores` 已經有 stats 與 draft，但沒有 skip-bigram 與 length penalty。加這兩個特徵的成本遠低於換模型。

#### 通用日文 reranker 能不能直接用

`hotchpotch/japanese-reranker-tiny-v2` 與 `xsmall-v2` 是檢索用 cross-encoder，MIT，ONNX int8 分別 29.6MB 與 37.4MB，和我們現有 `ja-rerank` 的 34MB int8 同一個大小級別。它們的輸入是 query/passage，訓練資料是 JQaRA、MIRACL、MS MARCO 等檢索資料，不是 IME 候選組。

結論：**不能 zero-shot 直接接**。若要用，必須把 `(左文脈, 讀音, 候選表記)` 改寫成 query/passage 格式，並在 IME 候選組上 fine-tune。`tiny-v2` 可當 CPU fallback 的 base，`xsmall-v2` 可當品質較高的 base。`ruri-v3-reranker-310m` 1.26GB，品質高但對 100ms 太重。

#### 補充結論

- 目前查到真正針對 IME 的公開 reranker 有 Saikyo 與 Shizuku 兩個。Saikyo 任務最接近，但 Gemma 授權、270M、程式無 LICENSE；Shizuku 大小與部署最接近，但 GPL 程式、macOS 限定、沒有公開 benchmark。
- 我們現有的 34MB int8 cross-encoder 在大小上已經合理。差距主要在訓練資料、listwise loss、候選 pool 與 context 規則，不在模型大小。
- 最值得立即借的是 Shizuku 的 fusion/fallback/unknown floor，以及 Saikyo 的 listwise loss 與 pool 32。這兩者都不需要換掉現有 ONNX runtime。
- 若要做新的 neural reranker，`hotchpotch` tiny/xsmall v2 是合法的 fine-tune base；Saikyo 的資料格式可參考，但資料與程式授權要先釐清。

## 7. 建議

### 7.1 有沒有值得直接接的模型？

有，但要先做 spike，不要直接進產品。

**第一順位：Jinen v2 xsmall，其次是 small。**

- 接哪裡：`jc-model` 新增 optional 的 neural 轉換角色，或獨立的 `jc-zenz` crate，輸出候選給 `ja/decode` 合併。不要直接取代 `ja/rerank` 的 scalar 路徑。
- 為什麼：35.7M、Q5 28MB、CPU 4 threads p50 13ms、AJIMEE 79.0% NFKC。這是目前最符合我們 CPU 與 100ms 預算的公開模型。
- 預估增益：在 AJIMEE 這類同音硬題，公開數字顯示有機會從統計 draft 的 44.5% 提升到 79% 左右；一般文未知。注意這是模型自由生成的數字，還沒有加上我們的讀音檢查與 fallback。
- 風險：small p90 111ms 超過預算；xsmall 品質較低；NFKC 前處理不能漏；授權 CC-BY-SA 4.0。

**第二順位：Zenz v3.2 small。**

- 接哪裡：同上，但優先借它的 constraint 機制，不是只接權重。
- 為什麼：74MB Q5、GPT-2 91M 級、CC-BY-SA 4.0、官方系統成熟。
- 預估增益：官方 AJIMEE small 80.0%，但 v3.2 沒有公開分數，CPU 延遲也未知。先量 p50/p95，再決定。
- 風險：沒有 CPU benchmark；v3.2 的右文脈在我們的即時輸入不一定拿得到；需要 llama.cpp，與現有 ONNX runtime 不同。

**不建議直接接：Zenz v2.5 medium、Genmai 310M。**

- 310M / 619MB / 1.24GB fp32，對 100ms CPU 路徑太重。
- Genmai 是 Core ML / ANE，Linux/Windows 不適用。
- 可當品質上限與 distillation teacher。

**先不要碰：kanakanji-edge-33m。**

- 品質與大小很吸引人，但權重 Apache-2.0 與訓練資料 CC-BY-SA 4.0 的關係未釐清。等授權確認再說。

**如果你問的是 reranker，不換轉換模型：**

- **Saikyo IME reranker 不值得直接接。** 任務定義最接近，但 base 270M、Gemma 授權、程式沒有 LICENSE，CPU 也沒有延遲數據。只借它的 listwise loss、候選 pool 32、`ctx/input/output` 格式。
- **Shizuku judge 值得借機制，不值得直接搬程式。** 41MB Q8、GPT-2 xsmall、context 32 tokens、3 threads、engine score 加 logprob、失敗回退，這些都符合我們的 100ms 與 CPU 前提。但它是 GPL-3.0 加 macOS Accelerate，直接搬進 AGPL 專案要處理授權與 BLAS 後端。
- **通用 hotchpotch reranker 不能 zero-shot。** tiny/xsmall v2 的 ONNX int8 大小和我們現有 34MB 同級，但訓練目標是檢索。要用的話，先建立 IME 候選組訓練資料，再 fine-tune。
- **我們現有的 cross-encoder 先留著。** 先加 listwise loss、候選 pool 16 到 32、左文不足就跳過 neural、未知 token floor、失敗回退。這些改動比換模型便宜，也比較容易量測。

### 7.2 有沒有值得照抄的機制？

有，優先順序如下：

1. **讀音忠實度檢查**：用 `Decoder::audit_target` 擋掉讀音不吻合的神經輸出。低工作量，立即降低幻覺風險。
2. **統計 draft ＋ token 驗證 ＋ 前綴約束重搜**：這是 Zenzai 的核心。先把 `JaReranker` 的回傳型別從 scalar 擴充成 `Pass/FixRequired/WholeResult`，再讓 `Decoder::nbest` 接受 prefix constraint。
3. **Token 級 logprob 與 mismatch 位置**：比單句分數更能定位錯誤，也能產生 alternative constraints。
4. **逐次輸入的 lattice/constraint 快取**：對 live conversion 的延遲幫助直接。
5. **右文脈與條件標籤**：v3.2 已經證明可行，但我們的即時輸入不一定有完整右文，先做離線評估。
6. **輸出 token 才計 loss 的訓練方式**：若我們自己訓練小模型，這是基本盤。
7. **Listwise loss 與候選 pool 32**：Saikyo 的 `listwise_cross_entropy + 0.2 * BCE` 直接對同組候選最佳化。我們現在是 pointwise 打分再融合，改成 listwise 有機會改善排序。候選 pool 從 8 拉到 16 或 32 也可以先量延遲。
8. **Engine score ＋ LM logprob ＋ 未知 token floor ＋ fallback**：Shizuku 的融合方式比純 neural 分數穩。未知 token 壓到最小值、模型失敗回統計排序，這兩個規則可以直接放進 `fuse_word_scores`。
9. **左文不足就跳過 neural**：Roman 的實測顯示沒有左文時 BERT 會變差。我們的 `ja/rerank` 可以加最小左文長度門檻。

### 7.3 有沒有授權上不能碰但技術上值得學的？

有：

- **ATOK**：專有，沒有模型與程式。可學的是「傳統轉換 ＋ 神經特徵/重排」的混合架構，以及輸入錯誤自動修正。廠商宣稱誤轉換降低約 30%，但没有獨立驗證。
- **Google 日本語入力**：專有，沒有 neural 技術揭露。可學的是大規模詞典與 Web 語料的自動更新；Mozc OSS 只給統計式 baseline。
- **Apple 日本語入力**：沒有官方技術文件，只有傳聞。沒有可學的具體機制。
- **Zenz 權重**：CC-BY-SA 4.0，不是 AGPL-compatible。技術上可以獨立使用，但不能把權重改標 AGPL，也不能在未遵守 ShareAlike 的情況下放進散布包。若只借機制、自己訓練模型，就沒有這個限制。
- **KanaKanji-HardBench**：沒有 LICENSE，不能散布。分數可以看，題目不能放進產品或公開題庫。
- **Genmai / kanakanji-edge**：授權標示有衝突或未釐清。技術值得學，權重先不要碰。
- **Saikyo IME reranker**：Gemma 授權加程式無 LICENSE。技術值得學 listwise loss 與 pool 設計，模型與程式都不要直接放進產品。
- **Shizuku judge**：程式 GPL-3.0-or-later，模型 MIT。技術值得學 fusion 與 fallback；直接搬程式要處理 GPL/AGPL 與 macOS Accelerate，先當設計參考。
- **Roman BERT PLL**：只有部落格，沒有程式與模型。可以學「左文不足就跳過」的規則，數字不能當 benchmark。
- **通用 hotchpotch reranker**：MIT 乾淨，但訓練目標是檢索。可以用來 fine-tune，不能宣稱直接接上就會改善 IME。

## 8. 建議的執行順序

### 現在

1. 用現有 `ajimee_200.json` 與 `anthy_corpus1.json`，跑 Jinen v2 xsmall、Jinen v2 small、Zenz v3.2 small 的 free greedy。記 top1、top5、CER、p50、p95、timeout rate。
2. 同一批資料加「讀音檢查」：神經輸出若不能由 `Decoder::audit_target` 解釋就丟棄，看 top1 下降多少、錯誤率下降多少。
3. 比較「只換模型」與「模型＋讀音檢查」的差異。若只換模型沒有贏，先不要進整合。
4. 用同一批候選組比較 reranker：現有 `ja-rerank`、候選 pool 32、listwise loss、Shizuku 式 `engine + logprob` 融合。先量 top1、MRR、p50、p95。若沒有贏，先不改 reranker。

### 之後

5. 若 Jinen xsmall 在 AJIMEE 有明顯增益、在 Anthy 不退化，做 `jc-model` 的 optional neural 角色，模型獨立下載，timeout 回統計路徑。
6. 若增益主要來自約束重搜，實作 `JaReranker` 的結構化回傳與 `Decoder` 的 prefix constraint。這一步比換模型更接近 Zenzai 的實際收益。
7. 若要做長期，使用 `zenz-v2.5-dataset` 訓練 30M 到 90M 的自家模型，目標 CPU p50 < 50ms。先確認 CC-BY-SA/ODC-BY 的散布義務。

### 不要做

- 不要直接把 310M 模型放進 CPU 路徑。
- 不要用 free generation 取代 Viterbi。T03 的 47.5% 已經示範過。
- 不要把不同 benchmark 的分數直接相比。
- 不要在未釐清授權前散布 kanakanji-edge、Genmai 或 HardBench 資料。
- 不要把 CC-BY-SA 4.0 權重說成 AGPL-compatible。

## 9. 未確認清單

| 項目 | 狀態 | 原因 |
|---|---|---|
| `zenz-v3.2-small` 實際參數量 | 未確認 | model card 空白；只有 GGUF metadata 顯示 GPT-2 small 級 |
| `zenz-v3.2` CPU 延遲 | 未確認 | 官方只給 GPU 建議，沒有 CPU benchmark |
| `zenz-v3.2` 的 AJIMEE / HardBench 分數 | 未確認 | 沒有公開評測 |
| Google 日本語輸入是否使用 neural | 未確認 | 沒有官方技術文件 |
| Apple 日本語輸入是否使用 neural | 未確認 | 只有傳聞 |
| ATOK 的模型大小與推論方式 | 未確認 | 專有，無公開資料 |
| `KanaKanji-HardBench` 資料授權 | 未確認 | repo 沒有 LICENSE |
| `genmai-90M` 的 base 授權 | 未確認 | 卡片說 Apache-2.0，官方 zenz 卡片說 CC-BY-SA 4.0 |
| `kanakanji-edge-33m` 權重與訓練資料的 ShareAlike 關係 | 未確認 | 權重宣告 Apache-2.0，資料含 CC-BY-SA 4.0 |
| Jinen v2 完整訓練資料組成 | 未確認 | model card 只列 NDL 書誌資料 |
| Saikyo IME reranker 的程式授權 | 未確認 | repo 沒有 LICENSE，GitHub API 回 `null` |
| Saikyo 訓練資料的全域授權 | 未確認 | 只有逐列 `from_license`，沒有 dataset-level license |
| Gemma 授權與 AGPL-3.0 的相容性 | 未確認 | Gemma 不是 OSI 授權，需法務 |
| Shizuku judge 的實際 top-1 與延遲 | 未確認 | repo 只有 `--judge-bench`，沒有公開結果 |
| 通用日文 reranker zero-shot 用在 IME 的效果 | 未確認 | 訓練目標是檢索，沒有 IME 評測 |
| Saikyo 的 0.8516 / 0.6719 是否可外推 | 未確認 | 訓練 run 內部 eval，不是獨立 benchmark |
| CC-BY-SA 4.0 模型輸出的授權地位 | 未確認 | 法律問題，需法務 |
| ACL 2024 alignment decoding 的程式與授權 | 未確認 | 匿名連結已過期 `repository_expired` |
| `Shion v4.4` 的來源與授權 | 未確認 | 只在 HardBench 出現，找不到官方模型頁 |

## 10. 參考來源

### Zenzai / azooKey

- [AzooKeyKanaKanjiConverter](https://github.com/azooKey/AzooKeyKanaKanjiConverter)
- [Docs/zenzai.md](https://github.com/azooKey/AzooKeyKanaKanjiConverter/blob/main/Docs/zenzai.md)
- [azooKey](https://github.com/azooKey/azooKey)
- [azooKey-Desktop](https://github.com/azooKey/azooKey-Desktop)
- [ANLP 2025 P1-19: ニューラルかな漢字変換システム Zenzai](https://www.anlp.jp/proceedings/annual_meeting/2025/pdf_dir/P1-19.pdf)
- [Zenn: Zenzai を azooKey on macOS に搭載します](https://zenn.dev/azookey/articles/ea15bacf81521e)
- [さくらのナレッジ: Zenzai の開発](https://knowledge.sakura.ad.jp/42901)
- [IPA 未踏 2024 成果詳細](https://www.ipa.go.jp/jinzai/mitou/it/2024/nl10bi0000006qh1-att/seikashosai-ok-3.pdf)

### 模型

- [zenz-v1](https://huggingface.co/Miwa-Keita/zenz-v1)
- [zenz-v2.5-xsmall](https://huggingface.co/Miwa-Keita/zenz-v2.5-xsmall)
- [zenz-v2.5-small](https://huggingface.co/Miwa-Keita/zenz-v2.5-small)
- [zenz-v2.5-medium](https://huggingface.co/Miwa-Keita/zenz-v2.5-medium)
- [zenz-v3.1-small](https://huggingface.co/Miwa-Keita/zenz-v3.1-small)
- [zenz-v3.2-small-gguf](https://huggingface.co/Miwa-Keita/zenz-v3.2-small-gguf)
- [zenz-v3.2-xsmall-gguf](https://huggingface.co/Miwa-Keita/zenz-v3.2-xsmall-gguf)
- [Jinen v2 small](https://huggingface.co/togatogah/jinen-v2-small.gguf)
- [Jinen v2 xsmall](https://huggingface.co/togatogah/jinen-v2-xsmall.gguf)
- [Jinen v1 small](https://huggingface.co/togatogah/jinen-v1-small)
- [Genmai 310M](https://huggingface.co/subdiox/genmai-310M)
- [Genmai 90M](https://huggingface.co/subdiox/genmai-90M)
- [kanakanji-edge-33m](https://huggingface.co/yuuki14202028/kanakanji-edge-33m)
- [duet-ja-kana-kanji-9m](https://huggingface.co/jcbutter/duet-ja-kana-kanji-9m)
- [ku-nlp/gpt2-small-japanese-char](https://huggingface.co/ku-nlp/gpt2-small-japanese-char)

### 系統與程式

- [Karukan](https://github.com/togatoga/karukan)
- [Karukan configuration](https://github.com/togatoga/karukan/blob/main/docs/configuration.md)
- [Akaza](https://github.com/akaza-im/akaza)
- [Akaza 評価方法](https://akaza-im.github.io/akaza/internals/evaluation.html)
- [Mozc](https://github.com/google/mozc)
- [Neural IME](https://github.com/yohokuno/neural_ime)

### 評測與資料

- [AJIMEE-Bench](https://github.com/azooKey/AJIMEE-Bench)
- [日本語Wikipedia入力誤りデータセット v2](https://nlp.ist.i.kyoto-u.ac.jp/?%E6%97%A5%E6%9C%AC%E8%AA%9EWikipedia%E5%85%A5%E5%8A%9B%E8%AA%A4%E3%82%8A%E3%83%87%E3%83%BC%E3%82%BF%E3%82%BB%E3%83%83%E3%83%88)
- [zenz-v2.5-dataset](https://huggingface.co/datasets/Miwa-Keita/zenz-v2.5-dataset)
- [KanaKanji-HardBench](https://github.com/fangquinlan/KanaKanji-HardBench)
- [Sumibi benchmark](https://github.com/kiyoka/Sumibi/blob/main/benchmark/README.md)
- [Anthy corpus](https://github.com/netsphere-labs/anthy/tree/master/corpus)

### Reranker 補充

- [Saikyo IME reranker](https://huggingface.co/nakasyou/saikyo-ime-reranker)
- [Saikyo reranker 程式](https://github.com/nakasyou/reranker-ime)
- [Saikyo 訓練資料](https://huggingface.co/datasets/nakasyou/japanese-conversion)
- [Shizuku IME](https://github.com/n-seiji/shizuku-ime)
- [rinna/japanese-gpt2-xsmall](https://huggingface.co/rinna/japanese-gpt2-xsmall)
- [hotchpotch/japanese-reranker-tiny-v2](https://huggingface.co/hotchpotch/japanese-reranker-tiny-v2)
- [hotchpotch/japanese-reranker-xsmall-v2](https://huggingface.co/hotchpotch/japanese-reranker-xsmall-v2)
- [cl-nagoya/ruri-v3-reranker-310m](https://huggingface.co/cl-nagoya/ruri-v3-reranker-310m)
- [Roman IME 開發記](https://zenn.dev/mobazou/articles/roman-local-llm-ime)
- [Akaza 変換エンジンの仕組み](https://akaza-im.github.io/akaza/internals/conversion-engine.html)
- [Karukan NLL scorer](https://github.com/togatoga/karukan/blob/main/karukan-engine/src/kanji/llamacpp.rs)
- [Karukan sudachi-dict 模型評分](https://github.com/togatoga/karukan/blob/main/karukan-cli/src/bin/sudachi_dict.rs)

### 商用產品

- [ATOK 高精度な変換エンジン](https://www.atok.com/info/features/engine.html)
- [Google 日本語入力](https://www.google.co.jp/ime/)

### 本專案內部

- `crates/jc-model/src/rerank.rs`
- `crates/jc-core/src/ja/rerank.rs`
- `crates/jc-core/src/ja/decode.rs`
- `crates/jc-core/src/decode/nbest.rs`
- `crates/jc-eval/src/ja.rs`
- `legal/README.md`
- `docs/models.md`
- `/mnt/work/jc/jc-ime-data/0c-testdata/ja/README.md`
