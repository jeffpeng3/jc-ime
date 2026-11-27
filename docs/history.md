# 專案背景與決策

`jc-ime` 是以 Rust 重寫的 Windows 輸入法，前身是 `0c-ime`（Go）。這一頁保留
「為什麼重寫」與已定案的決策，避免後續重新嘗試已知不可行的路線；逐日合併紀錄與
舊版進度留在 git 歷史。

## 為什麼重寫

| 事件 | 影響 |
|---|---|
| CI 產出的 MSVC TSF DLL 在 Windows 11 上讓 Explorer 崩潰 | 出貨包不可用；mingw 版正常 |
| Go cgo 在 Windows 只能用 mingw，Rust 卻是 MSVC | 工具鏈被迫混用，難驗證 |
| 安裝包漏檔案、設定程式與引擎路徑假設不一致 | 使用者實際踩到「設定打不開」「找不到引擎」 |
| 功能與平台細節高度耦合在單一 Go 服務 | 無法在 Linux 完整開發與評測 |

結論：問題不是功能不夠，是耦合與工具鏈讓每次修改都要回到 Windows 驗證，成本
過高。

## 已定的決策

| 主題 | 決策 | 理由 |
|---|---|---|
| 核心語言 | Rust | 無 GC、可嵌入、MSVC 原生 |
| 平台策略 | Windows 先做，其他平台之後 | 使用者實際需求 |
| 整合方式 | 核心為獨立行程，wrapper 極薄 | 崩潰隔離、核心可在 Linux 開發 |
| 抽象化 | `jc-platform` trait，核心不碰 OS API | 未來 Fcitx5／macOS 只補 wrapper |
| 模型 | candle 推論；Jinen v2 small 為日文預設 | 可重現、可量化 |
| 日文詞典 | Mozc OSS `dictionary_oss` | 唯一能達到可用品質的來源 |
| 注音詞典 | 沿用前身資料 | 前身已達可用品質 |
| 資料存放 | 不進 git；manifest 加雜湊固定 | 可重現、repo 不臃腫 |
| 授權 | AGPL-3.0-only | 與前身一致，便於搬資料與題庫 |

## 被否決的路線

| 路線 | 為什麼不行 |
|---|---|
| 調整詞成本 scale／offset | 只把 `modern-500` 從 0.0% 推到 1.6%，有數學上限 |
| 用 UniDic POS 聚合連接矩陣 | `modern-500` 只有 28%；context ID 才有 86% |
| 接上前身的 `lm.bin` | 只有 51 unigram／16 bigram，沒有作用 |
| reranker 擴大候選到 N=100 | 正確答案只有 19% 在候選內，延遲不可接受 |
| 全 MSVC（含 Go） | Go cgo 只支援 gcc；除非重寫 ONNX 綁定 |

## 可沿用的資產

- 前身 repo：`github.com/jeffpeng3/0c-ime`（AGPL-3.0），本機參考克隆在
  `/mnt/work/jc/ref/0c-ime`。
- 題庫 `0c-testdata`：`ja/`（modern-500、hard-cases、ajimee-200 等）、`eval/`、
  `fuzzy/`（919 題）、`golden/`（注音 45 例與 mixed）。
- 模型與資料的來源、revision 與雜湊見 [`models.md`](models.md)。

## 開發環境

| 項目 | 值 |
|---|---|
| 主機 | GB10，128 GB UMA，20 核心 |
| Rust | 1.100.0 |
| 網路 | GitHub、Hugging Face 可通 |
| 資料路徑 | 由 `JC_DATA`／`--data` 指定，不進 git |

Windows 實機驗證需要一台可安裝／解除安裝測試包的 Windows 11；Linux 只驗證核心、
評測與型別檢查。

## 出貨條件

- TSF DLL 會被 explorer 與任意程式載入，必須以 MSVC 建置並靜態連結 CRT，不能
  依賴 `vcruntime140.dll`。
- 安裝包的檔案清單由資料 manifest 驅動；打包後逐檔檢查存在、大小與 SHA-256，
  缺檔就讓建置失敗。
- x86 與 x64 各一份 TSF DLL，註冊時寫入對應的 `InprocServer32` 檢視。
