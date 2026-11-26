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
`idle_delay: 300 ms`。`deferred_sentence` 現在是逐鍵部分預覽：每個新單位先
顯示已確定的前綴與かな尾巴，最後一個文節等停頓再補完。已顯示的漢字會鎖定，
後續按鍵只轉換它後面的內容；未完成的羅馬字原樣留在 preedit 尾巴。
`ja_conversion = sentence` 或 CLI `--no-live` 回到手動整句轉換。

`trigger`／`neural_scope`／`keep_tail` 仍保留給邊界觸發對照，預設不參與
延遲整句路徑。完整數據見 `docs/reports/live-default-2026-11-20.md`。

## 外殼觸發（`idle`）

`jc-core` 每個按鍵都給部分預覽；停頓只補完最後一個文節。停頓本身由外殼
計時器判斷。

- TSF 在每次按鍵處理完、且引擎回應仍是組字中時，重排一個 300 ms 的
  `WM_TIMER`。計時器掛在候選窗的訊息視窗上，與 `settle_focus` 共用訊息迴圈，
  不另開執行緒。
- 到期送出 `{"op":"idle","session":n}`。引擎對目前 session 呼叫
  `Session::ja_live_idle()`，回傳與 `key` 相同的 `State`；外殼用既有的
  `present` 套用，沒有第二條更新路徑。
- 300 ms 沿用 `live::Config::idle_delay` 的預設值。外殼只提供固定節拍；真正
  要不要轉換仍由引擎的 `enabled` 與 `deferred_sentence` 決定。
- 下一次按鍵會用同一個 timer id 重設期限，所以連續輸入不會每字都多一次
  IPC。候選窗開著（`navigating`）、組字結束與 Deactivate 都會取消計時器。
- `idle` 歸類成互動請求，沿用 10 秒預算。計時器路徑不重試、不主動啟動引擎，
  連不上就放棄這一次，下一次按鍵再重連。
- 沒有日文組字時 `ja_live_idle()` 只回目前快照，是便宜的 no-op。
