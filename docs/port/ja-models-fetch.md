# ja-models-fetch：外部日文模型抓取與 Jinen smoke test

2026-11-20。來源清單取自 [`docs/reports/ja-ime-survey-2026-11-20.md`](../reports/ja-ime-survey-2026-11-20.md)。
所有權重放在 `/mnt/work/jc/jc-ime-data/models/external/<slug>/`，每個目錄都有
`manifest.json`（repo、revision、逐檔大小與 SHA-256、授權、來源）。這裡只下載與量測，
不改程式碼、不上傳 HF。

## 下載結果

總計 251.4 MiB（263,628,799 bytes），低於 400 MB 上限。

| 模型 | repo @ revision | 取的檔案 | 大小 | 授權 | 可否散布 |
|---|---|---|---:|---|---|
| Jinen v2 xsmall | `togatogah/jinen-v2-xsmall.gguf` @ `3910fd01` | `jinen-v2-xsmall-Q5_K_M.gguf`＋`README.md` | 28,261,056 | CC-BY-SA-4.0 | 可，需署名；衍生或散布需同授權 |
| Jinen v2 small | `togatogah/jinen-v2-small.gguf` @ `3461d057` | `jinen-v2-small-Q5_K_M.gguf`＋`README.md` | 81,117,824 | CC-BY-SA-4.0 | 同上 |
| zenz v3.2 small | `Miwa-Keita/zenz-v3.2-small-gguf` @ `ef3e66e3` | `ggml-model-Q5_K_M.gguf`＋`README.md`（空） | 73,871,936 | CC-BY-SA-4.0 | 同上 |
| zenz v3.2 xsmall | `Miwa-Keita/zenz-v3.2-xsmall-gguf` @ `42cab839` | `ggml-model-Q5_K_M.gguf`＋`README.md`（空） | 20,970,304 | CC-BY-SA-4.0 | 同上 |
| duet-ja-kana-kanji-9m | `jcbutter/duet-ja-kana-kanji-9m` @ `18585dad` | `app/JapaneseConverter.pte`＋embedding／vocab／merges | 12,281,664（整組 14,769,047） | CC-BY-SA-4.0 | 同上；只取 ExecuTorch int8，沒抓 `9m-wide.pt` |
| Shizuku judge | `rinna/japanese-gpt2-xsmall` @ `8e91527b` | 本地轉檔的 `model_q8_0.gguf`＋`tokenizer.json` | 41,405,248（整組 44,633,723） | 權重 MIT；產生程式 GPL-3.0-or-later | 權重可散布，需附 MIT；程式沒取用 |

主要權重 SHA-256：

| 檔案 | SHA-256 |
|---|---|
| `jinen-v2-xsmall/jinen-v2-xsmall-Q5_K_M.gguf` | `24ff3af5db712fbbb4aa9254ee28ec4d731207134471ab68b06c1828726284c2` |
| `jinen-v2-small/jinen-v2-small-Q5_K_M.gguf` | `80482707513d6b67dafc31774371cf95d765542abf8d74eebf5f32f92d788bd3` |
| `zenz-v3.2-small/ggml-model-Q5_K_M.gguf` | `29c223d4c23327b80fd13ebb5ab2555057a46317997d5da391584ffbef0db673` |
| `zenz-v3.2-xsmall/ggml-model-Q5_K_M.gguf` | `00c64b3d318045a708d0cad5434faccab10f5481a49e6362864551fd0995fa58` |
| `duet-ja-kana-kanji-9m/app/JapaneseConverter.pte` | `554135eba0f61466ede7c2c1c0934c88294ab4c5bdfa92761b55aba7aaed102d` |
| `shizuku-judge/model_q8_0.gguf` | `86727ed56aa0b669552b4c24cdf25c2f98300ed019938b75a6ec986eb4fc0856` |

HF 上這幾個 repo 沒有 `LICENSE` 檔，授權只寫在 model card 的 `license` 欄位，所以目錄裡只有
`README.md`。CC-BY-SA 4.0 全文由 `legal/licenses/CC-BY-SA-4.0.txt` 提供。

### Shizuku judge 是本地轉檔，不是 HF artifact

報告裡的「Q8 權重 41MB」沒有公開的 HF repo。實際做法在 `n-seiji/shizuku-ime` 的
`tools/gen_judge_model.py`：抓 `rinna/japanese-gpt2-xsmall`（MIT）的 `model.safetensors`
與 `spiece.model`，把 2-D 權重轉置後做 ggml q8_0 量化，其餘留在 f32。產物與上游
`tools/judge.sha256` 完全一致（`model_q8_0.gguf` 與 `tokenizer.json` 兩筆都比對通過）。

