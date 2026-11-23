# 變更記錄

這裡只記使用者看得到的變更。版本號對應 `Cargo.toml` 的
`[workspace.package] version`；release workflow 會抓 `## <version>` 段落放進
GitHub Release。內部重構、測試補強與移植對照留在 `docs/port/`。

## 0.1.19 - 2026-11-24

### 修正

- 混打模式的純注音組字改走注音狀態機（`in_zh_lane()`）：片段、游標、Backspace／
  Delete／Esc、方向鍵與 Home/End、聲調閘門、片段鎖定、學習、符號與表情選單、
  前導標點都跟純注音模式一致。這一項讓 `jc-core` 的測試得以全部改跑混打模式
  （實機預設），73 條原本只有純注音模式才通過的測試現在都在混打下通過。
- 候選窗第一次顯示時記錄版面尺寸、視窗位置與 `IsWindowVisible`：回報「看不到
  候選窗」時可以一行分辨是尺寸為零、視窗沒顯示，還是根本沒畫。

## 0.1.18 - 2026-11-24

### 修正

- 安裝／反安裝前先結束常駐的 `jc-engine`。它從安裝目錄執行並握著
  `jc-engine.exe`；不先收掉，Inno 只能把舊檔改名成 `.old-<時間戳>` 排到重開機
  刪除，使用者會遇到「DLL 是新的、引擎是舊的」這種半套升級。

### 文件

- `docs/port/mode-unification.md`：把 core 的 `Zh`／`Ja`／`En` 收斂成只剩
  `Mixed`、英文只留在 TSF 的分階段計畫。這輪所有候選／游標／刪除 bug 都源自
  「同一個行為只實作在 `Mode::Zh`」。

## 0.1.17 - 2026-11-24

### 修正

- 候選清單改成**游標所在的那一個字**，不再列整串詞：打 `ㄊㄞˊㄨㄢˊㄖㄣˊ`
  按 ↓ 只會看到「人／仁／壬…」，不會出現「台灣人」。選字也只換那一個字，
  前面的中英日片段原樣保留（`台灣仁`、`婐hello ` 這類）。
  游標移到片段中間時，取游標左側那個片段的最後一個注音音節。

## 0.1.16 - 2026-11-24

### 修正

- **候選窗從來沒顯示過**：`parse()` 把 `navigating` 讀成 `candidates.navigating`，
  但它是 snapshot 的兄弟欄位，所以永遠是 false，`present_in_edit` 的 `show`
  條件永遠不成立。方向鍵、引擎、候選清單都是好的，只有這一行讀錯位置。
  加上 `engine_state_reaches_snapshot_navigating_and_candidates` 鎖住整條轉換。
- 補上候選窗的排查日誌：`present: navigating 但 show=false …` 與
  `ui: candidates=… 但 show=false`，之後同類問題一行就能定位。

## 0.1.15 - 2026-11-24

### 修正

- 混打（預設模式）的組字編輯全面補齊。之前只有純注音模式有這些行為，實機是
  混打，所以「台」刪不掉、打到一半卡住、游標過不去：
  - Backspace／Delete 對已轉成詞的片段是**刪整個片段**（不是退回注音或逐字母），
    刪完重播剩餘按鍵，畫面與按鍵流保持一致。
  - 羅馬字被推斷成日文後遇到數字鍵，會退回混打繼續輸入，不再卡在
    `わたしわw`。
  - ←／→ 一次移動一個片段，英文與日文片段也算一格。
  - 打完注音後 `candidates` 會建立，↓ 或空白都能開候選窗；數字選字只在選取
    模式生效，不會把下一個注音吃掉。

## 0.1.14 - 2026-11-24

### 修正

- 宿主終止組字時送出的 `cancel` 被原樣送到引擎，引擎不認識就回 `UnknownOp`；
  外殼把它當失敗、重試三次，每次重試都重播按鍵紀錄——實機症狀是切換視窗回來
  再打一個鍵，組字區變成 `w96w96w96w`。改成翻成 Escape 鍵，並在成功後清掉
  重播紀錄。
