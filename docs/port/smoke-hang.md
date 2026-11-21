# smoke-hang：smoke test 在 Deactivate 路徑卡住的時間戳定位

Worktree：`/mnt/work/jc/wt/smoke-timestamp`（branch `smoke-timestamp`）

## 問題

真機 rc.11 砍掉引擎後跑 `jc-tsf-smoke.exe`，輸出卡在
`ITfKeystrokeMgr::TestKeyDown('u')` 超過 10 秒。靜態追蹤排除 smoke 自身路徑，
卡點指向 `checks.rs` 的 `manager.Deactivate()`：進入 msctf 後，thread manager
deactivation 同步回呼 TIP sink，而 smoke 行程沒有訊息幫浦，形成死結。引擎
活著時 TIP 狀態不同，避開這條路徑。這是 smoke 診斷工具的問題，不是輸入法
本身的問題（打字正常）。

## 做法

`smoke test 在引擎不存在時可能卡在 manager.Deactivate()，加時間戳定位中`。

在 `crates/jc-tsf-smoke/src/checks.rs` 檢查 5 尾段與檢查 6，於六個卡點前後各
印一行到 stderr，格式 `[smoke {:.3}s] <步名>`，秒數來自 `Instant::now()` 起算的
`start.elapsed()`。每行接 `std::io::stderr().flush()`，避免行程卡死時最後一行
留在緩衝區。純診斷，不改 Deactivate 順序、TIP、engine、CI、版本號、catalog、
pipe_name、transport。

| 步名 | 位置 |
|---|---|
| `before service.Deactivate()` | 檢查 5 尾段 |
| `after service.Deactivate()` | 檢查 5 尾段 |
| `before manager.Deactivate()` | 檢查 5 尾段 |
| `after manager.Deactivate()` | 檢查 5 尾段 |
| `before engine::probe()` | 檢查 6 |
| `after engine::probe()` | 檢查 6 |

## 真機怎麼看

砍掉引擎後跑 `jc-tsf-smoke.exe`，看 console 最後一筆時間戳：

- 只出現 `before manager.Deactivate()`，沒有 `after`：卡在 msctf 的
  `ITfThreadMgr::Deactivate()`，與推測一致。
- `before service.Deactivate()` 就斷：卡在 `ITfTextInputProcessor::Deactivate()`。
- 六行都出現：Deactivate 路徑正常，卡點在後面的 `engine::probe()` 或其他檢查。

`docs/CHANGELOG.md` 未動。

## 驗證

- `cargo check -p jc-tsf-smoke --all-targets`
- `cargo check --workspace --all-targets`
- `cargo clippy -p jc-tsf-smoke --all-targets -- -D warnings`
- `cargo fmt --all -- --check`
- `cargo test`：依共通規則未執行，由整合階段與 CI 跑。
