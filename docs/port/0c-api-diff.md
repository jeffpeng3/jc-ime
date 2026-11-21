# 0c API 差異對照：windows-rs 0.61.3 → 0.62.2

移植 0c 的 Windows TSF 程式碼到 jc-ime 之前，對照兩版 windows-rs 的 API。0c source 在 `/mnt/work/jc/ref/0c-ime/platforms/windows/tsf/src`，版本以各 crate 的 `Cargo.lock` 為準。

## 版本對應

| 項目 | 0c（舊） | jc-ime（新） |
|---|---|---|
| `windows` | 0.61.3 | 0.62.x（registry 為 0.62.2） |
| `windows-core` | 0.61.2 | 0.62.2 |
| `windows-implement` | 0.60.2 | 0.60.2（相同） |
| `windows-link` | 0.1.3 | 0.2.1 |
| `windows-result` | 0.3.4 | 0.4.1 |

0c 的 `platforms/windows/Cargo.lock` 解析出 `windows-implement 0.60.2`、`windows-link 0.1.3`、`windows-result 0.3.4`；新版 windows-core 0.62.2 解析出 `windows-implement 0.60.2`、`windows-link 0.2.1`、`windows-result 0.4.1`。

## 差異總覽

會影響 0c 原始碼的差異只有三類：

1. `windows_link::link!()` 改為 `windows_core::link!()`。0.62 的 windows-core 重新匯出 `link`（`pub use windows_link::link;`），呼叫端不必再直接依賴 `windows-link`。0c 的 `msctf.rs` 自己呼叫了 `windows_link::link!()`，必須改。
2. `windows_core::Error::from_win32()` 改名為 `windows_core::Error::from_thread()`（`windows-result` 0.3.4 → 0.4.1）。0c 的 `register.rs`、`langbar.rs` 直接用到，必須改。注意 `HRESULT::from_win32(u32)` 在新版仍存在，沒被改名。
3. `_Impl` trait 的 `windows_core::Ref<'_, T>` / `OutRef<'_, T>` 改成 `windows_core::Ref<T>` / `OutRef<T>`。生命週期改為省略寫法，兩者是同一個型別，0c 既有的 `Ref<'_, T>` 仍然編得過；只 affecting 風格，jc-tsf 已統一用 `Ref<T>`。

其餘本任務列出的 consumer API 簽章、`OVERLAPPED` 結構、`#[implement]` 展開路徑都沒有變化。

## 逐項對照

### 1. `ITfRange::SetText`

0.61.3 consumer（`src/Windows/Win32/UI/TextServices/mod.rs:11642`）：

```rust
impl ITfRange {
    pub unsafe fn SetText(&self, ec: u32, dwflags: u32, pchtext: &[u16]) -> windows_core::Result<()>
}
```

0.62.2 consumer：簽章完全相同。

0.61.3 的 `ITfRange_Impl` trait（`mod.rs:11801`）：

```rust
fn SetText(&self, ec: u32, dwflags: u32, pchtext: &windows_core::PCWSTR, cch: i32) -> windows_core::Result<()>;
```

0.62.2：相同。consumer 收 `&[u16]`、impl trait 收 `&PCWSTR, cch: i32` 是兩版都存在的既有不一致，不是差異。

0c 用法：`edit.rs` 的 `commit: Vec<u16>` / `preedit: Vec<u16>` 以 `range.SetText(ec, 0, &self.commit)` 呼叫，`no_text() -> &'static [u16]` 也直接傳入。測試假物件在 `edit.rs:650`、`focus_host_tests.rs:108` 以 `&PCWSTR, i32` 實作 trait。

需要改：無。

### 2. `ITfComposition` 方法

0.61.3 `ITfComposition_Impl`：

```rust
pub trait ITfComposition_Impl: windows_core::IUnknownImpl {
    fn GetRange(&self) -> windows_core::Result<ITfRange>;
    fn ShiftStart(&self, ecwrite: u32, pnewstart: windows_core::Ref<'_, ITfRange>) -> windows_core::Result<()>;
    fn ShiftEnd(&self, ecwrite: u32, pnewend: windows_core::Ref<'_, ITfRange>) -> windows_core::Result<()>;
    fn EndComposition(&self, ecwrite: u32) -> windows_core::Result<()>;
}
```

0.62.2：

```rust
pub trait ITfComposition_Impl: windows_core::IUnknownImpl {
    fn GetRange(&self) -> windows_core::Result<ITfRange>;
    fn ShiftStart(&self, ecwrite: u32, pnewstart: windows_core::Ref<ITfRange>) -> windows_core::Result<()>;
    fn ShiftEnd(&self, ecwrite: u32, pnewend: windows_core::Ref<ITfRange>) -> windows_core::Result<()>;
    fn EndComposition(&self, ecwrite: u32) -> windows_core::Result<()>;
}
```

