# installer-payload：授權文件與其他 payload 進 MSI

Worktree：`/mnt/work/jc/wt/installer-payload`（branch `installer-payload`）
任務書：`plan/jc-ime/installer-payload.md`

## 目標與結果

`LICENSE`、`NOTICE`、`legal/THIRD-PARTY-NOTICES.md` 與 `legal/licenses/` 全部 17 份
授權全文都進 MSI，安裝到 `%ProgramFiles%\jc-ime\legal\`；開始功能表多一個
「jc 注音授權」捷徑；`build.ps1` 與 `check-payload.py` 在缺檔時失敗；反安裝連
`legal\` 目錄一起刪除。使用者文件在 [`../../docs/install.md`](../../docs/install.md)
的「授權文件位置」。

## 移植對照

Go 參考實作沒有 MSI，授權文件由 NSIS 的安裝頁面與 Python 打包腳本處理。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `platforms/windows/installer/setup.nsi`（`MUI_PAGE_LICENSE "terms.txt"`，安裝時顯示使用條款） | `installer/jc-ime.wxs`（`LegalFiles`、`LegalShortcut`） | rewritten：MSI 沒有安裝精靈頁面，改成裝完可隨時開啟的 `legal\` 目錄加開始功能表捷徑 |
| `scripts/package_windows_installer.py`（`required` 清單含 `LICENSE`、`TERMS.md`、`THIRD_PARTY_NOTICES.txt`、`legal/README.md`） | `installer/check-payload.py`（授權文件清單以 `legal/licenses/` 實際內容為準） | rewritten：改成 repo／payload／WXS 三邊比對，並逐檔驗證雜湊 |
| `scripts/package_release.py`（`legal/TERMS.md` → `TERMS.md` 的改名對照） | `installer/build.ps1`（授權文件以 repo 相對路徑暫存） | rewritten：不再改名，payload 路徑與 repo 相同 |
| `scripts/collect_notices.py`（彙整第三方 notice） | 無 | n/a：本專案已有 `legal/THIRD-PARTY-NOTICES.md` 人工維護，自動彙整不屬本任務 |
| `platforms/windows/installer/setup.nsi` 的 `Uninstall` section（刪安裝目錄） | `installer/jc-ime.wxs` 的 `RemoveFiles`（普通元件，不設 `Permanent`） | rewritten |

## 設計決定

| 主題 | 決定 | 理由 |
|---|---|---|
| 安裝位置 | `%ProgramFiles%\jc-ime\legal\`，全文在 `legal\licenses\` | 授權文件跟著程式走；`%ProgramData%` 是共用資料區，反安裝不保證清空 |
| payload 路徑 | 與 repo 相同（`LICENSE`、`NOTICE`、`legal/...`） | `check-payload.py` 可直接比對 repo 與 payload 的雜湊，不必維護第二份清單 |
| WXS 結構 | `LegalFiles` 群組，一檔一 `Component`，固定 GUID | 與 `AppFiles`／`DataFiles` 一致；修復安裝時逐檔可辨識 |
| 反安裝 | 普通元件，不設 `Permanent` | 跟著 `RemoveFiles` 刪，不留孤兒檔 |
| 捷徑 | 「jc 注音授權」→ `jc-tip-register legal` → 記事本開 `THIRD-PARTY-NOTICES.md` | `.md` 在乾淨的 Windows 沒有預設關聯，`.lnk` 直接指向檔案會跳「選擇應用程式」 |
| 漏檔防線 | `check-payload.py` 以 `legal/licenses/` 實際內容為清單，並要求 WXS 引用 | 新增全文忘了進 WXS、payload 少檔、內容與 repo 不符都會失敗；`build.ps1` 對缺檔 `throw` |

## 驗證

| 指令 | 結果 |
|---|---|
| `cargo check --target x86_64-pc-windows-gnu --all-targets`（`installer/jc-tip-register`） | 通過，0 錯誤 |
| `cargo fmt --check`（`installer/jc-tip-register`） | 通過 |
| `python3 installer/check-payload.py <payload>` | 通過：WXS 引用 32 個檔案、授權文件 20 份 |
| 移除 payload 的 `legal/licenses/MIT.txt` | 失敗，回報「缺少 legal/licenses/MIT.txt」與「payload 缺少授權文件」，結束碼 1 |
| 改動 payload 的 `legal/THIRD-PARTY-NOTICES.md` | 失敗，回報大小不符，結束碼 1 |
| repo 多一份 `legal/licenses/EXTRA.txt` 而 WXS 沒引用 | 失敗，回報「沒有被 WiX 引用」，結束碼 1 |
| WXS 解析（`xml.etree`） | 通過，32 個 `File/@Source` |

Linux 上沒有 `wix` 與 `xmllint`，`validate-linux.sh` 與完整 `wix build` 未執行。
Windows 實機驗證項目列在 `docs/install.md` 的「必須在 Windows 驗證」，其中第 11 項
對應本任務。

## 未完成事項

1. `wix build` 產出 MSI、ICE 驗證、實機安裝與反安裝日誌（需要 Windows）。
2. 「jc 注音授權」捷徑在沒有 `.md` 關聯的機器上是否確實開記事本。
3. 反安裝後 `%ProgramFiles%\jc-ime\legal\` 是否整個消失、Program Files 是否留下空
   目錄。
