# 目前狀態

最後更新：2026-11-21

這一頁只放現況與可查證的數字。來源是
[`docs/integration-log.md`](integration-log.md) 與 [`docs/port/`](port/)；沒列出來源的
數字不要當成驗收依據。

## 里程碑

| 階段 | 現況 |
|---|---|
| M0 | 完成 |
| M1 注音 | 完成，golden 45/45、fuzzy 44.5% |
| M2 日文統計 | 完成，`modern-500` 86.0%／`hard-cases` 54.3%／`ajimee-200` 46.0% |
| M2 日文神經 | 完成，Jinen small 門檻混合，`auto` 92.2%／`quality` 93.6%（`hard-cases` 80.0%／`ajimee-200` 71.0%） |
| M2 日文預設 | 完成，延遲整句轉換（`ja_conversion = live`）；`modern-500` 92.2%、`--no-live` 92.2%／quality 93.6% |
| M3 Mixed | 完成，6/6 |
| M4 模型與個人化 | 完成，注音選字 Q8_0、reranker 棄用、個人化三個機制 |
| M5 wrapper | 引擎端與 TSF 前端完成，實機驗證待做 |
| M6 設定／更新／安裝 | 設定完成、安裝器完成、更新器不做 |

## 2026-11-21 之後的現況

| 項目 | 現況 | 來源 |
|---|---|---|
| 版本 | `v0.1.0-rc.7` 已 tag（rc.6 跳過），`Cargo.toml` 為 `0.1.0-rc.7` | `docs/integration-log.md` 2026-11-21 02:40、`docs/CHANGELOG.md`、`git tag` |
| TSF 註冊 | CLSID 改由 MSI 的 `<Class>` 寫入，helper 只註冊 profile 與三個分類；反安裝不呼叫 `InstallLayoutOrTip(ILOT_UNINSTALL)` | `docs/integration-log.md` 02:40、02:50，`docs/port/tsf-installer.md` |
| profile 啟用 | `bEnabledByDefault` 改 `true`，`Enable=1`；仍不呼叫 `ActivateProfile`，不改系統預設輸入法 | `docs/integration-log.md` 03:20，`docs/port/tsf-skeleton.md` |
| 量測工具 | `jc-cli` 新增 `mixed-diff`、`corpus-diff`，只加工具，不改 `mixed.rs`／`mixed_latin.rs` | `docs/integration-log.md` 04:40，`docs/reports/segment-diff-2026-11-21.md`、`docs/reports/real-corpus-2026-11-21.md` |
| workflow | `actions/checkout@v5`、`upload-artifact@v6`、`download-artifact@v7`，Node 20 的 annotation 消失 | `docs/integration-log.md` 05:10 |
| 發版快取 | tag scope 讓 Windows job 每次 `No cache found.`；打 tag 前先在 `main` 用 `workflow_dispatch` 暖機 | `docs/release.md`「快取與發版流程」 |
| 資料 catalog | `zh_punct` 從 catalog 移除，manifest 10 筆 → 8 筆（6 個隨附、2 個 `zh_select` 下載項）；`data verify` 人讀輸出隱藏缺席的下載項 | `jc-ime-data/manifest.json`、`docs/release.md` |

## 數字

