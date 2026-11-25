# keybinds：快捷鍵由引擎解析

Worktree：`/mnt/work/jc/wt/feat-keymap`（branch `feat-keymap`）

## 目標

符號選單與表情選單不再由 TSF 寫死按鍵。TSF 只把「帶修飾鍵、且命中綁定」的
按鍵送給引擎；引擎依 `settings.json` 的 `[keybinds]` 解析成 action，再呼叫
`Session::command`。設定檔改了不必重啟引擎，也不必重編 TSF。

## chord 語法

```text
chord = modifier ("+" modifier)* "+" key
modifier = "ctrl" | "alt" | "shift"
key = 單一字元 | "f1" .. "f12"
```

- 修飾鍵大小寫不敏感，順序不拘；正規化輸出固定依 `ctrl`、`alt`、`shift` 排序。
- **至少要有一個修飾鍵**。純字母或數字會偷走一般打字，直接拒絕
  （`ChordError::NoModifier`）。
- 最後一段必須是單一字元或 `f1`..`f12`，其餘回 `UnknownKey`。
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

缺 `keybinds` 或缺少其中一個欄位時用預設值。整份 `keybinds` 解析失敗時，引擎
沿用預設綁定並記一則 warning，不會因此起不來；`HealthInfo.settings_error` 會
帶最後一次重載的錯誤訊息。

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

## TSF 轉發

`crates/jc-tsf/src/service.rs`、`session.rs`：

- `Ready`／`OnSetFocus`／`OnTestKeyDown` 時向引擎拿一次 bindings；按鍵路徑最多
  每 5 秒用 `Health` 輪詢一次，只有 `settings_revision` 變了才換表。連不上時
  沿用上一次的表。
- `wants_key` 先用本地表比對 chord，**命中才回 true**；未命中的按鍵不會產生
  任何 IPC。這個判斷放在 `modifier_down()` 之前，避免 `Ctrl+Alt+,`／
  `Ctrl+Alt+.` 被既有的修飾鍵提前擋掉。
- 命中後送 `Key`（帶 `ctrl`／`alt`／`shift`）；引擎回 `handled = false` 或連不上
  就放行給應用程式。
- 沒有組字時也送，符號選單就是要在空白狀態開啟。

`=` 仍是字面字元；舊 `emoji_shortcut = equals` 的行為不會復活。

## 熱重載

`crates/jc-engine/src/config_watch.rs`：

- 背景執行緒每 1 秒 `stat` 設定檔（mtime + size），變了才重讀。
- 重讀流程：`load` → `effective` → 更新共用 `Registry` 的 keymap、bindings、
  effective 設定與 `settings_revision`；各 worker 在 `Engine::handle` 開頭用
  `sync_settings()` 把新設定套到自己的 session。
- 套用走 `Session::apply_config`，只換設定與衍生狀態，**不清組字**。
- 成功才 `settings_revision + 1`；失敗保留舊設定、記 warning，revision 不變。
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
6. 文件與測試：本檔、`docs/port/settings-wire.md`、引擎與 TSF 的單元測試。

## 測試與手動驗證

- `crates/jc-settings/src/keybinds.rs`：chord 解析、缺修飾鍵、未知鍵、重複綁定、
  round-trip。
- `crates/jc-engine/src/lib.rs`：`ctrl+alt+,` 命中並真的開選單、未綁定回
  `handled=false`、mtime 變更後 revision +1／keymap 換新／組字不清空、
  `ApplySettings` 立即生效、壞檔保留舊設定。
- `crates/jc-tsf/src/session.rs`：`Ready` 的 bindings 帶動本地比對、
  `Key` 帶 `alt`、本地查表不產生 IPC。
- `scripts/verify-keybinds.sh`：起引擎 → 送 `ctrl+alt+,` → 確認 `handled=true`
  且 state 有符號候選 → 改設定檔 → 1.5 秒後送舊鍵與新鍵 → 確認 revision 變、
  舊鍵放行、新鍵命中。

```sh
scripts/verify-keybinds.sh
```
