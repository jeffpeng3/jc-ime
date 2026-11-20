# proto-compat 對照表

本 task 只加協定凍結測試與手寫 fixture，不改任何欄位名稱或型別。Go 參考實作
的 `core/proto/proto.go` 早在 w1-proto-engine 就已 ports，這次補的是線格式的
回歸防護。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/proto/proto.go` | `crates/jc-proto/src/lib.rs` | n/a：本 task 不加實作，只加凍結用的 fixture 與測試 |
| `core/proto/proto.go` | `crates/jc-proto/tests/fixtures/*.json` | rewritten：Go 沒有 fixture，改成 11 個手寫 JSON 鎖住 v2 線格式 |
| 無對應（Go 版沒有） | `crates/jc-proto/tests/fixtures.rs` | rewritten：新增行為測試，涵蓋各訊息往返、v1 形狀相容與版本號一致性 |
