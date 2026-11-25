# tsf-smoke：安裝診斷工具（doctor）

`crates/jc-tsf-smoke`，產出一支 `jc-tsf-smoke.exe`。它不註冊輸入法、不寫
`HKLM`、不需要管理員，只把 `jc-tsf.dll` 當普通 DLL 載入，逐項驗證 COM 外殼、
`ITfThreadMgr`、`Activate`、按鍵鏈、引擎 pipe，再往下檢查**安裝後才會出現的
問題**：安裝檔案、machine profile 機碼、使用者啟用、HKCU 語言清單與引擎健康
狀態。總共 13 項，最後用結束碼表示成敗；加 `--json` 時 stdout 只輸出一個
`jc-ime-doctor/1` JSON 物件。

**這支程式只能在 Windows 上執行。** Linux 上 `main` 會被換成一行訊息並以 2
結束，只讓 `cargo check --workspace --all-targets` 驗證型別；`report.rs`、
`profile.rs`（輸出格式與機碼字串）與 `engine_failure.rs`（失敗分類）不含 Win32，
Linux 也會一起編譯並跑單元測試。

## 為什麼可以免管理員

真正的 TSF 啟用要靠 `HKLM\SOFTWARE\Microsoft\CTF\TIP` 與
`ITfInputProcessorProfiles::Register`，那需要管理員。但安裝後的常見錯誤在免管理員
下就抓得到：

- DLL 載得起來嗎、架構對嗎、依賴齊嗎
- `DllGetClassObject`／`IClassFactory`／四個介面的 vtable 對嗎
- `ITfTextInputProcessor` 與 `ITfKeyEventSink` 是不是同一個 COM 物件
- msctf 的 `ITfThreadMgr` 建得起來嗎、`Activate` 會不會崩
- 按鍵鏈接得上嗎（`ITfKeystrokeMgr::TestKeyDown`）
- 引擎 pipe 通不通、協定版本對不對、健康狀態與 session 數
- 安裝目錄的四個必要檔案在不在、SHA-256 是多少
- `LanguageProfile\0x00000404\{PROFILE}` 在 64／32 位元檢視是否都存在
- `IsEnabledLanguageProfile`／`EnumLanguageProfiles`／`GetActiveProfile` 的結果
- `HKCU\Control Panel\International\User Profile` 有沒有本產品
- 候選窗真的畫得出來嗎（檢查 13 的 UI 探針）

診斷流程**刻意不呼叫** `DllRegisterServer`、`ITfInputProcessorProfiles::Register`
或 `EnableLanguageProfile`：這些會改系統狀態、需要管理員，也會讓一次無權限的
執行看起來全部失敗。檢查 5 與 13 會呼叫
`ActivateProfile(TF_IPPMF_FORPROCESS)`，但只影響本行程，結束時還原。修復一律交給
`jc-tip-register.exe`。

## 現行身分

| 項目 | 值 |
|---|---|
| CLSID | `{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}` |
| profile GUID | `{C4E8D217-5B3F-4A96-8E0C-7F1A2B3D4E56}` |
| LANGID | `0x0404`（`0x00000404`） |
| 顯示名稱 | `jc 注音` |
| 引擎 pipe | `jc-ime-<SID 尾段>`（可用 `JC_IME_PIPE` 覆寫） |

值與 `crates/jc-tsf/src/guids.rs`、`installer/jc-tip-register/src/guids.rs` 相同。
舊版 CLSID／profile 已移除，本文不再保留歷史值。

## 建置與執行

```bat
cargo build --release --target x86_64-pc-windows-msvc -p jc-tsf -p jc-tsf-smoke
jc-tsf-smoke.exe
jc-tsf-smoke.exe --json
jc-tsf-smoke.exe --check ui-probe
jc-tsf-smoke.exe --ui-probe --json
```

- 沒給 `--dll` 時依序找：`JC_TSF_DLL` 環境變數 → 執行檔同目錄的
  `jc-tsf.dll` → `LoadLibraryW` 的標準搜尋路徑（目前目錄、`PATH`）。
- `--dll <path>` 或直接給一個位置參數可以指定 DLL。
- `JC_IME_PIPE` 可以覆寫引擎 pipe 名稱；預設讀
  `HKCU\Software\jc-ime\EnginePipe`，沒有就用 `jc-ime-<SID 尾段>`。
- `--json`：stdout 只有一個 JSON 物件，人類可讀的診斷行全部改走 stderr。
- `--check ui-probe`（也接受 `13`、`ui.probe`）只跑檢查 13；`--ui-probe` 是同一件事。
  這兩個模式不會載入 `--dll` 指定的 DLL：msctf 依註冊機碼載入 TIP，所以需要先
  安裝並啟用本產品。
- 結束碼：`0` 全部通過或只有警告、`1` 至少一項 `[FAIL]`、`2` 參數錯誤。

`jc_test_key` 只在 `debug_assertions` 或 `jc-tsf` 開了 `test-export` feature 時
才會匯出：

```bat
cargo build --release --features test-export -p jc-tsf
```

release DLL 沒有這個符號是正常出貨狀態，診斷程式只會印一行 `[INFO]`。

## 安裝後自動執行

