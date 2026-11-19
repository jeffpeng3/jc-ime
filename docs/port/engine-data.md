# engine-data：引擎資料載入

Worktree：`/mnt/work/jc/wt/engine-data`（branch `engine-data`）

## 目標

`jc-engine` 原本用 `Session::new(SessionConfig::default())`，沒有任何詞典，
所有按鍵都只能原樣輸出。這個 task 加上 `--data <dir>`，把注音與日文資料
接到 session，並讓 `jc-cli` 與引擎共用同一份載入程式碼。

## Go 對照

Go 參考實作沒有獨立的「資料載入」套件；載入散在 server 啟動流程。

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `cmd/0c-server/ja_language.go`（`newJaLanguage`、`loadJaDecoder`） | `crates/jc-engine/src/loader.rs`（`load_ja_decoder_with_pack`、`EngineData`） | rewritten；改成回傳 `Result`，缺檔帶路徑，不區分「未安裝」與「損壞」兩種 sentinel error |
| `cmd/0c-server/ja_models.go`（reranker／punct 載入） | `crates/jc-engine/src/loader.rs`（`LazyReranker`、`load_punctuator`） | rewritten；ONNX 改成第一次使用才載入 |
| `internal/jaload/lm.go`（日文 LM 開檔） | `crates/jc-core/src/ja/lexicon.rs`（pack 內的 LM） | n/a：Rust 的 v2 pack 已含詞典與連接矩陣，沒有獨立 LM 檔 |
| `core/lexicon/compact.go`（注音 compact lexicon） | `crates/jc-data/src/zh.rs`（`Bundle::open`） | n/a：二進位格式已由 `zh-data` 與 `bigram-data` 兩個 task 移植 |
| `cmd/0c-server/main_test.go`（啟動缺少資料的測試） | `crates/jc-engine/tests/engine_data.rs`（`missing_data_directory_reports_path`、`missing_data_flag_and_environment_fails`） | rewritten；改成行程層測試，斷言 stderr 含路徑 |

## 載入順序

`jc-engine --data <root>` 依序執行：

1. 檢查 `<root>` 是目錄，否則 `資料根目錄不存在：<root>`。
2. 注音詞典：`<root>/build-zh/lexicon.bin`；缺檔直接啟動失敗。
3. 注音 bigram：`<root>/build-zh-bigram/lm.bin`；存在就覆寫 `build-zh/lm.bin`。
4. 日文詞典：`packs/ja-lexicon-mozc-v2.pack`，找不到退回
   `packs/ja-lexicon-public-core.pack`，再退回 `ja-src/lexicon/words.tsv`。
   三個都沒有就啟動失敗。
5. 日文重排器：只記住 `<root>/models`（或 `JC_JA_MODELS`），第一次日文
   轉換才載入 ONNX。
6. 日文 tokenizer：`<root>/models/ja-tokenizer/public_core_ja.model`。
7. zh-select 與標點模型：只由 `jc-cli` 的明確旗標（`--select-model`、
   `--punct`）觸發；引擎目前不自動啟用，避免未校準的模型改變排序。

`--data` 未指定時讀 `JC_DATA`；兩者都沒有就啟動失敗並印出用法。

## 錯誤分級

| 資料 | 缺席或損壞 | 原因 |
|---|---|---|
| `build-zh/lexicon.bin` | 啟動失敗，訊息指出路徑 | 沒有詞典就完全無法轉注音 |
| `build-zh-bigram/lm.bin` | 警告一次，退回 `build-zh/lm.bin` | unigram 仍可轉換，只是少了 bigram |
| 日文 pack | 啟動失敗，訊息列出兩個候選路徑 | 沒有詞典就完全無法轉日文 |
| `models/ja-tokenizer/*.model` | 警告一次，`EngineData::ja_tokenizer` 為 `None` | 引擎轉換不依賴它 |
| reranker ONNX | 第一次使用時警告一次，退回統計排序 | 模型是加分項，不是必要資料 |
| zh-select／punct ONNX | 不自動載入；由 CLI 旗標決定 | 預設行為要與統計排序一致 |

## 共用載入的位置

載入程式碼放在 `crates/jc-engine/src/loader.rs`，`crates/jc-cli/src/data.rs`
只保留 CLI 路徑解析、個人化資料與 `CliError` 轉換。

沒有放進 `jc-data`：`jc-core` 已依賴 `jc-data`，而載入函式需要
`InMemoryLexicon`、`JaDecoder`、`SessionConfig` 等 `jc-core` 型別，反過來
依賴會形成循環。`jc-engine` 是任務書擁有的 crate，`jc-cli` 依賴它的 lib
即可共用，不必改 `jc-core` 的預設值。

## 測試

- `crates/jc-engine/src/loader.rs`：資料包載入、bigram、日文轉換、缺檔路徑。
- `crates/jc-engine/tests/engine_data.rs`：真的起引擎，送 `ji3` 取得
  `State` 首選「我」；`--mode ja` 送 `nihongo` 取得非空候選；缺 `--data`
  與 `JC_DATA` 時啟動失敗。
