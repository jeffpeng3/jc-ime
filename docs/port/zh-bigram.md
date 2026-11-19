# zh-bigram：把 bigram 接進注音解碼

## 範圍

`zh-data` 已能把 `lm.bin` 的 unigram 與 bigram 讀進來，但注音解碼只用
unigram 與 `P(r|w)`，所以 `build-zh` 與 `build-zh-bigram` 的 5-best 完全
相同。這個 task 把前身的 bigram 內插接進路徑評分，並讓
`convert`／`eval zh-golden` 能指定或自動載入語言模型。

## 公式

前身的機率不是寫在 `core/score`（那是神經重排的 scorer 介面），而是
`core/personal.Model.Prob`（沒有個人資料時就是 `SystemProb`）：

```text
P(w | left) = μ·P_bi(w | left) + (1−μ)·P_uni(w)
```

- `μ = personal.DefaultParams().Mu = 0.7`，只透過 `Params.Mu` 調整
  （`0c-eval --mu`）。Rust 對應 `ScoreContext::bigram_mu` 與
  `jc-cli --mu`，預設值 `score::DEFAULT_BIGRAM_MU`。
- `P_uni` 未登錄時是 `core/lm.Unseen = 1e-8`，對應
  `score::UNSEEN_UNIGRAM`。
- 詞格邊的成本原本是 `-ln P(r|w) + -ln P_uni(w)`（加上長度折扣）。
  `score::LmScorer` 只加上「內插後 − unigram」的差額：

  ```text
  cost' = cost + [ -ln P(w|left) − (-ln P_uni(w)) ] · 1000
  ```

  因此 `P(r|w)`、未知詞成本與長度折扣都不變；`--mu 0` 時差額為 0，
  輸出與沒有 bigram 完全相同。
- 句首照前身 `decode.InitialLeft() = lm.BOS`：第一個詞的 `left` 是
  `<s>`，只拿來查 bigram，不會出現在輸出。`lm.bin` 沒有 `<s>` 的
  bigram 時該項為 0，與 Go 相同。

## 解碼狀態

前身 `decode.state` 的鍵是「前一個詞面」；Rust 的
`decode::PathState` 原本只以詞性合併。現在前向 DP 與 N-best 的狀態鍵是
`(前一個詞面, 前一個詞性)`：

- `decode/mod.rs`：`forward`、`put_best`、`path_less` 的狀態表。
- `decode/nbest.rs`：每個位置的前 `k` 條路徑依前詞分桶；`beam` 限制每個
  位置展開幾個前詞狀態。
- `lattice/viterbi.rs`：公開的 Viterbi 同步改成詞面鍵，`Scorer::edge_cost`
  在弧上計價。
- 成本在邊（弧）上算，不在節點上；節點只累計路徑成本。

`beam` 對應前身 `core/session.Options.Beam = 8`：只在接上 bigram 時啟用，
沒有 bigram 時 `beam = 0`，結果與舊版逐詞性保留完全相同。

## Go 對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/personal/personal.go`（`Params.Mu`、`Model.Prob`、`SystemProb`） | `crates/jc-core/src/score.rs`（`DEFAULT_BIGRAM_MU`、`LmScorer::probability`／`delta`） | ported |
| `core/lm/lm.go`（`System.Uni`／`Bi`、`BOS`、`Unseen`） | `crates/jc-core/src/score.rs`（`BigramLm`、`ZhLm`、`BOS_WORD`、`UNSEEN_UNIGRAM`） | ported |
| `core/lm/compact.go`（`Uni`／`Bi` 查詢） | `crates/jc-data/src/zh/lm.rs`（`uni`／`bi`，`zh-data` 已完成） | ported |
| `core/decode/decode.go`（`state.last`、`forward`、`arcLog`） | `crates/jc-core/src/decode/mod.rs`（`PathState`、`forward`、`step_cost`） | rewritten：成本模型用 `i32` 毫成本與詞性連接成本，沒有個人化與 evidence |
| `core/decode/decode.go`（`frontier`、`Beam`） | `crates/jc-core/src/decode/mod.rs`（`frontier`）、`decode/nbest.rs`（`frontier_keys`） | rewritten：直接以路徑成本排序，未實作 `Contextual` 介面 |
| `core/decode/decode.go`（`InitialLeft`、`lm.BOS`） | `crates/jc-core/src/score.rs`（`BOS_WORD`）、`LmScorer::word_cost` | ported |
| `core/decode/decode.go`（`group`／尾端前綴的 `Prob`） | `crates/jc-core/src/decode/mod.rs`（`prefix_states`）、`decode/nbest.rs` | rewritten：尾端詞條直接走 `Scorer::arc_cost` |
| `core/decode/context_test.go`（`TestCandidatesUseRightContextAndRespectOtherLocks` 等） | `crates/jc-core/src/decode/nbest.rs`（`nbest_uses_bigram_context_of_previous_word`） | rewritten：候選視窗的 `CandidatesInContext` 仍是詞性近似，見下方限制 |
| `core/decode/nbest_test.go`（`TestNBestKeepsHistoriesAndDeduplicatesSegmentations`） | `crates/jc-core/src/decode/nbest.rs`（`nbest_dedupes_same_text_and_orders_by_cost`） | ported |
| `core/score/score.go` | `crates/jc-core/src/score.rs`（`Scorer`） | n/a：這是神經重排的 scorer／仲裁介面，不是語言模型內插 |
| `cmd/0c-eval/main.go`（`--mu`） | `crates/jc-cli/src/cli.rs`（`--mu`） | ported |
| `core/session/session.go`（`Options.Beam = 8`） | `crates/jc-core/src/decode/mod.rs`（`DEFAULT_BEAM`） | ported（只接 bigram 時） |