- 狀態列的「輸出簡體字」加上診斷日誌：點選時會寫一行
  `langbar: 輸出簡體 → true available=… simplified_live=… ok=… handled=…`，
  用來分辨「選單沒接到」、「能力沒開」與「引擎拒絕」。

## 0.1.13 - 2026-11-23

### 修正

- 「輸出簡體字」不再依賴資料檔：tw2s 字典（5 份，約 175KB，Apache-2.0）直接編進
  `jc-core`，安裝版不必再帶 `opencc/`。原本安裝包的資料根目錄根本沒有那個目錄，
  引擎因此從來不宣告 `output_simplified_live`，狀態列的開關等於空的。
  字典只在第一次使用時解析一次，之後所有 session 共用同一個 `Arc<Converter>`。

## 0.1.12 - 2026-11-23

### 修正

- 候選窗改成「按方向鍵／翻頁鍵才出現」：打字時只看組字（preedit），按 ↓↑
  進入選取才開清單，跟新注音的習慣一致。原本是每次組字都彈框，會蓋住上下文。
- 候選窗補上顯示／收起的一次性日誌（`ui: 顯示候選窗 …`／`ui: 收起候選窗`），
  實機排查時可以分得出「沒被呼叫」與「畫不出來」。

### 已知

- 「輸出簡體字」在安裝版仍無效：安裝包的資料根目錄沒有 `opencc/` 字典。
  程式端已完成，缺的是把字典納入資料包與 `jc.iss`（另外處理）。

## 0.1.11 - 2026-11-23

### 新增

- 候選窗：改由 TSF DLL 內自繪（Direct2D + DirectComposition），不再需要外部
  程式。`WS_EX_NOACTIVATE` 全程不搶焦點、跟隨插入點與監視器 DPI、深淺色主題、
  點選沿用原本的 edit session 核對流程。GPU 初始化失敗只收起候選窗並記一行
  日誌，不影響打字。
- 狀態列的「輸出簡體字」：引擎載入資料根目錄 `opencc/` 的 tw2s 字典，送出時
  轉成簡體，組字中的預覽仍是繁體。轉換器缺席時不宣告能力，也不會留下按了沒
  反應的假開關。

## 0.1.10 - 2026-11-23

### 修正

- Windows：打英文單字打到一半，整個輸入法變成英文模式，要按兩下 Shift 才回得來。
  引擎的 `Snapshot.mode` 是「這一段組字是什麼語言」，mixed 模式打英文字時它就是
  `english`；外殼原本把它當成中英模式直接採用。現在只有 Caps Lock 英文這種
  暫時狀態才以引擎為權威，一般按鍵一律用外殼記住的中英模式。

## 0.1.9 - 2026-11-23

### 修正

- Windows：左 Shift 不會切中英。引擎的 health 從來沒回報 `left_shift`／
  `right_shift`，jc-tsf 的 `ShiftConfig::from_info` 因此退回
  `left_shift=false`——只剩右 Shift 能用。現在依生效設定的 `toggle_key`
  回報兩個旗標（Windows 預設 `both`，左右都能切）。

## 0.1.8 - 2026-11-23

### 修正

- Windows：切到英文後切不回來，而且之後每個鍵都變成「server 不可用，放行按鍵」。
  0c 的 `display` 請求被原樣送給引擎，jc 沒有這個 op，回 `UnknownOp`；它又被記進
  重播紀錄，之後每次 `ensure_open` 重播都失敗，session 再也開不起來。改成翻成
  jc 的 `snapshot`（只是要目前的顯示狀態），重播也跟著正常。

## 0.1.7 - 2026-11-23

### 修正

- Windows：按鍵真的會進組字了。`to_jc_request` 只在 `private` 時才把字串鍵名
  轉成 jc 的結構化 `Key`，其餘一律送 `{"op":"key","key":"l"}`；引擎的 `Key`
  是 struct，反序列化失敗就把連線關掉，外殼之後每次寫入都失敗
  （`tsf.log` 的 `call: 寫入失敗，斷線`）。症狀是「打得很快、只出英文、沒有
  候選窗」——按鍵全被放行。

## 0.1.6 - 2026-11-23

