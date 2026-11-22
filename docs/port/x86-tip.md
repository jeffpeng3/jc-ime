# x86-tip：32 位元 TSF 元件

> 下一次 rc 已移除舊版，本文的 CLSID／profile 是移除前的歷史值，僅供對照。
> 現行身分與指令見 [`install.md`](../install.md) 與 [`rollout.md`](rollout.md)。


Worktree：`/mnt/work/jc/wt/x86-tip`（branch `x86-tip`）
任務書：`plan/jc-ime/x86-tip.md`

## 為什麼需要兩份 DLL

TSF 的 TIP 是 in-process COM 物件：應用程式要求輸入法時，`msctf.dll` 會在
**自己的行程裡** `CoCreateInstance` 那個 CLSID。行程的位元數決定它讀哪一份 COM
機碼，也決定它載得進哪一種 DLL：

| 宿主行程 | 讀的機碼 | 能載入的 DLL |
|---|---|---|
| 64 位元（explorer、新版 Office、Edge／Chromium） | `HKLM\SOFTWARE\Classes\CLSID\{…}` | x64 |
| 32 位元（舊版 Office、部分工具與安裝程式） | `HKLM\SOFTWARE\Classes\Wow6432Node\CLSID\{…}` | x86 |

只有一份 x64 DLL 時，32 位元程式在它自己的檢視裡找不到 `InprocServer32`，輸入法
就等於不存在。這是 64 位元 Windows 的 WOW64 設計，不是 TSF 的 bug，也不是靠
manifest 或 MSI 屬性可以繞過的。

所以產品出兩個檔案，**共用同一個 CLSID 與 profile**：

| 檔案 | 建置 | 註冊的檢視 |
|---|---|---|
| `jc-tsf.dll` | `cargo build -p jc-tsf --target x86_64-pc-windows-msvc --release` | 64 位元（`KEY_WOW64_64KEY`） |
| `jc-tsf-x86.dll` | `cargo build -p jc-tsf --target i686-pc-windows-msvc --release` | 32 位元（`KEY_WOW64_32KEY`、`Wow6432Node`） |

cargo 的產物兩個 target 都叫 `jc-tsf.dll`，暫存 payload 時把 i686 那份改名。

兩份的原始碼完全相同，`guids.rs` 沒有依架構分岔的常數；唯一的差別是
`register.rs` 的 `REGISTRY_VIEW`（`cfg(target_pointer_width)`）決定寫哪一個檢視。
引擎（`jc-engine.exe`）、CLI、設定程式與冒煙程式仍然只有 x64 版：它們是獨立的
行程，不受宿主位元數影響。

## 為什麼共用一個 CLSID

使用者的語言清單是 `(LangID, profile GUID, CLSID)` 的清單。如果兩個架構各用一個
CLSID，清單就會出現兩個「jc 注音」，而且只有架構相符的那一個能在該程式裡運作
——使用者得為 32 位元程式手動切換，這是不能接受的體驗。

共用 CLSID 的做法是 Windows 的標準解法：**同一個 CLSID 在兩個機碼檢視各有一個
`InprocServer32`，指向該架構的 DLL**。TSF 依行程位元數挑對的那一份，使用者只看
到一個輸入法、不必切換。

## GUID 對照

| 名稱 | 值 | 用途 |
|---|---|---|
| `CLSID_JC_TSF` | `{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}` | 文字服務物件；兩個檢視共用 |
| `GUID_PROFILE_JC_TSF` | `{8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}` | 注音 profile；兩個架構共用一個 |
| 顯示名稱 | `jc 注音` | 語言列與輸入法清單，只有一項 |
| LANGID | `0x0404`（zh-TW） | 第一版只有繁體中文 |
| `GUID_DISPLAY_ATTRIBUTE_INPUT` | `{A2C9F1D4-7B3E-4C58-9E21-5D8A6F0B4C72}` | 組字顯示屬性 |
| `GUID_CANDIDATE_UI_ELEMENT` | `{9C4E7A21-6B3D-4F08-B5A7-2E1D8C6F9034}` | 候選窗 UI 元素 |

`GUID_DISPLAY_ATTRIBUTE_INPUT` 與 `GUID_CANDIDATE_UI_ELEMENT` 不經過 COM：
`TfGuidAtom` 與 UI 元素 GUID 都只在單一行程內有意義，共用不會衝突。

