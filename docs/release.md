# 發佈流程

`ci.yml` 是唯一入口。push main 時 `check-tag` 與五個 job 同時起步：linux／windows／
test-windows 測試，以及 build-windows／build-linux。build 只 `needs: check-tag`，
不等測試；`publish` 等 `check-tag`、三個測試、兩個 build 與 `install-smoke` 都成功
才建立 GitHub Release。帶版本標籤的 push 會在 `build-windows` 產出 setup.exe 後跑
`install-smoke`（見〈Windows CI 驗證〉），安裝或反安裝失敗就不發版。
沒有版本標籤的 push 也會建 setup.exe 並跑 install-smoke，只是不發版：安裝器
自己就是最常壞的地方，等發版才發現的代價遠高於每次多花六分鐘 runner。
`workflow_dispatch` 沒有額外 input，行為與 push main 相同但不發版。
所有發版 job 的 ref 都是 `refs/heads/main`。

`check-tag` 明確用 `fetch-tags: true` checkout，再主動 `git fetch --tags --force
origin`，用 `github.sha`（完整 SHA）搭配 `git tag --points-at` 找 `v*` 標籤；
查不到時等 3 秒重試 fetch，共三次。`push --follow-tags` 的 commit 與 annotated
tag 到達 runner 有 race，只靠 `git tag --points-at HEAD` 會漏掉（run 44888270175
就是這種情形）。job 的 `if` 不再檢查 `github.ref`：`on.push.branches: [main]`
已限制分支，改靠 `event_name == 'push'` 與 `has_tag` 判斷。

## 觸發方式

| 觸發 | 行為 | 建立 GitHub Release |
|---|---|---|
| push main（帶 annotated `v<version>` 標籤） | `check-tag` + 測試 + build 同時起步 → `install-smoke` → `publish` | 是 |
| push main（無版本標籤） | `check-tag` + 測試 + `build-windows` → `install-smoke` | 否 |
| pull request | `check-tag` + 測試（PR 分支的快取是冷的，建置留給 push） | 否 |
| Actions → CI → Run workflow | `check-tag` + 測試 + `build-windows` → `install-smoke` | 否 |
| Actions → Release → Run workflow | `verify` → `windows` → `linux`（只產 artifact） | 否 |

`git push --follow-tags origin main` 把 commit 連同 annotated tag 一起推上 main，
所以 release 跑在 main scope，`release-*` 快取不再掛在 tag ref。
`ci.yml` 的 `workflow_dispatch` 沒有額外 input，手動觸發一定建 setup.exe 並跑
安裝 smoke。版本號取自 `Cargo.toml`，不會發 Release。`release.yml` 的手動觸發可以填 `data_bundle_url`、
`data_bundle_sha256` 覆寫公開 repo variable，適合換資料包來源。

## 版本號規則

- 唯一來源：`Cargo.toml` 的 `[workspace.package] version`。
- 標籤必須是 `v<version>`，而且與 `Cargo.toml` 完全相同；不同就讓 `check-tag`（CI）／`verify`（手動測試）失敗。
- 版本格式是 `x.y.z` 或 `x.y.z-<pre-release>`，例如 `0.1.0`、`0.1.0-rc.1`。
- Release 標題、artifact 名稱、setup.exe 檔名都用完整版本：
  - `jc-ime-<version>-x64-setup.exe`
  - `jc-ime-<version>-linux-x86_64.tar.gz`
- 手動觸發不會建立 Release；要發佈仍要在 main commit 上打 annotated tag，並用
  `--follow-tags` 一起 push。
- pre-release 版本的 GitHub Release 會標成 pre-release，不會蓋掉 Latest 版。

### pre-release 版本規則

setup.exe 的 `AppVersion` 與檔名可以帶 pre-release 後綴，但
`VersionInfoVersion` 只吃四段數字。兩者分開處理：

| 用途 | 值 | 來源 |
|---|---|---|
| git tag | `v0.1.0-rc.1` | 人工建立 |
| `Cargo.toml` version | `0.1.0-rc.1` | 必須與 tag 去掉 `v` 相同 |
| Release 標題／artifact 名稱／setup.exe 檔名 | `0.1.0-rc.1` | 完整版本 |
| setup.exe `AppVersion` | `0.1.0-rc.1` | 完整版本（`/DMyAppVersion=`） |
| setup.exe `VersionInfoVersion` | `0.1.0.1` | `check-tag`／`verify` 輸出三段，`build.ps1` 補上 rc 序號 |

規則：

- `check-tag`／`verify` 用 `^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$` 驗完整版本，接受 `-rc.1`。
- `iso_version` 是完整版本 `-` 之前的三段數字；Windows job 用 `-IsoVersion`
  傳進 `installer/build.ps1`。
