# 發佈流程

`release.yml` 負責 release 建置與 GitHub Release；日常 CI（`ci.yml`）仍然只做
debug 測試與 clippy。兩邊刻意分開：發佈前唯一一次 release 測試在 `verify` job。

## 觸發方式

| 觸發 | 行為 | 建立 GitHub Release |
|---|---|---|
| push tag `v<version>` | `verify` → `windows` → `linux` → `publish` | 是 |
| Actions → Release → Run workflow | `verify` → `windows` → `linux` | 否，只留 artifact |

手動觸發可以填 `data_bundle_url`、`data_bundle_sha256` 覆寫 repo secret／variable，
適合第一次試跑或換資料包來源。手動觸發的版本號取自 `Cargo.toml`。

## 版本號規則

- 唯一來源：`Cargo.toml` 的 `[workspace.package] version`。
- 標籤必須是 `v<version>`，而且與 `Cargo.toml` 完全相同；不同就讓 `verify` 失敗。
- 版本格式是 `x.y.z` 或 `x.y.z-<pre-release>`，例如 `0.1.0`、`0.1.0-rc.1`。
- Release 標題、artifact 名稱、MSI 檔名都用完整版本：
  - `jc-ime-<version>-x64.msi`
  - `jc-ime-<version>-linux-x86_64.tar.gz`
- 手動觸發不會建立 Release；要發佈仍要 push 對應的 tag。
- pre-release 版本的 GitHub Release 會標成 pre-release，不會蓋掉 Latest 版。

### pre-release 版本規則

MSI 的 `ProductVersion` 只接受三段數字，但發佈版本可以有 pre-release 後綴。
兩者分開處理：

| 用途 | 值 | 來源 |
|---|---|---|
| git tag | `v0.1.0-rc.1` | 人工建立 |
| `Cargo.toml` version | `0.1.0-rc.1` | 必須與 tag 去掉 `v` 相同 |
| Release 標題／artifact 名稱／MSI 檔名 | `0.1.0-rc.1` | 完整版本 |
| MSI `ProductVersion` | `0.1.0` | `verify` 推導後傳給 `build.ps1 -MsiVersion` |

規則：

- `verify` 用 `^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$` 驗完整版本，接受 `-rc.1`。
- `msi_version` 取完整版本 `-` 之前的三段數字；`verify` 輸出這個值，Windows
  job 用 `-MsiVersion` 傳進 `installer/build.ps1`。
- `build.ps1` 的 `-MsiVersion` 是選項。沒給時由 `Cargo.toml` 版本去掉
  pre-release 自己推導，本機建置不必手動指定。
- `wix build -d Version=` 只拿 MSI 版本；檔名、Release 標題與說明仍用完整版本。
- MSI 升級判斷看 `ProductVersion`，所以 `0.1.0-rc.1` 與 `0.1.0-rc.2` 在
  Windows 眼中是同一個版本，會走「同版本就地升級」。RC 之間不需要先移除舊版。
- 正式版 `0.1.0` 的 `ProductVersion` 也是 `0.1.0`；從最後一個 RC 升到正式版
  同樣是同版本升級，安裝後不會出現兩個項目。
- 版本一致性檢查沒有放寬：tag 去掉 `v` 仍必須等於 `Cargo.toml` 的完整版本。

發布 RC 的流程與正式版相同，只是 tag 帶後綴：

```sh
git tag v0.1.0-rc.1
git push origin v0.1.0-rc.1
```

## 前置設定

### 1. 資料包（必要）

資料與模型不進 git，Windows runner 也沒有 `/mnt/work/jc/jc-ime-data`。MSI 需要
資料根目錄，因此 release workflow 會下載一個預先打包的資料包。沒有資料包 URL
時 `windows` job 會直接失敗，不會產出缺資料的 MSI。

設定下列其中一個來源，優先序是 workflow_dispatch 輸入、repo secret、repo
variable：

- repo secret `JC_DATA_BUNDLE_URL`（建議）
- repo variable `JC_DATA_BUNDLE_URL`
- `workflow_dispatch` 輸入 `data_bundle_url`

`JC_DATA_BUNDLE_SHA256` 同樣支援 secret、variable 與輸入，建議設成 repo secret。
workflow 下載後會實際算 SHA-256；不一致就讓 `windows` job 失敗，錯誤訊息會印出
預期與實際值。沒有提供這個值時只警告並繼續，之後仍由 `jc-cli data verify` 驗
manifest 內每個檔案的雜湊。正式發版建議一定要鎖 SHA-256。

