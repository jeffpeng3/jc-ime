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
| Backspace | 刪游標左邊**一個 rune** | 候選與游標都以字元為單位；英文 `hello` 可以停在 `hell`，注音字與かな也一樣。 |
| Delete | 刪游標右邊**一個 rune** | 與 Backspace 對稱；游標在結尾時不動作。 |
| 刪除後 | 游標留在原位 | 注音片段會用剩下的按鍵重新解碼；片段鎖定與聲調資訊保留在 `syllables`。 |
| 候選窗開著時刪除 | 先關窗，再照上面的規則刪 | 不讓下一個數字被選字路徑吃掉。 |

實作上先由 `mixed_target_at_cursor()` 找到游標右側那一個字元在 `typed` 的位元組
範圍，只移除那個範圍再 `AsciiInput::replay()`；字面片段與注音、かな走同一條路。
重播規則與 `Session::press_char()` 相同：空白走 `space()`、聲調鍵先試 `tone()`、
其餘照 `press()`；非 ASCII 的固定字面直接放回片段。

## 游標規則

Mixed 的游標是**顯示字串上的 rune 位移** `mixed_cursor`：

- 游標停在 `preedit` 的 rune 邊界，跨越片段邊界時直接進到相鄰片段的第一或
  最後一個 rune；英文 `hello` 可以停在 `hel|lo`。
- 打字、空白、完成音節後一律回到結尾；`Session::MIXED_CURSOR_END`（`usize::MAX`）
  代表整段結尾。
- ←／→ 一次移動一個 rune。候選窗（↓／↑ 開啟）開著時，四方向鍵屬於候選介面，
  游標不動；`Esc` 關窗後才恢復移動。
- Home／End 在候選窗沒開時直接移到組字頭尾；選取模式中則是候選清單的第一／
  最後一項。
- `Snapshot.cursor_rune`／`cursor_utf16` 由 `mixed_cursor` 換算成 rune／UTF-16
  位移；游標在整段結尾時就是字串長度。
- 注音還沒打完時移動游標：待定音節退化成原始字母，交給混打管線重算，不擋住
  游標。

## 候選

候選永遠作用在**游標右側的那一個字元**（caret 模型）：

- 只有 `↓`（或 `↑`／PageDown／PageUp）會開候選窗並進入選取模式。空白鍵只插入
  一個字面空格，不開窗、不選第一個；舊的 `space_selects` 讀到也忽略。
- 結尾是特例：游標在整段結尾按 `↓` 時先往左移一格，再開最後那個字元的候選。
  因此 `我わたしhello|` 的候選是 `o`，不是整個 `hello`。
- 版面直的、一頁 10 個。`Enter` 套用目前高亮並關窗；第一次按 `→` 展開成 4 欄，
  之後 `↑`／`↓`／`←`／`→` 都在候選介面裡移動。`Esc` 關窗，游標恢復移動。
- 選完後游標往右一格；比一個 rune 長的替換（例如「私」→「わたし」）游標放在
  插入內容的第一個字元之後。
- 數字選字只在 `navigating == true` 時生效；超出候選數時吃掉按鍵、不改狀態。
- 滑鼠點選候選與 `Enter` 相同：套用並關窗。
- 候選不往左組詞；`我hello` 在 `o` 上不會列出整串英文的拼字建議。整段都還是
  英文待定詞時，候選是整串的拼字建議與補全。

### 可拆回原始輸入

以 `ji3` 打得「我」為例，游標停在「我」前面按 `↓`，候選清單會有：

| 候選 | 來源 | 選了之後 |
|---|---|---|
| 我 | 詞典 | 保持「我」，游標往右一格 |
| ㄨㄛˇ | 注音本身 | 換成字面注音，游標在注音後面 |
| `ji3` | 使用者原始按鍵 | 換成字面 `ji3`，游標留在原位 |
| `ji` | 去掉聲調的按鍵 | 拆成 `ji` ＋ 剩下的 `3`，游標停在 `ji` 之後 |
| `じ3` | 同一串按鍵的日文解讀 | 換成 `じ3`，游標在 `じ` 之後 |

實作在 `mixed_candidates_for()`：詞典候選走片段選字，注音、原始按鍵、無聲調
按鍵與其他語言解讀都寫成 `MixedChoice`，套用時只重播那一個字元的按鍵範圍。
日文片段則接 kana→漢字 的轉換：漢字是預設，かな與原始羅馬字都是候選。


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

## 長串英文照字面續接

`mixed_latin` 的英文詞表查詢窗口是 `MAX_ENGLISH_CHARS = 48`。這個窗口只限制
「拿去查詞表或 autocomplete 的字尾長度」，不是英文片段的覆蓋上限。