安裝器會在 `[Run]` 以原使用者身分執行
`%ProgramFiles%\jc-ime\post-install-doctor.ps1`。腳本呼叫
`jc-tsf-smoke.exe --json`，把結果寫到 `%LOCALAPPDATA%\jc-ime\doctor\`：

- `<yyyyMMdd-HHmmss>.json`：doctor 的 stdout。
- `<yyyyMMdd-HHmmss>.log`：stderr、版本、結束碼與摘要。
- `last.txt`：這次摘要，最後一行是完整 `.log` 路徑。

同一秒重跑會加 `-1`、`-2` 序號。手動執行且有 FAIL 時，腳本把 `last.txt` 複製
到剪貼簿並跳出訊息框顯示摘要前 15 行；Session 0、非互動或加 `-NoUi` 時只寫檔。
安裝器的 `[Run]` 固定帶 `-NoUi`，所以安裝期不會被訊息框卡住；CI 的
「安裝後 doctor」用 `-NoUi -ExitWithSmokeCode` 取得 `jc-tsf-smoke` 的結果。
install-smoke 也執行同一支腳本。

`jc-tsf-smoke` 最多跑 120 秒。逾時時腳本 `Kill()` 該行程，並把執行錯誤記成
`jc-tsf-smoke 超過 120 秒未結束`（doctor 判 FAIL），安裝器與 CI 都不會無限
等待。安裝器的 `[Run]` 預設忽略 doctor 的結束碼；`-ExitWithSmokeCode` 才會
讓離開碼反映診斷結果。

## 十三項檢查

| # | id | 檢查 | 判定 | 只回報 |
|---|---|---|---|---|
| 1 | `dll.load` | `LoadLibraryW(<path>)`，再 `GetProcAddress("DllCanUnloadNow")` | 載入成功**且**找得到 `DllCanUnloadNow` | `DllCanUnloadNow` 的回傳值 |
| 2 | `dll.class_factory` | `DllGetClassObject(CLSID_JC_TSF, IID_IClassFactory)` | `S_OK` 且拿到非空指標 | — |
| 3 | `dll.vtable` | `CreateInstance` 後 QI 四個介面，再比對 `ITfKeyEventSink` 與 `ITfTextInputProcessor` 的 `IUnknown` 指標 | 四個 QI 全 `S_OK`，且兩個介面是同一個 COM 物件 | 每個 QI 的 HRESULT |
| 4 | `tsf.thread_mgr` | `CoCreateInstance(CLSID_TF_ThreadMgr, CLSCTX_INPROC_SERVER)` | `S_OK` | — |
| 5 | `tsf.activate` | `ITfThreadMgr::Activate()` 取得**應用程式** client id；`ITfClientId::GetClientId(&CLSID_JC_TSF)` 取得**TIP** client id，再 `ITfTextInputProcessor::Activate(tm, tip_id)` | `S_OK`；已安裝時 msctf 已激活同一個 TIP 而回 `CONNECT_E_ADVISELIMIT` 也算通過 | 兩個 client id、`ITfKeystrokeMgr::TestKeyDown('u')` 的 consumed |
| 6 | `engine.pipe` | `jc-windows` 的 `LocalSocket::connect` → `Client::handshake` → `Hello` → `Key(Escape)` | 連得上且兩個往返都成功 | 連不上只印 `[SKIP]`，**不算失敗**；失敗原因會分類 |
| 7 | `registry.tip` | 兩個機碼檢視（`/reg:64`、`/reg:32`）各讀 `CTF\TIP\{CLSID}` 與 `HKCR\CLSID\{CLSID}\InprocServer32` | 兩邊都有同一個 CLSID，而且分別指到 `jc-tsf.dll` 與 `jc-tsf-x86.dll` | 只看得到一邊或指到錯的 DLL → `[FAIL]`；兩邊都沒有 → `[SKIP]`（尚未安裝） |
| 8 | `install.files` | 從 `InprocServer32` 推安裝目錄，檢查 `jc-tsf.dll`、`jc-tsf-x86.dll`、`jc-engine.exe`、`%ProgramData%\jc-ime\data\manifest.json` | 四個檔案都存在 | 每個檔案的 SHA-256；PE 沒有版本資源只 `[WARN]` |
| 9 | `profile.machine` | `HKLM\SOFTWARE\Microsoft\CTF\TIP\{CLSID}\LanguageProfile\0x00000404\{PROFILE}` 的 64／32 位元檢視 | 兩邊都存在，`Description` 非空 | `Enable`、`IconFile`；缺 `Enable`／`IconFile` 只 `[WARN]` |
| 10 | `profile.user` | `IsEnabledLanguageProfile`、`EnumLanguageProfiles(0x0404)`、`GetActiveProfile` | 已列舉**且**已啟用 | 列舉到的 profile 數、目前作用中的 profile GUID |
| 11 | `profile.language_list` | `HKCU\Control Panel\International\User Profile` 的 `Languages` 與 `zh-Hant-TW\InputMethodTips` | 含 `zh-Hant-TW`，且含 `0404:{CLSID}{PROFILE}`（有無大括號都接受） | 兩個清單的項目數 |
| 12 | `engine.detail` | 沿用檢查 6 的連線，再送 `Health` | 引擎健康且不在交接 | 引擎版本、協定、session、連線數、組字中、引擎 session 數、pid |
| 13 | `ui.probe` | 開 host 視窗＋`EDIT`、`ActivateProfile`、`SendInput` 打 `j` `i` `3` `↓` 開候選窗，再 `Enter` 提交；重新開窗後送 `Esc` | 互動桌面上：候選窗可見、rect 在虛擬螢幕內且與插入點 monitor 的 work area 相交、位置在插入點附近、`Enter` 後 EDIT 含「我」、`Esc` 後消失；`composition` 只回報、不參與判定 | `desktop`、`composition`、`candidate_visible`、`candidate_rect`、`latency_ms`、`escape_hides`、`committed_text`；沒有互動桌面、TIP 沒啟用，或讀不到組字字串且候選窗沒出現（inconclusive）時 `[SKIP]` |

### 檢查 3 的物件身分

檢查 3 的物件身分是**硬檢查**：msctf 要求 `ITfKeyEventSink` 與
`ITfTextInputProcessor` 是同一個 COM 物件，獨立 sink 物件會讓按鍵送不進來。
實作方式是把兩邊都 QI 到 `IUnknown` 再比指標；`#[implement]` 產生的 QI 對
`IID_IUnknown` 一律回同一個 identity 位址，所以指標相等就代表同一個物件。

### 檢查 5 的兩個 client id

`ITfThreadMgr::Activate()` 回的是**應用程式**的 id；
`ITfTextInputProcessor::Activate` 要的是**文字服務**的 id，由
`ITfClientId::GetClientId(&CLSID_JC_TSF)` 取得。之前直接把應用程式 id 傳進去，
msctf 在 `AdviseKeyEventSink` 回 `E_INVALIDARG`。已安裝 `jc-tsf` 時，msctf 會在
`ITfThreadMgr::Activate()` 內部先激活 TIP 並裝好 key sink；手動激活同一個 id 會回
`CONNECT_E_ADVISELIMIT`，這代表 TIP 已經在跑，判為通過。詳細文件依據與真機指令見
`docs/port/activate-second-call.md`。

「已為使用者啟用」還不等於「這個執行緒正在用它」。只註冊 profile 加上
`EnableLanguageProfile` 時，msctf 不會載入 TIP；`GetClientId` 會給一個背後沒有
任何 TIP 的 id，`AdviseKeyEventSink` 就回 `E_INVALIDARG`（run 45140346900 的
`tsf.log` 只有 `activate 開始 tid=29` 一行，之後空白）。所以檢查 5 會先呼叫
`ITfInputProcessorProfileMgr::ActivateProfile(TF_IPPMF_FORPROCESS)`，把本產品設成
**本行程**的作用中輸入法；msctf 這時才真的載入 DLL、用真正的 TIP client id 呼叫
`Activate`。旗標只影響本行程，不改系統或使用者的預設輸入法。

