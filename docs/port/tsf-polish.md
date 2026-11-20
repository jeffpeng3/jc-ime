# tsf-polish：組字區底線與候選窗滑鼠點選

`crates/jc-tsf` 的兩個外觀／互動缺口。兩件事各自一個 commit：

1. **組字底線**：在 `TextService` 上實作 `ITfDisplayAttributeProvider`，用
   `ITfProperty`（`GUID_PROP_ATTRIBUTE`）把虛線底線套到組字範圍。
2. **候選窗滑鼠點選**：讓 Windows 畫的候選清單實作
   `ITfCandidateListUIElementBehavior`，點選只送按鍵請求，由引擎決定結果。

## 移植對照

Go 參考實作沒有 TSF 外殼，這兩項都是 Windows 平台層的新程式碼，沒有可直接
搬過來的檔案。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| n/a（Go 沒有 TSF／顯示屬性） | `crates/jc-tsf/src/display.rs`（新增：`ITfDisplayAttributeInfo`、列舉器、atom 註冊） | rewritten |
| n/a | `crates/jc-tsf/src/lib.rs`（`TextService` 多實作 `ITfDisplayAttributeProvider`） | rewritten |
| n/a | `crates/jc-tsf/src/register.rs`（新增 `GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER` 分類） | rewritten |
| n/a | `crates/jc-tsf/src/guids.rs`（新增 `GUID_DISPLAY_ATTRIBUTE_INPUT`） | rewritten |
| n/a | `crates/jc-tsf/src/composition.rs`（套用／清除 `GUID_PROP_ATTRIBUTE`） | rewritten |
| `core/session/candidate_mode.go`（選字行為） | `crates/jc-core/src/session/**`（既有；引擎端執行） | 直接沿用；wrapper 只送 `Request::Key` |
| n/a | `crates/jc-tsf/src/candidates.rs`（新增 `ITfCandidateListUIElementBehavior`） | rewritten |

## 1. 組字底線

### GUID 與註冊

| 項目 | 值 |
|---|---|
| `GUID_DISPLAY_ATTRIBUTE_INPUT` | `{A2C9F1D4-7B3E-4C58-9E21-5D8A6F0B4C72}` |
| 分類 | `GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER`，值為 `CLSID_JC_TSF` |
| 物件 | `TextService` 本身（`QueryInterface` 取得，沒有獨立 CLSID） |

`DllRegisterServer` 除了原本的鍵盤與 `UIELEMENTENABLED` 分類，現在也把
`CLSID_JC_TSF` 註冊到 `GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER`；反註冊時一併
移除。少了這個分類，Windows 收到 atom 也找不到 provider，畫面就只會有預設
外觀。

### 顯示屬性內容

`GetAttributeInfo` 固定回傳：

| 欄位 | 值 | 說明 |
|---|---|---|
| `crText` | `TF_CT_NONE` | 不指定文字色，沿用應用程式 |
| `crBk` | `TF_CT_NONE` | 不指定背景色 |
| `lsStyle` | `TF_LS_DOT` | 虛線底線 |
| `fBoldLine` | `FALSE` | 細線 |
| `crLine` | `TF_CT_SYSCOLOR` / `COLOR_WINDOWTEXT` | 跟隨系統視窗文字色 |
| `bAttr` | `TF_ATTR_INPUT` | 語意是「尚未轉換的輸入」 |

`SetAttributeInfo` 回 `E_NOTIMPL`：屬性是唯讀常數。列舉器只有這一項，
`Clone`／`Reset`／`Skip` 照 COM 慣例實作。

### 套用流程

```text
Activate
  └─ CoCreateInstance(CLSID_TF_CategoryMgr)
       └─ RegisterGUID(GUID_DISPLAY_ATTRIBUTE_INPUT) → TfGuidAtom
           └─ 失敗：記 log、display_atom = None（只影響底線）

State 有 preedit
  └─ RequestEditSession(TF_ES_SYNC | TF_ES_READWRITE)
       ├─ StartComposition（第一次）
       ├─ ITfComposition::GetRange() → SetText
       ├─ 游標定位
       └─ ITfContext::GetProperty(GUID_PROP_ATTRIBUTE)
            └─ SetValue(ec, range, VARIANT{ VT_I4, lVal = atom })

結束組字／提交
  └─ 同一個 edit session：
       ├─ ITfComposition::GetRange()
       ├─ ITfProperty::Clear(ec, range)
       └─ EndComposition → InsertTextAtSelection
```

atom 只在同一個 TSF 用戶端內有效，所以每次 `Activate` 都重新註冊；沒有寫進
設定或註冊機碼。套用與清除的失敗都只記錄 `debug` log，不影響組字、提交或
按鍵處理。

## 2. 候選窗滑鼠點選

`CandidateListElement` 現在同時實作 `ITfUIElement`、`ITfCandidateListUIElement`
與 `ITfCandidateListUIElementBehavior`；Windows 直接從 `ITfUIElement` QI 到
Behavior，不需要另一個物件或 CLSID。

| 方法 | 行為 |
|---|---|
| `SetSelection(index)` | 把點到的絕對索引換成上／下鍵，送給引擎移動高亮；不改字、不直接挑候選 |
| `Finalize()` | 送 `Space`，由引擎以目前高亮項選字並提交 |
| `Abort()` | 送 `Escape`，由引擎關閉候選窗 |

