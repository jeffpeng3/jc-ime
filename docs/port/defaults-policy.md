# defaults-policy：預設啟用學習、模糊音與注音選字模型

Worktree：`/mnt/work/jc/wt/defaults-policy`
Branch：`defaults-policy`

## 結論

| 預設 | 舊值 | 新值 | 理由 |
|---|---|---|---|
| 個人化學習 `SessionConfig::learning` | 沒有開關，空 store 不作用 | `true` | 空 store 不影響排序；有紀錄才學習。關閉時只讀不寫。 |
| 模糊音 `SessionConfig::fuzzy_pairs` | `Vec::new()` | ㄗ／ㄓ、ㄘ／ㄔ、ㄙ／ㄕ、ㄣ／ㄥ 四組 | 產品預設要能處理常見近音；只補候選，不改自動顯示。 |
| 注音選字模型 | 預設 off | 預設 on（`zh-select`，`0c-sequence-d512`） | 沿用 `zh-select-calibrate` 留下的 `weight = 0.5`、`margin = 3.0`。 |
| 日文 reranker `ja_rerank` | `true` | `false` | 實測 modern-500 由 86.0% 退到 82.0%。 |

## 學習

`PersonalStore` 原本沒有總開關，排序一律讀 store。新增
`SessionConfig::learning`，預設 `true`：

- `Session::learn_candidate` 只在開啟時寫入 `note_manual`／`note_passive`。
- `Session::rebuild_zh` 只在開啟時呼叫 `PersonalStore::rank_hypotheses`。
- 沒有存檔路徑時只存在記憶體，不報錯；`Session` 不碰檔案。
- 數字鍵、空白鍵手選與 Enter 手選都走同一個學習入口；先前數字鍵選字
  不學習，這次補上。

上限與衰減走既有的 `personal::prune`：

- `Store::advance_time` 每小時至多整批衰減一次（30 日半衰期），由
  `learn_candidate` 用系統時鐘推進。
- `Store::prune_capacity` 每 64 次學習事件觸發一次，重用
  `Model::prune_with` 的容量上限與證據老化；沒有 LM 先驗，因此不會用
  `share × prior` 主動淘汰低頻詞。
- 手選、片語、自訂詞與讀音紀錄照 `protected_words` 保留。

「學習預設生效」的測試：
`crates/jc-core/src/session.rs` 的
`test_learning_enabled_by_default_changes_first_candidate`。同一讀音先移到
第二候選「窩」再 Enter，下一次同讀音的首選變成「窩」。關閉學習的對照測試是
`test_learning_disabled_keeps_statistical_order`。

## 模糊音

`DEFAULT_FUZZY_PAIRS` 四組全開。`eval zh-golden` 的題庫沒有指定 `fuzzy`
時沿用產品預設；有指定 `fuzzy` 的案例才整組覆寫。舊版會用空的題庫選項
蓋掉預設，這次修掉。

## 注音選字模型

`zh-select-calibrate` 已把參數留在 `SelectConfig::default()`：
`weight = 0.5`、`margin = 3.0`。該點是掃描網格中破壞最少的一端，主 dev
救回 0、改壞 0；壓力 dev 仍有 2/161 改壞。這次直接沿用，不改成任務書的
保守備案 `2.0 / 1.0`，因為 `2.0 / 1.0` 在壓力 dev 改壞 26/161，沒有比較保守。
狀態是「已預設啟用，保守參數，未證明能改善排序」。

`eval zh-golden` 與 `convert` 的 `--select-model` 預設改為 `on`，可用
`--select-model off` 關閉。模型或 ONNX Runtime 缺席時只印警告，維持統計
排序，不讓 golden 失敗。

## 日文 reranker

`SessionConfig::ja_rerank` 預設 `false`。`eval ja` 本來就預設不接
reranker，這次只把 session 預設對齊量測結果。

## 量測

| 指令 | 結果 |
|---|---|
| `eval zh-golden`（預設：模糊音四組 + select model on） | 45/45，100.0% |
| `eval zh-golden --select-model off` | 45/45，100.0% |
| `eval fuzzy` 注音 top-1 | 44.5%（門檻 44.5%，未退步） |
| `eval ja --corpus modern-500`（rerank off） | 86.0%，top5 98.0% |
| `eval ja --corpus modern-500 --rerank on` | 82.0%，top5 95.2% |

select model 延遲：推論 100 次，首次 p50 48.45 ms／p95 69.67 ms，重跑
p50 12.12 ms／p95 19.76 ms（受機器負載影響），timeout 0。
rerank on 的推論 p50 13.02 ms、p95 27.11 ms，changed 324/556。

## 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/fuzzy/fuzzy.go`（四組近音規則） | `crates/jc-core/src/zh/fuzzy.rs`、`crates/jc-core/src/session.rs` | ported：規則不變，`SessionConfig` 預設改為四組全開 |
| `core/personal/personal.go`（學習預設生效） | `crates/jc-core/src/personal/mod.rs`、`crates/jc-core/src/session.rs` | rewritten：新增 `learning` 開關，預設 `true` |
| `core/personal/prune.go`（上限與衰減） | `crates/jc-core/src/personal/prune.rs` | rewritten：新增 `Store::prune_capacity` 與 `Store::advance_time`，session 每 64 次學習事件清理一次 |
| `core/score/ranker.go`（選字模型 operating point） | `crates/jc-core/src/zh/select.rs`、`crates/jc-cli/src/cli.rs` | rewritten：沿用 0.5／3.0，CLI 預設 on |
| 日文 reranker 預設 | `crates/jc-core/src/session.rs` | n/a：Go 沒有這個 session 預設；依本專案量測關閉 |

## 已知限制

- `jc-engine` 沒有改；若其他入口直接建 `Session`，仍要自己安裝
  `zh-select` 模型。模型安裝是行程層級的 `select::install`。
- `Store::advance_time` 依系統時鐘推進；測試與 golden 不依賴時間，因為
  排序只讀 `uni` 權重，且新 store 的第一次呼叫只設定時間。
- `cargo test --workspace` 在 HEAD 已有 5 個失敗，與本任務無關：
  `jc-model::layout` 4 個（tokenizer piece id 與 mask 期望值不符）與
  `jc-core::ja::rerank::right_context_keeps_the_prefix_nearest_the_cursor`
  1 個。把本任務的 diff 移掉後在乾淨 HEAD worktree 重跑仍相同。
