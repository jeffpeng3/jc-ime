# testing：內圈與 gate 都用 nextest，doctest 另外跑

gate 的標準指令是 `cargo nextest run --workspace` 加
`cargo test --workspace --doc`。nextest 負責單元與整合測試，doctest 它完全不跑，
必須獨立補上。兩步都失敗即紅。

內圈（agent 自己驗一個小改動）用 `scripts/test-plan.sh`，從 diff 挑受影響的
crate／模組，底層是 nextest，最後補 doctest。mbx 只快取編譯，不會幫忙挑測試，
所以這一層要自己選；**test 一律不走 mbx**，nextest 與 cargo test 都直接跑。

clippy／check 則統一走 `scripts/check.sh`：有 mbx 就用共用快取，沒有就退回
cargo，`JC_NO_MBX=1` 可強制走 cargo。範圍與 gc 政策見
[`docs/port/mbx.md`](mbx.md)。

## 兩層分工

| 場景 | 指令 | 範圍 |
|---|---|---|
| 內圈 | `scripts/test-plan.sh --run` | 只跑 diff 推出來的 crate／模組 |
| 內圈（直接跑全部） | `scripts/test.sh` | 全部 nextest 加 doctest |
| clippy／check | `scripts/check.sh` | 走 mbx 共用快取；`JC_NO_MBX=1` 退回 cargo |
| gate（Linux／CI） | `cargo nextest run --workspace` + `cargo test --workspace --doc` | 全部，不准選擇性跳過 |
| 相容性守門 | `scripts/check-nextest-compat.sh` | 同一份 workspace 上比對 nextest 與 `cargo test` |

`test-plan.sh` 輸出的第一行就寫明「這不是 gate」。內圈指令通過不代表可以合併。
checker 與 Linux CI 都跑上面的 nextest gate。

Windows 的兩個測試 job 仍跑 `cargo test --workspace`。原因是 nextest 還沒在
Windows runner 的實機環境驗過能跑同一批測試（包含 Windows-only crate 與
`jc-tsf-smoke`）。驗過之前不要只看「nextest 官方支援 Windows」就切。

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

# 不挑檔案，直接跑內圈全套（nextest＋doctest）
scripts/test.sh

# 只跑單一 crate 或模組；參數原樣傳給 nextest
scripts/test.sh -p jc-core -E 'test(/session::mixed/)'
```

`cargo`、`cargo nextest`、`mbx` 都直接在 PATH 上，指令不需要再包
`mise exec --`。`~/.local/bin/cargo` 目前是 mbx 的 shim；test 子命令會直接
通過，要確定走真正的 cargo 時用 `/home/jeff/.cargo/bin/cargo`。clippy／check
走 `scripts/check.sh` 才用得到 mbx 快取。

輸出範例：

```
建議指令（nextest 跑單元＋整合，cargo test --doc 補 doctest）：
  cargo nextest run -p jc-engine
  cargo nextest run -p jc-core -E 'test(/session::mixed/)'
  cargo nextest run -p jc-core -E 'test(/session::input_style/)'
  cargo nextest run -p jc-core --test attested_cost
  cargo test -p jc-core --doc
  cargo test -p jc-engine --doc
