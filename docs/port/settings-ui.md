# settings-ui：設定程式（GPUI Kit）

Worktree：`/mnt/work/jc/wt/settings-ui`（branch `settings-ui`）

## 目標

讓使用者在一個原生視窗裡改 `settings.json`，並看到目前生效值、資料根目錄與
模型狀態。讀寫全部經過 `jc-settings`，不自己拼 JSON。

## 建置探針

在 `/tmp/opencode/gpui-probe` 用最小視窗驗證：

| 項目 | 結果 |
|---|---|
| Rust | `rustc 1.99.1` stable（aarch64-unknown-linux-gnu），**不需要 nightly** |
| gpui-kit | `0.10.0`（crates.io 最新；依賴 `gpui-pre 0.3.12`、`gpui-base 0.10.0`、`gpui-component 0.10.0`） |
| `cargo check` | 通過 |
| 本機已安裝的執行期函式庫 | `libfontconfig.so.1`、`libxkbcommon.so.0`、`libxkbcommon-x11.so.0`、`libxcb.so.1`、`libvulkan.so.1` 都有 |
| 缺少的開發套件 | `fontconfig.pc`、`libxkbcommon.so`、`libxkbcommon-x11.so`、`libxcb.so` 的開發 symlink／pkg-config 檔 |
| 本機建置方式 | `RUST_FONTCONFIG_DLOPEN=1` 讓 fontconfig 改成執行期 dlopen；再用暫時的 `-L` 目錄指向三個執行期 `.so` 完成連結。**沒有改 workspace 設定** |
| 乾淨建置需要的套件 | `libfontconfig-dev libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev`（Ubuntu 名稱） |
| Linux 視窗實測 | 本機沒有 `DISPLAY`／`WAYLAND_DISPLAY`，GPUI 自動進 headless；程式跑滿 12 秒未崩潰，只出現找不到系統 UI 字型的警告 |
| Windows 交叉 check | 未驗；本機沒有 Windows target 與 linker，留給 CI |

`gpui-kit` 的 `edition` 跟著 workspace 的 2021。授權 Apache-2.0，已加入
`legal/README.md` 的 L-26。

## Go 對照

Go 版設定程式是 Tauri（`ref/0c-ime/platforms/settings`）：Rust 外殼加 HTML／JS，
設定邏輯與 JSON 契約在 `0c-update`。本版改成純 Rust 的 GPUI 視窗，直接呼叫
`jc-settings`，不經過 `0c-update`。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `platforms/settings/ui/pages.js`（各頁項目與順序） | `crates/jc-settings-ui/src/options.rs` | rewritten；只留本 task 的四頁 |
| `platforms/settings/ui/app.js`（依契約產生欄位、送 `settings set`） | `crates/jc-settings-ui/src/ui.rs`、`state.rs` | rewritten；改成 `SettingField` 直接呼叫 `Settings::set` |
| `platforms/settings/ui/models.js`（模型頁） | `crates/jc-settings-ui/src/data.rs`、`ui.rs`（資料分頁） | rewritten；只顯示 manifest 與檔案存在，不提供下載 |
| `platforms/settings/ui/input-styles.js`（輸入習慣） | `state.rs`（`make_editable`）、`ui.rs`（輸入分頁） | 部分 ported：第一次修改時自動轉成自訂快照，沒有改名／刪除 |
| `platforms/settings/src/main.rs`（視窗、單一實例、指令白名單） | `crates/jc-settings-ui/src/main.rs` | rewritten；GPUI 進入點，無 IPC |
| `platforms/settings/src/updater.rs`、`watch.rs`（更新與詞彙同步） | n/a | 本版沒有更新器與「我的詞彙」 |
| `internal/settings/settings.go`（讀寫、驗證） | `crates/jc-settings/src/settings.rs` | 已在 settings-core 移植；本 task 只使用 |
| `cmd/0c-update` 的 settings 契約 | n/a | 直接呼叫 crate API，不需要契約 |

## 分頁與設定鍵

每個可編輯項目的說明與「預設值」分成兩行：`options::description()` 用 `\n`
連接，`SettingItem::description` 收的是 GPUI `Text`，其排版會把 `\n` 當成強制
換行，不需要拆成兩個元件。預設值取自 `Settings::default_for_platform(goos)`，
沒有另外複製一份。

