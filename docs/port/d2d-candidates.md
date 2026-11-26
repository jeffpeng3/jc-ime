# d2d-candidates：TSF DLL 內自繪候選窗

候選窗現在是 TSF DLL 裡的一個 HWND：Direct2D 畫、DirectComposition 上屏。
沒有新增程序，沒有第二個執行緒，也不再經 named pipe 把畫面交給 `jc-ui.exe`。
設定程式的 GPUI（`jc-settings-ui`）完全沒有動到。

## 檔案

| 檔案 | 內容 |
|---|---|
| `crates/jc-tsf/src/candidate_layout.rs` | 純版面：列高、多欄、命中測試、監視器夾取、DPI 換算。Linux 也編、也測 |
| `crates/jc-tsf/src/ui_window.rs` | HWND、D3D11／DXGI／Direct2D／DirectWrite／DirectComposition、主題、滑鼠點選 |
| `crates/jc-tsf/src/ui.rs` | `UiClient`：決定這次要不要畫、延遲初始化、失敗就收起、Deactivate 時銷毀 |
| `crates/jc-tsf/src/caret.rs` | 新增 `system_caret()`：`GetTextExt` 失敗時的 `GetCaretPos` 退路 |
| `crates/jc-tsf/src/service.rs` | `place()` 接上系統插入點退路；`forget_candidates()` 與 `Deactivate` 收起／銷毀候選窗 |
| `crates/jc-tsf/Cargo.toml` | 打開 Direct2D／DirectWrite／D3D11／DXGI／DirectComposition 的 `windows` features |

## 架構

```text
service::present_in_edit
  ├─ UiMessage（候選、highlight、key、page/pages、reading、caret、target、revision）
  └─ ui::UiClient::present
      ├─ uielement::Elements      UI-less 宿主（遊戲）自己畫 → 我們不畫
      └─ ui_window::LocalWindow   第一次真的有候選才建立
          ├─ candidate_layout    純函式：版面、命中、夾取、DPI
          └─ Graphics            D3D11 → DXGI swap chain → D2D device context
                                  → DirectComposition visual
```

`LocalWindow` 誕生在 TSF 的 UI 執行緒上，也只被那個執行緒碰。它不持有
`TextService` 的指標或 COM 參照，所以不會讓 `DllCanUnloadNow` 多算一個物件；
滑鼠點選只把訊息號碼、revision、候選 ID 送回去。

### 生命週期

| 時機 | 動作 |
|---|---|
| `Activate` | `UiClient::new()`；不建視窗、不碰 GPU |
| 第一次有候選 | `LocalWindow::new()`＋`Graphics::new()`；失敗就整個 Activate 期間不畫 |
| 每次 `present` | 讀主題（由隱藏變顯示時）、量文字、算版面、移到插入點、必要時 `ResizeBuffers`、畫、`Present(0)` |
| 沒有候選／`show=false` | `SW_HIDE`；GPU 資源留著，下一鍵直接重用 |
| `Escape`／commit／`OnSetFocus(false)` | `forget_candidates()` → `UiClient::hide()` → `SW_HIDE` |
| `Deactivate` | `UiClient::shutdown()` → `LocalWindow` 的 `Drop` → 釋放 DComp／D2D／D3D／DWrite 參照、`DestroyWindow` |

## 為什麼不用 GPUI

1. GPUI 是獨立應用程式的框架：自己的事件迴圈、自己的視窗生命週期。TSF 候選窗
   必須活在宿主的 UI 執行緒與訊息迴圈裡，多一個 loop 就多一個搶焦點與重入的來源。
2. GPUI 會把 wgpu、accesskit、文字排版等依賴拉進 DLL。TSF DLL 會載入每一個
   宿主行程（explorer、瀏覽器、Office），載入成本與記憶體直接乘上行程數。
3. 候選窗只要幾個圓角矩形和幾行字。Direct2D＋DirectWrite 已經在系統裡，
   DLL 不必多帶一套渲染器。
4. DirectComposition 讓畫面直接上屏，不需要 `WM_PAINT`、layered window 或
   自繪視窗背景；也比較容易保證「絕不啟用視窗」。

## 焦點：候選窗不能搶

- 視窗樣式：`WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST |
  WS_EX_NOREDIRECTIONBITMAP`，類別是 `WS_POPUP`，不建工作列按鈕。
- 顯示只走 `SW_SHOWNOACTIVATE`；移動與調整大小一律 `SWP_NOACTIVATE |
  SWP_NOOWNERZORDER`；`WM_MOUSEACTIVATE` 回 `MA_NOACTIVATE`。
