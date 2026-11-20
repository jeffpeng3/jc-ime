# jc 注音安裝器（WiX v5 MSI）

`installer/` 放 per-machine MSI 的來源。使用者文件在 [`../docs/install.md`](../docs/install.md)，
決策與驗證結果在 [`../docs/port/tsf-installer.md`](../docs/port/tsf-installer.md)。

## 為什麼是 MSI

`ITfInputProcessorProfiles::Register` 寫 `HKLM\SOFTWARE\Microsoft\CTF\TIP\{CLSID}`，
該機碼只有 Administrators 可寫。MSIX 的 `runFullTrust` 不提升權限、manifest 也沒有
對應 extension，所以 TSF IME 不能用 MSIX 部署。Store 接受的 native IME 形式也是
已簽章 MSI／EXE。理由與出處見 `plan/jc-ime/tsf-plan.md`。

## 檔案

| 檔案 | 用途 |
|---|---|
| `jc-ime.wxs` | MSI 本體：目錄、檔案、元件、自訂動作、`MajorUpgrade` |
| `jc-tip-register/` | MSI 呼叫的輔助程式：`DllRegisterServer`、`InstallLayoutOrTip`、退路清理；也是設定與授權捷徑的啟動器 |
| `build.ps1` | Windows 上的完整建置：cargo → 暫存 payload（含授權文件）→ `wix build` → 選用簽章 |
| `check-payload.py` | 檢查 WXS 引用的檔案、manifest 的 `shipped` 條目、大小與 SHA-256，以及授權文件清單 |
| `validate-linux.sh` | Linux 上的有限驗證：WiX 編譯與符號解析（不能綁定 MSI） |

## 建置

前置：

- Visual Studio 2022 的 MSVC 工具鏈，Rust target `x86_64-pc-windows-msvc`
- .NET 6 以上
- WiX v5：`dotnet tool install --global wix --version 5.*`
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

輸出：`target\installer\jc-ime-<version>-x64.msi`。

腳本做這些事：

1. 用 `-C target-feature=+crt-static` 建置 workspace 的六個執行檔／DLL 與
   `jc-tip-register`。靜態 CRT 是必要條件：TSF DLL 會被載入 explorer 與任意程式，
   不能依賴 `vcruntime140.dll`（`docs/history.md` 的既有決策）。
2. 用建置出來的 `jc-cli.exe data manifest` 對資料根目錄產生 `manifest.json`，
   只複製 `shipped: true` 的檔案。Funmula 的 `zh-select`、`zh-punct` 依授權不隨
   產品散布。
3. `jc-cli data verify` 驗證暫存資料的雜湊。
4. 把授權文件以 repo 的相對路徑複製進 payload：`LICENSE`、`NOTICE`、
   `legal/THIRD-PARTY-NOTICES.md`、`legal/licenses/` 全部。缺檔或
   `legal/licenses/` 空目錄時直接失敗。
5. `check-payload.py` 比對 WXS 引用、`shipped` 資料檔與授權文件三份清單；repo 的
   授權文件沒進 WXS、payload 少檔或內容與 repo 不同都會失敗。
6. 有憑證時簽 `jc-tsf.dll`、`jc-engine.exe`、`jc-cli.exe`、`jc-settings-ui.exe`、
   `jc-tsf-smoke.exe`、`jc-tip-register.exe`，再簽 MSI。沒憑證時輸出未簽章 MSI。
7. `wix build -arch x64 -d Version=<Cargo 版本> -d PayloadDir=<暫存目錄>`。

版本號取自 workspace `Cargo.toml` 的 `version`；MSI 只接受 `x.y.z`，pre-release
字串會讓腳本停下。

## MSI 內部

安裝目錄：

