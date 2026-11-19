# m2-cli-session：CLI 與 session

把 `z1` 到 `z3`、`j1` 到 `j3`、`m1-mixed` 的零件接成可用的 session、引擎與
開發 CLI。`jc-core::session` 只做本任務書列的 API（`new`／`press`／`commit`／
`reset`），Go `core/session` 的其他功能沒有整包搬。

## 對照表

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/session/session.go`（薄層：輸入→候選→送出） | `crates/jc-core/src/session.rs`：`Session`、`SessionConfig` | rewritten：只保留 `press`／`commit`／`reset` 與快照，個人化、學習、async rerank 不在本輪 |
| `core/session/session_test.go`：`TestCommitExactlyOnce` | 同上：`test_commit_exactly_once` | ported |
| `core/session/segments_test.go`：`TestSessionSegmentsFollowZhuyinPathAndCursor` | 同上：`test_session_segments_follow_zhuyin_path_and_cursor` | rewritten：沒有 `SetCursor`，只驗片段、送出字串與 UTF-16 游標 |
| `core/session/segments_test.go`：`TestSessionSegmentsKeepLongWordMerged` | 同上：`test_long_word_stays_merged` | ported |
| `core/session/session_test.go`（退格、取消） | 同上：`test_backspace_removes_last_syllable`、`test_escape_clears_composition` | rewritten：只保留薄層行為 |
| `core/session/candidate_*`（選字鍵、頁面） | 同上：`test_candidates_are_ranked_and_tab_moves_highlight`、`test_digit_selects_candidate` | rewritten：`Tab`／左右鍵開候選窗，數字鍵選字；分頁與詞長切換未實作 |
| `core/engine` 的組字狀態 | `crates/jc-engine/src/lib.rs`：`Engine::handle` 改呼叫 `Session` | rewritten：握手、訊框、錯誤處理不變 |
| `core/engine` 的 `Key` 契約 | `crates/jc-proto/src/lib.rs`：`pub use jc_core::{Key, KeyCode}` | ported（已知債清償：移除 jc-proto 的本地 `Key` 與 `NamedKey`） |
| `cmd/0c-eval-single`（注音 golden 驅動） | `crates/jc-cli/src/eval_cmd.rs`：`zh_golden` | rewritten：直接驅動 `Session`，跑 `golden/zh/cases.json` 45 例並比對 `expected/*.jsonl` 的 commit |
| `cmd/0c-eval-ja` | `crates/jc-cli/src/eval_cmd.rs`：`ja` | rewritten：用 `jc-eval::run_ja` 與 curated `words.tsv`／`connection.tsv` |
| `cmd/0c-eval-fuzzy` | `crates/jc-cli/src/eval_cmd.rs`：`fuzzy` | rewritten：依讀音自動選注音或日文轉換器 |
| `cmd/0c-cli` 的轉換／bench | `crates/jc-cli/src/converter.rs`、`crates/jc-cli/src/bench.rs` | rewritten：只保留 `convert` 與 `bench` |
| `internal/eval/zh_gold.go` | `crates/jc-cli/src/eval_cmd.rs`、`jc-eval` 既有 `Report` | rewritten：不重做題庫凍結流程，直接讀已凍結的 `0c-testdata` |
| `core/session` 其餘 12.7k 行 | — | n/a：本任務書明確排除；個人化、學習、簡繁、模糊音、重排、補點屬其他 task |

## 刻意差異

| 項目 | 說明 |
|---|---|
| 資料來源 | 注音用 `0c-testdata/golden/zh/data/lexicon.tsv`＋`lm.tsv`（與前身同一份）；日文優先 `ja-src/lexicon/words.tsv`＋`ja-src/connection.tsv`，與 `baseline.json` 的 `data/ja` 相同。curated 不存在時才退回 `packs/*.pack`。 |
| 沒有詞典 | 注音詞格只有未知詞邊，輸出等於讀音本身；引擎行程因此不需要資料也能跑完協定往返。 |
| 成本換算 | TSV 的機率轉成 `-ln(p) * 1000`；注音未知詞成本 30000。`eval ja` 的 `top1 23.2%`、`boundary_exact 14.4%`、`boundary_f1 0.4454` 與 `ja/baseline.json` 的 modern-500 完全一致。 |
| `Key` 序列化 | 改用 `jc_core::Key` 後，協定上的按鍵是 `{code:{kind,value},text,shift}`；`NamedKey` 移除，`Up`／`Down` 等由 wrapper 對應到 `ArrowLeft`／`ArrowRight`。傳輸、版本與錯誤處理不變。 |
| 游標 | `Snapshot::cursor_utf16` 用 UTF-16 code unit；協定 `State.cursor` 仍照原註解換算成字元數。 |
| 候選窗 | `Tab`／左右鍵才開啟候選窗；數字鍵只在窗開時選字，避免打注音聲調被誤判成選字。 |
| 未實作 | 注音無聲調展開、候選分頁、Tab 補全、簡繁轉換、學習／忘詞、混合模式實體鍵重播、自動補標點、重排與模糊音。`eval zh-golden` 會把這些案例列為失敗並照實印出通過數。 |

## 測試

| 測試 | 資料 | 缺資料時 |
|---|---|---|
| `jc-core::session::tests::*` | 記憶體詞典 | 不需要外部資料 |
| `jc-engine` 單元測試與 `tests/stdio.rs`、`tests/socket.rs` | 無（未接資料的 `Session`） | 不需要外部資料 |
| `jc-cli::cli`、`eval_cmd`、`converter`、`data` 單元測試 | 記憶體詞典或暫存檔 | 不需要外部資料 |

`cargo test` 依共通規則留給整合階段；本 task 只確認 `cargo check --all-targets`。

## 實跑結果（開發機）

| 指令 | 結果 |
|---|---|
| `jc-cli eval zh-golden` | 45 例，通過 27 例；失敗集中在未實作的選項（學習、簡繁、混合實體鍵、補全、重排、模糊音） |
| `jc-cli eval ja --corpus modern-500` | top1 23.2%／top5 23.2%／boundary_exact 14.4%／boundary_f1 0.4454（與 baseline 相同） |
| `jc-cli eval fuzzy` | 919 例，top1 11.0% |
| `jc-cli convert "わたしはほんをよむ。"` | `私は本を読む。` |
| `jc-cli convert "ㄨㄛˇ ㄒㄧㄤˇ"` | `我想` |
