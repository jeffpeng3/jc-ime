# composition-model：Auto／User 片段模型與語言後端

上游：`/mnt/work/jc/ref/0c-ime` `8f443af`。Branch：`composition-model`。
Base：`live-default` `3919a1e`（延遲整句轉換、手選片段保留）。

本輪把「使用者固定哪一段」從各語言路徑拉出來，變成一份語言無關的
`Composition`。注音與日文都只讀同一份 `User` 標記；語言後端只轉換連續的
`Auto` 區段。`selection-pin` 不再需要另做一套鎖定。

## 架構

```text
Session 編輯 ──→ Composition（text / cursor / segments / User 標記）
                      │
                      ├── auto_runs()：連續 Auto 讀音＋前後文脈
                      │        │
                      │        ├── ZhConvertBackend（現有 Decoder）
                      │        └── JaConvertBackend（現有 JaDecoder）
                      │
                      └── Snapshot（preedit、segments、owners）
```

- `crates/jc-core/src/composition.rs`：`Composition`、`CompSegment`、`Owner`、
  `AutoRun`、`ConvertContext`、`Converted`、`ConvertBackend`。
- `crates/jc-core/src/session/composition.rs`：`Session` 與模型的橋接與寫回點。
- `crates/jc-core/src/session/backends.rs`：注音／日文後端實作與
  `Session::convert_auto_runs`。

### 公開介面

| 介面 | 用途 |
|---|---|
| `Session::composition()` | 目前組字模型（文字、游標、片段、`User` 標記）。 |
| `Session::auto_runs()` | 目前會被送去轉換的連續 `Auto` 區段。 |
| `Session::user_marks()` | 使用者固定片段的讀音座標。 |
| `Session::convert_auto_runs()` | 逐段呼叫 `zh`／`ja` 後端，回傳每段結果；`User` 片段不會出現。 |
| `Session::ja_live_flush()` | 非延遲模式輸入結束時補齊日文尚未轉換的讀音。 |
| `Snapshot.owners` | 與 `Snapshot.segments` 對齊的所有權，供外殼標示。 |

### 所有權

| Owner | 來源 | 重轉行為 |
|---|---|---|
| `Auto` | 輸入、系統轉換結果 | 每次轉換都可以改寫 |
| `User` | 選字、`j`／`k`、F6–F10、拆回讀音、滑鼠鎖定 | 原樣保留，只送前後文脈給後端 |

`User` 標記以「同語言讀音單位」保存（注音音節或日文假名），不是顯示字串
位移。每次轉換後由 `Composition::replace_segments` 依讀音把標記套回新片段：

- 讀音完全相同：保留 `User`，顯示文字可以隨轉換結果更新。
- 讀音不同（片段內刪字、改音）：標記丟棄，片段退回 `Auto`。
- 在片段之前插入：`shift_units` 平移標記，片段保持固定。

### 轉換流程

1. 編輯只改組字狀態，並依規則更新 `Composition` 的 `User` 標記。
2. `auto_runs()` 把相鄰、同語言、有讀音單位的 `Auto` 片段連成一段；`User`
   片段、字面文字與語言切換都會切開區段。
3. 每段 `AutoRun` 帶 `ConvertContext { left, right }`，內容是這段前後所有
   已固定文字。
4. 後端回傳 `Vec<Converted>`，單位數必須等於 `AutoRun::units`；寫回時只換
   這一段，`User` 片段不在任何 run 裡。
5. `Snapshot.owners` 與 `Snapshot.segments` 一一對應，外殼可標示 `User`。

### 注音

`Session::locks` 現在是推導結果：`sync_zh_locks` 把 `Composition` 的
`User` 標記換成 `decode::Lock`。選字、`lock_segment`、加入詞庫都只呼叫
`mark_zh_user_syllables`；沒有任何路徑直接寫 `locks`。

### 日文

`ja::live::Controller` 的 `user_segments` 也是快取。`Session` 在每次
`live.input` 之前先用 `Composition` 的共同前綴規則裁掉失效片段，再把
`User` 清單推給控制器；選字、F6–F10、拆回讀音後由 `capture_ja_users` 把
控制器回報的讀音範圍抄回 `Composition`。

延遲整句模式本來就只轉 `User` 片段之間的空隙；本輪把那條路徑的權威來源
換成 `Composition`，控制器不再自行決定所有權。

## 附帶修正

驗證日文 `User` 片段時發現三個既有問題，會讓「切假名／拆回讀音」選到錯的
片段或看不到結果，已在同一輪修掉：

| 問題 | 位置 | 修正 |
|---|---|---|
| `JaSpan::new` 第二個參數是長度，卻傳入結束位置 | `Session::ja_active_span`、`open_ja_candidates` | 改傳 `end - start`。 |
| 每次重畫都把 `ja_cursor` 移到結尾 | `Session::apply_live_plan` | 保留舊游標；只有讀音真的改變時才移到結尾。 |
| 候選窗關閉後仍用高亮候選當組字字串 | `Session::displayed_base` | 只在候選窗開著時預覽。 |

## 上游對照表