- `build.ps1` 的 `-IsoVersion` 是選項，接受三段或四段；`-MsiVersion` 保留為別名。
  沒給時從 `Cargo.toml` 推導：正式版是 `x.y.z.0`，`x.y.z-rc.N` 是 `x.y.z.N`。
  給三段時補上從完整版本推出的 rc 序號，所以既有 CI 不必改。第四段只認
  `rc.<數字>`，其他 pre-release 要明確傳四段 `-IsoVersion`。本機建置不必手動指定。
- `ISCC` 的 `/DMyAppVersion=` 拿完整版本，`/DIsoVersion=` 拿四段版本；檔名、
  Release 標題與說明仍用完整版本。
- `AppId` 固定不變（`{{EF7AD1D4-1BA4-41AE-AD3A-768BEA4D64B4}`），Windows 靠它
  認出同一個程式的新版本。改了會變成兩個項目並存。
- 升級時 `PrepareToInstall` 會先移除舊 MSI（`WindowsInstaller=1` 且
  `DisplayName` 以「jc 注音」開頭），再讓舊 DLL／EXE 改名為 `.old-<時間戳>`。
  第一次從 MSI 換到 Inno 不需要使用者手動移除舊版。
- 版本一致性檢查沒有放寬：tag 去掉 `v` 仍必須等於 `Cargo.toml` 的完整版本。

發布 RC 的流程與正式版相同，只是 tag 帶後綴：

```sh
git tag -a v0.1.0-rc.1 -m release
git push --follow-tags origin main
```

## 前置設定

### 1. 資料包（必要）

資料與模型不進 git，Windows runner 也沒有 `/mnt/work/jc/jc-ime-data`。setup.exe 需要
資料根目錄，因此 build job 會下載一個預先打包的資料包。沒有資料包 URL
時 `windows` job 會直接失敗，不會產出缺資料的安裝程式。

資料包來源只有一個：公開 repo variable，或 `workflow_dispatch` 輸入覆寫。
**不使用 repo secret**，讓 fork PR 也能重現同一份資料包。優先序是
`workflow_dispatch` 輸入 → repo variable：

- repo variable `JC_DATA_BUNDLE_URL`
- `workflow_dispatch` 輸入 `data_bundle_url`

`JC_DATA_BUNDLE_SHA256` 同樣只支援 variable 與輸入。workflow 下載後會實際算
SHA-256；不一致就讓 `windows` job 失敗，錯誤訊息會印出預期與實際值。沒有提供
這個值時只警告並繼續，之後仍由 `scripts/verify-bundle.py` 驗 manifest 內每個檔案的
大小與雜湊（與 `jc-cli data verify` 的 Full 驗證等價）。正式發版建議一定要鎖 SHA-256。

CI 的 `linux`／`windows` job 也下載這同一份資料包，但只讀公開 repo variable
`JC_DATA_BUNDLE_URL`／`JC_DATA_BUNDLE_SHA256`（不讀 secret、也不吃 workflow_dispatch
輸入）。因為是公開 variable，fork PR 也讀得到；兩個值任一為空或 SHA 不符就讓 CI
失敗，不會退回沒有資料的降級環境。CI 把找到的資料根目錄寫進 `JC_IMEDATA`，並在
log 印出根目錄與 manifest 的 `version` 供核對。

解開後接近 1GB，因此 `linux`、`windows`、`build-windows` 三個 job 都用
`actions/cache` 快取 `${{ runner.temp }}/jc-ime-data`，key 直接綁
`jc-ime-data-<runner.os>-<JC_DATA_BUNDLE_SHA256>`：資料包沒換就整段跳過下載與解開，
換了 SHA 自然換 key、不必手動清。只有 `linux` 與 `build-windows` 負責存檔（各自對應
一個 `runner.os`），其餘 job 只還原，避免同一份快取被多個 job 同時上傳；PR 不存檔，
因為 PR 的快取只有自己讀得到。快取命中時不會跑原本寫 `JC_IMEDATA`／`JC_DATA_ROOT`
的那一步，改由緊接的「設定資料根目錄」負責。

資料包格式可以是 `.tar.zst`、`.tzst`、`.tar.gz`、`.tgz` 或 `.zip`，內容是資料
根目錄；上層多一層 `jc-ime-data/` 也可以，workflow 會自動找有 `manifest.json`
的那一層。Windows runner 的 `tar` 若不支援 zstd，workflow 會退回 7-Zip；兩者都
沒有時請改用 `.tar.gz`。

Funmula 的 `zh-select` 依 MIT 全文（`legal/licenses/MIT-Funmula.txt`）可散布。打包前先
用 `scripts/prepare-zh-select.sh` 從固定 revision 的 ONNX 轉出 `model.q8_0.gguf`，並把
GGUF、`vocab.json` 與兩份 model manifest 變成 catalog 的 `shipped: true` 項目。原始
ONNX 與中間的 safetensors 會被刪掉；`zh-punct` 已於 2026-11-21 從 catalog 移除：

