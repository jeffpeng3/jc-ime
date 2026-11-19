# w1-proto-engine：Go 對照

參考實作：`/mnt/work/jc/ref/0c-ime`（main `8f443af`）。

本 task 的協定是重新設計的列舉式請求／回應，Go 端的 `Envelope`＋session 模型
只當行為參考，不逐欄位照搬。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `bridge/ipc/ipc.go`（`WriteFrame`／`ReadFrame`／`MaxFrame`） | `crates/jc-proto/src/frame.rs`、`crates/jc-proto/src/message.rs` | ported；改成回傳錯誤而不是 panic，並加上 0 長度拒絕 |
| `bridge/ipc/ipc.go`（`Envelope`、`Reply`） | `crates/jc-proto/src/lib.rs`（`Request`／`Response`） | rewritten；改成 `op` 標籤的列舉，沒有 session 與 `OK` 布林 |
| `core/proto/proto.go`（`Request`、具名鍵、`Snapshot` 概念） | `crates/jc-proto/src/lib.rs`（`Request`、`Key`、`NamedKey`、`Response::State`） | rewritten；只留 `Hello`／`Key`／`Snapshot`／`Commit`／`Shutdown`，其餘狀態之後的 task 補 |
| `bridge/ipc/ipc_test.go::TestFrameRoundTrip` | `crates/jc-proto/tests/framing.rs`（`frame_round_trip_keeps_empty_and_non_ascii_text`、`oversized_frame_is_rejected_before_payload_arrives`） | rewritten；長度上限改成可設定 |
| `bridge/ipc/ipc_test.go`（drain、session、health、vocab 等測試） | n/a | 這個 task 沒有 session、drain、health、vocab，等功能進來再搬 |
| `cmd/0c-server/main.go`（啟動、socket 監聽） | `crates/jc-engine/src/main.rs` | rewritten；只留 `--socket`、`--max-frame`，一次一條連線 |
| `cmd/0c-server/main.go`（連線處理迴圈） | `crates/jc-engine/src/lib.rs::serve` | rewritten；同步 blocking IO，收到 `Shutdown` 或 EOF 結束 |
| `core/engine`（按鍵轉換） | n/a | 轉換邏輯不屬於本 task，`Engine::handle` 目前只是佔位 |

## 註記

- `Key` 是 `jc-proto` 的本地型別。`jc-core` 目前沒有公開的 `Key`，整合時再換。
- 版本不符與 `Shutdown` 都會讓連線結束；socket 模式下版本不符只收掉該連線，
  行程繼續服務下一個連線，`Shutdown` 才結束行程。
- 沒有 Go 對應的 stdio 傳輸：Go 版只有 unix socket 與 Windows named pipe。