| 分頁 | 項目 | 設定鍵 | 形式 |
|---|---|---|---|
| 注音 | ㄗ／ㄓ、ㄘ／ㄔ、ㄙ／ㄕ、ㄣ／ㄥ 不分 | `fuzzy-*` | 下拉：不啟用／只在選字時／自動（預設 select） |
| 注音 | 選字模型 | `selection` | 下拉：神經模型／統計（預設神經） |
| 注音 | 學習 | `document-context` | 開關（預設開） |
| 注音 | 選字模型狀態 | n/a | 唯讀；有權重顯示「已安裝」，缺 `models/zh-select` 時顯示「需下載」 |
| 日文 | 神經轉換模式 | `ja-neural-mode` | 下拉：自動／品質優先／關閉（預設自動） |
| 日文 | 逐字変換 | `ja-live-conversion` | 開關；說明寫「神經補正由《神經轉換模式》控制」 |
| 日文 | 寫入日文個人詞庫 | `ja-learning` | 開關（預設開） |
| 輸入 | 空白選字 | `space-selects` | 開關 |

已從 UI 移除的項目：`toggle-key`、`start-mode`、`esc-clear`、`ja-model`、
`ja-rerank`、`ja-input-method`、`ja-ambiguous`、`mode-memory`。其中前七項的 JSON
欄位保留給舊檔相容，只有 `mixed_input` 從 JSON 契約整個移除。

中英切換固定為單按左／右 Shift：TSF 送 `{"op":"toggle"}`，引擎在目前語言與
純英文 En 之間切換，再按一次回到原本的 Mixed 或 Ja。中文使用者一律用混打模式，
舊設定檔的 `mixed_input` 直接忽略，不寫回。
| 資料 | 資料根目錄、設定檔、manifest、模型 | n/a | 唯讀再加「重新檢查」按鈕 |

### 模型頁的狀態判斷

`data.rs` 從 `jc-data` 的 `CATALOG` 依角色解析實際路徑，不再自己維護一份相對
路徑表；catalog 改目錄或檔名時模型頁跟著改。`ui.rs` 的資料分頁「模型」群組只列
仍在使用或隨 MSI 散布的角色。有檔案顯示「已安裝」（正常色），缺檔才顯示
「需下載」（警告色）。目前列出四項：

| 顯示名稱 | catalog 角色 | 缺檔時 |
|---|---|---|
| 注音選字模型 | `ZhSelect` 的 `.gguf`／`.safetensors`／`.onnx` 候選 | 需下載 |
| 日文神經模型（Jinen） | `JaNeural` 的 `.gguf` 候選 | 需下載 |
| Jinen tokenizer | 環境變數 → 資料 manifest 的 `JaTokenizer` → GGUF 同目錄 → 隨附候選 | 需下載 |
| 日文詞典 pack | `JaLexicon` 的 `.pack` 候選 | 需下載 |

Jinen tokenizer 的探測順序與 `jc-engine::resolve_ja_neural` 一致，也補上環境
變數、資料 manifest 與 GGUF 同目錄三種來源。這三項模型檔隨 MSI 散布，正常安裝
應全部顯示「已安裝」。

已移除的角色不再列出，也不再探測：`ja-rerank` 已棄用且 `SessionConfig::ja_rerank`
預設 false；`ja-punct` 與舊的 `models/ja-tokenizer/public_core_ja.model`
自 2026-11-20 起不載入、不散布，2026-11-21 起也從集合 manifest 移除。
把它們顯示成「缺少／未安裝」是誤導，因此 UI 完全不顯示。相對應的死設定 UI
（`ja-model`、`ja-rerank`、`ja-input-method`、`ja-ambiguous`）也已移除，JSON
欄位保留給舊檔相容。

日文的 `ja-model`、`ja-input-method`、`ja-ambiguous` 在 `settings-wire` 的對照
表是「本版忽略」：引擎用 `ja-neural-mode` 決定神經轉換、只實作羅馬字、固定日英
並列。畫面不再顯示這些值；`ja-neural-mode` 已接進 `SessionConfig`，可直接修改。

輸入分頁的 14 個鍵位規則由 `input_profile` 管理。第一次啟動時
`SettingsView::load` 會把預設值存成 `settings.json`，同時把內建風格轉成自訂快照；
所有項目第一次開啟就能改，不再需要按「複製成自訂輸入習慣」。既有檔案若還用著
內建風格，第一次修改時自動轉成自訂快照。這與 Go 版 `items[].locked` 的行為不同，
是刻意配合使用者回報調整。較新 schema 與壞檔仍保留原有的唯讀保護。

## 存檔行為

