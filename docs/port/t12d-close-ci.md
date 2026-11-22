# T12d：Close session id 修復 + CI 驗證

基準：0c-ime fork main `8f443af`；T4 回報 `/tmp/opencode/t4-engine-result.md` 第八節。
Worktree：`/mnt/work/jc/wt/t12d-close-ci`，branch `t12d-close-ci`（base `tsf-overhaul`）。
本輪只修 session id 流轉與 Close 歸屬的測試，不改 PersonalStore（T12c）與 service（T12b）。

## 逐檔對照

Go 檔案｜Rust 檔案｜狀態
---|---|---
`bridge/ipc/server.go`（`OpOpen` 回 `Reply{Session: id}`；`OpClose` 依 `sessions` map 判歸屬）｜`crates/jc-proto/src/lib.rs`（`Response::Ready` 加 `session`）、`crates/jc-engine/src/lib.rs`（Hello 回實際配置的 id）｜rewritten：0c 用 map 判 id，jc 用 `Registry.current`／`meta.owner`；Ready 新增欄位，`#[serde(default = "default_ready_session")]` 補舊值 1
`bridge/ipc/server.go`（`OpHandle`／`OpPoll` 的 `sessions[env.Session]` 檢查，322-324 行）｜`crates/jc-engine/src/lib.rs::close_session()`（沿用 T4 的 `ownership_error()`）｜ported：跨連線 id 回 `Internal` 錯誤，不是靜默成功
`platforms/windows/tsf/src/session.rs`（`ensure_open()` 記 `reply["session"]`，reset／Drop 送 `close`）｜`crates/jc-tsf/src/session.rs`（`response_to_reply()`、`ensure_open()`）｜rewritten：jc 的 Ready 原本固定映射成 session 1，本輪改成原樣帶引擎 id；`ensure_open()` 缺欄位時退回 1
`bridge/ipc/ipc.go`（Envelope／Reply 的 `Session` 欄位）｜`crates/jc-proto/src/lib.rs`（`Request::Close{session}`、`Response::Acknowledged{session}`）｜n/a：T3 已定義，本輪只讓值正確

## session id 流轉

一、`Engine::handle(Request::Hello)` 呼叫 `open_session()` 後，從 `Registry.current[conn]` 取出實際 id 放進 `Response::Ready.session`。連線 id 從 1 開始，第一條連線拿 1，第二條拿 2。

二、`jc-proto` 的 Ready 反序列化用 `default_ready_session()` 補 1，所以舊引擎沒有這個欄位的訊框仍被當成 session 1。

三、T1a 的 `response_to_reply()` 把 Ready 的 session 原樣放進 0c reply 的 `session` 欄位；`Session::ensure_open()` 記住它，`generation` 不變就沿用。

四、`Session::reset()` 與 `Drop` 的 `{"op":"close","session":self.id}` 因此送到正確的 id；跨連線的 id 被引擎的歸屬檢查擋下並回 error。

Key／Toggle／Commit 不加 session 欄位。每條連線的 `current[conn]` 只對應自己持有的 session，引擎沒有讓一條連線開多個 session 的 op；帶 id 只會重複現有的連線隔離，還要多改 jc-proto 與 adapter 的 wire 形狀。

## CI 測試清單

新增 `crates/jc-engine/tests/close_session.rs`，五個測試：

一、`two_connections_get_distinct_session_ids_from_ready`：兩條連線 Hello 拿 1 與 2，`session_ids()` 各自吻合。

二、`each_connection_closes_its_own_session`：各自 Close 回 `Acknowledged{session}`，`HealthInfo.sessions` 從 2 降到 1 再降到 0。

三、`close_with_another_connections_id_is_rejected`：用別條連線的 id Close 回 `Internal` 錯誤，對方的 session 還活著且能自己 Close。

四、`closed_session_is_removed_from_registry`：Close 後 registry 不再計數、持有集合清空，重複 Close 回錯誤。

五、`ready_and_close_session_ids_travel_over_the_wire`：用 `serve()` 與記憶體 buffer 跑完整訊框，驗證 Ready 的 session、跨連線 Close 的錯誤與正確 Close 的 `Acknowledged` 都走同一條線路。

adapter 側新增兩個測試：`ready_reply_carries_the_engine_session_id` 驗證 Ready→reply 的 id 與舊形狀補 1；`engine_session_id_is_remembered_and_sent_on_close` 用真正 jc Ready 訊框驗證 `Session::id` 與 reset 送出的 Close id。

協定側新增 `ready_carries_the_engine_session_id` 與 `ready_defaults_to_session_one_for_old_engine_shape`。

Windows 覆蓋：`pipe_listener::serve()` 的 worker 進入點是 `accept()` → `into_engine()` → `serve_connection()`（等於 `crate::serve`）；`crates/jc-engine/src/pipe_listener.rs` 的 `accepted_connections_use_their_own_session_id_on_close` 走完全相同進入點，驗證兩條 accepted 連線拿 1／2、跨連線 Close 回錯、自己的 Close 回 `Acknowledged`。此測試掛在 `cfg(windows)`，由 Windows target 的 `--all-targets` check 編譯。

## 檢查結果

`cargo fmt -p jc-proto -p jc-engine -p jc-tsf -p jc-windows -p jc-tsf-smoke`：通過。
`cargo check --workspace --all-targets --offline`：通過，0 error、0 warning。
`cargo check -p jc-engine -p jc-proto -p jc-tsf -p jc-windows -p jc-tsf-smoke --target x86_64-pc-windows-msvc --all-targets --offline`：通過，0 error、0 warning。
依任務書未跑任何 test，未 commit。

## 與 T12c 的接合點

T12c  PersonalStore 的存檔路徑不變：`close_session()` 仍在移除 metadata 前 clone personal store，並在鎖外呼叫 `save_personal()`。本輪只改 Ready 的欄位與 Close 帶的 id，因此 T12c 若調整每 session 的存檔策略或合併邏輯，不需要改 session id 流轉。個人資料前綴（`personal`／`personal`）也未動。

詳細回報見 `/tmp/opencode/t12d-close-ci-result.md`。
