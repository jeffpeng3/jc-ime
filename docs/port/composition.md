# 組字模型、游標與片段編輯

`crates/jc-core/src/composition.rs` 的 `Composition` 是組字的權威來源：文字、游標、
片段與所有權都在同一份模型，語言只決定讀音與轉換後端。

## 資料模型

| 型別／介面 | 內容 |
|---|---|
| `CompSegment { span, reading, units, lang, owner }` | `span` 是輸出字串的 rune 半開區間 |
| `Owner::Auto` | 輸入或系統轉換結果；每次轉換都可以改寫 |
| `Owner::User` | 使用者選字、`j`／`k`、F6–F10、拆回讀音、滑鼠鎖定；原樣保留 |
| `Session::composition()` | 目前組字模型 |
| `Session::auto_runs()` | 會被送去轉換的連續 `Auto` 區段 |
| `Session::convert_auto_runs()` | 逐段呼叫注音／日文後端 |
| `Snapshot.owners` | 與 `segments` 對齊的所有權，供外殼標示 |

`User` 以同語言讀音單位保存，不是顯示字串位移。轉換後由
`Composition::replace_segments()` 依讀音套回：讀音相同保留 `User`（顯示可更新）；
讀音改變（片段內刪字、改音）退回 `Auto`；在片段前插入則平移標記。

`auto_runs()` 把相鄰、同語言、有讀音單位的 `Auto` 片段連成一段；`User`、字面
文字與語言切換會切開。每段帶 `ConvertContext { left, right }`，只轉換 `Auto`
區段，前後文脈供後端使用。

注音的 `Session::locks` 是推導結果：`sync_zh_locks` 把 `User` 標記換成
`decode::Lock`。日文 `ja::live::Controller` 的 `user_segments` 也是快取，所有權
由 `Composition` 推送。

## 游標

- `Session::cursor` 是音節格位置，對外換算成 rune 位移；`Snapshot.cursor_rune` 與
  `cursor_utf16` 描述同一個 rune 邊界。
- 游標在片段結尾屬於下一個片段；在整段結尾算最後一段。
- 暫存注音非空時不移動游標；完成的音節插在游標位置，游標移到該音節後。
- 游標移到中間後，候選只算游標所在片段；選字只鎖定該段，其他片段由整段解碼
  重算。游標回到結尾恢復整段候選。
- 對外游標一律用 rune；內部不得用位元組位移切 `base`。

## 按鍵

| 按鍵 | 行為 |
|---|---|
| ←／→ | 候選窗關閉時移動游標；已在頭尾時換候選。候選窗開著時屬候選介面 |
| Home／End | 候選窗關閉時移到組字頭／尾；選取模式中移到候選第一／最後一項 |
| Backspace | 先關候選窗，再刪游標左邊一個可見 rune |
| Delete | 先關候選窗，再刪游標右邊一個可見 rune；游標在結尾時不動作 |
| Shift＋←／→ | 縮短／加長游標片段的讀音長度，重新解碼候選 |
| ↓／↑ | 進入選取模式與換候選；第一下只進入選取模式 |
| `j`／`k` | 選取模式中換候選，只套用到游標片段 |
| Enter | 游標片段編輯中先把候選套到該段；其餘情況送出整段組字 |
| Esc | 先關候選窗並離開片段編輯，保留組字；再按才清暫存或整段 |

`keep_candidates` 在片段換字成功後保留候選窗與選取模式，Esc 或游標離開片段才恢復
一般顯示。

## Mixed 模式

Mixed 的邊界由 `crate::mixed::plan_input()` 對整段按鍵流切分，再由
`Session::apply_latin_split()` 寫進 `base`、`segments` 與 `Composition`。英文詞、
英文前綴、注音音節、日文羅馬字與數字／標點的邊界都由同一條 tokenize／DP／後處理
決定，UI 不自行猜。

- 顯示字串上的游標是 `mixed_cursor`（rune 位移）；`MIXED_CURSOR_END` 代表結尾。
- 未完成的注音不轉字：沒有聲調鍵也還沒按空白時顯示原始注音。打了聲調、按空白
  （一聲）或明確選字才轉換。
- 刪除逐 rune：找出游標右側那一個字元在按鍵流的範圍，移除後 `replay()`；游標
  留在原位。候選窗開著時先關窗。
- 候選作用在游標右側那一個字元（caret 模型）。只有 ↓／↑／PageDown／PageUp 開窗；
  空白只插空格。游標在結尾按 ↓ 時先左移一格，候選最後那個字元。
- 選了原始按鍵、注音字面或かな分解後，游標停在該字面尾端，字面維持可拆回的單位；
  再開候選仍還原完整清單。
- 日文推斷（`ja_run`）遇到羅馬字接不上的按鍵時，已顯示的日文變成固定字面，未完成
  的羅馬字放回 `AsciiInput`，觸發鍵交回混打；不會停在半截 `w`。
- 長英文 run 超過詞表查詢窗口（48 字元）後仍照字面續接，不會從中間被重判成注音
  或かな。

## 程式位置

| 檔案 | 內容 |
|---|---|
| `crates/jc-core/src/composition.rs` | `Composition`、`CompSegment`、`Owner`、`AutoRun`、`ConvertBackend` |
| `crates/jc-core/src/session/composition.rs` | Session 與模型的橋接與寫回 |
| `crates/jc-core/src/session/backends.rs` | 注音／日文後端與 `convert_auto_runs` |
| `crates/jc-core/src/session/core.rs` | 游標、片段、候選套用 |
| `crates/jc-core/src/session/mixed.rs` | Mixed 的刪除、游標與候選 |
| `crates/jc-core/src/mixed.rs`、`mixed_latin.rs` | 切分與拉丁字串 |