```text
%ProgramFiles%\jc-ime\            jc-tsf.dll、jc-engine.exe、jc-cli.exe、
                                  jc-settings-ui.exe、jc-tsf-smoke.exe、
                                  jc-tip-register.exe
%ProgramFiles%\jc-ime\legal\      LICENSE、NOTICE、THIRD-PARTY-NOTICES.md
%ProgramFiles%\jc-ime\legal\licenses\  legal/licenses/ 的 17 份授權全文
%ProgramData%\jc-ime\data\        manifest.json 與 5 個 shipped 資料檔
%LOCALAPPDATA%\jc-ime\            settings.json、personal.json、tsf.log（使用者資料，
                                  由程式建立；MSI 不寫入）
```

授權文件在 payload 內的路徑與 repo 相同，`check-payload.py` 因此可以直接比對
雜湊。WXS 的 `LegalFiles` 群組把它們放進 `LEGALDIR`／`LICENSEDIR`；這些是普通
元件，反安裝跟著 `RemoveFiles` 一起刪。

自訂動作：

| 動作 | 種類 | 身分 | 時機 | 內容 |
|---|---|---|---|---|
| `RegisterTip` | deferred | SYSTEM | `InstallFiles` 之後 | `jc-tip-register machine-register <jc-tsf.dll>` |
| `RollbackRegisterTip` | rollback | SYSTEM | `RegisterTip` 之前 | `machine-unregister` |
| `UnregisterTip` | deferred | SYSTEM | `RemoveFiles` 之前 | `machine-unregister`，DLL 不在時內建清理 |
| `RollbackUnregisterTip` | rollback | SYSTEM | `UnregisterTip` 之前 | `machine-register` |
| `InstallTipLayout` | immediate | 使用者 | `InstallFinalize` 之後 | `user-install`（`InstallLayoutOrTip`，旗標 0） |
| `RemoveTipLayout` | immediate | 使用者 | 反安裝的 `InstallFinalize` 之後 | `user-uninstall [KEEPUSERSETTINGS]` |

`machine-*` 必須是 SYSTEM：CTF 機碼在 HKLM。`user-*` 必須是使用者：`InstallLayoutOrTip`
只寫呼叫者的 HKCU，在 SYSTEM 執行會加到 SYSTEM 的語言清單。

`RegisterTip` 用 `Return="check"`：註冊失敗就讓安裝失敗並回滾，不留半套 TIP。
`UnregisterTip` 與兩個 `user-*` 用 `Return="ignore"`：反安裝不能被已隔離的 DLL 或
使用者的語言設定卡住。

開始功能表有兩個捷徑：`jc 注音設定`（`jc-tip-register settings`）與
`jc 注音授權`（`jc-tip-register legal`）。後者用記事本開啟
`%ProgramFiles%\jc-ime\legal\THIRD-PARTY-NOTICES.md`；`.md` 在乾淨的 Windows
沒有預設關聯，所以不讓 `.lnk` 直接指向檔案。

## 建置選項

| MSI 屬性 | 預設 | 說明 |
|---|---|---|
| `KEEPUSERSETTINGS` | `0` | 反安裝時保留 `%LOCALAPPDATA%\jc-ime`。`msiexec /x ... KEEPUSERSETTINGS=1` |

## 驗證

Linux 只能做一部分：

```sh
installer/validate-linux.sh /path/to/payload
uv run --no-project installer/check-payload.py /path/to/payload
cd installer/jc-tip-register && cargo check --target x86_64-pc-windows-gnu --all-targets
```

`validate-linux.sh` 會把 `Directory/@Name` 換成 `.` 後交給 WiX 編譯。這是 WiX v5
在 Linux 的已知限制：它用 `C:\` 當根目錄做路徑正規化，任何目錄名稱都會被誤判成
不是相對路徑（WIX0389）。編譯通過後，綁定階段會因為 Linux 沒有 `msi.dll` 停止
（WIX0001），這是預期結果。

**完整建置與 ICE 驗證必須在 Windows 執行。** 需要實機確認的項目列在
[`../docs/install.md`](../docs/install.md) 的「必須在 Windows 驗證」。
