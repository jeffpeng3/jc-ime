# c0-contracts 對照表

本 task 只凍結型別與 module 骨架，不含任何轉換演算法。標 `rewritten` 的列
代表契約已定、實作留給後續 task。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/lang/lang.go` | `crates/jc-core/src/lang.rs` | rewritten：只定義 `Mode`（含 `Mixed`），語言判定留給 ja／score task |
| `core/input/input.go` | `crates/jc-core/src/key.rs` | rewritten：改成 `Key`／`KeyCode` 的語義按鍵，不含平台鍵碼對應 |
| `core/segment/segment.go` | `crates/jc-core/src/segment.rs` | rewritten：`Segment` 加上 `script`，`Script` 縮成任務書列的六種 |
| `core/lattice/candidate.go` | `crates/jc-core/src/candidate.rs` | rewritten：只留 `surface`／`reading`／`cost`／`source`，不含 lattice 節點 |
| `core/output/output.go` | `crates/jc-core/src/output/mod.rs` | rewritten：先定 `Snapshot` 與 `Snapshot::empty()`，渲染留給 output task |
| `core/decode/decode.go` | `crates/jc-core/src/decode/mod.rs` | rewritten：先定 `Conversion`／`EngineError`，解碼留給 decode task |
| `core/reading/reading.go` | `crates/jc-core/src/reading.rs` | n/a：本 task 只凍結 module 位置，讀音型別由 zh／ja task 補 |
| `core/score/*.go` | `crates/jc-core/src/score.rs` | n/a：本 task 只凍結 module 位置，評分由 score task 補 |
| `core/lattice/*.go` | `crates/jc-core/src/lattice/mod.rs` | n/a：本 task 只凍結 module 位置，lattice 由 zh／ja task 補 |
| `core/zhuyin/*.go` | `crates/jc-core/src/zh/mod.rs` | n/a：本 task 只凍結 module 位置，注音解析由 zh task 補 |
| `core/ja/*.go` | `crates/jc-core/src/ja/mod.rs` | n/a：本 task 只凍結 module 位置，日文轉換由 ja task 補 |
| `core/session/*.go` | `crates/jc-core/src/session.rs` | rewritten：先放 `CORE_VERSION`，狀態機由 session task 補 |
| `core/proto/proto.go` | `crates/jc-core/src/output/mod.rs`、`src/key.rs` | rewritten：只取快照與按鍵的語義欄位，JSON 協定由 jc-proto 負責 |
| `core/purity_test.go` | — | n/a：Rust 版以 crate 邊界（`jc-core` 不依賴平台 API）保證，不需要對應測試 |
