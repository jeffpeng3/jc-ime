# 變更記錄

這裡只記使用者看得到的變更。版本號對應 `Cargo.toml` 的
`[workspace.package] version`；release workflow 會抓 `## <version>` 段落放進
GitHub Release。內部重構、測試補強與移植對照留在 `docs/port/`。

## 0.1.0-rc.17 - 2026-11-22

### 修正

- Windows 打字：修正按鍵處理的 `RefCell` 重入 panic。`styled_key`、
  `vocab_keys`、`japanese_key`、`ctrl_punct_key`、`simplified_hotkey` 與
  `shift_toggles` 移到 `impl State`，不再從已 borrow 的狀態內再次取用。
  rc.16 實機上這個 panic 會讓 TIP 只剩英文可輸入。
- Windows 安裝：註冊與反註冊補上 `SYSTRAYSUPPORT` 與 `COMLESS` 兩個 category。
  rc.16 缺少這兩項，工作列沒有輸入法圖示。

## 0.1.0-rc.16 - 2026-11-22

### 變更

- 移除舊版 TSF：只出貨單一 `jc-tsf.dll`／`jc-tsf-x86.dll`，對外只有一項「jc 注音」。
- 所有 `next` 後綴拿掉：`jc-tsf-next` 正名為 `jc-tsf`，pipe 改回 `jc-ime-<rid>`，
  個人資料前綴改回 `personal`，子命令改回 canonical 名稱。CLSID 與 profile GUID
  沿用 rc.15 已發行值，只把描述從「jc 注音（新版）」改成「jc 注音」。
- 升級時以使用者身分清理舊語言清單項目（`user-cleanup-legacy`），並移除舊
  CLSID、舊 Run 值與舊 DLL。個人化資料不搬移，全新安裝從空白開始。
- 修正 32 位元 MSI build：`candidate.rs` 的 WindowLongPtr 呼叫改用
  `cfg(target_pointer_width)` helper，讓 i686 也能編譯。

## 0.1.0-rc.15 - 2026-11-22

### 新增

- 0c TSF 架構移植（T0～T11）：新增 `jc-tsf-next` 與 `jc-ui`，引擎加入 pipe listener
  與多連線 session map。新版以平行建置方式註冊，使用新 CLSID 與
  「jc 注音（新版）」描述，不覆蓋已發行的舊版；可依 `docs/port/rollout.md`
  灰度切換與回退。
- UI 候選窗：移植 0c 的候選窗、訊息協定、Windows 訊息迴圈與 CUAS 接線，
  候選窗改走 `jc-ime-ui` pipe。
- CUAS：完成 service 端的 CandidateWindow／UiClient／Elements 分流與位置更新。
- 個人化全域：引擎改用單一共用 personal store，以增量合併取代各 session 各自存檔；
  reset 立即寫回空白 `personal-next.json`。
- Close session id：`Response::Ready` 攜帶實際 session id，Close 依 session map
  判斷歸屬，跨連線 id 回 `Internal` 錯誤，不再靜默成功。

### 修正

- 修正 integrator gate 的 20 項失敗，涵蓋引擎 shutdown／replay log 與新版 session
  的 reconnect、handoff、caps English、english autocomplete、簡繁輸出與全形預檢。
- 修正 CI：把 COM 註冊抽成 `jc-tsf-register` 共用 rlib，移除 `jc-tsf-next` 對
  `jc-tsf` 的依賴，消除 Windows 連結時的 LNK2005 重複 COM exports；
  並清掉 `jc-ui` 的 Clippy 警告。
- 修正 Windows focus host 測試的注音假引擎按鍵映射（`s` → ㄙ 改為大千 ㄋ）。

## 0.1.0-rc.14 - 2026-11-22

### 修正

- Windows 引擎 pipe：移除自建 SDDL，改用 named pipe 的預設 DACL。interprocess 2.4.9
  的 `SecurityDescriptor::to_owned_sd()` 複製 group 時誤呼叫 `set_owner`，把 owner
  從使用者 SID 覆蓋成 BA 群組；群組 SID 不能當 owner，`CreateNamedPipeW` 回
  os error 1307，pipe 建立失敗，TIP 因此一直重複 spawn。單機單使用者沒有安全損失，
  interprocess 預設會加 `PIPE_REJECT_REMOTE_CLIENTS`。
