# 發佈流程

`release.yml` 負責 release 建置與 GitHub Release；日常 CI（`ci.yml`）仍然只做
debug 測試與 clippy。兩邊刻意分開：發佈前唯一一次 release 測試在 `verify` job。

## 觸發方式

| 觸發 | 行為 | 建立 GitHub Release |
|---|---|---|
| push tag `v<version>` | `verify` → `windows` → `linux` → `publish` | 是 |
| Actions → Release → Run workflow | `verify` → `windows` → `linux` | 否，只留 artifact |

手動觸發可以填 `data_bundle_url`、`data_bundle_sha256` 覆寫 repo variable，
適合第一次試跑或換資料包來源。手動觸發的版本號取自 `Cargo.toml`。

## 版本號規則

- 唯一來源：`Cargo.toml` 的 `[workspace.package] version`。
- 標籤必須是 `v<version>`，而且與 `Cargo.toml` 完全相同；不同就讓 `verify` 失敗。
- 版本必須是 `x.y.z`。`build.ps1` 與 MSI `ProductVersion` 不接受 pre-release
  或 build metadata，所以 `v0.1.0-rc.1` 會在 `verify` 就先擋下。
- Release 標題、artifact 名稱、MSI 檔名都用同一個版本：
  - `jc-ime-<version>-x64.msi`
  - `jc-ime-<version>-linux-x86_64.tar.gz`
- 手動觸發不會建立 Release；要發佈仍要 push 對應的 tag。

## 前置設定

### 1. 資料包（必要）

資料與模型不進 git，Windows runner 也沒有 `/mnt/work/jc/jc-ime-data`。MSI 需要
資料根目錄，因此 release workflow 會下載一個預先打包的資料包。沒有資料包 URL
時 `windows` job 會直接失敗，不會產出缺資料的 MSI。

設定下列其中一個來源：

- repo variable `JC_DATA_BUNDLE_URL`（建議）
- repo secret `JC_DATA_BUNDLE_URL`（URL 需要保密時）
- `workflow_dispatch` 輸入 `data_bundle_url`

可選：`JC_DATA_BUNDLE_SHA256`（variable）或 `data_bundle_sha256` 輸入。有給就
在下載後比對；沒給只會警告，稍後仍由 `jc-cli data verify` 驗每個檔案的雜湊。

資料包格式可以是 `.tar.gz`、`.tgz` 或 `.zip`，內容是資料根目錄；上層多一層
`jc-ime-data/` 也可以，workflow 會自動找有 `manifest.json` 的那一層。

只打包 `shipped: true` 的檔案，不要把 Funmula 的 `models/zh-select`、
`models/zh-punct` 放進去：

```sh
# 1. 產生完整 manifest（下載項不存在也會標記，但不會複製）
cargo run -p jc-cli --release -- data manifest /mnt/work/jc/jc-ime-data \
  --out /tmp/opencode/jc-ime-data-manifest.json

# 2. 只複製 shipped: true 的檔案到暫存根目錄
python3 - <<'PY'
import json
import shutil
from pathlib import Path

source = Path("/mnt/work/jc/jc-ime-data")
stage = Path("/tmp/opencode/jc-ime-data")
manifest = json.loads(Path("/tmp/opencode/jc-ime-data-manifest.json").read_text())
stage.mkdir(parents=True, exist_ok=True)
shutil.copy2("/tmp/opencode/jc-ime-data-manifest.json", stage / "manifest.json")
for entry in manifest["files"]:
    if entry.get("shipped"):
        target = stage / entry["path"]
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source / entry["path"], target)
PY

# 3. 打包並記下雜湊
tar -C /tmp/opencode -czf /tmp/opencode/jc-ime-data.tar.gz jc-ime-data
sha256sum /tmp/opencode/jc-ime-data.tar.gz
```

