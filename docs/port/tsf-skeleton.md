# tsf-skeleton：TSF 文字服務骨架

`crates/jc-tsf`，`crate-type = ["cdylib"]`。第一版只做 COM 註冊、按鍵、組字、
提交與非同步 IPC；候選窗、設定 UI、安裝器都不是這個 crate 的事。

非 Windows 平台整個 crate 被 crate 層的 `#![cfg(windows)]` 包住，編出來是一個
沒有任何符號的 cdylib。Linux CI 的 `cargo check --workspace --all-targets` 因此
會過，但**沒有任何替代實作**。

## 移植對照

Go 參考實作（`/mnt/work/jc/ref/0c-ime`）沒有 TSF 外殼，也沒有 Windows 平台層，
所以這個 task 大部分是 n/a；實際沿用到的只有按鍵與鍵位的核心定義。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/zh/keyboard.go`（標準／大千排列） | `crates/jc-core/src/zh/keyboard.rs`（既有） | ported 並且直接沿用；`jc-tsf` 用 `Keyboard::standard()` 取得注音鍵位，沒有另抄一份表 |
| `internal/zh/input.go`（按鍵送進注音輸入） | `crates/jc-core/src/zh/input.rs`（既有，由引擎端執行） | ported；`jc-tsf` 只把按鍵轉成 `jc_proto::Key` |
| `internal/proto/*`（協定訊息） | `crates/jc-proto/src/lib.rs`（既有） | ported；`jc-tsf` 不重新定義任何訊息格式 |
| n/a（Go 沒有 TSF 實作） | `crates/jc-tsf/src/lib.rs`（COM 物件、class factory、四個匯出） | rewritten；本 task 新寫 |
| n/a（Go 沒有 TSF 實作） | `crates/jc-tsf/src/register.rs`（`DllRegisterServer`／`DllUnregisterServer`） | rewritten |
| n/a（Go 沒有 Windows 外殼） | `crates/jc-tsf/src/keys.rs`（虛擬鍵碼 → `Key`） | rewritten；行為規則照 `docs/wrapper.md` §4 |
| n/a（Go 的 wrapper 是同步的） | `crates/jc-tsf/src/engine.rs`（worker 執行緒、500 ms 逾時、退避重連） | rewritten |
| n/a | `crates/jc-tsf/src/composition.rs`（組字、提交、訊息視窗、edit session） | rewritten |
| n/a | `crates/jc-tsf/src/logging.rs`、`src/guids.rs` | rewritten |

## CLSID 與 GUID

| 名稱 | 值 | 用途 |
|---|---|---|
| `CLSID_JC_TSF` | `{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}` | 文字服務物件的 CLSID；x64 與 x86 共用 |
| `GUID_PROFILE_JC_TSF` | `{8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}` | TSF profile GUID（只存在 profile 資料庫） |
| LANGID | `0x0404`（zh-TW） | 第一版只有繁體中文注音 |
| 顯示名稱 | `jc 注音` | 語言列與設定頁看到的名稱 |

兩份常數都定義在 `crates/jc-tsf/src/guids.rs`。發佈後不可改。

這個 crate 建兩份（`x86_64-pc-windows-msvc` 與 `i686-pc-windows-msvc`），兩份拿到
**完全相同**的 CLSID 與 profile：同一個 CLSID 在 64 位元與 `Wow6432Node` 兩個機碼
檢視各有一個 `InprocServer32`，TSF 依宿主行程位元數挑對的 DLL。使用者的語言清單
因此只有一項「jc 注音」。差異只有 `register.rs` 的 `REGISTRY_VIEW`
（`KEY_WOW64_64KEY`／`KEY_WOW64_32KEY`）。詳見 `docs/port/x86-tip.md`。

## 註冊機碼

> MSI 安裝**不走這條路徑**：CLSID 由 `installer/jc-ime.wxs` 的 `<Class>` 寫，
> profile 由 `jc-tip-register machine-register` 註冊。以下是 DLL 被手動
> `regsvr32` 時的行為，程式碼保留在 `crates/jc-tsf/src/register.rs`。

`DllRegisterServer` 依序做：

1. 寫 `HKEY_CLASSES_ROOT\CLSID\{7B7D6E1A-...}\InprocServer32`
   - 預設值：這個 DLL 的完整路徑（`GetModuleFileNameW`）
   - `ThreadingModel`：`Apartment`
2. `CoCreateInstance(CLSID_TF_InputProcessorProfiles)` 之後：
   - `ITfInputProcessorProfiles::Register(CLSID)`
   - `AddLanguageProfile(CLSID, 0x0404, GUID_PROFILE, "jc 注音", 無圖示)`
3. `CoCreateInstance(CLSID_TF_CategoryMgr)` 之後
   `RegisterCategory(CLSID, GUID_TFCAT_TIP_KEYBOARD)`，Windows 8 以後的輸入法
   設定頁才會列出這個 TIP。

**刻意不呼叫** `ITfInputProcessorProfileMgr::ActivateProfile`：不會偷改使用者的
系統預設輸入法。MSI 的 `machine-register` 用 `RegisterProfile` 並把
`bEnabledByDefault` 傳 `true`，profile 註冊時就是 `Enable=1`，使用者把「jc 注音」
加入鍵盤後就能切換；手動 `regsvr32` 走的 `AddLanguageProfile` 沒有這個參數，
行為可能不同。

`DllUnregisterServer` 反向做：`RemoveLanguageProfile`、`Unregister`、
`UnregisterCategory`，最後 `RegDeleteTreeW` 刪掉整個 `CLSID\{...}`。機碼檢視一律由
`REGISTRY_VIEW`（依 `target_pointer_width` 選 `KEY_WOW64_64KEY` 或
`KEY_WOW64_32KEY`）明確指定，不靠行程位元數的預設檢視；x86 DLL 因此寫到
`Wow6432Node`。

寫 `HKCR` 與 TSF profile 資料庫需要系統管理員權限，`regsvr32` 要以管理員身分
執行。使用者若沒有安裝 zh-TW 語言，`AddLanguageProfile` 可能失敗；這是 TSF 的
限制，不是這個 DLL 能繞過的。

## 匯出的符號

| 符號 | 行為 |
|---|---|
| `DllGetClassObject` | 只認 `CLSID_JC_TSF`，其他回 `CLASS_E_CLASSNOTAVAILABLE`；不支援 aggregation。x64 與 x86 兩份接受同一個 CLSID，由行程位元數決定載入哪一份 |
| `DllCanUnloadNow` | 活著的文字服務物件為 0 時回 `S_OK`，否則 `S_FALSE` |
| `DllRegisterServer` | 見上一節 |
| `DllUnregisterServer` | 見上一節 |

## 實作的 COM 介面

| 介面 | 實作物件 | 用途 |
|---|---|---|
| `IClassFactory` | `ClassFactory` | `DllGetClassObject` |
| `ITfTextInputProcessor` | `TextService` | `Activate` 建狀態、`Deactivate` 清乾淨 |
| `ITfThreadMgrEventSink` | `TextService` | `OnSetFocus` 換焦點時收掉組字 |
| `ITfKeyEventSink` | `TextService` | `OnTestKeyDown`／`OnKeyDown` |
| `ITfCompositionSink` | `CompositionSink`（獨立物件） | 組字被外部終止時清狀態 |
| `ITfEditSession` | `ApplySession`（每次套用一個） | 在 `TF_ES_SYNC` 內改組字與插入文字 |

`CompositionSink` 刻意做成獨立物件而不是讓 `TextService` 自己兼任，避免「物件
持有自己的介面引用」讓 refcount 永遠不歸零。`key_sink` 與 `thread_sink` 兩個
自引用則由 `Deactivate` 明確丟掉（`Control::shutdown`）。

`ITfDisplayAttributeProvider`（虛線底線）**沒有實作**：第一版用 TSF 的預設組字
外觀，`tsf-plan` 把它列在後續工作。

## 按鍵

`OnTestKeyDown` 只在自己要處理時回 `TRUE`。

| 情況 | 攔截 | 送給引擎 |
|---|---|---|
| Ctrl／Alt／Win 按著 | 否 | 一律放行 |
| 引擎未連線 | 否 | 全部放行（至少能打出原來的字母） |
| 沒組字，可列印鍵且對到注音或全形標點 | 是 | `Key::char` |
| 沒組字，其他鍵（Enter、Space、Tab、方向鍵、`=`、`` ` ``…） | 否 | 放行 |
| 組字中，注音鍵／標點 | 是 | `Key::char` |
| 組字中，Enter／Esc／Backspace／Delete／Space／Tab／Shift+Tab／方向鍵／PageUp／PageDown | 是 | 對應的 `KeyCode` |

注音鍵位表來自 `jc_core::zh::Keyboard::standard()`：

- 字母與數字鍵：`1`→ㄅ、`q`→ㄆ …（大千排列，含四個聲調鍵與空白鍵的輕聲）
- Shift 過的符號：`<`→，、`>`→。、`?`→？、`^`→……、`_`→—— …
- 多字元標點（`……`、`——`）會拆成多個 `Key` 依序送出，同一個 generation

實體按鍵的字元用 `ToUnicodeEx`（目前執行緒的鍵盤配置與 Shift 狀態）解出來，
不自己假設鍵盤配置。死鍵（`ToUnicodeEx` 回負值）一律放行。

`Home`／`End` 不在第一版的攔截範圍：核心目前只支援在結尾編輯，沒有東西可移。

## 與引擎的連線

| 項目 | 值 |
|---|---|
| pipe 名稱 | 讀 `HKCU\Software\jc-ime` 的 `EnginePipe`；沒有就用 `jc-ime` |
| 客戶端名稱 | `jc-tsf/0.1.0` |
| 單次往返逾時 | 500 ms（引擎神經模型硬上限 180 ms） |
| 連線時機 | `Activate` 後 worker 立刻試一次；失敗後最多每 2 秒一次 |
| 失敗處理 | 標記斷線、丟棄該請求、清掉組字；下一個按鍵放行給應用程式 |

連線失敗**不重送**同一個按鍵：重送會多打一個字。重連是新 session，組字從空白
開始（`jc-windows` 的契約）。

`Deactivate` 只關掉這條連線，不送 `Shutdown`：引擎是跨應用程式共用的，關掉會
把別人的輸入法一起打死。

## 非同步流程

```text
UI 執行緒（TSF 呼叫）                       worker 執行緒
────────────────────                       ──────────────
OnTestKeyDown
  └─ 不在處理範圍 → FALSE（放行）
  └─ 要處理      → TRUE

OnKeyDown
  ├─ generation += 1
  ├─ mpsc send(Job { generation, Request::Key })
  └─ return TRUE（立即返回，不碰 IPC）
                                             recv()
                                             ├─ 沒連線 → 退避重連
                                             ├─ client.request(500 ms)
                                             └─ Queue::push(Incoming)
                                                ＋ PostMessageW(WM_APP+1)

訊息迴圈
  └─ WM_APP+1 → Control::drain()
       ├─ State 且 generation 過期 → 丟棄
       ├─ State 且 preedit 非空   → RequestEditSession(TF_ES_SYNC)
       │                             ├─ 沒組字 → StartComposition
       │                             ├─ 有組字 → SetText + Collapse 到游標
       │                             └─ 設 selection
       ├─ State 且 preedit 空     → EndComposition（如果有）
       ├─ Committed { text }      → EndComposition 後 InsertTextAtSelection
       └─ 錯誤                    → 記錄 log、EndComposition、不插入任何字
```

`RequestEditSession` 的回呼裡會用 `Interface::as_raw` 重新比對 context；不是
同一個就直接放棄這次套用。`RequestEditSession` 本身失敗（context 已失效）時
把存下來的 context 清掉，下一個按鍵會重新取得。

### 世代（generation）

每次按鍵 `generation += 1`，請求帶著當時的值。回應回來時只有 `State` 會因為
過期被丟棄；`Committed` 一律套用——丟掉提交文字就是掉字，不能因為過期而丟。

### 選字後的補提交

協定 v3 的既有缺口（`docs/wrapper.md` §7.2）：Space／數字選字後引擎回的是空
`State`，選到的文字還在引擎的待提交區。`Control::apply_state` 在上一次有組字、
這次 `preedit` 與 `candidates` 都空的時候補送一次 `Request::Commit`，把文字取
回來。沒有候選窗的第一版同樣需要這條路徑，否則空白鍵選字會掉字。

## 組字與提交的細節

| 動作 | 實作 |
|---|---|
| 開始組字 | `ITfContext::QueryInterface(ITfContextComposition)` → `StartComposition`，附著點由 `ITfInsertAtSelection::InsertTextAtSelection(TF_IAS_QUERYONLY)` 取得 |
| 更新組字 | `ITfComposition::GetRange()` → `SetText`（第一次 flags 0，之後 `TF_ST_CORRECTION`） |
| 游標 | `ITfRange::Clone` 出獨立範圍 → `Collapse(START)` → `ShiftEnd(cursor_rune 的 UTF-16 位移)` → `Collapse(END)` → `ITfContext::SetSelection` |
| 提交 | `EndComposition` 之後 `InsertTextAtSelection`，游標移到插入文字結尾 |
| 換焦點 | 丟掉 context 與組字參照，送一次 `Escape` 清引擎狀態；組字本身由 TSF 結束 |
| 外部終止 | `ITfCompositionSink::OnCompositionTerminated` 只清本地參照（先 `take()` 再結束，避免重入撞 `RefCell`） |

`ITfRange` 的 `Clone` 是 TSF 的方法（複製 anchors），不是 Rust 的 `Clone`
（只 AddRef）。直接拿組字範圍去 `Collapse` 會把組字縮成空字串，所以游標一定
用複製出來的範圍。

## 記錄

- 一律送 `OutputDebugStringW`（DebugView／VS 輸出視窗）。
- `JC_TSF_LOG=1` 時另外附加到 `%LOCALAPPDATA%\jc-ime\tsf.log`；開檔或寫檔失敗
  直接吞掉，不影響輸入。
- 每行都有 `[jc-tsf <pid> <level>]` 前綴。

## 驗證

| 指令 | 結果 |
|---|---|
| `cargo check --workspace --all-targets`（Linux） | 通過（`jc-tsf` 是空 crate） |
| `cargo check --target x86_64-pc-windows-gnu -p jc-tsf --all-targets` | 通過（本機沒有安裝 msvc target，用 gnu target 驗證型別；不連結） |
| `cargo check --target x86_64-pc-windows-msvc -p jc-tsf --all-targets` | 通過（x64 出貨 target） |
| `cargo check --target i686-pc-windows-msvc -p jc-tsf --all-targets` | 通過（x86 出貨 target；Linux 只驗到型別） |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通過；`jc-model` 有一個既有的 `unused_mut`，不在本 task 的 owned paths |
| `cargo clippy --target x86_64-pc-windows-gnu -p jc-tsf --all-targets` | `jc-tsf` 零警告 |

單元測試只涵蓋純邏輯（鍵位對照、UTF-16 位移、世代判定、GUID 格式），依
`common.md` 只編譯不執行。

## 未驗證清單（需要實機）

以下全部沒有在 Windows 上跑過。Linux 只能驗證型別與連結前的程式碼。

1. `regsvr32` 註冊與反註冊是否成功、機碼內容是否正確。
2. 輸入法是否出現在「設定 → 時間與語言 → 語言 → 中文（台灣）」的輸入法清單。
3. `Activate`／`Deactivate` 是否被 TSF 正確呼叫、有沒有漏放 sink 或漏關視窗。
4. `StartComposition`／`SetText`／`SetSelection` 在多行編輯器、Word、瀏覽器
   （Chromium 的 TSF 路徑）裡的行為與外觀。
5. `cursor_rune` 大於 0 的游標位置是否正確（核心目前游標都在結尾，這條路徑
   實際上很少走到）。
6. 提交後游標位置、選取範圍被取代的行為。
7. 換焦點時組字是否被收乾淨、引擎的 `Escape` 是否真的清掉狀態。
8. 空白鍵選字後的補 `Commit` 是否拿得到文字。
9. 500 ms 逾時與重連在真實 named pipe 上的行為（引擎重啟、pipe 斷線）。
10. 按鍵攔截範圍是否造成某些應用程式收不到 Enter／Tab／方向鍵。
11. 死鍵鍵盤配置（例如美式國際）下 `ToUnicodeEx` 的行為。
12. 日誌檔在 `%LOCALAPPDATA%` 是否正確建立。
13. 多個應用程式同時使用時的排隊行為（引擎一次只服務一條連線）。
14. ARM64 或其他架構的建置。目前有 x64 與 i686 兩個 target（i686 只在 Linux 上驗到
    `cargo check`，沒有在 Windows 連結與載入過），ARM64 沒有建置。

## 已知限制

- 沒有候選窗（`tsf-candidates`）與 `ITfDisplayAttributeProvider`。
- 沒有 IME 開關、中英切換、模式切換；profile 被選用就等於注音模式。
- `Home`／`End` 不攔截；組字游標只能在結尾。
- 設定只有 `HKCU\Software\jc-ime\EnginePipe` 一個值，沒有 `settings.json` 接線。
- 引擎沒起來時整個輸入法等於直通，使用者不會看到任何提示。
