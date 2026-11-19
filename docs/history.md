# 專案歷史與交接

這份文件是給「在 DGX 上新開 session 的人」看的：說明 `jc-ime` 為何存在、
前身 `0c-ime` 做到哪裡、哪些資產可直接沿用、哪些坑已經踩過。

最後更新：2026-11-19

---

## 1. 一句話

`0c-ime` 是一個能在 Windows 上輸入注音、日文與英文的輸入法，功能可用但架構
與工具鏈留下問題；`jc-ime` 是其核心以 Rust 重寫、平台整合改用獨立 wrapper 的
版本。

---

## 2. 前身 `0c-ime` 的現況

| 項目 | 值 |
|---|---|
| Repo | `github.com/jeffpeng3/0c-ime`（AGPL-3.0） |
| 分支 | `main` @ `8f443af` |
| CI | Windows／Checks／Golden 全綠 |
| 安裝包 | 由 CI 產出，`setup.exe` 453 MB、payload 700 MiB |
| 原始碼 | Go 195k 行（產品 106k、測試 89k）、Rust 19k、JS 4.5k、Python 21.7k（僅建置） |
| 本機路徑 | `/home/fedora/0c-ime` |
| 計畫文件 | `/home/fedora/.opencode/plan/0c-ja/phase2/` |

`0c-ime` 在 2026-11-17 至 11-19 三天內完成 Phase 2：日文資料、模型接線、
Windows 打包、CI、Mozc 詞典。相對分支點新增 546 個檔案、176,751 行。

---

## 3. 前身達到的品質基準（比較用）

以下數字是 `0c-ime` 的結果，不是 `jc-ime` 的現況；現況見
[`docs/status.md`](status.md)。

### 3.1 注音

| 項目 | 數值 |
|---|---|
| Golden | 45 個案例全數一致 |
| 注音評測 | 首選約 86% |

### 3.2 日文（凍結題庫，NBest=5）

以 Mozc 詞典 + context-ID 連接矩陣的結果：

| 題庫 | top-1 | top-5 | 邊界 exact |
|---|---:|---:|---:|
| `modern-500` | 86.0% | 98.0% | 88.2% |
| `anthy-corpus1` | 62.0% | 84.5% | 35.2% |
| `hard-cases` | 54.3% | 97.1% | 68.6% |
| `ajimee-200` | 46.0% | 65.5% | 不計 |

對照：同一引擎使用 UniDic public-core 詞典時依序為 0.0%／1.0%／2.9%／1.0%。

### 3.3 延遲（同機量測）

| 情境 | p50 |
|---|---:|
| 日文統計解碼 | 1.7–4.7 ms |
| reranker N=5（含模型） | 約 52 ms |
| reranker N=10 以上 | 線性上升且預設 100 ms 逾時，不建議 |

---

## 4. 為什麼重寫

