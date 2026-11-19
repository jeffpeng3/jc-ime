# ja-pack-v2：pack v2 讀取端

`jc-data` 的日文 pack reader 支援 `0CJADIC\0` v2：96 bytes 檔頭、24 bytes
詞條、每筆詞條的 `left_id`／`right_id` 與連接成本矩陣。v1 的版面、
成本語意與驗證維持不變，同一份程式依檔頭版本分流。

## 對照表

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `internal/jadic/pack.go` | `crates/jc-data/src/ja/pack.rs` | rewritten：version 決定 header size（80／96）與 entry stride（20／24）；v2 追加 connection section 的界線、不重疊與長度驗證；`Header` 的 reserved 欄位改成 `context_count`，另存 connection offset／length |
| `internal/jadic/pack_test.go` | `crates/jc-data/src/ja/tests.rs` | rewritten：v1 測試保留；新增合成 v2 pack（4×4 矩陣）、v2 壞檔（header_size、entry stride、context_count=0、connection 長度／越界／重疊、reserved、left／right id 越界、SHA）與 `verify()` 測試 |
| `internal/jadic/manifest.go` | `crates/jc-data/src/ja/manifest.rs` | rewritten：`pack_format` 增加 `entry_size` 與 `context_count`；version 必須等於 pack 檔頭，v2 的 `entry_size` 必須是 24 |
| `internal/jadic/manifest_test.go` | `crates/jc-data/src/ja/tests.rs` | rewritten：v1 manifest 可省略 `entry_size`；新增 v2 配對與 `entry_size`／`context_count`／version 不符的測試 |
| `internal/jadic/real_pack_test.go` | `crates/jc-data/src/ja/tests.rs` | rewritten：`ja-lexicon-public-core.pack` 追加 `context_count()==0`、`connection()` 為 `None`、entry 沒有 context id 的斷言；新增 `ja-lexicon-mozc-v2.pack` 真檔測試 |
| `internal/jadic/order.go` | `crates/jc-data/src/ja/order.rs` | n/a：排序語意與 v1 相同，這一版沒有改動 `compare_entries` |

## 設計決定

- **版本分流**：`FORMAT_VERSION = 1` 與 `HEADER_SIZE`／`ENTRY_SIZE` 保留為
  v1 常數，另外新增 `FORMAT_VERSION_V2`、`HEADER_SIZE_V2 = 96`、
  `ENTRY_SIZE_V2 = 24`，避免動到 `jc-core` 既有的 v1 測試 builder。
- **context_count**：v1 的 offset 30 是 reserved，必須是 0，`Pack::context_count()`
  回傳 0；v2 為 0 直接視為壞檔。v2 檔頭尾端兩個 reserved u32 也必須是 0。
- **entry context id**：`Entry::left_id()`／`right_id()` 以 `Option<u16>` 表示，
  v1 是 `None`；v2 解碼時檢查 id 小於 `context_count`，超出即失敗。
- **連接矩陣**：不複製 14 MB 的矩陣，`Pack::connection()` 依
  `left * context_count + right` 直接從 mmap 讀 `i16`；v1、越界都回 `None`。
- **manifest**：`pack_format.version` 必須與 pack 檔頭一致；v2 要求
  `entry_size == 24`，`context_count` 有記錄時必須與檔頭相同。v1 manifest
  可以沒有 `entry_size`。

## 測試

- 合成 v1／v2 pack：v2 斷言 24 bytes stride、`context_count()`、entry 的
  left／right、`connection()` 的具體值與越界行為、`verify()`。
- v1 相容：既有 sample pack 與 `ja-lexicon-public-core.pack`（1,147,021 筆）
  的斷言不變；另外確認沒有 context 資料。
- v2 真檔：`packs/ja-lexicon-mozc-v2.pack` 斷言 `context_count() == 2672`、
  `connection(0,0)` 有值、抽樣 entry 的 id 在範圍內；檔案不存在時由
  `require_data!` 跳過。
- 壞檔：header_size、entry stride、context_count=0、reserved、connection
  長度／越界／重疊、left／right id 越界、payload／trailer SHA。