檢查 5 成功後會立刻 `Deactivate` 並等 200 ms：`jc-tsf` 的 worker 在 `Activate`
時會連上引擎。引擎現在可接受多條連線；這裡先放掉，讓檢查 6 從乾淨狀態建立
自己的連線。

### 檢查 6 的引擎錯誤分類

連不上不再只寫「連不上」。`engine_failure.rs` 把原因分成六類，`detail` 會直接
寫出分類與下一步：

| 分類 | 判斷依據 | 意義 |
|---|---|---|
| `pipe-missing` | Win32 錯誤 2／3 | pipe 不存在：引擎沒啟動，或 Run 機碼還沒生效 |
| `access-denied` | Win32 錯誤 5 | pipe 存在但 `CreateFileW` 被拒：權限或安全性原則 |
| `connect-failed` | 其他連線錯誤 | pipe 忙碌、名稱無效、路徑不存在 |
| `handshake-failed` | `Error::Protocol`／`BadFrame`／`Io` | 連上了，但 `Hello` 握手失敗：引擎與 DLL 不相容 |
| `protocol-mismatch` | `Error::VersionMismatch` | 協定版本不同，要一起更新引擎與 DLL |
| `timeout` | `Error::Timeout` | 握手或往返超過 500 ms，引擎可能卡住 |

連線與握手刻意分成兩步（`LocalSocket::connect` + `Client::handshake`），才能
區分「連不上 pipe」與「連上了但握手失敗」。

### 檢查 8 的安裝檔案

安裝目錄取自 `HKCR\CLSID\{CLSID}\InprocServer32` 的預設值（先看 64 位元檢視，
沒有才退回 32 位元）。資料根目錄固定是 `%ProgramData%\jc-ime\data`，與 MSI 的
`[DATADIR]` 一致。每個存在的檔案都會算 SHA-256；三個 PE 檔另外讀
`VS_FIXEDFILEINFO`，沒有版本資源只記 `[WARN]`，不影響結束碼。

### 檢查 9 的兩個檢視

標準機碼名是 `0x00000404`（八位十六進位）。早期安裝器曾寫成 `0x0404`，診斷會
在找不到標準名稱時退回讀舊名稱，並在 `detail` 標明「只有非標準的 0x0404 機碼」；
這種情況只記 `[WARN]`，因為 msctf 實際讀哪一個名稱需要實機確認。若兩個名稱都
沒有，才 `[FAIL]`。

64 位元與 32 位元檢視都要有這個機碼。只寫了一邊時，另一邊的程式（例如 32 位元
Office）會看不到 profile；若 `reg query ... /reg:32` 也看不到，可能是這台機器
把 CTF 當成轉向機碼，`detail` 會保留兩個檢視各自的結果。`Enable` 是 machine
預設值，現行安裝器傳 `bEnabledByDefault=false`，所以 `Enable=0` 是正常；真正
「這個使用者有沒有啟用」由檢查 10 的 `IsEnabledLanguageProfile` 判定。`IconFile`
沒有值是正常出貨狀態（安裝器不帶圖示），只記 `[WARN]`。

### 檢查 10 的使用者啟用

只用 TSF API，不讀機碼：

- `ITfInputProcessorProfiles::IsEnabledLanguageProfile(CLSID, 0x0404, PROFILE)`
- `ITfInputProcessorProfileMgr::GetActiveProfile(GUID_TFCAT_TIP_KEYBOARD)`：
  回報目前作用中的 profile GUID。作用中的不是本產品不算失敗（使用者可能正在用
  別的輸入法），只寫進 `detail`。
- `ITfInputProcessorProfiles::EnumLanguageProfiles(0x0404)`：確認 profile 有被
  列舉。沒列舉代表 machine profile 沒寫好，`fix` 指向 `machine-register`。

`IsEnabledLanguageProfile=false` 要 `[FAIL]`，`fix` 是「以目前使用者執行
`jc-tip-register.exe user-install`」。

### 檢查 11 的語言清單

`HKCU\Control Panel\International\User Profile`：

- `Languages`（`REG_MULTI_SZ`）要含 `zh-Hant-TW`。
- `zh-Hant-TW\InputMethodTips`（`REG_MULTI_SZ`）要含
  `0404:{2F6B41D8-…}{C4E8D217-…}`。

Windows 各版本寫入的格式不完全一致，比對時接受有無大括號、大小寫不同、以及
`0x0404:` 前綴。缺任何一項就 `[FAIL]`，`fix` 指向 `user-install`。

### 檢查 12 的引擎細節

沿用檢查 6 的連線，再送一次 `Health`。`detail` 包含引擎識別字串、協定版本、
這條連線的 session id、`connections`、`composing`、引擎的 `sessions` 總數與
`pid`。`draining=true` 代表引擎正在交接，判 `[WARN]`；`Health` 失敗也是
`[WARN]`（引擎已連上，只是細節拿不到）；連不上則是 `[SKIP]`，並保留檢查 6 的
失敗分類。

### 檢查 13 的 UI 探針

前 12 項都是註冊表、檔案、COM 與 pipe 層，沒有任何一項證明候選窗真的畫在
螢幕上。檢查 13 走一次真正的前景輸入路徑：

1. `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)`（失敗就算了），再註冊
   `jcUiProbeHost` 類別，建立可見的 top-level 視窗與 `EDIT` 子視窗，
   `SetForegroundWindow` 給 host、`SetFocus` 給 EDIT。
2. `ITfInputProcessorProfiles::GetCurrentLanguage` 記下目前語言，記下
   `GetActiveProfile` 的 profile，再用 `ActivateProfile(TF_IPPMF_FORPROCESS)`
   把本 TIP 設成本行程的作用中輸入法。
3. 用 `SendInput` 送 `VK_J`、`VK_I`、`VK_3`、`VK_DOWN` 的按下與放開，每鍵之間
   抽訊息。`↓` 是必要的：`jc-tsf` 只在 `composing && navigating` 時顯示候選窗，
   只打字不會開。
4. 最多等 2000ms，每 20ms 用 `FindWindowExW` 找**本行程**的
   `jcCandidateUiWindow`（同名視窗可能長在別的程序），記錄第一次看到可見視窗
   的延遲。
