# rerank-domain：領域評測集與條件式指標

Date: 2026-11-20
Branch: `rerank-domain`
Worktree: `/mnt/work/0c/wt/rerank-domain`
jc-ime 量測 commit: `f681ed4f3de9a11166a7d8bc817c715529825177`
模型: `jc-ime-data/models/reranker-extra/model.int8.onnx`
（sha256 `edf1afeacb89fa5da3d10462b5d9afd85392a91056634dd9ac60d41e37faeeb3`）
Pack: `ja-lexicon-mozc-v2.pack`
（sha256 `15926f48dca7a6917f067a41f8ec49e2474c11d0cdebce5486a81f51fd0c84c6`）

## 結論

**現行重排模型在領域題庫上沒有淨效果。** 1,500 題中，統計錯的 1,121 題只救回
5 題（0.4%），統計對的 379 題改壞 5 題（1.3%），淨值 0，top-1 同為 25.3%。

分領域只有 finance 為正、medical 為負，幅度都在 0.4 個百分點：

- finance：+0.40pp（救回 1、改壞 0）
- medical：−0.40pp（救回 1、改壞 2）
- legal、it、science、daily：0.00pp（it 與 science 各救回 1 到 2 題，也各改壞
  1 到 2 題）

同一模型在 `modern-500` 的對照是 86.0% → 82.0%（救回 0、改壞 20）。重排會改變
排序，卻沒有把錯的修對，還會弄壞本來對的樣本。

原因是候選召回上限。重排只能在解碼器已產生的候選中重排；六個領域的統計 top-5
只有 19.6% 到 61.2%，多數統計錯誤的正解根本不在前五名。重排無法補回不存在的
候選，模型在領域長句上也沒有展現訓練時的歧義解決能力。

## 題庫

`data/benchmarks/ja-domain-v1/`，6 個領域 × 250 句，共 1,500 句。格式與
`0c-testdata/ja/modern_500.json` 相同：`reading`、`surface`、`reading_segments`、
`segments`、`tags`、`id`。

| 領域 | n | 來源 | 平均句長 | manifest sha256 前 16 碼 |
|---|---:|---|---:|---|
| medical | 250 | KAKEN | 37.6 | `d079d0187963c598` |
| legal | 250 | e-Gov（Heisei／Reiwa） | 33.3 | `53f1662b8f9feba6` |
| it | 250 | KAKEN 137、特許 113 | 40.3 | `663b5992a060bb23` |
| finance | 250 | e-Gov 151、KAKEN 62、SIP PDF 37 | 35.9 | `b2e1cb190d5c0131` |
| science | 250 | KAKEN 175、SIP PDF 75 | 37.6 | `af9622d420ecf5d5` |
| daily | 250 | 青空文庫會話引用 | 30.5 | `1945a3dbe06e5c97` |

完整雜湊與來源分布見 `data/benchmarks/ja-domain-v1/manifest.json`。

建置方式：

1. 從 `public_core_ja` 的 raw 文本切句，長度 18 到 110 字，過濾 ASCII 比例過高、
   舊假名、專利模板與破損行。
2. 領域用關鍵詞密度判定，不用模型分類。分數為命中詞長加權除以句長，命中詞先
   去除被長詞包含的重疊項；每個領域另有最低密度、最少命中詞數與與第二名的差距
   門檻。
3. 讀音用 `fugashi`＋`unidic-lite` 斷詞取得語境讀音，再以 Mozc v2 pack 對每個
   詞條取讀音。pack 沒有同讀音的詞條就整句丟棄，避免產生解碼器無法還原的讀音。
4. 與 `modern-500`、`hard_cases`、`ajimee_200` 共 735 句做 exact 與 4-gram
   Jaccard ≥ 0.65 near-dup 檢查：重疊 0 句。
5. 只用於評測，不寫入任何訓練 split。manifest 記錄 corpus manifest 雜湊
   `eee2b5985bff2d5b94dffa83496fb060de571eaabde7c2bc30f5369cafeddec8`、pack
   雜湊、每檔雜湊與來源分布。

建置統計（manifest `rejects`）：medical 64,484 句未過題材或讀音檢查、legal
252,545、it 199,515、finance 60,978、science 30,530、daily 63,950。精確重複
61,606 句。

## 量測方式

- 統計路徑與重排路徑共用同一份 jc-ime 解碼器與 Mozc v2 pack，只差
  `JaDecoder::with_reranker`。
- 以 jc-ime 的 `nbest(reading, JaContext::default(), 5)` 取前五名，避免不同候選
  生成造成差異。
- 條件式指標：
  - 統計錯樣本數 = 統計 top-1 未命中。
  - 救回 = 統計錯且重排 top-1 命中。
  - 統計對樣本數 = 統計 top-1 命中。
  - 改壞 = 統計對且重排 top-1 未命中。
- top-1 命中允許漢字數字與阿拉伯數字等價，例如 `第6条` 對 `第六条`。只轉換
  計數詞前的數字，避免把 `一般` 這類詞誤轉。這是排版偏好，不應算成語義錯誤。
- 量測 parity：同一台機器、同一 pack，`modern-500` 的 jc-cli 與本工具都是
  82.0%（重排開）與 86.0%（重排關）。

執行指令：

```sh
uv run --with pyarrow python scripts/build_domain_benchmark.py build
uv run python scripts/build_domain_benchmark.py verify
uv run --with pyarrow python scripts/eval_rerank_conditional.py \
    --out docs/reports/rerank-domain-2026-11-20.json
```

`eval_rerank_conditional.py` 會把 `scripts/rerank_probe.rs` 編成
`.build/rerank-probe`，以 jc-ime 的 crate 為 path dependency；不修改 jc-ime。

## 條件式指標

