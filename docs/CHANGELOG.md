# 變更記錄

這裡只記使用者看得到的變更。版本號對應 `Cargo.toml` 的
`[workspace.package] version`；release workflow 會抓 `## <version>` 段落放進
GitHub Release。內部重構、測試補強與移植對照留在 `docs/port/`。

## 未發佈

### 變更

- 日文預設改為延遲整句轉換：打字時顯示假名，停頓 300 ms 後自動轉成漢字。
  `modern-500` top-1 92.2%，與原本手動整句模式相同；每題模型推論 0.21 次，
  停頓到轉換完成 p95 50 ms。
- 設定新增 `ja_conversion`（`live`／`sentence`，預設 `live`）；舊設定檔沒有
  這個欄位時視為 `live`，舊 `ja_live_conversion: false` 會遷移成 `sentence`。
- 日文片段支援手動選字與假名變換：候選包含平假名、全角片假名、半角片假名；
  `F6` 平假名、`F7` 切換平假／片假、`F8` 半角片假名、`F9`／`F10` 全半角英數；
  漢字可拆回讀音。手動選過的片段在後續重轉時保持不變。
- 新增 `eval ja --no-live` 可完全回到手動整句轉換。

## 0.1.0 - 2026-11-20

第一個可發佈版本。Windows 提供 per-machine MSI，Linux 提供引擎、CLI 與設定
程式的 tar.gz。

### 新增

- Windows x64 TSF 輸入法「jc 注音」：組字、候選窗、分頁、滑鼠點選、虛線底線、
  `Win+Space` 選取。TSF DLL 以靜態 CRT 建置，不依賴 `vcruntime140.dll`。
- 注音輸入：大千鍵盤、聲調選填、候選翻頁、Tab 補全、簡繁轉換、Ctrl＋標點與
  mixed ASCII 推斷。注音 golden 45/45。
- 注音選字模型：candle Q8_0，33.9 MiB；權重不隨安裝包散布，由使用者另外下載。
- 日文輸入：羅馬字→かな→漢字，Mozc OSS 詞典 pack v2；預設以 Jinen v2 small
  門檻混合，`modern-500` 93.6%，關閉神經轉換時 86.0%。
- 英文與 Mixed：英文直通、前綴補完與拼字建議；Mixed 路由 6/6。
- 個人化：rollback window、弱負向訊號與最短觸發長度，資料寫入 `personal.json`。
- 引擎：stdio、Unix domain socket、Windows named pipe；`--supervise` 提供單一
  實例、崩潰重啟、退避與環狀 log。
- 設定程式 `jc-settings-ui`：讀寫 `settings.json`，顯示目前資料根目錄與模型狀態。
- 安裝器：WiX v5 per-machine MSI、`MajorUpgrade`、TIP 註冊／反註冊與開始功能表
  捷徑；反安裝可選擇保留使用者設定。
- CLI：`data manifest`、`data verify`、`data status`、`data download`，下載支援
  `.part`、續傳與 SHA-256 驗證。
- Linux 發佈包：`jc-engine`、`jc-cli`、`jc-settings-ui`，附 `LICENSE`、`NOTICE`、
  `legal/` 與安裝說明。

### 變更

- 注音標點模型自 2026-11-20 起不再載入，Funmula 的 `zh-select`、`zh-punct`
  都不隨產品散布。
- 日文 reranker 不再預設使用，改由 Jinen small 的門檻混合取代。