- 全程沒有 `SetFocus`、沒有 `SetForegroundWindow`、沒有 `SetActiveWindow`。
- `SWP_NOACTIVATE` 之外也刻意不在顯示前呼叫 `ShowWindow`：先畫好第一帧再顯示，
  避免空白閃一格。

## 點選

```text
WM_LBUTTONDOWN（客戶區像素）
  └─ dip_from_px → candidate_layout::hit_test
       └─ PostMessageW(target, WM_SELECT_CANDIDATE, revision, id)
            └─ candidate::callback（message-only 視窗，同一個 TSF 執行緒）
                 └─ service.select_candidate → ITfEditSession → 引擎
```

UI 執行緒只送訊息，不改組字狀態。`target` 是 `CandidateWindow::handle()`，
`revision` 與候選 ID 由 `service` 核對；過期的點選直接丟掉。這是原本
`WM_SELECT_CANDIDATE` 路徑，沒有另開一條。

## 定位與高 DPI

1. 插入點矩形優先用 `edit::apply` 的 `ITfContextView::GetTextExt` 結果。
2. `GetTextExt` 失敗（Chromium 的 `TS_E_NOLAYOUT` 等）時，`caret::system_caret()`
   依序問目前執行緒與前景執行緒的插入點（`GetCaretPos`／`GUITHREADINFO.rcCaret`），
   換成螢幕座標。
3. 仍然拿不到就沿用既有退路：同一次組字上一次的位置、這個視窗上一次的插入點。
4. `candidate_layout::place()` 預設貼在插入點下方，下方放不下翻到上方，最後
   把整個視窗夾進 `MonitorFromPoint` 取得的監視器工作區。
5. 視窗先移到插入點所在監視器，再用 `GetDpiForWindow` 問那個監視器的 DPI；
   DPI 變了就重算版面與位置。swap chain 的 bitmap DPI 設成該 DPI，所以繪製
   座標一律是 DIP，`GetDpiForWindow` 的縮放由 Direct2D 處理。

版面、命中、夾取、DPI 換算都是 `candidate_layout` 的純函式；Windows 端只提供
「這段文字在這個字級下多寬」的量測函式。

## 內容與版面

| 內容 | 畫法 |
|---|---|
| 候選清單 | 一頁最多十列（`columns > 1` 時每一欄是一直排，先填滿十列再換下一欄）；列高固定 26 DIP |
| highlight | 整格圓角底色；文字與編號改成高對比色 |
| 編號鍵 | 候選左側小字；`key` 是 `None` 或空字串時不畫，文字往左補 |
| 分頁指示 | `pages > 1` 時右下角畫 `目前頁 / 總頁`；頁碼夾在合法範圍 |
| `reading_hint` | 候選上方一行小字 |
| 附註（`note`） | 候選右側小字，低對比 |
| 空清單 | 直接收起，不留空框 |

`UiMessage.columns` 由核心的候選版面帶過來：預設 1 欄，按 `←`／`→` 後是 4 欄。
`UiMessage.candidates` 只放目前這一頁，外殼不再自行截斷列數。

## 主題

每次由隱藏變顯示時讀
`HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\AppsUseLightTheme`：
`0` 用深色配色，其餘（含讀不到）用淺色。文字用 DirectWrite 的
`Segoe UI`，CJK 由系統字型回退；標題、候選、編號、頁碼各一個
`IDWriteTextFormat`，量測與繪製共用同一份，版面不會和畫面差一個字。

## 失敗處理

- 視窗或 GPU 資源建立失敗：`zlog!` 一行原因，這次 `Activate` 期間不再嘗試，
  候選窗就是不出現。組字、提交、按鍵都不受影響。
- 執行期繪製失敗（裝置遺失、`ResizeBuffers` 失敗）：丟掉整組 `Graphics`、
  收起視窗，下一個按鍵重建；重建失敗才回到上一條。
- `IDXGISwapChain::Present(0, 0)`：不等垂直同步。第一帧的裝置建立是同步的，
  但那是唯一一次，且失敗就放棄；按鍵路徑不做 GPU 等待。
- `DllCanUnloadNow`：候選窗不計入 `LIVE_OBJECTS`，`Deactivate` 也會把
  DComp／D2D／D3D 參照全部放掉，計數回到 0。

## 對照表

