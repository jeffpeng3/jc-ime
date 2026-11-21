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
| `rerank-gating` | `a0a6f32` | `5932532` | test 1 個失敗；clippy 待 ja-cost 修 | 2026-11-20 04:55 |
| `ja-particle-rewrite` | `a5a5aab` | `af7ef2f` | test 2 個失敗；clippy 待 ja-cost 修 | 2026-11-20 05:05 |
| `candle-spike-ja` | `7cc9d8a` | `688f1f6` | check 通過；test build 被 gemm-f16 擋住 | 2026-11-20 05:08 |
| `ja-cost-diag`（修 lifetimes） | `e5b720d` | `fbea06a` | clippy 通過；test 同上 | 2026-11-20 05:09 |
| `rerank-gating`（left context 順序） | `b5d0e26` | `f681ed4` | clippy 通過；test 同上 | 2026-11-20 05:10 |
| `jinen-eval` | `9bd13b1` | `e2d6cfb` | check 通過 | 2026-11-20 05:12 |
| `mozkey-study` | `b8ad3ce` | `764ee82` | check 通過 | 2026-11-20 05:25 |
| `ja-particle-rewrite`（golden＋engine） | `aa5797f` | `c04e85e` | 通過（clippy＋全量 test） | 2026-11-20 05:30 |
| `jinen-integrate` | `34688d7` | `72985f6` | 通過（clippy＋全量 test） | 2026-11-20 05:38 |
| `quant-study` | `cf0926e` | `8e27af0` | check 通過 | 2026-11-20 05:38 |
| `candle-punct` | `9705bb5` | `f9b2e69` | 通過（clippy＋全量 test） | 2026-11-20 05:46 |
| `jinen-gating` | `f71d5cc` | `e610808` | test 通過；clippy 2 個錯 | 2026-11-20 05:55 |
| `session-core-split` | `63b81e7` | `22831f0` | test／fmt／兩個 eval 通過；clippy 剩 jinen-gate | 2026-11-20 05:59 |
| `jinen-gating`（修 clippy） | `1c7ba21` | `64f49e8` | 通過（clippy＋全量 test） | 2026-11-20 06:05 |
| `session-g3-symbols` | `c67c4e7` | `86d24e0` | 通過（fmt／clippy／test／兩個 eval） | 2026-11-20 06:08 |
| `session-g2-segments` | `57e3a5d` | `1fd343d` | 通過（fmt／clippy／test／兩個 eval） | 2026-11-20 06:16 |
| `session-g1-input` | `45a73fa` | `ded267a` | test 2 個失敗（branch 自帶）；fmt／clippy／兩個 eval 通過 | 2026-11-20 06:26 |
| `proto-extend` | `43c1741` | `a60766b` | test 剩同 2 個；fmt／clippy／兩個 eval 通過 | 2026-11-20 06:33 |
| `session-g1-input`（修測試） | `4a493c3` | `f442cd4` | 通過（fmt／clippy／全量 test／兩個 eval） | 2026-11-20 06:37 |
| `candle-quant` | `51f459c` | `1972ae1` | test 1 個失敗、clippy 1 個錯（皆 branch 自帶） | 2026-11-20 06:40 |
| `candle-quant`（修門檻） | `66347f4` | `77d8c1f` | 通過（fmt／clippy／全量 test） | 2026-11-20 06:52 |
| `session-g4-mixed` | `6b9f1b5` | `8de9306` | 通過（fmt／clippy／test／三個 eval） | 2026-11-20 07:02 |
| `session-g5-misc` | `12f161f` | `7379047` | 通過（fmt／clippy／test／三個 eval） | 2026-11-20 07:16 |
| `session-g6-core` | `ffccac1` | `b7af358` | 通過（fmt／clippy／test／四個 eval） | 2026-11-20 07:37 |
| `session-g4-mixed`（CI 修正） | `6b76ce1` | `fb4e7ab` | 通過（clippy＋全量 test） | 2026-11-20 07:58 |
| `session-backfill` | `ade1479` | `610498c` | 通過（fmt／clippy／test／四個 eval） | 2026-11-20 08:07 |
| `drop-punct` | `859f6d1` | `dacb52b` | 通過（fmt／clippy／test／兩個 eval；無殘留） | 2026-11-20 08:28 |
| `jinen-gating-v2` | `81e0cbe` | `83152fc` | test 通過；clippy 1 個錯 | 2026-11-20 08:31 |
| `candle-zh` | `131f606` | `d8da98f` | 通過（fmt／test／兩個 eval）；clippy 剩 jinen-gate | 2026-11-20 08:37 |
| `proto-compat` | `a622acd` | `06b4d5b` | 通過（fmt／test）；clippy 剩 jinen-gate | 2026-11-20 08:48 |

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

- `rerank-gating`（合併 `5932532`）：`cli.rs`、`eval_cmd.rs` 有衝突，照兩邊意圖都保留解：`--gate`（`WordGate`、`reject_ja_flags` 加 gate 參數）
  與 ajimee 的 `--items`／`JaOptions` 並存；`run_command` 以 `gate: rerank.then_some(*gate)` 傳入。合併後另補一個整合修正（`3ea5ab5`）：
  `ja()` 不再使用 `rerank` 欄位，改成 `rerank: _`，消掉 clippy 的 unused variable。
  gate：fmt exit 0；test exit 101（40 targets、934 passed、1 failed）：
  `ja::decode::tests::test_word_reranker_sees_decoded_left_context`（`decode.rs:2172`，`["今日", ""]` vs `["", "今日"]`），屬 rerank-gating。
  clippy 只剩 ja-cost 的 `needless_lifetimes`（ja-cost-diag 待修）。

  門檻表（`--gate Gap(100)`）：
  - modern-500：top-1 **86.0%**、top-5 **98.0%**、p50 **7.3 ms**，只送 **6%** 詞群進模型。
  - ajimee-200：**46.5%**。
  - hard-cases：**54.3%**，未達「> 54.3%」的目標。
  - 門檻放寬到 350 時 hard-cases 升到 **57.1%**，但 top-5 掉到 **94.3%**。

- `ja-particle-rewrite`（合併 `af7ef2f`）：`ja/decode.rs` 自動合併，無衝突。`JaBuilder` 加 `particle_rewrite`（核心預設 false，
  `jc-cli` 的 `data::load_ja_decoder*` 一律 `with_particle_rewrite(true)`），`rewrite.rs` 加助詞讀音交替。

  三組題庫 top-1 不變：modern-500 **86.0%**、hard-cases **54.3%**、ajimee-200 **46.0%**（`--items` 逐題比對開啟／關閉完全相同）。

  七個案例（關閉 → 開啟）：`わたしわ` 私羽 → **私は**；`こんにちわ` こんにちわ → **こんにちは**；`あるいわ` → **あるいは**；
  `ほんおよむ` 本お読む → **本を読む**；`にほんえいく` 日本栄区 → **日本へ行く**；`すなわち`／`いわば` 不變。

  限制：`え→へ`、`お→を` 只開放給助詞位置，不能給一般詞，因為實測 `〜についてえきではなす` 會讓 modern-500 的 `駅` 變成 `壁`。

  gate：fmt exit 0；test exit 101（40 targets、936 passed、2 failed）：
  - `eval_cmd::tests::mixed_golden_compares_surfaces_and_boundaries`（`eval_cmd.rs:1311`）：mixed golden 期望 `私わ`，實際 `私は`，
    是本次改寫的預期結果，golden 期望值需由相關 task 更新。
  - `ja::decode::tests::test_word_reranker_sees_decoded_left_context`：上一輪 rerank-gating 既有失敗，與本 branch 無關。
  clippy 仍是 ja-cost 的 `needless_lifetimes`。

  engine 缺口（依指示回報、未自行改）：`crates/jc-engine/src/loader.rs` 沒有任何 `with_particle_rewrite` 呼叫，
  核心預設是 `false`，所以 **jc-engine 目前沒有開啟助詞改寫**；只有 `jc-cli` 開了。

- `candle-spike-ja`（合併 `688f1f6`）：`cli.rs`、`eval_cmd.rs` 有衝突，照兩邊意圖都保留解：
  `EvalJa`／`JaOptions` 同時有 `gate` 與 `backend`，`--backend ort|candle` 與 `--gate` 並存；
  `ja()` 在載入 reranker 前呼叫 `jc_model::set_default_backend`，rerank 統計行同時印後端名稱與 `groups_total`／`skipped`。

  量測（作者）：candle fp32 與 ORT fp32 的 logits max|Δ| **1.75e-5**，排序 **200/200** 一致；
  端到端 candle **82.2%** vs ORT int8 **82.0%**、p50 **36.24** vs **33.62 ms**、timeout **0%** vs 0.2%；
  單次推論 **17.95** vs **17.67 ms**（candle 慢 1.6%）。full 124-token batch 時 candle 比 ORT fp32 慢 **3.2 倍**，
  靠跳過遮罩 token 才追平。

  **建置阻塞（重要）**：合併後 `cargo test --workspace` 無法編譯，`gemm-f16 v0.19.0`（candle 依賴）在 aarch64 預設 target 下
  報 `instruction requires: fullfp16`（11 個錯）。CPU 本身有 `fphp`／`asimdhp`，但 rustc 的 baseline 沒開 `fullfp16`。
  實測 `RUSTFLAGS="-C target-feature=+fullfp16" cargo test --workspace --no-fail-fast` 可以建置並跑完：
  **41 targets、943 passed、1 failed**（`eval_cmd::tests::mixed_golden_compares_surfaces_and_boundaries`，即 ja-particle-rewrite 的預期差異）。
  `cargo clippy --workspace --all-targets -- -D warnings` 在未加 RUSTFLAGS 時仍 exit 0。
  需決定：把 `+fullfp16` 寫進 `.cargo/config.toml`、改用 candle 的 f16 kernel 關閉選項，或把 candle 後端放到 feature 後面。

- `ja-cost-diag` 第二次合併（`fbea06a`，含 `e5b720d`）：無衝突，`ja-cost.rs` 的 `enumerate` 去掉多餘 lifetime。
  `cargo clippy --workspace --all-targets -- -D warnings` **exit 0**；test 建置仍被 gemm-f16 擋住（同上）。

- `rerank-gating` 第二次合併（`f681ed4`，含 `b5d0e26`）：無衝突，`ja/decode.rs` 3 行，詞群入選後照文本順序送模型
  （修 left context 的選取順序）。clippy exit 0；test 建置仍被 gemm-f16 擋住。

- `jinen-eval`（合併 `e2d6cfb`）：無衝突，只新增 `docs/reports/jinen-ajimee-2026-11-20.md`（190 行）與
  `scripts/eval_jinen_ajimee.py`（390 行），沒有 Rust 改動。`cargo check --workspace --all-targets` exit 0。

  AJIMEE 200 題（accept 判定，全題無文脈）：Jinen v2 xsmall **68.5%**、small **77.5%**；本引擎（Mozc v2 pack）**46.0%**、
  Google 日本語入力 **54.0%**、AzooKey 統計轉換器 44.5%。延遲 p50：xsmall **28.68 ms**、small **212.49 ms**（p95 65.63／725.47 ms）。
  給 64 字文脈：xsmall **+10 個百分點**（59→69）、small **+5 個百分點**（72→77）。

- `mozkey-study`（合併 `764ee82`）：無衝突，只新增 `docs/reports/mozkey-study-2026-11-20.md`（530 行），沒有程式改動。
  `cargo check --workspace --all-targets` exit 0。

  報告重點：
  - prompt 是條件欄位格式：`U+EE02 左文脈 [U+EE07 右文脈] [U+EE03 profile] [U+EE04 topic] [U+EE05 style] [U+EE06 settings] U+EE00 片假名讀音 U+EE01`，
    左文脈一定出現（可為空），其餘欄位非空才加。
  - scorer 走 **native-endian packed struct**，不是 JSON。
  - 推論確定性：`temperature=0.0`、`top_k=1`、`top_p=1.0`、`n_predict=64`、`cache_prompt=true`。
  - 五條採用條件中，**讀音保持預設 OFF**，context echo、首尾截斷、絵文字抑制、自動封鎖預設 ON。
  - zenz-v3.2-xsmall 用這份 prompt 跑 AJIMEE 前 20 題：exact **10/20**，暖機後 p50 **24 ms**（GB10、4 threads）。

  備註：main 工作區出現未追蹤的 `.cargo/config.toml`（aarch64 加 `-C target-feature=+fullfp16`，解 gemm-f16 編譯），
  不在本 branch 內，沒有 commit。

- `ja-particle-rewrite` 第二次合併（`c04e85e`，含 `aa5797f`）：`eval_cmd.rs` 自動合併，無衝突。
  mixed golden 的期望更新成 `私は`（6/6，原本 4/6）；`jc-engine/src/loader.rs` 載入日文解碼器時開啟 `with_particle_rewrite(true)`，
  補上先前回報的缺口（CLI 與 engine 現在一致）。`docs/port/mixed-e2e.md` 同步更新。

  gate：`cargo clippy --workspace --all-targets -- -D warnings` **exit 0**；
  `cargo test --workspace --no-fail-fast` **exit 0**（41 targets、945 passed、0 failed）。全量測試至此全綠。

  備註：main 的 `e868e4e` 已把 `.cargo/config.toml` 的 `+fullfp16` 收進版控，gemm-f16 在 aarch64 可以正常編譯。

- `jinen-integrate`（合併 `72985f6`，含 `34688d7`）：無衝突。新增 `crates/jc-cli/src/bin/jinen-eval.rs`（586 行）、
  `crates/jc-model/src/candle/jinen.rs`（337 行，用 candle 載入 Jinen v2 xsmall GGUF），`Cargo.lock` 帶入 candle 相關依賴。

  gate：`cargo clippy --workspace --all-targets -- -D warnings` **exit 0**；
  `cargo test --workspace --no-fail-fast` **exit 0**（42 targets、950 passed、0 failed）。

  Rust 版 Jinen xsmall 對照 Python：
  - `ajimee-200` accept **68.0%（136/200）**，Python 68.5%（137/200），差 −0.5pp。
  - `modern-500` top-1 **81.4%（407/500）**，Python 81.6%（408/500），差 −0.2pp。
  - `hard-cases` top-1 **77.1%（27/35）**，與 Python 相同。
  - 延遲 p50：ajimee **28.59 ms**、modern-500 **19.69 ms**、hard-cases **19.16 ms**（4 threads）。

  備註：main 另有 `254f4b4`（收錄 rerank-domain 報告），不是本輪合併帶入。

