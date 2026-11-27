# keybinds：快捷鍵由引擎解析

## 目標

符號選單與表情選單不再由 TSF 寫死按鍵。TSF 只把「帶修飾鍵、且命中綁定」的
按鍵送給引擎；引擎依 `settings.json` 的 `[keybinds]` 解析成 action，再呼叫
`Session::command`。設定檔改了不必重啟引擎，也不必重編 TSF。

## chord 語法

```text
chord = modifier ("+" modifier)* "+" key
modifier = "ctrl" | "alt" | "shift"
key = 單一字元
    | "f1" .. "f12"
    | "enter" | "escape" | "backspace" | "tab" | "space" | "delete"
    | "home" | "end" | "pageup" | "pagedown" | "up" | "down" | "left" | "right"
```

- 修飾鍵大小寫不敏感，順序不拘；正規化輸出固定依 `ctrl`、`alt`、`shift` 排序。
- **必須有 Ctrl 或 Alt**。純字母或數字會偷走一般打字（`ChordError::NoModifier`）；
  只有 Shift 的 `shift+a` 會吃掉所有大寫字母，回 `ChordError::ShiftOnly`。
- 最後一段必須是單一字元、`f1`..`f12` 或上面列出的具名鍵；其他回 `UnknownKey`。
- 兩個 action 不能綁同一個 chord，回 `DuplicateChord`。
- 空字串回 `Empty`。

範例：`ctrl+alt+,`、`Ctrl+Shift+S`（正規化為 `ctrl+shift+s`）、`alt+f12`。

## 預設綁定

| action | 預設 chord | 行為 |
|---|---|---|
| `symbol_menu` | `ctrl+alt+,` | 開符號選單（沒組字時也可以開） |
| `emoji_palette` | `ctrl+alt+.` | 開表情選單 |

設定檔形狀：

```json
{
  "keybinds": {
    "symbol_menu": "ctrl+alt+,",
    "emoji_palette": "ctrl+alt+."
  }
}
```

缺 `keybinds` 或缺少其中一個欄位時用預設值。啟動時整份 `keybinds` 解析失敗
（合法 JSON 但 chord 不合法）會沿用預設綁定並記一則 warning，引擎仍可啟動；
熱重載時同樣的錯誤會**保留目前生效的舊綁定**、設定 `settings_error`、revision
不變，不會把使用者的自訂綁定默默清掉。設定檔不是合法 JSON 時也走保留舊設定
這條路。

## 怎麼在設定程式改

設定程式（`jc-settings-ui`）的「輸入」分頁 →「鍵位與模式」群組有兩個欄位：

- **符號選單快捷鍵**：預設 `ctrl+alt+,`
- **表情選單快捷鍵**：預設 `ctrl+alt+.`

畫面由上往下是欄位標題、輸入框、「錄製」與「重設為預設」按鈕、chord 語法說明。
說明固定在欄位下方，內容如下：

```text
格式：ctrl+alt+,（表情欄位顯示 ctrl+alt+.）
可用修飾鍵：ctrl、alt、shift；至少要有一個 ctrl 或 alt
按鍵：單一字元（字母、數字、標點，例如 , . ; /）、f1–f12，或具名鍵
      enter／escape／backspace／space／tab／delete／home／end／
      pageup／pagedown／up／down／left／right
```

設定檔不存在、內容是 `{}`、缺少 `keybinds`，或 `keybinds` 成員是空字串時，
欄位仍會顯示預設 chord，不會空白。直接改輸入框，按 Enter 或移開焦點就存檔；
不需要另外按儲存。Chord 不合法時，例如只打 `a` 或 `shift+a`，中文錯誤會顯示
在該欄位下方，原設定檔保持不動。按「重設為預設」會把該欄位寫回上表的預設
chord，同樣立刻存檔。

### 錄製快捷鍵

不想手打 chord 時，按欄位旁的「錄製」：

1. 欄位換成「請按下要使用的組合鍵…」，按鈕變成「取消錄製」。
2. 按下含 Ctrl 或 Alt 的組合鍵，例如 `Ctrl+Alt+,`。欄位立即顯示正規化後的
   `ctrl+alt+,`，並走與手動編輯相同的 `apply`／存檔／熱重載路徑。
3. 按 `Esc` 取消錄製，原值保留，也不會寫檔。
4. 再按一次「取消錄製」同樣取消。
5. 錄製中切到別的分頁會自動取消錄製，不會留下看不到卻仍在等待的狀態。

錄製期間按鍵由設定程式吃掉，不會送進輸入框或其他控制項，也不會觸發應用程式
的快捷鍵。攔截綁在程式層級，不依賴目前焦點；`Tab` 也只會得到「需要 Ctrl 或
Alt」的錯誤並繼續等，不會把焦點帶走。只按 Shift、Ctrl、平台鍵或 fn 本身不算
組合鍵，會繼續等。

不合法或不能用的組合會顯示中文錯誤，並維持錄製狀態繼續等下一個按鍵：