## 限制

- `Decoder::candidates_in_context`（候選視窗右側上下文）仍以詞性合併狀態，
  有 bigram 時是近似；它不在 session 的解碼路徑上。
- `mixed` 模式的注音路由沒有接語言模型：`Router` 只拿 `ScoreContext`。
- 尾端補全的 `top_b` 仍只依成本取前幾個狀態；前身用 `TopB`／`TopM` 的
  完整近似沒有搬過來。

## 驗證

```sh
cargo run -p jc-cli --release -- convert "ㄨㄛˇ ㄞˋ" --data /mnt/work/jc/jc-ime-data/build-zh
cargo run -p jc-cli --release -- convert "ㄨㄛˇ ㄞˋ" --data /mnt/work/jc/jc-ime-data/build-zh-bigram
cargo run -p jc-cli --release -- eval zh-golden
```

`ㄨㄛˇ ㄞˋ` 的 5-best（表面相同，成本不同）：

| 排名 | build-zh | build-zh-bigram |
|---|---|---|
| 1 | 我愛 13366 | 我愛 12170 |
| 2 | 我艾 14803 | 我艾 17211 |
| 3 | 我礙 17997 | 我礙 20405 |
| 4 | 我隘 19310 | 我隘 21718 |
| 5 | 我曖 19338 | 我曖 21746 |

`我→愛` 的 bigram 讓第一名降 1196，與第二名的差距從 1437 拉到 5041。
同一個輸入的 5 個表面不變，是因為所有候選的第一個詞都是「我」；換成
有真正歧義的組合就會換位，例如 `ㄗㄞˋ ㄐㄧㄢˋ`：

| 排名 | build-zh | build-zh-bigram |
|---|---|---|
| 1 | 再見 | 再見 |
| 2 | 在見 | 在建 |
| 3 | 在件 | 在見 |

`--mu 0` 時 `build-zh-bigram` 的 5-best 與 `build-zh` 逐項相同（表面與
成本都一樣），確認沒有 bigram 或關掉內插時完全退化。

`convert` 的 `--stats` 直接顯示查詢次數：

```text
build-zh         bigram queries=0   hits=0
build-zh-bigram  bigram queries=496 hits=1   # 命中的是 我→愛
```

`ㄨㄛˇ ㄞˋ` 的表面順序不變是資料決定的：五個候選的第一個詞都是「我」，
而 `ㄞˋ` 的候選裡只有「愛」有 `我→愛` 的 bigram（`lm.bin` 中 我 的 997
筆後接詞只有「愛」屬於這個讀音），所以 bigram 只把第一名拉得更前面，
不會換位。真正會換位的輸入是 `ㄗㄞˋ ㄐㄧㄢˋ`（在見 ↔ 在建）。

## 與 main 合併後

base 合併 `main`（`af59d44`，含 `zh-golden-rest`）後：

| 指令 | 結果 |
|---|---|
| `eval zh-golden` | **41/45** |
| `eval zh-golden --lm build-zh-bigram/lm.bin --stats` | **41/45**，`bigram queries=22747 hits=9743` |
| `eval zh-golden --lm ... --mu 0` | 41/45，成本與 μ=0.7 不同但通過題數相同 |

golden 加 bigram 後查了 22,747 次、命中 9,743 次，分數確實改變（例如
`ㄨㄛˇ ㄒㄧㄤˇ ㄔ` 的最佳成本 20540 → 19238），但 41/45 的通過集合不變：
凍結詞庫每個讀音幾乎只有一個 `P(r|w)=1` 的候選，沒有能讓 bigram 翻盤的
同分組合。

候選順序不是由 `candidates_in_context` 決定的：這個方法在 workspace 沒有
任何呼叫端，`Session::rebuild_zh` 的最佳路徑用 `Decoder::best`、候選清單
用 `Decoder::nbest`，兩者都接上 `LmScorer`。所以「Viterbi 沒機會用
bigram」不成立；`--stats` 的查詢次數是直接證據。
