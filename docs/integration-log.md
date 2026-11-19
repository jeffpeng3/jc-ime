# 整合紀錄

主 checkout：`/mnt/work/jc/jc-ime`，branch `main`。
驗證指令：`cargo check --workspace --all-targets`（依整合規則，測試由控制中樞統一決定何時跑）。

## 合併紀錄

| Branch | Commit | Merge commit | `cargo check --workspace --all-targets` | 時間 |
|---|---|---|---|---|
| `c0-contracts` | `9280e26` | `aa5e32f` | 通過 | 2026-11-19（先前） |
| `w1-proto-engine` | `569b523` | `f85d20b` | 通過 | 2026-11-19 20:28 |
| `w1-data-ja` | `39daa6d` | `3da7a73` | 通過（修正後） | 2026-11-19 20:31 |
| `w1-eval` | `776ac8c` | `dd00506` | 通過 | 2026-11-19 20:34 |
| `z1-zh-front` | `3068450` | `9780d51` | 通過 | 2026-11-19 20:36 |
| `z2-zh-mid` | `dcb1114` | `26be893` | 通過 | 2026-11-19 20:41 |
| `j1-ja-input` | `5137e08` | `0f86061` | 通過 | 2026-11-19 20:51 |
| `zh-layout-daqian-only` | `bb7f4fa` | `f0918da` | 通過 | 2026-11-19 20:51 |
| `z3-zh-back` | `a78385c` | `9803fe4` | 通過 | 2026-11-19 21:00 |
| `j2-ja-lexicon` | `e2fd3e7` | `dd09211` | 通過 | 2026-11-19 21:07 |
| `j3-ja-decode` | `c014d52` | `ff4c955` | 通過 | 2026-11-19 21:31 |
| `ja-pack-v2` | `86fe251` | `626d20f` | 通過 | 2026-11-19 21:53 |
| `m1-mixed` | `c669d5b` | `918088e` | 通過 | 2026-11-19 21:56 |
| `ja-import` | `e404fa1` | `ff13578` | 通過 | 2026-11-19 22:00 |
| `ja-context` | `4bc5df8` | `83c2974` | 通過 | 2026-11-19 22:11 |
| `m2-cli-session` | `26de34b` | `8b44bc1` | 通過 | 2026-11-19 22:27 |
| `fix-cli-ja` | `a8fbc7e` | `7b7e8bc` | 通過 | 2026-11-19 22:41 |
| `zh-golden` | `9068771` | `2e6156b` | 通過 | 2026-11-19 23:07 |
| `jc-platform` | `92f57b2` | `7f19601` | 通過 | 2026-11-19 23:10 |
| `bigram-data` | `3d96d6a` | `27ca97e` | 通過 | 2026-11-19 23:12 |
| `eval-ja-ambiguous` | `f8fb416` | `ea85686` | 通過 | 2026-11-19 23:12 |
| `zh-golden-rest` | `59b67ce` | `09abcd6` | 通過 | 2026-11-19 23:16 |
| `jc-model` | `63d6049` | `8d4015c` | 通過 | 2026-11-19 23:18 |
| `eval-fuzzy` | `9ba1fe8` | `35c769f` | 通過（解衝突） | 2026-11-19 23:21 |
| `zh-data` | `333abac` | `b2f6d80` | 通過 | 2026-11-19 23:23 |
| `zh-models` | `f188bf7` | `5d85e54` | 通過 | 2026-11-19 23:27 |
| `docs-milestone` | `5f87d03` | `52ac22e` | 通過 | 2026-11-19 23:29 |
| `ci-clippy` | `34055f4` | `34ecac4` | 通過 | 2026-11-19 23:30 |
| `zh-symbols` | `f6db514` | `b6de2a9` | 通過 | 2026-11-19 23:30 |
| `english-pack` | `fd5d8cf` | `15055d2` | 通過（解衝突） | 2026-11-19 23:40 |
| `fix-mixed-clippy` | `c57992c` | `77f2788` | 通過（fmt＋clippy） | 2026-11-19 23:47 |
| `ci-lint` | `2732fb3` | `e08a59a` | 通過（fmt＋clippy） | 2026-11-19 23:47 |
| `fix-windows-cfg` | `0131ebe` | `cf3beba` | 通過（check＋clippy） | 2026-11-20 00:03 |
| `mixed-e2e` | `7c94971` | `b7ee298` | 通過（check＋fmt＋clippy） | 2026-11-20 00:16 |
| `zh-bigram` | `61b1ed5` | `b2d244f` | check 通過；clippy 退步 | 2026-11-20 00:20 |
| `zh-bigram`（clippy 修正） | `3ecd2a7` | `807c41d` | 通過（fmt＋clippy） | 2026-11-20 00:23 |
| `model-wire-zh` | `39d9bda` | `67d16e8` | 通過（fmt＋clippy） | 2026-11-20 00:39 |
| `model-wire-ja` | `f8d9d48` | `b147c61` | 通過（fmt＋clippy） | 2026-11-20 00:40 |
| `zh-bigram`（nbest 修正） | `0ec571b` | `8180133` | 通過 | 2026-11-20 00:44 |
| `fix-tests` | `3bda5a0` | `f660e30` | 通過 | 2026-11-20 00:44 |
| `mixed-e2e`（片語合併） | `6c72b5e` | `301c423` | 通過 | 2026-11-20 00:44 |
| `rerank-fix` | `5954b89` | `fadc139` | 通過（fmt＋clippy＋test） | 2026-11-20 01:04 |
| `core-misc` | `bc39909` | `2148cd6` | 通過 | 2026-11-20 01:10 |
| `ja-personal` | `841b35f` | `cf0c235` | 通過 | 2026-11-20 01:12 |
| `punct-wire` | `b79183c` | `117e2b9` | check／fmt／clippy 通過；3 個 test 失敗 | 2026-11-20 01:15 |
| `ja-live-rewrite` | `d1a3dca` | `7b7047a` | 通過（check／fmt／clippy＋針對性 test） | 2026-11-20 01:16 |
| `punct-wire`（修測試） | `d1ae090` | `d53703f` | 通過 | 2026-11-20 01:17 |
| `ja-personal`（修排序） | `0a376e5` | `88c482d` | 通過 | 2026-11-20 01:18 |
| `ja-live-rewrite`（修かな鎖定） | `7516c29` | `d83cf5b` | 通過（jc-core 全 crate test） | 2026-11-20 01:27 |
| `personal-zh` | `8b43057` | `55f2067` | fmt／test／golden 通過；clippy 失敗 | 2026-11-20 01:29 |
| `rerank-tokenizer` | `a9aebdc` | `bcb1515` | fmt／clippy 通過；4 個 test 失敗 | 2026-11-20 01:33 |
| `personal-zh`（清 clippy） | `6faec7f` | `e5af3b8` | fmt／clippy／golden 通過；test 剩 4 個既有失敗 | 2026-11-20 01:35 |
| `rerank-tokenizer`（SP 對齊） | `0eb7496` | `b07d098` | 通過（fmt／clippy／test） | 2026-11-20 01:57 |
| `settings-core` | `aceda9f` | `52cb1b9` | check／fmt／clippy 通過；jc-settings 4 個 test 失敗 | 2026-11-20 02:36 |
| `onnx-default` | `6942b66` | `79724ee` | check／fmt 通過 | 2026-11-20 02:44 |
| `zh-select-calibrate` | `b677022` | `9a04a60` | check 通過；clippy 出現 `onnx` cfg 警告 | 2026-11-20 02:46 |
| `english-suggest` | `b93633a` | `359d425` | check 通過；clippy／test 失敗 | 2026-11-20 02:50 |
| `zh-select-calibrate`（診斷） | `9f84483` | `eabae70` | check 通過 | 2026-11-20 03:04 |
| `english-suggest`（混合修正） | `a9e8fb0` | `b9459ac` | test／clippy 剩他人失敗 | 2026-11-20 03:10 |
| `legal-notices` | `d1be772` | `823eab9` | fmt 通過；clippy 剩 zh-calibrate | 2026-11-20 03:17 |
| `zh-select-calibrate`（移除 cfg） | `741e59a` | `5fcf8ca` | 通過（clippy＋jc-cli test） | 2026-11-20 03:19 |
| `settings-core`（修測試） | `87cfdf1` | `89a4f6f` | 通過（fmt＋clippy＋test） | 2026-11-20 03:28 |
| `rerank-wordlevel` | `277bd8e` | `9476f51` | fmt／clippy 通過；5 個 test 失敗 | 2026-11-20 03:35 |
| `rerank-wordlevel`（修測試） | `2c25f3d` | `2e649f5` | 通過（fmt＋clippy＋test） | 2026-11-20 04:00 |
| `defaults-policy` | `e116f5f` | `175f417` | 通過（fmt／clippy／test／兩個 eval） | 2026-11-20 04:08 |
| `engine-data` | `24b876b` | `8aa5ab1` | clippy 通過；jc-engine 4 個 test 失敗 | 2026-11-20 04:13 |
| `ja-ime-survey` | `4eac1ef` | `c9b8998` | check 通過 | 2026-11-20 04:16 |
| `engine-data`（zh-select） | `8dc6235` | `d2aba42` | 通過（fmt／clippy／test） | 2026-11-20 04:28 |
| `ja-personal-wire` | `ea161f8` | `b32beb2` | 通過（fmt／clippy／test） | 2026-11-20 04:28 |
| `ajimee-eval` | `8663bea` | `7885302` | 通過（fmt／clippy／全量 test） | 2026-11-20 04:31 |
| `ja-models-fetch` | `0f76eec` | `44e8a5f` | check 通過 | 2026-11-20 04:31 |
| `ja-cost-diag` | `fca198b` | `058f44b` | test 通過；clippy 1 個錯 | 2026-11-20 04:42 |