| 領域 | n | 統計 top-1 | 重排 top-1 | 統計錯 | 救回 | 救回率 | 統計對 | 改壞 | 改壞率 | 淨值 | Δpp |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| medical | 250 | 20.4% | 20.0% | 199 | 1 | 0.5% | 51 | 2 | 3.9% | −1 | −0.40 |
| legal | 250 | 12.8% | 12.8% | 218 | 0 | 0.0% | 32 | 0 | 0.0% | 0 | 0.00 |
| it | 250 | 42.8% | 42.8% | 143 | 1 | 0.7% | 107 | 1 | 0.9% | 0 | 0.00 |
| finance | 250 | 33.6% | 34.0% | 166 | 1 | 0.6% | 84 | 0 | 0.0% | +1 | +0.40 |
| science | 250 | 27.6% | 27.6% | 181 | 2 | 1.1% | 69 | 2 | 2.9% | 0 | 0.00 |
| daily | 250 | 14.4% | 14.4% | 214 | 0 | 0.0% | 36 | 0 | 0.0% | 0 | 0.00 |
| **合計** | 1,500 | 25.3% | 25.3% | 1,121 | 5 | 0.4% | 379 | 5 | 1.3% | 0 | 0.00 |

召回上限（統計 top-5）：medical 36.4%、legal 19.6%、it 61.2%、finance 46.4%、
science 45.2%、daily 24.4%。重排 top-5 幾乎不變（medical 34.8%、legal 20.8%、
it 60.8%、finance 46.4%、science 44.0%、daily 24.8%），再次顯示問題在候選召回，
不在排序。

模型實際工作量與 timeout（每次 250 題）：

| 領域 | groups | scored | changed | timeout | p50 | p95 |
|---|---:|---:|---:|---:|---:|---:|
| medical | 327 | 327 | 151 | 0 | 22.7 ms | 46.8 ms |
| legal | 445 | 445 | 107 | 1 | 25.3 ms | 52.6 ms |
| it | 371 | 371 | 212 | 0 | 22.5 ms | 44.4 ms |
| finance | 358 | 355 | 156 | 9 | 21.5 ms | 46.3 ms |
| science | 344 | 335 | 184 | 25 | 27.7 ms | 69.0 ms |
| daily | 204 | 200 | 79 | 10 | 30.5 ms | 60.0 ms |

每次量測最多重跑三次，取 scored 群組最多的一次，避免機器負載造成的 100 ms
timeout 左右品質。timeout 的群組保留統計排序，只會讓改動變少，不會製造假的
救回；上表仍有少量 timeout，但 top-1 與重跑結果一致。

## 各領域判讀

- **finance：唯一正報酬。** 救回 1、改壞 0，+0.40pp。樣本主要是 e-Gov 會計與
  歲入歲出條文，統計錯 166 題，重排只修好 1 題，幅度不足以支持預設開啟。
- **medical：小幅有害。** 救回 1、改壞 2，−0.40pp。KAKEN 醫學摘要的長複合詞
  （例如 `慢性期`、`虛血肢`）在統計路徑常輸給同音詞，重排沒有穩定修回來。
- **legal、it、science：淨零。** it 與 science 各有 1 到 2 題救回，同時也有
  1 到 2 題改壞。legal 完全沒有改動 top-1；統計 top-5 只有 19.6%，重排沒有
  可用的正確候選。
- **daily：淨零，且絕對值最低。** 青空文庫會話是文學日文，14.4% 的統計 top-1
  反映舊語彙與專有名詞，不能代表現代 IME 使用者的日常輸入。這一組只適合作為
  「重排沒有幫助」的對照，不宜當成日常會話難度基準。

## 限制

1. **題庫偏難。** 統計 top-1 只有 12.8% 到 42.8%，比 `modern-500` 的 86.0% 與
   `hard-cases` 的 54.3% 都低。這是刻意的領域外分佈，但結論只適用於這種難度，
   不能外推到一般輸入。
2. **讀音建置會丟句。** pack 沒有同讀音詞條的句子直接剔除，避免用錯讀音製造
   假錯誤。代價是題庫偏向 Mozc 詞典涵蓋的詞彙。
3. **數字等價。** 指標允許 `第6条` 與 `第六条` 視為相同；若不允許，legal 的
   統計 top-1 會再低約 6 個百分點。這是格式差異，不影響重排的救回與改壞計數。
4. **只測一種接線。** 量測的是 jc-ime 目前的詞層級重排與 `reranker-extra`
   int8。沒有測 top-3 gating、seq 240、或重訓後的模型。
5. **timeout 受機器負載影響。** ONNX Runtime 單次推論預算 100 ms，science 的
   p95 到 69 ms、daily 到 60 ms；timeout 只會減少重排改動。量測已用三次重跑
   取 scored 最多的一次，但無法完全消除排程抖動。

## 交付物

- 題庫：`data/benchmarks/ja-domain-v1/{medical,legal,it,finance,science,daily}.json`
- manifest：`data/benchmarks/ja-domain-v1/manifest.json`
- 建置：`scripts/build_domain_benchmark.py`
- 量測：`scripts/eval_rerank_conditional.py`、`scripts/rerank_probe.rs`
- 逐題證據：`docs/reports/rerank-domain-2026-11-20.json`（含每題讀音、統計與
  重排輸出、top-5）
- 原始 JSONL 快取：`.build/rerank-domain/*.jsonl`（不進 git）

## 決策建議

以目前模型與接線，**不要因為「統計會錯」就在領域文字預設開啟重排**。領域題庫
的救回率 0.4%，改壞率 1.3%，淨效果為零；`modern-500` 的 −4.0pp 反而更明確。
若要讓重排真的有用，下一個實驗應先提高候選召回（把正解放進 top-5），再比較
重排；目前的限制在召回，不在排序模型。