5. 斷言：
   - `IsWindowVisible` 為真
   - `GetWindowRect` 非空、在虛擬螢幕範圍內、與插入點所在 monitor 的 work area
     相交
   - 與 EDIT 的插入點比較：`GetCaretPos` 轉螢幕座標（取不到就用 EDIT client
     rect 的左下角），`|dx| <= 300`（依該 monitor 的 DPI 換算，300 DIP）、
     `dy` 在 `-200..=160`（視窗翻到上方是正常的）

   `composition` 只是把 EDIT 讀到的組字文字回報出來，不當成斷言。CUAS 的
   text store 常常讀不到，空字串不代表失敗；只有連候選窗都沒出現、又讀不到
   組字字串時，才改判 inconclusive。
6. 候選窗開著時送 `VK_RETURN`，讀 EDIT 的文字；必須包含 `ji3` 的第一個候選
   「我」。這是端到端證明，不只是視窗出現。
7. 重新送 `VK_J`、`VK_I`、`VK_3`、`VK_DOWN` 開第二次候選窗，再送 `Esc`，最多等
   1000ms 確認不再可見。不能在 Enter 之後直接驗消失：那時視窗已經因為提交而
   關掉了。
8. `DestroyWindow`、`UnregisterClassW`，還原 profile 與語言。

人類輸出與 JSON 的 `ui_probe` 物件用同一組 cell：

```text
desktop=interactive；composition=ㄨㄛˇ；candidate_visible=true；candidate_rect=104,142,304,182；latency_ms=87；escape_hides=true；committed_text=我
```

判定：

- `desktop=headless`：window station 不是 `WinSta0`、建視窗或取得前景視窗失敗、
  或 `SendInput` 被系統拒絕（Session 0、服務工作階段、鎖定中的桌面）。記
  `[SKIP]`，**不算失敗**；`detail` 會寫出原因。
- 本產品還沒安裝，或 `IsEnabledLanguageProfile=false`：記 `[SKIP]`，`detail`
  寫明「jc 注音未安裝或未為目前使用者啟用」並指向 `user-install`。真正的安裝
  問題由檢查 10 判 `[FAIL]`，不重複紅。
- 讀不到組字字串而且候選窗完全沒出現：TIP 根本沒接到按鍵（前景、TSF 或
  桌面環境不對）。記 `[SKIP]`（inconclusive），`detail` 寫明這是環境問題、
  無法判定產品行為，**不算失敗**。
- 只要候選窗出現過，就代表按鍵已經進到 TIP。之後任一斷言不成立都是
  `[FAIL]`：候選窗位置不合理、Enter 後 EDIT 沒有「我」、Esc 關不掉。`detail`
  帶實際 rect、延遲、所有不成立的地方與 `%LOCALAPPDATA%\jc-ime\tsf.log` 的
  建議。
- 候選窗出現、`committed_text` 含「我」、`escape_hides=true`：即使
  `composition` 是空的也判 `[PASS]`。`composition` 只是盡力取得的回報值，
  CUAS 的 text store 不保證給組字字串。
- `committed_text` 是空的代表候選窗出現、Esc 也能關掉，但 Enter 沒把字送進
  EDIT，可能是 CUAS 或 TIP 插入路徑。這是真的 `[FAIL]`，`detail` 會直接寫出
  這句話（候選窗沒出現時只報逾時，或依上一條判 inconclusive）。
- 候選字來自引擎：引擎沒開但讀得到 `composition`、只是沒有候選窗時，本項
  判 `[FAIL]`，先啟動 `jc-engine.exe` 再跑。若連 `composition` 都是空的，
  依上一條是 inconclusive 的 `[SKIP]`，不當成產品 bug。
- `composition` 是 EDIT 目前的組字文字，盡力取得；讀不到時是空字串，不代表
  失敗。

`--check ui-probe`／`--ui-probe` 只跑這一項，適合在實機上手動驗一次；它不看
`--dll`，TIP 是由 msctf 依註冊機碼載入的。

## 人類輸出

每項一行，格式固定：

```text
[PASS] 9. machine profile 機碼：…\LanguageProfile\0x00000404\{…}：64 位元：Enable=0、Description=jc 注音；32 位元：…
[FAIL] 10. 使用者啟用：IsEnabledLanguageProfile=false；EnumLanguageProfiles(0x0404)=true（共 12 個 profile）；目前啟用 {…}（不是本產品）
       修復：以目前使用者執行 "C:\Program Files\jc-ime\jc-tip-register.exe" user-install
```

最後兩行：

```text
結果：8 通過、1 警告、2 失敗
總結：安裝診斷失敗；請依上面各項的「修復」處理後重跑（2 項失敗）
```

`skip` 為 0 時不印「略過」；有略過時格式是
`結果：5 通過、0 警告、0 失敗、2 略過`。

## `--json` 輸出

`--json` 時 stdout 只有一個 JSON 物件（其他診斷訊息走 stderr）：

```json
{
  "schema": "jc-ime-doctor/1",
  "version": "0.1.24",
  "ok": false,
  "status": "fail",
  "summary": { "pass": 1, "warn": 1, "fail": 2, "skip": 1 },
  "checks": [
    { "id": "dll.load", "title": "DLL 載入", "status": "pass", "detail": "C:\\Program Files\\jc-ime\\jc-tsf.dll；DllCanUnloadNow = 0x00000000" },
    { "id": "install.files", "title": "安裝檔案", "status": "warn", "detail": "安裝目錄 C:\\Program Files\\jc-ime；…；1 個 PE 檔沒有版本資源（不影響載入）" },
    { "id": "profile.machine", "title": "machine profile 機碼", "status": "fail", "detail": "32 位元檢視的 … 不存在", "fix": "以管理員執行 jc-tip-register machine-register；若機碼完全不存在，先修復安裝（msiexec /fa <msi>）" },
    { "id": "profile.user", "title": "使用者啟用", "status": "fail", "detail": "IsEnabledLanguageProfile=false；…", "fix": "以目前使用者執行 \"C:\\Program Files\\jc-ime\\jc-tip-register.exe\" user-install" },
    { "id": "engine.pipe", "title": "引擎 pipe", "status": "skip", "detail": "pipe jc-ime-1001 不存在（引擎未啟動，或開機 Run 機碼還沒生效）；…；引擎未啟動或未安裝，只警告不判定失敗" }
  ]
}
```

規則：