- 一段已經走英文字面的 run 超過窗口後，DP 仍從 run 開頭保留一個英文字面
  種子，之後每個按鍵以 1 個字元續接同一個英文片段。片段成本沿用
  `en_word_cost`。
- 續接規則是「已字面化的英文 run 只能繼續字面」：後面再打小寫字母時，不會
  回頭把前面顯示過的字母重新解讀成假名或其他語言。`z` 這類打不成かな的字面
  run 會整串保留，`z` 連打 70 次再加 `a`、`b` 仍是 72 個英文字元。
- 日文仍然可以從 run 的開頭接手。產品預設開啟 ja_live 時，`a` 從一開始就
  可能走日文路徑（`aa` 顯示 `ああ`）；這不是長 run 的續接情境，也不受上面
  的英文字面鎖影響。
- 詞表與 autocomplete 還是只看最後 48 個字元；超過窗口的部分照按鍵字面顯示。
- 長 run 內不再讓日文或注音音節從中間接手。長 run 後面接標點時標點走
  symbol 片段，標點後的字母仍能繼續輸入。
- `Session` 的待定按鍵不再因為 `MAX_WORD_BYTES` 丟掉字母。上限只留給詞表
  查詢，輸入本身保留完整按鍵流。

測試在 `crates/jc-core/src/mixed_latin.rs`、`crates/jc-core/src/session/mixed.rs`
與 `crates/jc-engine/src/loader.rs`：

| 測試 | 內容 |
|---|---|
| `test_long_lowercase_run_stays_literal` | 70、200 個 `a` 在無日文解碼器時都是一個英文片段。 |
| `test_long_lowercase_run_continues_at_tail` | 70 個 `a` 後再打 `b`，增量 DP 不走失。 |
| `test_mixed_prefix_then_long_run_keeps_every_key` | `sa` 加 70 個 `a` 仍完整覆蓋。 |
| `test_mixed_long_literal_run_not_reinterpreted_as_japanese` | 有日文解碼器時，70 個 `z` 後打 `a`、`b` 不會變成假名。 |
| `test_mixed_long_lowercase_run_keeps_every_key` | Session 層長串 preedit 完整，按鍵不被丟掉。 |
| `test_mixed_long_lowercase_run_accepts_more_letters` | 長 run 後再打字母與標點會繼續變長。 |
| `mixed_session_keeps_long_literal_run` | 以 `load_engine_data` → `EngineData::session_config(Mode::Mixed)` 的 ja_live Session 驗證同一串按鍵與既有混打輸出。 |

## 測試

新增在 `crates/jc-core/src/session/mixed.rs` 的測試模組：

| 測試 | 內容 |
|---|---|
| `test_mixed_backspace_deletes_converted_syllable` | `w96` → `台`，Backspace 後 preedit 清空、游標回 0。 |
| `test_mixed_backspace_deletes_whole_english_segment` | `ji3hello` → Backspace 只留 `我hell`，逐 rune 退格。 |
| `test_mixed_ja_run_falls_back_on_non_romaji_key` | `watashiwaw96j0` 不停在 `w`，有 `ㄊㄞ`，再打 `bp6` 仍能繼續。 |
| `test_mixed_arrows_move_between_segments` | `我hello` 的 ← 從 6 逐格走到 0，→ 再逐格回結尾。 |
| `test_mixed_delete_forward_removes_segment_after_cursor` | 游標在 `hell|o` 按 Delete 只刪 `o`。 |
| `test_mixed_candidates_open_on_down_and_space` | `w96` 後 `candidates` 非空；↓ 開窗並 `navigating`，空白只插空格。 |
| `test_mixed_selects_zh_candidate_in_place` | ↓ 後用數字選「颱」，只換結尾注音片段。 |
| `test_mixed_unfinished_zhuyin_stays_bopomofo` | `saej`、`saej/` 停在原始注音；空白後才是「さ供」。 |
| `test_mixed_unfinished_zhuyin_with_ja_decoder` | 載入日文解碼器時 `j` 停在 `さえj`，`/` 後仍是 `さㄍㄨㄥ`。 |
| `test_mixed_unfinished_zhuyin_in_middle_stays_bopomofo` | `w96j0` 是 `台ㄨㄢ`、`j0w96` 是 `ㄨㄢ台`，中間與尾巴都不提早轉字。 |
| `test_down_lists_decomposition_for_zh_syllable` | `ji3` 的候選同時有「我」、`ㄨㄛˇ`、`ji3`。 |
| `test_select_original_keys_then_japanese_reading` | 選 `ji3` 留在原位；再把游標移到 `ji` 上，候選有 `じ3`。 |
| `test_down_at_end_targets_last_character` | `我hello` 結尾按 ↓ 先左移，候選是 `o`。 |
| `test_down_opens_japanese_kanji_and_kana_candidates` | `watashi` 預設「私」，候選有「わたし」。 |

