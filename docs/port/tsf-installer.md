# tsf-installer：MSI 安裝器

Worktree：`/mnt/work/jc/wt/tsf-installer`（branch `tsf-installer`）
任務書：`plan/jc-ime/tsf-installer.md`

## 目標與結果

per-machine MSI，安裝後 `Win+Space` 看得到「jc 注音」，反安裝後不留下檔案、機碼與
使用者的語言清單項目。打包形式是 WiX v5 MSI；理由見 `plan/jc-ime/tsf-plan.md` 的
「打包形式」與 [`../../installer/README.md`](../../installer/README.md)。

## 移植對照

Go 參考實作沒有 MSI，Windows 安裝過程分散在 `0c-update` 與 NSIS 腳本裡。可以對照的
只有「註冊、移除、語言清單、ACL」四段行為。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `cmd/0c-update/platform_windows.go`（`regsvr32 /s`／`/u`、AppContainer ACE） | `installer/jc-tip-register/src/platform.rs`（`machine-register`／`machine-unregister`、`grant_app_container`） | rewritten：不叫 `regsvr32`，直接呼叫 TSF 的 `ITfInputProcessorProfileMgr` 與 `ITfCategoryMgr`；CLSID 由 WXS 的 `<Class>` 寫，錯誤碼才拿得到；ACL 改用 `SetNamedSecurityInfoW` |
| `cmd/0c-update/input_list_windows.go`（`InstallLayoutOrTip` 安裝／移除） | `installer/jc-tip-register/src/platform.rs`（`layout_or_tip`） | ported 行為：旗標 0 安裝、`0x1` 移除，不改預設輸入法 |
| `cmd/0c-update/tsf_profiles_windows.go`（查 CTF profile 機碼） | `crates/jc-tsf-smoke/src/registry.rs`（既有）、`installer/jc-tip-register` 的 `ensure_tip_key` | rewritten：安裝器在註冊後立即驗證 `CTF\TIP\{CLSID}`，冒煙程式提供事後檢查 |
| `platforms/windows/installer/setup.nsi`（NSIS、per-user、`Uninstall` section） | `installer/jc-ime.wxs`＋`installer/build.ps1` | rewritten：per-machine MSI、`MajorUpgrade`、deferred 自訂動作 |
| `cmd/0c-update/uninstall_entry_windows.go`（HKCU 的 `Uninstall\0c-Zhuyin`） | 無 | n/a：MSI 由 Windows Installer 自動建立「應用程式與功能」項目，不需要自寫 ARP 機碼 |
| `scripts/installer/install.ps1.in`、`install.cmd.in`（線上一行安裝） | 無 | n/a：這一輪交付離線 MSI；下載與更新屬 `data-download` 任務 |
| `scripts/package_windows_installer.py`（NSIS 打包） | `installer/build.ps1` | rewritten：cargo → 暫存 payload → `wix build`；不再用 NSIS |

## 交付物

| 路徑 | 內容 |
|---|---|
| `installer/jc-ime.wxs` | MSI：目錄、34 個 `File`（7 程式，含 x64 與 x86 兩份 TSF DLL、1 資料 manifest、5 資料檔、20 授權文件）、兩個 `<Class>`、`MajorUpgrade`、6 個自訂動作 |
| `installer/jc-tip-register/` | 輔助程式（獨立 crate，不進 workspace）：`machine-register`、`machine-unregister`、`user-install`、`user-uninstall`、`settings`、`legal` |
| `installer/build.ps1` | Windows 完整建置與選用簽章 |
| `installer/check-payload.py` | WXS 引用與 manifest 的一致性檢查 |
| `installer/validate-linux.sh` | Linux 上能做到的 WiX 編譯驗證 |
| `docs/install.md` | 使用者的安裝、移除、機碼清單、簽章與實機驗證清單 |

## 設計決定

| 主題 | 決定 | 理由 |
|---|---|---|
| 打包 | WiX v5 MSI，`Scope="perMachine"` | CTF 機碼在 HKLM；MSIX 不提升權限 |
| 註冊 | CLSID 由 WXS 的 `<Class>` 註冊；`machine-register` 只註冊 profile 與三個分類 | rc.7 之後的順序：`WriteRegistryValues` 先寫好兩個檢視的 `InprocServer32`，`RegisterTip` 才不會在 `RegisterCategory` 拿到不存在的類別工廠（rc.5／rc.6 的 `0xC0000005`）。參考專案 windows-chewing-tsf 也是這個分工 |
| 反註冊 | `machine-unregister` 只清 profile 與分類；CLSID 機碼由 MSI 的 `RemoveRegistryValues` 移除 | 兩邊各清各的，不互相搶；DLL 被隔離時反安裝仍然乾淨 |
| 雙架構 | 兩份 DLL 共用同一個 CLSID 與 profile，各自掛一個 `<Class>` | 使用者的語言清單只有一項，32 位元程式不必手動切換；x86 的 `<Class>` 在 `Bitness="always32"` 元件內，WiX 寫到 `Wow6432Node`；見 `docs/port/x86-tip.md` |
| 權限拆分 | 註冊／移除 deferred＋`Impersonate="no"`；語言清單 immediate＋使用者身分 | `InstallLayoutOrTip` 寫呼叫者的 HKCU，SYSTEM 執行會加到 SYSTEM 的 hives |
| 註冊失敗 | `Return="check"` | 不留半套 TIP；rollback 動作會反向清除 |
| 移除失敗 | `Return="ignore"`，且自訂動作不碰 DLL | 防毒隔離 `jc-tsf.dll` 不影響反安裝；CLSID 由 MSI 自己移除，profile 殘留只記日誌 |
| 資料位置 | `%ProgramData%\jc-ime\data`（共用）、`%LOCALAPPDATA%\jc-ime`（使用者） | 程式與資料只裝一次；設定與個人化是使用者資料 |
| Funmula 模型 | Q8_0 GGUF 進 MSI | `zh-select` 依 MIT 全文可散布，`model.q8_0.gguf`／`vocab.json`／兩份 manifest 隨 MSI 安裝；原始 ONNX 與 `zh-punct` 不進 MSI |
| `settings.json` | MSI 不安裝 | 不覆蓋既有檔；第一次按「套用」由設定程式建立 |
| 引擎啟動 | `HKLM\...\Run` 的 `REG_EXPAND_SZ` | `jc-tsf` 只連 pipe，不會拉起引擎；`%LOCALAPPDATA%` 由每位使用者各自展開 |
| 引擎 pipe 名稱 | autostart 不帶 `--socket`，交給 `resolve_pipe_name()` | `jc-tsf` 與 `jc-engine` 共用這套規則，從 SID 尾段推出 `jc-ime-<rid>`；寫死 `--socket jc-ime` 會讓兩邊不同名，TIP 連不上時按鍵全放行成英文。自訂名稱要同時設 `JC_IME_PIPE` 或 `HKCU\Software\jc-ime\EnginePipe` |
| 版本 | `UpgradeCode` 固定、`Version` 取自 workspace `Cargo.toml` | 升級由 `MajorUpgrade` 處理，版本號不在 WXS 手抄 |
| CRT | `-C target-feature=+crt-static` 統一放在 `.cargo/config.toml` 的兩個 Windows target | TSF DLL 被載入 explorer，不能依賴 `vcruntime140.dll`（`docs/history.md`）；build.ps1 與 CI 共用同一組 rustflags，避免指紋失效重編 |

