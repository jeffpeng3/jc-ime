# live-default：逐字変換研究與自動変換

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/live/live.go`（debounce、鎖定、draft） | `crates/jc-core/src/ja/live.rs` | ported（既有）；新增自動觸發、片段模型、停頓補正 |
| `core/ja/live/live_test.go` | `crates/jc-core/src/ja/live.rs`（`mod tests`） | rewritten：沿用既有測試，新增片段選字／變換／拆回讀音測試 |
| `core/session`（Zenz 採用流程） | `crates/jc-core/src/session/neural.rs` | rewritten：新增 `correction_gap_ratio` 與補正階段 |
| `cmd/0c-eval-ja` | `crates/jc-cli/src/{cli,eval_cmd,converter}.rs` | rewritten：新增 `--live-report` 與觸發／補正旗標 |
| — | `crates/jc-core/src/ja/live.rs` 的 `Trigger`／`AutoTrigger` | n/a：Go 沒有 Microsoft IME 形狀的自動変換 |
| — | 停頓補正與 generation 丟棄 | n/a：Go 逐字変換沒有非同步模型補正 |
| — | `ClauseSpan`／`KanaTransform` | n/a：Go 沒有片段腳本切換 |

## 量測入口

- `cargo run -p jc-cli --release -- eval ja --corpus modern-500 --live --live-report`
- 完整數據與指令見 `docs/reports/live-default-2026-11-20.md`。

## 預設值

`live::Config::default()` 現在是 `enabled: true`、`deferred_sentence: true`、
`idle_delay: 300 ms`。日文打字期間只顯示假名，停頓後做一次完整轉換；
`ja_conversion = sentence` 或 CLI `--no-live` 回到手動整句轉換。

`trigger`／`neural_scope`／`keep_tail` 仍保留給邊界觸發對照，預設不參與
延遲整句路徑。完整數據見 `docs/reports/live-default-2026-11-20.md`。
