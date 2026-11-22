# tsf-smoke：免管理員的 TSF 冒煙測試

> 下一次 rc 已移除舊版，本文的 CLSID／profile 是移除前的歷史值，僅供對照。
> 現行身分與指令見 [`install.md`](../install.md) 與 [`rollout.md`](rollout.md)。


`crates/jc-tsf-smoke`，產出一支 `jc-tsf-smoke.exe`。它不註冊輸入法、不寫
`HKLM`、不需要管理員，只把 `jc-tsf.dll` 當普通 DLL 載入，逐項驗證 COM 外殼、
`ITfThreadMgr`、`Activate`、按鍵鏈、引擎 pipe 與註冊狀態，最後用結束碼表示
成敗。

**這支程式只能在 Windows 上執行。** Linux 上 `main` 會被換成一行訊息並以 2
結束，只讓 `cargo check --workspace --all-targets` 驗證型別；`report.rs`
（輸出格式）不含 Win32，Linux 也會一起編譯。

## 為什麼可以免管理員

真正的 TSF 啟用要靠 `HKLM\SOFTWARE\Microsoft\CTF\TIP` 與
`ITfInputProcessorProfiles::Register`，那需要管理員。但外殼本身的大多數錯誤在
免管理員下就抓得到：

- DLL 載得起來嗎、架構對嗎、依賴齊嗎
- `DllGetClassObject`／`IClassFactory`／四個介面的 vtable 對嗎
- `ITfTextInputProcessor` 與 `ITfKeyEventSink` 是不是同一個 COM 物件
- msctf 的 `ITfThreadMgr` 建得起來嗎、`Activate` 會不會崩
- 按鍵鏈接得上嗎（`ITfKeystrokeMgr::TestKeyDown`）
- 引擎 pipe 通不通、協定版本對不對

冒煙流程**刻意不呼叫** `DllRegisterServer`、`ITfInputProcessorProfiles::Register`
或 `ActivateProfile`：這些會改系統狀態、需要管理員，也會讓一次無權限的執行看
起來全部失敗。

## 建置與執行

```bat
cargo build --release --target x86_64-pc-windows-msvc -p jc-tsf -p jc-tsf-smoke
copy target\x86_64-pc-windows-msvc\release\jc-tsf.dll        target\x86_64-pc-windows-msvc\release\
jc-tsf-smoke.exe
```

- 沒給 `--dll` 時依序找：`JC_TSF_DLL` 環境變數 → 執行檔同目錄的
  `jc-tsf.dll` → `LoadLibraryW` 的標準搜尋路徑（目前目錄、`PATH`）。
- `--dll <path>` 或直接給一個位置參數可以指定 DLL。
- `JC_IME_PIPE` 可以覆寫引擎 pipe 名稱；預設讀
  `HKCU\Software\jc-ime\EnginePipe`，沒有就用 `jc-ime`（與 `jc-tsf` 相同）。
- 結束碼：`0` 全部通過、`1` 至少一項 `[FAIL]`、`2` 參數錯誤。

`jc_test_key` 只在 `debug_assertions` 或 `jc-tsf` 開了 `test-export` feature 時
才會匯出：

```bat
cargo build --release --features test-export -p jc-tsf
```

release DLL 沒有這個符號是正常出貨狀態，冒煙程式只會印一行 `[INFO]`。

## 移植對照

