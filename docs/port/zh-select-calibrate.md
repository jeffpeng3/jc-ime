# zh-select-calibrate：校準注音選字模型

用 fuzzy 題庫的注音題建立 dev 集，掃 `weight × margin`。golden 45 題只做最後
驗收，沒有進入 dev 或掃參。

## 結論

- dev 上「統計錯、模型救回」的組數是 **0**，所有掃描點的救援數都是 0。
- 模型單獨排序在主 dev 是 46/46，統計首選錯 0 組；瓶頸是 dev 沒有可救的
  案例，不是融合公式把模型壓住。
- 任務網格中破壞最少的是 `weight = 0.5`、`margin = 3.0`：主 dev 破壞 0，
  噪音壓力 dev 破壞 2/161。
- 2/161 是 1.24%，超過 1% 上限，因此**預設維持關閉**。
- 開啟模型的 golden 仍是 45/45，關閉也是 45/45。
- `SelectConfig::default()` 寫入 `weight = 0.5`、`margin = 3.0`；
  `MIN_OVERRIDE_MARGIN` 保留但不再參與重排。

## Dev 集

`zh-calibrate` 只讀 `fuzzy/fuzzy-input.json` 的注音題，取 2 個音節以上，
以 `(讀音, 正確答案)` 去重。候選來自 `select` 模糊音模式的讀音變體，和
`eval fuzzy` 的注音候選同一條路徑；詞典同音組自然包含在 N-best 內。

| 模式 | 讀音 | 變體 | 組數 | 答案在候選內 | 統計首選正確 |
|---|---|---|---|---|---|
| 主 dev | fuzzy 的 `reading` | 近音展開 | 46 | 46 | 46 |
| 壓力 dev | fuzzy 的 `noisy_reading` | 近音展開 | 161 | 76 | 75 |

主 dev 是任務指定的跑法。壓力 dev 把使用者實際打錯的讀音交給模型，用來看
模型會不會把錯字推向打錯的音。

兩份 dev 的統計首選幾乎全對，原因是 fuzzy 的注音題來自 0c 引擎自己的詞
片段，答案本來就是引擎切出來的詞。可達答案中只有 1 組統計首選錯：

| 題目 | 讀音 | 正確答案 | 統計首選 |
|---|---|---|---|
| `zh/long-sentence-8/2/wrong-key` | ㄕㄢ ㄕㄤˇ | 山上 | 三上 |

這一組模型給「山賞」最高分（-0.022），比「山上」（-8.959）高，權重再大也
救不回來。因此 dev 上沒有可救回的案例。

## 掃描結果

### 主 dev：46 組

`weight ∈ {0.5, 1, 2, 3}`、`margin ∈ {0, 0.25, 0.5, 1, 1.5, 2, 3}` 的
28 個點結果完全相同：

| 指標 | 數值 |
|---|---|
| top-1 | 46/46（100%） |
| 救回 | 0 |
| 改壞 | 0 |
| 覆寫 | 0 |

模型在乾淨讀音上不曾推翻統計首選。這個 dev 沒有任何鑑別力，只能證明
開啟模型不會破壞已知答案。

### 壓力 dev：161 組

| weight | margin | top-1 | 救回 | 改壞 | 覆寫 |
|---|---|---|---|---|---|
| 0.50 | 3.00 | 45.3% | 0 | 2 | 11 |
| 0.50 | 2.00 | 44.7% | 0 | 3 | 15 |
| 0.50 | 1.50 | 44.7% | 0 | 3 | 15 |
| 0.50 | 1.00 | 44.1% | 0 | 4 | 17 |
| 0.50 | 0.50 | 44.1% | 0 | 4 | 20 |
| 0.50 | 0.25 | 42.2% | 0 | 7 | 24 |
| 0.50 | 0.00 | 41.6% | 0 | 8 | 28 |
| 1.00 | 3.00 | 35.4% | 0 | 18 | 32 |
| 1.00 | 2.00 | 34.2% | 0 | 20 | 36 |
| 1.00 | 1.50 | 33.5% | 0 | 21 | 39 |
| 1.00 | 1.00 | 33.5% | 0 | 21 | 40 |
| 1.00 | 0.50 | 33.5% | 0 | 21 | 43 |
| 1.00 | 0.25 | 33.5% | 0 | 21 | 44 |
| 1.00 | 0.00 | 32.9% | 0 | 22 | 46 |
| 2.00 | 3.00 | 32.3% | 0 | 23 | 43 |
| 2.00 | 2.00 | 31.1% | 0 | 25 | 47 |
| 2.00 | 1.50 | 31.1% | 0 | 25 | 48 |
| 3.00 | 3.00 | 30.4% | 0 | 26 | 50 |
| 3.00 | 2.00 | 30.4% | 0 | 26 | 53 |
| 3.00 | 1.50 | 30.4% | 0 | 26 | 53 |
| 3.00 | 1.00 | 30.4% | 0 | 26 | 55 |
| 2.00 | 1.00 | 30.4% | 0 | 26 | 52 |
| 3.00 | 0.50 | 30.4% | 0 | 26 | 55 |
| 2.00 | 0.50 | 30.4% | 0 | 26 | 55 |
| 3.00 | 0.25 | 30.4% | 0 | 26 | 56 |
| 2.00 | 0.25 | 30.4% | 0 | 26 | 55 |
| 3.00 | 0.00 | 30.4% | 0 | 26 | 56 |
| 2.00 | 0.00 | 30.4% | 0 | 26 | 55 |

