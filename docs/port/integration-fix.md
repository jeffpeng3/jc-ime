# integration-fix：合併後語意衝突

Worktree：`/mnt/work/jc/wt/integration-fix`
Branch：`integration-fix`（base `main`，HEAD `5baa489`）

## 結論

四個失敗的判定都是「行為對、測試前提過時」，實作沒有改。修改的四支測試
都保留原本要驗的行為，並補上更具體的值；沒有任何一項是靠放寬斷言過關。

| 測試 | 行為對不對 | 處理 |
|---|---|---|
| `ja::live::tests::test_neural_gate_receives_locked_prefix_as_left` | 對 | 修測試：逐字変換只送未鎖定的尾端 |
| `session::neural::tests::test_half_reading_skips_engine` | 對 | 修測試：完整讀音的統計基準改成非讀音 |
| `session::segments::tests::test_manual_segments_expire_on_next_key` | 對 | 修測試：未完成注音自成一段 |
| `session::tests::test_english_without_learning_keeps_existing_order` | 對 | 修測試：全形變體與前綴補完本來就會出現 |

## 逐一判定

### 1. `test_neural_gate_receives_locked_prefix_as_left`

**舊前提**：第二次轉換時，神經採用器收到的 `reading` 是完整讀音
`"あいうえおか"`，`left` 是 `"[あい][うえ]"`。

**新定義**：逐字変換只轉換尚未鎖定的尾端。`Controller::convert` 從
`cut` 之後取 suffix 當讀音，已鎖定的詞面接進 `ctx.left`。這與 Go
`core/ja/live/live.go` 的 `convert()` 相同：`suffix := c.seq[c.cut:]`，
`convertContext()` 把 `lockedText()` 加到 `ctx.Left` 前面。

因此第一次轉換 `"あいうえお"` 鎖住 `[あい][うえ]` 後，第二次的讀音是
`"おか"`、左文脈是 `"[あい][うえ]"`。行為正確。

**不是放寬**：若把完整讀音送進採用器，`left` 與 `reading` 會重複包含已鎖定
的詞面；`adopt_neural` 會用「本次轉換讀音」建立一個文節，與
`clauses_units(suffix)` 的檢查及下一次邊界比較都對不上。測試改成斷言
suffix 反而更精確，另外補上第一次轉換的 `reading`／`left`。

### 2. `test_half_reading_skips_engine`

**舊前提**：第二段把 `statistical` 設成 `"京"`，模型也回 `"京"`，預期
`Accepted { surface: "京" }`。

**新定義**：`neural::validate` 的第二條規則是「輸出等於統計結果時回
`Unchanged`，直接接受但不替換」。第二段回 `Unchanged` 是正確結果，模型
確實被呼叫了一次。舊測試把「模型有回覆」與「輸出要替換」混在一起。

**不是放寬**：第二段改成完整讀音 `"きょう"`、統計結果 `"今日"`，維持
`Accepted { surface: "京" }` 的斷言。這比改成 `Unchanged` 更強，也符合
一般路徑 `test_gap_threshold_accepts_engine_output` 的形狀。半截讀音
（`reading == statistical`）不呼叫模型的第一次斷言沒有動。

### 3. `test_manual_segments_expire_on_next_key`

**舊前提**：手動改成日文片段後按 `x`，片段數仍是 1，只是模式回到注音。

**新定義**：`x` 是注音鍵盤的 ㄌ，按下後留下未完成注音。
`composition-cursor` 之後，未完成的注音會自成一個片段，確保片段覆蓋整個
`preedit`（`rebuild_zh` 尾端補上 pending 片段）。所以畫面是「我ㄌ」兩段；
手動日文標記確實過期，兩段都重新推導成 `Mode::Zh`。這與 Go
`segments.go` 用 `segmentRev == s.rev` 讓手動片段在下一次按鍵失效一致。

**不是放寬**：測試改為斷言兩段的內容分別是「我」與「ㄌ」，且兩段模式都是
`Mode::Zh`。原本只檢查片段數與第一段模式，現在同時檢查片段邊界與覆蓋範圍。

### 4. `test_english_without_learning_keeps_existing_order`

**舊前提**：打對的 `hello` 只有一個候選。

**新定義**：`hello` 的候選是 `hello`、全形變體 `ｈｅｌｌｏ`、前綴補完
`hellos`。前綴補完本來就會列出比原字更長的詞（Go
`refreshEnglishCandidates` 的條件是 `len(word) > len(raw)`），全形變體是
`rebuild_en` 既有的候選，兩者都與個人化無關。

**不是放寬**：保留三個原本就該成立的條件：

- 原字仍是第一。
- 沒有任何 `CandidateSource::Personal` 候選（沒有學習紀錄就不得重排）。
- 同長度的 ASCII 拼字建議必須為空，且第三項之後的候選都要以原字開頭。

這比「候選數等於 1」更精確，也避開英文資料包是否有 `hellos` 的環境差異。

## 不在 owned paths 的失敗

`cargo test --workspace` 還有一個失敗，位於 `jc-settings-ui`：

- `state::tests::test_apply_writes_readable_settings`
- `esc-clear` 是 profile 管理鍵，預設使用內建 profile，`profile_locked`
  會擋下 `apply_bool`，值維持 `on`。測試沒有先 `copy_profile`，與本輪
  修改的 `jc-core` 無關；`crates/jc-settings-ui` 不在本任務的 owned paths，
  因此沒有改。

另外，本機需要 `libxcb`／`libxkbcommon` 的開發用 `.so` 才能連結
`jc-settings-ui` 測試；這次用 `/tmp/opencode/libshim` 的 symlink 加上
`RUST_FONTCONFIG_DLOPEN=1` 執行。

## 上游對照

| Go 檔案／函式 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/live/live.go` `convert`、`convertContext` | `crates/jc-core/src/ja/live.rs` `Controller::convert` | 已驗證：只轉換未鎖定尾端，鎖定詞面進左文脈 |
| `core/session/segments.go` `segmentManual`、`segmentRev` | `crates/jc-core/src/session/segments.rs` `expire_segment_view` | 已驗證：下一次按鍵重新推導片段 |
| `core/session/english_autocomplete.go` `refreshEnglishCandidates` | `crates/jc-core/src/session/mixed.rs` `english_candidates` | 已驗證：較長的前綴補完照列，打對的字沒有拼字建議 |
| `session/neural` 的採用驗證 | `crates/jc-core/src/session/neural.rs` `validate`、`maybe_convert` | 已驗證：輸出等於統計結果回 `Unchanged` |

## 驗證

| 指令 | 結果 |
|---|---|
| `cargo test -p jc-core` | 737 passed |
| `cargo check -p jc-core --all-targets` | 通過 |
| `cargo test --workspace --no-fail-fast` | 只有上述 `jc-settings-ui` 測試失敗，其餘全過 |
| `cargo run -p jc-cli --release -- eval zh-golden` | 45/45 |
| `cargo run -p jc-cli --release -- eval ja --corpus modern-500 --neural quality` | 93.6% |
| `cargo run -p jc-cli --release -- eval ja --corpus modern-500 --neural off` | 86.0% |