測試鎖住的部分（`crates/jc-tsf/src/guids.rs`、`installer/jc-tip-register/src/guids.rs`、
`installer/jc-tip-register/src/platform.rs`）：

- `one_clsid_and_profile_for_both_architectures`、`clsid_matches_tsf_crate`：兩個
  建置拿到同一組 CLSID／profile／顯示名稱，值沒有漂移。
- `categories_are_five_distinct_guids`：五個分類都在，而且互不相同。
- `machine_register_takes_no_arguments`、`machine_unregister_takes_no_arguments`：
  安裝器不再接受 DLL 路徑或 `--arch`。

## 註冊流程

兩個檢視的 CLSID 都由 WXS 的 `<Class>` 寫，`machine-register` 只註冊 profile 與
分類：

```text
WriteRegistryValues（標準順序 5000）
  <Class Id="7B7D6E1A-…" Context="InprocServer32" ThreadingModel="apartment">
    TsfDll      → HKCR\CLSID\{…}\InprocServer32             = …\jc-tsf.dll
    TsfDllX86   → HKCR\Wow6432Node\CLSID\{…}\InprocServer32 = …\jc-tsf-x86.dll
RegisterTip        machine-register                                  （deferred、SYSTEM）
```

| 元件 | Bitness | 寫入的檢視 | 預設值 |
|---|---|---|---|
| `TsfDll` | 預設（x64 套件 → 64 位元） | `HKCR\CLSID\{…}\InprocServer32` | `…\jc-tsf.dll` |
| `TsfDllX86` | `always32` | `HKCR\Wow6432Node\CLSID\{…}\InprocServer32` | `…\jc-tsf-x86.dll` |

`<Class>` 省略 `Server` 表示伺服器就是同一個 `<File>`，所以兩個 `File` 都必須
`KeyPath="yes"`。rc.6 之前由 `DllRegisterServer`（x64）與安裝器自己
（`KEY_WOW64_32KEY`）各寫一份；rc.7 換成 MSI 統一寫，理由是
`RegisterCategory` 需要 64 位元檢視的 CLSID 已經存在，否則 msctf 會存取違規
（`0xC0000005`）。完整記錄見 [`tsf-installer.md`](tsf-installer.md) 的 RC7 一節。

profile 與分類（`HKLM\SOFTWARE\Microsoft\CTF`）**不是** WOW64 轉向機碼，兩個架構
讀寫同一份資料，所以整支程式只註冊一次：`ITfInputProcessorProfileMgr::RegisterProfile`
加三個 `RegisterCategory`（`TIP_KEYBOARD`、`TIPCAP_UIELEMENTENABLED`、
`DISPLAYATTRIBUTEPROVIDER`）。

DLL 內的 `register.rs` 仍保留給手動 `regsvr32`：它用 `cfg(target_pointer_width)`
選 `REGISTRY_VIEW`，只寫自己那個檢視。MSI 安裝不走這條。

註冊完會用兩個檢視各讀一次 `HKLM\SOFTWARE\Microsoft\CTF\TIP\{CLSID}`：64 位元
檢視讀不到就讓安裝失敗（`Return="check"`，不留半套）；32 位元檢視讀不到只送到
debugger（DebugView）警告，因為那是共用的一份資料，真正的判準是下面的
`reg query /reg:32`。

安裝目錄的 AppContainer ACE 只設一次，而且用 `SET_ACCESS`（取代該 trustee 的
既有 ACE）而不是 `GRANT_ACCESS`，修復安裝不會讓 DACL 愈長愈多。目錄取自
`jc-tip-register.exe` 自己的位置，不需要 MSI 傳路徑。

### 使用者語言清單

`InstallLayoutOrTip` 收的是 `(LangID, profile GUID, CLSID)` 三元組，共用 CLSID 與
profile 之後只有一組：

```text
0x0404:{8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}
```

## 反安裝流程

```text
UnregisterTip      machine-unregister                                  （deferred、SYSTEM，
                                                                        Before RemoveRegistryValues）
RemoveRegistryValues（既定順序 2600）                                  <Class> 移除兩個檢視
RemoveFiles（3500）
InstallFinalize
RemoveTipLayout    user-uninstall [KEEPUSERSETTINGS]
```

