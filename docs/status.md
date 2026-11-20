# 目前狀態

最後更新：2026-11-20

這一頁只放現況與可查證的數字。來源是
[`docs/integration-log.md`](integration-log.md) 與 [`docs/port/`](port/)；沒列出來源的
數字不要當成驗收依據。

## 里程碑

| 階段 | 現況 |
|---|---|
| M0 | 完成 |
| M1 注音 | 完成，golden 45/45、fuzzy 44.5% |
| M2 日文統計 | 完成，`modern-500` 86.0%／`hard-cases` 54.3%／`ajimee-200` 46.0% |
| M2 日文神經 | 完成，Jinen small 門檻混合，`modern-500` 93.6%／`hard-cases` 80.0%／`ajimee-200` 71.0% |
| M3 Mixed | 完成，6/6 |
| M4 模型與個人化 | 完成，注音選字 Q8_0、reranker 棄用、個人化三個機制 |
| M5 wrapper | 引擎端完成（named pipe、client crate）；Windows TSF 未開始 |
| M6 設定／更新／安裝 | 設定只有資料模型，引擎尚未讀取設定檔；更新器不做；安裝器未做 |

## 數字

| 項目 | 數值 | 來源 |
|---|---|---|
| crate | 15 個 workspace 成員 | `Cargo.toml` 的 `members`，`grep -c 'crates/jc-'` |
| `jc-core` | 68 個 `.rs`、44,298 行、40,923 非空白行 | `find crates/jc-core/src -name '*.rs'` 加 `wc -l`／`grep -cv '^[[:space:]]*$'` |
| 注音 golden | 45/45 | `docs/integration-log.md` 最終 gate |
| 注音 fuzzy | 919 題 top-1 16.4%；注音 319 題 44.5%，日文 600 題 1.5% | `docs/port/eval-fuzzy.md` |
| 日文 `modern-500`（統計） | top-1 86.0%、top-5 98.0%、boundary exact 88.2%、p50 3.072 ms | `docs/port/ajimee-eval.md` |
| 日文 `hard-cases`（統計） | top-1 54.3%、top-5 97.1%、boundary exact 71.4%、p50 0.738 ms | `docs/port/ajimee-eval.md` |
| 日文 `ajimee-200`（統計） | top-1 46.0%、top-5 65.0%、p50 6.521 ms、timeout 1.0% | `docs/port/ajimee-eval.md` |
| 日文預設（Jinen small 門檻） | `modern-500` 93.6%、`hard-cases` 80.0%、`ajimee-200` 71.0% | `docs/integration-log.md` 最終 gate |
| Mixed | 6/6 | `docs/integration-log.md` 最終 gate |
| Mozc pack v2 | 1,220,427 筆詞條、983,412 surfaces、2,672 context、81.5 MiB | `docs/port/ja-import.md` |
| 注音詞典 | `build-zh/lexicon.bin` 8,791,416 bytes；bigram 版 `build-zh-bigram/lm.bin` 32,879,418 bytes | `docs/port/zh-data.md`、`docs/port/bigram-data.md` |
| 注音選字模型 | candle Q8_0 35,515,136 bytes（33.9 MiB）、推論 p50 9.36 ms、逐對排序一致 99.51%（5439/5466） | `docs/port/candle-zh.md` |
| 日文神經模型 | Jinen v2 small Q5_K_M 81,117,824 bytes（77.4 MiB） | `models/external/jinen-v2-small/manifest.json` |
| 日文 reranker | int8 34,268,510 bytes；單次推論約 32 ms；預設關閉 | `docs/integration-log.md`、`docs/port/jc-model.md` |
| 測試 | 52 targets、1118 passed、0 failed | `docs/integration-log.md` 最終 gate |
| 協定 | `PROTOCOL_VERSION = 3`（v3 凍結） | `docs/proto.md` |

## 題庫

| 題庫 | 題數 | 用途 |
|---|---:|---|
| `zh-golden` | 45 | 注音 golden，含個人化依賴案例 |
| `fuzzy` | 919（注音 319、日文 600） | 模糊音與擾動鍵 |
| `modern-500` | 500 | 日文一般句 |
| `hard-cases` | 35 | 日文歧義與困難句 |
| `ajimee-200` | 200 | AJIMEE-Bench，無左文脈 |

## 已接上的路徑

| 路徑 | 現況 |
|---|---|
| 注音 golden | 讀 `0c-testdata/golden/zh` 的凍結 `lexicon.tsv`＋`lm.tsv`，不讀 `build-zh/lexicon.bin` |
| 注音 bigram | 讀取端在 `jc-data::zh`；`convert`／`bench --data` 可載入 `build-zh`／`build-zh-bigram`，`eval zh-golden` 仍讀凍結 TSV |
| 日文評測 | `ja-eval` 與 `jc-cli eval ja` 讀 Mozc pack v2（mmap）與 context 矩陣；預設統計轉換加上 Jinen small 門檻 |
| 引擎資料 | `jc-engine` 載入注音詞典、日文 pack 與 `models/`；缺模型只警告，維持統計排序 |
| 神經 runtime | `Session` 內有 `NeuralRuntime` worker 執行緒（`jc-core`）；模型載入與注入在 `jc-engine` |
| 傳輸 | `jc-engine` 支援 stdio、Unix domain socket 與 Windows named pipe |
| client | `jc-windows` 可連 named pipe 並完成握手；`jc-linux`／`jc-mac` 是回 `Unsupported` 的佔位 |

## 未完成

- 組字游標與片段編輯：任意位置插入／刪除、選字鎖定片段、`keepCandidates` 重開候選。
- 候選詞性與學習證據權重：`CandidateDetail` 沒有 part-of-speech，learned 只有布林值。
- `ja_live`（逐字変換）沒接神經轉換，仍走自己的路徑。
- 右文脈：Jinen prompt 只送左文脈，`max_context_chars.right` 目前沒有效果。
- 設定接線：`jc-settings` 只有資料模型，`jc-engine` 仍只吃命令列與環境變數。
- Windows TSF wrapper 與候選窗。
- 安裝器；更新器不做。
- 資料打包：詞典與模型目前不隨安裝包出貨。
