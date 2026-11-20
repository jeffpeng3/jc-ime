# data-pack：資料佈局、manifest 與驗證

Worktree：`/mnt/work/jc/wt/data-pack`（branch `data-pack`）

## 目標

把「安裝版資料根目錄長什麼樣、哪些檔要帶、帶的檔對不對」固定下來：

- `manifest.json` 是整個資料根目錄的索引，記錄版本、逐檔大小、SHA-256、角色、
  授權條目與是否隨產品散布。
- `jc-cli data manifest <root>` 實際掃描檔案並算出雜湊。
- `jc-cli data verify <root>` 逐檔檢查存在、大小與雜湊。
- 引擎啟動時只讀 manifest、比對存在與大小，不重算上百 MB 的雜湊。

這輪只做格式、工具與檢查，沒有搬動 `/mnt/work/jc/jc-ime-data` 的任何檔案。

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `cmd/0c-update/models.go`（`manifestModelSHA`、`modelView.SHA256`） | `crates/jc-data/src/manifest.rs`（`sha256_file`、`DataFile`） | rewritten：改成整個資料根目錄的索引；單一模型的狀態查詢不在這裡 |
| `cmd/0c-update/models.go`（`modelsStatus`、`download`） | — | n/a：下載、續傳與進度屬於 `data-download` 任務，本輪只提供 `url` 欄位 |
| `internal/modelsets/manifest.go` | `crates/jc-settings/src/modelset/manifest.rs` | n/a：既有模型包 manifest 只管 `models/` 的角色，與資料根目錄 manifest 是不同層 |
| 前身無對應 | `crates/jc-data/src/manifest.rs`（`DataManifest::generate`、`DataManifest::verify`） | 新行為：Go 沒有整根目錄 manifest 產生器 |
| 前身無對應 | `crates/jc-cli/src/manifest_cmd.rs` | 新行為：`data manifest`、`data verify` |
| `cmd/0c-server/main.go` 的啟動載入 | `crates/jc-engine/src/loader.rs`（`check_data_manifest`） | rewritten：啟動只驗存在與大小，雜湊留給 `data verify` |

## 佈局

目前 `/mnt/work/jc/jc-ime-data` 的實際佈局：

```text
<mnt/work/jc/jc-ime-data>/
  packs/ja-lexicon-mozc-v2.pack
  packs/ja-lexicon-mozc-v2.manifest.json
  build-zh/lexicon.bin
  build-zh/lm.bin
  build-zh/manifest.json
  build-zh-bigram/lm.bin                 # 選用；引擎優先載入
  english/words.bin
  english/manifest.json
  models/external/jinen-v2-small/jinen-v2-small-Q5_K_M.gguf
  models/zh-select/model.onnx            # 下載項
  models/zh-select/vocab.json            # 下載項
  models/zh-punct/model.onnx             # 下載項，程式自 2026-11-20 起不載入
  models/zh-punct/vocab.json             # 下載項
  opencc/*.txt
  manifest.json                          # 由工具產生；目前尚未寫進這個目錄
```

`models/external/jinen-v2-small/` 的層級是 `jc-engine` 目前寫死的路徑
（`JA_NEURAL_GGUF`）。安裝器要保留這個相對路徑，否則引擎找不到 Jinen。
`models/zh-select/model.q8_0.gguf` 是本機從 ONNX 轉出的量化衍生物，不是上游
下載檔，因此不列進 manifest，避免安裝器把它當成可下載的原始檔。

使用者資料（`settings.json`、`personal.json`）不列入 manifest。

## manifest 欄位

`schema` 固定 `0c.data-pack/1`。JSON 範例與欄位表也寫在
`crates/jc-data/src/manifest.rs` 的 module 文件。

