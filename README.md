# jc-ime

Windows 優先的輸入法，核心全部以 Rust 實作，平台整合由獨立 wrapper 完成。

M0–M4 完成：注音 golden 45/45、模糊音 919 題、日文統計與 Jinen 神經轉換、
Mixed 6/6、注音選字模型與個人化都已接線。M5 的引擎端與 Windows TSF 前端完成，
x64 MSI 可安裝；M6 的設定程式與安裝器完成，更新器不做。逐項現況見
[`docs/status.md`](docs/status.md)。

安裝與反安裝步驟見 [`docs/install.md`](docs/install.md)，版本與發佈流程見
[`docs/release.md`](docs/release.md)。

## 現在的能力與限制

| 能力 | 現況 |
|---|---|
| 注音（大千鍵盤） | 聲調選填、候選翻頁、Tab 補全、簡繁轉換、Ctrl＋標點、mixed ASCII 推斷；golden 45/45 |
| 模糊音 | `select` 模式（`z-zh`、`c-ch`、`s-sh`、`en-eng`）；919 題 top-1 16.4%，注音 319 題 44.5% |
| 注音選字模型 | candle Q8_0，33.9 MiB、推論 p50 9.36 ms、逐對排序一致 99.51%、golden 45/45 |
| 日文（羅馬字→かな→漢字） | Mozc pack v2 與 context ID 矩陣；預設 Jinen small 門檻混合，`auto` 92.2%、`quality` 93.6% |
| 日文神經轉換 | Jinen v2 small Q4_K_M；`auto` 只在成本差與候選數達門檻時呼叫，逾時回退統計結果 |
| Mixed 路由 | 注音／日文／英文分段、英文直通與前綴補完；`eval mixed` 6/6 |
| 個人化 | rollback window、弱負向訊號、最短觸發長度；機制在 `jc-core`，資料可寫成 `personal.json` |
| 引擎行程 | stdio、Unix domain socket、Windows named pipe；`--supervise` 提供單一實例與崩潰重啟 |
| Windows TSF | x64 MSI 安裝「jc 注音」：組字、候選窗、滑鼠點選、虛線底線、`Win+Space` |
| 設定程式 | `jc-settings-ui`（GPUI Kit）讀寫 `settings.json`，引擎啟動時讀取 |
| client crate | `jc-windows` 可連 named pipe；`jc-linux`／`jc-mac` 是佔位，回 `Unsupported` |
| 平台 wrapper | Windows TSF 完成（x64）；Fcitx5、macOS 尚未實作 |


## Crate

| Crate | 內容 |
|---|---|
| `jc-core` | 鍵序列、注音音節與鍵盤、lattice／Viterbi、注音解碼、日文輸入與解碼、Mixed 路由、session、神經 runtime、輸出 |
| `jc-data` | 日文 `0CJADIC` pack v1／v2 與注音 `lexicon.bin`／`lm.bin` 的零拷貝讀取、manifest 驗證 |
| `jc-model` | 推論接線：manifest、tokenizer、candle reranker、注音選字（Q8_0）與 Jinen |
| `jc-proto` | 引擎與 wrapper 的 v3 訊框、請求／回應、版本握手、候選明細 |
| `jc-platform` | 平台抽象：`InputHost`、`HostEvent`、`Clock`、`Scheduler` |
| `jc-engine` | 引擎行程，載入資料、模型與 `settings.json`，處理協定並把按鍵交給 `jc-core::Session`；`--supervise` 常駐 |
| `jc-eval` | 題庫讀取、top-1／top-5／邊界／延遲指標、`Report` 與 gate |
| `jc-testdata` | 測試資料定位（`JC_TESTDATA`、`require_data!`） |
| `jc-cli` | 開發工具：`eval`、`convert`、`bench`、`tokenize-ja`、`data`，另有 `ja-eval` |
| `jc-jadic` | Mozc `dictionary_oss` 匯入，產出 pack v2 與 manifest |
| `jc-englishpack` | 英文詞表建置與打包 |
| `jc-settings` | 設定與模型包的資料模型、驗證、原子寫檔 |
| `jc-settings-ui` | GPUI Kit 設定程式，讀寫 `settings.json` |
| `jc-windows` | wrapper 端 client：named pipe 傳輸、握手、逾時與重連、pipe 名稱解析 |
| `jc-tsf` | Windows TSF 文字服務（x64 cdylib）：組字、候選窗、顯示屬性與按鍵 |
| `jc-tsf-smoke` | 免管理員的 TSF 註冊與載入冒煙測試 |
| `jc-linux` | 佔位，之後接 Unix socket client；目前回 `Unsupported` |
| `jc-mac` | 佔位，之後接 Unix socket／launchd；目前回 `Unsupported` |

