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
| `jc-tip-register/` | MSI 呼叫的輔助程式：TSF profile 與分類、`InstallLayoutOrTip`；也是設定與授權捷徑的啟動器 |
| `build.ps1` | Windows 上的完整建置：cargo → 暫存 payload（含授權文件）→ `wix build` → 選用簽章 |
| `check-payload.py` | 檢查 WXS 引用的檔案、manifest 的 `shipped` 條目、大小與 SHA-256，以及授權文件清單 |
| `validate-linux.sh` | Linux 上的有限驗證：WiX 編譯與符號解析（不能綁定 MSI） |

## 建置

前置：

- Visual Studio 2022 的 MSVC 工具鏈，Rust target `x86_64-pc-windows-msvc` 與
  `i686-pc-windows-msvc`（`rustup target add i686-pc-windows-msvc`）
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

輸出：`target\installer\jc-ime-<version>-x64.msi`（同一個 MSI 內含 x64 與 x86 的
TSF DLL）。

腳本做這些事：

1. 依 `.cargo/config.toml` 的 `-C target-feature=+crt-static` 建置 workspace
   的六個執行檔／DLL 與 `jc-tip-register`，另外用 `i686-pc-windows-msvc` 再建
   一份 `jc-tsf`（暫存成 `jc-tsf-x86.dll`）。靜態 CRT 是必要條件：TSF DLL 會被
   載入 explorer 與任意程式，不能依賴 `vcruntime140.dll`（`docs/history.md` 的
   既有決策）；兩個 Windows target 的 rustflags 都放在 `.cargo/config.toml`。
2. 用建置出來的 `jc-cli.exe data manifest` 對資料根目錄產生 `manifest.json`，
   只複製 `shipped: true` 的檔案。Funmula 的 `zh-select` 依 MIT 全文可散布，
   `model.q8_0.gguf`、`vocab.json` 與兩份 manifest 隨 MSI 安裝；原始 ONNX 與
   `zh-punct` 不進 MSI。
3. `jc-cli data verify` 驗證暫存資料的雜湊。
4. 把授權文件以 repo 的相對路徑複製進 payload：`LICENSE`、`NOTICE`、
   `legal/THIRD-PARTY-NOTICES.md`、`legal/licenses/` 全部。缺檔或
   `legal/licenses/` 空目錄時直接失敗。
5. `check-payload.py` 比對 WXS 引用、`shipped` 資料檔與授權文件三份清單；repo 的
   授權文件沒進 WXS、payload 少檔或內容與 repo 不同都會失敗。
6. 有憑證時簽 `jc-tsf.dll`、`jc-tsf-x86.dll`、`jc-engine.exe`、`jc-cli.exe`、
   `jc-settings-ui.exe`、`jc-tsf-smoke.exe`、`jc-tip-register.exe`，再簽 MSI。
   沒憑證時輸出未簽章 MSI。
7. `wix build -arch x64 -d Version=<MSI 版本> -d PayloadDir=<暫存目錄>`。

版本號取自 workspace `Cargo.toml` 的 `version`。MSI `ProductVersion` 是四段
`x.y.z.N`，N 是 rc 序號：`0.1.0` → `0.1.0.0`，`0.1.0-rc.9` → `0.1.0.9`。
`-MsiVersion` 可覆寫，給三段時補上從完整版本推出的 rc 序號。PackageCode 由 WiX
每次 build 自動產生（`ProductCode="*"`）。

## MSI 內部

安裝目錄：