- `quant-study`（合併 `8e27af0`）：無衝突。新增 `docs/reports/quant-study-2026-11-20.md`（180 行）與四份 JSON、
  `scripts/quant/`（量測與量化腳本、`dump-inputs` 小 crate），`.gitignore` 加一行。沒有動既有 Rust 程式。
  `cargo check --workspace --all-targets` exit 0。

  建議（作者結論）：
  - 日文 reranker 用 **int8**：排序與 fp32 一致率 100%、Spearman 0.991，延遲 107.4 → 31.9 ms。
  - 日文標點用 **int8**：argmax 一致率 99.44%，端到端 93% 句子完全相同，38.5 → 12.1 ms。
  - 注音選字**維持 fp32**：對量化最敏感，int8 的 logits 最大誤差 10.3、Spearman 0.859、候選 top-1 只剩 92.3%。
  - 外部 **Jinen Q5_K_M 最划算**（28.3 MB，品質與延遲平衡）。
  - **int4 在 ARM 上反而變慢**：reranker 由 int8 的 31.9 ms 升到 **70.8 ms**（模型 18.5 MB，但 kernel 走慢路徑）。
  - 另記：reranker 目前讓端到端變差（modern-500 `--rerank off` 86.0%，開啟後更低），維持 opt-in。

- `candle-punct`（合併 `f9b2e69`，含 `9705bb5`，另 `f3249c7` 補 fmt）：`Cargo.toml`、`Cargo.lock`、
  `jc-model/src/candle/mod.rs`、`lib.rs` 有衝突。解法定成「共用 encoder、兩個模型並存」：
  - `candle/mod.rs` 以 candle-punct 的共用 `Encoder`／`Layer`（`Option<&Tensor>` 遮罩）為底，補回 reranker 的
    `position_ids`、`CandleConfig::for_spec`／`pool_slot`、`CandleReranker`（改用共用 `Encoder` 的 forward），以及 `pub mod jinen;`。
  - 環境變數拆開：punct 用 `JC_PUNCT_WEIGHTS`／`JC_PUNCT_BACKEND`，reranker 用 `JC_RERANK_WEIGHTS`／`JC_RERANK_BACKEND`；
    新增 `weights_path_with_env`，`rerank.rs` 改用它。
  - `Cargo.lock` 以 main 版為底，`Cargo.toml` 同時保留 `candle-transformers`／`tokenizers`（Jinen）與 `rayon`。

  gate：`cargo clippy --workspace --all-targets -- -D warnings` **exit 0**；
  `cargo test --workspace --no-fail-fast` **exit 0**（43 targets、952 passed、0 failed）。

  量測（作者）：candle 標點與 ORT fp32 的 logits **max|Δ| 2.289e-5**、argmax **51200/51200 一致**；
  端到端文字 **5099/5099 字元一致（100%）**、**500/500 句整句相同**。延遲：
  - 256 token（視窗上限）：candle 61.25 ms vs ORT fp32 17.51 ms，約 **3.5 倍**。
  - 13 token（IME 短句）：candle 7.3–10.2 倍慢（1.29–1.43 ms 對比 11.8–13.2 ms 的整段流程）。
  結論：candle 只適合 reranker 這類長序列，短序列標點不划算，**保留 `ort` 為預設**。

- `jinen-gating`（合併 `e610808`，含 `f71d5cc`）：無衝突，新增 `crates/jc-cli/src/bin/jinen-gate.rs`（1332 行）與
  `docs/port/jinen-gating.md`。

  gate：`cargo test --workspace --no-fail-fast` **exit 0**（44 targets、958 passed、0 failed）；
  `cargo clippy --workspace --all-targets -- -D warnings` **exit 101**，2 個錯都在 `jinen-gate.rs`，需回 jinen-gating：
  - `jinen-gate.rs:413` `clippy::int_plus_one`：`overlap >= span * 2 + 1` 應改 `overlap > span * 2`。
  - `jinen-gate.rs:672` `clippy::manual_is_multiple_of`：`hash % 2 == 0` 應改 `hash.is_multiple_of(2)`。

  三組結果（統計 → Jinen → 混合，括號為 Jinen 呼叫率與混合 p50）：

  | 題庫 | 統計 | Jinen | 混合 | Jinen 呼叫率 | 混合 p50 |
  |---|---:|---:|---:|---:|---:|
  | modern-500 | 86.0% | 81.4% | **90.4%** | 20.0% | 4.33 ms |
  | hard-cases | 54.3% | 77.1% | **71.4%** | 34.3% | 3.04 ms |
  | ajimee-200 | 46.0% | 68.0% | **64.0%** | 56.0% | 20.66 ms |

  方法：門檻只用輸入側訊號，並只在每組題庫的 dev 半邊（題號 FNV-1a 雜湊 50/50 切分）挑選，holdout 半邊只回報、
  不參與選擇；dev 上要求「驗收 + 4pp」安全邊際，取 Jinen 呼叫率最低的組合。

- `session-core-split`（合併 `22831f0`，含 `63b81e7`）：無衝突。`session.rs` 拆成 `session/mod.rs`（1390 行降到 44 行的差量）
  與 `session/core.rs`（1317 行），欄位改 `pub(super)`，沒有改名或增刪欄位，行為不變。

  gate：`cargo fmt --all -- --check` exit 0、`cargo test --workspace --no-fail-fast` **exit 0**（44 targets、958 passed、0 failed）。
  `cargo clippy --workspace --all-targets -- -D warnings` 仍 exit 101，但兩個錯都在 `jinen-gate.rs`（jinen-gating 帶入，
  與本 branch 無關）。

  實測：`eval zh-golden` **45/45**、top-1 100.0%；`eval ja --corpus modern-500` top-1 **86.0%**、top-5 98.0%、p50 3.085 ms。

  欄位分組預定表（Stage 1–5）記在 `docs/port/session-core-split.md`，列出每個 `Session` 欄位的主群組、其他讀取者與備註，
  後續拆檔照這張表走。

- `jinen-gating` 第二次合併（`64f49e8`，含 `1c7ba21`）：無衝突，`jinen-gate.rs` 2 行（`overlap > span * 2`、
  `hash.is_multiple_of(2)`）。
  `cargo clippy --workspace --all-targets -- -D warnings` **exit 0**；
  `cargo test --workspace --no-fail-fast` **exit 0**（44 targets、958 passed、0 failed）。先前 2 個 clippy 錯已解除。

- `session-g3-symbols`（合併 `86d24e0`，含 `c67c4e7`）：無衝突。新增 `crates/jc-core/src/session/symbols.rs`（250 行：`GROUPS`、`UNITS`、
  `home`、`alternatives`、提示列），`session/mod.rs` 1 行接上分頁／最近分頁。

  gate：`cargo fmt --all -- --check` exit 0、`cargo clippy --workspace --all-targets -- -D warnings` exit 0、
  `cargo test --workspace --no-fail-fast` **exit 0**（44 targets、964 passed、0 failed）。
  實測：`eval zh-golden` **45/45**；`eval ja --corpus modern-500` top-1 **86.0%**、top-5 98.0%、p50 3.173 ms。

  deferred 清單（等下游群組補齊，資料層已就位）：
  - **`↓` 同組變體**：在字面符號上列出同組變體並替換，依賴 `core` 的字面區間游標與編輯（`selectLiteral`／`literalBounds`／`literalStep`）。
  - **`↑` 符號定位**：組字中在符號上開選單並定位（`openSymbolMenuAt`），依賴 `core` 游標位置與替換區間、`segments` 候選呈現。
  - **格子移動**：`↑↓←→` 依候選欄位移動、跨頁（`gridMove`、`candidateColumns`），依賴 `segments` 的 `CandidateColumns`／`displayColumns`。
  - **片語分頁**：「我的片語」分頁、標籤截斷與換行（`phrases`、`phraseLabel`），依賴 `misc`／`personal` 的片語。
  - **標點寬度**：所有符號鍵打半形（`all_symbols.go`、`halfwidthShift`），依賴 `input_style` 的標點寬度設定與 idle keys。

- `session-g2-segments`（合併 `1fd343d`，含 `57e3a5d`）：`session/mod.rs` 衝突（`symbols` 與 `segments` 兩個 mod 宣告），
  照兩邊都保留解：`mod segments;`、`pub mod symbols;`、`pub use segments::tone_mark_keys;`。
  新增 `crates/jc-core/src/session/segments.rs`，`core.rs` 補鎖定與近音候選接線。

  gate：fmt exit 0、clippy exit 0、`cargo test --workspace --no-fail-fast` **exit 0**（44 targets、976 passed、0 failed）。
  實測：`eval zh-golden` **45/45**；`eval ja modern-500` **86.0%**、top-5 98.0%、p50 3.177 ms。

  5 個 n/a 都是協定缺口（依任務書先回報、不改協定）：
  - `KeyCode` 缺 `Down`／`Up`／`Home`／`End`／`PageUp`／`PageDown`，`Snapshot` 缺 `navigating`／候選範圍／頁碼 → `candidate_mode.go` 無法移植。
  - `Snapshot` 缺 `suggestion` 欄位 → `phonetic_hint.go` 無法移植（直接改組字字面會讓 Enter 誤提交提示）。
  - `Candidate`／`Snapshot` 缺 `label` 欄位 → `candidate_labels.go` 無法移植。
  - `Snapshot` 沒有游標 rune（只有字元游標），`ZhInput` 也沒有組字中游標 → `segments.go` 的 `SetCursor` 標 n/a。
  - `ZhInput::press_char` 收到聲調鍵就直接完成音節，沒有「擋下聲調、留在暫存」的 hook → `tone_check.go` 無法移植。

- `session-g1-input`（合併 `ded267a`，含 `45a73fa`）：`session/mod.rs` 兩處衝突。`mod` 宣告與 `pub use` 取聯集
  （`input_style`＋`segments`＋`symbols`）；`press` 的派送順序解成
  `!style_key && !menu_key` 外層、`segments_press` 內層，讓聲調鍵（g2）先於一般按鍵路徑處理，
  同時保留 g1 的 `style_key` 攔截。新增 `session/input_style.rs`，`core.rs` 的數字選字改走 `selection_index`。

  gate：fmt exit 0、clippy exit 0、`cargo test --workspace --no-fail-fast` exit 101（44 targets、1001 passed、**2 failed**）：
  - `session::input_style::tests::test_mcbopomofo_space_and_backslash`（`input_style.rs:835`）：期望 `景 `、得到 `井 `。
  - `session::input_style::tests::test_microsoft_width_is_state_and_includes_english`（`input_style.rs:735`）：`A` 得到 `ａＡ`、期望 `Ａ`。
  這兩者在 `session-g1-input` 分支單獨 checkout 後重跑也一樣失敗（已用暫存 worktree 驗證），
  是該 task 自帶的紅測試，不是合併造成，需回 session-g1-input 修。

  實測：`eval zh-golden` **45/45**；`eval ja modern-500` **86.0%**、top-5 98.0%、p50 3.114 ms。

  5 ported（`input_style.go` 部分移植、`shift_symbol.go` 部分、`keypolicy.go`、`idle_keys.go`、`selection_keys.go`），
  4 n/a：`zhuyin_keys.go`（缺逐格來源與 literal 格陣列）、`zh_profile.go`（沒有 profile 抽象層）、
  `layout_readings.go`（只支援大千排列，無 `AltCost`）、`number_context.go`（解碼器沒有跨片段左文參數）。

- `proto-extend`（合併 `a60766b`，含 `43c1741`）：`session/mod.rs` 兩處衝突。`press` 的 match 保留 main 的
  `ArrowRight`／`ArrowLeft`／`Char` 分支並補 `_ => {}`（新鍵暫不處理）；`snapshot` 同時輸出 `cursor_rune` 與
  main 版的 `segments()` 視圖。

  gate：fmt exit 0、clippy exit 0、`cargo test --workspace --no-fail-fast` exit 101（44 targets、1015 passed、**2 failed**），
  兩個失敗仍是 `session-g1-input` 自帶的 `test_mcbopomofo_space_and_backslash` 與
  `test_microsoft_width_is_state_and_includes_english`，與本 branch 無關。
  實測：`eval zh-golden` **45/45**；`eval ja modern-500` **86.0%**、p50 3.203 ms。

  協定變更：
  - `PROTOCOL_VERSION` 升到 **2**（`crates/jc-proto/src/lib.rs`）。
  - `Snapshot` 新增 `suggestion`／`label`／`navigating`／`cursor_rune`；前三者這一輪暫為空值（`Snapshot::default()`），
    只有 `cursor_rune` 真的填了組字游標的 rune 位置。
  - `KeyCode` 新增 `Down`／`Up`／`Home`／`End`／`PageUp`／`PageDown`／`ShiftTab`；session 目前以 `_ => {}` 忽略，
    行為不變，等後續候選選取任務接上。

- `session-g1-input` 第二次合併（`f442cd4`，含 `4a493c3`）：無衝突，`input_style.rs` 修兩個測試（同音詞成本、全形英文改成累加預覽）。
  fmt exit 0、clippy exit 0、`cargo test --workspace --no-fail-fast` **exit 0**（44 targets、1017 passed、0 failed）。
  實測：`eval zh-golden` **45/45**；`eval ja modern-500` **86.0%**、p50 3.107 ms。全量測試回到全綠。

- `candle-quant`（合併 `1972ae1`，含 `3f5b493`、`1580558`、`51f459c`）：`Cargo.toml`、`candle/mod.rs`、`candle/punct.rs`、
  `lib.rs`、`punctuation_model.rs` 有衝突。解法：candle 三個檔案與 `punctuation_model.rs` 採 branch 版（量化 Layer／Encoder 泛型、
  `punct_weights_path`、Q8_0/Q5K/Q4K 載入），再把 Jinen 接回：`pub mod jinen;` 與 `pub use candle::jinen::{Generation, Jinen};`；
  `rerank.rs` 改用 branch 的 `weights_path`／`WEIGHTS_ENV`（原本我加的 `weights_path_with_env` 已成私有）。

  gate：fmt exit 0；`cargo test --workspace --no-fail-fast` exit 101（45 targets、1021 passed、**1 failed**）；
  `cargo clippy --workspace --all-targets -- -D warnings` exit 101（**1 個錯**）。兩者都在 branch 自己的檔案：
  - `candle::quant::tests::quantized_levels_stay_close_to_fp32`（`candle/quant.rs:791`）：Q8_0 −0.79020256 vs fp32 −0.8029184，
    差 0.0127 超過測試容許值。
  - `candle/mod.rs:360` `Layer::new` 8 個參數，`clippy::too_many_arguments`。

  量測（作者）：
  - reranker **Q8_0**：35.2 MiB，100 次推論後 RSS **117 MiB**（fp32 300 MiB，**−61%**），逐對排序 **99.77%**，
    端到端 modern-500 逐題與 fp32 相同（500/500）。
  - Q5K 23.2 MiB／RSS 98.9 MiB，逐對 99.43% 但整組順序只有 **95.0%**；Q4K 19.2 MiB／92.3 MiB，整組順序 **90.5%**。
  - 標點 Q8_0 字元一致 **99.65%**，Q5K／Q4K 掉到 99.22%／97.40%。
  - 出貨建議：**Q8_0**（reranker 與標點都用），Q5K／Q4K 只當空間極限時的備案。