資料包格式可以是 `.tar.zst`、`.tzst`、`.tar.gz`、`.tgz` 或 `.zip`，內容是資料
根目錄；上層多一層 `jc-ime-data/` 也可以，workflow 會自動找有 `manifest.json`
的那一層。Windows runner 的 `tar` 若不支援 zstd，workflow 會退回 7-Zip；兩者都
沒有時請改用 `.tar.gz`。

用 `scripts/data-bundle.sh` 打包。它只收 manifest 的 `shipped: true` 項目與
`manifest.json`，不會把 Funmula 的 `models/zh-select`、`models/zh-punct` 放進去：

```sh
# 1. 產生完整 manifest。資料根目錄可以唯讀，manifest 放 repo 外。
cargo run -p jc-cli --release -- data manifest /mnt/work/jc/jc-ime-data \
  --out /tmp/opencode/jc-ime-data-manifest.json

# 2. 打包。腳本會先印檔案清單、總大小與 bundle SHA-256。
JC_DATA_MANIFEST=/tmp/opencode/jc-ime-data-manifest.json \
OUT_DIR=/tmp/opencode \
scripts/data-bundle.sh /mnt/work/jc/jc-ime-data
```

腳本用 `tar --sort=name`、固定 mtime、固定 UID/GID 與 `gzip -n`／`zstd -n`
讓同一份 manifest 產生相同 SHA-256。沒有 `zstd` 時會退回 `.tar.gz` 並在輸出
說明。檔名預設是 `jc-ime-data-<manifest version>.tar.zst`。

workflow 會把 `-Manifest <資料根>/manifest.json` 傳給 `build.ps1`，直接複製這份
對完整資料根目錄產生的 manifest；因此安裝包會保留 `shipped: false` 的下載項與
URL，但 payload 仍只複製 `shipped: true` 的檔案，Funmula 權重不會進 MSI 或
Release。如果沒有 `-Manifest`，`build.ps1` 會退回原本行為，用
`jc-cli data manifest` 重新掃描資料根目錄；那種情況資料包必須含下載項，否則
安裝後的 `data download` 會找不到 URL。

### 2. 資料 bundle 託管

建議把 bundle 放在 Hugging Face 的 `jeffpeng3/jc-ime-data` repo。若是 dataset
repo，URL 前面要加 `/datasets`。用固定 commit 的 URL，不要用 `main` 或可移動的
tag：

```text
https://huggingface.co/jeffpeng3/jc-ime-data/resolve/<commit>/jc-ime-data-<data-version>.tar.zst
```

HF 會把 `resolve` 轉到 CDN，`Invoke-WebRequest` 會跟隨 redirect。`<commit>` 是
上傳後的 commit SHA，內容不會因為之後 push 而改變。workflow 目前不帶 HF token，
所以 repo 必須允許 runner 匿名讀取；私有 repo 要先公開或另外加認證。

資料版本命名規則：

- 版本來源是 manifest 的 `version`，預設是產生當天的 UTC 日期 `YYYY-MM-DD`。
- 檔名用 `jc-ime-data-<data-version>.tar.zst`，版本字元只留英數、`.`、`_`、`-`。
- 同一天要換資料時，重新產生 manifest 並指定遞增版本，例如
  `--version 2026-11-20.2`，不要覆蓋同名的舊 bundle。
- HF 上的 commit message 或 tag 可以用 `data-<data-version>` 方便人看，但 workflow
  的 URL 必須指向 commit。

資料變更時的步驟：

1. 用新的 `--version` 重新產生 manifest，確認 `shipped: true` 清單正確。
2. 跑 `scripts/data-bundle.sh`，記下輸出的 SHA-256、檔案大小與資料版本。
3. 上傳到 `jeffpeng3/jc-ime-data`，取得上傳後的 commit SHA。
4. 更新 repo secret `JC_DATA_BUNDLE_URL` 為
   `https://huggingface.co/jeffpeng3/jc-ime-data/resolve/<commit>/jc-ime-data-<data-version>.tar.zst`。
5. 更新 repo secret `JC_DATA_BUNDLE_SHA256` 為腳本印出的 SHA-256。
6. 手動觸發 Release workflow 驗一次，確認 log 沒有「跳過資料包雜湊檢查」的警告，
   且 MSI 內的 `data/manifest.json` 版本正確。
