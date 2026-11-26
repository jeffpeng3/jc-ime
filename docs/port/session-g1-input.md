# session-g1-input：輸入風格與鍵位

Worktree：`/mnt/work/jc/wt/session-g1-input`
Branch：`session-g1-input`（base：`session-core-split`）
上游：`/mnt/work/jc/ref/0c-ime` `8f443af`，範圍 `core/session/` 的 9 個檔案。

## 範圍與切分

`crates/jc-core/src/session/input_style.rs` 放風格狀態機、全形與標點表、選字鍵、
閒置按鍵與注音按鍵語意表。`Session` 只新增三個欄位：`style`（設定複製進來的
[`InputStyle`]）、`full_width`、`symbol_prefix`。規則版本為 0（預設）時
`style_key` 立刻回 `false`，既有鍵位一行都不改。

## 檔案對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `input_style.go` | `session/input_style.rs` | ported（部分）：`Shift+Space`（fullwidth／space／single／disabled）、微軟反引號前導、全形英數與標點表、Shift＋字母 `commit`／`lower`、`\` 打全形。`CandidateTab=grid`、`CandidateNavigation=horizontal`、`cycleStyleCandidate`、`changeSegmentation`、數字鍵盤與 Shift＋Delete 聲調未搬，原因見下。 |
| `zhuyin_keys.go` | `session/input_style.rs` | n/a：上游以逐格 `syls`／`literals`／`mixedKeys`／`mixedOrigins` 記錄「這個注音是怎麼來的」，提交時才產生 `ZhuyinKeyCommit`。本埠的注音狀態是 `ZhInput`（Composer＋已完成音節），沒有逐格來源與 literal 格陣列，`keySymbol`／`zhuyinLeft` 也依附在同一組資料上。硬搬會需要在 `zh::input` 新增公開 API 並改 `core.rs` 的提交路徑，超出本群組 owned paths。個人端的 `personal::zhuyin_keys` 已存在，接線留給整合階段。 |
| `zh_profile.go` | 無 | n/a：上游用 `lang.Profile`／`Composer`／`KeyPolicy`／`Punctuator` 介面把注音包成語言 adapter；本埠沒有 profile 抽象層，注音就是 `ZhInput`，加一層只有形狀沒有呼叫端。 |
| `layout_readings.go` | 無 | n/a：排列歧義讀音（許氏 `a` 是 ㄘ／ㄟ 等）只在非標準排列出現；本埠 `zh::keyboard` 只支援標準（大千）排列，一鍵一符號，沒有 `layout.AltCost` 與 `Seg.Matched`。 |
| `shift_symbol.go` | `session/input_style.rs` | ported（部分）：`shiftedSymbols`、`halfwidthPunct`、`halfwidthShift`、`HalfwidthShiftKeys`。`↓` 第二項列全形／鍵帽字元的 `insertHalfwidth` 未搬：本埠的字面插入沒有「同一格候選」的概念。 |
| `number_context.go` | 無 | n/a：數字語境靠把句首的 `P(w|BOS)` 換成中文數詞平均，需要注音解碼器吃跨片段左文。本埠 `decode::Decoder` 的 `best`／`nbest` 沒有左文參數，`LmScorer` 只在同一句內傳前詞；接線要改 `decode`，不屬於本群組。 |
| `keypolicy.go` | `session/input_style.rs` | ported：`zhuyin_key_action`／`KeyAction` 照上游語意表。`Convert`、`NonConvert`、假名與全半形切換鍵不在本埠 `KeyCode`，對應分支不列。 |
| `idle_keys.go` | `session/input_style.rs` | ported：`idle_keys`、`full_width_idle_keys`；在 `session` 模組重新匯出給外殼。 |
| `selection_keys.go` | `session/input_style.rs` | ported：`page_size`、`selection_keys`、`selection_key_columns`、`selection_index`；`core::select_by_digit` 改呼叫 `selection_index`。 |

## 測試對照