```sh
# 1. 轉檔並重算 manifest（需要 jc-cli；ONNX 缺席時腳本會抓並驗 SHA-256）。
scripts/prepare-zh-select.sh /mnt/work/jc/jc-ime-data

# 2. 打包。腳本只收 manifest 的 shipped: true 項目與 manifest.json。
JC_DATA_MANIFEST=/mnt/work/jc/jc-ime-data/manifest.json \
OUT_DIR=/tmp/opencode \
scripts/data-bundle.sh /mnt/work/jc/jc-ime-data
```

catalog 目前沒有 `shipped: false` 的下載項。`scripts/verify-bundle.py` 會把 GGUF、
`vocab.json` 與兩份 manifest 當成必須存在的隨附檔；缺席就擋下。引擎不再背景下載或
轉檔，安裝完第一次啟動直接載入 GGUF。

腳本用 `tar --sort=name`、固定 mtime、固定 UID/GID 與 `gzip -n`／`zstd -n`
讓同一份 manifest 產生相同 SHA-256。沒有 `zstd` 時會退回 `.tar.gz` 並在輸出
說明。檔名預設是 `jc-ime-data-<manifest version>.tar.zst`。

workflow 會把 `-Manifest <資料根>/manifest.json` 傳給 `build.ps1`，直接複製這份
對完整資料根目錄產生的 manifest。payload 只複製 `shipped: true` 的檔案，所以
`model.q8_0.gguf`、`vocab.json` 與兩份 manifest 會進 setup.exe，原始 ONNX 不會。
如果沒有 `-Manifest`，`build.ps1` 會用 `jc-cli data manifest` 重新掃描資料根目錄；
那種情況資料包必須含上述四筆隨附檔。

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
4. 更新公開 repo variable `JC_DATA_BUNDLE_URL` 為
   `https://huggingface.co/datasets/jeffpeng3/jc-ime-data/resolve/<commit>/jc-ime-data-<data-version>.tar.zst`。
5. 更新公開 repo variable `JC_DATA_BUNDLE_SHA256` 為腳本印出的 SHA-256。
6. 手動觸發 CI 或 Release workflow 驗一次，確認 log
   沒有「跳過資料包雜湊檢查」的警告，且安裝包內的 `data/manifest.json` 版本正確。
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
`jc-engine.exe`、`jc-settings-ui.exe` 等執行檔，最後再簽 setup.exe。時間戳預設用
`http://timestamp.digicert.com`。

要關閉簽章，刪掉 `SIGNING_PFX_BASE64` secret。若 secret 存在但找不到
`signtool.exe` 或簽章失敗，流程會失敗；這是刻意的，避免把「以為簽了」的檔案
發出去。

驗證簽章（Windows）：

```bat
signtool verify /pa /v jc-ime-<version>-x64-setup.exe
```

## Job 內容

| Job | 產出 |
|---|---|
| `check-tag` | `fetch-tags: true` + `git fetch --tags --force origin`，用 `github.sha` 比對 `v*` 標籤（最多三次，間隔 3 秒）；輸出 `has_tag`／`version`／`iso_version` |
| `linux`（測試） | 下載資料包 → fmt／clippy／Windows target check／test → 清理快取 |
| `windows`（測試） | 下載資料包 → `cargo test --workspace` |
| `test-windows`（測試） | 所有 push／PR；`cargo test --workspace --exclude jc-settings-ui` → Windows 專屬 crate 的 clippy → i686 型別檢查 → `installer/jc-tip-register` 測試 |
| `build-windows` | `needs: check-tag`，與測試同時起步；下載資料包並驗 SHA → `verify-bundle.py` → Inno Setup 6 → `installer/build.ps1 -IsoVersion <x.y.z>`（腳本補上 rc 序號）→ setup.exe、checksum、`data-manifest.json`，輸出 `signed`；每次 push main 與 `workflow_dispatch` 都跑 |
| `install-smoke` | `needs: [check-tag, build-windows]`，條件同 `build-windows`；下載 setup.exe → 靜默安裝 → 跑 `post-install-doctor.ps1` → 斷言 TSF profile 機碼與語言清單 → `unins000.exe` 反安裝 → 斷言清乾淨 |
| `build-linux` | `needs: check-tag`，與測試同時起步；GPUI 系統依賴 → `cargo build --release -p jc-engine -p jc-cli -p jc-settings-ui` → tar.gz 與 checksum → 清理快取 |
| `publish` | `needs: [check-tag, linux, windows, test-windows, build-windows, build-linux, install-smoke]`；全部成功才下載兩個 artifact、建立或更新 GitHub Release，不查 CI run |

Windows 建置必須指定 target（`--target x86_64-pc-windows-msvc`），否則 `build.ps1` 的 payload 路徑會不符；x64 找不到 `target/<triple>/release` 時會退回 `target/release`，並印出警告與實際路徑。