consumer 方法 `GetRange` / `ShiftStart<P1: Param<ITfRange>>` / `ShiftEnd<P1>` / `EndComposition` 兩版完全相同。

0c 用法：`focus_host_tests.rs` 的 `#[implement(ITfComposition)]` 只實作 `GetRange`；`ShiftStart` / `ShiftEnd` 的假實作在 `focus_host_tests.rs:181,184`、`focus_host_tests.rs:313,316`、`service.rs:2162,2165`，型別寫成 `Ref<'_, ITfRange>`。

需要改：可選，把 `Ref<'_, ITfRange>` 改成 `Ref<ITfRange>` 以跟 jc-tsf 風格一致；不改也能編譯。

### 3. `ITfLangBarItemButton` / `ITfLangBarItemButton_Impl`

0.61.3 `ITfLangBarItemButton_Impl`（`mod.rs:10331`）：

```rust
pub trait ITfLangBarItemButton_Impl: ITfLangBarItem_Impl {
    fn OnClick(&self, click: TfLBIClick, pt: &POINT, prcarea: *const RECT) -> windows_core::Result<()>;
    fn InitMenu(&self, pmenu: windows_core::Ref<'_, ITfMenu>) -> windows_core::Result<()>;
    fn OnMenuSelect(&self, wid: u32) -> windows_core::Result<()>;
    fn GetIcon(&self) -> windows_core::Result<HICON>;
    fn GetText(&self) -> windows_core::Result<windows_core::BSTR>;
}
```

0.62.2：`InitMenu` 的參數改為 `windows_core::Ref<ITfMenu>`，其餘相同。父 trait `ITfLangBarItem_Impl`（`GetInfo` / `GetStatus` / `Show` / `GetTooltipString`）兩版完全相同。consumer 方法也相同。

0c 用法：`langbar.rs:720` 的 `fn InitMenu(&self, pmenu: Ref<'_, ITfMenu>)`。

需要改：可選，改成 `Ref<ITfMenu>`；不改也能編譯。

### 4. `OVERLAPPED` / `CancelIoEx`

`OVERLAPPED` 結構（`src/Windows/Win32/System/IO/mod.rs:82`）兩版完全相同：

```rust
#[repr(C)]
pub struct OVERLAPPED {
    pub Internal: usize,
    pub InternalHigh: usize,
    pub Anonymous: OVERLAPPED_0,
    pub hEvent: HANDLE,
}
```

`CancelIoEx` consumer 簽章兩版完全相同：

```rust
pub unsafe fn CancelIoEx(hfile: HANDLE, lpoverlapped: Option<*const OVERLAPPED>) -> windows_core::Result<()>
```

差異只在生成的函式本體：`windows_link::link!` → `windows_core::link!`，呼叫端不受影響。`GetOverlappedResult` 本體也把 `Error::from_win32` 換成 `from_thread`，簽章不變。

0c 用法：`ipc.rs:450` 的 `overlapped()` helper 建立 `OVERLAPPED { ... }`，`ipc.rs:465` 呼叫 `CancelIoEx(h, Some(&ov))`；`GetOverlappedResult` 在 `ipc.rs:19` 匯入。

需要改：無。

### 5. `CreateNamedPipeW` / `CreateFileW`

0.61.3 consumer（`Pipes/mod.rs:35`、`FileSystem/mod.rs:361`）：

```rust
pub unsafe fn CreateNamedPipeW<P0>(lpname: P0, dwopenmode: FILE_FLAGS_AND_ATTRIBUTES, dwpipemode: NAMED_PIPE_MODE, nmaxinstances: u32, noutbuffersize: u32, ninbuffersize: u32, ndefaulttimeout: u32, lpsecurityattributes: Option<*const SECURITY_ATTRIBUTES>) -> HANDLE
where P0: windows_core::Param<windows_core::PCWSTR>;

pub unsafe fn CreateFileW<P0>(lpfilename: P0, dwdesiredaccess: u32, dwsharemode: FILE_SHARE_MODE, lpsecurityattributes: Option<*const SECURITY_ATTRIBUTES>, dwcreationdisposition: FILE_CREATION_DISPOSITION, dwflagsandattributes: FILE_FLAGS_AND_ATTRIBUTES, htemplatefile: Option<HANDLE>) -> windows_core::Result<HANDLE>
where P0: windows_core::Param<windows_core::PCWSTR>;
```

0.62.2：兩個簽章完全相同。差異只在函式本體：`windows_link::link!` → `windows_core::link!`，以及 `CreateFileW` 的 `ok_or_else(windows_core::Error::from_win32)` → `ok_or_else(windows_core::Error::from_thread)`。兩者都是 crate 內部生成的程式碼，呼叫端不用改。