- `candle-quant` 第二次合併（`77d8c1f`，含 `66347f4`）：無衝突。`Layer::new` 的 4 個 norm tensor 收成 2 個 tuple，
  解掉 `too_many_arguments`；量化測試門檻依實測差距調整（Q8_0 容許值放寬到實測誤差之上）。
  fmt exit 0、clippy exit 0、`cargo test --workspace --no-fail-fast` **exit 0**（45 targets、1022 passed、0 failed）。
  先前 1 個 test 與 1 個 clippy 錯全部解除，全量測試回到全綠。

- `session-g4-mixed`（合併 `8de9306`，含 `6b9f1b5`）：無衝突。混合狀態搬進 `session/mixed.rs`（944 行），
  `mixed.rs` 只留 `Router` 與英文詞表，`core.rs` 的混打重建改呼叫新模組；`mod.rs` 加 `ja_run`／`ja_prev_mode`。

  gate：fmt exit 0、clippy exit 0、`cargo test --workspace --no-fail-fast` **exit 0**（45 targets、1029 passed、0 failed）。
  實測：`eval mixed` **6/6**；`eval zh-golden` **45/45**；`eval ja modern-500` **86.0%**、p50 3.100 ms。

  n/a 清單與理由：
  - `core/session/language.go` → n/a：Rust 用 `Mode`＋`SessionConfig`，沒有可插拔的 `lang.Profile`／`languageRuntime`，
    `SetLanguage`／`InputMethod` 也沒有產品介面。
  - `core/session/ja_cross.go` → n/a：日英同形需要語言 runtime 的 clause 與候選清單，目前 `Mode::Ja` 直接顯示最佳轉換。
  - `core/session/pronoun.go` → 模型已在 `personal/pronoun.rs`，但 Session 沒有 `personal::Model` 與 `path`／`hint` 顯示路徑，
    等 rerank／misc 群組接線。

- `session-g5-misc`（合併 `7379047`，含 `12f161f`）：`session/mod.rs` 只有 mod 宣告衝突，取聯集（`pub mod misc;`、`mod mixed;`）。
  新增 `session/misc.rs`（待定字面資料層、組字加詞、本地證據、文件文脈查詢、更正選取）。

  gate：fmt exit 0、clippy exit 0、`cargo test --workspace --no-fail-fast` **exit 0**（45 targets、1044 passed、0 failed）。
  實測：`eval mixed` **6/6**；`eval zh-golden` **45/45**；`eval ja modern-500` **86.0%**、p50 3.113 ms。

  deferred 清單（資料層已就位，缺下游能力）：
  - **`ZhInput` 逐格編輯**：`insertSyllableLiteral`／`restorePendingLiteral`／`retypeSyllableLiteral`／`clearPendingZhuyin`；
    目前 `ZhInput` 只有尾端 push／pop，沒有依索引插入或移除音節的 API。
  - **`Snapshot` 標示／notice**：`markState`／`markSegments`／候選窗反悔的 notice；`Snapshot` 沒有 `Notices`／`StyleMarked`，
    `Candidate` 也沒有來源 ID，批次復原與重開候選無處可放。
  - **`Decoder` 個人化成本接線**：`local_evidence` 已能建出證據，但 `decode::bias` 的 `reading_lift` 與 `correction_penalty`
    仍是回 0 的掛鉤，`Decoder` 沒有依音節位置查證據的評分器；文件前文混分也缺 `Context`／`ContextID` 協定欄位。

- `session-g6-core`（合併 `b7af358`，含 `ffccac1`）：無衝突。`core.rs` 加 406 行（候選選取模式、Esc 優先權、
  快照補全標示），`mod.rs` 配合調整按鍵派送。

  gate：fmt exit 0、clippy exit 0、`cargo test --workspace --no-fail-fast` **exit 0**（45 targets、1057 passed、0 failed）。
  實測：`eval mixed` **6/6**、`eval zh-golden` **45/45**、`eval ja modern-500` **86.0%**（p50 3.080 ms）、
  `eval fuzzy` 注音 **44.5%**（319 題，doubled-key 100%、missing-key 5.4%）。

  deferred 清單：
  - **組字游標**：←／→／Home／End 移動、任意位置插入與刪除、`eraseBeforeCursor`、`shiftLocks`；缺 `ZhInput`／`RomajiInput`
    的游標 API 與 `Segment` 的 rune 範圍。
  - **換詞長**：Tab／上下鍵的 `nextCandidateLength`／`candidateAnchor` 與跨詞長 `moveHighlight`；候選清單目前只有整段組字一份。
  - **`j`／`k` 換字**：`stepCandidate`／`unitStart`／`unitEnd`，依賴組字游標與分詞範圍。
  - **選字鎖定**：選字後鎖定片段並留在組字區；Rust 目前直接送進待提交區，改成鎖定會動到四組題庫的既有輸出。
  - **`keepCandidates`**：候選窗跟著之後的組字留在畫面；會讓數字鍵在開窗後變成選字鍵，與現有題庫流程衝突。
  - **`related` label**：需要已提交左文的最後一個完整詞與文件前文權重。

- `session-g4-mixed` 第二次合併（`fb4e7ab`，含 `6b76ce1`）：無衝突，`session/mixed.rs` 21 行，混合顯示測試改用注入的英文詞表。
  `cargo clippy --workspace --all-targets -- -D warnings` exit 0；`cargo test --workspace --no-fail-fast` **exit 0**
  （45 targets、1057 passed、0 failed）。

  根因：該測試原本靠 `JC_TESTDATA` 的真實英文詞表，CI 沒有資料時走注音路徑，`display_pending("hi")` 就變成「ㄘㄛ」，
  測試在不同環境得到不同結果。改成在測試內注入詞表後，不再依賴外部資料。

- `session-backfill`（合併 `610498c`，含 `ade1479`）：無衝突。補回 G2 的五個 n/a：新按鍵（`Down`／`Up`／`PageUp`／`PageDown`／`Home`／`End`）、
  組字游標（`cursor_rune`）、`navigating`／`suggestion`／`label` 三個 Snapshot 欄位、擋聲調（`tone_blocked`）。
  另有 owned paths 外的必要修改：`SessionConfig::zhuyin_error_hint`、`ToneGate` 改 `Arc` 閉包、`ZhInput` 手寫 `Debug`。`PROTOCOL_VERSION` 未變。

  gate：fmt exit 0、clippy exit 0、`cargo test --workspace --no-fail-fast` **exit 0**（45 targets、1070 passed、0 failed）。
  實測：`eval mixed` **6/6**、`eval zh-golden` **45/45**、`eval ja modern-500` **86.0%**（p50 3.281 ms）、`eval fuzzy` 注音 **44.5%**（319 題）。

  仍 n/a／部分：`displayed.go` 的失效模式在本埠不存在（`Lattice::build` 每個音節都有未知詞邊）；
  `phonetic_hint` 的同音替換排名需要 `decode.Window` 與 `SuggestionEdit`；`keepCandidates` 重開、候選格線與 `j`／`k` 換詞仍缺狀態；
  `related` label 需要 bigram，目前 `build-zh`／`build-zh-bigram` 的資料差異不影響測試。

- `drop-punct`（合併 `dacb52b`，含 `859f6d1`）：無衝突。移除標點模型推論路徑：`punct.rs`、`punct_plan.rs`、`punct_zh.rs`、
  `punctuation_model.rs`、`candle/punct.rs`、`probe.rs` 全數刪除或縮減（−2935 行），`manifest`／`role`／`runtime` 同步調整，
  `candle/quant` 只留 reranker 需要的部分；保留 `ctrl_punct` 與 `punct_mark`。

  gate：fmt exit 0、clippy exit 0、`cargo test --workspace --no-fail-fast` **exit 0**（43 targets、1046 passed、0 failed）。
  實測：`eval zh-golden` **45/45**；`eval ja modern-500` **86.0%**、p50 3.097 ms。

  殘留檢查：`grep -rn 'SharedPunctuator\|punctuate\|--punct' crates/ | grep -v ctrl_punct` 為 **0 筆**；
  `ctrl_punct` 與 `punct_mark` 仍保留（`session/mod.rs:110`、`:116`）。

- `jinen-gating-v2`（合併 `83152fc`，含 `81e0cbe`）：無衝突，只改 `jinen-gate` bin（+1047／−191）與 `docs/port/jinen-gating-v2.md`。

  gate：`cargo test --workspace --no-fail-fast` **exit 0**（43 targets、1050 passed、0 failed）；
  `cargo clippy --workspace --all-targets -- -D warnings` **exit 101**，1 個錯：
  `crates/jc-cli/src/bin/jinen-gate.rs:1376` `clippy::unnecessary_sort_by`（應改 `sort_by_key`），需回 jinen-gating-v2 修。

  品質優先版（gap < 460 改用 Jinen **small**）：
  | 題庫 | 前一版 | small 版 | 驗收 | 增益 | Jinen 呼叫率 |
  |---|---:|---:|---:|---:|---:|
  | modern-500 | 90.4% | **93.6%** | 86.0 | +3.2 | 30% |
  | hard-cases | 71.4% | **80.0%** | 71.4 | +8.6 | 54% |
  | ajimee-200 | 64.0% | **71.5%** | 64.0 | +7.5 | 74% |

  延遲：small 在空閒機器 p50 44–61 ms（排除線 150 ms）；品質優先版混合 p50 **4.76／22.74／55.60 ms**、
  p95 58／47.2／197.9 ms（modern／hard／ajimee）。ajimee 的 74% 呼叫率把 p50 推到 55 ms，是這個方案唯一明顯代價。

- `candle-zh`（合併 `d8da98f`，含 `131f606`）：`cli.rs`、`candle/mod.rs` 自動合併。新增 `candle/zh.rs`（1578 行）與
  `candle-zh-check` bin，`zh/select.rs` 加 116 行支援 candle 後端。合併時 branch 的 `load_weights` helper 被自動合併吃掉，
  已補回（`d85610d`，屬解衝突收尾，沒有新行為）。

  gate：fmt exit 0、`cargo test --workspace --no-fail-fast` **exit 0**（44 targets、1055 passed、0 failed）。
  `cargo clippy --workspace --all-targets -- -D warnings` 仍 exit 101，唯一錯誤是 `jinen-gate.rs:1376` 的
  `unnecessary_sort_by`（jinen-gating-v2 待修），與本 branch 無關。

  實測（`eval zh-golden`）：
  - `--backend candle`（預設 fp32 safetensors）：**45/45**，推論 p50 14.00 ms。
  - `--backend candle` ＋ `JC_ZH_SELECT_WEIGHTS=model.q8_0.gguf`：**45/45**，推論 p50 **10.73 ms**（作者量 9.36 ms，同機重跑有 1–2 ms 波動）。
  - `--backend ort`：**45/45**，這台環境報 `select-model：沒有推論`（ORT 版模型未安裝），不影響 golden。

  作者量測（Q8_0）：golden **45/45**、逐對排序一致 **99.51%**（5439/5466）、單次推論 p50 **9.36 ms**（fp32 14.05 ms）、
  常駐 RSS **45.3 MiB**、端到端峰值 RSS **74.7 MiB**（ORT fp32 309.7 MiB）。
  結論：注音選字在 candle 上比 ORT 更快也更省記憶體（Q8_0 p50 9.4–9.8 ms 對 ORT 11.8–12.6 ms）。

- `proto-compat`（合併 `06b4d5b`，含 `a622acd`）：主 checkout 先前有一份未追蹤的 `docs/proto.md`，先以
  `diff <(git show proto-compat:docs/proto.md) docs/proto.md` 確認只有第 133 行不同（branch 多一句「改線格式前先更新
  `crates/jc-proto/tests/fixtures/` 的手寫 fixture」），確認 branch 是超集後刪掉未追蹤檔，merge 才沒有被擋下。

  gate：fmt exit 0、`cargo test --workspace --no-fail-fast` **exit 0**（45 targets、1068 passed、0 failed）。
  clippy 仍只有 `jinen-gate.rs:1376` 的 `unnecessary_sort_by`（jinen-gating-v2 待修）。

  內容：`crates/jc-proto/tests/fixtures/` 共 **11 個手寫 JSON**（6 個 request、5 個 response）；新增 `tests/fixtures.rs`，
  涵蓋各訊息往返、**v1 形狀可解析**（舊欄位缺漏仍能解），以及 **PROTOCOL_VERSION 與 fixture 宣告的版本號一致性守門**。
  不改任何欄位名稱或型別。

## 2026-11-20 11:23 六條收尾合併與最終 gate

依序合併（每條合完都跑 `cargo check --workspace --all-targets`，全部 exit 0）：

| Branch | Commit | Merge commit | 結果 |
|---|---|---|---|
| `wrapper-spec` | `78ab884` | `df70788` | docs 831 行（`docs/wrapper.md`、README、architecture），無衝突 |
| `client-crates` | `183cd6c` | `2965c71` | 新 crate `jc-windows`／`jc-linux`／`jc-mac` 與 members，無衝突 |
| `named-pipe` | `67ab620` | `a402047` | `jc-engine` 加 `pipe_windows.rs`，無衝突 |
| `jinen-default` | `c39546c` | `881f8b0` | `cli.rs` 衝突：`reject_ja_flags` 加 `neural` 參數，兩邊語意都保留 |
| `personal-improve` | `d3d0202` | `5d6a90e` | 與 jinen 的 neural 路徑並存（rollback／skip 都在），無衝突 |
| `proto-candidates` | `e86e4b0` | `f333f72` | 協定升 v3（State 加 `details`／`segments`／`highlighted`） |
| `fp16-flag` | `bfd2d2a` | `d5147ec` | `.cargo/config.toml` 改 `+fp16`，unknown feature 警告消失 |
| `jinen-default`（修測試） | `56d98a8` | `19c6db5` | ZWJ cluster 與半截讀音重複呼叫修正 |

整合修正（不屬任何 branch 的實作）：
- `68b370f`：`eval zh-golden` 恢復接受 `--backend`（jinen-default 的 reject 蓋掉 candle-zh 的語意）。
- `f6466a7`：`jc-windows/tests/client.rs` 的 `State` fixture 補上 v3 新欄位。

最終 gate（HEAD `19c6db5`）：
- `cargo fmt --all -- --check` exit 0。
- `cargo clippy --workspace --all-targets -- -D warnings` **exit 0**，`unknown and unstable feature` 警告 0 筆。
- `cargo test --workspace --no-fail-fast` **exit 0**（52 targets、**1118 passed、0 failed**）。
- `cargo test -p jc-proto` 全過（v1／v2 形狀仍可解析）；`PROTOCOL_VERSION = 3` 與 `docs/proto.md`（v3 凍結）一致；
  `jc-windows` 握手全用 `jc_proto::PROTOCOL_VERSION`，沒有寫死。

題庫：
| 題庫 | 目標 | 實測 |
|---|---|---|
| `zh-golden`（預設 ort） | 45/45 | **45/45** |
| `zh-golden --backend candle` | 45/45 | **45/45** |
| `ja modern-500` | 92.0% | **93.6%**（top-5 98.4%、p50 11.4 ms） |
| `ja hard-cases` | 80.0% | **80.0%**（top-5 94.3%、p50 19.7 ms） |
| `ja ajimee-200` | 71.0% | **71.0%**（top-5 79.5%、timeout 24.5%） |
| `fuzzy` 注音 | 44.5% | **44.5%**（319 題） |
| `mixed` | 6/6 | **6/6** |

