# x86-tip：32 位元 TSF 元件

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

所以產品出兩個檔案：

| 檔案 | 建置 | 說明 |
|---|---|---|
| `jc-tsf.dll` | `cargo build -p jc-tsf --target x86_64-pc-windows-msvc --release` | 64 位元 |
| `jc-tsf-x86.dll` | `cargo build -p jc-tsf --target i686-pc-windows-msvc --release` | 32 位元；cargo 的產物同樣叫 `jc-tsf.dll`，暫存 payload 時改名 |

兩份的原始碼完全相同，差別只在 `guids.rs` 依 `target_pointer_width` 選到的
CLSID、profile GUID 與顯示名稱。引擎（`jc-engine.exe`）、CLI、設定程式與冒煙
程式仍然只有 x64 版：它們是獨立的行程，不受宿主位元數影響。

## GUID 對照

| 名稱 | 值 | 用途 |
|---|---|---|
| `CLSID_JC_TSF_X64` | `{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}` | x64 文字服務物件 |
| `CLSID_JC_TSF_X86` | `{3E9A5C72-8B14-4D6F-A0C8-5F2B7E9D1346}` | x86 文字服務物件 |
| `GUID_PROFILE_JC_TSF_X64` | `{8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}` | x64 語言 profile |
| `GUID_PROFILE_JC_TSF_X86` | `{6B1F8D43-5A27-4C90-9E36-7D4B2F0A8C51}` | x86 語言 profile |
| 顯示名稱（x64） | `jc 注音` | 語言列與輸入法清單 |
| 顯示名稱（x86） | `jc 注音（32 位元）` | 同上；兩個 profile 必須分得出來 |
| LANGID | `0x0404`（zh-TW） | 兩個 profile 共用 |
| `GUID_DISPLAY_ATTRIBUTE_INPUT` | `{A2C9F1D4-7B3E-4C58-9E21-5D8A6F0B4C72}` | 組字顯示屬性；兩個架構共用 |
| `GUID_CANDIDATE_UI_ELEMENT` | `{9C4E7A21-6B3D-4F08-B5A7-2E1D8C6F9034}` | 候選窗 UI 元素；兩個架構共用 |

`guids.rs` 另外提供 `CLSID_JC_TSF`、`GUID_PROFILE_JC_TSF` 與 `PROFILE_NAME` 三個
別名，由 `cfg(target_pointer_width)` 指向上面其中一組；TSF 的程式碼只看到別名。

「兩個架構共用」的兩個 GUID 不經過 COM：`TfGuidAtom` 與 UI 元素 GUID 都只在
單一行程內有意義，共用不會衝突。

測試鎖住的部分（`crates/jc-tsf/src/guids.rs`、`installer/jc-tip-register/src/guids.rs`）：

- `arch_specific_guids_differ`：兩組 CLSID／profile／顯示名稱兩兩不同。
- `active_guids_follow_target_arch`：這份 DLL 選到的是自己架構的那一組。
- `specs_use_their_own_guids_and_views`：安裝器兩個架構的 GUID 與機碼檢視
  （`KEY_WOW64_64KEY`／`KEY_WOW64_32KEY`）沒有互相汙染。
- `detects_x64_and_x86_machine_types`、`rejects_other_machine_types`：PE 標頭的
  架構判斷（含 ARM64 不猜成 x64）。

## 註冊流程

安裝時跑兩次 `jc-tip-register machine-register`，一次一個架構：

```text
RegisterTip      machine-register "[#FileTsfDll]"                  （x64）
RegisterTipX86   machine-register --arch x86 "[#FileTsfDllX86]"    （x86）
```

`jc-tip-register` 是 x64 程式，`LoadLibraryW` **載入不了** 32 位元 DLL
（`ERROR_BAD_EXE_FORMAT`），所以依目標架構分兩條路徑：

| 情況 | 做法 |
|---|---|
| 同架構（x64 DLL） | `LoadLibraryW` → `DllRegisterServer`；註冊邏輯只有 `crates/jc-tsf/src/register.rs` 一份 |
| 跨架構（x86 DLL） | 安裝器自己用 `KEY_WOW64_32KEY` 寫 `InprocServer32`，再呼叫同一組 TSF 介面（`ITfInputProcessorProfiles`、`ITfCategoryMgr`） |

跨架構那條路徑的內容與 DLL 內的同名函式逐項對應：寫 `InprocServer32`
（`ThreadingModel=Apartment`）、`Register`、`AddLanguageProfile`、三個
`RegisterCategory`（`TIP_KEYBOARD`、`TIPCAP_UIELEMENTENABLED`、
`DISPLAYATTRIBUTEPROVIDER`）。TSF 的 store（`HKLM\SOFTWARE\Microsoft\CTF`）不是
WOW64 轉向機碼，兩個架構寫的是同一個 64 位元位置，所以 x64 的安裝器代替 x86 的
DLL 註冊是合法的。