`machine-unregister` 只做 `UnregisterCategory` 三次與 `UnregisterProfile`。
`HKCR\CLSID` 兩個檢視由 MSI 的 `<Class>` 登錄資料移除：`Advertise="no"` 產生的是
Registry table 的資料列，移除發生在 `RemoveRegistryValues`（2600），比
`UnregisterClassInfo`（2700）與 `RemoveFiles`（3500）都早，所以 `UnregisterTip`
排 `Before="RemoveRegistryValues"`，`UnregisterProfile` 執行時 CLSID 還在。DLL 被
防毒隔離時，`machine-unregister` 完全不碰檔案，反安裝不會卡住。

反安裝**不呼叫** `InstallLayoutOrTip(ILOT_UNINSTALL)`：移除語系的最後一個 layout
會讓 Windows 把系統地區改成英文（windows-chewing-tsf issue 553）。

## 移植對照

Go 參考實作沒有 TSF、沒有 32 位元 Windows 外殼，這一輪沒有可搬的行為；既有的
GUID 與註冊程式碼是在原處擴充。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| n/a（Go 沒有 TSF） | `crates/jc-tsf/src/guids.rs`（單一 CLSID／profile 的說明與測試） | rewritten |
| n/a | `crates/jc-tsf/src/register.rs`（`REGISTRY_VIEW` 明確指定機碼檢視） | rewritten |
| n/a | `crates/jc-tsf/src/lib.rs`（建置說明、日誌帶架構與 CLSID） | rewritten |
| n/a | `crates/jc-tsf/src/composition.rs`（`SetWindowLongPtrW` 在 32 位元的參數型別） | rewritten：i686 才編得過的修正，不涉及行為 |
| n/a | `crates/jc-tsf/Cargo.toml`（加 `Win32_Security`，`RegCreateKeyExW` 在那個 feature 下） | rewritten |
| n/a | `installer/jc-tip-register/src/arch.rs`（PE 架構偵測） | 已移除：CLSID 交給 MSI 的 `<Class>` 之後不需要判斷 DLL 架構 |
| `cmd/0c-update/platform_windows.go`（`regsvr32` 呼叫） | `installer/jc-tip-register/src/platform.rs`（單一 profile、五個分類） | rewritten：CLSID 改由 MSI 的 `<Class>` 寫，安裝器不再載入 DLL |
| n/a | `installer/jc-ime.wxs`（`TsfDllX86` 元件、兩個 `<Class>`） | rewritten |
| n/a | `installer/build.ps1`（兩個 target、payload 改名、簽章清單） | rewritten |
| n/a | `.github/workflows/release.yml`（windows job 加 i686 target） | rewritten |

## 驗證

| 指令 | 結果 |
|---|---|
| `cargo check --workspace --all-targets` | 通過（本機缺 libfontconfig，需 `--exclude jc-settings-ui`，見下） |
| `cargo check --target i686-pc-windows-msvc -p jc-tsf` | 通過 |
| `cargo check --target x86_64-pc-windows-msvc -p jc-tsf` | 通過 |
| `cargo clippy --target i686-pc-windows-msvc -p jc-tsf --all-targets` | 0 警告 |
| `cargo clippy --target x86_64-pc-windows-msvc -p jc-tsf --all-targets` | 0 警告 |
| `cargo check --target x86_64-pc-windows-msvc --all-targets`（`installer/jc-tip-register`） | 通過 |
| `cargo check --target x86_64-pc-windows-gnu --all-targets`（同上） | 通過 |
| `cargo clippy --target x86_64-pc-windows-msvc --all-targets`（同上） | 0 警告 |
| `check-payload.py`（WXS 引用 34 個檔案） | 通過；拿掉 payload 的 `jc-tsf-x86.dll` 後回報「缺少 jc-tsf-x86.dll（File Id FileTsfDllX86）」、結束碼 1 |
| WXS 結構檢查（自訂動作、FileRef、Component GUID、`Bitness="always32"`） | 通過：6 個自訂動作、34 個 File、37 個 Component |
| `installer/validate-linux.sh` | **未執行**：這台機器的 .NET runtime 不見了，`wix` 起不來 |
| `wix build`／MSI／ICE | 未執行；`wix` 需要 Windows |

`cargo check --workspace --all-targets` 在這台 Linux 上會停在
`yeslogic-fontconfig-sys`：系統沒有 `libfontconfig1-dev`，`pkg-config` 找不到
`fontconfig.pc`。這與本次改動無關；`--exclude jc-settings-ui` 之後其餘 crate
全部通過。

單元測試依 `common.md` 只寫、只編譯，不執行。

