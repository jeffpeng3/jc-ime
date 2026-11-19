# bigram-data：找回注音 bigram 並重建詞典

用新酷音 `chewing-v4` 的 `bigram_p50.arpa` 補回注音 bigram，重建到
`/mnt/work/jc/jc-ime-data/build-zh-bigram/`，`build-zh/` 保持不動。

## 資料來源

| 項目 | 值 |
|---|---|
| 來源 | https://codeberg.org/chewing/libchewing-data |
| 路徑 | `dict/chewing_v4/bigram_p50.arpa`、`dict/chewing_v4/tsi_unigram.arpa` |
| commit | `e5d726d9c09afd562309b05d9adad29940de5ebb` |
| 授權 | CC-BY-4.0（姓名標示：libchewing-data（新酷音詞庫）dict/chewing_v4，libchewing Core Team） |
| `bigram_p50.arpa` | 150,068,322 bytes，SHA-256 `ce25ad7e282cfc710f61eea20ba666b3116ffc37de02f92e2df889fbeb811ee5` |
| `tsi_unigram.arpa` | 4,027,013 bytes，SHA-256 `0580f128fa47a42fc211c6774a4a4d558b1a022b8dc3b3f107618368d94dc7b8` |
| 轉出的 `chewing-v4.tsv` | 7,109,312 筆，191.2 MiB，SHA-256 `e94f8d467871b453ff0fc7c08342eb2c2bb1ce83c4dc36dd05cb71191d53f59b` |

轉檔用 `scripts/fetch_chewing_bigram.sh`：固定 commit 下載，驗 SHA-256，
把 ARPA 的 log10 `P(詞|前詞)` 換算成機率，輸出三欄
`前詞<TAB>詞<TAB>P(詞|前詞)`。`tsi_unigram.arpa` 只記錄來源，unigram 沿用
`ref/0c-ime/data/lm/unigram.tsv` 的 `chewing-v4` 欄。

## 重建

建置用的來源目錄是 `ref/0c-ime/data` 的複本（`/tmp/opencode/chewing/data`），
加上 `lm/bigram/chewing-v4.tsv`，避免改動唯讀的 ref repo：

```sh
cd /mnt/work/jc/ref/0c-ime
CGO_ENABLED=0 go run ./cmd/0c-compile \
  -data /tmp/opencode/chewing/data \
  -out /mnt/work/jc/jc-ime-data/build-zh-bigram \
  -bigram "chewing-v4=1"
```

| 項目 | `build-zh`（舊） | `build-zh-bigram`（新） |
|---|---|---|
| `counts.entries` | 271,170 | 271,170 |
| `counts.words` | 238,021 | 238,021 |
| `counts.bigrams` | 0 | 7,028,326 |
| `lm.bin` 大小 | 4,766,114 | 32,879,418 |
| `lm.bin` SHA-256 | `353fd25a…` | `10ffd7fc…` |
| `lexicon.bin` 大小 | 8,791,416 | 8,791,416 |
| `report.txt` 錯誤 | 無 | 無（只有不阻擋出版的「字詞不一致」2022 筆） |

7,109,312 筆原始 bigram 有 80,986 筆因為前詞或後詞不在詞庫被丟掉，剩
7,028,326 筆寫進 `lm.bin`。

## golden 比較

`jc-cli eval zh-golden` 只吃 `0c-testdata/golden/zh/data/lexicon.tsv` 與
`lm.tsv` 這份凍結 baseline，沒有 `--data` 參數；`crates/jc-cli/src/data.rs`
的 `load_zh_resources` 也不讀 `lexicon.bin`／`lm.bin`。因此新資料包無法接進
目前的 Rust golden，兩次執行結果必然相同：

| 版本 | 通過數 |
|---|---|
| 舊（`build-zh`） | 27/45 |
| 新（`build-zh-bigram`） | 27/45（同一份凍結 baseline，未載入新資料包） |

要真正比對 bigram 對 Rust 解碼的影響，得先等 `jc-data` 支援注音 pack 載入
（見 `docs/port/j2-ja-lexicon.md` 的待辦）。

替代驗證：用 ref 的 Go 引擎載入兩個資料包，跑同一組按鍵
（`su3cl3`、`ji394su3`、`a86qi6`），commit 結果都是「你好」「我愛你」「麻婆」，
但 `我愛` 的候選在舊包是 `我愛…｜░你 (:tab 接受)`，新包是 `我愛`，確認
bigram 有被載入並影響候選排序。

## Port 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `cmd/0c-compile/main.go` | — | n/a：直接用 ref 的 Go 編譯器產生資料包，不改 Rust |
| `internal/compile/source.go`（bigram glob） | — | n/a：同上 |
| `internal/compile/build.go`（`readBigrams`） | — | n/a：同上 |
| `cmd/0c-cli/main.go`（`datadir.Open` 讀 `lexicon.bin`／`lm.bin`） | `crates/jc-cli/src/data.rs` | n/a：Rust 端尚無注音 pack 載入，golden 仍讀固定 TSV |
| `scripts/fetch_chewing_bigram.sh` | `scripts/fetch_chewing_bigram.sh` | rewritten：改成固定 commit 下載加 SHA-256 驗證 |