7. 如果資料版本會影響使用者，更新 `docs/CHANGELOG.md` 與相關資料文件。

平常程式改版不需要換 bundle。只有 shipped 檔案內容、`shipped` 清單、manifest
版本或壓縮方式改變時才需要重新打包並更新上面兩個 secret。只改 Rust 程式、文件
或 CI 時，重用同一個 commit URL 與 SHA-256 即可。bundle 一旦上傳就不要原地重壓；
重壓會產生新檔與新 SHA，應視為新的資料版本。

### 3. 簽章（選用）

沒有簽章 secret 時流程照常完成，Release 說明會標「未簽章」。要開啟簽章，設定：

| Secret | 內容 |
|---|---|
| `SIGNING_PFX_BASE64` | PFX 檔的 base64（單行，不要換行） |
| `SIGNING_PFX_PASSWORD` | PFX 密碼；沒有密碼可留空 |

```sh
base64 -w0 jc-ime.pfx > jc-ime.pfx.b64
# 或 macOS：openssl base64 -A -in jc-ime.pfx -out jc-ime.pfx.b64
```

有 `SIGNING_PFX_BASE64` 時，workflow 會把 PFX 寫到 runner 暫存目錄，找 Windows
SDK 的 x64 `signtool.exe` 加進 PATH，然後設定 `JC_SIGN_CERT`／`JC_SIGN_PASSWORD`
給 `installer/build.ps1`。`build.ps1` 會簽 payload 內的 `jc-tsf.dll`、
`jc-engine.exe`、`jc-settings-ui.exe` 等執行檔，最後再簽 MSI。時間戳預設用
`http://timestamp.digicert.com`。

要關閉簽章，刪掉 `SIGNING_PFX_BASE64` secret。若 secret 存在但找不到
`signtool.exe` 或簽章失敗，流程會失敗；這是刻意的，避免把「以為簽了」的檔案
發出去。

驗證簽章（Windows）：

```bat
signtool verify /pa /v jc-ime-<version>-x64.msi
```

## Job 內容

| Job | 產出 |
|---|---|
| `verify` | 檢查 tag／`Cargo.toml` 版本一致且符合 `x.y.z` 或 `x.y.z-<pre-release>`，輸出完整版本與 `msi_version`；跑 `cargo test --workspace --release` |
| `windows` | 下載資料包並比對 SHA-256 → `cargo run -p jc-cli -- data verify`（只建 `jc-cli`）→ 安裝 WiX v5 → `installer/build.ps1 -MsiVersion <x.y.z>`（建其餘 crate）→ MSI、checksum、`data-manifest.json` |
| `linux` | 安裝 GPUI 系統依賴 → `cargo build --release -p jc-engine -p jc-cli -p jc-settings-ui` → 打包 tar.gz 與 checksum |
| `publish` | 只在 tag push 執行；下載兩個 artifact，建立或更新 GitHub Release |

Windows 建置必須指定 target（`--target x86_64-pc-windows-msvc`），否則 `build.ps1` 的 payload 路徑會不符；x64 找不到 `target/<triple>/release` 時會退回 `target/release`，並印出警告與實際路徑。

`+crt-static` 是 Windows 出貨條件，TSF DLL 不能依賴 `vcruntime140.dll`。旗標統一放在 `.cargo/config.toml` 的 `[target.x86_64-pc-windows-msvc]` 與 `[target.i686-pc-windows-msvc]`；workflow 的 `cargo run` 與 `build.ps1` 共用同一組 rustflags，不會因 `RUSTFLAGS` 變動而全量重編。

Windows 的 Python 輸出必須是 UTF-8。Windows job 設 `PYTHONUTF8=1`，`build.ps1` 另外設 `PYTHONIOENCODING=utf-8`，避免 Python 3.12 的 cp1252 讓 `check-payload.py` 印中文時失敗。

`release.yml` 不跑 `cargo fmt`、clippy；那些由 `ci.yml` 在日常 push／PR 負責。

## 建置快取

release 與 CI 都用 `Swatinem/rust-cache@v2`。

| Workflow | Job | shared-key | cache-all-crates |
|---|---|---|---|
| `ci.yml` | `linux` | `ci-linux` | `true` |
| `ci.yml` | `windows` | `ci-windows` | `true` |
| `release.yml` | `verify` | `release-linux` | `false` |
| `release.yml` | `linux` | `release-linux`（與 `verify` 共用） | `false` |
| `release.yml` | `windows` | `release-windows` | `false` |

