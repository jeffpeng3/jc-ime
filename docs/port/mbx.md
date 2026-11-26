# mbx：Rust 建置的共用快取

`mbx` 是 mise 的 Rust 建置快取。它把編譯結果放進全域 store，再用 hard link
接回各 worktree 的 `target/`，所以多個 worktree 跑同一組建置時只留一份產物。

本專案主要用它跑 **clippy／check**；啟用 shim 之後，`cargo build`、`cargo
nextest`、`cargo test --doc` 也會經過 mbx。mbx 只快取編譯，不會改變測試的
挑選與判準，也不會進 CI。

## 量測

同機、4 個 worktree、`--workspace --all-targets`：

| 情境 | mbx | cargo |
|---|---|---|
| 冷 store、4 個 worktree | 7.71 GiB | 7.22 GiB |
| 暖 store、4 個 worktree | **2.15 GiB** | 7.22 GiB |
| 暖 store、第 2 個之後的 worktree（邊際） | 約 61 MiB | 每個都重新編譯 |

暖 store 的命中時間：

| 指令 | cargo | mbx |
|---|---|---|
| `clippy --workspace --all-targets` | 24s | **1.7s** |
| Windows cross target | 18s | **1.3s** |

冷 store 沒有好處（甚至略大），所以 mbx 的價值在「同一台機器、多個
worktree、反覆跑同一組建置」；單一 checkout 只跑一次的情境不需要它。

## 全域啟用

`mbx setup --global` 會把 `[wrappers.cargo]` 寫進 mise 設定，但那條路在
`mise exec -- cargo` 不會生效。實際用法是把 mbx 的 shim 目錄加進 PATH：

```toml
# ~/.config/mise/config.toml
[env]
_.path = ["~/.local/share/mbx/bin"]
```

原因：`_.path` 由 mise 在每個 `mise exec` 環境開頭注入，會排在
`command-wrappers/bin` 前面，因此 `mise exec -- cargo ...` 直接命中
`~/.local/share/mbx/bin/cargo`。`[wrappers.cargo]` 只覆蓋部分進入方式，
在 `mise exec` 下會被略過，所以不能只靠它。

驗證 shim 有生效：

```sh
mise exec -- cargo build -p jc-proto
# 結尾應該出現 mbx[cache]: ... 或 mbx[savings]: ...
```

`MBX_DISABLE=1` 是 shim 層級的逃生口，對任何 `cargo` 指令都有效：

```sh
MBX_DISABLE=1 cargo nextest run -p jc-core
MBX_DISABLE=1 cargo build -p jc-engine
```

### `JC_NO_MBX=1` 與 `MBX_DISABLE=1` 的關係

- `MBX_DISABLE=1`：真正的繞過。shim 看到它就把指令原樣交給系統 cargo。
- `JC_NO_MBX=1`：只給 `scripts/check.sh` 用。腳本會加上
  `MBX_DISABLE=1` 再 `exec cargo clippy`，所以即使 `cargo` 本身是 shim，也
  會走真正的 cargo。沒有 mbx（或沒有 shim）時，`check.sh` 直接退回 cargo。
- `scripts/test.sh`、`scripts/test-plan.sh`、`scripts/verify-keybinds.sh`
  沒有專屬開關；要繞過就在指令前自己加 `MBX_DISABLE=1`。

## 用法

clippy 的統一入口是 `scripts/check.sh`，參數原樣傳給 clippy：

```sh
scripts/check.sh --workspace --all-targets -- -D warnings
```

- 找得到 `mbx` 就用 `mbx clippy`，找不到就退回 `cargo clippy`。
- `JC_NO_MBX=1` 會用 `MBX_DISABLE=1 cargo clippy` 強制走真正的 cargo，CI、
  除錯或懷疑快取污染時用。
- 執行前會印一行「使用 mbx（共用快取）」或「使用 cargo（...）」，回報時可以
  確認到底走了哪條路。

test 的入口是 `scripts/test.sh`（nextest 加 doctest）與
`scripts/test-plan.sh --run`。它們不需要特別呼叫 mbx；全域 shim 生效時，
底層的 `cargo nextest run` 與 `cargo test --doc` 會自動經過 mbx。輸出多出的
`mbx[cache]`、`mbx[savings]`、`mbx[progress]` 行不影響測試結果解析；真的不想
看到就加 `MBX_DISABLE=1`，但內圈預設不關。

## 不啟用的部分

- **CI**：workflow 不改，CI 直接用 `cargo clippy`／`cargo nextest`。CI 的
  cache 由 GitHub Actions 管，與本機 store 無關；Windows runner 也沒有 mbx。
- **release／簽章**：在 CI 的乾淨 checkout 上做，不碰本機 mbx 的 store。
- **測試判準**：mbx 只快取編譯，不挑測試、不改變 nextest 與 doctest 的範圍。

## gc 政策

`scripts/check.sh` 不做 gc：每次 clippy 都清一次沒有意義，暖快取也會被清掉。

要回收空間時：

```sh
scripts/mbx-gc.sh            # 只印 dry-run 摘要，不刪東西
scripts/mbx-gc.sh --yes      # 真的清除
```

等價於 `mbx gc --dry-run` / `mbx gc`。實測 gc 後穩定狀態是 action store
1.55 GiB、managed target 0；下一次建置會從 store 重建需要的部分。

**`cargo clean` 不會釋放 mbx 的空間。** clippy 產物在
`target/check/<triple>/debug`（host 沒有 triple，就是 `target/check/debug`；
cargo 是 `target/<triple>/debug`），build／test 產物仍在標準的
`target/debug`、`target/<triple>/debug`。這些路徑多半是 hard link，實際資料
在全域 store；刪 worktree 或用 `cargo clean` 只會讓 link 消失，store 不會
縮小。合併後回收 worktree 時，記得跑 `mbx gc`（或 `scripts/mbx-gc.sh`
先看 dry-run）把失去 link 的物件清掉。

## hard link 風險

mbx 把 store 的物件 hard link 進各 worktree 的 `target/`。如果在 `target/`
裡**原地修改**檔案（例如對執行檔 strip、簽章、patch），改的是 store 裡的同一
份資料，會污染其他 worktree 的快取。clippy 的 `target/check/...`、build 的
`target/debug`、nextest 的測試執行檔都可能有這種 link。

本專案的簽章與打包流程在 CI 的乾淨 checkout 上做，不會碰 mbx 管理的
`target/`，所以不受影響。若日後有人要在本機對 `target/` 的產物做原地修改，
請先：

```sh
MBX_DISABLE=1 cargo build ...
```

或先複製到別的位置再改。`JC_NO_MBX=1` 只涵蓋 `scripts/check.sh`，不是通用的
繞過開關。