| 參考 | Rust 檔案 | 狀態 |
|---|---|---|
| 0c `ui.rs`（pipe 候選窗） | `crates/jc-tsf/src/ui.rs` | rewritten：改成本行程自繪；`UiMessage`／`WM_SELECT_CANDIDATE` 仍共用 `jc_ui::message` |
| 0c `candidate.rs`（回呼視窗） | `crates/jc-tsf/src/candidate.rs` | 直接沿用，點選與延後焦點確認都沒改 |
| 0c `caret.rs`（系統插入點） | `crates/jc-tsf/src/caret.rs` | rewritten：新增 `system_caret()`，`choose()` 的退路順序不變 |
| Tsunagi `platform/windows/src/candidate_window.rs`、`d2d*` | — | n/a：授權不同（GPL-3.0-or-later），只參考「D2D＋DComp、NOACTIVATE、版面與命中分離」的架構 |
| `crates/jc-ui` 的候選窗程式 | — | n/a：TSF 不再送 pipe；本 task 沒有修改 `jc-ui` |
| 無對應 | `crates/jc-tsf/src/candidate_layout.rs` | 新增：純版面與命中 |
| 無對應 | `crates/jc-tsf/src/ui_window.rs` | 新增：HWND＋D2D＋DComp |

## 驗證

本機是 aarch64 Linux，只能做型別檢查與純函式測試：

```sh
cargo check -p jc-tsf --target x86_64-pc-windows-msvc --all-targets
cargo check -p jc-tsf --target i686-pc-windows-msvc --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo test --workspace
```

`candidate_layout` 的單元測試在 Linux 跑得到：

- 單欄堆疊、列高、內緣留白、編號與附註的位置。
- 空字串／`None` 編號不畫，文字往左補。
- highlight 超出範圍不反白。
- 多欄換列、等寬、不重疊、不超出視窗。
- 讀音列與頁碼列不參與命中。
- `pages <= 1` 不畫頁碼；頁碼越界夾回合法值。
- DPI 換算（96／120／144／168／192／240／288／384）與來回誤差 ≤ 0.5 像素。
- 插入點下方／上方翻轉、次要監視器、工作區比視窗小、零尺寸工作區。

本機**不能**驗證：真正的 HWND、D3D11 裝置、DirectComposition 上屏、焦點是否
真的沒被搶、滑鼠命中、主題切換、跨監視器 DPI。這些只能在實機看。

## 實機確認清單

1. 安裝 TSF 後在記事本打注音：候選窗出現在插入點下方，字型與深淺色正確。
2. 候選窗出現時在輸入框繼續打字：焦點與組字都沒斷；工作列沒有多一個視窗。
3. 點第 3 個候選：該候選被選取並提交；點空白或頁碼不會選字；點完焦點仍在原輸入框。
4. 把視窗拖到螢幕最下方再打字：候選窗翻到插入點上方；拖到最右邊：視窗被夾回工作區。
5. 兩顆不同 DPI 的螢幕之間移動同一個輸入框：候選窗大小與位置跟著 DPI 改變。
6. Windows 設定切換淺色／深色，重新打出候選：配色跟著換。
7. 按 Escape、切到別的程式、按 Enter 提交：候選窗都立刻收起。
8. 停用輸入法（切換到別的语言或登出）：`tsf.log` 沒有 GPU 錯誤，程式可卸載。
9. 故意讓 GPU 初始化失敗（例如在沒有 D3D 的環境）：`tsf.log` 只有一行
   `候選窗 Direct2D 初始化失敗`，打字與提交仍正常，沒有卡鍵。

## 已知限制

- **只畫候選與讀音提示。** 依任務要求「空清單就隱藏」，只有一次性提示
  （`notice`）或補全提示（`suggestion`）而沒有候選時，視窗不會出現。這是
  與舊 `jc-ui.exe` 的行為差異；提示訊息仍會產生，只是目前沒有畫面。
- `title`、`suggestion`、`notice` 沒有畫；`reading_hint` 只在有候選時一起出現。
- 多欄版面已實作並有測試；核心送 `rows`／`columns`，預設十列單欄，按
  `←`／`→` 後是十列四欄。
- 沒有滑鼠移入高亮、沒有滾輪、沒有候選捲動；一頁就是核心送來的這一頁。
- `WM_DPICHANGED` 不用系統建議矩形，而是用同一份訊息重算；視窗貼齊工作區
  邊緣時切換 DPI 可能有一帧位置微調。
- 視窗的 DPI 感知模式跟著宿主行程；宿主是 DPI-unaware 時，座標由系統虛擬化，
  候選窗仍然對齊插入點，但不會比宿主更清晰。
- `jc-ui.exe` 與它的 pipe 沒有刪除，只是 TSF 不再使用；其他工具若直接呼叫
  `ui_pipe()` 仍然可以連，但不會再收到 TSF 的訊息。
