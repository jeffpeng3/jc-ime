# deps-audit：依賴樹稽核

這不是 Go 移植任務，沒有對應的 Go 實作。表格記錄稽核產出與 owned paths 的對應。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| n/a（依賴樹稽核，Go 參考實作 `/mnt/work/jc/ref/0c-ime` 沒有對應單元） | `docs/reports/deps-audit-2026-11-20.md` | 已產出 |
| n/a | `crates/jc-settings-ui/Cargo.toml` | 已移除未使用的 `log` |
| n/a | `crates/jc-englishpack/Cargo.toml` | 已移除未使用的 `sha2` |
| n/a | `crates/jc-tsf-smoke/Cargo.toml` | 已移除未使用的 `Win32_UI_Input_KeyboardAndMouse` feat |
| n/a | `crates/jc-engine/Cargo.toml`、`crates/jc-tsf/Cargo.toml` | 加註解說明看似未用但必要的 feature 與 `windows-core`，內容未變 |

## 驗證

| 指令 | 結果 |
|---|---|
| `cargo check --workspace --all-targets` | 通過 |
| `cargo check -p jc-tsf -p jc-tsf-smoke -p jc-engine --target x86_64-pc-windows-msvc` | 通過 |
| `cargo machete` | workspace 內 0 個未使用依賴（`scripts/legacy/dump-inputs` 不在 workspace） |