- `schema` 固定 `jc-ime-doctor/1`。
- `version` 是 `jc-tsf-smoke` 的 `CARGO_PKG_VERSION`；安裝後腳本用它記錄版本，
  JSON 解析失敗時才改讀執行檔的 PE 版本。
- `ok` 等於「沒有任何 `fail`」；只有 `warn`／`skip` 時仍是 `true`。
- 頂層 `status` 是三態：`fail`（任一 `fail`）、`skip`（沒有 `fail` 但有 `skip`）、
  `pass`（其他）。`ok` 分不出「全部驗過」與「有項目沒驗到」，要驗有沒有真的
  跑過就用 `status`。
- `summary` 四個計數加起來等於 `checks` 的長度；上面的範例只列 5 項，完整執行
  固定 13 項。
- 每個檢查的 `status` 只接受 `pass`、`warn`、`fail`、`skip`。
- 只有 `fail` 一定有 `fix`；其他狀態不輸出這個欄位。
- 標題不含編號；人類輸出才由程式加 `1.`、`2.`……。
- 檢查 13 多一個 `ui_probe` 物件（其他檢查沒有這個欄位）：

```json
{
  "id": "ui.probe",
  "title": "UI 探針",
  "status": "skip",
  "detail": "desktop=headless；…；沒有互動桌面，跳過候選窗驗證，不算失敗",
  "ui_probe": {
    "desktop": "headless",
    "composition": "",
    "candidate_visible": false,
    "candidate_rect": "",
    "latency_ms": 0,
    "escape_hides": false,
    "committed_text": ""
  }
}
```

`candidate_rect` 是 `l,t,r,b` 字串，沒看到候選窗時是空字串；`latency_ms` 是送出
最後一個鍵到第一次看到可見候選窗的毫秒數；`committed_text` 是候選窗開著時按
`Enter` 之後 EDIT 的文字，正常要含「我」。`composition` 是 EDIT 目前的組字
文字，盡力取得；讀不到時是空字串，不代表失敗。

PowerShell 直接解析：

```powershell
$report = jc-tsf-smoke --json | ConvertFrom-Json
if (-not $report.ok) {
  $report.checks | Where-Object status -eq 'fail' |
    ForEach-Object { "{0}：{1}`n  修復：{2}" -f $_.title, $_.detail, $_.fix }
}
```

Windows PowerShell 5.1 的管線預設不是 UTF-8；中文標題要正確顯示時先
`chcp 65001`，或改用 PowerShell 7。JSON 語法本身是 ASCII，不影響解析。

## 結束碼

| 結束碼 | 條件 |
|---|---|
| `0` | 全部 `pass`，或只有 `warn`／`skip` |
| `1` | 至少一項 `fail` |
| `2` | 參數錯誤（`--dll` 沒給值、未知參數等） |

JSON 的頂層 `status` 對應：有 `fail` 是 `fail`、沒有 `fail` 但有 `skip` 是
`skip`、其他是 `pass`。CI 用 `status` 分辨「真的驗過」與「跳過」，不是只看
`ok`。

## 每個 fail 的修復指令

| id | 修復 |
|---|---|
| `dll.load` | 確認 `jc-tsf.dll` 與依賴（VC runtime）在同一個目錄；必要時 `--dll` 指定正確路徑 |
| `dll.class_factory` | DLL 與診斷程式的 CLSID 不一致或版本錯誤：重新建置／重新安裝 |
| `dll.vtable` | 把 `ITfKeyEventSink` 實作在 `ITfTextInputProcessor` 的同一個物件（`jc-tsf` 的 `Control`） |
| `tsf.thread_mgr` | 修復 Windows TSF（`msctf.dll`）；這不是 `jc-tsf` 的問題 |
| `tsf.activate` | `set JC_TSF_LOG=1` 後重跑，看 `%LOCALAPPDATA%\jc-ime\tsf.log`；確認傳入的是 TIP client id |
| `registry.tip` | `msiexec /fa <msi>` 修復安裝；再以管理員執行 `jc-tip-register.exe machine-register` |
| `install.files` | `msiexec /fa <msi>`；資料檔遺失時確認 `%ProgramData%\jc-ime\data` 沒被手動刪除 |
| `profile.machine` | 以管理員執行 `jc-tip-register.exe machine-register`；完全沒有機碼時先修復安裝 |
| `profile.user` | 以目前使用者執行 `jc-tip-register.exe user-install`；沒被列舉時改用管理員跑 `machine-register` |
| `profile.language_list` | 以目前使用者執行 `jc-tip-register.exe user-install` |
| `ui.probe` | 啟動 `jc-engine.exe`；`set JC_TSF_LOG=1` 後重跑並看 `%LOCALAPPDATA%\jc-ime\tsf.log`；確認 `user-install` 已完成 |

`engine.pipe`／`engine.detail` 不是 fail：引擎沒開是正常狀態。要看到 `[PASS]`，
從開始功能表或下列指令啟動引擎：

```bat
"%ProgramFiles%\jc-ime\jc-engine.exe" --data "%ProgramData%\jc-ime\data" ^
  --settings "%LOCALAPPDATA%\jc-ime\settings.json"
```

## 預期輸出

### 完整安裝、引擎已啟動

```text
jc-ime 安裝診斷（jc 注音）
不需要管理員；只讀機碼與 TSF API，不呼叫 Register／ActivateProfile。