CI 連 workspace crate 的產物一起快取：每個 commit 通常只改幾個 crate，還原後 cargo 只
重編改到的。release 只快取依賴，release 產物一份太大，每個 tag 存一份會擠掉其他快取。

`verify` 與 `linux（tar.gz）` 同為 ubuntu-24.04、同為 release profile、同一次 run、
同一份 `Cargo.lock`，因此共用 `release-linux`：`verify` 先跑並寫入快取，打包 job
直接命中。CI 是 debug profile，`target/debug` 與 `target/release` 的產物不能互換，
所以 `ci-*` 與 `release-*` 不共用。

第一次跑一定不會命中，這是正常的：job 照常編譯，結束後才存檔。CI 的第一個 commit 存
一份新快取；之後的 commit key 不變（key 不含 commit SHA），還原同一份再增量重建，後續
存檔會被跳過（cache 不可覆寫，post step 只印 warning）。`Cargo.lock` 或 rustc 版本
一變，key 才改變、存新的一份。

確認命中與否，開 run 的 log 找這幾行：

```text
Cache not found for input keys: ...      # 第一次，miss
Cache restored from key: ...             # 之後，hit
Cache saved with key: ...                # 存檔
Cache Size: ~NNN MB (NNNNNNNN B)         # 這次存了多大
```

`release-*` 的快取綁 tag：不同 tag 互不相通，`v0.1.1` 讀不到 `v0.1.0` 存的快取，只有
重跑同一個 tag 才省得到。

快取上限是每個 repo 10 GB（超過按最久未存取淘汰，7 天沒用到自動刪）。本機 debug 建置
量到 38 個測試執行檔共 4.5 GB，CI 的快取估計每個 OS 2–4 GB（壓縮後），Linux 加 Windows
可能吃掉一半以上的額度。接近上限時先把 `ci-windows` 的 `cache-all-crates` 改成
`"false"`，再不夠才關 `ci-linux` 的；release 的三個 job 維持只快取依賴。細節與清理
指令見 `docs/port/ci-cache.md`。

## Artifact 內容

### Windows

| 檔案 | 內容 |
|---|---|
| `jc-ime-<version>-x64.msi` | per-machine 安裝包：五個程式與 `jc-tip-register.exe`（共六個檔案）、資料 manifest 與 5 個 shipped 資料檔 |
| `jc-ime-<version>-x64.msi.sha256` | MSI 的 SHA-256 |
| `data-manifest.json` | 安裝包內資料 manifest 的副本，publish 用它填 Release 的資料版本 |

MSI 的完整建置與 ICE 驗證只在 Windows runner 做；本機 Linux 沒有 Windows
runner，能做的只有 WiX 語法驗證：

```sh
bash installer/validate-linux.sh
```

這支腳本需要 `dotnet tool install --global wix --version 5.*`，而且只到編譯與
符號解析，綁定階段會因 Linux 沒有 `msi.dll` 停止，是預期結果。

### Linux

```text
jc-ime-<version>-linux-x86_64/
  bin/jc-engine
  bin/jc-cli
  bin/jc-settings-ui
  LICENSE
  NOTICE
  legal/
  docs/install.md
```

tar.gz 旁附 `.sha256`。Linux 版沒有 Fcitx5／IBus 整合，`jc-settings-ui` 執行時
需要 X11／Wayland、libxkbcommon 與 fontconfig；發佈前在乾淨的桌面環境跑一次
`ldd` 或實際開窗確認。

## Release 說明

`publish` 會產生並附上：

- 版本與發佈日期（UTC）；版本有 pre-release 後綴時，另外註明 MSI
  `ProductVersion` 只有三段數字
- 模型與資料版本：安裝包內 `manifest.json` 的 `version`（涵蓋詞典與模型）
- `docs/CHANGELOG.md` 中 `## <version>` 的段落
- 兩個平台的下載檔名與 `.sha256` 提示
- 簽章狀態：已簽章，或明確標「未簽章」與 SmartScreen／AppLocker 影響
- 已知限制：x86 TIP 未支援、MSI 需要管理員、注音選字模型需另外下載、Linux
  沒有桌面整合、設定變更需重啟引擎

Release 已存在時會更新標題與說明，並用 `--clobber` 覆蓋 assets，方便重跑失敗
的發佈。Release 標題就是 `<version>`，tag 是 `v<version>`。版本含 `-` 時一律
建立成 pre-release，避免 RC 蓋掉 Latest 正式版。