既有測試沒有放寬。`test_incremental_matches_router` 仍要求工作階段與
`Router::convert` 對同一串按鍵逐段一致；「空白開候選／選第一個」與以片段為
單位的箭頭、刪除斷言，已依新規格改成逐字元期望。

## 驗證

本輪在 Linux（GB10）執行：

- `mise exec -- cargo test -p jc-core`：全部通過。
- `mise exec -- cargo test --workspace`：全部通過。
- `mise exec -- cargo clippy --workspace --all-targets -- -D warnings`：通過。
- `mise exec -- cargo fmt --all -- --check`：通過。

實機（Windows 安裝版 0.1.13，預設 `Mode::Mixed`）**尚未驗證**：本工作環境是
Linux，沒有 Windows TSF 可測。需要手動確認：

1. 打 `ㄊㄞˊ` →「台」→ Backspace 刪得掉。
2. 打 `watashiwaw96j0 bp6` 不會停在 `w`，能繼續輸入。
3. 組字中 ←／→ 能移過英文片段。
4. 打 `ㄊㄞˊ` 後按 ↓，外殼出現候選窗；空白也能開窗。

## 輕聲／一聲的延遲預算與字元邊界 panic

Branch：`fix-tone7-latency`。只改 `crates/jc-core`。

### 延遲預算

`gp6au42k7` 的 `7` 是輕聲 `ㄉㄜ˙`，同音字極多。`cargo test` 的 debug build 下，
`press()` 每鍵不得超過 **50ms**，`7` 最後要是「神秘的」。驗收用真實資料
`build-zh-bigram`；測試在資料缺席時跳過。

修正集中在三個地方：

| 位置 | 問題 | 修正 |
|---|---|---|
| `decode/nbest.rs` | 每個前詞狀態保留 `k` 條完整路徑，延伸時複製整條 `Vec<Seg>`，排序又用 `format!` 重建穩定鍵；沒有 bigram 時 `beam = 0` 完全不剪枝。 | 路徑改成共享前驅節點，延伸只新增一個節點；排序先比成本，再比預算好的指紋，只有需要與舊順序一致時才建完整穩定鍵。中間層每個前詞只留 2 條，最後一層留滿 `k` 條；`beam = 0` 時前詞狀態上限改成 `k`。 |
| `session/core.rs` | `fuzzy_candidates()` 對每個模糊音變體各跑一次完整 `nbest`，成本隨變體數倍增。 | 變體只取 `best()` 的最佳路徑，候選量交給主 `nbest`；判斷與既有模糊音測試不變。 |
| `lattice/lexicon.rs` | `InMemoryLexicon::prefix()` 每次掃全部詞條，候選補全在長讀音上會變慢。 | 加第一個音節索引，只掃同首音節的詞條。 |

`au42k7`、`w96j0`、`w96j0w96j0` 的候選清單在修正前後逐項相同，模糊音既有測試
沒有放寬。

### `sync_composition_zh` 的 panic

`gp6au42k7au4` 在某些早退路徑會讓 `base` 退回原始注音，而 `segs` 仍是轉換後的
單字。舊碼用 `seg.word.len()` 累加位元組位移，再切 `base[..at]`：

```
thread 'main' panicked at crates/jc-core/src/session/composition.rs:82:
end byte index 24 is not a char boundary; it is inside 'ㄧ'
```

現在位移一律以 rune 累加，並把片段範圍夾在 `base` 的 rune 長度內，不再對
`base` 做未檢查的位元組切片。回歸測試：

| 測試 | 內容 |
|---|---|
| `crates/jc-core/tests/tone7.rs` `tone7_key_stays_within_budget` | `gp6au42k7` 每鍵 `< 50ms`，最後 preedit 是「神秘的」。 |
| `crates/jc-core/tests/tone7.rs` `tone7_then_au4_does_not_panic` | `gp6au42k7au4` 不 panic，保留已轉換的字。 |
| `crates/jc-core/tests/tone7.rs` `tone7_fourth_syllable_never_reverts_both_characters` | 第四鍵不得把兩個字同時退回原始注音。 |
| `crates/jc-core/tests/tone7.rs` `existing_zh_sequences_stay_stable` | `au42k7`、`w96j0`、`w96j0w96j0` 的既有顯示不變。 |
| `crates/jc-core/src/session/composition.rs` `sync_composition_zh_uses_rune_offsets_when_widths_differ` | `base` 與 `segs` 寬度不同時不得 panic，片段仍在 rune 範圍內。 |
