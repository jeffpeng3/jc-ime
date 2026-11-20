# data-download：首次啟動下載模型

Worktree：`/mnt/work/jc/wt/data-download`（branch `data-download`，base 是 `data-pack` 合併後的 `main`）

> **現況（2026-11-21）**：`zh_punct` 已從 catalog 移除，不再下載。manifest 從 10 筆
> 改成 8 筆，下載項只剩 2 個 `zh_select`。本文的下載清單、狀態實測與測試數字是
> 2026-11-20 的移植歷史，保留供回溯。

## 目標

Funmula 的注音選字／標點模型不隨產品散布，只由使用者下載。這輪把「有哪些
下載項、從哪下載、下載到哪裡、怎麼確認內容」固定下來：

- `jc-cli data status <root>` 列出下載項的已存在／缺少／大小不符／雜湊不符。
- `jc-cli data download <root>` 依 manifest 的 `url` 下載，支援 `.part` 續傳。
- 下載完成前不產生正式檔名；大小或 SHA-256 不符就刪掉暫存檔並回報。
- 引擎啟動時缺少下載項只降級，不拒絕啟動（由 `data-pack` 實作，本輪沿用）。

## 移植對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `cmd/0c-update/models.go`（`modelsStatus`、`modelView`、`printModels`） | `crates/jc-data/src/download.rs`（`DownloadReport`、`DownloadStatus`）＋`crates/jc-cli/src/download_cmd.rs`（`status`） | rewritten：改成 manifest 驅動，沒有 `models.lock`、配對目錄或引擎狀態查詢；狀態固定為已存在／缺少／大小不符／雜湊不符 |
| `cmd/0c-update/models.go`（`download`、`progress`、`last_error`） | `crates/jc-cli/src/download_cmd.rs`（`download`） | rewritten：前景同步下載，進度寫 stderr；沒有背景 worker、`operation.json`、檔案鎖與 SIGTERM 取消 |
| `internal/models/store.go`（`Fetcher.fetch`、`checkFile`） | `crates/jc-cli/src/download_cmd.rs`（`fetch_file`、`hash_prefix`） | rewritten：新增 `.part`、`Range` 續傳、完成後 rename；Go 版直接寫目標檔且不續傳，也沒有伺服器忽略 Range 的退路 |
| `internal/models/store.go`（`Install`、`replaceDir`、`cleanup`、硬連結） | — | n/a：下載後配對、沿用舊權重與清理是舊模型目錄佈局；本專案的檔案位置由資料根目錄 manifest 決定 |
| `cmd/0c-update/model_status.go` | — | n/a：查執行中引擎的即時狀態；本專案沒有常駐引擎的健康查詢通道 |
| `cmd/0c-update/download_now.go` | — | n/a：那是程式本體自動更新，不是模型下載 |
| 前身無對應 | `crates/jc-data/src/download.rs`（`inspect_file`、`part_path`） | 新行為：manifest 下載項的唯讀狀態查詢 |
| 前身無對應 | `crates/jc-cli/src/cli.rs`（`DataStatus`、`DataDownload`） | 新行為：`data status`、`data download` 參數解析 |

對應的行為測試改了名字，因為舊測試都繞著 `models.Store` 的配對目錄：

| Go 測試 | Rust 測試 | 說明 |
|---|---|---|
| `TestInstallDownloadsVerifiesAndPairs` | `downloads_fresh_file_and_renames_part` | 只留「下載＋驗雜湊＋放上正式檔名」，不含 pairing |
| `TestInstallFailureLeavesNothing` | `hash_mismatch_deletes_part_and_reports`、`size_mismatch_is_rejected_before_writing` | 失敗不留半檔 |
| `TestInstallStallIsNetworkError` | `restarts_when_server_ignores_range`、本文件的斷線實測 | 續傳與中斷的處理和 Go 版不同，改成新的行為測試 |

## 下載清單與 URL