## 衝突與修正

- `w1-proto-engine`：只有 `Cargo.lock` 內容衝突（`jc-testdata`、`jiff`／`jiff-core`／`jiff-static`／`log`、`syn` 版本）。
  以 main 的 `Cargo.lock` 為底，交給 cargo 重新解析，`Cargo.lock` 增 231 行、刪 2 行，無需手改其他檔案。
- 沒有編譯錯誤要修，沒有改到任何 crate 的原始碼。

- `w1-data-ja`（合併 `3da7a73`，修正 `3a3faca`）：
  - 衝突：只有 `Cargo.lock`，以 main 版為底讓 cargo 重新解析。
  - `crates/jc-data/src/lib.rs`：補 `pub mod ja;`（c0 沒宣告）。
  - `crates/jc-data/Cargo.toml`：新增 dev-dependency `jc-testdata = { path = "../jc-testdata" }`。
  - `crates/jc-data/src/ja/tests.rs`：移除本地 `testdata_root`／`testdata_path`／`require_data!`，改用 `jc_testdata::require_data!`。
  - 已對 `jc-data` 跑 `cargo fmt`。

- `w1-eval`（合併 `dd00506`）：無衝突，`Cargo.lock` 由 git 自動合併；沒有編譯錯誤要修，未改任何程式碼。

- `z1-zh-front`（合併 `9780d51`）：
  - 衝突：只有 `Cargo.lock`，以 main 版為底讓 cargo 重新解析。
  - 契約變更：`crates/jc-core/src/segment.rs` 的 `Script` 新增 `Han`、`Mixed` 兩個變體，並在 `zh` 模組加入對應分類。其他 crate 沒有 exhaustive match 因此不需同步改動。
  - 沒有編譯錯誤要修，未改任何實作（`jc-core` 的改動全部來自 branch）。

- `z2-zh-mid`（合併 `26be893`）：無衝突（base `aa5e32f`，與 z1 的 `zh/`、`segment.rs`、`reading.rs` 不重疊）。
  改動集中在 `crates/jc-core/src/score.rs` 與 `crates/jc-core/src/lattice/`（`build`、`candidate`、`lexicon`、`viterbi`、`mod`）。
  沒有編譯錯誤要修，`Cargo.lock` 無變動，未改任何實作。

- `j1-ja-input`（合併 `0f86061`）：無衝突。新增 `crates/jc-core/src/ja/input.rs`、`ja/rewrite.rs`，
  `ja/mod.rs` 只加 `pub mod`，`Cargo.lock` 自動合併。沒有編譯錯誤要修，未改任何實作。

- `zh-layout-daqian-only`（合併 `f0918da`）：無衝突。`zh/keyboard.rs` 移除許氏與倚天排列，`LayoutKind` 只剩 `Standard`；
  同步移除 `zh/mod.rs` 的相關 re-export 並更新 `docs/port/z1-zh-front.md`。沒有編譯錯誤要修，未改任何實作。

- `z3-zh-back`（合併 `9803fe4`）：無衝突。新增 `crates/jc-core/src/decode/`（`across`、`bias`、`explain`、`nbest`、`repeat`、`untoned`、`mod`）
  與 `output/mod.rs`，與 `ja/` 不重疊。沒有編譯錯誤要修，未改任何實作。

- `j2-ja-lexicon`（合併 `dd09211`）：無衝突。新增 `crates/jc-core/src/ja/lexicon.rs`、`ja/lm.rs`，
  `ja/mod.rs` 加兩個 `pub mod`，`Cargo.lock`／`jc-core/Cargo.toml` 同步補依賴。沒有編譯錯誤要修，未改任何實作。

- `j3-ja-decode`（合併 `ff4c955`）：無衝突。新增 `crates/jc-core/src/ja/decode.rs`（2058 行）、
  `ja/mod.rs` 加一行 `pub mod`，`jc-core/Cargo.toml` 加 `serde_json` dev-dependency。沒有編譯錯誤要修，未改任何實作。

- `ja-pack-v2`（合併 `626d20f`）：無衝突。`crates/jc-data/src/ja/` 的 `pack.rs`、`manifest.rs`、`error.rs`、`mod.rs`、`tests.rs`
  支援 v2 與連接矩陣；v1 讀取路徑保留。沒有編譯錯誤要修，未改任何實作。

- `m1-mixed`（合併 `918088e`）：無衝突，兩個 commit（`119d281` 實作、`c669d5b` 宣告模組）。
  新增 `crates/jc-core/src/mixed.rs`（2757 行），`lib.rs` 加 `pub mod mixed;`。沒有編譯錯誤要修，未改任何實作。

- `ja-import`（合併 `ff13578`）：無衝突。新增 crate `crates/jc-jadic`（`error`、`import`、`mozc`、`pack`、`reading`、`lib`、`main`，2028 行），
  workspace `Cargo.toml` 加 member，`Cargo.lock` 自動合併。沒有編譯錯誤要修，未改任何實作。
  產出檔 `/mnt/work/jc/jc-ime-data/packs/ja-lexicon-mozc-v2.pack`（81.5M）與 manifest 在 git 外，不隨 repo 散布。

- `ja-context`（合併 `83c2974`）：無衝突。`ja/decode.rs`、`ja/lexicon.rs` 接上 pack v2 的 context 矩陣
  （`JaConnectionCost` 直接查 connection section，BOS／EOS 用 context ID 0，缺 ID 退回 POS）；
  新增 `crates/jc-cli/src/bin/ja-eval.rs`，`jc-cli/Cargo.toml` 與 `Cargo.lock` 同步補依賴。沒有編譯錯誤要修，未改任何實作。

  量測（`ja-eval`，pack `ja-lexicon-mozc-v2.pack`，`--top 5`）：

  | Corpus | n | top-1 | top-5 | boundary exact | boundary F1 | p50 ms |
  |---|---:|---:|---:|---:|---:|---:|
  | modern-500 | 500 | 86.0% | 98.0% | 88.2% | 0.9017 | 3.216 |
  | hard-cases | 35 | 54.3% | 97.1% | 71.4% | 0.8600 | 0.747 |

  對照前身 `data/ja/README.md`：modern-500 四項完全相同；hard-cases 的 top-1／top-5 相同，
  boundary exact 差 2.8 個百分點（71.4% vs 68.6%，本版較好），boundary F1 為 0.8600 對 0.863。
  負向對照：同支 bin 換 v1 pack，modern-500 top-1 為 0.0%，證明矩陣路徑確實生效。

- `m2-cli-session`（合併 `8b44bc1`）：無衝突。`jc-cli` 新增 `cli`、`converter`、`data`、`eval_cmd`、`bench`；
  `jc-core` 的 `session.rs`、`key.rs` 補工作階段 API；`jc-engine`、`jc-proto` 同步調整並更新測試檔；
  `Cargo.lock` 自動合併。沒有編譯錯誤要修，未改任何實作。