`weight = 0.5, margin = 3.0` 的兩個改壞案例都是近音題：

| 題目 | 讀音 | 正確答案 | 模型改成 |
|---|---|---|---|
| `zh/long-sentence-3/2/near-sound` | ㄓㄠˇ ㄕㄤˋ | 早上 | 找上 |
| `zh/long-sentence-8/2/near-sound` | ㄙㄢ ㄕㄤˋ | 山上 | 三上 |

模型看到的是打錯的讀音，于是把字推向符合錯音的候選。這是模型與模糊音
修正方向相反，不是 margin 太小。壓力 dev 上沒有任何一點能同時做到
「救回 > 0」與「改壞 0」。

## 模型單獨排序診斷

這一節不改產品行為，只用來分辨「模型本身排不出正確答案」與「融合公式或
margin 讓模型無法發言」。作法是不看統計成本，直接在合法候選中取模型分數
最高者。

| dev | model-only top-1 | 統計首選錯 | 模型只靠分數救回 |
|---|---|---|---|
| 主 dev（46 組） | 46/46 | 0 | 0 |
| 壓力 dev（161 組，127 組可評分） | 48/161 | 52 | 0 |

壓力 dev 的 52 組統計首選錯裡，只有 1 組的正確答案還在候選內；模型單獨
排序也沒有選它。其餘 51 組的答案不在候選內，模型無法救。

主 dev 的三個關鍵數字是 **46/46、0、0**：模型本身的排序正確率是 100%，
統計首選錯的組數是 0，所以救回數必然是 0。問題不在融合權重或 margin，
是 dev 沒有「統計錯、模型有機會救」的案例。壓力 dev 則相反：模型拿到
打錯的讀音，單獨排序只有 48/161，連唯一可救的 1 組也救不回來。

### 分數方向核對

前身的 `internal/neural/native/input/testdata/golden.json` 只凍結
`prepare()` 的陣列，沒有模型分數；它使用的 17 字元詞表也不在真實模型內，
無法直接送進 ONNX。這裡取其中可評分的案例形狀（`basic`、
`shared_prefix`、`neutral`），換成真實詞表，再補上模型 README 明確標示
排序的 4 個實例，確認正確候選的分數高於錯誤候選：

| 案例 | 正確候選 | 最佳錯誤候選 | 結果 |
|---|---|---|---|
| `golden-basic` | -0.0001 | -9.0579 | 正確較高 |
| `golden-shared-prefix` | -0.0133 | -4.6204 | 正確較高 |
| `golden-neutral` | -0.0000 | -17.0542 | 正確較高 |
| `readme-期中考` | -0.0272 | -4.3061 | 正確較高 |
| `readme-今天天氣很好` | -0.0108 | -4.7118 | 正確較高 |
| `readme-無聲調期中考` | -0.1737 | -3.5900 | 正確較高 |
| `readme-公園` | -0.0008 | -8.7485 | 正確較高 |

7/7 方向正確，沒有分數反向或正負號錯置。

## 選點理由

任務規則是先讓改壞數為 0（或 ≤1%），再最大化救回數。實際結果：

1. 主 dev 28 個點的救回與改壞都是 0，全部同分。同分時取最保守的一端：
   margin 最大、weight 最小，因此選 `0.5 / 3.0`。