- Windows 引擎啟動：spawn 的 creation_flags 從 `DETACHED_PROCESS` 改成
  `CREATE_NO_WINDOW`，不再彈出終端機視窗。
- Windows 設定：捷徑改直連 `jc-settings-ui.exe`，開啟設定不再先冒 terminal。
  `jc-tip-register settings` 保留為相容入口。
- Windows 模型頁：移除注音選字模型、Jinen GGUF、日文詞典 pack 三個恆為「已安裝」
  的狀態列，只保留 Jinen tokenizer。
- 設定：移除 8 個已失效的設定 UI；`mixed_input` 從 JSON 契約完全移除，舊設定檔
  的欄位忽略；Shift 單按改送 `Request::Toggle` 切換純英文，回到原本的 Mixed／Ja。
- 設定 UI：預設值說明從全形空格改成換行顯示，程式跟隨系統主題。

## 0.1.0-rc.13 - 2026-11-22

### 修正

- Windows 打字：修正引擎 detached spawn 的引號解析。原本把 Run 機碼字串直接交給
  `CreateProcessW` 的 `lpCommandLine`，末尾 `\"` 被當成跳脫引號，`--data` 的值
  吃到後面的 `--settings`，引擎以 os error 123 啟動失敗，TIP 無限重試。
  改用 `Command::new(exe).args([...])` 傳五個獨立參數，並用
  `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP` 啟動後立刻放手。
  3 秒冷卻、等 pipe、lockfile 與失敗只警告的行為不變；新增末尾反斜線案例測試。

## 0.1.0-rc.12 - 2026-11-22

### 修正

- Windows 切換：Shift 單按改送 `Request::Toggle` 切換純英文，回到原本的
  Mixed／Ja 模式（`toggle_previous_mode`）；協定與註解同步更新。
- Windows 打字：TIP 連不上引擎 pipe 時 detached spawn 引擎（`CreateProcessW` +
  `DETACHED_PROCESS`，立刻關 handle）。啟動指令從 Run 機碼讀取並展開環境變數，
  3 秒冷卻搭配 10×100ms 等待，lockfile 擋跨行程重複啟動。
- Windows 設定：移除 8 個已失效的設定 UI；`mixed_input` 從 JSON 契約完全移除，
  舊設定檔的欄位仍忽略不影響行為；模型頁改走 catalog 解析。
- 冒煙測試：Deactivate／probe 卡點加時間戳輸出並 flush，方便定位 Windows 上的
  掛起問題，純診斷不改行為。

## 0.1.0-rc.11 - 2026-11-21

### 修正

- Windows 多工作階段：監督者互斥從 session 範圍的 named mutex 改成
  `%LOCALAPPDATA%\jc-ime\Local-jc-ime-engine-supervisor.lock`（分享模式 0），
  避免兩個 RDP session 各拿到一把鎖、各啟動一個子行程搶同一條 pipe。
  子行程改放 `KILL_ON_JOB_CLOSE` job object，TIP 遇到 `ERROR_PIPE_BUSY` 時用
  `WaitNamedPipeW` 等待，`MaxInstances` 維持 1。
- Windows 打字：TSF 改在取得輸入焦點時才連引擎 pipe（`OnSetFocus`），
  `OnKillFocus` 排 120ms 寬限期後釋放，`Deactivate` 立刻清 sender。
  Activate 只建立訊息視窗與註冊 sink；狀態機吸收重複事件，focus 早於 Activate
  時用 `pending_focus` 保留意圖。
- Windows 設定：設定視窗首次開啟即可修改，移除「複製」與鎖定。
  `mixed-input` 開關完全移除，中文使用者永遠進入混合輸入模式；舊設定檔的欄位
  仍保留在 JSON 契約做相容，不影響行為。