- `fix-cli-ja`（合併 `7b7e8bc`）：無衝突。`jc-cli` 的 `data.rs`、`eval_cmd.rs`、`cli.rs` 改為
  `--pack` → `JC_JA_PACK` → `packs/ja-lexicon-mozc-v2.pack` → v1 pack 的解析順序，v2 走 `with_context_cost`；
  新增 `tests/eval_ja_parity.rs`。沒有編譯錯誤要修，未改任何實作。

  對齊結果：`jc-cli eval ja` 由 23.2% 升到 **86.0%**，與 `ja-eval` 同為 86.0%（modern-500、`--top 5`，兩者 top-5 均 98.0%）。
  `hard-cases` 前 5 題兩邊同為 `n=5`、top1 0.0%、top5 100.0%、boundary exact 100.0%。

- `zh-golden`（合併 `2e6156b`）：git 自動合併 `jc-cli/src/data.rs`、`eval_cmd.rs`，無衝突。
  `session.rs` 接上 toneless、翻頁、Tab 補全、toggle、Ctrl+,、標點替身、rerank、fuzzy select；
  `key.rs`、`decode/mod.rs` 同步調整。沒有編譯錯誤要修，未改任何實作。

  實測（`cargo run -p jc-cli --bin jc-cli -- eval zh-golden`）：**通過 35/45**，不是 41/45。
  剩 10 例：4 例學習依賴（`learning-select-preference`、`learning-forget-request`、
  `undo-learned-word`、`undo-custom-word`），6 例待 `zhconv` 與 mixed ASCII 按鍵推斷
  （`simplified-commit`、`simplified-sentence`、`mixed-chinese-english-chinese`、
  `mixed-english-prefix`、`english-completion`、`cjk-latin-space`）。分類與回報一致，總數差 6。

- `jc-platform`（合併 `7f19601`）：無衝突。新增 `clock.rs`、`headless.rs`、`host.rs`、`tests/contract.rs`，
  改寫 `lib.rs` 的宿主契約；`Cargo.lock` 自動合併。沒有編譯錯誤要修，未改任何實作。

- `bigram-data`（合併 `27ca97e`）：無衝突，只新增 `docs/port/bigram-data.md` 與 `scripts/fetch_chewing_bigram.sh`，沒有 Rust 改動。
  `cargo check` 全部 cached 通過。新酷音 bigram 已重建到 `/mnt/work/jc/jc-ime-data/build-zh-bigram/`
  （`lexicon.bin` 8.4M、`lm.bin` 31.4M、`manifest.json`、`NOTICE.txt`、`report.txt`），資料在 git 外。

- `eval-ja-ambiguous`（合併 `ea85686`）：無衝突。只改 `crates/jc-cli/src/bin/ja-eval.rs`（新增 `--file`／`--split`）
  與 `docs/port/eval-ja-ambiguous.md`。沒有編譯錯誤要修，未改任何實作。

  量測（`ja-lexicon-ambiguous`，1200 題）：v2 Mozc pack holdout top-1 **30.2%**（121/400）、top-5 58.5%；
  public-core pack holdout top-1 **52.2%**、top-5 93.8%。差異來自題庫原始建構用的是 public-core pack
  （候選成本取自 public-core，reference 是它的最低成本詞面），不是 decoder 問題，所以 `jc-core` 未改。

- `zh-golden-rest`（合併 `09abcd6`）：無衝突。新增 `crates/jc-core/src/zhconv.rs`，`mixed.rs`、`session.rs`、
  `output/mod.rs` 接上 mixed ASCII 推斷與簡體轉換，`jc-cli` 的 `data.rs`／`eval_cmd.rs` 接字典載入。
  沒有編譯錯誤要修，未改任何實作。

  golden 實測（`jc-cli eval zh-golden`）：**通過 41/45**（對照上一輪 35/45）。
  剩 4 例全是學習依賴：`learning-select-preference`、`learning-forget-request`、
  `undo-learned-word`、`undo-custom-word`，期望值未動。
  zhconv 字典在 `/mnt/work/jc/jc-ime-data/opencc/`（OpenCC 1.4.2，Apache-2.0），git 外。

- `jc-model`（合併 `8d4015c`）：`Cargo.lock` 自動合併。新增 `manifest.rs`、`layout.rs`、`tokenizer.rs`、`rerank.rs`、
  `punctuation_model.rs`、`runtime.rs`、`role.rs`、`error.rs`、`punct.rs`、`bin/probe.rs`；`ort` 走 optional feature `onnx`，
  預設建置不拉入。沒有編譯錯誤要修，未改任何實作。

  效能（`bin/probe` 回報）：reranker **32 ms**、punctuation **2 ms**（單次推論延遲）。
  行為差異：`vocab.json` 沒有 piece log-probability，tokenizer 改用**最長前綴匹配 + byte fallback**，
  不做 unigram Viterbi；已寫在 `docs/port/jc-model.md`。

- `eval-fuzzy`（合併 `35c769f`）：新增 `crates/jc-core/src/zh/fuzzy.rs`、`crates/jc-eval/src/fuzzy.rs`。
  `crates/jc-cli/src/eval_cmd.rs` 有兩處衝突，已按「兩邊意圖都保留」處理：
  匯入改成 union（保留 main 的 `RerankOptions`／`SessionConfig`，加上 branch 的 `Lattice`、`zh::fuzzy`）；
  main 的 `apply_options`／`fuzzy_pair` 與 branch 的 `run_fuzzy_zh`／`classify_zh_miss`／`run_fuzzy_ja`／
  `set_fuzzy_empty`／`file_sha256` 全數保留。沒有改任何實作邏輯。

  量測（`jc-cli eval fuzzy`）：全部 **919** 題；注音 319 題 top-1 **44.5%**（前身／舊版 40.4%），
  提升來自 near-sound 81.4% → 100%。日文 600 題 top-1 1.5%，降級主因是 unhandled 182 題。

- `zh-data`（合併 `b2f6d80`）：git 自動合併 `Cargo.lock`、`jc-cli/src/data.rs`，無衝突。
  新增 `crates/jc-data/src/zh/`（`format`、`lexicon`、`lm`、`error`、`mod`、`tests`、`testutil`），
  `jc-data/lib.rs` 加 `pub mod zh;`，`jc-cli` 接 `--data` 載入。沒有編譯錯誤要修，未改任何實作。

  bigram：`build-zh-bigram` 的 `lm.bin` 有 **7,028,326** 筆 bigram（成本 238021 詞、31.4 MB）；
  `jc-core` 的注音解碼器用詞性連接成本，**解碼未使用 bigram**，兩包 5-best 相同，
  需要時由呼叫端直接呼叫 `jc_data::zh::Lm::bi`。

- `zh-models`（合併 `5d85e54`）：無衝突，只新增 `docs/port/zh-models.md` 與 `scripts/fetch_zh_models.sh`，沒有 Rust 改動。
  兩個注音模型已取得（`zh-select`：`Funmula/0c-sequence-d512`、`zh-punct`：`Funmula/0c-punct-d512`，皆 MIT，revision 已固定）。
  `vocab.json` 是 **zuin-phonetic-v1**（27,857 字、416 注音 base），不是 sentencepiece，
  `jc-model` 的 tokenizer 目前不支援，實際注音推論接線待後續 task。

- `docs-milestone`（合併 `52ac22e`）：無衝突，只有文件改動：`README.md`、`docs/architecture.md`、`docs/history.md` 更新到 M2，
  新增 `docs/status.md`。`cargo check` 全部 cached 通過，確認沒動到程式。
  `README.md` 的 `## 致謝` 段落仍在（文字重排換行，內容未變）。

- `ci-clippy`（合併 `34ecac4`）：無衝突，29 個檔全部是 clippy／fmt 修正（245 增、158 刪），另有 `docs/port/ci-clippy.md`。
  `cargo check --workspace --all-targets` 通過，0 warning。
- `zh-symbols`（合併 `b6de2a9`）：`jc-core/src/session.rs` 自動合併，無衝突。新增 `symbols.rs`、`emoji.rs`、`punct.rs`，
  `session.rs` 接上符號、表情、全形數字與補標點，`lib.rs` 加模組宣告。`cargo check --workspace --all-targets` 通過，0 warning。

- `english-pack`（合併 `15055d2`）：新增 crate `crates/jc-englishpack`、`scripts/build_english_pack.sh`，workspace `Cargo.toml` 加 member。
  `crates/jc-core/src/mixed.rs` 的 `suggestion()` 有衝突：main 用 `prototype().words()` 加 `one_edit_apart` 掃原型詞表，
  branch 改用真詞表 `dict.corrections(raw, 1)`。採 branch 版讓載入真實詞表的意圖成真，`one_edit_apart`
  隨 branch 一起移除，沒有留下未使用的函式。沒有改其他實作邏輯。

  資料：`words.bin` 已建置在 `/mnt/work/jc/jc-ime-data/english/`（82,015 詞、1,522,028 bytes，另有 `manifest.json`、`NOTICE.txt`）。
  branch 回報的 `strace` 顯示 `manifest.json` 與 `words.bin` 都被開啟，`query` 來源是 `english/words.bin`，可證實不是 `prototype()`。

