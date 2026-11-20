# cache-prune：清理 GitHub Actions 快取

本任務只寫腳本與 workflow，沒有 Go 對應實作。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| n/a | `scripts/cache-prune.sh` | 新增：依 family／閒置時間判斷並用 `gh cache delete` 刪除 |
| n/a | `.github/workflows/cache-prune.yml` | 修改：每天排程（真刪）與手動 dispatch（預設 dry-run） |
| n/a | `.github/workflows/ci.yml` | 修改：`linux` job 加 `permissions.actions: write` 與 main 收尾清理步驟 |
| n/a | `.github/workflows/release.yml` | 修改：`linux` job 加 `permissions.actions: write` 與 main 收尾清理步驟 |
| n/a | `scripts/fixtures/cache-prune.json` | 新增：判定規則的測試資料 |
| n/a | `scripts/fixtures/cache-prune-total.json` | 新增：總量壓力的測試資料 |
| n/a | `scripts/fixtures/cache-prune-protected.json` | 新增：全部落在保護窗的測試資料 |
| n/a | `scripts/test-cache-prune.sh` | 新增：固定 `PRUNE_NOW` 的 dry-run 輸出檢查 |

## 觸發點

| 來源 | 時機 | 模式 |
|---|---|---|
| `ci.yml` 的 `linux` job | main 分支 push 後的收尾 | 真的刪 |
| `release.yml` 的 `linux` job | main 分支 `workflow_dispatch` 的收尾 | 真的刪 |
| `cache-prune.yml` 排程 | 每天 03:00 UTC | 真的刪 |
| `cache-prune.yml` 手動 dispatch | 人工 | 預設 dry-run，可取消勾選 |

清理是「收尾時順手做」，排程只補漏。原因：快取累積的速度跟 push 次數成正比，
每週跑一次會讓中間幾天持續逼近 10 GB 上限；收尾時跑一次成本只有幾秒。

`release.yml` 的標籤觸發不會清理，因為那時 `github.ref` 是 `refs/tags/v*`，不符合
`refs/heads/main`。標籤範圍的快取由每週排程的 `cache-prune.yml` 處理：`gh cache list`
會列出所有 ref 的快取，刪除只看 id，不受 ref 限制。

## 判定規則

每筆快取有 `key`、`createdAt`、`lastAccessedAt`、`sizeInBytes`。兩條規則取 **OR**：

1. **同一個 family 只留最新 N 份**（`KEEP_PER_FAMILY`，預設 2）。順序依 `createdAt`
   由新到舊，`createdAt` 相同時依 `id`。
2. **超過 T 天沒被讀取就刪**（`MAX_IDLE_DAYS`，預設 7）。

「沒被讀取」用 `lastAccessedAt`；`gh` 對從未讀取的項目給 `null`，此時退回
`createdAt`，也就是「建立後沒人用過」一律照建立時間算。

另外有一條保護：**建立未滿 N 小時的快取一律不刪**（`SKIP_RECENT_HOURS`，腳本預設 6，
workflow 用 1）。它優先於上面兩條，理由見下一節。

上面算完後還有第三段**總量規則**：若執行後預估仍超過 `MAX_TOTAL_GB`（預設 8 GB），
就從還沒刪的項目依 `createdAt` 由舊到新追加到刪除清單，直到低於門檻或沒有可刪的為止。
這段用自己的保護窗 `PROTECTED_HOURS`（預設 1 小時），不受 `SKIP_RECENT_HOURS` 影響，
因此多次發版留下的 tag 快取仍有出口。`MAX_TOTAL_GB` 設 0 或 -1 可關閉總量規則。
1 GB 以 1073741824 bytes 計，與輸出裡的 MB 口徑一致。

### 為什麼是 OR

兩條規則各自處理不同的浪費：

- 只留最新 N 份：`Cargo.lock` 每次變動就產生新 key，舊 key 即使昨天還在用，也已經
  沒有命中機會。這條把「同一組設定的歷史版本」壓到固定數量。
- 閒置 T 天：處理 family 之外的殘留，例如改過 `shared-key`、改過 workflow 檔名、
  或已刪除分支留下的快取。這些 family 可能只有一份，永遠不會被第一條挑到。

用 AND 會同時要求「不是最新」且「很久沒讀」，結果是舊 key 只要前幾天被讀過就留著，
數量還是會長；用 OR 才會真的收斂。

### 為什麼有 skip-recent-hours

`ci.yml` 與 `release.yml` 的收尾步驟和同一 workflow 的其他 job 平行執行，也可能和
同分支的另一個 run 重疊。那個 run 可能正在還原一份舊快取，或剛存完一份新快取。
直接刪會讓它下一次 miss，甚至讓正在寫入的一端收到刪除錯誤。