### 修正

- Windows：輸入法真的接上引擎了。`Session` 從頭到尾都用 `NullTransport`，每個
  請求都失敗、`available=false`，所有按鍵原樣放行——這就是「切到 jc 注音打不出
  字、只打出英文」的真正原因。現在正式路徑接 `jc-tsf` 的 named pipe client。
- Windows：引擎沒在跑時由 TIP 自己啟動，不必等下次登入。啟動後不在按鍵回呼裡
  等它載入（那會卡住宿主程式），後續按鍵會自己連上。
- Windows：個人學習資料改存 `%LOCALAPPDATA%\jc-ime`。原本寫在
  `C:\ProgramData\jc-ime\data`，一般使用者沒有寫入權，引擎每次載入都回
  「存取被拒 (os error 5)」，連帶讓請求失敗、連線被關掉。
- Windows：安裝完直接啟動引擎，不用重新登入。Run 機碼改成直接呼叫
  `jc-engine.exe`，不再靠 PowerShell 算 RID；pipe 名兩邊都走
  `resolve_pipe_name()`，autostart 不再帶 `--socket`。
- doctor：引擎沒起來時會說明自動啟動機碼在不在；語言清單列舉容忍
  `ERROR_MORE_DATA`（同一個機碼裡有舊版 `InputMethodTips` 時整項會變 SKIP）。
- CI：install-smoke 安裝後等引擎啟動，並要求 `engine.pipe` 檢查通過，不再只看
  「設定寫進去了」。

### 效能

- 注音詞典改成 `Arc` 共用：每個 session 原本深拷貝兩份（即使沒有個人詞），
  而 session 是每個用戶端一個。`jc-cli convert` 的 RSS 從 227MB 降到 145MB。

## 0.1.5 - 2026-11-23

### 新增

- Windows 安裝程式改用 Inno Setup（`jc-ime-<version>-x64-setup.exe`）。偵測到舊版
  MSI 會先移除再安裝；不搬移舊設定與個人化資料，一律全新安裝。
- `jc-tsf-smoke` 變成 12 項安裝診斷（doctor），`--json` 輸出供 CI 與腳本判讀，
  每一項失敗都附修復指令。
- CI 每次 push main 都建置 setup.exe，並在 Windows runner 真的安裝、跑 doctor、
  反安裝；安裝器壞掉不必等發版才知道。
- `legal/RUST-THIRD-PARTY-NOTICES.md`：617 個 Rust 相依的授權清單與全文，隨安裝包
  散布，CI 每次比對是否與 `Cargo.lock` 一致。

### 修正

- Windows：輸入法會出現在語言清單，也能被選為作用中的輸入法。
  - `InstallLayoutOrTip` 的字串改成官方格式 `0x0404:{CLSID}{PROFILE};`。原本少了
    結尾分號、兩個 GUID 的順序也相反，msctf 回 TRUE 但什麼都沒寫。
  - doctor 的語言清單檢查同時接受 Windows 11 的「每個輸入法一個值」與舊版
    `InputMethodTips` 多字串。
  - `machine-register` 的 profile 路徑改用八位 langid（`0x00000404`）；原本查
    `0x0404`，每次都在最後一步回失敗。
- Windows：私密欄位（密碼、信用卡等）不再寫進學習資料；`Key.private` 與內容守門
  兩層都擋。
- Windows：按鍵回呼 panic 不再穿過 COM 邊界，改成「這一鍵不處理」並重設組字
  狀態，宿主程式不會跟著崩潰。
- Windows：安裝器的機碼檢視改用 `HKLM64`／`HKLM32`，`user-install` 以原使用者
  身分執行，靜默安裝不會再寫到 Administrator 的語言清單。
- 日文詞庫改成零拷貝：私有匿名記憶體 483MB → 9.5MB，載入 1.16 秒，749 筆 golden
  全部一致。

## 0.1.4 - 2026-11-23

### 修正

- Windows 安裝：修正 `user-cleanup-legacy` 連現行 profile 一起砍掉的問題。
  現在只清理 `IDENTITY_LEGACY` 舊 CLSID 相關機碼，不再移除新版的 HKLM profile。
  0.1.3 註冊成功後，後續清理把 profile 刪掉，系統看不到輸入法，打字只剩英文。