`--arch` 省略時由 DLL 的 PE 標頭自動判斷；MSI 明確指定，指定值與檔案不符時
直接失敗（寫錯檢視會留下一個永遠載入不了的 TIP）。

註冊完每個架構都會驗一次 `HKLM\SOFTWARE\Microsoft\CTF\TIP\{CLSID}`，所以
`RegisterTipX86` 用 `Return="check"`：任何一邊註冊失敗就讓安裝失敗並回滾，不留
半套。

安裝目錄的 AppContainer ACE 在兩次註冊都會設一次。`grant_app_container` 用
`SET_ACCESS`（取代該 trustee 的既有 ACE）而不是 `GRANT_ACCESS`，重跑與修復安裝
不會讓 DACL 愈長愈多。

### 使用者語言清單

`InstallLayoutOrTip` 收的是 `(LangID, profile GUID, CLSID)` 三元組，兩個架構各
一組，少一組就會有一個架構的程式在 `Win+Space` 裡看不到輸入法：

```text
0x0404:{8F0A9B3C-2D4E-4F50-A1B2-C3D4E5F60718}{7B7D6E1A-3C1F-4B2E-9A55-6C1D2F3E4A01}
0x0404:{6B1F8D43-5A27-4C90-9E36-7D4B2F0A8C51}{3E9A5C72-8B14-4D6F-A0C8-5F2B7E9D1346}
```

`user-install`／`user-uninstall` 因此各呼叫 `InstallLayoutOrTip` 兩次；第一組
失敗仍然會試第二組，最後回報第一個錯誤。

## 反安裝流程

```text
UnregisterTip      machine-unregister "[#FileTsfDll]"                （x64）
UnregisterTipX86   machine-unregister --arch x86 "[#FileTsfDllX86]"  （x86）
RemoveFiles
InstallFinalize
RemoveTipLayout    user-uninstall [KEEPUSERSETTINGS]                 （兩個三元組都移除）
```

每個架構的移除順序都是「先 `RemoveLanguageProfile`、`Unregister`、
`UnregisterCategory`，再刪 `HKCR\CLSID\{…}`」；順序反過來會留下刪不掉的
`CTF\TIP` 機碼，那個機碼會讓「語言」設定頁一直顯示一個載入不了的輸入法。

`RegDeleteTreeW` 沒有檢視參數，所以刪 x86 的 CLSID 時先用 `KEY_WOW64_32KEY` 開
`HKCR\CLSID`，再相對刪除子機碼——handle 的檢視決定後續所有操作。

DLL 自己的 `DllUnregisterServer` 只在同架構時使用；DLL 不在（防毒隔離）或跨架構
時走上面的內建路徑。`machine-unregister` 不帶路徑時兩個架構都清，這是手動修復
與退路用的。

## 移植對照

Go 參考實作沒有 TSF、沒有 32 位元 Windows 外殼，這一輪沒有可搬的行為；既有的
GUID 與註冊程式碼是在原處擴充。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| n/a（Go 沒有 TSF） | `crates/jc-tsf/src/guids.rs`（新增 x86 CLSID／profile、架構別名） | rewritten |
| n/a | `crates/jc-tsf/src/register.rs`（改用架構別名與顯示名稱） | rewritten |
| n/a | `crates/jc-tsf/src/lib.rs`（建置說明、日誌帶架構與 CLSID） | rewritten |
| n/a | `crates/jc-tsf/src/composition.rs`（`SetWindowLongPtrW` 在 32 位元的參數型別） | rewritten：i686 才編得過的修正，不涉及行為 |
| n/a | `installer/jc-tip-register/src/arch.rs`（PE 架構偵測） | rewritten |
| `cmd/0c-update/platform_windows.go`（`regsvr32` 呼叫） | `installer/jc-tip-register/src/platform.rs`（跨架構直接註冊、兩組 GUID、兩次 `InstallLayoutOrTip`） | rewritten |
| n/a | `installer/jc-ime.wxs`（`TsfDllX86` 元件、四個新的自訂動作） | rewritten |
| n/a | `installer/build.ps1`（兩個 target、payload 改名、簽章清單） | rewritten |
| n/a | `.github/workflows/release.yml`（windows job 加 i686 target） | rewritten |

## 驗證

| 指令 | 結果 |
|---|---|
| `cargo check --workspace --all-targets` | 通過 |
| `cargo check --target i686-pc-windows-msvc -p jc-tsf` | 通過 |
| `cargo check --target x86_64-pc-windows-msvc -p jc-tsf` | 通過 |
| `cargo clippy --target i686-pc-windows-msvc -p jc-tsf --all-targets` | 0 警告 |
| `cargo check --target x86_64-pc-windows-msvc --all-targets`（`installer/jc-tip-register`） | 通過 |
| `cargo check --target x86_64-pc-windows-gnu --all-targets`（`installer/jc-tip-register`） | 通過 |
| `check-payload.py`（WXS 引用 34 個檔案） | 通過；拿掉 payload 的 `jc-tsf-x86.dll` 後回報「缺少 jc-tsf-x86.dll（File Id FileTsfDllX86）」、結束碼 1 |
| WXS 結構檢查（自訂動作、FileRef、Component GUID、`Win64="no"`） | 通過：10 個自訂動作、34 個 File、37 個 Component |
| `installer/validate-linux.sh` | **未執行**：這台機器的 .NET runtime 不見了，`wix` 起不來 |
| `wix build`／MSI／ICE | 未執行；`wix` 需要 Windows |

