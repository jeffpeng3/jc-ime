# real-corpus：用真人會打的句子量測分詞（2026-11-21）

## 1. 跑法

```bash
cd /mnt/work/jc/wt/real-corpus
cargo run -q -p jc-cli -- corpus-diff --limit 0 --top 5 \
  --cases /tmp/corpus-diff-cases.json
# 機器可讀摘要：
cargo run -q -p jc-cli -- corpus-diff --limit 0 --top 5 --json > /tmp/corpus-diff.json
```

`--source` 預設走 `JC_TESTDATA`（此機為 `/mnt/work/jc/jc-ime-data/0c-testdata`）；注音用
`<資料根>/build-zh` 的 `lexicon.bin`／`lm.bin`，日文用預設 pack，英文用
`English::load_default()`。`--cases` 寫出的每個案例都有 `source`、`origin`、
`variant` 欄位，壞字段與 `mixed-diff` 相容（`input`、`bucket`、`signals`、`a`、`b`）。

量測物件：A = `mixed_latin::split`，B = `Router::convert`（`mixed_latin_split: true`）。
比對與分類沿用 `mixed-diff`。`crates/jc-cli/src/diff_cmd.rs` 屬於 `segment-diff` 分支，
為避免兩 branch 同時改同一檔，分類程式（`Case`／`Bucket`／`classify`、`case_json`、
`describe`）在 `corpus_cmd.rs` 內複製一份，沒有抽共用模組。`mixed-diff` 本身完全沒改；
用 `--seed 1 --count 2000` 比對修改前後的 JSON，結果逐位元組相同。

## 2. 語料來源與筆數

全部都是既有題庫／測試／真機案例，去重後共 **938 筆唯一輸入 937 種**。
中文句子由注音轉大千按鍵，並同時產生「保留聲調」與「去掉聲調鍵」兩版；
後者連一聲的空白鍵一起拿掉。去重時若同一字串出現在多個來源，`source` 欄位會列出全部。

| 來源（別名） | 原始位置 | 筆數 |
|---|---|---:|
| `zh-golden` | `0c-testdata/golden/zh/cases.json` 的 `syllables`／`bare`／`keys` | 68 |
| `zh-expected` | `0c-testdata/golden/zh/expected/*.jsonl` 的 `# step`，含 `key <` 標點 | 2（其餘與 zh-golden 重複，已合併來源） |
| `mixed-golden` | `0c-testdata/golden/mixed/cases.json` 的 `input` | 5（另 2 筆與 mixed-tests 重複） |
| `ja-modern-500` | `0c-testdata/ja/modern_500.json` 的 `reading`（かな → 羅馬字） | 500 |
| `english-words` | `English::load_default()` 詞表：200 個高權重單詞、99 個二字片語、50 個三字片語 | 349 |
| `english-tests` | `crates/jc-core/src/mixed.rs`、`docs/reports/mixed-latin-2026-11-20.md` 的英文例句 | 4 |
| `mixed-tests` | `crates/jc-core/src/mixed.rs` 的真機／測試案例（`5k42. g4n0 vul3` 等） | 10 |

英文片語是從真實詞表取 1–3 個詞依固定順序組合，不是自編句子；每筆的 `source` 是
`english/words.bin#word1+word2`。日文 `modern-500` 的 reading 用
`default_romaji_table()` 反查（`ん` 補 `n`、`ー` 補 `-`），每筆可追溯到 `modern-001`
之類的題庫 id。

## 3. 總覽

| 指標 | 數量 | 比例 |
|---|---:|---:|
| 量測筆數 | 938 | 100% |
| 一致 | 258 | 27.5% |
| 不一致 | 680 | 72.5% |
| A 無法解析 | 509 | 54.3% |
| A 有結果 | 429 | 45.7% |
| A 有結果且一致 | 258 | 60.1%（以 A 有結果為分母） |
| A 有結果且不一致 | 171 | 39.9%（以 A 有結果為分母） |
| B 轉換失敗 | 0 | 0% |

各來源（去重後以第一次出現的來源計）：

| 來源 | 筆數 | A 無法解析 | 一致 | 不一致 |
|---|---:|---:|---:|---:|
| `ja-modern-500` | 500 | 500 | 0 | 500 |
| `english-words` | 349 | 0 | 199 | 150 |
| `zh-golden` | 68 | 1 | 50 | 18 |
| `mixed-tests` | 10 | 3 | 4 | 6 |
| `mixed-golden` | 5 | 2 | 3 | 2 |
| `english-tests` | 4 | 1 | 2 | 2 |
| `zh-expected` | 2 | 2 | 0 | 2 |

**去掉日文 500 題後**：438 筆中一致 258、不一致 180（41.1%），A 無法解析只剩 9 筆
（2.05%，佔不一致 5.0%）。也就是說，上一輪「A 根本不吃輸入」的失真在中文／英文真實
語料裡大幅消失；剩下的不一致主要是邊界，不是解析失敗。

## 4. 不一致分類（依真實頻率排序）

主分類；同一案例可能命中多桶，完整 `signals` 在 `--json`／`--cases` 裡。

