# 安裝與移除

`jc-ime-<version>-x64-setup.exe` 是 per-machine 安裝程式，用 Inno Setup 6 建置。
這份文件是給使用者與實機驗證的人看的；建置細節在
[`../installer/README.md`](../installer/README.md)。

安裝目錄與資料目錄和舊版相同：程式在 `%ProgramFiles%\jc-ime\`，資料在
`%ProgramData%\jc-ime\data\`，使用者資料在 `%LOCALAPPDATA%\jc-ime\`。

引擎從 `--data` 指到的資料根目錄載入 `english/words.bin`，不依賴
`JC_IMEDATA` 或 `JC_ENGLISH_PACK`。安裝版因此可以直接從
`%ProgramData%\jc-ime\data\english\` 載入完整英文詞表；health 的
`english_words` 會回報實際詞數（原型約 104 詞，安裝版應遠大於 1000）。

## 系統需求

| 項目 | 需求 |
|---|---|
| Windows | Windows 10 1809 以上或 Windows 11；x64 與 32 位元程式都支援（同一個輸入法項目） |
| 語言 | 已安裝「中文（台灣）」語言。沒有這個語言時 `RegisterProfile` 可能失敗，安裝器會回報錯誤 |
| 權限 | 系統管理員（寫 HKLM 的 COM、CTF 機碼） |
| 空間 | 程式數十 MiB（視建置而定），資料約 174 MiB，另加安裝暫存空間 |

## 安裝

雙擊 setup.exe，或從提權的命令提示字元靜默安裝：

```bat
jc-ime-0.1.0-x64-setup.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /LOG=%TEMP%\install.log
```

安裝器會依序做：

1. 把程式放到 `%ProgramFiles%\jc-ime\`，`LICENSE` 與 `NOTICE` 也放在同一層；
   其餘授權文件放到 `%ProgramFiles%\jc-ime\legal\`，資料放到
   `%ProgramData%\jc-ime\data\`。
2. 用 `[Registry]` 寫 `HKCR\CLSID\{2F6B41D8-...}\InprocServer32` 的兩個檢視：
   64 位元檢視指到 `jc-tsf.dll`，`Wow6432Node` 指到 `jc-tsf-x86.dll`。
3. 以管理員權限執行 `jc-tip-register.exe machine-register`，註冊「jc 注音」的
   profile 與分類（`ITfInputProcessorProfileMgr::RegisterProfile` 加
   `ITfCategoryMgr::RegisterCategory`）。這一步在 CLSID 機碼寫好之後執行，
   `RegisterCategory` 才找得到類別工廠；失敗時先看 `install.log` 的
   `machine-register` 結束碼，再看 `%TEMP%\jc-ime-install.log` 的註冊細節。
   32 位元程式因此看得到同一項「jc 注音」。
4. 寫入引擎開機啟動：`HKLM\Software\Microsoft\Windows\CurrentVersion\Run\jc-ime-engine`。
   Run 值直接執行 `jc-engine.exe --supervise`，並帶 `--data`、`--settings`、
   `--personal-dir` 與 `--personal-prefix personal`；**不帶** `--socket`，pipe
   名稱由引擎與 TIP 共用的 `jc_windows::resolve_pipe_name()` 從目前使用者 SID
   推出。
5. 勾選 `enableime`（預設）時，以 `runasoriginaluser` 執行 `user-install`，把
   單一「jc 注音」加進你的語言清單；再執行 `user-cleanup-legacy` 清掉舊語言
   清單項目。不改你的預設輸入法。
6. 建立開始功能表的「jc 注音設定」與「jc 注音授權」兩個捷徑。
7. 勾選 `enableime`（預設）時，以原使用者身分先啟動一次
   `jc-engine.exe --supervise`。若另外勾了 `smoke`，再執行
   `post-install-doctor.ps1 -NoUi`，跑一次安裝診斷並把紀錄寫到
   `%LOCALAPPDATA%\jc-ime\doctor\`。安裝期固定不跳訊息框，避免中斷靜默安裝；
   `jc-tsf-smoke` 超過 120 秒會被中止並記成 FAIL。見〈安裝診斷（doctor）〉。

### 安裝選項

安裝精靈的「選擇附加工作」頁有兩項，預設值如下：

| 選項（Task） | 預設 | 作用 |
|---|---|---|
| 安裝後把 jc 注音加入輸入法清單（`enableime`） | 勾選 | 以 `runasoriginaluser` 執行 `user-install`，把「jc 注音」加進目前使用者的語言清單，並在安裝結束時先啟動引擎。取消勾選時仍會註冊機器層級的 TIP，但要自己到語言設定裡加入。 |
| 安裝後執行診斷（`smoke`） | **不勾選** | 以 `runasoriginaluser` 執行 `post-install-doctor.ps1 -NoUi`，把 `<時間>.json`、`<時間>.log` 與 `last.txt` 寫到 `%LOCALAPPDATA%\jc-ime\doctor\`。不影響安裝成敗，只是留下疑難排解紀錄。 |

靜默安裝（`/VERYSILENT`）不會顯示這一頁，只採用預設值：`enableime` 會執行，
`smoke` 不會。要靜默安裝也跑診斷，明確指定工作，例如
`/TASKS="enableime,smoke"`；一旦帶了 `/TASKS`，預設勾選就不再套用，兩個都要
就兩個都寫。

從舊 MSI 升級時，安裝器會在複製檔案前先找舊的「jc 注音」MSI，停掉引擎與載入
`jc-tsf.dll` 的行程，再執行 `msiexec /x` 移除舊版；找不到或移除失敗都不會中斷
安裝。舊 MSI 的 Run 值、CLSID 與檔案由它的反安裝清除；半殘的部分再由後續的
`.old-*` 清理與讓路處理。

### 安裝後

1. 按 `Win+Space`，清單裡應該出現「jc 注音」。選它可以打注音。打字時單按左或
   右 Shift（不搭配其他鍵）會切到純英文，再按一次回到原本的中文或日文模式。
2. 安裝器會在完成時先啟動引擎；之後每次登入由 Run 項目再啟動。不想重新登入
   的人可以手動跑一次：

   ```bat
   "%ProgramFiles%\jc-ime\jc-engine.exe" --supervise ^
     --data "%ProgramData%\jc-ime\data" ^
     --settings "%LOCALAPPDATA%\jc-ime\settings.json" ^
     --personal-dir "%LOCALAPPDATA%\jc-ime"
   ```

   `--supervise` 不能省：直接伺服模式沒給 `--socket` 會走 stdio，TIP 連不上。
   不要手動加 `--socket`：引擎會用目前使用者的 SID 尾段推出 `jc-ime-<rid>`，
   與 TIP 相同。只有在 `JC_IME_PIPE` 環境變數或
   `HKCU\Software\jc-ime\EnginePipe` 同時設成同一個自訂名稱時，才能兩邊都指定。

   引擎沒跑時，`jc 注音` 仍然可以選取，TIP 的第一個按鍵會嘗試啟動引擎；真的
   起不來時按鍵會直接放行，看起來像「打了沒反應」。
3. 設定程式在「開始功能表 → jc 注音 → jc 注音設定」。修改後要重新啟動引擎
   （登出再登入，或從工作管理員結束 `jc-engine.exe` 後再跑上面的指令）才會生效；
   引擎目前不監看 `settings.json`。

### 安裝診斷（doctor）

`jc-tsf-smoke.exe` 就是 doctor：免管理員，載入 `jc-tsf.dll` 後跑 13 項檢查，
涵蓋 DLL、COM、TSF 啟動、引擎 pipe、註冊機碼、使用者語言清單、引擎健康狀態
與候選窗 UI 探針。

安裝完成後，若在安裝精靈勾選「安裝後執行診斷」（`smoke`；預設不勾），安裝器
會以 `runasoriginaluser` 執行 `%ProgramFiles%\jc-ime\post-install-doctor.ps1 -NoUi`。
這支腳本用 Windows PowerShell 5.1 呼叫 `jc-tsf-smoke.exe --json`，並把結果保存到
`%LOCALAPPDATA%\jc-ime\doctor\`：

| 檔案 | 內容 |
|---|---|
| `<yyyyMMdd-HHmmss>.json` | `jc-tsf-smoke --json` 的 stdout，`jc-ime-doctor/1` 物件 |
| `<yyyyMMdd-HHmmss>.log` | stderr 加上人類可讀摘要、版本、結束碼與 log 路徑 |
| `last.txt` | 這次的摘要；最後一行是同一筆 `.log` 的完整路徑 |

同一秒內重跑不會覆蓋上一筆，撞名時會加上 `-1`、`-2` 等序號。手動執行且有
FAIL 時，腳本會把 `last.txt` 複製到剪貼簿，並跳出訊息框顯示摘要前 15 行與 log
路徑。安裝器的 `[Run]` 固定帶 `-NoUi`，Session 0、非互動工作階段也一樣只寫
log，不碰剪貼簿也不跳視窗，所以靜默安裝不會被訊息框卡住。`jc-tsf-smoke`
最多跑 120 秒，逾時就中止並把 `jc-tsf-smoke 超過 120 秒未結束` 記成 FAIL。

```bat
:: 手動重跑（使用目前使用者）
powershell.exe -NoProfile -ExecutionPolicy Bypass ^
  -File "%ProgramFiles%\jc-ime\post-install-doctor.ps1"

