# mode 收斂：core 只剩 Mixed，En 留在 TSF

狀態：已實作；2026-11-24 設計，2026-11-26 補上現況與已知限制。

## 為什麼

這一輪的 bug 幾乎全部來自「同一個行為只實作在 `Mode::Zh`」：

- `navigate()` 只在 `Mode::Zh` 移動音節游標 → 混打（安裝版預設）左右鍵完全沒反應。
- `space()`／`down()` 只在 `Mode::Zh` 補完 pending、開候選 → 混打沒有候選清單。
- `backspace()` 只退 `typed` 的一個字元 → 混打刪不掉已轉成詞的片段。

測試 helper 用 `SessionConfig::new(Mode::Zh)`，實機是 `Mixed`，所以測試全綠、實機全壞。

## 目標形狀

| 現在 | 之後 |
|---|---|
| `Mode::Zh` | 刪除；注音是 Mixed 的子集 |
| `Mode::Ja` | 刪除；羅馬字在 Mixed 內自動走日文（現在的行為已經是這樣） |
| `Mode::En` | 從 core 刪除；TSF 在 `english == true` 時直接放行按鍵，不送引擎 |
| `Mode::Mixed` | core 唯一的模式 |

`SessionConfig.mode` 欄位保留（設定檔相容），但只接受 `mixed`；`zh`／`en`／`ja` 讀進來
一律映射成 `mixed`，並在 `settings.json` 載入時記一次 warning。

## 中英切換的語意（比照微軟注音）

切英文**不是重置組字**，只是「接下來打的鍵當哪種語言」：

- 打 `ㄨㄛˇ` 出現「我」→ Shift 切英文 → 組字區還是「我」。
- 接著打 `hello` → 同一段組字變成「我hello」。
- `←` 可以回到「我」，候選窗作用在游標所在的那一段。
- 再 Shift 切回中文，緩衝照樣保留。

G7 已移除 `Session::toggle_english()` 與 `Request::Toggle`；中英切換只由外殼
保存，引擎只回快照，不會 `clear_composition()`。

## 現況（2026-11-26）

- 中英模式完全由 TSF 外殼保存。`Session::toggle()` 只改本機狀態與 langbar，
  不再送 `{"op":"toggle"}`；Shift 本身不產生 IPC。
- 組字中切英文後，英文字母以 `literal` 接在中文組字後面，Enter 一起送出。
- 已知限制：`mode_memory=shared` 時，引擎仍有自己的模式記憶。切換視窗或焦點後，
  引擎回報的中文模式可能把外殼剛切好的英文模式蓋回去。Windows 預設的 native
  模式記憶不會走到這條路徑。
- 要修這個限制需要一個輕量的模式同步 op；但單按 Shift 的切換不能依賴引擎連線
  成功，所以不能在 Shift 路徑直接補送請求。

## 分階段

1. **測試先跑 Mixed**：把 `jc-core` 測試 helper 的 `Mode::Zh` 全部改成 `Mixed`，
   修到全綠。這一步不改產品行為，只暴露「只有 Zh 有」的路徑。
2. **`Mode::En` 移到 TSF**：`english` 時 `wants_key` 直接回 false（除了 toggle 與
   caps lock 相關鍵），引擎不再收到英文鍵。core 的 `Mode::En` 分支刪除。
3. **`Mode::Ja` 併進 Mixed**：`ja_run` 已經是 Mixed 的一部分；手動切日文的入口
   （langbar／設定）改成「Mixed 但只走羅馬字」，或直接移除。
4. **`Mode::Zh` 刪除**：把 `if mode == Zh` 的分支改成無條件（Mixed 也走），
   刪掉 `Mode::Zh` 變體與所有 match 臂。
5. **設定相容**：舊 `settings.json` 的 `mode` 值遷移；`zh`／`ja`／`en` 都映射到
   `mixed`，`en` 另外記成「開機直接英文」的旗標（TSF 讀 `start_mode`）。

## 風險

- `Mode::En` 已從 core 移除；TSF 自己記中英模式，引擎沒有 toggle 請求，
  重播邏輯也不再需要 toggle 特例。
- 日文 `F6`／`F8` 等變換鍵目前綁在 `Mode::Ja`；併進 Mixed 後要確認不會跟中文
  候選鍵衝突。
- 這是一整批行為變更，必須每個階段都能單獨發版回滾。

## 驗收

每個階段都要：

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

外加一組「實機腳本」人工驗：注音選字、混打英數、羅馬字日文、左右鍵移游標、
Backspace 刪片段、↓ 開候選、Shift 切換。這七項在 `docs/port/mixed-segment-editing.md`
與 CHANGELOG 都出現過，之後每次發版都照這七項跑一遍。