2. 壓力 dev 在 `0.5 / 3.0` 仍有 2 個改壞（1.24%），超過 1% 上限；其他點
   更差。網格外 `weight = 0.25, margin = 3.0` 可以做到改壞 0，但救回仍是
   0，不在指定網格內，也沒有理由啟用模型。
3. 救回數為 0，依完成條件維持預設 off。

`0.5 / 3.0` 的意義是「限制誤覆寫」，不是「模型已可用」。要真正改善排序，
需要一份統計首選會錯、答案可由模型救回、且模型拿到正確讀音的 dev 集；
目前的 fuzzy 題庫沒有這種案例。

## 程式變更

- `crates/jc-core/src/zh/select.rs`
  - 新增 `SelectConfig { weight, margin }`，`Default` 是 `0.5 / 3.0`。
  - `DEFAULT_SELECT_WEIGHT`、`DEFAULT_SELECT_MARGIN` 取代原本寫死的 manifest
    權重與 `MIN_OVERRIDE_MARGIN` 下限。
  - `MIN_OVERRIDE_MARGIN = 1.0` 保留給既有呼叫端，但 `rerank` 與
    `rerank_nbest` 已不再使用它。
- `crates/jc-cli/src/bin/zh-calibrate.rs`
  - 建立 dev 集、算統計與模型分數、掃網格、輸出每組 JSONL。
- `crates/jc-cli/Cargo.toml` 不需要新增依賴。

上游 manifest 仍是 `weight = 2.0`、`margin = 0`。這兩個值是 0c 引擎分數
尺度，`SelectConfig` 不沿用。

## 驗收

`eval zh-golden` 的 45 題在開啟與關閉模型時都全對：

| 項目 | 關閉 | 開啟 |
|---|---|---|
| 通過 | 45/45 | 45/45 |
| top-1 | 100% | 100% |
| 推論次數 | 0 | 100 |
| 模型延遲 p50 | 無 | 54.43 ms |
| 模型延遲 p95 | 無 | 67.63 ms |

延遲數字受同一台機器負載影響，只用來確認沒有觸發 100 ms watchdog；本次
`timeout` 是 0%。

## 執行

```sh
export ORT_DYLIB_PATH=/path/to/libonnxruntime.so

# 主 dev：46 組，選點 0.5 / 3.0，救回 0、改壞 0
cargo run -p jc-cli --release --features onnx --bin zh-calibrate -- \
  --dev /mnt/work/jc/jc-ime-data/0c-testdata/fuzzy/fuzzy-input.json \
  --out /tmp/zh-dev.jsonl

# 壓力 dev：用打錯的讀音，161 組
cargo run -p jc-cli --release --features onnx --bin zh-calibrate -- \
  --dev /mnt/work/jc/jc-ime-data/0c-testdata/fuzzy/fuzzy-input.json \
  --reading noisy --out /tmp/zh-dev-noisy.jsonl

# 驗收
cargo run -p jc-cli --release -- eval zh-golden
cargo run -p jc-cli --release --features onnx -- eval zh-golden --select-model on
```

`--exact` 關閉近音展開，只用原讀音建詞格；`--limit`、`--k` 方便縮小檢查。
跑完掃描後會固定印出 model-only top-1、統計首選錯的組數與救回數，以及
分數方向 7 個案例的核對結果；這些診斷不影響 `SelectConfig` 或產品排序。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/score/ranker.go`（權重併入成本） | `crates/jc-core/src/zh/select.rs`（`SelectConfig`、`choose`） | rewritten：改用本引擎校準的 0.5 / 3.0，不再用 manifest 的 2.0 / 0 |
| `core/rerank/rerank.go`（`Choose` 的 margin 語意） | `crates/jc-core/src/zh/select.rs`（`choose`） | ported：首選索引 0、嚴格大於 margin 才覆寫 |
| `internal/neural/native/manifest.go`（operating point） | `crates/jc-core/src/zh/select.rs`（`SelectManifest`） | rewritten：manifest 只保留模型契約，重排設定改由 `SelectConfig` 決定 |
| 前身沒有校準工具 | `crates/jc-cli/src/bin/zh-calibrate.rs` | 新增：fuzzy dev、權重與 margin 掃描 |
| `testdata/golden/zh/cases.json` | `crates/jc-cli/src/eval_cmd.rs`（`eval zh-golden`） | n/a：只當驗收，不進 dev 或掃參 |
