# 資料包：佈局、manifest 與驗證

資料根目錄（`--data` 或 `JC_DATA`）是一份由 `manifest.json` 索引的檔案樹。安裝版
的資料由 MSI／安裝器依 manifest 複製；開發時可以指向任何相容的資料根目錄。

## 佈局

```text
<data root>/
  manifest.json                       # 整個資料根目錄的索引
  packs/ja-lexicon-mozc-v2.pack       # 日文 Mozc pack v2
  packs/ja-lexicon-mozc-v2.manifest.json
  build-zh/lexicon.bin                # 注音詞典
  build-zh/lm.bin                     # 注音 unigram
  build-zh/manifest.json              # Bundle::open 的驗證中繼檔
  build-zh-bigram/lm.bin              # 選配；存在時優先載入
  english/words.bin
  english/manifest.json
  english/NOTICE.txt
  models/zh-select/model.q8_0.gguf    # 注音選字，Q8_0
  models/zh-select/vocab.json
  models/zh-select/manifest.json
  models/external/jinen-v2-small/jinen-v2-small-Q4_K_M.gguf
  models/quant/jinen-v2-xsmall/tokenizer.json
  opencc/*.txt                        # 簡繁轉換
```

`settings.json`、`personal.json` 是使用者資料，不列入 manifest。模型來源、雜湊與
授權見 [`docs/models.md`](../models.md)。

## manifest 格式

`schema` 固定 `0c.data-pack/1`，由 `jc-cli data manifest` 掃描實際檔案產生。
欄位定義同步寫在 `crates/jc-data/src/manifest.rs`。

| 欄位 | 型別 | 說明 |
|---|---|---|
| `schema` | string | 必須是 `0c.data-pack/1` |
| `version` | string | 資料內容標籤，預設產生當天 UTC 日期 |
| `generated_at` | string | RFC 3339 UTC |
| `files[].path` | string | 相對資料根目錄，用 `/`；不可絕對路徑或含 `..` |
| `files[].bytes` | integer | 檔案大小 |
| `files[].sha256` | string | 64 個小寫十六進位字元 |
| `files[].role` | string | `ja_lexicon`、`zh_lexicon`、`zh_lm`、`english`、`ja_neural`、`ja_tokenizer`、`zh_select` |
| `files[].license` | string[] | `legal/README.md` 的 `L-xx` 條目 |
| `files[].shipped` | bool | `true` 隨產品散布；`false` 由使用者下載 |
| `files[].url` | string? | 下載項的固定 revision URL；隨附檔沒有這個欄位 |

目前 catalog 的檔案都標 `shipped: true`，`zh_select` 的 GGUF、vocab 與 manifest
也隨安裝包散布。`zh_punct` 與 `ja_punct` 已移除，舊資料目錄的殘留檔不再收錄。

`build-zh-bigram/lm.bin` 是選配資料：安裝版不帶，存在時引擎優先載入；檔案存在但
載入失敗只記警告並退回 `build-zh/lm.bin` 的 unigram。

## 產生與驗證

```sh
# 掃描資料根目錄並算出大小與 SHA-256
jc-cli data manifest <root> --out <root>/manifest.json

# 逐檔檢查存在、大小與雜湊
jc-cli data verify <root> --manifest <root>/manifest.json
```

- `data manifest` 不信任既有 manifest；下載項不存在就略過，`shipped: true` 的
  檔案不存在直接失敗並列出找過的路徑。`--version` 可覆寫資料版本。
- `data verify` 省略 `--manifest` 時讀 `<root>/manifest.json`；`--json` 輸出完整
  report。
- `shipped: true` 缺少、任何已存在檔案大小或 SHA-256 不符都是錯誤，exit 1。
  下載項缺少只記警告，人讀輸出預設不列。
- `ja_tokenizer` 是例外：缺少時引擎只警告並停用 Jinen，日文維持統計轉換。

## 引擎啟動檢查

`jc-engine` 在載入詞典前執行 `check_data_manifest`：

1. 沒有 `<root>/manifest.json`：記一次警告後繼續，供尚未產生 manifest 的開發
   資料根目錄使用。
2. manifest 解析失敗或 `schema` 不符：拒絕啟動。
3. 逐檔比對存在與大小；`shipped: true` 缺少或大小不符拒絕啟動，`shipped: false`
   缺少只警告。
4. 不算 SHA-256；完整雜湊留給 `data verify`。

`schema` 是硬界線；`version` 只記錄與顯示，不參與比對。`settings.json` 的
`schema`（目前 2）另由 `jc-settings` 判定，較新或損毀時唯讀，不擋引擎啟動。

## 下載

`data status` 與 `data download` 處理 manifest 中 `shipped: false` 且帶 `url` 的
項目：

```sh
jc-cli data status <root> --manifest <root>/manifest.json
jc-cli data download <root> --manifest <root>/manifest.json
```

- `status` 實際計算 SHA-256，狀態為已存在／缺少／大小不符／雜湊不符；缺少下載項
  不算錯誤。
- `download` 先寫 `<檔名>.part`，以 `Range` 續傳；伺服器忽略 Range 時從 0 重下。
  完成後 `sync_all` 再改名。大小或雜湊不符會刪掉 `.part`，不留半個正式檔。
- 進度寫 stderr，`--quiet` 關閉。沒有 manifest 時兩個子命令都直接失敗。
- 目前 catalog 沒有下載項；工具保留給日後需要另外散布的權重。
