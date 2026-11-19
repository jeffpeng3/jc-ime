# session-core-split：session 重構 Stage 0

上游：`/mnt/work/jc/ref/0c-ime` `8f443af`。範圍：把 `crates/jc-core/src/session.rs`
拆成 `session/mod.rs` 與 `session/core.rs`，行為、常數、測試都不變。

## 逐檔對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/session/session.go`（既有對應部分） | `crates/jc-core/src/session/mod.rs` | moved：`Session`、`SessionConfig`、`RerankOptions`、`CORE_VERSION`、`DEFAULT_MAX_CANDIDATES`、`PAGE_SIZE`、`DEFAULT_FUZZY_PAIRS` 與全部公開 API。 |
| `core/session/session.go`（主狀態機） | `crates/jc-core/src/session/core.rs` | moved：按鍵分派、候選重建、學習、選單、送出前處理與混合字面。 |
| `core/session/mixed.go` | `crates/jc-core/src/mixed.rs` | deferred：目前沒有 `impl Session` 可搬，`AsciiInput`／`AsciiPiece` 留給 Stage 1 的 `mixed` 群組。 |
| `core/session/session_test.go` | `crates/jc-core/src/session/mod.rs` 的 `tests` | n/a：本輪不搬測試，既有 41 個測試原樣保留在 `mod.rs`。 |

## 切分原則

1. `mod.rs` 只放型別、設定、常數、模組文件、`Default` 與公開 API。公開方法
   依原順序保留，事件流（`press`、`set_mode`、`tick_live`、`commit`）都在這裡。
2. `core.rs` 放原本的私有實作方法：狀態轉移、候選重建、學習、選單與輸出
   前處理。方法本體逐字搬移，只有兩處因新增 `pub(super)` 超過行寬，由
   `cargo fmt` 換行。
3. `Session` 的欄位全部改成 `pub(super)`，讓 `core.rs` 讀寫；沒有改名、沒有
   新增或刪除欄位。`MenuKind`、`Menu` 與四個實作常數同樣是 `pub(super)`。
4. `now_seconds` 只有 `core.rs` 使用，搬到 `core.rs`。
5. `mod core;` 是私有子模組；外部只看得到 `mod.rs` 的公開 API。
6. `lib.rs` 的 `pub mod session;` 與 `pub use session::CORE_VERSION;` 不變。

## 欄位分組預定表（Stage 1–5）

| 欄位 | 主群組 | 其他讀取者 | 備註 |
|---|---|---|---|
| `config` | core | 全部 | `set_mode` 是唯一寫入點；各群組只讀自己的設定。 |
| `zh` | input_style | segments | 注音按鍵、待定音節與已完成音節。 |
| `romaji` | mixed | core | 日文輸入累積；`tick_live` 讀 `units()`。 |
| `lexicon`、`base_lexicon` | segments | fuzzy、mixed、misc | `misc` 的自訂詞變動會重建 `lexicon`。 |
| `personal` | misc | mixed | 注音學習在 `misc`，日文學習在 `mixed`。 |
| `ctx`、`zh_lm` | segments | fuzzy | 詞格與 bigram 評分。 |
| `literal`、`literal_anchor`、`literal_offset` | misc | symbols、segments | 字面按鍵插入位置；選單選取也走同一條路。 |
| `raw` | mixed | core | 英文模式原始輸入。 |
| `candidates`、`highlighted`、`candidate_window` | segments | core | 候選呈現與選取；`core` 負責移動與數字鍵。 |
| `segments` | segments | core | 目前轉換分段。 |
| `base`、`committed`、`pending` | core | segments | 已轉換、已送出左文、未轉換尾巴。 |
| `pending_commit` | core | 無 | `Enter` 與空白鍵的待提交結果。 |
| `accepted_completion`、`suggestion` | mixed | core | 英文補全；`core` 的 Tab 與送出會讀。 |
| `last_hypotheses` | segments | misc、rerank | 注音 N-best 快照。 |
| `last_ja_results` | mixed | misc | 日文 N-best 快照，提交時學文節。 |
| `ascii` | mixed | core | 混合模式按鍵推斷。 |
| `menu`、`palette_tab` | symbols | core | 開啟中的選單與上次分頁。 |
| `symbol_recent`、`symbol_stats`、`emoji_recent`、`emoji_stats` | symbols | 無 | 最近使用順序與常用分數。 |
| `live`、`live_plan` | mixed | core | 日文逐字変換；`tick_live` 在公開層。 |
| `learn_events` | misc | 無 | 清理個人資料的計數。 |

群組檔案與狀態：

| 群組 | 預定檔案 | 狀態 |
|---|---|---|
| core | `session/core.rs` | 本輪完成邊界。 |
| input_style | `session/input_style.rs` | 待 Stage 1。 |
| segments | `session/segments.rs` | 待 Stage 2。 |
| symbols | `session/symbols.rs` | 待 Stage 3。 |
| mixed | `session/mixed.rs` | 待 Stage 1／群組 4；`mixed.rs` 目前未動。 |
| misc | `session/misc.rs` | 待 Stage 5。 |
| fuzzy | `session/fuzzy.rs` | 待 Stage 6。 |
| rerank | `session/rerank.rs` | 待 Stage 7。 |

## 未完成與後續

- `mixed.rs` 沒有 `impl Session`，本輪不動。`AsciiInput`、`AsciiPiece` 與
  `space_text` 只被 session 使用，但屬混合群組，留到群組 4 一起搬，避免現在
  製造 `mixed.rs` 測試與 `Router` 的衝突。
- `Session` 欄位已是 `pub(super)`；Stage 1–5 的群組檔案都是 `session/` 子模組，
  沿用同一可見性即可，不需要再放寬到 `pub(crate)`。
- 沒有引入 trait 或抽象層；`core.rs` 只是原本的方法搬位置。

## 驗收

- `cargo test --workspace --no-fail-fast`：通過。
- `cargo clippy --workspace --all-targets -- -D warnings`：通過，只有環境的
  `-Ctarget-feature` 警告。
- `cargo fmt --all -- --check`：通過。
- `eval zh-golden`：45/45。
- `eval ja --corpus modern-500`：86.0%。
- 測試數量不變：`session` 41 個、`mixed` 原有測試未動。