### 引擎與 TIP 的 pipe 名稱一致性

Windows 上 `jc-tsf` 的 `pipe_name()` 與 `jc-engine` 的 `default_supervisor_socket()`
都呼叫 `jc_windows::resolve_pipe_name()`。順序是 `JC_IME_PIPE`、
`HKCU\Software\jc-ime\EnginePipe`、SID 尾段 `jc-ime-<rid>`，最後才退回共用的
`jc-ime`。`EngineAutostart` 只帶 `--data` 與 `--settings`，不帶 `--socket`；
`installer/validate-linux.sh` 會檢查 autostart 沒有 `--socket`，且兩個參數還在。
`crates/jc-windows/src/pipe_name.rs` 的測試鎖住「override 會讓兩端不同名」這件事。

覆寫只適合除錯，而且兩端都要設成同一個值。只改引擎的 `--socket` 時，TIP 仍從
SID 推出 `jc-ime-1001`，引擎監聽另一個名稱，log 會出現「連線引擎 pipe
jc-ime-1001 失敗」，按鍵全部放行成英文。

## WiX v5 與 v3 的差異

RC4 的錯誤是 `Component` 的 `Win64`：WiX v5 schema 沒有這個屬性，會回 `WIX0004`。本專案實際用到的對照：

| 主題 | WiX v3 | WiX v5 | 本檔 |
|---|---|---|---|
| 元件架構 | `Component Win64="no"`／`"yes"` | `Component Bitness="always32"`／`"always64"`；預設 `default` 跟隨套件架構 | `TsfDllX86` 用 `Bitness="always32"` |
| 標準目錄 | `DirectoryRef Id="ProgramFiles64Folder"` 再包 `Directory` | `StandardDirectory Id="ProgramFiles64Folder"` 再包 `Directory` | 用 `StandardDirectory` |
| 套件根元素 | `Product` 內含 `Package` | `Package` 直接放在 `Wix` 下 | 用 `Package` |
| 套件架構 | `Package/@Platform="x64"` | 移除；改用 `wix build -arch x64`，個別元件再用 `Bitness` 覆寫 | `build.ps1` 傳 `-arch x64` |
| Windows Installer 版本 | 常見 `Package/@InstallerVersion="200"` | 仍接受，預設 `500`，一般不需要手動指定 | 本檔沒設 |

`DirectoryRef` 在 v5 仍然存在，用來引用既有目錄；標準目錄改用 `StandardDirectory` 比較清楚。`CustomAction` 等元素在 v3 也有 `Win64`，v5 一律改用 `Bitness`；本檔的自訂動作沒有設定架構。

### 反安裝順序

```text
RollbackUnregisterTip（rollback，SYSTEM）
UnregisterTip（deferred，SYSTEM）        UnregisterCategory ×3 / UnregisterProfile
RemoveRegistryValues（標準順序 2600）    由 <Class> 移除兩個檢視的 CLSID 機碼
RemoveFiles（3500）                      刪程式、資料、Run 值
InstallFinalize
RemoveTipLayout（immediate，使用者）      只刪 %LOCALAPPDATA%\jc-ime
                                          不呼叫 InstallLayoutOrTip(ILOT_UNINSTALL)
```

`UnregisterTip` 排 `Before="RemoveRegistryValues"`（WiX 會排在 2600 之前）。
`<Class Advertise="no">` 產生的是 Registry table 的資料列，CLSID 機碼由
`RemoveRegistryValues` 移除，不是 `UnregisterClassInfo`（2700，只處理 Class
table）；排 `Before="RemoveFiles"`（3500）會太晚，那時機碼已經被刪掉。rc.5 的
真機反安裝之所以看起來沒問題，是因為當時 CLSID 是 helper 自己寫的、MSI 不擁有，
所以 `UnregisterProfile` 執行時它還存在。

`jc-tip-register.exe` 本身到 `RemoveFiles` 才刪，因為 `UnregisterTip` 要用它。
兩個檢視的 CLSID 由 Windows Installer 自己移除，`machine-unregister` 不碰，
兩邊不會互搶。

升級時 `RemoveTipLayout` 的條件排除 `UPGRADINGPRODUCTCODE`，使用者的語言清單項目
不會在版本交替的空檔被移除。

## 驗證

