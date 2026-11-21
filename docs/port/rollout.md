# T9：平行建置、灰度與回退

Worktree：`/mnt/work/jc/wt/t9-parallel-build`
Branch：`t9-parallel-build`（base：`tsf-overhaul`）
目的：讓新版 TSF（`jc-tsf-next`）與已發行的舊版 TSF（`jc-tsf`）同時註冊，
用系統輸入法切換，並能不改資料格式地回退。

## 新舊身分對照

舊版（已發行，不可改）：

- CLSID `{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}`
- profile `{8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}`
- 描述 `jc 注音`
- DLL `jc-tsf.dll`／`jc-tsf-x86.dll`
- pipe `jc-ime-<SID 尾段>`，覆寫變數 `JC_IME_PIPE`
- 個人資料 `personal-<session id>.json`
- 監督者鎖 `Local\jc-ime-engine-supervisor`，log `supervisor.log`

新版（T9 新配）：

- CLSID `{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}`
- profile `{C4E8D217-5B3F-4A96-8E0C-7F1A2B3D4E56}`
- 描述 `jc 注音（新版）`
- DLL `jc-tsf-next.dll`／`jc-tsf-next-x86.dll`（由 integrator 建置，見「發版還需要 integrator 做的事」）
- pipe `jc-ime-next-<SID 尾段>`，覆寫變數 `JC_IME_NEXT_PIPE`
- 個人資料 `personal-next-<session id>.json`
- 監督者鎖 `Local\jc-ime-engine-supervisor-next`，log `supervisor-next.log`
- 语言 ID 仍是 `0x0404`（zh-TW）

新值定義在 `crates/jc-tsf-next/src/guids.rs` 的「T9：平行建置的新身分」段，
舊值定義在 `crates/jc-tsf/src/guids.rs` 與 `installer/jc-tip-register/src/guids.rs`。
兩邊都加了測試鎖住「新值不等於舊值」，避免日後誤改回來。

## 註冊參數化

`crates/jc-tsf/src/register.rs` 新增 `Registration`，欄位是 `clsid`、`profile`、
`description`、`langid`、`pipe_prefix`：

- `register()`／`unregister()` 改用 `Registration::default()`，值與原本的
  `CLSID_JC_TSF`／`GUID_PROFILE_JC_TSF`／`PROFILE_DESCRIPTION`／`LANGID_ZH_TW` 完全相同，
  舊 DLL 的 `DllRegisterServer`／`DllUnregisterServer` 行為不變。
- 新版 DLL 的 COM 外殼呼叫 `register_with(&Registration { ... })`，傳入
  `CLSID_JC_TSF_NEXT`、`GUID_PROFILE_JC_TSF_NEXT`、`PROFILE_DESCRIPTION_NEXT`、
  `PIPE_PREFIX_NEXT`。反註冊用 `unregister_with()`，只刪自己那組 CLSID 與 profile，
  不會碰到舊版。
- `register_with()` 除了 `InprocServer32` 與三個分類，還在 CLSID 機碼寫一個
  `PipePrefix` 值，讓啟動器能讀到這組身分對應的 pipe 前綴。Windows 忽略不認識的值。
- `Registration::engine_pipe_name(rid)` 把前綴與 SID 尾段組回 pipe 名稱，供新版
  外殼與測試共用。

`regsvr32 jc-tsf-next.dll` 會走新版自己的 `DllRegisterServer` 並用新值註冊；
MSI 路徑需要在 `<Class>` 與 `jc-tip-register` 另外加一組新常數，見最後一節。

## pipe 與資料路徑隔離

`crates/jc-tsf-next/src/ipc.rs`：

- `PIPE_PREFIX` 改成 `jc-ime-next`，`PIPE_ENV` 改成 `JC_IME_NEXT_PIPE`。
- 新版 TIP 不讀舊版 `JC_IME_PIPE`。舊引擎的環境覆寫不會把新版 TIP 導到舊 pipe；
  若要手動覆寫新版，設 `JC_IME_NEXT_PIPE`。
- `legacy_server_pipe()` 仍指向舊前綴，只供文件對照；新版 adapter 預設不連它。
- 訊框仍是 4-byte 小端長度 + JSON，上限 1 MiB，兩版共用。回退後舊引擎讀得到
  同一份 lexicon、language model 與個人語料，不需要轉換或搬檔。

`crates/jc-engine`：

- 新增 `--personal-prefix <name>`，預設 `personal`（`personal-<id>.json`），
  新版用 `personal-next`（`personal-next-<id>.json`）。前綴從 `Engine` 傳到每條連線的
  worker，只改檔名，不改資料根目錄。`--data` 兩版指同一個根目錄即可。
- 新增 `--instance <name>`，預設 `main`，新版用 `next`。它只改監督者的鎖名與
  log 檔名，避免第二個 `jc-engine --supervise` 永遠收到 `AlreadyRunning`。