`+crt-static` 是 Windows 出貨條件，TSF DLL 不能依賴 `vcruntime140.dll`。旗標統一放在 `.cargo/config.toml` 的 `[target.x86_64-pc-windows-msvc]` 與 `[target.i686-pc-windows-msvc]`；`build.ps1` 的 `cargo build` 依照這組 rustflags，不會因 `RUSTFLAGS` 變動而全量重編。資料驗證改走 Python 後，workflow 不再為了驗證單獨編一次 `jc-cli`。

Windows 的 Python 輸出必須是 UTF-8。Windows job 設 `PYTHONUTF8=1`，`build.ps1` 另外設 `PYTHONIOENCODING=utf-8`，避免 Python 3.12 的 cp1252 讓 `check-payload.py` 與 `verify-bundle.py` 印中文時失敗。

build job 不跑 `cargo fmt`、clippy、test；那些由 `ci.yml` 在日常 push／PR 負責。`verify` 當初的 `cargo test --workspace --release` 已移除：CI 在 push main 時跑過同一批測試，release profile 重跑只是多花 8～9 分鐘。

`publish` 直接宣告 `needs: [check-tag, linux, windows, test-windows, build-windows, build-linux, install-smoke]`：check-tag 失敗、任一測試失敗、安裝或反安裝失敗，或沒有版本標籤時都不會發版。`build-windows`／`build-linux` 只 `needs: check-tag`，與測試同時起步；`install-smoke` 等 setup.exe 產出後才跑。`release.yml` 已移除 `workflow_call`，`workflow_dispatch` 只產 artifact，供手動測試 build。

## Windows CI 驗證

Windows 的程式有 66 處 `#[cfg(windows)]`，Linux 的 `cargo test` 完全看不到；
`windows` job 只跑 x64 的 `cargo test --workspace`。以下兩個 job 補上其餘驗證。

### `test-windows`

所有 push 與 pull_request 都跑，不只在 tag，所以一般 PR 就會擋下 Windows 迴歸。
它**不下載資料包**：需要資料的測試由 `require_data!()` 判斷，缺資料就 skip；
資料內容的正確性由 `linux`／`windows` 的完整測試負責。步驟：

1. `cargo test --workspace --no-fail-fast --exclude jc-settings-ui`。`jc-settings-ui`
   依賴 GPUI，與 Linux job 一樣排除；`--no-fail-fast` 讓一次 run 看到所有失敗的 crate。
2. `cargo clippy -p jc-tsf -p jc-windows -p jc-tsf-register -p jc-tsf-smoke -p jc-cli --all-targets -- -D warnings`。
   Linux 的 `--workspace` clippy 只編得到非 Windows 分支，這一步才檢查 Windows 專屬程式。
3. `cargo check -p jc-tsf -p jc-tsf-register -p jc-tsf-smoke --all-targets --target i686-pc-windows-msvc`。
   32 位元 TSF DLL 的型別檢查；不連結，實際 DLL 由 `build-windows` 建置。
4. `cargo test --manifest-path installer/jc-tip-register/Cargo.toml --all-targets`。
   `installer/jc-tip-register` 有自己的 `[workspace]`，不在 `--workspace` 裡。

快取用 `shared-key: test-windows`，與 `ci-windows`／`release-windows` 分開：
這支 job 會建 i686 產物，混用會讓另外兩個 job 還原到不對的 target 目錄。

### `install-smoke`

在每次 push main 與 `workflow_dispatch` 時跑
（`needs: [check-tag, build-windows]`，條件與 `build-windows` 相同），拿
`windows-setup-<version>` artifact 做一次真的安裝：

1. `setup.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /LOG=%TEMP%\install.log`，
   自己等 10 分鐘，超過就中止並印 log 最後 60 行；結束碼只接受 `0`。
