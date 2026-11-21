# fix-su-prefix：未完成輸入不鎖死語言、補全只提示

## 根因

`su` 在 Mixed 的統一 DP 下被判成假名す；`rebuild_mixed_candidates()` 無條件把英文補全放進候選，
`suggestion` 落在 index 2。外殼在單一數字鍵前補 `Down`，核心先跑 `select_by_digit()`，
`3` 選到 suggestion，`ascii.accept()` 把 pending 改成 suggestion，後面的 `cl3` 接不上。
`5k` 帶數字，不是合法羅馬字也不是英文補全前綴，所以沒有這個問題。

## 修正

| 項目 | 修正 |
|---|---|
| `rebuild_mixed_candidates()` | `display_pending(&self.lexicon) != raw` 代表 pending 是未完成的注音前綴，跳過英文補全候選；補全仍由 `Snapshot.suggestion` 提示 |
| `AsciiInput::tone_completes()` | 新增判據：只有聲調鍵真的把 pending 補成音節才回 `true`，避免 `sug3` 被當聲調照字面吸收 |
| `press_char()` | Mixed 模式下 `tone_completes()` 先於 `select_by_digit()`；吸收後清 `navigating`、`candidate_window` 再 rebuild |
| 不變 | `english_autocomplete` 開關、`pending_display_with()`、DP 分數、外殼 `needs_selection_prefix()` |

## 對照表

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/session/mixed.go`：`openMixedCandidates` | `crates/jc-core/src/session/mixed.rs`：`rebuild_mixed_candidates` | rewritten：注音前綴不進英文補全候選 |
| `core/session/mixed.go`：`finishMixedTone` | `crates/jc-core/src/session/mixed.rs`：`AsciiInput::tone`／`tone_completes` | rewritten：新增是否完成音節的只讀判據 |
| `core/session/session.go`：`key` 的數字選字分支 | `crates/jc-core/src/session/core.rs`：`press_char` | rewritten：Mixed 聲調鍵優先於選字鍵 |
| `core/session/selection_keys.go`：`selectionIndex` | `crates/jc-core/src/session/input_style.rs`：`selection_index` | n/a：既有port，本次未改 |

## 測試

`crates/jc-core/src/session/mod.rs` 新增五條路徑測試，外殼的 `Down` 用 `KeyCode::Down` 模擬：

| 測試 | 預期 |
|---|---|
| `test_su_prefix_tone_key_wins_over_digit_selection` | `su` 候選沒有英文補全；`su` + Down + `3cl3` 送出 `你好` |
| `test_5k4_tone_key_not_regressed` | `5k` + Down + `4` 送出 `這` |
| `test_hello_tab_completion_unchanged` | `hellp` + Tab 補成 `hello`，完整詞再按 Tab 不變 |
| `test_sug_digit_selects_english_candidate` | `sug` + Down + `3` 選到英文候選，不是 `sug3` |
| `test_ji3hellp_completion_not_regressed` | `ji3hellp` 提示與 Tab 補全照舊 |

## 驗證

- `cargo test -p jc-core`：720 + 97 通過，0 失敗。
- `cargo check --workspace --all-targets`、`cargo clippy -p jc-core --all-targets -- -D warnings`、`cargo fmt -p jc-core` 通過。
- 四組題库 `zh-golden`、`ja`、`fuzzy`、`mixed` 與 HEAD 基準比較，正確率指標全同；只有耗時數字浮動。
