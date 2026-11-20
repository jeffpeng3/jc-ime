# proto-candidates 對照表

本 task 把協定升到 v3，在 `State` 補上結構化候選、片段與高亮索引。核心的候選
產生流程與 session 決策邏輯沒有改動，Go 參考實作也沒有對應的線格式可搬，因此
沒有新的移植行為測試，只有序列化契約測試。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/proto/proto.go` | `crates/jc-proto/src/lib.rs` | rewritten：`State` 新增 `details`、`segments`、`highlighted`，`PROTOCOL_VERSION` 升到 3；`Response::state` 仍是唯一映射點 |
| 無對應（Go 版沒有結構化候選） | `crates/jc-proto/tests/fixtures/response_state_v3.json` | rewritten：手寫 v3 線格式 fixture |
| 無對應（Go 版沒有結構化候選） | `crates/jc-proto/tests/fixtures.rs` | rewritten：v3 往返、v1／v2 形狀取預設值、版本號與文件一致 |
| 無對應 | `crates/jc-core/src/candidate.rs` | rewritten：`CandidateSource` 只加 `Serialize`／`Deserialize` 與 snake_case 線格式，邏輯不動 |
| 無對應 | `docs/proto.md` | rewritten：v3 章節、`CandidateDetail` 與 `SegmentInfo` 欄位表、「尚未提供」小節 |