- `fix-mixed-clippy`（合併 `77f2788`，含 `f9ecb66`、`c57992c`）：無衝突。修掉兩個 `neg_cmp_op_on_partial_ord`：
  `jc-core/src/mixed.rs` 的 `near_common`、`jc-englishpack/src/compile.rs` 的權重檢查，都改成先算 `valid_*` 布林再判斷。
- `ci-lint`（合併 `e08a59a`）：無衝突。只改 `jc-core/src/emoji.rs` 的 `needless_range_loop`（`for i in 0..MAX_RECENT + 10` 改 iterator）
  與 `docs/port/ci-lint.md`。

  驗證結果：`cargo fmt --all -- --check` exit 0；`cargo clippy --workspace --all-targets -- -D warnings` **exit 0**；
  `cargo check --workspace --all-targets` 也通過（0 warning）。

- `fix-windows-cfg`（合併 `cf3beba`）：無衝突。`jc-engine/src/main.rs` 的 unix socket 路徑加 `#[cfg(unix)]`，
  非 unix 提供同名 `serve_socket` 回 `named pipe 尚未實作…`，`--socket` 說明改用平台常數；
  `jc-engine/tests/socket.rs` 加 `#![cfg(unix)]`。

  驗證結果：`cargo check --workspace --all-targets` exit 0、`cargo clippy --workspace --all-targets -- -D warnings` exit 0。
  Windows（msvc）編譯由 CI 驗證，本機 Linux 不做交叉編譯。

- `mixed-e2e`（合併 `b7ee298`）：無衝突。`jc-core/src/mixed.rs`、`jc-cli` 的 `cli.rs`／`eval_cmd.rs` 補 Mixed 整句轉換與表面比對，
  另有 `docs/port/mixed-e2e.md`。

  驗證結果：`cargo check --workspace --all-targets` exit 0、`cargo fmt --all -- --check` exit 0、
  `cargo clippy --workspace --all-targets -- -D warnings` exit 0（沒有跑 `cargo test`）。

  Mixed 對照前身 6 個案例：**4 例完全一致**，2 例（`scenario-a-primary-ja`、`scenario-b-primary-zh`）
  只差日文詞面 `私わ`（前身）與 `私羽`（Rust），其餘輸出相同；原因是 Mozc pack 的詞面成本差異，不是路由錯誤。

- `zh-bigram`（合併 `b2d244f`，含 `2a9eb10`、`61b1ed5`）：`jc-cli/src/cli.rs` 有 4 處衝突，照兩邊意圖都保留解：
  加回 branch 的 `lm`／`mu`／`stats` 宣告與參數解析、`ja`／`fuzzy` 的 `reject_zh_flags` 檢查，同時保留 main 的 `eval mixed`
  指令與 usage 行；`convert` 的 usage 採 branch 版（多了 `--lm`／`--mu`／`--stats`）。

  bigram 已生效（實測）：
  - `convert "ㄨㄛˇ ㄒㄧㄤˇ ㄔ" --lm build-zh-bigram/lm.bin --mu 0` → 20540；同指令 μ 預設 0.7 → **19238**。
  - `eval zh-golden --lm build-zh-bigram/lm.bin --stats` → **41/45**，`bigram queries=22747 hits=9743`。
  - `eval zh-golden`（不帶 `--lm`）也是 41/45。41/45 不變的原因是凍結詞庫每個讀音幾乎只有一個候選，
    bigram 改得動分數但翻不動排名。
  - 注意：帶 `--lm` 時失敗集合與預設不同（`candidates-digit`、`rerank-off` 退步，兩個 `learning-*` 反而通過），
    通過數同為 41。

  `cargo check --workspace --all-targets` exit 0、`cargo fmt --all -- --check` exit 0。
  **`cargo clippy --workspace --all-targets -- -D warnings` 退步（exit 101）**，兩個新錯都來自本 branch，需回 zh-bigram 修：
  - `crates/jc-cli/src/converter.rs:222`：`clippy::too_many_arguments`（`convert` 9 個參數，上限 7）。
  - `crates/jc-core/src/score.rs:573`：`clippy::field_reassign_with_default`（測試內 `let mut ctx = ScoreContext::default(); ctx.bigram_mu = 0.0;`）。

- `zh-bigram` 第二次合併（`807c41d`，含 `3ecd2a7`）：`jc-cli/src/cli.rs` 自動合併，無衝突。
  `converter.rs` 把 `convert` 的參數收成結構，`score.rs` 測試改用結構初始化，兩個 clippy 錯消失。
  `cargo fmt --all -- --check` exit 0、`cargo clippy --workspace --all-targets -- -D warnings` **exit 0**、
  `cargo check --workspace --all-targets` 也通過。`eval zh-golden --lm … --stats` 仍為 41/45、queries=22747 hits=9743。

- `model-wire-zh`（合併 `67d16e8`，含 `798d55a`、`8b15704`、`e42408a`、`39d9bda`）：無衝突。
  `zh_golden` 改成 `GoldenOptions` 收斂參數，解掉上一輪 `too_many_arguments` 的死結；
  `jc-cli` 加 `--select-model on|off`（需要 `--features onnx`）、`jc-core/src/zh/select.rs` 新增選字模型前處理。
  `cargo fmt --all -- --check` exit 0、`cargo clippy --workspace --all-targets -- -D warnings` **exit 0**、
  `cargo check --workspace --all-targets` 通過；依指示未跑 test。

  量測（`docs/port/model-wire-zh.md`）：`eval zh-golden` 開與關都是 **41/45**；開啟時推論 100 次、
  p50 **11.87 ms**、p95 19.42 ms。前處理與前身 Python 參考實作逐位一致（誤差 0：vocab 映射、序列長度、
  `[UNK]`、遮罩、`proj_pos`、`target_node`／`target_ids`）。結論是此題庫上無改善，`--select-model` 預設 off。

- `model-wire-ja`（合併 `b147c61`，含 `f8d9d48` 與 Send＋Sync 修正）：`jc-cli/Cargo.toml`、`jc-core/Cargo.toml`、
  `jc-cli/src/cli.rs` 有衝突，全部照兩邊意圖都保留解：
  - `jc-core` 的 `onnx` feature 取聯集 `["dep:jc-model", "dep:ort", "jc-model/onnx"]`，`ort` optional 依賴保留。
  - `jc-cli` 的 feature 註解合併成一句，涵蓋注音選字模型與日文 reranker。
  - `cli.rs` 同時保留 `--select-model on|off` 與 `--rerank on|off` 的解析、`EvalJa.rerank`、usage 兩行。
  `JaDecoder` 的 hook trait 都要求 `Send + Sync`，上一輪的 `arc_with_non_send_sync` 已消失。

  驗證：`cargo fmt --all -- --check` exit 0、`cargo clippy --workspace --all-targets -- -D warnings` **exit 0**、
  `cargo check --workspace --all-targets` 通過；依指示跳過 test（jc-cli 的 2 個 `eval_ja_parity` 等 `fix-tests`）。

  量測（`eval ja --corpus modern-500`）：

  | 設定 | n | top-1 | top-5 | boundary exact | p50 | p95 |
  |---|---:|---:|---:|---:|---:|---:|
  | `--rerank off` | 500 | 86.0% | 98.0% | 88.2% | 3.26 ms | 6.75 ms |
  | `--rerank on` | 500 | 85.6% | 98.0% | 87.8% | 77.87 ms | 88.10 ms |

  rerank 統計：calls 500、scored 500、changed 2、timeout 0、error 0；推論 p50 74.73 ms、p95 77.47 ms。
  兩個疑點：左文全空（題庫一次轉換整句，`JaContext::left` 沒有前文，模型只看到候選）；
  candidate 位置 62 與訓練 checkpoint 的 122 不一致（匯出端限制，不在本 task owned paths）。
  因此 `--rerank` 保持 opt-in、預設 off。