- Windows 模型頁：不再把已移除的 reranker／舊分詞器顯示為缺少，改探測 Jinen
  tokenizer 的正確路徑。
- 資料包：catalog 過濾已移除的 ja-punct／zh-punct 角色，移除過期 tokenizer 探測並
  重新產生 manifest；`build-zh/manifest.json` 隨 MSI 散布。

### 變更

- 引擎與 client transport 統一改用 `interprocess::local_socket`：Windows 底層是
  named pipe，Linux 是 abstract UDS。Windows ACL 改用 SDDL，pipe-busy 等待改用
  `ConnectWaitMode::Timeout`，舊的 `pipe.rs`／`pipe_windows.rs` 移除。
- 混合輸入：新增 `watashi` → `わたし` 的回歸測試，路由、開關與預設值未變。

## 0.1.0-rc.10 - 2026-11-21

### 修正

- Windows 打字：引擎自動啟動改用 `--supervise`，讓引擎開啟 named pipe 並在子行程
  結束時自動重啟。rc.9 拿掉固定 `--socket` 後引擎退回 stdio，完全不開 pipe，
  TSF 連不上，按鍵全部放行成英文。
- Windows 打字：注音資料包補上 `build-zh/manifest.json`。引擎的 `Bundle::open()`
  必須讀它驗證 `lexicon.bin`／`lm.bin`，缺檔會讓引擎啟動失敗。
- Windows 安裝：MSI `ProductVersion` 第四段帶 rc 序號（`0.1.0.10`），PackageCode
  每次建置自動產生，rc 版可以直接覆蓋安裝，不必先解除卸載。
- 注音選字模型：Funmula `0c-sequence-d512` 的 Q8_0 GGUF（33.9 MB）與
  `vocab.json`、兩份模型 manifest 隨 MSI 散布。MIT 授權允許散布，授權全文
  `MIT-Funmula.txt` 已在 MSI。原先的 ONNX 下載項移除，安裝後立即有模型選字，
  無需用戶端下載或轉換。

## 0.1.0-rc.9 - 2026-11-21

### 修正

- Windows 打字：引擎自動啟動的 `--socket jc-ime` 移除。引擎與 TSF 現在都用
  `resolve_pipe_name()` 從使用者 SID 推導同一個 pipe 名稱（例如 `jc-ime-1001`）。
  rc.8 之前兩邊名稱不同，TSF 連不上引擎，按鍵全部放行成英文。
- 冒煙測試：檢查 5 原本誤用 `ITfThreadMgr::Activate()` 回傳的應用程式 client id
  激活 TIP，msctf 查不到該 id 所屬的文字服務而回 `0x80070057`。改用
  `ITfClientId::GetClientId(&CLSID_JC_TSF)` 取得 TIP 自己的 id。jc-tsf 的
  `Activate` 本身正確，未改邏輯，只補診斷日誌（thread id、反註冊 HRESULT）。

## 0.1.0-rc.8 - 2026-11-21

### 修正

- Windows 安裝：TSF profile 改為預設啟用（`RegisterProfile` 的 `bEnabledByDefault`
  傳 `true`，`LanguageProfile` 的 `Enable=1`），使用者在語言設定加入鍵盤後就能切換；
  仍然不呼叫 `ActivateProfile`，不改系統預設輸入法。rc.7 上 `Enable=0` 時加入鍵盤
  無法切換。
- Windows 安裝：`%TEMP%\jc-ime-install.log` 第一次建立時寫入 UTF-8 BOM，
  PowerShell 5.1 的 `Get-Content` 不必加 `-Encoding` 就不會讀成亂碼。
- Windows TSF：`Activate` 失敗時可在 `%LOCALAPPDATA%\jc-ime\tsf.log` 看到逐步
  HRESULT，定位 `0x80070057` 卡在訊息視窗建立或 sink 註冊。另修正
  `create_message_window` 的 `hInstance`：原本用 `GetModuleHandleW(None)` 拿到宿主
  exe，改為以模組位址取得 DLL 自己。
