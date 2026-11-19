# 整合紀錄

主 checkout：`/mnt/work/jc/jc-ime`，branch `main`。
驗證指令：`cargo check --workspace --all-targets`（依整合規則，測試由控制中樞統一決定何時跑）。

## 合併紀錄

| Branch | Commit | Merge commit | `cargo check --workspace --all-targets` | 時間 |
|---|---|---|---|---|
| `c0-contracts` | `9280e26` | `aa5e32f` | 通過 | 2026-11-19（先前） |
| `w1-proto-engine` | `569b523` | `f85d20b` | 通過 | 2026-11-19 20:28 |

## 衝突與修正

- `w1-proto-engine`：只有 `Cargo.lock` 內容衝突（`jc-testdata`、`jiff`／`jiff-core`／`jiff-static`／`log`、`syn` 版本）。
  以 main 的 `Cargo.lock` 為底，交給 cargo 重新解析，`Cargo.lock` 增 231 行、刪 2 行，無需手改其他檔案。
- 沒有編譯錯誤要修，沒有改到任何 crate 的原始碼。
