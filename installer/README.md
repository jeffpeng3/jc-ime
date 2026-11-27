# jc 注音安裝器（Inno Setup）

`installer/` 放 per-machine setup.exe 的來源。使用者文件在
[`../docs/install.md`](../docs/install.md)，決策與驗證結果在
[`../docs/port/tsf-installer.md`](../docs/port/tsf-installer.md)。

## 為什麼是 setup.exe（Inno Setup）

`ITfInputProcessorProfiles::Register` 寫 `HKLM\SOFTWARE\Microsoft\CTF\TIP\{CLSID}`，
該機碼只有 Administrators 可寫，所以安裝器必須提權。MSI 也能提權，但 MSI 的
`InstallFiles` 會直接覆寫被 explorer 載入的 `jc-tsf.dll`，預設的 Restart Manager
又會嘗試關閉宿主；0.1.4 的 1603／1708 就是這條路失敗。

Inno Setup 可以關掉 `CloseApplications`／`RestartApplications`，並在檔案複製前用
`PrepareToInstall` 把舊檔讓開：先刪、刪不掉改名 `.old-<yyyymmddhhnnss>`、再不行
才中止。反安裝用同一套策略，最後把資料夾排開機刪除。這比在 MSI 裡對抗
component 追蹤與檔案版本規則單純。

## 檔案

| 檔案 | 用途 |
|---|---|
| `jc.iss` | 安裝腳本：`[Setup]`、`[Files]`、`[Registry]`、`[Run]`、`[UninstallRun]` 與 `[Code]` |
| `languages/ChineseTraditional.isl` | 繁體中文介面。來自 jrsoftware/issrc 的 Inno Setup 翻譯檔（GoneTone 維護，6.5.0+），不是 Tsunagi 的程式碼 |
| `jc-tip-register/` | 安裝器呼叫的輔助程式：TSF profile 與分類、`InstallLayoutOrTip`；也是授權捷徑的啟動器 |
| `build.ps1` | Windows 上的完整建置：cargo → 暫存 payload（含授權文件）→ `check-payload.py` → 簽章 → `ISCC` |
| `check-payload.py` | 檢查 `jc.iss` 引用的檔案、manifest 的 `shipped` 條目、大小與 SHA-256，以及授權文件清單 |
| `validate-linux.sh` | Linux 上的靜態驗證：`.iss`、`build.ps1` staging、manifest 與授權文件的一致性 |

## 建置

前置：

- Visual Studio 2022 的 MSVC 工具鏈，Rust target `x86_64-pc-windows-msvc` 與
  `i686-pc-windows-msvc`（`rustup target add i686-pc-windows-msvc`）
- Inno Setup 6：`choco install innosetup -y`，或從 jrsoftware 下載安裝。
  `ISCC.exe` 在 `Program Files (x86)\Inno Setup 6`，`build.ps1` 也會找 PATH
- 資料根目錄（預設 `JC_DATA_ROOT`），內含 `build-zh`、`english`、`packs`、`models`
- 選用：Windows SDK 的 `signtool.exe`（簽章時才需要）

```powershell
# 未簽章
.\installer\build.ps1 -DataRoot Z:\mnt\work\jc\jc-ime-data

# 簽章
$env:JC_SIGN_CERT = 'C:\keys\jc-ime.pfx'
$env:JC_SIGN_PASSWORD = '...'
.\installer\build.ps1 -DataRoot Z:\mnt\work\jc\jc-ime-data
```

輸出：`target\installer\jc-ime-<version>-x64-setup.exe`（同一個 setup.exe 內含
x64 與 x86 的 TSF DLL）。

腳本做這些事：

1. 依 `.cargo/config.toml` 的 `-C target-feature=+crt-static` 建置 workspace
   的七個執行檔／DLL 與 `jc-tip-register`，另外用 `i686-pc-windows-msvc` 再建
   一份 `jc-tsf`（暫存成 `jc-tsf-x86.dll`）。靜態 CRT 是必要條件：TSF DLL 會被
   載入 explorer 與任意程式，不能依賴 `vcruntime140.dll`（`docs/history.md` 的
   既有決策）；兩個 Windows target 的 rustflags 都放在 `.cargo/config.toml`。
2. 用建置出來的 `jc-cli.exe data manifest` 對資料根目錄產生 `manifest.json`，
   只複製 `shipped: true` 的檔案。Funmula 的 `zh-select` 依 MIT 全文可散布，
   `model.q8_0.gguf`、`vocab.json` 與兩份 manifest 隨 setup.exe 安裝；原始 ONNX
   與 `zh-punct` 不進安裝包。