| 欄位 | 型別 | 說明 |
|---|---|---|
| `schema` | string | 格式版本，必須是 `0c.data-pack/1` |
| `version` | string | 資料內容版本標籤；預設是產生當天的 UTC 日期 |
| `generated_at` | string | 產生時間，RFC 3339 UTC |
| `files[].path` | string | 相對資料根目錄，一律用 `/`；不可絕對路徑或含 `..` |
| `files[].bytes` | integer | 檔案大小 |
| `files[].sha256` | string | 64 個小寫十六進位字元 |
| `files[].role` | string | `ja_lexicon`、`zh_lexicon`、`zh_lm`、`english`、`ja_neural`、`zh_select`、`zh_punct` |
| `files[].license` | string[] | `legal/README.md` 的 `L-xx` 條目 |
| `files[].shipped` | bool | `true` 隨產品散布；`false` 由使用者下載 |
| `files[].url` | string? | 下載項的來源 URL；隨附檔沒有這個欄位 |

`zh_punct` 是額外角色，用來標記 Funmula 的標點模型。程式已不載入它，但檔案
仍在資料目錄，manifest 要能表達「這是哪個授權的下載項」。

## 真實 manifest

2026-11-20T03:48:18Z 對 `/mnt/work/jc/jc-ime-data` 產生的內容：

```json
{
  "schema": "0c.data-pack/1",
  "version": "2026-11-20",
  "generated_at": "2026-11-20T03:48:18Z",
  "files": [
    {
      "path": "packs/ja-lexicon-mozc-v2.pack",
      "bytes": 85508727,
      "sha256": "15926f48dca7a6917f067a41f8ec49e2474c11d0cdebce5486a81f51fd0c84c6",
      "role": "ja_lexicon",
      "license": ["L-01"],
      "shipped": true
    },
    {
      "path": "build-zh/lexicon.bin",
      "bytes": 8791416,
      "sha256": "46721ccb95e55c7fc36abd3a351f9191eef087e47bd30b580c5711c05dc30f57",
      "role": "zh_lexicon",
      "license": ["L-04", "L-05", "L-06", "L-13", "L-14", "L-15", "L-16", "L-17"],
      "shipped": true
    },
    {
      "path": "build-zh/lm.bin",
      "bytes": 4766114,
      "sha256": "353fd25aca9bf80cef0eb23fb6bbc551aa91abd46ee15f07b4d88f231bf858fb",
      "role": "zh_lm",
      "license": ["L-04", "L-05", "L-06", "L-13", "L-14", "L-15", "L-16", "L-17"],
      "shipped": true
    },
    {
      "path": "english/words.bin",
      "bytes": 1522028,
      "sha256": "aa03049852110645bce87b5a7bcb9a383a1b0748e59465827148adb534701782",
      "role": "english",
      "license": ["L-08"],
      "shipped": true
    },
    {
      "path": "models/external/jinen-v2-small/jinen-v2-small-Q5_K_M.gguf",
      "bytes": 81117824,
      "sha256": "80482707513d6b67dafc31774371cf95d765542abf8d74eebf5f32f92d788bd3",
      "role": "ja_neural",
      "license": ["L-21"],
      "shipped": true
    },
    {
      "path": "models/zh-select/model.onnx",
      "bytes": 130723137,
      "sha256": "4a80caf18acbaddde10c908dcbf35919908d616e94618cdde71fc3409c969261",
      "role": "zh_select",
      "license": ["L-11"],
      "shipped": false,
      "url": "https://huggingface.co/Funmula/0c-sequence-d512/resolve/3533e34ef25896afeacc928d136ba7f35783bba7/model.onnx"
    },
    {
      "path": "models/zh-select/vocab.json",
      "bytes": 312708,
      "sha256": "556dffaffbf4c05211a846dcee55ef735cea68c5754f9bffcc6f9588124172b3",
      "role": "zh_select",
      "license": ["L-11"],
      "shipped": false,
      "url": "https://huggingface.co/Funmula/0c-sequence-d512/resolve/3533e34ef25896afeacc928d136ba7f35783bba7/vocab.json"
    },
    {
      "path": "models/zh-punct/model.onnx",
      "bytes": 130584075,
      "sha256": "4db3ed40df777347d9c455da01285b01de192d4c210c5f7cd9f0646c727880d8",
      "role": "zh_punct",
      "license": ["L-12"],
      "shipped": false,
      "url": "https://huggingface.co/Funmula/0c-punct-d512/resolve/105c2770e0b7cf4ce44a792b3b186000dd1727d9/model.onnx"
    },
    {
      "path": "models/zh-punct/vocab.json",
      "bytes": 312708,
      "sha256": "556dffaffbf4c05211a846dcee55ef735cea68c5754f9bffcc6f9588124172b3",
      "role": "zh_punct",
      "license": ["L-12"],
      "shipped": false,
      "url": "https://huggingface.co/Funmula/0c-punct-d512/resolve/105c2770e0b7cf4ce44a792b3b186000dd1727d9/vocab.json"
    }
  ]
}
```

