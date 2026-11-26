# testing：內圈只跑受影響測試，gate 跑全部

`cargo test --workspace` 是 gate 的標準，不能拿掉。內圈（agent 自己驗一個小改動）
改用 `scripts/test-plan.sh`，只編譯、執行受影響的 crate 與模組，省下無關測試的
執行時間。mbx 只快取編譯，不會幫忙挑測試，所以這一層要自己選。

## 兩層分工

| 場景 | 指令 | 範圍 |
|---|---|---|
| 內圈 | `scripts/test-plan.sh --run` | 只跑 diff 推出來的 crate／模組 |
| gate | `cargo test --workspace` | 全部，不准選擇性跳過 |

`test-plan.sh` 輸出的第一行就寫明「這不是 gate」。checker／CI 仍跑完整
workspace；內圈指令通過不代表可以合併。

## 用法

```sh
# 預設跟 main 的 merge-base 比，含未 commit 的變更與未追蹤檔案
scripts/test-plan.sh

# 指定 base
scripts/test-plan.sh --base origin/main

# 直接給檔案清單，不讀 git（測試與 CI 除錯用）
scripts/test-plan.sh --files crates/jc-core/src/session/mixed.rs

# 位置參數也可以
scripts/test-plan.sh crates/jc-core/src/session/mixed.rs

# 真的執行建議指令；不加 --run 只印不跑
scripts/test-plan.sh --run
```

輸出範例：

```
建議指令：
  mise exec -- cargo test -p jc-core session::mixed
  mise exec -- cargo test -p jc-core session::input_style
  mise exec -- cargo test -p jc-engine
理由：
  crates/jc-core/src/session/mixed.rs      → jc-core，模組 session::mixed
  crates/jc-core/src/session/input_style.rs → jc-core，模組 session::input_style
  crates/jc-engine/src/lib.rs               → jc-engine（整包）
  docs/port/foo.md                          → 不影響 Rust 測試
```

## 對應規則

| 變更路徑 | 建議 |
|---|---|
| `crates/<name>/src/<file>.rs` | `-p <name> <file>` |
| `crates/<name>/src/<dir>/<file>.rs` | `-p <name> <dir>::<file>` |
| `crates/<name>/src/<dir>/mod.rs` | `-p <name> <dir>` |
| `crates/<name>/src/lib.rs`、`main.rs`、`src/bin/*` | `-p <name>`（整包） |
| `crates/<name>/tests/<test>.rs` | `-p <name> --test <test>` |
| `crates/<name>/tests/<test>/main.rs` | `-p <name> --test <test>` |
| `tests/<dir>/` 的其他檔案（共用 helper） | `-p <name>`（整包） |
| `Cargo.toml`、`Cargo.lock`、crate 根目錄 `build.rs` | `cargo test --workspace` |
| `docs/`、`scripts/` | 不跑 Rust 測試 |
| `installer/` | 不跑 Rust 測試；另外提醒跑 `installer/validate-linux.sh` 與 `jc-tsf-smoke --json`（doctor） |
| 其他無法判斷的路徑、未追蹤的未知檔案 | `cargo test --workspace` |

`src/<dir>/build.rs` 是普通模組，不當成 build script。`src/` 底下非 `.rs` 的檔案
無法判斷影響範圍，直接跑 workspace。

## 限制

- 模組過濾是 `cargo test` 的測試名稱子字串比對，只涵蓋名稱含該模組路徑的測試。
  一個模組一條指令，因為 `cargo test` 只接受一個 `TESTNAME`。
  整合測試不一定帶模組前綴；改 `src/` 不會自動跑 `tests/` 的每一支。
- 跨 crate 的間接影響（例如改 `jc-core` 的行為讓 `jc-engine` 測試失敗）抓不到。
  這種改動要自己加跑下游 crate，或直接跑 workspace。
- `--run` 只執行印出來的指令，不會自動加 `flock`；和其他 agent 同時跑重測試時
  仍要自己排隊。

## 測試

```sh
bash -n scripts/test-plan.sh
scripts/test-plan-test.sh
```

案例測試只走 `--files`，不編譯、不執行 cargo。