## 建置

```sh
cargo build --workspace
cargo check --workspace --all-targets
```

模型推論使用 candle，讀模型目錄的 `model.safetensors` 或 `model.gguf`，
不需要外部動態庫。需要驗證 reranker 時加 `--rerank on`。

## 快速開始

```sh
# 啟動引擎（Unix domain socket；--supervise 常駐並自動重啟）
cargo run -p jc-engine --release -- --data /mnt/work/jc/jc-ime-data --socket /tmp/jc-engine.sock

# Windows：同一個參數走 named pipe；沒給 --socket 時用 jc-ime-<SID 尾段>
cargo run -p jc-engine --release -- --data <資料根> --supervise
```

## 安裝（Windows）

下載 `jc-ime-<version>-x64.msi` 後雙擊，步驟與反安裝見
[`docs/install.md`](docs/install.md)。安裝後用 `Win+Space` 選「jc 注音」，
設定用開始功能表的「jc 設定」。未簽章警告與其他限制見
[`docs/release.md`](docs/release.md)。

## 跑評測

```sh
# 注音 golden（45 例）
cargo run -p jc-cli --release -- eval zh-golden

# 日文題庫（預設 --neural auto；quality 提高覆蓋、off 回到統計）
cargo run -p jc-cli --release -- eval ja --corpus modern-500
cargo run -p jc-cli --release -- eval ja --corpus hard-cases --neural quality
cargo run -p jc-cli --release -- eval ja --corpus modern-500 --neural off

# 模糊輸入與 Mixed
cargo run -p jc-cli --release -- eval fuzzy
cargo run -p jc-cli --release -- eval mixed

# 單句轉換
cargo run -p jc-cli --release -- convert "わたしはほんをよむ。"

# 改讀真實注音資料包（預設仍是凍結 TSV）
cargo run -p jc-cli --release -- convert "ㄨㄛˇ ㄞˋ" --data /mnt/work/jc/jc-ime-data/build-zh-bigram
```

題庫與詞典不在 git 內，預設讀 `/mnt/work/jc/jc-ime-data`；可用 `JC_TESTDATA`
或 `--testdata` 改路徑。日文 pack 的解析順序是 `--pack` → `JC_JA_PACK` →
`packs/ja-lexicon-mozc-v2.pack` → v1 pack。Jinen 模型路徑可用
`JC_JINEN_GGUF`、`JC_JINEN_TOKENIZER` 覆寫，`JC_JINEN=off` 關閉。

## 產生日文詞典

```sh
cargo run -p jc-jadic --release -- import \
  --mozc /mnt/work/jc/jc-ime-data/mozc-3.35.6354 \
  --out /mnt/work/jc/jc-ime-data/packs
```

輸出 `ja-lexicon-mozc-v2.pack` 與 `.manifest.json`。目前實測為 1,220,427 筆
詞條、983,412 個 surfaces、2,672 個 context。

## 測試

```sh
cargo nextest run --workspace     # 單元＋整合
cargo test --workspace --doc      # nextest 不跑 doctest
```

