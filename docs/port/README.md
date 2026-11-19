# Port 對照表

每個移植 task 在 `docs/port/<task>.md` 留下一份對照表，說明 Go 參考實作
的每個檔案去了哪裡。

## 格式

表格固定三欄：

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/segment/segment.go` | `crates/jc-core/src/segment.rs` | ported |
| `core/lattice/candidate.go` | `crates/jc-core/src/candidate.rs` | rewritten |
| `cmd/0c-ibus/engine.go` | — | n/a：屬於平台外殼，不在本 workspace 範圍 |

## 狀態值

- `ported`：行為與 Go 版本一致，測試也一起搬過來（`TestFoo_Bar` →
  `fn test_foo_bar()`）。
- `rewritten`：行為等價，但因為 Rust 的型別或所有權模型而重寫結構；
  必須在同一個 task 的說明裡寫出動機。
- `n/a`：不移植。**一定要寫原因**，不能只寫 `n/a`。常見原因是平台相關、
  已被其他 crate 取代，或 Go 版本只是內部實作細節。

## 規則

- 每個 Go 檔案都要有一列，包含測試檔；純內部結構的測試改寫成行為測試，
  狀態標 `rewritten` 並註明。
- 需要外部資料的測試用 `jc_testdata` 的 `require_data!()`，缺資料時跳過。
- 測試要斷言具體值，不要只斷言 `is_ok()`。
