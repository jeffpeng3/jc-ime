# ja-personal-wire：把日文學習接進 Session

Worktree：`/mnt/work/jc/wt/ja-personal-wire`
Branch：`ja-personal-wire`（base：`defaults-policy`）
前身：`/mnt/work/jc/ref/0c-ime/core/ja/personal`（main `8f443af`）

## 結論

日文選字現在會在送出時寫進**共用的**個人資料 store，下一次同讀音的排序會
反映學習結果。`core/ja/personal` 的公式與尺度原樣沿用，資料位置改成
`personal::Store.languages["ja"]`，與注音共用同一個 `personal.json`，用語言
欄位區分，不再另開孤島。

- 預設開啟：`SessionConfig::learning` 預設 `true`（沿用 `defaults-policy`）。
- `Session` 在日文提交時記錄 `(reading, surface, left, 次數)`；明確手選記兩倍
  證據。
- N-best 以學習加成重排；不用改 `ja/decode.rs` 的 lattice。
- `--user-data <dir>` 指定個人資料目錄；未指定時只存在記憶體。
- 載入失敗只警告並用空白資料，不 panic、不覆寫原檔。

## 資料位置

| 項目 | 注音 | 日文 |
|---|---|---|
| 檔案 | `<user-data>/personal.json` | 同一個 `personal.json` |
| 欄位 | `personal::Store` 的 `bi`／`uni`／`picked` 等 | `languages["ja"]` |
| 讀音鍵 | 注音音節，空白分隔 | かな逐字，空白分隔（`reading::key`） |
| 左文鍵 | — | `normalize_left(left)`：去空白、轉小寫、只留最後 24 字 |
| 提供者 | `note_manual`／`note_passive` | `note_language_weighted` |

`personal::Store` 新增的介面（`crates/jc-core/src/personal/mod.rs`）：

- `note_language_weighted(id, reading, surface, left, weight, at)`：記一次選字。
- `language_surface_count`／`language_reading_count`／`language_context_count`：
  查次數。
- `undo_language_note`：反悔最近一次語言學習。
- `prune_languages(policy)`：依 `PrunePolicy::max_word` 與
  `MAX_LANGUAGE_TABLE = 1024` 清理每張語言表。
- `advance_time` 現在也會以 30 日半衰期衰減語言次數（單次紀錄至少保留 1）。

`personal::ja::JA_FILE_NAME`／`personal.ja.json` 與 `ja::personal::Store` 的
獨立檔案介面保留原樣（`docs/port/ja-personal.md` 已凍結）；`Session` 與
`convert` 走的是上面的共用 store。

## 公式與尺度

單一文節的加成，與 Go `Learner.bonus` 相同：

```
bonus(count) = min(Boost × log2(1 + count), MaxBoost)
Boost = 0.75，MaxBoost = 2.0
```

查詢以「同讀音、同詞面」為前提，分兩級：

```
exact  = bonus(min(左文＋詞面次數, 讀音＋詞面次數))
global = bonus(讀音＋詞面次數) × 0.5
clause_bonus = max(exact, global)
```

- `exact` 對齊前身 `Store.Lookup(reading, left)`：換了前文就不套用。
- `global` 是為了「同一句在新前文下再打一次」仍然看得到學習；折半讓單次
  學習不會壓過成本差距明顯的首選。
- 整句分數是逐文節加成相加，第一個文節用 `ctx.left`，其餘用前一個文節
  詞面，與 Go `Learner.Score` 相同。
- 分數量尺：`JaResult.score = -cost`，詞典成本 1000 對應 1.0；因此 2.0 的
  加成最多翻掉約 2.0 的成本差距。
- 候選排序用 `-candidate.cost / 1000 + bonus`，只作用在
  `JaCandidateSource::Convert`，與 Go `boostable` 相同。

證據權重：

| 事件 | 權重 | 說明 |
|---|---|---|
| 被動接受首選 | 1 | `UNIFIED_PASSIVE_WEIGHT` |
| 明確手選（含非首選） | 2 | `UNIFIED_MANUAL_WEIGHT`；候選窗開著時 |

## 事件流

1. `Session::rebuild_ja` 取得 `JaDecoder::results`（含文節），套用
   `UnifiedLearning::rank_results` 後取第一名顯示，並把結果留在
   `last_ja_results`。
2. 送出（Enter、空白、數字鍵）時 `learn_candidate` 依模式分派；
   日文走 `learn_ja_candidate`，用工節切分寫進 `languages["ja"]`。
3. 每次學習事件推進 `Store::advance_time`，每 64 次呼叫
   `prune_capacity` 與 `prune_languages`。
4. `Session::undo_learning` 反悔最近一次語言學習並重建候選。