| Go 測試 | Rust 測試 | 狀態 |
|---|---|---|
| `TestOriginalStylePreservesLegacySequences` | `test_original_style_preserves_legacy_sequences` | ported |
| `TestMicrosoftWidthIsStateAndIncludesEnglish` | `test_microsoft_width_is_state_and_includes_english` | ported（英文模式以組字區顯示，非立即送出） |
| `TestMicrosoftPrefixAndInlineUp` | `test_microsoft_prefix_inserts_fullwidth_punctuation` | rewritten：只驗前導與標點；`Up=cycle` 未搬 |
| `TestMicrosoftPrefixCancellationPreservesComposition` | `test_microsoft_prefix_cancellation_preserves_composition` | ported |
| `TestMicrosoftObservedRawAndPrefixPunctuation` | `test_microsoft_observed_prefix_punctuation` | rewritten：閒置半形原字元交還應用程式那段 n/a，前導表照搬 |
| `TestMicrosoftRawPunctuationInComposition` | `test_microsoft_raw_punctuation_in_composition` | rewritten：選取模式離開那段 n/a |
| `TestStyleHotkeyClearsImmediatelyPreviousKeyState` | 無 | n/a：本埠沒有 `pendingToneKey` |
| `TestStyleLiteralPreservesPendingEnglishOrder` | 無 | n/a：本埠英文模式沒有「已標記的待定全形詞」 |
| `TestStyleLiteralPreservesMixedRunAndShiftCommit` | `test_shift_letter_commit_appends_to_composition` | rewritten：只驗 Shift 送出 |
| `TestMcBopomofoCycleInSecondWordAndCaps` | 同 Shift 送出測試 | rewritten：候選循環與 Caps Lock 未搬 |
| `TestMcBopomofoCycleAndShiftSpace` | `test_mcbopomofo_space_and_backslash` | rewritten：Shift＋空白與 `\` 照搬，Tab 循環與游標中插入空白 n/a |
| `TestMcBopomofoCycleReversesWithManyCandidates` | 無 | n/a：`styleCycle` 未搬 |
| `TestCustomCandidateSelectionAndCommit` | 無 | n/a：`CandidateSpace`／`CandidateEnter` 屬候選路徑 |
| `TestChewingTabChangesSegmentation` | 無 | n/a：`changeSegmentation` 未搬 |
| `TestChewingWidthAndHorizontalNavigation` | `test_chewing_english_width_table` | rewritten：只驗英文全形表 |
| `TestAppleShiftDeleteReopensTone` | 無 | n/a：`ZhInput` 沒有取回最後音節符號的 API |
| `TestStyleCandidatePageSizeAndSpace` | `test_candidate_page_size_can_be_nine`、`test_page_size_nine_digit_zero_not_a_selection_key` | rewritten：頁面大小與 `0` 不選字照搬，空白翻頁屬候選路徑 |
| `shift_symbol_test.go`（10 個） | `test_shift_symbol_halfwidth_*`（5 個） | rewritten：半形對照、前導、微軟原字元與 `halfwidth_shift_keys` 照搬；`↓` 候選、Ctrl 標點、混打括號、倚天／許氏排列與逐鍵 idle 比對 n/a（理由同上） |
| `selection_keys_test.go`（7 個） | `test_selection_keys_*`（4 個）、`test_selection_key_columns` | ported／rewritten：鍵標由核心回報的部分改成行為斷言；英文補字候選不標鍵屬混合群組 |
| `number_context_test.go`（6 個） | 無 | n/a：見 `number_context.go` |
| `input_style_compat_test.go` | `crates/jc-core/src/inputstyle.rs` 既有測試 | 已由 `inputstyle` 模組覆蓋 |

## 呼叫點

1. `Session::press`：`style_key` 在一般按鍵路徑之前；版本 0 直接跳過。
2. `Session::page`：頁面大小改讀 `page_size()`（預設仍是 10）。
3. `core::select_by_digit`：數字鍵改由 `selection_index` 判斷，預設規則與原本的
   1–9、0 完全相同，風格設定選字鍵時才多接受字母。

## 英文模式組字中的特殊鍵

英文模式是 raw input：沒有候選、沒有 suggestion。組字中的四個特殊鍵比照組字區
規則，一律由輸入法處理，不交給宿主；交給宿主會讓宿主結束組字，變成「順便送出」。

| 按鍵 | 行為 |
|---|---|
| Enter | 送出整段組字（含之前保留的中文緩衝），再把 Enter 放行給宿主：一次 Enter = 送出 + 換行／觸發預設按鈕 |
| Backspace | 只刪一個 rune，不送出；刪到空也只清組字 |
| Esc | 只清掉整段組字，不送出 |
| Tab | 沒有 suggestion 時 no-op：不動組字、不送出、不進選取模式，也不做宿主焦點切換 |

`crates/jc-tsf/src/service.rs` 的 `english_key_policy` 只回「這一鍵由輸入法處理」
的決策，不送 key、不 `present`；實際的 `session.key` 只在 `OnKeyDown` 執行一次，
避免 TestKeyDown／KeyDown 重複送同一鍵。Enter 在一般 TSF 程式由 `OnKeyDown`
回 `false` 放行原本的按鍵；IMM32 程式用 `complete_and_repost_enter` 經相容層完成
組字後把 Enter 重排回焦點視窗，兩條路徑都只換行一次。

Tab 在英文組字期間一律由輸入法吃掉，宿主不能用 Tab 切換焦點。這是刻意與注音
組字一致：組字中放行 Tab 會讓宿主結束組字，就是原本「Tab 送出＋縮排」的 bug。
要切換焦點請先按 Enter／Esc 結束組字；英文補全（Tab 接受 suggestion）不在這一輪
範圍，未來若要開啟必須另外設計，不能直接放行。

## 與上游的差異

- 本埠沒有 `handled` 旗標；上游「交還應用程式」的閒置半形符號改成直接送出
  （`pending_commit`），效果等於應用程式收到同一個字元。
- 英文模式的立即送出改成留在組字區（`raw`），與本埠既有英文模式一致；`commit`
  時一次取走。
- 微軟反引號前導在畫面上以字面 `` ` `` 顯示；上游用 marked text。具名鍵接手時
  反引號留在組字區（送出時保留），Esc／退格會移除標記。
- 全形 `Shift+Space` 只在風格版本非 0 時作用；`microsoft` 以外的預設不變。