實際輸出是完整展開的格式，每個授權代號各佔一行；上面為了閱讀把 `license`
陣列排成一行。內容與當次產物相同。

## 產生

```sh
cargo run -p jc-cli --release -- data manifest /mnt/work/jc/jc-ime-data \
  --out /tmp/opencode/manifest.json
```

輸出：

```text
manifest：/tmp/opencode/manifest.json
產生時間：2026-11-20T03:48:18Z
資料版本：2026-11-20
檔案：9（隨附 5，下載 4），共 443638737 位元組
```

- 大小與 SHA-256 都是實際讀檔算出，不信任既有的資料包 manifest。
- 下載項不存在就略過；隨附檔不存在直接失敗，並列出找過的路徑。
- `--out` 省略時寫 `<root>/manifest.json`。
- `--version <標籤>` 可覆寫資料版本；預設用 UTC 日期。

隨附 5 個檔共 181,706,109 位元組，下載 4 個檔共 261,932,628 位元組。

## 驗證

```sh
cargo run -p jc-cli --release -- data verify /mnt/work/jc/jc-ime-data \
  --manifest /tmp/opencode/manifest.json
```

對現有資料的實測結果：

```text
通過      packs/ja-lexicon-mozc-v2.pack（ja_lexicon）
通過      build-zh/lexicon.bin（zh_lexicon）
通過      build-zh/lm.bin（zh_lm）
通過      english/words.bin（english）
通過      models/external/jinen-v2-small/jinen-v2-small-Q5_K_M.gguf（ja_neural）
通過      models/zh-select/model.onnx（zh_select）
通過      models/zh-select/vocab.json（zh_select）
通過      models/zh-punct/model.onnx（zh_punct）
通過      models/zh-punct/vocab.json（zh_punct）
驗證 /tmp/opencode/manifest.json：通過 9/9，錯誤 0，警告 0
```

`--manifest` 省略時讀 `<root>/manifest.json`；`--json` 輸出逐檔狀態。

### 失敗判定

| 情況 | 結果 |
|---|---|
| `shipped: true` 的檔缺少 | 錯誤，`data verify` 回傳非零，引擎拒絕啟動 |
| 任何已存在檔案的大小不符 | 錯誤 |
| 任何已存在檔案的 SHA-256 不符 | 錯誤 |
| `shipped: false` 的檔缺少 | 警告，`data verify` 仍回傳 0，引擎只記警告後降級 |

實測：把 `english/words.bin` 複製到暫存根目錄並改 1 個位元組，`data verify`
回報 `雜湊不符`、`通過 8/9，錯誤 1`、exit 1。移除一個 zh-punct 下載項後，
回報 `缺少 …（下載項，警告）`、`錯誤 0，警告 1`、exit 0。移除
`english/words.bin` 後，回報 `缺少 …（隨附檔，錯誤）`、exit 1。

## 引擎啟動檢查

`crates/jc-engine/src/loader.rs` 的 `check_data_manifest` 在載入任何詞典前
執行：

1. `<root>/manifest.json` 不存在：記一次警告，繼續啟動。這是給還沒有 manifest
   的開發資料根目錄用的退路。
2. manifest 解析失敗或 `schema` 不符：拒絕啟動，錯誤帶 manifest 路徑與原因。
3. 逐檔比對存在與大小。`shipped: true` 缺少或大小不符：拒絕啟動；
   `shipped: false` 缺少：警告一次。