- 依控制中樞指定順序重跑：`zh-bigram`（`8180133`，含 `0ec571b` 修 nbest 測試的未知詞成本與連線方向）→
  `fix-tests`（`f660e30`）→ `mixed-e2e`（`301c423`，含 `6c72b5e` 片語合併）。三條都無衝突到產品邏輯：
  - `fix-tests` 與 zh-bigram 在 `jc-core/src/lattice/viterbi.rs` 衝突，採 zh-bigram 的連線方向修正
    （`set(2, 1, 100)`，`fix-tests` 的 `set(1, 2, 100)` 已被方向修正取代）。
  - `jc-cli/src/eval_cmd.rs` 的 `Ctrl+,` 斷言取兩邊聯集（保留 `<` 合法與 `Ctrl+Shift+,` 錯誤訊息）。
  - `mixed-e2e` 與 fix-tests 在 `mixed.rs` 只衝突註解，取較完整的位移說明。

  最終驗證：`cargo test --workspace --no-fail-fast` **exit 0**，33 個 target 全部 `0 failed`（含 `jc-core --lib`）；
  `cargo fmt --all -- --check` exit 0、`cargo clippy --workspace --all-targets -- -D warnings` exit 0。
  原以為會剩 `decode::nbest::tests::nbest_uses_bigram_context_of_previous_word` 一個失敗，實際隨 zh-bigram 的修正一起轉綠。

  `fix-tests` 的修復量：34 個失敗測試、9 個 target；4 個實作 bug（`InMemoryLexicon::prefix`、`Key.ctrl` 序列化、
  `Engine` shutdown 狀態、`Tokenizer::encode` 字串結尾），11 個測試期望值修正，其餘是測試機制修正。

- `rerank-fix`（合併 `fadc139`）：`jc-cli/src/cli.rs` 有 8 處衝突，照兩邊意圖都保留解：
  `LeftMode`／`--left off|rolling`／`reject_ja_flags` 與 main 的 `--select-model`、`GoldenOptions`、
  `--data`／`--lm`／`--mu`／`--stats`、`eval mixed` 全部並存；usage 兩行合併。

  驗證：`cargo fmt --all -- --check` exit 0、`cargo clippy --workspace --all-targets -- -D warnings` exit 0、
  `cargo test --workspace --no-fail-fast` **exit 0**（33 個 target、0 failed）。

  結論：
  - 位置布局**本來就正確**：shipped 圖的 pooling 是常數 `62`，`position_embedding` 索引從 61 起，
    所以 slot 62 的候選拿到絕對位置 **122**，與訓練 checkpoint 一致；`model-wire-ja` 寫的「0..123」是誤讀。
  - 左文 oracle 也只有 **20.5%**（空左文 21.0%、前一題 surface 24.0%、BOS 27.5%），模型在這批候選上排序力不足。
  - 融合權重照前身修正後，`--rerank on` 的 top-1 反而掉到 **40.8%**（統計基準 86.0%，滾動左文 57.8%）。
  - 原因：訓練候選是單一 SentencePiece token，評測餵整句 surface，**候選粒度不一致**；且 tokenizer 只有詞表、
    **缺 piece 分數**，只能用最長前綴匹配近似。`--rerank` 維持 opt-in、預設 off。

- 三條依序合併，全部無衝突：`core-misc`（`2148cd6`，新增 `vocab`、`suggest`、`inputstyle`、`lm`，3424 行）、
  `ja-personal`（`cf0c235`，新增 `ja/personal.rs`，2235 行）、`punct-wire`（`117e2b9`，新增 `jc-model` 的
  `punct_plan`／`punct_zh` 與 `jc-core/src/output/punct.rs`）。每條合完都跑過 `cargo check --workspace --all-targets`，全部 exit 0。

  最終 gate：`cargo fmt --all -- --check` exit 0、`cargo clippy --workspace --all-targets -- -D warnings` exit 0、
  `cargo test --workspace --no-fail-fast` **exit 101**，3 個失敗（依指示保留合併，沒有 reset）：

  | crate | 測試 | 位置 | 觀察 |
  |---|---|---|---|
  | jc-core | `ja::personal::tests::test_candidates_put_learned_first` | `crates/jc-core/src/ja/personal.rs:1944` | 學過「京」後第一名仍是「今日」，`left: "今日"`、`right: "京"` |
  | jc-model | `punct_plan::tests::test_window_starts_matches_reference_rule` | `crates/jc-model/src/punct_plan.rs:203` | `window_starts(300, 128, 0)` 回 `0..172`，期望 `[0, 1, 2, 172]`；`stride.max(1)` 與期望規則不一致 |
  | jc-model | `punct_plan::tests::test_merge_window_rows_prefers_central_prediction` | `crates/jc-model/src/punct_plan.rs:215` | `merged[1]` 得 `[9.0, 0.0]`，期望中央視窗的 `[0.0, 1.0]` |

  兩個 `punct_plan` 失敗都在 punct-wire 新增的檔案內，且該檔沒有合併衝突，屬該 task 自身測試與實作不一致；
  `personal.rs` 同樣是 ja-personal 新增檔案。三個失敗都需回原 task 修。

- `ja-live-rewrite`（合併 `7b7047a`）：`ja/mod.rs` 與 `session.rs` 自動合併，無衝突。新增 `ja/live.rs`（1099 行），
  `ja/rewrite.rs` 補到 1057 行，`session.rs` 接上逐字変換與轉換後改寫。

  驗證：`cargo check --workspace --all-targets` exit 0、`cargo fmt --all -- --check` exit 0、
  `cargo clippy --workspace --all-targets -- -D warnings` exit 0；
  `cargo test -p jc-core --lib ja::live` 13 passed、`ja::rewrite` 17 passed，皆 0 failed。
  依指示沒有跑全量 `cargo test`（punct-wire／ja-personal 的 3 個已知失敗仍在修）。

- `punct-wire` 第二次合併（`d53703f`，含 `d1ae090`）：無衝突，只改 `crates/jc-model/src/punct_plan.rs` 的視窗測試期望
  與 `window_starts` 的 stride 邊界。`cargo test -p jc-model --lib punct_plan` 4 passed、0 failed；
  `cargo check --workspace --all-targets` exit 0、`cargo clippy --workspace --all-targets -- -D warnings` exit 0。
  先前 punct_plan 的兩個失敗已解除；ja-personal 的 `test_candidates_put_learned_first` 仍待修。

- `ja-personal` 第二次合併（`88c482d`，含 `0a376e5`）：無衝突，`ja/personal.rs` 把候選重排分數換回浮點尺度（1 行）。
  `cargo test -p jc-core --lib ja::personal` **34 passed、0 failed**（先前的 `test_candidates_put_learned_first` 已修）；
  `cargo check --workspace --all-targets` exit 0。三個已知 test 失敗至此全部解除。

- `ja-live-rewrite` 第二次合併（`d83cf5b`，含 `7516c29`）：無衝突，`ja/live.rs` 的 `advance_lock` 不再鎖定詞面等於讀音的
  かな文節（16 增、2 刪），解掉「きょ」只轉出かな、跨詞組不出「今日」的問題。
  `cargo test -p jc-core --lib` **418 passed、0 failed**（含上一輪失敗的 `test_ja_live_enabled_converts_without_enter`）；
  `cargo check --workspace --all-targets` exit 0。全量 gate 的三項至此都應為綠。

- `personal-zh`（合併 `55f2067`）：`jc-cli/src/cli.rs`、`converter.rs`、`jc-core/src/session.rs` 有衝突，照兩邊意圖都保留解：
  `Convert`／`ConvertOptions` 同時有 `punct` 與 `user_data`，`SessionConfig` 同時有 `punct` 與 `personal`，
  usage 的 convert 行合併兩個旗標。新增 `crates/jc-core/src/personal/`（13 個模組）與 `tests/personal_port.rs`。

  四個 gate：
  | gate | 結果 |
  |---|---|
  | `cargo fmt --all -- --check` | exit 0（pass） |
  | `cargo clippy --workspace --all-targets -- -D warnings` | **exit 101（fail）**，jc-core 12 個錯誤 |
  | `cargo test --workspace --no-fail-fast` | exit 0（34 targets、766 passed、0 failed） |
  | `cargo run -p jc-cli --release -- eval zh-golden` | exit 0，**通過 45/45**（top-1 100.0%） |

  clippy 失敗清單（全部在 `crates/jc-core/src/personal/`，需回 personal-zh 修）：
  - `english.rs:3` `unused_imports`：`BTreeMap`
  - `preference.rs:7` `unused_imports`：`DAY_SECONDS`、`UsageEvidence`
  - `unlearn.rs:451` `unused_imports`：`super::*`
  - `mod.rs:642` `dead_code`：`now`
  - `mod.rs:796` `dead_code`：欄位 `undo`
  - `diversity.rs:87` `dead_code`：`refresh_diversity`
  - `english.rs:111` `dead_code`：欄位 `gen`、`store`、`base`、`index`
  - `prune.rs:325` `dead_code`：`decay_since`
  - `prune.rs:330` `dead_code`：`pair_key`
  - `diversity.rs:26` `neg_cmp_op_on_partial_ord`：`if !(value > 0.0)`
  - `english.rs:165` `let_and_return`
  - `mod.rs:622`、`mod.rs:627` `ptr_arg`：`&mut Vec<...>` 應改 `&mut [...]`

  依指示保留合併、沒有 reset；build 本身正常（只有 warning）。