- 純字母、數字或標點，例如 `a`：`快捷鍵需要 Ctrl 或 Alt（不能是純按鍵）`。
- 只有 Shift，例如 `Shift+A`：`快捷鍵不能只用 Shift；請加上 Ctrl 或 Alt`。
- 平台鍵（macOS 的 Command、Windows 的 Win）或 fn 加字母：視為缺少 Ctrl／Alt。
- 與另一個動作重複：`同一個快捷鍵 … 綁定了多個動作`。
- 不認得的按鍵：`不認得的按鍵 …`；只有 `f1`..`f12` 與文件列出的具名鍵可用。

錄製只負責產生 chord 字串，存檔前的驗證仍由 `Settings::set` 與
`Keybinds::chords` 負責，所以手動編輯與錄製不會出現兩套規則。

存檔後走既有熱重載：引擎背景每 1 秒檢查設定檔，成功套用後 revision +1；TSF
按鍵路徑最多每 5 秒輪詢一次 `Health`，或在下次取得焦點時換表。因此引擎端
約 1 秒生效，TSF 端最多 5 秒或下次取得焦點。

截圖位置描述：視窗標題「0c 設定」，左側選「輸入」，右側第一組「鍵位與模式」
的第二、第三列就是兩個快捷鍵欄位；欄位下方的灰字是語法說明，錯誤訊息以
紅色顯示在同一欄位下方。

## 協定

- `Request::Key` 的 `Key` 新增 `alt` 欄位；`false` 不序列化，舊客戶端形狀不變。
- `Response::Ready` 與 `HealthInfo` 新增 `bindings`：action → 正規化 chord。
- `HealthInfo` 新增 `settings_revision`（每次成功套用 +1）與
  `settings_error`（最近一次失敗原因）。

舊引擎沒有這些欄位時，TSF 視為空表，快捷鍵不攔截，其他按鍵照舊。

## 引擎解析

`crates/jc-engine/src/lib.rs`：

1. `Request::Key` 進來且 `key.ctrl || key.alt || key.shift` 為真時，先用
   `config_watch::chord_of_key` 轉成正規化 chord，再查
   `Registry::settings_keymap`。
2. 命中 → `Session::command(action)`（`crates/jc-core/src/session/command.rs`），
   回 `handled = Some(true)`。
3. 未命中 → 照原本流程（Ctrl＋標點、Ctrl+Space、Shift 切換等），不搶按鍵。
4. 只有 `alt` 的組合若沒命中，回 `handled = Some(false)`，讓外殼明確放行。

`handled` 的語意是「這一鍵被輸入法吃掉」，不是「還有組字」。組字中或候選窗／
選單開著時按 Backspace／Delete，即使把組字或候選刪光，那一鍵仍回
`handled = Some(true)`；`preedit` 與候選都空時才維持 `None`，讓外殼放行給
應用程式。引擎的「當時是否組字」與 TSF 的 `Snapshot::composing()` 用同一個
謂詞：`!preedit.is_empty() || !candidates.is_empty()`。

## TSF 轉發

`crates/jc-tsf/src/service.rs`、`session.rs`：

- `Ready` 拿到 bindings；`OnSetFocus`／`OnSetThreadFocus` 各刷新一次。
  `OnTestKeyDown` 與 `OnKeyDown` 的按鍵路徑走同一條 5 秒節流，最多每 5 秒用
  一次 `Health` 輪詢；只有 `settings_revision` 變了才換表。連不上時沿用上一次
  的表。
- `wants_key` 先用本地表比對 chord，**命中才回 true**；未命中的按鍵不會把
  `Key` 送給引擎，只可能觸發上述最多每 5 秒一次的 `Health` 輪詢。這個判斷放在
  `modifier_down()` 之前，避免 `Ctrl+Alt+,`／`Ctrl+Alt+.` 被既有的修飾鍵提前
  擋掉。
- `OnTestKeyDown` 的 chord 判斷早於 `commit_before_host_key` 與
  `english_key_policy`：組字中不會先提交，英文模式的補全政策也不會吃掉快捷鍵。
- 命中後送 `Key`（帶 `ctrl`／`alt`／`shift`）；引擎回 `handled = false` 或連不上
  就放行給應用程式。
- 沒有組字時也送，符號選單就是要在空白狀態開啟。

`=` 仍是字面字元；舊 `emoji_shortcut = equals` 的行為不會復活。

## 選單渲染與鍵盤

引擎開選單後，`State.menu` 帶種類、標題、分頁名稱與目前分頁，`candidates`
仍是目前分頁的項目。TSF 的顯示條件改成：

- `menu` 有值時一律顯示候選窗，不受 `navigating` 影響；
- 沒有選單時維持原本的「`composing()` 且 `navigating`」。

候選窗最上方依序畫標題與分頁列，目前分頁用反白標示；沒有選單時不佔高度，
外觀與原本相同。分頁列不參與滑鼠命中。

選單開著時的鍵盤：