下載項就是 manifest 裡 `shipped: false` 的檔案，必須同時有 `url`、`bytes`、
`sha256`。目前四項（`crates/jc-data/src/manifest.rs` 的 `CATALOG`）：

| 路徑 | 角色 | 授權 | 固定 revision |
|---|---|---|---|
| `models/zh-select/model.onnx` | `zh_select` | L-11（MIT） | `Funmula/0c-sequence-d512` `3533e34e` |
| `models/zh-select/vocab.json` | `zh_select` | L-11（MIT） | 同上 |
| `models/zh-punct/model.onnx` | `zh_punct` | L-12（MIT） | `Funmula/0c-punct-d512` `105c2770` |
| `models/zh-punct/vocab.json` | `zh_punct` | L-12（MIT） | 同上 |

- URL 一律指向 Hugging Face 的 `/resolve/<commit>/<檔名>`，不是 `main`。
- URL 只存在 manifest，程式不寫死鏡像站。要換鏡像就重新產生 manifest 並改
  `url`；下載器仍然只信 `bytes` 與 `sha256`，換網址不會換掉內容。
- 授權見 `legal/README.md` 的 L-11、L-12：兩個模型**不進 git、release 或安裝
  包**，只允許使用者下載。程式碼只提供下載工具，不轉散布權重。

## 用法

```sh
# manifest 是資料根目錄的索引；data-pack 沒有把它寫進 /mnt/work/jc/jc-ime-data（唯讀），
# 所以先產生到暫存位置。
cargo run -p jc-cli --release -- data manifest /mnt/work/jc/jc-ime-data \
  --out /tmp/opencode/manifest.json

cargo run -p jc-cli --release -- data status /mnt/work/jc/jc-ime-data \
  --manifest /tmp/opencode/manifest.json

cargo run -p jc-cli --release -- data download /mnt/work/jc/jc-ime-data \
  --manifest /tmp/opencode/manifest.json
```

- `data status`：`--json` 輸出逐項狀態。缺少下載項不算錯誤，exit 0。
- `data download`：`--quiet` 關閉 stderr 進度。全部成功 exit 0；任一檔案失敗
  exit 1，訊息列出每個失敗原因。
- 沒有 `manifest.json` 時兩個子命令都會直接失敗，錯誤帶路徑；不會自行猜測
  下載清單。
- `data status` 會實際算 SHA-256。對現有 4 個下載項（261,932,628 位元組）實測
  約 0.5 秒（頁快取已熱）。引擎啟動不做這件事，只比大小。

## 下載器行為

| 情況 | 行為 |
|---|---|
| 正式檔已存在且大小、雜湊都對 | 略過，印「已存在」 |
| 正式檔大小或雜湊不符 | 印「重新下載」，先刪掉舊檔再抓 |
| 沒有正式檔 | 寫 `<檔名>.part` |
| `.part` 已存在 | 送 `Range: bytes=<大小>-`，回應 `206` 就續傳 |
| 伺服器回 `200`（忽略 Range） | 截掉 `.part`，從 0 重下 |
| 伺服器回 `416` | 刪掉 `.part`，重試一次不帶 Range |
| `Content-Range` 起點與要求不符 | 第一次重下；再不符就報錯 |
| `.part` 大小已等於 manifest | 先驗雜湊；對就直接改名，不連網 |
| 傳輸中斷、提早 EOF | 保留 `.part`，回報「下載中斷，可重新執行續傳」 |
| 總大小不符 | 刪掉 `.part`，回報大小不符 |
| SHA-256 不符 | 刪掉 `.part`，回報雜湊不符，不留正式檔 |
| 完成 | `sync_all` 後把 `.part` rename 成正式檔名 |
| `shipped: false` 但 manifest 沒有 `url` | 該項失敗，其他項照跑 |

進度格式（stderr，每 250 ms 最多一行）：

```text
models/zh-select/model.onnx 42.3%（55.3 MB/130.7 MB，18.2 MB/s）
```

