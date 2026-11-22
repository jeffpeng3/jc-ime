# 升級與全新安裝（原 T9 平行建置已移除）

Worktree：`/mnt/work/jc/wt/remove-old-impl`
Branch：`remove-old-impl`（base：`main`，rc.15）

rc.15 是最後一個同時出貨新舊兩版 TSF 的版本。下一次 rc 移除舊版，並把所有
`next` 後綴拿掉：對外只有一組 `jc-tsf.dll`／`jc 注音`。本文件記錄現行程序；
T9 的平行灰度與回退已不適用，身分對照表留作歷史紀錄。

## 現行身分（唯一一組）

- CLSID `{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}`
- profile `{C4E8D217-5B3F-4A96-8E0C-7F1A2B3D4E56}`
- 描述 `jc 注音`
- DLL `jc-tsf.dll`／`jc-tsf-x86.dll`
- pipe `jc-ime-<SID 尾段>`，覆寫變數 `JC_IME_PIPE`
- 個人資料 `personal.json`
- 監督者鎖 `Local\jc-ime-engine-supervisor`，log `supervisor.log`
- 語言 ID `0x0404`（zh-TW）

CLSID 與 profile 是 rc.15 已發行值，保留不改；只把描述從「jc 注音（新版）」
收斂成「jc 注音」。常數定義在 `crates/jc-tsf/src/guids.rs`、
`crates/jc-tsf-register/src/guids.rs` 與 `installer/jc-tip-register/src/guids.rs`。

### 已移除的舊身分（歷史對照）

- CLSID `{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}`、profile
  `{8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}`，描述也是 `jc 注音`。
- rc.15 的「jc 注音（新版）」用的是現行 CLSID/profile，只有描述不同。

升級後這兩筆 HKCU 語言清單項目會變成孤兒，由 `user-cleanup-legacy` 清除。
常數改名為 `*_LEGACY`，只給清理程式對照，不再註冊。

## 註冊與 pipe

`crates/jc-tsf-register` 提供 `Registration`，欄位是 `clsid`、`profile`、
`description`、`langid`、`pipe_prefix`。`Registration::default()` 就是現行身分，
`DllRegisterServer` 呼叫 `register()`，反註冊呼叫 `unregister()`。
`register_with()`／`unregister_with()` 保留給需要覆寫身分的場合，並在 CLSID
機碼寫 `PipePrefix`，讓啟動器知道該連哪一條引擎 pipe。

`crates/jc-tsf/src/ipc.rs` 用 `jc-ime` 前綴解析 pipe，覆寫變數是 `JC_IME_PIPE`。
訊框是 4-byte 小端長度 + JSON，上限 1 MiB。`legacy_server_pipe()` 與
`Client::with_legacy_pipe()` 只供舊文件對照，現行 TIP 不連舊管道。

`crates/jc-engine` 的 `--personal-prefix` 預設 `personal`，存成 `personal.json`；
`--instance` 預設 `main`。個人化**不搬移**：全新安裝從空白開始，rc.15 留下的
`personal-next.json` 不讀不寫，也不會被覆寫。

## 引擎啟動

MSI 的 HKLM Run 值只有一筆 `jc-ime-engine`，內容由 PowerShell 在登入時算出
目前使用者的 SID 尾段，再啟動：

`jc-engine --supervise --data <資料根目錄> --settings %LOCALAPPDATA%\jc-ime\settings.json --socket jc-ime-$rid --personal-prefix personal`

不帶 `--instance`，用引擎預設的鎖與 log 名稱。`--socket` 讓引擎與 TIP 用同一條
pipe；環境變數 `JC_IME_PIPE` 可取代它，但 TIP 跑在宿主行程裡，逐 TIP 設環境
變數不可靠，所以 MSI 用 `--socket`。

## 安裝與升級

1. Windows Installer 的 `MajorUpgrade` 先移除舊產品：舊 CLSID 機碼、舊 Run 值、
   舊 DLL 與舊 profile 都會被清掉。`NOT UPGRADINGPRODUCTCODE` 讓
   `RemoveTipLayout` 在升級時跳過，所以 `%LOCALAPPDATA%\jc-ime` 保留。
