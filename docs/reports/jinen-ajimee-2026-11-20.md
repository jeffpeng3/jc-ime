# Jinen v2 xsmall / small 全量 AJIMEE-Bench 量測（2026-11-20）

## 摘要

用同一套判定跑完 AJIMEE-Bench 200 題，固定 prompt、greedy、不給左文脈：

| 模型 | literal exact | NFKC exact | accept 命中 | accept + NFKC | p50 |
|---|---:|---:|---:|---:|---:|
| Jinen v2 xsmall Q5_K_M | 104/200（52.0%） | 118/200（59.0%） | **137/200（68.5%）** | 147/200（73.5%） | 28.7 ms |
| Jinen v2 small Q5_K_M | 115/200（57.5%） | 128/200（64.0%） | **155/200（77.5%）** | 165/200（82.5%） | 212.5 ms |

`accept` 命中的判定與我們引擎相同（top-1 等於 `surface` 或 `accept` 任一項），
所以可直接和我們的 46.0%、Google 54.0%、AzooKey 統計轉換器 44.5% 並列。

兩個模型都明顯高於我們引擎，small 在「無文脈 100 題」是 83.0%。
先前 smoke test 的 20 題「exact」其實是 `accept` 命中，不是字串完全相等；
同一批前 20 題可完整重現 16/20、NFKC 18/20，但 literal exact 只有 10/20。

## 量測條件

| 項目 | 設定 |
|---|---|
| 題庫 | `/mnt/work/jc/jc-ime-data/0c-testdata/ja/ajimee_200.json`（sha256 `057b7bdc…`） |
| 左文脈 | 主組一律空；另跑「已知文脈」組，只取原始 `context_text` 尾端 64 字元 |
| 讀音 | `ajimee_200.json` 的平假名轉片假名（一對一），200 題與原始 `evaluation_items.json` 的 `input` 完全相同 |
| prompt | `U+EE02<left_context>U+EE00<katakana>U+EE01`，無文脈時省略 `U+EE02<left_context>`；整段 NFKC |
| 解碼 | greedy：`temperature=0`、`top_k=1`、`top_p=1.0`、`repeat_penalty=1.0` |
| 模型 | Jinen v2 xsmall / small，Q5_K_M；xsmall sha256 `24ff3af5…`、small `80482707…` |
| runtime | llama-cpp-python 0.3.37（CPU backend，NEON、REPACK、OPENMP） |
| 執行緒 | `n_threads=4`、`n_batch=512`、`n_ctx=1024`、`max_tokens=256` |
| KV cache | 每題 `reset()`，避免前一題的快取前綴影響 prefill 切批 |
| 計分 | 見下節；逐題結果在 `target/jinen-eval/jinen-ajimee-{xsmall,small}.json` |

本機是 20 核 heterogeneous SoC（10×Cortex-X925 3.9GHz、10×Cortex-A725 2.8GHz）。
延遲量測期間機器同時跑其他工作，loadavg 3.6–20.2；p50 會因共租負載大幅變動，
不可直接和模型卡的 13 ms／48 ms 比較。

### 計分定義

- **literal exact**：模型輸出與 `surface` 完全相等，不做事前正規化。
- **NFKC exact**：輸出與 `surface` 都做 NFKC 後再比。
- **accept 命中**：輸出等於 `surface` 或 `accept` 任一項（原始字串）。
- **accept + NFKC**：上述兩邊都 NFKC 後再比。

`ajimee_200.json` 的 200 題有 83 題帶 `accept`，共 215 個去重後變體。
AJIMEE 的 `surface` 是原始 `expected_output[0]`，已逐題核對 200/200 一致。

## 結果

### 全 200 題，無左文脈

| 模型 | n | literal exact | NFKC exact | accept 命中 | accept + NFKC | p50 | p95 |
|---|---:|---:|---:|---:|---:|---:|---:|
| xsmall | 200 | 52.0% | 59.0% | 68.5% | 73.5% | 28.68 ms | 65.63 ms |
| small | 200 | 57.5% | 64.0% | 77.5% | 82.5% | 212.49 ms | 725.47 ms |

載入時間（mmap、page cache 已熱）：xsmall 0.034 s、small 0.029 s。
峰值 RSS（`VmHWM`）：xsmall 142.2 MB、small 219.4 MB；載入前 Python 基礎 RSS 約 46–48 MB。
兩組都沒有空輸出、沒有 `max_tokens` 截斷，平均輸出長度與標準答案幾乎相同。

### 依原始題目有無文脈切開

