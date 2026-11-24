# Mixed 模式的片段編輯：刪除、游標、候選與日文推斷邊界

Branch：`fix-mixed-editing`。Owned paths 只有 `crates/jc-core/src/session/**` 與本文件；
`crates/jc-tsf`、`crates/jc-engine` 沒有改動。

## 症狀與根因

| 症狀 | 根因 |
|---|---|
| 打 `ㄊㄞˊ` 出現「台」後按 Backspace 沒反應 | `AsciiInput::backspace()` 只從 `typed` 退掉一個按鍵，顯示管線再用剩下的 `w9` 重算出「台」；`台` 是完成音節，`typed` 裡對應的是整段 `w96`，不是最後一個字元。 |
| `watashiwaw96j0` 停在「わたしわw」 | 混打推斷出日文片段後 `Session` 暫時切到 `Mode::Ja`；數字鍵被 `RomajiInput` 拒絕卻直接丟掉，沒有回到混打。 |
| 左右鍵移不到英文片段後面 | `navigate()` 只有 `Mode::Zh` 的游標分支，Mixed 直接落到候選換項。 |
| Mixed 沒有候選、↓ 沒反應 | 候選只在 `ascii.pending()` 非空時由 `rebuild_mixed_candidates()` 產生；注音音節完成後 pending 是空的，`candidates` 也就空了。 |

## 片段邊界怎麼判定

Mixed 的顯示、送出與片段都以同一條管線為準：`crate::mixed::plan_input()` 對整段
按鍵流（`AsciiInput::typed()`）切分，再由 `Session::apply_latin_split()` 寫進
`base`、`segments` 與 `Composition`。

- 不是「先遇到英文字母就開英文片段」。英文詞、英文前綴、注音音節、日文羅馬字與
  數字／標點的邊界，全部由 `plan_input()` 的 tokenize（R1～R3）、DP（R4）與
  後處理（R5）決定。
- 一個 `Segment` 的範圍就是該 plan 在**輸出字面**上的範圍；同一個 plan 在
  **按鍵流**上的範圍由 `Session::mixed_typed_spans()` 用同一份 plan 重算，刪除時
  兩邊靠索引對齊。
- 英文片段與注音片段相鄰時，各自是一個 plan；中間的空白、全形空白與標點由
  管線決定要不要獨立成片段，UI 不自行猜邊界。
- 尾巴連續的注音 plan 才是「目前的中文片段」，中文候選不會跨過英文或標點去做
  整句猜測。

## 未完成的注音不轉換

沒有聲調鍵、也還沒按空白的注音音節，一律顯示原始注音，不轉成字。這條規則在
純注音路徑與混打路徑都成立，尾巴與中間的音節都一樣；只有下列情況才轉換：

- 打了聲調鍵（`3`／`4`／`6`／`7`）
- 按了空白（空白是一聲；沒有未完成音節時才是字面空格）
- 使用者從候選窗明確選字

實例（真實詞典，`ㄍㄨ` 有「家」、`ㄍㄨㄥ` 有「供」）：

| 按鍵 | 顯示 | 說明 |
|---|---|---|
| `saej` | `さえj` 或 `さㄍㄨ` | `j` 之後停在原始按鍵／注音，沒有漢字 |
| `saej/` | `さㄍㄨㄥ` | `/` 只是韻母鍵，`ㄍㄨㄥ` 還沒完成 |
| `saej/ ` | `さ供` | 空白是一聲，這時才轉成詞典的字 |
| `w96j0` | `台ㄨㄢ` | 前段 `ㄊㄞˊ` 已完成照常轉字，尾巴 `ㄨㄢ` 留注音 |
| `j0w96` | `ㄨㄢ台` | 中間的 `ㄨㄢ` 未完成，一樣不轉字 |

實作上 `crate::mixed::Plan` 為每個注音音節記錄 `zh_done`：切分時看到聲調鍵或
空白一聲就標成完成。顯示與送出都只把連續完成的音節交給詞典轉換，未完成的音節
直接輸出讀音；候選清單仍照常產生，不受影響。

## 刪除規則

| 按鍵 | 行為 | 理由 |
|---|---|---|
| Backspace | 刪游標前**一整個片段** | 使用者看到的是「台」或 `hello`，退回 `ㄊㄞ` 或 `hell` 會出現畫面上沒有的內容。Mixed 的顯示由按鍵流重算，退一個字元會讓同一個詞用比較短的按鍵重新解碼，看起來就是「刪不掉」。 |
| Delete | 刪游標後**一整個片段** | 與 Backspace 對稱；游標在結尾時不動作。 |
| 刪除後 | 游標停在被刪片段的位置；刪到尾巴時回到結尾 | 游標以片段邊界表示，刪掉最後一段後邊界就是新的結尾。 |

實作上先由 `mixed_typed_spans()` 找到該片段在 `typed` 的位元組範圍，整段移除後
用 `AsciiInput::replay()` 重播剩下的按鍵，讓 `pending`／`pieces` 與按鍵流一致。
重播規則與 `Session::press_char()` 相同：空白走 `space()`、聲調鍵先試 `tone()`、
其餘照 `press()`；非 ASCII 的固定字面直接放回片段。

## 游標規則

Mixed 沒有音節格，游標改用**片段邊界索引** `mixed_cursor`：

- `0..=segments.len()` 是合法位置；`Session::MIXED_CURSOR_END`（`usize::MAX`）
  代表整段結尾。