2. `WriteRegistryValues` 寫入現行 CLSID 的 `InprocServer32`（兩個檢視）與
   `PipePrefix`，接著 `RegisterTip` 註冊 profile 與五個分類。
3. `InstallFinalize` 後以使用者身分執行 `user-cleanup-legacy`，呼叫
   `RemoveLanguageProfile` 清掉舊 CLSID 與 rc.15 舊描述項目的 HKCU 清單紀錄。
   失敗不擋安裝（`Return="ignore"`）。
4. `InstallTipLayout` 用現行身分把單一「jc 注音」加回語言清單。
5. 登出再登入。登出讓所有行程釋放舊 DLL，登入讓新的 Run 值生效；確認只跑一個
   `jc-engine.exe`，`supervisor.log` 沒有 `AlreadyRunning`。

清理動作只處理執行安裝的使用者帳號。其他帳號的舊清單項目要由各帳號自行跑一次
`jc-tip-register.exe user-cleanup-legacy`，或在語言設定手動移除。

## 驗證清單

登入後在同一項「jc 注音」下測：

1. **Enter double-commit**：打注音後按一次 Enter，畫面只提交一次。
2. **第二次 Activate**：切到別的輸入法再切回，連續兩次；日誌不出現 `TF_E_NOLOCK`。
3. **kill 引擎後自動恢復**：結束 `jc-engine.exe` 後立刻打字，5 秒內恢復，
   `supervisor.log` 有重啟紀錄。
4. **LangBar**：右鍵選單三項可用，`設定…` 不會帶出主控台。
5. **su3cl3**：得到「你好」，不是英文補全，數字不當選字鍵。
6. **32/64 位元**：舊版 Office 與 64 位元程式都用同一項；`reg query /reg:32`
   與 `/reg:64` 都指向 `jc-tsf*.dll`。

另外確認：輸入法清單只有一項「jc 注音」，沒有「jc 注音（新版）」或重複項目；
舊 CLSID 機碼與舊 DLL 檔案消失；個人資料還在（`personal.json` 為空白或原有資料，
`personal-next.json` 若仍存在不讀）；登出登入後只有一個引擎行程。

## 解除安裝

`KEEPUSERSETTINGS=0`（預設）時 `user-uninstall` 連 `%LOCALAPPDATA%\jc-ime`
一起清掉；`KEEPUSERSETTINGS=1` 保留。解除安裝不呼叫 `ILOT_UNINSTALL`，避免移除
語系最後一個 layout 時把系統地區改成英文。語言清單項目由使用者自行移除，
或先跑 `user-cleanup-legacy`。

## Windows 實機還要驗證

- `RemoveLanguageProfile` 跨 CLSID 是否真的移除 HKCU 項目，且不動系統地區。
- 描述改成「jc 注音」後，HKCU 顯示名稱是否更新；若沒有，需要用
  `ITfInputProcessorProfilesEx::SetLanguageProfileDisplayName` 補。
- 舊 DLL 被 explorer 鎖住時 MSI 是否排重開機；文件要引導登出再登入。
- 升級時 `NOT UPGRADINGPRODUCTCODE` 是否確實保留個人資料。

## Port 對照

| 0c Rust 來源 | Rust 檔案 | 狀態 |
|---|---|---|
| `platforms/windows/tsf/src/guids.rs` | `crates/jc-tsf/src/guids.rs` | rewritten：現行身分沿用 rc.15 新值，描述收斂成「jc 注音」 |
| `platforms/windows/tsf/src/lib.rs`、`factory.rs` | `crates/jc-tsf/src/dll.rs` | ported：COM 外殼與 class factory，`DllRegisterServer` 用現行 `Registration` |
| `register.rs` | `crates/jc-tsf-register/src/register.rs` | rewritten：`Registration::default()` 是現行身分 |
| `tsf/src/ipc.rs` | `crates/jc-tsf/src/ipc.rs` | rewritten：前綴 `jc-ime`，覆寫變數 `JC_IME_PIPE`，訊框格式不變 |
| `cmd/0c-server/transport_windows.go` | `crates/jc-engine/src/main.rs` | rewritten：`--personal-prefix`／`--instance` 保留，WXS 不再帶 `--instance` |
| `core/session` | `crates/jc-engine/src/lib.rs` | rewritten：個人資料前綴可參數化，預設 `personal` |
