# jc-platform：平台抽象對照與契約

參考實作：`/mnt/work/jc/ref/0c-ime`（main `8f443af`）。Go 版沒有獨立的
host 抽象層：Fcitx5／TSF／macOS wrapper 各自用 C++、Swift 直接接核心，
核心端則直接讀時間。因此本 crate 是重新設計的共同契約，不是逐檔移植。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `platforms/linux-fcitx5/src/session.cpp`（preedit、候選、送出） | `crates/jc-platform/src/host.rs` | n/a：C++ wrapper 端實作，行為只當語意參考；Rust 端改成 `InputHost` trait |
| `platforms/linux-fcitx5/src/panel.h`（游標位置、候選高亮） | `crates/jc-platform/src/host.rs` | n/a：同上；只取「游標是組字文字的位移」「highlight 是候選索引」兩點，位移單位改成 UTF-16 |
| `platforms/windows/tsf/`（TSF 文字服務回呼） | `crates/jc-platform/src/host.rs` | n/a：Windows wrapper 不在本 task，且 trait 不能引入 HWND 等 OS 型別 |
| `platforms/macos/`、`platforms/linux-ibus/` | — | n/a：本輪只定核心契約，不實作 wrapper |
| `core/ja/live/live.go`（`Config.Clock func() time.Time`） | `crates/jc-platform/src/clock.rs` | rewritten：改成 `Clock` trait，全核心共用；另外補 `Scheduler` 與假時鐘 |
| （無對應） | `crates/jc-platform/src/headless.rs` | rewritten：Go 版沒有可記錄呼叫的 headless host；測試直接呼叫 Go 函式 |
| （無對應） | `crates/jc-platform/src/host.rs`（`HostEvent`） | rewritten：Go 版事件散在各 wrapper 的回呼裡，這裡先定型別 |
| （無對應） | `crates/jc-platform/tests/contract.rs` | rewritten：Go 版沒有對應測試，改寫成契約測試 |

## Wrapper 要實作什麼

wrapper 是唯一碰 OS API 的地方，至少要提供：

| 項目 | 責任 |
|---|---|
| `InputHost` | 把核心推來的狀態畫到畫面上；不自行解讀組字內容 |
| `Clock` | 提供單調前進的毫秒時間；測試用 `ManualClock` |
| `Scheduler` | 依核心要求排計時器，到期時把 `HostEvent::Timer` 送回核心 |
| 事件轉譯 | 把平台事件轉成 `HostEvent`，焦點、點選、滾動都只送語意，不送 OS 座標 |

### `InputHost`

- `update_preedit(text, cursor_utf16)`：`text` 是完整組字文字（含未轉換的
  讀音），不是增量。`cursor_utf16` 以 UTF-16 code unit 計算，介於
  `0..=text.encode_utf16().count()`；emoji 等增補平面字元佔 2 個單位。
  重複呼叫同一個 `text` 是合法的，wrapper 應視為覆蓋。
- `clear_preedit()`：收掉組字區。核心在焦點遺失、取消、`commit` 之後都會
  呼叫；wrapper 不要在 `commit` 裡自己清，否則會與核心重複。
- `show_candidates(items, highlight)`：`items` 是「目前這一頁」的候選詞面，
  已按顯示順序排好；`highlight` 是 `items` 的索引。`items` 為空代表關閉
  候選窗，此時 `highlight` 一定是 `0`。
- `commit(text)`：把文字送進應用程式。核心保證 `text` 非空，且之後會送
  `clear_preedit`。
- `show_status(text)`：狀態列／提示訊息，例如「學習中」「模型未載入」。
  空字串代表清除提示。這只是提示，wrapper 可以選擇不畫。

### 錯誤處理

每個方法都回傳 `Result<(), HostError>`。wrapper 在視窗已關閉、候選窗被
系統封鎖、平台不支援狀態列等情況回傳對應變體，**不得 panic**。核心會把
錯誤當成「這次畫面更新沒成功」繼續處理輸入，不會因此丟掉組字狀態。

`HostError` 只帶訊息與分類，不帶 OS error code；需要的平台細節留在
wrapper 的記錄裡。

### 執行緒與重入

- trait 方法只在輸入法執行緒被呼叫，實作不需要自己加鎖。
- wrapper 不得在 `InputHost` 方法內同步把 `HostEvent` 餵回核心；事件要
  排到下一個回合，避免核心重入。
- `HostEvent::Timer` 的 `id` 只能是 `Scheduler::schedule_after` 回傳的值，
  同一個 id 只送一次。

## 核心保證什麼

- 只透過 `InputHost` 更新畫面；不持有視窗、輸入法上下文或檔案描述符。
- 不假設平台字串編碼：介面一律 UTF-8 `&str`，只有游標位移是 UTF-16。
- `commit` 之後一定接 `clear_preedit`；`commit` 的文字不為空。
- `show_candidates` 的 `highlight` 在 `items` 非空時一定小於 `items.len()`。
- 候選點選（`HostEvent::CandidateClicked`）的 `index` 以整個候選清單為準，
  不是目前頁面的相對索引；分頁由核心處理。
- 焦點遺失後不再呼叫 `update_preedit`／`show_candidates`，直到
  `HostEvent::FocusGained`。
- 時間只從注入的 `Clock`／`Scheduler` 取得；核心不在背景自行開執行緒。

## 記錄與除錯

`HeadlessHost` 是沒有 UI 的 `InputHost`：每個呼叫依序存成 `HostCall`，
測試直接比對 `calls()`。`RecordingHost<H>` 在轉給內層宿主前先記錄，內層
回傳錯誤時仍保留該筆記錄；`to_json()`／`to_json_pretty()` 輸出固定形狀：

```json
{"calls":[{"call":"preedit","text":"ㄋㄧ","cursor_utf16":2},
          {"call":"candidates","items":["你","尼"],"highlight":0},
          {"call":"commit","text":"你"}]}
```

`HostCall` 的 `call` 標籤與 `HostEvent` 的 `event` 標籤都是契約的一部分，
改名要同步更新 wrapper 與測試。

## 測試

`crates/jc-platform/tests/contract.rs` 覆蓋：

- 呼叫記錄順序（status → preedit → candidates → commit → clear）。
- 候選高亮索引與 `commit` 的組合（高亮項就是送出的文字）。
- JSON 形狀與 round trip。
- 內層宿主失敗時仍保留記錄並回傳錯誤。
- 假時鐘只能前進，`FakeScheduler` 依到期時間觸發、可取消。
- `HostEvent` 的 JSON round trip。
