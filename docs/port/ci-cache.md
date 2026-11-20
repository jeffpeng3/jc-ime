# ci-cache：CI 與 release 的建置快取

本任務不移植 Go 程式，只改 GitHub Actions workflow 與文件，因此 Go 檔案欄位全部是
n/a。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| n/a | `.github/workflows/ci.yml` | rewritten：兩個 job 改用 `Swatinem/rust-cache@v2`，移除原本的 `actions/cache` registry 區塊 |
| n/a | `.github/workflows/release.yml` | rewritten：三個建置 job 加入 `Swatinem/rust-cache@v2`，`verify` 與 `linux` 共用 `release-linux` |
| n/a | `docs/release.md` | rewritten：新增「建置快取」章節 |
| n/a | `docs/port/ci-cache.md` | rewritten：本檔案 |

## 分組

| Workflow | Job | Runner | Profile | shared-key |
|---|---|---|---|---|
| `ci.yml` | `linux` | ubuntu-24.04 | debug | `ci-linux` |
| `ci.yml` | `windows` | windows-2022 | debug | `ci-windows` |
| `release.yml` | `verify` | ubuntu-24.04 | release | `release-linux` |
| `release.yml` | `windows` | windows-2022 | release | `release-windows`（另有 `key: x86_64-pc-windows-msvc`） |
| `release.yml` | `linux` | ubuntu-24.04 | release | `release-linux` |

`verify` 與 `linux` 共用 `release-linux`：兩個 job 同為 ubuntu-24.04、同為 release
profile、同一次 workflow run、同一份 `Cargo.lock`，編出來的依賴產物完全一樣。`verify`
先跑並負責寫入快取，`linux` 直接命中。CI 的兩個 job 是 debug profile，與 release 不同，
不共用。

## 快取能共用的條件

`Swatinem/rust-cache` 的 key 由這幾項組成：`shared-key`、rustc 版本（release／host／
hash）、`Cargo.toml`／`Cargo.lock` 雜湊、`rust-toolchain*`、`.cargo/config.toml`、
以及環境變數前綴 `CARGO CC CFLAGS CXX CMAKE RUST` 的值。除此之外的差異要靠
`shared-key` 自己分。

實際能共用的是「同 OS ＋ 同 profile ＋ 同 `Cargo.lock` ＋ 同 rustc」：

- Rust 的 `target/debug` 與 `target/release` 是不同產物。`debug` 帶除錯資訊、
  `debug_assert` 開啟、整數溢出會 panic；`release` 最佳化、檢查關閉。兩邊的 `.rlib`
  不能互換，cargo 也不會嘗試重用。
- Linux 與 Windows 的產物格式不同（ELF vs PE/COFF），不可能共用。
- `Cargo.lock` 或 rustc 版本一變，key 就變，會重新編譯；這是預期行為。
- Windows 的 MSI job 指定 `--target x86_64-pc-windows-msvc`，cache key 另外用
  `key` 標明，避免和同一 runner 上的其他 Windows 快取混淆。

若之後想讓 CI 與 release 共用同一份快取，唯一辦法是 CI 也改用
`cargo test --release`，並讓 CI 的 job 使用 `release-linux`／`release-windows` 這組
shared-key。代價是 CI 變慢（要先編 release），而且 `debug_assert` 與整數溢位檢查不會
生效，測試抓到的問題會少一些。目前不做。

## 只快取依賴，不快取 workspace 產物

`cache-targets: "true"` 會快取 `target/` 目錄，但 `rust-cache` 存檔前會清掉 workspace
crate 的編譯產物與 incremental 目錄，只留依賴。`cache-workspace-crates` 保持預設
`false`。

不快取 workspace crate 的原因：快取到過期的自家 binary 會讓 CI 跑舊程式；而且每次
commit 都變，命中率低。代價是自家 crate 每次重編，本專案的依賴（candle、GPUI）遠大於
自家程式碼，這樣最划算。

`rust-cache` 也會自動設 `CARGO_INCREMENTAL=0`；`ci.yml` 本來就設了，不衝突。

## registry 快取只留一份

原本 `ci.yml` 兩個 job 各有 `actions/cache@v4` 快取 `~/.cargo/registry` 與
`~/.cargo/git`，已移除。`rust-cache` 預設就會快取整個 `~/.cargo`（registry cache、
git 依賴、已安裝的 binary），兩者並存只會同一批檔案下載兩次、上傳兩份、佔兩份額度。