2. 斷言 `C:\Program Files\jc-ime\` 有 `jc-tsf.dll`、`jc-tsf-x86.dll`、`jc-engine.exe`。
3. 以原使用者身分執行 `C:\Program Files\jc-ime\post-install-doctor.ps1`，參數
   `-NoUi -ExitWithSmokeCode`，也就是安裝器用的同一支腳本（安裝器的 `[Run]`
   固定帶 `-NoUi`，不會跳訊息框）。它會把 `<時間>.json`、`<時間>.log` 與
   `last.txt` 寫到 `%LOCALAPPDATA%\jc-ime\doctor\`；CI 從 `last.txt` 最後一行
   取得 log 路徑，複製成 `%TEMP%\smoke.err.log` 與 `%TEMP%\smoke.json`，並保留
   `%TEMP%\doctor-last.txt`。`jc-tsf-smoke` 超過 120 秒會被腳本中止並記成 FAIL。
   斷言 schema 是 `jc-ime-doctor/1`、`ok` 為真，而且 `install.files`、
   `registry.tip`、`profile.machine`、`profile.user`、`profile.language_list`
   這五項的 status 必須是 `pass` 或 `warn`。13 項檢查的意義見
   `docs/port/tsf-smoke.md`。
4. 用 PowerShell 直接斷言註冊狀態（與 doctor 的第 9 到 11 項互相備援）：
   - `HKLM:\SOFTWARE\Microsoft\CTF\TIP\{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}\LanguageProfile\0x00000404\{C4E8D217-5B3F-4A96-8E0C-7F1A2B3D4E56}`
     存在，`Enable` 為 `1`。
   - 同一路徑用 `reg query /reg:32` 也查得到。CTF 機碼不是 WOW64 轉向機碼，
     兩個檢視應該看得到同一份；查不到代表 32 位元程式載入不到輸入法。
   - `Get-WinUserLanguageList` 的 `InputMethodTips` 含 `0404:` 加同一個 profile GUID。
     `user-install` 以 `runasoriginaluser` 執行，這一步是同一個使用者。
5. 反安裝前先 `Stop-Process jc-engine -Force`、`Stop-Process explorer -Force`，等 3 秒，
   讓 TSF DLL 不再被載入；runner 上重開 explorer 不影響後續步驟。
6. `{app}\unins000.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /LOG=%TEMP%\uninstall.log`，
   同樣等 10 分鐘；結束碼只接受 `0`。
7. 斷言 `C:\Program Files\jc-ime` 消失、解除安裝登錄沒有「jc 注音」、Run 值
   `jc-ime-engine` 不存在，profile 機碼在 64 位元與 32 位元檢視都不存在。

### 失敗時去哪裡看

- 先看 job log：setup.exe／unins000.exe 非 0 結束碼會直接印出
  `install.log`／`uninstall.log` 最後 60 行。Inno 的 log 會列出
  `PrepareToInstall` 的讓路結果與 `machine-register` 的結束碼。
- `if: failure()` 會先從 `%LOCALAPPDATA%\jc-ime\doctor\` 收集當次 `last.txt`
  與 json，再上傳 artifact `install-smoke-logs-<version>`，內含
  `install.log`、`uninstall.log`、`doctor.json`、`smoke.json`、
  `smoke.err.log`、`doctor-last.txt` 與 `tsf.log`。即使安裝期 doctor 卡住、
  「安裝後 doctor」還沒跑，也拿得到當次診斷結果。
- `doctor.json`／`smoke.json` 的 `checks` 陣列會指出哪一項 status 是 `fail`，
  `fix` 欄位給修復指令；對照 `docs/port/tsf-smoke.md` 的 13 項檢查與可能原因。
  `doctor-last.txt` 是 `%LOCALAPPDATA%\jc-ime\doctor\last.txt` 的副本，最後一行
  指向原始 `.log`，但在 artifact 內要看的是複製出來的 `smoke.err.log`。
- `test-windows` 的失敗都在 job log：cargo test 的 crate 與測試名、clippy 的
  `-D warnings` 位置、i686 check 的型別錯誤。

`install-smoke` 在 `publish` 的 `needs` 裡：安裝、`jc-tsf-smoke`、profile 與語言
清單斷言、反安裝只要有一項失敗，就不會建立 Release。代價是發版多等一次 Windows
安裝與反安裝（約 5 到 10 分鐘）。

## 建置快取

release 與 CI 都用 `Swatinem/rust-cache@v2`。

| Workflow | Job | shared-key | cache-all-crates |
|---|---|---|---|
| `ci.yml` | `linux`（測試） | `ci-linux` | `true` |
| `ci.yml` | `windows`（測試） | `ci-windows` | `true` |
| `ci.yml` | `test-windows`（Windows 測試） | `test-windows` | `true` |
| `ci.yml` | `build-linux` | `release-linux` | `false` |
| `ci.yml` | `build-windows` | `release-windows` | `false` |
| `release.yml`（手動測試） | `linux` | `release-linux` | `false` |
| `release.yml`（手動測試） | `windows` | `release-windows` | `false` |

CI 連 workspace crate 的產物一起快取：每個 commit 通常只改幾個 crate，還原後 cargo 只
重編改到的。release 只快取依賴，release 產物一份太大，每個 tag 存一份會擠掉其他快取。

`linux` 與 `windows` 在測試前下載與 release 相同的資料包；`JC_IMEDATA` 由公開
variable 提供，fork PR 也讀得到。資料包本身不快取，每個 job 都重新下載並驗 SHA-256。
`test-windows` 不下載資料包：它驗的是 Windows 上編得過、跑得動，需要資料的測試由
`require_data!()` 判斷後 skip。

`linux（tar.gz）` 用自己的 `release-linux` 快取。`verify` 已不再編譯，既不裝工具鏈
也不掛 rust-cache，不會再先寫一份快取給打包 job。CI 是 debug profile，
`target/debug` 與 `target/release` 的產物不能互換，所以 `ci-*` 與 `release-*` 不共用。

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

`release-*` 快取現在寫在 `push main` 的 main scope：`build-*` 跑在 `ci.yml` 的
`refs/heads/main`，不同版本共用同一份快取，不再有 tag scope 每次 miss 的問題。
不需要暖機，也不需要手動 `workflow_dispatch`。

快取上限是每個 repo 10 GB（超過按最久未存取淘汰，7 天沒用到自動刪）。本機 debug 建置
量到 38 個測試執行檔共 4.5 GB，CI 的快取估計每個 OS 2–4 GB（壓縮後），Linux 加 Windows
可能吃掉一半以上的額度。接近上限時先把 `ci-windows` 的 `cache-all-crates` 改成
`"false"`，再不夠才關 `ci-linux` 的；release 的三個 job 維持只快取依賴。細節與清理
指令見 `docs/port/ci-cache.md`。

快取清理由 `scripts/cache-prune.sh` 執行：先照 family 上限與閒置天數算出刪除清單，再看執行後
預估是否超過總量上限 8 GB。超標時從剩下的快取依建立時間由舊到新追加刪除，直到低於 8 GB
或沒有可刪的為止。`cache-prune.yml` 每天 03:00 UTC 真的刪一次，手動 dispatch 預設 dry-run。

總量壓力下改用 `PROTECTED_HOURS`（預設 1 小時）當保護窗。原本 6 小時的保護窗適合一週一版，
不適合每天多次發版：rc.4／rc.5／rc.7 在 1～5 小時內接連產生，六個 tag 快取全部落在 6 小時
保護窗裡，清理完全沒作用，總量卡在 9.58 GB / 10 GB。1 小時只擋住並行 run 正在寫入的項目，
其餘照新舊釋出空間。`MAX_TOTAL_GB` 設 0 或 -1 可關閉總量規則，也可改用
`--max-total-gb N`、`--protected-hours N` 覆寫。判定與結果會寫入 step summary，
`ci.yml` 即使 `continue-on-error` 也看得到。

## 快取與發版流程

### 發現：tag scope 讓 Windows job 每次都 miss（已修正）

rc.5 與 rc.7 的 `windows` job log 都印出 `No cache found.`。跑完後
`Post Run Swatinem/rust-cache` 又存了一份 967 MB 快取，等於每次重蓋。

key 的組成欄位相同：

```text
Restore Key: v0-rust-release-windows-Windows_NT-x64-ea627442
Cache Key:   v0-rust-release-windows-Windows_NT-x64-ea627442-9e16f66d
```

差別在 scope。以前 release 由 `push tag v*` 觸發，快取寫在該 tag 的 scope；GitHub
只允許從同一 ref 或預設分支 `main` 還原。下一次 tag 是新 ref，讀不到前一個 tag 存的
快取，所以每次都 miss。

rc.7 windows job 的實際時間分佈：

| 步驟 | 時間 |
|---|---:|
| 驗證資料包（x64 release 建 `jc-cli`，舊流程） | 5m55s |
| 建置 MSI（x64 五個 crate + i686 `jc-tsf` + helper + 900 MB payload + WiX） | 19m19s |
| 存快取 | 59s |

現在驗證資料包改走 `scripts/verify-bundle.py`，不再單獨編 `jc-cli`，這一步從約 6 分鐘
降到幾秒；`verify` job 也不再跑 release 測試或寫入 `release-linux`。

release 改由 `ci.yml` 在 `push main` 時呼叫後，ref 固定是 `refs/heads/main`，
`release-windows`／`release-linux` 都寫在 main scope，後續版本直接命中這份快取。

### 操作順序

1. 版本號與 `Cargo.toml` 改好、commit 並 push 到 `main`，等 CI 測試通過。
2. 在剛 push 的 commit 上建立 annotated tag，連同 commit 一起推上 main：

   ```sh
   git tag -a v<version> -m release
   git push --follow-tags origin main
   ```

   `--follow-tags` 只推 annotated tag，所以必須用 `-a`；lightweight tag 不會被推上去，
   `check-tag` 也看不到它。tag 比 commit 晚到 runner 時，`check-tag` 會主動 fetch
   並重試三次。
3. `ci.yml` 的 `check-tag` 抓到 tag 後，`build-windows`／`build-linux` 與三個測試
   同時起步；`build-windows` 完成後跑 `install-smoke`；`publish` 等三個測試、
   兩個 build 與安裝 smoke 都成功。build 與 publish 都跑在 main scope。
4. 檢查 `publish` 建立或更新的 Release assets、說明與 checksum。

`workflow_dispatch` 不會建立 GitHub Release；`release.yml` 已移除 `publish` job 與
`workflow_call`，只留手動測試 build。CI 的 `publish` 只靠 `event_name == 'push'` 與
`has_tag` 判斷，可以重複跑同一個 tag。

bump 版本會改 `Cargo.lock`，精確 key 末段的 hash 跟著變。
`Swatinem/rust-cache` 還原時先用 restore key 前綴比對，仍會命中先前存的那份；
差別只是 `Cargo.lock` 變動部分的增量 rebuild。

## Artifact 內容

### Windows

| 檔案 | 內容 |
|---|---|
| `jc-ime-<version>-x64-setup.exe` | per-machine Inno 安裝程式：七個程式、資料 manifest 與 13 個 shipped 資料檔、`LICENSE`／`NOTICE`／兩份第三方聲明與 17 份授權全文；AppId 固定，升級靠它 |
| `jc-ime-<version>-x64-setup.exe.sha256` | setup.exe 的 SHA-256 |
| `data-manifest.json` | 安裝包內資料 manifest 的副本，publish 用它填 Release 的資料版本 |

setup.exe 的完整建置、簽章與安裝／升級／反安裝只能在 Windows runner 做；本機
Linux 沒有 ISCC，能做的只有 `.iss`、payload 與 manifest 的靜態一致性檢查：

```sh
JC_DATA_ROOT=/mnt/work/jc/jc-ime-data bash installer/validate-linux.sh
```

這支腳本會比對 `[Files]` 的 Source、`build.ps1` 的 staging、manifest 的
`shipped: true` 清單與授權文件；有 payload 時再跑 `check-payload.py` 的大小與
SHA-256 檢查。它不編譯 setup.exe。

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

`publish` 呼叫 `scripts/release-notes.sh` 產生並附上完整的 Release 說明，不再在
workflow 內 inline 拼字串：

```sh
scripts/release-notes.sh <版本> <iso版本> <true|false> <資料根目錄> <CHANGELOG 路徑>
```

- 版本與發佈日期（UTC）；版本有 pre-release 後綴時，另外註明 setup.exe 的
  `AppVersion` 是完整字串，`VersionInfoVersion` 只有四段數字
- 模型與資料版本：安裝包內 `manifest.json` 的 `version`（涵蓋詞典與模型）
- 資料包內容：bigram 語言模型是必要檔案、unigram 已退役、英文詞表與 zh-select
  神經選字模型都隨附
- `docs/CHANGELOG.md` 中 `## <version>` 的段落
- 兩個平台的下載檔名與 `.sha256` 提示
- 簽章狀態：已簽章，或明確標「未簽章」與 SmartScreen／AppLocker 影響
- 已知限制：x64／x86 TIP 共用同一個輸入法項目、安裝程式需要管理員且只加進
  執行安裝的帳號、Linux 沒有桌面整合、設定變更後引擎 1 秒內熱重載（TSF 的快捷鍵
  表最多 5 秒或下次取得焦點）、Windows 實機流程仍需人工驗證