全綠後 push `origin main`。

## 2026-11-20 11:55 gating-lowcall（預設低呼叫率門檻）

- 合併 `gating-lowcall` `a350ef8` → `f8efb1a`。`session/core.rs` 衝突兩邊都保留：jinen-default 的
  「半截讀音不呼叫模型」guard＋本 branch 的 `mean_candidates` 門檻；`NeuralConfig` 預設 `gap_threshold = 472.5`、
  `min_mean_candidates = 5.98`，`--neural` 改成 `auto`／`quality`／`off` 三值。
- gate：fmt exit 0、clippy exit 0、`cargo test --workspace --no-fail-fast` **exit 0**（52 targets、1126 passed、0 failed）；
  `zh-golden` **45/45**。
- **auto：92.2／77.1／69.0**；離線掃描 69.5，差 1 題逾時（`ajimee-200` neural timeout 1，回到統計結果；
  任務書下限為 69.0，判為達標，不調門檻）。
- 對照 `quality`：93.6／80.0／71.0；neural 逾時次數 0／0／1。
- 延遲（auto，本機重跑）：modern-500 p50 6.9／p95 60.6 ms、hard-cases 7.9／58.4 ms、ajimee-200 41.9／174.9 ms；
  三組合計 p95 依任務書為 112.7 ms（本輪未另行合併取樣覆核）。

## 2026-11-20 11:52 docs-sync 與 settings-wire

- `docs-sync` `39ff3a0` → merge `24a6d5b`：只改文件（`architecture`、`models`、`status`、`wrapper`、`docs/port/README.md`，582 增／273 刪），
  同步到 M0–M4 與 v3 協定。`cargo check --workspace --all-targets` exit 0。
- `settings-wire` `2f08daa` → merge `7713748`：新增 `jc-engine/src/config.rs`（482 行）與 `tests/settings.rs`，
  引擎讀 `settings.json` 轉 `SessionConfig`，CLI 參數優先、壞檔只警告不擋啟動。

  gate：fmt exit 0、clippy exit 0、`cargo test --workspace --no-fail-fast` **exit 0**（53 targets、1137 passed、0 failed）。
  第一次跑有 3 個 `jc-engine/tests/settings.rs` 的 `啟動 jc-engine: NotFound`，重跑即消失，屬首次建置時 bin 尚未產生的競態。

  題庫（與前一版相同）：`zh-golden` **45/45**、`ja modern-500` **92.2%**（第一次受負載影響量到 90.8／timeout 2.8%，
  重跑回到 92.2／0%）、`hard-cases` **77.1%**、`ajimee-200` **69.0%**（一次負載下量到 67.0，重跑 69.0）、
  `fuzzy` **44.5%**、`mixed` **6/6**。

## 2026-11-20 12:15 drop-ort 之後：data-pack、english-personal、tsf-skeleton、candidate-pos、composition-cursor、settings-ui

依序合併（每條合完都跑 `cargo check --workspace --all-targets`）：

| Branch | Commit | Merge commit | 備註 |
|---|---|---|---|
| `drop-ort` | `9ba142f` | `04c4be8` | 移除 ONNX Runtime：`crates/` 已無 `onnxruntime` 與 `--backend ort`（`--backend` 只接受 `candle`）；`scripts/legacy/` 保留歸檔腳本 |
| `data-pack` | `c076115` | `38d882e` | 新增資料 manifest 格式與 `data manifest`／`data verify`；`jc-engine` 啟動驗存在與大小 |
| `english-personal` | `08db287` | `7d73e3e` | 英文拼字修正接個人化學習 |
| `tsf-skeleton` | `5b31564` | `9d59c64` | 新 crate `jc-tsf`（cdylib）；Windows 實作、Linux 空殼，Windows 編譯由 CI 負責 |
| `candidate-pos` | `c75debd` | `d02d3a7` | 日文候選帶 POS；只加欄位傳遞，沒有動排序或成本（比對 `core.rs` diff 只有 `with_pos`／`clear_pos`） |
| `composition-cursor` | `631c384` | `714f9f8` | 組字游標、任意位置插入刪除、換詞長、片段換字、keepCandidates |
| `settings-ui` | `941a6e1` | `8b8c0ae` | 新 crate `jc-settings-ui`（GPUI Kit），帶入大量 `Cargo.lock` 變更 |

整合修正：`6fa6e62`（candidate-pos 的 `Convert` fixture 移除 drop-ort 已刪的 `backend` 欄位）。

資料驗證：`data manifest /mnt/work/jc/jc-ime-data --out /tmp/opencode/jc-data-manifest.json` 產生 9 個檔案索引，
`data verify` **9/9 通過、0 錯誤、0 警告**（packs、build-zh、english、jinen small GGUF、zh-select、zh-punct）。

### 目前阻塞

1. `cargo clippy -D warnings` 失敗：`crates/jc-core/src/session/core.rs:692` `clippy::collapsible_if`，來自 composition-cursor。
2. `cargo test --workspace` 2 個失敗，皆已確認在各自 branch 單獨重跑也失敗（不是合併造成）：
   - `session::tests::test_english_without_learning_keeps_existing_order`（english-personal）：`hello` 的候選數 3，期望 1。
   - `session::segments::tests::test_manual_segments_expire_on_next_key`（composition-cursor）：手動片段過期後數量 2，期望 1。
3. `jc-settings-ui` 的**測試**在這台機器無法連結：缺 `libxcb`／`libxkbcommon` 開發套件（`ld: cannot find -lxcb`）；
   `cargo check` 需要在環境加 `RUST_FONTCONFIG_DLOPEN=1`（系統沒有 `fontconfig.pc`）。這是環境限制，不是程式錯。

### 題庫（本機，load average 14–18）

| 項目 | 目標 | 實測 |
|---|---|---|
| `zh-golden` | 45/45 | **45/45** |
| `ja modern-500 --neural quality` | 93.6% | **93.6%** |
| `ja modern-500 --neural off` | 86.0% | **86.0%** |
| `fuzzy` 注音 | 44.5% | **44.5%** |
| `mixed` | 6/6 | **6/6** |
| `ja modern-500` auto | 92.2%（機器空閒時） | 負載高時 89.8%，依指示本輪不當門檻 |

### 待辦（不要現在做）

- **legal 帳本 L-21**（已完成）：`legal/README.md` 的 L-21 原本標成評估用、暫不散布；
  `legal-update` 已改散布欄位，同步 NOTICE、`docs/models.md` 與 manifest 對照表。
- **settings-ui 兩個已知缺口**：`--neural` 三態與 `rerank` 在 UI 是唯讀，因為 `Settings` 沒有對應欄位；
  「重新啟動引擎」要等安裝器提供。
- **tsf-skeleton 的 14 項未實機驗證清單**在 `docs/port/tsf-skeleton.md`，不要當成已驗證。
- deal with `jc-settings-ui` 的 Linux 連結依賴（`libxcb`、`libxkbcommon`、`fontconfig` 或 `RUST_FONTCONFIG_DLOPEN=1`）在 CI 映像補齊。

## 2026-11-20 12:40 tsf-smoke、ja-live-neural、tsf-candidates 與 CI

| Branch | Commit | Merge commit | 備註 |
|---|---|---|---|
| `ci-ui-deps` | `0504818` | `a3cddee` | CI 的 linux job 加裝 X11／Wayland／字型開發套件（已 push） |
| `tsf-smoke` | `7a77e73` | `9bd36dd` | 新 crate `jc-tsf-smoke`（免管理員冒煙程式）與 `jc-tsf` 的 `test-export` feature |
| `ja-live-neural` | `7438238` | `3d12ddf` | 逐字変換共用 Jinen 門檻；結論是逐字路徑**不建議預設開啟**：逐鍵 p95 < 200 ms，但每題累積 p95 313 ms、呼叫數 27 倍 |
| `tsf-candidates` | `6bc0a99` | `6a99421` | `ITfCandidateListUIElement` 候選窗與按鍵轉換 |

驗證：
- `RUST_FONTCONFIG_DLOPEN=1 cargo check --workspace --all-targets` exit 0；`cargo check -p jc-tsf -p jc-tsf-smoke --target x86_64-pc-windows-msvc` exit 0。
- Windows 目標一律改用 `--target x86_64-pc-windows-msvc`（本機已安裝該 target），不再退回 gnu。
- `ja modern-500 --neural quality` **93.6%**、`--neural off` **86.0%**，預設路徑不變；`--live` 才走新的逐字路徑。
- clippy 仍是紅的，兩個錯都來自既有 branch，需各自修：
  - `crates/jc-core/src/session/core.rs:692` `clippy::collapsible_if`（composition-cursor）。
  - `crates/jc-core/src/ja/live.rs:790` `clippy::type_complexity`（ja-live-neural，只在 lib test）。
- `cargo test --workspace` 仍有 2 個既有失敗（english-personal、composition-cursor），與本輪無關。

`tsf-candidates` 的已知缺陷（candidate-paging 會修，先不改）：
- 引擎預設 `max_candidates` 與 Windows 候選窗每頁 `PAGE_SIZE = 9` 不一致（`jc-engine/src/config.rs` 走 `DEFAULT_MAX_CANDIDATES`，
  `jc-core` 的預設是 5，設定檔的 `candidate_page_size` 預設 10；TSF 固定 9）。
- `Session::select_by_digit` 只看 `selection_index(ch)`，沒有把目前頁碼加回去，所以第 2 頁以後用數字鍵選字會選到第 1 頁的候選。

### CI（push `a3cddee`，run 44673374135）

- linux (core)：**failure**，失敗步驟是 `Clippy`（就是上面 `collapsible_if`，當時的 HEAD 已含 composition-cursor）。
- windows (msvc)：build 成功，`Test` 失敗，原因與本機相同：`test_manual_segments_expire_on_next_key`
  與 `test_english_without_learning_keeps_existing_order`（628 passed、2 failed）。

### 待辦：Jinen tokenizer 未列入 manifest

`models/quant/jinen-v2-xsmall/tokenizer.json` 是 Jinen 執行時必要檔（小模型沿用 xsmall 的 tokenizer），
但目前不在資料 manifest 內；出貨前要補 `shipped: true`，否則 `data verify` 不會涵蓋它。

### 待辦：tsf-installer 三個已知缺口（branch `f4f4ae1`，排在 legal-update 之後合）

- **x86 TIP 未做**：只有 64 位元 TIP，32 位元程式無法使用這個輸入法。
- **授權文件未打進 MSI**：安裝包內沒有 `LICENSE`／`NOTICE`／`legal/` 內容。
- **引擎啟動沒有監督程式**：靠 `HKLM\...\Run` 拉起，掛掉不會自動重啟。

## 2026-11-20 13:05 佇列合併：integration-fix → settings-ui-testfix → candidate-paging → data-download → legal-update → tsf-installer

| Branch | Commit | Merge commit |
|---|---|---|
| `integration-fix` | `a80461f` | `ed3ccd7` |
| `settings-ui-testfix` | `f01c016` | `d2f6b03` |
| `candidate-paging` | `61fd8ff` | `3b55f95` |
| `data-download` | `f1ed410` | `67961a6` |
| `legal-update` | `873918a` | `1c79412` |
| `tsf-installer` | `f4f4ae1` | `16373c2` |

- `legal-update` 對 `docs/port/data-pack.md`、`docs/port/ja-models-fetch.md`、`docs/integration-log.md` 的修改已逐行核對，
  只是把「僅本機評估」改成可散布的敘述，沒有動資料或程式。
- `tsf-installer` 的 `installer/` 不參與 cargo build：workspace member 未變，`cargo check --workspace --all-targets` exit 0。
- `integration-fix` 讓 jc-core 測試回到全綠（`cargo test -p jc-core --lib` 640 passed、0 failed）。

### gate（本機）

- `cargo fmt --all -- --check` exit 0。
- `RUST_FONTCONFIG_DLOPEN=1 cargo check --workspace --all-targets` exit 0；`cargo check -p jc-tsf -p jc-tsf-smoke --target x86_64-pc-windows-msvc` exit 0。
- `cargo clippy -D warnings` **失敗**，兩個錯仍待原 branch 修：
  - `crates/jc-core/src/session/core.rs:692` `collapsible_if`（composition-cursor）。
  - `crates/jc-core/src/ja/live.rs:790` `type_complexity`（ja-live-neural，僅 lib test）。
- `cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：**8 failed**（`jc-settings-ui` 本機無法連結，依指示只用 CI 驗）。
  已在各自 worktree 單獨重跑確認是 branch 自帶，不是合併造成：
  - `candidate-paging`（5 個）：`jc-proto` 的 `response_state_minimal_accepts_version_one_shape`、
    `response_state_v2_round_trips_all_new_fields`、`response_state_v3_round_trips_structured_fields`、
    `response_state_v3_pos_round_trips`（fixture 有 `page_size: 9`，序列化時被 `skip_serializing_if` 省略），
    以及 `session::input_style::tests::test_selection_key_columns`（5 vs 4）。
  - `data-download`（3 個）：`download::tests::present_requires_size_and_hash`、
    `different_size_is_size_mismatch`、`same_size_with_different_bytes_is_hash_mismatch`。

### 題庫（本機）

`zh-golden` **45/45**、`ja modern-500 --neural quality` **93.6%**、`--neural off` **86.0%**、`fuzzy` **44.5%**、`mixed` **6/6**。

## 2026-11-20 13:30 最後一次 gate（candidate-paging 修好後）

- `candidate-paging` `40426bb` → merge `bc3f91a`：`page_size` 從協定移除、改成核心公開常數（14 檔、+71／−199），
  proto fixture 與 `test_selection_key_columns` 都回到綠。
- `RUST_FONTCONFIG_DLOPEN=1 cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：**exit 0**（54 targets、1201 passed、0 failed）。
- `RUST_FONTCONFIG_DLOPEN=1 cargo clippy --workspace --all-targets -- -D warnings`：exit 0。
- `cargo fmt --all -- --check`：exit 0。
- 題庫：`zh-golden` **45/45**、`ja modern-500 --neural quality` **93.6%**、`--neural off` **86.0%**、`fuzzy` **44.5%**、`mixed` **6/6**。
- `jc-settings-ui` 測試本機缺 `libxcb`／`libxkbcommon`，依指示只由 CI 驗。

### CI（push `b21eafe`，run 44680265069）

- linux (core)：**success**（clippy 與測試都過，含 `jc-settings-ui` 的連結與測試）。
- windows (msvc)：**success**（build 與 test 都過）。
- 沒有需要重試或回報的失敗測試。

## 2026-11-20 13:45 補合 tsf-polish 與 engine-supervisor

