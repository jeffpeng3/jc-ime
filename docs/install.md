# 安裝與移除

`jc-ime-<version>-x64.msi` 是 per-machine 安裝包，用 WiX v5 建置。這份文件是給
使用者與實機驗證的人看的；建置細節在 [`../installer/README.md`](../installer/README.md)。

## 系統需求

| 項目 | 需求 |
|---|---|
| Windows | Windows 10 1809 以上或 Windows 11；x64 與 32 位元程式都支援（同一個輸入法項目） |
| 語言 | 已安裝「中文（台灣）」語言。沒有這個語言時 `RegisterProfile` 可能失敗，安裝器會停下並回報 |
| 權限 | 系統管理員（寫 HKLM 的 COM、CTF 機碼） |
| 空間 | 程式數十 MiB（視建置而定），資料約 174 MiB，另加 MSI 快取 |

## 安裝

雙擊 MSI，或從提權的命令提示字元：

```bat
msiexec /i jc-ime-0.1.0-x64.msi /l*v install.log
```

靜默安裝：

```bat
msiexec /i jc-ime-0.1.0-x64.msi /qn /l*v install.log
```

安裝器會依序做：

1. 把程式放到 `%ProgramFiles%\jc-ime\`，授權文件放到 `%ProgramFiles%\jc-ime\legal\`，
   資料放到 `%ProgramData%\jc-ime\data\`。
2. 以 SYSTEM 註冊 TSF 文字服務，一次兩個架構、共用同一個 CLSID：
   - MSI 的 `<Class>` 寫 `HKCR\CLSID\{7B7D6E1A-...}\InprocServer32` 的兩個檢視：
     64 位元檢視指到 `jc-tsf.dll`，`Wow6432Node` 指到 `jc-tsf-x86.dll`。這一步在
     自訂動作之前完成，`RegisterCategory` 才找得到類別工廠。
   - 「jc 注音」的 profile 與分類只註冊一次
     （`ITfInputProcessorProfileMgr::RegisterProfile` 加 `ITfCategoryMgr::RegisterCategory`）。
   註冊失敗會讓整個安裝回滾。32 位元程式因此看得到同一項「jc 注音」，不必切換。
3. 註冊引擎開機啟動：`HKLM\Software\Microsoft\Windows\CurrentVersion\Run\jc-ime-engine`。
4. 以目前使用者身分呼叫 `InstallLayoutOrTip`，把「jc 注音」加進你的語言清單。
   不改你的預設輸入法。
5. 建立開始功能表的「jc 注音設定」與「jc 注音授權」兩個捷徑。

### 安裝後

1. 按 `Win+Space`，清單裡應該出現「jc 注音」。選它可以打注音。
2. 引擎在**下次登入**時由 Run 項目啟動。不想登出的人可以手動跑一次：

   ```bat
   "%ProgramFiles%\jc-ime\jc-engine.exe" --data "%ProgramData%\jc-ime\data" ^
     --settings "%LOCALAPPDATA%\jc-ime\settings.json"
   ```

   不要手動加 `--socket`：引擎會用目前使用者的 SID 尾段推出 `jc-ime-<rid>`，
   與 TIP 相同。只有在 `JC_IME_PIPE` 環境變數或
   `HKCU\Software\jc-ime\EnginePipe` 同時設成同一個自訂名稱時，才能兩邊都指定。

   引擎沒跑時，`jc 注音` 仍然可以選取，但按鍵會直接放行，看起來像「打了沒反應」。
3. 設定程式在「開始功能表 → jc 注音 → jc 注音設定」。修改後要重新啟動引擎
   （登出再登入，或從工作管理員結束 `jc-engine.exe` 後再跑上面的指令）才會生效；
   引擎目前不監看 `settings.json`。

### 看不到「jc 注音」

先確認 profile 是否註冊成功：

```bat
reg query "HKLM\SOFTWARE\Microsoft\CTF\TIP\{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}" /s
reg query "HKLM\SOFTWARE\Classes\CLSID\{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}\InprocServer32" /reg:64
reg query "HKLM\SOFTWARE\Classes\CLSID\{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}\InprocServer32" /reg:32
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
| `%ProgramFiles%\jc-ime\jc-tip-register.exe` | MSI 自訂動作用；手動修復註冊；開始功能表捷徑的啟動器 |
| `%ProgramFiles%\jc-ime\legal\LICENSE` | 本專案 AGPL-3.0-only 全文 |
| `%ProgramFiles%\jc-ime\legal\NOTICE` | 專案 NOTICE（本專案與隨附第三方元件的關係） |
| `%ProgramFiles%\jc-ime\legal\THIRD-PARTY-NOTICES.md` | 逐來源的授權、版本與署名 |
| `%ProgramFiles%\jc-ime\legal\licenses\` | 授權全文 17 份：Mozc、UniDic、Apache-2.0、CC-BY-4.0、CC-BY-SA-3.0／4.0、MIT、ESDB-SCOWL 等 |
| `%ProgramData%\jc-ime\data\manifest.json` | 資料索引（版本、大小、SHA-256、授權） |
| `%ProgramData%\jc-ime\data\packs\ja-lexicon-mozc-v2.pack` | Mozc 日文詞典 |
| `%ProgramData%\jc-ime\data\build-zh\lexicon.bin` | 注音詞典 |
| `%ProgramData%\jc-ime\data\build-zh\lm.bin` | 注音語言模型 |
| `%ProgramData%\jc-ime\data\models\zh-select\model.q8_0.gguf` | 注音選字模型（candle Q8_0，33.9 MiB） |
| `%ProgramData%\jc-ime\data\models\zh-select\vocab.json` | 注音選字詞表，選字模型載入必要檔 |
| `%ProgramData%\jc-ime\data\english\words.bin` | 英文詞表 |
| `%ProgramData%\jc-ime\data\models\external\jinen-v2-small\jinen-v2-small-Q5_K_M.gguf` | 日文神經模型 |
| `%LOCALAPPDATA%\jc-ime\settings.json` | 使用者設定；第一次按「套用」才建立，MSI 不覆蓋既有檔 |
| `%LOCALAPPDATA%\jc-ime\personal.json` | 個人化資料（引擎之後才會寫） |
| `%LOCALAPPDATA%\jc-ime\tsf.log` | 只有 `JC_TSF_LOG=1` 時才寫 |

Funmula 的 `zh-select` 依 `legal\licenses\MIT-Funmula.txt` 可散布，MSI 從 ONNX 轉好
`model.q8_0.gguf`（33.9 MiB）後隨附，並帶 `vocab.json` 與兩份 model manifest。catalog
不再有下載項，`zh-punct` 已於 2026-11-21 移除。引擎第一次啟動直接載入 GGUF，沒有
背景下載或轉檔；缺檔時只警告並退回統計選字。原始 `model.onnx` 不進 MSI。

### 授權文件位置

授權文件全部集中在 `%ProgramFiles%\jc-ime\legal\`，安裝後可直接用檔案總管或
記事本開啟：

| 路徑 | 內容 |
|---|---|
| `legal\LICENSE` | AGPL-3.0-only 全文 |
| `legal\NOTICE` | 專案 NOTICE |
| `legal\THIRD-PARTY-NOTICES.md` | 第三方逐來源聲明，含 Mozc、UniDic、注音資料、Jinen 模型與 GPUI |
| `legal\licenses\` | 17 份授權全文與資料來源 notice |

「開始功能表 → jc 注音 → jc 注音授權」會用記事本開啟
`legal\THIRD-PARTY-NOTICES.md`。捷徑透過 `jc-tip-register.exe legal` 組合路徑，
所以換安裝磁碟或目錄名稱都不受影響；`.md` 在乾淨的 Windows 沒有預設關聯，
這裡不依賴檔案關聯。

反安裝會連 `legal\` 整個目錄一起刪除，不留孤兒檔。使用者資料
（`%LOCALAPPDATA%\jc-ime`）與 `data-download` 另外下載的模型不受影響。
Funmula 模型的授權說明在 `legal\licenses\MIT-Funmula.txt`，隨 MSI 安裝並保留
copyright，符合 MIT 的散布條件。

### 機碼

| 機碼 | 建立者 | 內容 |
|---|---|---|
| `HKLM\SOFTWARE\Classes\CLSID\{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}\InprocServer32` | MSI（WXS 的 `<Class>`，兩個檢視） | 預設值為該架構的 DLL 路徑，`ThreadingModel=Apartment` |
| `HKLM\SOFTWARE\Microsoft\CTF\TIP\{7B7D6E1A-...}` | `ITfInputProcessorProfileMgr::RegisterProfile` | TIP 註冊；兩個架構共用 |
| `HKLM\SOFTWARE\Microsoft\CTF\TIP\{7B7D6E1A-...}\LanguageProfile\0x0404\{8F0A9B3C-...}` | `RegisterProfile` | 顯示名稱「jc 注音」 |
| `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Run\jc-ime-engine` | MSI | `REG_EXPAND_SZ`，每位使用者登入時各啟動一個引擎 |
| `HKCU\Software\Microsoft\CTF`、`HKCU\Control Panel\International\User Profile`、`HKCU\Keyboard Layout` | `InstallLayoutOrTip` | 目前使用者的語言與輸入法清單 |

同一個 CLSID 在兩個機碼檢視各有一個 `InprocServer32`：64 位元檢視指到
`jc-tsf.dll`，`Wow6432Node` 指到 `jc-tsf-x86.dll`。TSF 依宿主行程的位元數挑一個，
使用者的語言清單仍然只有一項。

`HKCU` 那三處由 Windows 的 `InstallLayoutOrTip` 維護，安裝器不直接寫。反安裝時
**不**呼叫 `ILOT_UNINSTALL`：移除語系的最後一個 layout 會讓 Windows 把系統地區改成
英文（windows-chewing-tsf issue 553），語言清單的項目留給使用者自己移除。

## 反安裝

雙擊「應用程式與功能 → jc 注音 → 解除安裝」，或：

```bat
msiexec /x jc-ime-0.1.0-x64.msi /l*v uninstall.log
```

保留使用者設定（`%LOCALAPPDATA%\jc-ime`）：

```bat
msiexec /x jc-ime-0.1.0-x64.msi KEEPUSERSETTINGS=1
```

順序：

1. 反安裝的 deferred 動作先跑 `machine-unregister`：`UnregisterCategory` 三次與
   `UnregisterProfile`。它排在 `RemoveRegistryValues` 之前，所以 CLSID 還在；
   之後 MSI 用同一個動作把兩個檢視（64 位元與 `Wow6432Node`）的 CLSID 機碼一起
   移除。`machine-unregister` 完全不碰 DLL，被隔離也不影響。
2. MSI 刪檔案與 `Run` 值，再移除空目錄。授權文件（`legal\` 整個目錄）也在這一步
   刪除。之後由 `data-download` 下載到 `%ProgramData%\jc-ime\data` 的模型不屬於
   MSI，不會被刪；要完全清空需手動刪該目錄。
3. `InstallFinalize` 之後以目前使用者身分跑 `user-uninstall`：刪
   `%LOCALAPPDATA%\jc-ime`（`KEEPUSERSETTINGS=1` 時保留）。**不從你的語言清單
   移除「jc 注音」**：移除語系的最後一個 layout 會讓 Windows 把系統地區改成英文
   （windows-chewing-tsf issue 553），請自己到「設定 → 時間與語言 → 語言」移除。

如果 `jc-tsf.dll` 正被 explorer 或其他程式載入，MSI 可能要求關閉程式或安排重開機後
才刪得掉檔案；這是 TSF DLL 的常態，重開機一次即可完成。

反安裝由哪個帳號執行，就只清那個帳號的 `HKCU`。其他帳號的語言清單項目在 profile
被全域移除後會變成無效項目，需要該使用者自行在語言設定移除，或再跑一次
`jc-tip-register.exe user-uninstall 0`。

## 升級

同一個 `UpgradeCode`（`{6A97C593-1BE9-414D-B654-650FBDC98B66}`）與遞增的
`ProductVersion` 會觸發 `MajorUpgrade`：先移除舊版，再安裝新版。舊版反安裝時
**不會**移除使用者的語言清單項目（條件排除 `UPGRADINGPRODUCTCODE`），新版會沿用。
設定檔不受影響。

`AllowSameVersionUpgrades` 設為 `yes`，所以版本號相同時重裝會就地升級（舊版先移除、
新版再安裝），不會留下兩個「jc 注音」項目。

## 簽章

正式發佈要簽四個對象：`jc-tsf.dll`、`jc-tsf-x86.dll`、`jc-engine.exe`、MSI 本體。
`build.ps1` 也簽其餘執行檔。指令與參數：

```bat
signtool sign /fd sha256 /tr http://timestamp.digicert.com /td sha256 ^
  /f jc-ime.pfx /p <password> jc-tsf.dll