:: CI／無人環境：不顯示 UI，並讓離開碼反映 doctor 結果
powershell.exe -NoProfile -ExecutionPolicy Bypass ^
  -File "%ProgramFiles%\jc-ime\post-install-doctor.ps1" -NoUi -ExitWithSmokeCode
```

安裝器的 `[Run]` 不帶 `-ExitWithSmokeCode`，所以診斷失敗不會讓安裝失敗；預設
一律以 0 結束。直接跑 `jc-tsf-smoke.exe` 時仍然可以用原本的 `--json`：

```bat
"%ProgramFiles%\jc-ime\jc-tsf-smoke.exe" --json
```

- `--json` 時 stdout 只有一個 `jc-ime-doctor/1` JSON 物件，人類可讀的診斷行
  改走 stderr。
- `jc-tsf-smoke.exe` 的結束碼：`0` 全部通過或只有警告、`1` 至少一項 `[FAIL]`、
  `2` 參數錯誤。
- 13 項的意義與每個失敗的修復指令見
  [`port/tsf-smoke.md`](port/tsf-smoke.md)。檢查 13 只在候選窗出現後才把
  Enter 沒上屏、Esc 關不掉視為 FAIL；組字空且候選窗沒出現是 inconclusive
  的 `[SKIP]`，代表 TIP 沒接到按鍵。

`install-smoke` CI job 也執行同一支 `post-install-doctor.ps1`，加上 `-NoUi`
與 `-ExitWithSmokeCode`，再從 `last.txt` 取 log 路徑、複製 `.log`／`.json`
成 artifact。TSF 自己的 log 仍在 `%LOCALAPPDATA%\jc-ime\tsf.log`，只有設
`JC_TSF_LOG=1` 才會寫。

### 看不到「jc 注音」

先確認 profile 是否註冊成功：

```bat
reg query "HKLM\SOFTWARE\Microsoft\CTF\TIP\{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}" /s
reg query "HKLM\SOFTWARE\Classes\CLSID\{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}\InprocServer32" /reg:64
reg query "HKLM\SOFTWARE\Classes\CLSID\{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}\InprocServer32" /reg:32
```

第三條的預設值應該是 `jc-tsf-x86.dll`，第二條是 `jc-tsf.dll`。兩條都指到同一個
CLSID，這是 32 位元程式能用同一個「jc 注音」的原因。

有機碼但 `Win+Space` 沒有項目，代表只缺使用者的語言清單：

```bat
"%ProgramFiles%\jc-ime\jc-tip-register.exe" user-install
```

也可以從「設定 → 時間與語言 → 語言與區域 → 中文（台灣）→ 新增鍵盤」手動加入。

## 檔案與機碼

### 檔案

| 路徑 | 內容 |
|---|---|
| `%ProgramFiles%\jc-ime\jc-tsf.dll` | TSF 文字服務（x64） |
| `%ProgramFiles%\jc-ime\jc-tsf-x86.dll` | TSF 文字服務（x86，給 32 位元程式用；同一個 CLSID） |
| `%ProgramFiles%\jc-ime\jc-engine.exe` | 引擎，named pipe `jc-ime-<SID 尾段>`，與 TIP 同名 |
| `%ProgramFiles%\jc-ime\jc-cli.exe` | 資料驗證與評測工具 |
| `%ProgramFiles%\jc-ime\jc-settings-ui.exe` | 設定程式 |
| `%ProgramFiles%\jc-ime\jc-tsf-smoke.exe` | 免管理員冒煙測試 |
| `%ProgramFiles%\jc-ime\post-install-doctor.ps1` | 安裝後診斷腳本；勾選 `smoke` 時由安裝器執行，也可手動跑 |
| `%ProgramFiles%\jc-ime\jc-tip-register.exe` | 安裝器自訂動作用；手動修復註冊；開始功能表捷徑的啟動器 |
| `%ProgramFiles%\jc-ime\LICENSE` | 本專案 AGPL-3.0-only 全文 |
| `%ProgramFiles%\jc-ime\NOTICE` | 專案 NOTICE（本專案與隨附第三方元件的關係） |
| `%ProgramFiles%\jc-ime\legal\THIRD-PARTY-NOTICES.md` | 逐來源的授權、版本與署名 |
| `%ProgramFiles%\jc-ime\legal\RUST-THIRD-PARTY-NOTICES.md` | Rust crate 的第三方授權清單 |
| `%ProgramFiles%\jc-ime\legal\licenses\` | 授權全文 17 份：Mozc、UniDic、Apache-2.0、CC-BY-4.0、CC-BY-SA-3.0／4.0、MIT、ESDB-SCOWL 等 |
| `%ProgramData%\jc-ime\data\manifest.json` | 資料索引（版本、大小、SHA-256、授權） |
| `%ProgramData%\jc-ime\data\packs\ja-lexicon-mozc-v2.pack` | Mozc 日文詞典 |
| `%ProgramData%\jc-ime\data\build-zh\lexicon.bin` | 注音詞典 |
| `%ProgramData%\jc-ime\data\build-zh-bigram\lm.bin` | 注音 bigram 語言模型（必要資料） |
| `%ProgramData%\jc-ime\data\models\zh-select\model.q8_0.gguf` | 注音選字模型（candle Q8_0，33.9 MiB） |
| `%ProgramData%\jc-ime\data\models\zh-select\vocab.json` | 注音選字詞表，選字模型載入必要檔 |
| `%ProgramData%\jc-ime\data\english\words.bin` | 英文詞表 |
| `%ProgramData%\jc-ime\data\models\external\jinen-v2-small\jinen-v2-small-Q5_K_M.gguf` | 日文神經模型 |
| `%LOCALAPPDATA%\jc-ime\settings.json` | 使用者設定；第一次開啟設定程式就以預設值建立，可立即修改，安裝器不覆蓋既有檔 |
| `%LOCALAPPDATA%\jc-ime\personal.json` | 個人化資料；引擎啟動時載入，學習後原子寫回 |
| `%LOCALAPPDATA%\jc-ime\supervisor.log` | `--supervise` 的環狀 log；預設位置，可用 `--supervisor-log` 改 |
| `%LOCALAPPDATA%\jc-ime\tsf.log` | TSF 診斷 log；只有 `JC_TSF_LOG=1` 時才寫 |
| `%LOCALAPPDATA%\jc-ime\doctor\` | 安裝後（勾選 `smoke`）與手動 doctor 的 `<時間>.json`、`<時間>.log`、`last.txt`；反安裝會刪除這個子目錄 |

注音產品只帶 bigram 語言模型：`build-zh-bigram\lm.bin` 是必要資料。舊的
unigram-only `build-zh\lm.bin` 已退役，安裝包不提供，引擎也不再退回該檔；
`lm.bin` 本身若沒有 bigram，載入時會直接報錯。缺少 bigram 檔時引擎仍可
以詞典-only 啟動，只記 info，不視為安裝失敗。

### 資料包產生方式

安裝器與離線資料包都吃 `jc-data` catalog 的 `shipped: true` 清單，不要手動挑檔。
離線包由 `scripts/build-data-bundle.sh` 從資料根目錄重建：

```sh
scripts/build-data-bundle.sh /mnt/work/jc/jc-ime-data \
  /tmp/opencode/jc-ime-data-0125.tar.zst