| Go 檔案／函式 | Rust 位置 | 狀態 |
|---|---|---|
| `core/session/session.go` `locks`、`lockSegment` | `composition::Composition`、`session::mark_zh_user_syllables`、`sync_zh_locks` | rewritten：鎖定改由語言無關模型推導。 |
| `core/session/session.go` `shiftLocks` | `composition::Composition::shift_units`、`session::shift_zh_ownership` | rewritten：以讀音單位平移，跨過編輯點即解除。 |
| `core/session/segments.go` `segmentLocked`／`unlock` | `session::segment_locked`、`demote_zh_syllables` | rewritten：改讀 `Composition` 推導出的鎖定。 |
| `core/ja/live/live.go` `userSegments` | `ja::live::Controller::user_segments`、`set_user_segments` | rewritten：清單由 `Composition` 推送。 |
| `core/ja/live/live.go` 逐字変換的鎖定前綴 | `ja::live::convert_sentence` 的 gaps | ported：只轉 `User` 之間的空隙，文脈用左右固定文字。 |
| — | `composition::ConvertBackend`、`session::backends` | n/a：Go 沒有語言後端介面，各語言直接呼叫解碼器。 |
| — | `composition::AutoRun`、`ConvertContext` | n/a：Go 的鎖定是片段索引，沒有語言無關的 run。 |
| — | `Snapshot.owners` | n/a：Go 的 `Segment` 自帶鎖定旗標；本埠把旗標留在核心。 |

## 新增語言要實作什麼

1. 在 `composition::Lang` 加一個語言，並在 `Lang::join_readings` 定義讀音
   接法（注音用空白、日文直接串接）。
2. 在 `session/composition.rs` 加一個 `sync_composition_<lang>`：把該語言的
   顯示片段轉成 `CompSegment`，讀音單位要能對回原始輸入。
3. 實作 `ConvertBackend`：只接受自己的 `Lang`，用 `ConvertContext` 的左文與
   右文，回傳的 `Converted.units` 總和必須等於 `AutoRun::units`。
4. 把「使用者固定」的動作接到 `Composition::mark_user_units`；
   「片段內編輯」接到 `demote_user_units` 或 `shift_units`。
5. 語言原生結構只能當快取：每次重建都從 `Composition` 推導，不得反向寫回
   所有權。
6. 補一個行為測試：選定片段後繼續輸入保持，片段內編輯解鎖。

## 測試

`RUST_FONTCONFIG_DLOPEN=1 cargo test -p jc-core` 全綠（674 + 97）。

預設改成延遲整句後，日文測試改成照新流程驅動：打完字呼叫
`Session::ja_live_idle()`（延遲整句）或 `Session::ja_live_flush()`（非延遲
逐字）。沒有放寬任何期望值，也沒有改回立即轉換。

另外修掉測試暴露的實作問題：

- `flush()` 直接跳過邊界 hold，輸入結束一定把最後一段轉完。
- 游標移動或開片段候選窗時不再清掉候選清單；整句候選也不會蓋掉片段候選。
- 日文片段選字會記錄學習，學習翻掉第一名時顯示跟著換。
- 非延遲模式片段選字後用 `lock_through` 固定；整段鎖定時 `prefix_intact`
  不再誤判成前綴被破壞。

| 任務書測試 | Rust 測試 |
|---|---|
| 注音選定後繼續輸入保持、片段內刪字解鎖 | `session::core::tests::test_zh_user_segment_survives_more_input_and_demotes_on_edit` |
| 日文選字後停頓重轉不影響 | `session::tests::test_ja_deferred_sentence_keeps_user_segment_on_reconvert`（既有） |
| 日文 F7 後停頓重轉不影響 | `session::tests::test_ja_deferred_sentence_keeps_toggled_segment_on_reconvert` |
| 日文拆回讀音後停頓重轉不影響 | `session::tests::test_ja_deferred_sentence_keeps_unconverted_segment` |
| 中英日並存只送 Auto | `session::backends::tests::composition_reports_three_languages`、`only_auto_runs_are_sent` |
| `auto_runs()` 切開並帶文脈 | `composition::tests::user_segment_splits_runs_and_appears_in_context`、`session::core::tests::test_auto_runs_split_around_user_segment_with_context` |
| 提交後再輸入不變 | `session::core::tests::test_committed_text_stays_after_more_input` |
| 既有注音組字游標測試 | `test_insert_syllable_at_cursor_matches_direct_typing`、`test_backspace_and_delete_in_middle_keep_segments_whole`、`test_resize_segment_changes_candidates_and_keeps_segments`、`test_jk_changes_only_cursor_segment`、`test_keep_candidates_after_segment_change_until_escape`（全部保留） |

模型本身的單元測試在 `composition::tests`，涵蓋：

- `AutoRun` 串接、邊界與文脈。
- `User` 片段切開 run、重轉後依讀音保留。
- 片段內改讀音退回 `Auto`。
- 插入／刪除後標記平移與解除。
- 轉換結果寫回、單位數不符時不改狀態。
- 字面文字與多語言切開 run。

## 驗證

- `cargo check -p jc-core --all-targets`：通過。
- `cargo fmt -p jc-core`：已執行。
- `cargo run -p jc-cli --release -- eval zh-golden`：45/45。
- `cargo run -p jc-cli --release -- eval fuzzy`：44.5%。
- `cargo run -p jc-cli --release -- eval ja --corpus modern-500`：92.2%（auto）。
- `cargo run -p jc-cli --release -- eval ja --corpus modern-500 --quality`：93.6%。
- `cargo run -p jc-cli --release -- eval ja --corpus modern-500 --neural off`：86.0%。
- `cargo run -p jc-cli --release -- eval mixed`：6/6。