`~/.cargo/registry/src` 不在快取內，這是上游刻意的：從 `registry/cache` 的壓縮檔還原
比快取解壓後的原始碼快。

## 命中時機

第一次跑一定不會命中，這是正常現象：

1. 第一次：`Cache not found for input keys` → job 照常編譯 → job 結束後的 post step
   存檔，log 出現 `Cache saved with key: ...` 與 `Cache Size: ~NNN MB`。
2. 第二次（key 相同）：`Cache restored from key: ...`，依賴跳過重編，log 會有
   `Cache hit occurred on the primary key ...`。

要看 hit／miss，開一次 run 的 log，找這幾行：

```text
Cache not found for input keys: ...
Cache restored from key: ...
Cache saved with key: ...
Cache Size: ~NNN MB (NNNNNNNN B)
```

`rust-cache` 的 cache key 含 `Cargo.lock` 雜湊，所以改依賴的 commit 會 miss 一次，
之後再命中。改 rustc 版本（`dtolnay/rust-toolchain@stable` 換版）也會 miss。

同一組 `shared-key` 的兩個 job 在同一次 run 都想存同一把 key 時，GitHub 的 cache 不可
覆寫：先存的那份留著，後存的會在 post step 印警告但不影響 job。`verify` 一定先於
`linux`，所以寫入者是 `verify`。

## tag 上的快取範圍

GitHub 的快取綁 ref：一個 workflow run 只能還原「目前 ref」與「預設分支（`main`）」的
快取，不同 tag 之間互不相通。因此：

- `release-linux`／`release-windows` 的快取只在同一次 run 內（`verify` → `linux`）、
  重跑同一個 tag、或同一分支的 `workflow_dispatch` 之間命中。
- 從 `v0.1.0` 升到 `v0.1.1` 時，新的 tag 讀不到舊 tag 的快取，除非 `main` 上有相同
  key 的快取。
- `release.yml` 的建置一律是 release profile，`ci.yml` 是 debug，所以 release job
  也不會命中 `ci-*` 的快取。

若要讓每個新 tag 都吃到現成快取，需要另外在 `main` 上定期跑一次同樣的 release 建置
來預熱（或在 `ci.yml` 加一個 release profile 的 job，共用 `release-linux`／
`release-windows`）。目前沒做，release 的快取只在重跑時省時間；日常 push／PR 由
`ci-*` 受益，那才是次數最多的路徑。

## 大小上限

GitHub 每個 repo 的快取總量預設 10 GB，超過就從最久沒被存取的開始淘汰；超過 7 天沒被
存取的項目也會自動刪除。淘汰會造成 thrashing：剛存完又被擠掉，下次還是 miss。

先量再調。看目前用量：repo → Settings → Actions → Caches，或

```sh
gh api repos/{owner}/{repo}/actions/caches --paginate \
  --jq '.actions_caches[] | "\(.size_in_bytes/1048576|floor) MB\t\(.key)"'
```

判準與處理順序：

1. 總量在 10 GB 以下：維持現況，五個 job 都開 `cache-targets`。
2. 接近或超過 10 GB：先把最大的那一組改成 `cache-targets: "false"`。這樣只快取
   `~/.cargo`（registry／git，通常數百 MB），依賴每次重編，但仍省下載。
   取捨是建置時間回到接近現在的水準，只省網路。
3. 只想留最近狀態：對變動最頻繁的 CI job 加 `save-if`，例如只讓 `main` 寫入、
   PR 唯讀還原。
4. 已經爆量時，用上面 `gh api` 找出舊 key，`gh api -X DELETE
   repos/{owner}/{repo}/actions/caches/<id>` 清掉，再重跑。

不要用「開 `cache-workspace-crates`」來提高命中率：快取會變大，而且會拿到過期的自家
binary。也不要快取 `target/` 以外的路徑。

## 驗證

```sh
python3 -c "import yaml;[yaml.safe_load(open(p)) for p in ['.github/workflows/ci.yml','.github/workflows/release.yml']]"
```

實際命中率與時間改善由下一次 CI／release 驗證：第一次 miss，第二次開始變快。
