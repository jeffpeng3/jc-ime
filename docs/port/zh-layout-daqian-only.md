# zh-layout-daqian-only 對照表

產品只保留標準（大千）排列，移除許氏與倚天 41 鍵。Go 來源：`/mnt/work/jc/ref/0c-ime`（main `8f443af`）。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/layout/layout.go` | `crates/jc-core/src/zh/keyboard.rs` | rewritten：`LayoutKind` 只留 `Standard`；`name()`／`ByName` 只認 `standard`；`Keyboard::new` 不再有 `UnsupportedLayout` 路徑 |
| `core/layout/layout_test.go` | `crates/jc-core/src/zh/keyboard.rs` | rewritten：保留 `test_standard_round_trip`、`test_standard_punct`、`test_by_name`；刪除倚天表格、往返與標點案例，未知名稱改用 `test_by_name` 驗證 |
| `core/layout/hsu.go` | — | n/a：不移植，產品只支援大千 |
| `core/layout/hsu_test.go` | — | n/a：不移植，產品只支援大千 |

## 移除內容

- `LayoutKind::Hsu` 與 `LayoutKind::ETen`
- `LayoutKind::name()` 的 `"hsu"`、`"eten"` 分支與 `from_name` 的對應解析
- `Keyboard::new` 的 `UnsupportedLayout` 錯誤路徑
- `ZhError::UnsupportedLayout`；移除後沒有任何地方使用
- `ETEN_KEYS`、`ETEN_PUNCT` 與倚天相關測試
- `Keyboard::eten()` 建構子