| 鍵 | 行為 |
|---|---|
| `Tab`／`Shift+Tab` | 送引擎切換分頁；Shift+Tab 走 `BackTab` → `KeyCode::ShiftTab` |
| `↑`／`↓` | 在目前分頁內移動高亮 |
| `1`–`0` | 選目前欄的列 |
| `Enter` | 插入高亮項目並關閉選單 |
| `Esc` | 只關選單，不動組字 |

分頁切換由 core 的 `menu_key` 處理，TSF 只負責送鍵。英文模式開著選單時，
一般的英文直通政策會先讓位給選單路由，避免數字與 `Tab` 被當成補全操作。

鍵盤與版面的單元測試在 Linux 可跑；候選窗實際繪製、分頁列的外觀與焦點行為
仍只能在 Windows 實機確認。

### chord 診斷

0.1.29 盤點時這裡還是「已知缺口：選單沒有渲染」；menu-ui 已補上 `menu`
狀態、分頁列與 `wants_candidate_window()`。以下 chord 診斷 log 保留，用來分辨
「快捷鍵沒送到引擎」與「送到但選單狀態沒帶回 TSF」：

```text
bound_chord vk=... chord=ctrl+alt+, binding=Some("symbol_menu")
OnKeyDown 結束 vk=... key=... 綁定 chord=ctrl+alt+, eaten=true
handle_chord 引擎回 ok=true handled=true composing=true ...
handle_chord snapshot menu=true navigating=false candidates=... title=""
```

判讀：

- 沒有 `bound_chord`：按鍵沒進 TSF 快捷鍵路徑。
- `binding=None`：TSF 的 bindings 表沒收到 `symbol_menu`。
- 有 `handled=true` 且 `menu=true`：引擎與協定都正常，選單交給候選窗畫。
- `menu=false` 且 `navigating=false`：候選窗不會顯示，要查引擎回傳的 `State.menu`。

## 熱重載

`crates/jc-engine/src/config_watch.rs`：

- 背景執行緒每 1 秒 `stat` 設定檔（mtime + size），變了才重讀。
- 重讀流程：`load` → `effective` → 更新共用 `Registry` 的 keymap、bindings、
  effective 設定與 `settings_revision`；各 worker 在 `Engine::handle` 開頭用
  `sync_settings()` 把新設定套到自己的 session。
- 套用走 `Session::apply_config`，只換設定與衍生狀態，**不清組字**。
- 成功才 `settings_revision + 1`；讀檔失敗、JSON 壞掉或 chord 不合法時保留舊
  設定、記 warning、設定 `settings_error`，revision 不變。
- 第一次 `stat` 的基準是 `config::load` 當下的戳記，不是資料載入完成後的
  `stat`；啟動載入期間改的檔案也會在 watcher 起跑後補上。
- 沒有 TSF 連線時也照常重載（引擎常駐）。
- `Request::ApplySettings` 走同一條重載路徑，立即生效並回新的 `HealthInfo`。

設定程式選擇「存檔讓熱重載接手」，不另外送 `ApplySettings`。

## 新增一個 action

1. `crates/jc-core/src/session/command.rs`：在 `Session::command` 加 action 分支，
   回傳是否有開啟。
2. `crates/jc-settings/src/keybinds.rs`：`Keybinds` 加欄位、預設值與 `bindings()`
   輸出；必要時調整 `set` 的 action 對照。
3. `crates/jc-settings/src/settings.rs`：加 `keybind-*` spec、序列化與
   `apply_set` 支援。
4. `crates/jc-engine/src/config_watch.rs`：`chord_of_key` 不需改；確認新 chord
   能被一般按鍵表示。
5. `crates/jc-tsf/src/keys.rs`：若新 chord 用到目前沒轉出的按鍵，補
   `chord_key_name`／`chord_key`。
6. 文件與測試：本檔、`docs/port/settings.md`、引擎與 TSF 的單元測試。

## 測試與手動驗證

- `crates/jc-settings/src/keybinds.rs`：chord 解析、缺修飾鍵、Shift-only、
  未知鍵、重複綁定、round-trip。
- `crates/jc-engine/src/lib.rs`：`ctrl+alt+,` 命中並真的開選單、未綁定回
  `handled=false`、組字中命中不提交、mtime 變更後 revision +1／keymap 換新／
  組字不清空、`ApplySettings` 立即生效、壞 JSON 與合法 JSON 壞 chord 都保留
  舊設定、啟動載入期間的變更會被 watcher 補上。
- `crates/jc-tsf/src/session.rs`：`Ready` 的 bindings 帶動本地比對、英文模式下
  仍命中、`Key` 帶 `alt`、本地查表不把按鍵送給引擎。
- `scripts/verify-keybinds.sh`：起引擎 → 送 `ctrl+alt+,` → 確認 `handled=true`
  且 state 有符號候選 → 改設定檔 → 1.5 秒後送舊鍵與新鍵 → 確認 revision 變、
  舊鍵放行、新鍵命中；再寫入合法 JSON 壞 chord，確認 revision 不變、舊 chord
  仍命中、`health` 有 `settings_error`。

```sh
scripts/verify-keybinds.sh
```