| 指令 | 結果 |
|---|---|
| `cargo check --target x86_64-pc-windows-gnu --all-targets`（`installer/jc-tip-register`） | 通過，0 錯誤 |
| `cargo check --target x86_64-pc-windows-msvc --all-targets`（同上） | 通過，clippy 0 警告 |
| `installer/validate-linux.sh` | WiX 5.0.2 編譯與符號解析通過；綁定階段如預期因 Linux 沒有 `msi.dll` 停止 |
| `python3` 的 `xml.etree.ElementTree.parse` | 通過：37 個 Component；`TsfDllX86` 是唯一的 `Bitness="always32"`；沒有 `Win64`／`Win32`／`DirectoryRef`／`Platform`／`InstallerVersion` |
| `uv run --no-project installer/check-payload.py <payload>` | 通過：WXS 引用 34 個檔案，5 個 `shipped` 資料檔的大小與 SHA-256 全部符合 `docs/port/data-pack.md` 的真實 manifest |
| `wix build`（完整 MSI） | **做不到**，見下 |

### Linux 做不到的部分

WiX v5 官方只支援 Windows，Linux 上的 `wix build` 有兩個限制：

1. `Directory/@Name` 的正規化用 `C:\` 當根目錄，任何名稱都被誤判成「不是相對路徑」
   （WIX0389）。`validate-linux.sh` 在暫存副本把名稱換成 `.` 繞過，只為了驗證 XML、
   屬性、符號引用與自訂動作排程。
2. 綁定階段要載入 Windows 的 `msi.dll`，Linux 沒有，錯誤是 WIX0001。因此沒有產出
   真正的 MSI，也沒有跑 ICE 驗證。

`File/@Source` 統一用 `/` 當分隔符，Windows 的 WiX 也接受，這樣同一個檔案在 Linux
能做編譯驗證。

## 未完成事項

以下沒有在這台機器驗過，全部列入 `docs/install.md` 的「必須在 Windows 驗證」：

1. `wix build` 完整輸出 MSI、ICE 驗證、`msiexec` 安裝與反安裝日誌。
2. `Win+Space` 是否出現「jc 注音」、選取後能否輸入。
3. `InstallFinalize` 之後的 immediate 自訂動作在提權安裝下的實際身分（應為安裝者）。
4. 升級與修復（`msiexec /fa`）後的機碼狀態。
5. AppContainer ACL 是否讓 UWP 應用程式載入 TIP。
6. 32 位元應用程式（例如舊版 Office）與 64 位元應用程式共用同一個「jc 注音」項目，
   而且都能輸入；`reg query ... /reg:32` 指到 `jc-tsf-x86.dll`、`/reg:64` 指到
   `jc-tsf.dll`。
7. 引擎 Run 項目在多使用者、多工作階段下的行為：不同 SID 的 pipe 名稱不同；
   同一使用者的多個 RDP 工作階段仍共用名稱，client 連到哪個實例由 Windows 決定。
8. `installer-payload` 補進來的授權文件：`%ProgramFiles%\jc-ime\legal\` 的實際
   內容、「jc 注音授權」捷徑開啟記事本的行為、反安裝移除整個 `legal\` 目錄。

## 補充：授權文件（`installer-payload`）

原本 MSI 只帶執行檔與資料，`LICENSE`、`NOTICE` 與 `legal/` 都沒進安裝包。這一輪
補齊，做法與理由：

| 主題 | 決定 | 理由 |
|---|---|---|
| 安裝位置 | `%ProgramFiles%\jc-ime\legal\`，`legal\licenses\` 放全文 | 授權全文跟著程式，不散落在資料目錄；`ProgramData` 是共用資料區，反安裝不保證清空 |
| payload 路徑 | 與 repo 相同（`LICENSE`、`NOTICE`、`legal/...`） | `check-payload.py` 直接比對 repo 與 payload 的雜湊，不必另抄一份清單 |
| WXS 結構 | `LegalFiles` 群組，一份檔案一個 `Component`，固定 GUID | 與既有 `AppFiles`／`DataFiles` 一致；修復安裝時逐檔可辨識 |
| 反安裝 | 普通元件，不設 `Permanent` | 跟著 `RemoveFiles` 刪，不留孤兒檔 |
| 捷徑 | 「jc 注音授權」→ `jc-tip-register legal` | `.md` 在乾淨的 Windows 沒有預設關聯，直接指向檔案會跳「選擇應用程式」；輔助程式組路徑並用記事本開啟 |
| 漏檔防線 | `check-payload.py` 以 `legal/licenses/` 的實際內容為清單 | 新增授權全文卻忘了進 WXS 時直接失敗，`build.ps1` 也對缺檔 `throw` |

驗證：

| 指令 | 結果 |
|---|---|
| `cargo check --target x86_64-pc-windows-gnu --all-targets`（`installer/jc-tip-register`） | 通過，0 錯誤（新增 `legal` 子命令與 `open_legal`） |
| `python3 installer/check-payload.py <payload>` | 通過：WXS 引用 32 個檔案、授權文件 20 份 |
| 移除 payload 的 `legal/licenses/MIT.txt` | 失敗，回報「缺少 legal/licenses/MIT.txt」與「payload 缺少授權文件」，結束碼 1 |
| 改動 payload 的 `legal/THIRD-PARTY-NOTICES.md` | 失敗，回報大小不符，結束碼 1 |
| repo 多一份 `legal/licenses/EXTRA.txt` 而 WXS 沒引用 | 失敗，回報「沒有被 WiX 引用」，結束碼 1 |
| `wix build`（完整 MSI） | 這台 Linux 沒有 WiX 與 `msi.dll`，未執行；實機驗證項目見 `docs/install.md` |

## 補充：設定程式捷徑（settings-no-console）

「jc 注音設定」捷徑直接指向 `jc-settings-ui.exe`，不再經過
`jc-tip-register settings`：

| 主題 | 決定 | 理由 |
|---|---|---|
| 捷徑目標 | `[INSTALLFOLDER]jc-settings-ui.exe`，無參數 | `jc-tip-register` 必須維持 console subsystem（`machine-register`／`machine-unregister` 要讓管理員在 PowerShell 看到 stdout）；從它中轉會先開一個終端機 |
| GUI subsystem | `jc-settings-ui` 加 `#![windows_subsystem = "windows"]` | 即使直接從 Shortcut 啟動也不會配置 console 視窗 |
| 路徑展開 | 移到 `jc-settings-ui`：資料用 `%ProgramData%\jc-ime\data`，設定檔用 `%LOCALAPPDATA%\jc-ime\settings.json` | `.lnk` 的參數不展開環境變數，per-machine 捷徑不能寫死單一使用者路徑；原本由 `jc-tip-register` 展開，現在職責跟著搬過去 |
| `machine-*` 輸出 | 不改 | console subsystem 與既有 stdout 保持原樣，PowerShell 輸出不受影響 |

