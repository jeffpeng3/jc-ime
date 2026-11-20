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
| `cmd/0c-update/platform_windows.go`（`regsvr32 /s`／`/u`、AppContainer ACE） | `installer/jc-tip-register/src/platform.rs`（`machine-register`／`machine-unregister`、`grant_app_container`） | rewritten：不叫 `regsvr32`，直接 `LoadLibraryW`＋`DllRegisterServer`，錯誤碼才拿得到；ACL 改用 `SetNamedSecurityInfoW` |
| `cmd/0c-update/input_list_windows.go`（`InstallLayoutOrTip` 安裝／移除） | `installer/jc-tip-register/src/platform.rs`（`layout_or_tip`） | ported 行為：旗標 0 安裝、`0x1` 移除，不改預設輸入法 |
| `cmd/0c-update/tsf_profiles_windows.go`（查 CTF profile 機碼） | `crates/jc-tsf-smoke/src/registry.rs`（既有）、`installer/jc-tip-register` 的 `ensure_tip_key` | rewritten：安裝器在註冊後立即驗證 `CTF\TIP\{CLSID}`，冒煙程式提供事後檢查 |
| `platforms/windows/installer/setup.nsi`（NSIS、per-user、`Uninstall` section） | `installer/jc-ime.wxs`＋`installer/build.ps1` | rewritten：per-machine MSI、`MajorUpgrade`、deferred 自訂動作 |
| `cmd/0c-update/uninstall_entry_windows.go`（HKCU 的 `Uninstall\0c-Zhuyin`） | 無 | n/a：MSI 由 Windows Installer 自動建立「應用程式與功能」項目，不需要自寫 ARP 機碼 |
| `scripts/installer/install.ps1.in`、`install.cmd.in`（線上一行安裝） | 無 | n/a：這一輪交付離線 MSI；下載與更新屬 `data-download` 任務 |
| `scripts/package_windows_installer.py`（NSIS 打包） | `installer/build.ps1` | rewritten：cargo → 暫存 payload → `wix build`；不再用 NSIS |

## 交付物

| 路徑 | 內容 |
|---|---|
| `installer/jc-ime.wxs` | MSI：目錄、32 個 `File`（6 程式＋1 資料 manifest＋5 資料檔＋20 授權文件）、`MajorUpgrade`、6 個自訂動作 |
| `installer/jc-tip-register/` | 輔助程式（獨立 crate，不進 workspace）：`machine-register`、`machine-unregister`、`user-install`、`user-uninstall`、`settings` |
| `installer/build.ps1` | Windows 完整建置與選用簽章 |
| `installer/check-payload.py` | WXS 引用與 manifest 的一致性檢查 |
| `installer/validate-linux.sh` | Linux 上能做到的 WiX 編譯驗證 |
| `docs/install.md` | 使用者的安裝、移除、機碼清單、簽章與實機驗證清單 |

## 設計決定

| 主題 | 決定 | 理由 |
|---|---|---|
| 打包 | WiX v5 MSI，`Scope="perMachine"` | CTF 機碼在 HKLM；MSIX 不提升權限 |
| 註冊 | `machine-register` 呼叫 DLL 自己的 `DllRegisterServer` | 註冊邏輯只有一份（`crates/jc-tsf/src/register.rs`），不在安裝器重抄 |
| 權限拆分 | 註冊／移除 deferred＋`Impersonate="no"`；語言清單 immediate＋使用者身分 | `InstallLayoutOrTip` 寫呼叫者的 HKCU，SYSTEM 執行會加到 SYSTEM 的 hives |
| 註冊失敗 | `Return="check"` | 不留半套 TIP；rollback 動作會反向清除 |
| 移除失敗 | `Return="ignore"`＋內建退路 | DLL 被防毒隔離時仍能清掉 CTF 機碼，反安裝不會卡死 |
| 資料位置 | `%ProgramData%\jc-ime\data`（共用）、`%LOCALAPPDATA%\jc-ime`（使用者） | 程式與資料只裝一次；設定與個人化是使用者資料 |
| 下載項 | 不進 MSI | Funmula 的 `zh-select`、`zh-punct` 授權只允許使用者下載 |
| `settings.json` | MSI 不安裝 | 不覆蓋既有檔；第一次按「套用」由設定程式建立 |
| 引擎啟動 | `HKLM\...\Run` 的 `REG_EXPAND_SZ` | `jc-tsf` 只連 pipe，不會拉起引擎；`%LOCALAPPDATA%` 由每位使用者各自展開 |
| 版本 | `UpgradeCode` 固定、`Version` 取自 workspace `Cargo.toml` | 升級由 `MajorUpgrade` 處理，版本號不在 WXS 手抄 |
| CRT | `-C target-feature=+crt-static` | TSF DLL 被載入 explorer，不能依賴 `vcruntime140.dll`（`docs/history.md`） |

### 反安裝順序

```text
RollbackUnregisterTip（rollback，SYSTEM）
UnregisterTip（deferred，SYSTEM）        RemoveLanguageProfile / Unregister /
                                          UnregisterCategory / RegDeleteTree HKCR
RemoveFiles                              刪程式、資料、Run 值
InstallFinalize
RemoveTipLayout（immediate，使用者）      InstallLayoutOrTip(ILOT_UNINSTALL)
                                          ＋ 刪 %LOCALAPPDATA%\jc-ime
```

先移除 profile 再刪檔案；反過來會留下刪不掉的 `CTF\TIP` 機碼。`jc-tip-register.exe`
本身到 `RemoveFiles` 才刪，因為 `UnregisterTip` 要用它。

升級時 `RemoveTipLayout` 的條件排除 `UPGRADINGPRODUCTCODE`，使用者的語言清單項目
不會在版本交替的空檔被移除。

## 驗證

| 指令 | 結果 |
|---|---|
| `cargo check --target x86_64-pc-windows-gnu --all-targets`（`installer/jc-tip-register`） | 通過，0 錯誤 |
| `installer/validate-linux.sh` | WiX 5.0.2 編譯與符號解析通過；綁定階段如預期因 Linux 沒有 `msi.dll` 停止 |
| `uv run --no-project installer/check-payload.py <payload>` | 通過：WXS 引用 12 個檔案，5 個 `shipped` 資料檔的大小與 SHA-256 全部符合 `docs/port/data-pack.md` 的真實 manifest |
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
6. 32 位元應用程式看不到 TIP（只建 x64 DLL，已知限制）。
7. 引擎 Run 項目在多使用者、多工作階段下的行為：每個工作階段各一個引擎，pipe 同名，
   client 連到哪個實例由 Windows 決定。
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