因此：

- 建立未滿 `SKIP_RECENT_HOURS` 小時的項目一律跳過，剛存好的快取不會被同一個 run 的
  收尾步驟刪掉。`ci.yml` 用 1 小時；6 小時曾讓 rc.4／rc.5／rc.7 的 tag 快取全部落在
  保護窗內，清理完全沒作用。
- 被跳過的項目仍會出現在「保留」表格，原因寫「建立未滿 6 小時（N 小時），暫不刪」，
  必要時會附上它原本會命中的規則，例如 `family 第 3 份 > 2`。

總量壓力是第二段保護：`PROTECTED_HOURS` 縮到 1 小時，只擋正在寫入的項目。若所有可刪項目
都在這 1 小時內，summary 會寫「全部受保護」，不會勉強刪。

這條保護不能消除所有競爭：一份建立很久、但正在被並行 run 讀取的快取仍可能被刪。
GitHub 的快取是唯讀的，讀取端在下載開始後就與快取條目無關；刪除只影響還沒開始
下載的 job，代價是那一次 miss，不會讓 build 失敗。真正無法接受時把
`SKIP_RECENT_HOURS` 調大，或把收尾清理改成只跑排程。

### 清理失敗不影響 build

`ci.yml`、`release.yml` 的清理步驟都設了 `continue-on-error: true`。權限不足、
`gh` 網路錯誤、刪除個別 id 失敗都不會讓 build 變紅；腳本本身仍會以非 0 結束並在
step summary 標出失敗筆數，方便事後追查。排程 workflow 沒有這層保護，失敗會通知，
因為那是唯一的主動清理管道。

## 手動跑一次 dry-run

用 fixture，不需要 token：

```sh
PRUNE_NOW=2026-11-20T00:00:00Z \
CACHE_JSON=scripts/fixtures/cache-prune.json \
bash scripts/cache-prune.sh --dry-run
```

對真實 repo 預覽（需要 `gh` 已登入，唯讀即可）：

```sh
GH_REPO=owner/repo CACHE_JSON= bash scripts/cache-prune.sh --dry-run
```

`GH_REPO` 有設但 `GH_TOKEN` 沒設時，`gh` 會用 `gh auth login` 存的憑證。在 Actions
裡兩者都由 workflow 提供。

從 GitHub 介面跑：Actions → Cache prune → Run workflow，`dry_run` 預設勾選，取消
勾選才會真的刪除。

## 調整保留策略

| 參數 | 預設 | 位置 | 說明 |
|---|---|---|---|
| `KEEP_PER_FAMILY` | 2（workflow 用 1） | `cache-prune.yml` 的 `env`，或呼叫時給環境變數 | 每個 family 留幾份 |
| `MAX_IDLE_DAYS` | 7 | 同上 | 幾天沒讀就刪 |
| `SKIP_RECENT_HOURS` | 6（workflow 用 1） | 同上，也可用命令列 `--skip-recent-hours N` | 太新不刪 |
| `MAX_TOTAL_GB` | 8 | 同上，也可用命令列 `--max-total-gb N` | 總量上限；0/-1 關閉 |
| `PROTECTED_HOURS` | 1 | 同上，也可用命令列 `--protected-hours N` | 總量壓力下的保護窗 |

`ci.yml` 的步驟用命令列 `--skip-recent-hours 1`，`release.yml` 仍是 6，要調整就改各別
workflow。`KEEP_PER_FAMILY` 改成 1 的取捨：同一 family 的兩份通常是「目前 lockfile」與
「上一個 lockfile」，回退版本的 run 會 miss；但留 2 份 × 四個 family 就接近 9 GB，
等於永遠貼著 10 GB 上限，因此 `cache-prune.yml` 選 1。

family 的算法是把 key 尾端的雜湊拿掉：

- 尾端 40 位以上十六進位（可帶 `-` 或 `_`）直接視為雜湊，涵蓋
  `Linux-cargo-<sha>`、`ci-linux-<sha>`、`release-linux-<sha>`。
- 尾端 16～39 位十六進位，且同時含數字與字母，也視為雜湊。`Swatinem/rust-cache`
  的實際 key 是這種長度。要求混合字元是為了不要誤切 `-2026010112000000` 這類
  純數字時間戳。

## fixture 與預期輸出

`scripts/fixtures/cache-prune.json` 有 7 筆，涵蓋：

- `Linux-cargo`：4 筆 40 位雜湊。最新兩份保留；第 3 份建立 4 小時前，被
  `SKIP_RECENT_HOURS` 擋下；第 4 份同時命中 family 與閒置兩條規則。