- Windows 安裝：`ensure_tip_key` 改查完整 profile 路徑，不再只看部分機碼，
  避免誤判 profile 已存在而略過註冊。
- Windows 安裝：`bEnabledByDefault` 改成 `false`，代表由安裝程序明確控制啟用；
  `user_install` 補上 `EnableLanguageProfile`，在使用者安裝時正確啟用語言設定。
- MSI ProductVersion 為 `0.1.4.0`，與 0.1.3 不同，Windows 會換檔。

## 0.1.3 - 2026-11-23

### 修正

- Windows 安裝：修正 MSI 打包到舊 DLL 的問題。CI 建置前清掉快取的 target，
  移除舊目錄 fallback，並對本次建置的 DLL 做 mtime 與 SHA-256 驗證。
  0.1.2 因為 rust-cache 還原到 0.1.1-rc.2 的 DLL，pipe 診斷 log 沒有進到 MSI。
- 這版包含前一版的診斷 log：按鍵路徑全程 `zlog!`，以及 pipe 名稱、rid 與
  `CreateFileW` 錯誤碼，用來找出 TIP `connect_existing` 失敗的原因。
- MSI ProductVersion 為 `0.1.3.0`，與 0.1.2 不同，Windows 會換檔。

## 0.1.2 - 2026-11-22

### 診斷

- 這版是 debug 版，用來找出 TIP 連不上引擎的問題：pipe 存在、`CreateFileW`
  可以連上，但 TIP 的 `connect_existing` 失敗。
- 按鍵路徑維持上一版的詳細 `zlog!`（`OnTestKeyDown`、`allowed`、`wants_key`、
  `OnKeyDown`、`learn_mode`、`handle_key`、`english_key_policy`、`field_at`）。
- 新增 pipe 連線診斷：記錄 TIP 使用的 pipe 名稱、rid 與 `CreateFileW` 錯誤碼。
- 兩邊都只加 log，沒有改任何判斷邏輯。
- 版本規則改為直接拉小版本、不再用 rc 後綴。MSI ProductVersion 為 `0.1.2.0`，
  與 0.1.1.x 不同，Windows 會換檔。

## 0.1.1-rc.2 - 2026-11-22

### 診斷

- 這是按鍵路徑的 debug RC。在 `OnTestKeyDown`、`allowed`、`wants_key`、
  `OnKeyDown`、`learn_mode`、`handle_key`、`english_key_policy` 與 `field_at`
  全程加上詳細 `zlog!` 診斷紀錄，用來定位實機「打字只有英文」時按鍵在哪一步被丟。
- 只加 log，沒有改任何判斷邏輯；MSI ProductVersion 第四段從 1 變成 2，確保 Windows 換檔。

## 0.1.1-rc.1 - 2026-11-22

### 修正

- Windows 打字：ITfContext 欄位政策改用 cache，probe 走 `TF_ES_ASYNCDONTCARE`
  非同步路徑，避免同步讀取被 host 拒絕時誤判為 Blocked 而永久英文。
- Windows 測試：更新 host 拒絕同步鎖的測試期望值為 Open，與 probe 的新行為一致。
- 版本號：從 `0.1.0` 升到 `0.1.1`，讓 MSI ProductVersion 從 `0.1.0.N` 變成
  `0.1.1.1`。Windows Installer 只比對前三段，若沿用 0.1.0 會被當成同一版而拒絕換檔，
  造成「裝了新版但引擎還是舊的」。

## 0.1.0-rc.18 - 2026-11-22

### 修正

- Windows 打字：修正 privacy probe 讀不到欄位（callback 回 `TF_E_SYNCHRONOUS`）
  時把結果當成 Blocked 的行為，改回 Open。rc.17 實機上這會讓 TIP 永久停留在英文。
  敏感欄位仍明確 Blocked，不受影響。probe 失敗時另記錄 callback、vk、hr、client
  與 flags，方便後續 debug；新增相對應的回歸測試。

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
