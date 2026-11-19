# ci-lint：修掉 zh-symbols 之後的 clippy 錯誤

本任務不做移植，只修 lint。Go 檔案欄位對應「無」（n/a）。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| n/a | `crates/jc-core/src/emoji.rs` | rewritten：`test_pick_bounds_and_clean` 的 `for i in 0..MAX_RECENT + 10` 搭配 `FACES[i]` 改成先切 `&FACES[..MAX_RECENT + 10]` 再 `iter().enumerate()` |

## 修法

`clippy::needless_range_loop`。

原本：

```rust
for i in 0..MAX_RECENT + 10 {
    (order, stats) = pick(&order, &stats, FACES[i], DAY + i as i64);
}
```

改成：

```rust
// 先切出固定長度：FACES 不夠長時仍然在切片處 panic，與逐項索引相同。
let faces = &FACES[..MAX_RECENT + 10];
for (i, &face) in faces.iter().enumerate() {
    (order, stats) = pick(&order, &stats, face, DAY + i as i64);
}
```

`MAX_RECENT + 10` 目前是 74，`FACES` 有 120 項。切片在長度不足時會 panic，
與原本 `FACES[i]` 越界 panic 的語意相同；沒有用 `take`，避免長度不足時靜默少跑。

## 驗證結果

```
cargo fmt --all -- --check                                          # exit 0
cargo clippy --workspace --all-targets -- -D warnings               # exit 0
cargo clippy --workspace --all-targets --all-features -- -D warnings # exit 0
```
