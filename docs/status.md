# 目前狀態

最後更新：2026-11-19（M0–M2）

這一頁只放現況與可查證的數字；後續每個里程碑更新同一頁。數字來源是
[`docs/integration-log.md`](integration-log.md) 與 [`docs/port/`](port/)。

## 里程碑

| 階段 | 內容 | 狀態 |
|---|---|---|
| M0 | workspace、協定、資料讀取、評測 harness | 完成 |
| M1 | 注音核心 | 完成；golden 41/45（4 例依賴個人化），模糊音 919 題可跑 |
| M2 | 日文（羅馬字、Mozc、context ID） | 完成 |
| M3 | Mixed 路由＋英文 | 路由、英文直通與前綴補完完成；混合候選清單、帳號／型號切法未移植 |
| M4 | 模型接線＋個人化 | 模型載入與單次推論完成，尚未接進 `jc-core`；個人化學習未做 |
| M5 | Windows wrapper（TSF＋候選窗） | 未開始 |
| M6 | 設定、更新、安裝 | 未開始 |

## 數字

| 項目 | 數值 | 來源 |
|---|---|---|
| 注音 golden | 41/45；失敗 4 例全是學習依賴 | `docs/integration-log.md`、`docs/port/zh-golden-rest.md` |
| 日文 `modern-500` | top-1 86.0%、top-5 98.0%、boundary exact 88.2%、p50 3.216 ms | `docs/port/ja-context.md` |
| 日文 `hard-cases` | top-1 54.3%、top-5 97.1%、boundary exact 71.4%、p50 0.747 ms | `docs/port/ja-context.md` |
| 日文 `ja-lexicon-ambiguous`（holdout 400） | v2 Mozc pack top-1 30.2%、top-5 58.5% | `docs/port/eval-ja-ambiguous.md` |
| 同題庫的 public-core 對照 | top-1 52.2%、top-5 93.8%；題庫以 public-core 建構，不可與 v2 直接比較 | `docs/port/eval-ja-ambiguous.md` |
| Mozc pack v2 | 1,220,427 筆詞條、983,412 surfaces、2,672 context、81.5 MiB | `docs/port/ja-import.md` |
| 注音模糊音 | 919 題 top-1 16.4%；注音 319 題 44.5%，日文 600 題 1.5% | `docs/port/eval-fuzzy.md` |
| 注音詞典 | `build-zh/lexicon.bin` 8,791,416 bytes；bigram 版在 `build-zh-bigram/`（`lm.bin` 32,879,418 bytes） | `docs/port/bigram-data.md`、`docs/port/zh-data.md` |
| crate | 10 個；`jc-core` 約 1.7 萬非空白行 | `Cargo.toml`、`crates/jc-core/src` 的 `wc` |
| 模型 | reranker int8 32 ms、標點 2 ms（單次推論）；注音模型未取得 | `docs/integration-log.md`、`docs/port/jc-model.md` |

## 已接上的路徑

| 路徑 | 現況 |
|---|---|
| 注音 golden | 讀 `0c-testdata/golden/zh` 的凍結 `lexicon.tsv`＋`lm.tsv`，不讀 `build-zh/lexicon.bin` |
| 注音 bigram | 讀取端在 `jc-data::zh`；`convert`／`bench --data` 可載入 `build-zh`／`build-zh-bigram`，`eval zh-golden` 仍讀凍結 TSV |
| 日文評測 | `ja-eval` 與 `jc-cli eval ja` 讀 Mozc pack v2（mmap）與 context 矩陣；v1 退回 POS 成本 |
| 模型 | `jc-model` 的 `probe` 可載入 reranker 與標點；`jc-core`、`jc-engine` 沒有依賴 `jc-model` |
| 引擎 | `jc-engine` 支援 stdio 與 Unix domain socket；`Session` 未載入詞典時只回讀音 |

## 未完成

- 個人化學習：golden 的 4 例、`forget`／tombstone／自訂詞。
- Windows TSF wrapper、候選窗、named pipe。
- 設定程式、更新器、安裝器。
- Mixed 的混合候選清單、↓ 選注音解釋、英數型號／帳號切法。
- 注音模型下載（`Role::ZhSelect`／`Role::ZhPunct` 維持 `Unavailable`）。
- `jc-core` 與 `jc-model` 的接線，以及把注音 pack 接進 session／引擎。