3. `jc-cli data verify` 驗證暫存資料的雜湊。
4. 把授權文件以 repo 的相對路徑複製進 payload：`LICENSE`、`NOTICE`、
   `legal/THIRD-PARTY-NOTICES.md`、`legal/licenses/` 全部。缺檔或
   `legal/licenses/` 空目錄時直接失敗。
5. `check-payload.py` 比對 `jc.iss` 的 `[Files]`、`shipped` 資料檔與授權文件三份
   清單；repo 的授權文件沒進安裝腳本、payload 少檔或內容與 repo 不同都會失敗。
6. 有憑證時簽 `jc-tsf.dll`、`jc-tsf-x86.dll`、`jc-engine.exe`、
   `jc-engine-launcher.exe`、`jc-cli.exe`、`jc-settings-ui.exe`、
   `jc-tsf-smoke.exe`、`jc-tip-register.exe`，再用
   `ISCC` 打包，最後簽 setup.exe。沒憑證時輸出未簽章 setup.exe。
7. `ISCC /DMyAppVersion=<完整版本> /DIsoVersion=<四段版本> /DPayloadDir=<暫存目錄>
   /O<輸出目錄> installer\jc.iss`。

版本號取自 workspace `Cargo.toml` 的 `version`。`AppVersion` 與檔名用完整版本
（可含 `-rc.1`）；`VersionInfoVersion` 是四段 `x.y.z.N`，N 是 rc 序號：
`0.1.0` → `0.1.0.0`，`0.1.0-rc.9` → `0.1.0.9`。`-IsoVersion` 可覆寫，給三段時
補上從完整版本推出的 rc 序號；`-MsiVersion` 是舊指令的相容別名。

## 安裝器內部

安裝目錄：

```text
%ProgramFiles%\jc-ime\            jc-tsf.dll（x64）、jc-tsf-x86.dll（x86）、
                                  jc-engine.exe、jc-engine-launcher.exe、
                                  jc-cli.exe、jc-settings-ui.exe、
                                  jc-tsf-smoke.exe、jc-tip-register.exe
%ProgramFiles%\jc-ime\legal\      LICENSE、NOTICE、THIRD-PARTY-NOTICES.md
%ProgramFiles%\jc-ime\legal\licenses\  legal/licenses/ 的 17 份授權全文
%ProgramData%\jc-ime\data\        manifest.json 與 13 個 shipped 資料檔
%LOCALAPPDATA%\jc-ime\            settings.json、personal.json、tsf.log（使用者資料，
                                  由程式建立；安裝器不寫入）
```

授權文件在 payload 內的路徑與 repo 相同，`check-payload.py` 因此可以直接比對
雜湊。

### `[Setup]`

- `AppId={{EF7AD1D4-1BA4-41AE-AD3A-768BEA4D64B4}`：固定不變，升級靠它。
- `PrivilegesRequired=admin`、`ArchitecturesInstallIn64BitMode=x64compatible`。
- `DefaultDirName={autopf}\jc-ime`，與舊版 `C:\Program Files\jc-ime` 相同。
- `CloseApplications=no`、`RestartApplications=no`：不要讓 Restart Manager 關掉
  explorer。
- `AppVersion` 用完整版本；`VersionInfoVersion` 用四段數字。

### `[Files]` 與 `[Registry]`

八個程式都放 `{app}` 同一層；資料依 manifest 的子目錄放到
`{commonappdata}\jc-ime\data`。

CLSID `{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}` 的兩個機碼檢視由 `[Registry]` 寫：

- `HKCR\CLSID\{...}\InprocServer32` → `{app}\jc-tsf.dll`（64 位元）
- 同一路徑加 `Flags: 32bit` → `Wow6432Node`，指向 `{app}\jc-tsf-x86.dll`
- `PipePrefix=jc-ime` 兩個檢視各一份

引擎的登入自動啟動是 `HKLM\...\Run\jc-ime-engine`，值型別 `expandsz`，內容用
`jc-engine-launcher.exe --supervise` 啟動引擎。啟動器是 windows subsystem，以
`CREATE_NO_WINDOW` 帶起 console subsystem 的 `jc-engine.exe`，登入不會出現終端機
視窗。`%LOCALAPPDATA%` 由 REG_EXPAND_SZ 在每位使用者登入時展開。升級時先寫一筆
`ValueType: none` 刪掉舊值，再寫新值，避免新舊命令同時存在。

