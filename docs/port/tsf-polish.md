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

（下一個 commit 補上。）

## 驗證

（最後一節統一記錄三條指令的結果。）

## 未驗證清單（需要實機）

（最後一節統一記錄。）