`jc-tip-register settings` 子命令保留作相容入口，但不再是開始功能表的路徑。
「jc 注音授權」捷徑仍經過 `jc-tip-register legal`，開啟時仍會短暫出現
console，這不是本次修正的範圍。

## 建置順序與 throw 前置條件（RC1／RC2 修正）

RC1 的 Windows job 在 `installer/build.ps1:107` 失敗，訊息是找不到
`target\i686-pc-windows-msvc\release`。原因是 `Resolve-ReleaseDir` 在建置之前執行，
乾淨的 runner 還沒有任何 cargo 產物；真正建 i686 `jc-tsf` 的指令原本在後面。
現在 `installer/build.ps1` 的順序固定如下：

1. 解析參數，組出 `$cargoX64Args`、`$cargoX86Args`、`$cargoHelperArgs`。
2. 使用 `.cargo/config.toml` 的 `[target.x86_64-pc-windows-msvc]` 與
   `[target.i686-pc-windows-msvc]` rustflags；`build.ps1` 不設定 `RUSTFLAGS`。
3. 依序執行 x64 workspace、x86 `jc-tsf`、`jc-tip-register` 的 `cargo build`。
4. 建置完成後才呼叫 `Resolve-ReleaseDir`，後續的 `Test-Path` 檢查也都在建置之後。

`Resolve-ReleaseDir` 的錯誤訊息會帶上剛才實際執行的 cargo 指令，以及對應的
`rustup target add`。x86 的訊息因此包含
`cargo build --release --target i686-pc-windows-msvc -p jc-tsf` 與
`rustup target add i686-pc-windows-msvc`。x64 允許退回 `target\<profile>`；x86
不允許退回，避免把 x64 的 `jc-tsf.dll` 當成 x86 DLL 打包。

`jc-tsf` 是 cdylib，Cargo 的 lib target 名稱是 `jc_tsf`，建置產物是 `jc_tsf.dll`；payload 內才改名為 x64 的 `jc-tsf.dll` 與 x86 的 `jc-tsf-x86.dll`，WXS 不用改。其餘 bin target（`jc-engine.exe`、`jc-cli.exe`、`jc-settings-ui.exe`、`jc-tsf-smoke.exe`）保留連字號，已用 `cargo metadata --no-deps` 確認。

### crt-static 與建置指紋

`-C target-feature=+crt-static` 是出貨條件：TSF DLL 會被 explorer 與任意程式載入，不能依賴 `vcruntime140.dll`。旗標統一放在 `.cargo/config.toml` 的 `[target.x86_64-pc-windows-msvc]` 與 `[target.i686-pc-windows-msvc]`，`build.ps1` 不再設定 `RUSTFLAGS`。

Windows job 不再先跑 `cargo build --workspace`。「驗證資料包」只用 `cargo run -p jc-cli` 建驗證所需的 `jc-cli`，其餘 crate 由 `build.ps1` 建。兩邊讀同一份 config，cargo 指紋一致，`build.ps1` 會沿用已編譯的產物，不再因為 `RUSTFLAGS` 不同而全量重編。

Windows target tree 的 `cc` 只用於 GPUI 的 `embed-resource` 前置處理 `.rc`，沒有自行編譯 C/C++ 靜態庫。`cargo check -p jc-settings-ui --target x86_64-pc-windows-msvc` 與 `cargo check -p jc-tsf --target i686-pc-windows-msvc` 都通過；MSVC 連結本身只能在 Windows CI 或實機驗證。

### Windows 的 Python 輸出

Windows 上執行的 Python 腳本，stdout／stderr 必須使用 UTF-8。Windows 的 Python 3.12 預設是 cp1252，`check-payload.py` 印中文會直接 `UnicodeEncodeError`，Linux 的 UTF-8 環境看不出來。

目前有三層保護：

1. `installer/check-payload.py` 開頭呼叫 `force_utf8_stdio()`；有 `reconfigure` 就用，舊版 Python 改用 `TextIOWrapper` 包住 `buffer`。
2. `installer/build.ps1` 呼叫 python 前設 `PYTHONUTF8=1` 與 `PYTHONIOENCODING=utf-8`。
3. `.github/workflows/release.yml` 的 Windows job 設 `PYTHONUTF8=1`。

Windows job 目前唯一呼叫的 Python 腳本是 `installer/check-payload.py`；`scripts/` 下的模型轉檔與評測工具使用 Linux 路徑，不在 Windows job 內。

### MSI 檔名與 ProductVersion

| 項目 | 來源 | RC1 的值 |
|---|---|---|
| MSI 檔名 | `$Version`，取自 workspace `Cargo.toml` 的完整版本 | `target\installer\jc-ime-0.1.0-rc.1-x64.msi` |
| MSI ProductVersion | `-MsiVersion`；未給時由完整版本去掉 `-<pre-release>` | `0.1.0` |
| WXS `Version` | `wix build -d Version=$MsiVersion` | `0.1.0` |