- `rerank-tokenizer`（合併 `bcb1515`）：`jc-cli/src/cli.rs` 與 `jc-model/src/tokenizer.rs` 有衝突，照兩邊意圖都保留解：
  `TokenSpan`／`normalize_map`／`encode_with_map` 與 `Strategy`／`spm::Model` 並存，`encode` 先走 unigram，
  其餘路徑仍用 span 版最長前綴匹配；usage 保留 `--punct`／`--user-data` 並加上 `tokenize ja`。
  fmt 通過、clippy 通過；`cargo test --workspace` 4 個失敗（`spm`／`tokenizer` 的正規化測試）。
  結論（作者量測）：換回真正的 SentencePiece unigram 只讓 rerank top-1 從 40.8% 升到 **41.0%**（+0.2 個百分點），
  主要限制仍是候選粒度。

- `personal-zh` 第二次合併（`e5af3b8`，含 `6faec7f`）：無衝突，清掉先前 12 個 clippy 錯並接上英文索引快取。
  四個 gate 結果：
  | gate | 結果 |
  |---|---|
  | `cargo fmt --all -- --check` | exit 0 |
  | `cargo clippy --workspace --all-targets -- -D warnings` | **exit 0** |
  | `cargo test --workspace --no-fail-fast` | exit 101：34 targets、783 passed、**4 failed** |
  | `cargo run -p jc-cli --release -- eval zh-golden` | exit 0，**45/45**、top-1 100.0% |

  4 個失敗與 personal-zh 無關，是 rerank-tokenizer 帶入的正規化問題，需回該 task：
  - `spm::tests::normalize_matches_sentencepiece_identity_rule`（`jc-model/src/spm.rs:665`）：
    `"▁▁東京▁都"` vs 期望 `"▁東京▁都"`，dummy prefix 重複加了一次。
  - `spm::tests::unknown_runes_keep_unk_id_without_byte_pieces`（`spm.rs:653`）：`[4, 1, 1, 1]` vs `[1, 1, 1]`。
  - `tokenizer::tests::test_normalize`（`tokenizer.rs:651`）：`normalize("  a  b ")` 得 `"▁▁a▁b"`，期望 `"▁a▁b"`。
  - `tokenizer::tests::normalize_adds_dummy_prefix_and_collapses_spaces`（`tokenizer.rs:687`）：同樣多一個 `▁`。

  已由 `rerank-tokenizer` 的 `3770403`／後續 OOV 補測修正：前導空白先去乾淨再加
  dummy prefix；`unknown_runes_keep_unk_id_without_byte_pieces` 的期望改為
  `[▁, <unk>, <unk>, <unk>]` 與 `[▁, <unk>]`，並用 Python `sentencepiece` 0.2.3
  對真模型驗證 byte fallback 與連續 unk 合并，對照表見
  `docs/port/rerank-tokenizer.md` 的「byte fallback 與 OOV 補測」。

- `rerank-tokenizer` 第二次合併（`b07d098`，含 `3770403`、`0eb7496`）：無衝突。
  `spm.rs` 修掉 dummy prefix 重複（前導空白先去乾淨再加），並把「無 byte fallback 時連續 unknown 節點合并成一個 `<unk>`」
  對齊 `sentencepiece_processor.cc` 的 `EncodeOptimized`。

  驗證：`cargo fmt --all -- --check` exit 0、`cargo clippy --workspace --all-targets -- -D warnings` **exit 0**、
  `cargo test --workspace --no-fail-fast` **exit 0**（34 targets、787 passed、0 failed）。先前的 4 個正規化失敗全數解除。

  byte fallback 對照（Python `sentencepiece` 0.2.3，`public_core_ja.model`，32768 pieces／256 BYTE）：
  - 開啟：`𠮷`、`😀`、`東京𠮷`、`a😀b`、`𠮷😀①髙` 的 ids 與 pieces 全部與 SP 相同，OOV 走 `<0xNN>` 而非 `<unk>`。
  - 關閉（移除 BYTE piece）：`𠮷` → `7,1`、`東京𠮷` → `9759,1`、`𠮷𠮷` → `7,1`，連續 OOV 合并成單一 `<unk>`，與 SP 一致。
  - `byte_fallback=true` 但缺 byte piece 的模型，SP loader 會直接拒絕；Rust 保留防禦性 fallback 測試。

- `settings-core`（合併 `52cb1b9`）：無衝突，新增 crate `crates/jc-settings`（`settings`、`profiles`、`profile_rules`、`paths`、
  `modelset/{manifest,install,mod}`、`atomic_file`、`both_shift` 等，7581 行），workspace 加 member。

  驗證：
  | gate | 結果 |
  |---|---|
  | `cargo check --workspace --all-targets` | exit 0 |
  | `cargo test -p jc-settings` | **exit 101：72 passed、4 failed** |
  | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
  | `cargo fmt --all -- --check` | exit 0 |

  4 個失敗（都在 `crates/jc-settings`，需回 settings-core）：
  - `features_tests::test_left_shift_defaults_follow_platform_and_persists`（`features_tests.rs:257`）：
    `save(&path, …)` 回 `Io(NotFound)`，存檔前沒有建立上層目錄。
  - `tests::test_profile_layout_guard_cannot_be_bypassed_by_copy`（`tests.rs:755`）：選用 `microsoft` 後
    `set("keyboard-layout", "hsu", "linux")` 應該被風格鎖擋下，實際成功。
  - `tests::test_profiles_migration_and_complete_round_trip`（`tests.rs:687`）：來回一趟後 `read_only`／`profile_invalid`
    由 `false` 變 `true`。
  - `tests::test_profile_locks_all_managed_entry_points`（`tests.rs:826`）：`change(..., "output-simplified", ...)`
    回 `ReadOnly`，舊鍵被風格鎖誤擋。

  刻意排除的範圍（記在 `docs/port/settings-core.md`）：UI 契約（`DisabledBy`／`DisabledReason`、Items、停用原因）、
  網路 fetch 與 pin（Hugging Face 下載、`valid_revision` 只保留驗證）、`internal/filelock`（跨 process 鎖，
  由呼叫端負責）、placeholder 測試（需要 `OC_PLACEHOLDER_MODELS` 的模型檔）。

- 三條依序合併：`onnx-default`（`79724ee`，移除 `onnx` feature、改成執行期偵測 ONNX Runtime）、
  `zh-select-calibrate`（`9a04a60`，新增 `zh-calibrate` bin 與 select margin 掃描）、
  `english-suggest`（`359d425`，接上英文拼字建議與 `convert --mode en`）。
  `english-suggest` 在 `jc-cli/src/converter.rs` 有衝突：branch 的 `#[cfg(not(feature = "onnx"))]` 檢查
  在 `onnx-default` 之後已失效（feature 不存在），解衝突時移除該段，保留 `mode == En` 的候選路徑。

  最終 gate：
  | gate | 結果 |
  |---|---|
  | `cargo fmt --all -- --check` | exit 0 |
  | `cargo clippy --workspace --all-targets -- -D warnings` | **exit 101：13 個錯誤** |
  | `cargo test --workspace --no-fail-fast` | exit 101：38 targets、886 passed、**6 failed** |

  clippy 錯誤全在 `crates/jc-cli/src/bin/zh-calibrate.rs`，`unexpected cfg condition value: onnx`，
  行號 15、90、97、147、162、256、274、376、415、462、530（含 `#![cfg_attr]` 與各 `#[cfg(feature = "onnx")]`）。
  來源是 `zh-select-calibrate` 仍用舊 feature 名稱，需回該 task 改成執行期偵測。

  test 失敗 6 個：
  - `jc-settings`（settings-core 既有，4＋1 個）：`tests::test_profile_layout_guard_cannot_be_bypassed_by_copy`、
    `tests::test_profile_locks_all_managed_entry_points`、`tests::test_profiles_migration_and_complete_round_trip`、
    `features_tests::test_left_shift_defaults_follow_platform_and_persists`、`features_tests::test_choice_settings_defaults_and_persist`
    （後兩者都是存檔前沒建目錄的 `Io(NotFound)`）。
  - `jc-core`（english-suggest 新帶入）：`session::tests::test_mixed_english_suggestion_and_completion`
    （`session.rs:2130`，`left: "hellp"`、`right: "我hellp"`）。

