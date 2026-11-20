# 變更記錄

這裡只記使用者看得到的變更。版本號對應 `Cargo.toml` 的
`[workspace.package] version`；release workflow 會抓 `## <version>` 段落放進
GitHub Release。內部重構、測試補強與移植對照留在 `docs/port/`。

## 0.1.0-rc.4 - 2026-11-20

### 修正

- Windows 安裝器：`check-payload.py` 在 cp1252 主控台輸出中文會失敗，導致 MSI
  建置中斷。腳本、`build.ps1` 與 workflow 都改為強制 UTF-8 輸出。

## 0.1.0-rc.3 - 2026-11-20

### 修正

- Windows 安裝包：TSF 的 cdylib 產物是 `jc_tsf.dll`，打包時才改名成
  `jc-tsf.dll`／`jc-tsf-x86.dll`；先前找不到檔案導致 MSI 建置失敗。
- Windows 建置：`+crt-static` 移到 `.cargo/config.toml`，不再於安裝腳本裡改
  `RUSTFLAGS`，避免同一份程式被完整重編兩次（release 約省 10 分鐘）。

## 0.1.0-rc.2 - 2026-11-20

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

## 0.1.0-rc.1 - 2026-11-20

第一個候選版本（`0.1.0` 尚未發佈）。Windows 提供 per-machine MSI，Linux 提供
引擎、CLI 與設定程式的 tar.gz。

### 新增

- 注音：大千鍵盤、聲調選填、候選翻頁、Tab 補全、簡繁轉換、Ctrl＋標點與
  mixed ASCII 推斷；注音 golden 45/45。
- 注音模糊音：`z-zh`、`c-ch`、`s-sh`、`en-eng`；注音 319 題 top-1 44.5%。
- 注音個人化學習：rollback window、弱負向訊號與最短觸發長度，資料寫入
  `personal.json`。
- 注音選字模型：candle Q8_0，33.9 MiB、推論 p50 9.36 ms；權重不隨安裝包散布，
  由使用者另外下載。
- 日文：羅馬字→かな→漢字，Mozc OSS 詞典 pack v2；預設以 Jinen v2 small 門檻混合，
  `auto` 92.2%、`quality` 93.6%、`off` 86.0%。
- 日文助詞讀音交替與候選詞性（`details[].pos`）。
- Windows x64 TSF 輸入法「jc 注音」：組字、候選窗與分頁、滑鼠點選、虛線底線、
  `Win+Space` 選取。TSF DLL 以靜態 CRT 建置，不依賴 `vcruntime140.dll`。
- 設定程式 `jc-settings-ui`（GPUI Kit）：讀寫 `settings.json`，顯示目前資料根目錄
  與模型狀態；引擎啟動時讀取設定檔。
- 安裝：WiX v5 per-machine MSI、`MajorUpgrade`、TIP 註冊／反註冊、授權文件與
  開始功能表捷徑；反安裝可選擇保留使用者設定。
- 引擎：stdio、Unix domain socket、Windows named pipe；`--supervise` 提供單一
  實例、崩潰重啟、退避與環狀 log。
- CLI：`data manifest`、`data verify`、`data status`、`data download`，下載支援
  `.part`、續傳與 SHA-256 驗證。
- Linux 發佈包：`jc-engine`、`jc-cli`、`jc-settings-ui`，附 `LICENSE`、`NOTICE`、
  `legal/` 與安裝說明。

### 變更

- Windows pipe 名稱加入使用者身分，預設 `jc-ime-<SID 尾段>`；可用 `JC_IME_PIPE`
  或 `HKCU\Software\jc-ime\EnginePipe` 覆寫。
- 注音標點模型自 2026-11-20 起不再載入，Funmula 的 `zh-select`、`zh-punct`
  都不隨產品散布。
- 日文 reranker 不再預設使用，改由 Jinen small 的門檻混合取代。

### 已知限制

- 未簽章：沒有簽章憑證時，Windows 會顯示 SmartScreen 警告，企業白名單也會擋下。
- 日文逐字変換（`ja_live`）尚未預設開啟。
- 右文脈（`max_context_chars.right`）還沒接進 Jinen。
- 只建置 x64 TSF；32 位元程式不會載入這個輸入法。
- 實機安裝、升級、反安裝與 UWP／Office 相容性仍待人工驗證。
