//! 候選窗的共用訊息格式與繪製程式：`jc-ui.exe` 與 TSF DLL 都引用這個 crate。
//!
//! 移植自 0c 的 `platforms/windows/common/src/lib.rs`（訊息與 helper）、
//! `platforms/windows/ui/src/`（候選窗、主題、彩色字型、定位、延後結束）。
//! Pipe 前綴從 `0c-ui` 換成 `jc-ime-ui`，避免與平行建置的 0c-ui 互連；
//! 其餘結構、欄位與 WM_SELECT_CANDIDATE 的值照 0c 逐字保留，DLL 側（T12b）
//! 可以直接用同一份 [`message`] 組訊框。
//!
//! 非 Windows 平台只有 [`message`] 與 [`retire`] 這兩個不碰 Win32 的模組；
//! 繪製與 pipe 相關模組由 `#[cfg(windows)]` 擋掉。

/// TSF DLL → jc-ui 的呈現訊息、候選、幾何與選字常數。DLL 側要引用這裡。
pub mod message;

/// 候選窗繪製與視窗行為；`jc-ui.exe` 與沙箱宿主內的 DLL 共用。
#[cfg(windows)]
pub mod candidate;

/// DirectWrite／Direct2D 的彩色字型繪製，GDI 不可用時退回。
#[cfg(windows)]
mod color_text;

/// 跟隨系統主題與高對比模式的色盤。
#[cfg(windows)]
mod palette;

/// 候選窗在螢幕工作區內的定位。
#[cfg(windows)]
mod placement;

/// 升級交接用的延後結束判斷（純 std）。
pub mod retire;