| 項目 | 數值 | 來源 |
|---|---|---|
| crate | 18 個 workspace 成員 | `Cargo.toml` 的 `members`，`grep -c 'crates/jc-'` |
| `jc-core` | 72 個 `.rs`、52,531 行、48,666 非空白行 | `find crates/jc-core/src -name '*.rs'` 加 `wc -l`／`grep -cv '^[[:space:]]*$'`，2026-11-21 重算 |
| 注音 golden | 45/45 | `docs/integration-log.md` 最終 gate |
| 注音 fuzzy | 919 題 top-1 16.4%；注音 319 題 44.5%，日文 600 題 1.5% | `docs/port/eval-fuzzy.md` |
| 日文 `modern-500`（統計） | top-1 86.0%、top-5 98.0%、boundary exact 88.2%、p50 3.072 ms | `docs/port/ajimee-eval.md` |
| 日文預設（延遲整句） | `modern-500` 92.2%、0.21 推論／題、停頓 p95 50.4 ms、已穩定文字變動 0% | `docs/reports/live-default-2026-11-20.md` |
| 日文 `--no-live` | `modern-500` 92.2%、`--neural quality` 93.6% | 同上 |
| 日文 `hard-cases`（統計） | top-1 54.3%、top-5 97.1%、boundary exact 71.4%、p50 0.738 ms | `docs/port/ajimee-eval.md` |
| 日文 `ajimee-200`（統計） | top-1 46.0%、top-5 65.0%、p50 6.521 ms、timeout 1.0% | `docs/port/ajimee-eval.md` |
| 日文預設（`--neural auto`） | `modern-500` 92.2%（p95 57.5 ms）、`hard-cases` 77.1%、`ajimee-200` 69.0% | `docs/integration-log.md` gating-lowcall |
| 日文 `--neural quality` | `modern-500` 93.6%、`hard-cases` 80.0%、`ajimee-200` 71.0% | `docs/integration-log.md` 最終 gate |
| 日文 `--neural off` | `modern-500` 86.0% | `docs/integration-log.md` 最終 gate |
| Mixed | 6/6 | `docs/integration-log.md` 最終 gate |
| Mozc pack v2 | 1,220,427 筆詞條、983,412 surfaces、2,672 context、81.5 MiB | `docs/port/ja-import.md` |
| 注音詞典 | `build-zh/lexicon.bin` 8,791,416 bytes；bigram 版 `build-zh-bigram/lm.bin` 32,879,418 bytes | `docs/port/zh-data.md`、`docs/port/bigram-data.md` |
| 注音選字模型 | candle Q8_0 35,515,136 bytes（33.9 MiB）、推論 p50 9.36 ms、逐對排序一致 99.51%（5439/5466） | `docs/port/candle-zh.md` |
| 日文神經模型 | Jinen v2 small Q5_K_M 81,117,824 bytes（77.4 MiB） | `docs/models.md` |
| 日文 reranker | int8 34,268,510 bytes；單次推論約 32 ms；預設關閉 | `docs/integration-log.md`、`docs/port/jc-model.md` |
| 測試 | 54 targets、1301 passed、0 failed | `docs/integration-log.md` 2026-11-21 05:10 gate |
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
| 日文評測 | `ja-eval` 與 `jc-cli eval ja` 讀 Mozc pack v2（mmap）與 context 矩陣；預設 `auto` 門檻混合 |
| 引擎資料 | `jc-engine` 載入注音詞典、日文 pack 與 `models/`；缺模型只警告，維持統計排序 |
| 神經 runtime | `Session` 內有 `NeuralRuntime` worker 執行緒（`jc-core`）；模型載入與注入在 `jc-engine` |
| 傳輸 | `jc-engine` 支援 stdio、Unix domain socket 與 Windows named pipe；Windows pipe 名稱預設 `jc-ime-<SID 尾段>` |
| client | `jc-windows` 連 named pipe 並完成握手；`jc-linux`／`jc-mac` 是回 `Unsupported` 的佔位 |
| TSF | `jc-tsf`（x64 cdylib）組字、候選窗、滑鼠點選與虛線底線；`jc-tsf-smoke` 免管理員檢查 |
| 設定 | `jc-engine` 啟動讀 `settings.json`，CLI 參數優先；`jc-settings-ui` 讀寫同一個檔 |
| 安裝 | `installer/jc-ime.wxs` 與 `build.ps1` 產出 per-machine MSI，含授權文件與 TIP 註冊 |

## 未完成

- 實機驗證：Windows 安裝、升級、反安裝、UWP／Office 相容性，以及 x64 TSF 的日常使用。
- 簽章：目前產出未簽章 MSI，SmartScreen 與企業白名單會擋。
- 逐字変換預設化：`ja_live` 已接 Jinen，但預設關閉。
- 右文脈：Jinen prompt 只送左文脈，`max_context_chars.right` 目前沒有效果。
- 目前版本 `v0.1.0-rc.7`（rc.6 跳過，tag 已建立）；資料包上傳與 release 演練見
  `docs/release.md` 的第一次發版確認清單；x86 TSF 未建置。