「有文脈 100 題」在主組不給文脈；「給文脈」組只跑這 100 題，餵入原始 `context_text` 尾端 64 字元。

| 模型 | 子集 | n | literal exact | NFKC exact | accept 命中 | accept + NFKC |
|---|---|---:|---:|---:|---:|---:|
| xsmall | 原始無文脈 | 100 | 62.0% | 65.0% | 78.0% | 81.0% |
| xsmall | 原始有文脈（不給） | 100 | 42.0% | 53.0% | 59.0% | 66.0% |
| xsmall | 原始有文脈（給 64 字） | 100 | 53.0% | 61.0% | 69.0% | 76.0% |
| small | 原始無文脈 | 100 | 62.0% | 65.0% | 83.0% | 86.0% |
| small | 原始有文脈（不給） | 100 | 53.0% | 63.0% | 72.0% | 79.0% |
| small | 原始有文脈（給 64 字） | 100 | 57.0% | 69.0% | 77.0% | 86.0% |

給文脈對 small 的 `accept` 是 +5 百分點（72→77），對 xsmall 是 +10（59→69）；
但也各弄壞 2 題。文脈不是萬靈丹，主要還是解同音詞。

### 與公開成績並列（全 200 題、無文脈，accept 判定）

| 系統 | 全 200 | 無文脈 100 | 來源 |
|---|---:|---:|---|
| **Jinen v2 small（本次）** | **77.5** | **83.0** | 本報告 |
| **Jinen v2 xsmall（本次）** | **68.5** | **78.0** | 本報告 |
| Google 日本語入力 | 54.0 | 62.0 | Zenzai 論文 Table 1／Table 3 |
| 本引擎（Mozc v2 pack） | 46.0 | 50.0 | `docs/port/ajimee-eval.md` |
| AzooKeyKanaKanjiConverter（統計） | 44.5 | 46.0 | Zenzai 論文 Table 1／Table 3 |

若改成 NFKC 判定，Jinen 兩列分別是 82.5／73.5（全 200）與 86.0／81.0（無文脈 100）。
公開對照的 Google／AzooKey 數字沒有標示 NFKC，保持原樣。

### 與模型卡宣稱的差距

| 模型 | 模型卡 exact | 本次 accept 命中 | 模型卡 NFKC | 本次 accept + NFKC |
|---|---:|---:|---:|---:|
| xsmall | 74.0% | 68.5% | 79.0% | 73.5% |
| small | 80.0% | 77.5% | 86.0% | 82.5% |

模型卡的「exact」應該也是接受變體後的分數，因為 literal exact 只有 52.0%／57.5%，
和 74.0%／80.0% 差太遠；卡上的「參考 HF 版 73.0/78.0」也是同一種算法。
本次兩邊都低 3.5–5.5 個百分點，原因未追查；可能是 llama.cpp 版本、量化檔批次或
accept 集合定義不同。沒有為了追分改 prompt。

### 延遲與記憶體

| 模型 | p50 | p95 | 載入 | 峰值 RSS | 量測時 loadavg（起→訖） |
|---|---:|---:|---:|---:|---|
| xsmall | 28.68 ms | 65.63 ms | 0.034 s | 142.2 MB | 3.61 → 4.36 |
| small | 212.49 ms | 725.47 ms | 0.029 s | 219.4 MB | 17.34 → 20.24 |

small 這一列被共租負載汙染：同一設定重跑 p50 在 157–282 ms 之間跳動，
另外在 30 題、綁到 4 個 X925 核的短跑量到 p50 61.8 ms。
xsmall 的 28.7 ms 則在較乾淨的窗口取得，仍高於模型卡的 13 ms。
結論：這台機器只能保證 xsmall 落在「數十毫秒」、small 落在「數百毫秒」級，
要和模型卡的 13／48 ms 比較，需要一台閒置機器重跑。

## 20 題 smoke test 對帳

用本次協定重跑前 20 題：

| 模型 | literal exact | NFKC exact | accept | accept + NFKC |
|---|---:|---:|---:|---:|
| xsmall | 10/20 | 11/20 | 16/20 | 18/20 |
| small | 10/20 | 11/20 | 17/20 | 19/20 |

背景表格的 xsmall「16/20（原始）／18/20（NFKC）」與這裡的 accept 欄完全一致，
所以那張表的「exact」其實是 accept 命中。拿它和我們引擎的 46.0% 比是對的
（我們引擎也是 accept 判定），但不能說成「字串完全相同」。

## 10 個錯誤案例

以下取自 small、無文脈主組；分類是人工粗分，一題可能同時屬於多類。
「給文脈」欄只適用原始有文脈的題目。

