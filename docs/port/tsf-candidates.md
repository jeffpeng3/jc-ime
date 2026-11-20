# tsf-candidates：候選窗

`crates/jc-tsf` 的候選窗。第一版用 Windows 內建的
`ITfCandidateListUIElement`，不自繪視窗；資料來源是 `State.candidates`、
`State.details`、`State.highlighted` 與 `State.label`。

## 移植對照

Go 參考實作沒有 TSF 外殼，這個 task 沒有可以直接搬的檔案；分頁與選字的行為
照 `docs/wrapper.md` §6 與 `jc-core` 既有的 `Session`。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| n/a（Go 沒有 TSF 實作） | `crates/jc-tsf/src/candidates.rs`（UI 元素、分頁、Begin／Update／End） | rewritten；本 task 新寫 |
| `core/session/candidate_mode.go`（選字、翻頁） | `crates/jc-core/src/session/**`（既有；引擎端執行） | 直接沿用；wrapper 只送 `Request::Key` |
| `core/session/selection_keys.go` | `crates/jc-core/src/session/input_style.rs`（既有） | 直接沿用；wrapper 不重算選字鍵 |
| `core/session/idle_keys.go` | `crates/jc-tsf/src/keys.rs`（本 task 順帶修正） | rewritten：改成送實體按鍵字元，讓引擎決定注音或字面 |
| n/a | `crates/jc-tsf/src/register.rs`（新增 UIELEMENTENABLED 能力分類） | rewritten |
| n/a | `crates/jc-tsf/src/guids.rs`（新增 `GUID_CANDIDATE_UI_ELEMENT`） | rewritten |

## 架構

```text
Control::new
  └─ ITfThreadMgr::QI(ITfUIElementMgr)

State 進 drain()
  ├─ candidates 空 → CandidateHost::end()
  │                   └─ EndUIElement(id)
  └─ candidates 非空
       ├─ 第一次 → 建立 CandidateListElement
       │            ├─ 填 CandidateSnapshot
       │            ├─ BeginUIElement(&ITfUIElement, &bShow, &id)
       │            └─ bShow == FALSE 時再 UpdateUIElement(id) 一次
       └─ 之後   → 就地更新快照
                    ├─ 算出 TF_CLUIE_* 差異
                    └─ 有差異 → UpdateUIElement(id)
```

`CandidateListElement` 同時實作 `ITfUIElement` 與
`ITfCandidateListUIElement`，不是 agile（`Agile = false`）：所有方法都只在
TSF 的 UI 執行緒被呼叫，資料用 `Rc<RefCell<_>>` 與 `CandidateHost` 分享。

顯示生命週期跟著候選而不是跟著組字字串：`preedit` 空但 `candidates` 非空
（符號／表情選單）仍然算候選窗開著，這是 `docs/wrapper.md` §6.1 的既有狀態。
一般情況兩者同時清空，效果與任務書的「`preedit` 清空時關閉」相同。

## 介面行為

| 方法 | 行為 |
|---|---|
| `GetDescription` | 固定字串 `jc 注音：候選清單` |
| `GetGUID` | `GUID_CANDIDATE_UI_ELEMENT`（`{9C4E7A21-6B3D-4F08-B5A7-2E1D8C6F9034}`） |
| `Show`／`IsShown` | 記錄 TSF 要求顯示或隱藏的狀態 |
| `GetCount` | 候選總數，不是單頁數量 |
| `GetSelection` | 引擎的絕對高亮索引 `highlighted`；超出範圍時收回最後一項 |
| `GetString(i)` | `candidates[i]`；越界回 `E_INVALIDARG` |
| `GetPageIndex` | 每頁 9 項，回傳每頁起始索引；`pIndex == NULL` 時只回頁數 |
| `SetPageIndex` | 接受外部指定的頁首，下一次資料更新時重設回 9 項分頁 |
| `GetCurrentPage` | 由 `highlighted` 與頁首反推，不另外保存頁碼 |

`GetPageIndex` 的頁首是 `0, 9, 18, …`；`GetCount` 是全部候選數。Windows
用這兩個值加上 `GetCurrentPage` 決定目前要畫哪 9 項。

## 更新旗標

