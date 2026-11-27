# big5-table：Big5 → Unicode 查表資料

前導字元 `b` 的資料層。這一支只提供碼位查表，不動前導解析、按鍵或注音
行為；`b` 在 `PREFIX_RESERVED` 裡仍是保留但未實作的鍵。

```rust
pub fn big5_to_unicode(code: u16) -> Option<char>;
```

| 範例 | 結果 |
|---|---|
| `0xA440` | `一` |
| `0xA4A4` | `中` |
| `0xA140` | U+3000（全形空白） |
| `0xA141` | `，` |
| `0x0000`、`0x8140`、`0xFFFF` | `None` |

## 資料來源

| 項目 | 值 |
|---|---|
| 來源 | CPython 3.12 的 `big5` codec（`Modules/cjkcodecs`） |
| 合法區間 | lead `0xA1..=0xF9`、trail `0x40..=0x7E`／`0xA1..=0xFE`（Big5 `0xA140–0xF9FE`） |
| 映射數 | 13,710 |
| 產生檔 | `crates/jc-core/data/big5/big5.bin`，27,946 bytes |
| SHA-256 | `6599b630ec39a573fbb21bd09d08535e373199c4a69d9b6313e1977832e102c2` |
| 授權 | CPython 為 PSF-2.0；codec 表由 CPython 專案維護 |

CPython 的 `big5` codec 在 7 個 Unicode 官方 BIG5.TXT 標成「NO MAPPING」
的碼位改用 CP950 的映射，和微軟 Big5 產品一致：`0xA15A`、`0xA1C3`、
`0xA1C5`、`0xA1FE`、`0xA240`、`0xA2CC`、`0xA2CE`。本表照 codec 的結果，
因此這 7 個碼位也有對應。

## 檔案格式

固定寬度的小端 u16 陣列，沒有標頭：

- 每列 157 格：`0x40..=0x7E` 共 63 格，接著 `0xA1..=0xFE` 共 94 格。
- 89 列（`0xA1..=0xF9`），共 13,973 格、27,946 bytes。
- `0` 代表該碼位沒有對應；其餘是 BMP 字元。

查表不用二分搜尋：先由 lead／trail 算出格子索引，再直接讀兩個 bytes，
所以是 O(1)，也不依賴排序。所有映射都在 BMP 內，2 bytes 就夠；整份表約
27KB，直接以 `include_bytes!` 編進 `jc-core`。

## 重跑

```sh
scripts/gen-big5-table.py            # 重新產生 big5.bin
scripts/gen-big5-table.py --check    # 只比對，內容不同就 exit 1
```

輸出沒有時間戳，排序固定，重跑 diff 應為空。`jc-core` 的測試另外鎖住
SHA-256 與映射數，資料被改動時會失敗。

## 測試

`crates/jc-core/src/big5.rs` 涵蓋：

- `0xA440` → `一`、`0xA4A4` → `中`、`0xA140`／`0xA141` 邊界。
- `0xF9D5` 是最後一個有映射的碼位，`0xF9FE` 回 `None`。
- 無效碼位 `0x0000`、`0x8140`、`0xFFFF` 等回 `None`。
- 表格長度、合法字元、非零格數 13,710，以及 SHA-256。

## Port 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| — | `crates/jc-core/src/big5.rs` | n/a：Go 版沒有獨立的 Big5 對照表，這是新增的資料層 |
| — | `scripts/gen-big5-table.py` | n/a：同上，產生器也沒有 Go 對應 |
