# 整合紀錄

主 checkout：`/mnt/work/jc/jc-ime`，branch `main`。
驗證指令：`cargo check --workspace --all-targets`（依整合規則，測試由控制中樞統一決定何時跑）。

## 合併紀錄

| Branch | Commit | Merge commit | `cargo check --workspace --all-targets` | 時間 |
|---|---|---|---|---|
| `c0-contracts` | `9280e26` | `aa5e32f` | 通過 | 2026-11-19（先前） |
| `w1-proto-engine` | `569b523` | `f85d20b` | 通過 | 2026-11-19 20:28 |
| `w1-data-ja` | `39daa6d` | `3da7a73` | 通過（修正後） | 2026-11-19 20:31 |
| `w1-eval` | `776ac8c` | `dd00506` | 通過 | 2026-11-19 20:38 |

## 衝突與修正

- `w1-proto-engine`：只有 `Cargo.lock` 內容衝突（`jc-testdata`、`jiff`／`jiff-core`／`jiff-static`／`log`、`syn` 版本）。
  以 main 的 `Cargo.lock` 為底，交給 cargo 重新解析，`Cargo.lock` 增 231 行、刪 2 行，無需手改其他檔案。
- 沒有編譯錯誤要修，沒有改到任何 crate 的原始碼。

- `w1-data-ja`（合併 `3da7a73`，修正 `3a3faca`）：
  - 衝突：只有 `Cargo.lock`，以 main 版為底讓 cargo 重新解析。
  - `crates/jc-data/src/lib.rs`：補 `pub mod ja;`（c0 沒宣告）。
  - `crates/jc-data/Cargo.toml`：新增 dev-dependency `jc-testdata = { path = "../jc-testdata" }`。
  - `crates/jc-data/src/ja/tests.rs`：移除本地 `testdata_root`／`testdata_path`／`require_data!`，改用 `jc_testdata::require_data!`。
  - 已對 `jc-data` 跑 `cargo fmt`。

- `w1-eval`（合併 `dd00506`）：無衝突，`Cargo.lock` 由 git 自動合併；沒有編譯錯誤要修，未改任何程式碼。