```

腳本會依 `jc-data` catalog 的 `shipped: true` 產生根 `manifest.json`（路徑、
大小、SHA-256、角色、授權），把檔案放進暫存目錄後用 `tar --zstd` 打包，並
輸出 `<檔名>.sha256`。同一份資料與 `SOURCE_DATE_EPOCH` 會得到相同的
manifest 時間與 tar mtime。

必要檔案：

| 路徑 | 用途 |
|---|---|
| `build-zh/lexicon.bin` | 注音詞典 |
| `build-zh/manifest.json` | 詞典的 size／SHA-256 驗證；`load_zh_bin_lexicon` 會讀 |
| `build-zh-bigram/lm.bin` | bigram 語言模型；`ZhLm::open` 只驗檔內 magic、版本、CRC32 與版面，不需要同目錄 manifest |
| `english/words.bin`、`english/manifest.json`、`english/NOTICE.txt` | 英文詞表；三者缺一就退回內建原型 |
| `packs/ja-lexicon-mozc-v2.pack` | 日文詞典 |
| `models/manifest.json`、`models/zh-select/*` | zh-select 模型集合與 runtime manifest |
| `models/external/jinen-v2-small/*.gguf`、`models/quant/jinen-v2-xsmall/tokenizer.json` | 日文神經模型與 tokenizer |

資料包不含 `build-zh/lm.bin`，也不含 `build-zh-bigram/manifest.json`：前者已
退役，後者不是 loader 驗證路徑的一部分。根 `manifest.json` 會列出實際收進
資料包的檔案，引擎啟動時用 `check_data_manifest` 做存在與大小檢查。


Funmula 的 `zh-select` 依 `legal\licenses\MIT-Funmula.txt` 可散布，安裝包從 ONNX 轉好
`model.q8_0.gguf`（33.9 MiB）後隨附，並帶 `vocab.json` 與兩份 model manifest。catalog
不再有下載項，`zh-punct` 已於 2026-11-21 移除。引擎第一次啟動直接載入 GGUF，沒有
背景下載或轉檔；缺檔時只警告並退回統計選字。原始 `model.onnx` 不進安裝包。

### 授權文件位置

授權文件共 21 份：`LICENSE` 與 `NOTICE` 在安裝目錄根，其餘 19 份在
`%ProgramFiles%\jc-ime\legal\`，安裝後可直接用檔案總管或記事本開啟：

| 路徑 | 內容 |
|---|---|
| `LICENSE` | AGPL-3.0-only 全文 |
| `NOTICE` | 專案 NOTICE |
| `legal\THIRD-PARTY-NOTICES.md` | 第三方逐來源聲明，含 Mozc、UniDic、注音資料、Jinen 模型與 GPUI |
| `legal\RUST-THIRD-PARTY-NOTICES.md` | Rust crate 的第三方授權清單 |
| `legal\licenses\` | 17 份授權全文與資料來源 notice（含 `README.md`） |

「開始功能表 → jc 注音 → jc 注音授權」會用記事本開啟
`legal\THIRD-PARTY-NOTICES.md`。捷徑透過 `jc-tip-register.exe legal` 組合路徑，
所以換安裝磁碟或目錄名稱都不受影響；`.md` 在乾淨的 Windows 沒有預設關聯，
這裡不依賴檔案關聯。

反安裝會刪掉這些授權檔並收掉 `legal\` 目錄，不留孤兒檔。使用者資料
（`%LOCALAPPDATA%\jc-ime`）不在安裝目錄，反安裝只會刪除其中的 `doctor\`
診斷紀錄，其他使用者資料不主動刪除。
Funmula 模型的授權說明在 `legal\licenses\MIT-Funmula.txt`，隨安裝包安裝並保留
copyright，符合 MIT 的散布條件。

### 機碼

| 機碼 | 建立者 | 內容 |
|---|---|---|
| `HKLM\SOFTWARE\Classes\CLSID\{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}\InprocServer32` | 安裝器（`jc.iss` 的 `[Registry]`，兩個檢視） | 預設值為該架構的 DLL 路徑，`ThreadingModel=Apartment` |
| `HKLM\SOFTWARE\Classes\CLSID\{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}\PipePrefix` | 安裝器（`jc.iss` 的 `[Registry]`） | `jc-ime`，啟動器用來推導引擎 pipe |
| `HKLM\SOFTWARE\Microsoft\CTF\TIP\{2F6B41D8-...}` | `ITfInputProcessorProfileMgr::RegisterProfile` | TIP 註冊；兩個架構共用 |
| `HKLM\SOFTWARE\Microsoft\CTF\TIP\{2F6B41D8-...}\LanguageProfile\0x0404\{C4E8D217-...}` | `RegisterProfile` | 顯示名稱「jc 注音」 |
| `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Run\jc-ime-engine` | 安裝器（`jc.iss` 的 `[Registry]`） | `REG_EXPAND_SZ`，每位使用者登入時各啟動一個引擎 |
| `HKCU\Software\Microsoft\CTF`、`HKCU\Control Panel\International\User Profile`、`HKCU\Keyboard Layout` | `InstallLayoutOrTip` | 目前使用者的語言與輸入法清單 |

同一個 CLSID 在兩個機碼檢視各有一個 `InprocServer32`：64 位元檢視指到
`jc-tsf.dll`，`Wow6432Node` 指到 `jc-tsf-x86.dll`。TSF 依宿主行程的位元數挑一個，
使用者的語言清單仍然只有一項。

`HKCU` 那三處由 Windows 的 `InstallLayoutOrTip` 維護，安裝器不直接寫。升級前殘留的
舊項目（舊 CLSID 與 rc.15 的「jc 注音（新版）」）由 `user-cleanup-legacy` 呼叫
`RemoveLanguageProfile` 清除，排在 `user-install` 之前，不會動到 HKLM 的
profile 註冊，也不呼叫 `ILOT_UNINSTALL`。反安裝時**不**呼叫 `ILOT_UNINSTALL`：
移除語系的最後一個 layout 會讓 Windows 把系統地區改成英文
（windows-chewing-tsf issue 553），語言清單的項目留給使用者自己移除。

## 反安裝

從「應用程式與功能」找到 jc 注音的解除安裝項目，或：

```bat
"%ProgramFiles%\jc-ime\unins000.exe" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /LOG=%TEMP%\uninstall.log
```

反安裝會刪除 `%LOCALAPPDATA%\jc-ime\doctor`（doctor 的紀錄），但不會刪除
`%LOCALAPPDATA%\jc-ime` 其他內容（settings.json、personal.json、
supervisor.log、tsf.log）。要清全部使用者資料，以該使用者身分執行：

```bat
"%ProgramFiles%\jc-ime\jc-tip-register.exe" user-uninstall 0
```

順序：

1. `[UninstallRun]` 先跑 `machine-unregister`：`UnregisterCategory` 三次與
   `UnregisterProfile`。它不碰 DLL，被隔離也不影響。
2. `[Registry]` 用 `uninsdeletekey` 移除兩個檢視（64 位元與 `Wow6432Node`）的
   CLSID 子樹，再刪檔案與 `Run` 值。
3. `CurUninstallStepChanged(usPostUninstall)` 清 `.old-*` 殘骸；仍在的
   `jc-tsf.dll`、`jc-tsf-x86.dll`、`jc-engine.exe` 刪不掉就登記下次開機刪除，
   然後收掉 `legal\licenses` 與 `legal`。安裝目錄本身還留著反安裝程式檔案時交給
   Inno 收尾；只有前面真的排了開機刪除，才會把整個目錄一起登記。
   `[UninstallDelete]` 另外刪除 `%LOCALAPPDATA%\jc-ime\doctor`。

如果 `jc-tsf.dll` 正被 explorer 或其他程式載入，反安裝會把檔案改名或排到重開機後
才刪得掉；這是 TSF DLL 的常態，重開機一次即可完成。反安裝**不從你的語言清單
移除「jc 注音」**：profile 被移除後，清單項目會自然變成無效項目，請自己到
「設定 → 時間與語言 → 語言」移除。

反安裝由哪個帳號執行，就只清那個帳號的 `HKCU`。其他帳號的語言清單項目在 profile
被全域移除後會變成無效項目，需要該使用者自行在語言設定移除，或再跑一次
`jc-tip-register.exe user-cleanup-legacy`。這個清理只動 HKCU 的語言清單，不會
刪除 `%LOCALAPPDATA%\jc-ime` 的資料（doctor 的紀錄由反安裝的
`[UninstallDelete]` 另行清除）；要清全部資料請用 `user-uninstall 0`。

## 升級

安裝程式有固定的 `AppId`，直接執行新版的 setup.exe 就會升級，不需要先移除舊版。
`PrepareToInstall` 會在複製檔案前：

1. 掃 `HKLM` 的 Uninstall（含 `WOW6432Node`），找 `WindowsInstaller=1` 且
   `DisplayName` 以「jc 注音」開頭的舊 MSI。
2. 找到就停掉 `jc-engine` 與載入 `jc-tsf.dll` 的行程，執行
   `msiexec /x <ProductCode> /qn /norestart`。失敗只寫 log，不中斷安裝。
3. 清掉安裝目錄的 `.old-*` 殘骸。
4. 對 `jc-tsf.dll`、`jc-tsf-x86.dll`、`jc-engine.exe` 先刪；刪不掉就改名
   `.old-<yyyymmddhhnnss>` 讓路；再失敗才回報錯誤。

舊版反安裝後，新安裝器會重新寫入 CLSID、Run 值與 profile。`%LOCALAPPDATA%\jc-ime`
（settings.json、personal.json）不在安裝目錄，升級時保留；rc.15 加進語言清單的
舊「jc 注音」與「jc 注音（新版）」由 `user-cleanup-legacy` 移除，再由 `user-install`
加回單一「jc 注音」。

完成後請登出再登入，讓 explorer 與各行程重新載入 DLL，並確認只跑一個
`jc-engine.exe`。

## 簽章

正式發佈要簽兩份 TSF DLL（`jc-tsf.dll`、`jc-tsf-x86.dll`）、所有執行檔與 setup.exe。
`build.ps1` 會自動處理。指令與參數：

```bat
signtool sign /fd sha256 /tr http://timestamp.digicert.com /td sha256 ^
  /f jc-ime.pfx /p <password> jc-tsf.dll
signtool sign /fd sha256 /tr http://timestamp.digicert.com /td sha256 ^
  /f jc-ime.pfx /p <password> jc-tsf-x86.dll
signtool sign /fd sha256 /tr http://timestamp.digicert.com /td sha256 ^
  /f jc-ime.pfx /p <password> jc-engine.exe
signtool sign /fd sha256 /tr http://timestamp.digicert.com /td sha256 ^
  /f jc-ime.pfx /p <password> jc-ime-0.1.0-x64-setup.exe
```

沒有憑證時 `build.ps1` 直接跳過簽章，輸出仍可安裝，但會出現下列長相：

| 位置 | 未簽章的樣子 |
|---|---|
| 雙擊 setup.exe | SmartScreen「Windows 已保護您的電腦」，要按「更多資訊 → 仍要執行」 |
| 檔案內容 | `數位簽章` 空白，`發行者` 是 `Unknown` |
| `signtool verify /pa /v jc-ime-0.1.0-x64-setup.exe` | `SignTool Error: No signature found.` |
| UAC 提示 | 發行者顯示為未知或「Windows 命令處理程式」 |

安裝與功能不受影響；企業環境若用 AppLocker／WDAC 的白名單，未簽章的 setup.exe
會被擋。

## 疑難排解

安裝與反安裝的 log 由 `/LOG=` 指定；沒有指定時，請在重跑時加上
`/LOG=%TEMP%\install.log` 或 `/LOG=%TEMP%\uninstall.log`。

| 症狀 | 先查 |
|---|---|
| `Win+Space` 沒有「jc 注音」 | `HKLM\SOFTWARE\Microsoft\CTF\TIP\{...}` 是否存在；沒有就是註冊失敗，看 `install.log` 的 `machine-register` 結束碼與 `%TEMP%\jc-ime-install.log` |
| 看得到但打不出注音 | 引擎是否在跑：`tasklist /fi "imagename eq jc-engine.exe"`；沒有就跑上面的啟動指令 |
| 升級後有兩項「jc 注音」或舊項目 | 以該使用者執行 `jc-tip-register.exe user-cleanup-legacy`，再登出登入；仍重複就到語言設定手動移除 |
| 只有 32 位元程式打不出注音 | `reg query "HKLM\SOFTWARE\Classes\CLSID\{2F6B41D8-...}\InprocServer32" /reg:32`；預設值應該是 `jc-tsf-x86.dll`，不是就重跑 setup.exe |
| 安裝在「新增語言」前完成 | 先安裝 zh-TW 語言，再重跑 setup.exe |
| 安裝卡住或失敗 | 看 `/LOG=` 指定的 `install.log`；`PrepareToInstall` 的讓路與 `machine-register` 的結束碼都在裡面 |
| 想確認 DLL 與類廠 | `cd "%ProgramFiles%\jc-ime"` 後執行 `.\jc-tsf-smoke.exe` |
| 安裝後跳出「jc 注音 安裝診斷」訊息 | 看 `%LOCALAPPDATA%\jc-ime\doctor\last.txt`；最後一行是完整 `.log`。修正後可重跑 `post-install-doctor.ps1` |
| 想知道 TSF 內部發生什麼 | 設 `JC_TSF_LOG=1` 後重開應用程式，看 `%LOCALAPPDATA%\jc-ime\tsf.log` 與 DebugView |
| 反安裝後還有機碼 | `reg query "HKLM\SOFTWARE\Microsoft\CTF\TIP\{2F6B41D8-...}" /reg:32`（兩個檢視都查）；用 `jc-tip-register.exe machine-unregister` 補清 profile 與分類 |
| 找不到授權文件或捷徑打不開 | 看 `%ProgramFiles%\jc-ime\LICENSE`、`NOTICE` 與 `legal\`；缺檔就重跑 setup.exe |

## 必須在 Windows 驗證

Linux 只能做 `.iss`、payload 與 manifest 的靜態一致性檢查，下面全部沒跑過：

1. ISCC 完整產出 setup.exe，以及 `/VERYSILENT` 安裝與 `install.log`。
2. 安裝後 `Win+Space` 看得到「jc 注音」，選取後可打注音、選字、提交。
3. 安裝後重新登入，`jc-engine.exe` 是否由 Run 項目自動啟動，pipe
   `jc-ime-<目前使用者的 SID 尾段>` 是否建立；`tsf.log` 不應出現「連線引擎 pipe
   … 失敗」。
4. 組字外觀、游標、Enter／Esc／Backspace 與應用程式（記事本、Word、Chromium、
   PowerShell）的相容性。
5. UWP 應用程式（例如 Windows 設定或小算盤）能否載入 TIP；若不行，檢查安裝目錄上
   `ALL APPLICATION PACKAGES` 的讀取權限。
6. 升級：舊版反安裝、新版安裝後，語言清單只剩一項「jc 注音」、舊 CLSID 機碼與舊 DLL 檔案消失、個人資料與設定檔保留。
7. 反安裝後機碼、檔案是否清乾淨；反安裝只會刪除 `%LOCALAPPDATA%\jc-ime\doctor`，
   其他 `%LOCALAPPDATA%\jc-ime` 內容要另外確認 `user-uninstall 0` 是否清掉。
8. 非管理員帳號只按 `Win+Space` 使用時，是否會因為 Run 項目或 ACL 出問題。
9. 未簽章與已簽章兩種 setup.exe 的 SmartScreen 行為。
10. 32 位元應用程式（例如舊版 Office）與 64 位元應用程式使用**同一個**「jc 注音」
    項目都能輸入；`reg query ... /reg:32` 與 `/reg:64` 分別指到 `jc-tsf-x86.dll` 與
    `jc-tsf.dll`（完整指令見 `docs/port/x86-tip.md`）。
11. 安裝目錄的 `LICENSE`、`NOTICE` 與 `legal\` 底下 19 份授權文件是否齊全（共 21
    份）、「jc 注音授權」捷徑是否真的用記事本開啟 `THIRD-PARTY-NOTICES.md`，以及
    反安裝後 `legal\` 目錄與兩份根目錄授權檔是否消失。
12. 單按左／右 Shift 從中文或日文切到純英文，再按一次回到原本模式；
    Shift+字母、Shift+Ctrl 等組合不誤觸。設定程式不再顯示中英切換鍵、
    啟動模式、Esc 清除、日文神經模型等已停用項目。
13. 引擎啟動後 `jc-tsf-smoke` 的「安裝檔案」看得到
    `%ProgramData%\jc-ime\data\english\words.bin`，且 `Health` 的
    `english_words` 大於 1000；若是 0 或約 104，代表退回內建原型詞表。
14. 安裝完成後 `%LOCALAPPDATA%\jc-ime\doctor\` 是否產生 `<時間>.json`、
    `<時間>.log` 與 `last.txt`；有 FAIL 時剪貼簿與訊息框是否出現，Session 0
    是否只寫 log。install-smoke 會用 `-NoUi -ExitWithSmokeCode` 跑同一支腳本。

## 已知限制

- 只提供 x64 的引擎、CLI 與設定程式。TSF DLL 只有 x64 與 x86 兩份（共用同一個
  CLSID `{2F6B41D8-...}`，舊 CLSID 已移除），引擎本身是獨立行程，32 位元程式
  一樣連得到。
- 引擎啟動靠 `HKLM\...\Run` 與安裝完成時的啟動，不是服務；`--supervise` 會用
  Local mutex 保證單一實例並在子行程結束後重啟。多人同時登入時每個工作階段
  各有一個 `jc-engine.exe`；pipe 名稱含 SID 尾段，不同使用者各自連到自己的
  引擎。同一使用者開多個 RDP 工作階段仍共用同一個名稱，client 會連到其中一個
  實例。
- 語言清單只對執行安裝與反安裝的帳號生效；由 SYSTEM 或部署工具代裝時，使用者的
  `HKCU` 不會被加到。
- 設定變更後需要重啟引擎。
- 注音神經選字模型（Funmula `zh-select`）的 Q8_0 GGUF 隨安裝包散布；原始 ONNX 不含。
- 候選窗已由 TSF DLL 內的 D2D／DComp 自繪，組字底線用
  `ITfDisplayAttributeProvider`；Fcitx5／macOS 外殼仍要自己畫。