改說明內容或要確認資料檔齊全時，先跑檢查模式：

```sh
scripts/release-notes.sh --check /mnt/work/jc/jc-ime-data
```

`--check` 驗證說明引用的資料檔都存在，至少包含 `build-zh-bigram/lm.bin`、
`english/words.bin`、`models/zh-select/` 的 model／vocab／manifest 與 jinen 模型。
缺少時逐行印出 `::error::`、預期路徑，並以 exit 1 結束。

Release 已存在時會更新標題與說明，並用 `--clobber` 覆蓋 assets，方便重跑失敗
的發佈。Release 標題就是 `<version>`，tag 是 `v<version>`。版本含 `-` 時一律
建立成 pre-release，避免 RC 蓋掉 Latest 正式版。

## 發版步驟

1. 更新 `docs/CHANGELOG.md`，確認有 `## <version>` 段落。
2. 改 `Cargo.toml` 的 `[workspace.package] version`；必要時讓 `Cargo.lock` 跟著更新。
   RC 就寫 `0.1.0-rc.1`，`VersionInfoVersion` 會自動推出 `0.1.0.1`。
3. 跑 `cargo check --workspace --all-targets`，commit 到 `main` 並 push。等 main 的
   CI 測試通過；`publish` 的 needs 含測試，測試沒過就不會發版。