Go 參考實作（`/mnt/work/jc/ref/0c-ime`）沒有 TSF 外殼、沒有 Windows 平台層，
所以這個 task 幾乎全部是 n/a；實際沿用到的只有既有的按鍵定義與協定。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/proto/*`（訊框與訊息） | `crates/jc-proto/src/lib.rs`（既有） | ported；冒煙直接送 `Hello`／`Key`，不重寫訊框 |
| `internal/zh/keyboard.go`、`internal/zh/input.go` | `crates/jc-core/src/zh/keyboard.rs`、`key.rs`（既有） | ported；`jc_test_key` 經 `jc-tsf::keys::translate` 沿用，沒有另抄鍵位表 |
| n/a（Go 沒有 TSF 實作） | `crates/jc-tsf-smoke/src/main.rs`、`checks.rs` | rewritten；七項檢查的流程 |
| n/a | `crates/jc-tsf-smoke/src/dll.rs`（`LoadLibraryW`、`DllGetClassObject`、`DllCanUnloadNow`、`jc_test_key`） | rewritten |
| n/a | `crates/jc-tsf-smoke/src/product.rs`（CLSID、profile GUID、LANGID） | rewritten；值與 `jc-tsf/src/guids.rs` 相同（cdylib 無法 import，只能留一份） |
| n/a | `crates/jc-tsf-smoke/src/engine.rs`（引擎 pipe 探測） | rewritten；連線與握手交給 `jc-windows` |
| n/a | `crates/jc-tsf-smoke/src/registry.rs`（`CTF\TIP` 與 `InprocServer32`，64 位元與 32 位元兩個檢視） | rewritten |
| n/a | `crates/jc-tsf-smoke/src/report.rs`（固定輸出格式） | rewritten；Linux 也會編譯 |
| n/a（Go 沒有 Windows 外殼） | `crates/jc-tsf/src/lib.rs` 的 `jc_test_key` | rewritten；`debug_assertions` 或 feature `test-export` 才編譯 |

參考實作是 <https://github.com/LeafHW/hufu-ime-rust> 的
`platform/windows/hufu-tsf-smoke`（不是 Go）。沿用它的順序：DLL → 類廠 →
`CreateInstance` → msctf `ITfThreadMgr` → `Activate` → 按鍵鏈 → 引擎直驅；
但它會 `DllRegisterServer`／`Register`，本程式刻意不做。參考實作用
`windows-rs` 0.58；這裡跟 workspace 一致用 0.62，API 形狀相同。

## 七項檢查

| # | 檢查 | 判定 | 只回報 |
|---|---|---|---|
| 1 | `LoadLibraryW(<path>)`，再 `GetProcAddress("DllCanUnloadNow")` | 載入成功**且**找得到 `DllCanUnloadNow` | `DllCanUnloadNow` 的回傳值 |
| 2 | `DllGetClassObject(CLSID_JC_TSF, IID_IClassFactory)` | `S_OK` 且拿到非空指標 | — |
| 3 | `CreateInstance` 後 QI 四個介面，再比對 `ITfKeyEventSink` 與 `ITfTextInputProcessor` 的 `IUnknown` 指標 | 四個 QI 全 `S_OK`，且兩個介面是同一個 COM 物件 | 每個 QI 的 HRESULT |
| 4 | `CoCreateInstance(CLSID_TF_ThreadMgr, CLSCTX_INPROC_SERVER)` | `S_OK` | — |
| 5 | `ITfThreadMgr::Activate()` 取得**應用程式** client id；`ITfClientId::GetClientId(&CLSID_JC_TSF)` 取得**TIP** client id，再 `ITfTextInputProcessor::Activate(tm, tip_id)` | `S_OK`；已安裝時 msctf 已激活同一個 TIP 而回 `CONNECT_E_ADVISELIMIT` 也算通過 | 兩個 client id、`ITfKeystrokeMgr::TestKeyDown('u')` 的 consumed |
| 6 | `jc-windows` 的 `connect_with_retry` → `Hello` → `Key(Escape)` | 連得上且兩個往返都成功 | 連不上只印 `[SKIP]`，**不算失敗** |
| 7 | 兩個機碼檢視（`/reg:64`、`/reg:32`）各讀 `CTF\TIP\{CLSID}` 與 `HKCR\CLSID\{CLSID}\InprocServer32` | 兩邊都有同一個 CLSID，而且分別指到 `jc-tsf.dll` 與 `jc-tsf-x86.dll` | 只看得到一邊或指到錯的 DLL → `[FAIL]`；兩邊都沒有 → `[SKIP]`（尚未安裝） |

檢查 3 的物件身分是**硬檢查**：msctf 要求 `ITfKeyEventSink` 與
`ITfTextInputProcessor` 是同一個 COM 物件，獨立 sink 物件會讓按鍵送不進來。
實作方式是把兩邊都 QI 到 `IUnknown` 再比指標；`#[implement]` 產生的 QI 對
`IID_IUnknown` 一律回同一個 identity 位址，所以指標相等就代表同一個物件。

檢查 7 沒有真的去跑 `reg query`：它用 `RegOpenKeyExW`／`RegEnumKeyExW` 直接
讀同一個機碼，省掉子行程與主控台編碼問題，判定條件相同（只比對子機碼名稱）。

檢查 5 成功後會立刻 `Deactivate` 並等 200 ms：`jc-tsf` 的 worker 在 `Activate`
時會連上引擎，而引擎一次只服務一條連線，不先放掉的話檢查 6 會連不進去。

檢查 5 有兩個 client id，來源不能混用。`ITfThreadMgr::Activate()` 回的是
**應用程式**的 id；`ITfTextInputProcessor::Activate` 要的是**文字服務**的 id，
由 `ITfClientId::GetClientId(&CLSID_JC_TSF)` 取得。之前直接把應用程式 id 傳進去，
msctf 在 `AdviseKeyEventSink` 回 `E_INVALIDARG`。已安裝 `jc-tsf` 時，msctf 會在
`ITfThreadMgr::Activate()` 內部先激活 TIP 並裝好 key sink；手動激活同一個 id 會回
`CONNECT_E_ADVISELIMIT`，這代表 TIP 已經在跑，判為通過。詳細文件依據與真機指令見
`docs/port/activate-second-call.md`。

## 預期輸出

### 引擎沒開、輸入法沒安裝（正常開發機）

```text
jc-tsf 免管理員冒煙測試（jc 注音）
不需要管理員；不會呼叫 ITfInputProcessorProfiles::Register。

[INFO] CoInitializeEx(APARTMENTTHREADED) = 0x00000000
[INFO] DLL：C:\build\jc-tsf.dll
[INFO] CLSID {7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}；profile {8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}；LANGID 0x00000404
[PASS] 1. LoadLibraryW + DllCanUnloadNow：C:\build\jc-tsf.dll；DllCanUnloadNow = 0x00000000
[PASS] 2. DllGetClassObject(CLSID_JC_TSF, IClassFactory)：S_OK（0x00000000）
[PASS] 3. CreateInstance + QI（vtable 與物件身分）：ITfTextInputProcessor=S_OK、ITfKeyEventSink=S_OK、ITfThreadMgrEventSink=S_OK、ITfCompositionSink=S_OK；ITfKeyEventSink 與 ITfTextInputProcessor 是同一個物件
[PASS] 4. CoCreateInstance(CLSID_TF_ThreadMgr)：S_OK（0x00000000）
[INFO] ITfThreadMgr::Activate → 應用程式 client_id=1（不是 TIP 的 client_id）
[INFO] ITfClientId::GetClientId(CLSID_JC_TSF) → TIP client_id=2
[PASS] 5. Activate：ITfThreadMgr::Activate → 應用程式 client_id=1；ITfClientId::GetClientId → TIP client_id=2；ITfTextInputProcessor::Activate = S_OK（0x00000000）
[INFO] ITfKeystrokeMgr::TestKeyDown('u') consumed=false（引擎未連線時應為 false）
[SKIP] 6. 引擎 pipe：pipe jc-ime 連不上：讀寫失敗：…；引擎未啟動或未安裝，只警告不判定失敗
[INFO] jc_test_key(VK_J = 0x4A) = 2（連不上引擎 pipe）；僅供參考，不影響結果
[SKIP] 7. 註冊探測：64 位元與 32 位元檢視都沒有 {7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}（尚未安裝，不判定失敗）
[INFO] 釋放物件後 DllCanUnloadNow = 0x00000000（S_OK 代表沒有 refcount 洩漏）

結果：5 項通過、0 項失敗、2 項略過
```

### 引擎已啟動

（只列與上一份不同的行。）

```text
[PASS] 6. 引擎 pipe：Hello → 協定 3、引擎 jc-engine 0.1.0；Key(Escape) → State（組字 ""、候選 0 個）
[INFO] jc_test_key(VK_J = 0x4A) = 0（按鍵已送達引擎並取得回應）；僅供參考，不影響結果
[PASS] 7. 註冊探測：兩個檢視都有 {7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}：64 位元 → C:\Program Files\jc-ime\jc-tsf.dll（12 個 TIP）、32 位元 → C:\Program Files\jc-ime\jc-tsf-x86.dll（12 個 TIP）
```

### 只有一半註冊

x64 裝好、x86 沒裝（例如只跑過 `regsvr32 jc-tsf.dll`）時，這一項會失敗並指出缺
哪一邊：

```text
[FAIL] 7. 註冊探測：32 位元檢視的 CTF\TIP\{7B7D6E1A-...} 不存在；32 位元檢視的 HKCR\CLSID\{7B7D6E1A-...}\InprocServer32 不存在。修復：CLSID 由 MSI 寫，先修復安裝（msiexec /fa <msi>）；再以管理員執行 jc-tip-register machine-register 補 profile 與分類
```

（實際的 `引擎` 字串由引擎的 `engine_label()` 決定；`State` 的內容依當下組字
狀態而變。）

### 失敗長什麼樣子

```text
[FAIL] 3. CreateInstance + QI（vtable 與物件身分）：ITfTextInputProcessor=S_OK、ITfKeyEventSink=S_OK、ITfThreadMgrEventSink=S_OK、ITfCompositionSink=S_OK；ITfKeyEventSink 不是 ITfTextInputProcessor 同一個物件（msctf 不接受獨立 sink）
```

## 每個失敗項目的可能原因

| # | 看到的 HRESULT／現象 | 可能原因 |
|---|---|---|
| 1 | `0x8007007E` | 找不到 DLL 或它依賴的 DLL；`jc-windows`／VC runtime 沒在旁邊 |
| 1 | `0x800700C1` | 架構不符（32 位元 DLL 給 64 位元 exe，反之亦然） |
| 1 | `0x80070005` | DLL 檔案被鎖住或沒有讀取權限 |
| 1 | 找不到 `DllCanUnloadNow` | 載到別的 DLL、匯出被 strip，或建置腳本漏了 `#[no_mangle]` |
| 2 | `0x80040111`（`CLASS_E_CLASSNOTAVAILABLE`） | `DllGetClassObject` 認的 CLSID 與冒煙程式不一致（`guids.rs` 被改過） |
| 2 | `0x80004002`（`E_NOINTERFACE`） | `DllGetClassObject` 沒有正確把 `riid` 轉出去 |
| 2 | `0x8007007E` | 沒有匯出 `DllGetClassObject` |
| 3 | `0x80004002` | 某個介面沒列在 `#[implement(...)]`（例如 `ITfThreadMgrEventSink` 被拿掉） |
| 3 | QI 全過但「不是同一個物件」 | `ITfKeyEventSink` 被做成獨立物件；msctf 會拒絕，按鍵鏈不會動 |
| 4 | `0x80040154`（`REGDB_E_CLASSNOTREG`） | `msctf.dll` 不在或系統精簡；不是本 DLL 的問題 |
| 4 | `0x800401F0`（`CO_E_NOTINITIALIZED`） | 前面 `CoInitializeEx` 失敗（`[INFO]` 那行不是 `0x00000000`） |
| 5 | `ITfThreadMgr::Activate` 失敗 | TSF 已被同執行緒的其他程式佔用、COM apartment 不對 |
| 5 | `ITfTextInputProcessor::Activate` 回 `0x80004005` | `Control::start` 建訊息視窗或 `AdviseKeyEventSink` 失敗（看 `OutputDebugStringW` 的 `[jc-tsf]` 記錄） |
| 5 | `ITfTextInputProcessor::Activate` 回 `0x80070057`（`E_INVALIDARG`） | `AdviseKeyEventSink` 拿到的 tid 不是 TIP client id（例如誤用 `ITfThreadMgr::Activate` 的回傳值）。檢查 5 現在會分別印出兩個 id；日誌的 `advise` 行會附 thread id，見 `docs/port/activate-second-call.md` |
| 5 | `ITfTextInputProcessor::Activate` 回 `0x80040201`（`CONNECT_E_ADVISELIMIT`） | 已安裝時 msctf 先激活了同一個 TIP；檢查 5 判為通過，不是失敗 |
| 5 | `0x8001010E`（`RPC_E_WRONG_THREAD`） | TIP 物件被跨 apartment 呼叫；`Activate` 必須在建立它的執行緒 |
| 6 | 連不上 | 引擎沒開、pipe 名稱不同（`HKCU\Software\jc-ime\EnginePipe`）、連線被真正的輸入法佔住 |
| 6 | `VersionMismatch` | 引擎與 `jc-proto` 的 `PROTOCOL_VERSION` 不同 |
| 6 | `Timeout` | 引擎卡住或超過 500 ms；這條連線之後不可信，要重開 |
| 7 | `RegOpenKeyExW … 0x00000002` | 系統沒有任何 TSF TIP（極少見），不影響其他檢查 |
| 7 | 沒找到 CLSID | 還沒 `regsvr32` 安裝；這是預期情況，`[SKIP]` 不代表壞掉 |

## `jc_test_key`

`crates/jc-tsf/src/lib.rs` 的測試匯出，走「虛擬鍵 → `keys::translate` → 引擎
`Key`」的內部路徑；沒有 TSF context，所以只回引擎的回應碼，不套用組字。

| 回傳值 | 意義 |
|---|---|
| `0` | 按鍵已送出，引擎回應正常 |
| `1` | 這個鍵不歸輸入法處理（或 `vk` 不是合法虛擬鍵碼） |
| `2` | 連不上引擎 pipe |
| `3` | 引擎往返失敗（逾時、協定錯誤、連線已關閉） |

冒煙程式用它量測「DLL → 引擎」這一段；結果只印 `[INFO]`，不影響結束碼。原因：
它自己開一條 pipe 連線，會與檢查 6 的時間序互相影響，不適合當成硬性判定。

## 驗證

| 指令 | 結果 |
|---|---|
| `cargo check --workspace --all-targets`（Linux） | 通過；`main` 是佔位實作，`report.rs` 與其單元測試一起編譯 |
| `cargo check --target x86_64-pc-windows-gnu -p jc-tsf -p jc-tsf-smoke --all-targets` | 通過；`windows-rs` 的型別全部對得上 |
| `cargo check --target x86_64-pc-windows-gnu -p jc-tsf --features test-export --all-targets` | 通過；release 路徑的 `jc_test_key` 也編得過 |
| `cargo clippy --target x86_64-pc-windows-gnu -p jc-tsf -p jc-tsf-smoke --all-targets` | `jc-tsf`、`jc-tsf-smoke` 零警告 |

**本機沒有安裝 `x86_64-pc-windows-msvc` target**（`rustup target list
--installed` 只有 `x86_64-pc-windows-gnu`），所以任務書的 msvc 指令改用 gnu
target 做型別檢查；`cargo check` 不連結，兩者的 API 表面相同。實際的
`jc-tsf-smoke.exe` 必須在 Windows 上建置與執行。

## 未驗證清單（需要實機）

以下全部沒有在 Windows 上跑過，Linux 只能驗證編譯：

1. 冒煙程式本身的輸出與結束碼；上面「預期輸出」是照程式碼推導的。
2. 沒有 document manager 時 `ITfThreadMgr::Activate` + `AdviseKeyEventSink`
   是否真的回 `S_OK`（若有 application 正在使用 TSF，`Activate` 可能被拒）。
3. `ITfKeystrokeMgr::TestKeyDown` 是否真的把事件送到本 TIP 的
   `OnTestKeyDown`；`consumed` 依引擎連線狀態而變。
4. 檢查 5 的 `Deactivate` 到檢查 6 之間，200 ms 是否足以讓 `jc-tsf` 的 worker
   關掉 pipe（引擎一次只服務一條連線）。
5. `DllCanUnloadNow` 在釋放所有物件後是否回到 `S_OK`（refcount 洩漏）。
6. 中文輸出在 `cmd.exe` 的顯示（Rust 對 console handle 使用 `WriteConsoleW`，
   理論上正確，未實測）。
7. 32 位元 TIP 的註冊位置（`WOW6432Node`）：檢查 7 只驗到讀得到 `InprocServer32`
   的預設值，沒有實際讓 32 位元程式載入那份 DLL。
8. `jc_test_key` 在真實鍵盤配置（非美式、Dvorak、死鍵）下的鍵位對應。
9. 引擎正在被真正的輸入法使用時，檢查 6 的行為（可能 `[SKIP]`）。

## 已知限制

- 只驗證「外殼能動」，不驗證輸入法出現在語言列、也不驗證組字外觀；那些要
  安裝後由 `docs/port/tsf-skeleton.md` 的未驗證清單接手。
- 檢查 7 只讀 `HKLM`，不讀 `HKCU`；`regsvr32` 以管理員安裝時寫的是 `HKLM`。
- 引擎檢查會送一個 `Escape`。這是刻意的：即使引擎正在別人的組字中，這個鍵
  只會清掉狀態，不會插入字元。
- 不支援平行執行兩份冒煙程式：引擎 pipe 只有一個 instance。
