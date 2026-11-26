# mbx：clippy／check 的共用編譯快取

`mbx` 是 mise 的 Rust 建置快取。它把編譯結果放進全域 store，再用 hard link
接回各 worktree 的 `target/`，所以多個 worktree 跑同一組 clippy 時只留一份
產物。本專案只用它跑 **clippy／check**，不碰 test，也不進 CI。

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
worktree、反覆跑 clippy」；單一 checkout 只跑一次的情境不需要它。

## 用法

統一入口是 `scripts/check.sh`，參數原樣傳給 clippy：

```sh
scripts/check.sh --workspace --all-targets -- -D warnings
```

- 找得到 `mbx` 就用 `mbx clippy`，找不到就退回 `cargo clippy`。
- `JC_NO_MBX=1` 強制走 cargo，CI、除錯或懷疑快取污染時用。
- 執行前會印一行「使用 mbx（共用快取）」或「使用 cargo」，回報時可以確認
  到底走了哪條路。

**不要**執行 `mbx setup`（cargo shim）：那會讓所有 plain `cargo` 都改走 mbx，
變數太多。本專案只在 `scripts/check.sh` 裡明確呼叫 `mbx`。

## 不啟用的部分

- **test**：`scripts/test-plan.sh`、nextest、`cargo test` 都不走 mbx。mbx 只
  快取編譯，不會幫忙挑測試；而且測試會寫暫存檔與共用鎖，交給 mbx 管理沒有
  好處。test 的規則見 `docs/port/testing.md`。
- **CI**：workflow 不改，CI 直接用 `cargo clippy`。CI 的 cache 由 GitHub
  Actions 管，與本機 store 無關。
- **release／簽章**：見下面的 hard link 風險。

## gc 政策

`scripts/check.sh` 不做 gc：每次 clippy 都清一次沒有意義，暖快取也會被清掉。

要回收空間時：

```sh
scripts/mbx-gc.sh            # 只印 dry-run 摘要，不刪東西
scripts/mbx-gc.sh --yes      # 真的清除
```

等價於 `mbx gc --dry-run` / `mbx gc`。實測 gc 後穩定狀態是 action store
1.55 GiB、managed target 0；下一次 clippy 會從 store 重建需要的部分。

**`cargo clean` 不會釋放 mbx 的空間。** mbx 的 clippy 產物在
`target/check/<triple>/debug`（host 沒有 triple，就是 `target/check/debug`；
cargo 是 `target/<triple>/debug`），實際資料在全域 store。刪 worktree 或用
`cargo clean` 只會讓 link 消失，store 不會縮小；要回收請跑 `mbx gc`。

## hard link 風險

mbx 把 store 的物件 hard link 進各 worktree 的 `target/`。如果在 `target/`
裡**原地修改**檔案（例如對執行檔 strip、簽章、patch），改的是 store 裡的同一
份資料，會污染其他 worktree 的快取。

本專案的簽章與打包流程在 CI 的乾淨 checkout 上做，不會碰 mbx 管理的
`target/`，所以不受影響。若日後有人要在本機對 `target/check/<triple>/debug`
的產物做原地修改，請先 `JC_NO_MBX=1` 改用 cargo，或先複製到別的位置再改。