[INFO] CoInitializeEx(APARTMENTTHREADED) = 0x00000000
[INFO] DLL：C:\Program Files\jc-ime\jc-tsf.dll
[INFO] CLSID {2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}；profile {C4E8D217-5B3F-4A96-8E0C-7F1A2B3D4E56}；LANGID 0x00000404
[PASS] 1. DLL 載入：C:\Program Files\jc-ime\jc-tsf.dll；DllCanUnloadNow = 0x00000000
[PASS] 2. 類廠：DllGetClassObject(CLSID_JC_TSF, IClassFactory) = S_OK（0x00000000）
[PASS] 3. vtable 與物件身分：ITfTextInputProcessor=S_OK、ITfKeyEventSink=S_OK、ITfThreadMgrEventSink=S_OK、ITfCompositionSink=S_OK；ITfKeyEventSink 與 ITfTextInputProcessor 是同一個物件
[PASS] 4. ITfThreadMgr：CoCreateInstance(CLSID_TF_ThreadMgr) = S_OK（0x00000000）
[PASS] 5. Activate：ITfThreadMgr::Activate → 應用程式 client_id=1；ITfClientId::GetClientId → TIP client_id=2；ITfTextInputProcessor::Activate = S_OK（0x00000000）
[PASS] 6. 引擎 pipe：Hello → 協定 3、引擎 jc-engine 0.1.4、session 1；Key(Escape) → State（組字 ""、候選 0 個）
[PASS] 7. 註冊探測：兩個檢視都有 {2F6B41D8-…}：64 位元 → C:\Program Files\jc-ime\jc-tsf.dll（12 個 TIP）、32 位元 → C:\Program Files\jc-ime\jc-tsf-x86.dll（12 個 TIP）
[PASS] 8. 安裝檔案：安裝目錄 C:\Program Files\jc-ime；C:\Program Files\jc-ime\jc-tsf.dll（SHA-256 …，版本 0.1.4.0）；…
[PASS] 9. machine profile 機碼：…\LanguageProfile\0x00000404\{…}：64 位元：Enable=0、Description=jc 注音；32 位元：Enable=0、Description=jc 注音
[PASS] 10. 使用者啟用：IsEnabledLanguageProfile=true；EnumLanguageProfiles(0x0404)=true（共 12 個 profile）；目前啟用 {C4E8D217-…}（本產品）
[PASS] 11. 語言清單：Languages 3 項（含 zh-Hant-TW）；zh-Hant-TW\InputMethodTips 2 項（含 0404:{2F6B41D8-…}{C4E8D217-…}）
[PASS] 12. 引擎細節：引擎 jc-engine 0.1.4；協定 3；session 1；健康狀態 正常；連線 1、組字中 0、引擎 session 數 1、pid 4242
[PASS] 13. UI 探針：desktop=interactive；composition=；candidate_visible=true；candidate_rect=104,142,304,182；latency_ms=87；escape_hides=true；committed_text=我；候選窗在插入點附近出現、Enter 送出「我」、Esc 後消失
[INFO] 釋放物件後 DllCanUnloadNow = 0x00000000（S_OK 代表沒有 refcount 洩漏）

結果：13 通過、0 警告、0 失敗
總結：全部通過
```

### 引擎沒開、但輸入法裝好了

檢查 6、12 是 `[SKIP]`。檢查 13 要看按鍵有沒有進到 TIP：

- 候選窗出現，Enter 送出「我」、Esc 也能關掉：判 `[PASS]`，即使
  `composition` 是空的也一樣。
- 讀得到 `composition`、只是候選窗沒出現：候選字來自引擎，記 `[FAIL]`。
- 連 `composition` 都讀不到、候選窗完全沒出現：TIP 根本沒接到按鍵，記
  `[SKIP]`（inconclusive）。這是環境問題，不能當成產品 bug。

有組字但沒有候選窗的失敗長這樣：

```text
[SKIP] 6. 引擎 pipe：pipe jc-ime-1001 不存在（引擎未啟動，或開機 Run 機碼還沒生效）；…；引擎未啟動或未安裝，只警告不判定失敗
[SKIP] 12. 引擎細節：引擎連不上（pipe-missing）：pipe jc-ime-1001 不存在…；引擎未啟動時無法取得版本與健康狀態
[FAIL] 13. UI 探針：desktop=interactive；composition=ㄨㄛˇ；candidate_visible=false；candidate_rect=；latency_ms=2000；escape_hides=false；committed_text=；2000ms 內沒有看到本行程可見的 jcCandidateUiWindow；詳細原因看 %LOCALAPPDATA%\jc-ime\tsf.log（設定 JC_TSF_LOG=1 後重跑）
       修復：啟動 jc-engine（候選字來自引擎）…

結果：10 通過、0 警告、1 失敗、2 略過
總結：安裝診斷失敗；請依上面各項的「修復」處理後重跑（1 項失敗）
```

連組字都讀不到、也沒有候選窗的 inconclusive：

```text
[SKIP] 13. UI 探針：desktop=interactive；composition=；candidate_visible=false；candidate_rect=；latency_ms=2000；escape_hides=false；committed_text=；讀不到組字字串且候選窗沒有出現：TIP 沒有接到按鍵，這是環境問題，無法判定產品行為；這是探針環境問題，不能判定產品失敗；…

結果：10 通過、0 警告、0 失敗、3 略過
總結：全部通過
```

沒有互動桌面時（CI runner、Session 0）檢查 13 也是 `[SKIP]`，結果仍是
「全部通過」。

### 典型安裝後失敗

```text
[FAIL] 9. machine profile 機碼：32 位元檢視的 …\LanguageProfile\0x00000404\{C4E8D217-…} 不存在；64 位元：Enable=0、Description=jc 注音
       修復：以管理員執行 jc-tip-register machine-register；若機碼完全不存在，先修復安裝（msiexec /fa <msi>）
[FAIL] 10. 使用者啟用：IsEnabledLanguageProfile=false；EnumLanguageProfiles(0x0404)=true（共 12 個 profile）；目前啟用 {00000000-…}（不是本產品）
       修復：以目前使用者執行 "C:\Program Files\jc-ime\jc-tip-register.exe" user-install
[FAIL] 11. 語言清單：Languages 2 項（缺 zh-Hant-TW）；zh-Hant-TW\InputMethodTips 0 項（缺本產品）
       修復：以目前使用者執行 "C:\Program Files\jc-ime\jc-tip-register.exe" user-install