- `v0-rust-release-windows`：3 筆 16 位雜湊，證明 rust-cache 格式也會被歸成同一
  family。最舊那份 `lastAccessedAt` 是 `null`，證明「從未讀取」退回 `createdAt`。

`PRUNE_NOW=2026-11-20T00:00:00Z` 固定判定時間，輸出才能逐字比對：

```text
快取清理：family 上限 2 份，閒置上限 7 天，建立未滿 6 小時不動；總量上限 8 GB（0/-1 關閉），壓力下保護未滿 1 小時（現在 2026-11-20T00:00:00Z）

== 要刪除：2 筆 ==
key | 大小(MB) | 最後使用(UTC) | 判定原因
Linux-cargo-abcdef0123456789abcdef0123456789abcdef01 | 1024 | 2026-11-01 00:00 | family 第 4 份 > 2；閒置 19 天 > 7
v0-rust-release-windows-0a1b2c3d4e5f6071 | 300 | 從未讀取（建立於 2026-10-01 00:00） | family 第 3 份 > 2；閒置 50 天 > 7

== 保留：5 筆 ==
key | 大小(MB) | 最後使用(UTC) | 判定原因
Linux-cargo-3f2a1b4c5d6e7f8091a2b3c4d5e6f708192a3b4c | 500 | 2026-11-19 22:10 | 最新且近期有讀取
Linux-cargo-9e8d7c6b5a4938271605f4e3d2c1b0a998877665 | 512 | 2026-11-19 21:30 | 最新且近期有讀取
Linux-cargo-0123456789abcdef0123456789abcdef01234567 | 480 | 2026-11-19 20:30 | 建立未滿 6 小時（4 小時），暫不刪；family 第 3 份 > 2
v0-rust-release-windows-4c5d6e7f8a9b0c1d | 256 | 2026-11-19 00:00 | 最新且近期有讀取
v0-rust-release-windows-9f8e7d6c5b4a3021 | 280 | 2026-11-18 00:00 | 最新且近期有讀取

== 總結 ==
模式：dry-run（未刪除任何快取）
掃描：7 筆，3352.0 MB
待刪除：2 筆，1324.0 MB
保留：5 筆，2028.0 MB
因建立未滿 6 小時而跳過：1 筆
總量：掃描 3.27 GB，原規則後預估 1.98 GB，上限 8 GB
壓力清理：無（預估 1.98 GB 未超過 8 GB）
執行後預估：1.98 GB
```

把 `SKIP_RECENT_HOURS=0` 拿掉保護後，`Linux-cargo-0123…` 會多一條刪除，原因
`family 第 3 份 > 2`，待刪除變成 3 筆、保留 4 筆。這確認 skip-recent 只影響
「本來要刪」的項目，不會改變 family 排名。

`scripts/fixtures/cache-prune-total.json` 模擬回報的 9 筆、9.59 GB 快取：兩筆 ci 剛建立，
六筆 tag 快取落在 1～5 小時前。用 `KEEP_PER_FAMILY=1`、`PRUNE_NOW=2026-11-20T06:00:00Z`：

- 門檻 8 GB：追加刪最舊的 rc.4 windows/linux 共 2 筆、1.60 GB，執行後 7.99 GB。
- 門檻 7 GB：再刪 rc.5 windows/linux，共 4 筆、3.20 GB，執行後 6.39 GB；rc.7 仍保留。
- 門檻 0：總量規則關閉，待刪除 0 筆。

`scripts/fixtures/cache-prune-protected.json` 是 6 筆各 1.5 GB、全部未滿 1 小時的快取；
同一個 `PRUNE_NOW` 下 summary 印「全部受保護」，沒有快取被刪。

## 驗證

```sh
bash -n scripts/cache-prune.sh
bash scripts/test-cache-prune.sh
python3 -c "import yaml,glob; [yaml.safe_load(open(f)) for f in glob.glob('.github/workflows/*.yml')]; print('yaml ok')"
PRUNE_NOW=2026-11-20T06:00:00Z CACHE_JSON=scripts/fixtures/cache-prune-total.json \
  KEEP_PER_FAMILY=1 bash scripts/cache-prune.sh --dry-run
PRUNE_NOW=2026-11-20T06:00:00Z CACHE_JSON=scripts/fixtures/cache-prune-total.json \
  KEEP_PER_FAMILY=1 bash scripts/cache-prune.sh --dry-run --max-total-gb 0
```

`--dry-run` 不會呼叫 `gh cache delete`，可以重複執行；沒有要刪的項目時只印總結。
`gh cache list --limit 100` 只取最近 100 筆，超過時其他項目留給下一次，兩次之間
不會有狀態，重複執行安全。刪除中途失敗（例如別支流程先刪了同一筆）會計入失敗筆數，
下一輪自然不會再看到它。
