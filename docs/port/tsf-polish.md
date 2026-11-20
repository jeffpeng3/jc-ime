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
| `SetSelection(index)` | 把點到的頁內索引換成數字鍵，送給引擎；核心照鍵盤 `1`–`0` 的規則選取並提交 |
| `Finalize()` | 已經送出數字鍵時不補鍵；否則送 `Space`，由引擎以目前高亮項選字 |
| `Abort()` | 送 `Escape`，由引擎關閉候選窗 |

### 點選流程

```text
Windows 候選窗（滑鼠點擊）
  └─ QI(ITfCandidateListUIElementBehavior)
       ├─ SetSelection(nIndex)
       │    ├─ nIndex >= GetCount() → 記 log，忽略點選
       │    ├─ 引擎未連線 → 只記 log，回 S_OK
       │    ├─ nIndex 不在核心目前頁 → 記 log，忽略點選
       │    ├─ 頁內位置 = nIndex % CANDIDATE_PAGE_SIZE
       │    ├─ 數字鍵 = 頁內位置 + 1（頁面 10 項時第 10 項是 0）
       │    ├─ settled = true（數字鍵本身就是選取＋提交）
       │    └─ 送這一個數字鍵
       ├─ Finalize()
       │    ├─ 已 settled → 不重送
       │    └─ 送 Space
       └─ Abort()
            └─ 送 Escape（已 settled 則略過）
```

為什麼用數字鍵：

- 與鍵盤選字是同一條路徑：核心 `select_by_digit` 以
  `current_page * CANDIDATE_PAGE_SIZE + 頁內位置` 取候選，數字鍵本身就代表
  「選取並提交」，wrapper 沒有另外發明選字規則。
- 不再模擬方向鍵：點第 7 項原本要送 6 次 `Down` 再一次 `Space`（每次都是一趟
  pipe 往返），現在只有一個按鍵。
- 頁面大小讀 `jc_core::output::CANDIDATE_PAGE_SIZE`，外殼沒有寫死 9；之後改成
  10 項一頁時，第 10 項自動換成 `0`。
- 協定仍是 v3：不需要「選取指定索引」的新請求。

代價是 `SetSelection` 不再只是「移動高亮」，點下去就等於選字提交。連續點選
（拖曳）時第二下會落在已經提交後的畫面上，因此不改 `GetSelection`；畫面仍然
只反映引擎回應。`Finalize`／`Abort` 共用 `settled` 旗標，避免 Windows 重複
呼叫時送出兩次鍵。

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

依 `common.md`，測試只編譯不執行；新增測試涵蓋顯示屬性內容、滑鼠點選的數字鍵
換算（`1`–`9`、第 10 項為 `0`、頁內位移）、索引越界與確認／取消鍵。

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
   若是，數字鍵已提交、`Finalize` 被 `settled` 擋掉，不會多打一個空白。
8. 是否有應用程式會為了「只移動高亮」而呼叫 `SetSelection`（例如候選窗內的
   鍵盤導覽）；那種情況會變成提前選字提交。
9. 點選不在核心目前頁的項目時，目前是忽略並記 log；實機上 Windows 是否真的
   會給出跨頁索引。
10. 候選窗顯示中但核心的 `candidate_window` 為 false（例如注音還沒打完）時，
    數字鍵會退回注音鍵；此時點選會插入一個數字。需要實機確認這個狀態是否
    會出現候選窗。
11. `Abort` 送 `Escape` 是否會連未完成的注音一起清掉（引擎的 Escape 有優先
    順序：先關候選窗）。
12. 應用程式回 `bShow == FALSE`（自行繪製候選）時，是否還會呼叫 Behavior；
    若不會，滑鼠點選在該應用程式內仍然無效。
13. `GetSelection` 在點選後、`State` 回來前仍回舊值；Windows 是否會因此把
    選取框畫回舊位置。