結果：8 通過、1 警告、3 失敗
總結：安裝診斷失敗；請依上面各項的「修復」處理後重跑（3 項失敗）
```

## 每個失敗項目的可能原因

| # | 看到的 HRESULT／現象 | 可能原因 |
|---|---|---|
| 1 | `0x8007007E` | 找不到 DLL 或它依賴的 DLL；`jc-windows`／VC runtime 沒在旁邊 |
| 1 | `0x800700C1` | 架構不符（32 位元 DLL 給 64 位元 exe，反之亦然） |
| 1 | `0x80070005` | DLL 檔案被鎖住或沒有讀取權限 |
| 1 | 找不到 `DllCanUnloadNow` | 載到別的 DLL、匯出被 strip，或建置腳本漏了 `#[no_mangle]` |
| 2 | `0x80040111`（`CLASS_E_CLASSNOTAVAILABLE`） | `DllGetClassObject` 認的 CLSID 與診斷程式不一致（`guids.rs` 被改過） |
| 2 | `0x80004002`（`E_NOINTERFACE`） | `DllGetClassObject` 沒有正確把 `riid` 轉出去 |
| 2 | `0x8007007E` | 沒有匯出 `DllGetClassObject` |
| 3 | `0x80004002` | 某個介面沒列在 `#[implement(...)]`（例如 `ITfThreadMgrEventSink` 被拿掉） |
| 3 | QI 全過但「不是同一個物件」 | `ITfKeyEventSink` 被做成獨立物件；msctf 會拒絕，按鍵鏈不會動 |
| 4 | `0x80040154`（`REGDB_E_CLASSNOTREG`） | `msctf.dll` 不在或系統精簡；不是本 DLL 的問題 |
| 4 | `0x800401F0`（`CO_E_NOTINITIALIZED`） | 前面 `CoInitializeEx` 失敗（`[INFO]` 那行不是 `0x00000000`） |
| 5 | `ITfThreadMgr::Activate` 失敗 | TSF 已被同執行緒的其他程式佔用、COM apartment 不對 |
| 5 | `ITfTextInputProcessor::Activate` 回 `0x80004005` | `Control::start` 建訊息視窗或 `AdviseKeyEventSink` 失敗（看 `OutputDebugStringW` 的 `[jc-tsf]` 記錄） |
| 5 | `ITfTextInputProcessor::Activate` 回 `0x80070057`（`E_INVALIDARG`） | `AdviseKeyEventSink` 拿到的 tid 不是 TIP client id。檢查 5 會分別印出兩個 id，見 `docs/port/activate-second-call.md` |
| 5 | `ITfTextInputProcessor::Activate` 回 `0x80040201`（`CONNECT_E_ADVISELIMIT`） | 已安裝時 msctf 先激活了同一個 TIP；判為通過，不是失敗 |
| 5 | `0x8001010E`（`RPC_E_WRONG_THREAD`） | TIP 物件被跨 apartment 呼叫；`Activate` 必須在建立它的執行緒 |
| 6 | `pipe-missing` | 引擎沒開、Run 機碼沒生效、pipe 名稱不同（`HKCU\Software\jc-ime\EnginePipe`） |
| 6 | `access-denied` | 別的使用者或安全性原則佔用 pipe；用同一個使用者啟動引擎 |
| 6 | `handshake-failed` | 引擎與 `jc-tsf.dll` 不是同一個版本，或引擎在握手時關閉 |
| 6 | `protocol-mismatch` | `jc-proto` 的 `PROTOCOL_VERSION` 不同；一起更新引擎與 DLL |
| 6 | `timeout` | 引擎卡住或超過 500 ms；這條連線之後不可信，要重開 |
| 7 | `RegOpenKeyExW … 0x00000002` | 系統沒有任何 TSF TIP（極少見），不影響其他檢查 |
| 7 | 沒找到 CLSID | 還沒安裝；`[SKIP]` 不代表壞掉 |
| 8 | 檔案不存在 | MSI 沒裝完、被防毒隔離，或 `%ProgramData%\jc-ime\data` 被清掉 |
| 8 | 沒有版本資源 | 建置沒嵌版本；只 `[WARN]`，不影響載入 |
| 9 | 只有一邊檢視 | `machine-register` 沒跑完，或 32 位元檢視被 CTF 轉向 |
| 9 | 只有 `0x0404` 機碼 | 早期安裝器的非標準名稱；退回讀取後只 `[WARN]`，msctf 實際讀哪個名稱需實機確認 |
| 10 | `IsEnabledLanguageProfile=false` | 使用者沒跑 `user-install`，或 `EnableLanguageProfile` 失敗 |
| 10 | `EnumLanguageProfiles` 沒有本產品 | machine profile 沒寫好，先跑 `machine-register` |
| 11 | 缺 `zh-Hant-TW` | 使用者的語言清單沒有繁體中文（台灣） |
| 11 | 缺 `InputMethodTips` | `InstallLayoutOrTip` 沒跑，或跑在使用者沒有權限的情境 |

## `jc_test_key`

`crates/jc-tsf/src/lib.rs` 的測試匯出，走「虛擬鍵 → `keys::translate` → 引擎
`Key`」的內部路徑；沒有 TSF context，所以只回引擎的回應碼，不套用組字。

| 回傳值 | 意義 |
|---|---|
| `0` | 按鍵已送出，引擎回應正常 |
| `1` | 這個鍵不歸輸入法處理（或 `vk` 不是合法虛擬鍵碼） |
| `2` | 連不上引擎 pipe |
| `3` | 引擎往返失敗（逾時、協定錯誤、連線已關閉） |

診斷程式用它量測「DLL → 引擎」這一段；結果只印 `[INFO]`，不影響結束碼。原因：
它自己開一條 pipe 連線，會與檢查 6 的時間序互相影響，不適合當成硬性判定。

## 移植對照

