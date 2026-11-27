# 目前狀態

最後更新：2026-11-28（對應版本 `0.1.31`）

這一頁只放現況與可查證的事實；版本細節見 [`CHANGELOG.md`](CHANGELOG.md)，模型與
資料見 [`models.md`](models.md) 與 [`port/data.md`](port/data.md)。

## 版本與規模

| 項目 | 現況 |
|---|---|
| 版本 | `0.1.31`（`Cargo.toml` 的 `[workspace.package] version`） |
| Rust | 1.100.0（`rust-toolchain.toml`） |
| workspace | 20 個成員，含 `jc-core`、`jc-engine`、`jc-tsf`、`jc-settings-ui`、`jc-cli` |
| 協定 | `PROTOCOL_VERSION = 3`（v3 凍結） |
| 安裝 | Inno Setup per-machine `setup.exe`，x64 與 x86 兩份 TSF DLL |

## 已接上的路徑

| 路徑 | 現況 |
|---|---|
| 注音 | 大千鍵盤、模糊音、候選分頁與四欄、個人化、簡繁轉換；golden 45/45 |
| 日文 | Mozc pack v2、統計 Viterbi、Jinen v2 small 門檻混合；預設 `ja_conversion = live` |
| Mixed | 注音／英文／日文共用一條切分管線；候選窗語言無關 |
| 前導字元 | `u` 查 Unicode、`b` 查 Big5 碼位 |
| 傳輸 | stdio、Unix domain socket、Windows named pipe；每連線一個 worker |
| TSF | 組字、候選窗、滑鼠點選、虛線底線、Direct2D 自繪；x86／x64 |
| 引擎 | `--supervise` 單一實例與自動重啟；TIP 連不上時按需啟動 |
| 設定 | `settings.json`、引擎熱重載、GPUI 設定程式 |
| 資料 | manifest 產生與驗證；模型與詞典隨安裝包散布 |

## 未完成

- 實機驗證：Windows 安裝、升級、反安裝、UWP／Office 相容性與日常使用。
- 簽章：目前產出未簽章 `setup.exe`；SmartScreen 與企業白名單會擋。
- 右文脈：Jinen prompt 只送左文脈，`max_context_chars.right` 目前沒有效果。
- 32 位元程式能否載入 x86 TSF，仍待實機驗證。
- 更新器：本版不做自動更新。
