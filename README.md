# jc-ime

Windows 優先的輸入法，核心全部以 Rust 實作，平台整合由獨立 wrapper 完成。

目前 M0–M2 完成，M3 的路由可用但 session 端只做了一部分：注音與日文的統計
轉換可以在 Linux 上跑完整評測，並有凍結題庫的實測數字。個人化學習、
Windows wrapper、設定／更新／安裝器尚未實作。逐項現況見
[`docs/status.md`](docs/status.md)。

## 現在的能力與限制

| 能力 | 現況 |
|---|---|
| 注音（大千鍵盤） | 聲調選填、候選翻頁、Tab 補全、簡繁轉換、Ctrl＋標點、mixed ASCII 推斷已接上；golden 41/45，剩 4 例依賴個人化學習 |
| 模糊音 | `select` 模式（`z-zh`、`c-ch`、`s-sh`、`en-eng`）；919 題 top-1 16.4%，注音 319 題 44.5% |
| 日文（羅馬字→かな→漢字） | Mozc pack v2 與 context ID 連接矩陣；`modern-500` top-1 86.0%、top-5 98.0% |
| Mixed 路由 | 注音／日文／英文分段、英文直通與前綴補完；候選清單與帳號／型號切法未移植 |
| 模型 | `jc-model` 可載入日文 reranker 與標點模型（feature `onnx`，預設關閉）；尚未接進 `jc-core`；注音模型未取得 |
| 引擎行程 | stdio 與 Unix domain socket；Windows named pipe 未實作 |
| 平台 | `jc-platform` 只定義 wrapper 契約，沒有任何實際 wrapper |

## Crate

| Crate | 內容 |
|---|---|
| `jc-core` | 鍵序列、注音音節與鍵盤、lattice／Viterbi、注音解碼、日文輸入與解碼、Mixed 路由、session、輸出 |
| `jc-data` | 日文 `0CJADIC` pack v1／v2 與注音 `lexicon.bin`／`lm.bin` 的零拷貝讀取、manifest 驗證 |
| `jc-model` | ONNX Runtime 接線：manifest、tokenizer、reranker、標點模型 |
| `jc-proto` | 引擎與 wrapper 的訊框、請求／回應、版本握手 |
| `jc-platform` | 平台抽象：`InputHost`、`HostEvent`、`Clock`、`Scheduler` |
| `jc-engine` | 引擎行程，處理協定並把按鍵交給 `jc-core::Session` |
| `jc-eval` | 題庫讀取、top-1／top-5／邊界／延遲指標、`Report` 與 gate |
| `jc-testdata` | 測試資料定位（`JC_TESTDATA`、`require_data!`） |
| `jc-cli` | 開發工具：`eval`、`convert`、`bench`，另有 `ja-eval` |
| `jc-jadic` | Mozc `dictionary_oss` 匯入，產出 pack v2 與 manifest |

## 建置

```sh
cargo build --workspace
cargo check --workspace --all-targets
```

模型推論預設不編譯，也不下載 ONNX Runtime。需要驗證 reranker 時用：

```sh
cargo run -p jc-cli --release -- eval ja --corpus modern-500 --rerank on
```

需要系統上有 ONNX Runtime 動態庫，或改用 `--backend candle` 讀 safetensors。

## 跑評測

```sh
# 注音 golden（45 例）
cargo run -p jc-cli --bin jc-cli -- eval zh-golden

# 日文題庫
cargo run -p jc-cli --release --bin ja-eval -- --corpus modern-500 --top 5
cargo run -p jc-cli --release --bin jc-cli -- eval ja --corpus hard-cases

# 模糊輸入
cargo run -p jc-cli --bin jc-cli -- eval fuzzy

# 單句轉換
cargo run -p jc-cli --bin jc-cli -- convert "わたしはほんをよむ。"

# 改讀真實注音資料包（預設仍是凍結 TSV）
cargo run -p jc-cli --release -- convert "ㄨㄛˇ ㄞˋ" --data /mnt/work/jc/jc-ime-data/build-zh-bigram
```

題庫與詞典不在 git 內，預設讀 `/mnt/work/jc/jc-ime-data`；可用 `JC_TESTDATA`
或 `--testdata` 改路徑。日文 pack 的解析順序是 `--pack` → `JC_JA_PACK` →
`packs/ja-lexicon-mozc-v2.pack` → v1 pack。

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
cargo test --workspace
```

需要外部資料的測試用 `jc_testdata::require_data!`，資料不存在時跳過而不是
失敗。開發期間只做 `cargo check`，測試由整合階段統一執行；細節見
[`docs/blaze.md`](docs/blaze.md)。

## 資料

資料根目錄的預期內容（全部不進 git）：

```text
/mnt/work/jc/jc-ime-data/
  0c-testdata/      凍結題庫：golden、ja、eval、fuzzy
  build-zh/         注音詞典 lexicon.bin（8,791,416 bytes）與 lm.bin
  build-zh-bigram/  加上新酷音 bigram 的版本（lm.bin 32,879,418 bytes）
  packs/            日文 pack v2 與 manifest
  mozc-3.35.6354/   Mozc OSS dictionary_oss
  opencc/           OpenCC 1.4.2 的 tw2s 字典
  models/           reranker-extra；標點模型保留但不再使用
  english/、ja-src/ 英文詞表與日文 curated 來源
```

取得方式：題庫與注音資料來自前身 `0c-ime` 的 release／testdata；Mozc 由
上游 `dictionary_oss` 下載後用 `jc-jadic` 匯入；新酷音 bigram 用
`scripts/fetch_chewing_bigram.sh`（固定 commit 並驗 SHA-256）；模型由
Hugging Face 下載後記下 revision 與 SHA-256。每個檔案都要能對回大小與雜湊。

## 文件

- [`docs/status.md`](docs/status.md)：目前數字與缺口，每個里程碑更新這一頁。
- [`docs/history.md`](docs/history.md)：為什麼重寫、前身學到什麼、M0–M2 的紀錄。
- [`docs/architecture.md`](docs/architecture.md)：核心、引擎與 wrapper 的邊界。
- [`docs/port/`](docs/port/)：逐 task 的 Go → Rust 對照表。
- [`docs/integration-log.md`](docs/integration-log.md)：合併順序、衝突與量測。

## 致謝

本專案的輸入法設計、注音與日文題庫、詞典資料與模型，來自前身專案
[0c-ime](https://github.com/0c-biggo/0c-ime)。`jc-ime` 是它的核心以 Rust
重寫的版本，功能範圍與品質基準都由它啟發；沒有 0c-ime 累積的題庫與資料，
這個專案無法在同樣時間內達到可驗證的品質。

日文詞典來自 Mozc OSS 3.35.6354（BSD-3-Clause 與 NAIST／IPAdic／ICOT／
沖繩語料授權）；注音 bigram 來自 libchewing-data（CC-BY-4.0）；簡繁轉換字典
來自 OpenCC 1.4.2（Apache-2.0）。0c-ime 同為 AGPL-3.0 授權。

## 授權

AGPL-3.0-only，見 `LICENSE`。第三方詞典、模型與資料的散布帳本見
[`legal/README.md`](legal/README.md)，授權與署名見
[`legal/THIRD-PARTY-NOTICES.md`](legal/THIRD-PARTY-NOTICES.md)，完整聲明見
[`NOTICE`](NOTICE)。