每次更新會比較新舊快照，設定真正變更的 `TF_CLUIE_*` 位元：

| 旗標 | 觸發條件 |
|---|---|
| `TF_CLUIE_DOCUMENTMGR` | `ITfContext::GetDocumentMgr` 換了 |
| `TF_CLUIE_COUNT` | 候選數量變了 |
| `TF_CLUIE_STRING` | 任一候選字面變了 |
| `TF_CLUIE_SELECTION` | `highlighted` 變了 |
| `TF_CLUIE_PAGEINDEX` | 頁首變了（數量或 `SetPageIndex`） |
| `TF_CLUIE_CURRENTPAGE` | 目前頁碼變了 |

`BeginUIElement` 後的第一次讀取固定回全部旗標。沒有差異就不呼叫
`UpdateUIElement`，避免每個按鍵都叫 Windows 重畫。

## 選字與翻頁

候選窗顯示時，按鍵照舊全部轉成 `Request::Key`：

| 使用者動作 | 送出的 Key | 說明 |
|---|---|---|
| `1`–`9` | 實體數字 `Char('1')`…`Char('9')` | 由 `jc-core` 判斷是選字還是注音鍵 |
| 候選可見但 `navigating == false` 時按 `1`–`9` | 先送 `Down`，再送數字 | 引擎第一次 `Down` 只開候選窗、不動高亮；不改變使用者看到的選項 |
| `↑`／`↓` | `Up`／`Down` | 第一次進入選取模式，之後移動 `highlighted` |
| `PageUp`／`PageDown` | `PageUp`／`PageDown` | 引擎換頁，`highlighted` 跳到該頁第一項 |
| `Space` | `Space` | 引擎選取目前高亮項，回空 `State` 後補 `Commit` |
| `Enter` | `Enter` | 引擎選取並提交 |

wrapper 不自己從 `candidates` 挑字、不自己組提交字串、不改 `State`。
數字鍵的 `Down` 前綴只是把「請進入選取模式」交給引擎，選哪一項仍由引擎決定。

`keys.rs` 在這個 task 一併修正：先前把 `j` 直接翻成 `Key::char('ㄨ')`，引擎
的注音輸入其實期待實體按鍵（`j`），會把注音符號當字面插入。現在一律送實體
字元（字母轉小寫、Shift 放 `shift` 欄位），數字也一樣，所以同一個 `1` 在
候選窗開著時是選字、沒開時是 ㄅ。空白只在組字中攔，沒組字時放行給應用程式。

## `navigating` 與顯示條件

第一版**只要 `candidates` 非空就顯示**，不等 `navigating`。理由：

- 引擎打完一個音節就回候選；只顯示 `navigating == true` 會讓最常見的情況
  （正在打字）看不到任何候選，和 verify 清單 4.1 的預期相反。
- Windows 的候選清單沒有「選取框強調」這種外觀；能不能用數字選字由引擎
  狀態決定，不需要靠隱藏候選窗來表達。
- `docs/wrapper.md` §6.1 的既有契約就是以 `candidates` 非空當顯示條件。

`navigating` 只用來決定數字鍵要不要補 `Down`。之後接上 `Settings` 時，可以
加一個 `candidate_window = "always" | "navigating"` 的選項；本 task 沒有改
`settings.json`，也沒有新增別的設定來源。

## 顯示限制（需要自繪視窗的部分）

**`ITfCandidateListUIElement` 不足，讀音與 label 畫不出來。** 實際看到的限制：

1. 每個候選只有 `GetString(index)` 一個字串。沒有第二行、沒有附註欄位。
2. `GetDescription` 是整個 UI 元素的說明（給輔助技術讀），不是逐項的讀音或
   label；不能拿來假裝候選的第二行。
3. `GetGUID` 只識別元素種類，與內容無關。
4. `ITfCandidateListUIElementBehavior` 是給應用程式控制候選的介面；本版沒有
   實作，點擊 Windows 畫出來的候選窗不會選字。
5. 顏色、字型、每列欄數、組字字串下方的相對位置都由 Windows 決定，TIP
   不能指定。

因此 `CandidateItem.reading` 與 `CandidateItem.label` 只存在快照裡，畫面只會
看到候選字面。若產品要顯示「讀音／來源／label」，必須改成自繪視窗（例如
GDI 分層視窗），不能靠這個介面；本 task 沒有改協定去硬塞資料。

