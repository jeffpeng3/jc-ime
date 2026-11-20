# toolchain-pin：固定 Rust 工具鏈版本

本任務不移植 Go 程式，只新增工具鏈設定與 workflow 變更，因此 Go 檔案欄位全部是
n/a。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| n/a | `rust-toolchain.toml` | 新增：`channel = "1.100.0"`，`components = ["rustfmt", "clippy"]` |
| n/a | `.github/workflows/ci.yml` | rewritten：兩個 job 改用 `dtolnay/rust-toolchain@master` 加 `toolchain: 1.100.0`，linux job 另加一致性檢查 |
| n/a | `.github/workflows/release.yml` | rewritten：三個 job 同上，`toolchain: 1.100.0` |
| n/a | `docs/port/toolchain-pin.md` | 本檔案 |

## 為什麼要固定

`cargo fmt --check` 與 `cargo clippy -D warnings` 的判定會隨 rustc 版本變動：

- `rustfmt` 的預設排版規則會在新版調整，同一份原始碼在 1.9x 通過、在 1.10x
  可能要求重排。
- `clippy` 每次發版都會新增 lint 或調整既有 lint 的判定範圍，`-D warnings` 下
  任何一條新 lint 都會讓 CI 變紅。

原本本機用 `rustup update` 後的 stable、CI 用 `dtolnay/rust-toolchain@stable`，
兩邊各自追最新版，於是出現「本機 fmt 過了、CI 掛掉」或反向的落差，而且失敗
原因與程式改動無關。固定版本後，本機與 CI 執行的是同一個 `rustc`，lint 結果
可以直接比較。

注意 `Cargo.toml` 的 `rust-version = "1.90"` 是 **MSRV 下限**（相容性承諾），
不是實際使用的版本。實際版本只有 `rust-toolchain.toml` 一個來源。

## 設定方式

`rust-toolchain.toml` 放在 repo 根目錄，rustup 進入這個目錄就會自動切換：

```toml
[toolchain]
channel = "1.100.0"
components = ["rustfmt", "clippy"]
```

首次進入目錄時 rustup 會自行安裝缺少的工具鏈與 component；`rustup show` 的
「active toolchain」會顯示 `1.100.0`，`active because: overridden by
rust-toolchain.toml`。

GitHub Actions 的 `dtolnay/rust-toolchain` **不會**讀這個檔案（它的 `toolchain`
輸入是必填），所以 workflow 要把同一版再寫一次：

```yaml
      - uses: dtolnay/rust-toolchain@master
        with:
          toolchain: 1.100.0
          components: rustfmt, clippy
```

`ci.yml` 的 linux job 另有一步「工具鏈版本與 rust-toolchain.toml 一致」，用
`tomllib` 讀檔案、與 `rustc --version` 比對，不一致就讓 job 失敗。這樣漏改
workflow 時會明確報錯，而不是默默地用兩份不同版本跑。release workflow 沒有
這一步（它的目的是發佈產物，不是驗證 repo 設定），改版時仍必須一起改。

`rust-toolchain.toml` 只固定 channel 與 component，不固定 target；需要額外
target 的地方照舊寫在 workflow（linux job 的 `targets:
x86_64-pc-windows-msvc`、release windows job 的 `rustup target add
i686-pc-windows-msvc`）。

## 升級流程

1. 本機 `rustup toolchain install <新版>`，確認本機原本的落差已經浮現。
2. 改 `rust-toolchain.toml` 的 `channel`。
3. 改 **兩個** workflow：`.github/workflows/ci.yml`（linux、windows 兩個 job）
   與 `.github/workflows/release.yml`（verify、windows、linux 三個 job）的
   `toolchain:`。總共 5 處。
4. 跑一次全量 gate（見下節），把新版 fmt／clippy 造成的差異一併修掉，單獨
   一個 commit「升級工具鏈」。
5. 推上去看 CI 綠燈；release workflow 沒有排程，第一次實際驗證會落在下一個
   標籤。想先確認就把 `workflow_dispatch` 跑一次（只產 artifact，不建立 Release）。

漏改的症狀：CI 的 linux job 在「工具鏈版本與 rust-toolchain.toml 一致」這步
以 `::error::rustc ... 不一致` 失敗。

## 全量 gate

與 CI 的 linux job 相同的一組指令，在本機 repo 根目錄執行：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
rustup target add x86_64-pc-windows-msvc   # 只缺這個 target 時才需要
cargo check --workspace --all-targets \
  --exclude jc-cli --exclude jc-settings-ui \
  --target x86_64-pc-windows-msvc
cargo test --workspace
```

`jc-cli` 與 `jc-settings-ui` 的 Windows 建置需要 MSVC，Linux 上略過；這兩者的
Windows 驗證由 CI 的 windows job 負責。升級工具鏈時若只想先看 lint 差異，至少
要跑前兩條，其餘照常跑完再 commit。

## 驗證

```
rustup show
# active toolchain: 1.100.0-aarch64-unknown-linux-gnu
# active because: overridden by rust-toolchain.toml

python3 -c "import yaml;[yaml.safe_load(open(p)) for p in ['.github/workflows/ci.yml','.github/workflows/release.yml']]"
# exit 0
```

## 升級紀錄

### 1.100.0（2026-11-20）

固定版本後立刻量到一個 1.100.0 才有的 clippy 判定：

```
crates/jc-core/src/ja/live.rs:1377  needless_bool
  if trigger.boundary && self.is_boundary() { return true; }
  false
```

同一個 commit 在 `1.98.1` 上 `cargo clippy -p jc-core --all-targets -- -D
warnings` exit 0，在 `1.100.0` 上 exit 101——這正是本任務要消除的那種落差，
只是這次發生在升級當下。修法是把該段改成 `trigger.boundary &&
self.is_boundary()`；本任務不動 Rust 程式碼，因此留給後續的 lint 修正 commit。

## 沒有改動的部分

- 沒有改任何 Rust 原始碼；`Cargo.toml` 的 `rust-version` 也沒動。
- `Swatinem/rust-cache@v2` 的 key 不含 rustc 版本，但 `Cargo.lock` 或 rustc
  版本變動時 cache 會自動 miss（見 `docs/release.md`），所以不需要手動清快取。