- `tsf-polish` `639dc66` → merge `ef1f493`：組字用 `ITfDisplayAttributeProvider` 畫虛線底線、候選窗滑鼠點選走
  `ITfCandidateListUIElementBehavior`。協定未改；點選是模擬上／下鍵加 Space／Escape。
- `engine-supervisor` `95b2484` → merge `164d9f2`：`jc-engine --supervise`（Local mutex 單一實例、崩潰重啟、退避、
  健康檢查、環狀 log）。

gate：`cargo test --workspace --no-fail-fast --exclude jc-settings-ui` **exit 0**（54 targets、**1211 passed、0 failed**）；
clippy exit 0；fmt exit 0。題庫：zh-golden **45/45**、quality **93.6%**、off **86.0%**、fuzzy **44.5%**、mixed **6/6**。
`git branch --no-merged main` 已無任何分支。

待辦：
- 協定 v3 沒有「指定索引選字」，之後可加；現在由模擬按鍵達成，不影響功能。
- `pipe` 名稱 `jc-ime` 是機器級共用，多使用者同時登入會衝突；要改成含 SID 或 session id（jc-tsf／引擎後續）。

### CI（push `aa3dc6d`，run 44683332668）

- linux (core)：success，06:13 總時長約 6 分 18 秒（05:47:31–05:53:49）。
- windows (msvc)：success，約 13 分 12 秒（05:47:31–06:00:43）。

## 2026-11-20 14:10 ci-speed 與後續六條

### ci-speed（`d43cb9e` → `3b48e55`，push 後 run 44685683929）

改動：windows job 不再 `cargo build --release`（只 `cargo test`）、兩個 job 加 Cargo registry／git 快取、`CARGO_INCREMENTAL=0`。
YAML 用 `python3 -c "yaml.safe_load(...)"` 驗過。

CI 耗時對照：
| Job | 改前（run 44683332668） | 改後（run 44685683929） |
|---|---|---|
| linux (core) | 6 分 18 秒 | **6 分 09 秒** |
| windows (msvc) | 13 分 12 秒 | **7 分 38 秒** |

兩個 job 都 success；windows 少掉一次 release 建置，時間減約 42%。

### 合併清單（依序）

| Branch | Commit | Merge commit |
|---|---|---|
| `candidate-click` | `122a6f8` | `e00ce7c` |
| `installer-payload` | `7582804` | `ba3c783` |
| `manifest-tokenizer` | `0a77ae0` | `80f0334` |
| `settings-neural` | `26fd626` | `040713e` |
| `pipe-name` | `eeeec1d` | `f747861` |
| `os-isolation` | `17de715` | `2d92d81` |
| `release-flow` | `d8433d0` | `2027435` |

- `manifest-tokenizer` 與 `installer-payload` 同改 `installer/jc-ime.wxs`：合併時兩邊都保留，`legal\` 目錄與
  `models/quant/jinen-v2-xsmall/tokenizer.json` 都在同一份 payload 清單（WXS 內可 grep 到兩者）。
- `installer/check-payload.py` 需要 `target/installer/payload`，本機沒有產生 payload，無法完整跑；WXS 條目已用 grep 核對。
- `os-isolation` 與 `ci-speed` 的 `ci.yml` 衝突兩邊都保留：快取＋只跑 debug test，另加
  `cargo check --workspace --all-targets --exclude jc-cli --exclude jc-settings-ui --target x86_64-pc-windows-msvc` 守門。
- `release-flow` 的 `installer/build.ps1` 只有檔頭說明衝突，合併保留「使用或產生 manifest」與「授權文件進 payload」兩句。

### 最後 gate（HEAD `2027435`）

- `RUST_FONTCONFIG_DLOPEN=1 cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：**exit 0**（54 targets、1231 passed、0 failed）。
- clippy exit 0；fmt exit 0。
- `cargo check --workspace --all-targets --target x86_64-pc-windows-msvc`：**本機失敗**，`ring v0.17.14` 的 build script 需要 MSVC 工具鏈；
  依指示不算阻擋，Windows 目標由 CI 的 windows (msvc) job 驗。
- 題庫：zh-golden **45/45**、quality **93.6%**、off **86.0%**、fuzzy **44.5%**、mixed **6/6**。
- `git branch --no-merged main`：**空**。

### CI（push `ebf9774`，run 44687853660）

- linux (core)：success；windows (msvc)：success。

## 2026-11-20 14:35 release-flow 追加（`5135a8e` → `0bf50d1`）

內容：資料包 SHA-256 驗證、Hugging Face 託管文件、`scripts/data-bundle.sh`。
gate：test exit 0（54 targets、1231 passed、0 failed）、clippy exit 0、fmt exit 0；
題庫 zh-golden **45/45**、quality **93.6%**、off **86.0%**、fuzzy **44.5%**、mixed **6/6**；
`git branch --no-merged main` 空。

### CI（push `fbb76f0`，run 44689671075）

- linux (core)：success；windows (msvc)：success。

### CI（push `b2a1f87`，run 44691123049）

- linux (core)：success；windows (msvc)：success（只有文件變更，作為最後一次確認）。

## 2026-11-20 14:55 x86-tip（`a5583c9` → `d5c65a1`）

內容：新增 i686 TSF 元件與雙架構註冊／反安裝；改成 x64／x86 共用 CLSID，兩個機碼檢視各指一份 DLL；
冒煙程式同時檢查 64 與 32 位元機碼檢視。

- `cargo check -p jc-tsf --target x86_64-pc-windows-msvc`：exit 0。
- `cargo check -p jc-tsf --target i686-pc-windows-msvc`：exit 0（本機已裝 i686 target）。
- `RUST_FONTCONFIG_DLOPEN=1 cargo check --workspace --all-targets`：exit 0。
- `cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：exit 0（54 targets、1231 passed、0 failed）。
- `git branch --no-merged main`：rc-prep（等它完成後再合）。

## 2026-11-20 15:05 rc-prep（`ff4c1c1` → `ac080e5`）

內容：workspace 版本 `0.1.0-rc.1`、CHANGELOG、`docs/status.md`、README、`docs/wrapper.md`、
`docs/port/README.md` 索引（含 x86-tip）。

- `RUST_FONTCONFIG_DLOPEN=1 cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：exit 0（54 targets、1231 passed、0 failed）。
- `RUST_FONTCONFIG_DLOPEN=1 cargo clippy --workspace --all-targets -- -D warnings`：exit 0。
- `cargo fmt --all -- --check`：exit 0。
- `git branch --no-merged main`：空。
- `cargo check --workspace --all-targets --target x86_64-pc-windows-msvc`：本機失敗（`ring v0.17.14` build script 需要 MSVC 工具鏈），
  留給 CI 的 windows (msvc) job 驗。

### CI（push `4bbf535`，run 44698221828）

- linux (core)：success；windows (msvc)：success。

## 2026-11-20 15:40 composition-model（帶入 live-default）與 ci-cache

| Branch | Commit | Merge commit |
|---|---|---|
| `composition-model`（含 `live-default` 三個 commit） | `b919e1b` | `c0b1689` |
| `ci-cache` | `95f6ed4` | `9b45f1e` |

- `composition-model`：收斂成語言無關的 Auto／User 模型與轉換後端，並把日文預設改成延遲整句轉換（`ja_conversion = live`）；
  `docs/status.md` 只衝突里程碑一行，取兩邊資訊合併（auto 92.2／quality 93.6，hard-cases 80.0／ajimee 71.0）。
- `ci-cache`：`Swatinem/rust-cache` 分組，`ci-linux`／`ci-windows`、`release-linux`（verify 與 linux 共用）／`release-windows`；
  兩份 workflow 都通過 `python3 -c "yaml.safe_load(...)"`。

### gate（composition-model 合併後）

- `cargo check --workspace --all-targets` exit 0、fmt exit 0。
- `cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：**20 failed**（54 targets、1245 passed）。
- 已在 `composition-model` 的 worktree 單獨重跑：`jc-core` 19 failed、`jc-settings` 1 failed，**全部是 branch 自帶**，
  不是合併造成，需回 composition-model 修。清單：
  - `composition::tests::shift_units_moves_user_mark_after_edit_point`
  - `composition::tests::user_segment_splits_runs_and_appears_in_context`
  - `ja::live::tests::test_candidates_and_invalid_span`
  - `session::backends::tests::only_auto_runs_are_sent`
  - `session::mixed::tests::test_japanese_run_infers_from_romaji`
  - `session::segments::tests::test_fuzzy_near_reading_is_not_blocked`
  - `session::tests::test_ja_learning_disabled_keeps_statistical_order`
  - `session::tests::test_ja_learning_ranks_manual_choice_first_and_undo_restores`
  - `session::tests::test_ja_live_candidates_include_all_kana_scripts`
  - `session::tests::test_ja_live_cursor_and_segment_candidates`
  - `session::tests::test_ja_live_neural_adopts_complete_reading_only`
  - `session::tests::test_ja_neural_not_called_for_incomplete_reading`
  - `session::tests::test_ja_neural_replaces_when_gap_below_threshold`
  - `session::tests::test_ja_neural_skips_when_gap_above_threshold`
  - `session::tests::test_ja_neural_skips_when_mean_candidates_below_min`
  - `session::tests::test_ja_neural_timeout_keeps_statistical_result`
  - `session::tests::test_ja_neural_unavailable_keeps_statistical_result`
  - `session::tests::test_ja_passive_learning_keeps_clear_statistical_winner`
  - `session::tests::test_ja_short_reading_records_evidence_only`
  - `jc-settings`：`features_tests::test_bool_settings_defaults_and_persist`

## 2026-11-20 16:20 composition-model 修正（`3360141` → `bafeb8c`）

- `release-msi-fix` 的 `2aa57b1` 已在 `394bc3b` 合入，`git merge-base --is-ancestor` 確認包含。
- `ja-script`、`selection-pin` 兩條舊分支已刪（`selection-pin` 先移除 `/mnt/work/jc/wt/selection-pin` worktree）；
  `git branch --no-merged main` 現在是空的。

gate：
- `cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：**1 failed**（54 targets、1264 passed）：
  `jc-settings` 的 `features_tests::test_bool_settings_defaults_and_persist`，
  `ja-live-conversion 預設`：實際 `Some("on")`、期望 `Some("off")`。這是 live-default 把預設改成 ON 後，測試期望沒同步，
  屬 branch 自帶（已在 composition-model worktree 單獨重現）。
- `cargo clippy --workspace --all-targets -- -D warnings`：**2 個錯**，都在 `crates/jc-core/src/ja/live.rs`：
  `:343` `derivable_impls`（`impl Default for Trigger` 可 derive）、`:1093` `extend_with_drain`（`extend(self.tail.drain(..))` 應改 `append`）。
- fmt exit 0。

題庫（本機）：`zh-golden` **45/45**、`ja auto` **92.2%**、`quality` **93.6%**、`--neural off` **86.0%**、
`fuzzy` **44.5%**、`mixed` **6/6**。

## 2026-11-20 16:45 composition-model 收尾（`60ef215` → `55923ec`）

`ja-live-conversion` 預設改成 `on`，`jc-settings` 測試期望同步。

- `cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：**exit 0**（54 targets、1265 passed、0 failed）。
  第一次跑有 1 個 flake：`jc-engine` 的 `supervise::tests::second_supervisor_is_rejected_by_lock`（`supervise.rs:941`，
  `釋放後應能再取得: AlreadyRunning { name: "jc-ime-test-lock-…" }`），單獨重跑與第二次全量都通過，判定為鎖檔時序 flake，已記錄。
- clippy exit 0；fmt exit 0。
- 題庫：zh-golden **45/45**、`auto` **92.2%**、`quality` **93.6%**、off **86.0%**、fuzzy **44.5%**、mixed **6/6**。
- `git branch --no-merged main`：空。

## 2026-11-20 17:10 composition-model 收尾、toolchain-pin、release-msi-fix 與固定工具鏈 gate

| Branch | Commit | Merge commit |
|---|---|---|
| `composition-model`（needless_bool 修正） | `669eda8` | `ab12342` |
| `toolchain-pin` | `f16bacf` | `99e88d2` |
| `release-msi-fix` | `2aa57b1` | `394bc3b`（先前已合，ancestor 已確認） |

`toolchain-pin`：新增 `rust-toolchain.toml`（1.100.0、rustfmt／clippy），`ci.yml` 與 `release.yml` 的工具鏈都固定 1.100.0。
本機 `rustc --version` 已是 `1.100.0`，與 CI 一致。

gate（固定工具鏈）：
- `RUST_FONTCONFIG_DLOPEN=1 cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：**exit 0**（54 targets、1265 passed、0 failed）。
- clippy exit 0；fmt exit 0。
- 題庫：zh-golden **45/45**、`auto` **92.2%**、`quality` **93.6%**、off **86.0%**、fuzzy **44.5%**、mixed **6/6**。
- `git branch --no-merged main`：空。

### CI（push `63d4b17`，run 44742981033）

- linux (core)：success；windows (msvc)：success（固定 1.100.0 後，`needless_bool` 不再出現）。

## 2026-11-20 17:25 0.1.0-rc.2（`450d62d` → `59fadd0`）

- `Cargo.toml` 版本 `0.1.0-rc.2`，CHANGELOG 的「未發佈」改成 0.1.0-rc.2 段落。
- gate：test exit 0（54 targets、1265 passed、0 failed）、clippy exit 0、fmt exit 0。
- 題庫：zh-golden **45/45**、`auto` **92.2%**、`quality` **93.6%**、off **86.0%**、fuzzy **44.5%**、mixed **6/6**。
- `git branch --no-merged main`：空。

### CI（push `28c93a6`，run 44744661803；`0.1.0-rc.2`）

- linux (core)：success；windows (msvc)：success。

## 2026-11-20 17:50 mixed-latin（`a660183` → `e3dbb1d`）

- 修正 `split_cmd` 的 `redundant_closure` 與 `ja/live.rs` 的 `needless_bool`。`ja/live.rs` 的 `should_convert` 有衝突，
  取 main 已重構的版本（`reached_min_units || (boundary && is_boundary())`，語意等價且已是 clippy-clean）。
- gate：test exit 0（54 targets、1286 passed、0 failed）、clippy exit 0、fmt exit 0。
- 題庫（DP 切分預設關閉）：zh-golden **45/45**、ja **92.2%**、fuzzy **44.5%**、mixed **6/6**。
- `git branch --no-merged main`：空。

### CI（push `9cc570a`，run 44750225315）

- linux (core)：success；windows (msvc)：success。

## 2026-11-20 18:05 release-msi-fix（`253b451` → `c54afd4`）

- `1c578a3`：安裝器改用 cdylib 實際產物 `jc_tsf.dll`，payload 內改名。
- `253b451`：`crt-static` 移到 `.cargo/config.toml`（x86_64／i686 兩個 target），release workflow 移除 Windows 重複建置；
  與先前 `+fp16` 設定放在同一份檔案，避免 RUSTFLAGS 變動造成重編。
