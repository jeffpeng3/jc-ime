# TSF 候選窗與組字顯示

候選窗是 TSF DLL 裡的一個 HWND：Direct2D 畫、DirectComposition 上屏。沒有新增
程序、沒有第二個執行緒，也不經 named pipe 把畫面交給另一個 exe。設定程式
`jc-settings-ui` 是獨立程式，與候選窗無關。

## 架構

```text
jc-tsf（宿主行程內）
  service.rs        取得焦點、組字、提交、放置候選窗
  ui.rs             UiClient：決定是否繪製、延遲初始化、失敗收起、Deactivate 銷毀
  ui_window.rs      HWND、D3D11／DXGI、Direct2D／DirectWrite、DirectComposition、主題、滑鼠
  candidate_layout.rs  純版面：列高、多欄、命中測試、監視器夾取、DPI 換算
  caret.rs          GetTextExt 失敗時用 GetCaretPos 的退路
```

- `candidate_layout.rs` 是純函式，Linux 也編譯、也跑單元測試。
- `ui.rs` 延遲初始化：第一次需要畫候選時才建視窗與裝置；建立失敗就收起，不影響
  打字。`Deactivate` 時銷毀視窗並釋放資源。
- 候選數量、欄數、列數與高亮由核心的 `Snapshot` 決定；外殼只負責排版與命中測試。
  版面與數字鍵規則見 [`candidate-unify.md`](candidate-unify.md)。
- 候選點選換算成 `HostEvent::CandidateClicked`，索引以整個候選清單為準；一般候選
  的點選與 Enter 相同，套用後保留組字。
- 組字區的虛線底線由 `ITfDisplayAttributeProvider` 提供；沒有候選時只顯示組字。
- 系統插入點取得失敗時用 `GetCaretPos`；候選窗會夾在監視器工作區內，並依 DPI
  換算尺寸。
- 主題跟隨系統深色／淺色外觀。

## 檔案

| 檔案 | 內容 |
|---|---|
| `crates/jc-tsf/src/candidate_layout.rs` | 列高、多欄、命中測試、監視器夾取、DPI |
| `crates/jc-tsf/src/ui_window.rs` | HWND 與繪製裝置、主題、滑鼠 |
| `crates/jc-tsf/src/ui.rs` | `UiClient` 生命週期 |
| `crates/jc-tsf/src/caret.rs` | 插入點 |
| `crates/jc-tsf/src/service.rs` | 組字、提交、候選窗放置與收起 |
