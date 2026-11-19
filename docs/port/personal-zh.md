# personal-zh：個人化學習

Worktree：`/mnt/work/jc/wt/personal-zh`
Branch：`personal-zh`
前身：`/mnt/work/jc/ref/0c-ime/core/personal`（main `8f443af`）

## 範圍

這一輪把 `core/personal` 的上游功能整包搬過來，先求功能齊全，之後再刪掉
本專案用不到的部分。注音 golden 的 4 個學習案例仍是驗收門檻；`cargo test
--workspace` 必須全綠。

## 檔案對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `personal.go` | `crates/jc-core/src/personal/mod.rs` | ported：`Store`、`Model`、`Learn`、`Prob`、`Source` 視圖、`Seg`／`Commit` |
| `evidence.go` | `crates/jc-core/src/personal/evidence.rs` | ported |
| `diversity.go` | `crates/jc-core/src/personal/diversity.rs` | ported；索引失效時整份重建 |
| `prune.go` | `crates/jc-core/src/personal/prune.rs` | rewritten：門檻與容量上限照搬，`rankChecker` 的同音安全檢查暫以最弱值淘汰取代（整合點見下） |
| `unlearn.go` | `crates/jc-core/src/personal/unlearn.rs` | rewritten：`Undo` 保存操作前後整份 `Store`，復原時逐項比對；語意相同，記憶體用量較高 |
| `preference.go` | `crates/jc-core/src/personal/preference.rs` | ported |
| `corrections.go` | `crates/jc-core/src/personal/corrections.rs` | ported：精確與沒打聲調的相容比對、當次覆寫、補全證據、容量逐出 |
| `pronoun.go` | `crates/jc-core/src/personal/pronoun.rs` | ported |
| `english.go` | `crates/jc-core/src/personal/english.rs` | ported；英文詞典用 `EnglishLexicon` trait 隔開 |
| `language.go` | `crates/jc-core/src/personal/language.rs` | ported；語言 ID 用 `LanguageId`，不用前身的 `lang.ID` |
| `vocab.go` | `crates/jc-core/src/personal/vocab.rs` | ported；`VocabAdd`／`Remove`／`SetReadings`，批次復原回傳 `None`（整合點見下） |
| `emoji.go` | `crates/jc-core/src/personal/emoji.rs` | ported：`RememberEmoji`／`RememberSymbol`／`RememberPaletteTab` |
| `zhuyin_keys.go` | `crates/jc-core/src/personal/zhuyin_keys.rs` | ported |
| `ja.go` | `crates/jc-core/src/personal/ja.rs` | ported |
| `*_test.go` | `crates/jc-core/tests/personal_port.rs` | rewritten：94 個 Go 測試名全部對應，使用本版 API 斷言同一條規則 |

## 資料格式（格式 1）

存檔是 JSON，欄位用 snake_case。與前身格式 8 不相容；讀到 `format > 1`
回傳 `PersonalError::NewerFormat`，不猜測、不回寫。

主要欄位：

| 欄位 | 意義 |
|---|---|
| `format` | 格式版本 |
| `picked` | 音節 → 手選過的單字 |
| `uni`／`bi`／`ctx`／`total` | 階層式貝氏調適的個人計數 |
| `evidence_by_context`／`evidence_by_word` | 近期＋長期使用證據（`short`、`long`、`updated`、`day`、`day_weight`） |
| `corrections` | 讀音＋左右鄰 → 最近明確選取 |
| `confirmed` | 多字詞手選紀錄（讀音習慣證據） |
| `phrases`／`phrase_seen`／`phrase_order` | 個人片語與晉升順序 |
| `tombstones` | 不再推薦的詞 |
| `readings`／`rejects` | 個人讀音與被推翻的自動修正 |
| `zhuyin_keys` | 前字＋注音符號 → 直接出注音的紀錄 |
| `pronouns` | 人稱代名詞習慣 |
| `english` | 正規化英文詞 → 有界證據 |
| `emoji_recent`／`emoji_stats`／`symbol_recent`／`symbol_stats`／`palette_tab` | 選單記憶 |
| `languages`／`cross_language` | 分語言與跨語言學習 |

自訂詞 `vocab` 不寫進個人資料，和前身一樣由外層載入；`Store::augment_lexicon`
把它接進詞格。

## 規則

- **被動證據**：預設接受首選記 0.25；接受補全 0.5；明確手選 1.0。長期證據
  每個 UTC 日至多加 1，短期半衰期 2 日、長期 30 日。