```

## 對應規則

| 變更路徑 | 建議 |
|---|---|
| `crates/<name>/src/<file>.rs` | `nextest run -p <name> -E 'test(/<file>/)'` |
| `crates/<name>/src/<dir>/<file>.rs` | `nextest run -p <name> -E 'test(/<dir>::<file>/)'` |
| `crates/<name>/src/<dir>/mod.rs` | `nextest run -p <name> -E 'test(/<dir>/)'` |
| `crates/<name>/src/lib.rs`、`main.rs`、`src/bin/*` | `nextest run -p <name>`（整包） |
| `crates/<name>/tests/<test>.rs` | `nextest run -p <name> --test <test>` |
| `crates/<name>/tests/<test>/main.rs` | `nextest run -p <name> --test <test>` |
| `tests/<dir>/` 的其他檔案（共用 helper） | `nextest run -p <name>`（整包） |
| `Cargo.toml`、`Cargo.lock`、crate 根目錄 `build.rs` | `nextest run --workspace` |
| 任何受影響的 crate | 最後補一行 `cargo test -p <name> --doc` |
| `docs/`、`scripts/` | 不跑 Rust 測試 |
| `installer/` | 不跑 Rust 測試；另外提醒跑 `installer/validate-linux.sh` 與 `jc-tsf-smoke --json`（doctor） |
| 其他無法判斷的路徑、未追蹤的未知檔案 | `nextest run --workspace` + `cargo test --workspace --doc` |

`src/<dir>/build.rs` 是普通模組，不當成 build script。`src/` 底下非 `.rs` 的檔案
無法判斷影響範圍，直接跑 workspace。

## nextest 與 cargo test 的差異

- **每個測試獨立 process**。nextest 會為每個測試啟動新的 process，`static`、
  環境變數、panic hook、thread local 都不會跨測試共用。這比 libtest 的
  thread-per-test 隔離更強，但也表示依賴同一個 process 內共享狀態的測試會
  行為改變。目前 workspace 沒這種測試：`set_var` 只出現在 `jc-testdata`
  自己會還原環境變數的測試，以及 `jc-engine` 載入模型的正式程式碼；沒有
  `static mut`、沒有依賴 `--test-threads` 或測試順序。
- **不跑 doctest**。`nextest run` 只跑 lib／bin／integration target。doctest
  一定要另外跑 `cargo test --workspace --doc`。
- **`#[ignore]` 的統計名稱不同**。cargo test 算 `ignored`，nextest 算
  `skipped`。目前 workspace 的程式測試沒有 `#[ignore]`；唯一的 ignored 是
  `jc-testdata` 的 `require_data!` 文件範例（`crates/jc-testdata/src/lib.rs`
  第 44 行），本來就標成 `ignore`，保留不動。另有一個會執行的 doctest 在
  `jc-windows` 的 `pipe_name::name_from_sid`。
- **預設 fail-fast**。nextest 遇到第一個失敗會取消其餘測試；要完整統計用
  `--no-fail-fast`。失敗時 exit code 是 100，cargo test 是 101。兩者都會讓
  `set -e` 的腳本停下。
- **輸出格式不同**。nextest 會在失敗項目直接印測試全名、panic 位置、
  assertion 的 left／right 與 stdout／stderr 分段；定位資訊不比 cargo test 少。
- **過濾語法**。模組用 filterset：`-E 'test(/session::mixed/)'`；整合測試用
  `--test <name>`，和 cargo test 相同。

### 相容性實測（0.9.152，GB10，debug）

| 項目 | cargo test | nextest |
|---|---|---|
| 程式測試 passed | 1737 | 1737 |
| 程式測試 failed／ignored | 0／0 | 0／0（skipped 0） |
| doctest passed／ignored | 1／1 | 不跑，需另外補 |
| wall time | 550.95s | 86.67s |

同一份 `af822d5` 基準、同一台機器。nextest 快主要是把 68 個 target 的測試
排進同一批平行執行，不是少跑測試。doctest 另外跑一次約 25.6s（冷啟動）。

刻意讓一個測試失敗時，nextest 的輸出足以定位：

```
FAIL [   0.005s] (1/2) jc-core::__nextest_fail_probe fails_on_purpose
  thread 'fails_on_purpose' panicked at crates/jc-core/tests/__nextest_fail_probe.rs:10:5:
  assertion `left == right` failed: nextest failure output probe
    left: 4
   right: 5
```

## 版本釘選與相容性守門

本機用 `mise.toml` 釘住 `cargo-nextest = "0.9.152"`。CI 的
`NEXTEST_VERSION` 必須同版，linux job 會比對 `mise.toml` 與 workflow env，
不一致就紅。CI 用 `taiki-e/install-action` 安裝預編譯 binary，不從原始碼編。

每週一 03:00 UTC 的 `schedule` 會跑 `test-compat` job（也可以用
`workflow_dispatch` 手動觸發）。它先跑 nextest，再跑傳統
`cargo test --workspace --no-fail-fast`，比對 run／passed／failed／skipped
（cargo 的 ignored 對 nextest 的 skipped）與 exit code。數字分叉就紅，代表
nextest 漏跑、多跑或行為不同，不能只看兩邊都綠就放過。

平常 push／PR 的 gate 不跑 `test-compat`，只跑 nextest＋doctest；這支 job 是
長期漂移的守門，不是每次交件的負擔。

## 限制

- 模組過濾是對測試全名做 regex 子字串比對，只涵蓋名稱含該模組路徑的測試。
  整合測試不一定帶模組前綴；改 `src/` 不會自動跑 `tests/` 的每一支。
- 跨 crate 的間接影響（例如改 `jc-core` 的行為讓 `jc-engine` 測試失敗）抓不到。
  這種改動要自己加跑下游 crate，或直接跑 workspace。
- `--run` 只執行印出來的指令，不會自動加 `flock`；和其他 agent 同時跑重測試時
  仍要自己排隊。
- Windows 仍用 `cargo test`，所以 Windows 沒有 nextest 的 process 隔離與速度
  好處；等實機驗過再切。

## 測試

```sh
bash -n scripts/test-plan.sh
bash -n scripts/test.sh
bash -n scripts/check-nextest-compat.sh
scripts/test-plan-test.sh
```

`test-plan-test.sh` 只走 `--files`，不編譯、不執行 cargo。
`check-nextest-compat.sh` 會真的跑兩套完整測試，平常不要放進內圈。