把檔案上傳到 runner 讀得到的位置，再設定 `JC_DATA_BUNDLE_URL`。workflow 會把
`-Manifest <資料根>/manifest.json` 傳給 `build.ps1`，直接複製這份對完整資料根
目錄產生的 manifest；因此安裝包會保留 `shipped: false` 的下載項與 URL，但
payload 仍只複製 `shipped: true` 的檔案，Funmula 權重不會進 MSI 或 Release。
如果沒有 `-Manifest`，`build.ps1` 會退回原本行為，用 `jc-cli data manifest`
重新掃描資料根目錄；那種情況資料包必須含下載項，否則安裝後的 `data download`
會找不到 URL。

### 2. 簽章（選用）

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
| `verify` | 檢查 tag／`Cargo.toml` 版本一致且是 `x.y.z`；跑 `cargo test --workspace --release` |
| `windows` | 下載資料包 → `cargo build --workspace --release` → `jc-cli data verify` → 安裝 WiX v5 → `installer/build.ps1` → MSI、checksum、`data-manifest.json` |
| `linux` | 安裝 GPUI 系統依賴 → `cargo build --release -p jc-engine -p jc-cli -p jc-settings-ui` → 打包 tar.gz 與 checksum |
| `publish` | 只在 tag push 執行；下載兩個 artifact，建立或更新 GitHub Release |

`release.yml` 不跑 `cargo fmt`、clippy；那些由 `ci.yml` 在日常 push／PR 負責。

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

- 版本與發佈日期（UTC）
- 模型與資料版本：安裝包內 `manifest.json` 的 `version`（涵蓋詞典與模型）
- `docs/CHANGELOG.md` 中 `## <version>` 的段落
- 兩個平台的下載檔名與 `.sha256` 提示
- 簽章狀態：已簽章，或明確標「未簽章」與 SmartScreen／AppLocker 影響
- 已知限制：x86 TIP 未支援、MSI 需要管理員、注音選字模型需另外下載、Linux
  沒有桌面整合、設定變更需重啟引擎

Release 已存在時會更新標題與說明，並用 `--clobber` 覆蓋 assets，方便重跑失敗
的發佈。Release 標題就是 `<version>`，tag 是 `v<version>`。

## 發版步驟

1. 更新 `docs/CHANGELOG.md`，確認有 `## <version>` 段落。
2. 改 `Cargo.toml` 的 `[workspace.package] version`；必要時讓 `Cargo.lock` 跟著更新。
3. 跑 `cargo check --workspace --all-targets`，commit 到 `main` 並 push。
4. 建立並 push 標籤：

   ```sh
   git tag v0.1.0
   git push origin v0.1.0
   ```

5. 到 Actions → Release 看 `verify`、`windows`、`linux`、`publish` 四個 job。
6. 檢查 Release assets、說明與 checksum；Windows 找一臺乾淨機器跑一次安裝、
   升級與反安裝。

手動試跑：Actions → Release → Run workflow。版本取自 `Cargo.toml`，只產
artifact，不會建立 Release。

## 第一次發版要人工確認

release workflow 只能證明「建置與打包成功」，下面這些要在第一次 tag 前確認：

- [ ] `JC_DATA_BUNDLE_URL` 已設定，且資料包只含 `shipped: true` 的檔案。
- [ ] 資料包 `manifest.json` 包含 Jinen tokenizer
      （`models/quant/jinen-v2-xsmall/tokenizer.json`）。依 `docs/models.md`，這筆
      出貨前要補進 `jc-data` 的 catalog／manifest；沒列就不會進 MSI。
- [ ] `models/zh-select`、`models/zh-punct` 沒有出現在資料包、MSI 或 Release assets。
- [ ] `LICENSE`、`NOTICE`、`legal/` 已打進 MSI（`installer-payload` 的缺口）。
- [ ] `Cargo.toml` 版本是 `x.y.z`，`v<version>` 標籤與它一致。
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
bash installer/validate-linux.sh                # 需要 wix；沒有就跳過
cargo build --release -p jc-engine -p jc-cli
```

MSI 的完整建置、ICE 驗證與實機安裝只能在 Windows 做；release workflow 的
`windows` job 是第一個實際產出 MSI 的地方。
