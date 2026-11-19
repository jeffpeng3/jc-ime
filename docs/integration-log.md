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