### `[Tasks]` 與 `[Run]`

`enableime` 預設勾選，控制是否把 jc 注音加進目前使用者的語言清單。`smoke`
（安裝後執行診斷）是 `Flags: unchecked`，預設不勾；勾了才會執行
`post-install-doctor.ps1 -NoUi`，log 寫到原使用者的
`%LOCALAPPDATA%\jc-ime\doctor\`。順序：

1. `jc-tip-register.exe machine-register`：提升權限，寫 HKLM 的 profile 與分類。
2. `jc-tip-register.exe user-install`：`runasoriginaluser`，寫實際使用者的語言清單。
3. `jc-tip-register.exe user-cleanup-legacy`：`runasoriginaluser`，清舊 CLSID 殘留。

`runasoriginaluser` 不能省：setup.exe 是管理員身分，直接跑會寫到 Administrator
的語言清單，對實際使用者沒有作用。`user-install`／`user-cleanup-legacy` 失敗用
`ignoreerrors` 不擋安裝；`machine-register` 失敗會讓安裝停下來，不留半套 TIP。

### `[UninstallRun]` 與 `[Code]`

反安裝先跑 `jc-tip-register.exe machine-unregister`（提升權限）。`user-uninstall`
需要原使用者身分，但 `[UninstallRun]` 不支援 `runasoriginaluser`；profile 移除後
語言清單的項目自然失效。

`[Code]` 的重點：

- `MoveFileExW` 外部宣告，第二個參數是 `Cardinal`，傳 `0` 才是 NULL。
- `DeleteOrScheduleDelete`：能刪就刪，刪不掉登記 `MOVEFILE_DELAY_UNTIL_REBOOT`。
- `SweepLeftovers`：清 `jc-tsf.dll`／`jc-tsf-x86.dll`／`jc-engine.exe`／
  `jc-engine-launcher.exe` 的 `.old-*`。
- `PrepareToInstall`：先移除舊 MSI，再逐一讓開四個程式檔。先刪、刪不掉改名
  `.old-<yyyymmddhhnnss>`、再失敗才回錯誤訊息。時間戳命名是刻意的，固定名稱會在
  上一輪殘骸仍被占用時卡住。
- `RemoveLegacyMsi`：掃 `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall`
  與 `WOW6432Node`，找 `WindowsInstaller=1` 且 `DisplayName` 以「jc 注音」開頭
  的項目，從 `UninstallString` 取 ProductCode。找到就停掉 `jc-engine` 與載入
  `jc-tsf.dll` 的行程，再 `msiexec /x <ProductCode> /qn /norestart`。失敗只寫
  log（`%ProgramData%\jc-ime\msi-uninstall.log`），不中斷安裝。
- `CurUninstallStepChanged(usPostUninstall)`：清 `.old-*`、把仍在的程式檔排開機
  刪除，最後收掉 `legal\licenses`、`legal` 與 `{app}`；目錄刪不掉就排開機刪除。

## 驗證

Linux 只能做靜態檢查：

```sh
JC_DATA_ROOT=/mnt/work/jc/jc-ime-data bash installer/validate-linux.sh
uv run --no-project installer/check-payload.py /path/to/payload
cd installer/jc-tip-register && cargo check --target x86_64-pc-windows-gnu --all-targets
cargo check --target i686-pc-windows-msvc -p jc-tsf
```

`validate-linux.sh` 會檢查 `[Setup]`／`[Code]` 的關鍵設定、`[Files]` 的每個
`Source` 是否對得上 `build.ps1` 的 staging、manifest 的 `shipped` 清單與授權文件；
有 payload 時再跑 `check-payload.py`。它不執行 ISCC。

**ISCC 編譯、簽章與實機安裝／升級／反安裝必須在 Windows 執行。** 需要實機確認的
項目列在 [`../docs/install.md`](../docs/install.md) 的「必須在 Windows 驗證」。

## 授權

`PrepareToInstall`／`SweepLeftovers`／`CurUninstallStepChanged` 的讓路策略參考
[Zosuya/Tsunagi-IME](https://github.com/Zosuya/Tsunagi-IME)（GPL-3.0-or-later）。
jc 是 AGPL-3.0-only；GPLv3 與 AGPLv3 相容。這裡是照策略重寫的 jc 版本，沒有逐行
複製 Tsunagi 的程式碼。