目錄裡另存了 `gen_judge_model.py`、`judge.sha256`、`spiece.model` 與來源 model card，
可以重跑驗證。轉檔是 Adapted Material 的產生流程，但權重來源是 MIT，沒有 ShareAlike 問題。

## 各模型的 prompt 格式

先讀卡片再決定格式，沒有自己發明。

| 模型 | 輸入格式 | 依據 | 備註 |
|---|---|---|---|
| Jinen v2 | `U+EE02<left_context>U+EE00<片假名讀音>U+EE01`，整段 NFKC，greedy | HF model card 的用法段與 jinen-v1 的 `jinen Format` 表 | 卡片明寫「NFKC 前提」、`--temp 0 --top-k 1`；沒有 context 時前綴整個省略 |
| zenz v3.2 | `U+EE02<left>U+EE07<right>U+EE00<片假名>U+EE01` | AzooKeyKanaKanjiConverter `Docs/zenzai.md`、`ZenzPromptBuilder.swift` | HF repo 的 README 只有 29 bytes，沒有範例；格式來自實作文件，**沒有實測驗證** |
| duet-ja-kana-kanji-9m | 需 ExecuTorch runtime | repo 的 `student.py`、`app/` | 沒有 llama.cpp 路線，本專案的 runtime 不能直接跑 |
| Shizuku judge | 左文脈＋完整候選，取候選在文脈下的 log-likelihood | `src/judge.rs` | 不是生成式轉換，是 reranker |

## Jinen v2 xsmall smoke test

### 環境

- 機器：GB10（aarch64，20 核：10×Cortex-X925 ＋ 10×Cortex-A725，128GB UMA）
- runtime：`uv run --with llama-cpp-python`，llama-cpp-python 0.3.37
- `n_ctx=1024`、`n_threads=4`、`n_batch=512`、`temperature=0.0`、`top_k=1`、`max_tokens=256`、`seed=1234`
- 題目：`/mnt/work/jc/jc-ime-data/0c-testdata/ja/ajimee_200.json` 前 20 題

### 完整 prompt

模型卡的做法逐字照用：

```python
INPUT_START, OUTPUT_START, CONTEXT = "\uee00", "\uee01", "\uee02"

def hira2kata(s):  # fixture 的 reading 是平假名，模型吃片假名
    ...

def build_prompt(reading, context=""):
    p = f"{CONTEXT}{context}" if context else ""
    p += f"{INPUT_START}{reading}{OUTPUT_START}"
    return unicodedata.normalize("NFKC", p)
```

`ajimee_200.json` 沒有 context 欄位，所以 20 題的 context 都是空字串，實際送出的 prompt 是
`U+EE00<片假名讀音>U+EE01`。第 1 題的完整字串（可見字元表示）：

```text
\uee00セイネンシデカクコトノリユウトシテ、ダンセイセイネンシニオケルダンセイユウイノセイイシキ(\uee01
```