| 分類 | 數量 | 佔不一致 | A 有結果 |
|---|---:|---:|---:|
| `a-none` | 505 | 74.26% | 0 |
| `space-merged-segments` | 148 | 21.76% | 148 |
| `space-tone-vs-separator` | 21 | 3.09% | 21 |
| `atom-uppercase-fullwidth` | 2 | 0.29% | 0 |
| `mid-run-symbol` | 2 | 0.29% | 0 |
| `short-en` | 1 | 0.15% | 1 |
| `trailing-punct-stripped` | 1 | 0.15% | 1 |
| `leading-punct` / `en-word-missing` / `en-word-extra` / `ja-vs-zh` / `ja-vs-en` / `b-empty` / `b-error` / `other` | 0 | 0% | — |

多重訊號（`ja-vs-zh` 5、`short-en` 2、`space-merged-segments` 153 等）都比主分類多，
但它們不是主因；例如 5 個 `ja-vs-zh` 全部先被 `a-none` 收走。

### 前五個最常見的不一致案例（每桶取代表）

1. **`a-none`（505）**：`"asahatenkigaii。"`（`ja/modern_500.json#modern-421`）
   —— A 完全不吃 `。`；B 是 `ja/latin` + `ja/symbol`。
2. **`space-merged-segments`（148）**：`"and for"`（`english/words.bin#and+for`）
   —— A 切成兩個 `en` 片段，B 合成一個 `en/latin`。
3. **`space-tone-vs-separator`（21）**：`"ji3hellot "`
   （`golden/zh/cases.json#mixed-chinese-english-chinese`）
   —— A 的 `zh "t "` 把空白當一聲，B 的 `zh "t"` 不含空白。
4. **`atom-uppercase-fullwidth`（2）**：`"kyouhaAPI"`
   （`crates/jc-core/src/mixed.rs:3546`）—— A 拒絕大寫，B 分出 `en "API"`。
5. **`mid-run-symbol`（2）**：`"ji3<vu;3"`
   （`golden/zh/expected/shift-punct.jsonl:3`）—— A 不吃 `<`，B 切出 `zh/symbol`。

各桶最多 5 個代表案例（含 A/B 片段明細）都在 `/tmp/corpus-diff-human.txt` 的對應命令輸出裡，
或 `--json` 的 `buckets[].examples`。

## 5. 與 `segment-diff-2026-11-21.md` 對照

| 分類 | 上一輪（20000 隨機） | 真實語料（938） | 判讀 |
|---|---:|---:|---|
| `a-none` | 54.94% | 74.26% | **變更重要**，但全部來自日文 500 題的句點；中英子集只有 2.05% |
| `space-merged-segments` | 1.21% | 21.76% | **大幅變重要**：真實英文是多詞片語，B 跨空白合併而 A 逐 run 切 |
| `space-tone-vs-separator` | 9.21% | 3.09% | 仍存在，但下降到真實中文句子中的少數案例 |
| `atom-uppercase-fullwidth` | 22.76% | 0.29% | **降權**：只有 `kyouhaAPI`、`kubectl pCloud GitHub` 兩個 |
| `mid-run-symbol` | 8.26% | 0.29% | **降權**：只剩 shift 標點 `<` 的兩個案例 |
| `short-en` | 0.83% | 0.15% | 降權，仍有 `version go` 一例 |
| `trailing-punct-stripped` | 0.89% | 0.15% | 降權，只剩 `hello.` |
| `leading-punct` | 0.09% | 0 | 消失 |
| `en-word-missing` / `en-word-extra` | 0.05% | 0 | 消失 |
| `ja-vs-zh` / `ja-vs-en` | 1.75% | 0（訊號 6） | 主分類消失，只留在多重訊號 |
| `b-empty` / `b-error` / `other` | 0 | 0 | 不變 |

上一輪 91% 的「不一致」被山谷資料灌高；真實語料仍為 72.5%，但其中 74% 是 A 不吃
日文句點與空白的結構性限制，不是分詞 bug。去掉日文後，39.9% 的「A 有結果不一致」
裡，`space-merged-segments` 與 `space-tone-vs-separator` 合計 169/180 = 93.9%。

## 6. 結論

- 上一輪最大的兩桶 `atom-uppercase-fullwidth`、`mid-run-symbol` 在真實語料裡幾乎消失，
  當初不值得為它們改行為。
- 真正值得修的是空白語意：A 把空白當 run 邊界／一聲，B 把空白兩側合成同一片段。
  這兩桶在真實中英語料裡佔不一致的 24.8%（169/680），在中英子集裡佔 93.9%。
- 若目標是讓 A/B 對真實輸入一致，`unify-segment` 應優先統一空白處理；
  只修上一輪的標點／大寫案例，對真人輸入幾乎沒有幫助。
- 本 task 沒有改 `mixed.rs`、`mixed_latin.rs`、`split_cmd.rs`，也沒有改任何既有子命令
  的行為；只新增 `corpus-diff`、`docs/port/real-corpus.md` 與本報告。