- 驗證：`ci.yml`／`release.yml` YAML 有效；`cargo check --workspace --all-targets` exit 0；
  `cargo test --workspace --no-fail-fast --exclude jc-settings-ui` exit 0（54 targets、1286 passed）。

## 2026-11-20 18:15 0.1.0-rc.3（`71879e6` → `92186b0`）

- `Cargo.toml` 版本 `0.1.0-rc.3`，CHANGELOG 補 MSI 檔名與建置時間修正；`Cargo.lock` 另以 `bed2967` 同步。
- gate：test exit 0（54 targets、1286 passed）、clippy exit 0、fmt exit 0。
- 題庫：zh-golden **45/45**、ja **92.2%**、fuzzy **44.5%**、mixed **6/6**。

## 2026-11-20 18:20 deps-audit（`ff39e10`）

移除 2 個沒用的直接依賴（`log`、`sha2`）與 1 個 feature，其餘只是報告。

- `cargo check --workspace --all-targets` exit 0；`cargo clippy --workspace --all-targets -- -D warnings` exit 0。
- 沒有行為改動，依指示不重跑題庫。
- `git branch --no-merged main`：空。

## 2026-11-20 18:35 release-msi-fix（UTF-8）與 0.1.0-rc.4

- `release-msi-fix` `2254346` → `b904012`：Windows Python 輸出改 UTF-8（`build.ps1`、`check-payload.py`）。
- `rc4` `0af7223` → `d3348c2`：版本 `0.1.0-rc.4`，CHANGELOG 補編碼修正；`Cargo.lock` 另以 `chore` commit 同步。
- gate：test exit 0（54 targets、1286 passed）、clippy exit 0、fmt exit 0。依指示不重跑題庫（只改安裝器與文件）。

## 2026-11-21 00:20 0.1.0-rc.5 與 WiX Bitness 修正（`283a0b7` → `ec6a986`、`49f23d9` → `b757c8a`）

- `rc5` `283a0b7` → `ec6a986`：版本 `0.1.0-rc.5`、CHANGELOG 補 WiX v5 說明；`Cargo.lock` 以 `67b29c2` 同步。
- `release-msi-fix` `49f23d9` → `b757c8a`：`installer/jc-ime.wxs` 的 x86 元件由 WiX v3 的 `Win64="no"` 改為
  `Bitness="always32"`（WiX v5 寫法），另更新 `installer/README.md`、`docs/port/tsf-installer.md`、`docs/port/x86-tip.md`。
  先前的 rc.5 CHANGELOG 已提前描述此修正，現在程式與文件一致。
- gate：`cargo check --workspace --all-targets` exit 0、clippy exit 0、fmt exit 0；`release.yml`／`ci.yml` YAML 有效。
- 依指示只改安裝器與文件，未重跑題庫。
- `git branch --no-merged main`：空。

### CI（push `3f2b343`，run 44771900689）

- linux (core)：success；windows (msvc)：success。

## 2026-11-21 02:20 0.1.0-rc.6 與 TSF 註冊存取違規修正（`7b43b83` → `6e1633a`、`4da4f08` → `b7588eb`）

背景：rc.5 真機安裝時自訂動作 `RegisterTip` 以 `0xC0000005` 結束。bisect 顯示 `Register` 與
`AddLanguageProfile` 都成功寫入機碼，但分類（CategoryMgr）沒寫、ACL 也沒改，崩點在 CategoryMgr 段。

| Branch | Commit | Merge commit |
|---|---|---|
| `register-crash`（路徑／圖示指標＋安裝日誌） | `2694dbe` | `6e1633a` |
| `register-crash`（跳過 AppContainer ACL、細分 CategoryMgr 日誌） | `7b43b83` | `6e1633a` |
| `rc6` | `4da4f08` | `b7588eb` |

- `crates/jc-tsf/src/register.rs`：修正 `module_path()` 取得的 DLL 路徑與圖示指標型別；分類註冊日誌細分。
- `installer/jc-tip-register`：新增 `log.rs`，ACL 寫入移出關鍵路徑（預設跳過 AppContainer ACL）。
- `Cargo.lock` 以 `42b6eaa` 同步到 `0.1.0-rc.6`（`installer/jc-tip-register` 是獨立 workspace，版本仍 `0.1.0`，不隨 workspace 升版）。

gate：
- `cargo check --workspace --all-targets` exit 0；clippy exit 0；fmt exit 0。
- `cargo test`（`installer/jc-tip-register`，獨立 crate，不在 workspace 內）：**15 passed、0 failed**。
- 補驗 Windows 目標：`cargo check -p jc-tsf --target x86_64-pc-windows-msvc` 與
  `cargo check --target x86_64-pc-windows-msvc`（tip-register）都 exit 0。
- 依指示只改安裝器與 TSF 註冊程式，未重跑題庫。
- `git branch --no-merged main`：空。

### CI（push `ad9038d`，run 44791173536；`0.1.0-rc.6`）

- linux (core)：success；windows (msvc)：success。

## 2026-11-21 02:40 0.1.0-rc.7 與 TSF 註冊 v2（`c6c9729` → `d36700c`、`53f2664` → `4fe3dc4`）

背景：rc.5／rc.6 真機 `RegisterTip` 的 `0xC0000005` 崩在 `RegisterCategory`——64 位元檢視的 CLSID 在
自訂動作執行時還不存在。這輪改成分工：CLSID 由 MSI 寫，helper 只註冊 profile 與分類。

| Branch | Commit | Merge commit |
|---|---|---|
| `tsf-reg-v2` | `c6c9729` | `d36700c` |
| `rc7` | `53f2664` | `4fe3dc4` |

- `installer/jc-ime.wxs`：兩個 TSF DLL 各掛一個 `<Class>`（由 `WriteRegistryValues` 寫 `InprocServer32`）；
  `RegisterTip` 改排在其後（`After="WriteRegistryValues"`），custom action 不再帶 DLL 路徑。
- `installer/jc-tip-register`：`machine-*` 不收參數、改用 `ITfInputProcessorProfileMgr`；刪除 `arch.rs`、
  `write_com_registration`、`register_in_store`、`Target`、`delete_clsid_tree`（約 900 行跨架構註冊碼）。
- `user-uninstall` 不再呼叫 `InstallLayoutOrTip(ILOT_UNINSTALL)`（避免 Windows 把系統地區改成英文，windows-chewing-tsf#553）。
- 驗證：WXS XML 有效、`<Class>` 共 2 個；`RegisterTip` 位於 `WriteRegistryValues` 之後；
  `ILOT_UNINSTALL` 只出現在註解，程式碼不再呼叫。
- `Cargo.lock` 以 `3da9da9` 同步到 `0.1.0-rc.7`。

gate：
- `cargo check --workspace --all-targets` exit 0；clippy exit 0；fmt exit 0。
- `cargo test`（`installer/jc-tip-register`，獨立 crate）：**9 passed、0 failed**（`arch.rs` 刪除後測試數由 15 降為 9）。
- 補驗 Windows 目標：`cargo check -p jc-tsf -p jc-tsf-smoke --target x86_64-pc-windows-msvc` 與 tip-register 同 target 都 exit 0。
- 依指示未重跑題庫；`rc.6` 跳過不發。
- `git branch --no-merged main`：空。

### CI（push `f20b5ab`，run 44794084291；`0.1.0-rc.7`）

- linux (core)：success；windows (msvc)：success。

## 2026-11-21 02:50 tsf-reg-v2 後續：`UnregisterTip` 排序修正（`a153e95` → `add9f10`）

`<Class Advertise="no">` 由 WiX 展開成 Registry table 的資料列，CLSID 機碼是
`WriteRegistryValues`（2500）寫、`RemoveRegistryValues`（2600）刪；`RegisterClassInfo`／
`UnregisterClassInfo`（2700）只處理 Class table，不參與 TSF 的 CLSID 機碼。

- 原本 `UnregisterTip` 排 `Before="RemoveFiles"`（3500），執行時 CLSID 已被 2600 刪掉；
  rc.5 未出事是因為當時 CLSID 由 helper 自己寫、MSI 不擁有。
- 改為 `Before="RemoveRegistryValues"`，`RollbackUnregisterTip` 仍排在其前。
- 同步修正 WXS／`installer/README.md`／`docs/port/tsf-installer.md`／`docs/port/x86-tip.md`／helper 文件裡
  「CLSID 由 `RegisterClassInfo` 寫」的說法（已用 WiX IR 確認）。

gate（版本不變，仍 `0.1.0-rc.7`）：
- `cargo check --workspace --all-targets` exit 0；clippy exit 0；fmt exit 0。
- `cargo test`（`installer/jc-tip-register`）：**9 passed、0 failed**。
- WXS XML 有效；Windows 目標 `cargo check -p jc-tsf -p jc-tsf-smoke --target x86_64-pc-windows-msvc`
  與 tip-register 同 target 都 exit 0。
- `git branch --no-merged main`：空。

### CI（push `2f980d8`，run 44795803352）

- linux (core)：success；windows (msvc)：success。

## 2026-11-21 03:05 mixed-latin-final（`112566c` → `432f9a7`）

修 `mixed-split` 曝光的路由 bug：`split_token` 會把拉丁 run 尾端的 `.` `,` `;` 當英文標點剝掉，
但在大千鍵盤它們是韻母 ㄡ ㄝ ㄤ，`5k42. g4n0 vul3` 因此只解出 `5k42. 是三小`。

- 新增 `split_latin_run`：剝標點前先問切分器，條件是「最後片段是注音、結束在 run 結尾、
  起點在標點之前」。只看整段能否解析不夠（`hello.` 也會解析成 `hello` 加獨立 ㄡ）。
- `mixed_latin_split` 關閉時直接走原本的 `split_token`，行為完全不變。

gate：
- `cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：**exit 0**（54 targets、1289 passed、0 failed）。
- clippy exit 0；fmt exit 0。
- 題庫（與合併前逐項相同，無回歸）：zh-golden **45/45**、ja modern-500 **92.2%**、fuzzy **44.5%**、mixed **6/6**。
- `git branch --no-merged main`：空。

### CI（push `9ed1873`，run 44800443513）

- linux (core)：success；windows (msvc)：success。

## 2026-11-21 03:20 profile-enabled（`9f6b9a7` → `a759984`）

真機問題：`HKLM\SOFTWARE\Microsoft\CTF\TIP\{CLSID}\LanguageProfile\0x00000404\{PROFILE}\Enable`
是 0，使用者手動加入鍵盤後切不過去。

- `installer/jc-tip-register/src/platform.rs`：`RegisterProfile` 的 `bEnabledByDefault` 由 `false` 改 `true`
  （寫入 `Enable=1`；仍不呼叫 `ActivateProfile`，不改系統預設輸入法）。
- `installer/jc-tip-register/src/log.rs`：安裝日誌第一次建立時先寫 UTF-8 BOM（以
  `metadata().len() == 0` 判斷），PowerShell 5.1 讀中文不再亂碼；後續附加不重複寫 BOM。
- 文件 `docs/port/tsf-installer.md`、`docs/port/tsf-skeleton.md` 同步成新行為。

gate：
- `cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：**exit 0**（54 targets、1289 passed、0 failed）。
- clippy exit 0；fmt exit 0。
- 題庫（無變動）：zh-golden **45/45**、ja modern-500 **92.2%**、fuzzy **44.5%**、mixed **6/6**。
- Windows 目標：`cargo check --target x86_64-pc-windows-msvc`（`installer/jc-tip-register`，獨立 crate）exit 0。
- `cargo test`（`installer/jc-tip-register`）：9 passed。
- `git branch --no-merged main`：`segment-diff`（新分支，待後續指示）。

### CI（push `7bbd64c`，run 44805931861；profile-enabled）

- linux (core)：success；windows (msvc)：success。

## 2026-11-21 03:35 activate-diagnose（`8632d51` → `b7773e5`）

針對 `ITfTextInputProcessor::Activate` 回 `0x80070057` 的定位工具，純診斷加上一處確定的 HINSTANCE 修正，
不改 TSF 行為。

- `crates/jc-tsf/src/composition.rs`：`create_message_window` 的 `hInstance` 由 `GetModuleHandleW(None)`（宿主 exe）
  改為 `GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | ...UNCHANGED_REFCOUNT)` 取得 DLL 自己；
  `RegisterClassW` 遇 1410 `ERROR_CLASS_ALREADY_EXISTS` 視為成功；`advise` 逐步記 HRESULT。
- `crates/jc-tsf/src/lib.rs`：`Activate` 開始／完成／失敗各一行日誌；`crates/jc-tsf/src/logging.rs` 新增 `hresult` 格式化。
- `crates/jc-tsf-smoke/src/checks.rs`：檢查 5 失敗時提示 `JC_TSF_LOG=1` 與日誌路徑。
- `docs/port/activate-diagnose.md`：逐項結論（`WINDOW_STYLE(0)` 合法、`client_id` 正確、`AdviseSink` IID 正確，
  根因只剩三處）；成功路徑用 `debug`、失敗用 `warn`。

gate：
- `cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：**exit 0**（54 targets、1289 passed、0 failed）。
- clippy exit 0；fmt exit 0。
- `cargo check -p jc-tsf -p jc-tsf-smoke --all-targets --target x86_64-pc-windows-msvc` exit 0。
- 題庫：zh-golden **45/45**、ja modern-500 **92.2%**、fuzzy **44.5%**、mixed **6/6**。
- `git branch --no-merged main`：`segment-diff`、`real-corpus`（新分支，待指示）。

## 2026-11-21 04:40 segment-diff、real-corpus 與量測工具修正（`72426ab`、`0058651`）

| Branch | Commit | Merge commit |
|---|---|---|
| `segment-diff` | `f619b46` | `72426ab` |
| `real-corpus`（rebase 後 tip，parent=`72426ab`） | `2a3e53c` | `0058651` |

兩支都只新增 `jc-cli` 量測子命令（`mixed-diff`、`corpus-diff`），不改 `mixed.rs`／`mixed_latin.rs`。

### 四個測試期望的事實修正（逐筆對照按鍵表／資料檔）

| 測試 | 舊值 | 新值 | 依據 |
|---|---|---|---|
| `zh_keys("ㄐㄧㄣ")` | `fup ` | `rup ` | `jc-core/src/zh/keyboard.rs:52/62/74`：`r=ㄐ, u=ㄧ, p=ㄣ`；同檔 `:219` `("ㄑㄧㄣ","fup ")` 證明 `f` 是 ㄑ、尾空白是預設一聲鍵 |
| `zh_toneless_keys("ㄐㄧㄣ")` | `fup` | `rup` | 同一張按鍵表，去掉聲調鍵後為 `r+u+p` |
| `convert_mixed(..., false)` | `watashiwa284ru418 desu` | `watashiwa284ru18desu` | `:45/65/81/52/62/40`：`2=ㄉ, 8=ㄚ, 4=ˋ, r=ㄐ, u=ㄧ, 1=ㄅ`；`ㄉㄚˋ="284"`、`ㄐㄧㄅㄚ="ru18"` |
| `convert_mixed(..., true)` | `watashiwa28ru418desu` | `watashiwa28ru18desu` | 同上，toneless 應去掉 `4`；`kana_to_romaji("わたしは")` 舊 `watashiha` → 新 `wataciha`：`ja/input.rs:149/150/301` し 有 `si`/`shi`/`ci`，reverse table 取最短 `ci`（2 字，字母序小於 `si`）|

第五筆（同一測試的 `kana_to_romaji`）也列在上面；共修正 5 個期望值。

### `convert_mixed` 實作修正與報告重算

- `convert_mixed` 原本在每個注音字元前就 `flush()`，聲調記號被切成獨立 run，`toneless` 因此漏掉調號 `4`。
  改成只在切換 run 時 flush；修正後 toneless 與保留聲調兩版都符合按鍵表。
- 重跑 `corpus-diff`，`docs/reports/real-corpus-2026-11-21.md` 全部數字已用實際結果更新並加第 7 節新舊對照：
  938→939 筆、937→939 種、A 無法解析 509→510、不一致 680→681、`a-none` 505→506（74.26%→74.30%）、
  `space-merged-segments` 21.76%→21.73%、`space-tone-vs-separator` 3.09%→3.08%、
  `mixed-golden` 5→6 筆；前五代表案例的 `a-none` 與 `mid-run-symbol` 也換成 toneless 版。
  結論方向不變：空白語意仍是主因。

### gate

- `cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：**exit 0**（54 targets、1301 passed、0 failed）。
- clippy exit 0；fmt exit 0。
- 題庫：zh-golden **45/45**、ja modern-500 **92.2%**、fuzzy **44.5%**、mixed **6/6**。
- 題庫與指標語料無回歸；worktree `w73`（segment-diff）與 `w77`（real-corpus）已回收，branch 保留。
- `git branch --no-merged main`：`unify-segment`（使用者自行處理）。