NFKC 把讀音裡的「（」轉成半角「(」，模型輸出也跟著用半角，參考答案保留全角，所以評分要同時算
原始與 NFKC 兩種。
20 題的 prompt 長度 8～50 tokens（平均 25.9），輸出平均 12.8 tokens。

### 結果

| 指標 | 數值 |
|---|---:|
| exact match（原始字串，比 `surface` 或 `accept` 任一） | **16 / 20（80.0%）** |
| exact match（雙方 NFKC，模型卡的操作條件） | **18 / 20（90.0%）** |
| 載入時間 | 0.021 s |
| 每題延遲 p50 | 65.5 ms |
| 每題延遲 p95 | 81.4 ms |
| 每題延遲 mean／max | 64.0／82.3 ms |
| 峰值記憶體（VmHWM） | 103.3 MB |

逐題輸出（第二次跑，含暖機一題；兩次跑的判定完全相同）：

| id | 判定 | ms | 輸出 |
|---|---|---:|---|
| ajimee-001 | NFKC | 23.6 | 青年誌で書くことの理由として、男性青年誌における男性優位の性意識( |
| ajimee-002 | exact | 55.3 | の依頼が来るように |
| ajimee-003 | exact | 82.3 | の男性方の土蔵から、棟方志功作品と思われる版画など、骨董品三百点が盗難 |
| ajimee-004 | exact | 61.5 | 例えば直交座標平面上の任意の点の原点から |
| ajimee-005 | exact | 50.5 | 、本作もその |
| ajimee-006 | exact | 72.3 | た。」と個人的な感想を残したが、アインシュタインは歴史を専攻しておらず、このような勘違いをした |
| ajimee-007 | exact | 69.2 | では、議員の不逮捕特権 |
| ajimee-008 | miss | 71.3 | に変更、あるいは複合性を選択するためにはそのように婚姻前に手続きを行わ |
| ajimee-009 | exact | 73.8 | は新暦八月一日もしくは月遅れの九月一日 |
| ajimee-010 | exact | 78.6 | 満たせば変身ベルトを手にしたものが誰でも仮面ライダーになれるという、ここまでの平成ライダーで培われたイメージの集大成的な作品 |
| ajimee-011 | exact | 55.6 | た後、そのまま東方向に伸び |
| ajimee-012 | exact | 79.7 | 三年九月十五日、友人のデニース・マクネーアが白人優越論者による十六番街聖ヨハネ教会での爆弾テロ |
| ajimee-013 | exact | 54.4 | 矩形状に配列した |
| ajimee-014 | exact | 59.6 | の研究テーマである多変数複素関数論 |
| ajimee-015 | exact | 81.4 | を収めたルーカスはその資金を映像・音響関係のデジタル化を中心とした技術開発に投資し、映画界に多大 |
| ajimee-016 | exact | 70.2 | サプリメントの摂取などにより継続的に過剰摂取した場合には以下のよう |
| ajimee-017 | NFKC | 65.5 | あり(後述)、相撲部屋の数は減少傾向 |
| ajimee-018 | exact | 59.7 | 浄土真宗辞典が、どれに抵触するんです |
| ajimee-019 | miss | 51.7 | 数節あるが、 |
| ajimee-020 | exact | 64.4 | 硬い殻を持つ卵を産むこと、高い代謝率 |

001 的 23.6 ms 是暖機時用過同一條 prompt，命中 KV 前綴；其他題目都是新 prompt。
兩題真正的錯誤是 008（`複合姓` → `複合性`）與 019（`数説` → `数節`）。001 與 017 只差
全角／半角括號，NFKC 後相同，這正是模型卡把 NFKC 當成實際操作條件的原因。

### 延遲：新 prompt 與快取命中差 4 倍

| 情境 | p50 | p95 | 說明 |
|---|---:|---:|---|
| 20 題，每題都是新 prompt | 65.5 ms | 81.4 ms | 主要成本是 prompt 評測 |
| 同一條 prompt 連跑 4 次 | 93.2 → 25.9 → 15.6 → 15.2 ms | — | llama.cpp 重用 KV 前綴 |
| 冷跑（含第一次 graph 配置） | 60.6 ms | 81.2 ms | max 107.0 ms |

拆開新 prompt 的成本：prompt 評測（平均 26 tokens）約 49 ms，解碼（12.8 tokens）約 15 ms，
每 token 約 1.5～1.9 ms。模型卡寫的 p50 13 ms 對得上的是「KV 前綴已快取」那個情境，
不是每題重算整段 prompt 的情境。

輔助測量：

- 37 tokens 的 prompt 連跑 20 次，4 threads p50 48.6 ms，8 threads 44.6 ms，20 threads 退化到 421 ms
  （big.LITTLE 遷移）。所以正式量測用 4 threads。
- 這台機器 numpy sgemm 有 420 GFLOPS，負載時大核跑 3.8GHz，不是降頻造成。
  剩下的差距可能是 aarch64 wheel 的 llama.cpp kernel 沒吃到 SVE2／i8mm，**未確認**。

對我們的意義：smoke test 證明「能跑、格式對」。要走這條路，先做逐字輸入的 KV 前綴快取，
它比換 runtime 或換機器更能決定延遲。

## 授權注意

- 五個模型是 CC-BY-SA 4.0：可商用、可散布，但要附署名與授權全文；**量化、轉檔、微調會產生
  Adapted Material，衍生或散布需同授權**，不能改標 AGPL-3.0-only。
- Shizuku judge 的權重來源 `rinna/japanese-gpt2-xsmall` 是 MIT；產生程式是 GPL-3.0-or-later，
  這次只取產物，沒有取用程式碼。
- 這批全部標成「僅本機評估，暫不散布」，見 [`legal/README.md`](../../legal/README.md) L-20～L-25。

## 沒做與未確認

- 沒有下載 `zenz-v2.5-medium`、`genmai`、`saikyo` base、`zenz-v2.5-dataset`；
  `kanakanji-edge-33m` 只記錄 metadata，沒有抓權重。
- zenz v3.2 的 prompt 格式來自 zenzai 實作文件，HF 卡片是空的，**沒有實測**。
- 只跑前 20 題。完整 200 題、top-5、CER、讀音檢查、Anthy 回歸另開任務。
- Jinen v2 small、zenz v3.2 兩個檔已下載但還沒量延遲。