- 兩個參數都會由監督者轉給子行程（`--instance` 只給監督者自己）。
- `--socket` 照舊由 `jc-engine` 接收並轉給子行程，健康檢查也連它。
- Windows 監聽端仍用 `CreateNamedPipeW` + 0c 逐字相同的 SDDL；pipe 名稱不同，
  不需要改 SDDL 或 buffer 設定。

## 新版引擎需要的 CLI 參數

舊版（不改）：

`jc-engine --supervise --data <資料根目錄>`

新版：

`jc-engine --supervise --data <資料根目錄> --socket jc-ime-next-<rid> --personal-prefix personal-next --instance next`

`<rid>` 是目前使用者的 SID 尾段，可用 `whoami /user` 查。用 `--socket` 明確指定是
推薦做法：新舊引擎各自跑一個監督者與一條 pipe，不需要在系統或登入指令設定環境變數。
若 integrator 選擇環境變數路線，新版引擎與新版 TIP 所在行程都要有
`JC_IME_NEXT_PIPE=jc-ime-next-<rid>`；TIP 跑在宿主行程裡，逐 TIP 設環境變數不可靠，
所以仍建議 `--socket`。

`--allow-empty-data`、`--settings`、`--mode`、`--neural`、`--max-frame` 的行為不變。
新版不需要獨立的 engine 執行檔，沿用同一支 `jc-engine.exe` 即可。

## 安裝順序

1. 先安裝已發行的舊版 MSI，確認 `jc 注音` 出現在系統輸入法清單且能打字。
2. 建置新版 DLL（`jc-tsf-next.dll`／`jc-tsf-next-x86.dll`）與新 CLSID 的註冊資料，
   這一步由 integrator 完成，見最後一節。
3. 用系統管理員註冊新版：`regsvr32 jc-tsf-next.dll`；32 位元宿主用
   `regsvr32 jc-tsf-next-x86.dll`。新舊 CLSID 不同，可以同時存在。
4. 啟動新版引擎監督者（上面的 CLI），確認 `%LOCALAPPDATA%\jc-ime\supervisor-next.log`
   出現「監督者啟動」且 pipe 名稱是 `jc-ime-next-<rid>`。
5. 登出再登入，讓 Explorer 與各行程重新讀取輸入法清單。此時清單應同時出現
   `jc 注音` 與 `jc 注音（新版）`。

不要改舊版 CLSID、profile、描述、pipe 前綴或個人資料檔名。灰度期間只新增，不替換。

## 系統輸入法切換

- 在工作列輸入法圖示按右鍵 → `設定` → `時間與語言` → `語言與地區` → 語言
  `中文（繁體，台灣）` → `選項` → `鍵盤` → 加入 `jc 注音（新版）`。
- 用 `Win + Space` 或 `Shift + Alt` 在 `jc 注音` 與 `jc 注音（新版）` 之間切換。
- 兩項同時存在代表平行註冊成功；切換後在記事本各打幾個字，確認各自連到自己的引擎。
- 新版 DLL 還沒有 COM 外殼時，清單只會出現舊版；這是已知缺口，不是註冊失敗。

## 驗證清單

每項都在新版 `jc 注音（新版）` 下測，並與舊版交叉比對：

1. **Enter double-commit**：在記事本打注音後按一次 Enter，畫面只提交一次，不會多出
   一個字或多一行。對應 T5 的 `range.SetText` 取代、`holds_commit` 讀回與 `committed` 旗標。
2. **第二次 Activate**：切到別的輸入法再切回新版，連續兩次；日誌不出現 `TF_E_NOLOCK`，
   Deactivate 後 worker 仍在，接著打字正常。對應 T6 的 idempotent guard 與「只清 UI
   不拆 client」。
3. **kill 引擎後自動恢復**：工作管理員結束新版 `jc-engine.exe`，立刻打字；5 秒內新版
   引擎重新連上，斷線前的組字狀態由 log 重播回來。`supervisor-next.log` 有重啟紀錄，
   舊版監督者與舊版打字不受影響。
4. **LangBar**：新版右鍵選單三項可用，`設定…` 啟動 `jc-settings-ui.exe` 不會帶出主控台，
   路徑含空白也正常。對應 T6 + T9 的 `Command::args()` 與 `CREATE_NO_WINDOW`。
5. **su3cl3**：輸入 `su3cl3` 應得到「你好」，不是英文補全 `su...`；數字不會被當成
   英文選字鍵，也沒有額外的 `Down` 前綴。對應 T1 的 suggestion 映射與 T3 的 op。
6. **兩條連線並存**：舊版與新版同時各開一個記事本打字，互不干擾；引擎 `health` 回傳
   兩個以上 connections／sessions。再確認新版個人資料只有 `personal-next-*.json`，
   舊版只有 `personal-*.json`，沒有互寫。