signtool sign /fd sha256 /tr http://timestamp.digicert.com /td sha256 ^
  /f jc-ime.pfx /p <password> jc-tsf-x86.dll
signtool sign /fd sha256 /tr http://timestamp.digicert.com /td sha256 ^
  /f jc-ime.pfx /p <password> jc-engine.exe
signtool sign /fd sha256 /tr http://timestamp.digicert.com /td sha256 ^
  /f jc-ime.pfx /p <password> jc-ime-0.1.0-x64.msi
```

沒有憑證時 `build.ps1` 直接跳過簽章，輸出仍可安裝，但會出現下列長相：

| 位置 | 未簽章的樣子 |
|---|---|
| 雙擊 MSI | SmartScreen「Windows 已保護您的電腦」，要按「更多資訊 → 仍要執行」 |
| 檔案內容 | `數位簽章` 空白，`發行者` 是 `Unknown` |
| `signtool verify /pa /v jc-ime.msi` | `SignTool Error: No signature found.` |
| UAC 提示 | 發行者顯示為未知或「Windows 命令處理程式」 |

安裝與功能不受影響；企業環境若用 AppLocker／WDAC 的白名單，未簽章的 MSI 會被擋。

## 疑難排解

| 症狀 | 先查 |
|---|---|
| `Win+Space` 沒有「jc 注音」 | `HKLM\SOFTWARE\Microsoft\CTF\TIP\{...}` 是否存在；沒有就是註冊失敗，看 `install.log` 的 `RegisterTip` |
| 看得到但打不出注音 | 引擎是否在跑：`tasklist /fi "imagename eq jc-engine.exe"`；沒有就跑上面的啟動指令 |
| 只有 32 位元程式打不出注音 | `reg query "HKLM\SOFTWARE\Classes\CLSID\{7B7D6E1A-...}\InprocServer32" /reg:32`；預設值應該是 `jc-tsf-x86.dll`，不是就修復安裝 |
| 安裝在「新增語言」前完成 | 先安裝 zh-TW 語言，再修復安裝：`msiexec /fa jc-ime-0.1.0-x64.msi` |
| 想確認 DLL 與類廠 | `cd "%ProgramFiles%\jc-ime"` 後執行 `.\jc-tsf-smoke.exe` |
| 想知道 TSF 內部發生什麼 | 設 `JC_TSF_LOG=1` 後重開應用程式，看 `%LOCALAPPDATA%\jc-ime\tsf.log` 與 DebugView |
| 反安裝後還有機碼 | `reg query "HKLM\SOFTWARE\Microsoft\CTF\TIP\{7B7D6E1A-...}" /reg:32`（兩個檢視都查）；用 `jc-tip-register.exe machine-unregister` 補清 profile 與分類 |
| 找不到授權文件或捷徑打不開 | 看 `%ProgramFiles%\jc-ime\legal\`；缺檔就修復安裝 `msiexec /fa jc-ime-0.1.0-x64.msi` |

## 必須在 Windows 驗證

Linux 只能驗證 WiX 專案能被編譯器解析與 payload 清單一致，下面全部沒跑過：

1. `wix build` 完整產出 MSI，以及 `msiexec /i` 的 ICE 驗證與安裝日誌。
2. 安裝後 `Win+Space` 看得到「jc 注音」，選取後可打注音、選字、提交。
3. 安裝後重新登入，`jc-engine.exe` 是否由 Run 項目自動啟動，pipe
   `jc-ime-<目前使用者的 SID 尾段>` 是否建立；`tsf.log` 不應出現「連線引擎 pipe
   … 失敗」。
4. 組字外觀、游標、Enter／Esc／Backspace 與應用程式（記事本、Word、Chromium、
   PowerShell）的相容性。
5. UWP 應用程式（例如 Windows 設定或小算盤）能否載入 TIP；若不行，檢查安裝目錄上
   `ALL APPLICATION PACKAGES` 的讀取權限。
6. 升級：舊版反安裝、新版安裝後，語言清單與設定檔是否保留。
7. 反安裝後機碼、檔案與 `%LOCALAPPDATA%\jc-ime` 是否清乾淨；`KEEPUSERSETTINGS=1`
   是否真的保留。
8. 非管理員帳號只按 `Win+Space` 使用時，是否會因為 Run 項目或 ACL 出問題。
9. 未簽章與已簽章兩種 MSI 的 SmartScreen 行為。
10. 32 位元應用程式（例如舊版 Office）與 64 位元應用程式使用**同一個**「jc 注音」
    項目都能輸入；`reg query ... /reg:32` 與 `/reg:64` 分別指到 `jc-tsf-x86.dll` 與
    `jc-tsf.dll`（完整指令見 `docs/port/x86-tip.md`）。
11. `%ProgramFiles%\jc-ime\legal\` 的 20 個檔案是否齊全、「jc 注音授權」捷徑是否
    真的用記事本開啟 `THIRD-PARTY-NOTICES.md`，以及反安裝後整個 `legal\` 目錄
    是否消失。

## 已知限制

- 只提供 x64 的引擎、CLI 與設定程式。TSF DLL 有 x64 與 x86 兩份（共用同一個
  CLSID），但引擎本身是獨立行程，32 位元程式一樣連得到。
- 引擎啟動靠 `HKLM\...\Run`，不是服務，也沒有監督程式。多人同時登入時每個工作階段
  各有一個 `jc-engine.exe`；pipe 名稱含 SID 尾段，不同使用者各自連到自己的引擎。
  同一使用者開多個 RDP 工作階段仍共用同一個名稱，client 會連到其中一個實例。
- 語言清單只對執行安裝與反安裝的帳號生效；由 SYSTEM 或部署工具代裝時，使用者的
  `HKCU` 不會被加到。
- 設定變更後需要重啟引擎。
- 注音神經選字模型（Funmula `zh-select`）的 Q8_0 GGUF 隨安裝包散布；原始 ONNX 不含。
- 沒有候選窗自繪、沒有 `ITfDisplayAttributeProvider`；這些屬於其他任務或後續版本。