- **單字手選**：`picked` 直接釘選該音節的候選，照前身 `PickedSingle`。
- **多字詞**：`confirmed` 累積讀音習慣，需跨日證據過門檻才加分，上限
  `Params::habit_lift`。
- **忘記**：撤銷學習並留 `tombstones`；被動提交不得學回，只有再次明確手選、
  親手加詞或 `Relearn` 能解除。
- **移除**：撤銷學習但不留 tombstone，之後照常學回來。
- **候選排序**：`Store::rank_key` 先釘選自訂詞與手選單字，再用 `uni` 折抵
  成本，折扣上限 1500 毫成本；`PersonalView` 提供加上個人片語與自訂詞的
  詞庫視圖。
- **兩種復原鍵**：`Shift+Delete` 對學到的詞走 `Forget`；`Shift+Backspace`
  對自訂詞走 `VocabRemove`。外殼由 `Session::forget_word` 與
  `Session::forget_candidate` 接入。

## 整合點與已知差異

| 項目 | 現況 | 整合點 |
|---|---|---|
| `lexicon.Source` | 由 `LexiconSource` trait 取代，`InMemoryLexicon` 有實作 | 二進位詞庫接上 `Model::new` |
| `lm.System` | 由 `SystemLm` trait 取代，`BigramLm` 有 blanket impl | `ZhLm` 可直接當 `SystemLm` |
| `english.Dictionary` | 由 `EnglishLexicon` trait 隔開；`NoEnglish` 是空實作 | 英文 pack 實作 `prefix`／`corrections` |
| 神經重排 | 尚未接 `Model`；`Session` 仍用 `RerankOptions` 替身 | `Model::source()` 與 `reading_lift` 可接到 decoder |
| `prune` 的同音安全檢查 | 目前只按最弱值淘汰，沒有 `rankChecker` 的區間比較 | 接上 `PersonalView::exact` 後補 |
| 自訂詞批次復原 | `vocab_undo` 回傳 `None` | 需要宿主保存 `VocabBatch` 歷史 |
| 混合輸入／英文補字 | 學習規則已搬，外殼尚未呼叫 `learn_english` | `Session` 提交時呼叫 |

## 驗證

```sh
cargo check -p jc-core -p jc-cli --all-targets
cargo test --workspace
cargo run -p jc-cli --release -- eval zh-golden
```

- `cargo test --workspace`：全綠。
- `eval zh-golden`：45/45。
- `cargo test -p jc-core --test personal_port`：97 個案例，對應前身 94 個測試名。

## 注音 golden 的四個案例

| 案例 | 本版行為 |
|---|---|
| `learning-select-preference` | 首次 `Down` 只開候選窗，第二次移到「井」；手選記入 `picked`，同讀音再打時「井」排第一 |
| `learning-forget-request` | `forget` 請求呼叫 `Session::forget_word`，撤銷 `picked` 與證據，排序回到「景」 |
| `undo-learned-word` | 候選窗 `Shift+Delete` 走 `forget_candidate`，忘記學到的詞並留 tombstone |
| `undo-custom-word` | golden seed 的自訂詞加入 `Store::vocab`，`Shift+Backspace` 走 `VocabRemove` |

## Clippy 清理（第二輪）

- `personal/mod.rs`：刪掉未使用的自由函式 `now()`；`Model::now()` 已由 `now_source` 走實際路徑。刪掉 `Model::undo` 欄位，本版 `Undoable` 以整份 `Store` 快照實作，不需要上游的 `recording()` 錄製欄位。
- `personal/diversity.rs`：刪掉自建的 `refresh_diversity()`；索引重建仍由 `diversity()` 的失效檢查走實際路徑。`diversity_units` 的比較改成 `value <= 0.0 || value.is_nan()`。
- `personal/english.rs`：`EnglishIndex` 的 `gen`／`store`／`base`／`index` 接進 `english_corrections`，用版本與詞典位址判斷快取失效；`clean_english` 去掉多餘的 `let`。
- `personal/prune.rs`：刪掉本版自行加入、上游沒有的 `decay_since`、`pair_key`。
- `rank_hypotheses`／`rank_candidates` 改成 `&mut [_]`。
- 上游 `personal.go` 的 `Model.undo` 有呼叫端（`recording()`），但本版改用快照式 `Undoable`，因此不再需要；原因是實作策略不同，不是漏接。