- 打字、空白、完成音節後一律回到結尾；←／→ 一次移動一個片段，英文片段也算
  一格。明確候選選取模式（↓／↑ 進入）中的左右鍵仍然換候選。
- `Snapshot.cursor_rune`／`cursor_utf16` 由游標邊界對應到 `preedit` 的 rune／
  UTF-16 位移；游標在整段結尾時就是字串長度。
- 移動游標只改插入／刪除位置，不改片段內容與候選字面。

## 候選

注音音節完成、`ascii.pending()` 為空時，`rebuild_mixed_zh_candidates()` 取尾巴
連續的注音 piece，用與注音模式相同的 `Lattice`＋`Decoder::nbest()` 產生候選。

- `↓` 第一次只開窗並進入選取模式（`candidate_window = true`、
  `navigating = true`），與注音模式一致。
- 空白在 Mixed 遇到已完成的注音片段時，先把殘留注音補完，再把空白留在按鍵流
  並開候選窗；第二次空白或 Enter 送出。
- 數字選字只在 `navigating == true` 時生效。空白開窗後直接打數字仍照一般按鍵
  處理，避免把下一個注音／英數吃掉；Tab 在 Mixed 會明確進入選取模式。
- 選中文候選時只替換結尾的注音片段：`select_mixed_zh_candidate()` 把該段在
  `typed` 的範圍換成候選字面再重播，前面已固定的英文、日文片段不受影響。
- 開候選窗用的尾端空白不算內容，送出／選字前由
  `trim_mixed_candidate_space()` 拿掉。

## 日文推斷的結束條件

`try_japanese_run()` 會在純羅馬字可完整轉かな時把工作階段暫時切到 `Mode::Ja`。
這個推斷不是永久狀態：

- `RomajiInput::press()` 回 `Rejected`（數字、標點等接不上的按鍵）且目前是
  推斷出來的日文片段時，`leave_ja_run_for_mixed()` 會結束推斷。
- 已顯示的日文（`base`，沒有時用讀音）先變成按鍵流裡的固定字面前綴，尚未完成
  的羅馬字前綴（例如 `w`）放回 `AsciiInput` 的待定按鍵，再把觸發鍵交給混打。
- 因此 `watashiwaw96j0` 會變成 `わたしわ` ＋ `ㄊㄞˊㄨㄢ`，後續空白與 `bp6`
  仍可繼續輸入；不會停在未完成的 `w`。
- 直接以 `Mode::Ja` 啟動、或使用者手動切換的日文不受影響，只有 `ja_run` 推斷
  出來的片段有這條退回規則。

## 測試

新增在 `crates/jc-core/src/session/mixed.rs` 的測試模組：

| 測試 | 內容 |
|---|---|
| `test_mixed_backspace_deletes_converted_syllable` | `w96` → `台`，Backspace 後 preedit 清空、游標回 0。 |
| `test_mixed_backspace_deletes_whole_english_segment` | `ji3hello` → Backspace 只留 `我`，不是 `我hell`。 |
| `test_mixed_ja_run_falls_back_on_non_romaji_key` | `watashiwaw96j0` 不停在 `w`，有 `ㄊㄞ`，再打 `bp6` 仍能繼續。 |
| `test_mixed_arrows_move_between_segments` | `我hello` 的 ←／→ 在 0、1、6 之間移動，英文片段也停得住。 |
| `test_mixed_delete_forward_removes_segment_after_cursor` | 游標在英文片段前按 Delete 只刪該段。 |
| `test_mixed_candidates_open_on_down_and_space` | `w96` 後 `candidates` 非空；↓ 開窗並 `navigating`，空白開窗不插空白。 |
| `test_mixed_selects_zh_candidate_in_place` | ↓ 後用數字選「颱」，只換結尾注音片段。 |
| `test_mixed_unfinished_zhuyin_stays_bopomofo` | `saej`、`saej/` 停在原始注音；空白後才是「さ供」。 |
| `test_mixed_unfinished_zhuyin_with_ja_decoder` | 載入日文解碼器時 `j` 停在 `さえj`，`/` 後仍是 `さㄍㄨㄥ`。 |
| `test_mixed_unfinished_zhuyin_in_middle_stays_bopomofo` | `w96j0` 是 `台ㄨㄢ`、`j0w96` 是 `ㄨㄢ台`，中間與尾巴都不提早轉字。 |

既有測試沒有放寬；`test_incremental_matches_router` 仍要求工作階段與
`Router::convert` 對同一串按鍵逐段一致。

這 7 條測試先在 `4f10d27` 上跑過，全部失敗（Backspace 留下「台」、英文只退到
`我hell`、日文停在 `Mode::Ja`、游標停在 6、沒有候選），修完才轉綠。

## 驗證

本輪在 Linux（GB10）執行：

- `mise exec -- cargo test -p jc-core`：lib 743 passed，integration 1＋97 passed。
- `mise exec -- cargo test --workspace`：全部通過。
- `mise exec -- cargo clippy --workspace --all-targets -- -D warnings`：通過。
- `mise exec -- cargo fmt --all -- --check`：通過。

實機（Windows 安裝版 0.1.13，預設 `Mode::Mixed`）**尚未驗證**：本工作環境是
Linux，沒有 Windows TSF 可測。需要手動確認：

1. 打 `ㄊㄞˊ` →「台」→ Backspace 刪得掉。
2. 打 `watashiwaw96j0 bp6` 不會停在 `w`，能繼續輸入。
3. 組字中 ←／→ 能移過英文片段。
4. 打 `ㄊㄞˊ` 後按 ↓，外殼出現候選窗；空白也能開窗。