4. 在剛 push 的 commit 上建立 annotated 標籤，連同 commit 一起推上 main：

   ```sh
   git tag -a v0.1.0 -m release
   git push --follow-tags origin main
   ```

   `--follow-tags` 只推 annotated tag，所以必須用 `-a`。RC 版本帶 pre-release
   後綴，例如 `v0.1.0-rc.1`；`VersionInfoVersion` 是 `0.1.0.1`。

5. 到 Actions → CI 看 `check-tag`、三個測試、`build-windows`／`build-linux`、
   `install-smoke` 與 `publish`。build 與測試同時起步，`install-smoke` 等 setup.exe，
   `publish` 等全部成功。
6. 檢查 Release assets、說明與 checksum。`install-smoke` 已在乾淨 runner 跑過
   安裝與反安裝；Windows 實機仍建議再跑一次升級與反安裝。

手動試跑：Actions → CI → Run workflow。版本取自 `Cargo.toml`，只產 artifact
並跑安裝 smoke，不會建立 Release。另一個入口是 Actions → Release → Run
workflow，只產 artifact。

## 第一次發版要人工確認

release workflow 只能證明「建置與打包成功」，下面這些要在第一次 tag 前確認：

- [ ] `JC_DATA_BUNDLE_URL` 已設定，且資料包只含 `shipped: true` 的檔案。
- [ ] `JC_DATA_BUNDLE_SHA256` 已設成公開 repo variable，值與 `scripts/data-bundle.sh`
      印出的 SHA-256 相同；手動觸發時 log 沒有「跳過資料包雜湊檢查」警告。