### CI（push `dd0d835`，run 44809704573；activate-diagnose）

- linux (core)：success；windows (msvc)：success。

### CI（push `dffee88`，run 44815017478；segment-diff／real-corpus 與量測工具修正）

- linux (core)：success；windows (msvc)：success。

## 2026-11-21 05:10 workflow-modernize（`e688666` → `b5cf837`）

- 所有 workflow 的 `actions/checkout@v4` → `v5`、`actions/upload-artifact@v4` → `v6`、
  `actions/download-artifact@v4` → `v7`（都是第一個宣告 node24 的主要版本；upload-artifact v5 仍 node20 故略過）。
  CI／release 的 Node 20 黃色 annotation 預期消失。`publish` 條件未改。
- `docs/release.md`：新增「快取與發版流程」，記錄 tag scope 讓 Windows job 每次 `No cache found.`、
  跑完存 967 MB，以及「打 tag 前先在 main 用 `workflow_dispatch` 暖機」的程序。
- 驗證：三個 workflow YAML 有效；grep 確認沒有殘留 `@v4`。
- gate：test exit 0（54 targets、1301 passed、0 failed）、clippy exit 0、fmt exit 0。
- 題庫：zh-golden **45/45**、ja modern-500 **92.2%**、fuzzy **44.5%**、mixed **6/6**。
- `git branch --no-merged main`：`payload-warnings`（還在跑）、`unify-segment`（使用者自行處理）。

### CI（push `829e0ad`，run 44817233222；workflow-modernize）

- linux (core)：success；windows (msvc)：success。Node 20 annotation 已消失。

## 2026-11-21 05:25 payload-warnings（`6a73db7` → `cf49224`）

分支 tip 是 `6a73db7`（你給的 `73c8202` 是 rebase 前 hash；parent 實際為 `dffee88`，
與 main 的 `docs/release.md` 自動合併無衝突）。

- 移除 `DataRole::ZhPunct` 與 catalog 兩筆 `models/zh-punct/*`；`zh_select` 保留。
- `jc-cli data verify` 人讀輸出略過 `Missing && !shipped`，summary 只算顯示的 check；
  `--json` 仍是完整 8 項（含 2 個 zh_select missing），`DataManifest::verify` 回傳值沒動，
  引擎啟動降級警示保留。
- `docs/release.md` 補一句說明；agent 實測「通過 6/6，錯誤 0，警告 0」。

gate：
- `cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：**exit 0**（54 targets、1303 passed、0 failed）。
- clippy exit 0；fmt exit 0；`cargo check -p jc-data -p jc-cli --all-targets` exit 0。
- 題庫：zh-golden **45/45**、ja modern-500 **92.2%**、fuzzy **44.5%**、mixed **6/6**。
- `git branch --no-merged main`：`cache-prune-fix`、`docs-consistency`、`unify-segment`（待處理）。

### CI（push `a91224d`，run 44818247747；payload-warnings）

- linux (core)：success；windows (msvc)：success。

## 2026-11-21 05:40 cache-prune-fix（`17b0c26` → `67d328f`）

分支 tip `17b0c26`，parent 是 `cf49224`（payload-warnings 合併點），與 main 的文件改動自動合併無衝突。

- `scripts/cache-prune.sh`：新增 `--max-total-gb`（預設 8）與 `--protected-hours`（預設 1）；
  原有 family／閒置規則跑完後，若預估總量仍超標，就依舊到新追加刪除，保護未滿 1 小時的項目。
- `ci.yml` 清理由 `--skip-recent-hours 6` 改 1；`cache-prune.yml` 改 `KEEP_PER_FAMILY=1`、
  `SKIP_RECENT_HOURS=1`、`MAX_TOTAL_GB=8`，排程改每天。
- 新增兩個 fixture 與 `scripts/test-cache-prune.sh`（五案例全過）；`docs/port/cache-prune.md`、
  `docs/release.md` 補新規則。目標：目前 9.58 GB / 10 GB、九筆中六筆卡在 6 小時保護窗，
  新規則清完預估約 4.9 GB。
- gate：test exit 0（54 targets、1303 passed、0 failed）、clippy exit 0、fmt exit 0；
  `bash scripts/test-cache-prune.sh` 全部通過、兩個 workflow YAML 有效。
- 題庫：zh-golden **45/45**、ja modern-500 **92.2%**、fuzzy **44.5%**、mixed **6/6**。
- `git branch --no-merged main`：`docs-consistency`、`unify-segment`（都還在跑／待處理）。

### CI（push `ba6e93f`，run 44818846314；cache-prune-fix）

- linux (core)：success；windows (msvc)：success。

## 2026-11-21 05:55 docs-consistency（`21a5ae5`＋`0b5ee4b` → `1989a6d`）

只改文件，沒動程式碼、CI、版本號、`CHANGELOG.md` 或本紀錄。

- 現況文件 `docs/release.md`、`docs/install.md`、`installer/README.md`、`docs/port/tsf-installer.md`：
  zh-punct 從現行下載項改成已移除（zh_select 保留）。
- `docs/status.md`：更新到 11-21，補 rc.7／profile 啟用／量測工具／workflow 升級／快取規則／catalog 現況，
  重算 `jc-core` 行數與 1301 passed；另補「Node 20 annotation 待下次 run 驗證」與
  「x86 TSF DLL 已隨 MSI 出貨，32 位元載入待實機驗證」。
- 移植歷史文件 `docs/port/data-pack.md`、`data-download.md`、`legal-update.md`、`zh-models.md`、
  `docs/models.md`：加現況註記、保留歷史表格；data-pack 的 data verify 範例改成 6/6、0 警告。

gate：test exit 0（54 targets、1303 passed、0 failed）、clippy exit 0、fmt exit 0；
題庫 zh-golden **45/45**、ja **92.2%**、fuzzy **44.5%**、mixed **6/6**。

### CI（push `41e81d8`，run 44819844056；docs-consistency）

- linux (core)：success；windows (msvc)：success。

## 2026-11-21 06:10 unify-segment（`4b6928f`／`f0a68c7`／`1809a18`／`276b402` → `7e4ecb3`）

分支 rebase 到 main `41e81d8`，automerge 無衝突。把 `mixed_latin::split`（A）與 `Router::convert`（B）
合成單一管線，移除 `Config::mixed_latin_split` 開關，DP 成為唯一路徑。

五條規則：
- R1 切段：注音／假名漢字／英數 run／符號／空白；大寫與全角不進 DP。
- R2 空白是一聲：看 run **最後一個片段**（注音、空白被吃掉、原音節無聲調鍵）；前面可是英文或注音。
  限制：一聲只能接在 run 開頭、注音或英文片段後，不能接在日本 kana 後（見下方回歸）。
- R3 標點當韻母：`. , ; / -` 在 run 開頭、或前面不是缺韻母的合法音節時是符號 atom。
- R4 語言連貫：`Options::initial_lang` 帶前一段語言，切換成本照算。
- R5 後處理：只保留 `group_words`。

既有期望值變更：開關移除的 `test_mixed_latin_split_off_keeps_literal` 改名為
`test_mixed_latin_split_is_default`；`split_latin_run` 被 R3 取代，
`test_latin_run_keeps_trailing_final_only_when_split_completes` 改名
`test_symbol_punctuation_stays_symbol` 並改測路由片段；
`helloji `（`hello窩`）與 `ji3hellp`（`我hellp`）兩條已還原原始期望值，改的是實作。

增量路徑與路由一致性 property test **532 筆**通過（`test_incremental_matches_router`）。

### 第一次合併的回歸與修正

第一次合併（`6af9270`）mixed golden 從 6/6 掉到 4/6：`scenario-a-primary-ja`／
`scenario-b-primary-zh` 的 `desu` 被 DP 算成 `で(ja) + su(zh 一聲)`，高分於整段 `です`，
R2 per-piece 又讓一聲接在日文 kana 後，切成 `で` + `尼`。
`276b402` 限制一聲只能接在 run 開頭、注音或英文片段後，不能接在 kana 後（`mixed.rs` +12、
`mixed_latin.rs` +23、文件 +7），修正後 mixed 回到 6/6。

gate：
- `cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：**exit 0**（54 targets、
  **1313 passed、0 failed**，比合併前的 1303 多 10 筆）。
- clippy exit 0；fmt exit 0。
- 題庫（逐項與合併前相同）：zh-golden **45/45**、ja modern-500 **92.2%**、fuzzy **44.5%**、mixed **6/6**。
- `git branch --no-merged main`：空。

## 2026-11-21 06:35 ci-data-pack（`5971dce` → `7a6b09d`）

CI 的 linux 與 windows job 在測試前下載正式資料包，公開 repo variable `JC_DATA_BUNDLE_URL`／
`JC_DATA_BUNDLE_SHA256` 已設定；強制驗 SHA-256、解開、找 manifest 根目錄並寫入 `JC_IMEDATA`；
URL／SHA 為空或 SHA 不符直接失敗。`release.yml`、`Cargo.toml`、Rust 測試沒改。
`docs/release.md`、`docs/status.md` 補說明。

這也處理了上一輪 unify-segment 的 CI 失敗：`test_mixed_english_suggestion_and_completion`
在本機有英文詞表時通過，CI 沒有詞表時退回內建原型而失敗（本機以
`JC_ENGLISH_PACK=/tmp/opencode/empty-pack` 可重現 `"我ㄘㄍㄠㄠㄣ"` vs `"我hellp"`）。
CI 現在會先下載正式資料包，英文詞表隨之可用。

gate（以 `JC_IMEDATA=/mnt/work/jc/jc-ime-data` 模擬 CI）：
- `cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：**exit 0**（54 targets、1313 passed、0 failed）。
- clippy exit 0；fmt exit 0；`ci.yml` YAML 有效。
- 題庫：zh-golden **45/45**、ja modern-500 **92.2%**、fuzzy **44.5%**、mixed **6/6**。
  （第一次帶 env 跑 ja 得到 92.0%，該次 timeout 欄 1.0%；連續重跑四次都是 92.2%、timeout 0.0%，
  判定為單題 timeout 抖動，不是回歸。）
- `git branch --no-merged main`：空。

### CI（push `b7650c0`，run 44824780413／重跑同號；unify-segment）

- linux (core)、windows (msvc) 都 failure，同在 `Test` 的
  `session::tests::test_mixed_english_suggestion_and_completion`：`"我ㄘㄍㄠㄠㄣ"` vs `"我hellp"`。
- 根因：CI 沒有英文詞表，`English::load_default()` 退回原型詞彙，`hell` 查不到，
  「英文詞＋單字母尾巴」候選消失，`hellp` 被切成注音。本機以 `JC_ENGLISH_PACK=/tmp/opencode/empty-pack`
  可重現。重跑兩平台仍失敗，不是 flake。

### CI（push `f6cb47f`，run 44829770518；ci-data-pack）

- 資料包下載／驗 SHA／寫 `JC_IMEDATA` 的步驟成功，但同一條 `hellp` 測試仍失敗（兩平台），
  原因是 CI 資料包仍沒有讓 `hell` 命中；修法須落在 unify-segment。

## 2026-11-21 06:55 unify-segment 補充修復（`ab13ffa` → `6ed72d3`）

- 小寫 run 內沒有任何詞表單字命中時，整體給一個帶懲罰的英文候選；
  該候選標記的 cell 只能接單字母尾巴，避免吞掉 `kyouhahello`。
- 新增 `test_english_tail_without_word_pack` 鎖住降級環境行為。
- 驗證：`JC_IMEDATA=/nonexistent` 跑 `jc-core --lib` 全數 **709 passed、0 failed**
  （新測試單獨 1 passed），降級環境不再失敗。

gate（`JC_IMEDATA=/mnt/work/jc/jc-ime-data`，模擬 CI）：
- `cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：**exit 0**（54 targets、
  **1314 passed、0 failed**）。
- clippy exit 0；fmt exit 0。
- 題庫：zh-golden **45/45**、ja modern-500 **92.2%**、fuzzy **44.5%**、mixed **6/6**。
- `git branch --no-merged main`：空。

### CI（push `a795413`，run 44830979177；unify-segment 環境修復後）

- linux (core)：success；windows (msvc)：success。`hellp` 測試在兩個平台都通過。

## 2026-11-21 07:15 data-pack-english-meta（`a28fb68` → `7741c1c`）與 legal 帳本校正

- CATALOG 補 `english/manifest.json` 與 `english/NOTICE.txt`（`DataRole::English`／L-08／`shipped: true`），
  `data-bundle.sh` 會一起包進資料包；CI 下載後 `Dictionary::load_pack` 能找到這兩份中繼檔，
  不再讓 `English::load_default()` 退回原型詞彙。
- Orchestrator 已用新 manifest 重包並上傳 HF（dataset commit `34a7025`），公開 repo variables 更新為
  URL `.../resolve/34a7025.../jc-ime-data-2026-11-20.tar.zst`、SHA
  `9ce77be471afe43151a2d97580788821461ab4c19c2011eb91accaaa4514dcad`（104,608,074 bytes，
  從 HF 重下載驗證 SHA 相符、`english/` 三檔齊全）。