| # | 題號 | 讀音（節錄） | 模型輸出 | 標準答案 | 分類 | 給文脈後 |
|---|---|---|---|---|---|---|
| 1 | ajimee-008 | ふくごうせいをせんたく | 複合**性** | 複合**姓** | 同音詞 | 仍錯 |
| 2 | ajimee-021 | しゅくしょとしてつくったしろ | 宿所として**作った城** | 宿所として**造った城** | 同音詞＋文脈不足 | 仍錯（「白」） |
| 3 | ajimee-023 | さいしんのうえかいほう | **最新**の上開放 | **再審**の上解放 | 同音詞＋文脈不足 | 語意修好，但「かちとった」不符合 accept |
| 4 | ajimee-026 | ようせいをうたっていた | **妖精**を歌っていた | **養成**を謳っていた | 同音詞 | 修正 |
| 5 | ajimee-040 | こうかん…ぎせいかん | **交換**…**犠牲感** | **高官**…**議政官** | 專有名詞＋同音詞 | 無文脈可給 |
| 6 | ajimee-042 | すずきたかのり | 鈴木**隆徳** | 鈴木**尚典** | 專有名詞（人名） | 仍錯（貴紀） |
| 7 | ajimee-075 | たいくうきじゅうぐん | 対空機**従軍** | 対空機**銃群** | 斷詞 | 無文脈可給 |
| 8 | ajimee-148 | どくさいせい | 独裁**政** | 独裁**制** | 同音詞 | 仍錯 |
| 9 | ajimee-179 | たいせんごじゅうねん | 大戦**五十年** | 大戦**後１０年** | 斷詞＋數字表記 | 無文脈可給 |
| 10 | ajimee-196 | ほけんかにかいしょう | **保険下に解消** | **保健科に改称** | 同音詞 | 修正 |

觀察：

- 錯誤集中在漢字同音詞，這正是 AJIMEE 的設計目標。
- 專有名詞（人名、官職）幾乎沒有救，文脈給 64 字也不夠。
- 斷詞錯誤會把兩個詞合成一個不存在的詞（機従軍、五十年），但讀音仍可解釋，
  外部讀音檢查擋不住。
- 模型的弱點不是「讀音不忠實」，而是「選錯同音漢字」；這一點和自由生成的預期不同。

## zenz-v3.2-small

**格式未知，未測。**

- HF repo `Miwa-Keita/zenz-v3.2-small-gguf`（revision `ef3e66e…`）只有
  `README.md`（29 bytes，只有 `license: cc-by-sa-4.0`）與 GGUF，
  沒有 `tokenizer_config.json`、`chat_template` 或 prompt 範例。
- 非 GGUF 的 `Miwa-Keita/zenz-v3.2-small` repo 回 401，`tokenizer_config.json` 回 404。
- 額外障礙：llama-cpp-python 0.3.37 載入這個 GGUF 時直接失敗，
  `unknown pre-tokenizer type: 'gpt2-small-japanese-char'`；就算照 AzooKey 文件
  猜 `U+EE02<left>U+EE07<right>U+EE00<katakana>U+EE01`，也需要更新的 llama.cpp。

## 限制與未完成

- 延遲在共用機器上量測，small 的 p50／p95 只能當上界；要進 100 ms 預算的判斷，
  需要閒置機器重跑 xsmall／small 與不同 quant。
- 只測 Q5_K_M，沒有測 f16／Q8_0／Q4_K_M，也沒有測 small 的 p90／p99。
- 模型卡的數字無法完全重現，差 3.5–5.5 個百分點，未追查。
- 沒有實作「模型 + 讀音檢查 + 候選重排」；這裡只量自由生成的上限與錯誤型態。

## 重現方式

```sh
cd /mnt/work/jc/wt/jinen-eval
uv run --with llama-cpp-python python scripts/eval_jinen_ajimee.py --model xsmall
uv run --with llama-cpp-python python scripts/eval_jinen_ajimee.py --model small
```

腳本會抓原始 `evaluation_items.json`（固定 commit、sha256 檢查）到
`/tmp/opencode/jinen-eval/`，逐題結果寫到 `target/jinen-eval/`（已被 `.gitignore` 排除）。
若要綁核，加 `--affinity 5-8`；預設不綁。

## 授權

題庫 AJIMEE-Bench 是 **CC-BY-SA-3.0**（`0c-testdata/ja/LICENSE-ajimee.txt`），
只用於評測，不進訓練。模型 jinen v2 是 **CC-BY-SA-4.0**。
本報告引用公開分數時保留原始出處。