大小用十進位 MB，和 `legal/README.md`、`data-pack.md` 的數字一致。`--quiet`
只關進度，stdout 的「下載／完成／失敗／下載結果」照常輸出。

## 狀態判定與實測

用本機 HTTP 伺服器（支援 Range）與一份 3,000,000 位元組的測試檔：

```text
$ jc-cli data status /tmp/opencode/dl/root
缺少        models/zh-select/model.onnx（zh_select，預期 3000000 位元組）
缺少        models/zh-select/vocab.json（zh_select，預期 29 位元組）
下載項：已存在 0/2，缺少 2，不符 0
```

下載後：

```text
$ jc-cli data download /tmp/opencode/dl/root
下載        models/zh-select/model.onnx（zh_select，3000000 位元組）
完成        models/zh-select/model.onnx
下載        models/zh-select/vocab.json（zh_select，29 位元組）
完成        models/zh-select/vocab.json
下載結果：完成 2/2，失敗 0
```

三種狀態都實際跑過：

| 操作 | `data status` 輸出 |
|---|---|
| 正常檔 | `已存在      models/zh-select/model.onnx（zh_select，3000000 位元組）` |
| 刪檔 | `缺少        …（預期 3000000 位元組）`；有 `.part` 時附「已下載 … 可續傳」 |
| 改 1 個位元組（大小相同） | `雜湊不符    …（manifest <hash>，實際 <hash>）` |
| 多 1 個位元組 | `大小不符    …（manifest 29 位元組，實際 30 位元組）` |

真實資料根目錄（4 個下載項都在）：

```text
$ jc-cli data status /mnt/work/jc/jc-ime-data --manifest /tmp/opencode/manifest.json
已存在      models/zh-select/model.onnx（zh_select，130723137 位元組）
已存在      models/zh-select/vocab.json（zh_select，312708 位元組）
已存在      models/zh-punct/model.onnx（zh_punct，130584075 位元組）
已存在      models/zh-punct/vocab.json（zh_punct，312708 位元組）
下載項：已存在 4/4，缺少 0，不符 0
```

## 續傳與失敗實測

### 1. 手動留下的 `.part` 續傳（本機 HTTP）

先放 1,000,000 位元組的 `.part`，再執行 `data download --quiet`：

```text
下載        models/zh-select/model.onnx（zh_select，3000000 位元組）
完成        models/zh-select/model.onnx（續傳自 1.0 MB）
已存在      models/zh-select/vocab.json（zh_select）
下載結果：完成 2/2，失敗 0
```

伺服器紀錄只有一次 Range 請求：

```text
model.onnx range='bytes=1000000-' start=1000000
```

下載後 SHA-256 與來源檔相同。

### 2. 傳輸中斷後接續

伺服器送完 900,000 位元組就斷線：

```text
下載        models/zh-select/model.onnx（zh_select，3000000 位元組）
失敗        models/zh-select/model.onnx：… 下載中斷：Peer disconnected（保留 .part，可重新執行續傳）
下載結果：完成 1/2，失敗 1
jc-cli: 下載失敗 1 個檔案：…
```

`.part` 留在 900,000 位元組；換回正常伺服器再跑：

```text
下載        models/zh-select/model.onnx（zh_select，3000000 位元組）
完成        models/zh-select/model.onnx（續傳自 900.0 KB）
下載結果：完成 2/2，失敗 0
```

### 3. 伺服器忽略 Range

`--no-range` 的伺服器收到 `Range: bytes=123456-` 仍回 `200` 與完整內容：

```text
下載        models/zh-select/model.onnx（zh_select，3000000 位元組）
完成        models/zh-select/model.onnx
下載結果：完成 1/1，失敗 0
```

結果檔的 SHA-256 正確，`.part` 已消失。客戶端沒有把 123,456 位元組接在完整
內容前面。

### 4. 雜湊不符

manifest 故意寫錯 SHA-256：

