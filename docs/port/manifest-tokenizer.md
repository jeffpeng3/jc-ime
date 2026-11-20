# manifest-tokenizer：把 Jinen tokenizer 納入資料 manifest

Worktree：`/mnt/work/jc/wt/manifest-tokenizer`（branch `manifest-tokenizer`）

## 目標

- `DataRole` 新增 `JaTokenizer`（`ja_tokenizer`），manifest 收錄
  `models/quant/jinen-v2-xsmall/tokenizer.json`，`shipped: true`、授權沿用
  Jinen 的 `L-21`。
- `jc-engine` 載入 Jinen 時從 manifest 解析 GGUF 與 tokenizer 路徑；manifest
  沒有對應項目才退回寫死的候選路徑。
- tokenizer 缺席時只記一次警告，日文維持統計轉換，不擋引擎啟動；
  `jc-cli data verify` 仍把缺席的隨附檔算成錯誤，出貨前會擋下。
- 安裝器 WXS 引用新檔，`installer/check-payload.py` 的「manifest 隨附檔必須被
  WXS 引用」檢查才會通過。

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `cmd/0c-update/models.go`（模型清單與 SHA-256） | `crates/jc-data/src/manifest.rs`（`DataRole::JaTokenizer`、`CATALOG`） | rewritten：改成資料根目錄 manifest 的角色記錄 |
| `cmd/0c-server/main.go` 的模型啟動載入 | `crates/jc-engine/src/loader.rs`（`resolve_ja_neural`、`manifest_file`） | rewritten：路徑優先從 manifest 解析，缺席降級 |
| `cmd/0c-update` 的檔案驗證 | `crates/jc-cli/src/manifest_cmd.rs` | n/a：`data verify` 既有逐檔檢查已涵蓋新角色，不需改程式 |
| 前身無對應 | `installer/jc-ime.wxs`（`JaTokenizer` component） | 新行為：安裝包要帶 tokenizer |

## 行為測試

| 測試 | 驗證 |
|---|---|
| `role_names_are_snake_case` | `ja_tokenizer` 字串與 serde 名稱 |
| `generated_manifest_records_real_size_and_hash` | manifest 收錄 tokenizer，路徑、授權、`shipped` 正確 |
| `manifest_resolves_jinen_tokenizer_path` | tokenizer 路徑取自 manifest，不走寫死候選 |
| `missing_manifest_tokenizer_degrades_without_failing_startup` | manifest 列了但檔案缺少時啟動檢查放行，解析回報缺 tokenizer |
| `real_data_root_verifies` | 真實資料根目錄全量驗證通過 |

## 實測

```text
檔案：10（隨附 6，下載 4），共 445686235 位元組
驗證 /tmp/opencode/manifest.json：通過 10/10，錯誤 0，警告 0
```

移除 tokenizer 後啟動引擎：

```text
WARN  jc_engine::loader] Jinen tokenizer 缺席：models/quant/jinen-v2-xsmall/tokenizer.json，日文神經轉換停用，維持統計轉換
INFO  jc_engine::loader] 資料 manifest 檢查通過（通過 9/10，降級 1，資料版本 2026-11-20）
```

引擎持續服務，沒有結束。