### 點選流程

```text
Windows 候選窗（滑鼠點擊）
  └─ QI(ITfCandidateListUIElementBehavior)
       ├─ SetSelection(nIndex)
       │    ├─ nIndex >= GetCount() → E_INVALIDARG
       │    ├─ 引擎未連線 → 只記 log，回 S_OK
       │    ├─ 以「上一次請求的索引」為基準算前後距離
       │    ├─ 未進選取模式 → 先補一個 Down（不移動高亮）
       │    ├─ 送較短方向的 Down／Up（同一世代）
       │    └─ 記下 requested；不改 GetSelection
       ├─ Finalize()
       │    ├─ 已 settled → 不重送
       │    └─ 送 Space
       └─ Abort()
            └─ 送 Escape（已 settled 則略過）
```

為什麼用上／下鍵而不是數字鍵：

- 數字鍵在引擎裡是「直接選字並提交」，沒有「只移動高亮」的語意；`SetSelection`
  之後可能還有 `Finalize`，用數字鍵會提前提交。
- `jc-core::session::select_by_digit` 目前沒有加頁首位移，第 2 頁以後的數字選字
  會選到清單最前面的候選；上／下鍵是絕對索引移動，不受這個 bug 影響。
- 協定 v3 沒有「選取指定索引」的請求，owned paths 也不能改 `jc-core`／
  `jc-proto`。因此**不需要協定變更**；代價是索引離目前高亮很遠時會連續送多個
  按鍵（每個都是一次 pipe 往返，同一批共用世代，引擎依序處理）。

`SetSelection` 連續呼叫（例如拖曳）時，`requested` 會記住已送出但還沒被
`State` 確認的目標，下一次只補差額；引擎的 `State` 一回來就以它為準並清掉
`requested`。wrapper 不改 `GetSelection` 的值，畫面仍然只反映引擎回應。

`Finalize`／`Abort` 共用 `settled` 旗標，避免 Windows 重複呼叫時送出兩次
`Space`／`Escape`。

### 與一般按鍵的關係

滑鼠點選與鍵盤共用同一個 `mpsc` 通道與世代計數器（`Control::generation`）；
點選不是繞過引擎的捷徑。若點選後使用者立刻打字，按鍵仍照原本的 `generation`
規則淘汰過期 `State`，`Committed` 一律套用。

## 驗證

| 指令 | 結果 |
|---|---|
| `cargo check -p jc-tsf --target x86_64-pc-windows-msvc --all-targets` | 通過 |
| `cargo check --workspace --all-targets` | 通過（需 `RUST_FONTCONFIG_DLOPEN=1`；本機沒有 fontconfig 的 `.pc`，與本 task 無關） |
| `cargo clippy -p jc-tsf --target x86_64-pc-windows-msvc --all-targets --no-deps -- -D warnings` | `jc-tsf` 零警告 |

`cargo clippy -p jc-tsf ... -- -D warnings`（不加 `--no-deps`）會撞到
`jc-core/src/session/core.rs:692` 既有的 `collapsible_if`；該檔不在 owned
paths，沒有動它。`cargo fmt -p jc-tsf` 已執行。

依 `common.md`，測試只編譯不執行；新增測試涵蓋顯示屬性內容、滑鼠點選的前後
方向、選取模式前綴、索引越界與確認／取消鍵。

## 未驗證清單（需要實機）

底線：

1. `GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER` 註冊後，Windows 是否真的用這個
   provider 解讀 atom；未註冊時的外觀沒有實測。
2. `ITfProperty::SetValue` 的 `VT_I4` atom 是否被 Word、記事本、Chromium
   一致接受；不同 TSF 版本可能有差異。
3. 虛線底線的粗細、顏色與 `COLOR_WINDOWTEXT` 在深色主題下的實際外觀。
4. 快速更新組字（`TF_ST_CORRECTION`）時底線會不會閃爍或殘留。
5. `EndComposition` 前的 `ITfProperty::Clear` 是否足以清乾淨；外部終止組字
   （`OnCompositionTerminated`）時屬性是否由 TSF 自動移除。
6. `CoCreateInstance(CLSID_TF_CategoryMgr)` 在 `Activate` 期間失敗時的降級
   路徑（只有 log，沒有底線）。

滑鼠點選：

7. Windows 內建候選窗是否在單擊時先呼叫 `SetSelection` 再呼叫 `Finalize`；
   若只呼叫其中一個，行為會是「只移動高亮」或「直接確認目前高亮」。
8. 候選很多（索引離高亮很遠）時，連續上／下鍵的延遲與畫面更新順序。
9. 拖曳或連點時 `requested` 的差額計算是否符合實際呼叫序列。
10. `Abort` 送 `Escape` 是否會連未完成的注音一起清掉（引擎的 Escape 有優先
    順序：先關候選窗）。
11. 應用程式回 `bShow == FALSE`（自行繪製候選）時，是否還會呼叫 Behavior；
    若不會，滑鼠點選在該應用程式內仍然無效。
12. 與數字鍵、`PageUp`／`PageDown` 交錯使用時的高亮一致性。
13. `GetSelection` 在點選後、`State` 回來前仍回舊值；Windows 是否會因此把
    選取框畫回舊位置。