## `reg query` 驗證步驟（實機）

以「系統管理員」開啟命令提示字元。`/reg:32`、`/reg:64` 只在 64 位元 Windows
有效；32 位元 Windows 上兩個檢視是同一個，指令會報錯，屬正常。

**1. 64 位元檢視指到 x64 DLL**

```bat
reg query "HKLM\SOFTWARE\Classes\CLSID\{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}\InprocServer32" /reg:64
```

預期：

```text
    (預設)    REG_SZ    C:\Program Files\jc-ime\jc-tsf.dll
    ThreadingModel    REG_SZ    Apartment
```

**2. 32 位元檢視指到 x86 DLL（同一個 CLSID）**

```bat
reg query "HKLM\SOFTWARE\Classes\CLSID\{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}\InprocServer32" /reg:32
```

預期同上，但預設值是 `C:\Program Files\jc-ime\jc-tsf-x86.dll`。
等價的完整路徑也可以直接查：

```bat
reg query "HKLM\SOFTWARE\Classes\Wow6432Node\CLSID\{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}\InprocServer32"
```

**3. 只有一個 profile，而且兩個檢視都讀得到**

```bat
reg query "HKLM\SOFTWARE\Microsoft\CTF\TIP\{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}" /s /reg:64
reg query "HKLM\SOFTWARE\Microsoft\CTF\TIP\{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}" /s /reg:32
```

預期兩次的輸出相同，都含
`LanguageProfile\0x0404\{8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}` 與
`Description = jc 注音`。`CTF` 不是 WOW64 轉向機碼，所以 `/reg:32` 看到的是同一份
資料；這裡若看不到，代表該版 Windows 把它轉向了，需要先確認 CTF 是不是轉向機碼
（`reg query /reg:32` 與 `/reg:64` 的輸出不同），再決定要不要改用 32 位元身分
重跑 `machine-register`。

**4. 反安裝後兩個檢視都乾淨**

```bat
reg query "HKLM\SOFTWARE\Classes\CLSID\{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}" /reg:64
reg query "HKLM\SOFTWARE\Classes\CLSID\{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}" /reg:32
reg query "HKLM\SOFTWARE\Microsoft\CTF\TIP\{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}"
```

三條都應該回「找不到」或 ERROR: The system was unable to find the specified
registry key or value。

## 需要實機驗證

1. 32 位元程式（舊版 Office、`C:\Windows\SysWOW64\` 的小工具）在 `Win+Space` 裡
   看得到**一個**「jc 注音」並打得出字；64 位元程式用同一個項目也正常。
2. 語言清單沒有重複項目，切換後圖示與行為一致。
3. 上面的四組 `reg query` 都符合預期。跑 `jc-tsf-smoke.exe` 時檢查 7 也要是
   `[PASS]`（它做的正是這四組查詢的程式版）。
4. 反安裝後兩個 CLSID 機碼、CTF 機碼、兩個 `jc-tsf*.dll` 與使用者語言清單的項目
   都不留。
5. 升級安裝（舊版 x64-only → 新版雙架構）：舊版的 x64 註冊不會變成孤兒，
   使用者語言清單不需要重加。
6. MSI 日誌裡 `RegisterTip` 執行 `machine-register`（不吃參數）、`WriteRegistryValues`
   排在它之前，rollback 動作能反向清乾淨。
7. `i686-pc-windows-msvc` 的 crate 在 Windows 上連結得出真的 DLL（Linux 只驗到
   `cargo check`，沒有 MSVC linker）。
8. `wix build` 的 ICE 驗證：32 位元元件（`Bitness="always32"`）裝在
   `ProgramFiles64Folder` 底下，ICE80 可能會提出警告。這個元件沒有任何轉向的
   檔案或機碼，實際不受影響；若驗證把警告升級成錯誤，把 `TsfDllX86` 的 `Bitness`
   拿掉即可（檔案位置只由明確的 `Directory` 決定）。

## 已知限制

- `jc-tip-register` 是 x64 程式，永遠不會從 32 位元行程執行；反安裝的
  `InstallLayoutOrTip` 也是 x64 身分，這對 `input.dll` 沒有影響（它寫的是呼叫者
  的 HKCU，與 COM 架構無關）。
- ARM64 Windows 的 x64 模擬可以載入 x64 DLL，但沒有 ARM64 原生建置；這仍列在
  `docs/port/tsf-skeleton.md` 的未驗證清單。