## 發版步驟

1. 更新 `docs/CHANGELOG.md`，確認有 `## <version>` 段落。
2. 改 `Cargo.toml` 的 `[workspace.package] version`；必要時讓 `Cargo.lock` 跟著更新。
   RC 就寫 `0.1.0-rc.1`，不用為了 MSI 改成三段數字。
3. 跑 `cargo check --workspace --all-targets`，commit 到 `main` 並 push。
4. 建立並 push 標籤：

   ```sh
   git tag v0.1.0
   git push origin v0.1.0
   ```

   RC 版本帶 pre-release 後綴，例如 `v0.1.0-rc.1`；MSI `ProductVersion` 仍是
   `0.1.0`。

5. 到 Actions → Release 看 `verify`、`windows`、`linux`、`publish` 四個 job。
6. 檢查 Release assets、說明與 checksum；Windows 找一臺乾淨機器跑一次安裝、
   升級與反安裝。

手動試跑：Actions → Release → Run workflow。版本取自 `Cargo.toml`，只產
artifact，不會建立 Release。

## 第一次發版要人工確認

release workflow 只能證明「建置與打包成功」，下面這些要在第一次 tag 前確認：

- [ ] `JC_DATA_BUNDLE_URL` 已設定，且資料包只含 `shipped: true` 的檔案。
- [ ] `JC_DATA_BUNDLE_SHA256` 已設成 repo secret，值與 `scripts/data-bundle.sh`
      印出的 SHA-256 相同；手動觸發時 log 沒有「跳過資料包雜湊檢查」警告。
- [ ] 資料包 `manifest.json` 包含 Jinen tokenizer
      （`models/quant/jinen-v2-xsmall/tokenizer.json`）。依 `docs/models.md`，這筆
      出貨前要補進 `jc-data` 的 catalog／manifest；沒列就不會進 MSI。
- [ ] `models/zh-select`、`models/zh-punct` 沒有出現在資料包、MSI 或 Release assets。
- [ ] `LICENSE`、`NOTICE`、`legal/` 已打進 MSI（`installer-payload` 的缺口）。
- [ ] `Cargo.toml` 版本符合 `x.y.z` 或 `x.y.z-<pre-release>`，`v<version>` 標籤
      與它完全一致；RC 的 MSI `ProductVersion` 是去掉後綴的三段數字。
- [ ] `docs/CHANGELOG.md` 有對應版本段落，內容沒有承諾未實作的功能。
- [ ] 要簽章的話，`SIGNING_PFX_BASE64`、`SIGNING_PFX_PASSWORD` 已設定，而且
      憑證沒有過期；不簽章則接受 Release 標「未簽章」。
- [ ] Windows 實機完成 `docs/install.md` 的「必須在 Windows 驗證」清單，至少
      確認安裝、`Win+Space`、打注音、升級與反安裝。
- [ ] 32 位元應用程式與 UWP 的行為符合已知限制，沒有被寫成支援。
- [ ] 在乾淨的 Linux 桌面環境開過 `jc-settings-ui`，必要共用函式庫都在。

## 本機可做的驗證

```sh
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"
bash -n scripts/data-bundle.sh
bash installer/validate-linux.sh                # 需要 wix；沒有就跳過
cargo build --release -p jc-engine -p jc-cli

# 版本推導乾跑：完整版本可含 pre-release，MSI 版只取三段數字
python3 - <<'PY'
import re

for full in ("0.1.0", "0.1.0-rc.1", "0.1.0-rc.2"):
    assert re.fullmatch(r"\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?", full), full
    print(full, "-> msi", full.split("-", 1)[0])
PY
```

PowerShell 的版本推導與 `wix build` 只能在 Windows 驗；本機 Linux 最多做到
上面這些。`build.ps1` 的 `-MsiVersion` 若沒給，會自己從 `Cargo.toml` 版本推導。

`installer/validate-linux.sh` 目前直接把 `Cargo.toml` 的完整版本傳給 WiX
`-d Version=`；`Cargo.toml` 是 pre-release（例如 `0.1.0-rc.1`）時，這支腳本會
被 WiX 拒絕。要本機驗 WiX 語法，先暫時把版本改成三段數字，或等它套用同樣的
`-pre-release` 推導。

MSI 的完整建置、ICE 驗證與實機安裝只能在 Windows 做；release workflow 的
`windows` job 是第一個實際產出 MSI 的地方。
