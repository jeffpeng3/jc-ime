# focus-pipe：用 focus 事件管理引擎 pipe 連線

Worktree：`/mnt/work/jc/wt/focus-pipe`（branch `focus-pipe`）

## 問題

`Activate` 是「切換到 jc 注音」的行程層級事件；每個載入 TIP 的程式都會
Activate，即使使用者沒在那個文字框打字。原本 `Control::new` 直接呼叫
`engine::spawn`，worker 一啟動就連 pipe，所以開兩個程式就有兩條連線搶一對一
pipe，拿到 `ERROR_PIPE_BUSY (231)`。

TSF 同一時間只有一個文字框在組字，pipe 一對一夠用。連線時機改成跟著 focus。

## 時序

| 事件 | 動作 |
|---|---|
| `Activate` | 建立訊息視窗、註冊 sink；不 spawn worker、不連 pipe |
| `OnSetFocus` | 啟動 worker 並連 pipe；已連線時忽略 |
| `OnKillFocus` | 排定 120 ms 後放掉 worker；期間再 set focus 就取消 |
| `Deactivate` | 立刻清掉 worker 的 sender，確保 pipe 最終釋放 |

`ITfThreadMgrEventSink` 在 windows-rs 0.62 沒有 `OnKillFocus` 方法。kill focus
用兩個等價入口：`OnSetFocus(pdimfocus = null)`（焦點離開所有文件）與
`ITfKeyEventSink::OnSetFocus(FALSE)`（鍵盤焦點離開 TIP）。

`OnSetFocus` 可能早於 `Activate`，`TextService::pending_focus` 記下
`Some(true)`／`Some(false)`，`Activate` 成功後補送；`Deactivate` 清掉它。

## 實作要點

- `composition.rs` 的 `FocusPipe` 狀態機：`Off` → `Active` → `Draining` →
  `Active`（取消）或 `Off`（Deactivate）。重複的 set／kill focus 回
  `PipeAction::None`，不重複連或斷。
- worker 的生命週期跟著 pipe：`OnSetFocus` 呼叫 `engine::spawn`，釋放時清掉
  `active.sender`，worker 的 `recv()` 收到 channel 關閉就結束並 drop pipe handle。
- 候選清單持有普通的 `Sender<Job>`。為避免在 jc-tsf 以外更動型別，
  `Control::new` 建立一條長期 bridge channel 與 dispatch 執行緒，轉送請求給
  當前的 worker；worker 隨 focus 今生今滅，bridge 不受影響。
- 寬限期用一條短命計時器執行緒實作，不用 async runtime。`ActiveWorker` 的
  `generation` 讓過期的計時器不會關掉新一代 worker。
- 不碰 `engine.rs` 的連線 API、`pipe_name.rs`、supervisor 互斥、
  `mode_from_settings`、CI、版本號與 catalog。

## 按需啟動引擎（補充）

`focus-pipe` 的狀態機與 120 ms 寬限期沒有變動。`OnSetFocus` 啟動 worker 後，
worker 在 `engine.rs::connect()` 連不上 pipe 時（pipe 不存在或忙碌）會 detached
spawn 引擎：用 `CreateProcessW` + `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`
啟動後立刻 `CloseHandle`，引擎是獨立行程。3 秒冷卻避免反覆連線失敗造成 spawn
風暴；supervisor 的 lockfile 保證全系統只有一個引擎，第二個 spawn 直接結束不算
錯誤。Run 機碼仍是主要啟動路徑，按需啟動只是 kill 引擎後的補救。

spawn 程式碼、指令路徑推導與測試見 `docs/port/engine-on-demand.md`。

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| n/a（Go 版沒有 TSF） | `crates/jc-tsf/src/lib.rs` | rewritten：Activate 只註冊 sink；補 kill focus 與 pending focus |
| n/a | `crates/jc-tsf/src/composition.rs` | rewritten：`FocusPipe` 狀態機、worker 隨 focus 啟動／釋放 |
| n/a | `crates/jc-tsf-smoke/src/checks.rs` | rewritten：檢查 6 改為不依賴 Activate 連線 |
| n/a | `docs/port/tsf-pipe.md` | 文件更新：連線時序圖 |

## 驗證

- `cargo check --workspace --all-targets`：通過
- `cargo check -p jc-tsf --all-targets --target x86_64-pc-windows-msvc`：通過
- `cargo clippy -p jc-tsf --all-targets --target x86_64-pc-windows-msvc -- -D warnings`：通過
- `cargo fmt --all -- --check`：通過
- `cargo test`：依共通規則未執行，由整合階段與 CI 跑

Windows 測試（只寫、未執行）：`focus_pipe_connects_once_and_releases_on_deactivate`、
`focus_pipe_deactivate_from_draining_stops`。

## 真機驗證

1. 開兩個記事本，都切到 jc 注音，輪流點文字框打字；兩邊都要出字。
2. `jc-tsf.log` 看不到 `ERROR_PIPE_BUSY`／231；只看到一條「已連上引擎 pipe」。
3. 點另一個記事本時，前一個的 `OnKillFocus` 日誌後接「已釋放引擎 pipe」。
4. 同一文字框內連續點擊，只看到一次「已啟動引擎 worker」。