- 第一次啟動就建立 `settings.json`；後續每次變更立刻 `save`：先寫暫存檔再改名
  （`jc_settings::atomic_file`），不會留下半個檔案。
- 壞檔或讀不到時不寫入，畫面顯示原因；schema 比本版新時只讀。
- 變更被拒（值不合法、風格鎖定、風格組合衝突）時保留原值並顯示錯誤。
- 變更後不需要重開程式；但引擎只在啟動時讀設定，所以畫面固定顯示「引擎下次啟動才生效」。
- 沒有跨行程鎖。`jc-settings::change` 文件要求呼叫端在
  `<資料根目錄>/update/settings-write.lock` 上互斥；本程式與 `0c-update`
  同時寫入時是 last-writer-wins，引擎本身只讀，不受影響。

## 與引擎的一致性

用 `jc-settings` 寫出一份改過的設定，再交給 `jc-engine --print-settings`：

```sh
./target/debug/jc-engine --print-settings --allow-empty-data \
  --settings /tmp/opencode/ui-settings/settings.json
```

`settings_state` 為 `ok`，`mode` 變 `en`（`start-mode=english`）、
`selection` 為 `statistical`、`neural` 為 `false`、`ja_live` 為 `true`，
`fuzzy_pairs` 仍是四組。欄位名稱與 `crates/jc-engine/src/config.rs` 讀的一致。

## 跟隨系統主題

`main.rs` 在 `open_window` 的建構回呼裡呼叫 `follow_system_theme()`，啟動時先
`Theme::sync_system_appearance(Some(window), cx)` 讀目前系統外觀，再用
`window.observe_window_appearance(...)` 註冊變化回呼。回呼內重複呼叫同一個 API，
所以 Windows 切換深色／淺色時主題跟著換；訂閱用 `detach()` 保留到視窗關閉。
沒有新增「主題」設定項。

GPUI 0.10 的 API 對應（`gpui-kit 0.10` 底層是 `gpui-pre 0.3.12`、
`gpui-base 0.10.0`、`gpui-component 0.10.0`）：

| API | 位置與用途 |
|---|---|
| `ThemeAppearance` | `gpui_base` 的 Light／Dark；由 `Theme` 的 `ThemeMode` 投影而來 |
| `Theme`／`ActiveTheme` | `gpui_component::theme`；`ActiveTheme::theme()` 從 `App` 取得全域主題 |
| `ThemeMode` | `gpui_component::theme` 的 Light／Dark，可由 `WindowAppearance` 轉換 |
| `Theme::sync_system_appearance` | 讀 `Window::appearance()`（無視窗時讀 `App::window_appearance()`），再呼叫 `Theme::change()` |
| `Window::observe_window_appearance` | 平台外觀變化回呼；Windows 由 `WM_SETTINGCHANGE` 的 `ImmersiveColorSet` 觸發 |
| `WindowOptions` | 沒有 appearance 欄位，只有 `window_background`（透明度語意）；系統外觀要從 `Window::appearance()` 讀 |

`gpui_kit::init(cx)` 一開始會把主題設成 Light，接著上面的同步會立刻改成系統
目前的值。

## 驗證

```sh
cargo test -p jc-settings-ui                            # 24 passed
cargo check --workspace --all-targets                    # 通過
cargo clippy --workspace --all-targets -- -D warnings    # 無警告
cargo fmt --all -- --check                               # 通過
```

`cargo run -p jc-settings-ui -- --data <資料根目錄>` 在有顯示器的機器上會開
1080×720 的視窗；本機 headless 跑 12 秒沒有 panic。

Windows 交叉 check 卡在 `gpui-pre 0.3.12` 的 build script：
`NotAttempted("llvm-rc")`（`build.rs:46`）。本機沒有 llvm-rc／Windows SDK，
屬於環境限制，CI 的 windows job 會驗證。

## 未完成

1. **重新啟動引擎**：按鈕停用；需要安裝器提供服務控制（systemd／launchd／Windows 服務）。
2. **輸入習慣管理**：第一次編輯時自動轉成自訂快照，沒有改名、刪除與多風格預覽。
3. **Windows／macOS 建置**：未交叉 check；GPUI 的 Windows linker 與 WebView 無關，
   CI 需要 Windows target。
4. **顯示器實測**：本機沒有 DISPLAY，沒有做到真正開窗截圖與滑鼠操作。
5. **其他 Go 頁面**：輸出設定、更新、問題排解、關於、我的詞彙、模型下載都没搬。