`Session::new` 會複製 `SessionConfig::personal`；宿主可透過
`Session::personal()` 取得學習結果後寫檔。

## CLI

```sh
jc-cli convert "わたしはほんをよむ" --user-data /tmp/opencode/ja-user
jc-cli convert "わたしはほんをよむ" --user-data /tmp/opencode/ja-user --pick 2
```

`convert` 會把選定的候選（預設第 1 名，`--pick` 指定第 n 名）記進
`<user-data>/personal.json`，下一次同讀音就套用。`--pick` 只支援日文；
注音與英文轉換會拒絕。載入失敗（檔案損壞、格式較新）只印警告並用空白資料，
這次不寫回。

## Go 對照

| Go 檔案／行為 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/ja/personal/learning.go` `DefaultBoost`／`DefaultMaxBoost`／`bonus` | `crates/jc-core/src/ja/personal.rs` `UnifiedLearning` | ported：公式與上限相同，次數改存共用 store |
| `core/ja/personal/learning.go` `ClauseBonus`／`Score` | 同上 `clause_bonus`／`score` | ported；另加 `global` 折半層 |
| `core/ja/personal/learning.go` `LearnClausesAt` | 同上 `learn_unified` | rewritten：寫入 `languages["ja"]`，不寫獨立檔 |
| `core/ja/personal/learning.go` `Converter.Convert`／`Candidates` | 同上 `rank_results`／`rank_candidates` | rewritten：在 `Session` 與 `convert` 的 N-best 之後套用 |
| `core/ja/personal/store.go` `Store.Lookup` | `crates/jc-core/src/personal/mod.rs` `language_*_count` | rewritten：鍵值改用共用 store 的語言表 |
| `core/ja/personal/store.go` `Undo` | 同上 `undo_language_note` | rewritten：只還原單次加權 |
| `core/ja/personal/store.go` `prune` | `prune_languages` 與 `Store::advance_time` | rewritten：沿用 `PrunePolicy` 容量與 30 日半衰期 |
| `core/personal/ja.go` `JaPath`／`JaFileName` | `crates/jc-core/src/personal/ja.rs`、`ja/personal.rs` | ported（未變動） |
| `internal/settings` `JaLearning` | `SessionConfig::learning` | ported：預設 `true` |

## 測試

| 測試 | 內容 |
|---|---|
| `session::tests::test_ja_learning_ranks_manual_choice_first_and_undo_restores` | 第一次手選第二候選「京」，第二次同讀音首選變「京」，`undo_learning` 回到「今日」 |
| `session::tests::test_ja_learning_disabled_keeps_statistical_order` | 關閉學習時排序不變 |
| `session::tests::test_ja_passive_learning_keeps_clear_statistical_winner` | 單次被動接受不翻掉明顯的首選 |
| `ja::personal::tests::test_unified_learning_records_into_shared_store` | 讀音、前文、次數寫進 `ja` 命名空間，注音命名空間不變 |
| `test_unified_bonus_is_capped_like_go` | 加成上限 2.0 |
| `test_unified_learning_does_not_override_clear_statistical_winner` | 成本差距 2.0 時學習不翻盤 |
| `test_unified_undo_restores_ranking` | 反悔後命名空間清空 |
| `test_unified_store_round_trips_json` | `personal.json` 存讀後日文條目仍在 |
| `personal::tests::test_note_language_*`／`test_prune_languages_*`／`test_language_counts_decay_*` | 權重、上限、prune 與衰減 |

## 驗證

| 指令 | 結果 |
|---|---|
| `cargo check --workspace --all-targets` | 通過 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通過 |
| `eval ja --corpus modern-500` | 86.0%（top5 98.0%，未退步） |
| `convert "わたしはほんをよむ" --user-data DIR` 兩次 | 第二次排序反映第一次的選擇；`languages.ja` 有 3 個文節 |
| `convert ... --pick 2` 後再跑一次 | 第二次首選變成第一次的第 2 名（わたしは本を読む） |

依共通規則沒有執行 `cargo test`／`blaze test`；測試只寫、只編譯，由整合階段
統一跑。

## 已知限制

- 日文學習只調整 N-best 與候選排序，不進 Viterbi 的 lattice；逐字変換
  （`ja_live`）開啟時候選會重排，但 debounce 中間結果仍由 live controller
  決定。
- `global` 折半層是為了連續輸入的可用性，Go 前身只有 `exact`；這是刻意差異。
- `prune_languages` 只處理容量與時間衰減，不做 Go `rankChecker` 的同音安全
  比較；`prune.rs` 的既有整合點不變。
- `convert` 會改寫 `--user-data` 的個人資料；不想留下學習紀錄時不要指定
  `--user-data`。