workflow 的 `RELEASE_VERSION` 是完整版本，`MSI_VERSION` 是三段數字，兩者都取自
同一份 `Cargo.toml`。`build.ps1` 的檔名用完整版本、ProductVersion 用三段數字，
與 `.github/workflows/release.yml` 期望的
`target\installer\jc-ime-0.1.0-rc.1-x64.msi` 一致。

### 每個 throw 的前置條件

行號以本版 `installer/build.ps1` 為準；條件成立才會走到該 `throw`。

| 位置 | 訊息 | 前置條件與處理 |
|---|---|---|
| `Invoke-Checked`（80） | `<Label> 失敗（結束碼 N）` | 被包裹的原生命令回傳非 0。PowerShell 的 `$ErrorActionPreference = 'Stop'` 不會攔原生命令，所以由 `$LASTEXITCODE` 檢查；修正該 cargo、`check-payload.py` 或簽章指令。 |
| `Resolve-ReleaseDir`（164） | 找不到建置輸出 ... | 三個 cargo build 都執行過，但 `target\<triple>\<profile>` 不存在。x64 可退回 `target\<profile>`；x86 不行。訊息含實際 cargo 指令與 `rustup target add`。 |
| 資料根目錄（173） | 沒有資料根目錄 | `-DataRoot` 與 `JC_DATA_ROOT` 都是空值。 |
| 資料根目錄（176） | 資料根目錄不存在 | `$DataRoot` 非空，但 `Test-Path $DataRoot` 為 false。 |
| 版本（182） | Cargo.toml 找不到 [workspace.package] version | `Select-String` 在 repo 根目錄 `Cargo.toml` 找不到行首 `version = "..."`。確認 `[workspace.package]` 有 `version`。 |
| 版本格式（185） | 版本必須是 x.y.z 或 x.y.z-<pre-release> | `$Version` 不符 `^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$`。 |
| MSI 版本（192） | MSI ProductVersion 必須是 x.y.z | `$MsiVersion` 沒給時由 `$Version` 去掉 `-` 後綴；結果仍不是三段數字，或使用者傳入的值格式錯誤。 |
| payload 程式（210） | 建置輸出缺少 `<name>` | `$ReleaseDir` 內缺 `jc-engine.exe`、`jc-cli.exe`、`jc-settings-ui.exe`、`jc-tsf-smoke.exe` 其中之一。這些 bin target 保留連字號。 |
| x64 TSF DLL（216） | 建置輸出缺少預期的 jc_tsf.dll ... | `$ReleaseDir\jc_tsf.dll` 不存在。訊息會列出目錄中實際存在的 `.dll`，可直接看出連字號與底線不符。 |
| x86 TSF DLL（224） | 建置輸出缺少預期的 jc_tsf.dll ... | `$ReleaseDirX86\jc_tsf.dll` 不存在。訊息會列出該目錄實際存在的 `.dll`，並提示 `rustup target add i686-pc-windows-msvc`。 |
| 指定 manifest（236） | 指定的資料 manifest 不存在 | 有 `-Manifest` 或 `JC_DATA_MANIFEST`，但該路徑不存在。 |
| manifest 內容（246） | manifest 沒有任何 shipped: true 的檔案 | `manifest.json` 可解析，但 `files[].shipped` 全是 false 或缺少欄位。 |
| manifest 檔案（251） | manifest 列出的檔案不存在 | 某個 shipped entry 的 `path` 轉成 `\` 後，在 `$DataRoot` 下找不到對應檔案。 |
| 授權文件（267） | 缺少授權文件 `<name>` | repo 根目錄缺 `LICENSE` 或 `NOTICE`。 |
| 授權文件（272） | 缺少授權文件 legal\THIRD-PARTY-NOTICES.md | repo 缺 `legal\THIRD-PARTY-NOTICES.md`。 |
| 授權全文（277） | legal\licenses 沒有任何授權全文 | `legal\licenses` 目錄存在但沒有任何檔案。若目錄本身不存在，前面的 `Get-ChildItem` 會先以 terminating error 失敗。 |
| 簽章（298） | 指定了簽章憑證但找不到 signtool.exe | 有 `-Certificate` 或 `JC_SIGN_CERT`，且沒有 `-SkipSign`，但 PATH 找不到 `signtool.exe`。 |
| WiX（312） | 找不到 wix.exe | PATH 找不到 `wix.exe`，通常是沒有安裝 WiX v5 或 dotnet tool 目錄不在 PATH。 |

### 這輪的靜態檢查

這台 Linux 沒有 `pwsh`，所以沒有實際執行 `build.ps1`。已逐段核對：

- 三個 `cargo build` 都位在 `Resolve-ReleaseDir` 與所有執行期 `Test-Path` 之前。
- 建置與錯誤訊息共用同一組參數陣列，錯誤中的 cargo 指令與實際執行一致。
- `$cargoX86Args` 明確帶 `--target i686-pc-windows-msvc`，錯誤訊息會顯示同一個 triple。
- `.cargo/config.toml` 的 x86_64 與 i686 Windows target 都設 `+crt-static`；`build.ps1` 已移除 `$env:RUSTFLAGS`。
- `cargo check -p jc-settings-ui --target x86_64-pc-windows-msvc` 與 `cargo check -p jc-tsf --target i686-pc-windows-msvc` 通過；Linux 沒有 MSVC 連結器，連結留給 Windows CI。
- `release.yml` 的 Windows job 已移除 `cargo build --workspace`，資料驗證只建 `jc-cli`。
- `check-payload.py` 在 `PYTHONIOENCODING=cp1252` 下仍能輸出中文；`reconfigure` 與舊版 `TextIOWrapper` 兩條路徑都測過。
- `$msi` 用 `$Version`（完整版本）命名，`wix build` 的 `-d Version` 用 `$MsiVersion`（三段數字）。
- `$Version`、`$MsiVersion`、`$ReleaseDir`、`$ReleaseDirX86` 都在第一次使用前賦值；`Set-StrictMode -Version Latest` 下沒有未定義變數。
- `git diff --check` 通過。

## RC5：RegisterTip 存取違規（0xC0000005）

> 這一節記錄 rc.5 的觀察與當時的介面。`machine-register`／`machine-unregister` 在
> RC7 已改成不吃 DLL 路徑，CLSID 也改由 MSI 寫；現行流程見文末「RC7：CLSID 交給
> MSI」。崩點與 bisect 結論沒有變。

RC5 的 MSI 在真機安裝時，`RegisterTip` 回 `-1073741819`（`0xC0000005`），耗時
236 ms。同一個 exe、同一份 DLL 的 rollback `machine-unregister` 呼叫
`DllUnregisterServer` 沒有崩，所以 `LoadLibraryW`、`CoCreateInstance` 與
`ITfCategoryMgr` 都正常，問題限縮在只有註冊才會走的路徑。

### 兩個 bug

| 位置 | 問題 | 修正 |
|---|---|---|
| `crates/jc-tsf/src/register.rs` 的 `module_path` | `GetModuleFileNameW(None, ...)` 的 `hModule = NULL` 拿到的是行程的 exe。`DllRegisterServer` 由 `jc-tip-register.exe` 載入後呼叫，`InprocServer32` 因此寫成 `jc-tip-register.exe` | 改用 `GetModuleHandleExW`，以 `DllRegisterServer` 所在模組的函式位址當錨點（`FROM_ADDRESS | UNCHANGED_REFCOUNT`），再對這個 module 取路徑 |
| `AddLanguageProfile` 的 `pchIconFile` | 四個呼叫點都傳 `&[]`。空 slice 的 `as_ptr()` 是對齊用的非 NULL 位址（`0x2`），msctf 收到非 NULL 指標配上 `cchIconFile = 0` | 改傳 `&NO_ICON[..0]`：指標指向真正的 NUL、長度仍是 0，API 讀得到也不會越界 |

`InprocServer32` 必須是 DLL 的絕對路徑，不能是 `jc-tip-register.exe`。安裝後各查
一次：

```bat
reg query "HKCR\CLSID\{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}\InprocServer32" /ve /reg:64
reg query "HKCR\CLSID\{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}\InprocServer32" /ve /reg:32
```

64 位元檢視要指到 `...\jc-tsf.dll`，32 位元檢視要指到 `...\jc-tsf-x86.dll`。
指到 `.exe` 代表 `module_path` 又退回了行程路徑。

### 安裝日誌

`jc-tip-register` 每個動作寫一行到 `%TEMP%\jc-ime-install.log`。deferred 自訂
動作以 SYSTEM 執行，`%TEMP%` 是 `C:\Windows\Temp`；一般使用者身分執行時則是該
使用者的 `%TEMP%`。取不到 `%TEMP%` 時寫在 `jc-tip-register.exe` 同目錄。

日誌是 **UTF-8 with BOM**：第一次建立檔案時先寫 BOM，之後附加的內容不重複。
Windows PowerShell 5.1 的 `Get-Content` 預設用 ANSI 解讀，對這份日誌不用加
`-Encoding`，中文才不會變亂碼。

`jc-tip-register` 用環境變數 `JC_INSTALL_LOG` 把同一個路徑傳給 `jc-tsf.dll`，
DLL 內的標記會附加到同一份檔案。手動重跑時也可以自己設這個變數（rc.7 之後
`machine-register` 不吃參數）：

```bat
set JC_INSTALL_LOG=%TEMP%\jc-manual.log
jc-tip-register.exe machine-register
```

格式是 `[time] [pid] step`，一行一個動作。下面是 rc.5 的日誌（當時還會載入 DLL；
rc.7 起改為直接呼叫 TSF 介面，步驟名稱換成 `RegisterProfile` 與
`RegisterCategory`）：

```text
[1763700000123] [4321] 啟動參數：machine-register ...\jc-tsf.dll ...\jc-tsf-x86.dll
[1763700000124] [4321] 檢查 DLL：...\jc-tsf.dll
[1763700000125] [4321] 解析架構：...\jc-tsf.dll → x64
[1763700000126] [4321] LoadLibraryW：...\jc-tsf.dll
[1763700000127] [4321] LoadLibraryW：成功（HMODULE(0x...））
[1763700000128] [4321] GetProcAddress：DllRegisterServer
[1763700000129] [4321] 呼叫 DllRegisterServer
[1763700000130] [4322] jc-tsf register：開始
[1763700000131] [4322] module_path：C:\Program Files\jc-ime\jc-tsf.dll
```

`[time]` 是 Unix epoch 毫秒，`[pid]` 是行程 ID。每一行都獨立開檔、`flush`、
`sync_all`，沒有緩衝聚合；行程被存取違規終止後，最後一行仍在磁碟上。

怎麼看：

1. 先看最後一行。它前面的標記就是崩潰或失敗的位置。
2. 有「呼叫 DllRegisterServer」卻沒有「DllRegisterServer 回傳 ...」，代表崩在
   DLL 內；接著找同一個 pid 的 `jc-tsf register` 標記。
3. 有「回傳 0x...」是 HRESULT 失敗，錯誤明確。`0xC0000005` 不會被回報，行程
   直接終止，只能靠最後一個標記定位。
4. 「ensure_tip_key：成功」代表
   `HKLM\SOFTWARE\Microsoft\CTF\TIP\{CLSID}` 的 64 位元與 32 位元檢視都讀得到。

### 驗證

| 指令 | 結果 |
|---|---|
| `cargo check --workspace --all-targets` | 通過，0 錯誤 |
| `cargo check --target x86_64-pc-windows-msvc -p jc-tsf`（repo 根目錄） | 通過，0 錯誤 |
| `cargo check --target x86_64-pc-windows-msvc`（`installer/jc-tip-register`） | 通過，0 錯誤 |
| `cargo test`（`installer/jc-tip-register`） | 15 passed；純解析測試，Linux 可跑 |
| 真機安裝 MSI、查兩個 `InprocServer32` | 未執行，需 Windows |

### 第二次 bisect：崩點在 `ITfCategoryMgr`

用 RC5 的檔案在真機手動執行，結果：

| 觀察 | 結果 |
|---|---|
| `machine-register` | 崩 `0xC0000005` |
| `machine-unregister` | exit 0 |
| `HKLM\SOFTWARE\Microsoft\CTF\TIP\{CLSID}\Enable` | 已寫入 `1` |
| `...\LanguageProfile\0x00000404\{PROFILE}\Description` | 已寫入「jc 注音」 |
| `reg query HKLM\SOFTWARE\Microsoft\CTF /s /f "{CLSID}"` | 只有 TIP 主鍵 1 個相符，沒有任何分類資料 |
| 安裝目錄 ACL 的 `S-1-15-2-1`／`S-1-15-2-2` | 沒有 |

`Register` 與 `AddLanguageProfile` 都成功，崩點在 `ITfCategoryMgr` 的
`RegisterCategory`。後面的 `ensure_tip_key` 與 `grant_app_container` 都沒有跑到，
所以 ACL 也沒有加上。日誌因此把 CategoryMgr 這一段補到「前後各一行、每次呼叫都
帶 HRESULT」的粒度：

```text
[time] [pid] CoCreateInstance(CLSID_TF_CategoryMgr)：開始
[time] [pid] CoCreateInstance(CLSID_TF_CategoryMgr)：成功（HRESULT 0x00000000）
[time] [pid] RegisterCategory(GUID_TFCAT_TIP_KEYBOARD)：開始
[time] [pid] RegisterCategory(GUID_TFCAT_TIP_KEYBOARD)：成功（HRESULT 0x00000000）
```

下一次若再崩，最後一行會直接指出是哪一個 `RegisterCategory`。

### AppContainer ACL 預設關閉

`grant_app_container` 現在只在環境變數 `JC_IME_APP_CONTAINER=1` 時執行，預設跳過，
日誌會留下「grant_app_container：已跳過（未設定 JC_IME_APP_CONTAINER）」。

預設關閉的理由：

- `S-1-15-2-1`／`S-1-15-2-2` 只給 UWP／AppContainer 應用程式載入 TIP 用，桌面程式
  不受影響。
- 這是 `GetNamedSecurityInfoW` → `SetEntriesInAclW` → `SetNamedSecurityInfoW` 的
  手工 ACL 操作，放在安裝關鍵路徑上，任何一步出錯都會讓整個註冊失敗。
- 這次 bisect 已確認崩點在 CategoryMgr，但 ACL 不該是安裝的必要條件；需要 UWP
  支援時再明確開啟即可。

開啟方式：

```bat
set JC_IME_APP_CONTAINER=1
jc-tip-register.exe machine-register
```

開啟後用 `icacls "%ProgramFiles%\jc-ime"` 確認兩個 SID 的讀取／執行 ACE 存在。

## RC7：CLSID 交給 MSI、反安裝不呼叫 `ILOT_UNINSTALL`

rc.5／rc.6 的結論是：`Register`（當時）與 `AddLanguageProfile` 都寫入成功，
`RegisterCategory` 沒寫入，ACL 也沒改，行程在 CategoryMgr 那一段以 `0xC0000005`
結束。參考專案 windows-chewing-tsf 的分工提供了對照：**`InprocServer32` 由 MSI 的
`<Class>` 寫，不是由 DLL 的 `DllRegisterServer` 寫**。我們的版本在
`RegisterCategory` 執行時，64 位元檢視的 CLSID 根本不存在（或指到 exe），
msctf 拿不到類別工廠，這就是崩因。

### 新的分工

| 機碼 | 誰寫 | 時機 |
|---|---|---|
| `HKCR\CLSID\{CLSID}` 與 `...\InprocServer32`（64 位元檢視） | WXS 的 `<Class Advertise="no">`（`TsfDll` 元件） | `WriteRegistryValues`（Registry table 的資料列） |
| 同一組機碼（`Wow6432Node`） | 同上（`TsfDllX86`，`Bitness="always32"`） | `WriteRegistryValues` |
| `HKLM\SOFTWARE\Microsoft\CTF\TIP\{CLSID}` | `machine-register` 的 `RegisterProfile` | `RegisterTip`，在 `WriteRegistryValues` 之後 |
| `HKLM\SOFTWARE\Microsoft\CTF\Category` | `machine-register` 的三次 `RegisterCategory` | 同上 |
| 使用者語言清單 | `user-install` 的 `InstallLayoutOrTip(ILOT_INSTALL)` | `InstallFinalize` 之後，使用者身分 |

`Advertise="no"` 是關鍵：WiX 文件寫明它會「generate Registry rows that perform
similar registration」，而不是 Class table。`RegisterClassInfo`／
`UnregisterClassInfo` 只處理 Class table，這份 WXS 沒有那些資料列。

```text
InstallFiles
WriteRegistryValues（標準順序 5000）
  ├─ <Class Id="7B7D6E1A-…" Context="InprocServer32" ThreadingModel="apartment" Advertise="no">
  │    TsfDll       → HKCR\CLSID\{…}\InprocServer32          = …\jc-tsf.dll
  │    TsfDllX86    → HKCR\Wow6432Node\CLSID\{…}\InprocServer32 = …\jc-tsf-x86.dll
  └─ RegisterTip（deferred、SYSTEM）  machine-register
       CoCreateInstance(CLSID_TF_InputProcessorProfiles) → ITfInputProcessorProfileMgr
       RegisterProfile(CLSID, 0x0404, PROFILE, "jc 注音", 無圖示, 0, HKL::default(), 0, true, 0)
       CoCreateInstance(CLSID_TF_CategoryMgr) → ITfCategoryMgr
       RegisterCategory ×3（TIP_KEYBOARD、TIPCAP_UIELEMENTENABLED、DISPLAYATTRIBUTEPROVIDER）
       ensure_tip_key（只讀，驗證 64／32 位元檢視）
InstallFinalize
InstallTipLayout（immediate、使用者）  user-install → InstallLayoutOrTip(ILOT_INSTALL)
```

`RegisterProfile` 的 `bEnabledByDefault` 傳 `true`，`LanguageProfile\0x0404\{PROFILE}`
的 `Enable` 會寫成 `1`。使用者把「jc 注音」加入鍵盤（MSI 由 `InstallTipLayout`
先加進語言清單）之後就能直接切換，不會像 `Enable=0` 時選了也切不過去。這裡仍然
不呼叫 `ActivateProfile`，所以不會改掉使用者的系統預設輸入法。

反安裝：

```text
UnregisterTip（deferred、SYSTEM，Before="RemoveRegistryValues"）  machine-unregister
  UnregisterCategory ×3
  UnregisterProfile(CLSID, 0x0404, PROFILE, 0)
  （不刪 HKCR\CLSID，也不刪 Wow6432Node 那一份）
RemoveRegistryValues（2600）    移除兩個檢視的 CLSID
RemoveFiles（3500）
InstallFinalize
RemoveTipLayout（immediate、使用者）  user-uninstall
  只刪 %LOCALAPPDATA%\jc-ime；不呼叫 InstallLayoutOrTip(ILOT_UNINSTALL)
```

### 為什麼 CLSID 交給 MSI

1. **順序保證**：`Advertise="no"` 的 `<Class>` 由 WiX 展開成 Registry table 的
   資料列，`WriteRegistryValues` 負責寫入；`RegisterTip` 排在它之後，
   `RegisterCategory` 執行時兩個檢視都查得到類別工廠。反安裝則反過來：
   `UnregisterTip` 排在 `RemoveRegistryValues` 之前，`UnregisterProfile` 執行時
   CLSID 還在。
2. **少一層 API 表面**：不用載入 DLL、不用判斷 PE 架構，也就不需要在安裝器裡重寫
   `InprocServer32`。`machine-register` 因此沒有參數，`arch.rs`、
   `write_com_registration`、`register_in_store`、`Target`、`delete_clsid_tree`
   全部移除。
3. **反安裝對稱**：`<Class>` 產生的是 MSI 自己的登錄資料，`RemoveRegistryValues`
   會把 64 位元與 `Wow6432Node` 兩份一起刪。自訂動作若先刪，MSI 就找不到目標，
   兩邊清理範圍也會不一致。
4. **profile 用 profile 的 API**：`ITfInputProcessorProfileMgr::RegisterProfile`
   一次寫完 TIP 機碼與 `LanguageProfile` 子鍵，不必再走
   `ITfInputProcessorProfiles::Register` + `AddLanguageProfile` 兩個呼叫。這與參考
   專案的 `crates/tsfreg` 相同。

`crates/jc-tsf/src/register.rs` 的檔案頭已註明：MSI 安裝不走
`DllRegisterServer`，那份程式只給手動 `regsvr32` 修復用。`module_path()` 的
`GetModuleHandleExW` 修正保留，手動路徑仍然需要它。

### 為什麼反安裝不呼叫 `ILOT_UNINSTALL`

移除某個語系的最後一個 layout 時，Windows 會把系統地區（system locale）改成英文
或其他可用語言，使用者的介面語言、日期格式與非 Unicode 程式的編碼全部跟著變。
參考專案在 `disable()` 明確停用這一步並留下註解，追蹤見
<https://codeberg.org/chewing/windows-chewing-tsf/issues/553>。

我們的做法相同：

- `user-uninstall` 只寫一行日誌說明原因，不再呼叫 `InstallLayoutOrTip`。
- `RemoveTipLayout` 保留（`ExeCommand="user-uninstall [KEEPUSERSETTINGS]"`），
  但只剩刪 `%LOCALAPPDATA%\jc-ime` 的功能；`KEEPUSERSETTINGS=1` 仍然有效。
- `ILOT_UNINSTALL` 常數從 `guids.rs` 移除，避免有人又接回去。
- MSI 移除 profile 之後，語言清單裡的「jc 注音」會變成無效項目，由使用者自己到
  「設定 → 時間與語言 → 語言」移除。

### 介面變化

| 子命令 | rc.6 | rc.7 |
|---|---|---|
| `machine-register` | `<dll>...`、`--arch`；x64 交 DLL 註冊、x86 自己寫機碼 | 不吃參數；只註冊 profile 與三個分類 |
| `machine-unregister` | `[<dll>...]`；可交 DLL 或自己清 | 不吃參數；只清 profile 與分類 |
| `user-install` | `InstallLayoutOrTip(ILOT_INSTALL)` | 不變 |
| `user-uninstall` | `InstallLayoutOrTip(ILOT_UNINSTALL)` + 刪使用者資料 | 只刪使用者資料 |

參數若還是照舊帶進來，`parse_args` 直接回用法錯誤（結束碼 2），不會默默忽略。
測試 `machine_register_takes_no_arguments`、`machine_unregister_takes_no_arguments`
固定這個行為。

### 驗證

| 指令 | 結果 |
|---|---|
| `cargo check --workspace --all-targets` | 通過，0 錯誤 |
| `cargo check --target x86_64-pc-windows-msvc -p jc-tip-register -p jc-tsf`（`installer/jc-tip-register` 與 repo 根目錄各跑一次） | 通過，0 錯誤、0 警告 |
| `cargo test -p jc-tip-register` | 9 passed（Linux 可跑的部分：參數解析與 GUID） |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通過，0 警告 |
| `cargo fmt --all -- --check` | 通過 |
| `installer/validate-linux.sh` | WiX 編譯與符號解析通過；IR 內有兩組 `CLSID\{…}\InprocServer32`，各指向自己的 DLL |
| 真機安裝 MSI、`Win+Space`、兩個 `InprocServer32` | 未執行，需 Windows |