| 事件 | 影響 |
|---|---|
| CI 產出的 MSVC TSF DLL（793 KB）在 Windows 11 26100 上，開系統匣必讓 Explorer 崩潰（`windowsudk.shellcommon.dll` c0000005） | 出貨包不可用；mingw 版（3.8 MB）正常 |
| Go cgo 在 Windows 只能用 mingw，Rust 卻是 MSVC | 工具鏈被迫混用，難解釋與驗證 |
| 安裝包漏 `0c-settings.exe`、`bin\` 缺 `0c-server.exe` | 使用者實際踩到「設定打不開」「模型頁找不到引擎」 |
| 設定程式與引擎的路徑假設不一致（`Zuin\data` vs 版本目錄） | 模型頁報錯，且難從 log 判斷 |
| 功能與平台細節高度耦合在單一 Go 服務中 | 無法在 Linux 上完整開發與評測 |

結論：不是功能不夠，是**耦合與工具鏈**讓每次修改都要回到 Windows 驗證，成本
過高。

---

## 5. 已做出的決策

| 主題 | 決策 | 理由 |
|---|---|---|
| 核心語言 | Rust | 無 GC、可嵌入、MSVC 原生、`ort` 直接接 ONNX |
| 平台策略 | Windows 先做，其他平台之後 | 使用者實際需求 |
| 整合方式 | 核心為獨立行程，wrapper 極薄 | 崩潰隔離、核心可在 DGX 開發 |
| 抽象化 | `jc-platform` trait，核心不碰 OS API | 未來 Fcitx5／macOS 只補 wrapper |
| 腳本 | 允許 Python | 建置與資料處理，不在執行期 |
| 模型 | 沿用既有 ONNX（`jeffpeng3/0c-ja-reranker`、`0c-ja-punct`、注音 select／punct） | 語言無關，已驗證 |
| 日文詞典 | Mozc OSS 3.35.6354（BSD-3-Clause＋NAIST/IPAdic/ICOT/沖繩） | 唯一能達到可用品質的來源 |
| 注音詞典 | 沿用 `0c-ime` 上游資料 | 前身已達 86% |
| 資料存放 | 不進 git；HF／上游 release＋雜湊固定 | 可重現、repo 不臃腫 |
| 授權 | AGPL-3.0-only | 與前身一致，便於搬資料與題庫 |

### 被否決的路線（不要再試）

| 路線 | 為什麼不行 |
|---|---|
| 調整詞成本 scale／offset | 只把 modern-500 從 0.0% 推到 1.6%，hold-out 不動；有數學上限證明 |
| 用 UniDic POS 聚合連接矩陣 | modern-500 只有 28%，context ID 才有 86% |
| 接上 `0c-ime` 的 `lm.bin` | 只有 51 unigram／16 bigram，沒有作用 |
| reranker 擴大候選到 N=100 | 正確答案只有 19% 在候選內，且延遲不可接受 |
| 全 MSVC（含 Go） | Go cgo 只支援 gcc；除非重寫 ONNX 綁定 |

---

## 6. 可沿用的資產

### 6.1 模型（ONNX，不變）

| 角色 | 來源 | 備註 |
|---|---|---|
| 日文 reranker | `jeffpeng3/0c-ja-reranker` rev `7450e10f` | CC BY-SA 4.0；int8 |
| 日文標點 | `jeffpeng3/0c-ja-punct` rev `ff9166ab` | MIT；int8 |
| 注音選字 | `FunML/0c-sequence-d512` @ `3533e34e` | MIT；fp32 130 MB，可量化 |
| 注音標點 | `FunML/0c-punct-d512` @ `105c2770` | MIT；fp32 130 MB，可量化 |

兩個日文 repo 只公開 `manifest.json`、`vocab.json` 與 ONNX 權重，沒有
`tokenizer.model`。`jc-model` 因此直接讀 `vocab.json`（`0c.sentencepiece.vocab/1`），
用最長前綴匹配加 byte fallback，不做 unigram Viterbi；`vocab.json` 只有
`id_to_piece`，沒有 piece log-probability。注音兩個模型目前尚未下載。

### 6.2 資料

| 資料 | 來源 | 大小 |
|---|---|---|
| 日文詞典＋連接矩陣 | Mozc OSS `dictionary_oss` 3.35.6354，commit `0136c357` | pack v2 85,508,727 bytes |
| 注音詞典 | `0c-biggo/0c-ime` release `v0.2.12` | `lexicon.bin` 8,791,416 bytes |
| 注音 bigram | libchewing-data `dict/chewing_v4/bigram_p50.arpa`，commit `e5d726d9` | `lm.bin` 32,879,418 bytes |
| 簡繁字典 | OpenCC 1.4.2 | `opencc/` |
| 英文詞表 | 隨注音資料 | 1.5 MB |

### 6.3 題庫（最重要的資產）

位於資料根目錄的 `0c-testdata`：

```text
0c-testdata/ja/      modern-500、hard-cases、anthy-corpus1、ajimee-200
0c-testdata/eval/    注音與綜合評測、ja-lexicon-ambiguous
0c-testdata/fuzzy/   模糊音題庫（919 題）
0c-testdata/golden/  注音 golden 45 例、mixed golden
```

新引擎必須先能在同一批題目上重現第 3 節的數字，才談新功能。

---

## 7. 前身的架構（可借鏡）

```text
TSF DLL (Rust) ──named pipe──> 0c-server (Go) ──ONNX──> 模型
```

- 協定：長度前綴 JSON，快照式
- 這個「核心獨立行程」的形狀是對的，`jc-ime` 沿用，只把核心換成 Rust 並把
  wrapper 做得更薄
- 前身的問題在於核心同時綁了 zh/ja/en、詞典、模型與平台假設，難以在 Linux
  完整驗證

---

## 8. 前身已踩過的坑（直接寫進設計）

| 坑 | 對策 |
|---|---|
| TSF DLL 在 explorer 內崩潰 | wrapper 只做轉發；核心獨立行程；DLL 以 MSVC 建置並靜態連結 CRT |
| 安裝包漏檔案 | 打包後以清單驗證，缺檔即讓 CI 失敗 |
| 設定程式找不到引擎／資料 | 由 wrapper 明確傳入版本目錄；不要用 `LOCALAPPDATA\Zuin\data` 這種猜測 |
| 兩個版本號相同的安裝並存，選單仍顯示舊字串 | 版本字串不可作為判斷依據；改以 build commit 或檔案路徑 |
| 詞典格式重複（bin＋tsv＋words） | 新格式只留一種，載入失敗就回報，不做多格式 fallback 的隱性負擔 |
| Fedora 的 NSIS 套件損壞 | 打包在 Windows runner 上做，不在 Linux |
| CI 不下載模型的老政策 | 新專案明確允許：下載但只驗雜湊，不在 CI 跑推論 |
| 模型輸出與執行期參數不一致（`-data` 期望 TSV） | 協定與資料格式由同一份程式碼定義，不靠慣例 |

---

## 9. 2026-11-19：M0–M2 完成

一天內從空 workspace 走到可評測的注音與日文核心。合併順序、衝突與每次量測
都記在 [`docs/integration-log.md`](integration-log.md)。

| 項目 | 結果 |
|---|---|
| workspace | 10 個 crate；`jc-core` 約 1.7 萬非空白行 |
| 注音 golden | 41/45；剩 4 例是學習依賴 |
| 注音模糊音 | 919 題，top-1 16.4%；注音 319 題 44.5%（前身 40.4%） |
| 日文 `modern-500` | top-1 86.0%、top-5 98.0%、boundary exact 88.2%、p50 3.216 ms |
| 日文 `hard-cases` | top-1 54.3%、top-5 97.1%、boundary exact 71.4% |
| 日文詞典 | pack v2：1,220,427 筆、983,412 surfaces、2,672 context |
| 模型 | reranker int8 32 ms、標點 2 ms（單次推論）；尚未接進核心 |

### 這一天踩到的坑

| 坑 | 影響與處理 |
|---|---|
| blaze 的 user namespace sandbox | blaze 用 user namespace 建 sandbox；這個環境可正常建立，遇到失敗要回報，不可 `BLAZE_SANDBOX=off` 繞過。blaze 只是加速器，交件仍以 `cargo check` 為準（`docs/blaze.md`）。 |
| pack v1 沒有 context ID | v1 的 offset 30 是 reserved，entry 也沒有 left／right ID；只有 v2 有 2,672 個 context 與 `2,672²` 的 i16 矩陣。同一支解碼器換 v1 pack，modern-500 top-1 從 86.0% 掉到 0.0%，證明走的是不同路徑，不是靜默 fallback。 |
| golden fixture 與真實詞典的差別 | 注音 golden 讀凍結的 `lexicon.tsv`＋`lm.tsv`，不讀 `build-zh/*.bin`。bigram 重建後，兩次 golden 都是 27/45（同一份 baseline），不能拿來說 bigram 沒用；`convert --data` 才是讀真實資料包的路徑。 |
| 評測資料源換了，不是解碼器變了 | `jc-cli eval ja` 從 23.2% 升到 86.0%，差別是資料源從 curated `ja-src` 換成 Mozc v2 pack；同一個入口換 pack 才是有效的對照。 |
| 題庫 reference 綁定 public-core | `ja-lexicon-ambiguous` 的 reference 是 public-core 成本最低詞面。v2 Mozc pack 的 30.2% 只能跟自己比，不能宣稱比 public-core 的 52.2% 退步 22 個百分點。 |
| 模型 vocab 缺 piece 分數 | `vocab.json` 只有 `id_to_piece`，沒有 log-probability；tokenizer 改用最長前綴匹配＋byte fallback。與前身 `internal/spm` 的差異已記在 `docs/port/jc-model.md`。 |
| 階段回報與實測不一致 | `zh-golden` 合併時文件寫 41/45，實測是 35/45，總數差 6；補完 zhconv 與 mixed ASCII 推斷後才真的是 41/45。之後每個數字都以指令輸出為準。 |
| 測試依賴外部資料 | 一律用 `jc_testdata::require_data!`；資料不在時跳過，不讓缺 92 MB 詞典變成 CI 紅燈。 |

---

## 10. 里程碑現況

| 階段 | 內容 | 現況 |
|---|---|---|
| M0 | workspace、CI、資料下載、評測 harness | 完成 |
| M1 | 注音核心 | 完成；golden 41/45 |
| M2 | 日文（羅馬字、Mozc、context ID） | 完成 |
| M3 | Mixed 路由＋英文 | 路由與英文直通完成；混合候選與帳號切法未移植 |
| M4 | 模型接線＋個人化 | 模型載入與單次推論完成，未接 `jc-core`；個人化未做 |
| M5 | Windows wrapper（TSF＋候選窗） | 未開始 |
| M6 | 設定、更新、安裝 | 未開始 |

---

## 11. 開發環境

### DGX

| 項目 | 值 |
|---|---|
| 主機 | `dgx`（GB10，128 GB UMA） |
| 核心數 | 20 |
| Rust | 1.99.1 |
| 網路 | GitHub、Hugging Face 可通 |
| 專案路徑 | `/mnt/work/jc/jc-ime` |
| 資料路徑 | `/mnt/work/jc/jc-ime-data/`（不進 git） |

### 本機（Windows 測試）

- Windows 11 26100，已可安裝／解除安裝測試包
- 測試成品放 `/mnt/e/freerun/`

---

## 12. 目前缺口

1. 個人化學習：golden 的 4 例、`forget`／tombstone／自訂詞。
2. Windows TSF wrapper、候選窗、named pipe。
3. 設定程式、更新器、安裝器。
4. Mixed 的混合候選清單、↓ 選注音解釋、英數型號／帳號切法。
5. 注音模型下載；`jc-model` 的 `ZhSelect`／`ZhPunct` 維持 `Unavailable`。
6. `jc-core` 與 `jc-model` 的接線（reranker、標點）。

---

## 13. 參考

| 項目 | 位置 |
|---|---|
| 前身 repo（本機） | `/home/fedora/0c-ime`（`github.com/jeffpeng3/0c-ime`，main `8f443af`） |
| 前身 repo（DGX，參考實作） | `/mnt/work/jc/ref/0c-ime`，已 clone 到 `8f443af`，`CGO_ENABLED=0 go build ./...` 通過（Go 1.27.4 arm64） |
| 訓練 repo（DGX） | `/mnt/work/0c`（`main` `9fde9ef`，與本專案不同） |
| 計畫與紀錄 | `/home/fedora/.opencode/plan/0c-ja/phase2/` |
| 前身安裝包（可用但無 Mozc） | `/mnt/e/freerun/0c-0.2.13-preview.1-windows-amd64-preview-setup.exe` |
| 前身完整包（會讓 explorer 崩潰，勿安裝） | `/mnt/e/freerun/preview.2/` |