- 混合輸入：拉丁 run 尾端的 `.` `,` `;` 先問切分器再決定是否當標點，避免 `5k42.`
  的 ㄡ 被剝掉；`hello.` 的句點仍是標點。
- 混合輸入：離線 DP 與正式路由合成單一管線，`mixed_latin_split` 開關移除。空白
  在可完整解析的注音 run 尾端是一聲；標點韻母只在緊接缺韻母音節時成立；相鄰 run
  保留前一段語言；片語合併只有一套規則。增量輸入與路由一致性由 532 筆 property
  test 鎖住。
- 資料包：`english/` 補上 `manifest.json` 與 `NOTICE.txt`，CI 不再因為缺少中繼檔
  退回內建原型英文詞表。資料 catalog 移除已不載入的 `zh_punct`。

### 變更

- CI：linux 與 windows job 在測試前下載與 release 相同的正式資料包，強制驗
  SHA-256 並寫入 `JC_IMEDATA`；資料包 URL 與雜湊是公開 repo variable，fork PR 也
  讀得到。HF dataset 補上來源與授權說明。
- CI／release：workflow 動作升級到 Node 24 版本（`checkout@v5`、
  `upload-artifact@v6`、`download-artifact@v7`），消除 Node 20 棄用警告。
- CI：快取加入總量上限 8 GB 與每日清理；tag scope 讓 release 快取每次 miss 的問題
  另在 `docs/release.md` 記錄暖機流程。
- 新增 `jc-cli mixed-diff`、`jc-cli corpus-diff` 兩個只量測、不改變行為的研究工具，
  報告見 `docs/reports/segment-diff-2026-11-21.md` 與
  `docs/reports/real-corpus-2026-11-21.md`。

## 0.1.0-rc.7 - 2026-11-21

### 修正

- Windows 安裝：TSF 的 `HKCR\CLSID\{...}\InprocServer32` 改由 MSI 的 `<Class>`
  寫入，不再由 DLL 的 `DllRegisterServer` 在自訂動作裡寫。先前 64 位元檢視的 CLSID
  在 `RegisterCategory` 執行時還不存在或指到 exe，導致存取違規（`0xC0000005`）與
  1722 失敗。
- Windows 安裝：註冊改用 `ITfInputProcessorProfileMgr::RegisterProfile`，取代
  `Register` 加 `AddLanguageProfile` 兩步。
- Windows 安裝：反安裝不再呼叫 `InstallLayoutOrTip(ILOT_UNINSTALL)`。移除語系最後
  一個 layout 會讓 Windows 變更系統地區；現在只清 `%LOCALAPPDATA%\jc-ime`。
- Windows 安裝：CLSID 機碼改由 Windows Installer 管理，自訂動作不再自行刪除。

### 移除

- `jc-tip-register` 的跨架構註冊程式碼（`arch.rs` 與自行寫機碼的路徑），約 900 行。

## 0.1.0-rc.6 - 2026-11-21

### 修正

- Windows 安裝：TSF 註冊的自訂動作在真機上以存取違規結束（`0xC0000005`），
  導致 MSI 回報 1722 並回滾。`DllRegisterServer` 取模組路徑時用的是行程的 exe，
  使 `InprocServer32` 指到 `jc-tip-register.exe`；已改為以模組位址錨定取得 DLL
  自己的路徑。
- Windows 安裝：`AddLanguageProfile` 的 `pchIconFile` 原本傳空 slice，指標落在
  對齊用的 `0x2`；改為指向真正的 NUL。
- Windows 安裝：AppContainer 的目錄 ACL 預設不再執行（需 `JC_IME_APP_CONTAINER=1`），
  把它移出安裝關鍵路徑。
- Windows 安裝：註冊流程逐步寫入 `%TEMP%\jc-ime-install.log`，失敗時可直接看出
  停在哪一個 TSF 呼叫。

## 0.1.0-rc.5 - 2026-11-20

### 修正

- Windows 安裝器：`jc-ime.wxs` 使用 WiX v3 的 `Win64` 屬性，v5 已改為
  `Bitness`，導致 MSI 建置在最後一步失敗。x86 元件改為 `Bitness="always32"`。

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