0c 用法：`session.rs:1763,1949,2150,2268` 呼叫 `CreateNamedPipeW`；`ipc.rs:309` 呼叫 `CreateFileW`。

需要改：無。

### 6. `PeekNamedPipe` / `WaitNamedPipeW`

0.61.3 consumer（`Pipes/mod.rs`）：

```rust
pub unsafe fn PeekNamedPipe(hnamedpipe: HANDLE, lpbuffer: Option<*mut c_void>, nbuffersize: u32, lpbytesread: Option<*mut u32>, lptotalbytesavail: Option<*mut u32>, lpbytesleftthismessage: Option<*mut u32>) -> windows_core::Result<()>

pub unsafe fn WaitNamedPipeW(lpnamedpipename: windows_core::PCWSTR, ntimeout: u32) -> windows_core::Result<()>
```

0.62.2：簽章完全相同。差異只在函式本體的 `windows_link::link!` → `windows_core::link!`。

0c 用法：`ipc.rs:236,259` 呼叫 `PeekNamedPipe`（`None` 當 optional 參數）；`ipc.rs:368` 呼叫 `WaitNamedPipeW(PCWSTR::from_raw(...), IO_TIMEOUT_MS)`。

需要改：無。

### 7. `#[implement]` 展開的 windows_core 路徑

兩版都解析到 `windows-implement 0.60.2`，macro 本身相同。展開出來的程式碼一律使用絕對路徑 `::windows_core::...`，例如：

```rust
::windows_core::Interface::Vtable
::windows_core::IUnknownImpl
::windows_core::ComObject
::windows_core::imp::Box
::windows_core::imp::WeakRefCount
```

因此 crate 根必須有名叫 `windows_core` 的直接依賴。0c 的 `Cargo.toml` 是 `windows-core = "=0.61.2"`，jc-tsf 是 `windows-core = "0.62"`（其 Cargo.toml 已有註解說明這點）。`IUnknownImpl` trait（`type Impl` / `get_impl` / `get_impl_mut` / `into_inner`）兩版相同，macro 展開需求不變。

需要改：無。補充：0c 的 `msctf.rs:9` 自己呼叫 `windows_link::link!()`，這是唯一需要手動從 `windows_link::link!` 改成 `windows_core::link!` 的地方；改完後可直接移除 `windows-link` direct dependency。windows-link 0.1.3 與 0.2.1 的 `link!` macro 本體相同（x86 用 undecorated raw-dylib，其他架構用 `extern "C"` raw-dylib），語法不用調整。

### 8. `ITfRange::GetRange` / `Collapse` / `Clone`

標題裡的 `GetRange` 實際上是 `ITfComposition` 的方法；`Collapse` 與 `Clone` 屬於 `ITfRange`。consumer 方法兩版完全相同：

```rust
impl ITfComposition {
    pub unsafe fn GetRange(&self) -> windows_core::Result<ITfRange>;
}
impl ITfRange {
    pub unsafe fn Collapse(&self, ec: u32, apos: TfAnchor) -> windows_core::Result<()>;
    pub unsafe fn Clone(&self) -> windows_core::Result<ITfRange>;
}
```

`ITfComposition_Impl::GetRange`、`ITfRange_Impl::Collapse`、`ITfRange_Impl::Clone` 的簽章兩版都相同，且都沒有 `Ref` 參數，不受生命週期省略變化影響。

0c 用法：`edit.rs:133` 的 `comp.GetRange()?.Clone()?`、`edit.rs:134,137` 的 `caret.Collapse(ec, TF_ANCHOR_START/END)`、`edit.rs:411,436,451,461,486` 的 `comp.GetRange()` 與 `range.Clone()`。

需要改：無。

## 0c 原始碼需要動的點

以下是完整清單，涵蓋上面八項以外的全域差異：

1. `msctf.rs:9`：`windows_link::link!` → `windows_core::link!`。
2. `register.rs:59`：`Error::from_win32()` → `Error::from_thread()`。
3. `langbar.rs:747`：`windows_core::Error::from_win32` → `windows_core::Error::from_thread`。
4. `Ref<'_, T>` / `OutRef<'_, T>` → `Ref<T>` / `OutRef<T>`，共 68 處：
   - `edit.rs` 19
   - `service.rs` 18
   - `focus_host_tests.rs` 15
   - `privacy_host_tests.rs` 12
   - `langbar.rs` 2
   - `uielement.rs` 1
   - `factory.rs` 1
   這項是型別等價的風格調整，不改也能編譯；建議跟 jc-tsf 現有寫法一致。

 consumer API 簽章、結構定義與 macro 展開路徑不需要改。