- `zh-select-calibrate` 第二次合併（`eabae70`，含 `9f84483`）：無衝突，`zh-calibrate` 加 model-only 與分數方向診斷
  （+160 行），文件 +43 行。`cargo check --workspace --all-targets` exit 0；13 個 `unexpected cfg condition value: onnx`
  警告仍在（屬上一輪已回報的 clippy 待修項）。

  診斷結論：
  - 主 dev（46 組）：模型單獨排序 **46/46**、統計首選錯 **0**、模型只靠分數救回 **0**；瓶頸是沒有可救回的樣本，
    不是融合公式壓住模型。
  - 壓力 dev（161 組、127 組可評分）：模型單獨排序 48/161，統計首選錯 52 組，其中只有 1 組答案仍在候選內，
    模型也沒選到；其餘 51 組答案不在候選內，屬讀音本身錯誤，模型無能為力。
  - 分數方向核對 7/7 正確（正確候選分數高於錯誤候選），沒有反向或正負號錯置。
  - 破壞最少的掃描點是 `weight = 0.5`、`margin = 3.0`，壓力 dev 仍破壞 2/161（1.24%）超過 1% 上限，
    因此 `--select-model` **預設維持 off**。

- `english-suggest` 第二次合併（`b9459ac`，含 `a9e8fb0`）：無衝突，`jc-core/src/session.rs` 3 行，
  修掉混合模式候選覆蓋整個組字字串的問題。

  驗證：`cargo test --workspace --no-fail-fast` exit 101（38 targets、889 passed、**3 failed**），
  `cargo clippy --workspace --all-targets -- -D warnings` exit 101（**13 個錯誤**）。兩個 gate 的失敗都不是本 branch：

  - test 3 個全在 `jc-settings`（settings-core 既有）：`test_profile_layout_guard_cannot_be_bypassed_by_copy`、
    `test_profile_locks_all_managed_entry_points`、`test_profiles_migration_and_complete_round_trip`。
    先前的兩個 `Io(NotFound)` 測試這次通過（前一次執行已建出目錄），通過數由 886 升到 889。
  - clippy 13 個全是 `crates/jc-cli/src/bin/zh-calibrate.rs` 的 `unexpected cfg condition value: onnx`
    （行 15、90、97、149、164、258、276、378、417、464、534、585、690），屬 zh-select-calibrate。
  - 本 branch 的 `session::tests::test_mixed_english_suggestion_and_completion` 已轉綠。

- `legal-notices`（合併 `823eab9`）：無衝突，22 個檔案、2967 行，全是法律與文件：
  `legal/licenses/`（IPADIC、Mozc、UNIDIC、MIT、Funmula、注音資料 NOTICE 等 10 份）、`legal/README.md`、
  `legal/THIRD-PARTY-NOTICES.md`、`NOTICE`、`docs/models.md`，README 授權段補上帳本與 NOTICE 連結。

  `cargo fmt --all -- --check` exit 0。`cargo clippy --workspace --all-targets -- -D warnings` 仍 exit 101，
  13 個錯誤全是 `crates/jc-cli/src/bin/zh-calibrate.rs` 的 `unexpected cfg condition value: onnx`，
  屬 zh-select-calibrate 待修項，與本 branch 無關。

- `zh-select-calibrate` 第三次合併（`5fcf8ca`，含 `741e59a`）：無衝突，`zh-calibrate.rs` 移除 13 處失效的 `onnx` cfg
  （22 行刪、4 行增），文件同步。
  `cargo clippy --workspace --all-targets -- -D warnings` **exit 0**；`cargo test -p jc-cli` exit 0（6 targets、40 passed、0 failed）。
  先前 13 個 `unexpected cfg condition value: onnx` 錯誤全部消失。jc-settings 的 3 個既有失敗依指示先不處理。

- `settings-core` 第二次合併（`89a4f6f`，含 `87cfdf1`）：無衝突，`profiles.rs` 修內建風格快照（+2 行）、
  `modelset/tests.rs` 修測試暫存目錄清理（3 行），未改任何測試斷言。
  三個 gate 全過：`cargo fmt --all -- --check` exit 0、`cargo clippy --workspace --all-targets -- -D warnings` exit 0、
  `cargo test --workspace --no-fail-fast` **exit 0**（38 targets、892 passed、0 failed）。
  先前 jc-settings 的 3 個失敗全部解除，main 全量測試歸零。

- `rerank-wordlevel`（合併 `9476f51`）：`jc-core/src/ja/rerank.rs` 有衝突（檔頭說明與 `left_context` 註解）。
  檔頭採 branch 的詞層級說明；`left_context` 保留 main 的實作與註解（含「切法非單調、不能提早停」的處理），
  因為合併後的函式體已是該版本，branch 的簡化說明與實際行為不符。`layout.rs`、`rerank.rs`、`probe.rs` 自動合併。

  量測（`modern-500`，`--rerank on`）：top-1 **82.0%**（`--rerank off` 86.0%）、top-5 **95.2%**、p50 **78.8 ms**（p95 146.49 ms、timeout 29.0%）。
  只重排最有歧義的 1 個詞群時 top-1 是 85.2%，介入越多越接近 82.0%。
  模型單獨排序 Rust 版 **83.0%**，與同一份模型在 Python 的 **83.0%** 完全一致，確認 Rust 端推論與前處理沒有偏差。

  gate：`cargo fmt --all -- --check` exit 0、`cargo clippy --workspace --all-targets -- -D warnings` exit 0、
  `cargo test --workspace --no-fail-fast` exit 101（38 targets、890 passed、**5 failed**）。5 個失敗都在本 branch 的檔案，需回 rerank-wordlevel：
  - `ja::rerank::tests::right_context_keeps_the_prefix_nearest_the_cursor`（`jc-core/src/ja/rerank.rs:380`）：
    測試 fixture 把 `▁東京都` 當單一 piece，`right_context("東京都東京", 1)` 回 `東京都`，測試期望 `東京`。
  - `layout::tests::candidate_piece_prefers_the_bare_piece`（`jc-model/src/layout.rs:238`）：fixture 有裸 `東`，
    `candidate_piece` 回 261，測試期望 `▁東` 的 264。
  - `layout::tests::candidate_longer_than_slot_is_truncated`（`layout.rs:258`）：mask 總和 6，期望 7。
  - `layout::tests::empty_context_uses_bos_marker`（`layout.rs:297`）：piece id 266 vs 265。
  - `layout::tests::real_tokenizer_fills_the_60_token_left_window`（`layout.rs:363`）：`context_ids` 會剝掉開頭 `▁`，
    測試仍拿 `t.encode(left)[..60]` 直接比對（1324 vs 10015）。

- `rerank-wordlevel` 退回修正：5 個失敗都是測試期望與現行契約不符，未改產品行為。
  `ja/rerank.rs` 的右文 fixture 改期望 `東京都`，因為 `▁東京都` 是單一 token，取前 1 個 token 就是整個詞。
  `layout.rs` 的候選 slot 改比裸 piece（`東京`、`東`），補上只有行首變體時才退回 `▁東京都` 的案例；
  截斷測試的 mask 總和改 6；空文 BOS 測試改比裸 `東京`。
  真實 tokenizer 測試先依 `context_ids` 規則剝獨立詞首 `▁` 再算左文，候選 slot 改用
  `candidate_piece`。原本第 5 個失敗其實出在 `row[62]` 與 `t.encode("東京")[0]` 的差異
  （裸 `東京` 對 `▁東京`）；這份模型把 `▁` 併進後面的 piece，`context_ids` 沒有東西可剝，
  測試把兩種切法混用了。

  `docs/port/rerank-wordlevel.md` 補上 `eval timeout` 29% 的來源：那是整次轉換超過 100 ms
  的比例，包含兩次 Viterbi、最多 3 組詞群準備與實際推論；重排統計只記整批放棄，兩者用不同計時器。

  gate：`cargo fmt --all -- --check` exit 0、`cargo clippy --workspace --all-targets -- -D warnings` exit 0、
  `cargo test --workspace --no-fail-fast` **exit 0**（38 targets、895 passed、1 ignored）。
  重跑 `modern-500 --rerank on`：timeout 23.2%，與原紀錄 29.0% 同屬延遲尾端，差異來自機器負載。

