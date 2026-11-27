# x86／x64 TSF 元件

TSF 的 TIP 是 in-process COM 物件：應用程式要求輸入法時，`msctf.dll` 會在**自己的
行程裡** `CoCreateInstance` 那個 CLSID。行程的位元數決定它讀哪一份 COM 機碼，也
決定它載得進哪一種 DLL：

| 行程 | COM 機碼檢視 | DLL |
|---|---|---|
| 64 位元（explorer、新版 Office、Edge） | `HKLM\SOFTWARE\Classes\CLSID\{…}\InprocServer32` | `jc-tsf.dll` |
| 32 位元（舊版 Office、部分工具） | `HKLM\SOFTWARE\Classes\Wow6432Node\CLSID\{…}\InprocServer32` | `jc-tsf-x86.dll` |

產品出兩個檔案，共用同一個 CLSID 與 profile：

| 常數 | 值 |
|---|---|
| CLSID | `{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}` |
| profile | `{C4E8D217-5B3F-4A96-8E0C-7F1A2B3D4E56}` |
| 描述 | `jc 注音` |
| 語言 ID | `0x0404`（zh-TW） |

常數定義在 `crates/jc-tsf/src/guids.rs`、`crates/jc-tsf-register/src/guids.rs`
與 `installer/jc-tip-register/src/guids.rs`。

## 為什麼共用一個 CLSID

使用者的語言清單是 `(LangID, profile GUID, CLSID)` 的清單。兩個架構各用一個 CLSID
會出現兩個「jc 注音」，而且只有架構相符的那一個能在該程式裡運作。共用 CLSID 是
Windows 的標準解法：同一個 CLSID 在兩個機碼檢視各有一個 `InprocServer32`，分別
指向 x64 與 x86 DLL。

## 註冊

`installer/jc.iss` 的 `[Registry]` 統一寫兩個檢視的 `InprocServer32` 與
`PipePrefix`；`jc-tip-register` 只註冊 profile 與分類。`uninsdeletekey` 讓反安裝
清掉整個 CLSID 子樹。

修改後用兩個檢視各讀一次：

```bat
reg query "HKLM\SOFTWARE\Classes\CLSID\{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}\InprocServer32" /reg:64
reg query "HKLM\SOFTWARE\Classes\CLSID\{2F6B41D8-9E07-4C3A-B5F1-8D2C6A9E0473}\InprocServer32" /reg:32
```

64 位元應指向 `jc-tsf.dll`，32 位元指向 `jc-tsf-x86.dll`。

## 建置

`installer/build.ps1` 用 `x86_64-pc-windows-msvc` 建 x64 元件，再用
`i686-pc-windows-msvc` 建一份 `jc-tsf`，暫存成 `jc-tsf-x86.dll`。兩個 target 都
用 `.cargo/config.toml` 的 `-C target-feature=+crt-static`；TSF DLL 會被 explorer
載入，不能依賴 `vcruntime140.dll`。

## 排查

- 只有 32 位元程式打不出注音：確認 `InprocServer32` 的 `reg:32` 預設值是
  `jc-tsf-x86.dll`。
- 兩個檢視指向同一個檔案：安裝器沒有寫入 `KEY_WOW64_32KEY`，重跑 `setup.exe`。
- ARM64 Windows 可用 x64 模擬載入 `jc-tsf.dll`，但沒有 ARM64 原生建置。