Go 參考實作（`/mnt/work/jc/ref/0c-ime`）沒有 TSF 外殼、沒有 Windows 平台層，
所以這個 task 幾乎全部是 n/a；實際沿用到的只有既有的按鍵定義與協定。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/proto/*`（訊框與訊息） | `crates/jc-proto/src/lib.rs`（既有） | ported；診斷直接送 `Hello`／`Key`／`Health`，不重寫訊框 |
| `internal/zh/keyboard.go`、`internal/zh/input.go` | `crates/jc-core/src/zh/keyboard.rs`、`key.rs`（既有） | ported；`jc_test_key` 經 `jc-tsf::keys::translate` 沿用，沒有另抄鍵位表 |
| n/a（Go 沒有 TSF 實作） | `crates/jc-tsf-smoke/src/main.rs`、`checks.rs` | rewritten；十三項檢查的流程與 `--json` |
| n/a | `crates/jc-tsf-smoke/src/dll.rs`（`LoadLibraryW`、`DllGetClassObject`、`DllCanUnloadNow`、`jc_test_key`） | rewritten |
| n/a | `crates/jc-tsf-smoke/src/product.rs`（CLSID、profile GUID、LANGID） | rewritten；值與 `jc-tsf/src/guids.rs` 相同（cdylib 無法 import，只能留一份） |
| n/a | `crates/jc-tsf-smoke/src/profile.rs`（機碼路徑、`InputMethodTips` 比對、安裝目錄） | rewritten；純字串，Linux 也編譯與測試 |
| n/a | `crates/jc-tsf-smoke/src/registry.rs`（`CTF\TIP`、`LanguageProfile`、`InprocServer32`、HKCU 語言清單，64 位元與 32 位元兩個檢視） | rewritten |
| n/a | `crates/jc-tsf-smoke/src/install.rs`（安裝檔案、SHA-256、PE 版本資源） | rewritten |
| n/a | `crates/jc-tsf-smoke/src/tsf_profile.rs`（`IsEnabledLanguageProfile`、`EnumLanguageProfiles`、`GetActiveProfile`） | rewritten |
| n/a | `crates/jc-tsf-smoke/src/engine.rs`、`engine_failure.rs`（引擎 pipe、健康狀態、失敗分類） | rewritten；連線與握手交給 `jc-windows` |
| n/a | `crates/jc-tsf-smoke/src/ui_probe.rs`（host 視窗、`SendInput`、候選窗輪詢） | 新增；沒有互動桌面時回 `[SKIP]` |
| n/a | `crates/jc-tsf-smoke/src/ui_probe_rules.rs`（rect／插入點／提交文字／Esc 的判定） | 新增；純函式，Linux 也有單元測試 |
| n/a | `crates/jc-tsf-smoke/src/report.rs`（人類輸出與 `jc-ime-doctor/1` JSON） | rewritten；Linux 也會編譯 |
| n/a | `installer/post-install-doctor.ps1`（安裝後執行、log、剪貼簿與訊息框） | 新增；install-smoke 共用 |
| n/a（Go 沒有 Windows 外殼） | `crates/jc-tsf/src/lib.rs` 的 `jc_test_key` | rewritten；`debug_assertions` 或 feature `test-export` 才編譯 |

參考實作是 <https://github.com/LeafHW/hufu-ime-rust> 的
`platform/windows/hufu-tsf-smoke`（不是 Go）。沿用它的順序：DLL → 類廠 →
`CreateInstance` → msctf `ITfThreadMgr` → `Activate` → 按鍵鏈 → 引擎直驅；
但它會 `DllRegisterServer`／`Register`，本程式刻意不做。參考實作用
`windows-rs` 0.58；這裡跟 workspace 一致用 0.62，API 形狀相同。

## 驗證

| 指令 | 結果 |
|---|---|
| `cargo check -p jc-tsf-smoke --all-targets`（Linux） | 通過；`main` 是佔位實作，`report.rs`／`profile.rs`／`engine_failure.rs` 與其單元測試一起編譯 |
| `cargo check -p jc-tsf-smoke --all-targets --target x86_64-pc-windows-msvc` | 通過，0 警告 |
| `cargo check -p jc-tsf-smoke --all-targets --target i686-pc-windows-msvc` | 通過，0 警告 |
| `cargo clippy -p jc-tsf-smoke --all-targets -- -D warnings`（Linux） | 零警告 |
| `cargo clippy -p jc-tsf-smoke --all-targets --target x86_64-pc-windows-msvc -- -D warnings` | 零警告 |
| `cargo clippy -p jc-tsf-smoke --all-targets --target i686-pc-windows-msvc -- -D warnings` | 零警告 |
| `cargo test -p jc-tsf-smoke`（Linux） | 40 passed |

實際的 `jc-tsf-smoke.exe` 必須在 Windows 上建置與執行；Linux 的 `cargo check`
不連結，只驗證型別與純函式測試。

## 未驗證清單（需要實機）

以下全部沒有在 Windows 上跑過，Linux 只能驗證編譯與純函式：

1. 十三項檢查的實際輸出與結束碼；上面「預期輸出」是照程式碼推導的。
2. 沒有 document manager 時 `ITfThreadMgr::Activate` + `AdviseKeyEventSink`
   是否真的回 `S_OK`（若有 application 正在使用 TSF，`Activate` 可能被拒）。
3. `ITfKeystrokeMgr::TestKeyDown` 是否真的把事件送到本 TIP 的
   `OnTestKeyDown`；`consumed` 依引擎連線狀態而變。
4. 檢查 5 的 `Deactivate` 到檢查 6 之間，200 ms 是否足以讓 `jc-tsf` 的 worker
   關掉 pipe（引擎可接受多條連線，但這裡仍要確認舊連線已釋放）。
5. `DllCanUnloadNow` 在釋放所有物件後是否回到 `S_OK`（refcount 洩漏）。
6. 中文輸出在 `cmd.exe` 的顯示（Rust 對 console handle 使用 `WriteConsoleW`，
   理論上正確，未實測）。
7. 32 位元 TIP 的註冊位置（`WOW6432Node`）：檢查 7 只驗到讀得到
   `InprocServer32` 的預設值，沒有實際讓 32 位元程式載入那份 DLL。
8. `IsEnabledLanguageProfile`／`EnumLanguageProfiles` 在「只有 64 位元 profile
   機碼」時的實際回報；這決定檢查 9 的 32 位元檢視失敗是否會連帶影響檢查 10。
9. `GetActiveProfile` 在沒有作用中 profile 時的 HRESULT；目前一律當成
   「不是本產品」寫進 `detail`，不影響判定。
10. 引擎正在被真正的輸入法使用時，檢查 6／12 的行為（可能 `[SKIP]`）。
11. `%ProgramData%` 被改到非預設位置時，檢查 8 的 `manifest.json` 是否找得到。
12. 檢查 13 在真機上的實際行為：`SendInput` 的掃描碼是否讓 msctf 產生正確的
    注音組字、`↓` 是否真的讓候選窗出現、候選窗 rect 與 `GetCaretPos` 的相對
    位置是否落在容許範圍內、`Enter` 是否送出「我」、`Esc` 後候選窗是否真的在
    1000ms 內消失。CI runner 是 Session 0，只會走到 `desktop=headless` 的
    `[SKIP]`。
13. 150%／200% DPI 下 `SetProcessDpiAwarenessContext` 與 rect 斷言的實際值；
    Linux 只驗到換算公式。

## 已知限制

- 只驗證安裝與外殼，不驗證組字外觀；那要實際打字或由
  `docs/port/tsf-skeleton.md` 的未驗證清單接手。檢查 13 只驗候選窗的存在、
  位置與消失，不比較字型或配色。
- 檢查 7 只讀 `HKLM`，不讀 `HKCU`；`regsvr32` 以管理員安裝時寫的是 `HKLM`。
- 檢查 9 的 `Enable` 是 machine 預設值，不是「目前使用者已啟用」；後者只看
  檢查 10。
- 引擎檢查會送一個 `Escape`。這是刻意的：即使引擎正在別人的組字中，這個鍵
  只會清掉狀態，不會插入字元。
- 不支援平行執行兩份診斷程式：引擎 pipe 只有一個 instance。
- `--json` 的 stdout 是 UTF-8；Windows PowerShell 5.1 需要 `chcp 65001` 才能
  正確顯示中文，但 JSON 語法本身不受影響。