- `legal/README.md`（直接在 main 改）：Jinen tokenizer 從「否（manifest 未列，待補）」改成
  「是（隨安裝包散布）」；「待補」段落改成「已補」，並修正對照結果段落裡仍把 Jinen tokenizer
  列為「否」的說法。全文已搜過，沒有其他「tokenizer 不在 manifest」或「待補」殘留。
  HF dataset 的 README（commit `6ac09f0`）由使用者處理，本筆不含。

gate（`JC_IMEDATA=/mnt/work/jc/jc-ime-data`）：
- `cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：前兩次 exit 101，
  唯一的失敗是既有 flake `jc-engine` `supervise::tests::second_supervisor_is_rejected_by_lock`
  （`supervise.rs:941`，`AlreadyRunning { name: "jc-ime-test-lock-…" }`）；單獨跑 1 passed，
  第三次全量 **exit 0**（54 targets、**1315 passed、0 failed**）。
- clippy exit 0；fmt exit 0。
- 題庫：zh-golden **45/45**、ja modern-500 **92.2%**、fuzzy **44.5%**、mixed **6/6**。
- `git branch --no-merged main`：空。

### CI（push `04bb19c`，run 44879174088；data-pack-english-meta）

- 第一次：linux (core) success；windows (msvc) failure，唯一失敗是 timing-sensitive 的
  `session::tests::test_ja_neural_timeout_keeps_statistical_result`（`session/mod.rs:2494`，
  `left: "きょう"` / `right: "今日"`，stub runtime 80ms delay 對 5ms timeout）。資料包 SHA 驗證通過。
- `gh run rerun --failed` 後：**linux (core) success、windows (msvc) success**，`hellp` 測試也通過。

## 2026-11-21 07:40 activate-second-call（`13478cb` → `3242dde`）

根因：冒煙測試誤用 `ITfThreadMgr::Activate()` 回傳的**應用程式** client id 去激活 TIP；
正確的 TIP client id 要用 `ITfClientId::GetClientId(&CLSID_JC_TSF)`。
這解釋了記事本成功（msctf 傳入正確的 TIP id）而 smoke test 失敗（拿到應用程式 id 32，
msctf 查不到所屬 TIP，回 `E_INVALIDARG`）。

- `crates/jc-tsf-smoke/src/checks.rs`：檢查 5 改用 `ITfClientId::GetClientId`，印出兩個 id；
  `CONNECT_E_ADVISELIMIT`（已激活同 TIP）判為通過；新增重複激活測試。
- `crates/jc-tsf/src/lib.rs`、`composition.rs`：只加診斷日誌（thread id、Unadvise HRESULT、
  重複 Deactivate 標註），Activate 邏輯沒動。jc-tsf 的 Activate／cleanup 本身是對的
  （記事本實機驗證兩次成功）。
- 新文件 `docs/port/activate-second-call.md`，並更新 `activate-diagnose.md`、`tsf-smoke.md`。

gate：`cargo check --workspace --all-targets` exit 0；
`cargo check -p jc-tsf -p jc-tsf-smoke --all-targets --target x86_64-pc-windows-msvc` exit 0；
完整 test exit 0（54 targets、1315 passed、0 failed）；clippy exit 0；fmt exit 0。
題庫：zh-golden **45/45**、ja **92.2%**、fuzzy **44.5%**、mixed **6/6**。

## 2026-11-21 07:50 engine-pipe-name（`bff593f` → `5290a6f`）與 WXS 註解修正

根因：WXS 的 EngineAutostart 寫死 `--socket jc-ime`，引擎監聽固定名；TIP 從 SID 推出
`jc-ime-1001`，兩邊不同，pipe 連不上、按鍵全部放行成英文。這是使用者實機
「打字全是英文」的真正原因。

- `installer/jc-ime.wxs`：EngineAutostart 移除固定 `--socket`，讓引擎用
  `resolve_pipe_name()` 從 SID 推出與 TIP 相同的名字。
- `installer/validate-linux.sh`：加檢查擋 autostart 再出現 `--socket`。
- `crates/jc-engine/src/main.rs`、`crates/jc-windows/src/pipe_name.rs`：加註解與一致性測試
  （解析邏輯沒動）；文件 `docs/install.md`、`docs/wrapper.md`、`docs/port/tsf-installer.md`、
  新檔 `docs/port/engine-pipe-name.md`。
- 整合時另修正 WXS XML 有效性：新加的 XML 註解裡寫了 `--socket`，
  XML 註解不容許連續 `--`，會讓 WiX 解析失敗；改寫成「不帶 socket 覆寫參數」。
  修正後 XML parse 通過，WXS 內已無 `--socket`（validate-linux.sh 的 grep 檢查仍會擋回歸）。

gate：
- `cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：**exit 0**
  （54 targets、**1316 passed、0 failed**）。
- `cargo check --workspace --all-targets` exit 0；clippy exit 0；fmt exit 0。
- 題庫：zh-golden **45/45**、ja modern-500 **92.2%**、fuzzy **44.5%**、mixed **6/6**。
- WXS XML 有效；`validate-linux.sh` 本機因缺 .NET runtime 無法完成（已知限制，WIx 綁定交給 CI）。
- worktree `w7N`、`w7P` 已回收，branch 保留；`git branch --no-merged main`：空。

### CI（push `1276f27`，run 44893221254；activate-second-call／engine-pipe-name）

- linux (core)：success；windows (msvc)：success；check-tag：success。
- build-linux／build-windows／publish：skipped（main scope，標籤流程才會跑）。

## 2026-11-21 08:10 rc.9 後三筆 installer 修正（`a86cfe2`／`57d91c2`／`159f4a0`）

三筆 parent 都是 `98c98af`。第一筆 ff-only 快轉到 `a86cfe2`，後兩筆因同 parent 已分歧，
改以 `--no-ff` 合併：`build-zh-manifest` → `5c84499`、`msi-overwrite` → `4f0d9e2`。

| Branch | Commit | 合併 |
|---|---|---|
| `engine-supervise` | `a86cfe2` | fast-forward（`98c98af` → `a86cfe2`） |
| `build-zh-manifest` | `57d91c2` | `5c84499` |
| `msi-overwrite` | `159f4a0` | `4f0d9e2` |

- `engine-supervise`：WXS autostart 加 `--supervise`，引擎才會開 pipe 並自動重啟。
  rc.9 打字全英文的根因：上一輪拿掉 `--socket` 後引擎退回 stdio，完全不開 pipe。
  `validate-linux.sh` 改成同時擋「缺 `--supervise`」與「有 `--socket`」。
- `build-zh-manifest`：catalog 補 `build-zh/manifest.json`（`Bundle::open` 一定要讀，缺了就 fatal），
  WXS DataFiles 加對應 File；實機 `supervisor.log` 的「讀取 build-zh\manifest.json 失敗」根因。
- `msi-overwrite`：ProductVersion 第四段帶 rc 序號（`0.1.0.9`），PackageCode 每次 build 自動產生，
  rc 版可直接覆蓋安裝，不必先卸載。

三筆都沒改 crates 的 TSF Activate／pipe 解析、沒改 CI、沒改 `Cargo.toml` 版本號。
WXS XML 解析通過，`--supervise` 在 RegistryValue 值內（非註解），沒有 XML 非法的連續 `--`。

gate：
- `cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：**exit 0**（54 targets、
  **1317 passed、0 failed**，比上一輪 1316 多 1）。
- `cargo check --workspace --all-targets` exit 0；clippy exit 0；fmt exit 0。
- 題庫：zh-golden **45/45**、ja modern-500 **92.2%**、fuzzy **44.5%**、mixed **6/6**。
- `git branch --no-merged main`：`auto-download-zh-select`、`zh-select-pipeline`（都還沒合，未動）。

### CI（push `2816d31`，run 44905481893；rc.9 後三筆 installer 修正）

- linux (core)：success；windows (msvc)：success；check-tag：success。
- build-linux／build-windows／publish：skipped（main scope，標籤流程才會跑）。

## 2026-11-21 08:40 punct-cleanup（`9af1be0`，已在 main）與 pipe-busy（`576bb65`，ff → `576bb65`）

### punct-cleanup

明確過濾已移除的 ja-punct／zh-punct 角色、移除過期 tokenizer 探測；新增
`scripts/clean-model-set-manifest.sh`。Orchestrator 已清資料、重產 manifest、重新打包上傳 HF，
公開 `JC_DATA_BUNDLE_SHA256` 已更新為 `58395f56b9223ff03a23dacad6bd76d78f6a94729ac1b0354dd25510f6a48e4e`。

### pipe-busy

根因：舊 mutex `Local\jc-ime-engine-supervisor` 是 session 範圍，兩個 RDP session 拿到兩把鎖，
兩個 supervisor 各 spawn 子行程搶同一 pipe。

- 鎖改成 `%LOCALAPPDATA%\jc-ime\Local-jc-ime-engine-supervisor.lock`（分享模式 0）。
- 子行程放 `KILL_ON_JOB_CLOSE` job object。
- TIP 端 `connect_with_wait` 處理 `ERROR_PIPE_BUSY`，用 `WaitNamedPipeW` 等待。
- `MaxInstances` 維持 1（Session 不是 `Send`，改多 instance 超出範圍，見 `docs/port/tsf-pipe.md`）。
- 新增兩個 Windows 測試（只寫未跑）；`docs/port/tsf-pipe.md` 新檔，`engine-supervisor.md`／
  `named-pipe.md`／`pipe-name.md` 更新。TSF Activate／pipe 解析沒改。

gate：
- `cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：**exit 0**（54 targets、
  **1319 passed、0 failed**，比上一輪 1317 多 2）。
- `cargo check --workspace --all-targets` exit 0；clippy exit 0；fmt exit 0。
- Windows 目標縮小範圍 `cargo check -p jc-windows -p jc-engine --target x86_64-pc-windows-msvc` exit 0。
  全 workspace 的 Windows clippy 在本機卡在 `ring v0.17.14` 的 build script
  （缺 Windows MSVC C 工具鏈，環境限制，與本筆無關；CI 的 windows job 會驗證）。
- 題庫：zh-golden **45/45**、ja modern-500 **92.2%**、fuzzy **44.5%**、mixed **6/6**。
- `git branch --no-merged main`：`settings-default-file`、`romaji-input`、`model-ui-status`（都還在跑，未動）。
- worktree `w7W`（pipe-busy）與 `punct-cleanup` 的 worktree 已回收，branch 保留。

### CI（push `84cd944`，run 44911494690；punct-cleanup／pipe-busy）

- linux (core)：success；windows (msvc)：success；check-tag：success。
- build-linux／build-windows／publish：skipped（main scope，標籤流程才會跑）。
  本機全 workspace Windows clippy 卡在 `ring` 的 C 工具鏈，CI windows job 已驗證通過。

## 2026-11-21 09:10 focus-pipe（`7e38a14` → rebase `683735f` → `18741be`）與 settings-default-file 修正（`51422c9` → rebase `15828cc` → `9cc1ab5`）

### focus-pipe

把引擎 pipe 連線從 Activate 移到 focus 事件：Activate 只建訊息視窗與註冊 sink；
`OnSetFocus` 才 spawn worker 連 pipe；`OnKillFocus` 排 120ms 寬限期後釋放；
`Deactivate` 立刻清 sender。用狀態機吸收重複事件，`pending_focus` 處理 focus 早於 Activate。
Scope 只有 `jc-tsf/src/lib.rs`、`composition.rs`、`jc-tsf-smoke/src/checks.rs` 與兩份文件，
沒碰 `engine.rs`、`pipe_name.rs`、supervisor、`Cargo.toml`、CI、版本號、catalog。

合併後第一次完整 test 是 1317 passed、6 failed：五個已知的 `settings-default-file` 整合測試失敗，
加上 `supervise::tests::second_supervisor_is_rejected_by_lock` 鎖檔 flake。該 flake 單獨跑 1 passed，
第二次全量 **1318 passed、5 failed**，五筆都在同樣的 `jc-engine` 整合測試，focus-pipe 沒有新增失敗。

### settings-default-file 修正

修正 mixed 預設導致的五筆 `jc-engine` 整合測試：`engine_data.rs`／`socket.rs`／`stdio.rs`
明確指定 `--mode zh`，`settings.rs` 兩筆期望 zh→mixed。保留原本 state／提交語意，沒有弱化測試。

gate（合併後）：
- `cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：**exit 0**
  （54 targets、**1323 passed、0 failed、1 ignored**）。
- `cargo check --workspace --all-targets` exit 0；clippy exit 0；fmt exit 0。
- Windows 目標 `cargo check -p jc-tsf -p jc-tsf-smoke --all-targets --target x86_64-pc-windows-msvc` exit 0。
- 題庫：zh-golden **45/45**、ja modern-500 **92.2%**、fuzzy **44.5%**、mixed **6/6**。
- **不 push**：`transport-unified` 還在跑，等它完成後一次 push 並回收所有 worktree。

## 2026-11-21 09:40 transport-unified（`b217ddd` → `be46e12`）

用 `interprocess::local_socket` 統一引擎與 client transport：Windows 底層 named pipe、
Linux abstract UDS，一套程式碼。Windows ACL 用 SDDL，pipe-busy 等待用
`ConnectWaitMode::Timeout`。刪掉 `pipe_windows.rs`、`pipe.rs`、`named-pipe.md`，
換成 `transport.rs`、`transport.md`；與 focus-pipe 只在 `jc-tsf/src/engine.rs` 改了 3 行型別。

agent 報告的兩個失敗（engine_data、socket）是在舊 base `d3caea3` 跑的；
rebase 到 `16e05cd` 後已隨 settings-default-file 修正消失。

gate：
- `cargo test --workspace --no-fail-fast --exclude jc-settings-ui`：**exit 0**
  （54 targets、**1323 passed、0 failed、1 ignored**）。
- `cargo check --workspace --all-targets` exit 0；clippy exit 0；fmt exit 0。
- Windows 目標 `cargo check -p jc-windows -p jc-engine --target x86_64-pc-windows-msvc` exit 0。
- 題庫：zh-golden **45/45**、ja modern-500 **92.2%**、fuzzy **44.5%**、mixed **6/6**。
- `git branch --no-merged main`：空（本機所有 feature branch 都已合入）。

### 版本與發版（`0.1.0-rc.11`）

- `27d933c`：`Cargo.toml` 版本 `0.1.0-rc.11`、CHANGELOG 補 rc.11 段落。
- `779e589`：`Cargo.lock` 同步到 rc.11；annotated tag `v0.1.0-rc.11` 打在這個 commit。
- push：`ca2d318..9d639c4`（整合紀錄），再 `9d639c4..779e589`（版本與 lock）。tag 已推送。
- CI（push tag，run **44915796659**，HEAD `779e589`）：linux (core) success、windows (msvc) success、
  check-tag success、build-linux (tar.gz) success、build-windows (MSI) success、publish success。
  另一個 run 44915725027（`9d639c4`，tag push 前的 main）被新 run 取代而 cancelled。
- 所有整合者以外的 worktree 已回收，branch 保留；目前只剩 `/mnt/work/jc/jc-ime`。
