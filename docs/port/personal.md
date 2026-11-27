# 個人化學習

個人化資料存在 `personal.json`，由 `jc-engine` 載入與 flush；預設
`%LOCALAPPDATA%\jc-ime\personal.json`，可用 `--personal-dir` 改。內容是注音、日文
與英文共用的學習紀錄，不進資料包 manifest。

## 資料格式

JSON、snake_case、`format = 1`。讀到較新的格式回 `PersonalError::NewerFormat`，
不猜測也不回寫。

| 欄位 | 意義 |
|---|---|
| `picked` | 音節 → 手選過的單字 |
| `uni`／`bi`／`ctx`／`total` | 階層式貝氏調適的個人計數 |
| `evidence_by_context`／`evidence_by_word` | 近期與長期使用證據 |
| `corrections` | 讀音＋左右鄰 → 最近明確選取 |
| `confirmed` | 多字詞手選紀錄 |
| `phrases`／`phrase_seen`／`phrase_order` | 個人片語與晉升順序 |
| `tombstones` | 不再推薦的詞 |
| `readings`／`rejects` | 個人讀音與被推翻的自動修正 |
| `zhuyin_keys` | 前字＋注音符號 → 直接出注音 |
| `pronouns`／`english` | 人稱代名詞與英文證據 |
| `emoji_recent`／`emoji_stats`／`symbol_recent`／`symbol_stats`／`palette_tab` | 選單記憶 |
| `languages`／`cross_language` | 分語言與跨語言學習 |

自訂詞 `vocab` 不寫進個人資料，由外層載入後用 `Store::augment_lexicon()` 接進
詞格。

## 學習規則

- 被動證據：接受首選記 0.25、接受補全 0.5、明確手選 1.0。長期證據每個 UTC 日至多
  加 1；短期半衰期 2 日、長期 30 日。
- 單字手選：`picked` 直接釘選該音節的候選。
- 多字詞：`confirmed` 累積讀音習慣，需跨日證據過門檻才加分，上限
  `Params::habit_lift`。
- 候選排序：`Store::rank_key` 先釘選自訂詞與手選單字，再用 `uni` 折抵成本，折扣
  上限 1500 毫成本。
- 忘記：撤銷學習並留 `tombstones`；被動提交不得學回，只有再次明確手選、親手加詞
  或 `Relearn` 能解除。
- 移除：撤銷學習但不留 tombstone，之後照常學回來。
- `Shift+Delete` 對學到的詞走 `Forget`；`Shift+Backspace` 對自訂詞走
  `VocabRemove`。外殼由 `Session::forget_word` 與 `Session::forget_candidate`
  接入。

## 程式位置

| 檔案 | 內容 |
|---|---|
| `crates/jc-core/src/personal/mod.rs` | `Store`、`Model`、`Learn`、`Prob`、候選排序入口 |
| `personal/evidence.rs`、`diversity.rs`、`preference.rs`、`corrections.rs`、`pronoun.rs` | 證據、多樣性、偏好與修正 |
| `personal/prune.rs`、`unlearn.rs` | 容量淘汰、忘記與復原 |
| `personal/vocab.rs` | 自訂詞與讀音 |
| `personal/english.rs`、`zhuyin_keys.rs`、`ja.rs`、`language.rs` | 各語言學習 |
| `personal/emoji.rs` | 符號與表情選單記憶 |

`Store` 只依賴 `LexiconSource`、`SystemLm`、`EnglishLexicon` 等 trait；詞庫與
語言模型由呼叫端注入，個人化不自己開檔。`Undo` 保存操作前後整份 `Store` 快照，
復原時逐項比對。
