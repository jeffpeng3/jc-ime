# T12a：0c UI 候選窗 → crates/jc-ui

Source：`/mnt/work/jc/ref/0c-ime`（main `8f443af`）。windows-rs 從 0.61.3 換成 0.62.2。

| 0c 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| platforms/windows/common/src/lib.rs（UiMessage 與 helper） | crates/jc-ui/src/message.rs | ported：欄位、serde 屬性、幾何 helper、WM_SELECT_CANDIDATE 逐字保留；`ui_pipe()` 前綴改 `jc-ime-ui` |
| platforms/windows/ui/src/lib.rs | crates/jc-ui/src/lib.rs | ported：加 `#[cfg(windows)]` 擋掉繪製模組，Linux 只編 message／retire |
| platforms/windows/ui/src/candidate.rs | crates/jc-ui/src/candidate.rs | ported：helper 改從 `crate::message` 引；視窗標題 `jc`、reflow 訊息改 `WM_JC_REFLOW` |
| platforms/windows/ui/src/color_text.rs | crates/jc-ui/src/color_text.rs | ported：Direct2D／DirectWrite 彩色 Emoji；0.62 的 `CreateDCRenderTarget`／`DrawText` 簽章相同 |
| platforms/windows/ui/src/palette.rs | crates/jc-ui/src/palette.rs | ported |
| platforms/windows/ui/src/placement.rs | crates/jc-ui/src/placement.rs | ported |
| platforms/windows/ui/src/retire.rs | crates/jc-ui/src/retire.rs | ported：純 std，非 Windows 也編 |
| platforms/windows/ui/src/main.rs | crates/jc-ui/src/main.rs | rewritten：移到 `#[cfg(windows)] mod imp`；拿掉 legacy pipe 與 menu／watch_updater 執行緒；mutex 名 `Local\jc-ui`；`create()` 失敗改成靜默退出而非 `expect` |
| platforms/windows/ui/src/menu.rs | 無 | n/a：0c-menu 服務 0c-update 命令與狀態查詢；jc 的語言列選單由 jc-settings-ui 負責（T6／T9） |
| platforms/windows/ui/Cargo.toml | crates/jc-ui/Cargo.toml | rewritten：`windows = "0.62"`，補 `Win32_System_IO`（0.62 把 `ReadFile`／`ConnectNamedPipe` 移到這個 feature 後） |

未移植：0c 的 `#[cfg(test)]` 測試模組。任務限制不跑 test，且 host 測試由 T8 統一處理。
