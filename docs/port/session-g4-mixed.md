# session-g4-mixed：混合與語言群組

上游：`/mnt/work/jc/ref/0c-ime` `8f443af`。本輪做差異補齊，沒有重寫路線。

## 上游有、我們有、我們缺

| 上游有 | 我們有 | 我們缺（本輪處理） |
|---|---|---|
| `pendingKeys`、`mixedText`、`clearMixed` | `session::mixed::AsciiInput` 的 `pending` 與 `pending_display` | 只有數字與標點、英數反序、數字加字母組不成音節的字面顯示（已補）；`assistedPrefix`、`choice` 仍未接 |
| `finishMixedTone`、`mixedBoundary`、`mixedComposer` | `AsciiInput::tone`／`space`／`boundary` | 英→中切點只看詞表英文前段；`mixedAfterAlnum`、`mixedAfterToken`、`mixedAfterTypo`、`mixedAfterKnownAlnum` 未搬 |
| `mixedSpaceNumber`、`digitWord`、`digitsReadAsChinese`、`rareDigitReading`、`digitToneNumber` | `space` 先試一聲讀音、再切點、最後字面；聲調鍵走 `tone` | 上游靠 `literalNumberBefore` 判斷「緊接數字」，Rust 沒有跨片段數字上下文；罕用字門檻、反序數字規則未搬 |
| `mixedToneChars`（↓ 選中文字）、`mixedKeys` | `reading_candidate`（注音本身、結尾聲調注音） | 中文字候選需要 `mixedKeys`、鎖定與顯示保留；未實作 |
| `openMixedCandidates`、`selectMixed` | `Session::rebuild_mixed`：原按鍵、注音、英文前綴／拼字 | `mixedChars` 選取、`choice=2` 的注音解釋路徑未接 |
| `mixedSuggestion`、`acceptMixed` | `AsciiInput::suggestion`／`accept_suggestion`；`MAX_WORD_BYTES` 上限 | 個人英文字彙（「我的詞」）優先與 `vocabEnglish` 沒有接進 `Session`（需要 `personal::Model`） |
| `mixedEnglishCommits`、`language_learning.go` | `Session::learn_mixed_languages`：送出時記 Zh／En 選字 | 英文提交事件（assisted／explicit／suspicious）與跨語言偏好排序未接 |
| `pronoun.go` 的代名詞習慣 | `personal::pronoun` 的模型與測試已存在 | Session 只有 `PersonalStore`，沒有 `personal::Model` 與顯示路徑，接線留給 rerank／misc |
| `language.go` 的 profile、runtime、`SetLanguage`／`InputMethod` | `lang::Mode` 與 `SessionConfig` 涵蓋目前四種模式 | 可插拔語言 profile 與語言專屬組字狀態未實作 |
| `ja_mixed.go` 的按鍵推斷 | `Session::try_japanese_run`：完整羅馬字自動轉日文，送出後回混合 | 沒有 `kana-first`／`ambiguous=both` 設定；Space 変換候選未接 |
| `ja_cross.go` 的日英同形候選 | 無 | 依賴 `languageRuntime` 的 clause／candidate；`Mode::Ja` 目前直接顯示最佳轉換 |
| `english_autocomplete.go` | `english_candidates`、`accept_suggestion`、`MAX_WORD_BYTES` | Escape 關閉建議、上下鍵選取、`englishTokenBlocked`、邊界送出規則未接 |
| `cjk_latin_space.go` | `mixed.rs` 的 `space_text(Spacing::Always)` | Rust 版本把假名與全角拉丁字母也算進去（Go 只算漢字與 ASCII 字母）；保留現行為 |

## 逐檔對照

| Go 檔案 | Rust 檔案 | 狀態 |
|---|---|---|
| `core/session/mixed.go` | `crates/jc-core/src/session/mixed.rs`（`AsciiInput`、`rebuild_mixed`、`mixed_surface`、`reading_candidate`、`english_candidates`）、`crates/jc-core/src/mixed.rs`（`Router`、英文詞表） | rewritten：核心顯示、空白、字面、候選與上限已補；切點變體、罕用字、`mixedToneChars` 未搬 |
| `core/session/language.go` | `crates/jc-core/src/lang.rs`、`session/mod.rs` | n/a：Rust 用 `Mode`＋`SessionConfig`，沒有可插拔 `lang.Profile`／`languageRuntime`；`SetLanguage`、`InputMethod` 沒有對應的產品介面 |
| `core/session/ja_cross.go` | 無 | n/a：日英同形需要語言 runtime 的 clause 與候選清單；目前 `Mode::Ja` 直接顯示最佳轉換 |
| `core/session/ja_mixed.go` | `crates/jc-core/src/session/mixed.rs`（`try_japanese_run`、`leave_empty_japanese_run`） | rewritten：完整羅馬字自動轉日文、送出後回混合；沒有日英並列與明確優先序設定 |
| `core/session/english_autocomplete.go` | `crates/jc-core/src/session/mixed.rs`（`suggestion`、`accept_suggestion`、`english_candidates`） | rewritten：詞長上限、補全與更正、Tab 接受；個人英文字彙與 Escape／方向鍵未接 |
| `core/session/language_learning.go` | `crates/jc-core/src/session/mixed.rs`（`learn_mixed_languages`） | rewritten：混打送出記 Zh／En 選字；跨語言排序與 undo 未接 |
| `core/session/pronoun.go` | `crates/jc-core/src/personal/pronoun.rs`（既有模型） | n/a：Session 沒有 `personal::Model` 與 `path`／`hint` 顯示路徑，等 rerank／misc 群組接線 |
| `core/session/cjk_latin_space.go` | `crates/jc-core/src/mixed.rs`（`space_text`、`Spacing::Always`） | ported：漢字與 ASCII 字母之間補半形空格；Rust 另外涵蓋假名與全形字母 |

## 搬移與新增

- `crates/jc-core/src/mixed.rs`：`AsciiInput`、`AsciiPiece`、`compose_keys`、`keys_in_syllable_order`、`mixed_reading`、`reading_reachable` 搬到 `session/mixed.rs`；`mixed.rs` 只留 `Router`、英文詞表與 `space_text`。
- `crates/jc-core/src/session/core.rs`：`rebuild_mixed`、`mixed_surface`、`decode_syllable`、`english_candidates` 搬到 `session/mixed.rs`；`learn_candidate` 的 `Mode::Mixed` 分支改呼叫 `learn_mixed_languages`；空白與退格加日文片段 hook。
- `crates/jc-core/src/session/mod.rs`：新增 `ja_run`、`ja_prev_mode` 兩個狀態欄位，`clear_composition` 結束日文片段時還原模式。
- `crates/jc-core/src/session/mixed.rs`：新增 7 個行為測試，涵蓋顯示、空白、注音候選、英文上限、語言學習、日文推斷。

## 驗證

- `cargo test -p jc-core --lib session::`：93 passed。
- `cargo check -p jc-core --all-targets`：通過。
- `cargo clippy --workspace --all-targets -- -D warnings`：通過（只有環境的 `-Ctarget-feature` 警告）。
- `eval mixed`：6/6。
- `eval zh-golden`：45/45。
- `eval ja --corpus modern-500`：86.0%。
