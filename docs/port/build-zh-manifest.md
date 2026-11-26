# build-zh-manifest：catalog 補 build-zh/manifest.json

Worktree：`/mnt/work/jc/wt/build-zh-manifest`（branch `build-zh-manifest`）

## 根因

真機 rc.9 手動啟動引擎（`--supervise`）後，`supervisor.log` 顯示子行程一直崩潰：

```text
jc-engine: 載入資料失敗：C:\ProgramData\jc-ime\data: 讀取 ...\build-zh 失敗：
讀取 ...\build-zh\manifest.json 失敗：系統找不到指定的檔案 (os error 2)
連續失敗 5 次，停止重試 300s
```

`build-zh` 目錄存在，`lexicon.bin`、`lm.bin` 都有，但 catalog 漏了
`build-zh/manifest.json`。`jc_data::zh::Bundle::open` 會讀
`<dir>/manifest.json` 驗證 `lexicon.bin` 與 `lm.bin` 的大小與 SHA-256，缺了就
直接 fatal。這跟上一次 `english/manifest.json` 是同一類問題：catalog 只列主要
資料檔，漏了子目錄的 manifest。上一輪只補 english，漏了 build-zh。

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| 無（Go 的資料根目錄索引由出貨流程管理） | `crates/jc-data/src/manifest.rs`（`CATALOG`、`DataFileSpec`） | n/a：修既有 catalog 的收錄清單，不是新的移植 |
| 無 | `installer/jc-ime.wxs`（`DataFiles`） | n/a：MSI 資料清單同步補 `<File>` |
| 無 | `crates/jc-data/src/manifest.rs`（`build_zh_manifest_is_cataloged`） | rewritten：仿 `english_pack_metadata_is_cataloged` 的行為測試，斷言具體路徑、角色、授權與 `shipped` |

## 修正

1. `CATALOG` 在 `build-zh/lm.bin` 後加一筆 `build-zh/manifest.json`，角色沿用
   `DataRole::ZhLexicon`、授權 `ZH_LICENSES`、`shipped: true`、`url: None`。
   不新增角色：這份 manifest 是 `lexicon.bin` 的驗證中繼檔，共用角色與授權最
   簡單，也不會改既有 manifest schema。
2. WXS 的 `ZhLexicon` component 加 `FileZhManifest`，指向
   `data/build-zh/manifest.json`。`build.ps1` 本來就照 `shipped: true` 複製
   catalog 列出的檔案，所以不必改 staging 清單。
3. 測試：隨附檔筆數 8 → 9、真實資料根目錄的 `shipped` 筆數 8 → 9，並新增
   `build_zh_manifest_is_cataloged`。

manifest 從 10 筆變成 11 筆（9 個 `shipped: true`、2 個 `zh_select` 下載項）。

## 引擎所需資料檔盤點

| 引擎載入的檔案 | catalog | 說明 |
|---|---|---|
| `build-zh/manifest.json` | 本次補 | `Bundle::open` 必要，缺席即 fatal |
| `build-zh/lexicon.bin`、`build-zh/lm.bin` | 已有 | `Bundle::open` 一併驗證 |
| `english/words.bin`、`english/manifest.json`、`english/NOTICE.txt` | 已有 | 英文載入器要求三者同時存在 |
| `packs/ja-lexicon-mozc-v2.pack` | 已有 | pack 內容自帶 manifest，不逐檔列入 catalog |
| `models/external/jinen-v2-small/jinen-v2-small-Q5_K_M.gguf` | 已有 | Jinen 的預設 GGUF 路徑 |
| `models/quant/jinen-v2-xsmall/tokenizer.json` | 已有 | Jinen tokenizer；`JA_TOKENIZER_CANDIDATES` 還有舊候選，出貨用這一份 |
| 根 `manifest.json` | 不列（正確） | `build.ps1` 用 `jc-cli data manifest` 產生；`CATALOG` 是它的輸入 |

未列且不影響啟動：

- `build-zh-bigram/`：選用。`load_engine_data` 找不到 `lm.bin` 就退回
  `build-zh/lm.bin`；catalog 目前沒有收錄，維持現狀。
- `build-zh/NOTICE.txt`、`build-zh/report.txt`：授權與建置報告，`Bundle::open`
  不讀。
- `ja-src/`、`opencc/`：開發來源與 CLI eval 用的繁簡字典；`load_engine_data`
  不開 `opencc/`。
- `packs/ja-lexicon-public-core.pack`：日文 pack 備援，目前出貨只用 mozc-v2。

結論：引擎啟動路徑需要的資料檔已全數收錄，本次之後沒有其它漏網。

## 驗證

| 指令 | 結果 |
|---|---|
| `cargo check --workspace --all-targets` | 通過 |
| `JC_TESTDATA=/mnt/work/jc/jc-ime-data cargo test -p jc-data` | 61 passed、0 failed；`real_data_root_verifies` 對真實資料根目錄通過 |
| `cargo fmt --all -- --check` | 通過 |
| `uv run --no-project python` 呼叫 `check-payload.py` 的 `wxs_files()` | 回傳 `('FileZhManifest', 'data/build-zh/manifest.json', ...)` |

`check-payload.py` 的完整檢查需要建置好的 payload 目錄，本機只依任務書要求用
`wxs_files()` 驗證 WXS 引用。