4. 只算 `std::fs::metadata`，不算 SHA-256。

實測：在只有 5 個小檔的暫存根目錄產生 manifest，刪掉 `build-zh/lm.bin` 後啟動
`jc-engine`，得到：

```text
[ERROR jc_engine::loader] 資料檔檢查失敗：build-zh/lm.bin（zh_lm）：Missing
jc-engine: 載入資料失敗：… 資料 manifest 檢查失敗：通過 4/5，錯誤 1，警告 0
```

## 版本欄位

| 檔案 | 欄位 | 不符時的行為 |
|---|---|---|
| `manifest.json` | `schema`（string） | manifest 結構未知，拒絕啟動；`data verify` 直接失敗 |
| `manifest.json` | `version`（string） | 只記錄與顯示。引擎寫進 log，不參與比對 |
| `settings.json` | `schema`（integer，目前 2） | 由 `jc-settings` 判定；未知或比程式新的版本會把設定標成唯讀，不擋引擎啟動 |

`manifest.json` 的 `schema` 是硬界線，因為欄位語意可能改變；`version` 是資料
內容標籤，允許同一版程式搭配不同日期的資料。`settings.json` 由使用者擁有，
壞掉或版本較新時採唯讀，避免覆寫使用者的設定。

## 隨附與下載清單

### 隨附（`shipped: true`）

| 路徑 | 角色 | 授權 | 理由 |
|---|---|---|---|
| `packs/ja-lexicon-mozc-v2.pack` | `ja_lexicon` | L-01 | Mozc OSS 詞典為 BSD-3-Clause 系，可散布，附 LICENSE 與 README |
| `build-zh/lexicon.bin` | `zh_lexicon` | L-04、L-05、L-06、L-13～L-17 | 自製詞條、libchewing、政府開放資料等，全部允許散布 |
| `build-zh/lm.bin` | `zh_lm` | 同上 | 同上 |
| `english/words.bin` | `english` | L-08 | ESDB／SCOWL 為 MIT 類授權，附 NOTICE |
| `models/external/jinen-v2-small/jinen-v2-small-Q5_K_M.gguf` | `ja_neural` | L-21 | Jinen v2 small 為 CC-BY-SA-4.0，散布需附授權全文與 notice |

`legal/README.md` 的 L-21 目前仍寫「僅本機評估，暫不散布」。本輪只建立
manifest 與檢查工具，沒有改寫帳本；實際出貨前要由 legal-notices 任務確認
CC-BY-SA-4.0 的 notice 與同授權條件，再把帳本改成可散布。

### 下載（`shipped: false`）

Funmula 的兩個模型都是第三方權重，依 `legal/README.md` 的散布邊界只允許
使用者另外下載，不進 git、release 或安裝包。

| 路徑 | 角色 | 授權 | 理由 |
|---|---|---|---|
| `models/zh-select/model.onnx` | `zh_select` | L-11 | Funmula `0c-sequence-d512`，rev `3533e34e`，MIT，僅下載 |
| `models/zh-select/vocab.json` | `zh_select` | L-11 | 同上 |
| `models/zh-punct/model.onnx` | `zh_punct` | L-12 | Funmula `0c-punct-d512`，rev `105c2770`，MIT，僅下載；程式已不載入 |
| `models/zh-punct/vocab.json` | `zh_punct` | L-12 | 同上 |

`url` 直接指向 Hugging Face 的固定 revision。下載器要驗 manifest 的 `bytes` 與
`sha256`，不能只信 HTTP 成功。

## 測試

- `crates/jc-data/src/manifest.rs`：產生、JSON round-trip、schema 不符、路徑
  逃逸、雜湊格式、啟動級與完整驗證的差異、下載項警告與隨附檔錯誤、真實資料
  根目錄全量驗證。
- `crates/jc-cli/src/cli.rs`：`data manifest`、`data verify` 的參數解析與
  互斥旗標。
- `crates/jc-engine/src/loader.rs`：manifest schema 不符時拒絕啟動。

依照共通規則，這輪只寫測試與編譯，不執行 `cargo test`。