```text
%ProgramFiles%\jc-ime\            jc-tsf.dll（x64）、jc-tsf-x86.dll（x86）、
                                  jc-engine.exe、jc-cli.exe、
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

`ProductVersion` 的第四段是 rc 序號，Windows Installer 比較版本時只看前三段，
所以 WXS 保留 `AllowSameVersionUpgrades="yes"`。固定的 `UpgradeCode` 搭配每次
build 自動產生的 `ProductCode`，讓相鄰 rc 直接上升級，不必先手動移除舊版。
第四段讓 ARP 顯示與升級 log 能區分 rc；前三段相同時也接受降級，發版順序仍要遞增。

自訂動作：

| 動作 | 種類 | 身分 | 時機 | 內容 |
|---|---|---|---|---|
| `RegisterTip` | deferred | SYSTEM | `WriteRegistryValues` 之後 | `machine-register`；`RegisterProfile` 與三個 `RegisterCategory`。CLSID 已由 `<Class>` 寫好 |
| `RollbackRegisterTip` | rollback | SYSTEM | `RegisterTip` 之前 | `machine-unregister` |
| `UnregisterTip` | deferred | SYSTEM | 反安裝、`RemoveRegistryValues` 之前 | `machine-unregister`；清 profile 與分類，CLSID 機碼留給 MSI |
| `RollbackUnregisterTip` | rollback | SYSTEM | `UnregisterTip` 之前 | `machine-register` |
| `InstallTipLayout` | immediate | 使用者 | `InstallFinalize` 之後 | `user-install`（`InstallLayoutOrTip(ILOT_INSTALL)`） |
| `RemoveTipLayout` | immediate | 使用者 | 反安裝的 `InstallFinalize` 之後 | `user-uninstall [KEEPUSERSETTINGS]`；只清使用者資料，語言清單那一步是 no-op |

`machine-*` 必須是 SYSTEM：CTF 機碼在 HKLM。`user-*` 必須是使用者：`InstallLayoutOrTip`
只寫呼叫者的 HKCU，在 SYSTEM 執行會加到 SYSTEM 的語言清單。

CLSID 由 WXS 的 `<Class Advertise="no">` 註冊，不是由 DLL 的 `DllRegisterServer`。
`Advertise="no"` 讓 WiX 產生 Registry table 的資料列，`WriteRegistryValues` 在
`RegisterTip` 之前把 `HKCR\CLSID\{...}\InprocServer32` 的兩個檢視都寫好，
`RegisterCategory` 才不會拿到不存在的類別工廠（rc.5／rc.6 的 `0xC0000005`）。
反安裝時這批資料列由 `RemoveRegistryValues`（標準順序 2600）移除，所以
`UnregisterTip` 排 `Before="RemoveRegistryValues"`，`UnregisterProfile` 執行時
CLSID 還在。`RegisterClassInfo`／`UnregisterClassInfo` 只處理 Class table，這裡
沒有用到。

`RegisterTip` 用 `Return="check"`：註冊失敗就讓安裝失敗並回滾，不留半套 TIP。
`UnregisterTip` 與兩個 `user-*` 用 `Return="ignore"`：反安裝不能被已隔離的 DLL 或
使用者的語言設定卡住。

### 兩個架構

TSF 依宿主行程的位元數載入 DLL，所以 32 位元程式需要另一份：`jc-tsf-x86.dll`
（`i686-pc-windows-msvc`）註冊在 `Wow6432Node`。兩份**共用同一個 CLSID 與
profile**，只差註冊的機碼檢視，因此使用者的語言清單只有一項「jc 注音」。
兩個檢視的 `InprocServer32` 都由 WXS 的 `<Class>` 寫：`TsfDll` 是 64 位元元件，
`TsfDllX86` 標了 `Bitness="always32"`（WiX v5；v3 的 `Win64="no"` 已移除），WiX
因此把它的機碼放進 `Wow6432Node`。完整理由、GUID 對照、`reg query` 驗證步驟與
反安裝流程見 [`../docs/port/x86-tip.md`](../docs/port/x86-tip.md)。

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
cargo check --target i686-pc-windows-msvc -p jc-tsf
```

`validate-linux.sh` 會把 `Directory/@Name` 換成 `.` 後交給 WiX 編譯。這是 WiX v5
在 Linux 的已知限制：它用 `C:\` 當根目錄做路徑正規化，任何目錄名稱都會被誤判成
不是相對路徑（WIX0389）。編譯通過後，綁定階段會因為 Linux 沒有 `msi.dll` 停止
（WIX0001），這是預期結果。

**完整建置與 ICE 驗證必須在 Windows 執行。** 需要實機確認的項目列在
[`../docs/install.md`](../docs/install.md) 的「必須在 Windows 驗證」。