- [ ] 資料包 `manifest.json` 包含 Jinen tokenizer
      （`models/quant/jinen-v2-xsmall/tokenizer.json`）。依 `docs/models.md`，這筆
      出貨前要補進 `jc-data` 的 catalog／manifest；沒列就不會進 setup.exe。
- [ ] 資料包 `english/` 同時含 `words.bin`、`manifest.json`、`NOTICE.txt`。
      `Dictionary::load_pack` 三者都要過大小與 SHA 檢查；少任一個就會退回原型
      詞彙，英文輸入查不到正式詞表。可用 `tar -tf <bundle> | grep '^english/'`
      確認三筆都在。
- [ ] `models/zh-select/model.q8_0.gguf`、`vocab.json`、`manifest.json` 與
      `models/manifest.json` 都在資料包與 setup.exe，catalog 標 `shipped: true`；
      原始 `model.onnx` 與 `models/zh-punct/` 不在（`zh-punct` 已於 2026-11-21
      從 catalog 移除）。
- [ ] `legal/licenses/MIT-Funmula.txt` 已隨 setup.exe 安裝，copyright 與授權全文保留。
- [ ] `LICENSE`、`NOTICE`、`legal/` 已打進 setup.exe（`installer-payload` 的缺口）。
- [ ] `legal/RUST-THIRD-PARTY-NOTICES.md` 與 `Cargo.lock` 一致：
      `python3 scripts/gen-rust-notices.py --check`（CI 的 linux job 每次都跑）。
- [ ] `Cargo.toml` 版本符合 `x.y.z` 或 `x.y.z-<pre-release>`，`v<version>` 標籤
      與它完全一致；RC 的 `VersionInfoVersion` 是三段數字加 rc 序號
      （`0.1.0-rc.9` → `0.1.0.9`）。
- [ ] `docs/CHANGELOG.md` 有對應版本段落，內容沒有承諾未實作的功能。
- [ ] 要簽章的話，`SIGNING_PFX_BASE64`、`SIGNING_PFX_PASSWORD` 已設定，而且
      憑證沒有過期；不簽章則接受 Release 標「未簽章」。
- [ ] Windows 實機完成 `docs/install.md` 的「必須在 Windows 驗證」清單，至少
      確認安裝、`Win+Space`、打注音、升級與反安裝。
- [ ] 32 位元應用程式與 UWP 的行為符合已知限制，沒有被寫成支援。
- [ ] 在乾淨的 Linux 桌面環境開過 `jc-settings-ui`，必要共用函式庫都在。

## 本機可做的驗證

```sh
python3 -c "import yaml;[yaml.safe_load(open(p)) for p in ['.github/workflows/ci.yml','.github/workflows/release.yml']]"
bash -n scripts/data-bundle.sh
bash -n installer/validate-linux.sh
bash scripts/test-verify-bundle.sh              # Python 與 jc-cli 六情境比對
python3 scripts/verify-bundle.py /mnt/work/jc/jc-ime-data
cargo run -q -p jc-cli -- data verify /mnt/work/jc/jc-ime-data
JC_DATA_ROOT=/mnt/work/jc/jc-ime-data bash installer/validate-linux.sh
cargo build --release -p jc-engine -p jc-cli

# 版本推導乾跑：完整版本可含 pre-release，VersionInfoVersion 補上 rc 序號
python3 - <<'PY'
import re

for full in ("0.1.0", "0.1.0-rc.1", "0.1.0-rc.9"):
    assert re.fullmatch(r"\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?", full), full
    core, _, prerelease = full.partition("-")
    rc = 0 if not prerelease else int(re.fullmatch(r"rc\.(\d+)", prerelease).group(1))
    print(full, "-> VersionInfoVersion", f"{core}.{rc}")
PY
```

PowerShell 的版本推導與 ISCC 編譯只能在 Windows 驗；本機 Linux 最多做到上面這些。
`build.ps1` 的 `-IsoVersion` 若沒給，會自己從 `Cargo.toml` 推出四段版本；給三段時
會補上 rc 序號。`-MsiVersion` 是相容別名。

setup.exe 的完整建置、簽章與實機安裝只能在 Windows 做；`ci.yml` 的
`build-windows` 與 `install-smoke` 是第一個實際產出與安裝 setup.exe 的地方。
`release.yml` 的手動 job 也走同一支 `build.ps1`，產出一樣是 setup.exe。