任何一項失敗就停在灰度，不擴大安裝範圍，也不要回頭改舊版值。

## 回退程序

1. 登出 Windows（不是只切換輸入法），讓所有行程 release 新版 DLL。
2. 登入後到 `設定 → 時間與語言 → 語言與地區 → 中文（繁體，台灣） → 選項 → 鍵盤`，
   移除 `jc 注音（新版）`，或把預設輸入法切回 `jc 注音`。
3. 停止新版引擎監督者，保留舊版引擎繼續服務。新版 DLL 與新版 CLSID 機碼可以留著，
   只要不選它就不會載入；要徹底移除再執行 `regsvr32 /u jc-tsf-next.dll`。
4. 資料不需要搬移或轉換。新版個人學習存在 `personal-next-<id>.json`，舊版讀的是
   `personal-<id>.json`；回退後舊版語料完整保留，新版資料留在目錄裡供下次灰度使用。
5. 回退後重跑驗證清單的 1、2、5 三項，確認舊版行為未受影響。

資料格式共用（4-byte LE + JSON）是回退不必搬檔的前提；不要用版本號不同的帧格式，
也不要把新版個人資料改名覆蓋舊檔。

## 發版還需要 integrator 做的事

- 建置新版 DLL：`jc-tsf-next` 目前沒有 `DllGetClassObject`／`DllRegisterServer`／
  `DllCanUnloadNow`，也沒有把 `service::TextService` 接上 COM class factory。需要補一層
  外殼，並讓它的 `DllRegisterServer` 呼叫 `register_with()` 傳入新 `Registration`。
  `jc-tsf` 是 `cdylib`，要讓新版外殼引用 `register::Registration` 需另外加 `rlib` crate type
  （或把 `Registration` 移到共用 crate），這是整合階段的決定。
- MSI：`installer/jc-ime.wxs` 要為新版 DLL 增加一組 `<Class Id="{2F6B41D8-...}">`，
  指向 `jc-tsf-next.dll`／`jc-tsf-next-x86.dll`；`PipePrefix` 值也要進 Registry table，
  或改由新版外殼的 `DllRegisterServer` 在 regsvr32 路徑寫入。
- `installer/jc-tip-register/src/guids.rs` 只剩舊版常數，`machine-register`／`user-install`
  也寫死舊身分。要新增一組新常數與子命令（例如 `machine-register-next`／
  `user-install-next`），用新 CLSID 與 profile 註冊，描述用 `jc 注音（新版）`。
- 新版引擎 autostart（Run 機碼或工作）要獨立一筆，帶 `--instance next`、
  `--personal-prefix personal-next`、`--socket jc-ime-next-<rid>`；不要覆蓋舊版 autostart。
- 灰度穩定後若要「升級而非平行」，再把舊版 CLSID 的 `InprocServer32` 指向新版 DLL；
  這一步會動到已發行值，必須另外走變更流程，不屬於 T9。

## 限制

- 本任務不跑測試，只做 `cargo check`；行為驗證在灰度清單，由整合階段在 Windows 實機執行。
- 新版 DLL 的 COM 外殼尚未存在，所以「系統清單同時看到兩項」目前需要先完成上一節的整合。
- `PROFILE_DESCRIPTION`、`CLSID_JC_TSF`、`PIPE_PREFIX_OLD` 之類的舊常數沒有更名，
  避免這次改動影響已發行程式。

## Port 對照

來源是 0c 的 Rust 與 jc 既有 Raspberry Pi 式移植紀錄；第一欄沿用共通規則的名稱。

| Go 檔案（0c Rust 來源） | Rust 檔案 | 狀態 |
|---|---|---|
| `platforms/windows/tsf/src/guids.rs`（CLSID／profile／描述） | `crates/jc-tsf-next/src/guids.rs` | rewritten：新增一組平行建置用新值，舊值留在 `crates/jc-tsf/src/guids.rs` |
| `register.rs`（`DllRegisterServer` 的註冊邏輯） | `crates/jc-tsf/src/register.rs` | rewritten：`register_with`／`unregister_with` 加 `Registration` 參數，舊預設值不變 |
| `tsf/src/ipc.rs`（`server_pipe`） | `crates/jc-tsf-next/src/ipc.rs` | rewritten：前綴改 `jc-ime-next`，覆寫變數改 `JC_IME_NEXT_PIPE`，訊框格式不變 |
| `cmd/0c-server/transport_windows.go`（pipe SDDL／instance） | `crates/jc-engine/src/main.rs` | rewritten：新增 `--instance` 與 `--personal-prefix`；SDDL 維持 0c 逐字相同 |
| `core/session`（PersonalStore 存檔） | `crates/jc-engine/src/lib.rs` | rewritten：存檔前綴可參數化，預設 `personal` |
| `docs/port/rollout.md`（本文件） | — | n/a：灰度與回退程序，無 Go 對應 |
