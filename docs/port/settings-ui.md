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
| `platforms/settings/ui/input-styles.js`（輸入習慣） | `state.rs`（`copy_profile`）、`ui.rs`（輸入分頁） | 部分 ported：顯示鎖定與複製成自訂，沒有改名／刪除 |
| `platforms/settings/src/main.rs`（視窗、單一實例、指令白名單） | `crates/jc-settings-ui/src/main.rs` | rewritten；GPUI 進入點，無 IPC |
| `platforms/settings/src/updater.rs`、`watch.rs`（更新與詞彙同步） | n/a | 本版沒有更新器與「我的詞彙」 |
| `internal/settings/settings.go`（讀寫、驗證） | `crates/jc-settings/src/settings.rs` | 已在 settings-core 移植；本 task 只使用 |
| `cmd/0c-update` 的 settings 契約 | n/a | 直接呼叫 crate API，不需要契約 |

## 分頁與設定鍵

每個可編輯項目都有一行說明與「預設值」標記；預設值取自
`Settings::default_for_platform(goos)`，沒有另外複製一份。

| 分頁 | 項目 | 設定鍵 | 形式 |
|---|---|---|---|
| 注音 | ㄗ／ㄓ、ㄘ／ㄔ、ㄙ／ㄕ、ㄣ／ㄥ 不分 | `fuzzy-*` | 下拉：不啟用／只在選字時／自動（預設 select） |
| 注音 | 選字模型 | `selection` | 下拉：神經模型／統計（預設神經） |
| 注音 | 學習 | `document-context` | 開關（預設開） |
| 注音 | 選字模型狀態 | n/a | 唯讀；缺 `models/zh-select` 權重時顯示「需下載」 |
| 日文 | 神經模型（Jinen） | `ja-model` | 開關（預設開） |
| 日文 | 逐字変換 | `ja-live-conversion` | 開關（預設關） |
| 日文 | 寫入日文個人詞庫 | `ja-learning` | 開關（預設開） |
| 日文 | 輸入方式 | `ja-input-method` | 下拉：羅馬字／直接かな |
| 日文 | 日英同形 | `ja-ambiguous` | 下拉：並列／英文優先／かな優先 |
| 日文 | 神經轉換模式 | n/a | 唯讀；說明 `--neural` 的 auto／quality／off 與目前引擎只收 on／off |
| 日文 | 候選重排 | n/a | 唯讀，標「實驗」；`SessionConfig::ja_rerank` 仍固定關閉 |
| 輸入 | 中英切換鍵 | `toggle-key` | 下拉：右 Shift／左右 Shift／左 Shift／不使用 |
| 輸入 | 啟動模式 | `start-mode` | 下拉：注音／英文 |
| 輸入 | Esc 清除 | `esc-clear` | 開關 |
| 輸入 | 空白選字 | `space-selects` | 開關 |
| 輸入 | 中英混打 | `mixed-input` | 開關 |
| 輸入 | 複製成自訂輸入習慣 | `input_profile` | 按鈕；內建風格時才出現 |
| 資料 | 資料根目錄、設定檔、manifest、模型 | n/a | 唯讀再加「重新檢查」按鈕 |

日文的 `ja-model`、`ja-input-method`、`ja-ambiguous` 在 `settings-wire` 的對照
表是「本版忽略」：引擎目前用 `selection` 決定神經轉換、只實作羅馬字、固定日英
並列。畫面照存這些值，說明文字直接寫明尚未生效，避免把保留值當成已生效。

輸入分頁的 14 個鍵位規則由 `input_profile` 管理。選用內建風格時
`Settings::profile_locked` 會拒絕修改，畫面把該項反灰並說明原因；按「複製成
自訂輸入習慣」後就能改。這與 Go 版的 `items[].locked` 行為一致。

## 存檔行為

- 每次變更立刻 `save`：先寫暫存檔再改名（`jc_settings::atomic_file`），不會留下半個檔案。
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

## 驗證

```sh
RUST_FONTCONFIG_DLOPEN=1 cargo check -p jc-settings-ui --all-targets        # 通過
RUST_FONTCONFIG_DLOPEN=1 cargo check --workspace --all-targets              # 通過
RUST_FONTCONFIG_DLOPEN=1 cargo clippy -p jc-settings-ui --all-targets -- -D warnings  # 通過
RUST_FONTCONFIG_DLOPEN=1 cargo fmt -p jc-settings-ui
```

`cargo run -p jc-settings-ui -- --data <資料根目錄>` 在有顯示器的機器上會開
1080×720 的視窗；本機 headless 跑 12 秒沒有 panic。

## 未完成

1. **`--neural` 三態**：`jc-engine` 目前只解析 `on`／`off`，三態在 `gating-lowcall`
   任務。畫面先列出 auto／quality／off 的意義並標成唯讀，沒有寫入假鍵。
2. **rerank 開關**：`SessionConfig::ja_rerank` 沒有 CLI 或設定檔入口，維持唯讀。
3. **重新啟動引擎**：按鈕停用；需要安裝器提供服務控制（systemd／launchd／Windows 服務）。
4. **輸入習慣管理**：只做「複製成自訂」，沒有改名、刪除與多風格預覽。
5. **Windows／macOS 建置**：未交叉 check；GPUI 的 Windows linker 與 WebView 無關，
   CI 需要 Windows target。
6. **顯示器實測**：本機沒有 DISPLAY，沒有做到真正開窗截圖與滑鼠操作。
7. **其他 Go 頁面**：輸出設定、更新、問題排解、關於、我的詞彙、模型下載都没搬。