- `rerank-wordlevel` 第二次合併（`2e649f5`，含 `2c25f3d`）：無衝突，修 5 個測試期望（未改產品行為），
  `ja/rerank.rs` 右文 fixture 改期望 `東京都`、`layout.rs` 候選 slot 改比裸 piece、截斷 mask 改 6、
  真實 tokenizer 測試先依 `context_ids` 剝詞首 `▁`。

  gate：`cargo fmt --all -- --check` exit 0、`cargo clippy --workspace --all-targets -- -D warnings` exit 0、
  `cargo test --workspace --no-fail-fast` **exit 0**（38 targets、895 passed、0 failed）。

  timeout 解釋（作者補在 `docs/port/rerank-wordlevel.md`）：500 題挑出 3973 個詞群、截到每題 3 組後剩 1452 組，
  實際送模型 556 組，平均每題 1.1 次推論；**單次推論 p50 約 20 ms、p95 約 46–50 ms**。100 ms 預算只比整題中位數
  高 1.3–1.5 倍，一題碰到 2–3 次推論或第二次 Viterbi 落到尾端就會超過，所以 `eval timeout` 落在 **23–29%**；
  重排自身的 `timeout` 只有 **1–2 次**（1/500 到 2/500），兩者用的是不同計時器。

- `defaults-policy`（合併 `175f417`）：無衝突。預設值改成學習開啟、模糊音四組全開、注音選字模型 on、`ja_rerank` false；
  `session.rs` 的 `SessionConfig::default` 與 CLI 的資料載入同步調整。
  gate：fmt exit 0、clippy exit 0、`cargo test --workspace --no-fail-fast` exit 0（38 targets、898 passed、0 failed）。
  `eval zh-golden`：**45/45**、top-1 100.0%（`select-model：沒有推論`，因 golden 案例皆單音節，模型未觸發）。
  `eval ja --corpus modern-500`：top-1 **86.0%**、top-5 98.0%、boundary 88.2%、p50 3.147 ms。

- `engine-data`（合併 `8aa5ab1`）：`jc-cli/src/data.rs` 自動合併（載入邏輯搬進 `jc-engine/src/loader.rs`，jc-cli 反向依賴）。
  新增 `jc-engine/src/loader.rs`（762 行）、`tests/engine_data.rs`，`main.rs` 接上 `--data`／`JC_DATA`。

  1) gate：`cargo clippy --workspace --all-targets -- -D warnings` exit 0；
     `cargo test --workspace --no-fail-fast` exit 101（39 targets、907 passed、**4 failed**），全在 `jc-engine`：
     `tests/stdio.rs` 的 `stdio_round_trip_and_clean_shutdown`（:38）、`version_mismatch_is_rejected_and_closed`（:38）、
     `eof_without_requests_exits_cleanly`（:126，exit 1 而非 0），與 `tests/socket.rs` 的
     `socket_serves_connections_sequentially_and_shutdown_cleans_up`（:46，5 秒內沒開始監聽）。
     原因：`jc-engine` 現在沒給 `--data`／`JC_DATA` 就拒絕啟動（實測輸出
     `jc-engine: 沒有指定 --data，且 JC_DATA 未設定`），這些整合測試仍用裸 `Command::new(env!("CARGO_BIN_EXE_jc-engine"))`
     啟動，需由 engine-data 補上資料目錄。

  2) stdio 實測（`jc-engine --data /mnt/work/jc/jc-ime-data`，握手→按鍵→snapshot→commit→shutdown）：
     - `--mode mixed`：`ji3` → **我**；`watashiwa` → `watashiwa`（mixed 對純 ASCII 仍走英文直通，屬既有行為）。
     - `--mode ja`：`watashiwa` → **私羽**（實際日文漢字，與既有 `私わ`／`私羽` 差異一致）；`ji3` → 時（日文讀法）。
     結論：注音與日文都能轉出實際文字；要一次看到兩者需分別用 `mixed` 與 `ja` 模式。

  3) 預設套用稽核：`zh_session_config` 用 `..SessionConfig::default()`，因此
     **學習（`learning: true`）與模糊音（`DEFAULT_FUZZY_PAIRS` 四組）都有套用**；
     但 **zh-select 沒有套用**：`SessionConfig` 沒有選字模型欄位，CLI 是靠全域 `zh::select::install` 安裝，
     `jc-engine/src/loader.rs` 雖然有 `install_select_model`／`disable_select_model` 兩個函式，`main.rs` 從未呼叫，
     所以引擎實際只走統計排序。這是實作缺口，依指示回報，沒有自行修改。

- `ja-ime-survey`（合併 `c9b8998`）：無衝突，只新增 `docs/reports/ja-ime-survey-2026-11-20.md`（898 行），沒有程式改動。
  `cargo check --workspace --all-targets` exit 0（全部 cached）。

- `engine-data` 第二次合併（`d2aba42`，含 `8dc6235`）：無衝突。加 `--allow-empty-data` 讓整合測試不必準備資料，
  並在啟動時安裝 zh-select 模型；`tests/stdio.rs`、`tests/socket.rs` 各加一行參數。上一輪的 4 個 jc-engine 失敗全部解除。
  worktree 內的未追蹤 `Cargo.fetches.lock`（blaze 產生）沒有進 main，另外在 `.gitignore` 補一行避免誤入（commit `3f67a14`）。

- `ja-personal-wire`（合併 `b32beb2`）：無衝突。`ja/personal.rs` 加 300 行、`personal/mod.rs` 加 424 行，
  `session.rs` 接上日文學習，`jc-cli` 的 `cli.rs`／`converter.rs` 接 `--user-data`。

  三個 gate：`cargo fmt --all -- --check` exit 0、`cargo clippy --workspace --all-targets -- -D warnings` exit 0、
  `cargo test --workspace --no-fail-fast` **exit 0**（39 targets、925 passed、0 failed）。

  engine 實測（`jc-engine --data /mnt/work/jc/jc-ime-data`，握手→按鍵→snapshot→commit→shutdown）：
  - `--mode mixed`：`ji3` → **我**（正確）。
  - `--mode ja`：`watashiwa` → **私羽**；Go 參考是 `私わ`，差異與先前 `mixed-e2e` 記錄一致（Mozc pack 詞面成本，
    不是路由或解碼錯誤），照實記錄不視為失敗。`--mode mixed` 下 `watashiwa` 仍走英文直通輸出 `watashiwa`。

- `ajimee-eval`（合併 `7885302`）：`jc-cli/src/cli.rs` 自動合併，無衝突。`jc-eval/src/ja.rs` 加 306 行（目錄掃描題庫），
  `eval_cmd.rs` 接上 `ajimee-200`，文件 `docs/port/ajimee-eval.md`。
  gate：fmt exit 0、clippy exit 0、**定期全量 `cargo test --workspace --no-fail-fast` exit 0**（39 targets、931 passed、0 failed）。

- `ja-models-fetch`（合併 `44e8a5f`）：無衝突，只新增 `docs/port/ja-models-fetch.md`（169 行）與 `legal/README.md` 條目（27 行），
  沒有程式碼。內容是下載外部日文模型（Jinen v2 xsmall）並跑 AJIMEE smoke test 的紀錄。
  `cargo check --workspace --all-targets` exit 0（全部 cached）。

- `ja-cost-diag`（合併 `058f44b`）：無衝突。新增 `crates/jc-cli/src/bin/ja-cost.rs`（416 行）與 `docs/port/ja-cost-diag.md`，
  `jc-core/src/ja/decode.rs` 加 14 行診斷用介面。

  gate：`cargo test --workspace --no-fail-fast` **exit 0**（40 targets、932 passed、0 failed）、fmt exit 0；
  `cargo clippy --workspace --all-targets -- -D warnings` **exit 101**，1 個錯：
  `crates/jc-cli/src/bin/ja-cost.rs:272` `clippy::needless_lifetimes`（`fn enumerate<'a>(lat: &'a Lattice, …)` 可省略），
  需回 ja-cost-diag 修。

  診斷結論（`わたしわ` → `私羽` 的 9068）：
  - 9068 = 詞成本 **4612** + 連接成本 **4456**。
  - `私+羽` 的連接成本 **966**（`matrix(1902, 1949)`），`私+わ` 的連接成本 **5556**（`matrix(1902, 431)`），差 **4590**。
  - `私わ` 的詞成本便宜 **4346**，仍被連接成本差 4590 蓋過，淨差 244，所以輸出 `私羽` 而非 Go 參考的 `私わ`。
    這是 Mozc context 矩陣的既有權重，不是解碼錯誤。