需要外部資料的測試用 `jc_testdata::require_data!`，資料不存在時跳過而不是
失敗。開發期間的內圈用 `scripts/test-plan.sh --run`（只跑受影響範圍），
細節見 [`docs/port/testing.md`](docs/port/testing.md) 與
[`docs/blaze.md`](docs/blaze.md)。

## 資料

資料根目錄的預期內容（全部不進 git）：

```text
/mnt/work/jc/jc-ime-data/
  0c-testdata/      凍結題庫：golden、ja、fuzzy、mixed
  build-zh/         注音詞典 lexicon.bin（8,791,416 bytes）與 lm.bin
  build-zh-bigram/  加上新酷音 bigram 的版本（lm.bin 32,879,418 bytes）
  packs/            日文 pack v2 與 manifest
  mozc-3.35.6354/   Mozc OSS dictionary_oss
  opencc/           OpenCC 1.4.2 的 tw2s 字典
  models/           zh-select（Q8_0 33.9 MiB）、reranker、Jinen small、tokenizer
  english/、ja-src/ 英文詞表與日文 curated 來源
```

取得方式：題庫與注音資料來自前身 `0c-ime` 的 release／testdata；Mozc 由
上游 `dictionary_oss` 下載後用 `jc-jadic` 匯入；新酷音 bigram 用
`scripts/fetch_chewing_bigram.sh`（固定 commit 並驗 SHA-256）；模型由
Hugging Face 下載後記下 revision 與 SHA-256。每個檔案都要能對回大小與雜湊。

## 文件

- [`docs/status.md`](docs/status.md)：目前數字與缺口，每個里程碑更新這一頁。
- [`docs/install.md`](docs/install.md)：MSI 安裝、反安裝、檔案位置與已知限制。
- [`docs/release.md`](docs/release.md)：版本規則、發佈流程、資料包與 release 檢查清單。
- [`docs/CHANGELOG.md`](docs/CHANGELOG.md)：使用者看得到的變更，release 說明取自這裡。
- [`docs/proto.md`](docs/proto.md)：v3 協定規格（凍結）。
- [`docs/wrapper.md`](docs/wrapper.md)：wrapper 作者規格，接引擎不用讀 Rust。
- [`docs/models.md`](docs/models.md)：模型來源、授權、雜湊與散布規則。
- [`docs/architecture.md`](docs/architecture.md)：核心、引擎與 wrapper 的邊界。
- [`docs/port/`](docs/port/)：逐 task 的 Go → Rust 對照表。
- [`docs/history.md`](docs/history.md)：為什麼重寫、前身學到什麼、M0 以後的紀錄。
- [`docs/integration-log.md`](docs/integration-log.md)：合併順序、衝突與量測。

## 致謝

本專案的輸入法設計、注音與日文題庫、詞典資料與模型，來自前身專案
[0c-ime](https://github.com/0c-biggo/0c-ime)。`jc-ime` 是它的核心以 Rust
重寫的版本，功能範圍與品質基準都由它啟發；沒有 0c-ime 累積的題庫與資料，
這個專案無法在同樣時間內達到可驗證的品質。

日文詞典來自 Mozc OSS 3.35.6354（BSD-3-Clause 與 NAIST／IPAdic／ICOT／
沖繩語料授權）；注音 bigram 來自 libchewing-data（CC-BY-4.0）；簡繁轉換字典
來自 OpenCC 1.4.2（Apache-2.0）；Jinen v2 來自 `togatogah/jinen-v2-small.gguf`
（CC-BY-SA-4.0）。0c-ime 同為 AGPL-3.0 授權。

## 授權

AGPL-3.0-only，見 `LICENSE`。第三方詞典、模型與資料的散布帳本見
[`legal/README.md`](legal/README.md)，授權與署名見
[`legal/THIRD-PARTY-NOTICES.md`](legal/THIRD-PARTY-NOTICES.md)，完整聲明見
[`NOTICE`](NOTICE)。