單元測試依 `common.md` 只寫、只編譯，不執行。

## 需要實機驗證

1. 32 位元程式（舊版 Office、`C:\Windows\SysWOW64\` 的小工具）在 `Win+Space` 裡
   看得到「jc 注音（32 位元）」並打得出字；64 位元程式用「jc 注音」。
2. 兩個 profile 同時出現在「設定 → 時間與語言 → 語言 → 中文（台灣）」時，使用者
   分得出來哪個是哪個，切換後的輸入法圖示與行為正確。
3. `HKLM\SOFTWARE\Classes\Wow6432Node\CLSID\{3E9A5C72-…}\InprocServer32` 指向
   `%ProgramFiles%\jc-ime\jc-tsf-x86.dll`，`ThreadingModel` 是 `Apartment`。
   64 位元的 `HKLM\SOFTWARE\Classes\CLSID\{7B7D6E1A-…}` 沒有被蓋掉。
4. `HKLM\SOFTWARE\Microsoft\CTF\TIP\` 底下兩個 CLSID 都在，各自的
   `LanguageProfile\0x0404\{…}` 顯示名稱正確。
5. 反安裝後兩個 CLSID、兩個 profile、兩個 `HKCR` 機碼與兩個 `jc-tsf*.dll` 都不
   留；使用者的語言清單兩個項目都消失。
6. 升級安裝（舊版 x64-only → 新版雙架構）後，舊的 x64 profile 不會變成孤兒，
   使用者的語言清單會補上 x86 那一組。
7. MSI 日誌裡 `RegisterTipX86` 的 `--arch x86` 有正確解析，且 rollback 動作能
   反向清除。
8. `i686-pc-windows-msvc` 的 crate 在 Windows 上連結得出真的 DLL（Linux 只驗到
   `cargo check`，沒有 MSVC linker）。
9. `wix build` 的 ICE 驗證：32 位元元件（`Win64="no"`）裝在
   `ProgramFiles64Folder` 底下，ICE80 可能會提出警告。這是 WiX 對「32 位元元件
   落在 64 位元目錄」的常規提醒，這個元件沒有任何轉向的檔案或機碼，實際不受
   影響；若驗證把警告升級成錯誤，把 `TsfDllX86` 的 `Win64` 拿掉即可（檔案只由
   明確的 `Directory` 決定位置）。

## 取捨：兩個 CLSID，還是共用一個

Windows 本身**不要求**兩個架構用不同 CLSID：同一個 CLSID 在兩個機碼檢視各指到
不同 DLL 是合法且常見的做法，使用者的語言清單只會有一個項目，TSF 依行程位元數
挑對的 DLL。

這一版照任務書用兩個 CLSID，代價是：

- 語言清單有兩個項目，32 位元程式要切到「jc 注音（32 位元）」才有作用；
  選到另一個的話 TSF 在該行程裡建立不了物件，該程式收不到任何注音輸入。
- 兩個 profile 各自記住設定（未來的候選數量等）沒有共用。

好處是機碼與日誌一眼看得出架構，`DllGetClassObject` 也只認自己那一個 CLSID，
不會有「同一個 CLSID 在不同行程指向不同檔案」的除錯困難。

若實機驗證後決定改成共用 CLSID，需要動的地方很少：`guids.rs` 兩組常數合併、
`installer/jc-tip-register` 的 `Spec::clsid` 兩個架構共用同一值、
`user-install` 只送一個三元組、`UnregisterTipX86` 不再需要。機碼檢視與
`Win64="no"` 元件都不用改。

## 待同步的文件

以下檔案不在這個 task 的 owned paths，內容仍寫著「只有 x64 TIP」，需要由擁有者
更新：

- `docs/install.md`：檔案清單要加 `jc-tsf-x86.dll`、機碼清單要加 `Wow6432Node`
  與第二個 CLSID、反安裝段落要說明兩個架構。
- `docs/port/tsf-skeleton.md`：GUID 表格與「未驗證清單」第 14 項（ARM64／其他
  架構）要改寫成兩個架構。
- `docs/port/tsf-installer.md`：自訂動作表要加 `RegisterTipX86`／
  `UnregisterTipX86`，交付物表格的 `File` 數量要更新。

`installer/README.md` 屬於本 task 的 `installer/**`，已同步更新。