```text
下載        models/zh-select/model.onnx（zh_select，3000000 位元組）
失敗        models/zh-select/model.onnx：… SHA-256 不符：manifest 000…，實際 195f…
下載結果：完成 0/1，失敗 1
jc-cli: 下載失敗 1 個檔案：…
```

目錄裡沒有 `model.onnx`，也沒有 `model.onnx.part`；exit 1。

### 5. 真實 Hugging Face（需要網路）

用真實 manifest 的 `models/zh-select/vocab.json`（312,708 位元組）實際下載：

```text
下載        models/zh-select/vocab.json（zh_select，312708 位元組）
完成        models/zh-select/vocab.json
下載結果：完成 1/1，失敗 0

$ sha256sum …
556dffaffbf4c05211a846dcee55ef735cea68c5754f9bffcc6f9588124172b3
```

`HEAD`＋`Range: bytes=0-1023` 顯示 Hugging Face 回 `302` 到 CDN 後回
`206`、`content-range: bytes 0-1023/130723137`。再留 100,000 位元組的 `.part`
重跑，得到「續傳自 100.0 KB」，最終雜湊相同，代表 Range 有跟著轉址帶到 CDN。

## 引擎啟動時的降級

`crates/jc-engine/src/loader.rs` 的 `check_data_manifest`（`data-pack` 已實作）
在載入詞典前逐項比對存在與大小：

- `shipped: false` 缺少：每個項目記一次
  `下載項缺席：<path>（<role>），相關功能維持停用` 的警告，繼續啟動。
- `shipped: true` 缺少或任何檔案大小不符：拒絕啟動。
- 不重算 SHA-256；雜湊留給 `data verify` 與下載器。

本輪沒有改 `jc-engine`（不在 owned paths），沿用既有行為。缺少四個下載項時
引擎仍可啟動，zh-select 與 zh-punct 的功能自然降級；`models/zh-select/model.onnx`
是本專案目前唯一會載入的下載項。

## 離線時會怎樣

| 情境 | 結果 |
|---|---|
| 引擎啟動，下載項缺少 | 每個缺少項目一次警告後繼續，使用統計選字 |
| `data status` | 完全離線，只讀 manifest 與本機檔案 |
| `data download`，沒有網路 | 該檔失敗，訊息帶連線錯誤；`.part` 保留，下次可續傳 |
| 下載到一半斷線 | `.part` 保留；重跑從斷點續傳 |
| 伺服器不支援續傳 | 自動從 0 重下，結果仍要通過大小與雜湊 |
| 有 `.part` 但 manifest 換版 | 大小或雜湊不符就刪掉重下，不會沿用錯的內容 |

## 測試

| 檔案 | 測試 |
|---|---|
| `crates/jc-data/src/download.rs` | 已存在、缺少（含 `.part`）、同大小不同內容、大小不符、`.part` 路徑、真實資料根目錄有 4 個下載項且都帶 URL |
| `crates/jc-cli/src/download_cmd.rs` | 進度格式與十進位單位、`Content-Range` 解析、全新下載後改名、Range 續傳、伺服器忽略 Range 時重下、雜湊不符刪檔、大小不符先拒絕 |
| `crates/jc-cli/src/cli.rs` | `data status`／`data download` 參數解析、各子命令的旗標互斥 |

依照共通規則，這輪只寫測試與編譯，沒有執行 `cargo test`；上面的數字是手動
執行 release 建置的二進位取得的。

## 需要網路才能驗的項目

- `https://huggingface.co` 的完整 130 MB 模型下載：本輪只實際下載 312 KB 的
  `vocab.json`（含轉址與續傳），其餘三個檔案用本機 HTTP 驗流程。
- Hugging Face 若改路徑或撤掉 revision，`data download` 會回報 HTTP 狀態；
  需要網路才能確認四個固定 revision 仍可取用。
- 憑證鏈：ureq 使用 rustls＋webpki 根憑證；換到自簽或企業 proxy 的環境需要
  另外驗證。