## 分頁與引擎的落差（跨 task）

`ITfCandidateListUIElement` 每頁固定 9 項，是任務書的規格。但引擎的每頁
項數由 `InputStyle.rules.candidate_page_size` 決定，**目前預設是 10**
（`0c` 預設；只有小麥注音是 9）。協定 v3 沒有帶頁面大小，wrapper 無法查詢。

影響：

- 第 1 頁（索引 0–8）顯示與數字選字一致。
- `PageDown` 時引擎把 `highlighted` 移到 `page * 10`，UI 卻用 9 計算頁碼；
  兩邊的頁邊界會逐漸錯開。畫面仍會換頁，但同一頁的內容可能與引擎認定的
  頁不同。
- 數字鍵選字在 `jc-core::session::select_by_digit` 少了頁首位移：
  參考實作是 `candidateID(candidatePage()*pageSize() + n)`，Rust 版直接
  `candidates.get(n)`。因此第 2 頁以後按數字會選到清單最前面的候選。這是
  `jc-core` 的既有 bug，不在本 task 的 owned paths。

需要後續處理：

1. `settings-wire` 接上後，讓 wrapper 與引擎使用同一個 `candidate_page_size`；
   或在協定 v4 增加可忽略的頁面資訊。
2. 修 `jc-core` 的 `select_by_digit` 頁首位移，並補行為測試（第 2 頁選第 2 項）。

## 驗證

| 指令 | 結果 |
|---|---|
| `cargo check -p jc-tsf --target x86_64-pc-windows-msvc --all-targets` | 通過 |
| `cargo check -p jc-tsf --target x86_64-pc-windows-gnu --all-targets` | 通過 |
| `cargo clippy -p jc-tsf --target x86_64-pc-windows-msvc --all-targets` | `jc-tsf` 零警告（`jc-model` 有既有的 `unused_mut`） |
| `cargo check --workspace --all-targets` | 通過（Linux 上 `jc-tsf` 是空 crate） |

依 `common.md`，測試只編譯不執行；新增的測試涵蓋分頁、頁碼反推、高亮夾取與
數字鍵的 `Down` 前綴判定。

## 未驗證清單（需要實機）

以下全部沒有在 Windows 上跑過：

1. `BeginUIElement` 是否讓 Windows 畫出候選窗，位置是否貼在組字字串下方。
2. `UpdateUIElement` 的 `TF_CLUIE_*` 是否讓畫面正確重畫（換頁、換高亮、改字）。
3. `EndUIElement` 是否讓候選窗立刻消失；快速打字時會不會殘留。
4. `GetPageIndex` 以 `pIndex == NULL` 先問頁數的呼叫路徑。
5. `GetSelection` 的絕對索引是否符合 Windows 預期（目前照 SDK 範例）。
6. `ITfUIElementMgr` 在 Chromium、Word、記事本是否都拿得到。
7. 應用程式回 `bShow == FALSE`（自行繪製候選）時的 `UpdateUIElement` 行為。
8. 數字鍵在 Windows 鍵盤配置下（含 NumLock 數字鍵盤）是否都能選字。
9. `GUID_TFCAT_TIPCAP_UIELEMENTENABLED` 是否為 `BeginUIElement` 成功的必要
   條件；未註冊時的行為沒有實測。
10. 候選窗與組字區的焦點、換視窗、外部終止組字時的收尾。
11. 讀音與 label 確定看不到（見上一節）；若實機意外看得到，請附截圖與
    Windows 版本。

## 已知限制

- 沒有 `ITfCandidateListUIElementBehavior`：滑鼠點選候選不支援。
- 讀音與 label 不顯示；需要自繪視窗。
- 每頁固定 9 項；與引擎預設的 10 項頁面不一致（見上一節）。
- 第 2 頁以後的數字選字受 `jc-core` 頁首 bug 影響。
- 沒有 `Settings` 接線；顯示條件固定是「有候選就顯示」。
- `docs/port/tsf-skeleton.md` 仍寫「沒有候選窗」；該檔不在本 task 的 owned
  paths，交給整合階段一起更新。
